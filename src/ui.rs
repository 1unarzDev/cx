//! Native workspace. Remote work runs off the input/render thread; attachment owns the terminal.
use crate::{
    model::{CreateSession, Device, Operation, RunCommand, Session},
    sessions, store, transport,
};
use anyhow::{Context, Result};
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    execute,
    terminal::{self, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, BorderType, Borders, Cell, Clear, List, ListItem, Paragraph, Row, Table, TableState,
        Wrap,
    },
    Frame, Terminal,
};
use serde_json::Value;
use std::{
    collections::{BTreeSet, HashMap, VecDeque},
    io,
    sync::mpsc,
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
enum View {
    Work,
    Files,
    Network,
}
#[derive(Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
enum Focus {
    Devices,
    Actions,
    Workspace,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Input {
    Search,
    Palette,
    Command,
    Mkdir,
    Filter,
    Rename,
    Add,
}
#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct Entry {
    name: String,
    path: String,
    kind: String,
    size: u64,
    #[serde(default)]
    identity: Option<String>,
    #[serde(default)]
    hidden: bool,
    #[serde(default)]
    rename_name: Option<String>,
}
#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct Clipboard {
    #[serde(default)]
    id: String,
    device: usize,
    entries: Vec<Entry>,
    cut: bool,
    #[serde(default)]
    source_label: String,
}
#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct Browser {
    device: usize,
    path: String,
    display_path: String,
    parent: Option<String>,
    entries: Vec<Entry>,
    selected: usize,
    search: String,
    #[serde(default)]
    filter: String,
    #[serde(default)]
    show_hidden: bool,
    #[serde(default)]
    marked: BTreeSet<String>,
    #[serde(skip)]
    visual_anchor: Option<usize>,
    #[serde(skip)]
    visual_base: BTreeSet<String>,
    #[serde(skip)]
    loading: bool,
    #[serde(skip)]
    preview: Option<String>,
    preview_scroll: u16,
    #[serde(default)]
    restore_selection: Option<String>,
}
impl Browser {
    fn new(device: usize, path: String) -> Self {
        Self {
            device,
            display_path: path.clone(),
            path,
            parent: None,
            entries: vec![],
            selected: 0,
            search: String::new(),
            filter: String::new(),
            show_hidden: false,
            marked: BTreeSet::new(),
            visual_anchor: None,
            visual_base: BTreeSet::new(),
            loading: false,
            preview: None,
            preview_scroll: 0,
            restore_selection: None,
        }
    }
}
struct Cached {
    sessions: Vec<Session>,
    loading: bool,
    error: Option<String>,
    fetched: u64,
}
struct Task {
    device: usize,
    execution: Device,
    op: Operation,
    generation: u64,
}
struct Reply {
    device: usize,
    op: Operation,
    generation: u64,
    result: Result<Value>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    Add,
    Files,
    New,
    Destination,
    TransferTo,
    Conflict,
    Jobs,
    Command,
    Observe,
    Refresh,
    Network,
    Work,
    Copy,
    Cut,
    Rename,
    Delete,
    Hidden,
    Filter,
    Select,
    Visual,
    Paste,
    Mkdir,
    Help,
    Update,
    Quit,
}
const ACTIONS: &[(Action, &str)] = &[
    (Action::Add, "Add device · existing SSH alias / user@host"),
    (
        Action::Files,
        "Files · selected session directory / device home",
    ),
    (
        Action::New,
        "New session · current folder / choose workspace",
    ),
    (
        Action::Destination,
        "Destination · paste clipboard on another device",
    ),
    (
        Action::TransferTo,
        "Transfer to… · choose device and folder · t",
    ),
    (
        Action::Conflict,
        "Existing files · skip / overwrite / rename",
    ),
    (Action::Jobs, "Transfers · progress / cancel / retry"),
    (Action::Command, "Run command here · :"),
    (
        Action::Observe,
        "Watch session · read-only terminal, no input",
    ),
    (Action::Copy, "Copy selected files · c / y"),
    (Action::Cut, "Cut selected files · x"),
    (Action::Rename, "Rename hovered file · r"),
    (
        Action::Delete,
        "Delete selected files · confirm permanently · d",
    ),
    (Action::Hidden, "Show / hide hidden files · ."),
    (Action::Filter, "Filter filenames · f"),
    (Action::Select, "Select file and move down · Space"),
    (Action::Visual, "Visual range selection · v"),
    (Action::Paste, "Paste here · copy into this directory"),
    (Action::Mkdir, "Create directory here"),
    (Action::Network, "Network"),
    (Action::Work, "Sessions"),
    (Action::Refresh, "Refresh"),
    (Action::Help, "Keyboard help"),
    (Action::Update, "Update cx · check verified releases"),
    (Action::Quit, "Quit workspace"),
];
#[derive(Clone, Copy, PartialEq, Eq)]
enum ChooseDevice {
    New,
    Files,
    Destination,
}
#[derive(Clone)]
enum Dialog {
    Device(ChooseDevice),
    Provider(usize, Option<String>),
    Matching(usize, String, String, Session),
    Jobs,
    Delete(usize, Vec<Entry>),
    PendingExit(usize),
}

struct App {
    devices: Vec<Device>,
    device: usize,
    focus: Focus,
    view: View,
    work: Vec<Cached>,
    selected: usize,
    side_selected: usize,
    search: String,
    input: Option<Input>,
    text: String,
    palette_selected: usize,
    motion_count: Option<usize>,
    pending_g: bool,
    help: bool,
    help_scroll: u16,
    notice: String,
    browser: Option<Browser>,
    other_browser: Option<Browser>,
    destination_active: bool,
    conflict: usize,
    dialog: Option<Dialog>,
    dialog_selected: usize,
    dialog_detail_focus: bool,
    dialog_scroll: u16,
    launch_provider: Option<String>,
    submitted: HashMap<String, crate::model::TransferSpec>,
    submitted_clipboards: HashMap<String, String>,
    watched_jobs: BTreeSet<String>,
    browser_cache: HashMap<(usize, String), Browser>,
    file_locations: HashMap<usize, String>,
    generation: u64,
    creating: bool,
    providers: HashMap<usize, (Vec<String>, u64)>,
    provider_loading: std::collections::HashSet<usize>,
    network: HashMap<usize, Value>,
    network_loading: bool,
    clipboard: Option<Clipboard>,
    rename_target: Option<(usize, Entry)>,
    rename_cursor: usize,
    file_queue: VecDeque<(usize, Operation)>,
    file_busy: bool,
    file_errors: Vec<String>,
    transfer_drawer: bool,
    jobs: HashMap<usize, Value>,
    pending_attach: Option<(usize, Session, bool)>,
    command_target: Option<(usize, String)>,
    pending_command: Option<(Device, RunCommand)>,
    pending_add: Option<String>,
    quit: bool,
    force_update: bool,
    pending_requests: std::cell::Cell<usize>,
    panels: std::cell::RefCell<Vec<(Focus, bool, Rect)>>,
    tx: mpsc::SyncSender<Task>,
}
impl App {
    fn new(devices: Vec<Device>, tx: mpsc::SyncSender<Task>) -> Self {
        let work = devices
            .iter()
            .map(|_| Cached {
                sessions: vec![],
                loading: false,
                error: None,
                fetched: 0,
            })
            .collect();
        Self {
            devices,
            device: 0,
            focus: Focus::Workspace,
            view: View::Work,
            work,
            selected: 0,
            side_selected: 0,
            search: String::new(),
            input: None,
            text: String::new(),
            palette_selected: 0,
            motion_count: None,
            pending_g: false,
            help: false,
            help_scroll: 0,
            notice: "Ctrl+P actions · ? help".into(),
            browser: None,
            other_browser: None,
            destination_active: false,
            conflict: 2,
            dialog: None,
            dialog_selected: 0,
            dialog_detail_focus: false,
            dialog_scroll: 0,
            launch_provider: None,
            submitted: HashMap::new(),
            submitted_clipboards: HashMap::new(),
            watched_jobs: BTreeSet::new(),
            browser_cache: HashMap::new(),
            file_locations: HashMap::new(),
            generation: 0,
            creating: false,
            providers: HashMap::new(),
            provider_loading: std::collections::HashSet::new(),
            network: HashMap::new(),
            network_loading: false,
            clipboard: None,
            rename_target: None,
            rename_cursor: 0,
            file_queue: VecDeque::new(),
            file_busy: false,
            file_errors: Vec::new(),
            transfer_drawer: false,
            jobs: HashMap::new(),
            pending_attach: None,
            command_target: None,
            pending_command: None,
            pending_add: None,
            quit: false,
            force_update: false,
            pending_requests: std::cell::Cell::new(0),
            panels: std::cell::RefCell::new(Vec::new()),
            tx,
        }
    }
    fn focus_label(&self) -> &'static str {
        if self.help {
            return "Help";
        }
        if let Some(dialog) = &self.dialog {
            return match dialog {
                Dialog::Device(_) => "Device picker",
                Dialog::Provider(..) => "Provider",
                Dialog::Matching(..) => "Session choice",
                Dialog::Jobs => {
                    if self.dialog_detail_focus {
                        "Transfer details"
                    } else {
                        "Transfers"
                    }
                }
                Dialog::Delete(..) => "Confirm delete",
                Dialog::PendingExit(_) => "Pending actions",
            };
        }
        if let Some(input) = self.input {
            return match input {
                Input::Search => "Search",
                Input::Palette => "Actions",
                Input::Command => "Run command",
                Input::Add => "Add device",
                Input::Mkdir => "New folder",
                Input::Rename => "Rename",
                Input::Filter => "Filter",
            };
        }
        match self.focus {
            Focus::Devices => "Devices",
            Focus::Actions => "Actions",
            Focus::Workspace => match self.view {
                View::Work => "Sessions",
                View::Network => "Network",
                View::Files if self.other_browser.is_some() => {
                    if self.destination_active {
                        "Destination"
                    } else {
                        "Source"
                    }
                }
                View::Files => "Files",
            },
        }
    }
    fn send(&self, device: usize, op: Operation) -> bool {
        let sent = self
            .tx
            .try_send(Task {
                device,
                execution: self.devices[device].clone(),
                op,
                generation: self.generation,
            })
            .is_ok();
        if sent {
            self.pending_requests.set(self.pending_requests.get() + 1);
        }
        sent
    }
    fn refresh_work(&mut self) {
        for index in 0..self.devices.len() {
            if (self.device == 0 || self.device == index + 1) && !self.work[index].loading {
                self.work[index].loading = self.send(index, Operation::Sessions);
                if !self.work[index].loading {
                    self.notice = "Refresh queue busy · retry shortly".into();
                }
            }
        }
        for index in 0..self.devices.len() {
            self.check_providers(index);
        }
    }
    fn actual_device(&self) -> Option<usize> {
        if self.device > 0 {
            Some(self.device - 1)
        } else {
            self.selected_session()
                .map(|(index, _)| index)
                .or_else(|| self.devices.iter().position(|d| d.target.is_none()))
        }
    }
    fn session_rows(&self) -> Vec<(usize, &Session)> {
        let mut rows: Vec<_> = self
            .work
            .iter()
            .enumerate()
            .filter(|(i, _)| self.device == 0 || self.device == i + 1)
            .flat_map(|(i, c)| c.sessions.iter().map(move |s| (i, s)))
            .filter_map(|(i, s)| {
                fuzzy_score(
                    &self.search,
                    &format!(
                        "{} {} {} {} {}",
                        s.name, s.directory, s.host, s.account, s.provider
                    ),
                )
                .map(|score| (score, i, s))
            })
            .collect();
        rows.sort_by(|(a, i, x), (b, j, y)| {
            b.cmp(a).then_with(|| {
                (x.directory.as_str(), *i, x.name.as_str(), x.id.as_str()).cmp(&(
                    y.directory.as_str(),
                    *j,
                    y.name.as_str(),
                    y.id.as_str(),
                ))
            })
        });
        rows.into_iter().map(|(_, i, s)| (i, s)).collect()
    }
    fn selected_session(&self) -> Option<(usize, Session)> {
        self.session_rows()
            .get(self.selected)
            .map(|(i, s)| (*i, (*s).clone()))
    }
    fn visible_entries(&self) -> Vec<Entry> {
        self.browser
            .as_ref()
            .map(browser_entries)
            .unwrap_or_default()
    }
    fn chosen_entries(&self) -> Vec<Entry> {
        let Some(b) = &self.browser else {
            return vec![];
        };
        if b.marked.is_empty() {
            browser_entries(b)
                .get(b.selected)
                .cloned()
                .into_iter()
                .collect()
        } else {
            // Marks are path identities, independent of hidden/filter state.
            b.entries
                .iter()
                .filter(|e| b.marked.contains(&e.path))
                .cloned()
                .collect()
        }
    }
    fn request_quit(&mut self) {
        self.help = false;
        self.dialog_detail_focus = false;
        if self.file_busy || !self.file_queue.is_empty() {
            self.dialog = Some(Dialog::PendingExit(self.file_queue.len()));
            self.dialog_selected = 0;
        } else {
            self.quit = true;
        }
    }
    fn queue_file_actions(&mut self, actions: Vec<(usize, Operation)>) {
        if !self.file_busy && self.file_queue.is_empty() {
            self.file_errors.clear();
        }
        for (_, op) in &actions {
            if let Operation::TransferRetry { key } = op {
                self.watched_jobs.insert(key.clone());
            }
        }
        self.file_queue.extend(actions);
        self.start_next_file_action();
    }
    fn start_next_file_action(&mut self) {
        if !self.file_busy {
            if let Some((d, op)) = self.file_queue.front().cloned() {
                if self.send(d, op) {
                    self.file_queue.pop_front();
                    self.file_busy = true;
                } else {
                    self.notice = "Request queue busy · file action remains pending".into();
                }
            }
        }
    }
    fn finish_visual(&mut self) {
        if let Some(b) = &mut self.browser {
            b.visual_anchor = None;
            b.visual_base.clear();
        }
    }
    fn live_search(&mut self) {
        if self.view == View::Files {
            if let Some(b) = &mut self.browser {
                b.search = self.text.clone();
                b.restore_selection = None;
                b.selected = 0;
            }
        } else {
            self.search = self.text.clone();
            self.selected = 0;
        }
    }
    fn command_context(&self) -> Option<(usize, String)> {
        if self.view == View::Files {
            return self.browser.as_ref().map(|b| (b.device, b.path.clone()));
        }
        if self.view == View::Work {
            if let Some((d, session)) = self.selected_session() {
                return Some((d, session.directory));
            }
        }
        self.actual_device().map(|d| (d, "~".into()))
    }
    fn palette(&self) -> Vec<(Action, &'static str)> {
        ACTIONS
            .iter()
            .copied()
            .filter(|(a, label)| {
                workspace_action(*a)
                    && self.action_enabled(*a)
                    && label.to_lowercase().contains(&self.text.to_lowercase())
            })
            .collect()
    }
    fn provider_choices(&self, device: usize) -> Vec<&'static str> {
        let mut choices = vec!["shell"];
        if let Some((available, checked)) = self.providers.get(&device) {
            if transport::now().saturating_sub(*checked) < 60 {
                for provider in ["claude", "codex"] {
                    if available.iter().any(|p| p == provider) {
                        choices.push(provider);
                    }
                }
            }
        }
        choices
    }
    fn check_providers(&mut self, device: usize) {
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
    fn action_enabled(&self, action: Action) -> bool {
        match action {
            Action::Work => self.view != View::Work,
            Action::New => !self.creating,
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
            Action::Observe => self.view == View::Work && self.selected_session().is_some(),
            _ => true,
        }
    }
    fn open_browser(&mut self, device: usize, path: String) {
        self.device = device + 1;
        let show_hidden = self
            .browser
            .as_ref()
            .or(self.other_browser.as_ref())
            .is_some_and(|b| b.show_hidden);
        self.finish_visual();
        self.generation += 1;
        if let Some(old) = self.browser.take() {
            self.file_locations.insert(old.device, old.path.clone());
            if self.browser_cache.len() >= 8 {
                if let Some(key) = self.browser_cache.keys().next().cloned() {
                    self.browser_cache.remove(&key);
                }
            }
            self.browser_cache
                .insert((old.device, old.path.clone()), old);
        }
        self.browser = Some(
            self.browser_cache
                .remove(&(device, path.clone()))
                .unwrap_or_else(|| Browser::new(device, path)),
        );
        if let Some(b) = &mut self.browser {
            b.show_hidden = show_hidden;
        }
        self.view = View::Files;
        self.focus = Focus::Workspace;
        self.refresh_browser();
        self.check_providers(device);
    }
    fn select_file_device(&mut self) {
        // Files always has one execution host; All devices returns to the local location.
        let device = self
            .device
            .checked_sub(1)
            .or_else(|| self.devices.iter().position(|d| d.target.is_none()));
        if let Some(device) = device {
            if self.browser.as_ref().is_none_or(|b| b.device != device) {
                let path = self
                    .file_locations
                    .get(&device)
                    .cloned()
                    .unwrap_or_else(|| "~".into());
                self.open_browser(device, path);
            }
        }
        self.focus = Focus::Devices;
    }
    fn refresh_browser(&mut self) {
        self.generation += 1;
        if let Some(b) = self.browser.as_mut() {
            if b.restore_selection.is_none() {
                b.restore_selection = browser_entries(b)
                    .get(b.selected)
                    .map(|entry| entry.path.clone());
            }
            b.loading = true;
            b.preview = None;
            let device = b.device;
            let path = b.path.clone();
            if !self.send(device, Operation::List { path }) {
                if let Some(b) = &mut self.browser {
                    b.loading = false;
                }
                self.notice = "Refresh queue busy · retry shortly".into();
            }
        }
    }
    fn refresh(&mut self) {
        for d in 0..self.devices.len() {
            self.providers.remove(&d);
            self.check_providers(d);
        }
        match self.view {
            View::Work => self.refresh_work(),
            View::Files => self.refresh_browser(),
            View::Network => {
                if let Some(d) = self.actual_device().filter(|_| !self.network_loading) {
                    self.network_loading = self.send(d, Operation::Network);
                }
            }
        }
    }
    fn execute(&mut self, action: Action) {
        self.input = None;
        self.text.clear();
        match action {
            Action::Update => self.force_update = true,
            Action::Quit => self.request_quit(),
            Action::Add => {
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
                        self.notice =
                            "Checking command support on this device · press : again shortly"
                                .into();
                        return;
                    }
                    let supported = self.devices[d].target.is_none()
                        || self.providers.get(&d).is_some_and(|(caps, checked)| {
                            transport::now().saturating_sub(*checked) < 60
                                && caps.iter().any(|c| c == "native-command-v1")
                        });
                    if !supported {
                        self.check_providers(d);
                        self.notice = "Run command needs a current cx helper on this device · update it and retry".into();
                        return;
                    }
                    self.rename_cursor = 0;
                    self.command_target = Some((d, path));
                    self.text.clear();
                    self.input = Some(Input::Command);
                } else {
                    self.notice = "Select a device or open its files first".into();
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
                        self.check_providers(d);
                        self.dialog = Some(Dialog::Provider(d, Some(session.directory)));
                        self.dialog_selected = 0;
                        return;
                    }
                }
                self.choose_device(ChooseDevice::New);
            }
            Action::Files => {
                self.launch_provider = None;
                self.other_browser = None;
                self.destination_active = false;
                if let Some((d, session)) =
                    self.selected_session().filter(|_| self.view == View::Work)
                {
                    self.open_browser(d, session.directory);
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
                self.notice = format!("Existing files: {}", self.conflict_policy());
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
            Action::Observe => {
                if let Some((d, s)) = self.selected_session() {
                    self.pending_attach = Some((d, s, true));
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
                            source_label: identity(&self.devices[b.device]),
                        });
                        self.launch_provider = None;
                        self.notice = format!("{} {count} item{} · p pastes here · switch device then p to paste · t chooses a destination",
                            if action == Action::Cut { "Cut" } else { "Copied" }, if count == 1 { "" } else { "s" });
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
    fn choose_device(&mut self, purpose: ChooseDevice) {
        if self.device > 0 && purpose != ChooseDevice::Destination {
            self.chosen_device(self.device - 1, purpose);
        } else {
            self.dialog = Some(Dialog::Device(purpose));
            self.dialog_selected = self.actual_device().unwrap_or(0);
        }
    }
    fn chosen_device(&mut self, d: usize, purpose: ChooseDevice) {
        self.dialog = None;
        match purpose {
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
                self.open_browser(d, "~".into());
            }
            ChooseDevice::Destination => {
                self.device = d + 1;
                if !self.destination_active {
                    self.other_browser = self.browser.take();
                    self.destination_active = true;
                }
                self.open_browser(d, "~".into());
            }
        }
    }
    fn conflict_policy(&self) -> &'static str {
        ["skip", "overwrite", "rename"][self.conflict]
    }
    fn start_at(&mut self, d: usize, directory: String, provider: String) {
        if !self.provider_choices(d).contains(&provider.as_str()) {
            self.check_providers(d);
            self.notice = "Launch profile unavailable or not yet checked on this device".into();
            return;
        }
        if let Some(s) = self.work[d]
            .sessions
            .iter()
            .find(|s| s.directory == directory && s.provider == provider)
            .cloned()
        {
            self.dialog = Some(Dialog::Matching(d, directory, provider, s));
            self.dialog_selected = 0;
        } else {
            self.create_at(d, directory, provider);
        }
    }
    fn create_at(&mut self, d: usize, directory: String, provider: String) {
        let key = unique_key();
        let location = self
            .browser
            .as_ref()
            .filter(|b| b.device == d && b.path == directory)
            .map(|b| b.display_path.as_str())
            .unwrap_or(&directory);
        let folder = location
            .rsplit('/')
            .find(|s| !s.is_empty())
            .unwrap_or("root");
        let name = format!("{provider} · {}", safe_label(folder));
        self.creating = self.send(
            d,
            Operation::Create(CreateSession {
                key: key.clone(),
                directory,
                provider: provider.clone(),
                name,
            }),
        );
        self.notice = if self.creating {
            format!("Creating {provider} on {}…", identity(&self.devices[d]))
        } else {
            "Request queue busy · retry shortly".into()
        };
    }
    fn submit_transfer(&mut self) {
        let (Some(clip), Some(b)) = (self.clipboard.clone(), self.browser.as_ref()) else {
            return;
        };
        let Some(local) = self.devices.iter().position(|d| d.target.is_none()) else {
            return;
        };
        let destination = self.devices[b.device].clone();
        let destination_path = b.path.clone();
        let mut actions = Vec::new();
        for entry in &clip.entries {
            let spec = crate::model::TransferSpec {
                source: self.devices[clip.device].clone(),
                source_path: entry.path.clone(),
                destination: destination.clone(),
                destination_path: destination_path.clone(),
                conflict: self.conflict_policy().into(),
                key: unique_key(),
                cut: clip.cut,
                source_identity: if clip.cut {
                    entry.identity.clone()
                } else {
                    None
                },
            };
            if self.submitted.len() >= 256 {
                if let Some(key) = self.submitted.keys().next().cloned() {
                    self.submitted.remove(&key);
                }
            }
            self.submitted.insert(spec.key.clone(), spec.clone());
            if clip.cut {
                self.submitted_clipboards
                    .insert(spec.key.clone(), clip.id.clone());
            }
            actions.push((local, Operation::Transfer(spec)));
        }
        self.notice = format!(
            "{} {} items · {} → {} · existing: {}",
            if clip.cut { "Moving" } else { "Copying" },
            clip.entries.len(),
            identity(&self.devices[clip.device]),
            identity(&destination),
            self.conflict_policy()
        );
        self.queue_file_actions(actions);
        self.transfer_drawer = true;
    }
    fn job_rows(&self) -> Vec<(usize, Value)> {
        let mut unique = HashMap::new();
        for (d, value) in &self.jobs {
            if let Some(jobs) = value["jobs"].as_array() {
                for job in jobs {
                    let key = (
                        job["key"].as_str().unwrap_or("").to_string(),
                        job["source_host"].as_str().unwrap_or("").to_string(),
                        job["destination_host"].as_str().unwrap_or("").to_string(),
                    );
                    let row = unique.entry(key).or_insert((*d, job.clone()));
                    // Prefer the viewer's durable forwarding reference, which knows job ownership.
                    if (self.devices[*d].target.is_none() && self.devices[row.0].target.is_some())
                        || (self.devices[*d].target.is_none()
                            == self.devices[row.0].target.is_none()
                            && *d < row.0)
                    {
                        *row = (*d, job.clone());
                    }
                }
            }
        }
        let mut rows = unique.into_values().collect::<Vec<_>>();
        rows.sort_by(|(a, x), (b, y)| {
            y["updated"]
                .as_u64()
                .cmp(&x["updated"].as_u64())
                .then_with(|| a.cmp(b))
                .then_with(|| x["key"].as_str().cmp(&y["key"].as_str()))
        });
        rows
    }
    fn dialog_key(&mut self, key: KeyEvent, dialog: Dialog) {
        if matches!(dialog, Dialog::Delete(..))
            && !key
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
        if matches!(dialog, Dialog::Jobs | Dialog::Delete(..)) {
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
                    if self.dialog_detail_focus && matches!(dialog, Dialog::Delete(..)) =>
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
        let count = match &dialog {
            Dialog::Device(_) => self.devices.len(),
            Dialog::Provider(d, _) => self.provider_choices(*d).len(),
            Dialog::Matching(..) => 2,
            Dialog::Jobs => self.job_rows().len(),
            Dialog::Delete(..) => 2,
            Dialog::PendingExit(_) => 2,
        };
        if let Dialog::Provider(device, _) = dialog {
            if key.code == KeyCode::Enter && self.dialog_selected >= count {
                self.check_providers(device);
                self.notice = "Agent availability changed · checking before launch".into();
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
            KeyCode::Enter => match dialog {
                Dialog::Device(purpose) => {
                    if self.dialog_selected < self.devices.len() {
                        self.chosen_device(self.dialog_selected, purpose);
                    }
                }
                Dialog::Provider(d, path) => {
                    let choices = self.provider_choices(d);
                    let Some(provider) = choices.get(self.dialog_selected) else {
                        return;
                    };
                    let provider = (*provider).to_string();
                    self.dialog = None;
                    if let Some(directory) = path {
                        self.start_at(d, directory, provider);
                        return;
                    }
                    self.launch_provider = Some(provider);
                    self.open_browser(d, "~".into());
                    self.notice = "Browse to a folder · n starts here (Enter opens files)".into();
                }
                Dialog::Matching(d, path, provider, session) => {
                    self.dialog = None;
                    if self.dialog_selected == 0 {
                        self.pending_attach = Some((d, session, false));
                    } else {
                        self.create_at(d, path, provider);
                    }
                }
                Dialog::Jobs => {
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
                        self.notice = "Deleting confirmed items permanently…".into();
                    }
                }
                Dialog::PendingExit(_) => {
                    if self.dialog_selected == 1 {
                        self.quit = true;
                    }
                    self.dialog = None;
                }
            },
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
    fn apply(&mut self, reply: Reply) {
        self.pending_requests
            .set(self.pending_requests.get().saturating_sub(1));
        let file_action = matches!(
            reply.op,
            Operation::Remove { .. }
                | Operation::Rename { .. }
                | Operation::Transfer(_)
                | Operation::TransferRetry { .. }
        );
        if file_action {
            self.file_busy = false;
            self.start_next_file_action();
        }
        let listing_offset = match &reply.op {
            Operation::ListPage { offset, .. } => *offset,
            _ => 0,
        };
        if matches!(reply.op, Operation::Info) {
            self.provider_loading.remove(&reply.device);
        }
        if matches!(reply.op, Operation::Network) {
            self.network_loading = false;
        }
        let is_sessions = matches!(reply.op, Operation::Sessions);
        if is_sessions {
            self.work[reply.device].loading = false;
        }
        if matches!(reply.op, Operation::Create(_)) {
            self.creating = false;
        }
        let value = match reply.result {
            Ok(v) => v,
            Err(e) => {
                let message =
                    safe_text(&format!("{}: {e:#}", identity(&self.devices[reply.device])));
                if matches!(reply.op, Operation::Info) {
                    self.providers.remove(&reply.device);
                }
                if is_sessions {
                    self.work[reply.device].error = Some(message.clone());
                }
                if reply.generation == self.generation || file_action {
                    if file_action {
                        if self.file_errors.len() < 8 {
                            self.file_errors.push(message.clone());
                        }
                        self.notice = format!(
                            "{} file actions failed · {}",
                            self.file_errors.len(),
                            message
                        );
                    } else {
                        self.notice = message;
                    }
                }
                if reply.generation == self.generation {
                    if let Some(b) = &mut self.browser {
                        b.loading = false;
                    }
                    self.network_loading = false;
                    if matches!(reply.op, Operation::Network) {
                        if let Some(v) = self.network.get_mut(&reply.device) {
                            v["internet"]["state"] = serde_json::json!("unknown");
                            v["internet"]["stale"] = serde_json::json!(true);
                        }
                    }
                }
                return;
            }
        };
        if let Operation::Transfer(spec) = &reply.op {
            if spec.cut
                && matches!(
                    value["status"].as_str(),
                    Some("queued" | "running" | "complete" | "submitting")
                )
            {
                let clipboard_id = self.submitted_clipboards.remove(&spec.key);
                if let Some(c) = self
                    .clipboard
                    .as_mut()
                    .filter(|c| Some(&c.id) == clipboard_id.as_ref())
                {
                    c.entries.retain(|e| e.path != spec.source_path);
                    if c.entries.is_empty() {
                        self.clipboard = None;
                    }
                }
            }
        }
        // Session caches remain useful across view changes; navigation responses do not.
        match reply.op {
            Operation::Info => {
                let previous = if matches!(self.dialog, Some(Dialog::Provider(d, _)) if d == reply.device)
                {
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
                            .filter(|p| ["claude", "codex", "native-command-v1"].contains(p))
                            .map(str::to_owned)
                            .collect()
                    })
                    .unwrap_or_default();
                self.providers
                    .insert(reply.device, (available, transport::now()));
                if matches!(self.dialog, Some(Dialog::Provider(d, _)) if d == reply.device) {
                    self.dialog_selected = previous
                        .and_then(|provider| {
                            self.provider_choices(reply.device)
                                .iter()
                                .position(|p| *p == provider)
                        })
                        .unwrap_or(0);
                }
            }
            Operation::Sessions => {
                let selected_identity = self.selected_session().map(|(d, s)| (d, s.id));
                let data = if value.is_array() {
                    value.clone()
                } else {
                    value.get("sessions").cloned().unwrap_or(Value::Null)
                };
                match serde_json::from_value::<Vec<Session>>(data) {
                    Ok(s) => {
                        self.work[reply.device].sessions = s;
                        self.work[reply.device].error = None;
                        self.work[reply.device].fetched = transport::now();
                        if let Some((device, id)) = selected_identity {
                            if let Some(index) = self
                                .session_rows()
                                .iter()
                                .position(|(d, s)| *d == device && s.id == id)
                            {
                                self.selected = index;
                            } else {
                                self.selected = self
                                    .selected
                                    .min(self.session_rows().len().saturating_sub(1));
                            }
                        }
                    }
                    Err(_) => {
                        self.work[reply.device].error = Some("Invalid session metadata".into())
                    }
                }
            }
            Operation::Create(_) => match serde_json::from_value::<Session>(value) {
                Ok(s) => {
                    self.work[reply.device].sessions.push(s.clone());
                    self.pending_attach = Some((reply.device, s, false));
                }
                Err(_) => {
                    self.notice = "Creation response invalid · refresh before retrying".into()
                }
            },
            Operation::Jobs | Operation::TransferJobs => {
                let newly_finished = value["jobs"]
                    .as_array()
                    .map(|rows| {
                        rows.iter()
                            .filter(|j| {
                                j["key"].as_str().is_some_and(|k| {
                                    self.submitted.contains_key(k) || self.watched_jobs.contains(k)
                                }) && matches!(
                                    j["status"].as_str(),
                                    Some("complete" | "failed" | "cancelled")
                                ) && !self
                                    .jobs
                                    .get(&reply.device)
                                    .and_then(|v| v["jobs"].as_array())
                                    .is_some_and(|old| {
                                        old.iter().any(|x| {
                                            x["key"] == j["key"] && x["status"] == j["status"]
                                        })
                                    })
                            })
                            .cloned()
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                let refresh_files = self.view == View::Files
                    && self.browser.as_ref().is_some_and(|b| {
                        let host = identity(&self.devices[b.device]);
                        value["jobs"].as_array().is_some_and(|rows| {
                            rows.iter().any(|j| {
                                let completed = j["status"] == "complete";
                                let affects_location = j["destination_host"].as_str()
                                    == Some(host.as_str())
                                    || (j["operation"] == "move"
                                        && j["source_host"].as_str() == Some(host.as_str()));
                                let already_known = self
                                    .jobs
                                    .get(&reply.device)
                                    .and_then(|v| v["jobs"].as_array())
                                    .is_some_and(|old| {
                                        old.iter().any(|x| {
                                            x["key"] == j["key"] && x["status"] == "complete"
                                        })
                                    });
                                completed && affects_location && !already_known
                            })
                        })
                    });
                let selected = if matches!(self.dialog, Some(Dialog::Jobs)) {
                    self.job_rows()
                        .get(self.dialog_selected)
                        .map(|(owner, job)| (*owner, job["key"].clone()))
                } else {
                    None
                };
                self.jobs.insert(reply.device, value);
                if let Some(job) = newly_finished
                    .last()
                    .filter(|j| self.file_errors.is_empty() || j["status"] == "failed")
                {
                    self.notice = format!(
                        "{} {} · {}{}",
                        if job["operation"] == "move" {
                            "Move"
                        } else {
                            "Copy"
                        },
                        transfer_status(job),
                        transfer_name(job),
                        job["error"]
                            .as_str()
                            .map(|e| format!(" · {}", safe_label(e)))
                            .unwrap_or_default()
                    );
                }
                if let Some((owner, key)) = selected {
                    self.dialog_selected = self
                        .job_rows()
                        .iter()
                        .position(|(d, job)| *d == owner && job["key"] == key)
                        .unwrap_or(0);
                }
                if refresh_files {
                    self.browser_cache.clear();
                    self.refresh_browser();
                }
            }
            Operation::TransferCancel { .. } => {
                self.send(reply.device, Operation::TransferJobs);
                self.notice = "Cancellation requested · waiting for worker".into();
            }
            Operation::Transfer(_) | Operation::TransferRetry { .. } => {
                if self.file_errors.is_empty() {
                    self.notice = format!(
                        "Transfer {} · {}",
                        safe_label(value["status"].as_str().unwrap_or("queued")),
                        safe_label(value["route"].as_str().unwrap_or("worker host"))
                    );
                } else {
                    self.notice = format!(
                        "{} file actions failed · {}",
                        self.file_errors.len(),
                        self.file_errors.last().unwrap()
                    );
                }
                self.send(reply.device, Operation::TransferJobs);
            }
            Operation::Copy { .. } => {
                self.notice = format!(
                    "Copy {} on {}",
                    value
                        .get("status")
                        .and_then(Value::as_str)
                        .unwrap_or("status unknown"),
                    identity(&self.devices[reply.device])
                );
                self.send(reply.device, Operation::Jobs);
            }
            Operation::Network if reply.generation == self.generation => {
                self.network.insert(reply.device, value);
                self.network_loading = false;
            }
            Operation::List { .. } | Operation::ListPage { .. }
                if reply.generation == self.generation =>
            {
                if let Some(b) = self.browser.as_mut().filter(|b| b.device == reply.device) {
                    b.path = value
                        .get("path")
                        .and_then(Value::as_str)
                        .unwrap_or(&b.path)
                        .to_owned();
                    b.display_path = value
                        .get("display_path")
                        .and_then(Value::as_str)
                        .unwrap_or(&b.path)
                        .to_owned();
                    b.parent = value
                        .get("parent")
                        .and_then(Value::as_str)
                        .map(str::to_owned);
                    let entries: Vec<Entry> = value
                        .get("entries")
                        .and_then(Value::as_array)
                        .map(|entries| {
                            entries
                                .iter()
                                .filter_map(|e| {
                                    Some(Entry {
                                        name: e.get("name")?.as_str()?.into(),
                                        path: e.get("path")?.as_str()?.into(),
                                        kind: e.get("kind")?.as_str()?.into(),
                                        size: e.get("size").and_then(Value::as_u64).unwrap_or(0),
                                        identity: e
                                            .get("identity")
                                            .and_then(Value::as_str)
                                            .map(str::to_owned),
                                        hidden: e
                                            .get("hidden")
                                            .and_then(Value::as_bool)
                                            .unwrap_or_else(|| {
                                                e["name"]
                                                    .as_str()
                                                    .is_some_and(|s| s.starts_with('.'))
                                            }),
                                        rename_name: e
                                            .get("rename_name")
                                            .and_then(Value::as_str)
                                            .map(str::to_owned),
                                    })
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    if b.visual_anchor.take().is_some() {
                        b.visual_base.clear();
                        self.notice =
                            "Directory refreshed · range finished; selected files kept".into();
                    }
                    if b.restore_selection.is_none() && listing_offset > 0 {
                        b.restore_selection = browser_entries(b)
                            .get(b.selected)
                            .map(|entry| entry.path.clone());
                    }
                    if listing_offset == 0 {
                        b.entries = entries;
                    } else {
                        b.entries.extend(entries);
                    }
                    b.entries.sort_by(|a, b| {
                        (a.kind != "directory", a.name.as_str())
                            .cmp(&(b.kind != "directory", b.name.as_str()))
                    });
                    if let Some(path) = &b.restore_selection {
                        let visible = browser_entries(b);
                        if let Some(index) = visible.iter().position(|entry| &entry.path == path) {
                            b.selected = index;
                            if value["next_offset"].is_null() {
                                b.restore_selection = None;
                            }
                        } else {
                            b.selected = 0;
                            if value["next_offset"].is_null() {
                                b.restore_selection = None;
                            }
                        }
                    } else {
                        b.selected = b.selected.min(browser_entries(b).len().saturating_sub(1));
                    }
                    b.loading = false;
                    if value["next_offset"].is_null() {
                        let existing: BTreeSet<_> =
                            b.entries.iter().map(|e| e.path.clone()).collect();
                        b.marked.retain(|p| existing.contains(p));
                    }
                }
                if let Some(offset) = value["next_offset"].as_u64() {
                    if let Some(b) = &self.browser {
                        if b.entries.len() < 10000 {
                            self.send(
                                b.device,
                                Operation::ListPage {
                                    path: b.path.clone(),
                                    offset,
                                    limit: 1000,
                                },
                            );
                            if let Some(b) = &mut self.browser {
                                b.loading = true;
                            }
                            self.notice =
                                "Loading more directory entries · fuzzy search stays live".into();
                        } else {
                            self.notice="10,000 entries loaded · remaining entries require a narrower directory".into();
                        }
                    }
                }
            }
            Operation::Preview { .. } if reply.generation == self.generation => {
                if let Some(b) = &mut self.browser {
                    b.preview = Some(safe_text(
                        value
                            .get("text")
                            .and_then(Value::as_str)
                            .unwrap_or("Preview unavailable"),
                    ));
                }
            }
            Operation::Mkdir { .. } if reply.generation == self.generation => {
                self.refresh_browser()
            }
            Operation::Rename { path, .. } | Operation::Remove { path, .. } => {
                if self.file_errors.is_empty() && self.file_queue.is_empty() && !self.file_busy {
                    self.notice = format!(
                        "File action complete · {}",
                        identity(&self.devices[reply.device])
                    );
                }
                self.browser_cache.retain(|(d, _), _| *d != reply.device);
                for b in [&mut self.browser, &mut self.other_browser]
                    .into_iter()
                    .flatten()
                {
                    if b.device == reply.device {
                        b.marked.remove(&path);
                        b.entries.retain(|e| e.path != path);
                        b.visual_anchor = None;
                    }
                }
                if self
                    .browser
                    .as_ref()
                    .is_some_and(|b| b.device == reply.device)
                {
                    self.refresh_browser();
                }
            }
            _ => {}
        }
    }
    fn key(&mut self, key: KeyEvent) {
        if key.kind == KeyEventKind::Release {
            return;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.request_quit();
            return;
        }
        if self.help {
            match key.code {
                KeyCode::Down | KeyCode::Char('j') => {
                    self.help_scroll = self.help_scroll.saturating_add(1).min(40)
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    self.help_scroll = self.help_scroll.saturating_sub(1)
                }
                KeyCode::PageDown => self.help_scroll = self.help_scroll.saturating_add(8).min(40),
                KeyCode::PageUp => self.help_scroll = self.help_scroll.saturating_sub(8),
                KeyCode::Home => self.help_scroll = 0,
                _ => {}
            }
            if matches!(key.code, KeyCode::Esc | KeyCode::F(1) | KeyCode::Char('?')) {
                self.help = false;
            }
            return;
        }
        if let Some(dialog) = self.dialog.clone() {
            if matches!(key.code, KeyCode::Char('?') | KeyCode::F(1)) {
                self.help = true;
                return;
            }
            self.dialog_key(key, dialog);
            return;
        }
        // Input fields own every printable key, including navigation shortcuts.
        if let Some(mode) = self.input {
            let before_text = self.text.clone();
            match key.code {
                KeyCode::Esc => {
                    self.input = None;
                    self.text.clear();
                    self.rename_target = None;
                    self.command_target = None;
                }
                KeyCode::Backspace => {
                    if matches!(mode, Input::Rename | Input::Command) {
                        if let Some((at, _)) = self.text[..self.rename_cursor].char_indices().last()
                        {
                            self.text.replace_range(at..self.rename_cursor, "");
                            self.rename_cursor = at;
                        }
                    } else {
                        self.text.pop();
                    }
                    self.palette_selected = 0;
                }
                KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.text.clear();
                    self.rename_cursor = 0;
                }
                KeyCode::Left if matches!(mode, Input::Rename | Input::Command) => {
                    self.rename_cursor = self.text[..self.rename_cursor]
                        .char_indices()
                        .last()
                        .map(|(at, _)| at)
                        .unwrap_or(0);
                }
                KeyCode::Right if matches!(mode, Input::Rename | Input::Command) => {
                    if let Some(c) = self.text[self.rename_cursor..].chars().next() {
                        self.rename_cursor += c.len_utf8();
                    }
                }
                KeyCode::Home if matches!(mode, Input::Rename | Input::Command) => {
                    self.rename_cursor = 0
                }
                KeyCode::End if matches!(mode, Input::Rename | Input::Command) => {
                    self.rename_cursor = self.text.len()
                }
                KeyCode::Delete if matches!(mode, Input::Rename | Input::Command) => {
                    if let Some(c) = self.text[self.rename_cursor..].chars().next() {
                        self.text.replace_range(
                            self.rename_cursor..self.rename_cursor + c.len_utf8(),
                            "",
                        );
                    }
                }
                KeyCode::Char(c)
                    if !key
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                {
                    if self.text.len() + c.len_utf8()
                        <= if mode == Input::Command { 8192 } else { 256 }
                    {
                        if matches!(mode, Input::Rename | Input::Command) {
                            self.text.insert(self.rename_cursor, c);
                            self.rename_cursor += c.len_utf8();
                        } else {
                            self.text.push(c);
                        }
                    }
                    self.palette_selected = 0;
                }
                KeyCode::Down if mode == Input::Search => self.move_selection(1),
                KeyCode::Up if mode == Input::Search => self.move_selection(-1),
                KeyCode::Down if mode == Input::Filter => self.move_selection(1),
                KeyCode::Up if mode == Input::Filter => self.move_selection(-1),
                KeyCode::Tab | KeyCode::BackTab
                    if matches!(mode, Input::Search | Input::Filter) =>
                {
                    self.input = None;
                    self.cycle_focus(
                        key.code == KeyCode::BackTab || key.modifiers.contains(KeyModifiers::SHIFT),
                    );
                }
                KeyCode::Down if mode == Input::Palette => {
                    let n = self.palette().len();
                    self.palette_selected = (self.palette_selected + 1).min(n.saturating_sub(1));
                }
                KeyCode::Up if mode == Input::Palette => {
                    self.palette_selected = self.palette_selected.saturating_sub(1)
                }
                KeyCode::Enter => match mode {
                    Input::Palette => {
                        if let Some((action, _)) =
                            self.palette().get(self.palette_selected).copied()
                        {
                            self.execute(action);
                        }
                    }
                    Input::Command => {
                        if !self.text.trim().is_empty() {
                            if let Some((d, directory)) = self.command_target.take() {
                                self.pending_command = Some((
                                    self.devices[d].clone(),
                                    RunCommand {
                                        directory,
                                        command: std::mem::take(&mut self.text),
                                    },
                                ));
                                self.input = None;
                            }
                        }
                    }
                    Input::Search => {
                        self.input = None;
                        self.focus = Focus::Workspace;
                        self.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
                    }
                    Input::Filter => {
                        self.input = None;
                        self.focus = Focus::Workspace;
                    }
                    Input::Rename => {
                        if !self.text.is_empty()
                            && self.text != "."
                            && self.text != ".."
                            && !self.text.contains('/')
                            && !self.text.chars().any(char::is_control)
                        {
                            if let Some((d, entry)) = self.rename_target.take() {
                                if entry.rename_name.as_deref() == Some(self.text.as_str()) {
                                    self.notice = "Name unchanged".into();
                                } else {
                                    self.queue_file_actions(vec![(
                                        d,
                                        Operation::Rename {
                                            path: entry.path,
                                            name: self.text.clone(),
                                            expected_identity: entry.identity,
                                        },
                                    )]);
                                }
                            }
                            self.input = None;
                        } else {
                            self.notice =
                                "Enter one filename · existing files are never replaced".into();
                        }
                    }
                    Input::Add => {
                        if transport::valid_target(&self.text) {
                            self.pending_add = Some(self.text.clone());
                            self.input = None;
                        } else {
                            self.notice = "Use an SSH alias or user@host".into();
                        }
                    }
                    Input::Mkdir => {
                        if !self.text.is_empty()
                            && self.text != "."
                            && self.text != ".."
                            && !self.text.contains('/')
                            && !self.text.contains('\0')
                        {
                            if let Some(b) = &self.browser {
                                let path = join_component(&b.path, &self.text);
                                self.send(b.device, Operation::Mkdir { path });
                            }
                            self.input = None;
                        } else {
                            self.notice = "Enter a single directory name".into();
                        }
                    }
                },
                _ => {}
            }
            if mode == Input::Search
                && self.input == Some(Input::Search)
                && self.text != before_text
            {
                self.live_search();
            }
            if mode == Input::Filter
                && self.input == Some(Input::Filter)
                && self.text != before_text
            {
                if let Some(b) = &mut self.browser {
                    b.filter = self.text.clone();
                    b.selected = 0;
                    b.restore_selection = None;
                }
            }
            return;
        }
        if self.view == View::Files && self.focus == Focus::Workspace {
            if let KeyCode::Char(c @ '0'..='9') = key.code {
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                    && (c != '0' || self.motion_count.is_some())
                {
                    self.motion_count = Some(
                        self.motion_count
                            .unwrap_or(0)
                            .saturating_mul(10)
                            .saturating_add((c as u8 - b'0') as usize)
                            .min(10000),
                    );
                    self.pending_g = false;
                    return;
                }
            }
            if key.modifiers.is_empty() && key.code == KeyCode::Char('g') {
                if self.pending_g {
                    self.move_selection(-100_000);
                    self.pending_g = false;
                    self.motion_count = None;
                } else {
                    self.pending_g = true;
                }
                return;
            }
            self.pending_g = false;
            if key.code == KeyCode::Char('G')
                && !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
            {
                self.move_selection(100_000);
                self.motion_count = None;
                return;
            }
            if matches!(
                key.code,
                KeyCode::Up | KeyCode::Down | KeyCode::Char('j' | 'k')
            ) && key.modifiers.is_empty()
            {
                let count = self.motion_count.take().unwrap_or(1) as isize;
                self.move_selection(if matches!(key.code, KeyCode::Up | KeyCode::Char('k')) {
                    -count
                } else {
                    count
                });
                return;
            }
        }
        self.motion_count = None;
        self.pending_g = false;
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Char('p') => {
                    self.input = Some(Input::Palette);
                    self.text.clear();
                    self.palette_selected = 0;
                }
                KeyCode::Char('c') => self.request_quit(),
                KeyCode::Left | KeyCode::Char('h') => self.directional_focus(-1, 0),
                KeyCode::Right | KeyCode::Char('l') => self.directional_focus(1, 0),
                KeyCode::Up | KeyCode::Char('k') => self.directional_focus(0, -1),
                KeyCode::Down | KeyCode::Char('j') => self.directional_focus(0, 1),
                _ => {}
            }
            return;
        }
        if self.view == View::Files
            && self.focus == Focus::Workspace
            && self.browser.as_ref().is_some_and(|b| b.preview.is_none())
        {
            let action = match key.code {
                KeyCode::Char('.') => Some(Action::Hidden),
                KeyCode::Char(' ') => Some(Action::Select),
                KeyCode::Char('v') => Some(Action::Visual),
                KeyCode::Char('r') => Some(Action::Rename),
                KeyCode::Char('c' | 'y') => Some(Action::Copy),
                KeyCode::Char('x') => Some(Action::Cut),
                KeyCode::Char('d') => Some(Action::Delete),
                KeyCode::Char('f') => Some(Action::Filter),
                KeyCode::Char('p') => Some(Action::Paste),
                KeyCode::Char('t') => Some(Action::TransferTo),
                KeyCode::Char('T') => Some(Action::Jobs),
                KeyCode::Char('M') => Some(Action::Mkdir),
                KeyCode::Char('o') => Some(Action::Conflict),
                _ => None,
            };
            if key.code == KeyCode::Char('Y') {
                self.clipboard = None;
                self.notice = "Clipboard cleared · files unchanged".into();
                return;
            }
            if key.code == KeyCode::Char('u') {
                self.finish_visual();
                if let Some(b) = &mut self.browser {
                    b.marked.clear();
                }
                return;
            }
            if let Some(action) = action {
                if self.action_enabled(action) {
                    self.execute(action);
                }
                return;
            }
        }
        match key.code {
            KeyCode::Char('n') => self.execute(Action::New),
            KeyCode::Char(':') => self.execute(Action::Command),
            KeyCode::Char('?') | KeyCode::F(1) => self.help = true,
            KeyCode::Char('T') => self.execute(Action::Jobs),
            KeyCode::Char('/') => {
                self.finish_visual();
                self.input = Some(Input::Search);
                self.focus = Focus::Workspace;
                self.text = if self.view == View::Files {
                    self.browser
                        .as_ref()
                        .map(|b| b.search.clone())
                        .unwrap_or_default()
                } else {
                    self.search.clone()
                };
            }
            KeyCode::Tab | KeyCode::BackTab => {
                self.cycle_focus(
                    key.code == KeyCode::BackTab || key.modifiers.contains(KeyModifiers::SHIFT),
                );
            }
            KeyCode::Left | KeyCode::Char('h') => {
                if self.view == View::Files && self.focus == Focus::Workspace {
                    self.parent_directory();
                } else {
                    self.focus = Focus::Devices;
                }
            }
            KeyCode::Right | KeyCode::Char('l') => {
                if self.view == View::Files && self.focus == Focus::Workspace {
                    if let Some(b) = &self.browser {
                        if let Some(e) = self.visible_entries().get(b.selected).cloned() {
                            if e.kind == "directory" {
                                self.open_browser(b.device, e.path);
                            }
                        }
                    }
                } else {
                    self.focus = Focus::Workspace;
                }
            }
            KeyCode::Down | KeyCode::Char('j') => self.move_selection(1),
            KeyCode::Up | KeyCode::Char('k') => self.move_selection(-1),
            KeyCode::PageDown => self.move_selection(10),
            KeyCode::PageUp => self.move_selection(-10),
            KeyCode::Home => self.move_selection(-100_000),
            KeyCode::End => self.move_selection(100_000),
            KeyCode::Enter => {
                if self.focus == Focus::Actions {
                    let actions = sidebar_actions(self);
                    if let Some((action, _)) = actions.get(self.side_selected) {
                        self.execute(*action)
                    };
                } else if self.focus == Focus::Devices {
                    if self.view == View::Files {
                        self.select_file_device();
                    } else {
                        self.refresh();
                    }
                    self.focus = Focus::Workspace;
                } else {
                    match self.view {
                    View::Work => { if let Some((d,s)) = self.selected_session() { self.pending_attach = Some((d,s,false)); } },
                    View::Files => { if let Some(b) = &self.browser { if let Some(e) = self.visible_entries().get(b.selected).cloned() { if e.kind == "directory" { self.open_browser(b.device,e.path); } else { self.generation += 1; self.send(b.device,Operation::Preview { path:e.path }); } } } },
                    View::Network => self.notice = "Ctrl+P · shell / files are available through Sessions and the device selector".into(),
                }
                }
            }
            KeyCode::Esc => {
                if self.view == View::Files {
                    if let Some(b) = &mut self.browser {
                        if b.preview.take().is_none() {
                            if b.visual_anchor.take().is_some() {
                                b.visual_base.clear();
                            } else if !b.marked.is_empty() {
                                b.marked.clear();
                            } else if !b.filter.is_empty() {
                                b.filter.clear();
                                b.selected = 0;
                            } else if !b.search.is_empty() {
                                b.search.clear();
                                b.selected = 0;
                            } else {
                                self.view = View::Work;
                                self.generation += 1;
                                b.loading = false;
                            }
                        } else {
                            self.generation += 1;
                        }
                    }
                } else if !self.search.is_empty() {
                    self.search.clear();
                } else {
                    self.focus = Focus::Devices;
                }
            }
            _ => {}
        }
    }
    fn switch_pane(&mut self) {
        std::mem::swap(&mut self.browser, &mut self.other_browser);
        if let Some(browser) = &self.browser {
            self.device = browser.device + 1;
        }
        self.destination_active = !self.destination_active;
        self.generation += 1;
        self.refresh_browser();
    }
    fn directional_focus(&mut self, dx: i32, dy: i32) {
        let panels = self.panels.borrow().clone();
        let Some((_, _, current)) = panels.iter().find(|(focus, dest, _)| {
            *focus == self.focus && (*focus != Focus::Workspace || *dest == self.destination_active)
        }) else {
            return;
        };
        let center = |r: &Rect| {
            (
                r.x as i32 * 2 + r.width as i32,
                r.y as i32 * 2 + r.height as i32,
            )
        };
        let (cx, cy) = center(current);
        let selected = panels
            .iter()
            .filter_map(|(focus, dest, r)| {
                let (x, y) = center(r);
                let primary = (x - cx) * dx + (y - cy) * dy;
                let cross = if dx != 0 {
                    (y - cy).abs()
                } else {
                    (x - cx).abs()
                };
                (primary > 0).then_some((primary + cross * 2, *focus, *dest))
            })
            .min_by_key(|(score, _, _)| *score);
        if let Some((_, focus, dest)) = selected {
            if focus == Focus::Workspace
                && self.other_browser.is_some()
                && dest != self.destination_active
            {
                self.switch_pane();
            }
            self.focus = focus;
        }
    }
    fn cycle_focus(&mut self, reverse: bool) {
        if self.view == View::Files && self.other_browser.is_some() {
            if self.focus == Focus::Workspace && self.destination_active == reverse {
                self.switch_pane();
                return;
            }
            if (self.focus == Focus::Actions && !reverse)
                || (self.focus == Focus::Devices && reverse)
            {
                if self.destination_active != reverse {
                    self.switch_pane();
                }
            }
        }
        self.focus = match (self.focus, reverse) {
            (Focus::Workspace, false) | (Focus::Actions, true) => Focus::Devices,
            (Focus::Devices, false) | (Focus::Workspace, true) => Focus::Actions,
            (Focus::Actions, false) | (Focus::Devices, true) => Focus::Workspace,
        };
    }
    fn parent_directory(&mut self) {
        if let Some(b) = &self.browser {
            if let Some(path) = b.parent.clone() {
                self.open_browser(b.device, path);
            }
        }
    }
    fn move_selection(&mut self, delta: isize) {
        if self.view == View::Files && self.focus == Focus::Workspace {
            if let Some(b) = &mut self.browser {
                if b.preview.is_some() {
                    b.preview_scroll = b.preview_scroll.saturating_add_signed(delta as i16);
                    return;
                }
            }
        }
        if self.focus == Focus::Actions {
            self.side_selected = shift(self.side_selected, delta, sidebar_actions(self).len());
        } else if self.focus == Focus::Devices {
            self.device = shift(self.device, delta, self.devices.len() + 1);
            self.selected = 0;
            if self.view == View::Files {
                self.select_file_device();
            } else {
                self.refresh();
            }
        } else if self.view == View::Files {
            let len = self.visible_entries().len();
            if let Some(b) = &mut self.browser {
                b.restore_selection = None;
                b.selected = shift(b.selected, delta, len);
                if let Some(anchor) = b.visual_anchor {
                    b.marked = b.visual_base.clone();
                    let entries = browser_entries(b);
                    let start = anchor.min(b.selected);
                    let end = anchor.max(b.selected);
                    for entry in entries.iter().skip(start).take(end - start + 1) {
                        if b.marked.len() < 256 {
                            b.marked.insert(entry.path.clone());
                        }
                    }
                }
            }
        } else {
            self.selected = shift(self.selected, delta, self.session_rows().len());
        }
    }
}
/// Subsequence matcher with word-start and contiguous bonuses; empty queries retain order.
fn fuzzy_score(query: &str, text: &str) -> Option<i64> {
    let hay: Vec<char> = text.to_lowercase().chars().collect();
    let needle: Vec<char> = query
        .to_lowercase()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    if needle.is_empty() {
        return Some(0);
    }
    let mut at = 0;
    let mut prior = None;
    let mut score = 0;
    for c in needle {
        let pos = (at..hay.len()).find(|&i| hay[i] == c)?;
        score += 10;
        if pos == 0 || !hay[pos - 1].is_alphanumeric() {
            score += 12;
        }
        if prior == Some(pos.saturating_sub(1)) {
            score += 16;
        }
        score -= if let Some(p) = prior {
            (pos - p - 1) as i64
        } else {
            pos.min(30) as i64
        };
        prior = Some(pos);
        at = pos + 1;
    }
    Some(score)
}
fn shift(index: usize, delta: isize, length: usize) -> usize {
    index
        .saturating_add_signed(delta)
        .min(length.saturating_sub(1))
}
fn join_component(path: &str, name: &str) -> String {
    use base64::Engine;
    if let Some(encoded) = path.strip_prefix("cx-bytes:") {
        if let Ok(mut bytes) = base64::engine::general_purpose::STANDARD.decode(encoded) {
            if !bytes.ends_with(b"/") {
                bytes.push(b'/');
            }
            bytes.extend_from_slice(name.as_bytes());
            return format!(
                "cx-bytes:{}",
                base64::engine::general_purpose::STANDARD.encode(bytes)
            );
        }
    }
    format!("{}/{}", path.trim_end_matches('/'), name)
}
fn unique_key() -> String {
    format!(
        "{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    )
}
fn fit_label(text: &str, width: usize) -> String {
    let safe = safe_label(text);
    if Span::raw(&safe).width() <= width {
        return safe;
    }
    let mut out = safe
        .chars()
        .take(width.saturating_sub(1))
        .collect::<String>();
    out.push(if ascii() { '~' } else { '…' });
    out
}
fn safe_text(text: &str) -> String {
    text.chars()
        .map(|c| {
            if c.is_control() && c != '\n' && c != '\t' {
                '�'
            } else {
                c
            }
        })
        .collect()
}
fn identity(device: &Device) -> String {
    safe_label(&format!("{}@{}", device.account, device.host))
}
fn safe_label(text: &str) -> String {
    text.chars()
        .map(|c| {
            if c.is_control() || matches!(c,'\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}') {
                '�'
            } else {
                c
            }
        })
        .collect()
}
fn block(title: String, focused: bool) -> Block<'static> {
    Block::default()
        .title(Line::from(Span::styled(
            if focused {
                format!(" {} {title} ", if ascii() { ">" } else { "▸" })
            } else {
                format!(" {title} ")
            },
            accent().add_modifier(Modifier::BOLD),
        )))
        .border_type(if ascii() {
            BorderType::Plain
        } else {
            BorderType::Rounded
        })
        .borders(Borders::ALL)
        .border_style(if focused {
            Style::default().add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        })
}
fn selected_style() -> Style {
    accent().add_modifier(Modifier::REVERSED | Modifier::BOLD)
}
fn accent() -> Style {
    if std::env::var_os("NO_COLOR").is_some() {
        Style::default()
    } else {
        Style::default().fg(Color::Cyan)
    }
}
fn muted() -> Style {
    Style::default().add_modifier(Modifier::DIM)
}
fn ascii() -> bool {
    std::env::var_os("CX_ASCII").is_some() || std::env::var("TERM").is_ok_and(|v| v == "dumb")
}
fn workspace_action(action: Action) -> bool {
    !matches!(
        action,
        Action::Destination
            | Action::TransferTo
            | Action::Conflict
            | Action::Copy
            | Action::Cut
            | Action::Rename
            | Action::Delete
            | Action::Hidden
            | Action::Filter
            | Action::Select
            | Action::Visual
            | Action::Paste
            | Action::Mkdir
    )
}

fn sidebar_actions(app: &App) -> Vec<(Action, &'static str)> {
    let mut actions = vec![
        (Action::Work, "Sessions"),
        (Action::Files, "Files"),
        (Action::Network, "Network"),
        (Action::New, "New session"),
        (Action::Jobs, "Transfers"),
        (Action::Add, "Add device"),
    ];
    actions.retain(|(action, _)| {
        app.action_enabled(*action) && !(*action == Action::Files && app.view == View::Files)
    });
    if app.selected_session().is_some() && app.view == View::Work {
        actions.push((Action::Observe, "Watch · read-only"));
    }
    actions
}

fn render(frame: &mut Frame<'_>, app: &App) {
    let area = frame.area();
    if area.width < 36 || area.height < 10 {
        frame.render_widget(
            Paragraph::new("cx · enlarge terminal\nCtrl+P actions · Ctrl+C quit")
                .wrap(Wrap { trim: false }),
            area,
        );
        return;
    }
    let query = if app.input == Some(Input::Filter) {
        app.browser
            .as_ref()
            .map(|b| b.filter.as_str())
            .unwrap_or("")
    } else if app.view == View::Files {
        app.browser
            .as_ref()
            .map(|b| b.search.as_str())
            .unwrap_or("")
    } else {
        app.search.as_str()
    };
    let show_search = matches!(app.input, Some(Input::Search | Input::Filter)) || !query.is_empty();
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(5),
            Constraint::Length(if show_search { 7 } else { 4 }),
        ])
        .split(area);
    let label = match (app.view, area.width < 70) {
        (View::Work, true) => "Sessions",
        (View::Files, true) => "Files",
        (View::Network, true) => "Network",
        (View::Work, false) => "[Sessions]  Network",
        (View::Files, false) => "Sessions / Files  Network",
        (View::Network, false) => "Sessions  [Network]",
    };
    let viewer = app
        .devices
        .iter()
        .find(|d| d.target.is_none())
        .map(identity)
        .unwrap_or_else(|| "local viewer".into());
    let top = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Min(20),
            Constraint::Length(viewer.len().min(40) as u16 + 8),
        ])
        .split(vertical[0]);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" cx ", accent().add_modifier(Modifier::BOLD)),
            Span::raw("  "),
            Span::styled(label, Style::default().add_modifier(Modifier::BOLD)),
        ])),
        top[0],
    );
    frame.render_widget(
        Paragraph::new(format!("viewing {viewer}"))
            .style(muted())
            .alignment(ratatui::layout::Alignment::Right),
        top[1],
    );
    let sidebar_width: u16 =
        if area.width < 60 && app.view == View::Files && app.focus == Focus::Workspace {
            14
        } else if area.width < 70 {
            17
        } else {
            21
        };
    let content = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(sidebar_width), Constraint::Min(15)])
        .split(vertical[1]);
    let mut device_items = vec![ListItem::new("All devices")];
    for (i, d) in app.devices.iter().enumerate() {
        let count = app.work[i].sessions.len();
        let status = if app.work[i].loading {
            "checking"
        } else if app.work[i].error.is_some() {
            "unavailable"
        } else if app.work[i].fetched == 0
            || transport::now().saturating_sub(app.work[i].fetched) > 60
        {
            "unknown"
        } else {
            "ready"
        };
        let indicator = match status {
            "ready" => {
                if ascii() {
                    "+"
                } else {
                    "●"
                }
            }
            "checking" => {
                if ascii() {
                    "~"
                } else {
                    "◌"
                }
            }
            "unavailable" => {
                if ascii() {
                    "x"
                } else {
                    "○"
                }
            }
            _ => "?",
        };
        let dot_style = if std::env::var_os("NO_COLOR").is_some() {
            Style::default()
        } else {
            Style::default().fg(match status {
                "ready" => Color::Green,
                "checking" => Color::Cyan,
                "unavailable" => Color::Yellow,
                _ => Color::DarkGray,
            })
        };
        device_items.push(ListItem::new(Line::from(vec![
            Span::styled(format!("{indicator} "), dot_style),
            Span::raw(format!(
                "{:<width$} {count:>2}",
                fit_label(&d.name, sidebar_width.saturating_sub(9) as usize),
                width = sidebar_width.saturating_sub(9) as usize
            )),
        ])));
    }
    let sidebar = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length((app.devices.len() as u16 + 3).min(content[0].height / 2)),
            Constraint::Length(
                (sidebar_actions(app).len() as u16 + 2)
                    .min(content[0].height.saturating_sub(
                        (app.devices.len() as u16 + 3).min(content[0].height / 2) + 4,
                    ))
                    .max(5),
            ),
            Constraint::Min(0),
        ])
        .split(content[0]);
    *app.panels.borrow_mut() = vec![
        (Focus::Devices, false, sidebar[0]),
        (Focus::Actions, false, sidebar[1]),
    ];
    if app.view != View::Files || app.other_browser.is_none() {
        app.panels
            .borrow_mut()
            .push((Focus::Workspace, app.destination_active, content[1]));
    }
    let mut state = ratatui::widgets::ListState::default().with_selected(Some(app.device));
    frame.render_stateful_widget(
        List::new(device_items)
            .block(block("Devices".into(), app.focus == Focus::Devices))
            .highlight_style(if app.focus == Focus::Devices {
                selected_style()
            } else {
                accent().add_modifier(Modifier::BOLD)
            })
            .highlight_symbol(if ascii() { "> " } else { "› " }),
        sidebar[0],
        &mut state,
    );
    let actions = sidebar_actions(app);
    let items = actions
        .iter()
        .map(|(_, label)| ListItem::new(format!(" {label}")))
        .collect::<Vec<_>>();
    let mut state =
        ratatui::widgets::ListState::default().with_selected(if app.focus == Focus::Actions {
            Some(app.side_selected.min(actions.len().saturating_sub(1)))
        } else {
            None
        });
    frame.render_stateful_widget(
        List::new(items)
            .block(block("Actions".into(), app.focus == Focus::Actions))
            .highlight_style(selected_style()),
        sidebar[1],
        &mut state,
    );
    let details = if let Some(b) = app.browser.as_ref().filter(|_| app.view == View::Files) {
        let entry = browser_entries(b).get(b.selected).cloned();
        format!(
            "{}\n\n{}\n{}\n\n{} selected{}",
            identity(&app.devices[b.device]),
            entry
                .as_ref()
                .map(|e| safe_label(&e.name))
                .unwrap_or_else(|| "No file".into()),
            entry
                .as_ref()
                .map(|e| format!("{} · {}", e.kind, human_size(e.size)))
                .unwrap_or_default(),
            b.marked.len(),
            if b.marked
                .iter()
                .any(|p| !browser_entries(b).iter().any(|e| &e.path == p))
            {
                " (includes hidden/filtered)"
            } else {
                ""
            }
        )
    } else if let Some((i, s)) = app.selected_session() {
        format!(
            "{}\n{}\n\n{}\n\n{}",
            safe_label(&s.name),
            safe_label(&s.provider),
            identity(&app.devices[i]),
            safe_label(&s.directory)
        )
    } else if let Some(b) = &app.browser {
        format!(
            "{}\n\n{}",
            identity(&app.devices[b.device]),
            safe_label(&b.display_path)
        )
    } else if let Some(i) = app.actual_device() {
        format!(
            "{}\n{}\n\n{} sessions",
            safe_label(&app.devices[i].name),
            identity(&app.devices[i]),
            app.work[i].sessions.len()
        )
    } else {
        "Select work to see\nits execution context".into()
    };
    if sidebar[2].height > 3 {
        frame.render_widget(
            Paragraph::new(details)
                .style(muted())
                .wrap(Wrap { trim: false })
                .block(
                    Block::default()
                        .title(Line::from(Span::styled(
                            " Selected ",
                            accent().add_modifier(Modifier::BOLD),
                        )))
                        .borders(Borders::TOP),
                ),
            sidebar[2],
        );
    }
    let split_workspace = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(5),
            Constraint::Length(
                if app.view == View::Files && app.transfer_drawer && content[1].height >= 12 {
                    4
                } else {
                    0
                },
            ),
        ])
        .split(content[1]);
    let workspace = split_workspace[0];
    match app.view {
        View::Work => {
            let rows = app.session_rows();
            if rows.is_empty() {
                let checking = app.work.iter().any(|w| w.loading);
                let errors: Vec<_> = app
                    .work
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| app.device == 0 || app.device == i + 1)
                    .filter_map(|(_, w)| w.error.as_deref())
                    .collect();
                let text = if checking {
                    "Checking sessions…".into()
                } else if !errors.is_empty() {
                    errors.join("\n")
                } else {
                    "No live sessions in this view.\nCtrl+P → New session → choose provider and folder".into()
                };
                frame.render_widget(
                    Paragraph::new(text)
                        .wrap(Wrap { trim: false })
                        .block(block("Sessions".into(), app.focus == Focus::Workspace)),
                    workspace,
                );
            } else {
                let mut display_rows = Vec::new();
                let mut selected_row = 0;
                let mut last_project = None;
                for (index, (i, s)) in rows.iter().enumerate() {
                    if last_project != Some(s.directory.as_str()) {
                        display_rows.push(
                            Row::new([
                                Cell::from(""),
                                Cell::from(safe_label(&s.directory)),
                                Cell::from(""),
                                Cell::from(""),
                            ])
                            .style(muted())
                            .height(2),
                        );
                        last_project = Some(s.directory.as_str());
                    }
                    if index == app.selected {
                        selected_row = display_rows.len();
                    }
                    let fresh = app.work[*i].fetched > 0
                        && transport::now().saturating_sub(app.work[*i].fetched) <= 60
                        && app.work[*i].error.is_none();
                    let badge = format!("[{}]", safe_label(&s.provider));
                    display_rows.push(Row::new([
                        Cell::from(badge).style(if std::env::var_os("NO_COLOR").is_some() {
                            Style::default()
                        } else {
                            Style::default().fg(match s.provider.as_str() {
                                "claude" => Color::Yellow,
                                "codex" => Color::Cyan,
                                _ => Color::Gray,
                            })
                        }),
                        Cell::from(safe_label(&s.name)),
                        Cell::from(identity(&app.devices[*i])),
                        Cell::from(if !fresh {
                            "? cached"
                        } else if s.external {
                            if ascii() {
                                "+ ext"
                            } else {
                                "● ext"
                            }
                        } else {
                            if ascii() {
                                "+ live"
                            } else {
                                "● live"
                            }
                        })
                        .style(if !fresh { muted() } else { accent() }),
                    ]));
                }
                let mut state = TableState::default().with_selected(Some(selected_row));
                let table = Table::new(
                    display_rows,
                    [
                        Constraint::Length(9),
                        Constraint::Min(10),
                        Constraint::Length(if workspace.width > 65 { 24 } else { 18 }),
                        Constraint::Length(8),
                    ],
                )
                .header(
                    Row::new(["AGENT", "SESSION", "EXECUTION", "STATE"])
                        .style(muted())
                        .bottom_margin(1),
                )
                .column_spacing(2)
                .block(block(
                    format!("Sessions · {} sessions", rows.len()),
                    app.focus == Focus::Workspace,
                ))
                .row_highlight_style(selected_style())
                .highlight_symbol(if ascii() { "> " } else { "› " });
                frame.render_stateful_widget(table, workspace, &mut state);
            }
        }
        View::Files => {
            if let Some(b) = &app.browser {
                if let Some(other) = &app.other_browser {
                    let panes = Layout::default()
                        .direction(if workspace.width >= 52 {
                            Direction::Horizontal
                        } else {
                            Direction::Vertical
                        })
                        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                        .split(workspace);
                    app.panels.borrow_mut().extend([
                        (Focus::Workspace, false, panes[0]),
                        (Focus::Workspace, true, panes[1]),
                    ]);
                    let (source, destination) = if app.destination_active {
                        (other, b)
                    } else {
                        (b, other)
                    };
                    render_browser(
                        frame,
                        source,
                        &app.devices[source.device],
                        panes[0],
                        !app.destination_active && app.focus == Focus::Workspace,
                        "Source",
                        app.clipboard.as_ref(),
                    );
                    render_browser(
                        frame,
                        destination,
                        &app.devices[destination.device],
                        panes[1],
                        app.destination_active && app.focus == Focus::Workspace,
                        &format!("Destination · {}", app.conflict_policy()),
                        app.clipboard.as_ref(),
                    );
                } else {
                    let label = app
                        .launch_provider
                        .as_ref()
                        .map(|p| format!("Start {p} here · n"))
                        .unwrap_or_else(|| "Files".into());
                    render_browser(
                        frame,
                        b,
                        &app.devices[b.device],
                        workspace,
                        app.focus == Focus::Workspace,
                        &label,
                        app.clipboard.as_ref(),
                    );
                }
            }
        }
        View::Network => {
            let d = app.actual_device();
            let title = d
                .map(|d| format!("Network · {}", identity(&app.devices[d])))
                .unwrap_or_else(|| "Network · select a device".into());
            let text = if let Some(value) = d.and_then(|d| app.network.get(&d)) {
                format!(
                    "{}{}",
                    network_summary(value),
                    if app.network_loading {
                        "\nRefreshing…"
                    } else {
                        ""
                    }
                )
            } else if app.network_loading {
                "Checking interface and route observations…".into()
            } else {
                "No recent network observation.\nCtrl+P → Refresh".into()
            };
            frame.render_widget(
                Paragraph::new(text)
                    .wrap(Wrap { trim: false })
                    .block(block(title, app.focus == Focus::Workspace)),
                workspace,
            );
        }
    }
    if split_workspace[1].height > 0 {
        let rows = app.job_rows();
        let mut lines = rows
            .iter()
            .take(2)
            .map(|(_, j)| {
                Line::from(vec![
                    Span::styled(
                        format!(" {} ", transfer_status(j)),
                        tint(if j["status"] == "failed" {
                            Color::Red
                        } else if j["status"] == "complete" {
                            Color::Green
                        } else {
                            Color::Yellow
                        }),
                    ),
                    Span::raw(format!(
                        "{} {}",
                        if j["operation"] == "move" {
                            "move"
                        } else {
                            "copy"
                        },
                        transfer_name(j)
                    )),
                    Span::styled(
                        format!(
                            " · {}",
                            if j["source_host"] == j["destination_host"] {
                                safe_label(j["source_host"].as_str().unwrap_or("?"))
                            } else {
                                format!(
                                    "{} → {}",
                                    safe_label(j["source_host"].as_str().unwrap_or("?")),
                                    safe_label(j["destination_host"].as_str().unwrap_or("?"))
                                )
                            }
                        ),
                        muted(),
                    ),
                ])
            })
            .collect::<Vec<_>>();
        if !app.file_errors.is_empty() {
            lines.insert(
                0,
                Line::from(Span::styled(
                    format!(
                        " × {} actions rejected · {}",
                        app.file_errors.len(),
                        app.file_errors.last().unwrap()
                    ),
                    tint(Color::Red),
                )),
            );
            lines.truncate(2);
        }
        frame.render_widget(
            Paragraph::new(if lines.is_empty() {
                vec![Line::from(" Submitting selected items…")]
            } else {
                lines
            })
            .block(block("Transfers · T details".into(), false)),
            split_workspace[1],
        );
    }
    let footer = Layout::default()
        .direction(Direction::Vertical)
        .constraints(if show_search {
            vec![
                Constraint::Length(3),
                Constraint::Length(1),
                Constraint::Length(2),
                Constraint::Length(1),
            ]
        } else {
            vec![
                Constraint::Length(1),
                Constraint::Length(2),
                Constraint::Length(1),
            ]
        })
        .split(vertical[2]);
    let status_row = if show_search { 1 } else { 0 };
    let key_row = if show_search { 2 } else { 1 };
    let mut unique_jobs = std::collections::BTreeMap::new();
    for j in app
        .jobs
        .values()
        .filter_map(|v| v["jobs"].as_array())
        .flatten()
    {
        unique_jobs.insert(
            (
                j["key"].as_str().unwrap_or(""),
                j["source_host"].as_str().unwrap_or(""),
                j["destination_host"].as_str().unwrap_or(""),
            ),
            j,
        );
    }
    let jobs = unique_jobs.values().copied().collect::<Vec<_>>();
    let active = jobs
        .iter()
        .filter(|j| {
            matches!(
                j["status"].as_str(),
                Some("running" | "queued" | "submitting")
            )
        })
        .count();
    let failed = jobs.iter().filter(|j| j["status"] == "failed").count();
    let ready = app
        .work
        .iter()
        .filter(|w| {
            w.fetched > 0 && transport::now().saturating_sub(w.fetched) < 60 && w.error.is_none()
        })
        .count();
    let errors = app.work.iter().filter(|w| w.error.is_some()).count();
    let shown = if app.view == View::Files {
        app.visible_entries().len()
    } else {
        app.session_rows().len()
    };
    let total = if app.view == View::Files {
        app.browser.as_ref().map(|b| b.entries.len()).unwrap_or(0)
    } else {
        app.work
            .iter()
            .enumerate()
            .filter(|(i, _)| app.device == 0 || app.device == i + 1)
            .map(|(_, w)| w.sessions.len())
            .sum()
    };
    let status_sections = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(if area.width < 60 {
            vec![
                Constraint::Length(0),
                Constraint::Percentage(55),
                Constraint::Percentage(45),
            ]
        } else {
            vec![
                Constraint::Percentage(30),
                Constraint::Percentage(30),
                Constraint::Percentage(40),
            ]
        })
        .split(footer[status_row]);
    let healthy = if std::env::var_os("NO_COLOR").is_some() {
        Style::default()
    } else {
        Style::default().fg(Color::Green)
    };
    let warning = if std::env::var_os("NO_COLOR").is_some() {
        Style::default()
    } else {
        Style::default().fg(Color::Yellow)
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            if errors > 0 {
                format!(" ◆ {errors} unavailable  ")
            } else {
                format!(
                    " {} {ready}/{} devices  ",
                    if ascii() { "+" } else { "●" },
                    app.devices.len()
                )
            },
            if errors > 0 { warning } else { healthy },
        )])),
        status_sections[0],
    );
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                format!("Focus: {}", app.focus_label()),
                accent().add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                if app.dialog.is_some() || app.help {
                    String::new()
                } else {
                    format!(" · {shown}/{total}")
                },
                muted(),
            ),
        ]))
        .alignment(ratatui::layout::Alignment::Center),
        status_sections[1],
    );
    let transfer = if active > 0 {
        format!("{} {active} active", if ascii() { "~" } else { "↔" })
    } else if failed > 0 {
        format!("{} {failed} failed", if ascii() { "x" } else { "×" })
    } else if !jobs.is_empty() {
        let completed = jobs.iter().filter(|j| j["status"] == "complete").count();
        let skipped = jobs
            .iter()
            .map(|j| j["skipped"].as_u64().unwrap_or(0))
            .sum::<u64>();
        if skipped > 0 {
            format!(
                "{} {completed} done · {skipped} skipped",
                if ascii() { "+" } else { "●" }
            )
        } else {
            format!("{} {completed} complete", if ascii() { "+" } else { "●" })
        }
    } else {
        format!("{} idle", if ascii() { "~" } else { "↔" })
    };
    frame.render_widget(
        Paragraph::new(transfer)
            .style(if failed > 0 { warning } else { accent() })
            .alignment(ratatui::layout::Alignment::Right),
        status_sections[2],
    );
    if show_search {
        let editing = matches!(app.input, Some(Input::Search | Input::Filter));
        let text = if editing { app.text.as_str() } else { query };
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(
                    if app.input == Some(Input::Filter) {
                        " f  "
                    } else {
                        " /  "
                    },
                    accent().add_modifier(Modifier::BOLD),
                ),
                Span::raw(safe_label(text)),
            ]))
            .block(block(
                if app.input == Some(Input::Filter) {
                    "Filter"
                } else {
                    "Search"
                }
                .into(),
                editing,
            )),
            footer[0],
        );
        if editing {
            let width = Span::raw(safe_label(text)).width() as u16;
            frame.set_cursor_position((
                footer[0].x + 5 + width.min(footer[0].width.saturating_sub(7)),
                footer[0].y + 1,
            ));
        }
    } else {
        frame.render_widget(
            Paragraph::new(safe_text(&app.notice)).style(muted()),
            footer[2],
        );
    }
    if show_search {
        frame.render_widget(
            Paragraph::new(safe_text(&app.notice)).style(muted()),
            footer[3],
        );
    }
    let mut hints = if app.help {
        vec![("↑↓", "Scroll"), ("PgUpDn", "Page"), ("Esc", "Close")]
    } else if let Some(dialog) = &app.dialog {
        if matches!(dialog, Dialog::Jobs) {
            vec![
                ("c", "Cancel job"),
                ("r", "Retry job"),
                ("Esc", "Back"),
                ("Tab", "Details"),
                ("PgUpDn", "Scroll"),
                (
                    "Enter",
                    if app.dialog_detail_focus {
                        "Back"
                    } else {
                        "Details"
                    },
                ),
            ]
        } else if matches!(dialog, Dialog::Delete(..)) {
            vec![
                (
                    "↑↓",
                    if app.dialog_detail_focus {
                        "Scroll"
                    } else {
                        "Choose"
                    },
                ),
                (
                    "Enter",
                    if app.dialog_detail_focus {
                        "Back"
                    } else {
                        "Confirm"
                    },
                ),
                ("y / n", "Delete / cancel"),
                ("Tab", "Details"),
                ("PgUpDn", "Scroll"),
                ("Home", "Top"),
            ]
        } else {
            vec![("↑↓", "Choose"), ("Enter", "Confirm"), ("Esc", "Cancel")]
        }
    } else if matches!(
        app.input,
        Some(Input::Rename | Input::Mkdir | Input::Add | Input::Palette | Input::Command)
    ) {
        vec![
            (
                "Enter",
                if app.input == Some(Input::Command) {
                    "Run"
                } else {
                    "Confirm"
                },
            ),
            ("Esc", "Cancel"),
            ("Ctrl U", "Clear"),
        ]
    } else if matches!(app.input, Some(Input::Search | Input::Filter)) {
        vec![
            ("↑↓", "Select"),
            (
                "Enter",
                if app.input == Some(Input::Filter) {
                    "Done"
                } else {
                    "Open"
                },
            ),
            ("Esc", "Done"),
            ("Tab", "Focus"),
        ]
    } else if app.view == View::Files && app.focus == Focus::Workspace {
        vec![
            ("Space", "Select"),
            ("c / x", "Copy / cut"),
            ("p / t", "Paste / transfer"),
            ("n / :", "Session / command"),
            ("r / d", "Rename / delete"),
            ("?", "All keys"),
        ]
    } else {
        vec![
            (
                "Enter",
                if app.focus == Focus::Actions {
                    "Run"
                } else {
                    "Open"
                },
            ),
            ("/", "Search"),
            ("Tab", "Focus"),
            ("Ctrl P", "Actions"),
            ("?", "Help"),
            ("Ctrl C", "Quit"),
        ]
    };
    let columns = if footer[key_row].width >= 60 { 3 } else { 2 };
    if columns == 2 && !matches!(app.input, Some(Input::Search | Input::Filter)) {
        hints.retain(|(_, label)| !matches!(*label, "Search" | "Focus"));
        hints.truncate(4);
        if app.dialog.is_none() && app.input.is_none() && !app.help && app.view == View::Files {
            hints[3] = ("t / T", "Send / jobs");
        }
    }
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1)])
        .split(footer[key_row]);
    for (i, (key, label)) in hints.into_iter().enumerate() {
        let key = if ascii() && key == "↑↓" {
            "j/k"
        } else {
            key
        };
        let cells = Layout::default()
            .direction(Direction::Horizontal)
            .constraints(vec![Constraint::Ratio(1, columns as u32); columns])
            .split(rows[i / columns]);
        let key_style = if std::env::var_os("NO_COLOR").is_some() {
            Style::default().add_modifier(Modifier::BOLD)
        } else {
            accent().add_modifier(Modifier::BOLD)
        };
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(format!("  {key:<6}  "), key_style),
                Span::raw(label),
            ])),
            cells[i % columns],
        );
    }
    if let Some(input) = app
        .input
        .filter(|i| !matches!(*i, Input::Search | Input::Filter))
    {
        let rect = popup(
            area,
            76,
            if input == Input::Palette {
                16
            } else if matches!(input, Input::Rename | Input::Command) {
                3
            } else {
                5
            },
        );
        frame.render_widget(Clear, rect);
        if input == Input::Palette {
            let parts = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(3), Constraint::Min(1)])
                .split(rect);
            frame.render_widget(
                Paragraph::new(format!("> {}", safe_text(&app.text)))
                    .block(block("Actions · type to filter".into(), true)),
                parts[0],
            );
            let items = app
                .palette()
                .iter()
                .map(|(_, s)| ListItem::new(*s))
                .collect::<Vec<_>>();
            let mut state =
                ratatui::widgets::ListState::default().with_selected(Some(app.palette_selected));
            frame.render_stateful_widget(
                List::new(items)
                    .block(Block::default().borders(Borders::ALL))
                    .highlight_style(selected_style()),
                parts[1],
                &mut state,
            );
        } else {
            let editing = matches!(input, Input::Rename | Input::Command);
            let cursor_width = if editing {
                Span::raw(safe_label(&app.text[..app.rename_cursor])).width() as u16 + 2
            } else {
                0
            };
            let scroll = cursor_width.saturating_sub(rect.width.saturating_sub(3));
            frame.render_widget(
                Paragraph::new(if matches!(input, Input::Rename | Input::Command) {
                    format!("> {}", safe_label(&app.text))
                } else {
                    format!("> {}\nEnter confirm · Escape cancel", safe_label(&app.text))
                })
                .scroll((0, scroll))
                .block(block(
                    if input == Input::Command {
                        app.command_target
                            .as_ref()
                            .map(|(d, path)| {
                                format!(
                                    "Run command · {} · {}",
                                    identity(&app.devices[*d]),
                                    safe_label(
                                        app.browser
                                            .as_ref()
                                            .filter(|b| b.device == *d && b.path == *path)
                                            .map(|b| b.display_path.as_str())
                                            .unwrap_or(path)
                                    )
                                )
                            })
                            .unwrap_or_else(|| "Run command".into())
                    } else {
                        match input {
                            Input::Search => "Search",
                            Input::Add => "Add device · SSH alias or user@host",
                            Input::Rename => "Rename",
                            Input::Command => "Run command",
                            _ => "Directory name",
                        }
                        .into()
                    },
                    true,
                )),
                rect,
            );
            if editing {
                frame.set_cursor_position((rect.x + 1 + cursor_width - scroll, rect.y + 1));
            }
        }
    }
    if let Some(dialog) = &app.dialog {
        let (title, labels, detail) = match dialog {
            Dialog::Device(purpose) => (
                match purpose {
                    ChooseDevice::New => "New session · execution device",
                    ChooseDevice::Files => "Files · choose device",
                    ChooseDevice::Destination => "Transfer to · destination device",
                }
                .to_string(),
                app.devices
                    .iter()
                    .map(|d| format!("{} · {}", safe_label(&d.name), identity(d)))
                    .collect::<Vec<_>>(),
                if *purpose == ChooseDevice::Destination {
                    app.clipboard
                        .as_ref()
                        .map(|c| {
                            format!(
                                "Source: {} · {}\nChoose device, browse folder, then p Paste here\nEnter choose · Escape cancel",
                                identity(&app.devices[c.device]),
                                format!("{} {} items", if c.cut { "cut" } else { "copy" }, c.entries.len())
                            )
                        })
                        .unwrap_or_else(|| "Enter choose · Escape cancel".into())
                } else {
                    "Enter choose · Escape cancel".to_string()
                },
            ),
            Dialog::Provider(d, path) => (
                format!("New session · {}", identity(&app.devices[*d])),
                app.provider_choices(*d)
                    .iter()
                    .map(|provider| match *provider {
                        "claude" => "Claude · existing host profile".into(),
                        "codex" => "Codex · existing host profile".into(),
                        _ => "Shell · ordinary terminal".into(),
                    })
                    .collect(),
                if app.provider_loading.contains(d) {
                    "Checking installed agents… · Shell is available now".into()
                } else if !app
                    .providers
                    .get(d)
                    .is_some_and(|(_, checked)| transport::now().saturating_sub(*checked) < 60)
                {
                    "Agent availability unknown · Escape, then Refresh to retry".into()
                } else if let Some(path) = path {
                    let location = app
                        .browser
                        .as_ref()
                        .filter(|b| b.device == *d && b.path == *path)
                        .map(|b| b.display_path.as_str())
                        .unwrap_or(path);
                    format!(
                        "Enter starts here: {} · Escape cancel",
                        safe_label(location)
                    )
                } else {
                    "Enter choose · next: browse folder, then n to start".into()
                },
            ),
            Dialog::Delete(d, entries) => (
                format!("Delete {} items permanently?", entries.len()),
                vec!["Cancel · keep files".into(), "Delete permanently".into()],
                format!("No undo. {} folders include all contents.\nHost: {}\n{}",
                    entries.iter().filter(|e| e.kind == "directory").count(), identity(&app.devices[*d]),
                    entries.iter().map(|e| format!("{} {}", if ascii() { "-" } else { "•" }, safe_label(&e.name))).collect::<Vec<_>>().join("\n")),
            ),
            Dialog::PendingExit(count) => (
                "File actions are still being submitted".into(),
                vec!["Stay · finish submitting".into(), "Quit · discard unsubmitted actions".into()],
                format!("{count} actions have not been submitted.\nAlready detached transfers continue.\nEnter chooses · Escape stays"),
            ),
            Dialog::Matching(d, path, provider, s) => (
                format!("Matching {provider} · {}", identity(&app.devices[*d])),
                vec![
                    format!("Open existing · {}", safe_label(&s.name)),
                    "Create another session".into(),
                ],
                safe_label(path),
            ),
            Dialog::Jobs => {
                let jobs = app.job_rows();
                let detail = jobs
                    .get(app.dialog_selected)
                    .map(|(_, j)| {
                        format!(
                            "{}\nRoute: {}\nSource: {} · {}\nDestination: {} · {}",
                            j["error"].as_str().map(safe_label).unwrap_or_else(|| format!("{} · {}", transfer_status(j), if j["operation"] == "move" { "move" } else { "copy" })),
                            safe_label(j["route"].as_str().unwrap_or("route unknown")),
                            safe_label(j["source_host"].as_str().unwrap_or("unknown host")),
                            safe_label(j["source_display"].as_str().or(j["source_path"].as_str()).unwrap_or("")),
                            safe_label(j["destination_host"].as_str().unwrap_or("unknown host")),
                            transfer_destination(j),
                        )
                    })
                    .unwrap_or_else(|| "No transfers yet · c copy / x cut → folder → p paste".into());
                (
                    "Transfers".into(),
                    jobs.iter()
                        .map(|(_, j)| {
                            format!(
                                "{} {} · {} → {} · {} / {}",
                                transfer_status(j),
                                transfer_name(j),
                                safe_label(j["source_host"].as_str().unwrap_or("?")),
                                safe_label(j["destination_host"].as_str().unwrap_or("?")),
                                human_size(j["bytes"].as_u64().unwrap_or(0)),
                                human_size(j["total"].as_u64().unwrap_or(0))
                            )
                        })
                        .collect(),
                    detail,
                )
            }
        };
        if matches!(dialog, Dialog::Delete(..)) {
            let rect = popup(area, 76, 10);
            frame.render_widget(Clear, rect);
            frame.render_widget(
                Block::default()
                    .borders(Borders::ALL)
                    .title(Span::styled(title, accent().add_modifier(Modifier::BOLD))),
                rect,
            );
            let inner = Rect::new(
                rect.x + 2,
                rect.y + 1,
                rect.width.saturating_sub(4),
                rect.height.saturating_sub(2),
            );
            let rows = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Min(3),
                    Constraint::Length(1),
                    Constraint::Length(2),
                ])
                .split(inner);
            frame.render_widget(
                Paragraph::new(detail)
                    .wrap(Wrap { trim: false })
                    .scroll((app.dialog_scroll, 0)),
                rows[0],
            );
            let choices = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                .split(rows[2]);
            for (i, label) in ["n  Cancel", "y  Delete"].iter().enumerate() {
                frame.render_widget(
                    Paragraph::new(*label)
                        .alignment(ratatui::layout::Alignment::Center)
                        .style(if app.dialog_selected == i && !app.dialog_detail_focus {
                            selected_style()
                        } else {
                            if i == 1 {
                                tint(Color::Red)
                            } else {
                                accent()
                            }
                        }),
                    choices[i],
                );
            }
        } else {
            let mut rect = popup(
                area,
                90,
                if matches!(dialog, Dialog::Jobs) {
                    area.height.saturating_sub(6)
                } else {
                    15
                },
            );
            if matches!(dialog, Dialog::Jobs) {
                rect.y = area.y + area.height.saturating_sub(rect.height + 5);
            }
            frame.render_widget(Clear, rect);
            let parts = Layout::default()
                .direction(Direction::Vertical)
                .constraints(if matches!(dialog, Dialog::Jobs) {
                    vec![
                        Constraint::Length((rect.height / 3).max(4)),
                        Constraint::Min(5),
                    ]
                } else {
                    vec![
                        Constraint::Length((labels.len() as u16 + 2).min(6)),
                        Constraint::Min(4),
                    ]
                })
                .split(rect);
            let selection_style = if app.dialog_detail_focus {
                accent()
            } else {
                selected_style()
            };
            if matches!(dialog, Dialog::Jobs) {
                let progress_width = if parts[0].width < 60 { 10 } else { 18 };
                let file_width = parts[0].width.saturating_sub(progress_width + 15) as usize;
                let rows = app
                    .job_rows()
                    .into_iter()
                    .map(|(_, job)| {
                        let status_color = match job["status"].as_str() {
                            Some("failed" | "incomplete") => Color::Red,
                            Some("complete") => Color::Green,
                            _ => Color::Yellow,
                        };
                        let operation = if job["operation"] == "move" {
                            "move"
                        } else {
                            "copy"
                        };
                        Row::new(vec![
                            Cell::from(Span::styled(
                                safe_label(job["status"].as_str().unwrap_or("unknown")),
                                tint(status_color),
                            )),
                            Cell::from(Line::from(vec![
                                Span::styled(
                                    format!("{operation} "),
                                    tint(if operation == "move" {
                                        Color::Magenta
                                    } else {
                                        Color::Cyan
                                    }),
                                ),
                                Span::raw(compact_path(
                                    &transfer_name(&job),
                                    file_width.saturating_sub(5),
                                )),
                            ])),
                            Cell::from(format!(
                                "{}/{}",
                                human_size(job["bytes"].as_u64().unwrap_or(0)),
                                human_size(job["total"].as_u64().unwrap_or(0))
                            )),
                        ])
                    })
                    .collect::<Vec<_>>();
                let mut state = TableState::default().with_selected(Some(app.dialog_selected));
                frame.render_stateful_widget(
                    Table::new(
                        rows,
                        [
                            Constraint::Length(10),
                            Constraint::Min(8),
                            Constraint::Length(progress_width),
                        ],
                    )
                    .header(Row::new(["Status", "File", "Progress"]).style(muted()))
                    .column_spacing(1)
                    .block(block(title, true))
                    .row_highlight_style(selection_style),
                    parts[0],
                    &mut state,
                );
            } else {
                let mut state =
                    ratatui::widgets::ListState::default().with_selected(Some(app.dialog_selected));
                frame.render_stateful_widget(
                    List::new(labels.into_iter().map(ListItem::new).collect::<Vec<_>>())
                        .block(block(title, true))
                        .highlight_style(selection_style),
                    parts[0],
                    &mut state,
                );
            }
            frame.render_widget(
                Paragraph::new(detail)
                    .wrap(Wrap { trim: false })
                    .scroll((app.dialog_scroll, 0))
                    .block(block("Details · PgUp/PgDn".into(), app.dialog_detail_focus)),
                parts[1],
            );
        }
    }
    if app.help {
        let rect = popup(area, 76, 20);
        frame.render_widget(Clear, rect);
        let key_style = accent().add_modifier(Modifier::BOLD);
        let key_row = |keys: &str, description: &str| {
            Line::from(vec![
                Span::styled(format!("{keys:<22} "), key_style),
                Span::raw(description.to_owned()),
            ])
        };
        let mut help = vec![
            key_row("Arrows / h j k l", "Navigate"),
            key_row("Ctrl+arrows / hjkl", "Move panel focus (hold Ctrl)"),
            key_row("Enter", "Open / enter session with input"),
            key_row("Escape", "Back / clear current mode"),
            key_row("Tab / Shift+Tab", "Next / previous panel"),
            key_row("/", "Search"),
            key_row("Ctrl+P", "Actions"),
            key_row("n", "New session in current folder"),
            key_row(":", "Run command in current folder"),
            key_row("Ctrl+C", "Quit cx; work keeps running"),
        ];
        if app.view == View::Files {
            help.push(Line::from(Span::styled("Files", key_style)));
            for (keys, description) in [
                ("Left/h · Right/l", "Parent / enter directory"),
                ("Space", "Select and advance"),
                ("v · u", "Visual range / clear selection"),
                (".", "Show / hide hidden files"),
                ("f", "Filter names"),
                (
                    "c/y · x",
                    "Copy / cut, then choose another folder or device",
                ),
                ("p · Y", "Paste here / clear clipboard"),
                ("t", "Transfer to… choose device, folder, then p"),
                ("T", "Transfer jobs and results"),
                ("r · d", "Rename / confirm permanent deletion"),
                ("M · o", "New folder / cycle copy conflict policy"),
                ("gg / G · 5j / 5k", "First/last file / counted movement"),
            ] {
                help.push(key_row(keys, description));
            }
        }
        if matches!(app.dialog, Some(Dialog::Jobs)) {
            help.push(Line::from(Span::styled("Transfer jobs", key_style)));
            help.extend([
                key_row("c · r", "Cancel / retry saved job"),
                key_row("Enter / Tab", "Focus details"),
                key_row("PgUp/PgDn", "Scroll details"),
            ]);
        }
        help.push(Line::default());
        help.push(Line::raw("Native terminals own their input."));
        help.push(key_row("Ctrl+]", "Return from a managed session"));
        help.push(Line::raw("External sessions keep their tmux bindings."));
        help.push(Line::default());
        help.push(Line::from(Span::styled(
            "Available actions · Ctrl+P",
            key_style,
        )));
        for (action, label) in ACTIONS
            .iter()
            .filter(|(a, _)| workspace_action(*a) && app.action_enabled(*a))
        {
            let keys = match action {
                Action::TransferTo => "t",
                Action::Jobs => "T",
                Action::Copy => "c / y",
                Action::Cut => "x",
                Action::Rename => "r",
                Action::Delete => "d",
                Action::Hidden => ".",
                Action::Filter => "f",
                Action::Select => "Space",
                Action::Visual => "v",
                Action::Paste => "p",
                Action::Help => "? / F1",
                Action::Quit => "Ctrl+C",
                _ => "",
            };
            let description = if matches!(
                action,
                Action::TransferTo
                    | Action::Copy
                    | Action::Cut
                    | Action::Rename
                    | Action::Delete
                    | Action::Hidden
                    | Action::Filter
                    | Action::Select
                    | Action::Visual
            ) {
                label
                    .rsplit_once(" · ")
                    .map(|(description, _)| description)
                    .unwrap_or(label)
            } else {
                label
            };
            help.push(key_row(keys, description));
        }
        frame.render_widget(
            Paragraph::new(help)
                .wrap(Wrap { trim: false })
                .scroll((app.help_scroll, 0))
                .block(block("Help".into(), true)),
            rect,
        );
    }
}
fn browser_entries(b: &Browser) -> Vec<Entry> {
    let mut rows = b
        .entries
        .iter()
        .enumerate()
        .filter(|(_, e)| {
            (b.show_hidden || !(e.hidden || e.name.starts_with('.')))
                && e.name.to_lowercase().contains(&b.filter.to_lowercase())
        })
        .filter_map(|(i, e)| fuzzy_score(&b.search, &e.name).map(|score| (score, i, e.clone())))
        .collect::<Vec<_>>();
    rows.sort_by(|(a, i, _), (b, j, _)| b.cmp(a).then_with(|| i.cmp(j)));
    rows.into_iter().map(|(_, _, e)| e).collect()
}
fn compact_path(path: &str, width: usize) -> String {
    let safe = safe_label(path);
    if Span::raw(&safe).width() <= width {
        return safe;
    }
    let mut tail = String::new();
    let mut used = 1;
    for c in safe.chars().rev() {
        let w = Span::raw(c.to_string()).width();
        if used + w > width {
            break;
        }
        tail.insert(0, c);
        used += w;
    }
    format!("…{tail}")
}
fn render_browser(
    frame: &mut Frame<'_>,
    b: &Browser,
    device: &Device,
    area: Rect,
    focused: bool,
    label: &str,
    clipboard: Option<&Clipboard>,
) {
    let parts = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(area);
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(vec![
                Span::styled(format!(" {label}  "), accent().add_modifier(Modifier::BOLD)),
                Span::styled(identity(device), muted()),
            ]),
            Line::from(Span::styled(
                format!(
                    " {}",
                    compact_path(&b.display_path, area.width.saturating_sub(2) as usize)
                ),
                accent(),
            )),
            Line::from(vec![
                Span::styled(
                    format!(
                        " {} ",
                        if b.visual_anchor.is_some() {
                            "VISUAL"
                        } else {
                            "NORMAL"
                        }
                    ),
                    tint(if b.visual_anchor.is_some() {
                        Color::Magenta
                    } else {
                        Color::Green
                    })
                    .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("{} selected{}", b.marked.len(), {
                        let shown = browser_entries(b)
                            .iter()
                            .filter(|e| b.marked.contains(&e.path))
                            .count();
                        if shown < b.marked.len() {
                            format!(" ({shown} shown)")
                        } else {
                            String::new()
                        }
                    }),
                    if b.marked.is_empty() {
                        muted()
                    } else {
                        tint(Color::Yellow)
                    },
                ),
                Span::styled(
                    format!(
                        " · hidden {}{}",
                        if b.show_hidden { "shown" } else { "off" },
                        if b.loading { " · loading" } else { "" }
                    ),
                    muted(),
                ),
            ]),
        ]),
        parts[0],
    );
    if let Some(preview) = &b.preview {
        frame.render_widget(
            Paragraph::new(preview.as_str())
                .wrap(Wrap { trim: false })
                .scroll((b.preview_scroll, 0))
                .block(block("Preview · Escape back".into(), focused)),
            parts[1],
        );
    } else {
        let rows = browser_entries(b);
        if rows.is_empty() {
            frame.render_widget(
                Paragraph::new(if b.loading {
                    " Loading…"
                } else if !b.search.is_empty() || !b.filter.is_empty() {
                    " No matches · Escape clears filter"
                } else {
                    " Empty folder · . shows hidden files"
                })
                .style(muted())
                .block(block("Files".into(), focused)),
                parts[1],
            );
        } else {
            let table_rows = rows
                .iter()
                .map(|e| {
                    let clip = clipboard.filter(|c| {
                        c.device == b.device && c.entries.iter().any(|x| x.path == e.path)
                    });
                    let (marker, style) = if b.marked.contains(&e.path) {
                        (if ascii() { "*" } else { "●" }, tint(Color::Yellow))
                    } else if let Some(c) = clip {
                        (
                            if c.cut { "x" } else { "c" },
                            tint(if c.cut { Color::Magenta } else { Color::Green }),
                        )
                    } else {
                        (" ", Style::default())
                    };
                    Row::new(vec![
                        Cell::from(marker).style(style.add_modifier(Modifier::BOLD)),
                        Cell::from(match e.kind.as_str() {
                            "directory" => "/",
                            "symlink" => "@",
                            _ => " ",
                        })
                        .style(accent()),
                        Cell::from(compact_path(
                            &e.name,
                            area.width.saturating_sub(17) as usize,
                        ))
                        .style(if e.kind == "directory" {
                            tint(Color::Blue).add_modifier(Modifier::BOLD)
                        } else {
                            Style::default()
                        }),
                        Cell::from(if e.kind == "file" {
                            human_size(e.size)
                        } else {
                            String::new()
                        })
                        .style(muted()),
                    ])
                })
                .collect::<Vec<_>>();
            let mut state = TableState::default().with_selected(Some(b.selected));
            frame.render_stateful_widget(
                Table::new(
                    table_rows,
                    [
                        Constraint::Length(1),
                        Constraint::Length(1),
                        Constraint::Min(1),
                        Constraint::Length(8),
                    ],
                )
                .column_spacing(1)
                .block(block(format!("{} items", rows.len()), focused))
                .row_highlight_style(selected_style())
                .highlight_symbol(if ascii() { "> " } else { "› " }),
                parts[1],
                &mut state,
            );
        }
    }
    let bottom = if let Some(c) = clipboard {
        format!(
            " {} {} · {}{}",
            if c.cut { "CUT" } else { "COPY" },
            c.entries.len(),
            safe_label(&c.source_label),
            if b.filter.is_empty() {
                " · p paste here".into()
            } else {
                format!(" · f {}", safe_label(&b.filter))
            }
        )
    } else if !b.filter.is_empty() {
        format!(" f Filter: {}", safe_label(&b.filter))
    } else {
        " Space select · v range · . hidden".into()
    };
    frame.render_widget(
        Paragraph::new(bottom).style(if clipboard.is_some() {
            tint(Color::Yellow)
        } else {
            muted()
        }),
        parts[2],
    );
}
fn tint(color: Color) -> Style {
    if std::env::var_os("NO_COLOR").is_some() {
        Style::default()
    } else {
        Style::default().fg(color)
    }
}
fn human_size(size: u64) -> String {
    if size < 1024 {
        format!("{size} B")
    } else if size < 1024 * 1024 {
        format!("{:.1} KiB", size as f64 / 1024.0)
    } else if size < 1024 * 1024 * 1024 {
        format!("{:.1} MiB", size as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.1} GiB", size as f64 / (1024.0 * 1024.0 * 1024.0))
    }
}
fn transfer_name(job: &Value) -> String {
    let path = job["source_display"]
        .as_str()
        .or(job["source_path"].as_str())
        .unwrap_or("file");
    safe_label(path.rsplit('/').find(|s| !s.is_empty()).unwrap_or(path))
}
fn transfer_status(job: &Value) -> String {
    let state = safe_label(job["status"].as_str().unwrap_or("unknown"));
    let skipped = job["skipped"].as_u64().unwrap_or(0);
    if state == "complete" && skipped > 0 {
        format!("complete · {skipped} skipped")
    } else {
        state
    }
}
fn transfer_destination(job: &Value) -> String {
    let path = job["actual_destinations"]
        .as_array()
        .and_then(|a| a.first())
        .and_then(Value::as_str)
        .or(job["destination_display"].as_str())
        .or(job["destination_path"].as_str())
        .unwrap_or("");
    match crate::files::decode_path(path) {
        Ok(p) => safe_label(&crate::files::display(&p.to_string_lossy())),
        Err(_) => safe_label(path),
    }
}

fn popup(area: Rect, width: u16, height: u16) -> Rect {
    let w = width.min(area.width.saturating_sub(2));
    let h = height.min(area.height.saturating_sub(2));
    Rect::new(
        area.x + (area.width - w) / 2,
        area.y + (area.height - h) / 2,
        w,
        h,
    )
}
fn network_summary(value: &Value) -> String {
    let internet = &value["internet"];
    let age = transport::now().saturating_sub(internet["observed_at"].as_u64().unwrap_or(0));
    let stale = internet["stale"].as_bool().unwrap_or(true) || age > 90;
    let reachable = internet["state"] == "reachable" && !stale;
    let mut lines = vec![
        format!(
            "Internet {} · {}{}",
            if reachable {
                if ascii() {
                    "+"
                } else {
                    "●"
                }
            } else {
                "?"
            },
            if reachable {
                "reachable (ICMP)"
            } else {
                "not confirmed"
            },
            if internet["observed_at"].as_u64().unwrap_or(0) > 0 {
                format!(" · {age}s ago{}", if stale { " · stale" } else { "" })
            } else {
                String::new()
            }
        ),
        "DNS / HTTPS and overlay access: unknown".to_string(),
    ];
    if let Some(backend) = value.get("backend").and_then(Value::as_str) {
        lines.push(format!("Backend: {}", safe_label(backend)));
    }
    let interfaces = value.get("interfaces");
    if interfaces
        .and_then(|v| v.get("state"))
        .and_then(Value::as_str)
        == Some("unknown")
    {
        lines.push("\nInterfaces: unknown · observation unavailable".into());
    }
    if let Some(interfaces) = interfaces.and_then(|v| {
        v.as_array()
            .or_else(|| v.get("data").and_then(Value::as_array))
    }) {
        lines.push("\nInterfaces".into());
        for interface in interfaces.iter().take(24) {
            let name = interface
                .get("display_name")
                .or_else(|| interface.get("name"))
                .or_else(|| interface.get("ifname"))
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            let state = interface
                .get("state")
                .or_else(|| interface.get("operstate"))
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            lines.push(format!("  {} · {}", safe_label(name), safe_label(state)));
        }
    }
    let sharing = value.get("sharing");
    let state = sharing
        .and_then(|v| v.get("state"))
        .and_then(Value::as_str)
        .unwrap_or("unsupported");
    let reason = sharing
        .and_then(|v| v.get("reason"))
        .and_then(Value::as_str)
        .unwrap_or("No supported mutation backend is available");
    lines.push(format!(
        "\nSharing: {}\n{}",
        safe_label(state),
        safe_label(reason)
    ));
    lines.join("\n")
}
fn load_cache(app: &mut App) {
    let path = store::state_dir().join("ui-sessions.json");
    if std::fs::metadata(&path).map_or(true, |m| m.len() > 1024 * 1024) {
        return;
    }
    if let Ok(data) = std::fs::read(path) {
        if let Ok(cache) = serde_json::from_slice::<HashMap<String, Vec<Session>>>(&data) {
            for (d, w) in app.devices.iter().zip(&mut app.work) {
                if let Some(s) = cache.get(&d.id) {
                    w.sessions = s.clone();
                }
            }
        }
    }
}
fn save_cache(app: &App) {
    // Only launch metadata is retained; no prompts, transcripts, environment or provider secrets.
    let cache: HashMap<_, _> = app
        .devices
        .iter()
        .zip(&app.work)
        .map(|(d, w)| (d.id.clone(), w.sessions.clone()))
        .collect();
    if let Ok(data) = serde_json::to_vec(&cache) {
        if data.len() > 1024 * 1024 {
            return;
        }
        if let Ok(dir) = store::ensure() {
            use std::{io::Write, os::unix::fs::OpenOptionsExt};
            let temp = dir.join(format!("ui-sessions.{}.tmp", std::process::id()));
            if let Ok(mut file) = std::fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .mode(0o600)
                .open(&temp)
            {
                if file.write_all(&data).is_ok() {
                    let _ = std::fs::rename(&temp, dir.join("ui-sessions.json"));
                }
                let _ = std::fs::remove_file(temp);
            }
        }
    }
}

struct Screen {
    terminal: Terminal<CrosstermBackend<io::Stdout>>,
    active: bool,
}
impl Screen {
    fn new() -> Result<Self> {
        terminal::enable_raw_mode()?;
        if let Err(e) = execute!(io::stdout(), EnterAlternateScreen) {
            let _ = terminal::disable_raw_mode();
            return Err(e.into());
        }
        let terminal = match Terminal::new(CrosstermBackend::new(io::stdout())) {
            Ok(t) => t,
            Err(e) => {
                let _ = terminal::disable_raw_mode();
                let _ = execute!(io::stdout(), LeaveAlternateScreen);
                return Err(e.into());
            }
        };
        Ok(Self {
            terminal,
            active: true,
        })
    }
    fn suspend(&mut self) -> Result<()> {
        if self.active {
            self.active = false;
            let raw = terminal::disable_raw_mode();
            let leave = execute!(self.terminal.backend_mut(), LeaveAlternateScreen);
            let cursor = self.terminal.show_cursor();
            raw?;
            leave?;
            cursor?;
        }
        Ok(())
    }
    fn resume(&mut self) -> Result<()> {
        terminal::enable_raw_mode()?;
        self.active = true;
        execute!(self.terminal.backend_mut(), EnterAlternateScreen)?;
        self.terminal.clear()?;
        Ok(())
    }
}
impl Drop for Screen {
    fn drop(&mut self) {
        let _ = self.suspend();
    }
}

const MAX_RESTART_STATE: usize = 4 * 1024 * 1024;
#[derive(serde::Serialize, serde::Deserialize)]
struct RestartState {
    schema: u32,
    expires_at: u64,
    device_ids: Vec<String>,
    device: usize,
    focus: Focus,
    view: View,
    selected: usize,
    #[serde(default)]
    selected_session: Option<(String, String)>,
    side_selected: usize,
    search: String,
    browser: Option<Browser>,
    other_browser: Option<Browser>,
    destination_active: bool,
    conflict: usize,
    launch_provider: Option<String>,
    clipboard: Option<Clipboard>,
    submitted: HashMap<String, crate::model::TransferSpec>,
}
fn restart_browser(browser: &Option<Browser>) -> Option<Browser> {
    browser.as_ref().map(|browser| {
        let mut snapshot = browser.clone();
        snapshot.restore_selection = browser_entries(browser)
            .get(browser.selected)
            .map(|entry| entry.path.clone());
        snapshot
    })
}
fn save_restart(app: &App) -> Result<String> {
    save_restart_at(app, &store::ensure()?)
}
fn save_restart_at(app: &App, directory: &std::path::Path) -> Result<String> {
    use std::{io::Write, os::unix::fs::OpenOptionsExt};
    let state = RestartState {
        schema: 1,
        expires_at: transport::now() + 300,
        device_ids: app.devices.iter().map(|d| d.id.clone()).collect(),
        device: app.device,
        focus: app.focus,
        view: app.view,
        selected: app.selected,
        selected_session: app
            .selected_session()
            .map(|(device, session)| (app.devices[device].id.clone(), session.id)),
        side_selected: app.side_selected,
        search: app.search.clone(),
        browser: restart_browser(&app.browser),
        other_browser: restart_browser(&app.other_browser),
        destination_active: app.destination_active,
        conflict: app.conflict,
        launch_provider: app.launch_provider.clone(),
        clipboard: app.clipboard.clone(),
        submitted: app.submitted.clone(),
    };
    let bytes = serde_json::to_vec(&state)?;
    anyhow::ensure!(
        bytes.len() <= MAX_RESTART_STATE,
        "Workspace too large for automatic restart; return to Sessions first"
    );
    let name = format!("viewer-restart-{}.json", unique_key());
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(directory.join(&name))?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    Ok(name)
}
fn restore_restart(app: &mut App, name: &str) -> Result<()> {
    restore_restart_at(app, name, &store::ensure()?)
}
fn restore_restart_at(app: &mut App, name: &str, directory: &std::path::Path) -> Result<()> {
    use std::{
        io::Read,
        os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    };
    anyhow::ensure!(
        name.starts_with("viewer-restart-")
            && name.ends_with(".json")
            && name.len() < 100
            && name
                .bytes()
                .all(|b| b.is_ascii_digit() || b"viewer-sta.json".contains(&b)),
        "Invalid restart state name"
    );
    let path = directory.join(name);
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&path)?;
    let metadata = file.metadata()?;
    anyhow::ensure!(
        metadata.is_file()
            && metadata.uid() == unsafe { libc::geteuid() }
            && metadata.permissions().mode() & 0o077 == 0
            && metadata.len() <= MAX_RESTART_STATE as u64,
        "Unsafe restart state"
    );
    let mut bytes = Vec::new();
    file.by_ref()
        .take(MAX_RESTART_STATE as u64 + 1)
        .read_to_end(&mut bytes)?;
    anyhow::ensure!(
        bytes.len() <= MAX_RESTART_STATE,
        "Restart state exceeds limit"
    );
    let mut state: RestartState = serde_json::from_slice(&bytes)?;
    anyhow::ensure!(
        state.schema == 1
            && state.expires_at >= transport::now()
            && state.expires_at <= transport::now() + 300,
        "Restart state expired or unsupported"
    );
    let remap = |old: usize| {
        state
            .device_ids
            .get(old)
            .and_then(|id| app.devices.iter().position(|d| &d.id == id))
    };
    app.device = if state.device == 0 {
        0
    } else {
        remap(state.device - 1).map(|d| d + 1).unwrap_or(0)
    };
    for browser in [&mut state.browser, &mut state.other_browser] {
        if let Some(b) = browser {
            if let Some(device) = remap(b.device) {
                b.device = device;
            } else {
                *browser = None;
            }
        }
    }
    app.clipboard = state.clipboard.and_then(|mut c| {
        c.device = remap(c.device)?;
        Some(c)
    });
    app.browser = state.browser;
    app.other_browser = state.other_browser;
    app.view = if state.view == View::Files && app.browser.is_none() {
        View::Work
    } else {
        state.view
    };
    app.focus = state.focus;
    app.search = state.search;
    app.selected = state
        .selected_session
        .and_then(|(device, id)| {
            app.session_rows()
                .iter()
                .position(|(d, s)| app.devices[*d].id == device && s.id == id)
        })
        .unwrap_or(
            state
                .selected
                .min(app.session_rows().len().saturating_sub(1)),
        );
    app.side_selected = state.side_selected;
    app.destination_active = state.destination_active;
    app.conflict = state.conflict.min(2);
    app.launch_provider = state.launch_provider;
    app.submitted = state.submitted;
    std::fs::remove_file(path)?;
    Ok(())
}
fn can_restart(app: &App) -> bool {
    app.input.is_none()
        && app.dialog.is_none()
        && !app.help
        && !app.creating
        && app.pending_attach.is_none()
        && app.pending_command.is_none()
        && app.pending_add.is_none()
        && app.pending_requests.get() == 0
        && !app.file_busy
        && app.file_queue.is_empty()
        && app.browser.as_ref().is_none_or(|b| b.preview.is_none())
}

fn restart_ready(app: &App, idle: Duration, pending_input: bool) -> bool {
    !pending_input && idle >= Duration::from_secs(3) && can_restart(app)
}
enum UpdatePhase {
    Idle,
    Checking,
    Ready(crate::update::UpdatePlan),
    Installing,
    Installed(std::path::PathBuf),
}
enum UpdateEvent {
    Checked(bool, Result<crate::update::CheckOutcome>),
    Installed(Result<std::path::PathBuf>),
}
fn start_update_check(tx: &mpsc::Sender<UpdateEvent>, force: bool) {
    let tx = tx.clone();
    thread::spawn(move || {
        let result = crate::update::check(force);
        let _ = tx.send(UpdateEvent::Checked(force, result));
    });
}

// Four workers permit independent hosts to progress while preserving FIFO per SSH target.
// Both the incoming channel and scheduler backlog are bounded. Sleeping workers use a
// condition variable, so an idle fleet causes no polling or redraw loop.
fn start_task_workers(rx: mpsc::Receiver<Task>, replies: mpsc::Sender<Reply>) {
    use std::sync::{Arc, Condvar, Mutex};
    #[derive(Default)]
    struct Queue {
        tasks: VecDeque<Task>,
        busy: BTreeSet<String>,
        closed: bool,
    }
    fn endpoint(task: &Task) -> String {
        task.execution.target.clone().unwrap_or_else(|| {
            if matches!(
                task.op,
                Operation::List { .. } | Operation::ListPage { .. } | Operation::Preview { .. }
            ) {
                "<local-files-read>".into()
            } else {
                "<local>".into()
            }
        })
    }
    let shared = Arc::new((Mutex::new(Queue::default()), Condvar::new()));
    let input = shared.clone();
    thread::spawn(move || {
        let (lock, changed) = &*input;
        while let Ok(task) = rx.recv() {
            let mut queue = lock.lock().unwrap();
            while queue.tasks.len() >= 32 {
                queue = changed.wait(queue).unwrap();
            }
            queue.tasks.push_back(task);
            changed.notify_all();
        }
        lock.lock().unwrap().closed = true;
        changed.notify_all();
    });
    for _ in 0..4 {
        let shared = shared.clone();
        let replies = replies.clone();
        thread::spawn(move || {
            let (lock, changed) = &*shared;
            loop {
                let (task, host) = {
                    let mut queue = lock.lock().unwrap();
                    loop {
                        if let Some(index) = queue
                            .tasks
                            .iter()
                            .position(|task| !queue.busy.contains(&endpoint(task)))
                        {
                            let task = queue.tasks.remove(index).unwrap();
                            let host = endpoint(&task);
                            queue.busy.insert(host.clone());
                            changed.notify_all();
                            break (task, host);
                        }
                        if queue.closed && queue.tasks.is_empty() {
                            return;
                        }
                        queue = changed.wait(queue).unwrap();
                    }
                };
                let result = transport::request(&task.execution, task.op.clone());
                let delivered = replies
                    .send(Reply {
                        device: task.device,
                        op: task.op,
                        generation: task.generation,
                        result,
                    })
                    .is_ok();
                lock.lock().unwrap().busy.remove(&host);
                changed.notify_all();
                if !delivered {
                    return;
                }
            }
        });
    }
}

pub fn run() -> Result<()> {
    run_restored(None)
}
pub fn run_restored(restore: Option<&str>) -> Result<()> {
    let stopping = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    for sig in [
        signal_hook::consts::SIGTERM,
        signal_hook::consts::SIGHUP,
        signal_hook::consts::SIGINT,
    ] {
        signal_hook::flag::register(sig, stopping.clone())?;
    }
    let devices = store::devices()?;
    anyhow::ensure!(!devices.is_empty(), "No local device available");
    let (tx, rx) = mpsc::sync_channel::<Task>(32);
    let (result_tx, result_rx) = mpsc::channel::<Reply>();
    start_task_workers(rx, result_tx);
    let mut app = App::new(devices, tx);
    load_cache(&mut app);
    if let Some(name) = restore {
        if let Err(error) = restore_restart(&mut app, name) {
            app.notice = safe_text(&format!("Update restored workspace defaults: {error:#}"));
        }
    }
    if restore.is_some() && app.view == View::Files {
        app.refresh_browser();
    }
    app.refresh_work();
    for d in 0..app.devices.len() {
        app.check_providers(d);
        app.send(d, Operation::TransferJobs);
    }
    let mut screen = Screen::new().context("open terminal workspace")?;
    let mut dirty = true;
    let (update_tx, update_rx) = mpsc::channel::<UpdateEvent>();
    start_update_check(&update_tx, false);
    let mut update_phase = UpdatePhase::Checking;
    let mut update_report_requested = false;
    let mut last_update_check = Instant::now();
    let mut last_interaction = Instant::now();
    let mut last_refresh = Instant::now();
    let mut last_jobs = Instant::now();
    while !app.quit && !stopping.load(std::sync::atomic::Ordering::Relaxed) {
        app.start_next_file_action();
        if app.force_update {
            app.force_update = false;
            if matches!(update_phase, UpdatePhase::Idle) {
                start_update_check(&update_tx, true);
                update_phase = UpdatePhase::Checking;
                last_update_check = Instant::now();
                app.notice = "Checking verified cx releases…".into();
                dirty = true;
            } else if matches!(update_phase, UpdatePhase::Checking) {
                update_report_requested = true;
                app.notice = "Checking verified cx releases…".into();
                dirty = true;
            }
        }
        while let Ok(event) = update_rx.try_recv() {
            match event {
                UpdateEvent::Checked(force, result) => {
                    let force = force || std::mem::take(&mut update_report_requested);
                    update_phase = match result {
                        Ok(crate::update::CheckOutcome::Ready(plan)) => {
                            app.notice = format!(
                                "cx {} ready · updating when workspace is idle",
                                safe_label(&plan.version)
                            );
                            dirty = true;
                            UpdatePhase::Ready(plan)
                        }
                        other => {
                            if force {
                                app.notice = match other {
                                    Ok(crate::update::CheckOutcome::Offline) => {
                                        "Update service unreachable · current cx kept".into()
                                    }
                                    Ok(crate::update::CheckOutcome::Unavailable(message)) => {
                                        safe_text(&message)
                                    }
                                    Ok(crate::update::CheckOutcome::Skipped) => {
                                        "Another update check is running".into()
                                    }
                                    Ok(crate::update::CheckOutcome::Current) => {
                                        "cx is current".into()
                                    }
                                    Err(_) => "Update unavailable · current cx kept".into(),
                                    _ => unreachable!(),
                                };
                                dirty = true;
                            }
                            UpdatePhase::Idle
                        }
                    };
                }
                UpdateEvent::Installed(result) => {
                    update_phase = match result {
                        Ok(path) => UpdatePhase::Installed(path),
                        Err(_) => {
                            app.notice = "Update could not install · current cx kept".into();
                            dirty = true;
                            UpdatePhase::Idle
                        }
                    };
                }
            }
        }
        if matches!(
            update_phase,
            UpdatePhase::Ready(_) | UpdatePhase::Installed(_)
        ) && restart_ready(
            &app,
            last_interaction.elapsed(),
            event::poll(Duration::ZERO)?,
        ) {
            if matches!(update_phase, UpdatePhase::Ready(_)) {
                if let UpdatePhase::Ready(plan) =
                    std::mem::replace(&mut update_phase, UpdatePhase::Installing)
                {
                    let tx = update_tx.clone();
                    thread::spawn(move || {
                        let _ = tx.send(UpdateEvent::Installed(crate::update::install(&plan)));
                    });
                    app.notice = "Installing verified cx update…".into();
                    dirty = true;
                }
            } else if let UpdatePhase::Installed(path) = &update_phase {
                match save_restart(&app) {
                    Ok(name) => {
                        // A key can arrive while serializing the snapshot; consume it before takeover.
                        if event::poll(Duration::ZERO)? {
                            let _ = std::fs::remove_file(store::state_dir().join(name));
                            last_interaction = Instant::now();
                            continue;
                        }
                        use std::os::unix::process::CommandExt;
                        screen.suspend()?;
                        let error = std::process::Command::new(path)
                            .arg("restart")
                            .arg(&name)
                            .exec();
                        let _ = std::fs::remove_file(store::state_dir().join(name));
                        screen.resume()?;
                        app.notice =
                            safe_text(&format!("Update installed; reopen cx to use it: {error}"));
                        dirty = true;
                        update_phase = UpdatePhase::Idle;
                    }
                    Err(_) => {
                        app.notice="Update installed · workspace too large to restore; reopen cx when convenient".into();
                        dirty = true;
                        update_phase = UpdatePhase::Idle;
                    }
                }
            }
        }
        let mut cache_changed = false;
        while let Ok(reply) = result_rx.try_recv() {
            cache_changed |= matches!(reply.op, Operation::Sessions) && reply.result.is_ok();
            app.apply(reply);
            dirty = true;
        }
        if cache_changed {
            save_cache(&app);
        }
        if let Some(target) = app.pending_add.take() {
            screen.suspend()?;
            let result = crate::add(&target);
            screen.resume()?;
            match result {
                Ok(()) => {
                    let updated = store::devices()?;
                    let old: HashMap<_, _> = app
                        .devices
                        .iter()
                        .zip(app.work.drain(..))
                        .map(|(d, w)| (d.id.clone(), w))
                        .collect();
                    app.work = updated
                        .iter()
                        .map(|d| {
                            old.get(&d.id)
                                .map(|w| Cached {
                                    sessions: w.sessions.clone(),
                                    loading: false,
                                    error: w.error.clone(),
                                    fetched: w.fetched,
                                })
                                .unwrap_or(Cached {
                                    sessions: vec![],
                                    loading: false,
                                    error: None,
                                    fetched: 0,
                                })
                        })
                        .collect();
                    app.devices = updated;
                    app.notice = format!("Added {}", safe_label(&target));
                    app.refresh_work();
                }
                Err(e) => app.notice = safe_text(&format!("Enrollment failed: {e:#}")),
            }
            dirty = true;
        }
        if let Some((device, command)) = app.pending_command.take() {
            screen.suspend()?;
            let result = sessions::run_command(&device, &command);
            screen.resume()?;
            last_interaction = Instant::now();
            app.notice = match result {
                Ok(()) => format!("Returned from {}", identity(&device)),
                Err(e) => safe_text(&format!("Command failed: {e:#}")),
            };
            if app.view == View::Files {
                app.refresh();
            }
            dirty = true;
        }
        if let Some((d, session, observe)) = app.pending_attach.take() {
            screen.suspend()?;
            let result = sessions::attach(&app.devices[d], &session, observe);
            screen.resume()?;
            last_interaction = Instant::now();
            app.notice = match result {
                Ok(()) => format!(
                    "Returned from {} · session remains on execution host",
                    identity(&app.devices[d])
                ),
                Err(e) => safe_text(&format!("Attachment failed: {e:#}")),
            };
            app.refresh_work();
            dirty = true;
        }
        if dirty {
            screen.terminal.draw(|frame| render(frame, &app))?;
            dirty = false;
        }
        if event::poll(Duration::from_millis(100))? {
            match event::read()? {
                Event::Key(k) => {
                    last_interaction = Instant::now();
                    app.key(k);
                }
                Event::Resize(_, _) => {}
                _ => {}
            }
            dirty = true;
        }
        if last_jobs.elapsed() >= Duration::from_secs(2) {
            let active = app
                .jobs
                .values()
                .filter_map(|v| v["jobs"].as_array())
                .flatten()
                .any(|j| {
                    matches!(
                        j["status"].as_str(),
                        Some("queued" | "running" | "submitting")
                    )
                });
            if active {
                if let Some(local) = app.devices.iter().position(|d| d.target.is_none()) {
                    app.send(local, Operation::TransferJobs);
                }
            }
            last_jobs = Instant::now();
        }
        if matches!(update_phase, UpdatePhase::Idle)
            && last_update_check.elapsed() >= Duration::from_secs(3600)
        {
            start_update_check(&update_tx, false);
            update_phase = UpdatePhase::Checking;
            last_update_check = Instant::now();
        }
        // Bounded fleet refresh; idle does not continuously redraw.
        if last_refresh.elapsed() >= Duration::from_secs(30) {
            if let Some(browser) = &app.browser {
                app.check_providers(browser.device);
            }
            if let Some(Dialog::Provider(device, _)) = app.dialog {
                app.check_providers(device);
            }
            if app.view == View::Work {
                app.refresh_work();
            } else if app.view == View::Network {
                app.refresh();
            }
            if app.browser.is_some() {
                if let Some(local) = app.devices.iter().position(|d| d.target.is_none()) {
                    app.send(local, Operation::TransferJobs);
                }
            }
            last_refresh = Instant::now();
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    fn app() -> App {
        let (tx, _) = mpsc::sync_channel(32);
        App::new(
            vec![Device {
                id: "local".into(),
                name: "workstation".into(),
                target: None,
                account: "tester".into(),
                host: "workstation".into(),
                status: "unknown".into(),
                observed_at: 0,
            }],
            tx,
        )
    }
    fn queued_app() -> (App, mpsc::Receiver<Task>) {
        let (tx, rx) = mpsc::sync_channel(64);
        let mut devices = app().devices;
        let mut remote = devices[0].clone();
        remote.id = "remote".into();
        remote.name = "laptop".into();
        remote.account = "peace".into();
        remote.host = "laptop".into();
        remote.target = Some("laptop".into());
        devices.push(remote);
        let mut a = App::new(devices, tx);
        for d in 0..a.devices.len() {
            a.providers
                .insert(d, (vec!["claude".into(), "codex".into()], transport::now()));
        }
        (a, rx)
    }
    fn file_app() -> (App, mpsc::Receiver<Task>) {
        let (mut a, rx) = queued_app();
        let mut b = Browser::new(0, "/files".into());
        b.entries = ["alpha.txt", "beta.txt", "gamma.txt", ".secret"]
            .iter()
            .map(|name| Entry {
                name: (*name).into(),
                path: format!("/files/{name}"),
                kind: "file".into(),
                size: 15,
                identity: Some(format!("identity-{name}")),
                hidden: name.starts_with('.'),
                rename_name: Some((*name).into()),
            })
            .collect();
        a.browser = Some(b);
        a.view = View::Files;
        (a, rx)
    }
    fn press(a: &mut App, c: char) {
        a.key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
    }
    #[test]
    fn file_device_selection_retargets_only_the_focused_location() {
        let (mut a, rx) = file_app();
        a.device = 1;
        a.focus = Focus::Devices;
        a.other_browser = Some(Browser::new(0, "/destination".into()));
        press(&mut a, 'j');
        assert_eq!(a.device, 2);
        assert_eq!(
            a.browser.as_ref().unwrap().device,
            1,
            "Files must follow selected device"
        );
        assert_eq!(a.browser.as_ref().unwrap().path, "~");
        assert_eq!(a.other_browser.as_ref().unwrap().path, "/destination");
        assert!(a.focus == Focus::Devices);
        assert!(rx
            .try_iter()
            .any(|t| t.device == 1 && matches!(t.op, Operation::List { .. })));
        a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(a.focus == Focus::Workspace);
        assert_eq!(a.browser.as_ref().unwrap().device, 1);
    }
    #[test]
    fn transfer_shortcut_captures_selection_and_opens_destination_picker() {
        let (mut a, _rx) = file_app();
        press(&mut a, ' ');
        press(&mut a, 't');
        assert!(matches!(
            a.dialog,
            Some(Dialog::Device(ChooseDevice::Destination))
        ));
        assert_eq!(a.clipboard.as_ref().unwrap().entries[0].name, "alpha.txt");
        assert!(!a.clipboard.as_ref().unwrap().cut);
        assert_eq!(a.browser.as_ref().unwrap().device, 0);
        a.chosen_device(1, ChooseDevice::Destination);
        assert_eq!(a.browser.as_ref().unwrap().device, 1);
        assert_eq!(a.other_browser.as_ref().unwrap().device, 0);
        assert_eq!(a.device, 2);
        a.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        a.view = View::Files;
        a.focus = Focus::Workspace;
        press(&mut a, 'T');
        assert!(matches!(a.dialog, Some(Dialog::Jobs)));
    }
    #[test]
    fn device_switch_restores_folder_and_does_not_consume_cut_clipboard() {
        let (mut a, _rx) = file_app();
        press(&mut a, 'x');
        a.device = 1;
        a.focus = Focus::Devices;
        a.browser.as_mut().unwrap().preview = Some("preview".into());
        press(&mut a, 'j');
        assert_eq!(a.browser.as_ref().unwrap().device, 1);
        press(&mut a, 'k');
        assert_eq!(a.browser.as_ref().unwrap().device, 0);
        assert_eq!(a.browser.as_ref().unwrap().path, "/files");
        assert!(a.clipboard.as_ref().unwrap().cut);
        assert_eq!(a.clipboard.as_ref().unwrap().device, 0);
        a.execute(Action::TransferTo);
        assert!(a.clipboard.as_ref().unwrap().cut);
        assert!(matches!(
            a.dialog,
            Some(Dialog::Device(ChooseDevice::Destination))
        ));
    }
    #[test]
    fn escape_clearing_selection_does_not_discard_inflight_directory_listing() {
        let (mut a, _rx) = file_app();
        let b = a.browser.as_mut().unwrap();
        b.loading = true;
        b.marked.insert("/files/alpha.txt".into());
        let generation = a.generation;
        a.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        a.apply(Reply { device: 0, op: Operation::List { path: "/files".into() }, generation,
            result: Ok(serde_json::json!({"path":"/files", "entries":[{"name":"fresh.txt","path":"/files/fresh.txt","kind":"file","size":1}]})) });
        assert!(!a.browser.as_ref().unwrap().loading);
        assert_eq!(a.visible_entries()[0].name, "fresh.txt");
    }
    #[test]
    fn opening_remote_files_from_all_devices_keeps_execution_context_on_enter() {
        let (mut a, rx) = queued_app();
        a.device = 0;
        a.open_browser(1, "/project".into());
        assert_eq!(a.device, 2);
        a.focus = Focus::Devices;
        a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(a.browser.as_ref().unwrap().device, 1);
        assert_eq!(a.browser.as_ref().unwrap().path, "/project");
        assert!(!rx
            .try_iter()
            .any(|t| t.device == 0 && matches!(t.op, Operation::List { .. })));
    }
    #[test]
    fn yazi_motion_counts_and_delete_yes_no_do_not_leak_into_inputs() {
        let (mut a, rx) = file_app();
        press(&mut a, '2');
        press(&mut a, 'j');
        assert_eq!(a.browser.as_ref().unwrap().selected, 2);
        press(&mut a, 'g');
        press(&mut a, 'g');
        assert_eq!(a.browser.as_ref().unwrap().selected, 0);
        press(&mut a, 'G');
        assert_eq!(a.browser.as_ref().unwrap().selected, 2);
        press(&mut a, 'd');
        press(&mut a, 'n');
        assert!(a.dialog.is_none() && rx.try_recv().is_err());
        press(&mut a, 'd');
        press(&mut a, 'y');
        assert!(matches!(
            rx.try_recv().unwrap().op,
            Operation::Remove { .. }
        ));
        press(&mut a, 'f');
        press(&mut a, '5');
        press(&mut a, 'j');
        assert_eq!(a.text, "5j");
    }
    #[test]
    fn space_advances_range_shrinks_and_escape_preserves_then_clears_marks() {
        let (mut a, _) = file_app();
        press(&mut a, ' ');
        assert_eq!(a.browser.as_ref().unwrap().selected, 1);
        press(&mut a, 'v');
        press(&mut a, 'j');
        assert_eq!(a.browser.as_ref().unwrap().marked.len(), 3);
        press(&mut a, 'k');
        assert_eq!(a.browser.as_ref().unwrap().marked.len(), 2);
        a.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(a.browser.as_ref().unwrap().visual_anchor.is_none());
        assert_eq!(a.browser.as_ref().unwrap().marked.len(), 2);
        a.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(a.browser.as_ref().unwrap().marked.is_empty() && a.view == View::Files);
    }
    #[test]
    fn hidden_and_filter_do_not_lose_selected_paths_or_turn_text_into_actions() {
        let (mut a, _) = file_app();
        assert_eq!(a.visible_entries().len(), 3);
        press(&mut a, '.');
        assert_eq!(a.visible_entries().len(), 4);
        a.browser.as_mut().unwrap().selected = 3;
        press(&mut a, ' ');
        press(&mut a, '.');
        assert_eq!(a.chosen_entries()[0].name, ".secret");
        press(&mut a, 'f');
        press(&mut a, 'b');
        assert_eq!(a.visible_entries().len(), 1);
        assert!(a
            .browser
            .as_ref()
            .unwrap()
            .marked
            .contains("/files/.secret"));
        press(&mut a, 'd');
        assert!(a.dialog.is_none() && a.input == Some(Input::Filter));
        a.key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
        assert_eq!(a.visible_entries().len(), 3);
        a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(a.input.is_none() && a.browser.as_ref().unwrap().preview.is_none());
    }
    #[test]
    fn delete_defaults_to_cancel_and_uses_captured_identity() {
        let (mut a, rx) = file_app();
        press(&mut a, 'd');
        a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(rx.try_recv().is_err());
        press(&mut a, 'd');
        a.browser.as_mut().unwrap().entries[0].identity = Some("replacement".into());
        press(&mut a, 'j');
        a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(
            matches!(rx.try_recv().unwrap().op, Operation::Remove { path, expected_identity: Some(id) }
            if path == "/files/alpha.txt" && id == "identity-alpha.txt")
        );
    }
    #[test]
    fn rename_is_one_component_and_input_owns_file_shortcuts() {
        let (mut a, rx) = file_app();
        press(&mut a, 'r');
        assert_eq!(a.text, "alpha.txt");
        a.key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
        for c in "../oops".chars() {
            press(&mut a, c);
        }
        a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(a.input == Some(Input::Rename) && rx.try_recv().is_err());
        a.key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
        for c in "cxd.txt".chars() {
            press(&mut a, c);
        }
        a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(
            matches!(rx.try_recv().unwrap().op, Operation::Rename { name, expected_identity: Some(_), .. } if name == "cxd.txt")
        );
        assert!(a.clipboard.is_none());
    }
    #[test]
    fn copy_batch_stays_in_folder_and_paste_is_nonmodal_and_serialized() {
        let (mut a, rx) = file_app();
        press(&mut a, ' ');
        press(&mut a, ' ');
        press(&mut a, 'c');
        assert!(a.dialog.is_none());
        assert_eq!(a.clipboard.as_ref().unwrap().entries.len(), 2);
        a.browser.as_mut().unwrap().path = "/destination".into();
        press(&mut a, 'p');
        let first = rx.try_recv().unwrap();
        let Operation::Transfer(ref spec) = first.op else {
            panic!()
        };
        assert_eq!(spec.source_path, "/files/alpha.txt");
        assert!(!spec.cut);
        assert_eq!(spec.destination_path, "/destination");
        assert_eq!(spec.conflict, "rename");
        assert_eq!(a.file_queue.len(), 1);
        assert!(a.transfer_drawer && a.dialog.is_none());
        a.apply(Reply {
            device: 0,
            op: first.op,
            generation: a.generation,
            result: Ok(serde_json::json!({"status":"queued"})),
        });
        assert!(
            matches!(rx.try_recv().unwrap().op, Operation::Transfer(s) if s.source_path=="/files/beta.txt")
        );
    }
    #[test]
    fn cut_captures_identity_and_queued_quit_requires_intent() {
        let (mut a, rx) = file_app();
        press(&mut a, 'x');
        a.browser.as_mut().unwrap().path = "/destination".into();
        press(&mut a, 'p');
        assert!(
            matches!(rx.try_recv().unwrap().op, Operation::Transfer(s) if s.cut && s.source_identity.as_deref()==Some("identity-alpha.txt"))
        );
        a.request_quit();
        assert!(!a.quit);
        assert!(matches!(a.dialog, Some(Dialog::PendingExit(_))));
        a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(!a.quit);
    }
    #[test]
    fn restart_restores_browser_host_search_focus_and_copy_without_preview_content() {
        let fixture = tempfile::tempdir().unwrap();
        let (mut a, _rx) = queued_app();
        a.device = 2;
        a.view = View::Files;
        a.focus = Focus::Actions;
        a.browser = Some(Browser::new(1, "/recordings/日本語".into()));
        let browser = a.browser.as_mut().unwrap();
        browser.search = "record".into();
        browser.preview = Some("SYNTHETIC_PRIVATE_PREVIEW".into());
        browser.entries.push(Entry {
            name: "record.bin".into(),
            path: "/recordings/日本語/record.bin".into(),
            kind: "file".into(),
            size: 5,
            identity: None,
            hidden: false,
            rename_name: None,
        });
        a.clipboard = Some(Clipboard {
            id: "fixture".into(),
            device: 1,
            entries: vec![browser.entries[0].clone()],
            cut: false,
            source_label: "fixture".into(),
        });
        let name = save_restart_at(&a, fixture.path()).unwrap();
        let bytes = std::fs::read(fixture.path().join(&name)).unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains("SYNTHETIC_PRIVATE_PREVIEW"));
        let (mut restored, _) = queued_app();
        restored.devices.swap(0, 1);
        restore_restart_at(&mut restored, &name, fixture.path()).unwrap();
        assert_eq!(restored.device, 1);
        assert!(restored.view == View::Files && restored.focus == Focus::Actions);
        let browser = restored.browser.as_ref().unwrap();
        assert_eq!(browser.device, 0);
        assert_eq!(browser.search, "record");
        assert_eq!(browser.path, "/recordings/日本語");
        assert!(browser.preview.is_none());
        assert_eq!(restored.clipboard.as_ref().unwrap().device, 0);
        assert!(!fixture.path().join(name).exists());
    }
    #[test]
    fn rejected_cut_retains_clipboard_and_late_mutation_errors_remain_visible() {
        let (mut a, rx) = file_app();
        press(&mut a, 'x');
        press(&mut a, 'p');
        let task = rx.try_recv().unwrap();
        a.generation += 1;
        a.apply(Reply {
            device: 0,
            op: task.op,
            generation: task.generation,
            result: Err(anyhow::anyhow!(
                "Unsupported directory move; source retained"
            )),
        });
        assert!(a
            .clipboard
            .as_ref()
            .is_some_and(|c| c.cut && c.entries.len() == 1));
        assert!(a.notice.contains("source retained"));
        let old = a.generation;
        a.generation += 1;
        a.apply(Reply {
            device: 0,
            op: Operation::Rename {
                path: "/files/alpha.txt".into(),
                name: "beta.txt".into(),
                expected_identity: None,
            },
            generation: old,
            result: Err(anyhow::anyhow!("destination exists")),
        });
        assert!(a.notice.contains("destination exists"));
    }
    fn capture_app(a: &App, width: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, 24)).unwrap();
        terminal.draw(|f| render(f, a)).unwrap();
        terminal
            .backend()
            .buffer()
            .content
            .chunks(width as usize)
            .map(|row| row.iter().map(|c| c.symbol()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }
    #[test]
    fn long_failed_transfer_details_scroll_and_modal_keys_own_footer() {
        let (mut a, _rx) = file_app();
        let destination = format!("/output/{}/recording.copy-1", "long-path/".repeat(30));
        a.jobs.insert(0, serde_json::json!({"jobs":[{"key":"fixture","status":"failed","error":"Disk full · free space then retry","route":"fixture route","source_host":"tester@workstation","destination_host":"peace@laptop","source_display":format!("/source/{}", "nested/".repeat(30)),"actual_destinations":[destination]}]}));
        a.dialog = Some(Dialog::Jobs);
        for width in [48, 80] {
            let first = capture_app(&a, width);
            assert!(first.contains("Disk full") && first.contains("Route:"));
            assert!(!first
                .lines()
                .skip(21)
                .collect::<String>()
                .contains("Copy / cut"));
            let mut reached = false;
            for _ in 0..30 {
                if capture_app(&a, width).contains("recording.copy-1") {
                    reached = true;
                    break;
                }
                a.key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE));
            }
            assert!(
                reached,
                "actual renamed destination must be reachable by scrolling"
            );
            a.dialog_scroll = 0;
        }
    }
    #[test]
    fn recursive_delete_warning_is_visible_and_detail_scrolling_cannot_confirm() {
        let (mut a, rx) = file_app();
        let b = a.browser.as_mut().unwrap();
        b.entries[0].kind = "directory".into();
        b.marked
            .extend(["/files/alpha.txt".into(), "/files/beta.txt".into()]);
        press(&mut a, 'd');
        let text = capture_app(&a, 48);
        assert!(text.contains("No undo.") && text.contains("1 folders") && text.contains("Cancel"));
        assert!(!text
            .lines()
            .skip(21)
            .collect::<String>()
            .contains("Copy / cut"));
        a.key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        press(&mut a, 'j');
        a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(a.dialog.is_some() && !a.dialog_detail_focus && rx.try_recv().is_err());
        a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(a.dialog.is_none() && rx.try_recv().is_err());
        press(&mut a, 'd');
        a.key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        a.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(a.dialog.is_none() && !a.dialog_detail_focus && rx.try_recv().is_err());
    }
    #[test]
    fn restart_preserves_selected_session_identity_when_devices_reorder() {
        let fixture = tempfile::tempdir().unwrap();
        let (mut a, _) = queued_app();
        for d in 0..2 {
            a.work[d].sessions.push(serde_json::from_value(serde_json::json!({"id":format!("session-{d}"),"name":format!("session-{d}"),"directory":"/tmp","provider":"shell","account":a.devices[d].account,"host":a.devices[d].host,"pid":1,"started":"x","boot_id":"boot","external":false,"socket":null})).unwrap());
        }
        a.search = "session-1".into();
        a.selected = a
            .session_rows()
            .iter()
            .position(|(_, s)| s.id == "session-1")
            .unwrap();
        let name = save_restart_at(&a, fixture.path()).unwrap();
        let (mut restored, _) = queued_app();
        restored.devices.swap(0, 1);
        restored.work[0].sessions = a.work[1].sessions.clone();
        restored.work[1].sessions = a.work[0].sessions.clone();
        restore_restart_at(&mut restored, &name, fixture.path()).unwrap();
        assert_eq!(restored.selected_session().unwrap().1.id, "session-1");
    }
    #[test]
    fn restored_file_identity_survives_page_one_clamping() {
        let fixture = tempfile::tempdir().unwrap();
        let (mut a, _) = queued_app();
        let mut browser = Browser::new(0, "/recordings".into());
        browser.entries = (0..1200)
            .map(|i| Entry {
                name: format!("f{i:04}"),
                path: format!("/recordings/f{i:04}"),
                kind: "file".into(),
                size: 1,
                identity: None,
                hidden: false,
                rename_name: None,
            })
            .collect();
        browser.selected = 1100;
        a.browser = Some(browser);
        a.view = View::Files;
        let name = save_restart_at(&a, fixture.path()).unwrap();
        let (mut restored, _rx) = queued_app();
        restore_restart_at(&mut restored, &name, fixture.path()).unwrap();
        for (offset, end, next) in [(0, 1000, Some(1000)), (1000, 1200, None)] {
            let entries=(offset..end).map(|i|serde_json::json!({"name":format!("f{i:04}"),"path":format!("/recordings/f{i:04}"),"kind":"file","size":1})).collect::<Vec<_>>();
            restored.apply(Reply {
                device: 0,
                op: Operation::ListPage {
                    path: "/recordings".into(),
                    offset,
                    limit: 1000,
                },
                generation: 0,
                result: Ok(
                    serde_json::json!({"path":"/recordings","entries":entries,"next_offset":next}),
                ),
            });
        }
        let browser = restored.browser.as_ref().unwrap();
        assert_eq!(browser.entries[browser.selected].path, "/recordings/f1100");
    }
    #[test]
    fn restart_rejects_unsafe_names_symlinks_and_expires() {
        let fixture = tempfile::tempdir().unwrap();
        let (a, _) = queued_app();
        let name = save_restart_at(&a, fixture.path()).unwrap();
        let (mut restored, _) = queued_app();
        assert!(restore_restart_at(&mut restored, "../anything", fixture.path()).is_err());
        let link = "viewer-restart-123.json";
        std::os::unix::fs::symlink(fixture.path().join(&name), fixture.path().join(link)).unwrap();
        assert!(restore_restart_at(&mut restored, link, fixture.path()).is_err());
        let mut value: Value =
            serde_json::from_slice(&std::fs::read(fixture.path().join(&name)).unwrap()).unwrap();
        value["expires_at"] = serde_json::json!(0);
        std::fs::write(
            fixture.path().join(&name),
            serde_json::to_vec(&value).unwrap(),
        )
        .unwrap();
        assert!(restore_restart_at(&mut restored, &name, fixture.path()).is_err());
    }
    #[test]
    fn queued_palette_input_prevents_restart_before_and_after_dispatch() {
        let (mut a, _) = queued_app();
        assert!(restart_ready(&a, Duration::from_secs(4), false));
        assert!(!restart_ready(&a, Duration::from_secs(4), true));
        a.key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL));
        assert!(a.input == Some(Input::Palette));
        assert!(!restart_ready(&a, Duration::from_secs(4), false));
    }
    #[test]
    fn later_directory_pages_cannot_retarget_the_selected_file() {
        let (mut a, _rx) = queued_app();
        let mut browser = Browser::new(0, "/files".into());
        browser.restore_selection = Some("/files/z".into());
        a.browser = Some(browser);
        a.view = View::Files;
        for (offset, name, next) in [(0, "z", Some(1)), (1, "a", None)] {
            a.apply(Reply{device:0,op:Operation::ListPage{path:"/files".into(),offset,limit:1},generation:0,result:Ok(serde_json::json!({"path":"/files","entries":[{"name":name,"path":format!("/files/{name}"),"kind":"file","size":1}],"next_offset":next}))});
        }
        let browser = a.browser.as_ref().unwrap();
        assert_eq!(browser.entries[browser.selected].path, "/files/z");
    }
    #[test]
    fn restart_waits_for_inputs_dialogs_and_mutation_responses() {
        let (mut a, _) = queued_app();
        assert!(can_restart(&a));
        a.input = Some(Input::Search);
        assert!(!can_restart(&a));
        a.input = None;
        a.dialog = Some(Dialog::Provider(0, None));
        assert!(!can_restart(&a));
        a.dialog = None;
        a.creating = true;
        assert!(!can_restart(&a));
        a.creating = false;
        a.pending_requests.set(1);
        assert!(!can_restart(&a));
    }
    #[test]
    fn progress_reordering_preserves_job_cancellation_identity() {
        let (mut a, rx) = queued_app();
        a.dialog = Some(Dialog::Jobs);
        a.jobs.insert(
            0,
            serde_json::json!({"jobs":[
            {"key":"a", "status":"running", "updated":2},
            {"key":"b", "status":"running", "updated":1}]}),
        );
        assert_eq!(a.job_rows()[a.dialog_selected].1["key"], "a");
        a.apply(Reply {
            device: 0,
            op: Operation::TransferJobs,
            generation: 0,
            result: Ok(serde_json::json!({"jobs":[
                {"key":"a", "status":"running", "updated":2},
                {"key":"b", "status":"running", "updated":3}]})),
        });
        a.key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE));
        assert!(matches!(rx.try_recv().unwrap().op, Operation::TransferCancel {key} if key == "a"));
    }
    #[test]
    fn obsolete_network_reply_releases_loading_on_success_and_error() {
        let (mut a, _rx) = queued_app();
        a.generation = 5;
        for result in [Ok(serde_json::json!({})), Err(anyhow::anyhow!("offline"))] {
            a.network_loading = true;
            a.apply(Reply {
                device: 1,
                op: Operation::Network,
                generation: 0,
                result,
            });
            assert!(!a.network_loading);
        }
    }
    #[test]
    fn expired_agent_selection_cannot_turn_into_shell_launch() {
        let (mut a, rx) = queued_app();
        a.dialog = Some(Dialog::Provider(1, None));
        a.dialog_selected = 2;
        a.providers.insert(
            1,
            (
                vec!["claude".into(), "codex".into()],
                transport::now().saturating_sub(61),
            ),
        );
        a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(a.launch_provider.is_none());
        assert!(matches!(a.dialog, Some(Dialog::Provider(1, None))));
        assert!(matches!(rx.try_recv().unwrap().op, Operation::Info));
        a.apply(Reply {
            device: 1,
            op: Operation::Info,
            generation: 0,
            result: Ok(serde_json::json!({"capabilities":["claude", "codex"]})),
        });
        assert_eq!(a.dialog_selected, 2);
    }
    #[test]
    fn launch_choices_follow_execution_device_and_unknown_hides_agents() {
        let (mut a, _rx) = queued_app();
        a.providers
            .insert(1, (vec!["codex".into()], transport::now()));
        assert_eq!(a.provider_choices(0), vec!["shell", "claude", "codex"]);
        assert_eq!(a.provider_choices(1), vec!["shell", "codex"]);
        a.open_browser(1, "~".into());
        assert!(a.provider_choices(1).contains(&"codex"));
        assert!(!a.provider_choices(1).contains(&"claude"));
        a.dialog = Some(Dialog::Provider(1, None));
        a.dialog_selected = 1;
        a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(a.launch_provider.as_deref(), Some("codex"));
        a.providers.insert(1, (vec![], transport::now()));
        assert_eq!(a.provider_choices(1), vec!["shell"]);
        assert!(!a.provider_choices(1).contains(&"codex"));
        assert!(a.provider_choices(1).contains(&"shell"));
        a.providers.remove(&1);
        assert_eq!(a.provider_choices(1), vec!["shell"]);
        a.providers.insert(
            1,
            (vec!["codex".into()], transport::now().saturating_sub(61)),
        );
        assert_eq!(a.provider_choices(1), vec!["shell"]);
    }
    #[test]
    fn provider_results_survive_navigation_but_failed_checks_hide_agents() {
        let (mut a, _rx) = queued_app();
        a.generation = 8;
        a.apply(Reply {
            device: 1,
            op: Operation::Info,
            generation: 0,
            result: Ok(serde_json::json!({"capabilities":["tmux", "codex"]})),
        });
        assert_eq!(a.provider_choices(1), vec!["shell", "codex"]);
        a.apply(Reply {
            device: 1,
            op: Operation::Info,
            generation: 1,
            result: Err(anyhow::anyhow!("unreachable")),
        });
        assert_eq!(a.provider_choices(1), vec!["shell"]);
    }
    #[test]
    fn new_from_files_starts_current_execution_location_and_attaches_on_reply() {
        let (mut a, rx) = queued_app();
        a.browser = Some(Browser::new(1, "/project/remote".into()));
        a.browser.as_mut().unwrap().search = "recording".into();
        a.view = View::Files;
        a.focus = Focus::Actions;
        a.execute(Action::New);
        assert!(
            matches!(&a.dialog, Some(Dialog::Provider(1, Some(path))) if path == "/project/remote")
        );
        a.dialog_selected = 2;
        a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        let task = rx.try_recv().unwrap();
        assert_eq!(task.device, 1);
        let Operation::Create(spec) = &task.op else {
            panic!("expected direct create, not browser reset")
        };
        assert_eq!(spec.directory, "/project/remote");
        assert_eq!(spec.provider, "codex");
        let session = serde_json::json!({"id":"new","name":"Codex","directory":"/project/remote","provider":"codex","account":"peace","host":"laptop","pid":1,"started":"x","boot_id":"boot","external":false,"socket":null});
        a.apply(Reply {
            device: 1,
            op: task.op,
            generation: 0,
            result: Ok(session),
        });
        assert!(a.pending_attach.is_some());
        assert_eq!(a.browser.as_ref().unwrap().search, "recording");
        assert!(a.view == View::Files);
    }
    #[test]
    fn actions_are_workspace_only_and_do_not_duplicate_shell_launch() {
        let (mut a, _) = queued_app();
        a.view = View::Files;
        a.browser = Some(Browser::new(0, "/project".into()));
        for (action, _) in sidebar_actions(&a).into_iter().chain(a.palette()) {
            assert!(workspace_action(action));
        }
        assert!(sidebar_actions(&a)
            .iter()
            .any(|(_, label)| *label == "Sessions"));
        assert_eq!(
            a.palette()
                .iter()
                .filter(|(a, _)| *a == Action::New)
                .count(),
            1
        );
        assert!(!a
            .palette()
            .iter()
            .any(|(_, label)| label.contains("Start here")));
    }
    #[test]
    fn new_shortcut_uses_focused_folder_and_available_providers() {
        let (mut a, _) = queued_app();
        a.view = View::Files;
        a.browser = Some(Browser::new(1, "/projects/robot".into()));
        a.device = 1; // deliberately different selector: browser owns execution location
        press(&mut a, 'n');
        assert!(
            matches!(&a.dialog, Some(Dialog::Provider(1, Some(path))) if path == "/projects/robot")
        );
    }
    #[test]
    fn moving_to_client_only_host_drops_unavailable_launch_profile() {
        let (mut a, rx) = queued_app();
        a.view = View::Files;
        a.browser = Some(Browser::new(1, "/server/files".into()));
        a.launch_provider = Some("codex".into());
        a.providers
            .insert(1, (vec!["shell".into()], transport::now()));
        press(&mut a, 'n');
        assert!(
            matches!(&a.dialog, Some(Dialog::Provider(1, Some(path))) if path == "/server/files")
        );
        assert!(a.launch_provider.is_none());
        assert!(rx.try_recv().is_err());
        assert_eq!(a.provider_choices(1), vec!["shell"]);
    }
    #[test]
    fn command_input_pins_host_folder_and_owns_printable_shortcuts() {
        let (mut a, _) = queued_app();
        a.view = View::Files;
        a.browser = Some(Browser::new(0, "/projects/quoted ' robot".into()));
        press(&mut a, ':');
        assert_eq!(
            a.command_target,
            Some((0, "/projects/quoted ' robot".into()))
        );
        for c in "printf n:hjl".chars() {
            press(&mut a, c);
        }
        assert_eq!(a.text, "printf n:hjl");
        a.browser = Some(Browser::new(1, "/elsewhere".into()));
        a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        let (device, command) = a.pending_command.as_ref().unwrap();
        assert_eq!(device.id, a.devices[0].id);
        assert_eq!(command.directory, "/projects/quoted ' robot");
        assert_eq!(command.command, "printf n:hjl");
        assert!(a.input.is_none() && a.text.is_empty());
        assert!(!can_restart(&a));
    }
    #[test]
    fn long_command_scrolls_and_supports_cursor_edits() {
        let (mut a, _) = queued_app();
        a.view = View::Files;
        a.browser = Some(Browser::new(0, "/files".into()));
        press(&mut a, ':');
        for c in "x".repeat(200).chars() {
            press(&mut a, c);
        }
        for c in "TAIL".chars() {
            press(&mut a, c);
        }
        let text = capture_app(&a, 80);
        assert!(text.contains("TAIL"));
        a.key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        a.key(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE));
        assert!(a.text.ends_with("TAI"));
        a.key(KeyEvent::new(KeyCode::Home, KeyModifiers::NONE));
        press(&mut a, '☃');
        assert!(a.text.starts_with('☃'));
        a.key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        a.key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        assert!(a.text.starts_with("☃x"));
    }
    #[test]
    fn real_info_response_retains_native_command_capability() {
        let (mut a, _) = queued_app();
        a.view = View::Files;
        a.browser = Some(Browser::new(1, "/remote/folder".into()));
        a.apply(Reply { device: 1, op: Operation::Info, generation: a.generation,
            result: Ok(serde_json::json!({"capabilities": ["shell", "tmux", "native-command-v1", "unrecognized"]})) });
        assert_eq!(a.provider_choices(1), vec!["shell"]);
        assert_eq!(a.providers[&1].0, vec!["native-command-v1"]);
        press(&mut a, ':');
        assert!(a.input == Some(Input::Command));
        assert_eq!(a.command_target, Some((1, "/remote/folder".into())));
    }
    #[test]
    fn command_cancel_and_unsupported_remote_never_dispatch() {
        let (mut a, _) = queued_app();
        a.view = View::Files;
        a.browser = Some(Browser::new(0, "/files".into()));
        press(&mut a, ':');
        press(&mut a, 'n');
        a.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(a.pending_command.is_none() && a.command_target.is_none());
        a.browser = Some(Browser::new(1, "/remote".into()));
        a.providers.remove(&1);
        press(&mut a, ':');
        assert!(a.input.is_none() && a.pending_command.is_none());
        assert!(a.notice.contains("Checking"));
        a.providers
            .insert(1, (vec!["native-command-v1".into()], transport::now()));
        press(&mut a, ':');
        assert!(a.input == Some(Input::Command));
    }
    #[test]
    fn new_session_from_all_explicitly_selects_execution_provider_directory() {
        let (mut a, rx) = queued_app();
        assert!(a.action_enabled(Action::New));
        assert!(!a.action_enabled(Action::Work));
        assert_eq!(
            ACTIONS
                .iter()
                .filter(|(_, label)| label.starts_with("Files"))
                .count(),
            1
        );
        a.execute(Action::New);
        assert!(matches!(a.dialog, Some(Dialog::Device(ChooseDevice::New))));
        a.dialog_selected = 1;
        a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(matches!(a.dialog, Some(Dialog::Provider(1, None))));
        a.dialog_selected = 2;
        a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(a.browser.as_ref().unwrap().device, 1);
        assert_eq!(a.launch_provider.as_deref(), Some("codex"));
        let sessions = rx.try_recv().unwrap();
        assert!(matches!(sessions.op, Operation::Sessions));
        let list = rx.try_recv().unwrap();
        assert!(matches!(list.op, Operation::List { .. }));
        a.browser.as_mut().unwrap().path = "/projects/test".into();
        press(&mut a, 'n');
        let task = rx.try_recv().unwrap();
        assert_eq!(task.device, 1);
        let Operation::Create(spec) = task.op else {
            panic!("expected create")
        };
        assert_eq!(spec.provider, "codex");
        assert_eq!(spec.directory, "/projects/test");
    }
    #[test]
    fn matching_live_session_offers_reuse_and_explicit_new() {
        let (mut a, rx) = queued_app();
        let session: Session=serde_json::from_value(serde_json::json!({"id":"same","name":"existing","directory":"/project","provider":"codex","account":"peace","host":"laptop","pid":1,"started":"x","boot_id":"boot","external":false,"socket":null})).unwrap();
        a.work[1].sessions.push(session);
        a.start_at(1, "/project".into(), "codex".into());
        assert!(matches!(a.dialog, Some(Dialog::Matching(..))));
        assert!(a.pending_attach.is_none());
        a.dialog_selected = 1;
        a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(matches!(rx.try_recv().unwrap().op, Operation::Create(_)));
    }
    #[test]
    fn transfer_locations_stay_independent_and_submit_real_spec() {
        let (mut a, rx) = queued_app();
        a.view = View::Files;
        let mut source = Browser::new(1, "/recordings".into());
        source.entries.push(Entry {
            name: "test.bin".into(),
            path: "/recordings/test.bin".into(),
            kind: "file".into(),
            size: 3,
            identity: None,
            hidden: false,
            rename_name: None,
        });
        a.browser = Some(source);
        a.execute(Action::Copy);
        assert!(a.dialog.is_none());
        assert_eq!(a.browser.as_ref().unwrap().path, "/recordings");
        a.execute(Action::Destination);
        a.dialog_selected = 0;
        a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(a.destination_active);
        assert_eq!(a.other_browser.as_ref().unwrap().device, 1);
        a.browser.as_mut().unwrap().path = "/receive".into();
        while rx.try_recv().is_ok() {}
        assert_eq!(a.conflict_policy(), "rename");
        a.execute(Action::Conflict);
        assert_eq!(a.conflict_policy(), "skip");
        a.execute(Action::Conflict);
        assert_eq!(a.conflict_policy(), "overwrite");
        a.execute(Action::Paste);
        let task = rx.try_recv().unwrap();
        assert_eq!(task.device, 0);
        let Operation::Transfer(spec) = task.op else {
            panic!("expected durable transfer")
        };
        assert_eq!(spec.source.id, "remote");
        assert_eq!(spec.destination.id, "local");
        assert_eq!(spec.source_path, "/recordings/test.bin");
        assert_eq!(spec.destination_path, "/receive");
        assert_eq!(spec.conflict, "overwrite");
        assert!(a.dialog.is_none() && a.transfer_drawer);
        a.dialog = None;
        a.focus = Focus::Workspace;
        a.cycle_focus(true);
        assert!(!a.destination_active);
        assert_eq!(a.browser.as_ref().unwrap().path, "/recordings");
        a.cycle_focus(false);
        assert!(a.destination_active);
        a.cycle_focus(false);
        assert!(a.focus == Focus::Devices);
    }
    #[test]
    fn jobs_cancel_owner_and_retry_same_idempotency_spec() {
        let (mut a, rx) = queued_app();
        let spec = crate::model::TransferSpec {
            cut: false,
            source_identity: None,
            source: a.devices[1].clone(),
            source_path: "/data/source".into(),
            destination: a.devices[0].clone(),
            destination_path: "/destination".into(),
            conflict: "skip".into(),
            key: "test-transfer".into(),
        };
        a.submitted.insert(spec.key.clone(), spec);
        a.jobs.insert(0,serde_json::json!({"jobs":[{"key":"test-transfer","status":"running","source_host":"peace@laptop","destination_host":"tester@workstation","route":"direct on laptop","bytes":2,"total":10}]}));
        a.dialog = Some(Dialog::Jobs);
        a.key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE));
        let task = rx.try_recv().unwrap();
        assert_eq!(task.device, 0);
        assert!(matches!(task.op,Operation::TransferCancel { key } if key=="test-transfer"));
        a.jobs.get_mut(&0).unwrap()["jobs"][0]["status"] = serde_json::json!("failed");
        a.key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE));
        let Operation::TransferRetry { key } = rx.try_recv().unwrap().op else {
            panic!("real retry operation required")
        };
        assert_eq!(key, "test-transfer");
    }
    #[test]
    fn directional_panel_focus_obeys_geometry_and_input_ownership() {
        let (mut a, _rx) = queued_app();
        a.panels.borrow_mut().extend([
            (Focus::Devices, false, Rect::new(0, 0, 20, 8)),
            (Focus::Actions, false, Rect::new(0, 8, 20, 8)),
            (Focus::Workspace, false, Rect::new(20, 0, 60, 16)),
        ]);
        a.key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::CONTROL));
        assert!(a.focus == Focus::Devices);
        a.key(KeyEvent::new(KeyCode::Down, KeyModifiers::CONTROL));
        assert!(a.focus == Focus::Actions);
        a.key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::CONTROL));
        assert!(a.focus == Focus::Workspace);
        a.input = Some(Input::Search);
        a.key(KeyEvent::new(KeyCode::Left, KeyModifiers::CONTROL));
        assert!(a.focus == Focus::Workspace);
    }
    #[test]
    fn internet_evidence_is_not_https_or_stale_readiness() {
        let value = serde_json::json!({"internet":{"state":"reachable","stale":false,"observed_at":transport::now()},"sharing":{"state":"unsupported"}});
        let text = network_summary(&value);
        assert!(text.contains("reachable (ICMP)"));
        assert!(text.contains("DNS / HTTPS"));
        let mut stale = value;
        stale["internet"]["observed_at"] = serde_json::json!(transport::now() - 120);
        assert!(network_summary(&stale).contains("not confirmed"));
    }
    #[test]
    fn reverse_tab_cycles_focus_and_leaves_search() {
        let mut a = app();
        a.focus = Focus::Workspace;
        a.key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT));
        assert!(a.focus == Focus::Actions);
        a.key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT));
        assert!(a.focus == Focus::Devices);
        a.key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT));
        assert!(a.focus == Focus::Workspace);
        a.input = Some(Input::Search);
        a.text = "abc".into();
        a.key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT));
        assert!(a.input.is_none());
        assert!(a.focus == Focus::Actions);
    }
    #[test]
    fn horizontal_folder_keys_enter_and_leave_without_previewing_files() {
        for (forward, backward) in [
            (KeyCode::Right, KeyCode::Left),
            (KeyCode::Char('l'), KeyCode::Char('h')),
        ] {
            let mut a = app();
            a.view = View::Files;
            a.focus = Focus::Workspace;
            a.browser = Some(Browser::new(0, "/fixture".into()));
            a.browser.as_mut().unwrap().entries = vec![Entry {
                name: "child".into(),
                path: "/fixture/child".into(),
                kind: "directory".into(),
                size: 0,
                identity: None,
                hidden: false,
                rename_name: None,
            }];
            a.key(KeyEvent::new(forward, KeyModifiers::NONE));
            assert_eq!(a.browser.as_ref().unwrap().path, "/fixture/child");
            a.browser.as_mut().unwrap().parent = Some("/fixture".into());
            a.key(KeyEvent::new(backward, KeyModifiers::NONE));
            assert_eq!(a.browser.as_ref().unwrap().path, "/fixture");
            a.browser.as_mut().unwrap().entries[0].kind = "file".into();
            let generation = a.generation;
            a.key(KeyEvent::new(forward, KeyModifiers::NONE));
            assert_eq!(a.generation, generation);
            assert!(a.browser.as_ref().unwrap().preview.is_none());
        }
    }
    #[test]
    fn fuzzy_search_is_live_and_selection_survives_arrow_keys() {
        let mut a = app();
        a.view = View::Files;
        a.browser = Some(Browser::new(0, "/fixture".into()));
        a.browser.as_mut().unwrap().entries = vec![
            Entry {
                name: "recording-first.bin".into(),
                path: "/fixture/a".into(),
                kind: "file".into(),
                size: 0,
                identity: None,
                hidden: false,
                rename_name: None,
            },
            Entry {
                name: "recording-second.bin".into(),
                path: "/fixture/b".into(),
                kind: "file".into(),
                size: 0,
                identity: None,
                hidden: false,
                rename_name: None,
            },
            Entry {
                name: "other".into(),
                path: "/fixture/c".into(),
                kind: "file".into(),
                size: 0,
                identity: None,
                hidden: false,
                rename_name: None,
            },
        ];
        a.key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE));
        for c in "rcb".chars() {
            a.key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
        }
        assert_eq!(a.visible_entries().len(), 2);
        assert_eq!(a.browser.as_ref().unwrap().search, "rcb");
        a.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(a.browser.as_ref().unwrap().selected, 1);
        let mut t = Terminal::new(TestBackend::new(80, 24)).unwrap();
        t.draw(|f| render(f, &a)).unwrap();
        let bottom = t
            .backend()
            .buffer()
            .content
            .iter()
            .skip(80 * 18)
            .map(|c| c.symbol())
            .collect::<String>();
        assert!(bottom.contains("Search"));
        assert!(bottom.contains("rcb"));
    }
    #[test]
    fn ctrl_c_exits_from_every_modal() {
        for input in [
            None,
            Some(Input::Search),
            Some(Input::Palette),
            Some(Input::Add),
            Some(Input::Mkdir),
        ] {
            let mut a = app();
            a.input = input;
            a.help = true;
            a.key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
            assert!(a.quit);
        }
    }
    #[test]
    fn printable_navigation_is_text_in_inputs() {
        let mut a = app();
        a.input = Some(Input::Search);
        for c in "hjkl?q/".chars() {
            a.key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
        }
        assert_eq!(a.text, "hjkl?q/");
        assert!(!a.quit);
        assert!(!a.help);
    }
    #[test]
    fn enter_file_previews_never_launches() {
        let mut a = app();
        a.view = View::Files;
        a.browser = Some(Browser::new(0, "/tmp".into()));
        a.browser.as_mut().unwrap().entries = vec![Entry {
            name: "file".into(),
            path: "/tmp/file".into(),
            kind: "file".into(),
            size: 1,
            identity: None,
            hidden: false,
            rename_name: None,
        }];
        a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(a.pending_attach.is_none());
    }
    #[test]
    fn hostile_controls_are_not_rendered() {
        assert_eq!(safe_text("\x1b]52;secret\x07"), "�]52;secret�");
    }
    #[test]
    fn stale_directory_reply_cannot_retarget_browser() {
        let mut a = app();
        a.browser = Some(Browser::new(0, "/new".into()));
        a.generation = 2;
        a.apply(Reply {
            device: 0,
            op: Operation::List {
                path: "/old".into(),
            },
            generation: 1,
            result: Ok(serde_json::json!({"path":"/old","entries":[]})),
        });
        assert_eq!(a.browser.unwrap().path, "/new");
    }
    #[test]
    fn terminal_sizes_render_without_panic() {
        for (w, h) in [(80, 24), (120, 40), (40, 15), (25, 8)] {
            let a = app();
            let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
            t.draw(|f| render(f, &a)).unwrap();
            let content = t
                .backend()
                .buffer()
                .content
                .iter()
                .map(|c| c.symbol())
                .collect::<String>();
            assert!(content.contains("cx"));
        }
    }
    #[test]
    fn opaque_parent_and_browser_state_are_preserved() {
        let mut a = app();
        a.open_browser(0, "/recordings".into());
        let b = a.browser.as_mut().unwrap();
        b.selected = 7;
        b.search = "robot".into();
        b.parent = Some("cx-bytes:Lw==".into());
        a.open_browser(0, "/other".into());
        a.open_browser(0, "/recordings".into());
        assert_eq!(a.browser.as_ref().unwrap().selected, 7);
        assert_eq!(a.browser.as_ref().unwrap().search, "robot");
        a.parent_directory();
        assert_eq!(a.browser.as_ref().unwrap().path, "cx-bytes:Lw==");
    }
    #[test]
    fn bounded_queue_never_blocks_or_leaves_checking() {
        let (tx, _rx) = mpsc::sync_channel(1);
        let mut a = app();
        a.tx = tx;
        assert!(a.send(0, Operation::Info));
        a.refresh_work();
        assert!(!a.work[0].loading);
        assert!(a.notice.contains("busy"));
    }
    #[test]
    fn network_observation_uses_real_shape_and_honest_sharing() {
        let text = network_summary(
            &serde_json::json!({"interfaces":{"state":"observed","data":[{"ifname":"eth0","operstate":"UP"}]},"sharing":{"state":"unsupported","reason":"No privileges requested"}}),
        );
        assert!(text.contains("eth0 · UP"));
        assert!(text.contains("Sharing: unsupported"));
        assert!(text.contains("unknown"));
    }
    #[test]
    fn labels_cannot_spoof_rows_or_direction() {
        assert_eq!(safe_label("name\n\t\u{202e}abc"), "name���abc");
    }
}
