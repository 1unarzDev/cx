//! Task replies for session discovery, creation, adoption and command panels.
use super::*;

impl App {
    pub(in crate::ui) fn apply_session_reply(&mut self, reply: Reply, value: Value) {
        match reply.op {
            Operation::Info => {
                let previous = if matches!(self.dialog, Some(Dialog::SessionChooser(d, _)) if d == reply.device)
                {
                    self.host_session_choices(reply.device)
                        .get(self.dialog_selected)
                        .map(|choice| choice.provider.to_string())
                } else if matches!(self.dialog, Some(Dialog::Provider(d, _)) if d == reply.device) {
                    let mut choices = vec!["shell".to_string()];
                    if let Some((available, _)) = self.providers.get(&reply.device) {
                        choices.extend(
                            ["claude", "codex"]
                                .iter()
                                .filter(|p| available.iter().any(|v| v == **p))
                                .map(|p| (*p).to_string()),
                        );
                    }
                    choices.get(self.dialog_selected).cloned()
                } else {
                    None
                };
                let available = value["capabilities"]
                    .as_array()
                    .map(|caps| {
                        caps.iter()
                            .filter_map(Value::as_str)
                            .filter(|p| {
                                [
                                    "claude",
                                    "codex",
                                    "native-command-v1",
                                    "session-preview-v1",
                                    "live-session-pane-v1",
                                    "watch-v1",
                                    "stop-session-v1",
                                    "stop-agent-session-v1",
                                    "session-yolo-v1",
                                    "adopt-session-v1",
                                    "adopt-reptyr-v1",
                                    "containers-v1",
                                    "container-agents-v1",
                                    "pdf-pages-v1",
                                    "stable-update-v1",
                                ]
                                .contains(p)
                            })
                            .map(str::to_owned)
                            .collect()
                    })
                    .unwrap_or_default();
                self.providers
                    .insert(reply.device, (available, transport::now()));
                if self
                    .pending_adoption
                    .as_ref()
                    .is_some_and(|(d, _, _)| d.id == self.devices[reply.device].id)
                {
                    let (device, session, started) = self.pending_adoption.take().unwrap();
                    if device.target == self.devices[reply.device].target
                        && started.elapsed() < Duration::from_secs(12)
                        && self.work[reply.device].sessions.iter().any(|s| {
                            workspaces::runtime_key(s) == workspaces::runtime_key(&session)
                        })
                    {
                        self.adopt_external_session(reply.device, session);
                    }
                }
                if let Some(Dialog::ContainerProvider(d, scope)) = self.dialog.clone() {
                    if d == reply.device {
                        self.check_container_profiles(d, &scope);
                    }
                }
                if matches!(self.dialog, Some(Dialog::SessionChooser(d, _)) if d == reply.device) {
                    self.dialog_selected = previous
                        .as_ref()
                        .and_then(|provider| {
                            self.host_session_choices(reply.device)
                                .iter()
                                .position(|choice| choice.provider == provider)
                        })
                        .unwrap_or(0);
                }
                if matches!(self.dialog, Some(Dialog::Provider(d, _)) if d == reply.device) {
                    self.dialog_selected = previous
                        .and_then(|provider| {
                            self.provider_choices(reply.device)
                                .iter()
                                .position(|p| *p == provider)
                        })
                        .unwrap_or(0);
                }
                if self
                    .providers
                    .get(&reply.device)
                    .is_some_and(|(caps, _)| caps.iter().any(|c| c == "pdf-pages-v1"))
                    && self.browser.as_ref().is_some_and(|b| {
                        b.device == reply.device
                            && b.preview_pending_page.is_none()
                            && b.preview_rich.as_ref().is_some_and(|p| {
                                p.kind == "pdf"
                                    && b.preview_requested_page > 0
                                    && b.preview_requested_page != p.page
                            })
                    })
                {
                    self.start_pdf_page_request();
                }
            }
            Operation::Sessions => {
                let selected_identity = self
                    .selected_session()
                    .map(|(d, s)| (d, workspaces::runtime_key(&s)));
                let data = if value.is_array() {
                    value.clone()
                } else {
                    value.get("sessions").cloned().unwrap_or(Value::Null)
                };
                match serde_json::from_value::<Vec<Session>>(data) {
                    Ok(mut s) => {
                        // Keep discovered external terminals (a separate request);
                        // the managed list owns everything else.
                        s.extend(
                            self.work[reply.device]
                                .sessions
                                .drain(..)
                                .filter(is_discovered_external),
                        );
                        self.work[reply.device].sessions = s;
                        self.work[reply.device].error = None;
                        self.work[reply.device].fetched = transport::now();
                        self.session_list.record(
                            &self.devices[reply.device],
                            &self.work[reply.device].sessions,
                            self.work[reply.device].fetched,
                        );
                        if self.work[reply.device]
                            .sessions
                            .iter()
                            .any(|s| s.container.is_some())
                            && !self.containers.contains_key(&reply.device)
                            && !self.container_errors.contains_key(&reply.device)
                            && !self.containers_loading.contains(&reply.device)
                            && self.send(reply.device, Operation::Containers)
                        {
                            self.containers_loading.insert(reply.device);
                        }
                        if let Some((device, id)) = selected_identity {
                            if let Some(index) = self
                                .session_rows()
                                .iter()
                                .position(|(d, s)| *d == device && workspaces::runtime_key(s) == id)
                            {
                                self.selected = index;
                            } else {
                                self.selected = self.session_rows().len();
                                self.set_notice(
                                    "Selected terminal no longer observed · choose another session"
                                        .into(),
                                );
                            }
                        }
                    }
                    Err(_) => {
                        self.work[reply.device].error = Some("Invalid session metadata".into());
                    }
                }
                let identity = self
                    .selected_session()
                    .map(|(d, s)| (d, workspaces::runtime_key(&s)));
                self.reconcile_session_selection(identity);
                self.reconcile_picker(reply.device);
                if self
                    .attach_check
                    .as_ref()
                    .is_some_and(|(d, _, _, generation)| {
                        *d == reply.device && *generation == reply.generation
                    })
                {
                    let (d, expected, watch, _) = self.attach_check.take().unwrap();
                    if self.work[d].error.is_none() {
                        if let Some(current) = self.work[d]
                            .sessions
                            .iter()
                            .find(|s| {
                                workspaces::runtime_key(s) == workspaces::runtime_key(&expected)
                            })
                            .cloned()
                        {
                            self.pending_attach = Some((d, current, watch));
                        } else {
                            self.set_notice_as(
                                NoticeKind::Warning,
                                "Selected terminal ended or changed · no attachment started".into(),
                            );
                        }
                    }
                }
            }
            Operation::ExternalSessions => {
                if let Ok(mut external) = serde_json::from_value::<Vec<Session>>(value) {
                    let selected = self
                        .selected_session()
                        .map(|(d, s)| (d, workspaces::runtime_key(&s)));
                    let rows = &mut self.work[reply.device].sessions;
                    rows.retain(|s| !is_discovered_external(s));
                    rows.append(&mut external);
                    self.reconcile_session_selection(selected);
                }
            }
            Operation::AdoptSession { id, .. } => match serde_json::from_value::<Session>(value) {
                Ok(s) => {
                    self.work[reply.device].sessions.retain(|row| row.id != *id);
                    self.work[reply.device].sessions.push(s.clone());
                    self.open_session(reply.device, s, false);
                }
                Err(_) => self.set_notice_as(
                    NoticeKind::Error,
                    "Move response invalid · refresh before retrying".into(),
                ),
            },
            Operation::Create(_)
            | Operation::CreateYolo(_)
            | Operation::ContainerCreate { .. }
            | Operation::ContainerAgentCreate { .. } => {
                match serde_json::from_value::<Session>(value) {
                    Ok(s) => {
                        self.work[reply.device].sessions.retain(|current| {
                            workspaces::runtime_key(current) != workspaces::runtime_key(&s)
                        });
                        self.work[reply.device].sessions.push(s.clone());
                        self.pending_attach = Some((reply.device, s, false));
                    }
                    Err(_) => self.set_notice_as(
                        NoticeKind::Error,
                        "Creation response invalid · refresh before retrying".into(),
                    ),
                }
            }
            Operation::CommandPanel { .. } => {
                if let (Some(id), Some(cols), Some(rows)) = (
                    value["id"].as_str(),
                    value["cols"].as_u64(),
                    value["rows"].as_u64(),
                ) {
                    self.command_panel = Some(CommandPanel {
                        device: reply.device,
                        id: id.to_string(),
                        cols: cols as u16,
                        rows: rows as u16,
                        lines: vec![],
                        alive: true,
                        status: None,
                        inflight: false,
                        last_poll: std::time::Instant::now()
                            - std::time::Duration::from_millis(500),
                        label: self.command_panel_label.take().unwrap_or_default(),
                        quiet: std::mem::take(&mut self.yz.shell_quiet),
                    });
                } else {
                    self.set_notice_as(
                        NoticeKind::Error,
                        "Command could not start · refresh before retrying".into(),
                    );
                }
            }
            Operation::CapturePane { id, .. } => {
                let mut finished = None;
                if let Some(panel) = self.command_panel.as_mut().filter(|p| p.id == *id) {
                    panel.inflight = false;
                    panel.last_poll = std::time::Instant::now();
                    // tmux clears the pane and shows "Pane is dead" on exit, so
                    // drop that marker and keep the last frame that had output
                    // rather than overwriting it with the emptied screen.
                    let fresh: Vec<String> = value["lines"]
                        .as_array()
                        .map(|lines| {
                            lines
                                .iter()
                                .filter_map(|l| l.as_str())
                                .filter(|l| !l.trim_start().starts_with("Pane is dead (status"))
                                .map(str::to_owned)
                                .collect()
                        })
                        .unwrap_or_default();
                    let meaningful = fresh.iter().any(|l| !l.trim().is_empty());
                    if meaningful || panel.lines.iter().all(|l| l.trim().is_empty()) {
                        panel.lines = fresh;
                    }
                    let was_alive = panel.alive;
                    panel.alive = value["alive"].as_bool().unwrap_or(false);
                    panel.status = value["status"].as_i64().map(|s| s as i32);
                    if was_alive && !panel.alive {
                        let empty = panel.lines.iter().all(|l| l.trim().is_empty());
                        // `;` (quiet): a success leaves nothing to read either.
                        let empty = empty || (panel.quiet && panel.status == Some(0));
                        finished = Some((panel.status, empty, panel.id.clone(), panel.device));
                    }
                }
                if let Some((status, empty, id, device)) = finished {
                    self.set_notice(match status {
                        Some(0) | None => "Command finished".into(),
                        Some(code) => format!("Command exited with status {code}"),
                    });
                    // Nothing to read: close the panel rather than show an empty box.
                    if empty {
                        self.command_panel = None;
                        self.send(device, Operation::PaneClose { id });
                    }
                }
            }
            Operation::PaneInput { .. } | Operation::PaneClose { .. } => {}
            Operation::StopSession { .. } | Operation::StopAgentSession { .. } => {
                self.set_notice(format!(
                    "Session stopped · {}",
                    identity(&self.devices[reply.device])
                ));
                self.work[reply.device].loading = self.send(reply.device, Operation::Sessions);
            }
            _ => {}
        }
    }
}
