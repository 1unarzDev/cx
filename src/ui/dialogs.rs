//! Modal dialog keys: shared list/confirm navigation in dialog_key, Enter per dialog family,
//! and the confirmation button layout.
use super::*;

impl App {
    pub(super) fn dialog_key(&mut self, key: KeyEvent, dialog: Dialog) {
        if matches!(dialog, Dialog::Jobs | Dialog::Device(_))
            && matches!(key.code, KeyCode::Char('/' | 'f'))
            && !key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            self.navigation.reset();
            self.begin_list_filter();
            return;
        }
        let count = match &dialog {
            Dialog::SessionChooser(d, _) => self.host_session_choices(*d).len(),
            Dialog::DevcontainerUp(..) => 2,
            Dialog::ContainerActions(_, c) => container_action_labels(c).len(),
            Dialog::ContainerProvider(d, scope) => self.container_session_choices(*d, scope).len(),
            Dialog::DestinationScope(_, containers) => 1 + containers.len(),
            Dialog::ContainerConfirm(..) => 2,
            Dialog::Links(links) => links.len(),
            Dialog::Device(purpose) => self.device_choices(*purpose).len(),
            Dialog::Provider(d, _) => self.provider_choices(*d).len(),
            Dialog::Permissions(..) | Dialog::ContainerPermissions(..) => 2,
            Dialog::SessionPicker(_, _, scope, provider, rows) => {
                rows.len()
                    + 1
                    + usize::from(container_agents::has_history(scope.as_ref(), provider))
            }
            Dialog::PinnedEnded(..) => 2,
            Dialog::Jobs => self.job_rows().len(),
            Dialog::AgentTasks(_, session) => session.agent_tasks.len(),
            Dialog::Delete(..) | Dialog::StopShell(..) => 2,
            Dialog::PendingExit(_) => 2,
            Dialog::Neighbor(..) => 1,
            Dialog::Peer(_) => 5,
            Dialog::RemoveDevice(_) => 2,
            Dialog::EditDevice(_) => 4,
            Dialog::Reset => 2,
        };
        let horizontal = matches!(
            dialog,
            Dialog::SessionChooser(..) | Dialog::ContainerProvider(..)
        );
        // A left arrow is an explicit back action in the new-session menus. Keep
        // it separate from list navigation so it never skips a provider choice.
        if matches!(dialog, Dialog::SessionChooser(..) | Dialog::Provider(..))
            && !key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
            && key.code == KeyCode::Left
        {
            self.dialog = None;
            self.dialog_detail_focus = false;
            return;
        }
        if let Some(motion) = self.navigation.read(key, 5, horizontal) {
            let detail = self.dialog_detail_focus
                || (matches!(key.code, KeyCode::PageDown | KeyCode::PageUp)
                    && self.dialog_scroll_max.get() > 0);
            if detail {
                navigation::Scroll {
                    offset: &mut self.dialog_scroll,
                    max: self.dialog_scroll_max.get(),
                }
                .apply(motion);
            } else if !matches!(motion, Navigation::Pending) {
                navigation::List {
                    selected: &mut self.dialog_selected,
                    length: count,
                }
                .apply(motion);
                self.dialog_scroll = 0;
            }
            return;
        }
        let session_choices = match &dialog {
            Dialog::SessionChooser(d, _) => Some(self.host_session_choices(*d)),
            Dialog::ContainerProvider(d, scope) => Some(self.container_session_choices(*d, scope)),
            _ => menus::session_choices(&dialog).map(|choices| choices.iter().collect()),
        };
        if let Some(choices) = session_choices {
            if !key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
            {
                if let KeyCode::Char(c @ ('c' | 'C' | 'x' | 'X' | 's' | 'S' | 'd' | 'D')) = key.code
                {
                    if let Some(index) = choices
                        .iter()
                        .position(|choice| choice.key == c.to_ascii_lowercase())
                    {
                        self.dialog_selected = index;
                        self.dialog_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), dialog);
                    }
                    return;
                }
            }
        }
        if matches!(
            dialog,
            Dialog::Delete(..) | Dialog::StopShell(..) | Dialog::RemoveDevice(..) | Dialog::Reset
        ) && !key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            match key.code {
                KeyCode::Char('n' | 'N') => {
                    self.dialog = None;
                    self.dialog_detail_focus = false;
                    return;
                }
                KeyCode::Char('y' | 'Y') => {
                    self.dialog_selected = 1;
                    self.dialog_detail_focus = false;
                    self.dialog_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), dialog);
                    return;
                }
                KeyCode::Left | KeyCode::Char('h') => {
                    self.dialog_selected = 0;
                    self.dialog_detail_focus = false;
                    return;
                }
                KeyCode::Right | KeyCode::Char('l') => {
                    self.dialog_selected = 1;
                    self.dialog_detail_focus = false;
                    return;
                }
                _ => {}
            }
        }
        if matches!(
            dialog,
            Dialog::Jobs
                | Dialog::AgentTasks(..)
                | Dialog::Delete(..)
                | Dialog::StopShell(..)
                | Dialog::RemoveDevice(..)
                | Dialog::Reset
                | Dialog::ContainerConfirm(..)
                | Dialog::DestinationScope(..)
        ) {
            match key.code {
                KeyCode::Tab | KeyCode::BackTab => {
                    self.dialog_detail_focus = !self.dialog_detail_focus;
                    return;
                }
                KeyCode::PageDown => {
                    self.dialog_scroll = self.dialog_scroll.saturating_add(5).min(4096);
                    return;
                }
                KeyCode::PageUp => {
                    self.dialog_scroll = self.dialog_scroll.saturating_sub(5);
                    return;
                }
                KeyCode::Home => {
                    self.dialog_scroll = 0;
                    return;
                }
                KeyCode::Down | KeyCode::Char('j') if self.dialog_detail_focus => {
                    self.dialog_scroll = self.dialog_scroll.saturating_add(1).min(4096);
                    return;
                }
                KeyCode::Up | KeyCode::Char('k') if self.dialog_detail_focus => {
                    self.dialog_scroll = self.dialog_scroll.saturating_sub(1);
                    return;
                }
                KeyCode::Esc
                    if self.dialog_detail_focus
                        && matches!(
                            dialog,
                            Dialog::Delete(..)
                                | Dialog::StopShell(..)
                                | Dialog::RemoveDevice(..)
                                | Dialog::Reset
                        ) =>
                {
                    self.dialog_detail_focus = false;
                    self.dialog = None;
                    return;
                }
                KeyCode::Enter | KeyCode::Esc if self.dialog_detail_focus => {
                    self.dialog_detail_focus = false;
                    return;
                }
                _ => {}
            }
        }
        if let Dialog::Provider(device, _) = dialog {
            if key.code == KeyCode::Enter && self.dialog_selected >= count {
                self.check_providers(device);
                self.set_notice("Agent availability changed · checking before launch".into());
                return;
            }
        }
        self.dialog_selected = self.dialog_selected.min(count.saturating_sub(1));
        match key.code {
            KeyCode::Esc => self.dialog = None,
            KeyCode::Down | KeyCode::Char('j') => {
                self.dialog_selected = shift(self.dialog_selected, 1, count);
                self.dialog_scroll = 0;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.dialog_selected = shift(self.dialog_selected, -1, count);
                self.dialog_scroll = 0;
            }
            KeyCode::Enter => self.dialog_enter(dialog),
            KeyCode::Char('c') if matches!(dialog, Dialog::Jobs) => {
                if let Some((d, job)) = self.job_rows().get(self.dialog_selected).cloned() {
                    if matches!(
                        job["status"].as_str(),
                        Some("running" | "queued" | "submitting")
                    ) {
                        if let Some(key) = job["key"].as_str() {
                            self.send(d, Operation::TransferCancel { key: key.into() });
                        }
                    }
                }
            }
            KeyCode::Char('r') if matches!(dialog, Dialog::Jobs) => {
                if let Some((d, job)) = self.job_rows().get(self.dialog_selected).cloned() {
                    if matches!(
                        job["status"].as_str(),
                        Some("failed" | "cancelled" | "incomplete")
                    ) {
                        if let Some(key) = job["key"].as_str() {
                            self.queue_file_actions(vec![(
                                d,
                                Operation::TransferRetry { key: key.into() },
                            )]);
                        }
                    }
                }
                for d in 0..self.devices.len() {
                    self.send(d, Operation::TransferJobs);
                }
            }
            _ => {}
        }
    }
    fn dialog_enter(&mut self, dialog: Dialog) {
        match dialog {
            dialog @ (Dialog::SessionChooser(..)
            | Dialog::Provider(..)
            | Dialog::Permissions(..)
            | Dialog::SessionPicker(..)
            | Dialog::PinnedEnded(..)
            | Dialog::StopShell(..)
            | Dialog::PendingExit(..)) => self.session_dialog_enter(dialog),
            dialog @ (Dialog::DevcontainerUp(..)
            | Dialog::ContainerActions(..)
            | Dialog::DestinationScope(..)
            | Dialog::ContainerProvider(..)
            | Dialog::ContainerPermissions(..)
            | Dialog::ContainerConfirm(..)) => self.container_dialog_enter(dialog),
            dialog @ (Dialog::Device(..)
            | Dialog::Peer(..)
            | Dialog::RemoveDevice(..)
            | Dialog::EditDevice(..)
            | Dialog::Reset
            | Dialog::Neighbor(..)) => self.device_dialog_enter(dialog),
            Dialog::Links(links) => {
                if let Some(link) = links.get(self.dialog_selected) {
                    self.open_markdown_link(link);
                }
            }

            Dialog::Jobs | Dialog::AgentTasks(..) => {
                self.dialog_detail_focus = true;
            }
            Dialog::Delete(d, entries) => {
                self.dialog = None;
                if self.dialog_selected == 1 {
                    self.queue_file_actions(
                        entries
                            .into_iter()
                            .map(|e| {
                                (
                                    d,
                                    Operation::Remove {
                                        path: e.path,
                                        expected_identity: e.identity,
                                    },
                                )
                            })
                            .collect(),
                    );
                    self.set_notice("Deleting confirmed items permanently…".into());
                }
            }
        }
    }
    fn session_dialog_enter(&mut self, dialog: Dialog) {
        match dialog {
            Dialog::SessionChooser(d, path) => {
                let Some(choice) = self
                    .host_session_choices(d)
                    .get(self.dialog_selected)
                    .copied()
                else {
                    return;
                };
                let provider = choice.provider;
                if !self.provider_choices(d).contains(&provider) {
                    self.check_providers(d);
                    self.set_notice_as(NoticeKind::Warning, if self.provider_loading.contains(&d) {
                    "Checking availability · choose again when ready".into()
                } else {
                    format!("{} is unavailable on {} · install its runtime or update the helper", provider, self.devices[d].name)
                });
                    return;
                }
                self.dialog = None;
                if provider == "container" {
                    self.device = d + 1;
                    self.open_containers();
                } else {
                    self.start_at(d, path, provider.into());
                }
            }

            Dialog::Provider(d, path) => {
                let choices = self.provider_choices(d);
                let Some(provider) = choices.get(self.dialog_selected) else {
                    return;
                };
                let provider = (*provider).to_string();
                self.dialog = None;
                if provider == "container" {
                    self.device = d + 1;
                    self.open_containers();
                    return;
                }
                if let Some(directory) = path {
                    self.start_at(d, directory, provider);
                    return;
                }
                self.launch_provider = Some(provider);
                self.open_browser(d, "~".into());
            }
            Dialog::Permissions(d, path, provider, resume, pick) => {
                let yolo = self.dialog_selected == 1;
                if yolo
                    && self.devices[d].target.is_some()
                    && !self.providers.get(&d).is_some_and(|(caps, checked)| {
                        transport::now().saturating_sub(*checked) < 60
                            && caps.iter().any(|c| c == "session-yolo-v1")
                    })
                {
                    self.check_providers(d);
                    self.set_notice_as(
                        NoticeKind::Warning,
                        "YOLO needs an updated execution helper · update this device, then retry"
                            .into(),
                    );
                    return;
                }
                self.dialog = None;
                self.create_permission_session(d, path, provider, yolo, resume, pick);
            }
            Dialog::SessionPicker(d, path, scope, provider, rows) => {
                if let Some(session) = rows.get(self.dialog_selected) {
                    self.dialog = None;
                    self.open_session(d, session.clone(), false);
                } else if let Some(scope) = scope {
                    if provider.is_empty() {
                        self.open_container_chooser(d, scope);
                    } else if provider == "shell" {
                        self.dialog = None;
                        self.start_container_session(d, scope, provider, false);
                    } else {
                        let resume = self.dialog_selected == rows.len() + 1;
                        self.container_permissions(d, scope, provider, resume);
                    }
                } else if self.dialog_selected == rows.len() + 1
                    && container_agents::has_history(None, &provider)
                {
                    self.dialog = Some(Dialog::Permissions(d, path, provider, true, true));
                    self.dialog_selected = 0;
                } else {
                    self.dialog = None;
                    self.create_at(
                        d,
                        path,
                        if provider.is_empty() {
                            "shell".into()
                        } else {
                            provider
                        },
                    );
                }
            }
            Dialog::PinnedEnded(d, session) => {
                self.dialog = None;
                if self.dialog_selected == 0 {
                    if let Some(scope) = session.container {
                        self.open_container_browser(d, scope);
                    } else {
                        self.open_host_browser(d, session.directory);
                    }
                } else {
                    self.refresh_sessions_on(d);
                }
            }
            Dialog::StopShell(d, session) => {
                self.dialog = None;
                if self.dialog_selected == 1 {
                    self.send(
                        d,
                        if session.provider == "shell" {
                            Operation::StopSession {
                                id: session.id,
                                pid: session.pid,
                                started: session.started,
                                boot_id: session.boot_id,
                            }
                        } else {
                            Operation::StopAgentSession {
                                id: session.id,
                                pid: session.pid,
                                started: session.started,
                                boot_id: session.boot_id,
                                provider: session.provider,
                            }
                        },
                    );
                    self.set_notice("Stopping confirmed session…".into());
                }
            }
            Dialog::PendingExit(_) => {
                if self.dialog_selected == 1 {
                    self.quit = true;
                }
                self.dialog = None;
            }
            _ => {}
        }
    }
    fn container_dialog_enter(&mut self, dialog: Dialog) {
        match dialog {
            Dialog::DevcontainerUp(d, workspace) => {
                self.dialog = None;
                if self.dialog_selected == 1 {
                    self.send(d, Operation::DevcontainerUp { workspace });
                    self.set_notice(
                        "Starting workspace · configuration hooks may run · up to 10 minutes"
                            .into(),
                    );
                }
            }
            Dialog::ContainerActions(d, c) => {
                let labels = container_action_labels(&c);
                let Some(action) = labels.get(self.dialog_selected) else {
                    return;
                };
                let action = *action;
                self.dialog = None;
                match action {
                    "Shell" | "Devcontainer terminal" => self.open_container_sessions(d, c.scope()),
                    "Files" => self.open_container_browser(d, c.scope()),
                    "Start" | "Stop" | "Rebuild" | "Enable access" | "Disable access" => {
                        self.dialog = Some(Dialog::ContainerConfirm(d, c, action.into()));
                        self.dialog_selected = 0;
                    }
                    _ => (),
                }
            }
            Dialog::DestinationScope(d, containers) => {
                if self.dialog_selected == 0 {
                    self.dialog = None;
                    self.open_host_browser(d, "~".into());
                } else if let Some(c) = containers.get(self.dialog_selected - 1).cloned() {
                    self.dialog = None;
                    self.open_scoped_browser(d, c.folder.clone(), Some(c.scope()));
                }
            }
            Dialog::ContainerProvider(d, scope) => {
                if let Some(choice) = self
                    .container_session_choices(d, &scope)
                    .get(self.dialog_selected)
                {
                    self.choose_container_profile(d, scope, choice.provider.into());
                }
            }
            Dialog::ContainerPermissions(d, scope, provider, resume) => {
                self.launch_container_agent(d, scope, provider, self.dialog_selected == 1, resume);
            }
            Dialog::ContainerConfirm(d, c, action) => {
                self.dialog = None;
                if self.dialog_selected == 1 {
                    let op = if action == "Rebuild" {
                        if self.devices[d].target.is_some()
                            && !self.providers.get(&d).is_some_and(|(caps, _)| {
                                caps.iter().any(|v| v == "devcontainer-rebuild-v1")
                            })
                        {
                            self.set_notice_as(
                                NoticeKind::Warning,
                                "Update CX on this device before rebuilding devcontainers".into(),
                            );
                            return;
                        }
                        Operation::DevcontainerRebuild { scope: c.scope() }
                    } else if action == "Enable access" || action == "Disable access" {
                        Operation::ContainerAccess {
                            engine: c.engine,
                            id: c.id,
                            enabled: action == "Enable access",
                        }
                    } else {
                        Operation::ContainerLifecycle {
                            engine: c.engine,
                            id: c.id,
                            started_at: c.started_at,
                            action: action.to_lowercase(),
                        }
                    };
                    if let Operation::DevcontainerRebuild { scope } = &op {
                        let key = (d, scope.id.clone());
                        if self.rebuilding.contains(&key) {
                            self.set_notice_as(
                                NoticeKind::Warning,
                                "Rebuild already running · wait for its result".into(),
                            );
                            return;
                        }
                        if !self.send(d, op.clone()) {
                            self.set_notice_as(
                                NoticeKind::Warning,
                                "Request queue busy · rebuild was not started".into(),
                            );
                            return;
                        }
                        self.rebuilding.insert(key);
                    } else if !self.send(d, op) {
                        self.set_notice_as(
                            NoticeKind::Warning,
                            "Request queue busy · action was not started".into(),
                        );
                        return;
                    }
                    self.set_notice(format!(
                        "{action} requested for {} · refresh checks the outcome",
                        c.name
                    ));
                }
            }
            _ => {}
        }
    }
    fn device_dialog_enter(&mut self, dialog: Dialog) {
        match dialog {
            Dialog::Device(purpose) => {
                let choice = self
                    .device_choices(purpose)
                    .get(self.dialog_selected)
                    .copied();
                match choice {
                    Some(Some(d)) => self.chosen_device(d, purpose),
                    Some(None) if purpose == ChooseDevice::AddGateway => {
                        self.dialog = None;
                        self.pending_add_via = None;
                        self.network_add_target = None;
                        self.input = Some(Input::Add);
                        self.text.clear();
                    }
                    Some(None) => {
                        self.dialog = None;
                        self.device = 0;
                        self.view = if purpose == ChooseDevice::Containers {
                            View::Containers
                        } else if purpose == ChooseDevice::Network {
                            View::Network
                        } else {
                            View::Work
                        };
                        self.focus = Focus::Workspace;
                        self.network_selected = 0;
                        self.refresh();
                    }
                    None => (),
                }
            }
            Dialog::Peer(d) => {
                self.dialog = None;
                match self.dialog_selected {
                    0 => {
                        self.device = d + 1;
                        self.view = View::Work;
                        self.selected = 0;
                        self.refresh_work();
                    }
                    1 => self.open_browser(d, "~".into()),
                    2 => self.start_at(d, "~".into(), "shell".into()),
                    4 => self.pending_terminal = Some(self.devices[d].clone()),
                    _ => {
                        self.check_providers(d);
                        self.dialog = Some(Dialog::Provider(d, None));
                        self.dialog_selected = 0;
                    }
                }
            }
            Dialog::RemoveDevice(_) => {
                if self.dialog_selected == 1 {
                    self.dialog = None;
                    self.remove_selected_device();
                } else {
                    self.dialog = None;
                }
            }
            Dialog::EditDevice(d) => match self.dialog_selected {
                0 => {
                    self.dialog = None;
                    self.pending_edit = Some(d);
                    self.input = Some(Input::Add);
                    self.text = self.devices[d].target.clone().unwrap_or_default();
                }
                1 => {
                    self.device = d + 1;
                    self.toggle_access();
                }
                2 => {
                    self.dialog = Some(Dialog::RemoveDevice(d));
                    self.dialog_selected = 0;
                }
                _ => self.dialog = None,
            },
            Dialog::Reset => {
                self.dialog = None;
                if self.dialog_selected == 1 {
                    match crate::sessions::reset() {
                        Ok(report) => self.set_notice_as(
                            NoticeKind::Success,
                            format!(
                                "Managed tmux reset · {} sessions terminated",
                                report["managed_sessions"].as_u64().unwrap_or(0)
                            ),
                        ),
                        Err(error) => self.set_notice_as(
                            NoticeKind::Error,
                            format!("Managed session reset failed: {error:#}"),
                        ),
                    }
                }
            }
            Dialog::Neighbor(d, candidate) => {
                self.dialog = None;
                self.connect_neighbor(d, candidate);
            }
            _ => {}
        }
    }
}

pub(super) fn confirmation_popup(area: Rect, detail: &str) -> Rect {
    let width = popup(area, 68, 12).width.saturating_sub(4).max(1);
    let lines = detail
        .lines()
        .map(|line| Line::raw(line).width().max(1).div_ceil(usize::from(width)))
        .sum::<usize>()
        .clamp(2, 6);
    popup(area, 68, lines as u16 + 7)
}
#[derive(Clone, Copy)]
pub(super) enum ConfirmationLayout {
    Separate,
    #[cfg(test)]
    Joined,
    #[cfg(test)]
    Compact,
}
pub(super) fn confirmation_buttons(
    frame: &mut Frame,
    area: Rect,
    stop: bool,
    selected: usize,
    detail_focus: bool,
    layout: ConfirmationLayout,
    labels: [&str; 2],
) {
    frame.render_widget(Clear, area);
    let labels = if stop && area.width < 38 {
        ["Keep", "Stop"]
    } else {
        labels
    };
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);
    #[cfg(test)]
    if matches!(layout, ConfirmationLayout::Joined) {
        frame.render_widget(
            Block::default()
                .borders(Borders::ALL)
                .border_set(overlay_border())
                .border_style(faint()),
            area,
        );
    }
    for (i, label) in labels.into_iter().enumerate() {
        let active = selected == i && !detail_focus;
        let tone = if i == 1 { err() } else { accent() };
        let emphasis = if active {
            Modifier::BOLD
        } else {
            Modifier::empty()
        };
        let style = tone.add_modifier(emphasis);
        let line = Line::from(vec![
            Span::styled(
                if active {
                    if ascii() {
                        "> "
                    } else {
                        "› "
                    }
                } else {
                    "  "
                },
                style,
            ),
            Span::styled(
                format!("{} ", if i == 0 { "n" } else { "y" }),
                tone.add_modifier(Modifier::BOLD),
            ),
            Span::styled(label, style),
        ]);
        match layout {
            ConfirmationLayout::Separate => {
                let button = Rect::new(
                    columns[i].x + u16::from(i == 1),
                    columns[i].y,
                    columns[i].width.saturating_sub(1),
                    columns[i].height,
                );
                frame.render_widget(
                    Paragraph::new(line)
                        .alignment(ratatui::layout::Alignment::Center)
                        .block(
                            Block::default()
                                .borders(Borders::ALL)
                                .border_set(border_set(active))
                                .border_style(if active { style } else { faint() }),
                        ),
                    button,
                );
            }
            #[cfg(test)]
            ConfirmationLayout::Joined => {
                let button = Rect::new(
                    columns[i].x + 1,
                    columns[i].y + 1,
                    columns[i].width.saturating_sub(2),
                    1,
                );
                frame.render_widget(
                    Paragraph::new(line).alignment(ratatui::layout::Alignment::Center),
                    button,
                );
                if i == 1 {
                    frame.render_widget(
                        Paragraph::new(if ascii() { "|" } else { "│" }).style(muted()),
                        Rect::new(columns[i].x, columns[i].y + 1, 1, 1),
                    );
                }
            }
            #[cfg(test)]
            ConfirmationLayout::Compact => {
                frame.render_widget(
                    Paragraph::new(Line::from(vec![
                        Span::styled("[ ", style),
                        Span::styled(
                            format!(
                                "{}{} {label}",
                                if active { "> " } else { "  " },
                                if i == 0 { "n" } else { "y" }
                            ),
                            style,
                        ),
                        Span::styled(" ]", style),
                    ]))
                    .alignment(ratatui::layout::Alignment::Center),
                    Rect::new(columns[i].x, columns[i].y + 1, columns[i].width, 1),
                );
            }
        }
    }
}
