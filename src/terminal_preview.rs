//! Optional bitmap rendering. This module never reads terminal input or opens a source file.
//! Call prepare/hide then cleanup BEFORE Terminal::draw; clear its cache if cleanup returns true.
use image::{DynamicImage, RgbaImage};
use ratatui::{layout::Rect, Frame};
use ratatui_image::{
    protocol::{kitty::Kitty, sixel::Sixel, Protocol},
    Image,
};
use std::{
    io::{self, Write},
    sync::mpsc::{self, Receiver},
    time::{SystemTime, UNIX_EPOCH},
};

const MAX_PIXELS: usize = 1280 * 960;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Graphics {
    Sixel,
    Kitty,
}
#[derive(Clone, Copy, Debug)]
struct Capability {
    graphics: Graphics,
    cell: (u16, u16),
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Target {
    key: String,
    area: Rect,
    cell: (u16, u16),
}
struct Ready {
    target: Target,
    protocol: Protocol,
    id: u32,
}
struct Finished {
    target: Target,
    result: Result<(Protocol, u32), String>,
}
#[derive(Clone, Copy)]
struct Visible {
    graphics: Graphics,
    id: u32,
    area: Rect,
}

/// One bounded encoder at a time; stale completion never renders into a changed view.
pub struct NativePreview {
    capability: Option<Capability>,
    graphics_hint: Option<Graphics>,
    requested: Option<Target>,
    ready: Option<Ready>,
    worker: Option<Receiver<Finished>>,
    visible: Option<Visible>,
    cleanup_queue: Vec<Visible>,
    failed: Option<Target>,
}
impl NativePreview {
    pub fn detect() -> Self {
        let graphics_hint = detect_graphics();
        let mut native = Self::new(
            graphics_hint
                .and_then(|graphics| cell_size().map(|cell| Capability { graphics, cell })),
        );
        native.graphics_hint = graphics_hint;
        native
    }
    fn new(capability: Option<Capability>) -> Self {
        Self {
            capability,
            graphics_hint: capability.map(|c| c.graphics),
            requested: None,
            ready: None,
            worker: None,
            visible: None,
            cleanup_queue: Vec::new(),
            failed: None,
        }
    }
    pub fn supported(&self) -> bool {
        self.capability.is_some()
    }
    pub fn pending(&self) -> bool {
        self.worker.is_some() && self.requested.is_some()
    }
    /// Stable key must change when source pixels change. No filenames enter graphics commands.
    /// Returns immediately; caller retains its half-block fallback while encoding completes.
    pub fn prepare(&mut self, key: &str, width: u32, height: u32, rgba: &[u8], area: Rect) {
        // Foot initially reports 0x0 pixel geometry until compositor configure.
        // Nonblocking ioctl also observes font-size changes without consuming input.
        let observed = self
            .graphics_hint
            .and_then(|graphics| cell_size().map(|cell| Capability { graphics, cell }));
        if let (Some(old), Some(new)) = (self.capability, observed) {
            if old.cell != new.cell {
                self.hide();
            }
        }
        if observed.is_some() {
            self.capability = observed;
        }
        let Some(capability) = self.capability else {
            return;
        };
        let len = (width as usize)
            .checked_mul(height as usize)
            .and_then(|n| n.checked_mul(4));
        if width == 0
            || height == 0
            || width > 1280
            || height > 960
            || len != Some(rgba.len())
            || rgba.len() > MAX_PIXELS * 4
            || area.width == 0
            || area.height == 0
        {
            self.hide();
            return;
        }
        let target = Target {
            key: key.to_owned(),
            area,
            cell: capability.cell,
        };
        if self.requested.as_ref() != Some(&target) {
            self.hide();
            self.requested = Some(target.clone());
            self.failed = None;
        }
        if let Some(receiver) = &self.worker {
            match receiver.try_recv() {
                Ok(done) => {
                    self.worker = None;
                    if self.requested.as_ref() == Some(&done.target) {
                        match done.result {
                            Ok((protocol, id)) => {
                                self.ready = Some(Ready {
                                    target: done.target,
                                    protocol,
                                    id,
                                })
                            }
                            Err(_) => self.failed = Some(done.target),
                        }
                    }
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.worker = None;
                    self.failed = Some(target.clone());
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if self.worker.is_some()
            || self.ready.as_ref().is_some_and(|r| r.target == target)
            || self.failed.as_ref() == Some(&target)
        {
            return;
        }
        let pixels = rgba.to_vec();
        let (tx, rx) = mpsc::sync_channel(1);
        self.worker = Some(rx);
        std::thread::spawn(move || {
            let result = encode(capability, width, height, pixels, area);
            let _ = tx.send(Finished { target, result });
        });
    }
    pub fn render(&mut self, frame: &mut Frame<'_>, area: Rect) -> bool {
        let Some(ready) = &self.ready else {
            return false;
        };
        if ready.target.area != area || self.requested.as_ref() != Some(&ready.target) {
            return false;
        }
        frame.render_widget(Image::new(&ready.protocol), area);
        if let Some(capability) = self.capability {
            self.visible = Some(Visible {
                graphics: capability.graphics,
                id: ready.id,
                area,
            });
        }
        true
    }
    /// Must precede any overlay, view change, native attach, resize, and terminal restoration.
    pub fn hide(&mut self) {
        if let Some(visible) = self.visible.take() {
            self.cleanup_queue.push(visible);
        }
        self.requested = None;
        self.ready = None;
        self.failed = None;
    }
    /// Emits deletion only for graphics owned by this instance. A clear is needed because the
    /// terminal buffer diff can otherwise skip blank cells formerly covered by a bitmap.
    pub fn cleanup<W: Write>(&mut self, out: &mut W) -> io::Result<bool> {
        if self.cleanup_queue.is_empty() {
            return Ok(false);
        }
        for visible in &self.cleanup_queue {
            cleanup_visible(out, *visible)?;
        }
        out.flush()?;
        self.cleanup_queue.clear();
        Ok(true)
    }
}
impl Drop for NativePreview {
    fn drop(&mut self) {
        self.hide();
        let _ = self.cleanup(&mut io::stdout());
    }
}
fn encode(
    capability: Capability,
    width: u32,
    height: u32,
    rgba: Vec<u8>,
    area: Rect,
) -> Result<(Protocol, u32), String> {
    let Some(raster) = RgbaImage::from_raw(width, height, rgba) else {
        return Err("invalid raster".into());
    };
    // Never enlarge source pixels. Bound both raster and viewport, including hostile geometry.
    let w = (u32::from(area.width) * u32::from(capability.cell.0))
        .min(1280)
        .min(width);
    let h = (u32::from(area.height) * u32::from(capability.cell.1))
        .min(960)
        .min(height);
    let image = DynamicImage::ImageRgba8(raster).thumbnail(w, h);
    let cell_area = Rect::new(
        0,
        0,
        image.width().div_ceil(u32::from(capability.cell.0)) as u16,
        image.height().div_ceil(u32::from(capability.cell.1)) as u16,
    );
    let id = next_id();
    let protocol = match capability.graphics {
        Graphics::Kitty => {
            Protocol::Kitty(Kitty::new(image, cell_area, id, false).map_err(|e| e.to_string())?)
        }
        // Sixel has no alpha channel. Avoid rendering a transparent source on an invented opaque
        // background; the existing half-block renderer preserves terminal defaults correctly.
        Graphics::Sixel => {
            if image.to_rgba8().pixels().any(|p| p[3] != 255) {
                return Err("transparent raster uses half-block fallback".into());
            }
            Protocol::Sixel(Sixel::new(image, cell_area, false).map_err(|e| e.to_string())?)
        }
    };
    Ok((protocol, id))
}
fn next_id() -> u32 {
    use std::sync::atomic::{AtomicU32, Ordering};
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    if id != 0 {
        return id;
    }
    let seed = (SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos()
        ^ std::process::id())
    .max(1000);
    NEXT.store(seed.wrapping_add(1), Ordering::Relaxed);
    seed
}
fn cleanup_visible<W: Write>(out: &mut W, visible: Visible) -> io::Result<()> {
    match visible.graphics {
        Graphics::Kitty => write!(out, "\x1b_Ga=d,d=I,i={},q=2\x1b\\", visible.id),
        Graphics::Sixel => {
            // Local occupied cells only; no global graphics erase and no scrollback clearing.
            write!(out, "\x1b7")?;
            for row in visible.area.y..visible.area.bottom() {
                write!(
                    out,
                    "\x1b[{};{}H\x1b[{}X",
                    row + 1,
                    visible.area.x + 1,
                    visible.area.width
                )?;
            }
            write!(out, "\x1b8")
        }
    }
}
fn detect_graphics() -> Option<Graphics> {
    if std::env::var_os("NO_COLOR").is_some()
        || std::env::var_os("CX_ASCII").is_some()
        || std::env::var_os("TMUX").is_some()
    {
        return None;
    }
    let term = std::env::var("TERM").unwrap_or_default();
    let program = std::env::var("TERM_PROGRAM").unwrap_or_default();
    let kitty_window = std::env::var("KITTY_WINDOW_ID")
        .ok()
        .is_some_and(|v| v.parse::<u64>().is_ok_and(|n| n > 0));
    let ancestor = terminal_ancestor();
    select_graphics(&term, &program, kitty_window, ancestor.as_deref())
}
fn select_graphics(
    term: &str,
    program: &str,
    kitty_window: bool,
    ancestor: Option<&str>,
) -> Option<Graphics> {
    match (term, program, ancestor) {
        ("foot" | "foot-direct", _, Some("foot" | "footclient")) => Some(Graphics::Sixel),
        ("xterm-kitty", _, _) if kitty_window => Some(Graphics::Kitty),
        ("xterm-kitty", _, Some("kitty")) => Some(Graphics::Kitty),
        (_, "ghostty", Some("ghostty")) => Some(Graphics::Kitty),
        _ => None,
    }
}
#[cfg(target_os = "linux")]
fn terminal_ancestor() -> Option<String> {
    let mut pid = std::process::id();
    for _ in 0..12 {
        let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
        if status.len() > 16384 {
            return None;
        }
        let name = status.lines().find_map(|l| l.strip_prefix("Name:\t"))?;
        if matches!(name, "foot" | "footclient" | "kitty" | "ghostty") {
            return Some(name.to_owned());
        }
        pid = status
            .lines()
            .find_map(|l| l.strip_prefix("PPid:\t"))?
            .parse()
            .ok()?;
        if pid <= 1 {
            return None;
        }
    }
    None
}
#[cfg(not(target_os = "linux"))]
fn terminal_ancestor() -> Option<String> {
    None
}
#[cfg(unix)]
fn cell_size() -> Option<(u16, u16)> {
    use std::os::fd::AsRawFd;
    let mut size: libc::winsize = unsafe { std::mem::zeroed() };
    if unsafe { libc::ioctl(io::stdout().as_raw_fd(), libc::TIOCGWINSZ, &mut size) } != 0
        || size.ws_col == 0
        || size.ws_row == 0
    {
        return None;
    }
    let cell = (size.ws_xpixel / size.ws_col, size.ws_ypixel / size.ws_row);
    if cell.0 == 0 || cell.1 == 0 || cell.0 > 64 || cell.1 > 128 {
        return None;
    }
    Some(cell)
}
#[cfg(not(unix))]
fn cell_size() -> Option<(u16, u16)> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn known_identity_required_no_term_only_detection() {
        assert_eq!(select_graphics("foot", "", false, None), None);
        assert_eq!(
            select_graphics("foot", "", false, Some("foot")),
            Some(Graphics::Sixel)
        );
        assert_eq!(select_graphics("xterm-kitty", "", false, None), None);
        assert_eq!(
            select_graphics("xterm-kitty", "", true, None),
            Some(Graphics::Kitty)
        );
        assert_eq!(
            select_graphics("xterm-256color", "ghostty", false, None),
            None
        );
    }
    #[test]
    fn deletion_targets_owned_id_only() {
        let mut out = Vec::new();
        cleanup_visible(
            &mut out,
            Visible {
                graphics: Graphics::Kitty,
                id: 123,
                area: Rect::new(1, 2, 3, 4),
            },
        )
        .unwrap();
        assert_eq!(out, b"\x1b_Ga=d,d=I,i=123,q=2\x1b\\");
        let mut out = Vec::new();
        cleanup_visible(
            &mut out,
            Visible {
                graphics: Graphics::Sixel,
                id: 0,
                area: Rect::new(2, 3, 4, 2),
            },
        )
        .unwrap();
        assert_eq!(out, b"\x1b7\x1b[4;3H\x1b[4X\x1b[5;3H\x1b[4X\x1b8");
    }
    #[test]
    fn real_sixel_and_kitty_output_from_pixels() {
        for graphics in [Graphics::Sixel, Graphics::Kitty] {
            let (protocol, id) = encode(
                Capability {
                    graphics,
                    cell: (8, 16),
                },
                16,
                16,
                vec![255; 16 * 16 * 4],
                Rect::new(0, 0, 10, 10),
            )
            .unwrap();
            assert!(id > 0);
            assert_eq!(protocol.area(), Rect::new(0, 0, 2, 1));
        }
    }
    #[test]
    fn rendered_bitmap_is_removed_on_hide() {
        use ratatui::{backend::TestBackend, Terminal};
        let mut preview = NativePreview::new(Some(Capability {
            graphics: Graphics::Kitty,
            cell: (8, 16),
        }));
        let area = Rect::new(1, 1, 4, 4);
        let rgba = vec![255; 16 * 16 * 4];
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            preview.prepare("source", 16, 16, &rgba, area);
            if preview.ready.is_some() {
                break;
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let id = preview.ready.as_ref().unwrap().id;
        let mut terminal = Terminal::new(TestBackend::new(8, 8)).unwrap();
        terminal.draw(|f| assert!(preview.render(f, area))).unwrap();
        preview.hide();
        let mut bytes = Vec::new();
        assert!(preview.cleanup(&mut bytes).unwrap());
        assert_eq!(
            String::from_utf8(bytes).unwrap(),
            format!("\x1b_Ga=d,d=I,i={id},q=2\x1b\\")
        );
        assert!(!preview.cleanup(&mut Vec::new()).unwrap());
    }
    #[test]
    fn transparent_sixel_falls_back_without_black_panels() {
        assert!(encode(
            Capability {
                graphics: Graphics::Sixel,
                cell: (8, 16)
            },
            1,
            1,
            vec![255, 0, 0, 0],
            Rect::new(0, 0, 1, 1)
        )
        .is_err());
    }
    #[test]
    fn malformed_bounds_rejected_without_worker() {
        let mut preview = NativePreview::new(Some(Capability {
            graphics: Graphics::Sixel,
            cell: (8, 16),
        }));
        preview.prepare("bad", 1281, 1, &[], Rect::new(0, 0, 1, 1));
        assert!(!preview.pending());
        preview.prepare("bad", 1, 1, &[1, 2, 3], Rect::new(0, 0, 1, 1));
        assert!(!preview.pending());
    }
    #[test]
    fn old_cell_geometry_completion_is_discarded() {
        let mut preview = NativePreview::new(Some(Capability {
            graphics: Graphics::Kitty,
            cell: (10, 20),
        }));
        // Disable observations so this simulates a delayed completion after a font change.
        preview.graphics_hint = None;
        let area = Rect::new(0, 0, 4, 4);
        let old = Target {
            key: "same".into(),
            area,
            cell: (8, 16),
        };
        let (tx, rx) = mpsc::sync_channel(1);
        preview.requested = Some(old.clone());
        preview.worker = Some(rx);
        tx.send(Finished {
            target: old,
            result: encode(
                Capability {
                    graphics: Graphics::Kitty,
                    cell: (8, 16),
                },
                16,
                16,
                vec![255; 16 * 16 * 4],
                area,
            ),
        })
        .unwrap();
        preview.prepare("same", 16, 16, &vec![255; 16 * 16 * 4], area);
        assert!(preview.ready.is_none());
        assert_eq!(preview.requested.as_ref().unwrap().cell, (10, 20));
    }
    #[test]
    fn stale_worker_completion_never_renders_new_selection() {
        let mut preview = NativePreview::new(Some(Capability {
            graphics: Graphics::Sixel,
            cell: (8, 16),
        }));
        let area = Rect::new(0, 0, 4, 4);
        preview.prepare("first", 16, 16, &vec![255; 16 * 16 * 4], area);
        preview.hide();
        std::thread::sleep(std::time::Duration::from_millis(30));
        preview.prepare("second", 16, 16, &vec![128; 16 * 16 * 4], area);
        assert!(preview.ready.is_none());
        assert_eq!(preview.requested.as_ref().unwrap().key, "second");
    }
}
