//! Action dispatch: the command palette, action availability and execute().
use super::*;

impl App {
    pub(super) fn command_context(&self) -> Option<(usize, String)> {
        if self.view == View::Containers
            || (self.view == View::Files
                && self.browser.as_ref().is_some_and(|b| b.container.is_some()))
        {
            return None;
        }
        if self.view == View::Network {
            return self.network_action_device().map(|d| (d, "~".into()));
        }
        if self.view == View::Files {
            return self.browser.as_ref().map(|b| (b.device, b.path.clone()));
        }
        if self.view == View::Work {
            if let Some((d, session)) = self.selected_session() {
                if session.container.is_some() || session.directory.is_empty() {
                    return None;
                }
                return Some((d, session.directory));
            }
        }
        self.actual_device().map(|d| (d, "~".into()))
    }
    pub(super) fn palette(&self) -> Vec<(Action, &'static str)> {
        ACTIONS
            .iter()
            .copied()
            .filter(|(a, label)| {
                (matches!(
                    a,
                    Action::Work | Action::Network | Action::Files | Action::Terminal | Action::Add
                ) || self.action_enabled(*a))
                    && label.to_lowercase().contains(&self.text.to_lowercase())
            })
            .collect()
    }
    pub(super) fn palette_scope(&self) -> String {
        let Some((action, _)) = self.palette().get(self.palette_selected).copied() else {
            return "No matching actions · type to filter · Esc cancel".into();
        };
        if matches!(
            action,
            Action::Work
                | Action::Network
                | Action::Files
                | Action::Terminal
                | Action::New
                | Action::Shell
                | Action::Add
        ) {
            return if action == Action::Add {
                "Next: choose direct SSH or a gateway · Esc cancel".into()
            } else {
                "Next: choose device · Esc cancel".into()
            };
        }
        if matches!(action, Action::Update | Action::Jobs | Action::Quit) {
            return "Scope: this viewer · Enter run · Esc cancel".into();
        }
        self.command_context()
            .map(|(d, path)| {
                format!(
                    "Current: {} · {}",
                    safe_label(&self.devices[d].name),
                    safe_label(&path)
                )
            })
            .unwrap_or_else(|| "Scope: All devices · Enter run · Esc cancel".into())
    }
    pub(super) fn container_read_only_action(&self, action: Action) -> bool {
        self.view == View::Files
            && self.browser.as_ref().is_some_and(|b| b.container.is_some())
            && action == Action::Command
    }
    pub(super) fn action_enabled(&self, action: Action) -> bool {
        if self.container_read_only_action(action) {
            return false;
        }
        match action {
            Action::DevcontainerUp => {
                self.view == View::Files
                    && self
                        .browser
                        .as_ref()
                        .is_some_and(|b| b.container.is_none() && b.preview.is_none())
            }
            Action::Containers => self.view != View::Containers,
            Action::Work => self.view != View::Work,
            Action::Terminal if self.view == View::Network => self
                .network_rows()
                .get(self.network_selected)
                .is_some_and(|r| r["_peer"].is_u64() || r["_known_peer"].is_u64()),
            Action::New | Action::Shell => !self.creating,
            Action::Command => self.command_context().is_some(),
            Action::Network => self.view != View::Network,
            Action::Destination | Action::Conflict => self.clipboard.is_some(),
            Action::TransferTo => {
                self.view == View::Files
                    && self.browser.as_ref().is_some_and(|b| b.preview.is_none())
                    && (self.clipboard.is_some() || !self.chosen_entries().is_empty())
            }
            Action::Copy | Action::Cut | Action::Delete => {
                self.view == View::Files
                    && self.browser.as_ref().is_some_and(|b| b.preview.is_none())
                    && !self.chosen_entries().is_empty()
                    && (action == Action::Copy
                        || self.chosen_entries().iter().all(|e| e.identity.is_some()))
            }
            Action::Rename => {
                self.view == View::Files
                    && self.browser.as_ref().is_some_and(|b| b.preview.is_none())
                    && self
                        .visible_entries()
                        .get(self.browser.as_ref().map(|b| b.selected).unwrap_or(0))
                        .is_some_and(|e| e.identity.is_some())
            }
            Action::Mkdir | Action::Hidden | Action::Filter | Action::Select | Action::Visual => {
                self.view == View::Files
                    && self.browser.as_ref().is_some_and(|b| b.preview.is_none())
            }
            Action::Paste => {
                self.view == View::Files && self.clipboard.is_some() && self.browser.is_some()
            }
            Action::Observe | Action::PinSession | Action::SendCtrlBackslash => {
                self.view == View::Work && self.selected_session().is_some()
            }
            Action::FolderSessions => {
                self.view == View::Files
                    && self.browser.as_ref().is_some_and(|b| b.preview.is_none())
            }
            _ => true,
        }
    }
    pub(super) fn execute(&mut self, action: Action) {
        if self.container_read_only_action(action) {
            self.set_notice_as(
                NoticeKind::Warning,
                "Open the container terminal to run commands".into(),
            );
            return;
        }
        if action == Action::New && self.view == View::Containers {
            if let Some((d, c)) = self.selected_container() {
                if (c.devcontainer || c.allowed) && c.state == "running" {
                    self.open_container_chooser(d, c.scope());
                } else {
                    self.open_container_actions();
                }
            }
            return;
        }
        if action == Action::New && self.view == View::Files {
            if let Some(b) = &self.browser {
                if let Some(scope) = &b.container {
                    let mut scope = scope.clone();
                    scope.folder = b.path.clone();
                    self.open_container_chooser(b.device, scope);
                    return;
                }
            }
        }
        self.input = None;
        self.text.clear();
        match action {
            Action::DevcontainerUp => {
                if let Some(b) = self
                    .browser
                    .as_ref()
                    .filter(|b| self.view == View::Files && b.container.is_none())
                {
                    self.dialog = Some(Dialog::DevcontainerUp(b.device, b.path.clone()));
                    self.dialog_selected = 0;
                }
            }
            Action::Containers => self.choose_device(ChooseDevice::Containers),
            Action::Terminal => {
                if self.view == View::Network {
                    if let Some(d) = self
                        .network_rows()
                        .get(self.network_selected)
                        .and_then(|r| r["_peer"].as_u64().or_else(|| r["_known_peer"].as_u64()))
                    {
                        self.pending_terminal = Some(self.devices[d as usize].clone());
                    }
                } else {
                    self.choose_device(ChooseDevice::Terminal);
                }
            }
            Action::Shell => self.start_shell(),
            Action::Update => self.force_update = true,
            Action::Reset => {
                self.dialog = Some(Dialog::Reset);
                self.dialog_selected = 0;
            }
            Action::Quit => self.request_quit(),
            Action::Add => {
                self.network_add_target = None;
                self.pending_add_via = if self.view == View::Network {
                    self.network_action_device()
                        .map(|d| self.devices[d].clone())
                } else {
                    self.actual_device().map(|d| self.devices[d].clone())
                }
                .filter(|d| d.target.is_some());
                self.input = Some(Input::Add);
                self.text.clear();
            }
            Action::Help => self.help = true,
            Action::Refresh => self.refresh(),
            Action::Work => {
                self.view = View::Work;
                self.refresh_work();
            }
            Action::Network => {
                self.view = View::Network;
                self.refresh();
            }
            Action::Command => {
                if let Some((d, path)) = self.command_context() {
                    if self.devices[d].target.is_some()
                        && self.providers.get(&d).is_none_or(|(_, checked)| {
                            transport::now().saturating_sub(*checked) >= 60
                        })
                    {
                        self.check_providers(d);
                        self.set_notice(
                            "Checking command support on this device · press : again shortly"
                                .into(),
                        );
                        return;
                    }
                    let supported = self.devices[d].target.is_none()
                        || self.providers.get(&d).is_some_and(|(caps, checked)| {
                            transport::now().saturating_sub(*checked) < 60
                                && caps.iter().any(|c| c == "native-command-v1")
                        });
                    if !supported {
                        self.check_providers(d);
                        self.set_notice_as(NoticeKind::Warning, "Run command needs a current cx helper on this device · update it and retry".into());
                        return;
                    }
                    self.rename_cursor = 0;
                    self.command_target = Some((d, path));
                    self.text.clear();
                    self.input = Some(Input::Command);
                } else {
                    self.set_notice_as(
                        NoticeKind::Warning,
                        "Select a device or open its files first".into(),
                    );
                }
            }
            Action::New => {
                if self.creating {
                    return;
                }
                if self.view == View::Network {
                    if let Some(d) = self.network_action_device() {
                        self.check_providers(d);
                        self.dialog = Some(Dialog::Provider(d, None));
                        self.dialog_selected = 0;
                        return;
                    }
                }
                if self.view == View::Files {
                    if let Some(browser) = &self.browser {
                        let (device, path) = (browser.device, browser.path.clone());
                        if let Some(provider) = self.launch_provider.take() {
                            if self.provider_choices(device).contains(&provider.as_str()) {
                                self.start_at(device, path, provider);
                                return;
                            }
                        }
                        self.check_providers(device);
                        self.dialog = Some(Dialog::Provider(device, Some(path)));
                        self.dialog_selected = 0;
                        return;
                    }
                }
                self.other_browser = None;
                self.destination_active = false;
                self.launch_provider = None;
                if self.view == View::Work {
                    if let Some((d, session)) = self.selected_session() {
                        if let Some(scope) = session.container {
                            self.open_container_chooser(d, scope);
                            return;
                        }
                        self.check_providers(d);
                        self.dialog = Some(Dialog::Provider(
                            d,
                            (!session.directory.is_empty()).then_some(session.directory),
                        ));
                        self.dialog_selected = 0;
                        return;
                    }
                }
                self.choose_device(ChooseDevice::New);
            }
            Action::Files => {
                if self.view == View::Network {
                    if let Some(d) = self.network_action_device() {
                        self.open_browser(d, "~".into());
                        return;
                    }
                }
                self.launch_provider = None;
                self.other_browser = None;
                self.destination_active = false;
                if let Some((d, session)) =
                    self.selected_session().filter(|_| self.view == View::Work)
                {
                    if let Some(scope) = session.container {
                        self.open_container_browser(d, scope);
                    } else {
                        self.open_host_browser(
                            d,
                            if session.directory.is_empty() {
                                "~".into()
                            } else {
                                session.directory
                            },
                        );
                    }
                } else {
                    self.choose_device(ChooseDevice::Files);
                }
            }
            Action::Destination => self.choose_device(ChooseDevice::Destination),
            Action::TransferTo => {
                if self.clipboard.is_none() {
                    self.execute(Action::Copy);
                }
                if self.clipboard.is_some() {
                    self.choose_device(ChooseDevice::Destination);
                }
            }
            Action::Conflict => {
                self.conflict = (self.conflict + 1) % 3;
                self.set_notice(format!("Existing files: {}", self.conflict_policy()));
            }
            Action::Jobs => {
                self.dialog = Some(Dialog::Jobs);
                self.dialog_selected = 0;
                self.dialog_detail_focus = false;
                self.dialog_scroll = 0;
                for d in 0..self.devices.len() {
                    self.send(d, Operation::TransferJobs);
                }
            }
            Action::PinSession => self.toggle_pin(),
            Action::SendCtrlBackslash => self.send_ctrl_backslash(),
            Action::FolderSessions => self.sessions_here(),
            Action::Observe => {
                if let Some((d, s)) = self.selected_session() {
                    self.open_session(d, s, true);
                }
            }
            Action::Copy | Action::Cut => {
                let entries = self.chosen_entries();
                if let Some(b) = &self.browser {
                    if !entries.is_empty() {
                        let count = entries.len();
                        self.clipboard = Some(Clipboard {
                            id: unique_key(),
                            device: b.device,
                            entries,
                            cut: action == Action::Cut,
                            source_label: file_endpoint_label(
                                &self.devices[b.device],
                                &b.container,
                            ),
                            container: b.container.clone(),
                        });
                        self.launch_provider = None;
                        self.set_notice(format!(
                            "{} {count} item{}",
                            if action == Action::Cut {
                                "Cut"
                            } else {
                                "Copied"
                            },
                            if count == 1 { "" } else { "s" }
                        ));
                        self.finish_visual();
                        if let Some(b) = &mut self.browser {
                            b.marked.clear();
                        }
                    }
                }
            }
            Action::Rename => {
                if let Some(b) = &self.browser {
                    if let Some(e) = browser_entries(b).get(b.selected).cloned() {
                        if e.identity.is_some() {
                            self.text = e.rename_name.clone().unwrap_or_default();
                            self.rename_cursor = self
                                .text
                                .rfind('.')
                                .filter(|at| *at > 0)
                                .unwrap_or(self.text.len());
                            self.rename_target = Some((b.device, e));
                            self.input = Some(Input::Rename);
                            self.finish_visual();
                        }
                    }
                }
            }
            Action::Delete => {
                let entries = self.chosen_entries();
                if let Some(b) = &self.browser {
                    if !entries.is_empty() {
                        self.dialog = Some(Dialog::Delete(b.device, entries));
                        self.dialog_selected = 0; // Cancel is always the default.
                        self.dialog_detail_focus = false;
                        self.dialog_scroll = 0;
                        self.finish_visual();
                    }
                }
            }
            Action::Hidden => {
                if let Some(b) = &mut self.browser {
                    let path = browser_entries(b).get(b.selected).map(|e| e.path.clone());
                    b.show_hidden = !b.show_hidden;
                    b.visual_anchor = None;
                    b.selected = path
                        .and_then(|p| browser_entries(b).iter().position(|e| e.path == p))
                        .unwrap_or(0);
                }
            }
            Action::Filter => {
                self.finish_visual();
                self.text = self
                    .browser
                    .as_ref()
                    .map(|b| b.filter.clone())
                    .unwrap_or_default();
                // Remember the committed filter so Esc cancels back to it.
                self.filter_restore = Some(self.text.clone());
                self.input = Some(Input::Filter);
            }
            Action::Select => {
                self.finish_visual();
                if let Some(b) = &mut self.browser {
                    if let Some(e) = browser_entries(b).get(b.selected) {
                        if !b.marked.remove(&e.path) && b.marked.len() < 256 {
                            b.marked.insert(e.path.clone());
                        }
                        b.selected =
                            (b.selected + 1).min(browser_entries(b).len().saturating_sub(1));
                    }
                }
            }
            Action::Visual => {
                if let Some(b) = &mut self.browser {
                    if b.visual_anchor.take().is_none() {
                        b.visual_anchor = Some(b.selected);
                        b.visual_base = b.marked.clone();
                        if let Some(e) = browser_entries(b).get(b.selected) {
                            b.marked.insert(e.path.clone());
                        }
                    } else {
                        b.visual_base.clear();
                    }
                }
            }
            Action::Paste => self.submit_transfer(),
            Action::Mkdir => {
                self.input = Some(Input::Mkdir);
                self.text.clear();
            }
        }
    }
    pub(super) fn execute_palette(&mut self, action: Action) {
        let purpose = match action {
            Action::Containers => Some(ChooseDevice::Containers),
            Action::Work => Some(ChooseDevice::Work),
            Action::Network => Some(ChooseDevice::Network),
            Action::Files => Some(ChooseDevice::Files),
            Action::Terminal => Some(ChooseDevice::Terminal),
            Action::New => Some(ChooseDevice::New),
            Action::Shell => Some(ChooseDevice::Shell),
            Action::Add => Some(ChooseDevice::AddGateway),
            _ => None,
        };
        if let Some(purpose) = purpose {
            self.input = None;
            self.text.clear();
            self.launch_provider = None;
            self.other_browser = None;
            self.destination_active = false;
            let choices = self.device_choices(purpose);
            self.dialog_selected = choices
                .iter()
                .position(|d| *d == self.actual_device())
                .unwrap_or(0);
            self.dialog = Some(Dialog::Device(purpose));
        } else {
            self.execute(action);
        }
    }
}
