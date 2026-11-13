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
    pane: String,
    live: bool,
    cursor: (u16, u16),
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
    /// When the selection last changed: a move after a pause fetches at once,
    /// moves in a burst wait for SELECT_DEBOUNCE.
    changed_at: Option<Instant>,
    /// Last frame of recent sessions, by key: memory only, never written out.
    frames: HashMap<String, Remembered>,
    /// Per device id: its helper watch (pushed list and selected screen).
    watches: HashMap<String, Watched>,
    /// Thumbnail captures by key: when the last was asked for (see `fetch_thumbnails`).
    thumbs: HashMap<String, Instant>,
}
/// A remembered screen, shown dimmed the moment its session is selected.
#[derive(Clone)]
struct Remembered {
    lines: Vec<String>,
    cols: u16,
    rows: u16,
    palette8: Option<Color>,
    cursor: (u16, u16),
    history_total: u32,
    line_start: u32,
}
const REMEMBERED_FRAMES: usize = 32;
/// The generation that marks a thumbnail capture: its reply fills `frames`, never the
/// selected screen, and the queue never trades it for (or against) a preview poll.
pub(super) const THUMB_GENERATION: u64 = u64::MAX;
/// How often a visible tile's screen is captured again.
const THUMB_EVERY: Duration = Duration::from_secs(2);
#[derive(Default)]
struct Watched {
    target: Option<String>,
    /// The watch last requested: the preview key of its pane, "" for list only.
    requested: Option<String>,
    /// The helper accepted it; its events stand in for polling.
    live: bool,
    /// After a refusal or a closed watch: poll, and ask again from then.
    retry_at: Option<Instant>,
    /// The helper's id of the watch whose pushed screen was shown before its reply
    /// (in-process events are not ordered behind it): that reply's older first
    /// screen is dropped. A leftover frame of a replaced watch has another id.
    pushed: Option<u64>,
    /// The id the reply named; from then on only that watch's frames apply.
    /// (Helpers before v0.2.1 send no id: 0 for both, as before.)
    watch: Option<u64>,
}
/// A watch reply's or pushed frame's watch id (0 from a helper that sends none).
fn watch_id(value: &Value) -> u64 {
    value["watch"].as_u64().unwrap_or(0)
}
/// See `live_frame_grace`.
const LIVE_GRACE: Duration = Duration::from_millis(8);
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
        self.selected_session()
            .filter(|(d, s)| self.previewable(*d, s))
    }
    /// A tmux session on a device that answers, whose helper captures screens.
    fn previewable(&self, d: usize, s: &Session) -> bool {
        !outside_tmux(s)
            && s.terminal.is_some()
            && self.providers.get(&d).is_none_or(|(caps, _)| {
                caps.iter()
                    .any(|c| c == "session-preview-v1" || c == "live-session-pane-v1")
            })
            && self.work[d].error.is_none()
            && self.session_status(d, s).starts_with('+')
    }
    /// Tiles on the hub and in a device show every session's screen, not only the
    /// selected one's: each previewable session is captured once at once (its last
    /// frame stays in memory) and again every THUMB_EVERY while tiles are on screen.
    /// The selected session's live screen is never asked twice.
    pub(super) fn fetch_thumbnails(&mut self) {
        if !matches!(self.view, View::Work | View::Network) || self.pane_focused() {
            return;
        }
        let now = Instant::now();
        let live = self.session_preview.key.clone();
        let mut due = Vec::new();
        for d in 0..self.work.len().min(self.devices.len()) {
            if !self.background_checks_enabled(d) {
                continue;
            }
            for s in self.slot_sessions(d) {
                let k = key(&self.devices[d], s);
                let asked = self.session_preview.thumbs.get(&k);
                if Some(&k) == live.as_ref()
                    || asked.is_some_and(|t| now.duration_since(*t) < THUMB_EVERY)
                    || !self.previewable(d, s)
                {
                    continue;
                }
                due.push((d, s.clone(), k));
            }
        }
        for (d, session, k) in due.into_iter().take(16) {
            let sent = self
                .tx
                .try_send(Task {
                    device: d,
                    execution: self.devices[d].clone(),
                    op: Operation::SessionPreview {
                        session,
                        scrollback: false,
                        history_start: None,
                    },
                    generation: THUMB_GENERATION,
                })
                .is_ok();
            if !sent {
                break;
            }
            self.pending_requests.set(self.pending_requests.get() + 1);
            self.session_preview.thumbs.insert(k, now);
        }
    }
    /// A thumbnail capture: its session's remembered frame (the tile draws it).
    fn apply_thumbnail(&mut self, reply: Reply) {
        let Operation::SessionPreview { session, .. } = &reply.op else {
            return;
        };
        let Ok(value) = reply.result else {
            return;
        };
        let k = key(&self.devices[reply.device], session);
        // (The selected session's own live screen is newer than any thumbnail.)
        if self.session_preview.key.as_ref() == Some(&k) {
            return;
        }
        let mut lines = Vec::new();
        let mut budget = 64 * 1024;
        for line in value
            .get("styled_lines")
            .unwrap_or(&value["lines"])
            .as_array()
            .into_iter()
            .flatten()
            .take(400)
            .filter_map(|v| v.as_str())
        {
            if line.len() > budget {
                break;
            }
            budget -= line.len();
            lines.push(line.to_owned());
        }
        let num = |name: &str| value[name].as_u64().unwrap_or(0).min(u64::from(u32::MAX)) as u32;
        let frame = Remembered {
            lines,
            cols: value["cols"].as_u64().unwrap_or(0).min(1000) as u16,
            rows: value["rows"].as_u64().unwrap_or(0).min(300) as u16,
            palette8: palette8(&value),
            cursor: (
                value["cursor_x"].as_u64().unwrap_or(0).min(65535) as u16,
                value["cursor_y"].as_u64().unwrap_or(0).min(65535) as u16,
            ),
            history_total: value
                .get("history_total")
                .map_or(num("history_rows").min(500), |_| num("history_total")),
            line_start: num("line_start"),
        };
        let frames = &mut self.session_preview.frames;
        if frames.len() >= REMEMBERED_FRAMES && !frames.contains_key(&k) {
            frames.clear();
        }
        frames.insert(k, frame);
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
    /// Palette "Send Ctrl+\": Ctrl+\ always detaches, so this is how the
    /// literal 0x1c (SIGQUIT, nvim Ctrl+\ Ctrl+N) reaches a pane. It forwards
    /// one to the last detached session (the selection after a detach) and
    /// re-attaches it, so the follow-up key is typed natively.
    pub(super) fn send_ctrl_backslash(&mut self) {
        let Some((d, s)) = self.last_detached() else {
            return;
        };
        let p = &self.session_preview;
        let pane_id = (p.live && p.key.as_ref() == Some(&key(&self.devices[d], &s)))
            .then(|| p.pane.clone())
            .filter(|pane| !pane.is_empty());
        let Some(pane_id) = pane_id else {
            // Its screen (and so its pane) loads once it is the selection again.
            self.reconcile_session_selection(Some((d, workspaces::runtime_key(&s))));
            self.set_notice("Loading the last session's screen · send again in a moment".into());
            return;
        };
        let input = Operation::SessionPaneInput {
            session: s.clone(),
            pane_id,
            hex: "1c".into(),
        };
        if !self.send(d, input) {
            self.set_notice("Terminal input was not queued".into());
            return;
        }
        self.open_session(d, s, false);
    }
    pub(super) fn session_pane_key(&mut self, event: KeyEvent) {
        // Ctrl+\ and Esc Esc leave inline input; every other key (one Esc, Ctrl+]
        // and Ctrl+O too) belongs to the application in the pane.
        if event.code == KeyCode::Esc && event.modifiers.is_empty() {
            let double = self
                .last_esc
                .is_some_and(|t| t.elapsed() < super::screens::DOUBLE_ESC);
            self.last_esc = (!double).then(Instant::now);
            if double {
                self.leave_session_pane();
                return;
            }
        }
        if detach_key(&event) {
            self.leave_session_pane();
            return;
        }
        let Some((d, _)) = self.preview_selection() else {
            self.reset_session_preview();
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
    /// The live capture of the selected session (its tile and peek); empty while a
    /// carried frame (possibly another session's) stands in for it.
    pub(super) fn session_preview_lines(&self) -> &[String] {
        if self.session_preview.carried {
            return &[];
        }
        &self.session_preview.lines
    }
    /// A session's last frame from memory (for its tile): the screen's own rows.
    pub(super) fn remembered_frame(&self, d: usize, s: &Session) -> Option<(&[String], u16)> {
        let f = self.session_preview.frames.get(&key(&self.devices[d], s))?;
        let start = (f.history_total.saturating_sub(f.line_start) as usize).min(f.lines.len());
        Some((&f.lines[start..], f.cols))
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
    /// A click on the peeked screen starts inline input; while typing, a click
    /// outside it (the bar's chip, the crumb, the hub) stops. Returns true
    /// when the click was the preview's.
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
        let on_screen = self.session_preview.screen.get().contains(at);
        if self.pane_focused() {
            if !on_screen {
                self.leave_session_pane();
            }
            return true;
        }
        if on_screen {
            self.type_into_preview();
            return true;
        }
        false
    }
    /// Right after a selection change the remembered frame waits this long
    /// for the live one, so a fast reply is drawn directly instead of a
    /// dimmed frame followed by the live one a whole frame interval later.
    pub(super) fn live_frame_grace(&self) -> Option<Duration> {
        let p = &self.session_preview;
        let waiting = p.inflight || self.pane_watched();
        let left = LIVE_GRACE.saturating_sub(p.changed_at?.elapsed());
        (p.carried && waiting && !left.is_zero()).then_some(left)
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
        // Replies and pushed screens wake the loop themselves.
        if p.inflight || self.pane_watched() {
            return None;
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
            self.reset_session_preview();
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
    pub(super) fn refresh_session_preview(&mut self) -> bool {
        let selection = self.preview_selection();
        let next = selection.as_ref().map(|(d, s)| key(&self.devices[*d], s));
        let changed = next != self.session_preview.key;
        if changed {
            let mut previous = std::mem::take(&mut self.session_preview);
            if let Some(old) = previous
                .key
                .clone()
                .filter(|_| !previous.carried && previous.live && !previous.lines.is_empty())
            {
                if previous.frames.len() >= REMEMBERED_FRAMES && !previous.frames.contains_key(&old)
                {
                    previous.frames.clear();
                }
                previous.frames.insert(old, Remembered::of(&previous));
            }
            let now = Instant::now();
            let burst = previous
                .changed_at
                .is_some_and(|t| now.saturating_duration_since(t) < SELECT_DEBOUNCE);
            let remembered = next.as_ref().and_then(|k| previous.frames.get(k).cloned());
            let moved = previous.key.is_some() && next.is_some();
            self.session_preview = Snapshot {
                serial: previous.serial.wrapping_add(1),
                changed_at: Some(now),
                ..Snapshot::default()
            };
            // This session's own last frame, else the old one: dimmed, so
            // moving never blanks the preview, until the live one arrives.
            let shown = remembered.or_else(|| moved.then(|| Remembered::of(&previous)));
            let p = &mut self.session_preview;
            p.frames = std::mem::take(&mut previous.frames);
            p.watches = std::mem::take(&mut previous.watches);
            p.thumbs = std::mem::take(&mut previous.thumbs);
            if let Some(frame) = shown.filter(|f| !f.lines.is_empty()) {
                p.carried = true;
                p.lines = frame.lines;
                p.cols = frame.cols;
                p.rows = frame.rows;
                p.palette8 = frame.palette8;
                p.cursor = frame.cursor;
                p.history_total = frame.history_total;
                p.line_start = frame.line_start;
                p.revision = previous.revision.wrapping_add(1);
            }
            // Fetch at once after a pause; a burst of moves waits to settle.
            if burst {
                p.selected_at = Some(now);
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
        if self.preview_fetch_parked() || self.pane_watched() {
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
        if reply.generation == THUMB_GENERATION {
            return self.apply_thumbnail(reply);
        }
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
                self.session_preview.palette8 = palette8(&value);
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
            }
            Err(_) => {
                self.session_preview.focused = None;
                self.session_preview.live = false;
                self.session_preview.lines.clear();
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
}
impl Remembered {
    fn of(p: &Snapshot) -> Self {
        Remembered {
            lines: p.lines.clone(),
            cols: p.cols,
            rows: p.rows,
            palette8: p.palette8,
            cursor: p.cursor,
            history_total: p.history_total,
            line_start: p.line_start,
        }
    }
}
/// After a refused watch: poll, and ask again this much later.
const WATCH_RETRY: Duration = Duration::from_secs(10);
/// After a watch ended (helper or control client gone): recover quickly.
const WATCH_RECOVER: Duration = Duration::from_secs(1);
impl App {
    /// Forget the screen, keeping remembered frames and watches.
    fn reset_session_preview(&mut self) {
        let previous = std::mem::take(&mut self.session_preview);
        self.session_preview.frames = previous.frames;
        self.session_preview.watches = previous.watches;
        self.session_preview.thumbs = previous.thumbs;
        self.session_preview.serial = previous.serial.wrapping_add(1);
    }
    /// This host runs this binary in-process; a remote helper says so in Info.
    fn watch_capable(&self, d: usize) -> bool {
        self.background_checks_enabled(d)
            && (self.devices[d].target.is_none()
                || self
                    .providers
                    .get(&d)
                    .is_some_and(|(caps, _)| caps.iter().any(|c| c == "watch-v1")))
    }
    /// The selected screen arrives as pushed events (or the watch reply is
    /// on its way): no polling. Scrollback pages are still fetched.
    fn pane_watched(&self) -> bool {
        let Some((d, s)) = self.preview_selection() else {
            return false;
        };
        let key = key(&self.devices[d], &s);
        self.session_preview.scroll_top.is_none()
            && self
                .session_preview
                .watches
                .get(&self.devices[d].id)
                .is_some_and(|w| w.requested.as_ref() == Some(&key) && w.retry_at.is_none())
    }
    /// True while `d`'s helper pushes its session list.
    pub(super) fn sessions_pushed(&self, d: usize) -> bool {
        self.session_preview
            .watches
            .get(&self.devices[d].id)
            .is_some_and(|w| w.live && w.target == self.devices[d].target)
    }
    /// Keep one watch per capable device: its list, plus the selected
    /// managed session's screen on that device once the selection settles.
    pub(super) fn sync_watches(&mut self) {
        let now = Instant::now();
        let settled = self.session_preview.selected_at.is_none();
        let selected = self
            .preview_selection()
            .filter(|(_, s)| !s.external && self.session_preview.scroll_top.is_none());
        for d in 0..self.devices.len() {
            if !self.watch_capable(d) {
                continue;
            }
            let pane = selected
                .as_ref()
                .filter(|(sd, _)| *sd == d)
                .map(|(_, s)| s.clone());
            let want = pane
                .as_ref()
                .map(|s| key(&self.devices[d], s))
                .unwrap_or_default();
            let device = &self.devices[d];
            let w = self
                .session_preview
                .watches
                .entry(device.id.clone())
                .or_default();
            if w.target != device.target {
                *w = Watched {
                    target: device.target.clone(),
                    ..Watched::default()
                };
            }
            if w.retry_at.is_some_and(|t| now < t) {
                continue;
            }
            // Keep the current watch through a burst of moves.
            if w.requested.as_ref() == Some(&want) || (pane.is_some() && !settled) {
                continue;
            }
            let op = Operation::Watch {
                sessions: true,
                pane,
            };
            let sent = self
                .tx
                .try_send(Task {
                    device: d,
                    execution: self.devices[d].clone(),
                    op,
                    generation: self.generation,
                })
                .is_ok();
            if sent {
                self.pending_requests.set(self.pending_requests.get() + 1);
                let w = self
                    .session_preview
                    .watches
                    .get_mut(&self.devices[d].id)
                    .unwrap();
                w.requested = Some(want);
                w.retry_at = None;
                w.pushed = None;
                w.watch = None;
            }
        }
    }
    /// Before a terminal handoff (attach, expand, a command): end every watch.
    /// Nothing reads helper events while the terminal is handed off, so an
    /// attached pane's frames would pile up; sync_watches asks again on return.
    pub(super) fn stop_watches(&mut self) {
        for d in 0..self.devices.len() {
            let Some(w) = self.session_preview.watches.get_mut(&self.devices[d].id) else {
                continue;
            };
            if w.requested.is_none() {
                continue;
            }
            *w = Watched {
                target: w.target.clone(),
                ..Watched::default()
            };
            let sent = self
                .tx
                .try_send(Task {
                    device: d,
                    execution: self.devices[d].clone(),
                    op: Operation::Watch {
                        sessions: false,
                        pane: None,
                    },
                    generation: self.generation,
                })
                .is_ok();
            if sent {
                self.pending_requests.set(self.pending_requests.get() + 1);
            }
        }
    }
    fn watched_device(
        &self,
        device_id: Option<&String>,
        target: Option<&Option<String>>,
    ) -> Option<usize> {
        self.devices.iter().position(|d| {
            device_id.is_none_or(|id| &d.id == id) && target.is_none_or(|t| &d.target == t)
        })
    }
    /// A Watch reply: accepted (its first screen included) or refused (poll).
    pub(super) fn watch_replied(&mut self, reply: Reply) -> bool {
        self.pending_requests
            .set(self.pending_requests.get().saturating_sub(1));
        let Some(d) =
            self.watched_device(reply.device_id.as_ref(), reply.execution_target.as_ref())
        else {
            return false;
        };
        let Operation::Watch { pane, .. } = &reply.op else {
            return false;
        };
        let want = pane
            .as_ref()
            .map(|s| key(&self.devices[d], s))
            .unwrap_or_default();
        let Some(w) = self.session_preview.watches.get_mut(&self.devices[d].id) else {
            return false;
        };
        if w.requested.as_ref() != Some(&want) {
            return false; // Superseded by a newer watch.
        }
        match reply.result {
            Ok(value) => {
                w.live = true;
                let id = watch_id(&value);
                w.watch = Some(id);
                match (pane, value.get("pane")) {
                    (Some(session), Some(screen)) if w.pushed != Some(id) => {
                        self.apply_watched_screen(d, session.clone(), Ok(screen.clone()))
                    }
                    _ => false,
                }
            }
            Err(_) => {
                // Unsupported here (old tmux, control mode failed): poll.
                w.live = false;
                w.requested = None;
                w.retry_at = Some(Instant::now() + WATCH_RETRY);
                self.session_preview.last = None;
                true
            }
        }
    }
    /// A pushed helper event; returns (redraw, navigation cache changed).
    pub(super) fn watch_event(&mut self, target: Option<String>, event: Value) -> (bool, bool) {
        let Some(d) = self.watched_device(None, Some(&target)) else {
            return (false, false);
        };
        let id = self.devices[d].id.clone();
        let live = self
            .session_preview
            .watches
            .get(&id)
            .is_some_and(|w| w.live);
        match event["kind"].as_str() {
            Some("closed") => {
                if let Some(w) = self.session_preview.watches.get_mut(&id) {
                    w.live = false;
                    w.requested = None;
                    w.retry_at = Some(Instant::now() + WATCH_RECOVER);
                }
                // Poll the screen at once until the watch is back.
                self.session_preview.last = None;
                (false, false)
            }
            Some("sessions") if live => {
                // The same path as a polled list, so selection and caches agree.
                self.pending_requests.set(self.pending_requests.get() + 1);
                self.apply(Reply {
                    device_id: Some(id),
                    execution_target: Some(target),
                    preview: None,
                    device: d,
                    op: Operation::Sessions,
                    generation: self.generation,
                    result: Ok(event["sessions"].clone()),
                });
                (true, true)
            }
            Some("pane") => {
                let Some((sd, session)) = self.preview_selection() else {
                    return (false, false);
                };
                let matches = sd == d
                    && session.terminal.as_ref().map(|t| t.session_id.as_str())
                        == event["session_id"].as_str();
                if !matches || self.session_preview.scroll_top.is_some() {
                    return (false, false);
                }
                // The watch asked for this screen is live, even if its reply
                // (in-process, on another thread) has not landed yet.
                let want = key(&self.devices[d], &session);
                let Some(w) = self.session_preview.watches.get_mut(&id).filter(|w| {
                    w.requested.as_ref() == Some(&want)
                        && w.retry_at.is_none()
                        && w.watch.is_none_or(|id| id == watch_id(&event))
                }) else {
                    return (false, false);
                };
                w.live = true;
                if w.watch.is_none() {
                    w.pushed = Some(watch_id(&event));
                }
                let result = match event["error"].as_str() {
                    Some(error) => Err(anyhow::anyhow!("{error}")),
                    None => Ok(event),
                };
                (self.apply_watched_screen(d, session, result), false)
            }
            _ => (false, false),
        }
    }
    /// Apply a pushed screen exactly like a polled one.
    fn apply_watched_screen(&mut self, d: usize, session: Session, result: Result<Value>) -> bool {
        let revision = self.session_preview.revision;
        // apply_session_preview clears `inflight`; a poll in flight then still
        // lands, and its own (older) reply is dropped by the serial.
        let inflight = self.session_preview.inflight;
        self.apply_session_preview(Reply {
            device_id: Some(self.devices[d].id.clone()),
            execution_target: Some(self.devices[d].target.clone()),
            preview: None,
            device: d,
            op: Operation::SessionPreview {
                session,
                scrollback: false,
                history_start: None,
            },
            generation: self.session_preview.serial,
            result,
        });
        if inflight {
            self.session_preview.serial = self.session_preview.serial.wrapping_add(1);
        }
        self.session_preview.revision != revision
    }
    /// Enter on the previewed session with its live screen loaded: show that
    /// screen full-screen before the handoff, so tmux paints over the same picture.
    pub(super) fn show_attach_frame(&self, session: &Session) -> bool {
        let p = &self.session_preview;
        self.preview_selection().is_some_and(|(d, s)| {
            workspaces::runtime_key(&s) == workspaces::runtime_key(session)
                && p.key.as_ref() == Some(&key(&self.devices[d], &s))
        }) && p.live
            && !p.carried
            && !p.lines.is_empty()
    }
}
/// The live screen alone, top-left, at the pane's size (no chrome).
pub(super) fn render_attach_frame(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let p = &app.session_preview;
    let lines = rendering::ansi_screen(&p.lines, p.palette8, p.cols);
    let start = (p.history_total.saturating_sub(p.line_start) as usize).min(lines.len());
    let rows = usize::from(p.rows.min(area.height));
    let end = (start + rows).min(lines.len());
    let screen = Rect::new(area.x, area.y, p.cols.min(area.width), rows as u16);
    frame.render_widget(Paragraph::new(lines[start..end].to_vec()), screen);
}
/// The pane's colour 8 as the helper reports it (`#rrggbb`).
fn palette8(value: &Value) -> Option<Color> {
    let hex = value["palette8"].as_str()?.strip_prefix('#')?;
    if hex.len() != 6 {
        return None;
    }
    let rgb = u32::from_str_radix(hex, 16).ok()?;
    Some(Color::Rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8))
}
fn fingerprint(p: &Snapshot) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (
        &p.lines,
        &p.pane,
        p.live,
        p.cursor,
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
}
