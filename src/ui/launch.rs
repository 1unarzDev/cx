//! Session launch flows: device/provider choice, permission prompts, shells and adoption.
use super::*;

impl App {
    pub(super) fn provider_choices(&self, device: usize) -> Vec<&'static str> {
        let mut choices = vec!["shell"];
        if let Some((available, checked)) = self.providers.get(&device) {
            if transport::now().saturating_sub(*checked) < 60 {
                for provider in ["claude", "codex"] {
                    if available.iter().any(|p| p == provider) {
                        choices.push(provider);
                    }
                }
                if available.iter().any(|p| p == "containers-v1") {
                    choices.push("container");
                }
            }
        }
        choices
    }
    pub(super) fn host_session_choices(&self, device: usize) -> Vec<&'static menus::SessionChoice> {
        let available = self.provider_choices(device);
        menus::HOST
            .iter()
            .filter(|choice| available.contains(&choice.provider))
            .collect()
    }
    /// Attach to a tmux session, or move a terminal started outside CX into
    /// managed tmux first when the device has a verified relay.
    pub(super) fn adopt_external_session(&mut self, device: usize, session: Session) {
        let Some(origin) = session.origin.clone().filter(|_| session.external) else {
            self.check_session(device, session, false);
            return;
        };
        let Some(caps) = self
            .providers
            .get(&device)
            .filter(|(_, checked)| transport::now().saturating_sub(*checked) < 60)
            .map(|(caps, _)| caps.clone())
        else {
            self.pending_adoption = Some((self.devices[device].clone(), session, Instant::now()));
            self.check_providers(device);
            self.set_notice("Checking terminal availability… · Esc cancels".into());
            return;
        };
        self.pending_adoption = None;
        if caps.iter().any(|c| c == "adopt-reptyr-v1") {
            if origin.holder.is_none() {
                self.set_notice_as(
                    NoticeKind::Warning,
                    "This terminal's owner is not accessible, so it cannot be moved".into(),
                );
            } else if self.send(
                device,
                Operation::AdoptSession {
                    id: session.id.clone(),
                    origin,
                    boot_id: session.boot_id.clone(),
                },
            ) {
                // Async move with the animated loader. A local move that needs a
                // sudo password gets it through the persistent askpass broker
                // the event loop services, so the loop never blocks.
                self.creating = true;
                self.set_notice("Moving terminal into CX…".into());
            }
        } else if !outside_tmux(&session) {
            self.check_session(device, session, false);
        } else if caps.iter().any(|c| c == "adopt-session-v1") {
            // Set the relay up from inside CX rather than asking the user to run
            // a CLI command; the move can be retried once it is installed.
            self.pending_ptrace = Some(device);
        } else {
            self.set_notice_as(
                NoticeKind::Warning,
                "Update CX on this device to move terminals".into(),
            );
        }
    }
    pub(super) fn check_providers(&mut self, device: usize) {
        if !self.background_checks_enabled(device) {
            return;
        }
        if !self.provider_loading.contains(&device)
            && self
                .providers
                .get(&device)
                .is_none_or(|(_, checked)| transport::now().saturating_sub(*checked) >= 60)
            && self.send(device, Operation::Info)
        {
            self.provider_loading.insert(device);
        }
    }
    pub(super) fn device_choices(&self, purpose: ChooseDevice) -> Vec<Option<usize>> {
        let mut choices = Vec::new();
        if matches!(
            purpose,
            ChooseDevice::Work
                | ChooseDevice::Network
                | ChooseDevice::Containers
                | ChooseDevice::AddGateway
        ) {
            choices.push(None);
        }
        choices.extend(
            (0..self.devices.len())
                .filter(|d| {
                    self.device_matches(*d)
                        && (purpose != ChooseDevice::AddGateway
                            || self.devices[*d].target.is_some())
                })
                .map(Some),
        );
        choices
    }
    pub(super) fn session_shortcut(&mut self) {
        if self.creating {
            return;
        }
        self.launch_provider = None;
        if self.focus != Focus::Devices && self.view == View::Containers {
            if let Some((d, c)) = self.selected_container() {
                if (c.devcontainer || c.allowed) && c.state == "running" {
                    self.open_container_chooser(d, c.scope());
                } else {
                    self.open_container_actions();
                }
            }
            return;
        }
        if self.focus != Focus::Devices && self.view == View::Files {
            if let Some(b) = &self.browser {
                if let Some(scope) = &b.container {
                    let mut scope = scope.clone();
                    scope.folder = b.path.clone();
                    self.open_container_chooser(b.device, scope);
                    return;
                }
            }
        }
        if self.focus == Focus::Workspace && self.view == View::Work {
            if let Some((d, session)) = self.selected_session() {
                if let Some(scope) = session.container {
                    self.open_container_chooser(d, scope);
                    return;
                }
            }
        }
        let location = if self.focus == Focus::Devices {
            self.actual_device().map(|d| (d, "~".into()))
        } else if self.view == View::Files {
            self.browser.as_ref().map(|b| (b.device, b.path.clone()))
        } else if self.view == View::Network {
            self.network_action_device().map(|d| (d, "~".into()))
        } else {
            self.actual_device().map(|d| (d, "~".into()))
        };
        if let Some((d, path)) = location {
            self.open_session_chooser(d, path);
        } else {
            self.dialog = Some(Dialog::Device(ChooseDevice::SessionChooser));
            self.dialog_selected = 0;
        }
    }
    pub(super) fn open_session_chooser(&mut self, d: usize, path: String) {
        self.check_providers(d);
        self.dialog = Some(Dialog::SessionChooser(d, path));
        self.dialog_selected = 0;
    }
    pub(super) fn start_shell(&mut self) {
        if self.view == View::Containers {
            if let Some((d, c)) = self.selected_container() {
                if (!c.devcontainer && !c.allowed) || c.state != "running" {
                    self.open_container_actions();
                    return;
                }
                self.open_container_sessions(d, c.scope());
            }
            return;
        }
        if self.view == View::Files {
            if let Some(b) = &self.browser {
                if let Some(scope) = &b.container {
                    let d = b.device;
                    let mut scope = scope.clone();
                    scope.folder = b.path.clone();
                    self.start_container_session(d, scope, "shell".into(), false);
                    return;
                }
            }
        }
        if self.creating {
            return;
        }
        let location = if self.view == View::Files {
            self.browser.as_ref().map(|b| (b.device, b.path.clone()))
        } else if self.view == View::Network {
            self.network_action_device().map(|d| (d, "~".into()))
        } else if self.device > 0 {
            self.actual_device().map(|d| (d, "~".into()))
        } else {
            None
        };
        self.launch_provider = None;
        if let Some((d, path)) = location {
            self.create_at(d, path, "shell".into());
        } else {
            self.dialog = Some(Dialog::Device(ChooseDevice::Shell));
            self.dialog_selected = 0;
        }
    }
    pub(super) fn choose_device(&mut self, purpose: ChooseDevice) {
        if self.device > 0 && purpose != ChooseDevice::Destination {
            self.chosen_device(self.device - 1, purpose);
        } else {
            self.dialog = Some(Dialog::Device(purpose));
            self.dialog_selected = self.actual_device().unwrap_or(0);
        }
    }
    pub(super) fn chosen_device(&mut self, d: usize, purpose: ChooseDevice) {
        self.dialog = None;
        match purpose {
            ChooseDevice::Work | ChooseDevice::Network => {
                self.device = d + 1;
                self.view = if purpose == ChooseDevice::Work {
                    View::Work
                } else {
                    View::Network
                };
                self.focus = Focus::Workspace;
                self.network_selected = 0;
                self.refresh();
            }
            ChooseDevice::AddGateway => {
                self.pending_add_via = self.devices[d]
                    .target
                    .as_ref()
                    .map(|_| self.devices[d].clone());
                self.network_add_target = None;
                self.input = Some(Input::Add);
                self.text.clear();
            }
            ChooseDevice::Containers => {
                self.device = d + 1;
                self.open_containers();
            }
            ChooseDevice::SessionChooser => self.open_session_chooser(d, "~".into()),
            ChooseDevice::Shell => {
                self.device = d + 1;
                self.view = View::Work;
                self.create_at(d, "~".into(), "shell".into());
            }
            ChooseDevice::Terminal => self.pending_terminal = Some(self.devices[d].clone()),
            ChooseDevice::New => {
                self.check_providers(d);
                self.work[d].loading = self.send(d, Operation::Sessions);
                if self.launch_provider.is_some() {
                    self.open_browser(d, "~".into());
                } else {
                    self.dialog = Some(Dialog::Provider(d, None));
                    self.dialog_selected = 0;
                }
            }
            ChooseDevice::Files => {
                self.device = d + 1;
                self.open_host_browser(d, "~".into());
            }
            ChooseDevice::Destination => {
                self.device = d + 1;
                if !self.destination_active {
                    self.other_browser = self.browser.take();
                    self.destination_active = true;
                }
                if self
                    .providers
                    .get(&d)
                    .is_some_and(|(caps, _)| caps.iter().any(|v| v == "containers-v1"))
                    || self.containers.contains_key(&d)
                {
                    self.send(d, Operation::Containers);
                    self.containers_loading.insert(d);
                    self.dialog = Some(Dialog::DestinationScope(d, self.destination_containers(d)));
                    self.dialog_selected = 0;
                } else {
                    self.open_host_browser(d, "~".into());
                }
            }
        }
    }
    pub(super) fn start_at(&mut self, d: usize, directory: String, provider: String) {
        if !self.provider_choices(d).contains(&provider.as_str()) {
            self.check_providers(d);
            self.set_notice_as(
                NoticeKind::Warning,
                "Launch profile unavailable or not yet checked on this device".into(),
            );
            return;
        }
        let rows = self.scoped_sessions(d, &directory, None, &provider);
        if !rows.is_empty() || container_agents::has_history(None, &provider) {
            self.dialog = Some(Dialog::SessionPicker(d, directory, None, provider, rows));
            self.dialog_selected = 0;
            self.refresh_sessions_on(d);
        } else {
            self.create_at(d, directory, provider);
        }
    }
    pub(super) fn create_at(&mut self, d: usize, directory: String, provider: String) {
        if provider != "shell" {
            self.dialog = Some(Dialog::Permissions(d, directory, provider, false, false));
            self.dialog_selected = 0;
            return;
        }
        self.create_permission_session(d, directory, provider, false, false, false);
    }
    pub(super) fn create_permission_session(
        &mut self,
        d: usize,
        directory: String,
        provider: String,
        yolo: bool,
        resume: bool,
        pick: bool,
    ) {
        let key = unique_key();
        // Execution helper generates the label after resolving the directory.
        let name = String::new();
        let spec = CreateSession {
            key: key.clone(),
            directory,
            provider: provider.clone(),
            name,
            resume,
            resume_pick: pick,
        };
        self.creating = self.send(
            d,
            if yolo {
                Operation::CreateYolo(spec)
            } else {
                Operation::Create(spec)
            },
        );
        self.set_notice_as(
            NoticeKind::Warning,
            if self.creating {
                format!("Creating {provider} on {}…", identity(&self.devices[d]))
            } else {
                "Request queue busy · retry shortly".into()
            },
        );
    }
}
