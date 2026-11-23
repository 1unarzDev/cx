//! Session launch flows: device/provider choice, permission prompts, shells and adoption.
use super::*;

/// A YOLO launch that may wait for a remote device's capabilities (`yolo_ready`).
#[derive(Clone)]
pub(super) struct PendingLaunch {
    pub(super) device: usize,
    pub(super) directory: String,
    pub(super) provider: String,
    pub(super) resume: bool,
    pub(super) pick: bool,
}
/// A launch waiting on its device's answer: when it was asked for, and the screen
/// (view, device) it was asked from.
pub(super) type Waiting = (PendingLaunch, Instant, (View, usize));

impl App {
    /// Record sessions that ended on device `d` (newest first, at most 32 in all).
    pub(super) fn note_ended(&mut self, d: usize, gone: Vec<Session>) {
        let id = self.devices[d].id.clone();
        let now = transport::now();
        for s in gone {
            let key = workspaces::runtime_key(&s);
            self.ended
                .retain(|(x, e, _)| *x != id || workspaces::runtime_key(e) != key);
            self.ended.insert(0, (id.clone(), s, now));
        }
        self.ended.truncate(32);
    }
    /// What a device with nothing running offers besides new sessions (the launch
    /// panel, in key order from `a`): the newest session that ended there, then recent
    /// folders (other ended sessions', pinned ones', the last Files folder; at most five).
    pub(super) fn launch_items(&self, d: usize) -> (Option<(&Session, u64)>, Vec<String>) {
        let id = &self.devices[d].id;
        let ended: Vec<(&Session, u64)> = self
            .ended
            .iter()
            .filter(|(x, s, _)| x == id && s.container.is_none())
            .map(|(_, s, at)| (s, *at))
            .collect();
        let resume = ended.first().copied();
        let pinned = self
            .pins
            .iter()
            .filter(|p| workspaces::device_matches(&self.devices[d], &p.device))
            .map(|p| &p.session)
            .filter(|s| s.container.is_none());
        let mut dirs: Vec<String> = Vec::new();
        let candidates = ended
            .iter()
            .map(|(s, _)| *s)
            .chain(pinned)
            .map(|s| s.directory.clone())
            .chain(self.file_locations.get(&d).cloned());
        for dir in candidates {
            let own = resume.is_some_and(|(s, _)| s.directory == dir);
            if !dir.is_empty() && !own && !dirs.contains(&dir) {
                dirs.push(dir);
            }
        }
        dirs.truncate(5);
        (resume, dirs)
    }
    /// A launch panel key (`a s d ...` while nothing runs on `d`): resume the session
    /// that ended (the agent's latest conversation in its folder, or a shell there), or
    /// open the launch chooser in a recent folder. False when the key names nothing.
    pub(super) fn launch_item(&mut self, d: usize, c: char) -> bool {
        let Some(i) = super::views::mission::SLOTS.iter().position(|k| *k == c) else {
            return false;
        };
        let (resume, dirs) = self.launch_items(d);
        let resume = resume.map(|(s, _)| (s.provider.clone(), s.directory.clone()));
        let skip = usize::from(resume.is_some());
        match (i, resume) {
            (0, Some((provider, dir))) => {
                let agent = crate::model::agent(&provider).is_some();
                let provider = if agent { provider.as_str() } else { "shell" };
                self.quick_launch(d, dir, provider, agent);
            }
            _ => match dirs.get(i.wrapping_sub(skip)) {
                Some(dir) => self.open_session_chooser(d, dir.clone()),
                None => return false,
            },
        }
        true
    }
    /// From the device's last Info, however old: the chooser never waits for a recheck
    /// (opening it starts one). A launch the device can no longer run says why there.
    pub(super) fn provider_choices(&self, device: usize) -> Vec<&'static str> {
        let mut choices = vec!["shell"];
        if let Some((available, _)) = self.providers.get(&device) {
            for agent in crate::model::AGENTS {
                if available.iter().any(|p| p == agent.id) {
                    choices.push(agent.id);
                }
            }
            if available.iter().any(|p| p == "containers-v1") {
                choices.push("container");
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
            match self.adoption_unavailable.get(&device) {
                // (Yama scope 3: no relay can help.)
                Some(why) if !why.contains("install") => {
                    self.set_notice_as(NoticeKind::Warning, format!("Cannot move: {why}"))
                }
                // Set the relay up from inside CX rather than asking the user to run a
                // CLI command; the move continues once it is installed.
                _ => self.pending_ptrace = Some((device, session)),
            }
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
        if self.view == View::Containers {
            if let Some((d, c)) = self.selected_container() {
                if (c.devcontainer || c.allowed) && c.state == "running" {
                    self.open_container_chooser(d, c.scope());
                } else {
                    self.open_container_actions();
                }
            }
            return;
        }
        if self.view == View::Files {
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
        let location = if self.view == View::Files {
            self.browser.as_ref().map(|b| (b.device, b.path.clone()))
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
    pub(super) fn choose_device(&mut self, purpose: ChooseDevice) {
        if self.device > 0 && purpose != ChooseDevice::Destination {
            self.chosen_device(self.device - 1, purpose);
        } else {
            self.dialog = Some(Dialog::Device(purpose));
            self.dialog_selected = self.actual_device().unwrap_or(0);
        }
    }
    pub(super) fn chosen_device(&mut self, d: usize, purpose: ChooseDevice) {
        let chooser = matches!(self.dialog.take(), Some(Dialog::Device(_)));
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
            ChooseDevice::New => {
                self.check_providers(d);
                self.work[d].loading = self.send(d, Operation::Sessions);
                if self.launch_provider.is_some() {
                    self.open_browser(d, "~".into());
                } else {
                    self.dialog_back = chooser
                        .then_some((Dialog::Device(ChooseDevice::New), self.dialog_selected));
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
            self.ask_permissions(d, directory, provider, false, false);
            return;
        }
        self.create_permission_session(d, directory, provider, false, false, false);
    }
    /// Where `c x n` start on device `d`: on this device the folder cx was started in,
    /// elsewhere the last folder Files showed there, else the device's home.
    pub(super) fn launch_dir(&self, d: usize) -> String {
        self.here
            .clone()
            .filter(|_| self.devices[d].target.is_none())
            .or_else(|| self.file_locations.get(&d).cloned())
            .unwrap_or_else(|| "~".into())
    }
    /// The one quick launch (the hub's and a device's `c x n C X`, Files' `!` keys, the
    /// launch panel's resume): `provider` in `directory` on `d`, resumed or new, in the
    /// mode it was last started in on that device, attached once it starts. No dialog.
    pub(super) fn quick_launch(
        &mut self,
        d: usize,
        directory: String,
        provider: &str,
        resume: bool,
    ) {
        if self.launch_blocked(d) {
            return;
        }
        let yolo = provider != "shell" && self.last_yolo(d, provider);
        let then = PendingLaunch {
            device: d,
            directory,
            provider: provider.into(),
            resume,
            pick: false,
        };
        if yolo && !self.yolo_ready(then.clone()) {
            return;
        }
        self.launch_provider = None;
        self.create_permission_session(d, then.directory, then.provider, yolo, resume, false);
    }
    /// Nothing starts while a session is being created, or on a device that failed; a
    /// notice says which. True when blocked.
    pub(super) fn launch_blocked(&mut self, d: usize) -> bool {
        if self.creating {
            self.set_notice("A session is already starting…".into());
        } else if let Some(e) = &self.work[d].error {
            let e = safe_text(e);
            self.set_notice_as(NoticeKind::Warning, format!("Not started: {e}"));
        } else {
            return false;
        }
        true
    }
    /// Default / YOLO for an agent launch, the last mode used for it there preselected.
    pub(super) fn ask_permissions(
        &mut self,
        d: usize,
        directory: String,
        provider: String,
        resume: bool,
        pick: bool,
    ) {
        self.dialog_selected = usize::from(self.last_yolo(d, &provider));
        self.dialog = Some(Dialog::Permissions(d, directory, provider, resume, pick));
    }
    /// Whether the last launch of `provider` on device `d` was YOLO. Read from the
    /// shared state each time, so another viewer's (or the CLI's) choice counts.
    pub(super) fn last_yolo(&self, d: usize, provider: &str) -> bool {
        store::launch_yolo()
            .unwrap_or_default()
            .contains(&(self.devices[d].id.clone(), provider.to_string()))
    }
    /// YOLO on a remote device needs a helper that knows it. True when it is known to.
    /// Otherwise a notice: the helper is old, or (its capabilities are stale or unread)
    /// cx asks the device and starts `then` once it answers (`resume_pending_launch`).
    pub(super) fn yolo_ready(&mut self, then: PendingLaunch) -> bool {
        let d = then.device;
        if self.devices[d].target.is_none() {
            return true;
        }
        match self
            .providers
            .get(&d)
            .filter(|(_, checked)| transport::now().saturating_sub(*checked) < 60)
        {
            Some((caps, _)) if caps.iter().any(|c| c == "session-yolo-v1") => return true,
            Some(_) => self.set_notice_as(
                NoticeKind::Warning,
                "YOLO needs an updated execution helper · update this device, then retry".into(),
            ),
            None => {
                self.check_providers(d);
                if self.provider_loading.contains(&d) {
                    let name = safe_label(&self.devices[d].name);
                    let waiting = (then, Instant::now(), (self.view, self.device));
                    let dropped = match self.pending_launch.replace(waiting) {
                        Some(_) => " · the earlier waiting launch is dropped",
                        None => "",
                    };
                    self.set_notice(format!(
                        "Checking {name} for YOLO · starts when it answers{dropped}"
                    ));
                } else {
                    self.set_notice_as(
                        NoticeKind::Warning,
                        "Not started: device unavailable".into(),
                    );
                }
            }
        }
        false
    }
    /// The device's capabilities arrived (`ok`) or failed: start the launch waiting on
    /// them, unless the user has since moved on (another device's screen, Files, an
    /// attached session) or it waited too long; a dropped launch always says so.
    pub(super) fn resume_pending_launch(&mut self, d: usize, ok: bool) {
        let Some((p, asked, at)) = self.pending_launch.take_if(|w| w.0.device == d) else {
            return;
        };
        let name = safe_label(&self.devices[d].name);
        if !ok || asked.elapsed() > Duration::from_secs(12) {
            self.set_notice_as(
                NoticeKind::Warning,
                format!("Not started: {name} did not answer"),
            );
        } else if at != (self.view, self.device) {
            self.set_notice_as(
                NoticeKind::Warning,
                format!("Not started on {name}: you moved on · press again"),
            );
        } else if self.creating {
            self.set_notice_as(
                NoticeKind::Warning,
                "Not started: another session was starting · press again".into(),
            );
        } else if self.yolo_ready(p.clone()) {
            self.create_permission_session(d, p.directory, p.provider, true, p.resume, p.pick);
        }
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
        if crate::model::agent(&provider).is_some() {
            if let Err(e) = store::set_launch_yolo(&self.devices[d].id, &provider, yolo) {
                let e = safe_text(&format!("{e:#}"));
                self.set_notice_as(NoticeKind::Warning, format!("Mode not remembered: {e}"));
            }
        }
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
