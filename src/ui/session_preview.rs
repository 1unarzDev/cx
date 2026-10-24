//! Selected-only live tmux screen, with explicitly focused and identity-bound input.
use super::*;
#[derive(Default)]
pub(super) struct Snapshot {
    key: Option<String>,
    serial: u64,
    inflight: bool,
    last: Option<Instant>,
    lines: Vec<String>,
    status: String,
    pane: String,
    live: bool,
    cursor: (u16, u16),
    cursor_visible: bool,
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
        self.session_preview.focused = Some((
            self.devices[d].clone(),
            s,
            self.session_preview.pane.clone(),
        ));
        true
    }
    pub(super) fn session_pane_key(&mut self, event: KeyEvent) {
        if event.modifiers.contains(KeyModifiers::CONTROL) && event.code == KeyCode::Char(']') {
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
                op: Operation::SessionPreview { session },
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
        let Operation::SessionPreview { session } = &reply.op else {
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
                self.session_preview.lines.clear();
                let lines = value.get("styled_lines").unwrap_or(&value["lines"]);
                let mut budget = 256 * 1024;
                for line in lines
                    .as_array()
                    .into_iter()
                    .flatten()
                    .take(300)
                    .filter_map(|v| v.as_str())
                {
                    if line.len() > budget {
                        break;
                    }
                    budget -= line.len();
                    self.session_preview.lines.push(line.to_owned());
                }
                self.session_preview.status = if self.session_preview.live {
                    "Enter · focus terminal"
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
    let Some((d, s)) = app.preview_selection().or_else(|| app.selected_session()) else {
        return area;
    };
    let focused = app.pane_focused();
    if !focused && (area.width < 90 || frame.area().width < 120) {
        return area;
    }
    let (list, pane) = if focused {
        (Rect::new(area.x, area.y, 0, 0), area)
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
    frame.render_widget(
        Paragraph::new(if focused {
            "Ctrl+] · back to sessions · Ctrl+O · expand"
        } else {
            &app.session_preview.status
        })
        .style(muted()),
        Rect::new(inner.x, inner.y, inner.width, inner.height.min(1)),
    );
    if app.session_preview.key.as_ref() == Some(&key(&app.devices[d], &s)) {
        let (cx, cy) = app.session_preview.cursor;
        let top = cy.saturating_sub(screen.height.saturating_sub(1));
        let left = cx.saturating_sub(screen.width.saturating_sub(1));
        let lines: Vec<_> = app
            .session_preview
            .lines
            .iter()
            .map(|s| rendering::ansi_line(s))
            .collect();
        frame.render_widget(Paragraph::new(lines).scroll((top, left)), screen);
        if focused && app.session_preview.cursor_visible && screen.width > 0 && screen.height > 0 {
            frame.set_cursor_position((
                screen.x + cx.saturating_sub(left).min(screen.width - 1),
                screen.y + cy.saturating_sub(top).min(screen.height - 1),
            ));
        }
    }
    list
}
