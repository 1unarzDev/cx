//! Native workspace. Remote work runs off the input/render thread; attachment owns the terminal.
use crate::{
    model::{CreateSession, Device, Operation, RunCommand, Session},
    sessions, store, transport,
};
use anyhow::{Context, Result};
use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
        KeyModifiers, MouseEvent, MouseEventKind,
    },
    execute,
    terminal::{self, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, BorderType, Borders, Cell, Clear, List, ListItem, Padding, Paragraph, Row, Table,
        TableState, Wrap,
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
#[derive(Clone)]
struct RichPreview {
    kind: String,
    title: String,
    raster: Option<(usize, usize, Vec<u8>)>,
    styled: Option<Vec<Line<'static>>>,
    page: u32,
    pages: Option<u32>,
}
impl RichPreview {
    fn from_value(value: &Value) -> Self {
        use base64::Engine;
        let raster = (|| {
            let image = value.get("image")?;
            let w = usize::try_from(image["width"].as_u64()?).ok()?;
            let h = usize::try_from(image["height"].as_u64()?).ok()?;
            if let Some(encoded) = image["png"].as_str() {
                if w == 0 || h == 0 || w > 1280 || h > 960 || encoded.len() > 800_000 {
                    return None;
                }
                let png = base64::engine::general_purpose::STANDARD
                    .decode(encoded)
                    .ok()?;
                if png.len() > 600_000 || !png.starts_with(b"\x89PNG\r\n\x1a\n") {
                    return None;
                }
                let mut reader = image::ImageReader::with_format(
                    std::io::Cursor::new(png),
                    image::ImageFormat::Png,
                );
                let mut limits = image::Limits::default();
                limits.max_image_width = Some(1280);
                limits.max_image_height = Some(960);
                limits.max_alloc = Some(8 * 1024 * 1024);
                reader.limits(limits);
                let decoded = reader.decode().ok()?.to_rgba8();
                if decoded.width() as usize != w || decoded.height() as usize != h {
                    return None;
                }
                return Some((w, h, decoded.into_raw()));
            }
            if w == 0 || h == 0 || w > 160 || h > 100 {
                return None;
            }
            let encoded = image["rgba"].as_str()?;
            if encoded.len() > 86_000 {
                return None;
            }
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .ok()?;
            (bytes.len() == w * h * 4).then_some((w, h, bytes))
        })();
        Self {
            kind: value["kind"].as_str().unwrap_or("text").to_owned(),
            title: safe_label(value["title"].as_str().unwrap_or("Preview")),
            raster,
            page: value["page"]
                .as_u64()
                .and_then(|v| u32::try_from(v).ok())
                .filter(|v| (1..=10000).contains(v))
                .unwrap_or(1),
            pages: value["pages"]
                .as_u64()
                .and_then(|v| u32::try_from(v).ok())
                .filter(|v| (1..=10000).contains(v)),
            styled: match value["kind"].as_str() {
                Some("code") => Some(crate::syntax_preview::highlight(
                    &safe_text(value["text"].as_str().unwrap_or("")),
                    value["path"].as_str().unwrap_or(""),
                )),
                Some("markdown") => Some(preview_lines(
                    value["text"].as_str().unwrap_or(""),
                    "markdown",
                )),
                _ => None,
            },
        }
    }
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
    #[serde(skip)]
    preview_rich: Option<RichPreview>,
    #[serde(skip)]
    preview_revision: u64,
    #[serde(skip)]
    preview_path: Option<String>,
    #[serde(skip)]
    preview_requested_page: u32,
    #[serde(skip)]
    preview_pending_page: Option<u32>,
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
            preview_rich: None,
            preview_revision: 0,
            preview_path: None,
            preview_requested_page: 1,
            preview_pending_page: None,
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
    preview: Option<RichPreview>,
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
    (
        Action::Update,
        concat!(
            "Update cx v",
            env!("CARGO_PKG_VERSION"),
            " · check releases"
        ),
    ),
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
    StopShell(usize, Session),
    PendingExit(usize),
    Neighbor(usize, Value),
    Peer(usize),
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
    peer_checks: VecDeque<usize>,
    peer_inflight: BTreeSet<usize>,
    peer_evidence: HashMap<usize, (u64, bool)>,
    network_loading: bool,
    network_inflight: BTreeSet<usize>,
    network_selected: usize,
    network_candidates_inflight: BTreeSet<usize>,
    network_refresh_queue: VecDeque<usize>,
    neighbor_probes: BTreeSet<(usize, String, Option<String>)>,
    neighbor_queue: VecDeque<(usize, Value)>,
    neighbor_evidence: HashMap<(usize, String, Option<String>), Value>,
    neighbor_budget: usize,
    network_add_target: Option<(usize, String)>,
    pending_add_via: Option<Device>,
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
            notice: String::new(),
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
            peer_checks: VecDeque::new(),
            peer_inflight: BTreeSet::new(),
            peer_evidence: HashMap::new(),
            network_loading: false,
            network_inflight: BTreeSet::new(),
            network_selected: 0,
            network_candidates_inflight: BTreeSet::new(),
            network_refresh_queue: VecDeque::new(),
            neighbor_probes: BTreeSet::new(),
            neighbor_queue: VecDeque::new(),
            neighbor_evidence: HashMap::new(),
            neighbor_budget: 32,
            network_add_target: None,
            pending_add_via: None,
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
                Dialog::Delete(..) => "Delete",
                Dialog::StopShell(..) => "Stop shell",
                Dialog::PendingExit(_) => "Pending actions",
                Dialog::Neighbor(..) => "Neighbor actions",
                Dialog::Peer(_) => "Device actions",
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
        if self.view == View::Network {
            return self.network_action_device().map(|d| (d, "~".into()));
        }
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
        if let Some(mut old) = self.browser.take() {
            old.preview = None;
            old.preview_rich = None;
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
            b.preview_rich = None;
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
    // Network browser owns candidate selection and captures scope before actions.
    fn network_device(&self) -> Option<usize> {
        if self.device > 0 {
            Some(self.device - 1)
        } else {
            self.network_rows()
                .get(self.network_selected)
                .and_then(|c| c["_device"].as_u64())
                .map(|d| d as usize)
                .or_else(|| self.devices.iter().position(|d| d.target.is_none()))
        }
    }
    fn network_candidates(&self) -> Vec<Value> {
        let mut rows = Vec::new();
        for d in 0..self.devices.len() {
            if self.device > 0 && self.device != d + 1 {
                continue;
            }
            if let Some(candidates) = self
                .network
                .get(&d)
                .and_then(|v| v["candidates"].as_array())
            {
                for candidate in candidates
                    .iter()
                    .filter(|c| c["address"].as_str().is_some())
                {
                    let mut candidate = candidate.clone();
                    candidate["_device"] = serde_json::json!(d);
                    candidate["_snapshot"] = self.network[&d]["candidates_observed_at"].clone();
                    rows.push(candidate);
                }
            }
        }
        rows.sort_by_key(|c| {
            (
                c["_device"].as_u64().unwrap_or(0),
                c["interface"].as_str().unwrap_or("").to_string(),
                c["source"].as_str().unwrap_or("").to_string(),
                c["address"]
                    .as_str()
                    .and_then(|s| s.parse::<std::net::IpAddr>().ok()),
            )
        });
        rows
    }
    // Enrolled rows are independent of the selected helper's passive LAN snapshot.
    fn network_rows(&self) -> Vec<Value> {
        let mut rows: Vec<_> = self
            .devices
            .iter()
            .enumerate()
            .map(|(d, device)| serde_json::json!({"_peer": d, "id": device.id}))
            .collect();
        rows.extend(self.network_candidates());
        rows
    }
    fn network_action_device(&self) -> Option<usize> {
        self.network_rows()
            .get(self.network_selected)
            .and_then(|r| r["_peer"].as_u64().or_else(|| r["_device"].as_u64()))
            .map(|d| d as usize)
            .or_else(|| self.network_device())
    }
    fn peer_status(&self, d: usize) -> String {
        if self.peer_inflight.contains(&d) {
            return "checking helper".into();
        }
        if let Some((checked, success)) = self.peer_evidence.get(&d) {
            let age = transport::now().saturating_sub(*checked);
            return if *success && age <= 90 {
                format!("helper reached · {age}s ago")
            } else if *success {
                format!("cached helper · {age}s ago")
            } else {
                format!("helper unavailable · {age}s ago")
            };
        }
        let checked = self.providers.get(&d).map(|(_, time)| *time).or_else(|| {
            self.work
                .get(d)
                .filter(|w| w.error.is_none() && w.fetched > 0)
                .map(|w| w.fetched)
        });
        checked
            .map(|time| {
                format!(
                    "cached helper · {}s ago",
                    transport::now().saturating_sub(time)
                )
            })
            .unwrap_or_else(|| "helper not checked".into())
    }
    fn pump_peer_checks(&mut self) {
        while self.peer_inflight.len()
            + self.neighbor_probes.len()
            + self
                .network_inflight
                .union(&self.network_candidates_inflight)
                .count()
            < 4
        {
            let Some(d) = self.peer_checks.pop_front() else {
                break;
            };
            if self.provider_loading.contains(&d) {
                continue;
            }
            if !self.send(d, Operation::Info) {
                self.peer_checks.push_front(d);
                break;
            }
            self.provider_loading.insert(d);
            self.peer_inflight.insert(d);
        }
    }
    fn open_neighbor(&mut self) {
        if let Some(row) = self.network_rows().get(self.network_selected).cloned() {
            self.dialog_selected = 0;
            if let Some(d) = row["_peer"].as_u64() {
                self.dialog = Some(Dialog::Peer(d as usize));
            } else if let Some(d) = row["_device"].as_u64() {
                self.connect_neighbor(d as usize, row);
            }
        } else {
            self.notice = "No devices or observed LAN neighbors".into();
        }
    }
    fn pump_network_refresh(&mut self) {
        // Four enrolled helpers at once; each host has one observation and one passive snapshot.
        while self.peer_inflight.len()
            + self.neighbor_probes.len()
            + self
                .network_inflight
                .union(&self.network_candidates_inflight)
                .count()
            < 4
        {
            let Some(d) = self.network_refresh_queue.pop_front() else {
                break;
            };
            let observation =
                self.network_inflight.contains(&d) || self.send(d, Operation::Network);
            if observation {
                self.network_inflight.insert(d);
            }
            let candidates = self.network_candidates_inflight.contains(&d)
                || self.send(d, Operation::NetworkCandidates);
            if candidates {
                self.network_candidates_inflight.insert(d);
            }
            if !observation || !candidates {
                self.network_refresh_queue.push_front(d);
                break;
            }
        }
        self.pump_peer_checks();
        self.pump_neighbor_checks();
        self.network_loading = !self.network_inflight.is_empty()
            || !self.network_candidates_inflight.is_empty()
            || !self.network_refresh_queue.is_empty();
    }
    fn connect_neighbor(&mut self, d: usize, candidate: Value) {
        if !neighbor_connectable(&candidate) {
            self.notice =
                "Link-local SSH enrollment needs an interface scope · unavailable here".into();
        } else if let Some(address) = candidate["address"].as_str() {
            self.network_add_target = Some((d, address.into()));
            self.input = Some(Input::Add);
            self.text.clear();
            self.notice =
                "Enter SSH account · enrollment checks authentication and helper access".into();
        }
    }
    fn queue_neighbor_checks(&mut self) {
        if self.view != View::Network {
            self.neighbor_queue.clear();
            return;
        }
        let current = self.network_candidates();
        self.neighbor_queue.retain(|(d, c)| {
            current.iter().any(|row| {
                row["_device"] == *d
                    && row["address"] == c["address"]
                    && row["interface"] == c["interface"]
                    && row["_snapshot"] == c["_snapshot"]
            })
        });
        self.neighbor_evidence.retain(|_, evidence| {
            transport::now().saturating_sub(evidence["observed_at"].as_u64().unwrap_or(0)) <= 90
        });
        for candidate in self.network_candidates() {
            let Some(d) = candidate["_device"].as_u64().map(|d| d as usize) else {
                continue;
            };
            let Some(address) = candidate["address"].as_str() else {
                continue;
            };
            let Some(interface) = candidate["interface"].as_str() else {
                continue;
            };
            let observed = candidate["_snapshot"].as_u64().unwrap_or(0);
            if observed == 0
                || transport::now().saturating_sub(observed) > 90
                || address.parse::<std::net::IpAddr>().is_err()
                || interface.is_empty()
                || interface.len() >= libc::IFNAMSIZ
                || !interface
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_.-:".contains(&b))
            {
                continue;
            }
            let key = (d, address.to_owned(), Some(interface.to_owned()));
            if self.neighbor_probes.contains(&key)
                || self.neighbor_evidence.get(&key).is_some_and(|v| {
                    transport::now().saturating_sub(v["observed_at"].as_u64().unwrap_or(0)) <= 90
                })
                || self.neighbor_queue.iter().any(|(owner, c)| {
                    *owner == d
                        && c["address"] == candidate["address"]
                        && c["interface"] == candidate["interface"]
                })
                || self.neighbor_budget == 0
            {
                continue;
            }
            self.neighbor_budget -= 1;
            self.neighbor_queue.push_back((d, candidate));
        }
    }
    fn pump_neighbor_checks(&mut self) {
        if self.view != View::Network {
            self.neighbor_queue.clear();
            return;
        }
        while self.neighbor_probes.len()
            + self.peer_inflight.len()
            + self
                .network_inflight
                .union(&self.network_candidates_inflight)
                .count()
            < 4
        {
            let Some((d, candidate)) = self.neighbor_queue.pop_front() else {
                break;
            };
            if self.device > 0 && self.device != d + 1 {
                continue;
            }
            let observed = candidate["_snapshot"].as_u64().unwrap_or(0);
            if observed == 0 || transport::now().saturating_sub(observed) > 90 {
                continue;
            }
            if !self.network_candidates().iter().any(|c| {
                c["_device"] == d
                    && c["address"] == candidate["address"]
                    && c["interface"] == candidate["interface"]
                    && c["_snapshot"] == candidate["_snapshot"]
            }) {
                continue;
            }
            if !self.probe_neighbor(d, &candidate) {
                self.neighbor_queue.push_front((d, candidate));
                break;
            }
        }
    }
    fn probe_neighbor(&mut self, d: usize, candidate: &Value) -> bool {
        let Some(address) = candidate["address"].as_str() else {
            return true;
        };
        let interface = candidate["interface"].as_str().map(str::to_string);
        let key = (d, address.to_string(), interface.clone());
        if self.neighbor_probes.contains(&key) {
            return true;
        }
        if self.send(
            d,
            Operation::ProbeCandidate {
                address: address.into(),
                interface,
            },
        ) {
            self.neighbor_probes.insert(key);
            true
        } else {
            false
        }
    }
    fn refresh(&mut self) {
        if self.view != View::Network {
            for d in 0..self.devices.len() {
                self.providers.remove(&d);
                self.check_providers(d);
            }
        }
        match self.view {
            View::Work => self.refresh_work(),
            View::Files => self.refresh_browser(),
            View::Network => {
                self.neighbor_budget = 32;
                self.neighbor_queue.clear();
                for d in 0..self.devices.len() {
                    if !self.peer_inflight.contains(&d)
                        && !self.peer_checks.contains(&d)
                        && self
                            .providers
                            .get(&d)
                            .is_none_or(|(_, t)| transport::now().saturating_sub(*t) >= 60)
                    {
                        self.peer_checks.push_back(d);
                    }
                }
                for d in 0..self.devices.len() {
                    if (self.device == 0 || self.device == d + 1)
                        && !self.network_inflight.contains(&d)
                        && !self.network_candidates_inflight.contains(&d)
                        && !self.network_refresh_queue.contains(&d)
                    {
                        self.network_refresh_queue.push_back(d);
                    }
                }
                self.pump_network_refresh();
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
                self.network_add_target = None;
                self.pending_add_via = None;
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
                        self.check_providers(d);
                        self.dialog = Some(Dialog::Provider(d, Some(session.directory)));
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
        if matches!(dialog, Dialog::Delete(..) | Dialog::StopShell(..))
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
        if matches!(
            dialog,
            Dialog::Jobs | Dialog::Delete(..) | Dialog::StopShell(..)
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
                        && matches!(dialog, Dialog::Delete(..) | Dialog::StopShell(..)) =>
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
            Dialog::Delete(..) | Dialog::StopShell(..) => 2,
            Dialog::PendingExit(_) => 2,
            Dialog::Neighbor(..) => 1,
            Dialog::Peer(_) => 4,
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
                Dialog::StopShell(d, session) => {
                    self.dialog = None;
                    if self.dialog_selected == 1 {
                        self.send(
                            d,
                            Operation::StopSession {
                                id: session.id,
                                pid: session.pid,
                                started: session.started,
                                boot_id: session.boot_id,
                            },
                        );
                        self.notice = "Stopping confirmed shell…".into();
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
                        _ => {
                            self.check_providers(d);
                            self.dialog = Some(Dialog::Provider(d, None));
                            self.dialog_selected = 0;
                        }
                    }
                }
                Dialog::Neighbor(d, candidate) => {
                    self.dialog = None;
                    self.connect_neighbor(d, candidate);
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
            if self.peer_inflight.remove(&reply.device) {
                self.peer_evidence
                    .insert(reply.device, (transport::now(), reply.result.is_ok()));
                self.pump_network_refresh();
            }
        }
        if matches!(reply.op, Operation::Network) {
            self.network_inflight.remove(&reply.device);
            self.network_loading = self
                .network_device()
                .is_some_and(|d| self.network_inflight.contains(&d));
        }
        if matches!(reply.op, Operation::NetworkCandidates) {
            self.network_candidates_inflight.remove(&reply.device);
        }
        if matches!(reply.op, Operation::Network | Operation::NetworkCandidates) {
            self.pump_network_refresh();
        }
        if let Operation::ProbeCandidate { address, interface } = &reply.op {
            self.neighbor_probes
                .remove(&(reply.device, address.clone(), interface.clone()));
            let evidence = match &reply.result {
                Ok(v) => v.clone(),
                Err(_) => {
                    serde_json::json!({"observed_at":transport::now(), "ssh":{"state":"unknown"}})
                }
            };
            self.neighbor_evidence
                .insert((reply.device, address.clone(), interface.clone()), evidence);
            self.pump_network_refresh();
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
                if let Operation::ProbeCandidate { address, interface } = &reply.op {
                    if let Some(candidates) = self
                        .network
                        .get_mut(&reply.device)
                        .and_then(|v| v["candidates"].as_array_mut())
                    {
                        if let Some(candidate) = candidates.iter_mut().find(|c| {
                            c["address"].as_str() == Some(address.as_str())
                                && c["interface"].as_str() == interface.as_deref()
                        }) {
                            candidate["ssh"]["state"] = serde_json::json!("unknown");
                        }
                    }
                }
                if matches!(reply.op, Operation::ProbeCandidate { .. }) {
                    return;
                }
                let message =
                    safe_text(&format!("{}: {e:#}", identity(&self.devices[reply.device])));
                if matches!(reply.op, Operation::Network | Operation::NetworkCandidates) {
                    if let Some(v) = self.network.get_mut(&reply.device) {
                        v["candidates_observed_at"] = Value::Null;
                        v["internet"]["state"] = serde_json::json!("unknown");
                        v["internet"]["stale"] = serde_json::json!(true);
                        if let Some(candidates) = v["candidates"].as_array_mut() {
                            for candidate in candidates {
                                candidate["ssh"]["state"] = serde_json::json!("unknown");
                            }
                        }
                    }
                }
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
                if reply.generation == self.generation
                    && matches!(
                        reply.op,
                        Operation::Preview { .. } | Operation::PreviewPage { .. }
                    )
                {
                    if let Some(b) = &mut self.browser {
                        b.preview_pending_page = None;
                        b.preview_requested_page = b.preview_rich.as_ref().map_or(1, |p| p.page);
                        if b.preview_rich.is_none() {
                            b.preview = Some(self.notice.clone());
                        }
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
                            .filter(|p| {
                                [
                                    "claude",
                                    "codex",
                                    "native-command-v1",
                                    "stop-session-v1",
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
            Operation::Network => {
                let candidates = self
                    .network
                    .get(&reply.device)
                    .map(|v| (v["candidates"].clone(), v["candidates_observed_at"].clone()));
                self.network.insert(reply.device, value);
                if let Some((candidates, observed)) = candidates {
                    self.network.get_mut(&reply.device).unwrap()["candidates"] = candidates;
                    self.network.get_mut(&reply.device).unwrap()["candidates_observed_at"] =
                        observed;
                }
                self.network_selected = self
                    .network_selected
                    .min(self.network_rows().len().saturating_sub(1));
            }
            Operation::NetworkCandidates => {
                let selected = self.network_rows().get(self.network_selected).map(|c| {
                    (
                        c["_peer"].clone(),
                        c["_device"].clone(),
                        c["address"].clone(),
                        c["interface"].clone(),
                    )
                });
                self.network
                    .entry(reply.device)
                    .or_insert_with(|| serde_json::json!({}))["candidates"] =
                    value["candidates"].clone();
                self.network.get_mut(&reply.device).unwrap()["candidates_observed_at"] =
                    value["observed_at"].clone();
                if let Some(candidates) = self
                    .network
                    .get_mut(&reply.device)
                    .and_then(|v| v["candidates"].as_array_mut())
                {
                    for candidate in candidates {
                        let key = (
                            reply.device,
                            candidate["address"].as_str().unwrap_or("").to_owned(),
                            candidate["interface"].as_str().map(str::to_owned),
                        );
                        if let Some(evidence) = self.neighbor_evidence.get(&key).filter(|v| {
                            transport::now().saturating_sub(v["observed_at"].as_u64().unwrap_or(0))
                                <= 90
                        }) {
                            candidate["ssh"] = evidence["ssh"].clone();
                            candidate["observed_at"] = evidence["observed_at"].clone();
                        }
                    }
                }
                self.queue_neighbor_checks();
                self.pump_neighbor_checks();
                if self.device == 0 || self.device == reply.device + 1 {
                    let rows = self.network_rows();
                    self.network_selected = selected
                        .and_then(|(peer, device, address, interface)| {
                            rows.iter().position(|c| {
                                c["_peer"] == peer
                                    && c["_device"] == device
                                    && c["address"] == address
                                    && c["interface"] == interface
                            })
                        })
                        .unwrap_or_else(|| self.network_selected.min(rows.len().saturating_sub(1)));
                }
            }
            Operation::ProbeCandidate { address, interface } => {
                if let Some(candidates) = self
                    .network
                    .get_mut(&reply.device)
                    .and_then(|v| v["candidates"].as_array_mut())
                {
                    if let Some(candidate) = candidates.iter_mut().find(|c| {
                        c["address"].as_str() == Some(&address)
                            && c["interface"].as_str() == interface.as_deref()
                    }) {
                        candidate["ssh"] = value["ssh"].clone();
                        candidate["observed_at"] = value["observed_at"].clone();
                    }
                }
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
            Operation::Preview { ref path } | Operation::PreviewPage { ref path, .. }
                if reply.generation == self.generation
                    && self
                        .browser
                        .as_ref()
                        .is_some_and(|b| b.device == reply.device) =>
            {
                let page = match reply.op {
                    Operation::PreviewPage { page, .. } => page,
                    _ => 1,
                };
                let mut next = false;
                if let Some(b) = &mut self.browser {
                    if b.preview_path
                        .as_ref()
                        .is_some_and(|requested| requested != path)
                    {
                        return;
                    }
                    b.preview_path = Some(path.clone());
                    b.preview_pending_page = None;
                    if b.preview_requested_page == 0 {
                        b.preview_requested_page = page;
                    }
                    if b.preview_requested_page != page && b.preview_rich.is_some() {
                        next = true;
                    } else {
                        b.preview_requested_page = page;
                        b.preview_scroll = 0;
                        b.preview_revision = b.preview_revision.wrapping_add(1);
                        let rich = reply
                            .preview
                            .unwrap_or_else(|| RichPreview::from_value(&value));
                        let malformed = value["image"].is_object() && rich.raster.is_none();
                        b.preview = Some(if malformed {
                            "Image preview unavailable · invalid bitmap response".into()
                        } else {
                            safe_text(value["text"].as_str().unwrap_or("Preview unavailable"))
                        });
                        b.preview_rich = Some(rich);
                    }
                }
                if next {
                    self.start_pdf_page_request();
                }
            }
            Operation::StopSession { .. } => {
                self.notice = format!("Shell stopped · {}", identity(&self.devices[reply.device]));
                self.work[reply.device].loading = self.send(reply.device, Operation::Sessions);
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
                    self.network_add_target = None;
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
                        let target = if let Some((_, address)) = &self.network_add_target {
                            format!("{}@{}", self.text, address)
                        } else {
                            self.text.clone()
                        };
                        if transport::valid_target(&target) {
                            self.pending_add_via = self
                                .network_add_target
                                .take()
                                .map(|(d, _)| self.devices[d].clone());
                            self.pending_add = Some(target);
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
        if self.view == View::Work
            && self.focus == Focus::Workspace
            && key.code == KeyCode::Char('d')
            && key.modifiers.is_empty()
        {
            if let Some((device, session)) = self.selected_session() {
                if session.external || session.provider != "shell" {
                    self.notice = "Only cx-managed shell sessions can be stopped here".into();
                } else if self.devices[device].target.is_some()
                    && !self.providers.get(&device).is_some_and(|(caps, checked)| {
                        transport::now().saturating_sub(*checked) < 60
                            && caps.iter().any(|c| c == "stop-session-v1")
                    })
                {
                    self.check_providers(device);
                    self.notice = "Checking shell-stop support · press d again when ready".into();
                } else {
                    self.dialog_selected = 0;
                    self.dialog_scroll = 0;
                    self.dialog_detail_focus = false;
                    self.dialog = Some(Dialog::StopShell(device, session));
                }
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
            KeyCode::Char('a') if self.view == View::Work && local_only(self) => {
                self.execute(Action::Add)
            }
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
            KeyCode::PageDown => {
                self.move_selection(if self.pdf_preview_active() { 1 } else { 10 })
            }
            KeyCode::PageUp => {
                self.move_selection(if self.pdf_preview_active() { -1 } else { -10 })
            }
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
                        View::Work => {
                            if let Some((d, s)) = self.selected_session() {
                                self.pending_attach = Some((d, s, false));
                            } else if empty_work_can_create(self) {
                                self.execute(Action::New);
                            }
                        }
                        View::Files => {
                            if let Some(b) = &self.browser {
                                if let Some(e) = self.visible_entries().get(b.selected).cloned() {
                                    if e.kind == "directory" {
                                        self.open_browser(b.device, e.path);
                                    } else {
                                        self.open_preview(b.device, e.path);
                                    }
                                }
                            }
                        }
                        View::Network => self.open_neighbor(),
                    }
                }
            }
            KeyCode::Esc => {
                if self.view == View::Files {
                    if let Some(b) = &mut self.browser {
                        b.preview_rich = None;
                        b.preview_pending_page = None;
                        b.preview_path = None;
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
    fn host_updated(&mut self, update: transport::HostUpdate) {
        let indices = self
            .devices
            .iter()
            .enumerate()
            .filter_map(|(d, device)| {
                (device.target.as_deref() == Some(update.target.as_str())).then_some(d)
            })
            .collect::<Vec<_>>();
        for device in indices {
            self.providers.remove(&device);
            self.provider_loading.remove(&device);
            self.check_providers(device);
            self.work[device].loading = self.send(device, Operation::Sessions);
            self.send(device, Operation::TransferJobs);
            let preview = self
                .browser
                .as_ref()
                .filter(|b| b.device == device && b.preview.is_some())
                .and_then(|b| {
                    b.preview_path
                        .clone()
                        .map(|path| (path, b.preview_requested_page.max(1)))
                });
            if let Some((path, page)) = preview {
                self.generation += 1;
                let operation = if page > 1 {
                    Operation::PreviewPage { path, page }
                } else {
                    Operation::Preview { path }
                };
                if self.send(device, operation) {
                    if let Some(b) = &mut self.browser {
                        b.preview_pending_page = Some(page);
                    }
                }
            }
            self.notice = format!(
                "{} · cx {} updated",
                identity(&self.devices[device]),
                update.version
            );
        }
    }
    fn pdf_preview_active(&self) -> bool {
        self.view == View::Files
            && self.focus == Focus::Workspace
            && self.browser.as_ref().is_some_and(|b| {
                b.preview.is_some()
                    && b.preview_rich
                        .as_ref()
                        .is_some_and(|p| p.kind == "pdf" && p.raster.is_some())
            })
    }
    fn open_preview(&mut self, device: usize, path: String) {
        self.generation += 1;
        if let Some(b) = &mut self.browser {
            b.preview_path = Some(path.clone());
            b.preview_requested_page = 1;
            b.preview_pending_page = Some(1);
            b.preview_rich = None;
            b.preview = Some("Loading preview…".into());
        }
        if !self.send(device, Operation::Preview { path }) {
            if let Some(b) = &mut self.browser {
                b.preview_pending_page = None;
                b.preview = Some("Preview queue busy · Escape and retry".into());
            }
        }
    }
    fn start_pdf_page_request(&mut self) {
        let Some(b) = self.browser.as_ref() else {
            return;
        };
        if b.preview_pending_page.is_some()
            || b.preview_rich
                .as_ref()
                .is_some_and(|p| p.page == b.preview_requested_page)
        {
            return;
        }
        let device = b.device;
        let path = b
            .preview_path
            .clone()
            .or_else(|| browser_entries(b).get(b.selected).map(|e| e.path.clone()));
        let Some(path) = path else {
            return;
        };
        let page = b.preview_requested_page.max(1);
        if self.devices[device].target.is_some()
            && !self
                .providers
                .get(&device)
                .is_some_and(|(caps, _)| caps.iter().any(|c| c == "pdf-pages-v1"))
        {
            self.providers.remove(&device);
            self.check_providers(device);
            self.notice = "PDF pages need a current device helper · checking update".into();
            return;
        }
        if self.send(
            device,
            Operation::PreviewPage {
                path: path.clone(),
                page,
            },
        ) {
            if let Some(b) = &mut self.browser {
                b.preview_path = Some(path);
                b.preview_pending_page = Some(page);
            }
        } else {
            self.notice = "Preview queue busy · retry shortly".into();
        }
    }
    fn preview_mouse(&mut self, mouse: MouseEvent, area: Rect) -> bool {
        if self.help || self.dialog.is_some() || self.input.is_some() || !self.pdf_preview_active()
        {
            return false;
        }
        if !native_preview_area(self, area)
            .is_some_and(|r| r.contains(ratatui::layout::Position::new(mouse.column, mouse.row)))
        {
            return false;
        }
        match mouse.kind {
            MouseEventKind::ScrollDown => self.move_selection(1),
            MouseEventKind::ScrollUp => self.move_selection(-1),
            _ => return false,
        }
        true
    }
    fn move_selection(&mut self, delta: isize) {
        if self.view == View::Files && self.focus == Focus::Workspace {
            if let Some(b) = &mut self.browser {
                if b.preview.is_some() {
                    if b.preview_rich.as_ref().is_some_and(|p| p.kind == "pdf") {
                        let max = b
                            .preview_rich
                            .as_ref()
                            .and_then(|p| p.pages)
                            .unwrap_or(10000);
                        let current = b.preview_requested_page.max(1);
                        let requested =
                            (i64::from(current) + delta as i64).clamp(1, i64::from(max)) as u32;
                        if requested != current {
                            b.preview_requested_page = requested;
                            self.start_pdf_page_request();
                        }
                        return;
                    }
                    b.preview_scroll = b.preview_scroll.saturating_add_signed(
                        delta.clamp(i16::MIN as isize, i16::MAX as isize) as i16,
                    );
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
        } else if self.view == View::Network {
            self.network_selected = shift(self.network_selected, delta, self.network_rows().len());
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
            if (c.is_control() && c != '\n' && c != '\t')
                || matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
            {
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
    accent().add_modifier(Modifier::UNDERLINED | Modifier::BOLD)
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
        Action::Command
            | Action::Destination
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

fn local_only(app: &App) -> bool {
    !app.devices.is_empty() && app.devices.iter().all(|d| d.target.is_none())
}
fn empty_work_can_create(app: &App) -> bool {
    app.view == View::Work
        && app.search.is_empty()
        && app.session_rows().is_empty()
        && !app.creating
        && !app
            .work
            .iter()
            .enumerate()
            .filter(|(i, _)| app.device == 0 || app.device == i + 1)
            .any(|(_, w)| w.loading || w.error.is_some())
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

#[cfg(test)]
fn render(frame: &mut Frame<'_>, app: &App) {
    render_with_native(frame, app, None)
}
fn render_with_native(
    frame: &mut Frame<'_>,
    app: &App,
    native: Option<&mut crate::terminal_preview::NativePreview>,
) {
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
                _ => Color::Reset,
            })
        };
        device_items.push(ListItem::new(Line::from(vec![
            Span::styled(format!("{indicator} "), dot_style),
            Span::raw(fit_label(&d.name, sidebar_width.saturating_sub(8) as usize)),
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
            .block(
                block("Devices".into(), app.focus == Focus::Devices)
                    .padding(Padding::horizontal(1)),
            )
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
        .map(|(_, label)| ListItem::new(*label))
        .collect::<Vec<_>>();
    let mut state =
        ratatui::widgets::ListState::default().with_selected(if app.focus == Focus::Actions {
            Some(app.side_selected.min(actions.len().saturating_sub(1)))
        } else {
            None
        });
    frame.render_stateful_widget(
        List::new(items)
            .block(
                block("Actions".into(), app.focus == Focus::Actions)
                    .padding(Padding::horizontal(1)),
            )
            .highlight_style(selected_style()),
        sidebar[1],
        &mut state,
    );
    let details = if app.view == View::Network {
        app.network_rows()
            .get(app.network_selected)
            .map(|c| {
                if let Some(d) = c["_peer"].as_u64().map(|d| d as usize) {
                    return format!(
                        "{}\n{}\nViewer helper evidence",
                        identity(&app.devices[d]),
                        app.peer_status(d)
                    );
                }
                format!(
                    "Via {}\n{}\nSSH {}",
                    c["_device"]
                        .as_u64()
                        .and_then(|d| app.devices.get(d as usize))
                        .map(identity)
                        .unwrap_or_else(|| "unknown".into()),
                    safe_label(c["address"].as_str().unwrap_or("unknown")),
                    neighbor_ssh(c)
                )
            })
            .unwrap_or_else(|| "No observed neighbors".into())
    } else if let Some(b) = app.browser.as_ref().filter(|_| app.view == View::Files) {
        let mut lines = vec![
            format!("Host {}", identity(&app.devices[b.device])),
            format!("Folder {}", safe_label(&b.display_path)),
            format!("{} marked · t send", b.marked.len()),
        ];
        if let Some(entry) = app.visible_entries().get(b.selected) {
            lines.insert(2, format!("File {}", safe_label(&entry.name)));
        }
        if let Some(other) = &app.other_browser {
            let (destination, label) = if app.destination_active {
                (b, "To")
            } else {
                (other, "To")
            };
            lines.push(format!(
                "{label} {}",
                identity(&app.devices[destination.device])
            ));
            lines.push(safe_label(&destination.display_path));
        }
        lines.join("\n")
    } else if let Some((i, s)) = app.selected_session() {
        format!(
            "Host {}\nFolder {}\nEnter open · n new",
            identity(&app.devices[i]),
            safe_label(&s.directory),
        )
    } else if let Some(i) = app.actual_device() {
        format!(
            "Host {}\n{} sessions\nn new · Ctrl+P actions",
            identity(&app.devices[i]),
            app.work[i].sessions.len()
        )
    } else {
        "Choose a device\nn new · Ctrl+P actions".into()
    };
    if sidebar[2].height > 3 {
        let details = details
            .lines()
            .map(|line| fit_label(line, sidebar_width.saturating_sub(2) as usize))
            .collect::<Vec<_>>()
            .join("\n");
        frame.render_widget(
            Paragraph::new(details).wrap(Wrap { trim: false }).block(
                Block::default()
                    .padding(Padding::horizontal(1))
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
                let checking = app
                    .work
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| app.device == 0 || app.device == i + 1)
                    .any(|(_, w)| w.loading);
                let errors: Vec<_> = app
                    .work
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| app.device == 0 || app.device == i + 1)
                    .filter_map(|(_, w)| w.error.as_deref())
                    .collect();
                let mut lines = vec![Line::from("")];
                if checking {
                    lines.push(Line::from(Span::styled("Checking sessions…", accent())));
                    lines.push(Line::from("Waiting for device evidence"));
                } else if !errors.is_empty() {
                    lines.push(Line::from(Span::styled(
                        "Device unavailable",
                        tint(Color::Yellow),
                    )));
                    for error in errors {
                        lines.push(Line::from(safe_text(error)));
                    }
                    lines.push(Line::from("Ctrl+P → Refresh"));
                } else if !app.search.is_empty() {
                    lines.push(Line::from("No matching sessions"));
                    lines.push(Line::from("Esc clears search"));
                    lines.push(Line::from("n starts a new session"));
                } else {
                    lines.push(Line::from(Span::styled(
                        "No live sessions",
                        Style::default().add_modifier(Modifier::BOLD),
                    )));
                    lines.push(Line::from("Choose a provider and folder."));
                    lines.push(Line::from(""));
                    lines.push(Line::from(Span::styled(
                        if app.creating {
                            "Creating session…"
                        } else {
                            "[ Enter / n  New session ]"
                        },
                        if app.focus == Focus::Workspace && !app.creating {
                            selected_style()
                        } else {
                            accent().add_modifier(Modifier::BOLD)
                        },
                    )));
                    if local_only(app) {
                        lines.push(Line::from(""));
                        lines.push(Line::from("Use this computer now."));
                        lines.push(Line::from(Span::styled(
                            "a  Add device",
                            accent().add_modifier(Modifier::BOLD),
                        )));
                        lines.push(Line::from("Connect an existing SSH target."));
                    }
                }
                frame.render_widget(
                    Paragraph::new(lines).wrap(Wrap { trim: false }).block(
                        block("Sessions".into(), app.focus == Focus::Workspace)
                            .padding(Padding::horizontal(1)),
                    ),
                    workspace,
                );
            } else {
                let selected_error = rows
                    .get(app.selected)
                    .and_then(|(device, _)| app.work[*device].error.as_deref());
                let table_area = if let Some(error) = selected_error {
                    let areas = Layout::default()
                        .direction(Direction::Vertical)
                        .constraints([Constraint::Min(3), Constraint::Length(2)])
                        .split(workspace);
                    frame.render_widget(
                        Paragraph::new(format!(
                            "{}\nCtrl+P → Refresh · state unconfirmed",
                            fit_label(&safe_text(error), usize::from(areas[1].width))
                        ))
                        .style(muted()),
                        areas[1],
                    );
                    areas[0]
                } else {
                    workspace
                };
                let compact = workspace.width < 50;
                let mut display_rows = Vec::new();
                let mut selected_row = 0;
                let mut last_project = None;
                for (index, (i, s)) in rows.iter().enumerate() {
                    if last_project != Some(s.directory.as_str()) {
                        if last_project.is_some() {
                            display_rows.push(Row::new(vec![Cell::from("")]).height(1));
                        }
                        display_rows.push(
                            Row::new(if compact {
                                vec![Cell::from(safe_label(&s.directory)), Cell::from("")]
                            } else {
                                vec![
                                    Cell::from(""),
                                    Cell::from(safe_label(&s.directory)),
                                    Cell::from(""),
                                    Cell::from(""),
                                ]
                            })
                            .style(muted().add_modifier(Modifier::BOLD))
                            .height(1),
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
                    let cells = vec![
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
                    ];
                    display_rows.push(Row::new(if compact {
                        vec![cells[1].clone(), cells[3].clone()]
                    } else {
                        cells
                    }));
                }
                let mut state = TableState::default().with_selected(Some(selected_row));
                let table = Table::new(
                    display_rows,
                    if compact {
                        vec![Constraint::Min(10), Constraint::Length(8)]
                    } else {
                        vec![
                            Constraint::Length(9),
                            Constraint::Min(10),
                            Constraint::Length(if workspace.width > 65 { 24 } else { 18 }),
                            Constraint::Length(8),
                        ]
                    },
                )
                .header(
                    Row::new(if compact {
                        vec!["SESSION", "STATE"]
                    } else {
                        vec!["AGENT", "SESSION", "EXECUTION", "STATE"]
                    })
                    .style(muted())
                    .bottom_margin(1),
                )
                .column_spacing(if compact { 1 } else { 2 })
                .block(block(
                    format!("Sessions · {} sessions", rows.len()),
                    app.focus == Focus::Workspace,
                ))
                .row_highlight_style(selected_style())
                .highlight_symbol(if ascii() { "> " } else { "› " });
                frame.render_stateful_widget(table, table_area, &mut state);
            }
        }
        View::Files => {
            if let Some(b) = &app.browser {
                if b.preview.is_some()
                    && b.preview_rich
                        .as_ref()
                        .is_some_and(|p| matches!(p.kind.as_str(), "image" | "pdf"))
                {
                    let rows = Layout::default()
                        .direction(Direction::Vertical)
                        .constraints([Constraint::Length(1), Constraint::Min(1)])
                        .split(workspace);
                    frame.render_widget(
                        Paragraph::new(Line::from(vec![
                            Span::styled(
                                format!(" {} ", identity(&app.devices[b.device])),
                                accent().add_modifier(Modifier::BOLD),
                            ),
                            Span::styled(
                                fit_label(
                                    &b.display_path,
                                    usize::from(rows[0].width.saturating_sub(24)),
                                ),
                                muted(),
                            ),
                        ])),
                        rows[0],
                    );
                    render_preview_with_native(
                        frame,
                        rows[1],
                        b,
                        b.preview.as_deref().unwrap_or(""),
                        app.focus == Focus::Workspace,
                        native,
                    );
                } else if let Some(other) = &app.other_browser {
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
        View::Network => render_network(frame, app, workspace),
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
    let shown = if app.view == View::Network {
        app.network_rows().len()
    } else if app.view == View::Files {
        app.visible_entries().len()
    } else {
        app.session_rows().len()
    };
    let total = if app.view == View::Network {
        shown
    } else if app.view == View::Files {
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
                format!(
                    " {} {errors} unavailable  ",
                    if ascii() { "!" } else { "◆" }
                )
            } else {
                format!(
                    " {} {ready}/{} devices  ",
                    if ascii() { "+" } else { "●" },
                    app.devices.len()
                )
            },
            if errors > 0 {
                warning
            } else if ready == 0 {
                muted()
            } else {
                healthy
            },
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
            Paragraph::new(
                if app.view == View::Network && app.notice == "Ctrl+P actions · ? help" {
                    String::new()
                } else {
                    safe_text(&app.notice)
                },
            )
            .style(muted()),
            notice_area(footer[2]),
        );
    }
    if show_search {
        frame.render_widget(
            Paragraph::new(
                if app.view == View::Network && app.notice == "Ctrl+P actions · ? help" {
                    String::new()
                } else {
                    safe_text(&app.notice)
                },
            )
            .style(muted()),
            notice_area(footer[3]),
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
        } else if matches!(dialog, Dialog::Delete(..) | Dialog::StopShell(..)) {
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
                (
                    "y / n",
                    if matches!(dialog, Dialog::StopShell(..)) {
                        "Stop / keep"
                    } else {
                        "Delete / cancel"
                    },
                ),
                ("Tab", "Details"),
                ("PgUpDn", "Scroll"),
                ("Home", "Top"),
            ]
        } else {
            vec![("↑↓", "Choose"), ("Enter", "Confirm"), ("Esc", "Cancel")]
        }
    } else if app.input == Some(Input::Add) {
        vec![("Ctrl U", "Clear")]
    } else if matches!(
        app.input,
        Some(Input::Rename | Input::Mkdir | Input::Palette | Input::Command)
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
    } else if app.view == View::Files
        && app.focus == Focus::Workspace
        && app.browser.as_ref().is_some_and(|b| b.preview.is_some())
    {
        vec![
            (
                "j/k",
                if app.pdf_preview_active() {
                    "Pages"
                } else {
                    "Scroll"
                },
            ),
            ("PgUpDn", "Page"),
            ("Esc", "Back"),
            ("?", "Help"),
        ]
    } else if app.view == View::Network && app.focus == Focus::Workspace {
        vec![
            ("j/k", "Devices / LAN"),
            ("Enter", "Actions"),
            ("Tab", "Device scope"),
            ("Ctrl P", "Actions"),
            ("?", "Help"),
            ("Ctrl C", "Quit"),
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
    } else if empty_work_can_create(app) && app.focus == Focus::Workspace {
        vec![
            ("Enter/n", "New session"),
            (
                if local_only(app) { "a" } else { "/" },
                if local_only(app) {
                    "Add device"
                } else {
                    "Search"
                },
            ),
            ("Tab", "Focus"),
            ("Ctrl P", "Actions"),
            ("Ctrl C", "Quit"),
            ("?", "Help"),
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
        if app.dialog.is_none()
            && app.input.is_none()
            && !app.help
            && app.view == View::Files
            && app.browser.as_ref().is_some_and(|b| b.preview.is_none())
        {
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
                Span::styled(format!(" {key} "), key_style),
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
            if input == Input::Add { 64 } else { 76 },
            if input == Input::Palette {
                16
            } else if matches!(input, Input::Rename | Input::Command | Input::Add) {
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
            let editing = matches!(input, Input::Rename | Input::Command | Input::Add);
            let cursor_width = if editing {
                Span::raw(safe_label(if input == Input::Add {
                    &app.text
                } else {
                    &app.text[..app.rename_cursor]
                }))
                .width() as u16
                    + 2
            } else {
                0
            };
            let scroll = cursor_width.saturating_sub(rect.width.saturating_sub(3));
            frame.render_widget(
                Paragraph::new(
                    if matches!(input, Input::Rename | Input::Command | Input::Add) {
                        format!("> {}", safe_label(&app.text))
                    } else {
                        format!("> {}\nEnter confirm · Escape cancel", safe_label(&app.text))
                    },
                )
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
                    } else if input == Input::Add && app.network_add_target.is_some() {
                        let (d, address) = app.network_add_target.as_ref().unwrap();
                        format!(
                            "SSH account · {} via {}",
                            safe_label(address),
                            safe_label(&app.devices[*d].name)
                        )
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
            Dialog::StopShell(d, session) => (
                "Stop shell?".into(), vec!["Keep shell".into(), "Stop shell".into()],
                format!("Running commands in this shell will end.\n{}\n{}\n{}", identity(&app.devices[*d]), safe_label(&session.name), safe_label(&session.directory)),
            ),
            Dialog::Delete(d, entries) => (
                format!("Delete {} {}?", entries.len(), if entries.len() == 1 { "item" } else { "items" }),
                vec!["Cancel · keep files".into(), "Delete permanently".into()],
                format!("{}\n{}\n{}",
                    if entries.iter().any(|e| e.kind == "directory") { "No undo · folders include their contents." } else { "Deletion cannot be undone." }, identity(&app.devices[*d]),
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
            Dialog::Peer(d) => (
                format!("Device · {}", safe_label(&app.devices[*d].name)),
                vec!["Open sessions".into(), "Browse files".into(), "New shell".into(), "New session".into()],
                format!("{}\n{}\nFrom viewer · helper evidence\nVia selected device: unknown", identity(&app.devices[*d]), app.peer_status(*d)),
            ),
            Dialog::Neighbor(d, candidate) => (
                format!("Neighbor · {}", safe_label(candidate["address"].as_str().unwrap_or("unknown"))),
                vec![if neighbor_connectable(candidate) { "Connect via this device".into() } else { "Connect unavailable · link-local scope".into() }],
                format!("Via {}\n{}\nEnter chooses · Escape cancels", identity(&app.devices[*d]), neighbor_detail(candidate)),
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
        if matches!(dialog, Dialog::Delete(..) | Dialog::StopShell(..)) {
            let stop = matches!(dialog, Dialog::StopShell(..));
            let rect = confirmation_popup(area, &detail);
            frame.render_widget(Clear, rect);
            frame.render_widget(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(if ascii() {
                        ratatui::widgets::BorderType::Plain
                    } else {
                        ratatui::widgets::BorderType::Rounded
                    })
                    .border_style(tint(Color::Red))
                    .title(Line::from(Span::styled(
                        format!(" {title} "),
                        tint(Color::Red).add_modifier(Modifier::BOLD),
                    ))),
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
                    Constraint::Min(2),
                    Constraint::Length(1),
                    Constraint::Length(3),
                    Constraint::Length(1),
                ])
                .split(inner);
            frame.render_widget(
                Paragraph::new(detail)
                    .wrap(Wrap { trim: false })
                    .scroll((app.dialog_scroll, 0)),
                rows[0],
            );
            confirmation_buttons(
                frame,
                rows[2],
                stop,
                app.dialog_selected,
                app.dialog_detail_focus,
                ConfirmationLayout::Separate,
            );
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
        if app.view == View::Work {
            help.push(key_row("d", "Stop selected cx-managed shell · confirm"));
        }
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
        if app.view == View::Work
            && app
                .selected_session()
                .is_some_and(|(_, s)| !s.external && s.provider == "shell")
        {
            help.push(key_row(
                "d",
                "Stop selected cx-managed shell · confirmation",
            ));
        }
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
fn preview_inline(text: &str) -> Vec<Span<'static>> {
    // A deliberately small prose renderer: no HTML execution, links or image fetches.
    let mut spans = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        let found = rest.char_indices().find(|(_, c)| *c == '`' || *c == '*');
        let Some((at, marker)) = found else {
            spans.push(Span::raw(rest.to_owned()));
            break;
        };
        if at > 0 {
            spans.push(Span::raw(rest[..at].to_owned()));
        }
        let token = if marker == '*' && rest[at..].starts_with("**") {
            "**"
        } else if marker == '*' {
            "*"
        } else {
            "`"
        };
        let after = &rest[at + token.len()..];
        if let Some(end) = after.find(token) {
            let style = match token {
                "`" => tint(Color::Yellow),
                "**" => Style::default().add_modifier(Modifier::BOLD),
                _ => Style::default().add_modifier(Modifier::ITALIC),
            };
            spans.push(Span::styled(after[..end].to_owned(), style));
            rest = &after[end + token.len()..];
        } else {
            spans.push(Span::raw(rest[at..].to_owned()));
            break;
        }
    }
    spans
}
fn preview_lines(text: &str, kind: &str) -> Vec<Line<'static>> {
    let safe = safe_text(text);
    let code = if kind == "markdown" {
        crate::syntax_preview::fenced_lines(&safe)
    } else {
        Vec::new()
    };
    let mut fenced = false;
    safe.lines()
        .enumerate()
        .map(|(index, line)| {
            if kind == "markdown" {
                if let Some(Some(highlighted)) = code.get(index) {
                    return highlighted.clone();
                }
                if line.trim_start().starts_with("```") || line.trim_start().starts_with("~~~") {
                    fenced = !fenced;
                    return Line::from(Span::styled(line.to_owned(), muted()));
                }
                if fenced {
                    return Line::from(Span::styled(line.to_owned(), tint(Color::Yellow)));
                }
                let hashes = line.chars().take_while(|c| *c == '#').count();
                if (1..=6).contains(&hashes) && line.as_bytes().get(hashes) == Some(&b' ') {
                    return Line::from(Span::styled(
                        line[hashes + 1..].to_owned(),
                        accent().add_modifier(Modifier::BOLD),
                    ));
                }
                if line.trim_start().starts_with("- ") || line.trim_start().starts_with("* ") {
                    let prefix = line.len() - line.trim_start().len();
                    let mut spans = vec![Span::styled(
                        format!("{}{} ", " ".repeat(prefix), if ascii() { "-" } else { "•" }),
                        accent(),
                    )];
                    spans.extend(preview_inline(&line.trim_start()[2..]));
                    return Line::from(spans);
                }
                return Line::from(preview_inline(line));
            }
            if kind == "code" {
                let trimmed = line.trim_start();
                let style = if trimmed.starts_with("//") || trimmed.starts_with('#') {
                    muted()
                } else if [
                    "fn ",
                    "pub ",
                    "let ",
                    "use ",
                    "def ",
                    "class ",
                    "import ",
                    "const ",
                    "function ",
                ]
                .iter()
                .any(|word| trimmed.starts_with(word))
                {
                    accent()
                } else {
                    Style::default()
                };
                return Line::from(Span::styled(line.to_owned(), style));
            }
            Line::raw(line.to_owned())
        })
        .collect()
}
fn raster_lines(w: usize, h: usize, bytes: &[u8], area: Rect) -> Vec<Line<'static>> {
    let available_w = usize::from(area.width);
    let available_h = usize::from(area.height) * 2;
    if available_w == 0 || available_h == 0 {
        return vec![];
    }
    let scale = (available_w as f64 / w as f64).min(available_h as f64 / h as f64);
    let width = ((w as f64 * scale).floor() as usize).max(1);
    let height = ((h as f64 * scale).floor() as usize).max(1);
    let pixel = |x: usize, y: usize| -> Option<Color> {
        if y >= height {
            return None;
        }
        let at = ((y * h / height) * w + x * w / width) * 4;
        (bytes[at + 3] >= 128).then_some(Color::Rgb(bytes[at], bytes[at + 1], bytes[at + 2]))
    };
    (0..height.div_ceil(2))
        .map(|row| {
            Line::from(
                (0..width)
                    .map(|x| match (pixel(x, row * 2), pixel(x, row * 2 + 1)) {
                        (Some(top), Some(bottom)) => {
                            Span::styled("▀", Style::default().fg(top).bg(bottom))
                        }
                        (Some(top), None) => Span::styled("▀", Style::default().fg(top)),
                        (None, Some(bottom)) => Span::styled("▄", Style::default().fg(bottom)),
                        (None, None) => Span::raw(" "),
                    })
                    .collect::<Vec<_>>(),
            )
        })
        .collect()
}
fn confirmation_popup(area: Rect, detail: &str) -> Rect {
    let width = popup(area, 68, 12).width.saturating_sub(4).max(1);
    let lines = detail
        .lines()
        .map(|line| Line::raw(line).width().max(1).div_ceil(usize::from(width)))
        .sum::<usize>()
        .clamp(2, 6);
    popup(area, 68, lines as u16 + 7)
}

#[derive(Clone, Copy)]
enum ConfirmationLayout {
    Separate,
    #[cfg(test)]
    Joined,
    #[cfg(test)]
    Compact,
}
fn confirmation_buttons(
    frame: &mut Frame,
    area: Rect,
    stop: bool,
    selected: usize,
    detail_focus: bool,
    layout: ConfirmationLayout,
) {
    frame.render_widget(Clear, area);
    let labels = if stop {
        if area.width < 38 {
            ["Keep", "Stop"]
        } else {
            ["Keep shell", "Stop shell"]
        }
    } else {
        ["Cancel", "Delete"]
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
                .border_type(if ascii() {
                    BorderType::Plain
                } else {
                    BorderType::Rounded
                })
                .border_style(muted()),
            area,
        );
    }
    for (i, label) in labels.into_iter().enumerate() {
        let active = selected == i && !detail_focus;
        let color = if i == 1 { Color::Red } else { Color::Cyan };
        let emphasis = if active {
            Modifier::BOLD
        } else {
            Modifier::empty()
        };
        let style = tint(color).add_modifier(emphasis);
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
                tint(color).add_modifier(Modifier::BOLD),
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
                                .border_type(if ascii() {
                                    BorderType::Plain
                                } else {
                                    BorderType::Rounded
                                })
                                .border_style(if active { style } else { muted() }),
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

fn render_preview(frame: &mut Frame, area: Rect, browser: &Browser, text: &str, focused: bool) {
    render_preview_with_native(frame, area, browser, text, focused, None)
}
fn render_preview_with_native(
    frame: &mut Frame,
    area: Rect,
    browser: &Browser,
    text: &str,
    focused: bool,
    native: Option<&mut crate::terminal_preview::NativePreview>,
) {
    let rich = browser.preview_rich.as_ref();
    let pdf_title = rich.filter(|p| p.kind == "pdf").map(|p| {
        format!(
            "PDF · {}/{}{}",
            p.page,
            p.pages.map(|n| n.to_string()).unwrap_or("?".into()),
            browser
                .preview_pending_page
                .map(|page| format!(" · {} page {}", if ascii() { "~" } else { "◌" }, page))
                .unwrap_or_default()
        )
    });
    let title = pdf_title
        .as_deref()
        .or_else(|| rich.map(|p| p.title.as_str()))
        .filter(|t| !t.is_empty())
        .unwrap_or("Preview");
    let border = block(
        format!(
            "{} · Escape back",
            fit_label(title, usize::from(area.width.saturating_sub(22)))
        ),
        focused,
    );
    let inner = border.inner(area);
    frame.render_widget(border, area);
    if native.is_some_and(|native| native.render(frame, inner)) {
        return;
    }
    if !ascii() && std::env::var_os("NO_COLOR").is_none() {
        if let Some((w, h, bytes)) = rich.and_then(|p| p.raster.as_ref()) {
            frame.render_widget(Paragraph::new(raster_lines(*w, *h, bytes, inner)), inner);
            return;
        }
    }
    let fallback;
    let text = if rich.is_some_and(|p| p.raster.is_some()) {
        fallback = format!(
            "{}\n{}\nColor thumbnail unavailable in ASCII/monochrome mode.",
            title, text
        );
        fallback.as_str()
    } else {
        text
    };
    frame.render_widget(
        Paragraph::new(if !ascii() && std::env::var_os("NO_COLOR").is_none() {
            rich.and_then(|p| p.styled.clone()).unwrap_or_else(|| {
                preview_lines(text, rich.map(|p| p.kind.as_str()).unwrap_or("text"))
            })
        } else {
            safe_text(text)
                .lines()
                .map(|line| Line::raw(line.to_owned()))
                .collect()
        })
        .wrap(Wrap { trim: false })
        .scroll((browser.preview_scroll, 0)),
        inner,
    );
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
        render_preview(frame, parts[1], b, preview, focused);
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
                .enumerate()
                .map(|(index, e)| {
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
                    let selected = index == b.selected;
                    let name_style = if selected && focused {
                        // Swap the terminal's default pair for reliable light/dark
                        // contrast; the separate cursor rail carries the accent.
                        Style::default().add_modifier(Modifier::BOLD | Modifier::REVERSED)
                    } else if selected {
                        Style::default().add_modifier(Modifier::BOLD)
                    } else if e.kind == "directory" {
                        tint(Color::Blue).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default()
                    };
                    Row::new(vec![
                        Cell::from(if selected {
                            if focused && !ascii() {
                                "▌"
                            } else {
                                ">"
                            }
                        } else {
                            " "
                        })
                        .style(if focused {
                            accent().add_modifier(Modifier::BOLD)
                        } else {
                            muted()
                        }),
                        Cell::from(marker).style(style.add_modifier(Modifier::BOLD)),
                        Cell::from(match e.kind.as_str() {
                            "directory" => "/",
                            "symlink" => "@",
                            _ => " ",
                        })
                        .style(accent()),
                        Cell::from(Line::from(Span::styled(
                            format!(
                                " {} ",
                                compact_path(&e.name, area.width.saturating_sub(20) as usize)
                            ),
                            name_style,
                        ))),
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
                        Constraint::Length(2),
                        Constraint::Length(1),
                        Constraint::Length(1),
                        Constraint::Min(1),
                        Constraint::Length(8),
                    ],
                )
                .column_spacing(1)
                .block(block(format!("{} items", rows.len()), focused))
                .row_highlight_style(Style::default()),
                parts[1],
                &mut state,
            );
        }
    }
    let bottom = if b.preview.is_some() {
        " Preview · j/k scroll · Escape back".into()
    } else if let Some(c) = clipboard {
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
// Network browser rendering: evidence remains scoped to the selected enrolled device.
fn neighbor_connectable(candidate: &Value) -> bool {
    !matches!(candidate["address"].as_str().and_then(|s| s.parse::<std::net::IpAddr>().ok()), Some(std::net::IpAddr::V6(ip)) if ip.is_unicast_link_local())
}
fn neighbor_ssh(candidate: &Value) -> &'static str {
    let observed = candidate["observed_at"].as_u64().unwrap_or(0);
    if observed == 0 || transport::now().saturating_sub(observed) > 90 {
        return "unknown";
    }
    match candidate["ssh"]["state"].as_str() {
        Some("open" | "tcp_reachable") => "port open",
        Some("closed") => "port closed",
        _ => "unknown",
    }
}
fn neighbor_detail(candidate: &Value) -> String {
    format!(
        "Host {} · link {}\nSSH {} · authentication unknown\nInternet unknown · source {}",
        safe_label(candidate["hostname"].as_str().unwrap_or("unknown")),
        safe_label(candidate["link_state"].as_str().unwrap_or("unknown")),
        neighbor_ssh(candidate),
        safe_label(candidate["source"].as_str().unwrap_or("unknown"))
    )
}
fn render_network(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let d = app.network_device();
    let title = d
        .map(|d| format!("Network · {}", identity(&app.devices[d])))
        .unwrap_or_else(|| "Network · select a device".into());
    let title = if app.device == 0 {
        "Network · All devices".to_string()
    } else {
        title
    };
    let outer = block(title, app.focus == Focus::Workspace);
    let inner = outer.inner(area);
    frame.render_widget(outer, area);
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(6),
            Constraint::Min(2),
            Constraint::Length(5),
        ])
        .split(inner);
    let value = d.and_then(|d| app.network.get(&d));
    let summary = value.map(|v| {
        let observed = v["internet"]["observed_at"].as_u64().unwrap_or(0);
        let age = transport::now().saturating_sub(observed);
        let stale = v["internet"]["stale"].as_bool().unwrap_or(true) || age > 90;
        let internet = if v["internet"]["state"] == "reachable" && !stale { "reachable (ICMP)" } else { "unknown" };
        let freshness = if d.is_some_and(|d| app.network_inflight.contains(&d)) { "refreshing".into() }
            else if observed == 0 { "observation age unknown".into() }
            else { format!("{age}s ago{}", if stale { " · stale" } else { "" }) };
        let interfaces = v["interfaces"].as_array().or_else(|| v["interfaces"]["data"].as_array())
            .map(|rows| rows.iter().take(8).map(|i| format!("{} {}", safe_label(i["ifname"].as_str().or_else(|| i["name"].as_str()).unwrap_or("?")), safe_label(i["operstate"].as_str().or_else(|| i["state"].as_str()).unwrap_or("unknown")))).collect::<Vec<_>>().join(" · ")).unwrap_or_else(|| "unknown".into());
        let routes = v["routes"].as_array().or_else(|| v["routes"]["data"].as_array()).map(|r| format!("{} observed", r.len())).unwrap_or_else(|| "unknown".into());
        format!("{} · Internet {internet} · {freshness}\nInterfaces: {interfaces}\nRoutes: {routes} · DNS / HTTPS unknown\nSharing: {}\nCached neighbors · automatic SSH-port checks", d.map(|d| identity(&app.devices[d])).unwrap_or_else(|| "unknown".into()), safe_label(v["sharing"]["state"].as_str().unwrap_or("unknown")))
    }).unwrap_or_else(|| if app.network_loading { "Reading network evidence…".into() } else { "No network observation · select device and refresh".into() });
    frame.render_widget(
        Paragraph::new(summary).wrap(Wrap { trim: false }),
        sections[0],
    );
    let candidates = app.network_rows();
    if candidates.is_empty() {
        frame.render_widget(
            Paragraph::new("No enrolled devices or observed LAN neighbors"),
            sections[1],
        );
        return;
    }
    let compact = sections[1].width < 45;
    let rows = candidates.iter().map(|c| {
        if let Some(d) = c["_peer"].as_u64().map(|d| d as usize) {
            let device = &app.devices[d];
            if compact {
                let status = app.peer_status(d);
                let label = if status.contains("checking") {
                    "checking"
                } else if status.contains("reached") {
                    "ready"
                } else if status.contains("unavailable") {
                    "offline"
                } else if status.contains("cached") {
                    "cached"
                } else {
                    "unknown"
                };
                return Row::new(vec![
                    Cell::from(format!("P {}", safe_label(&device.name))),
                    Cell::from(label),
                ]);
            }
            Row::new(vec![
                Cell::from(safe_label(&device.name)),
                Cell::from(identity(device)),
                Cell::from("Enrolled"),
                Cell::from(app.peer_status(d)),
            ])
        } else {
            if compact {
                return Row::new(vec![
                    Cell::from(format!(
                        "L {}",
                        safe_label(c["address"].as_str().unwrap_or("unknown"))
                    )),
                    Cell::from(match neighbor_ssh(c) {
                        "port open" => "open",
                        "port closed" => "closed",
                        _ => "unknown",
                    }),
                ]);
            }
            Row::new(vec![
                Cell::from(format!(
                    "{} / {}",
                    c["_device"]
                        .as_u64()
                        .and_then(|d| app.devices.get(d as usize))
                        .map(|d| safe_label(&d.name))
                        .unwrap_or_else(|| "unknown".into()),
                    safe_label(c["interface"].as_str().unwrap_or("unknown"))
                )),
                Cell::from(safe_label(c["address"].as_str().unwrap_or("unknown"))),
                Cell::from("LAN cache"),
                Cell::from(neighbor_ssh(c)),
            ])
        }
    });
    let table = Table::new(
        rows,
        if compact {
            vec![Constraint::Min(13), Constraint::Length(8)]
        } else {
            vec![
                Constraint::Percentage(25),
                Constraint::Percentage(30),
                Constraint::Length(8),
                Constraint::Percentage(30),
            ]
        },
    )
    .header(
        Row::new(if compact {
            vec!["P peer / L LAN", "Access"]
        } else {
            vec!["Device / link", "Execution / address", "Source", "Evidence"]
        })
        .style(muted()),
    )
    .row_highlight_style(selected_style());
    let mut state = TableState::default().with_selected(Some(app.network_selected));
    frame.render_stateful_widget(table, sections[1], &mut state);
    if let Some(candidate) = candidates.get(app.network_selected) {
        let detail = if let Some(d) = candidate["_peer"].as_u64().map(|d| d as usize) {
            format!(
                "{} · {}\n{}\nFrom viewer · helper evidence\nVia selected device: unknown",
                safe_label(&app.devices[d].name),
                identity(&app.devices[d]),
                app.peer_status(d)
            )
        } else {
            format!(
                "{} · {}\n{}",
                candidate["_device"]
                    .as_u64()
                    .and_then(|d| app.devices.get(d as usize))
                    .map(identity)
                    .unwrap_or_else(|| "unknown".into()),
                safe_label(candidate["address"].as_str().unwrap_or("unknown")),
                neighbor_detail(candidate)
            )
        };
        frame.render_widget(
            Paragraph::new(detail).wrap(Wrap { trim: false }),
            sections[2],
        );
    }
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

// Secrets have their own short-lived editor; they never enter App/cache/snapshots.
struct SecretInput(String);
impl Drop for SecretInput {
    fn drop(&mut self) {
        unsafe {
            for b in self.0.as_bytes_mut() {
                std::ptr::write_volatile(b, 0);
            }
        }
    }
}
fn authentication_modal(
    screen: &mut Screen,
    prompt: crate::auth::Prompt,
) -> Result<crate::auth::Answer> {
    use crate::auth::{Answer, PromptKind};
    let trust = prompt.kind == PromptKind::HostKey;
    let title = match prompt.kind {
        PromptKind::HostKey => "Verify host key",
        PromptKind::Password => "SSH password",
        PromptKind::KeyPassphrase => "Unlock SSH key",
        PromptKind::Verification => "SSH verification",
    };
    let mut secret = SecretInput(String::with_capacity(8192));
    let started = Instant::now();
    let mut scroll = 0u16;
    let mut reviewed = false;
    loop {
        if started.elapsed() > Duration::from_secs(240) {
            return Ok(Answer::Cancel);
        }
        screen.terminal.draw(|f| {
            let area = popup(f.area(), 76, if trust { 13 } else { 7 });
            f.render_widget(Clear, area);
            let panel = block(title.into(), true).padding(Padding::horizontal(1));
            let inner = panel.inner(area);
            f.render_widget(panel, area);
            if trust {
                let content = Rect {
                    height: inner.height.saturating_sub(2),
                    ..inner
                };
                let mut lines = Vec::new();
                for original in prompt.text.lines() {
                    let mut line = String::new();
                    let mut width = 0usize;
                    for c in safe_label(original).chars() {
                        let cells = if c.is_ascii() { 1 } else { 2 };
                        if width + cells > content.width as usize && !line.is_empty() {
                            lines.push(Line::from(std::mem::take(&mut line)));
                            width = 0;
                        }
                        line.push(c);
                        width += cells;
                    }
                    lines.push(Line::from(line));
                }
                let last = lines
                    .len()
                    .saturating_sub(content.height as usize)
                    .min(u16::MAX as usize) as u16;
                scroll = scroll.min(last);
                reviewed |= content.height > 0 && scroll >= last;
                f.render_widget(Paragraph::new(lines).scroll((scroll, 0)), content);
                let footer = Rect {
                    y: inner.y + inner.height.saturating_sub(1),
                    height: 1,
                    ..inner
                };
                f.render_widget(
                    Paragraph::new(if reviewed {
                        "y Trust fingerprint   n Cancel"
                    } else {
                        "↓ Review fingerprint   n Cancel"
                    })
                    .style(Style::default().fg(Color::Yellow)),
                    footer,
                );
            } else {
                let mut lines: Vec<Line> = prompt
                    .text
                    .lines()
                    .take(2)
                    .map(|line| Line::from(safe_label(line)))
                    .collect();
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    "*".repeat(secret.0.chars().count().min(inner.width as usize)),
                    Style::default().fg(Color::Cyan),
                )));
                f.render_widget(Paragraph::new(lines), inner);
            }
        })?;
        if !event::poll(Duration::from_millis(100))? {
            continue;
        }
        match event::read()? {
            Event::Key(key) if key.kind != KeyEventKind::Release => {
                if key.code == KeyCode::Esc
                    || (key.code == KeyCode::Char('c')
                        && key.modifiers.contains(KeyModifiers::CONTROL))
                {
                    return Ok(Answer::Cancel);
                }
                if trust {
                    match key.code {
                        KeyCode::Char('y') if reviewed => return Ok(Answer::Submit("yes".into())),
                        KeyCode::Char('n') => return Ok(Answer::Cancel),
                        KeyCode::Down | KeyCode::PageDown => scroll = scroll.saturating_add(1),
                        KeyCode::Up | KeyCode::PageUp => scroll = scroll.saturating_sub(1),
                        _ => {}
                    }
                } else {
                    match key.code {
                        KeyCode::Enter => return Ok(Answer::Submit(std::mem::take(&mut secret.0))),
                        KeyCode::Backspace => {
                            if let Some((index, _)) = secret.0.char_indices().last() {
                                unsafe {
                                    for b in &mut secret.0.as_bytes_mut()[index..] {
                                        std::ptr::write_volatile(b, 0);
                                    }
                                }
                                secret.0.truncate(index);
                            }
                        }
                        KeyCode::Char(c)
                            if !c.is_control()
                                && !key
                                    .modifiers
                                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                                && secret.0.len() + c.len_utf8() < 8192 =>
                        {
                            secret.0.push(c)
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
}

struct Screen {
    terminal: Terminal<CrosstermBackend<io::Stdout>>,
    active: bool,
    mouse: bool,
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
            mouse: false,
        })
    }
    fn preview_mouse(&mut self, enabled: bool) -> Result<()> {
        if enabled != self.mouse {
            if enabled {
                execute!(self.terminal.backend_mut(), EnableMouseCapture)?;
            } else {
                execute!(self.terminal.backend_mut(), DisableMouseCapture)?;
            }
            self.mouse = enabled;
        }
        Ok(())
    }
    fn suspend(&mut self) -> Result<()> {
        if self.active {
            let mouse = self.preview_mouse(false);
            self.active = false;
            let raw = terminal::disable_raw_mode();
            let leave = execute!(self.terminal.backend_mut(), LeaveAlternateScreen);
            let cursor = self.terminal.show_cursor();
            mouse?;
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
        && app.network_add_target.is_none()
        && app.pending_add_via.is_none()
        && app.pending_requests.get() == 0
        && !app.file_busy
        && app.file_queue.is_empty()
        && app.browser.as_ref().is_none_or(|b| b.preview.is_none())
}

fn restart_ready(app: &App, idle: Duration, pending_input: bool) -> bool {
    !pending_input && idle >= Duration::from_secs(3) && can_restart(app)
}
// Align notices with the padded footer keys and sidebar text, including narrow
// layouts. The notice remains on the terminal's default background.
fn notice_area(area: Rect) -> Rect {
    area.inner(ratatui::layout::Margin::new(2, 0))
}
fn version_notice(message: &str) -> String {
    format!(
        "cx v{} {} {}",
        env!("CARGO_PKG_VERSION"),
        if ascii() { "|" } else { "·" },
        safe_text(message)
    )
}
fn update_check_notice(result: &Result<crate::update::CheckOutcome>) -> String {
    use crate::update::CheckOutcome;
    version_notice(&match result {
        Ok(CheckOutcome::Offline) => "update unreachable; retry".into(),
        Ok(CheckOutcome::Unavailable(message)) => safe_text(message),
        Ok(CheckOutcome::Skipped) => "update check already running".into(),
        Ok(CheckOutcome::Current) => "up to date".into(),
        Ok(CheckOutcome::Ready(plan)) => format!(
            "v{} ready · waiting for idle workspace",
            safe_label(&plan.version)
        ),
        Err(_) => "update unavailable; version kept".into(),
    })
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
                Operation::List { .. }
                    | Operation::ListPage { .. }
                    | Operation::Preview { .. }
                    | Operation::PreviewPage { .. }
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
                let preview = if matches!(
                    task.op,
                    Operation::Preview { .. } | Operation::PreviewPage { .. }
                ) {
                    result.as_ref().ok().map(RichPreview::from_value)
                } else {
                    None
                };
                let delivered = replies
                    .send(Reply {
                        preview,
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

/// Geometry mirrors the existing workspace layout; no preview owns another location.
fn native_preview_area(app: &App, area: Rect) -> Option<Rect> {
    if area.width < 36
        || area.height < 10
        || app.view != View::Files
        || app.help
        || app.dialog.is_some()
        || app.input.is_some()
    {
        return None;
    }
    let browser = app.browser.as_ref()?;
    browser.preview.as_ref()?;
    let rich = browser.preview_rich.as_ref()?;
    if !matches!(rich.kind.as_str(), "image" | "pdf") || rich.raster.is_none() {
        return None;
    }
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(5),
            Constraint::Length(if browser.search.is_empty() { 4 } else { 7 }),
        ])
        .split(area);
    let sidebar_width = if area.width < 60 && app.focus == Focus::Workspace {
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
    let workspace = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(5),
            Constraint::Length(if app.transfer_drawer && content[1].height >= 12 {
                4
            } else {
                0
            }),
        ])
        .split(content[1]);
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(1)])
        .split(workspace[0]);
    let inner = Block::default().borders(Borders::ALL).inner(rows[1]);
    (inner.width > 0 && inner.height > 0).then_some(inner)
}
fn prepare_native_preview(
    app: &App,
    area: Rect,
    native: &mut crate::terminal_preview::NativePreview,
) {
    if let Some(area) = native_preview_area(app, area) {
        let browser = app.browser.as_ref().unwrap();
        let (width, height, rgba) = browser
            .preview_rich
            .as_ref()
            .unwrap()
            .raster
            .as_ref()
            .unwrap();
        // Revision changes on every accepted response, even if title/path/dimensions match.
        let key = format!(
            "{}:{}:{}",
            app.generation, browser.device, browser.preview_revision
        );
        native.prepare(&key, *width as u32, *height as u32, rgba, area);
    } else {
        native.hide();
    }
}
fn cleanup_native_preview(
    screen: &mut Screen,
    native: &mut crate::terminal_preview::NativePreview,
) -> Result<()> {
    native.hide();
    if native.cleanup(&mut io::stdout())? {
        screen.terminal.clear()?;
    }
    Ok(())
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
    // Drop ordering cleans owned bitmaps before Screen restores the original terminal.
    let mut native_preview = crate::terminal_preview::NativePreview::detect();
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
        for update in transport::drain_host_updates() {
            app.host_updated(update);
            dirty = true;
        }
        app.start_next_file_action();
        if app.force_update {
            app.force_update = false;
            if matches!(update_phase, UpdatePhase::Idle) {
                start_update_check(&update_tx, true);
                update_phase = UpdatePhase::Checking;
                last_update_check = Instant::now();
                app.notice = version_notice("checking verified releases…");
                dirty = true;
            } else if matches!(update_phase, UpdatePhase::Checking) {
                update_report_requested = true;
                app.notice = version_notice("checking verified releases…");
                dirty = true;
            }
        }
        while let Ok(event) = update_rx.try_recv() {
            match event {
                UpdateEvent::Checked(force, result) => {
                    let force = force || std::mem::take(&mut update_report_requested);
                    update_phase = match result {
                        Ok(crate::update::CheckOutcome::Ready(plan)) => {
                            app.notice = version_notice(&format!(
                                "v{} ready · updating when workspace is idle",
                                safe_label(&plan.version)
                            ));
                            dirty = true;
                            UpdatePhase::Ready(plan)
                        }
                        other => {
                            if force {
                                app.notice = update_check_notice(&other);
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
                            app.notice = version_notice("update could not install; version kept");
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
                    app.notice = version_notice("installing verified update…");
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
                        cleanup_native_preview(&mut screen, &mut native_preview)?;
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
            cleanup_native_preview(&mut screen, &mut native_preview)?;
            let via = app.pending_add_via.take();
            let owned_target = target.clone();
            app.notice = format!("Connecting to {}", safe_label(&target));
            screen
                .terminal
                .draw(|frame| render_with_native(frame, &app, None))?;
            let result = crate::auth::with_broker(
                move || crate::add_via(&owned_target, via.as_ref()),
                |prompt| authentication_modal(&mut screen, prompt),
            );
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
            cleanup_native_preview(&mut screen, &mut native_preview)?;
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
            cleanup_native_preview(&mut screen, &mut native_preview)?;
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
        let was_encoding = native_preview.pending();
        let size = screen.terminal.size()?;
        prepare_native_preview(
            &app,
            Rect::new(0, 0, size.width, size.height),
            &mut native_preview,
        );
        if was_encoding && !native_preview.pending() {
            // The fitted native image may occupy fewer cells than the fallback.
            // Reset the terminal cache so old half-block rows cannot survive.
            screen.terminal.clear()?;
            dirty = true;
        }
        if native_preview.cleanup(&mut io::stdout())? {
            screen.terminal.clear()?;
            dirty = true;
        }
        screen.preview_mouse(
            app.pdf_preview_active() && !app.help && app.dialog.is_none() && app.input.is_none(),
        )?;
        if dirty {
            screen
                .terminal
                .draw(|frame| render_with_native(frame, &app, Some(&mut native_preview)))?;
            dirty = false;
        }
        if event::poll(Duration::from_millis(100))? {
            match event::read()? {
                Event::Key(k) => {
                    last_interaction = Instant::now();
                    app.key(k);
                    dirty = true;
                }
                Event::Resize(_, _) => {
                    native_preview.hide();
                    dirty = true;
                }
                Event::Mouse(mouse) => {
                    let size = screen.terminal.size()?;
                    dirty |= app.preview_mouse(mouse, Rect::new(0, 0, size.width, size.height));
                }
                _ => {}
            }
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
    cleanup_native_preview(&mut screen, &mut native_preview)?;
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
    fn network_fixture(a: &mut App) {
        a.view = View::Network;
        a.focus = Focus::Workspace;
        a.network_selected = a.devices.len();
        a.network.insert(0, serde_json::json!({"internet":{"state":"unknown"}, "candidates":[
            {"address":"192.0.2.2","interface":"eth0","source":"neighbor","link_state":"STALE","ssh":{"state":"unknown"}},
            {"address":"192.0.2.1","interface":"eth0","source":"neighbor","link_state":"REACHABLE","observed_at":transport::now(),"ssh":{"state":"open"}}
        ]}));
    }
    #[test]
    fn network_enrolled_peers_exist_without_lan_and_capture_execution_identity() {
        let (mut a, rx) = queued_app();
        a.view = View::Network;
        a.device = 1;
        assert!(a.network_candidates().is_empty());
        let rows = a.network_rows();
        assert_eq!(rows.len(), a.devices.len());
        assert_eq!(rows[1]["_peer"], 1);
        a.network_selected = 1;
        a.open_neighbor();
        assert!(matches!(a.dialog, Some(Dialog::Peer(1))));
        a.device = 2; // Action identity was captured before scope changed.
        a.dialog_selected = 1;
        a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(a.browser.as_ref().unwrap().device, 1);
        assert!(rx
            .try_iter()
            .any(|t| t.device == 1 && matches!(t.op, Operation::List { .. })));
        assert!(a.pending_add.is_none());
    }
    #[test]
    fn network_peer_checks_bounded_cached_offline_and_no_redraw_requests() {
        let (mut a, rx) = queued_app();
        a.view = View::Network;
        a.device = 1;
        a.providers.clear();
        for index in 2..7 {
            let mut d = a.devices[1].clone();
            d.id = format!("peer-{index}");
            a.devices.push(d);
            a.work.push(Cached {
                sessions: vec![],
                loading: false,
                error: None,
                fetched: 0,
            });
        }
        a.refresh();
        let tasks: Vec<_> = rx.try_iter().collect();
        assert_eq!(
            tasks
                .iter()
                .filter(|t| matches!(t.op, Operation::Info))
                .count(),
            3
        );
        assert!(tasks
            .iter()
            .filter(|t| matches!(t.op, Operation::Network | Operation::NetworkCandidates))
            .all(|t| t.device == 0));
        assert_eq!(a.peer_inflight.len(), 3);
        a.apply(Reply {
            device: 1,
            generation: a.generation,
            op: Operation::Info,
            result: Err(anyhow::anyhow!("offline")),
            preview: None,
        });
        assert_eq!(a.peer_inflight.len(), 3);
        assert!(a.peer_status(1).contains("unavailable"));
        let next: Vec<_> = rx.try_iter().collect();
        assert!(next
            .iter()
            .any(|t| t.device == 3 && matches!(t.op, Operation::Info)));
        a.peer_evidence
            .insert(1, (transport::now().saturating_sub(100), true));
        assert!(a.peer_status(1).contains("cached helper"));
        a.network_selected = 1;
        let text = capture_app(&a, 120);
        assert!(text.contains("laptop"));
        assert!(text.contains("From viewer"));
        assert!(text.contains("Via selected device: unknown"));
        assert!(rx.try_recv().is_err());
    }
    #[test]
    fn network_peer_selection_survives_lan_reordering_and_actions_target_peer() {
        let (mut a, rx) = queued_app();
        network_fixture(&mut a);
        a.network_selected = 1;
        a.apply(Reply {
            device: 0,
            generation: a.generation,
            op: Operation::NetworkCandidates,
            result: Ok(
                serde_json::json!({"candidates":[{"address":"192.0.2.0","interface":"eth0"}]}),
            ),
            preview: None,
        });
        assert_eq!(a.network_selected, 1);
        assert_eq!(a.command_context(), Some((1, "~".into())));
        a.execute(Action::New);
        assert!(matches!(a.dialog, Some(Dialog::Provider(1, _))));
        a.dialog = None;
        a.execute(Action::Files);
        assert_eq!(a.browser.as_ref().unwrap().device, 1);
        assert!(rx
            .try_iter()
            .any(|t| t.device == 1 && matches!(t.op, Operation::List { .. })));
    }
    #[test]
    fn add_device_modal_is_compact_and_keeps_contextual_keys() {
        let (mut a, _rx) = queued_app();
        a.execute(Action::Add);
        let text = capture_app(&a, 100);
        assert!(text.contains("Add device"));
        assert!(!text.contains("Enter confirm"));
        assert!(!text.contains("Escape cancel"));
        a.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(a.input.is_none());
        assert!(a.pending_add.is_none());
    }
    #[test]
    fn network_numeric_order_stale_port_and_linklocal_enrollment() {
        let (mut a, _rx) = queued_app();
        network_fixture(&mut a);
        let stale = serde_json::json!({"address":"fe80::1","interface":"eth0","observed_at":transport::now().saturating_sub(100),"ssh":{"state":"tcp_reachable"}});
        assert_eq!(neighbor_ssh(&stale), "unknown");
        assert!(!neighbor_connectable(&stale));
        a.dialog = Some(Dialog::Neighbor(0, stale));
        a.dialog_selected = 1;
        a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(a.input.is_none());
        assert!(a.pending_add.is_none());
        a.network.insert(0, serde_json::json!({"candidates":[{"address":"192.0.2.100","interface":"eth0"},{"address":"192.0.2.2","interface":"eth0"}]}));
        assert_eq!(a.network_candidates()[0]["address"], "192.0.2.2");
    }
    #[test]
    fn network_browser_capture_matrix() {
        for width in [48, 80, 120] {
            for height in [24, 40] {
                for state in ["all", "unknown", "menu", "account"] {
                    let (mut a, _rx) = queued_app();
                    network_fixture(&mut a);
                    a.network.insert(1, serde_json::json!({"internet":{"state":"unknown"},"candidates":[{"address":"192.0.2.20","interface":"wlan0","source":"neighbor","link_state":"STALE","ssh":{"state":"unknown"}}]}));
                    if state == "unknown" {
                        a.device = 2;
                    }
                    if state == "menu" || state == "account" {
                        a.network_selected = a.devices.len() + 2;
                        a.open_neighbor();
                    }
                    if state == "account" {
                        press(&mut a, 'u');
                    }
                    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
                    terminal.draw(|f| render(f, &a)).unwrap();
                    let text = terminal
                        .backend()
                        .buffer()
                        .content
                        .chunks(width as usize)
                        .map(|row| row.iter().map(|c| c.symbol()).collect::<String>())
                        .collect::<Vec<_>>()
                        .join("\n");
                    assert!(text.contains("Network"));
                    if let Some(directory) = std::env::var_os("CX_NETWORK_CAPTURE_DIR") {
                        let directory = std::path::PathBuf::from(directory);
                        std::fs::create_dir_all(&directory).unwrap();
                        std::fs::write(
                            directory.join(format!("{width}x{height}-{state}.txt")),
                            text,
                        )
                        .unwrap();
                    }
                }
            }
        }
    }
    #[test]
    fn network_all_scope_batches_every_host_and_captures_neighbor_owner() {
        let (mut a, rx) = queued_app();
        network_fixture(&mut a);
        let template = a.devices[1].clone();
        for i in 2..7 {
            let mut d = template.clone();
            d.id = format!("remote-{i}");
            a.devices.push(d);
        }
        a.refresh();
        let tasks: Vec<_> = rx.try_iter().collect();
        assert_eq!(tasks.len(), 8);
        assert_eq!(a.network_inflight.len(), 4);
        assert_eq!(a.network_refresh_queue.len(), 3);
        for op in [Operation::Network, Operation::NetworkCandidates] {
            a.apply(Reply {
                device: 0,
                generation: a.generation,
                op,
                result: Err(anyhow::anyhow!("offline")),
                preview: None,
            });
        }
        let next: Vec<_> = rx.try_iter().collect();
        assert_eq!(next.len(), 2);
        assert!(next.iter().all(|t| t.device == 4));
        assert_eq!(a.network_inflight.len(), 4);
        a.network.insert(1, serde_json::json!({"candidates":[{"address":"192.0.2.1","interface":"eth0","source":"neighbor","ssh":{"state":"unknown"}}]}));
        a.network_selected = a.devices.len() + 2;
        a.open_neighbor();
        assert_eq!(a.network_add_target, Some((1, "192.0.2.1".into())));
        a.device = 1;
        press(&mut a, 'u');
        a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(a.pending_add.as_deref(), Some("u@192.0.2.1"));
        assert_eq!(a.pending_add_via.as_ref().unwrap().id, "remote");
        assert!(rx.try_recv().is_err());
    }
    #[test]
    fn network_fleet_reply_preserves_selected_owner_at_duplicate_address() {
        let (mut a, _rx) = queued_app();
        network_fixture(&mut a);
        a.network.insert(1, serde_json::json!({"candidates":[{"address":"192.0.2.1","interface":"eth0","source":"neighbor"}]}));
        a.network_selected = a.devices.len() + 2;
        a.apply(Reply {device:0, generation:a.generation, op:Operation::NetworkCandidates, result:Ok(serde_json::json!({"candidates":[{"address":"192.0.2.0","interface":"eth0","source":"neighbor"}]})), preview:None});
        assert_eq!(a.network_selected, a.devices.len() + 1);
        assert_eq!(a.network_device(), Some(1));
    }
    #[test]
    fn network_neighbors_sorted_and_unknown_is_honest() {
        let mut a = app();
        network_fixture(&mut a);
        assert_eq!(a.network_candidates()[0]["address"], "192.0.2.1");
        let text = capture_app(&a, 100);
        assert!(text.contains("Internet unknown"));
        assert!(text.contains("port open"));
        assert!(text.contains("authentication unknown"));
        assert!(text.contains("Internet unknown · source neighbor"));
    }
    #[test]
    fn network_neighbor_enter_starts_account_without_manual_check() {
        let (mut a, rx) = queued_app();
        network_fixture(&mut a);
        a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(rx.try_recv().is_err());
        assert!(a.input == Some(Input::Add));
        assert!(a.dialog.is_none());
        assert_eq!(a.network_add_target, Some((0, "192.0.2.1".into())));
        assert!(!capture_app(&a, 100).contains("Check SSH"));
        a.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(a.pending_add.is_none());
    }
    #[test]
    fn network_auto_ssh_is_bounded_cached_and_scoped_without_redraw_work() {
        let (mut a, rx) = queued_app();
        a.view = View::Network;
        a.device = 1;
        let candidates: Vec<_> = (1..=50).map(|i| serde_json::json!({"address":format!("192.0.2.{i}"), "interface":"eth0", "ssh":{"state":"unknown"}})).collect();
        let snapshot = serde_json::json!({"observed_at":transport::now(), "candidates":candidates});
        a.apply(Reply {
            device: 0,
            generation: a.generation,
            op: Operation::NetworkCandidates,
            result: Ok(snapshot.clone()),
            preview: None,
        });
        let tasks: Vec<_> = rx.try_iter().collect();
        assert_eq!(tasks.len(), 4);
        assert!(tasks
            .iter()
            .all(|t| t.device == 0 && matches!(t.op, Operation::ProbeCandidate { .. })));
        assert_eq!(a.neighbor_queue.len(), 28);
        assert_eq!(a.neighbor_budget, 0);
        a.queue_neighbor_checks();
        a.pump_neighbor_checks();
        capture_app(&a, 120);
        assert!(rx.try_recv().is_err());
        let task = &tasks[0];
        let Operation::ProbeCandidate { address, interface } = &task.op else {
            panic!("probe");
        };
        let evidence = serde_json::json!({"address":address, "interface":interface, "observed_at":transport::now(), "ssh":{"state":"tcp_reachable"}});
        a.apply(Reply {
            device: 0,
            generation: a.generation,
            op: task.op.clone(),
            result: Ok(evidence),
            preview: None,
        });
        assert_eq!(rx.try_iter().count(), 1);
        assert_eq!(a.neighbor_probes.len(), 4);
        a.device = 2;
        a.apply(Reply {
            device: 0,
            generation: a.generation + 1,
            op: tasks[1].op.clone(),
            result: Err(anyhow::anyhow!("offline")),
            preview: None,
        });
        assert!(rx.try_recv().is_err());
        assert!(a.input.is_none());
        assert!(a.dialog.is_none());
        assert!(a.neighbor_queue.is_empty());
        a.device = 1;
        a.neighbor_budget = 32;
        a.apply(Reply {
            device: 0,
            generation: a.generation,
            op: Operation::NetworkCandidates,
            result: Ok(snapshot),
            preview: None,
        });
        assert!(a
            .network_candidates()
            .iter()
            .any(|c| c["address"] == *address && neighbor_ssh(c) == "port open"));
        assert!(!rx.try_iter().any(
            |t| matches!(t.op, Operation::ProbeCandidate {address:ref ip,..} if ip == address)
        ));
    }
    #[test]
    fn network_auto_ssh_ignores_stale_unnumeric_and_missing_interface() {
        let (mut a, rx) = queued_app();
        a.view = View::Network;
        a.apply(Reply {device:0, generation:a.generation, op:Operation::NetworkCandidates, result:Ok(serde_json::json!({"observed_at":transport::now().saturating_sub(91),"candidates":[{"address":"192.0.2.1","interface":"eth0"}]})), preview:None});
        a.apply(Reply {device:0, generation:a.generation, op:Operation::NetworkCandidates, result:Ok(serde_json::json!({"observed_at":transport::now(),"candidates":[{"address":"example.test","interface":"eth0"},{"address":"192.0.2.2"},{"address":"192.0.2.3","interface":"bad iface"}]})), preview:None});
        assert!(rx.try_recv().is_err());
        assert!(a.neighbor_probes.is_empty());
    }
    #[test]
    fn queued_neighbor_check_does_not_outlive_snapshot() {
        let (mut a, rx) = queued_app();
        a.view = View::Network;
        a.network.insert(0, serde_json::json!({"candidates_observed_at":transport::now().saturating_sub(91),"candidates":[{"address":"192.0.2.1","interface":"eth0"}]}));
        let candidate = a.network_candidates()[0].clone();
        a.neighbor_queue.push_back((0, candidate));
        a.pump_neighbor_checks();
        assert!(rx.try_recv().is_err());
        assert!(a.neighbor_queue.is_empty());
    }
    #[test]
    fn network_connect_account_input_owns_keys_and_scope() {
        let (mut a, _rx) = queued_app();
        network_fixture(&mut a);
        a.open_neighbor();
        for c in "root".chars() {
            press(&mut a, c);
        }
        assert_eq!(a.text, "root");
        assert!(a.dialog.is_none());
        a.device = 2;
        a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(a.pending_add.as_deref(), Some("root@192.0.2.1"));
        assert_eq!(a.pending_add_via.as_ref().unwrap().id, "local");
    }
    #[test]
    fn network_offline_invalidates_previous_evidence_and_scope_does_not_block() {
        let (mut a, rx) = queued_app();
        network_fixture(&mut a);
        a.refresh();
        a.device = 2;
        a.refresh();
        let tasks: Vec<_> = rx.try_iter().collect();
        assert!(tasks
            .iter()
            .any(|t| t.device == 0 && matches!(t.op, Operation::Network)));
        assert!(tasks
            .iter()
            .any(|t| t.device == 1 && matches!(t.op, Operation::Network)));
        a.apply(Reply {
            device: 0,
            generation: a.generation + 1,
            op: Operation::Network,
            result: Err(anyhow::anyhow!("offline")),
            preview: None,
        });
        assert_eq!(a.network[&0]["internet"]["state"], "unknown");
        assert_eq!(a.network[&0]["candidates"][1]["ssh"]["state"], "unknown");
        assert!(!a.network_inflight.contains(&0));
        assert!(a.network_inflight.contains(&1));
    }
    #[test]
    fn failed_cached_session_keeps_refresh_visible_at_small_widths() {
        let (mut a, _) = queued_app();
        a.view = View::Work;
        a.device = 1;
        a.focus = Focus::Workspace;
        a.work[0].sessions.push(disposable_shell());
        a.work[0].error = Some("account@execution-device: Device unreachable over SSH · check its connection, then Refresh: helper closed before response".into());
        for width in [48, 80, 120] {
            let text = capture_app(&a, width);
            assert!(text.contains("Ctrl+P → Refresh"), "{width}: {text}");
            if width >= 80 {
                assert!(text.contains("cached"), "{width}: {text}");
            }
        }
        a.apply(Reply {
            device: 0,
            op: Operation::Sessions,
            generation: a.generation,
            result: Ok(serde_json::json!([])),
            preview: None,
        });
        assert!(a.work[0].sessions.is_empty() && a.work[0].error.is_none());
    }
    #[test]
    fn shell_stop_requires_confirmation_and_rejects_external_and_agent_rows() {
        let (mut a, rx) = queued_app();
        let session: Session = serde_json::from_value(serde_json::json!({
            "id":"cx-fixture","name":"shell · fixture","directory":"/fixture","provider":"shell",
            "account":"tester","host":"workstation","pid":123,"started":"start","boot_id":"boot","external":false
        })).unwrap();
        a.work[0].sessions.push(session);
        a.view = View::Work;
        a.focus = Focus::Workspace;
        press(&mut a, 'd');
        assert!(matches!(a.dialog, Some(Dialog::StopShell(..))));
        assert!(rx.try_recv().is_err());
        press(&mut a, 'n');
        assert!(a.dialog.is_none());
        press(&mut a, 'd');
        press(&mut a, 'y');
        assert!(matches!(
            rx.try_recv().unwrap().op,
            Operation::StopSession { pid: 123, .. }
        ));
        a.work[0].sessions[0].external = true;
        press(&mut a, 'd');
        assert!(a.dialog.is_none());
        assert!(rx.try_recv().is_err());
        a.work[0].sessions[0].external = false;
        a.work[0].sessions[0].provider = "codex".into();
        press(&mut a, 'd');
        assert!(a.dialog.is_none());
        assert!(rx.try_recv().is_err());
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
        a.apply(Reply { preview:None, device: 0, op: Operation::List { path: "/files".into() }, generation,
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
            preview: None,
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
            preview: None,
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
            preview: None,
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
    fn update_menu_and_notices_identify_the_running_binary() {
        let label = ACTIONS
            .iter()
            .find(|(action, _)| *action == Action::Update)
            .unwrap()
            .1;
        assert!(label.contains(concat!("cx v", env!("CARGO_PKG_VERSION"))));
        for width in [48, 80, 120] {
            let (mut a, _rx) = file_app();
            a.input = Some(Input::Palette);
            a.text = "update".into();
            let rendered = capture_app(&a, width);
            assert!(
                rendered.contains(concat!("Update cx v", env!("CARGO_PKG_VERSION"))),
                "{rendered}"
            );
        }
        for result in [
            Ok(crate::update::CheckOutcome::Current),
            Ok(crate::update::CheckOutcome::Offline),
            Ok(crate::update::CheckOutcome::Skipped),
            Err(anyhow::anyhow!(
                "synthetic failure; no raw detail displayed"
            )),
        ] {
            let text = update_check_notice(&result);
            assert!(text.starts_with(concat!("cx v", env!("CARGO_PKG_VERSION"), " ")));
            assert!(!text.contains("synthetic failure"));
        }
        assert!(update_check_notice(&Ok(crate::update::CheckOutcome::Offline)).contains("retry"));
        assert!(
            update_check_notice(&Ok(crate::update::CheckOutcome::Current)).contains("up to date")
        );
        assert_eq!(notice_area(Rect::new(0, 0, 2, 1)).width, 0);
    }
    #[test]
    fn update_notice_capture_matrix_aligns_and_preserves_default_background() {
        for width in [48, 80, 120] {
            for search in [false, true] {
                for (state, result) in [
                    ("current", Ok(crate::update::CheckOutcome::Current)),
                    ("offline", Ok(crate::update::CheckOutcome::Offline)),
                ] {
                    let (mut a, _rx) = file_app();
                    a.view = View::Work;
                    a.browser = None;
                    a.search = if search {
                        "no-match".into()
                    } else {
                        String::new()
                    };
                    a.notice = update_check_notice(&result);
                    let mut terminal = Terminal::new(TestBackend::new(width, 24)).unwrap();
                    terminal.draw(|frame| render(frame, &a)).unwrap();
                    let buffer = terminal.backend().buffer();
                    let row = (0..width)
                        .map(|x| buffer.cell((x, 23)).unwrap().symbol())
                        .collect::<String>();
                    assert!(
                        row.starts_with(&format!("  {}", a.notice)),
                        "{width} {state}: {row}"
                    );
                    assert!(buffer.content.iter().all(|cell| cell.bg == Color::Reset));
                    if let Some(directory) = std::env::var_os("CX_UPDATE_CAPTURE_DIR") {
                        let directory = std::path::PathBuf::from(directory);
                        std::fs::create_dir_all(&directory).unwrap();
                        let text = buffer
                            .content
                            .chunks(width as usize)
                            .map(|cells| cells.iter().map(|cell| cell.symbol()).collect::<String>())
                            .collect::<Vec<_>>()
                            .join("\n");
                        std::fs::write(
                            directory.join(format!("{width}-{state}-search-{search}.txt")),
                            text,
                        )
                        .unwrap();
                    }
                }
            }
        }
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
        assert!(
            text.contains("No undo")
                && text.contains("folders include their contents.")
                && text.contains("Cancel")
        );
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
                preview: None,
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
            a.apply(Reply{preview:None,device:0,op:Operation::ListPage{path:"/files".into(),offset,limit:1},generation:0,result:Ok(serde_json::json!({"path":"/files","entries":[{"name":name,"path":format!("/files/{name}"),"kind":"file","size":1}],"next_offset":next}))});
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
            preview: None,
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
                preview: None,
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
            preview: None,
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
            preview: None,
            device: 1,
            op: Operation::Info,
            generation: 0,
            result: Ok(serde_json::json!({"capabilities":["tmux", "codex"]})),
        });
        assert_eq!(a.provider_choices(1), vec!["shell", "codex"]);
        a.apply(Reply {
            preview: None,
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
            preview: None,
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
        assert!(!a
            .palette()
            .iter()
            .any(|(action, _)| *action == Action::Command));
        press(&mut a, ':');
        assert!(a.input == Some(Input::Command));
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
        a.apply(Reply { preview:None, device: 1, op: Operation::Info, generation: a.generation,
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
            preview: None,
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
    fn empty_session_action_respects_loading_errors_search_and_focus() {
        let (mut a, _rx) = queued_app();
        a.devices.truncate(1);
        a.work.truncate(1);
        a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(matches!(a.dialog, Some(Dialog::Device(ChooseDevice::New))));
        for state in ["loading", "error", "search", "creating"] {
            let mut a = app();
            match state {
                "loading" => a.work[0].loading = true,
                "error" => a.work[0].error = Some("offline".into()),
                "search" => a.search = "missing".into(),
                "creating" => a.creating = true,
                _ => unreachable!(),
            }
            a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
            assert!(a.dialog.is_none(), "{state}");
        }
        let mut a = app();
        press(&mut a, 'a');
        assert!(a.input == Some(Input::Add));
        a.text.clear();
        press(&mut a, 'a');
        assert_eq!(a.text, "a");
        let (mut a, _rx) = queued_app();
        press(&mut a, 'a');
        assert!(a.input.is_none());
    }
    #[test]
    fn selected_file_context_follows_focused_pane_and_keeps_destination() {
        let (mut a, _rx) = file_app();
        a.other_browser = Some(Browser::new(1, "/output".into()));
        for (switch, host) in [(false, "tester"), (true, "peace")] {
            if switch {
                a.switch_pane();
            }
            let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
            terminal.draw(|f| render(f, &a)).unwrap();
            let text = terminal
                .backend()
                .buffer()
                .content
                .chunks(120)
                .map(|row| row.iter().map(|c| c.symbol()).collect::<String>())
                .collect::<Vec<_>>()
                .join("\n");
            assert!(text.contains(&format!("Host {host}@")), "{text}");
            assert!(text.contains("To peace@"), "{text}");
        }
    }
    #[test]
    fn project_headers_render_immediately_above_sessions_with_space_before_later_group() {
        let (mut a, _rx) = queued_app();
        a.device = 1;
        for (id, name, directory) in [
            ("a", "alpha-session", "/alpha"),
            ("b", "beta-session", "/beta"),
        ] {
            let mut session = disposable_shell();
            session.id = id.into();
            session.name = name.into();
            session.directory = directory.into();
            a.work[0].sessions.push(session);
        }
        a.work[0].fetched = transport::now();
        for width in [80, 120] {
            let rendered = capture_app(&a, width);
            let lines: Vec<_> = rendered.lines().collect();
            let first = lines
                .iter()
                .position(|line| line.contains("alpha-session"))
                .expect("first session");
            let second = lines
                .iter()
                .position(|line| line.contains("beta-session"))
                .expect("second session");
            assert!(lines[first - 1].contains("/alpha"), "{rendered}");
            assert!(lines[second - 1].contains("/beta"), "{rendered}");
            // Ignore the sidebar; the workspace column must be blank before its later header.
            let workspace_start = lines[second - 1][..lines[second - 1].find("/beta").unwrap()]
                .chars()
                .count();
            assert!(
                lines[second - 2]
                    .chars()
                    .skip(workspace_start)
                    .all(|c| c.is_whitespace() || matches!(c, '│' | '┃' | '|')),
                "{rendered}"
            );
        }
    }
    #[test]
    fn workspace_polish_capture_matrix() {
        for width in [48, 80, 120] {
            for height in [24, 40] {
                for state in [
                    "first-run",
                    "checking",
                    "unavailable",
                    "search",
                    "sessions",
                    "destination",
                ] {
                    let (mut a, _rx) = file_app();
                    if state != "destination" {
                        a.view = View::Work;
                        a.browser = None;
                        a.devices.truncate(1);
                        a.work.truncate(1);
                    }
                    match state {
                        "checking" => a.work[0].loading = true,
                        "unavailable" => {
                            a.work[0].error = Some("Device unreachable over SSH".into())
                        }
                        "search" => a.search = "no-match".into(),
                        "sessions" => {
                            a.work[0].sessions.push(disposable_shell());
                            a.work[0].fetched = transport::now();
                        }
                        "destination" => a.other_browser = Some(Browser::new(1, "/output".into())),
                        _ => {}
                    }
                    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
                    terminal.draw(|f| render(f, &a)).unwrap();
                    let buffer = terminal.backend().buffer();
                    let text = buffer
                        .content
                        .chunks(width as usize)
                        .map(|row| row.iter().map(|c| c.symbol()).collect::<String>())
                        .collect::<Vec<_>>()
                        .join("\n");
                    assert!(
                        buffer.content.iter().all(|c| c.bg == Color::Reset),
                        "{state} {width}x{height}: opaque background"
                    );
                    if state == "sessions" {
                        assert!(text.contains("STATE"), "{width}x{height}: {text}");
                        assert!(text.contains("live"), "{width}x{height}: {text}");
                    }
                    if state == "first-run" {
                        assert!(text.contains("New session"), "{width}x{height}: {text}");
                        assert!(text.contains("Add device"), "{width}x{height}: {text}");
                    }
                    if let Some(directory) = std::env::var_os("CX_WORKSPACE_CAPTURE_DIR") {
                        let directory = std::path::PathBuf::from(directory);
                        std::fs::create_dir_all(&directory).unwrap();
                        let file = format!("{width}x{height}-{state}");
                        std::fs::write(directory.join(format!("{file}.txt")), text).unwrap();
                        let cells = buffer.content.iter().map(|c| serde_json::json!({
                            "text": c.symbol(), "fg": format!("{:?}", c.fg),
                            "bg": format!("{:?}", c.bg), "modifier": format!("{:?}", c.modifier)
                        })).collect::<Vec<_>>();
                        std::fs::write(
                            directory.join(format!("{file}.json")),
                            serde_json::to_vec(&serde_json::json!({
                                "width":width, "height":height, "cells":cells,
                                "backend":"Ratatui TestBackend fixture; not a physical terminal"
                            }))
                            .unwrap(),
                        )
                        .unwrap();
                    }
                }
            }
        }
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
    #[test]
    fn maximum_png_preview_is_visible() {
        use base64::Engine;
        let image = image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            1280,
            960,
            image::Rgba([220, 30, 20, 255]),
        ));
        let mut png = std::io::Cursor::new(Vec::new());
        image.write_to(&mut png, image::ImageFormat::Png).unwrap();
        let value = serde_json::json!({"kind":"image", "title":"PNG",
            "image":{"width":1280,"height":960,"png":base64::engine::general_purpose::STANDARD.encode(png.into_inner())}});
        assert!(
            RichPreview::from_value(&value).raster.is_some(),
            "valid maximum-size preview disappeared"
        );
    }
    #[test]
    fn pdf_pages_coalesce_keys_and_preserve_last_page_on_failure() {
        let (mut a, rx) = file_app();
        let b = a.browser.as_mut().unwrap();
        b.preview_path = Some("/files/book.pdf".into());
        b.preview = Some("".into());
        b.preview_rich = Some(RichPreview::from_value(
            &serde_json::json!({"kind":"pdf","page":1,"pages":3,"image":{"width":1,"height":1,"rgba":"/////w=="}}),
        ));
        a.key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE));
        assert!(matches!(
            rx.try_recv().unwrap().op,
            Operation::PreviewPage { page: 2, .. }
        ));
        press(&mut a, 'j');
        press(&mut a, 'j');
        assert!(
            rx.try_recv().is_err(),
            "only one page render may be in flight"
        );
        assert_eq!(a.browser.as_ref().unwrap().preview_requested_page, 3);
        a.apply(Reply {device:0,generation:a.generation,op:Operation::PreviewPage {path:"/files/book.pdf".into(),page:2},preview:None,result:Ok(serde_json::json!({"kind":"pdf","page":2,"pages":3,"text":"","image":{"width":1,"height":1,"rgba":"/////w=="}}))});
        assert!(matches!(
            rx.try_recv().unwrap().op,
            Operation::PreviewPage { page: 3, .. }
        ));
        assert_eq!(
            a.browser
                .as_ref()
                .unwrap()
                .preview_rich
                .as_ref()
                .unwrap()
                .page,
            1
        );
        a.apply(Reply {
            device: 0,
            generation: a.generation,
            op: Operation::PreviewPage {
                path: "/files/book.pdf".into(),
                page: 3,
            },
            preview: None,
            result: Err(anyhow::anyhow!("renderer unavailable")),
        });
        assert_eq!(
            a.browser
                .as_ref()
                .unwrap()
                .preview_rich
                .as_ref()
                .unwrap()
                .page,
            1
        );
        assert!(a.browser.as_ref().unwrap().preview_pending_page.is_none());
        let generation = a.generation;
        a.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        a.apply(Reply {
            device: 0,
            generation,
            op: Operation::PreviewPage {
                path: "/files/book.pdf".into(),
                page: 3,
            },
            preview: None,
            result: Ok(serde_json::json!({"kind":"pdf","page":3,"pages":3})),
        });
        assert!(a.browser.as_ref().unwrap().preview.is_none());
    }
    #[test]
    fn pdf_mouse_wheel_is_scoped_to_preview() {
        let (mut a, rx) = file_app();
        let b = a.browser.as_mut().unwrap();
        b.preview = Some("".into());
        b.preview_rich = Some(RichPreview::from_value(
            &serde_json::json!({"kind":"pdf","page":1,"pages":3,"image":{"width":1,"height":1,"rgba":"/////w=="}}),
        ));
        let mut event = MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 1,
            row: 3,
            modifiers: KeyModifiers::NONE,
        };
        assert!(!a.preview_mouse(event, Rect::new(0, 0, 80, 24)));
        assert!(rx.try_recv().is_err());
        event.column = 40;
        event.row = 10;
        assert!(a.preview_mouse(event, Rect::new(0, 0, 80, 24)));
        assert!(matches!(
            rx.try_recv().unwrap().op,
            Operation::PreviewPage { page: 2, .. }
        ));
        a.help = true;
        assert!(!a.preview_mouse(event, Rect::new(0, 0, 80, 24)));
    }
    #[test]
    fn host_update_reloads_metadata_and_preview_without_replaying_work() {
        let (mut a, rx) = file_app();
        let mut b = Browser::new(1, "/remote".into());
        b.preview = Some("".into());
        b.preview_path = Some("/remote/book.pdf".into());
        b.preview_requested_page = 2;
        b.preview_rich = Some(RichPreview::from_value(
            &serde_json::json!({"kind":"pdf","page":2,"pages":3}),
        ));
        a.browser = Some(b);
        a.host_updated(transport::HostUpdate {
            target: "laptop".into(),
            version: "0.1.10".into(),
        });
        let tasks = rx.try_iter().collect::<Vec<_>>();
        assert!(tasks.iter().all(|t| t.device == 1));
        assert!(tasks.iter().any(|t| matches!(t.op, Operation::Info)));
        assert!(tasks
            .iter()
            .any(|t| matches!(t.op, Operation::PreviewPage { page: 2, .. })));
        assert!(tasks.iter().all(|t| matches!(
            t.op,
            Operation::Info
                | Operation::Sessions
                | Operation::TransferJobs
                | Operation::PreviewPage { .. }
        )));
        assert_eq!(a.browser.as_ref().unwrap().path, "/remote");
        assert!(a.pending_attach.is_none() && !a.creating);
    }
    #[test]
    fn old_helper_page_check_does_not_loop() {
        let (mut a, rx) = file_app();
        a.browser = Some(Browser::new(1, "/remote".into()));
        let b = a.browser.as_mut().unwrap();
        b.preview = Some("".into());
        b.preview_requested_page = 2;
        b.preview_rich = Some(RichPreview::from_value(
            &serde_json::json!({"kind":"pdf","page":1}),
        ));
        a.apply(Reply {
            device: 1,
            generation: a.generation,
            op: Operation::Info,
            preview: None,
            result: Ok(serde_json::json!({"version":"0.1.9","capabilities":["codex"]})),
        });
        assert!(
            rx.try_recv().is_err(),
            "unsupported page capability must not create an Info retry loop"
        );
    }
    #[test]
    fn pdf_scroll_requests_the_next_page() {
        let (mut a, rx) = file_app();
        let b = a.browser.as_mut().unwrap();
        b.preview = Some("PDF".into());
        b.preview_rich = Some(RichPreview::from_value(
            &serde_json::json!({"kind":"pdf","title":"PDF · page 1","page":1,"pages":3}),
        ));
        a.key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));
        assert!(
            rx.try_iter()
                .any(|t| matches!(t.op, Operation::PreviewPage { page: 2, .. })),
            "PDF scrolling never requests another page"
        );
    }
    #[test]
    fn rich_preview_rejects_bad_images_and_keeps_transparent_pixels_default() {
        use base64::Engine;
        let valid = serde_json::json!({"kind":"image","title":"PNG 2x2", "image":{"width":2,"height":2,"rgba":base64::engine::general_purpose::STANDARD.encode([255u8,0,0,255,0,0,0,0,0,255,0,255,0,0,255,255])}});
        let rich = RichPreview::from_value(&valid);
        let (w, h, bytes) = rich.raster.unwrap();
        let lines = raster_lines(w, h, &bytes, Rect::new(0, 0, 2, 1));
        assert_eq!(lines[0].spans[0].style.fg, Some(Color::Rgb(255, 0, 0)));
        assert_eq!(lines[0].spans[0].style.bg, Some(Color::Rgb(0, 255, 0)));
        assert_eq!(lines[0].spans[1].content, "▄");
        assert_eq!(lines[0].spans[1].style.bg, None);
        for image in [
            serde_json::json!({"width":0,"height":1,"rgba":""}),
            serde_json::json!({"width":161,"height":1,"rgba":""}),
            serde_json::json!({"width":1,"height":1,"rgba":"bad"}),
            serde_json::json!({"width":1,"height":1,"rgba":"AA=="}),
        ] {
            assert!(RichPreview::from_value(&serde_json::json!({"image":image}))
                .raster
                .is_none());
        }
        assert!(raster_lines(w, h, &bytes, Rect::new(0, 0, 0, 0)).is_empty());
    }
    #[test]
    fn markdown_preview_styles_without_controls_or_active_content() {
        let lines=preview_lines("# Heading\n- **strong** and `code`\n```rust\nfn main() {}\n```\n<img src='https://example.test'>\n\x1b[31m", "markdown");
        assert!(lines[0].spans[0]
            .style
            .add_modifier
            .contains(Modifier::BOLD));
        assert!(lines[1]
            .spans
            .iter()
            .any(|s| s.content == "strong" && s.style.add_modifier.contains(Modifier::BOLD)));
        let rendered = lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .map(|s| s.content.as_ref())
            .collect::<String>();
        assert!(!rendered.contains('\x1b'));
        assert!(rendered.contains("https://example.test"));
    }
    #[test]
    fn preview_reply_is_scoped_and_content_is_not_serialized() {
        let (mut a, _rx) = file_app();
        let reply = || serde_json::json!({"text":"# Safe","kind":"markdown","title":"Heading"});
        a.apply(Reply {
            preview: None,
            device: 1,
            op: Operation::Preview {
                path: "/remote".into(),
            },
            generation: a.generation,
            result: Ok(reply()),
        });
        assert!(a.browser.as_ref().unwrap().preview.is_none());
        a.apply(Reply {
            preview: None,
            device: 0,
            op: Operation::Preview {
                path: "/files/a".into(),
            },
            generation: a.generation,
            result: Ok(reply()),
        });
        assert!(a.browser.as_ref().unwrap().preview_rich.is_some());
        let encoded = serde_json::to_string(a.browser.as_ref().unwrap()).unwrap();
        assert!(!encoded.contains("Heading") && !encoded.contains("# Safe"));
        a.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(a.browser.as_ref().unwrap().preview_rich.is_none());
        a.apply(Reply {
            preview: None,
            device: 0,
            op: Operation::Preview {
                path: "/files/a".into(),
            },
            generation: a.generation - 1,
            result: Ok(reply()),
        });
        assert!(a.browser.as_ref().unwrap().preview.is_none());
    }
    fn disposable_shell() -> Session {
        serde_json::from_value(serde_json::json!({"id":"cx-disposable", "name":"Disposable shell", "directory":"/tmp/cx-disposable", "provider":"shell", "host":"workstation", "account":"tester", "pid":1234,"started":"100", "boot_id":"test-boot", "external":false,"socket":"cx"})).unwrap()
    }
    #[test]
    fn stop_shell_confirmation_protects_agents_and_captures_runtime_identity() {
        let (mut a, rx) = queued_app();
        a.work[0].sessions = vec![disposable_shell()];
        a.providers
            .insert(0, (vec!["stop-session-v1".into()], transport::now()));
        press(&mut a, 'd');
        assert!(matches!(a.dialog, Some(Dialog::StopShell(..))) && a.dialog_selected == 0);
        a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(rx.try_recv().is_err());
        press(&mut a, 'd');
        a.work[0].sessions[0].pid = 9999;
        press(&mut a, 'y');
        assert!(
            matches!(rx.try_recv().unwrap().op,Operation::StopSession{id,pid:1234,started,boot_id} if id=="cx-disposable" && started=="100" && boot_id=="test-boot")
        );
        assert!(a.pending_attach.is_none());
        a.work[0].sessions[0].provider = "codex".into();
        press(&mut a, 'd');
        assert!(a.dialog.is_none());
        a.work[0].sessions[0].provider = "shell".into();
        a.work[0].sessions[0].external = true;
        press(&mut a, 'd');
        assert!(a.dialog.is_none());
    }
    #[test]
    fn preview_and_confirmation_capture_sizes() {
        use base64::Engine;
        for (width, height) in [(120, 40), (80, 24), (48, 24)] {
            let (mut a, _rx) = file_app();
            a.browser.as_mut().unwrap().preview =
                Some("# Recording\n- **Robot** capture\n```rust\nfn main() {}\n```".into());
            a.browser.as_mut().unwrap().preview_rich = Some(RichPreview::from_value(
                &serde_json::json!({"kind":"markdown","title":"README.md"}),
            ));
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            let write = |terminal: &Terminal<TestBackend>, name: &str| {
                if let Some(directory) = std::env::var_os("CX_PREVIEW_CAPTURE_DIR") {
                    let directory = std::path::PathBuf::from(directory);
                    std::fs::create_dir_all(&directory).unwrap();
                    let buffer = terminal.backend().buffer();
                    let text = buffer
                        .content
                        .chunks(usize::from(width))
                        .map(|row| row.iter().map(|c| c.symbol()).collect::<String>())
                        .collect::<Vec<_>>()
                        .join("\n");
                    std::fs::write(directory.join(format!("{width}x{height}-{name}.txt")), text)
                        .unwrap();
                    let cells=buffer.content.iter().map(|cell|serde_json::json!({"text":cell.symbol(),"fg":format!("{:?}",cell.fg),"bg":format!("{:?}",cell.bg),"modifier":format!("{:?}",cell.modifier)})).collect::<Vec<_>>();
                    std::fs::write(directory.join(format!("{width}x{height}-{name}.json")),serde_json::to_vec(&serde_json::json!({"width":width,"height":height,"cells":cells,"backend":"Ratatui TestBackend fixture; not physical emulator"})).unwrap()).unwrap();
                }
            };
            terminal.draw(|f| render(f, &a)).unwrap();
            write(&terminal, "markdown");
            let rgba = (0..32 * 20)
                .flat_map(|i| {
                    [
                        if i % 32 < 16 { 255 } else { 0 },
                        if i / 32 < 10 { 180 } else { 0 },
                        180,
                        if i % 5 == 0 { 0 } else { 255 },
                    ]
                })
                .collect::<Vec<u8>>();
            a.browser.as_mut().unwrap().preview_rich = Some(RichPreview::from_value(
                &serde_json::json!({"kind":"image","title":"PNG 32x20","image":{"width":32,"height":20,"rgba":base64::engine::general_purpose::STANDARD.encode(rgba)}}),
            ));
            terminal.draw(|f| render(f, &a)).unwrap();
            write(&terminal, "image");
            a.browser.as_mut().unwrap().preview = None;
            a.dialog = Some(Dialog::Delete(
                0,
                vec![a.browser.as_ref().unwrap().entries[0].clone()],
            ));
            terminal.draw(|f| render(f, &a)).unwrap();
            write(&terminal, "delete");
            a.dialog = Some(Dialog::StopShell(0, disposable_shell()));
            terminal.draw(|f| render(f, &a)).unwrap();
            write(&terminal, "stop-shell");
        }
    }
    #[test]
    fn boxed_confirmation_variant_captures() {
        for (width, height) in [(120, 40), (80, 24), (48, 24), (36, 24)] {
            for (variant, name) in [
                (ConfirmationLayout::Separate, "a-separate"),
                (ConfirmationLayout::Joined, "b-joined"),
                (ConfirmationLayout::Compact, "c-compact"),
            ] {
                for stop in [false, true] {
                    for selected in [0, 1] {
                        let (mut a, _rx) = file_app();
                        a.dialog = Some(if stop {
                            Dialog::StopShell(0, disposable_shell())
                        } else {
                            Dialog::Delete(0, vec![a.browser.as_ref().unwrap().entries[0].clone()])
                        });
                        a.dialog_selected = selected;
                        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
                        terminal
                            .draw(|frame| {
                                render(frame, &a);
                                let detail = if stop {"Running commands in this shell will end.\ntester@workstation\nDisposable shell\n/tmp/cx-disposable"} else {"Deletion cannot be undone.\ntester@workstation\n• alpha.txt"};
                            let rect = confirmation_popup(frame.area(), detail);
                                let inner = Rect::new(
                                    rect.x + 2,
                                    rect.y + 1,
                                    rect.width.saturating_sub(4),
                                    rect.height.saturating_sub(2),
                                );
                                let rows = Layout::default()
                                    .direction(Direction::Vertical)
                                    .constraints([
                                        Constraint::Min(2),
                                        Constraint::Length(1),
                                        Constraint::Length(3),
                                        Constraint::Length(1),
                                    ])
                                    .split(inner);
                                confirmation_buttons(
                                    frame, rows[2], stop, selected, false, variant,
                                );
                            })
                            .unwrap();
                        let buffer = terminal.backend().buffer();
                        let text = buffer
                            .content
                            .chunks(usize::from(width))
                            .map(|row| row.iter().map(|c| c.symbol()).collect::<String>())
                            .collect::<Vec<_>>()
                            .join("\n");
                        assert!(text.contains(if stop {
                            "Running commands"
                        } else {
                            "Deletion cannot be"
                        }));
                        assert!(text.contains("tester@workstation"));
                        assert!(
                            text.contains(if stop {
                                if width < 48 {
                                    "Keep"
                                } else {
                                    "Keep shell"
                                }
                            } else {
                                "Cancel"
                            }) && text.contains(if stop { "Stop shell" } else { "Delete" })
                        );
                        if let Some(directory) = std::env::var_os("CX_CONFIRMATION_CAPTURE_DIR") {
                            let directory = std::path::PathBuf::from(directory);
                            std::fs::create_dir_all(&directory).unwrap();
                            let file = format!(
                                "{width}x{height}-{name}-{}-{}",
                                if stop { "stop-shell" } else { "delete" },
                                if selected == 0 { "cancel" } else { "confirm" }
                            );
                            std::fs::write(directory.join(format!("{file}.txt")), text).unwrap();
                            let cells=buffer.content.iter().map(|cell|serde_json::json!({"text":cell.symbol(),"fg":format!("{:?}",cell.fg),"bg":format!("{:?}",cell.bg),"modifier":format!("{:?}",cell.modifier)})).collect::<Vec<_>>();
                            std::fs::write(directory.join(format!("{file}.json")),serde_json::to_vec(&serde_json::json!({"width":width,"height":height,"cells":cells,"backend":"TestBackend fixture; not physical emulator"})).unwrap()).unwrap();
                        }
                    }
                }
            }
        }
    }
    #[test]
    fn larger_image_preview_uses_workspace_without_changing_locations() {
        use base64::Engine;
        let (mut a, _rx) = file_app();
        a.other_browser = Some(Browser::new(1, "/other-device".into()));
        let browser = a.browser.as_mut().unwrap();
        browser.selected = 1;
        browser.preview = Some("Image preview".into());
        browser.preview_rich = Some(RichPreview::from_value(
            &serde_json::json!({"kind":"image","title":"Photo","image":{"width":2,"height":2,"rgba":base64::engine::general_purpose::STANDARD.encode([255u8;16])}}),
        ));
        let text = capture_app(&a, 80);
        assert!(
            text.contains("tester@workstation")
                && text.contains("Photo")
                && !text.contains("NORMAL")
        );
        assert_eq!(a.browser.as_ref().unwrap().selected, 1);
        assert_eq!(a.other_browser.as_ref().unwrap().path, "/other-device");
        a.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(a.browser.as_ref().unwrap().preview.is_none());
        assert_eq!(a.browser.as_ref().unwrap().selected, 1);
        assert_eq!(a.other_browser.as_ref().unwrap().device, 1);
        let lines = raster_lines(2, 2, &[255u8; 16], Rect::new(0, 0, 20, 10));
        assert_eq!(lines.len(), 10);
        assert_eq!(lines[0].spans.len(), 20);
    }
    #[test]
    fn native_bitmap_geometry_tracks_workspace_and_suppresses_overlays() {
        use base64::Engine;
        let (mut a, _rx) = file_app();
        let browser = a.browser.as_mut().unwrap();
        browser.preview = Some("Image".into());
        browser.preview_rich = Some(RichPreview::from_value(
            &serde_json::json!({"kind":"image","title":"PNG","image":{"width":2,"height":2,"rgba":base64::engine::general_purpose::STANDARD.encode([255u8;16])}}),
        ));
        let screen = Rect::new(0, 0, 80, 24);
        assert_eq!(
            native_preview_area(&a, screen),
            Some(Rect::new(22, 4, 57, 15))
        );
        a.transfer_drawer = true;
        assert_eq!(
            native_preview_area(&a, screen),
            Some(Rect::new(22, 4, 57, 11))
        );
        a.browser.as_mut().unwrap().search = "photo".into();
        assert_eq!(
            native_preview_area(&a, screen),
            Some(Rect::new(22, 4, 57, 8))
        );
        a.transfer_drawer = false;
        a.browser.as_mut().unwrap().search.clear();
        assert_eq!(
            native_preview_area(&a, Rect::new(0, 0, 48, 24)),
            Some(Rect::new(15, 4, 32, 15))
        );
        a.help = true;
        assert!(native_preview_area(&a, screen).is_none());
        a.help = false;
        for input in [Input::Palette, Input::Search, Input::Filter, Input::Rename] {
            a.input = Some(input);
            assert!(native_preview_area(&a, screen).is_none());
        }
        a.input = None;
        a.dialog = Some(Dialog::Jobs);
        assert!(native_preview_area(&a, screen).is_none());
        a.dialog = None;
        a.view = View::Work;
        assert!(native_preview_area(&a, screen).is_none());
        a.view = View::Files;
        assert!(native_preview_area(&a, Rect::new(0, 0, 35, 24)).is_none());
        assert!(native_preview_area(&a, Rect::new(0, 0, 80, 9)).is_none());
    }
    #[test]
    fn accepted_preview_responses_change_native_identity_even_when_metadata_matches() {
        let (mut a, _rx) = file_app();
        for revision in [1, 2] {
            a.apply(Reply {
                device: 0,
                op: Operation::Preview {
                    path: "/files/alpha.txt".into(),
                },
                generation: a.generation,
                preview: None,
                result: Ok(serde_json::json!({"kind":"text","title":"same","text":"same"})),
            });
            assert_eq!(a.browser.as_ref().unwrap().preview_revision, revision);
        }
        let encoded = serde_json::to_string(a.browser.as_ref().unwrap()).unwrap();
        assert!(!encoded.contains("preview_revision"));
        let before = a.browser.as_ref().unwrap().preview_revision;
        a.apply(Reply {
            device: 1,
            op: Operation::Preview {
                path: "/remote".into(),
            },
            generation: a.generation,
            preview: None,
            result: Ok(serde_json::json!({"text":"stale"})),
        });
        assert_eq!(a.browser.as_ref().unwrap().preview_revision, before);
    }
}
