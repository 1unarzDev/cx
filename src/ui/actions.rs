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
                (matches!(a, Action::Files | Action::Add) || self.action_enabled(*a))
                    && label.to_lowercase().contains(&self.text.to_lowercase())
            })
            .collect()
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
            Action::New => !self.creating,
            Action::Command => self.command_context().is_some(),
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
            Action::Observe | Action::PinSession => {
                self.view == View::Work && self.selected_session().is_some()
            }
            Action::SendCtrlBackslash => self.last_session.is_some(),
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
            Action::Command => {
                self.yz.shell_quiet = false;
                // Spec 6.6: `:` is the palette in run mode.
                if let Some(target) = self.command_ready() {
                    self.command_target = Some(target);
                    self.open_palette(RUN_PREFIX);
                }
            }
            Action::New => {
                if self.creating {
                    return;
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
                    // yazi keeps the cursor's row, not the file: hidden folders sort
                    // first, so from the top `.` lands on the first of them.
                    b.show_hidden = !b.show_hidden;
                    b.visual_anchor = None;
                    b.selected = b.selected.min(browser_entries(b).len().saturating_sub(1));
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
                self.rename_cursor = self.text.len();
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
            Action::Files => Some(ChooseDevice::Files),
            Action::New => Some(ChooseDevice::New),
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

/// What a palette row does when chosen.
#[derive(Clone, PartialEq, Debug)]
pub(super) enum PaletteTarget {
    Action(Action),
    Cmd(keymap::Cmd),
    /// A device-rail value (0 = All devices).
    Device(usize),
    /// Device index and runtime key.
    Session(usize, String),
    /// Device index and folder path.
    Folder(usize, String),
    /// A LAN neighbour an enrolled device observed: connect (enroll) it.
    Neighbor(usize, String),
    /// Run the typed command (palette run mode).
    Run,
}
#[derive(Clone, Debug)]
pub(super) struct PaletteItem {
    pub section: &'static str,
    pub label: String,
    pub detail: String,
    pub key: String,
    pub target: PaletteTarget,
    /// Stable identity for MRU ranking.
    pub id: String,
}
/// Section order on equal scores (spec 6.6: switcher first, then actions).
const PALETTE_SECTIONS: [&str; 6] = [
    "Sessions",
    "Actions",
    "Go to",
    "Devices",
    "Folders",
    "Neighbors",
];
const PALETTE_MRU: usize = 32;

impl App {
    /// Every palette source: sessions on all devices, valid actions, goto,
    /// devices and recently used folders. Unranked.
    pub(super) fn palette_sources(&self) -> Vec<PaletteItem> {
        let mut items = Vec::new();
        let mut folders: Vec<(usize, String)> = Vec::new();
        for (d, cache) in self.work.iter().enumerate() {
            for s in &cache.sessions {
                let key = workspaces::runtime_key(s);
                items.push(PaletteItem {
                    section: "Sessions",
                    label: session_preview::label(s),
                    detail: format!(
                        "{} · {}",
                        safe_label(&self.devices[d].name),
                        session_folder_label(&s.directory)
                    ),
                    key: "Enter".into(),
                    id: format!("session:{}:{key}", self.devices[d].id),
                    target: PaletteTarget::Session(d, key),
                });
                if !s.directory.is_empty() && s.container.is_none() {
                    folders.push((d, s.directory.clone()));
                }
            }
        }
        // `:` is the run-command path; the palette does not list it as an
        // action (user direction recorded in STATUS: "Removed Command from Ctrl+P/Actions").
        for (action, label) in self
            .palette()
            .into_iter()
            .filter(|(a, _)| *a != Action::Command)
        {
            let (text, suffix) = match label.rsplit_once(" · ") {
                Some((text, key)) if key.chars().count() <= 6 => (text, key),
                _ => (label, ""),
            };
            let bound = keymap::BINDINGS
                .iter()
                .any(|b| b.cmd == keymap::Cmd::Do(action));
            items.push(PaletteItem {
                section: "Actions",
                label: text.to_owned(),
                detail: String::new(),
                key: if bound {
                    keymap::key_for(self, keymap::Cmd::Do(action)).unwrap_or(suffix)
                } else {
                    suffix
                }
                .to_owned(),
                id: format!("action:{action:?}"),
                target: PaletteTarget::Action(action),
            });
        }
        for (label, cmd, key) in [
            ("Hub", keymap::Cmd::View("hub"), "`"),
            ("Files", keymap::Cmd::Goto(View::Files), ""),
            ("Keys / Secrets", keymap::Cmd::View("secrets"), "K"),
        ] {
            items.push(PaletteItem {
                section: "Go to",
                label: label.into(),
                detail: String::new(),
                key: key.into(),
                id: format!("goto:{label}"),
                target: PaletteTarget::Cmd(cmd),
            });
        }
        for value in self.device_rows().into_iter().filter(|v| *v > 0) {
            let (label, detail, id) = if value == 0 {
                (
                    "All devices".to_owned(),
                    String::new(),
                    "device:*".to_owned(),
                )
            } else {
                let device = &self.devices[value - 1];
                (
                    safe_label(&device.name),
                    identity(device),
                    format!("device:{}", device.id),
                )
            };
            items.push(PaletteItem {
                section: "Devices",
                label,
                detail,
                key: String::new(),
                id,
                target: PaletteTarget::Device(value),
            });
        }
        let mut locations: Vec<_> = self
            .file_locations
            .iter()
            .map(|(d, p)| (*d, p.clone()))
            .collect();
        locations.sort();
        if let Some(b) = &self.browser {
            if b.container.is_none() {
                locations.insert(0, (b.device, b.path.clone()));
            }
        }
        for (d, path) in locations.into_iter().chain(folders) {
            if d >= self.devices.len()
                || items
                    .iter()
                    .any(|i| i.target == PaletteTarget::Folder(d, path.clone()))
            {
                continue;
            }
            items.push(PaletteItem {
                section: "Folders",
                label: session_folder_label(&path),
                detail: safe_label(&self.devices[d].name),
                key: String::new(),
                id: format!("folder:{}:{path}", self.devices[d].id),
                target: PaletteTarget::Folder(d, path),
            });
        }
        // LAN neighbours the devices observed (Refresh on the hub observes): the
        // palette is where an unenrolled one is found and connected.
        for c in self.network_candidates() {
            let (Some(d), Some(address)) = (c["_device"].as_u64(), c["address"].as_str()) else {
                continue;
            };
            let d = d as usize;
            if d >= self.devices.len() {
                continue;
            }
            items.push(PaletteItem {
                section: "Neighbors",
                label: safe_label(c["hostname"].as_str().unwrap_or(address)),
                detail: format!(
                    "{address} via {} · {}",
                    safe_label(&self.devices[d].name),
                    neighbor_detail(&c).replace('\n', " · ")
                ),
                key: String::new(),
                id: format!("neighbor:{}:{address}", self.devices[d].id),
                target: PaletteTarget::Neighbor(d, address.into()),
            });
        }
        items
    }
    /// Ranked palette rows for the current query: fuzzy score plus recency,
    /// grouped by section (sections ordered by their best row).
    pub(super) fn palette_view(&self) -> Vec<PaletteItem> {
        if self.palette_run_mode() {
            return vec![self.palette_run_item()];
        }
        let query = self.text.trim();
        let mut scored: Vec<(i64, usize, PaletteItem)> = self
            .palette_sources()
            .into_iter()
            .enumerate()
            .filter_map(|(order, item)| {
                // Fuzzy on the label; the detail (device, path) only counts as a
                // contiguous match, ranked below label hits, so scattered path
                // letters do not pull in unrelated rows.
                let score = fuzzy_score(query, &item.label).or_else(|| {
                    let needle = query.to_lowercase();
                    item.detail
                        .to_lowercase()
                        .contains(&needle)
                        .then(|| fuzzy_score(query, &item.detail).unwrap_or(0) - 100)
                })?;
                let recency = self
                    .palette_mru
                    .iter()
                    .position(|id| *id == item.id)
                    .map_or(0, |at| (PALETTE_MRU - at.min(PALETTE_MRU)) as i64 * 2);
                Some((score + recency, order, item))
            })
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        let section_rank = |section: &str| {
            PALETTE_SECTIONS
                .iter()
                .position(|s| *s == section)
                .unwrap_or(PALETTE_SECTIONS.len())
        };
        let mut best: Vec<(&'static str, i64)> = Vec::new();
        for (score, _, item) in &scored {
            if !best.iter().any(|(s, _)| *s == item.section) {
                best.push((item.section, *score));
            }
        }
        best.sort_by(|a, b| {
            b.1.cmp(&a.1)
                .then(section_rank(a.0).cmp(&section_rank(b.0)))
        });
        let mut out = Vec::with_capacity(scored.len());
        for (section, _) in best {
            out.extend(
                scored
                    .iter()
                    .filter(|(_, _, item)| item.section == section)
                    .map(|(_, _, item)| item.clone()),
            );
        }
        out
    }
    /// Run the highlighted palette row and remember it for ranking.
    pub(super) fn run_palette_selected(&mut self) {
        let Some(item) = self.palette_view().into_iter().nth(self.palette_selected) else {
            return;
        };
        if item.target == PaletteTarget::Run {
            return self.run_palette_command();
        }
        self.command_target = None;
        self.palette_mru.retain(|id| *id != item.id);
        self.palette_mru.push_front(item.id.clone());
        self.palette_mru.truncate(PALETTE_MRU);
        match item.target {
            PaletteTarget::Action(action) => self.execute_palette(action),
            PaletteTarget::Cmd(cmd) => {
                self.input = None;
                self.text.clear();
                self.run_cmd(cmd);
            }
            PaletteTarget::Device(value) => {
                self.input = None;
                self.text.clear();
                if let Some(d) = value.checked_sub(1) {
                    self.open_device(d);
                }
            }
            PaletteTarget::Session(device, key) => {
                self.input = None;
                self.text.clear();
                // Enter on a session attaches directly (user decision), from its device.
                self.open_device(device);
                self.reconcile_session_selection(Some((device, key)));
                if let Some((d, s)) = self.selected_session() {
                    self.open_session(d, s, false);
                }
            }
            PaletteTarget::Folder(device, path) => {
                self.input = None;
                self.text.clear();
                self.other_browser = None;
                self.destination_active = false;
                self.open_host_browser(device, path);
                self.focus = Focus::Workspace;
            }
            PaletteTarget::Neighbor(d, address) => {
                self.input = None;
                self.text.clear();
                if let Some(c) = self.network_candidates().into_iter().find(|c| {
                    c["_device"].as_u64() == Some(d as u64) && c["address"] == address.as_str()
                }) {
                    self.connect_neighbor(d, c);
                }
            }
            PaletteTarget::Run => {}
        }
    }
    /// Tab in the palette: jump to the first row of the next section.
    pub(super) fn palette_next_section(&mut self, reverse: bool) {
        let view = self.palette_view();
        let Some(current) = view.get(self.palette_selected).map(|i| i.section) else {
            return;
        };
        let starts: Vec<usize> = (0..view.len())
            .filter(|&i| i == 0 || view[i].section != view[i - 1].section)
            .collect();
        let at = starts
            .iter()
            .rposition(|&s| s <= self.palette_selected)
            .unwrap_or(0);
        let next = if reverse {
            if view[starts[at]].section == current && starts[at] < self.palette_selected {
                starts[at]
            } else {
                starts[(at + starts.len() - 1) % starts.len()]
            }
        } else {
            starts[(at + 1) % starts.len()]
        };
        self.palette_selected = next;
    }
}

/// The palette query prefix that turns the palette into the run-command field.
pub(super) const RUN_PREFIX: &str = "run: ";

impl App {
    /// Open the palette with a prefilled query (empty for Ctrl+P).
    pub(super) fn open_palette(&mut self, query: &str) {
        self.input = Some(Input::Palette);
        self.text = query.to_owned();
        self.rename_cursor = self.text.len();
        self.palette_selected = 0;
    }
    /// The palette is in run mode (`:` or a typed `run:` prefix).
    pub(super) fn palette_run_mode(&self) -> bool {
        self.input == Some(Input::Palette) && self.text.starts_with("run:")
    }
    /// The command typed after the `run:` prefix.
    pub(super) fn palette_run_command(&self) -> &str {
        self.text
            .strip_prefix("run:")
            .map(|rest| rest.strip_prefix(' ').unwrap_or(rest))
            .unwrap_or("")
    }
    /// Where a command would run now, after the remote capability check.
    /// Sets a notice and returns None when it cannot run yet.
    pub(super) fn command_ready(&mut self) -> Option<(usize, String)> {
        let Some((d, path)) = self.command_context() else {
            self.set_notice_as(
                NoticeKind::Warning,
                "Select a device or open its files first".into(),
            );
            return None;
        };
        if self.devices[d].target.is_some()
            && self
                .providers
                .get(&d)
                .is_none_or(|(_, checked)| transport::now().saturating_sub(*checked) >= 60)
        {
            self.check_providers(d);
            self.set_notice(
                "Checking command support on this device · press : again shortly".into(),
            );
            return None;
        }
        let supported = self.devices[d].target.is_none()
            || self.providers.get(&d).is_some_and(|(caps, checked)| {
                transport::now().saturating_sub(*checked) < 60
                    && caps.iter().any(|c| c == "native-command-v1")
            });
        if !supported {
            self.check_providers(d);
            self.set_notice_as(
                NoticeKind::Warning,
                "Run command needs a current cx helper on this device · update it and retry".into(),
            );
            return None;
        }
        Some((d, path))
    }
    /// Enter on the run row: start the typed command in the floating panel.
    fn run_palette_command(&mut self) {
        let command = self.palette_run_command().to_owned();
        if command.trim().is_empty() {
            return;
        }
        let Some((d, directory)) = self.command_target.clone().or_else(|| self.command_ready())
        else {
            return;
        };
        self.command_target = None;
        self.input = None;
        self.text.clear();
        self.rename_cursor = 0;
        self.pending_command_panel = Some((d, RunCommand { directory, command }));
    }
    /// The single row the palette shows in run mode.
    fn palette_run_item(&self) -> PaletteItem {
        let command = self.palette_run_command();
        let detail = self
            .command_target
            .clone()
            .or_else(|| self.command_context())
            .map(|(d, path)| {
                format!(
                    "{} · {}",
                    identity(&self.devices[d]),
                    safe_label(
                        self.browser
                            .as_ref()
                            .filter(|b| b.device == d && b.path == path)
                            .map(|b| b.display_path.as_str())
                            .unwrap_or(&path)
                    )
                )
            })
            .unwrap_or_else(|| "select a device or open its files first".into());
        PaletteItem {
            section: "Run command",
            label: if command.trim().is_empty() {
                "type a command, Enter runs it in a panel".into()
            } else {
                command.to_owned()
            },
            detail,
            key: "Enter".into(),
            id: "run".into(),
            target: PaletteTarget::Run,
        }
    }
}
