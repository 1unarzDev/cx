//! Selected-only, ephemeral snapshots. No terminal input, emulation or persistence.
use super::*;

#[derive(Default)]
pub(super) struct Snapshot {
    key: Option<String>,
    serial: u64,
    inflight: bool,
    last: Option<Instant>,
    lines: Vec<String>,
    status: String,
}
fn key(device: &Device, session: &Session) -> String {
    serde_json::json!([
        device.id,
        device.target,
        session.id,
        session.pid,
        session.started,
        session.boot_id,
        session.socket,
        session.process,
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
        .unwrap_or("~");
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
    fn preview_selection(&self) -> Option<(usize, Session)> {
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
                && self
                    .providers
                    .get(d)
                    .is_none_or(|(caps, _)| caps.iter().any(|c| c == "session-preview-v1"))
                && self.work[*d].error.is_none()
                && self.session_status(*d, s).starts_with('+')
        })
    }
    pub(super) fn refresh_session_preview(&mut self, width: u16) -> bool {
        // Work uses a 21-column device rail; 120 columns leaves room for both panes.
        let selection = (width >= 120).then(|| self.preview_selection()).flatten();
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
        if self.session_preview.inflight
            || self
                .session_preview
                .last
                .is_some_and(|t| t.elapsed() < Duration::from_secs(2))
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
            if self.session_preview.lines.is_empty() {
                self.session_preview.status = "Reading terminal snapshot…".into();
            }
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
        self.session_preview.lines.clear();
        match reply.result {
            Ok(value) => {
                let mut budget = 24 * 1024;
                for line in value["lines"].as_array().into_iter().flatten().take(60) {
                    let Some(line) = line.as_str() else {
                        continue;
                    };
                    let safe = safe_text(line);
                    let end = safe
                        .char_indices()
                        .map(|(i, _)| i)
                        .chain(std::iter::once(safe.len()))
                        .take_while(|i| *i <= budget)
                        .last()
                        .unwrap_or(0);
                    self.session_preview.lines.push(safe[..end].to_owned());
                    budget -= end;
                    if budget == 0 {
                        break;
                    }
                }
                self.session_preview.status = if self.session_preview.lines.is_empty() {
                    "Terminal snapshot is empty".into()
                } else {
                    "Read-only · refreshes every 2s".into()
                };
            }
            Err(_) => self.session_preview.status = "Preview unavailable · Enter to attach".into(),
        }
    }
}
pub(super) fn render(frame: &mut Frame, app: &App, area: Rect) -> Rect {
    let Some((d, s)) = app.selected_session() else {
        return area;
    };
    if area.width < 90 || frame.area().width < 120 {
        if area.height < 10 {
            return area;
        }
        let parts = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(5), Constraint::Length(3)])
            .split(area);
        frame.render_widget(
            Paragraph::new(format!(
                "{} · {}\n{} · widen for snapshot",
                label(&s),
                identity(&app.devices[d]),
                safe_label(&s.directory)
            ))
            .style(muted()),
            parts[1],
        );
        return parts[0];
    }
    let parts = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
        .split(area);
    let mut lines = vec![
        Line::from(label(&s)),
        Line::from(identity(&app.devices[d])),
        Line::from(safe_label(&s.directory)),
        Line::from(format!(
            "{} · {}",
            safe_label(&s.provider),
            safe_label(&s.id)
        )),
        Line::from(app.session_status(d, &s)),
        Line::from(""),
    ];
    if app.session_preview.key.as_ref() == Some(&key(&app.devices[d], &s)) {
        lines.push(Line::from(app.session_preview.status.clone()).style(muted()));
        let available = usize::from(parts[1].height.saturating_sub(9));
        let skip = app.session_preview.lines.len().saturating_sub(available);
        lines.extend(
            app.session_preview
                .lines
                .iter()
                .skip(skip)
                .cloned()
                .map(Line::from),
        );
    } else {
        let reason = if s.terminal.is_none() || outside_tmux(&s) {
            "No verified tmux terminal · metadata only"
        } else if !app.session_status(d, &s).starts_with('+') {
            "Session not confirmed live · refresh to check"
        } else {
            "Preview unavailable · helper support required"
        };
        lines.push(Line::from(reason).style(muted()));
    }
    frame.render_widget(
        Paragraph::new(lines).block(block("Selected session · snapshot".into(), false)),
        parts[1],
    );
    parts[0]
}
