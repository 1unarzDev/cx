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
    let name = s.name.trim();
    if !name.is_empty() && name != s.id && name != s.provider && name != "shell" {
        return safe_label(name);
    }
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
    format!(
        "{} · {} · {}",
        safe_label(folder),
        safe_label(&s.provider),
        safe_label(
            s.terminal
                .as_ref()
                .map(|t| t.session_id.as_str())
                .unwrap_or_else(|| s.id.get(s.id.len().saturating_sub(8)..).unwrap_or(&s.id))
        )
    )
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
    fn resume_session_preview(&mut self) {
        self.session_preview.scroll_top = None;
        self.session_preview.history_loaded = false;
        self.session_preview.last = None;
    }
    pub(super) fn scroll_session_preview(&mut self, mouse: MouseEvent) -> bool {
        if self.view != View::Work
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
        if self.view != View::Work
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
    pub(super) fn session_pane_key(&mut self, event: KeyEvent) {
        // Legacy terminals encode Ctrl+] as byte0x1d, which crossterm reports as Ctrl+5.
        if event.modifiers.contains(KeyModifiers::CONTROL)
            && matches!(event.code, KeyCode::Char(']') | KeyCode::Char('5'))
        {
            self.session_preview.focused = None;
            return;
        }
        let Some((d, _)) = self.preview_selection() else {
            self.session_preview = Snapshot::default();
            return;
        };
        let (_, session, pane_id) = self.session_preview.focused.clone().unwrap();
        if event.modifiers.contains(KeyModifiers::CONTROL) && event.code == KeyCode::Char('o') {
            self.session_preview.focused = None;
            self.open_session(d, session, false);
            return;
        }
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
            if !self.send(
                d,
                Operation::SessionPaneInput {
                    session,
                    pane_id,
                    hex,
                },
            ) {
                self.session_preview.focused = None;
                self.set_notice("Terminal input was not queued; focus released".into());
            }
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
        if !self.send(
            d,
            Operation::SessionPaneInput {
                session,
                pane_id,
                hex,
            },
        ) {
            self.session_preview.focused = None;
            self.set_notice("Paste was not queued; focus released".into());
        }
    }
    pub(super) fn refresh_session_preview(&mut self, _width: u16) -> bool {
        let selection = self.preview_selection();
        let next = selection.as_ref().map(|(d, s)| key(&self.devices[*d], s));
        let changed = next != self.session_preview.key;
        if changed {
            let serial = self.session_preview.serial.wrapping_add(1);
            self.session_preview = Snapshot {
                key: next,
                serial,
                ..Snapshot::default()
            };
        }
        let Some((device, session)) = selection else {
            return changed;
        };
        if self
            .session_preview
            .scroll_top
            .is_some_and(|top| self.preview_page_contains(top))
            && self.session_preview.history_loaded
        {
            return changed;
        }
        let interval = if self.pane_focused() { 100 } else { 250 };
        if self.session_preview.inflight
            || self
                .session_preview
                .last
                .is_some_and(|t| t.elapsed() < Duration::from_millis(interval))
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
        changed || sent
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
        self.session_preview.inflight = false;
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
                self.session_preview.status = if self.session_preview.live {
                    "Enter · focus terminal · Wheel · scroll"
                } else {
                    "Enter · attach"
                }
                .into();
            }
            Err(_) => {
                self.session_preview.focused = None;
                self.session_preview.live = false;
                self.session_preview.lines.clear();
                self.session_preview.status = "Terminal unavailable · refresh to check".into();
            }
        }
    }
}
pub(super) fn render(frame: &mut Frame, app: &App, area: Rect) -> Rect {
    app.session_preview.screen.set(Rect::default());
    let Some((d, s)) = app.preview_selection().or_else(|| app.selected_session()) else {
        return area;
    };
    let focused = app.pane_focused();
    if !focused && (area.width < 28 || area.height < 12) {
        return area;
    }
    let (list, pane) = if focused {
        (Rect::new(area.x, area.y, 0, 0), area)
    } else if area.width < 90 || frame.area().width < 120 {
        let parts = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(area);
        (parts[0], parts[1])
    } else {
        let parts = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
            .split(area);
        (parts[0], parts[1])
    };
    let border = block(
        format!("{} · {}", label(&s), identity(&app.devices[d])),
        focused,
    );
    let inner = border.inner(pane);
    frame.render_widget(border, pane);
    let screen = Rect::new(
        inner.x,
        inner.y.saturating_add(1),
        inner.width,
        inner.height.saturating_sub(1),
    );
    app.session_preview.screen.set(screen);
    frame.render_widget(
        Paragraph::new(if app.session_preview.scroll_top.is_some() {
            "Scrollback · scroll down to return live"
        } else if focused {
            "Wheel · scroll · Ctrl+] · back to sessions · Ctrl+O · expand"
        } else if outside_tmux(&s) {
            "External terminal · Enter to move into CX"
        } else if app.session_preview.status.is_empty() {
            "Loading terminal…"
        } else {
            &app.session_preview.status
        })
        .style(muted()),
        Rect::new(inner.x, inner.y, inner.width, inner.height.min(1)),
    );
    if app.session_preview.key.as_ref() == Some(&key(&app.devices[d], &s)) {
        let (cx, cy) = app.session_preview.cursor;
        let cy = u32::from(cy).saturating_add(app.session_preview.history_total);
        let top = app
            .session_preview
            .scroll_top
            .unwrap_or_else(|| app.live_top());
        let left = if app.session_preview.scroll_top.is_some() {
            0
        } else {
            cx.saturating_sub(screen.width.saturating_sub(1))
        };
        let lines = rendering::ansi_screen(
            &app.session_preview.lines,
            app.session_preview.palette8,
            app.session_preview.cols,
        );
        let relative_top = top
            .saturating_sub(app.session_preview.line_start)
            .min(u32::from(u16::MAX)) as u16;
        frame.render_widget(Paragraph::new(lines).scroll((relative_top, left)), screen);
        protect_content(frame.buffer_mut(), screen);
        if focused
            && app.session_preview.scroll_top.is_none()
            && app.session_preview.cursor_visible
            && screen.width > 0
            && screen.height > 0
        {
            frame.set_cursor_position((
                screen.x + cx.saturating_sub(left).min(screen.width - 1),
                screen.y + cy.saturating_sub(top).min(u32::from(screen.height - 1)) as u16,
            ));
        }
    }
    list
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
