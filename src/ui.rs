//! Native workspace. Remote work runs off the input/render thread; attachment owns the terminal.
use crate::{
    model::{CreateSession, Device, Operation, Session},
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
    collections::HashMap,
    io,
    sync::mpsc,
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum View {
    Work,
    Files,
    Network,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Focus {
    Devices,
    Actions,
    Workspace,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Input {
    Search,
    Palette,
    Mkdir,
    Add,
}
#[derive(Clone)]
struct Entry {
    name: String,
    path: String,
    kind: String,
    size: u64,
}
#[derive(Clone)]
struct Browser {
    device: usize,
    path: String,
    display_path: String,
    parent: Option<String>,
    entries: Vec<Entry>,
    selected: usize,
    search: String,
    loading: bool,
    preview: Option<String>,
    preview_scroll: u16,
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
            loading: false,
            preview: None,
            preview_scroll: 0,
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
    Conflict,
    Jobs,
    Shell,
    DeviceShell,
    Claude,
    Codex,
    Observe,
    Refresh,
    Network,
    Work,
    Copy,
    Paste,
    Mkdir,
    Help,
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
    (Action::Destination, "Destination · choose transfer device"),
    (
        Action::Conflict,
        "Existing files · skip / overwrite / rename",
    ),
    (Action::Jobs, "Transfers · progress / cancel / retry"),
    (Action::DeviceShell, "Shell on selected device"),
    (Action::Shell, "Start here · shell"),
    (Action::Claude, "Start here · Claude"),
    (Action::Codex, "Start here · Codex"),
    (Action::Observe, "Observe session · read only"),
    (Action::Copy, "Copy selected file / directory"),
    (Action::Paste, "Paste here · copy into this directory"),
    (Action::Mkdir, "Create directory here"),
    (Action::Network, "Network"),
    (Action::Work, "Work"),
    (Action::Refresh, "Refresh"),
    (Action::Help, "Keyboard help"),
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
    help: bool,
    help_scroll: u16,
    notice: String,
    browser: Option<Browser>,
    other_browser: Option<Browser>,
    destination_active: bool,
    conflict: usize,
    dialog: Option<Dialog>,
    dialog_selected: usize,
    launch_provider: Option<String>,
    submitted: HashMap<String, crate::model::TransferSpec>,
    browser_cache: HashMap<(usize, String), Browser>,
    generation: u64,
    creating: bool,
    providers: HashMap<usize, (Vec<String>, u64)>,
    provider_loading: std::collections::HashSet<usize>,
    network: HashMap<usize, Value>,
    network_loading: bool,
    clipboard: Option<(usize, Entry)>,
    jobs: HashMap<usize, Value>,
    pending_attach: Option<(usize, Session, bool)>,
    pending_add: Option<String>,
    quit: bool,
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
            help: false,
            help_scroll: 0,
            notice: "Ctrl+P actions · ? help".into(),
            browser: None,
            other_browser: None,
            destination_active: false,
            conflict: 2,
            dialog: None,
            dialog_selected: 0,
            launch_provider: None,
            submitted: HashMap::new(),
            browser_cache: HashMap::new(),
            generation: 0,
            creating: false,
            providers: HashMap::new(),
            provider_loading: std::collections::HashSet::new(),
            network: HashMap::new(),
            network_loading: false,
            clipboard: None,
            jobs: HashMap::new(),
            pending_attach: None,
            pending_add: None,
            quit: false,
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
                Dialog::Jobs => "Transfers",
            };
        }
        if let Some(input) = self.input {
            return match input {
                Input::Search => "Search",
                Input::Palette => "Actions",
                Input::Add => "Add device",
                Input::Mkdir => "New folder",
            };
        }
        match self.focus {
            Focus::Devices => "Devices",
            Focus::Actions => "Actions",
            Focus::Workspace => match self.view {
                View::Work => "Work",
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
        self.tx
            .try_send(Task {
                device,
                execution: self.devices[device].clone(),
                op,
                generation: self.generation,
            })
            .is_ok()
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
    fn live_search(&mut self) {
        if self.view == View::Files {
            if let Some(b) = &mut self.browser {
                b.search = self.text.clone();
                b.selected = 0;
            }
        } else {
            self.search = self.text.clone();
            self.selected = 0;
        }
    }
    fn palette(&self) -> Vec<(Action, &'static str)> {
        ACTIONS
            .iter()
            .copied()
            .filter(|(a, label)| {
                self.action_enabled(*a) && label.to_lowercase().contains(&self.text.to_lowercase())
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
            Action::Network => self.view != View::Network,
            Action::Destination | Action::Conflict => self.clipboard.is_some(),
            Action::Copy | Action::Mkdir => self.view == View::Files && self.browser.is_some(),
            Action::Paste => {
                self.view == View::Files && self.clipboard.is_some() && self.destination_active
            }
            Action::Shell | Action::Claude | Action::Codex => {
                self.view == View::Files
                    && !self.creating
                    && self.browser.as_ref().is_some_and(|b| {
                        let provider = match action {
                            Action::Claude => "claude",
                            Action::Codex => "codex",
                            _ => "shell",
                        };
                        self.provider_choices(b.device).contains(&provider)
                    })
            }
            Action::Observe => self.view == View::Work && self.selected_session().is_some(),
            _ => true,
        }
    }
    fn open_browser(&mut self, device: usize, path: String) {
        self.check_providers(device);
        self.generation += 1;
        if let Some(old) = self.browser.take() {
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
        self.view = View::Files;
        self.focus = Focus::Workspace;
        self.refresh_browser();
    }
    fn refresh_browser(&mut self) {
        self.generation += 1;
        if let Some(b) = self.browser.as_mut() {
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
            Action::Quit => self.quit = true,
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
            Action::New | Action::DeviceShell => {
                if action == Action::New && self.view == View::Files {
                    if let Some(browser) = &self.browser {
                        let (device, path) = (browser.device, browser.path.clone());
                        self.check_providers(device);
                        self.dialog = Some(Dialog::Provider(device, Some(path)));
                        self.dialog_selected = 0;
                        return;
                    }
                }
                self.other_browser = None;
                self.destination_active = false;
                self.clipboard = None;
                self.launch_provider = if action == Action::DeviceShell {
                    Some("shell".into())
                } else {
                    None
                };
                self.choose_device(ChooseDevice::New);
            }
            Action::Files => {
                self.clipboard = None;
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
            Action::Conflict => {
                self.conflict = (self.conflict + 1) % 3;
                self.notice = format!("Existing files: {}", self.conflict_policy());
            }
            Action::Jobs => {
                self.dialog = Some(Dialog::Jobs);
                self.dialog_selected = 0;
                for d in 0..self.devices.len() {
                    self.send(d, Operation::TransferJobs);
                }
            }
            Action::Observe => {
                if let Some((d, s)) = self.selected_session() {
                    self.pending_attach = Some((d, s, true));
                }
            }
            Action::Shell | Action::Claude | Action::Codex => {
                if let Some(b) = &self.browser {
                    let provider = match action {
                        Action::Claude => "claude",
                        Action::Codex => "codex",
                        _ => "shell",
                    };
                    self.start_at(b.device, b.path.clone(), provider.into());
                }
            }
            Action::Copy => {
                if let Some(b) = &self.browser {
                    if let Some(e) = self.visible_entries().get(b.selected) {
                        self.clipboard = Some((b.device, e.clone()));
                        self.other_browser = None;
                        self.destination_active = false;
                        self.launch_provider = None;
                        self.notice =
                            "Choose destination device · Enter browses, Paste here confirms copy"
                                .into();
                        self.dialog = Some(Dialog::Device(ChooseDevice::Destination));
                        self.dialog_selected = b.device;
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
            ChooseDevice::Files => self.open_browser(d, "~".into()),
            ChooseDevice::Destination => {
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
        if !self.destination_active {
            return;
        }
        if let (Some((source, entry)), Some(b)) = (&self.clipboard, &self.browser) {
            if let Some(local) = self.devices.iter().position(|d| d.target.is_none()) {
                let spec = crate::model::TransferSpec {
                    source: self.devices[*source].clone(),
                    source_path: entry.path.clone(),
                    destination: self.devices[b.device].clone(),
                    destination_path: b.path.clone(),
                    conflict: self.conflict_policy().into(),
                    key: unique_key(),
                };
                if self.send(local, Operation::Transfer(spec.clone())) {
                    self.notice = format!(
                        "Copy {} → {} · {}",
                        identity(&spec.source),
                        identity(&spec.destination),
                        spec.conflict
                    );
                    if self.submitted.len() >= 256 {
                        if let Some(key) = self.submitted.keys().next().cloned() {
                            self.submitted.remove(&key);
                        }
                    }
                    self.submitted.insert(spec.key.clone(), spec);
                    self.dialog = Some(Dialog::Jobs);
                    self.dialog_selected = 0;
                } else {
                    self.notice = "Request queue busy · retry shortly".into();
                }
            }
        }
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
        let count = match &dialog {
            Dialog::Device(_) => self.devices.len(),
            Dialog::Provider(d, _) => self.provider_choices(*d).len(),
            Dialog::Matching(..) => 2,
            Dialog::Jobs => self.job_rows().len(),
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
                self.dialog_selected = shift(self.dialog_selected, 1, count)
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.dialog_selected = shift(self.dialog_selected, -1, count)
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
                    self.notice =
                        "Browse to a folder · Ctrl+P → Start here (Enter only opens files)".into();
                }
                Dialog::Matching(d, path, provider, session) => {
                    self.dialog = None;
                    if self.dialog_selected == 0 {
                        self.pending_attach = Some((d, session, false));
                    } else {
                        self.create_at(d, path, provider);
                    }
                }
                Dialog::Jobs => {}
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
                    if matches!(job["status"].as_str(), Some("failed" | "cancelled")) {
                        if let Some(spec) = job["key"]
                            .as_str()
                            .and_then(|k| self.submitted.get(k))
                            .cloned()
                        {
                            self.send(d, Operation::Transfer(spec));
                        } else {
                            self.notice = "Reopen source and copy to retry this older job · saved resume data is preserved".into();
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
                if reply.generation == self.generation {
                    self.notice = message;
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
                            .filter(|p| ["claude", "codex"].contains(p))
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
                let selected = if matches!(self.dialog, Some(Dialog::Jobs)) {
                    self.job_rows()
                        .get(self.dialog_selected)
                        .map(|(owner, job)| (*owner, job["key"].clone()))
                } else {
                    None
                };
                self.jobs.insert(reply.device, value);
                if let Some((owner, key)) = selected {
                    self.dialog_selected = self
                        .job_rows()
                        .iter()
                        .position(|(d, job)| *d == owner && job["key"] == key)
                        .unwrap_or(0);
                }
            }
            Operation::TransferCancel { .. } => {
                self.send(reply.device, Operation::TransferJobs);
                self.notice = "Cancellation requested · waiting for worker".into();
            }
            Operation::Transfer(_) => {
                self.notice = format!(
                    "Transfer {} · {}",
                    safe_label(value["status"].as_str().unwrap_or("queued")),
                    safe_label(value["route"].as_str().unwrap_or("worker host"))
                );
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
                                    })
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    if listing_offset == 0 {
                        b.entries = entries;
                    } else {
                        b.entries.extend(entries);
                    }
                    b.entries.sort_by(|a, b| {
                        (a.kind != "directory", a.name.as_str())
                            .cmp(&(b.kind != "directory", b.name.as_str()))
                    });
                    b.selected = b.selected.min(b.entries.len().saturating_sub(1));
                    b.loading = false;
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
            _ => {}
        }
    }
    fn key(&mut self, key: KeyEvent) {
        if key.kind == KeyEventKind::Release {
            return;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.quit = true;
            return;
        }
        if let Some(dialog) = self.dialog.clone() {
            self.dialog_key(key, dialog);
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
        // Input fields own every printable key, including navigation shortcuts.
        if let Some(mode) = self.input {
            let before_text = self.text.clone();
            match key.code {
                KeyCode::Esc => {
                    self.input = None;
                    self.text.clear();
                }
                KeyCode::Backspace => {
                    self.text.pop();
                    self.palette_selected = 0;
                }
                KeyCode::Char(c)
                    if !key
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                {
                    if self.text.chars().count() < 256 {
                        self.text.push(c);
                    }
                    self.palette_selected = 0;
                }
                KeyCode::Down if mode == Input::Search => self.move_selection(1),
                KeyCode::Up if mode == Input::Search => self.move_selection(-1),
                KeyCode::Tab | KeyCode::BackTab if mode == Input::Search => {
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
                    Input::Search => {
                        self.input = None;
                        self.focus = Focus::Workspace;
                        self.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
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
            return;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Char('p') => {
                    self.input = Some(Input::Palette);
                    self.text.clear();
                    self.palette_selected = 0;
                }
                KeyCode::Char('c') => self.quit = true,
                KeyCode::Left | KeyCode::Char('h') => self.directional_focus(-1, 0),
                KeyCode::Right | KeyCode::Char('l') => self.directional_focus(1, 0),
                KeyCode::Up | KeyCode::Char('k') => self.directional_focus(0, -1),
                KeyCode::Down | KeyCode::Char('j') => self.directional_focus(0, 1),
                _ => {}
            }
            return;
        }
        match key.code {
            KeyCode::Char('?') | KeyCode::F(1) => self.help = true,
            KeyCode::Char('/') => {
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
            KeyCode::Enter => {
                if self.focus == Focus::Actions {
                    let actions = sidebar_actions(self);
                    if let Some((action, _)) = actions.get(self.side_selected) {
                        self.execute(*action)
                    };
                } else if self.focus == Focus::Devices {
                    self.focus = Focus::Workspace;
                    self.refresh();
                } else {
                    match self.view {
                    View::Work => { if let Some((d,s)) = self.selected_session() { self.pending_attach = Some((d,s,false)); } },
                    View::Files => { if let Some(b) = &self.browser { if let Some(e) = self.visible_entries().get(b.selected).cloned() { if e.kind == "directory" { self.open_browser(b.device,e.path); } else { self.generation += 1; self.send(b.device,Operation::Preview { path:e.path }); } } } },
                    View::Network => self.notice = "Ctrl+P · shell / files are available through Work and the device selector".into(),
                }
                }
            }
            KeyCode::Esc => {
                self.generation += 1;
                if self.view == View::Files {
                    if let Some(b) = &mut self.browser {
                        if b.preview.take().is_none() {
                            if !b.search.is_empty() {
                                b.search.clear();
                                b.selected = 0;
                            } else {
                                self.view = View::Work;
                            }
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
        if self.view == View::Files {
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
            if self.view != View::Files {
                self.refresh();
            }
        } else if self.view == View::Files {
            let len = self.visible_entries().len();
            if let Some(b) = &mut self.browser {
                b.selected = shift(b.selected, delta, len);
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
fn sidebar_actions(app: &App) -> Vec<(Action, &'static str)> {
    let mut a = vec![
        (Action::Files, "Files"),
        (Action::New, "New session"),
        (Action::DeviceShell, "Shell"),
        (Action::Jobs, "Transfers"),
        (Action::Network, "Network"),
        (Action::Add, "Add device"),
    ];
    if app.view != View::Work {
        a.insert(0, (Action::Work, "Work"));
    }
    if app.view == View::Files {
        if let Some(provider) = &app.launch_provider {
            let action = match provider.as_str() {
                "claude" => Action::Claude,
                "codex" => Action::Codex,
                _ => Action::Shell,
            };
            a.insert(0, (action, "Start here"));
        }
        if app.clipboard.is_some() {
            a.insert(0, (Action::Destination, "Destination…"));
            a.insert(
                0,
                (
                    Action::Conflict,
                    ["Existing: skip", "Existing: overwrite", "Existing: rename"][app.conflict],
                ),
            );
            if app.destination_active {
                a.insert(0, (Action::Paste, "Paste here"));
            }
        } else {
            a.insert(0, (Action::Copy, "Copy to…"));
        }
    }
    a.retain(|(action, _)| app.action_enabled(*action));
    if app.selected_session().is_some() && app.view == View::Work {
        a.insert(2, (Action::Observe, "Observe"));
    }
    a
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
    let query = if app.view == View::Files {
        app.browser
            .as_ref()
            .map(|b| b.search.as_str())
            .unwrap_or("")
    } else {
        app.search.as_str()
    };
    let show_search = app.input == Some(Input::Search) || !query.is_empty();
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(5),
            Constraint::Length(if show_search { 6 } else { 4 }),
        ])
        .split(area);
    let label = match (app.view, area.width < 70) {
        (View::Work, true) => "Work",
        (View::Files, true) => "Files",
        (View::Network, true) => "Network",
        (View::Work, false) => "[Work]  Network",
        (View::Files, false) => "Work / Files  Network",
        (View::Network, false) => "Work  [Network]",
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
    let sidebar_width = if area.width < 70 { 17 } else { 21 };
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
            Span::raw(format!("{:<12} {count:>2}", fit_label(&d.name, 12))),
        ])));
    }
    let sidebar = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length((app.devices.len() as u16 + 3).min(content[0].height / 2)),
            Constraint::Length(8),
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
        .map(|(_, label)| ListItem::new(format!("  {label}")))
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
    let details = if let Some((i, s)) = app.selected_session() {
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
                        .block(block("Work".into(), app.focus == Focus::Workspace)),
                    content[1],
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
                        Constraint::Length(if content[1].width > 65 { 24 } else { 18 }),
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
                    format!("Work · {} sessions", rows.len()),
                    app.focus == Focus::Workspace,
                ))
                .row_highlight_style(selected_style())
                .highlight_symbol(if ascii() { "> " } else { "› " });
                frame.render_stateful_widget(table, content[1], &mut state);
            }
        }
        View::Files => {
            if let Some(b) = &app.browser {
                if let Some(other) = &app.other_browser {
                    let panes = Layout::default()
                        .direction(if content[1].width >= 52 {
                            Direction::Horizontal
                        } else {
                            Direction::Vertical
                        })
                        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                        .split(content[1]);
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
                    );
                    render_browser(
                        frame,
                        destination,
                        &app.devices[destination.device],
                        panes[1],
                        app.destination_active && app.focus == Focus::Workspace,
                        &format!("Destination · {}", app.conflict_policy()),
                    );
                } else {
                    let label = app
                        .launch_provider
                        .as_ref()
                        .map(|p| format!("Start {p} here · actions"))
                        .unwrap_or_else(|| "Files".into());
                    render_browser(
                        frame,
                        b,
                        &app.devices[b.device],
                        content[1],
                        app.focus == Focus::Workspace,
                        &label,
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
                content[1],
            );
        }
    }
    let footer = Layout::default()
        .direction(Direction::Vertical)
        .constraints(if show_search {
            vec![
                Constraint::Length(3),
                Constraint::Length(1),
                Constraint::Length(2),
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
        .constraints([
            Constraint::Percentage(30),
            Constraint::Percentage(30),
            Constraint::Percentage(40),
        ])
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
            Span::styled(format!(" · {shown}/{total}"), muted()),
        ]))
        .alignment(ratatui::layout::Alignment::Center),
        status_sections[1],
    );
    let transfer = if active > 0 {
        format!("↔ {active} copying")
    } else if failed > 0 {
        format!("× {failed} failed copies")
    } else if !jobs.is_empty() {
        format!("● {} copied", jobs.len())
    } else {
        "↔ transfers idle".into()
    };
    frame.render_widget(
        Paragraph::new(transfer)
            .style(if failed > 0 { warning } else { accent() })
            .alignment(ratatui::layout::Alignment::Right),
        status_sections[2],
    );
    if show_search {
        let editing = app.input == Some(Input::Search);
        let text = if editing { app.text.as_str() } else { query };
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(" /  ", accent().add_modifier(Modifier::BOLD)),
                Span::raw(safe_label(text)),
            ]))
            .block(block("Search".into(), editing)),
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
    let mut hints = if app.input == Some(Input::Search) {
        vec![
            ("↑↓", "Select"),
            ("Enter", "Open"),
            ("Esc", "Done"),
            ("Tab", "Focus"),
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
    if columns == 2 && app.input != Some(Input::Search) {
        hints.retain(|(_, label)| !matches!(*label, "Search" | "Focus"));
    }
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1)])
        .split(footer[key_row]);
    for (i, (key, label)) in hints.into_iter().enumerate() {
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
    if let Some(input) = app.input.filter(|i| *i != Input::Search) {
        let rect = popup(area, 76, if input == Input::Palette { 16 } else { 5 });
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
            frame.render_widget(
                Paragraph::new(format!(
                    "> {}\nEnter confirm · Escape cancel",
                    safe_text(&app.text)
                ))
                .block(block(
                    match input {
                        Input::Search => "Search",
                        Input::Add => "Add device · SSH alias or user@host",
                        _ => "Directory name",
                    }
                    .into(),
                    true,
                )),
                rect,
            );
        }
    }
    if let Some(dialog) = &app.dialog {
        let (title, labels, detail) = match dialog {
            Dialog::Device(purpose) => (
                match purpose {
                    ChooseDevice::New => "New session · execution device",
                    ChooseDevice::Files => "Files · choose device",
                    ChooseDevice::Destination => "Copy · destination device",
                }
                .to_string(),
                app.devices
                    .iter()
                    .map(|d| format!("{} · {}", safe_label(&d.name), identity(d)))
                    .collect::<Vec<_>>(),
                if *purpose == ChooseDevice::Destination {
                    app.clipboard
                        .as_ref()
                        .map(|(d, e)| {
                            format!(
                                "Source: {} · {}\nEnter choose · Escape cancel",
                                identity(&app.devices[*d]),
                                safe_label(&e.name)
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
                    "Enter choose · next: browse folder, then Start here".into()
                },
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
                            "{} → {}\n{}\n{}",
                            safe_label(j["source_path"].as_str().unwrap_or("")),
                            safe_label(j["destination_path"].as_str().unwrap_or("")),
                            safe_label(j["route"].as_str().unwrap_or("route unknown")),
                            safe_label(j["error"].as_str().unwrap_or(""))
                        )
                    })
                    .unwrap_or_else(|| "No transfers yet · Files → Copy to…".into());
                (
                    "Transfers · c cancel · r retry/refresh · Esc close".into(),
                    jobs.iter()
                        .map(|(_, j)| {
                            format!(
                                "{}  {} → {}  {} / {} B",
                                safe_label(j["status"].as_str().unwrap_or("unknown")),
                                safe_label(j["source_host"].as_str().unwrap_or("?")),
                                safe_label(j["destination_host"].as_str().unwrap_or("?")),
                                j["bytes"].as_u64().unwrap_or(0),
                                j["total"].as_u64().unwrap_or(0)
                            )
                        })
                        .collect(),
                    detail,
                )
            }
        };
        let mut rect = popup(area, 90, 15);
        if matches!(dialog, Dialog::Jobs) {
            rect.y = area.y + area.height.saturating_sub(rect.height + 5);
        }
        frame.render_widget(Clear, rect);
        let parts = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(3), Constraint::Length(4)])
            .split(rect);
        let mut state =
            ratatui::widgets::ListState::default().with_selected(Some(app.dialog_selected));
        frame.render_stateful_widget(
            List::new(labels.into_iter().map(ListItem::new).collect::<Vec<_>>())
                .block(block(title, true))
                .highlight_style(selected_style()),
            parts[0],
            &mut state,
        );
        frame.render_widget(
            Paragraph::new(detail)
                .wrap(Wrap { trim: false })
                .block(block("Context".into(), false)),
            parts[1],
        );
    }
    if app.help {
        let rect = popup(area, 76, 20);
        frame.render_widget(Clear, rect);
        let mut help = String::from("Arrows / h j k l  navigate\nCtrl+arrows / Ctrl+h j k l  move panel focus\nEnter  enter directory / preview file / take control\nFiles: Left/h parent · Right/l enter directory\nEscape  back     Tab / Shift+Tab  focus     /  search\nCtrl+P  actions  Ctrl+C  quit\n\nNative terminal: cx keys are suspended.\nManaged sessions: Ctrl+] returns to cx.\nExternal sessions keep their own tmux bindings.\n\nAvailable actions\n");
        for (_, label) in ACTIONS.iter().filter(|(a, _)| app.action_enabled(*a)) {
            help.push_str(label);
            help.push('\n');
        }
        frame.render_widget(
            Paragraph::new(help)
                .wrap(Wrap { trim: false })
                .scroll((app.help_scroll, 0))
                .block(block("Help · ↑↓ scroll · Escape closes".into(), true)),
            rect,
        );
    }
}
fn browser_entries(b: &Browser) -> Vec<Entry> {
    let mut rows = b
        .entries
        .iter()
        .enumerate()
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
) {
    let parts = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(1)])
        .split(area);
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled(
                format!(" {label}"),
                accent().add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(format!(" {}", identity(device)), accent())),
            Line::from(Span::styled(
                format!(
                    " {}{}",
                    compact_path(&b.display_path, area.width.saturating_sub(2) as usize),
                    if b.loading { " · checking" } else { "" }
                ),
                muted(),
            )),
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
        let items = rows
            .iter()
            .map(|e| {
                ListItem::new(Line::from(vec![
                    Span::styled(
                        if e.kind == "directory" {
                            "+ "
                        } else if e.kind == "symlink" {
                            "@ "
                        } else {
                            "  "
                        },
                        if e.kind == "directory" {
                            accent()
                        } else {
                            Style::default()
                        },
                    ),
                    Span::raw(safe_label(&e.name)),
                    Span::styled(
                        if e.kind == "file" {
                            format!("  {} B", e.size)
                        } else {
                            String::new()
                        },
                        muted(),
                    ),
                ]))
            })
            .collect::<Vec<_>>();
        let items = if items.is_empty() {
            vec![ListItem::new(if b.loading {
                "Checking…"
            } else if b.search.is_empty() {
                "Empty folder"
            } else {
                "No matches"
            })
            .style(muted())]
        } else {
            items
        };
        let mut state = ratatui::widgets::ListState::default().with_selected(Some(b.selected));
        frame.render_stateful_widget(
            List::new(items)
                .block(block("Enter open · h/l folders".into(), focused))
                .highlight_style(selected_style()),
            parts[1],
            &mut state,
        );
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

pub fn run() -> Result<()> {
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
    // One worker bounds simultaneous probes. No redraw-triggered SSH or credential prompts.
    thread::spawn(move || {
        while let Ok(task) = rx.recv() {
            let result = transport::request(&task.execution, task.op.clone());
            if result_tx
                .send(Reply {
                    device: task.device,
                    op: task.op,
                    generation: task.generation,
                    result,
                })
                .is_err()
            {
                break;
            }
        }
    });
    let mut app = App::new(devices, tx);
    load_cache(&mut app);
    app.refresh_work();
    for d in 0..app.devices.len() {
        app.check_providers(d);
        app.send(d, Operation::TransferJobs);
    }
    let mut screen = Screen::new().context("open terminal workspace")?;
    let mut dirty = true;
    let mut last_refresh = Instant::now();
    let mut last_jobs = Instant::now();
    while !app.quit && !stopping.load(std::sync::atomic::Ordering::Relaxed) {
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
        if let Some((d, session, observe)) = app.pending_attach.take() {
            screen.suspend()?;
            let result = sessions::attach(&app.devices[d], &session, observe);
            screen.resume()?;
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
                Event::Key(k) => app.key(k),
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
        assert!(a.action_enabled(Action::Codex));
        assert!(!a.action_enabled(Action::Claude));
        a.dialog = Some(Dialog::Provider(1, None));
        a.dialog_selected = 1;
        a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(a.launch_provider.as_deref(), Some("codex"));
        a.providers.insert(1, (vec![], transport::now()));
        assert_eq!(a.provider_choices(1), vec!["shell"]);
        assert!(!a.action_enabled(Action::Codex));
        assert!(a.action_enabled(Action::Shell));
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
        a.execute(Action::Codex);
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
        });
        a.browser = Some(source);
        a.execute(Action::Copy);
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
        assert!(matches!(a.dialog, Some(Dialog::Jobs)));
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
        let Operation::Transfer(spec) = rx.try_recv().unwrap().op else {
            panic!("real retry operation required")
        };
        assert_eq!(spec.key, "test-transfer");
        assert_eq!(spec.source_path, "/data/source");
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
            },
            Entry {
                name: "recording-second.bin".into(),
                path: "/fixture/b".into(),
                kind: "file".into(),
                size: 0,
            },
            Entry {
                name: "other".into(),
                path: "/fixture/c".into(),
                kind: "file".into(),
                size: 0,
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
