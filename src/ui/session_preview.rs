//! Selected-only live tmux screen, with explicitly focused and identity-bound input.
use super::*;
#[derive(Default)]
pub(super) struct Snapshot {
    key: Option<String>,
    screen: std::cell::Cell<Rect>,
    scroll_top: Option<u32>,
    history_loaded: bool,
    history_total: u32,
    line_start: u32,
    cols: u16,
    rows: u16,
    application_mouse: bool,
    mouse_sgr: bool,
    serial: u64,
    inflight: bool,
    last: Option<Instant>,
    lines: Vec<String>,
    status: String,
    pane: String,
    live: bool,
    cursor: (u16, u16),
    cursor_visible: bool,
    palette8: Option<Color>,
    bracketed_paste: bool,
    application_cursor: bool,
    pub(super) focused: Option<(Device, Session, String)>,
    /// Bumped on every applied capture; keys the parsed-screen cache.
    revision: u64,
    /// The previous session's frame, kept dimmed until the new one arrives.
    carried: bool,
    /// Selection-driven fetches wait for the selection to settle.
    selected_at: Option<Instant>,
    /// Last keystroke or paste sent to the pane; echo polling follows it.
    last_input: Option<Instant>,
    echo_due: bool,
    received: Option<Instant>,
    stale_shown: std::cell::Cell<bool>,
}
/// Moving the selection settles for this long before a preview is requested.
pub(super) const SELECT_DEBOUNCE: Duration = Duration::from_millis(60);
/// True for every encoding of Ctrl+\: the legacy 0x1c byte (which crossterm
/// reports as Ctrl+4) and the kitty keyboard protocol's Ctrl+backslash.
pub(super) fn detach_key(event: &KeyEvent) -> bool {
    event.modifiers.contains(KeyModifiers::CONTROL)
        && matches!(event.code, KeyCode::Char('\\') | KeyCode::Char('4'))
}
fn key(device: &Device, session: &Session) -> String {
    serde_json::json!([
        device.id,
        device.target,
        session.boot_id,
        session.socket,
        session.terminal,
        session.container
    ])
    .to_string()
}
pub(super) fn label(s: &Session) -> String {
    let folder = s
        .directory
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .filter(|folder| !folder.is_empty())
        .unwrap_or(if s.directory.starts_with('/') {
            "/"
        } else {
            "~"
        });
    let name = s.name.trim();
    // Automatic names (`shell · cx-demo`, from the helper) are not user names.
    let automatic = name
        .strip_prefix(s.provider.as_str())
        .and_then(|rest| rest.strip_prefix(" · "))
        .is_some_and(|rest| rest == folder || rest == "root");
    let generic = name.is_empty()
        || automatic
        || name == s.id
        || name == s.provider
        || name == "shell"
        || name == "New shell";
    if !generic {
        return safe_label(name);
    }
    // Spec 7.1: provider title, else user name, else "<folder> shell".
    // Never a tmux id; the list disambiguates equal fallbacks itself.
    format!("{} {}", safe_label(folder), safe_label(&s.provider))
}
impl App {
    fn live_top(&self) -> u32 {
        live_view_top(
            self.session_preview.history_total,
            self.session_preview.cursor.1,
            self.session_preview.screen.get().height,
        )
    }
    fn preview_page_contains(&self, top: u32) -> bool {
        let end = self
            .session_preview
            .line_start
            .saturating_add(self.session_preview.lines.len() as u32);
        top >= self.session_preview.line_start
            && top
                .saturating_add(u32::from(self.session_preview.screen.get().height))
                .min(
                    self.session_preview
                        .history_total
                        .saturating_add(u32::from(self.session_preview.rows)),
                )
                <= end
    }
    /// A loaded scrollback page covers the view: no request until the user
    /// scrolls past it or returns to live.
    fn preview_fetch_parked(&self) -> bool {
        self.session_preview.history_loaded
            && self
                .session_preview
                .scroll_top
                .is_some_and(|top| self.preview_page_contains(top))
    }
    fn resume_session_preview(&mut self) {
        self.session_preview.scroll_top = None;
        self.session_preview.history_loaded = false;
        self.session_preview.last = None;
    }
    pub(super) fn scroll_session_preview(&mut self, mouse: MouseEvent) -> bool {
        if !matches!(self.view, View::Work | View::Network)
            || self.dialog.is_some()
            || self.help
            || self.command_panel.is_some()
            || !self
                .session_preview
                .screen
                .get()
                .contains((mouse.column, mouse.row).into())
        {
            return false;
        }
        let up = match mouse.kind {
            MouseEventKind::ScrollUp => true,
            MouseEventKind::ScrollDown => false,
            _ => return false,
        };
        // Full-screen TUIs may keep history inside the application rather than tmux.
        // Forward only wheel reports, only when that application explicitly enabled mouse tracking.
        if self.session_preview.application_mouse && self.session_preview.live {
            if let Some((d, session)) = self.preview_selection() {
                let area = self.session_preview.screen.get();
                let x = mouse
                    .column
                    .saturating_sub(area.x)
                    .saturating_add(
                        self.session_preview
                            .cursor
                            .0
                            .saturating_sub(area.width.saturating_sub(1)),
                    )
                    .saturating_add(1)
                    .min(self.session_preview.cols);
                let y = u32::from(mouse.row.saturating_sub(area.y))
                    .saturating_add(self.live_top())
                    .saturating_add(1)
                    .min(u32::from(self.session_preview.rows)) as u16;
                let button = if up { 64 } else { 65 };
                let bytes = if self.session_preview.mouse_sgr {
                    format!("\x1b[<{button};{x};{y}M").into_bytes()
                } else if x <= 223 && y <= 223 {
                    vec![27, b'[', b'M', button + 32, x as u8 + 32, y as u8 + 32]
                } else {
                    return true;
                };
                let pane_id = self.session_preview.pane.clone();
                let hex = bytes.iter().map(|b| format!("{b:02x}")).collect();
                if !self.send(
                    d,
                    Operation::SessionPaneInput {
                        session,
                        pane_id,
                        hex,
                    },
                ) {
                    self.set_notice("Terminal scroll was not queued".into());
                }
                self.session_preview.last = None;
            }
            return true;
        }
        let live = self.live_top();
        let current = self.session_preview.scroll_top.unwrap_or(live);
        if up {
            self.session_preview.scroll_top = Some(current.saturating_sub(3));
            self.session_preview.last = None;
        } else if current.saturating_add(3) >= live {
            self.resume_session_preview();
        } else {
            self.session_preview.scroll_top = Some(current + 3);
        }
        true
    }
    pub(super) fn pane_focused(&self) -> bool {
        self.session_preview.focused.is_some()
    }
    fn preview_selection(&self) -> Option<(usize, Session)> {
        if let Some((device, session, _)) = &self.session_preview.focused {
            let d = self
                .devices
                .iter()
                .position(|d| d.id == device.id && d.target == device.target)?;
            if self.work[d].error.is_some()
                || !self.work[d]
                    .sessions
                    .iter()
                    .any(|s| key(device, s) == key(device, session))
            {
                return None;
            }
            return Some((d, session.clone()));
        }
        if !matches!(self.view, View::Work | View::Network)
            || self.dialog.is_some()
            || self.help
            || self.command_panel.is_some()
        {
            return None;
        }
        self.selected_session().filter(|(d, s)| {
            !outside_tmux(s)
                && s.terminal.is_some()
                && self.providers.get(d).is_none_or(|(caps, _)| {
                    caps.iter()
                        .any(|c| c == "session-preview-v1" || c == "live-session-pane-v1")
                })
                && self.work[*d].error.is_none()
                && self.session_status(*d, s).starts_with('+')
        })
    }
    pub(super) fn focus_session_pane(&mut self) -> bool {
        let Some((d, s)) = self.preview_selection() else {
            return false;
        };
        if !self.session_preview.live
            || self.session_preview.pane.is_empty()
            || self.session_preview.key.as_ref() != Some(&key(&self.devices[d], &s))
        {
            return false;
        }
        self.resume_session_preview();
        self.session_preview.focused = Some((
            self.devices[d].clone(),
            s,
            self.session_preview.pane.clone(),
        ));
        true
    }
    /// Palette "Send Ctrl+\ to session": Ctrl+\ always detaches, so this is
    /// how the literal 0x1c (SIGQUIT, nvim Ctrl+\ Ctrl+N) reaches the pane.
    /// It leaves the pane focused so a follow-up key can be typed.
    pub(super) fn send_ctrl_backslash(&mut self) {
        if !self.pane_focused() && !self.focus_session_pane() {
            return self.type_into_preview();
        }
        let Some((d, _)) = self.preview_selection() else {
            return;
        };
        let (_, session, pane_id) = self.session_preview.focused.clone().unwrap();
        if self.send(
            d,
            Operation::SessionPaneInput {
                session,
                pane_id,
                hex: "1c".into(),
            },
        ) {
            self.input_sent();
        } else {
            self.session_preview.focused = None;
            self.set_notice("Terminal input was not queued; focus released".into());
        }
    }
    pub(super) fn session_pane_key(&mut self, event: KeyEvent) {
        // Ctrl+\ leaves inline input; every other key (Ctrl+] and Ctrl+O too)
        // belongs to the application in the pane.
        if detach_key(&event) {
            self.leave_session_pane();
            return;
        }
        let Some((d, _)) = self.preview_selection() else {
            self.session_preview = Snapshot::default();
            return;
        };
        let (_, session, pane_id) = self.session_preview.focused.clone().unwrap();
        self.resume_session_preview();
        if let Some(mut bytes) = encode_key(event) {
            if self.session_preview.application_cursor
                && event.modifiers.is_empty()
                && matches!(
                    event.code,
                    KeyCode::Up
                        | KeyCode::Down
                        | KeyCode::Left
                        | KeyCode::Right
                        | KeyCode::Home
                        | KeyCode::End
                )
                && bytes.len() == 3
            {
                bytes[1] = b'O';
            }
            let hex = bytes.iter().map(|b| format!("{b:02x}")).collect();
            if self.send(
                d,
                Operation::SessionPaneInput {
                    session,
                    pane_id,
                    hex,
                },
            ) {
                self.input_sent();
            } else {
                self.session_preview.focused = None;
                self.set_notice("Terminal input was not queued; focus released".into());
            }
        }
    }
    /// Poll for the echo right away; an in-flight capture may predate the key.
    fn input_sent(&mut self) {
        self.session_preview.last_input = Some(Instant::now());
        self.session_preview.last = None;
        if self.session_preview.inflight {
            self.session_preview.echo_due = true;
        }
    }
    pub(super) fn leave_session_pane(&mut self) {
        self.session_preview.focused = None;
        self.session_preview.last_input = None;
    }
    /// `i`: type into the inline preview. Explains why when it is not ready.
    pub(super) fn type_into_preview(&mut self) {
        if self.focus_session_pane() {
            return;
        }
        let message = match self.selected_session() {
            None => return,
            Some((_, s)) if outside_tmux(&s) => {
                "This terminal is outside tmux · Enter moves it into CX"
            }
            Some(_) if self.preview_selection().is_none() => {
                "No live preview for this session · Enter attaches"
            }
            Some(_) => "Live preview is still loading · try again in a moment",
        };
        self.set_notice(message.into());
    }
    /// The live capture of the selected session (its tile and peek).
    pub(super) fn session_preview_lines(&self) -> &[String] {
        &self.session_preview.lines
    }
    pub(super) fn session_preview_cols(&self) -> u16 {
        self.session_preview.cols
    }
    #[cfg(test)]
    pub(super) fn preview_screen(&self) -> Rect {
        self.session_preview.screen.get()
    }
    /// Where the peek drew the session's screen (wheel and taps act on it there).
    pub(super) fn set_preview_screen(&self, r: Rect) {
        self.session_preview.screen.set(r);
    }
    /// Clicks on the preview: the border chip detaches or zooms; the screen
    /// starts inline input. Returns true when the click was the preview's.
    pub(super) fn click_session_preview(&mut self, mouse: MouseEvent) -> bool {
        if !matches!(self.view, View::Work | View::Network)
            || self.dialog.is_some()
            || self.help
            || self.command_panel.is_some()
            || self.input.is_some()
            || mouse.kind != MouseEventKind::Down(event::MouseButton::Left)
        {
            return false;
        }
        let at = ratatui::layout::Position::new(mouse.column, mouse.row);
        if !self.pane_focused() && self.session_preview.screen.get().contains(at) {
            self.type_into_preview();
            return true;
        }
        false
    }
    /// How soon the event loop must wake for preview work, if sooner than idle.
    pub(super) fn session_preview_wait(&self) -> Option<Duration> {
        if !matches!(self.view, View::Work | View::Network) && !self.pane_focused() {
            return None;
        }
        let p = &self.session_preview;
        if let Some(at) = p.selected_at {
            return Some(SELECT_DEBOUNCE.saturating_sub(at.elapsed()));
        }
        if !self.pane_focused() {
            return None;
        }
        // Nothing would be requested: wake on input, never spin.
        if self.preview_fetch_parked() || self.preview_selection().is_none() {
            return None;
        }
        let typing = p
            .last_input
            .is_some_and(|t| t.elapsed() < Duration::from_secs(1));
        if p.inflight {
            // Replies arrive on a channel the input poll cannot see.
            return Some(Duration::from_millis(if typing { 4 } else { 20 }));
        }
        let due = p
            .last
            .map(|t| self.echo_interval().saturating_sub(t.elapsed()))
            .unwrap_or_default();
        Some(due)
    }
    /// Keystroke echo is fast right after input and backs off when idle.
    fn echo_interval(&self) -> Duration {
        if !self.pane_focused() {
            return Duration::from_millis(250);
        }
        match self.session_preview.last_input.map(|t| t.elapsed()) {
            Some(e) if e < Duration::from_secs(1) => Duration::from_millis(16),
            Some(e) if e < Duration::from_secs(5) => Duration::from_millis(50),
            _ => Duration::from_millis(120),
        }
    }
    pub(super) fn session_pane_paste(&mut self, text: &str) {
        self.resume_session_preview();
        let text = if self.session_preview.bracketed_paste {
            format!("\x1b[200~{text}\x1b[201~")
        } else {
            text.to_owned()
        };
        if text.len() > 4096 {
            self.set_notice("Paste exceeds 4096 bytes; nothing sent".into());
            return;
        }
        let Some((d, _)) = self.preview_selection() else {
            self.session_preview = Snapshot::default();
            return;
        };
        let Some((_, session, pane_id)) = self.session_preview.focused.clone() else {
            return;
        };
        let hex = text.as_bytes().iter().map(|b| format!("{b:02x}")).collect();
        if self.send(
            d,
            Operation::SessionPaneInput {
                session,
                pane_id,
                hex,
            },
        ) {
            self.input_sent();
        } else {
            self.session_preview.focused = None;
            self.set_notice("Paste was not queued; focus released".into());
        }
    }
    pub(super) fn refresh_session_preview(&mut self, _width: u16) -> bool {
        let selection = self.preview_selection();
        let next = selection.as_ref().map(|(d, s)| key(&self.devices[*d], s));
        let changed = next != self.session_preview.key;
        if changed {
            let previous = std::mem::take(&mut self.session_preview);
            let moved = previous.key.is_some() && next.is_some();
            self.session_preview = Snapshot {
                serial: previous.serial.wrapping_add(1),
                ..Snapshot::default()
            };
            if moved {
                // Keep the old frame, dimmed, so moving never blanks the
                // preview; fetch only once the selection settles.
                let p = &mut self.session_preview;
                p.carried = !previous.lines.is_empty();
                p.lines = previous.lines;
                p.cols = previous.cols;
                p.rows = previous.rows;
                p.palette8 = previous.palette8;
                p.cursor = previous.cursor;
                p.history_total = previous.history_total;
                p.line_start = previous.line_start;
                p.revision = previous.revision.wrapping_add(1);
                p.selected_at = Some(Instant::now());
            }
            self.session_preview.key = next;
        }
        let Some((device, session)) = selection else {
            return changed;
        };
        if let Some(at) = self.session_preview.selected_at {
            if at.elapsed() < SELECT_DEBOUNCE {
                return changed;
            }
            self.session_preview.selected_at = None;
        }
        if self.preview_fetch_parked() {
            return changed;
        }
        let interval = self.echo_interval();
        if self.session_preview.inflight
            || self
                .session_preview
                .last
                .is_some_and(|t| t.elapsed() < interval)
        {
            return changed;
        }
        self.session_preview.serial = self.session_preview.serial.wrapping_add(1);
        let sent = self
            .tx
            .try_send(Task {
                device,
                execution: self.devices[device].clone(),
                op: Operation::SessionPreview {
                    session,
                    scrollback: self.session_preview.scroll_top.is_some(),
                    history_start: self
                        .session_preview
                        .scroll_top
                        .map(|top| top.saturating_sub(150)),
                },
                generation: self.session_preview.serial,
            })
            .is_ok();
        self.session_preview.last = Some(Instant::now());
        if sent {
            self.pending_requests.set(self.pending_requests.get() + 1);
            self.session_preview.inflight = true;
        }
        // A request alone changes nothing on screen; its reply decides.
        changed
    }
    pub(super) fn apply_session_preview(&mut self, reply: Reply) {
        let Operation::SessionPreview {
            session,
            scrollback,
            ..
        } = &reply.op
        else {
            return;
        };
        let current = self
            .preview_selection()
            .map(|(d, s)| key(&self.devices[d], &s));
        if reply.generation != self.session_preview.serial
            || self.session_preview.key != current
            || current.as_ref() != Some(&key(&self.devices[reply.device], session))
        {
            return;
        }
        let before = fingerprint(&self.session_preview);
        self.session_preview.inflight = false;
        if std::mem::take(&mut self.session_preview.echo_due) {
            self.session_preview.last = None;
        }
        if *scrollback && self.session_preview.scroll_top.is_none() {
            self.session_preview.last = None;
            return; // A late history page must not replace the resumed live screen.
        }
        match reply.result {
            Ok(value) => {
                let pane = value["pane_id"].as_str().unwrap_or("").to_owned();
                if self
                    .session_preview
                    .focused
                    .as_ref()
                    .is_some_and(|(_, _, p)| p != &pane)
                {
                    self.session_preview.focused = None;
                }
                self.session_preview.pane = pane;
                self.session_preview.palette8 = value["palette8"].as_str().and_then(|s| {
                    let hex = s.strip_prefix('#')?;
                    if hex.len() != 6 {
                        return None;
                    }
                    let rgb = u32::from_str_radix(hex, 16).ok()?;
                    Some(Color::Rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8))
                });
                self.session_preview.live = value["live"].as_bool() == Some(true);
                if !self.session_preview.live {
                    self.session_preview.focused = None;
                }
                self.session_preview.cursor = (
                    value["cursor_x"].as_u64().unwrap_or(0).min(65535) as u16,
                    value["cursor_y"].as_u64().unwrap_or(0).min(65535) as u16,
                );
                self.session_preview.bracketed_paste =
                    value["bracketed_paste"].as_bool().unwrap_or(false);
                self.session_preview.application_cursor =
                    value["application_cursor"].as_bool().unwrap_or(false);
                self.session_preview.cursor_visible =
                    value["cursor_visible"].as_bool().unwrap_or(false);
                let history = value["history_rows"].as_u64().unwrap_or(0).min(500) as u32;
                if value.get("history_total").is_none() {
                    // Older helpers expose only a recent-history window.
                    if let Some(top) = &mut self.session_preview.scroll_top {
                        *top = top
                            .saturating_sub(self.session_preview.history_total)
                            .saturating_add(history);
                    }
                }
                self.session_preview.history_total = value["history_total"]
                    .as_u64()
                    .unwrap_or(u64::from(history))
                    .min(u64::from(u32::MAX))
                    as u32;
                self.session_preview.line_start = value["line_start"]
                    .as_u64()
                    .unwrap_or(0)
                    .min(u64::from(u32::MAX))
                    as u32;
                self.session_preview.cols = value["cols"].as_u64().unwrap_or(0).min(1000) as u16;
                self.session_preview.rows = value["rows"].as_u64().unwrap_or(0).min(300) as u16;
                self.session_preview.application_mouse = value["alternate"].as_bool() == Some(true)
                    && value["mouse_tracking"].as_bool() == Some(true);
                self.session_preview.mouse_sgr = value["mouse_sgr"].as_bool() == Some(true);

                self.session_preview.history_loaded = *scrollback;
                self.session_preview.lines.clear();
                let lines = value.get("styled_lines").unwrap_or(&value["lines"]);
                let mut budget = 256 * 1024;
                for line in lines
                    .as_array()
                    .into_iter()
                    .flatten()
                    .take(800)
                    .filter_map(|v| v.as_str())
                {
                    if line.len() > budget {
                        break;
                    }
                    budget -= line.len();
                    self.session_preview.lines.push(line.to_owned());
                }
                self.session_preview.status.clear();
                self.session_preview.received = Some(Instant::now());
                if !*scrollback {
                    let (lines, cols) = (
                        self.session_preview.lines.clone(),
                        self.session_preview.cols,
                    );
                    self.remember_frame(reply.device, session, &lines, cols);
                }
            }
            Err(_) => {
                self.session_preview.focused = None;
                self.session_preview.live = false;
                self.session_preview.lines.clear();
                self.session_preview.status = "Terminal unavailable · refresh to check".into();
            }
        }
        let was_carried = std::mem::take(&mut self.session_preview.carried);
        if was_carried || fingerprint(&self.session_preview) != before {
            // Identical captures neither reparse nor redraw.
            self.session_preview.revision = self.session_preview.revision.wrapping_add(1);
        }
    }
    /// The runtime redraws for a preview reply only when this changed.
    pub(super) fn session_preview_revision(&self) -> u64 {
        self.session_preview.revision
    }
    /// True once each time the capture age crosses the live/stale threshold,
    /// whichever label render then draws, so a parked preview never keeps
    /// the loop redrawing.
    pub(super) fn session_preview_staleness_changed(&self) -> bool {
        let p = &self.session_preview;
        let stale = p.received.is_some_and(|t| t.elapsed() >= PREVIEW_STALE);
        p.stale_shown.replace(stale) != stale
    }
}
/// Liveness turns into `stale <age>` once captures stop arriving for this long.
const PREVIEW_STALE: Duration = Duration::from_secs(3);
fn fingerprint(p: &Snapshot) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (
        &p.lines,
        &p.status,
        &p.pane,
        p.live,
        p.cursor,
        p.cursor_visible,
        p.cols,
        p.rows,
        p.history_total,
        p.line_start,
        p.history_loaded,
        format!("{:?}", p.palette8),
    )
        .hash(&mut h);
    p.scroll_top.hash(&mut h);
    p.focused.is_some().hash(&mut h);
    h.finish()
}
fn live_view_top(history: u32, cursor_y: u16, height: u16) -> u32 {
    history.saturating_add(u32::from(cursor_y.saturating_sub(height.saturating_sub(1))))
}
#[cfg(test)]
mod scroll_tests {
    use super::*;
    #[test]
    fn cursor_at_home_does_not_skip_recent_history() {
        assert_eq!(live_view_top(1200, 0, 20), 1200);
        assert_eq!(live_view_top(1200, 39, 20), 1220);
        assert_eq!(live_view_top(100_000, 0, 20), 100_000);
    }
    #[test]
    fn a_parked_preview_reports_the_stale_switch_once() {
        let (tx, _rx) = std::sync::mpsc::sync_channel(1);
        let mut a = App::new(vec![], tx);
        // Scrollback (or an error) draws no live/stale label; the old capture
        // ages past the threshold and must cost one redraw, not one per loop.
        a.session_preview.live = true;
        a.session_preview.scroll_top = Some(10);
        a.session_preview.received = Some(Instant::now() - Duration::from_secs(5));
        let changes = (0..5)
            .filter(|_| a.session_preview_staleness_changed())
            .count();
        assert_eq!(changes, 1);
        a.session_preview.received = Some(Instant::now());
        assert!(a.session_preview_staleness_changed());
        assert!(!a.session_preview_staleness_changed());
    }
}
