//! Optional bitmap rendering. This module never reads terminal input or opens a source file.
//! Call prepare/hide then cleanup BEFORE Terminal::draw; clear its cache if cleanup returns true.
use image::{DynamicImage, RgbaImage};
use ratatui::{layout::Rect, Frame};
use ratatui_image::{
    protocol::{iterm2::Iterm2, kitty::Kitty, sixel::Sixel, Protocol},
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
    /// Kitty graphics with unicode placeholders (they also work inside tmux).
    Kitty,
    /// iTerm2 inline images (iTerm2, WezTerm).
    Iterm2,
}
#[derive(Clone, Copy, Debug)]
struct Capability {
    graphics: Graphics,
    cell: (u16, u16),
    /// Inside tmux: sequences are wrapped for passthrough.
    tmux: bool,
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
    /// Kitty: the image's own transmit (PNG), written with the first frame that shows it.
    transmit: Option<String>,
}
/// An encoded preview: the protocol, its image id and (kitty) the transmit to send.
type Encoded = (Protocol, u32, Option<String>);
struct Finished {
    target: Target,
    result: Result<Encoded, String>,
}
/// Kitty images the terminal holds (re-hovering one sends no pixels again).
const KITTY_KEPT: usize = 16;
#[derive(Clone, Copy)]
struct Visible {
    graphics: Graphics,
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
    /// Kitty images transmitted and still held by the terminal, oldest first:
    /// (target, image id, placeholder area).
    sent: std::collections::VecDeque<(Target, u32, Rect)>,
    /// Kitty image ids to free in the terminal (evicted from `sent`).
    evict: Vec<u32>,
}
impl NativePreview {
    pub fn detect() -> Self {
        let graphics_hint = detect_graphics();
        let mut native = Self::new(graphics_hint.and_then(|graphics| {
            cell_size().map(|cell| Capability {
                graphics,
                cell,
                tmux: wraps(graphics),
            })
        }));
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
            sent: std::collections::VecDeque::new(),
            evict: Vec::new(),
        }
    }
    pub fn pending(&self) -> bool {
        self.worker.is_some() && self.requested.is_some()
    }
    /// Stable key must change when source pixels change. No filenames enter graphics commands.
    /// Returns immediately; caller retains its half-block fallback while encoding completes.
    pub fn prepare(&mut self, key: &str, width: u32, height: u32, rgba: &[u8], area: Rect) {
        // Foot initially reports 0x0 pixel geometry until compositor configure.
        // Nonblocking ioctl also observes font-size changes without consuming input.
        let observed = self.graphics_hint.and_then(|graphics| {
            cell_size().map(|cell| Capability {
                graphics,
                cell,
                tmux: wraps(graphics),
            })
        });
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
                            Ok((protocol, id, transmit)) => {
                                self.ready = Some(Ready {
                                    target: done.target,
                                    protocol,
                                    id,
                                    transmit,
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
        // A kitty image the terminal still holds: placeholders only, no pixels again.
        if let Some(&(_, id, cells)) = self.sent.iter().find(|(t, _, _)| *t == target) {
            if let Ok(protocol) = kitty_placeholders(id, cells, capability.tmux) {
                self.ready = Some(Ready {
                    target,
                    protocol,
                    id,
                    transmit: None,
                });
                return;
            }
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
        let Some(ready) = &mut self.ready else {
            return false;
        };
        if ready.target.area != area || self.requested.as_ref() != Some(&ready.target) {
            return false;
        }
        frame.render_widget(Image::new(&ready.protocol), area);
        if let Some(seq) = ready.transmit.take() {
            // The pixels go out just before the first placeholders that show them.
            if let Some(cell) = frame.buffer_mut().cell_mut((area.x, area.y)) {
                let symbol = format!("{seq}{}", cell.symbol());
                cell.set_symbol(&symbol);
            }
            self.sent.retain(|(t, _, _)| *t != ready.target);
            self.sent
                .push_back((ready.target.clone(), ready.id, ready.protocol.area()));
            while self.sent.len() > KITTY_KEPT {
                if let Some((_, id, _)) = self.sent.pop_front() {
                    self.evict.push(id);
                }
            }
        }
        if let Some(capability) = self.capability {
            self.visible = Some(Visible {
                graphics: capability.graphics,
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
        for id in self.evict.drain(..) {
            kitty_delete(out, id)?;
        }
        if self.cleanup_queue.is_empty() {
            out.flush()?;
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
        self.evict.extend(self.sent.drain(..).map(|(_, id, _)| id));
        let _ = self.cleanup(&mut io::stdout());
    }
}
fn encode(
    capability: Capability,
    width: u32,
    height: u32,
    rgba: Vec<u8>,
    area: Rect,
) -> Result<Encoded, String> {
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
    let tmux = capability.tmux;
    let protocol = match capability.graphics {
        Graphics::Kitty => {
            let transmit = kitty_transmit(&image, id, tmux)?;
            return Ok((kitty_placeholders(id, cell_area, tmux)?, id, Some(transmit)));
        }
        Graphics::Iterm2 => {
            Protocol::ITerm2(Iterm2::new(image, cell_area, tmux).map_err(|e| e.to_string())?)
        }
        // Sixel has no alpha channel. Avoid rendering a transparent source on an invented opaque
        // background; the existing half-block renderer preserves terminal defaults correctly.
        Graphics::Sixel => {
            if image.to_rgba8().pixels().any(|p| p[3] != 255) {
                return Err("transparent raster uses half-block fallback".into());
            }
            Protocol::Sixel(Sixel::new(image, cell_area, tmux).map_err(|e| e.to_string())?)
        }
    };
    Ok((protocol, id, None))
}
/// ratatui-image's kitty placeholders for image `id` over `cells`, with its own
/// transmit (raw RGBA of a 1x1 stand-in) used up, so a render draws placeholders only:
/// the real pixels go out once, compressed, from `kitty_transmit`.
fn kitty_placeholders(id: u32, cells: Rect, tmux: bool) -> Result<Protocol, String> {
    let stand_in = DynamicImage::ImageRgba8(RgbaImage::new(1, 1));
    let protocol =
        Protocol::Kitty(Kitty::new(stand_in, cells, id, tmux).map_err(|e| e.to_string())?);
    let mut scratch = ratatui::buffer::Buffer::empty(cells);
    ratatui::widgets::Widget::render(Image::new(&protocol), cells, &mut scratch);
    Ok(protocol)
}
/// The kitty transmit of `image` as PNG (`f=100`; RGB when opaque) with a virtual
/// placement for unicode placeholders, in 4096-byte chunks, tmux-wrapped when asked.
/// About a third of raw RGBA for photos, a few percent for flat images.
fn kitty_transmit(image: &DynamicImage, id: u32, tmux: bool) -> Result<String, String> {
    use base64::Engine;
    use image::{
        codecs::png::{CompressionType, FilterType, PngEncoder},
        ExtendedColorType, ImageEncoder,
    };
    let (w, h) = (image.width(), image.height());
    let rgba = image.to_rgba8();
    let mut png = Vec::new();
    let encoder =
        PngEncoder::new_with_quality(&mut png, CompressionType::Fast, FilterType::Adaptive);
    if rgba.pixels().all(|p| p[3] == 255) {
        let rgb = image.to_rgb8();
        encoder.write_image(rgb.as_raw(), w, h, ExtendedColorType::Rgb8)
    } else {
        encoder.write_image(rgba.as_raw(), w, h, ExtendedColorType::Rgba8)
    }
    .map_err(|e| e.to_string())?;
    let data = base64::engine::general_purpose::STANDARD.encode(&png);
    let (start, esc, end) = if tmux {
        ("\x1bPtmux;", "\x1b\x1b", "\x1b\\")
    } else {
        ("", "\x1b", "")
    };
    let chunks: Vec<&[u8]> = data.as_bytes().chunks(4096).collect();
    let mut out = String::with_capacity(data.len() + chunks.len() * 32 + 64);
    out.push_str(start);
    for (i, chunk) in chunks.iter().enumerate() {
        let more = u8::from(i + 1 < chunks.len());
        let payload = std::str::from_utf8(chunk).unwrap_or_default();
        if i == 0 {
            out.push_str(&format!(
                "{esc}_Gq=2,i={id},a=T,U=1,f=100,m={more};{payload}{esc}\\"
            ));
        } else {
            out.push_str(&format!("{esc}_Gq=2,m={more};{payload}{esc}\\"));
        }
    }
    out.push_str(end);
    Ok(out)
}
/// Free a kitty image's pixels in the terminal.
fn kitty_delete<W: Write>(out: &mut W, id: u32) -> io::Result<()> {
    if in_tmux() {
        write!(
            out,
            "\x1bPtmux;\x1b\x1b_Ga=d,d=I,i={id},q=2\x1b\x1b\\\x1b\\"
        )
    } else {
        write!(out, "\x1b_Ga=d,d=I,i={id},q=2\x1b\\")
    }
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
        // Placeholders vanish with the redraw this asks for; the terminal keeps the
        // pixels for a re-hover (freed on eviction from `sent`, and on exit).
        Graphics::Kitty => Ok(()),
        Graphics::Sixel | Graphics::Iterm2 => {
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
/// The cell size in pixels when this terminal draws bitmaps (else None: half-blocks).
pub fn graphics_cell() -> Option<(u16, u16)> {
    static GRAPHICS: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if !*GRAPHICS.get_or_init(|| detect_graphics().is_some()) {
        return None;
    }
    cell_size()
}
pub fn in_tmux() -> bool {
    std::env::var_os("TMUX").is_some_and(|t| !t.is_empty())
}
/// Whether the tmux this runs in passes escape sequences through (`allow-passthrough`,
/// which cx's managed server sets; another server keeps its own choice). Read-only,
/// bounded, asked once.
pub fn tmux_passthrough() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| {
        let Ok(mut child) = std::process::Command::new("tmux")
            .args(["show-options", "-gv", "allow-passthrough"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
        else {
            return false;
        };
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(300);
        loop {
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) if std::time::Instant::now() < deadline => {
                    std::thread::sleep(std::time::Duration::from_millis(5))
                }
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return false;
                }
            }
        }
        let mut out = String::new();
        use std::io::Read;
        let _ = child
            .stdout
            .take()
            .map(|o| o.take(64).read_to_string(&mut out));
        matches!(out.trim(), "on" | "all")
    })
}
/// The terminal outside tmux, from what tmux leaves in the environment. Only when the
/// startup probe got no answer from it: these variables are the tmux server's, stale
/// once a client attaches from another terminal.
fn tmux_outer_graphics() -> Option<Graphics> {
    let var = |k: &str| std::env::var(k).unwrap_or_default();
    if var("KITTY_WINDOW_ID").parse::<u64>().is_ok_and(|n| n > 0)
        || !var("GHOSTTY_RESOURCES_DIR").is_empty()
    {
        Some(Graphics::Kitty)
    } else if !var("WEZTERM_PANE").is_empty() || var("LC_TERMINAL") == "iTerm2" {
        Some(Graphics::Iterm2)
    } else {
        None
    }
}
/// What the terminal answered to the startup probe (`caps::query_osc11`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Probe {
    /// The protocol its answers name.
    graphics: Option<Graphics>,
    /// The terminal itself answered (inside tmux: the one outside it, through
    /// passthrough), so the environment is not consulted.
    answered: bool,
    /// The cell size from `CSI 16 t`, for when the pty reports no pixels.
    cell: Option<(u16, u16)>,
}
static PROBED: std::sync::OnceLock<Probe> = std::sync::OnceLock::new();
/// Record the startup probe's reply. `tmux`: the probe also asked the terminal outside
/// tmux through passthrough (see [`probe_query`]).
pub fn note_probe(reply: &[u8], tmux: bool) {
    let _ = PROBED.set(parse_probe(reply, tmux));
}
/// The startup probe (yazi's): the kitty graphics query (a 1x1 image never stored),
/// `CSI 16 t` (cell pixels) and DA1, which ends it. Inside a tmux that allows
/// passthrough the kitty query, XTVERSION and a second DA1 go to the terminal outside
/// it, wrapped; tmux answers the plain DA1 (with `4` when it draws sixel itself) and
/// `16 t`. Returns the query and how many DA1 replies end the answer.
pub fn probe_query() -> (Vec<u8>, usize, bool) {
    let kitty: &[u8] = b"\x1b_Gi=31,s=1,v=1,a=q,t=d,f=24;AAAA\x1b\\";
    let mut q = b"\x1b[16t".to_vec();
    if in_tmux() && tmux_passthrough() {
        let wrap = |q: &mut Vec<u8>, s: &[u8]| {
            q.extend_from_slice(b"\x1bPtmux;");
            for &b in s {
                if b == 0x1b {
                    q.push(0x1b);
                }
                q.push(b);
            }
            q.extend_from_slice(b"\x1b\\");
        };
        wrap(&mut q, kitty);
        wrap(&mut q, b"\x1b[>q");
        q.extend_from_slice(b"\x1b[c");
        wrap(&mut q, b"\x1b[c");
        (q, 2, true)
    } else {
        q.extend_from_slice(kitty);
        q.extend_from_slice(b"\x1b[c");
        (q, 1, false)
    }
}
/// DA1 replies (`ESC [ ? params c`) in order, as their parameter lists.
pub fn da1_replies(reply: &[u8]) -> Vec<Vec<String>> {
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(at) = reply[i..].windows(3).position(|w| w == b"\x1b[?") {
        let from = i + at + 3;
        let len = reply[from..]
            .iter()
            .position(|b| !(b.is_ascii_digit() || *b == b';'))
            .unwrap_or(reply.len() - from);
        if reply.get(from + len) == Some(&b'c') {
            let params = String::from_utf8_lossy(&reply[from..from + len]);
            out.push(params.split(';').map(str::to_owned).collect());
        }
        i = from;
    }
    out
}
fn parse_probe(reply: &[u8], tmux: bool) -> Probe {
    let between = |open: &[u8], close: &[u8]| -> Option<String> {
        let a = reply.windows(open.len()).position(|w| w == open)? + open.len();
        let b = reply[a..].windows(close.len()).position(|w| w == close)?;
        Some(String::from_utf8_lossy(&reply[a..a + b]).into_owned())
    };
    let da1 = da1_replies(reply);
    let sixel = |r: Option<&Vec<String>>| r.is_some_and(|p| p.iter().any(|x| x == "4"));
    let kitty = reply.windows(10).any(|w| w == b"\x1b_Gi=31;OK");
    let name = between(b"\x1bP>|", b"\x1b\\").unwrap_or_default();
    let cell = between(b"\x1b[6;", b"t").and_then(|t| {
        let (h, w) = t.split_once(';')?;
        let (w, h) = (w.parse::<u16>().ok()?, h.parse::<u16>().ok()?);
        (w > 0 && h > 0 && w <= 64 && h <= 128).then_some((w, h))
    });
    let (answered, sixel) = if tmux {
        // The first DA1 is tmux's own: its `4` means tmux draws sixel itself.
        (da1.len() >= 2, sixel(da1.first()))
    } else {
        (!da1.is_empty(), da1.iter().any(|p| sixel(Some(p))))
    };
    let graphics = if kitty {
        Some(Graphics::Kitty)
    } else if name.starts_with("WezTerm") || name.starts_with("iTerm2") {
        Some(Graphics::Iterm2)
    } else if sixel {
        Some(Graphics::Sixel)
    } else {
        None
    };
    Probe {
        graphics,
        answered,
        cell,
    }
}
/// Inside tmux, kitty and iTerm2 sequences are wrapped for passthrough; sixel is drawn
/// by tmux itself (3.4+), so it is not.
fn wraps(graphics: Graphics) -> bool {
    in_tmux() && graphics != Graphics::Sixel
}
fn detect_graphics() -> Option<Graphics> {
    if std::env::var_os("NO_COLOR").is_some() || std::env::var_os("CX_ASCII").is_some() {
        return None;
    }
    let probe = PROBED.get().copied().unwrap_or_default();
    if in_tmux() {
        // The outer terminal's own answer (passthrough); else tmux's own sixel; the
        // environment (possibly stale) only when nothing answered.
        if probe.answered || probe.graphics.is_some() {
            return probe.graphics;
        }
        return tmux_outer_graphics().filter(|_| tmux_passthrough());
    }
    let term = std::env::var("TERM").unwrap_or_default();
    let program = std::env::var("TERM_PROGRAM").unwrap_or_default();
    let kitty_window = std::env::var("KITTY_WINDOW_ID")
        .ok()
        .is_some_and(|v| v.parse::<u64>().is_ok_and(|n| n > 0));
    let ancestor = terminal_ancestor();
    let known = select_graphics(&term, &program, kitty_window, ancestor.as_deref());
    choose(known, probe.graphics)
}
/// The terminal's own answer wins for kitty graphics (WezTerm with it enabled, an SSH
/// shell, Konsole); the environment names the rest; sixel from DA1 comes last.
fn choose(known: Option<Graphics>, probed: Option<Graphics>) -> Option<Graphics> {
    match (known, probed) {
        (_, Some(Graphics::Kitty)) => Some(Graphics::Kitty),
        (Some(g), _) => Some(g),
        (None, probed) => probed,
    }
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
        ("xterm-ghostty", _, Some("ghostty")) => Some(Graphics::Kitty),
        (_, "WezTerm", _) => Some(Graphics::Iterm2),
        (_, "iTerm.app", _) => Some(Graphics::Iterm2),
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
        // No pixels in the pty size (older tmux, some SSH clients, mosh): the
        // terminal's `CSI 16 t` answer from the startup probe.
        return PROBED.get().and_then(|p| p.cell);
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
    fn terminal_answers_choose_the_protocol() {
        let g = |r: &[u8]| parse_probe(r, false).graphics;
        let kitty = b"\x1b]11;rgb:0000/0000/0000\x07\x1b_Gi=31;OK\x1b\\\x1b[?62;22c";
        assert_eq!(g(kitty), Some(Graphics::Kitty));
        assert_eq!(g(b"\x1b[?62;4;22c"), Some(Graphics::Sixel));
        assert_eq!(g(b"\x1b[?62;22;44c"), None);
        assert_eq!(g(b""), None);
        assert!(!parse_probe(b"", false).answered);
        // CSI 16 t: cell height then width.
        assert_eq!(
            parse_probe(b"\x1b[6;20;10t\x1b[?62c", false).cell,
            Some((10, 20))
        );
        assert_eq!(parse_probe(b"\x1b[6;0;10t", false).cell, None);
        // WezTerm with kitty graphics enabled; foot's sixel stays; nothing known: probe.
        assert_eq!(
            choose(Some(Graphics::Iterm2), Some(Graphics::Kitty)),
            Some(Graphics::Kitty)
        );
        assert_eq!(
            choose(Some(Graphics::Iterm2), Some(Graphics::Sixel)),
            Some(Graphics::Iterm2)
        );
        assert_eq!(choose(None, Some(Graphics::Sixel)), Some(Graphics::Sixel));
        assert_eq!(choose(None, None), None);
    }
    /// What tmux 3.8 hands the pane (an isolated tmux server
    /// an isolated server attached from a pty playing the outer terminal): its own 16t
    /// and DA1, then the outer terminal's answers to the wrapped queries.
    #[test]
    fn tmux_probe_reads_the_outer_terminal() {
        let tmux =
            b"\x1b[6;20;10t\x1b[?1;2;4c\x1b_Gi=31;OK\x1b\\\x1bP>|OuterTerm 1.0\x1b\\\x1b[?62;4;22c";
        let p = parse_probe(tmux, true);
        assert_eq!(p.graphics, Some(Graphics::Kitty));
        assert!(p.answered);
        assert_eq!(p.cell, Some((10, 20)));
        // WezTerm (no kitty graphics) outside: iTerm2 images.
        let wez = b"\x1b[?1;2c\x1bP>|WezTerm 20240203\x1b\\\x1b[?65;4c";
        assert_eq!(parse_probe(wez, true).graphics, Some(Graphics::Iterm2));
        // foot outside a tmux with sixel: tmux draws it (its own DA1 lists 4).
        let foot = b"\x1b[?1;2;4c\x1bP>|foot(1.20)\x1b\\\x1b[?62;4;22c";
        assert_eq!(parse_probe(foot, true).graphics, Some(Graphics::Sixel));
        // A tmux without sixel: the outer terminal's 4 is not enough (tmux would not
        // keep the image).
        let plain = b"\x1b[?1;2c\x1bP>|foot(1.20)\x1b\\\x1b[?62;4;22c";
        assert_eq!(parse_probe(plain, true).graphics, None);
        assert!(parse_probe(plain, true).answered);
        // Only tmux answered (passthrough off): not answered, env fallback applies.
        assert!(!parse_probe(b"\x1b[?1;2c", true).answered);
        assert_eq!(da1_replies(tmux).len(), 2);
    }
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
        // Kitty: hiding sends nothing (the redraw drops the placeholders); freeing an
        // evicted image names its id only.
        let mut out = Vec::new();
        cleanup_visible(
            &mut out,
            Visible {
                graphics: Graphics::Kitty,
                area: Rect::new(1, 2, 3, 4),
            },
        )
        .unwrap();
        assert!(out.is_empty());
        if !in_tmux() {
            kitty_delete(&mut out, 123).unwrap();
            assert_eq!(out, b"\x1b_Ga=d,d=I,i=123,q=2\x1b\\");
        }
        let mut out = Vec::new();
        cleanup_visible(
            &mut out,
            Visible {
                graphics: Graphics::Sixel,
                area: Rect::new(2, 3, 4, 2),
            },
        )
        .unwrap();
        assert_eq!(out, b"\x1b7\x1b[4;3H\x1b[4X\x1b[5;3H\x1b[4X\x1b8");
    }
    #[test]
    fn real_sixel_and_kitty_output_from_pixels() {
        for graphics in [Graphics::Sixel, Graphics::Kitty] {
            let (protocol, id, transmit) = encode(
                Capability {
                    graphics,
                    cell: (8, 16),
                    tmux: false,
                },
                16,
                16,
                vec![255; 16 * 16 * 4],
                Rect::new(0, 0, 10, 10),
            )
            .unwrap();
            assert!(id > 0);
            assert_eq!(protocol.area(), Rect::new(0, 0, 2, 1));
            // Kitty: PNG (base64 of the PNG signature), not raw RGBA (f=32).
            assert_eq!(
                transmit
                    .as_deref()
                    .map(|t| t.contains(&format!("i={id},a=T,U=1,f=100,m=0;iVBORw0KGgo"))),
                (graphics == Graphics::Kitty).then_some(true)
            );
        }
        // A photo-sized raster: PNG chunks, tmux-wrapped, far under raw RGBA's base64.
        let (w, h) = (400u32, 300u32);
        let rgba: Vec<u8> = (0..w * h)
            .flat_map(|i| [(i % w) as u8, (i / w) as u8, 128, 255])
            .collect();
        let (_, _, transmit) = encode(
            Capability {
                graphics: Graphics::Kitty,
                cell: (10, 20),
                tmux: true,
            },
            w,
            h,
            rgba,
            Rect::new(0, 0, 40, 15),
        )
        .unwrap();
        let t = transmit.unwrap();
        assert!(t.starts_with("\x1bPtmux;\x1b\x1b_Gq=2,i=") && t.ends_with("\x1b\\"));
        assert!(
            t.len() < (w * h * 4) as usize * 4 / 3 / 4,
            "{} bytes",
            t.len()
        );
    }
    #[test]
    fn rendered_bitmap_is_removed_on_hide() {
        use ratatui::{backend::TestBackend, Terminal};
        let mut preview = NativePreview::new(Some(Capability {
            graphics: Graphics::Kitty,
            cell: (8, 16),
            tmux: false,
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
        let first = |t: &Terminal<TestBackend>| t.backend().buffer()[(1, 1)].symbol().to_owned();
        terminal.draw(|f| assert!(preview.render(f, area))).unwrap();
        assert!(
            first(&terminal).contains("f=100"),
            "pixels with the first frame"
        );
        terminal.draw(|f| assert!(preview.render(f, area))).unwrap();
        assert!(
            !first(&terminal).contains("_G"),
            "placeholders only after that"
        );
        preview.hide();
        // A redraw drops the placeholders; the terminal keeps the pixels.
        let mut bytes = Vec::new();
        assert!(preview.cleanup(&mut bytes).unwrap());
        assert!(bytes.is_empty());
        assert!(!preview.cleanup(&mut Vec::new()).unwrap());
        // Hovering it again sends no pixels: ready at once, the same id.
        preview.prepare("source", 16, 16, &rgba, area);
        assert_eq!(
            preview.ready.as_ref().map(|r| (r.id, r.transmit.is_none())),
            Some((id, true))
        );
        terminal.draw(|f| assert!(preview.render(f, area))).unwrap();
        assert!(!first(&terminal).contains("_G"));
        // Past KITTY_KEPT images the oldest is freed in the terminal.
        for n in 0..KITTY_KEPT {
            let key = format!("other-{n}");
            loop {
                preview.prepare(&key, 16, 16, &rgba, area);
                if preview.ready.is_some() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            terminal.draw(|f| assert!(preview.render(f, area))).unwrap();
        }
        let mut bytes = Vec::new();
        preview.cleanup(&mut bytes).unwrap();
        if !in_tmux() {
            assert_eq!(
                String::from_utf8(bytes).unwrap(),
                format!("\x1b_Ga=d,d=I,i={id},q=2\x1b\\")
            );
        }
    }
    #[test]
    fn transparent_sixel_falls_back_without_black_panels() {
        assert!(encode(
            Capability {
                graphics: Graphics::Sixel,
                cell: (8, 16),
                tmux: false,
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
            tmux: false,
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
            tmux: false,
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
                    tmux: false,
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
            tmux: false,
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
