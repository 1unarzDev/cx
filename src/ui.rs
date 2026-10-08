//! Native workspace. Remote work runs off the input/render thread; attachment owns the terminal.
use crate::{
    model::{ContainerScope, CreateSession, Device, Operation, RunCommand, Session},
    sessions, store, transport,
};
mod filters;
mod rendering;
#[cfg(test)]
use rendering::render;
use rendering::render_with_native;
mod menus;
mod navigation;
mod notifications;
mod panels;
use filters::score as fuzzy_score;
use menus::{container_action_labels, render_session_chooser};
use navigation::{Motion as Navigation, Target};
use notifications::Kind as NoticeKind;

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
    collections::{BTreeMap, BTreeSet, HashMap, VecDeque},
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
    Containers,
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
    PreviewSearch,
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
    #[serde(default)]
    container: Option<ContainerScope>,
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

#[derive(Clone, Default)]
struct PreviewFind {
    query: String,
    matches: Vec<(usize, Vec<usize>)>,
    selected: usize,
    reveal: std::cell::Cell<bool>,
}
#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct Browser {
    #[serde(default)]
    container: Option<ContainerScope>,
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
    markdown_cache: std::cell::RefCell<Option<(u64, u16, String, Vec<Line<'static>>)>>,
    #[serde(skip)]
    preview_path: Option<String>,
    #[serde(skip)]
    preview_requested_page: u32,
    #[serde(skip)]
    preview_pending_page: Option<u32>,
    preview_scroll: std::cell::Cell<u16>,
    #[serde(skip)]
    preview_viewport: std::cell::Cell<(u16, u16)>,
    #[serde(skip)]
    preview_link_cells: std::cell::RefCell<Vec<(u16, u16, Color)>>,
    #[serde(skip)]
    markdown_hit_lines: std::cell::RefCell<Vec<Line<'static>>>,
    #[serde(skip)]
    markdown_anchor_rows: std::cell::RefCell<BTreeMap<String, usize>>,
    #[serde(skip)]
    preview_find: PreviewFind,
    #[serde(skip)]
    preview_history: Vec<(String, u16, PreviewFind)>,
    #[serde(skip)]
    preview_restore: Option<(u16, PreviewFind)>,
    #[serde(skip)]
    preview_anchor: Option<String>,
    #[serde(default)]
    restore_selection: Option<String>,
}
impl Browser {
    fn new(device: usize, path: String) -> Self {
        Self {
            container: None,
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
            markdown_cache: std::cell::RefCell::new(None),
            preview_path: None,
            preview_requested_page: 1,
            preview_pending_page: None,
            preview_scroll: std::cell::Cell::new(0),
            preview_viewport: std::cell::Cell::new((0, 0)),
            preview_link_cells: std::cell::RefCell::new(Vec::new()),
            markdown_hit_lines: std::cell::RefCell::new(Vec::new()),
            markdown_anchor_rows: std::cell::RefCell::new(BTreeMap::new()),
            preview_find: PreviewFind::default(),
            preview_history: Vec::new(),
            preview_restore: None,
            preview_anchor: None,
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
    DevcontainerUp,
    Containers,
    Add,
    Terminal,
    Files,
    New,
    Shell,
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
    (
        Action::DevcontainerUp,
        "Start devcontainer workspace here · Files folder",
    ),
    (
        Action::Containers,
        "Containers · choose device / All devices",
    ),
    (Action::Add, "Add by SSH address · alias / user@host"),
    (Action::Files, "Files · choose device · browse home"),
    (Action::Shell, "New shell · choose device · start in home"),
    (
        Action::New,
        "New session · choose device, provider and folder",
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
    (Action::Network, "Network · choose device / All devices"),
    (Action::Work, "Sessions · choose device / All devices"),
    (Action::Refresh, "Refresh"),
    (Action::Help, "Keyboard help"),
    (
        Action::Update,
        concat!("Update cx v", env!("CARGO_PKG_VERSION"), ": check releases"),
    ),
    (Action::Quit, "Quit workspace"),
];
#[derive(Clone, Copy, PartialEq, Eq)]
enum ChooseDevice {
    SessionChooser,
    Containers,
    Work,
    Network,
    AddGateway,
    Shell,
    Terminal,
    New,
    Files,
    Destination,
}
#[derive(Clone)]
enum Dialog {
    SessionChooser(usize, String),
    DevcontainerUp(usize, String),
    ContainerActions(usize, crate::containers::Container),
    ContainerProvider(usize, ContainerScope),
    DestinationScope(usize, Vec<crate::containers::Container>),
    ContainerConfirm(usize, crate::containers::Container, String),
    Device(ChooseDevice),
    Provider(usize, Option<String>),
    Permissions(usize, String, String),
    Matching(usize, String, String, Session),
    Jobs,
    Links(Vec<crate::markdown_links::Link>),
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
    navigation: navigation::State,
    device_filter: String,
    job_filter: String,
    help: bool,
    help_scroll: u16,
    notice: String,
    notice_kind: NoticeKind,
    notice_started: Option<Instant>,
    notice_deadline: Option<Instant>,
    unavailable_notified: BTreeSet<String>,
    browser: Option<Browser>,
    other_browser: Option<Browser>,
    destination_active: bool,
    conflict: usize,
    dialog: Option<Dialog>,
    dialog_selected: usize,
    dialog_detail_focus: bool,
    dialog_scroll: u16,
    dialog_scroll_max: std::cell::Cell<u16>,
    launch_provider: Option<String>,
    submitted: HashMap<String, crate::model::TransferSpec>,
    submitted_clipboards: HashMap<String, String>,
    pending_transfers: BTreeSet<String>,
    transfer_frame: usize,
    watched_jobs: BTreeSet<String>,
    browser_cache: HashMap<(usize, String, String), Browser>,
    container_locations: HashMap<(usize, String), String>,
    containers: HashMap<usize, Vec<crate::containers::Container>>,
    container_errors: HashMap<usize, String>,
    containers_loading: BTreeSet<usize>,
    rebuilding: BTreeSet<(usize, String)>,
    container_selected: usize,
    container_collapsed: BTreeSet<usize>,
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
    network_expanded: BTreeSet<String>,
    network_detail_scroll: u16,
    network_jump_routes: HashMap<usize, Option<Vec<String>>>,
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
    pending_terminal: Option<Device>,
    pending_add: Option<String>,
    quit: bool,
    force_update: bool,
    pending_requests: std::cell::Cell<usize>,
    panels: std::cell::RefCell<Vec<(Focus, bool, Rect)>>,
    tx: mpsc::SyncSender<Task>,
}
impl App {
    fn set_notice(&mut self, message: String) {
        self.set_notice_as(NoticeKind::Info, message);
    }
    fn set_notice_as(&mut self, kind: NoticeKind, message: String) {
        self.notice_kind = kind;
        self.notice_started = (!message.is_empty()).then(Instant::now);
        self.notice_deadline = self
            .notice_started
            .map(|start| start + Duration::from_secs(5));
        self.notice = message;
    }
    fn expire_notice(&mut self, now: Instant) -> bool {
        if self.notice_deadline.is_some_and(|deadline| now >= deadline) {
            self.notice.clear();
            self.notice_deadline = None;
            self.notice_started = None;
            true
        } else {
            false
        }
    }

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
            device_filter: String::new(),
            job_filter: String::new(),
            navigation: navigation::State::default(),
            help: false,
            help_scroll: 0,
            notice: String::new(),
            notice_kind: NoticeKind::Info,
            notice_started: None,
            notice_deadline: None,
            unavailable_notified: BTreeSet::new(),
            browser: None,
            other_browser: None,
            destination_active: false,
            conflict: 2,
            dialog: None,
            dialog_selected: 0,
            dialog_detail_focus: false,
            dialog_scroll: 0,
            dialog_scroll_max: std::cell::Cell::new(4096),
            launch_provider: None,
            submitted: HashMap::new(),
            submitted_clipboards: HashMap::new(),
            pending_transfers: BTreeSet::new(),
            transfer_frame: 0,
            watched_jobs: BTreeSet::new(),
            browser_cache: HashMap::new(),
            container_locations: HashMap::new(),
            containers: HashMap::new(),
            container_errors: HashMap::new(),
            containers_loading: BTreeSet::new(),
            rebuilding: BTreeSet::new(),
            container_selected: 0,
            container_collapsed: BTreeSet::new(),
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
            network_expanded: BTreeSet::new(),
            network_detail_scroll: 0,
            network_jump_routes: HashMap::new(),
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
            pending_terminal: None,
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
                Dialog::DevcontainerUp(..) => "Start devcontainer workspace",
                Dialog::ContainerActions(..) => "Container actions",
                Dialog::ContainerProvider(..) => "Container session",
                Dialog::DestinationScope(..) => "Transfer destination",
                Dialog::ContainerConfirm(..) => "Container confirmation",
                Dialog::Links(_) => "Links",
                Dialog::Device(_) => "Device picker",
                Dialog::SessionChooser(..) => "New session",
                Dialog::Provider(..) => "Provider",
                Dialog::Permissions(..) => "Session permissions",
                Dialog::Matching(..) => "Session choice",
                Dialog::Jobs => {
                    if self.dialog_detail_focus {
                        "Transfer details"
                    } else {
                        "Transfers"
                    }
                }
                Dialog::Delete(..) => "Delete",
                Dialog::StopShell(..) => "Stop session",
                Dialog::PendingExit(_) => "Pending actions",
                Dialog::Neighbor(..) => "Neighbor actions",
                Dialog::Peer(_) => "Device actions",
            };
        }
        if let Some(input) = self.input {
            return match input {
                Input::Search | Input::PreviewSearch => "Search",
                Input::Palette => "Actions",
                Input::Command => "Run command",
                Input::Add => "Add by SSH address",
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
                View::Containers => "Containers",
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
    fn scoped_file_operation(&self, device: usize, op: Operation) -> Operation {
        if let Some(scope) = self
            .browser
            .as_ref()
            .filter(|b| self.view == View::Files && b.device == device)
            .and_then(|b| b.container.as_ref())
        {
            if matches!(
                op,
                Operation::List { .. }
                    | Operation::ListPage { .. }
                    | Operation::Preview { .. }
                    | Operation::PreviewPage { .. }
                    | Operation::FileInfo { .. }
            ) {
                return Operation::ContainerFiles {
                    scope: scope.clone(),
                    operation: Box::new(op),
                };
            }
            if matches!(
                op,
                Operation::Mkdir { .. }
                    | Operation::Remove { .. }
                    | Operation::Rename { .. }
                    | Operation::Move { .. }
                    | Operation::SetPermissions { .. }
            ) {
                return Operation::ContainerFileAction {
                    scope: scope.clone(),
                    operation: Box::new(op),
                };
            }
        }
        op
    }
    fn send(&self, device: usize, op: Operation) -> bool {
        self.send_explicit(device, self.scoped_file_operation(device, op))
    }
    fn send_explicit(&self, device: usize, op: Operation) -> bool {
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
                    self.set_notice_as(
                        NoticeKind::Warning,
                        "Refresh queue busy · retry shortly".into(),
                    );
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
                        "{} {} {} {} {} {}",
                        s.name,
                        s.directory,
                        s.host,
                        s.account,
                        s.provider,
                        menus::session_label(self, i, s)
                    ),
                )
                .map(|score| (score, i, s))
            })
            .collect();
        rows.sort_by(|(a, i, x), (b, j, y)| {
            b.cmp(a).then_with(|| {
                (x.directory.as_str(), *i, x.id.as_str()).cmp(&(
                    y.directory.as_str(),
                    *j,
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
        let actions = actions
            .into_iter()
            .map(|(d, op)| (d, self.scoped_file_operation(d, op)))
            .collect::<Vec<_>>();
        self.file_queue.extend(actions);
        self.start_next_file_action();
    }
    fn start_next_file_action(&mut self) {
        if !self.file_busy {
            if let Some((d, op)) = self.file_queue.front().cloned() {
                if self.send_explicit(d, op) {
                    self.file_queue.pop_front();
                    self.file_busy = true;
                } else {
                    self.set_notice_as(
                        NoticeKind::Warning,
                        "Request queue busy · file action remains pending".into(),
                    );
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
                let rows = browser_entries(b);
                if !b.search.is_empty() {
                    let matches = file_search_matches(&rows, &b.search);
                    if let Some(index) = matches
                        .iter()
                        .map(|(i, _)| *i)
                        .find(|i| *i >= b.selected)
                        .or_else(|| matches.first().map(|(i, _)| *i))
                    {
                        b.selected = index;
                    }
                }
            }
        } else {
            self.search = self.text.clone();
            self.selected = 0;
            self.container_selected = 0;
        }
    }
    fn next_file_match(&mut self, direction: isize) {
        if let Some(b) = &mut self.browser {
            let matches = file_search_matches(&browser_entries(b), &b.search);
            let index = if direction > 0 {
                matches
                    .iter()
                    .find(|(i, _)| *i > b.selected)
                    .or_else(|| matches.first())
            } else {
                matches
                    .iter()
                    .rev()
                    .find(|(i, _)| *i < b.selected)
                    .or_else(|| matches.last())
            };
            if let Some((index, _)) = index {
                b.selected = *index;
                b.restore_selection = None;
            }
        }
    }
    fn command_context(&self) -> Option<(usize, String)> {
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
                if session.container.is_some() {
                    return None;
                }
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
                (matches!(
                    a,
                    Action::Work | Action::Network | Action::Files | Action::Terminal | Action::Add
                ) || self.action_enabled(*a))
                    && label.to_lowercase().contains(&self.text.to_lowercase())
            })
            .collect()
    }
    fn palette_scope(&self) -> String {
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
    fn provider_choices(&self, device: usize) -> Vec<&'static str> {
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
    fn container_read_only_action(&self, action: Action) -> bool {
        self.view == View::Files
            && self.browser.as_ref().is_some_and(|b| b.container.is_some())
            && action == Action::Command
    }
    fn action_enabled(&self, action: Action) -> bool {
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
            Action::Observe => self.view == View::Work && self.selected_session().is_some(),
            _ => true,
        }
    }
    fn open_browser(&mut self, device: usize, path: String) {
        let scope = self
            .browser
            .as_ref()
            .filter(|b| self.view == View::Files && b.device == device)
            .and_then(|b| b.container.clone());
        self.open_scoped_browser(device, path, scope);
    }
    fn open_host_browser(&mut self, device: usize, path: String) {
        self.open_scoped_browser(device, path, None);
    }
    fn open_container_browser(&mut self, device: usize, scope: ContainerScope) {
        self.other_browser = None;
        self.destination_active = false;
        let path = self
            .container_locations
            .get(&(device, scope.id.clone()))
            .cloned()
            .unwrap_or_else(|| scope.folder.clone());
        self.open_scoped_browser(device, path, Some(scope));
    }
    fn open_scoped_browser(&mut self, device: usize, path: String, scope: Option<ContainerScope>) {
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
            old.preview_history.clear();
            old.preview_restore = None;
            old.preview_anchor = None;
            old.preview_link_cells.borrow_mut().clear();
            if let Some(scope) = &old.container {
                self.container_locations
                    .insert((old.device, scope.id.clone()), old.path.clone());
            } else {
                self.file_locations.insert(old.device, old.path.clone());
            }
            if self.browser_cache.len() >= 8 {
                if let Some(key) = self.browser_cache.keys().next().cloned() {
                    self.browser_cache.remove(&key);
                }
            }
            self.browser_cache.insert(
                (
                    old.device,
                    old.path.clone(),
                    browser_scope_key(old.container.as_ref()),
                ),
                old,
            );
        }
        self.browser = Some(
            self.browser_cache
                .remove(&(device, path.clone(), browser_scope_key(scope.as_ref())))
                .unwrap_or_else(|| Browser::new(device, path)),
        );
        if let Some(b) = &mut self.browser {
            b.show_hidden = show_hidden;
            b.container = scope;
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
            if self
                .browser
                .as_ref()
                .is_none_or(|b| b.device != device || b.container.is_some())
            {
                let path = self
                    .file_locations
                    .get(&device)
                    .cloned()
                    .unwrap_or_else(|| "~".into());
                self.open_host_browser(device, path);
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
            b.preview_history.clear();
            b.preview_restore = None;
            b.preview_anchor = None;
            b.preview_link_cells.borrow_mut().clear();
            let device = b.device;
            let path = b.path.clone();
            if !self.send(device, Operation::List { path }) {
                if let Some(b) = &mut self.browser {
                    b.loading = false;
                }
                self.set_notice_as(
                    NoticeKind::Warning,
                    "Refresh queue busy · retry shortly".into(),
                );
            }
        }
    }
    // Network browser owns candidate selection and captures scope before actions.
    fn open_containers(&mut self) {
        self.view = View::Containers;
        self.focus = Focus::Workspace;
        self.search.clear();
        self.container_selected = 0;
        self.refresh_containers();
    }
    fn refresh_containers(&mut self) {
        for d in 0..self.devices.len() {
            if (self.device == 0 || self.device == d + 1)
                && !self.containers_loading.contains(&d)
                && self.send(d, Operation::Containers)
            {
                self.containers_loading.insert(d);
            }
        }
    }
    fn container_rows(&self) -> Vec<(usize, Option<crate::containers::Container>)> {
        let mut rows = vec![];
        for d in 0..self.devices.len() {
            if self.device > 0 && self.device != d + 1 {
                continue;
            }
            rows.push((d, None));
            if self.container_collapsed.contains(&d) {
                continue;
            }
            if let Some(items) = self.containers.get(&d) {
                for c in items {
                    if fuzzy_score(
                        &self.search,
                        &format!(
                            "{} {} {} {}",
                            c.name,
                            c.image,
                            c.state,
                            if c.devcontainer {
                                "devcontainer"
                            } else {
                                "docker"
                            }
                        ),
                    )
                    .is_some()
                    {
                        rows.push((d, Some(c.clone())));
                    }
                }
            }
        }
        rows
    }
    fn selected_container(&self) -> Option<(usize, crate::containers::Container)> {
        self.container_rows()
            .get(self.container_selected)
            .and_then(|(d, c)| c.clone().map(|c| (*d, c)))
    }
    fn open_container_actions(&mut self) {
        if let Some((d, c)) = self.selected_container() {
            self.dialog = Some(Dialog::ContainerActions(d, c));
            self.dialog_selected = 0;
        } else if let Some((d, _)) = self.container_rows().get(self.container_selected) {
            let d = *d;
            if !self.container_collapsed.remove(&d) {
                self.container_collapsed.insert(d);
            }
        }
    }
    fn container_tree_motion(&mut self, expand: bool) {
        if let Some((d, c)) = self.container_rows().get(self.container_selected) {
            let d = *d;
            if expand {
                self.container_collapsed.remove(&d);
                if c.is_some() {
                    self.open_container_actions();
                }
            } else {
                self.container_collapsed.insert(d);
                self.container_selected = self
                    .container_rows()
                    .iter()
                    .position(|(i, c)| *i == d && c.is_none())
                    .unwrap_or(0);
            }
        }
    }
    fn start_container_session(
        &mut self,
        d: usize,
        scope: ContainerScope,
        provider: String,
        yolo: bool,
    ) {
        let request = CreateSession {
            key: unique_key(),
            directory: scope.folder.clone(),
            provider,
            name: String::new(),
        };
        self.creating = self.send(
            d,
            Operation::ContainerCreate {
                scope,
                request,
                yolo,
            },
        );
        self.set_notice("Starting container session · existing networks are preserved".into());
    }
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
    fn network_key(parts: Value) -> String {
        parts.to_string()
    }
    fn peer_key(&self, d: usize) -> String {
        Self::network_key(serde_json::json!(["peer", self.devices[d].id]))
    }
    fn known_neighbor(&self, candidate: &Value) -> Option<usize> {
        let mac = normalized_mac(candidate["lladdr"].as_str()?)?;
        let ip: std::net::IpAddr = candidate["address"].as_str()?.parse().ok()?;
        if candidate["_snapshot"]
            .as_u64()
            .is_none_or(|time| transport::now().saturating_sub(time) > 90)
        {
            return None;
        }
        let matches: Vec<_> = self
            .devices
            .iter()
            .enumerate()
            .filter_map(|(d, _)| {
                let value = self.network.get(&d)?;
                let checked = value["_authenticated_at"].as_u64()?;
                if transport::now().saturating_sub(checked) > 90 {
                    return None;
                }
                let interfaces = value["interfaces"]
                    .as_array()
                    .or_else(|| value["interfaces"]["data"].as_array())?;
                interfaces
                    .iter()
                    .any(|interface| {
                        normalized_mac(interface["address"].as_str().unwrap_or(""))
                            == Some(mac.clone())
                            && interface["addr_info"].as_array().is_some_and(|addresses| {
                                addresses.iter().any(|address| {
                                    address["local"]
                                        .as_str()
                                        .and_then(|ip| ip.parse::<std::net::IpAddr>().ok())
                                        == Some(ip)
                                })
                            })
                    })
                    .then_some(d)
            })
            .collect();
        (matches.len() == 1).then(|| matches[0])
    }
    // Every passive observation remains a separate route beneath its enrolled observer.
    fn network_rows(&self) -> Vec<Value> {
        let candidates = self.network_candidates();
        let mut rows = Vec::new();
        for (d, device) in self.devices.iter().enumerate() {
            if self.device > 0 && self.device != d + 1 {
                continue;
            }
            let key = self.peer_key(d);
            let owned: Vec<_> = candidates
                .iter()
                .filter(|c| c["_device"] == d)
                .cloned()
                .collect();
            let in_scope = self.device == 0 || self.device == d + 1;
            let label = if owned.is_empty() {
                device.name.clone()
            } else {
                format!("{} · {} LAN", device.name, owned.len())
            };
            rows.push(serde_json::json!({"_peer":d,"_device":d,"_key":key,"_depth":0,"_label":label,"_branch":self.device == 0 && !owned.is_empty(),"_count":owned.len()}));
            if in_scope && (self.device > 0 || self.network_expanded.contains(&key)) {
                for candidate in owned {
                    rows.push(self.network_route(
                        candidate,
                        &key,
                        if self.device == 0 { 1 } else { 0 },
                    ));
                }
            }
        }
        rows
    }
    fn network_route(&self, mut candidate: Value, parent: &str, depth: usize) -> Value {
        let d = candidate["_device"].as_u64().unwrap_or(0) as usize;
        candidate["_key"] = serde_json::json!(Self::network_key(serde_json::json!([
            "route",
            self.devices[d].id,
            candidate["interface"],
            candidate["address"]
        ])));
        candidate["_parent"] = serde_json::json!(parent);
        candidate["_depth"] = serde_json::json!(depth);
        if let Some(peer) = self.known_neighbor(&candidate) {
            candidate["_known_peer"] = serde_json::json!(peer);
        }
        candidate
    }
    fn network_selection(&self) -> Option<(Value, Value, usize)> {
        self.network_rows().get(self.network_selected).map(|row| {
            (
                row["_key"].clone(),
                row["_parent"].clone(),
                row["_device"].as_u64().unwrap_or(0) as usize,
            )
        })
    }
    fn restore_network_selection(&mut self, selection: Option<(Value, Value, usize)>) {
        let rows = self.network_rows();
        self.network_selected = selection
            .and_then(|(key, parent, owner)| {
                rows.iter()
                    .position(|r| r["_key"] == key)
                    .or_else(|| rows.iter().position(|r| r["_key"] == parent))
                    .or_else(|| rows.iter().position(|r| r["_key"] == self.peer_key(owner)))
            })
            .unwrap_or_else(|| self.network_selected.min(rows.len().saturating_sub(1)));
    }
    fn network_tree_motion(&mut self, expand: bool) {
        self.network_detail_scroll = 0;
        let rows = self.network_rows();
        let Some(row) = rows.get(self.network_selected) else {
            return;
        };
        let key = row["_key"].as_str().unwrap_or("").to_owned();
        if expand {
            if row["_branch"] == true {
                if !self.network_expanded.insert(key.clone()) {
                    if let Some(child) =
                        self.network_rows().iter().position(|r| r["_parent"] == key)
                    {
                        self.network_selected = child;
                    }
                }
            }
        } else if !self.network_expanded.remove(&key) {
            if let Some(parent) = rows.iter().position(|r| r["_key"] == row["_parent"]) {
                self.network_selected = parent;
            }
        }
    }
    fn network_detail(&self, row: &Value) -> String {
        let d = row["_device"].as_u64().unwrap_or(0) as usize;
        let observer = &self.devices[d];
        let arrow = if ascii() { " -> " } else { " → " };
        let hops = match self.network_jump_routes.get(&d) {
            Some(Some(hops)) if hops.is_empty() => "none saved".into(),
            Some(Some(hops)) => hops
                .iter()
                .map(|h| safe_label(h))
                .collect::<Vec<_>>()
                .join(arrow),
            _ => "not read".into(),
        };
        if row["_peer"].is_u64() {
            let overview = if self.device > 0 {
                self.network
                    .get(&d)
                    .map(|v| format!("\n\n{}", network_summary(v)))
                    .unwrap_or_else(|| "\nNetwork observation unavailable · Refresh".into())
            } else {
                String::new()
            };
            return format!("{} · {}\nRoute found: viewer{arrow}{}\n{} · helper evidence from viewer\nSSH jumps (saved): {hops}\nVia another observer: unknown{overview}", safe_label(&observer.name), identity(observer), safe_label(&observer.name), self.peer_status(d));
        }
        let name = row["_known_peer"]
            .as_u64()
            .and_then(|d| self.devices.get(d as usize))
            .map(|d| {
                format!(
                    "{} · matching fresh authenticated MAC + IP\n",
                    safe_label(&d.name)
                )
            })
            .unwrap_or_default();
        let observed = row["_snapshot"].as_u64().unwrap_or(0);
        let age = if observed == 0 {
            "age unknown".into()
        } else {
            format!(
                "{}s ago{}",
                transport::now().saturating_sub(observed),
                if transport::now().saturating_sub(observed) > 90 {
                    " · stale"
                } else {
                    ""
                }
            )
        };
        let hostname = row["hostname"]
            .as_str()
            .map(|h| {
                format!(
                    "Host hint: {} · name lookup, SSH identity unchecked\n",
                    safe_label(h)
                )
            })
            .unwrap_or_default();
        format!("{name}{hostname}Enter: connect via observer and add to All devices\nDiscovery: {} · {} · {age}\nRoute found: viewer{arrow}{}{arrow}{} / {}\nSSH jumps to observer (saved): {hops}\nSSH {} · authentication unknown\nInternet unknown · source {} · link {}", safe_label(&observer.name), safe_label(row["source"].as_str().unwrap_or("unknown")), safe_label(&observer.name), safe_label(row["address"].as_str().unwrap_or("unknown")), safe_label(row["interface"].as_str().unwrap_or("unknown")), neighbor_ssh(row), safe_label(row["source"].as_str().unwrap_or("unknown")), safe_label(row["link_state"].as_str().unwrap_or("unknown")))
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
            self.set_notice_as(
                NoticeKind::Warning,
                "No devices or observed LAN neighbors".into(),
            );
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
            self.set_notice_as(
                NoticeKind::Warning,
                "Link-local SSH enrollment needs an interface scope · unavailable here".into(),
            );
        } else if let Some(address) = candidate["address"].as_str() {
            self.network_add_target = Some((d, address.into()));
            self.input = Some(Input::Add);
            self.text.clear();
            self.set_notice(
                "Enter SSH account · enrollment checks authentication and helper access".into(),
            );
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
            View::Containers => self.refresh_containers(),
            View::Work => self.refresh_work(),
            View::Files => self.refresh_browser(),
            View::Network => {
                for (d, device) in self.devices.iter().enumerate() {
                    self.network_jump_routes.insert(
                        d,
                        device
                            .target
                            .as_deref()
                            .map(|target| store::route(target).ok())
                            .unwrap_or_else(|| Some(Vec::new())),
                    );
                }
                self.neighbor_budget = 32;
                self.neighbor_queue.clear();
                self.peer_checks
                    .retain(|d| self.device == 0 || self.device == d + 1);
                self.network_refresh_queue
                    .retain(|d| self.device == 0 || self.device == d + 1);
                for d in 0..self.devices.len() {
                    if self.device > 0 && self.device != d + 1 {
                        continue;
                    }
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
                    self.dialog = Some(Dialog::ContainerProvider(d, c.scope()));
                    self.dialog_selected = 0;
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
                    self.dialog = Some(Dialog::ContainerProvider(b.device, scope));
                    self.dialog_selected = 0;
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
                            self.dialog = Some(Dialog::ContainerProvider(d, scope));
                            self.dialog_selected = 0;
                            return;
                        }
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
                    if let Some(scope) = session.container {
                        self.open_container_browser(d, scope);
                    } else {
                        self.open_host_browser(d, session.directory);
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
    fn execute_palette(&mut self, action: Action) {
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
    fn device_choices(&self, purpose: ChooseDevice) -> Vec<Option<usize>> {
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
    fn session_shortcut(&mut self) {
        if self.creating {
            return;
        }
        self.launch_provider = None;
        if self.focus != Focus::Devices && self.view == View::Containers {
            if let Some((d, c)) = self.selected_container() {
                if (c.devcontainer || c.allowed) && c.state == "running" {
                    self.dialog = Some(Dialog::ContainerProvider(d, c.scope()));
                    self.dialog_selected = 0;
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
                    self.dialog = Some(Dialog::ContainerProvider(b.device, scope));
                    self.dialog_selected = 0;
                    return;
                }
            }
        }
        if self.focus == Focus::Workspace && self.view == View::Work {
            if let Some((d, session)) = self.selected_session() {
                if let Some(scope) = session.container {
                    self.dialog = Some(Dialog::ContainerProvider(d, scope));
                    self.dialog_selected = 0;
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
    fn open_session_chooser(&mut self, d: usize, path: String) {
        self.check_providers(d);
        self.dialog = Some(Dialog::SessionChooser(d, path));
        self.dialog_selected = 0;
    }
    fn start_shell(&mut self) {
        if self.view == View::Containers {
            if let Some((d, c)) = self.selected_container() {
                if (!c.devcontainer && !c.allowed) || c.state != "running" {
                    self.open_container_actions();
                    return;
                }
                self.start_container_session(d, c.scope(), "shell".into(), false);
            }
            return;
        }
        if self.view == View::Files {
            if let Some(b) = &self.browser {
                if let Some(scope) = &b.container {
                    let d = b.device;
                    let mut scope = scope.clone();
                    scope.folder = b.path.clone();
                    self.start_container_session(
                        d,
                        scope,
                        menus::CONTAINER[self.dialog_selected].provider.into(),
                        false,
                    );
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
    fn destination_containers(&self, device: usize) -> Vec<crate::containers::Container> {
        let mut containers = self.containers.get(&device).cloned().unwrap_or_default();
        containers.retain(|c| c.state == "running" && (c.devcontainer || c.allowed));
        containers.sort_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
        containers
    }
    fn conflict_policy(&self) -> &'static str {
        ["skip", "overwrite", "rename"][self.conflict]
    }
    fn start_at(&mut self, d: usize, directory: String, provider: String) {
        if !self.provider_choices(d).contains(&provider.as_str()) {
            self.check_providers(d);
            self.set_notice_as(
                NoticeKind::Warning,
                "Launch profile unavailable or not yet checked on this device".into(),
            );
            return;
        }
        if let Some(s) = self.work[d]
            .sessions
            .iter()
            .find(|s| s.container.is_none() && s.directory == directory && s.provider == provider)
            .cloned()
        {
            self.dialog = Some(Dialog::Matching(d, directory, provider, s));
            self.dialog_selected = 0;
        } else {
            self.create_at(d, directory, provider);
        }
    }
    fn create_at(&mut self, d: usize, directory: String, provider: String) {
        if provider != "shell" {
            self.dialog = Some(Dialog::Permissions(d, directory, provider));
            self.dialog_selected = 0;
            return;
        }
        self.create_permission_session(d, directory, provider, false);
    }
    fn create_permission_session(
        &mut self,
        d: usize,
        directory: String,
        provider: String,
        yolo: bool,
    ) {
        let key = unique_key();
        // Execution helper generates the label after resolving the directory.
        let name = String::new();
        let spec = CreateSession {
            key: key.clone(),
            directory,
            provider: provider.clone(),
            name,
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
    fn active_transfer_keys(&self) -> BTreeSet<String> {
        let mut keys = self.pending_transfers.clone();
        for (_, job) in self.job_rows() {
            if matches!(
                job["status"].as_str(),
                Some("queued" | "running" | "submitting")
            ) {
                if let Some(key) = job["key"].as_str() {
                    keys.insert(key.into());
                }
            }
        }
        keys
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
        let destination_container = b.container.clone();
        let active = self.active_transfer_keys();
        if self.submitted.iter().any(|(key, spec)| {
            active.contains(key)
                && spec.source.id == self.devices[clip.device].id
                && spec.destination.id == destination.id
                && spec.source_container == clip.container
                && spec.destination_container == destination_container
                && spec.destination_path == destination_path
                && clip.entries.iter().any(|e| e.path == spec.source_path)
        }) {
            self.set_notice_as(
                NoticeKind::Warning,
                "Transfer already pending · T shows progress".into(),
            );
            return;
        }
        let mut actions = Vec::new();
        for entry in &clip.entries {
            let spec = crate::model::TransferSpec {
                source_container: clip.container.clone(),
                destination_container: destination_container.clone(),
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
            self.pending_transfers.insert(spec.key.clone());
            self.submitted.insert(spec.key.clone(), spec.clone());
            if clip.cut {
                self.submitted_clipboards
                    .insert(spec.key.clone(), clip.id.clone());
            }
            actions.push((local, spec.operation()));
        }
        self.set_notice(format!(
            "{} {} items · {} → {} · existing: {}",
            if clip.cut { "Moving" } else { "Copying" },
            clip.entries.len(),
            file_endpoint_label(&self.devices[clip.device], &clip.container),
            file_endpoint_label(&destination, &destination_container),
            self.conflict_policy()
        ));
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
        let mut rows = unique
            .into_values()
            .filter(|(_, job)| {
                fuzzy_score(
                    &self.job_filter,
                    &format!(
                        "{} {} {} {} {} {}",
                        transfer_name(job),
                        transfer_status(job),
                        job["source_host"].as_str().unwrap_or(""),
                        job["destination_host"].as_str().unwrap_or(""),
                        job["source_display"]
                            .as_str()
                            .or(job["source_path"].as_str())
                            .unwrap_or(""),
                        transfer_destination(job)
                    ),
                )
                .is_some()
            })
            .collect::<Vec<_>>();
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
            Dialog::SessionChooser(..) => menus::HOST.len(),
            Dialog::DevcontainerUp(..) => 2,
            Dialog::ContainerActions(_, c) => container_action_labels(c).len(),
            Dialog::ContainerProvider(..) => menus::CONTAINER.len(),
            Dialog::DestinationScope(_, containers) => 1 + containers.len(),
            Dialog::ContainerConfirm(..) => 2,
            Dialog::Links(links) => links.len(),
            Dialog::Device(purpose) => self.device_choices(*purpose).len(),
            Dialog::Provider(d, _) => self.provider_choices(*d).len(),
            Dialog::Matching(..) | Dialog::Permissions(..) => 2,
            Dialog::Jobs => self.job_rows().len(),
            Dialog::Delete(..) | Dialog::StopShell(..) => 2,
            Dialog::PendingExit(_) => 2,
            Dialog::Neighbor(..) => 1,
            Dialog::Peer(_) => 5,
        };
        let horizontal = matches!(
            dialog,
            Dialog::SessionChooser(..) | Dialog::ContainerProvider(..)
        );
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
        if let Some(choices) = menus::session_choices(&dialog) {
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
            Dialog::Jobs
                | Dialog::Delete(..)
                | Dialog::StopShell(..)
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
            KeyCode::Enter => {
                match dialog {
                    Dialog::SessionChooser(d, path) => {
                        let provider = menus::HOST[self.dialog_selected].provider;
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

                    Dialog::Links(links) => {
                        if let Some(link) = links.get(self.dialog_selected) {
                            self.open_markdown_link(link);
                        }
                    }

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
                    Dialog::DevcontainerUp(d, workspace) => {
                        self.dialog = None;
                        if self.dialog_selected == 1 {
                            self.send(d, Operation::DevcontainerUp { workspace });
                            self.set_notice("Starting workspace · configuration hooks may run · up to 10 minutes"
                                .into());
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
                            "Shell" | "Devcontainer terminal" => {
                                self.start_container_session(d, c.scope(), "shell".into(), false)
                            }
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
                        self.dialog = None;
                        self.start_container_session(d, scope, "shell".into(), false);
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
                                        "Update CX on this device before rebuilding devcontainers"
                                            .into(),
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
                    Dialog::Permissions(d, path, provider) => {
                        let yolo = self.dialog_selected == 1;
                        if yolo
                            && self.devices[d].target.is_some()
                            && !self.providers.get(&d).is_some_and(|(caps, checked)| {
                                transport::now().saturating_sub(*checked) < 60
                                    && caps.iter().any(|c| c == "session-yolo-v1")
                            })
                        {
                            self.check_providers(d);
                            self.set_notice_as(NoticeKind::Warning, "YOLO needs an updated execution helper · update this device, then retry".into());
                            return;
                        }
                        self.dialog = None;
                        self.create_permission_session(d, path, provider, yolo);
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
                            self.set_notice("Deleting confirmed items permanently…".into());
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
                }
            }
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
    fn apply(&mut self, mut reply: Reply) {
        self.pending_requests
            .set(self.pending_requests.get().saturating_sub(1));
        if let Operation::ContainerFiles { scope, operation } = &reply.op {
            if self
                .browser
                .as_ref()
                .is_none_or(|b| b.device != reply.device || b.container.as_ref() != Some(scope))
                || reply.generation != self.generation
            {
                return;
            }
            reply.op = *operation.clone();
        }
        if let Operation::ContainerFileAction { operation, .. } = &reply.op {
            // Mutation replies release the serialized queue even after navigating away.
            reply.op = *operation.clone();
        }
        let file_action = matches!(
            reply.op,
            Operation::Remove { .. }
                | Operation::Rename { .. }
                | Operation::Transfer(_)
                | Operation::ScopedTransfer(_)
                | Operation::TransferRetry { .. }
        );
        if file_action {
            self.file_busy = false;
            self.start_next_file_action();
        }
        let connection_failure = reply
            .result
            .as_ref()
            .err()
            .is_some_and(transport::is_connection_failure);
        let notify_error = if reply.result.is_ok() {
            if reply.generation == self.generation {
                self.unavailable_notified
                    .remove(&self.devices[reply.device].id);
            }
            true
        } else if connection_failure {
            // Quiet neighbor probes and obsolete file replies must not consume the first notice.
            !matches!(reply.op, Operation::ProbeCandidate { .. })
                && (reply.generation == self.generation || file_action)
                && self
                    .unavailable_notified
                    .insert(self.devices[reply.device].id.clone())
        } else {
            true
        };
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
        if matches!(
            reply.op,
            Operation::Create(_) | Operation::CreateYolo(_) | Operation::ContainerCreate { .. }
        ) {
            self.creating = false;
        }
        if matches!(reply.op, Operation::Containers) {
            self.containers_loading.remove(&reply.device);
            match &reply.result {
                Ok(v) => {
                    match serde_json::from_value::<Vec<crate::containers::Container>>(
                        v["containers"].clone(),
                    ) {
                        Ok(items) => {
                            self.containers.insert(reply.device, items);
                            self.container_errors.remove(&reply.device);
                        }
                        Err(_) => {
                            self.container_errors.insert(
                                reply.device,
                                "Invalid container discovery response".into(),
                            );
                        }
                    }
                }
                Err(e) => {
                    self.container_errors.insert(reply.device, format!("{e:#}"));
                    self.containers.remove(&reply.device);
                    if connection_failure && notify_error {
                        self.set_notice_as(
                            NoticeKind::Error,
                            safe_text(&format!("{}: {e:#}", identity(&self.devices[reply.device]))),
                        );
                    }
                }
            }
            if let Some(Dialog::DestinationScope(d, choices)) = &self.dialog {
                if *d == reply.device && choices.is_empty() && reply.result.is_ok() {
                    // The initial scan fills an empty picker; once populated, choices
                    // stay pinned so refresh cannot silently select a different container.
                    self.dialog = Some(Dialog::DestinationScope(
                        *d,
                        self.destination_containers(*d),
                    ));
                }
            }
            self.container_selected = self
                .container_selected
                .min(self.container_rows().len().saturating_sub(1));
            return;
        }
        if matches!(
            reply.op,
            Operation::DevcontainerUp { .. } | Operation::DevcontainerRebuild { .. }
        ) {
            let rebuild = if let Operation::DevcontainerRebuild { scope } = &reply.op {
                self.rebuilding.remove(&(reply.device, scope.id.clone()));
                true
            } else {
                false
            };
            self.send(reply.device, Operation::Containers);
            self.containers_loading.insert(reply.device);
            match &reply.result {
                Ok(_) => {
                    if !rebuild {
                        self.device = reply.device + 1;
                        self.open_containers();
                    }
                    self.set_notice_as(
                        NoticeKind::Success,
                        "Workspace ready · choose its terminal or files".into(),
                    );
                }
                Err(e) if notify_error => self.set_notice_as(
                    NoticeKind::Error,
                    format!("{e:#} · refresh checks workspace state"),
                ),
                Err(_) => (),
            }
            return;
        }
        if matches!(
            reply.op,
            Operation::ContainerLifecycle { .. } | Operation::ContainerAccess { .. }
        ) {
            if reply.result.is_ok() || notify_error {
                self.set_notice_as(
                    if reply.result.is_err() {
                        NoticeKind::Error
                    } else {
                        NoticeKind::Success
                    },
                    match &reply.result {
                        Ok(_) => {
                            "Container action complete · network configuration preserved".into()
                        }
                        Err(e) => format!("{e:#}"),
                    },
                );
            }
            self.refresh_containers();
            return;
        }
        let value = match reply.result {
            Ok(v) => v,
            Err(e) => {
                if let Operation::Transfer(spec) | Operation::ScopedTransfer(spec) = &reply.op {
                    self.pending_transfers.remove(&spec.key);
                }
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
                        v["_authenticated_at"] = Value::Null;
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
                        if notify_error {
                            self.set_notice_as(
                                NoticeKind::Error,
                                format!(
                                    "{} file actions failed · {}",
                                    self.file_errors.len(),
                                    message
                                ),
                            );
                        }
                    } else if notify_error {
                        self.set_notice_as(NoticeKind::Error, message.clone());
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
                            b.preview = Some(message.clone());
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
        if let Operation::Transfer(spec) | Operation::ScopedTransfer(spec) = &reply.op {
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
                                    "stop-agent-session-v1",
                                    "session-yolo-v1",
                                    "containers-v1",
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
            Operation::Create(_) | Operation::CreateYolo(_) | Operation::ContainerCreate { .. } => {
                match serde_json::from_value::<Session>(value) {
                    Ok(s) => {
                        self.work[reply.device].sessions.push(s.clone());
                        self.pending_attach = Some((reply.device, s, false));
                    }
                    Err(_) => self.set_notice_as(
                        NoticeKind::Error,
                        "Creation response invalid · refresh before retrying".into(),
                    ),
                }
            }
            Operation::Jobs | Operation::TransferJobs => {
                if let Some(rows) = value["jobs"].as_array() {
                    for job in rows {
                        if let Some(key) = job["key"].as_str() {
                            self.pending_transfers.remove(key);
                        }
                    }
                }
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
                    self.set_notice_as(
                        match job["status"].as_str() {
                            Some("complete") => NoticeKind::Success,
                            Some("failed") => NoticeKind::Error,
                            Some("cancelled") => NoticeKind::Warning,
                            _ => NoticeKind::Info,
                        },
                        format!(
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
                        ),
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
                self.set_notice("Cancellation requested · waiting for worker".into());
            }
            Operation::Transfer(_)
            | Operation::ScopedTransfer(_)
            | Operation::TransferRetry { .. } => {
                if self.file_errors.is_empty() {
                    self.set_notice(format!(
                        "Transfer {} · {}",
                        safe_label(value["status"].as_str().unwrap_or("queued")),
                        safe_label(value["route"].as_str().unwrap_or("worker host"))
                    ));
                } else {
                    self.set_notice_as(
                        NoticeKind::Error,
                        format!(
                            "{} file actions failed · {}",
                            self.file_errors.len(),
                            self.file_errors.last().unwrap()
                        ),
                    );
                }
                self.send(reply.device, Operation::TransferJobs);
            }
            Operation::Copy { .. } => {
                self.set_notice(format!(
                    "Copy {} on {}",
                    value
                        .get("status")
                        .and_then(Value::as_str)
                        .unwrap_or("status unknown"),
                    identity(&self.devices[reply.device])
                ));
                self.send(reply.device, Operation::Jobs);
            }
            Operation::Network => {
                let selection = self.network_selection();
                let mut value = value;
                value["_authenticated_at"] = serde_json::json!(transport::now());
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
                self.restore_network_selection(selection);
            }
            Operation::NetworkCandidates => {
                let selected = self.network_selection();
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
                self.restore_network_selection(selected);
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
                    let range_finished = b.visual_anchor.take().is_some();
                    if range_finished {
                        b.visual_base.clear();
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
                    if range_finished {
                        self.set_notice(
                            "Directory refreshed · range finished; selected files kept".into(),
                        );
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
                            self.set_notice(
                                "Loading more directory entries · fuzzy search stays live".into(),
                            );
                        } else {
                            self.set_notice("10,000 entries loaded · remaining entries require a narrower directory".into());
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
                        b.preview_scroll.set(0);
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
                        let restored = b.preview_restore.take();
                        let restoring = restored.is_some();
                        if let Some((scroll, find)) = restored {
                            b.preview_scroll.set(scroll);
                            b.preview_find = find;
                        }
                        b.preview_find.matches = preview_matches(
                            &preview_display_lines(b, b.preview.as_deref().unwrap_or("")),
                            &b.preview_find.query,
                        );
                        b.preview_find.selected = b
                            .preview_find
                            .selected
                            .min(b.preview_find.matches.len().saturating_sub(1));
                        b.preview_find.reveal.set(!restoring);
                        if let Some(anchor) = b.preview_anchor.take() {
                            if let Some(scroll) = markdown_anchor_scroll(b, &anchor) {
                                b.preview_scroll.set(scroll);
                                b.preview_find.reveal.set(false);
                            } else {
                                self.set_notice_as(
                                    NoticeKind::Warning,
                                    "Linked file opened · heading not found".into(),
                                );
                            }
                        }
                    }
                }
                if next {
                    self.start_pdf_page_request();
                }
            }
            Operation::StopSession { .. } | Operation::StopAgentSession { .. } => {
                self.set_notice(format!(
                    "Session stopped · {}",
                    identity(&self.devices[reply.device])
                ));
                self.work[reply.device].loading = self.send(reply.device, Operation::Sessions);
            }
            Operation::Mkdir { .. } if reply.generation == self.generation => {
                self.refresh_browser()
            }
            Operation::Rename { path, .. } | Operation::Remove { path, .. } => {
                if self.file_errors.is_empty() && self.file_queue.is_empty() && !self.file_busy {
                    self.set_notice(format!(
                        "File action complete · {}",
                        identity(&self.devices[reply.device])
                    ));
                }
                self.browser_cache.retain(|(d, _, _), _| *d != reply.device);
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
            if let Some(motion) = self.navigation.read(key, 8, false) {
                navigation::Scroll {
                    offset: &mut self.help_scroll,
                    max: 40,
                }
                .apply(motion);
                return;
            }
            if matches!(key.code, KeyCode::Esc | KeyCode::F(1) | KeyCode::Char('?')) {
                self.help = false;
            }
            return;
        }
        if let Some(dialog) = self.dialog.clone().filter(|_| self.input.is_none()) {
            if matches!(key.code, KeyCode::Char('?') | KeyCode::F(1)) {
                self.help = true;
                return;
            }
            self.dialog_key(key, dialog);
            return;
        }
        // Input fields own every printable key, including navigation shortcuts.
        if let Some(mode) = self.input {
            self.navigation.reset();
            let before_text = self.text.clone();
            match key.code {
                KeyCode::Esc => {
                    self.input = None;
                    self.text.clear();
                    self.rename_target = None;
                    self.command_target = None;
                    self.network_add_target = None;
                    self.pending_add_via = None;
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
                KeyCode::Down if mode == Input::PreviewSearch => self.next_preview_match(1),
                KeyCode::Up if mode == Input::PreviewSearch => self.next_preview_match(-1),
                KeyCode::Down if mode == Input::Search && self.view == View::Files => {
                    self.next_file_match(1)
                }
                KeyCode::Up if mode == Input::Search && self.view == View::Files => {
                    self.next_file_match(-1)
                }
                KeyCode::Down if mode == Input::Search => self.move_selection(1),
                KeyCode::Up if mode == Input::Search => self.move_selection(-1),
                KeyCode::Down if mode == Input::Filter && self.dialog.is_some() => {
                    if let Some(dialog) = self.dialog.clone() {
                        self.dialog_key(key, dialog);
                    }
                }
                KeyCode::Up if mode == Input::Filter && self.dialog.is_some() => {
                    if let Some(dialog) = self.dialog.clone() {
                        self.dialog_key(key, dialog);
                    }
                }
                KeyCode::Down if mode == Input::Filter => self.move_selection(1),
                KeyCode::Up if mode == Input::Filter => self.move_selection(-1),
                KeyCode::Tab | KeyCode::BackTab
                    if matches!(mode, Input::Search | Input::PreviewSearch | Input::Filter) =>
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
                            self.execute_palette(action);
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
                        if self.view != View::Files {
                            self.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
                        }
                    }
                    Input::PreviewSearch => {
                        self.input = None;
                        self.focus = Focus::Workspace;
                    }
                    Input::Filter => {
                        self.input = None;
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
                                    self.set_notice_as(
                                        NoticeKind::Warning,
                                        "Name unchanged".into(),
                                    );
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
                            self.set_notice(
                                "Enter one filename · existing files are never replaced".into(),
                            );
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
                                .map(|(d, _)| self.devices[d].clone())
                                .or_else(|| self.pending_add_via.take());
                            self.pending_add = Some(target);
                            self.input = None;
                        } else {
                            self.set_notice_as(
                                NoticeKind::Warning,
                                "Use an SSH alias or user@host".into(),
                            );
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
                            self.set_notice_as(
                                NoticeKind::Warning,
                                "Enter a single directory name".into(),
                            );
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
            if mode == Input::PreviewSearch
                && self.input == Some(Input::PreviewSearch)
                && self.text != before_text
            {
                self.update_preview_search();
            }
            if mode == Input::Filter
                && self.input == Some(Input::Filter)
                && self.text != before_text
            {
                self.update_list_filter();
            }
            return;
        }
        if self.view == View::Files
            && self.focus == Focus::Workspace
            && self.browser.as_ref().is_some_and(|b| b.preview.is_some())
            && !key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            match key.code {
                KeyCode::Char('o') if self.markdown_preview_active() => {
                    self.show_preview_links();
                    return;
                }
                KeyCode::Char('/') => {
                    if self.text_preview_active() {
                        self.input = Some(Input::PreviewSearch);
                        self.text = self.browser.as_ref().unwrap().preview_find.query.clone();
                    } else {
                        self.set_notice(
                            if self
                                .browser
                                .as_ref()
                                .is_some_and(|b| b.preview_pending_page.is_some())
                            {
                                "Preview is still loading"
                            } else {
                                "Search is available in text previews"
                            }
                            .into(),
                        );
                    }
                    return;
                }
                KeyCode::Char('n') => {
                    let count = self.navigation.take_count().unwrap_or(1);
                    for _ in 0..count {
                        self.next_preview_match(1);
                    }
                    return;
                }
                KeyCode::Char('N') => {
                    let count = self.navigation.take_count().unwrap_or(1);
                    for _ in 0..count {
                        self.next_preview_match(-1);
                    }
                    return;
                }
                KeyCode::Esc
                    if self
                        .browser
                        .as_ref()
                        .is_some_and(|b| !b.preview_find.query.is_empty()) =>
                {
                    self.browser.as_mut().unwrap().preview_find = PreviewFind::default();
                    return;
                }
                KeyCode::Esc
                    if self
                        .browser
                        .as_ref()
                        .is_some_and(|b| !b.preview_history.is_empty()) =>
                {
                    let b = self.browser.as_mut().unwrap();
                    let device = b.device;
                    let (path, scroll, find) = b.preview_history.pop().unwrap();
                    self.open_preview(device, path);
                    self.browser.as_mut().unwrap().preview_restore = Some((scroll, find));
                    return;
                }
                _ => {}
            }
        }
        if self.view == View::Files
            && self.focus == Focus::Workspace
            && self.browser.as_ref().is_some_and(|b| !b.search.is_empty())
            && matches!(key.code, KeyCode::Char('n' | 'N'))
            && !key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            let count = self.navigation.take_count().unwrap_or(1);
            self.finish_visual();
            for _ in 0..count {
                self.next_file_match(if key.code == KeyCode::Char('n') {
                    1
                } else {
                    -1
                });
            }
            return;
        }
        if let Some(motion) =
            self.navigation
                .read(key, if self.pdf_preview_active() { 1 } else { 10 }, false)
        {
            panels::active(self).apply(motion);
            return;
        }
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
                if session.external
                    || !["shell", "codex", "claude"].contains(&session.provider.as_str())
                {
                    self.set_notice_as(
                        NoticeKind::Warning,
                        "Only CX-managed sessions can be stopped here".into(),
                    );
                } else if self.devices[device].target.is_some()
                    && !self.providers.get(&device).is_some_and(|(caps, checked)| {
                        transport::now().saturating_sub(*checked) < 60
                            && caps.iter().any(|c| {
                                c == if session.provider == "shell" {
                                    "stop-session-v1"
                                } else {
                                    "stop-agent-session-v1"
                                }
                            })
                    })
                {
                    self.check_providers(device);
                    self.set_notice(
                        "Checking session-stop support · press d again when ready".into(),
                    );
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
                self.set_notice("Clipboard cleared · files unchanged".into());
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
            KeyCode::Char('J') | KeyCode::Char('K')
                if self.view == View::Network && self.focus == Focus::Workspace =>
            {
                self.network_detail_scroll = if key.code == KeyCode::Char('J') {
                    self.network_detail_scroll.saturating_add(1).min(256)
                } else {
                    self.network_detail_scroll.saturating_sub(1)
                };
            }
            KeyCode::Char('n' | 'N')
                if self.focus == Focus::Workspace
                    && self.view == View::Files
                    && self.browser.as_ref().is_some_and(|b| !b.search.is_empty()) =>
            {
                self.finish_visual();
                self.next_file_match(if key.code == KeyCode::Char('n') {
                    1
                } else {
                    -1
                });
            }
            KeyCode::Char('n') => self.session_shortcut(),
            KeyCode::Char('w') if self.view == View::Work && self.focus == Focus::Workspace => {
                self.execute(Action::Observe)
            }
            KeyCode::Char('a') if self.view == View::Work && local_only(self) => {
                self.execute(Action::Add)
            }
            KeyCode::Char(':') => self.execute(Action::Command),
            KeyCode::Char('?') | KeyCode::F(1) => self.help = true,
            KeyCode::Char('T') => self.execute(Action::Jobs),
            KeyCode::Char('f') if self.view != View::Files || self.focus == Focus::Devices => {
                self.begin_list_filter()
            }
            KeyCode::Char('/') if self.focus == Focus::Devices => self.begin_list_filter(),
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
                if self.view == View::Containers && self.focus == Focus::Workspace {
                    self.container_tree_motion(false);
                } else if self.view == View::Network && self.focus == Focus::Workspace {
                    self.network_tree_motion(false);
                } else if self.view == View::Files && self.focus == Focus::Workspace {
                    self.parent_directory();
                } else {
                    self.focus = Focus::Devices;
                }
            }
            KeyCode::Right | KeyCode::Char('l') => {
                if self.view == View::Containers && self.focus == Focus::Workspace {
                    self.container_tree_motion(true);
                } else if self.view == View::Network && self.focus == Focus::Workspace {
                    self.network_tree_motion(true);
                } else if self.view == View::Files && self.focus == Focus::Workspace {
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
                        View::Containers => self.open_container_actions(),
                        View::Work => {
                            if let Some((d, s)) = self.selected_session() {
                                self.pending_attach = Some((d, s, false));
                            } else if empty_work_can_create(self) {
                                self.start_shell();
                            }
                        }
                        View::Files => {
                            if self.destination_active
                                && self.clipboard.is_some()
                                && self.browser.as_ref().is_some_and(|b| b.preview.is_none())
                            {
                                if self.browser.as_ref().is_some_and(|b| !b.loading) {
                                    self.submit_transfer();
                                }
                            } else if let Some(b) = &self.browser {
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
                        b.preview_find = PreviewFind::default();
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
            self.set_notice(format!(
                "{} · cx {} updated",
                identity(&self.devices[device]),
                update.version
            ));
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
            b.preview_restore = None;
            b.preview_anchor = None;
            b.preview_link_cells.borrow_mut().clear();
            b.preview_requested_page = 1;
            b.preview_pending_page = Some(1);
            b.preview_rich = None;
            b.preview_scroll.set(0);
            b.preview_find = PreviewFind::default();
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
            self.set_notice("PDF pages need a current device helper · checking update".into());
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
            self.set_notice_as(
                NoticeKind::Warning,
                "Preview queue busy · retry shortly".into(),
            );
        }
    }
    fn text_preview_active(&self) -> bool {
        self.view == View::Files
            && self.browser.as_ref().is_some_and(|b| {
                b.preview.is_some()
                    && b.preview_pending_page.is_none()
                    && b.preview_rich
                        .as_ref()
                        .is_none_or(|p| p.raster.is_none() && p.kind != "pdf")
            })
    }
    fn update_preview_search(&mut self) {
        if let Some(b) = &mut self.browser {
            b.preview_find.query = self.text.clone();
            b.preview_find.matches = preview_matches(
                &preview_display_lines(b, b.preview.as_deref().unwrap_or("")),
                &self.text,
            );
            b.preview_find.selected = 0;
            b.preview_find.reveal.set(true);
        }
    }
    fn next_preview_match(&mut self, direction: isize) {
        if let Some(b) = &mut self.browser {
            b.preview_find.matches = preview_matches(
                &preview_display_lines(b, b.preview.as_deref().unwrap_or("")),
                &b.preview_find.query,
            );
            let count = b.preview_find.matches.len();
            if count > 0 {
                b.preview_find.selected = b.preview_find.selected.min(count - 1);
                b.preview_find.selected = (b.preview_find.selected as isize + direction)
                    .rem_euclid(count as isize) as usize;
                b.preview_find.reveal.set(true);
            }
        }
    }
    fn markdown_preview_active(&self) -> bool {
        self.text_preview_active()
            && self.browser.as_ref().is_some_and(|b| {
                b.preview_rich
                    .as_ref()
                    .is_some_and(|p| p.kind == "markdown")
            })
    }
    fn open_markdown_link(&mut self, link: &crate::markdown_links::Link) {
        if crate::markdown_links::web_target(&link.target) {
            if std::env::var_os("DISPLAY").is_none()
                && std::env::var_os("WAYLAND_DISPLAY").is_none()
            {
                self.set_notice_as(
                    NoticeKind::Warning,
                    "No graphical browser on this viewer · inspect the URL with o".into(),
                );
                return;
            }
            match std::process::Command::new("xdg-open")
                .arg(&link.target)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
            {
                Ok(mut child) => {
                    std::thread::spawn(move || {
                        let _ = child.wait();
                    });
                    self.set_notice_as(NoticeKind::Success, "Link sent to viewer browser".into());
                    self.dialog = None;
                }
                Err(_) => self.set_notice_as(
                    NoticeKind::Warning,
                    "Viewer browser unavailable · inspect the URL with o".into(),
                ),
            }
            return;
        }
        let Some(b) = self.browser.as_ref() else {
            return;
        };
        let Some(document) = b.preview_path.as_ref() else {
            self.set_notice_as(
                NoticeKind::Warning,
                "Document path unavailable · reopen this file".into(),
            );
            return;
        };
        let resolved = crate::files::decode_path(document)
            .and_then(|p| crate::markdown_links::resolve_file_link(&p, &link.target));
        let (path, anchor) = match resolved {
            Ok(Some(destination)) => destination,
            Ok(None) => {
                self.set_notice_as(
                    NoticeKind::Warning,
                    "Only web URLs and file links can open".into(),
                );
                return;
            }
            Err(error) => {
                self.set_notice_as(NoticeKind::Error, safe_text(&error.to_string()));
                return;
            }
        };
        let device = b.device;
        let same = crate::files::decode_path(document).ok().as_ref() == Some(&path);
        if same && anchor.is_some() {
            self.dialog = None;
            let b = self.browser.as_mut().unwrap();
            if let Some(scroll) = markdown_anchor_scroll(b, anchor.as_deref().unwrap()) {
                b.preview_scroll.set(scroll);
                b.preview_find.reveal.set(false);
            } else {
                self.set_notice_as(
                    NoticeKind::Warning,
                    "Heading not found in this preview".into(),
                );
            }
            return;
        }
        self.dialog = None;
        if link
            .target
            .split('#')
            .next()
            .is_some_and(|p| p.ends_with('/'))
        {
            self.open_browser(device, crate::files::encode_path(&path));
            return;
        }
        let b = self.browser.as_mut().unwrap();
        if let Some(previous) = b.preview_path.clone() {
            if b.preview_history.len() >= 16 {
                b.preview_history.remove(0);
            }
            b.preview_history
                .push((previous, b.preview_scroll.get(), b.preview_find.clone()));
        }
        self.open_preview(device, crate::files::encode_path(&path));
        self.browser.as_mut().unwrap().preview_anchor = anchor;
    }
    fn show_preview_links(&mut self) {
        let links = self
            .browser
            .as_ref()
            .and_then(|b| b.preview.as_ref())
            .map(|s| markdown_preview_links(s))
            .unwrap_or_default();
        if links.is_empty() {
            self.set_notice_as(
                NoticeKind::Warning,
                "No Markdown links in this preview".into(),
            );
            return;
        }
        self.dialog_selected = 0;
        self.dialog_scroll = 0;
        self.dialog_detail_focus = false;
        self.dialog = Some(Dialog::Links(links));
    }
    fn mouse(&mut self, mouse: MouseEvent, area: Rect) -> bool {
        let delta = match mouse.kind {
            MouseEventKind::ScrollDown => 1,
            MouseEventKind::ScrollUp => -1,
            MouseEventKind::Down(event::MouseButton::Left)
                if self.view == View::Files
                    && !self.help
                    && self.dialog.is_none()
                    && self.input.is_none() =>
            {
                let target = self
                    .panels
                    .borrow()
                    .iter()
                    .rev()
                    .find(|(focus, _, rect)| {
                        *focus == Focus::Workspace
                            && rect
                                .contains(ratatui::layout::Position::new(mouse.column, mouse.row))
                    })
                    .map(|(_, destination, _)| *destination);
                let Some(destination) = target else {
                    return false;
                };
                // Click-to-focus applies only to previews, never opens or selects files.
                let browser =
                    if self.other_browser.is_some() && destination != self.destination_active {
                        self.other_browser.as_ref()
                    } else {
                        self.browser.as_ref()
                    };
                if !browser.is_some_and(|b| b.preview.is_some()) {
                    return false;
                }
                if self.other_browser.is_some() && destination != self.destination_active {
                    self.switch_pane();
                }
                self.focus = Focus::Workspace;
                self.preview_mouse(mouse, area);
                return true;
            }
            _ => return self.preview_mouse(mouse, area),
        };
        if mouse.modifiers != KeyModifiers::NONE {
            return false;
        }
        // Wheel motion reviews confirmation details; it cannot select the
        // destructive/exit choice and leave a subsequent Enter armed.
        if matches!(
            self.dialog,
            Some(
                Dialog::Delete(..)
                    | Dialog::StopShell(..)
                    | Dialog::PendingExit(..)
                    | Dialog::ContainerConfirm(..)
                    | Dialog::DevcontainerUp(..)
            )
        ) {
            self.dialog_scroll = if delta > 0 {
                self.dialog_scroll.saturating_add(1).min(4096)
            } else {
                self.dialog_scroll.saturating_sub(1)
            };
            return true;
        }
        // Inline file search/filter keep the browser visible. Wheel motion uses
        // adjacent rows/lines, not the keyboard's next-match navigation. Preserve
        // the query without re-running live_search and snapping to a match.
        let inline_file_input = self.view == View::Files
            && matches!(
                self.input,
                Some(Input::Search | Input::Filter | Input::PreviewSearch)
            );
        // Other editors/modals own navigation; a wheel never acts behind them.
        if self.help || self.dialog.is_some() || (self.input.is_some() && !inline_file_input) {
            self.key(KeyEvent::new(
                if delta > 0 {
                    KeyCode::Down
                } else {
                    KeyCode::Up
                },
                KeyModifiers::NONE,
            ));
            return true;
        }
        if area.width < 36 || area.height < 10 {
            return false;
        }
        let target = self
            .panels
            .borrow()
            .iter()
            .rev()
            .find(|(_, _, rect)| {
                rect.contains(ratatui::layout::Position::new(mouse.column, mouse.row))
            })
            .map(|(focus, destination, _)| (*focus, *destination));
        let Some((focus, destination)) = target else {
            return false;
        };
        if focus == Focus::Workspace
            && self.other_browser.is_some()
            && destination != self.destination_active
        {
            self.switch_pane();
        }
        self.focus = focus;
        if focus == Focus::Workspace && self.preview_mouse(mouse, area) {
            return true;
        }
        self.move_selection(delta);
        true
    }

    fn preview_mouse(&mut self, mouse: MouseEvent, area: Rect) -> bool {
        if self.help || self.dialog.is_some() || self.input.is_some() {
            return false;
        }
        if self.markdown_preview_active() {
            if matches!(mouse.kind, MouseEventKind::Down(event::MouseButton::Left)) {
                let tag = self.browser.as_ref().and_then(|b| {
                    b.preview_link_cells
                        .borrow()
                        .iter()
                        .find(|(x, y, _)| *x == mouse.column && *y == mouse.row)
                        .map(|(_, _, tag)| *tag)
                });
                if let Some(tag) = tag {
                    let links = self
                        .browser
                        .as_ref()
                        .and_then(|b| b.preview.as_ref())
                        .map(|s| markdown_preview_links(s))
                        .unwrap_or_default();
                    let matches: Vec<_> = links
                        .iter()
                        .enumerate()
                        .filter(|(_, l)| markdown_link_tag(&l.target) == tag)
                        .collect();
                    if matches.is_empty() {
                        return false;
                    }
                    let target = matches.first().map(|(_, l)| l.target.as_str());
                    let unique = target.is_some()
                        && matches
                            .iter()
                            .all(|(_, l)| Some(l.target.as_str()) == target);
                    if unique
                        && mouse
                            .modifiers
                            .intersects(KeyModifiers::SHIFT | KeyModifiers::CONTROL)
                    {
                        let link = matches[0].1.clone();
                        self.open_markdown_link(&link);
                    } else {
                        self.show_preview_links();
                        if unique {
                            self.dialog_selected = matches[0].0;
                        }
                    }
                    return true;
                }
            }
            return false;
        }
        if !self.pdf_preview_active() {
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
        panels::active(self).move_by(delta);
    }
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
        (Action::Containers, "Containers"),
        (Action::Shell, "New shell"),
        (Action::Jobs, "Transfers"),
    ];
    actions.retain(|(action, _)| {
        app.action_enabled(*action) && !(*action == Action::Files && app.view == View::Files)
    });
    if app.selected_session().is_some() && app.view == View::Work {
        actions.push((Action::Observe, "Watch · read-only"));
    }
    actions
}

fn markdown_heading_slug(text: &str) -> String {
    text.chars()
        .flat_map(char::to_lowercase)
        .filter_map(|c| {
            if c.is_whitespace() {
                Some('-')
            } else if c.is_alphanumeric() || c == '_' || c == '-' {
                Some(c)
            } else {
                None
            }
        })
        .collect()
}
fn markdown_anchor_scroll(browser: &Browser, anchor: &str) -> Option<u16> {
    let lines = preview_display_lines(browser, browser.preview.as_deref()?);
    let row = *browser
        .markdown_anchor_rows
        .borrow()
        .get(&markdown_heading_slug(anchor))?;
    Some(
        Paragraph::new(lines[..row].to_vec())
            .wrap(Wrap { trim: false })
            .line_count(browser.preview_viewport.get().0.max(1))
            .min(u16::MAX as usize) as u16,
    )
}

fn markdown_preview_links(source: &str) -> Vec<crate::markdown_links::Link> {
    let safe = safe_text(source);
    let fences = crate::syntax_preview::fenced_blocks(&safe);
    let mut links = Vec::new();
    for (index, line) in source.lines().enumerate() {
        if fences
            .iter()
            .any(|f| index >= f.opening && index <= f.closing.unwrap_or(usize::MAX))
        {
            continue;
        }
        links.extend(crate::markdown_links::links(line));
        if links.len() >= 128 {
            links.truncate(128);
            break;
        }
    }
    links
}

fn markdown_link_tag(target: &str) -> Color {
    use sha2::Digest;
    let hash = sha2::Sha256::digest(target.as_bytes());
    Color::Rgb(hash[0], hash[1], hash[2])
}
fn preview_inline(text: &str) -> Vec<Span<'static>> {
    preview_inline_tagged(text, false)
}
fn preview_inline_tagged(text: &str, tagged: bool) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut at = 0;
    for link in crate::markdown_links::links(text) {
        spans.extend(preview_emphasis(&text[at..link.start]));
        let mut style = tint(Color::Cyan).add_modifier(Modifier::UNDERLINED);
        // Tags exist only in a private hit-test layout, never the visible frame.
        if tagged {
            style.bg = Some(markdown_link_tag(&link.target));
        }
        for mut span in preview_emphasis(&link.label) {
            span.style = span.style.patch(style);
            spans.push(span);
        }
        at = link.end;
    }
    spans.extend(preview_emphasis(&text[at..]));
    spans
}

// Wrap the body using Ratatui's own Unicode/word layout, then restore the
// list prefix. Continuation rows have the exact same content starting column.
fn hanging_lines(line: &Line<'static>, prefix: usize, width: u16) -> Vec<Line<'static>> {
    if prefix >= usize::from(width) || prefix == 0 {
        return vec![line.clone()];
    }
    let mut leading = Vec::new();
    let mut body = Vec::new();
    let mut remaining = prefix;
    for span in &line.spans {
        let take = remaining.min(span.content.chars().count());
        let split = span
            .content
            .char_indices()
            .nth(take)
            .map(|(i, _)| i)
            .unwrap_or(span.content.len());
        if split > 0 {
            leading.push(Span::styled(span.content[..split].to_owned(), span.style));
        }
        if split < span.content.len() {
            body.push(Span::styled(span.content[split..].to_owned(), span.style));
        }
        remaining -= take;
    }
    let body_width = width - prefix as u16;
    let paragraph = Paragraph::new(Line::from(body)).wrap(Wrap { trim: true });
    let height = paragraph.line_count(body_width).max(1);
    if height > 8192 {
        return vec![line.clone()];
    }
    let rect = Rect::new(0, 0, body_width, height as u16);
    let mut buffer = ratatui::buffer::Buffer::empty(rect);
    ratatui::widgets::Widget::render(paragraph, rect, &mut buffer);
    (0..height as u16)
        .map(|y| {
            let mut spans = if y == 0 {
                leading.clone()
            } else {
                vec![Span::raw(" ".repeat(prefix))]
            };
            let mut x = 0;
            while x < body_width {
                let cell = &buffer[(x, y)];
                let symbol = cell.symbol();
                let step = Span::raw(symbol).width().max(1) as u16;
                if let Some(last) = spans.last_mut().filter(|s| s.style == cell.style()) {
                    last.content.to_mut().push_str(symbol);
                } else {
                    spans.push(Span::styled(symbol.to_owned(), cell.style()));
                }
                x = x.saturating_add(step);
            }
            if let Some(last) = spans.last_mut() {
                last.content = last.content.trim_end().to_owned().into();
            }
            Line::from(spans)
        })
        .collect()
}
fn list_prefix(source: &str) -> Option<usize> {
    let trimmed = source.trim_start_matches(' ');
    let indent = source.len() - trimmed.len();
    if ["- ", "* ", "+ "].iter().any(|p| trimmed.starts_with(p)) {
        return Some(indent + 2);
    }
    let digits = trimmed.bytes().take_while(u8::is_ascii_digit).count();
    if (1..=9).contains(&digits)
        && (trimmed[digits..].starts_with(". ") || trimmed[digits..].starts_with(") "))
    {
        return Some(indent + digits + 2);
    }
    None
}

fn preview_emphasis(text: &str) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut at = 0;
    let mut plain = 0;
    let mut slashes = 0;
    while at < text.len() {
        let rest = &text[at..];
        if rest.starts_with('`') {
            let run = rest.bytes().take_while(|b| *b == b'`').count();
            if let Some(end) = rest[run..].find(&"`".repeat(run)) {
                at += run + end + run;
                slashes = 0;
                continue;
            }
            break;
        }
        let escaped = slashes % 2 == 1;
        let delimiter = if !escaped && rest.starts_with("$$") {
            Some(("$$", "$$"))
        } else if !escaped && rest.starts_with('$') {
            Some(("$", "$"))
        } else if !escaped && rest.starts_with("\\(") {
            Some(("\\(", "\\)"))
        } else if !escaped && rest.starts_with("\\[") {
            Some(("\\[", "\\]"))
        } else {
            None
        };
        if let Some((open, close)) = delimiter {
            let after = &rest[open.len()..];
            if let Some(end) = after.match_indices(close).find_map(|(end, _)| {
                (after[..end]
                    .bytes()
                    .rev()
                    .take_while(|b| *b == b'\\')
                    .count()
                    % 2
                    == 0)
                    .then_some(end)
            }) {
                let expression = &after[..end];
                if end == 0
                    || (open == "$"
                        && (expression.starts_with(char::is_whitespace)
                            || expression.ends_with(char::is_whitespace)
                            || after[end + close.len()..]
                                .starts_with(|c: char| c.is_ascii_digit())))
                {
                    at += open.len();
                    slashes = 0;
                    continue;
                }
                if !ascii() {
                    if let Some(rendered) = crate::markdown_math::render_math(expression) {
                        spans.extend(preview_emphasis_only(&text[plain..at]));
                        spans.push(Span::styled(rendered, tint(Color::Magenta)));
                        at += open.len() + end + close.len();
                        plain = at;
                        slashes = 0;
                        continue;
                    }
                }
                // Unsupported math remains literal, including delimiters and scripts.
                spans.extend(preview_emphasis_only(&text[plain..at]));
                let length = open.len() + end + close.len();
                spans.push(Span::raw(rest[..length].to_owned()));
                at += length;
                plain = at;
                slashes = 0;
                continue;
            }
        }
        let ch = text[at..].chars().next().unwrap();
        slashes = if ch == '\\' { slashes + 1 } else { 0 };
        at += ch.len_utf8();
    }
    spans.extend(preview_emphasis_only(&text[plain..]));
    spans
}

fn preview_emphasis_only(text: &str) -> Vec<Span<'static>> {
    // A deliberately small prose renderer: no HTML execution, links or image fetches.
    let mut spans = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        let offset = text.len() - rest.len();
        let found = rest.char_indices().find(|(at, c)| {
            if *c != '_' {
                return *c == '`' || *c == '*';
            }
            let absolute = offset + at;
            let previous = text[..absolute].chars().next_back();
            let escaped = text[..absolute]
                .chars()
                .rev()
                .take_while(|c| *c == '\\')
                .count()
                % 2
                == 1;
            let length = if rest[*at..].starts_with("__") { 2 } else { 1 };
            !escaped
                && !previous.is_some_and(|c| c.is_alphanumeric() || c == '_')
                && rest[*at + length..]
                    .chars()
                    .next()
                    .is_some_and(|c| !c.is_whitespace() && c != '_')
        });
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
        } else if marker == '_' && rest[at..].starts_with("__") {
            "__"
        } else if marker == '_' {
            "_"
        } else {
            "`"
        };
        let after = &rest[at + token.len()..];
        let end = if marker == '_' {
            after.match_indices(token).find_map(|(end, _)| {
                let before = after[..end].chars().next_back();
                let next = after[end + token.len()..].chars().next();
                let escaped = after[..end]
                    .chars()
                    .rev()
                    .take_while(|c| *c == '\\')
                    .count()
                    % 2
                    == 1;
                (!escaped
                    && before.is_some_and(|c| !c.is_whitespace() && c != '_')
                    && !next.is_some_and(|c| c.is_alphanumeric() || c == '_'))
                .then_some(end)
            })
        } else {
            after.find(token)
        };
        if let Some(end) = end {
            let style = match token {
                "`" => tint(Color::Yellow),
                "**" | "__" => Style::default().add_modifier(Modifier::BOLD),
                _ => Style::default().add_modifier(Modifier::ITALIC),
            };
            spans.push(Span::styled(after[..end].to_owned(), style));
            rest = &after[end + token.len()..];
        } else if marker == '_' {
            spans.push(Span::raw(token.to_owned()));
            rest = after;
        } else {
            spans.push(Span::raw(rest[at..].to_owned()));
            break;
        }
    }
    spans
}
fn preview_lines(text: &str, kind: &str) -> Vec<Line<'static>> {
    preview_lines_tagged(text, kind, false)
}
fn preview_lines_tagged(text: &str, kind: &str, tagged: bool) -> Vec<Line<'static>> {
    let safe = safe_text(text);
    let code = if kind == "markdown" {
        crate::syntax_preview::fenced_lines(&safe)
    } else {
        Vec::new()
    };
    let fences = if kind == "markdown" {
        crate::syntax_preview::fenced_blocks(&safe)
    } else {
        Vec::new()
    };
    let openings: HashMap<_, _> = fences.iter().map(|f| (f.opening, f)).collect();
    let closings: BTreeSet<_> = fences.iter().filter_map(|f| f.closing).collect();
    safe.lines()
        .enumerate()
        .map(|(index, line)| {
            if kind == "markdown" {
                if let Some(Some(highlighted)) = code.get(index) {
                    let mut spans = vec![Span::styled(if ascii() { "| " } else { "│ " }, muted())];
                    spans.extend(highlighted.spans.clone());
                    return Line::from(spans);
                }
                if let Some(fence) = openings.get(&index) {
                    return Line::from(vec![
                        Span::styled(if ascii() { "+-- " } else { "┌─ " }, muted()),
                        Span::styled(
                            if fence.language.is_empty() {
                                "code".into()
                            } else {
                                fence.language.clone()
                            },
                            accent().add_modifier(Modifier::BOLD),
                        ),
                    ]);
                }
                if closings.contains(&index) {
                    return Line::from(Span::styled(if ascii() { "+--" } else { "└─" }, muted()));
                }
                let hashes = line.chars().take_while(|c| *c == '#').count();
                if (1..=6).contains(&hashes) && line.as_bytes().get(hashes) == Some(&b' ') {
                    let style = accent().add_modifier(Modifier::BOLD);
                    let mut spans = preview_inline_tagged(&line[hashes + 1..], tagged);
                    for span in &mut spans {
                        span.style = style.patch(span.style);
                    }
                    return Line::from(spans);
                }
                if ["- ", "* ", "+ "]
                    .iter()
                    .any(|p| line.trim_start().starts_with(p))
                {
                    let prefix = line.len() - line.trim_start().len();
                    let mut spans = vec![Span::styled(
                        format!("{}{} ", " ".repeat(prefix), if ascii() { "-" } else { "•" }),
                        accent(),
                    )];
                    spans.extend(preview_inline_tagged(&line.trim_start()[2..], tagged));
                    return Line::from(spans);
                }
                return Line::from(preview_inline_tagged(line, tagged));
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
            ["Keep session", "Stop session"]
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

fn markdown_display_lines(browser: &Browser, text: &str) -> Vec<Line<'static>> {
    let width = browser.preview_viewport.get().0;
    if let Some((revision, cached_width, source, lines)) = browser.markdown_cache.borrow().as_ref()
    {
        if *revision == browser.preview_revision && *cached_width == width && source == text {
            return lines.clone();
        }
    }
    let lines = markdown_layout_lines(browser, text, false);
    *browser.markdown_hit_lines.borrow_mut() = markdown_layout_lines(browser, text, true);
    *browser.markdown_cache.borrow_mut() = Some((
        browser.preview_revision,
        width,
        text.to_owned(),
        lines.clone(),
    ));
    lines
}
fn markdown_layout_lines(browser: &Browser, text: &str, tagged: bool) -> Vec<Line<'static>> {
    let width = browser.preview_viewport.get().0;
    let mut lines = if tagged {
        preview_lines_tagged(text, "markdown", true)
    } else {
        browser
            .preview_rich
            .as_ref()
            .and_then(|p| p.styled.clone())
            .unwrap_or_else(|| preview_lines(text, "markdown"))
    };
    let safe = safe_text(text);
    let source: Vec<_> = safe.lines().collect();
    let fences = crate::syntax_preview::fenced_blocks(&safe);
    let mut at = 0;
    let mut protected = BTreeSet::new();
    let mut fence = fences.iter().peekable();
    while at < source.len() {
        if let Some(block) = fence.peek().filter(|block| block.opening == at) {
            let end = block.closing.map(|i| i + 1).unwrap_or(source.len());
            protected.extend(at..end);
            at = end;
            fence.next();
            continue;
        }
        // A table body must never consume an upcoming fenced block.
        let table_end = fence
            .peek()
            .map(|block| block.opening)
            .unwrap_or(source.len());
        if let Some((consumed, rendered)) = crate::markdown_tables::render_inline(
            &source[..table_end],
            at,
            if width == 0 { 80 } else { usize::from(width) },
            ascii(),
            |text| preview_inline_tagged(text, tagged),
        ) {
            if consumed > 0 && rendered.len() == consumed && at + consumed <= lines.len() {
                protected.extend(at..at + consumed);
                lines.splice(at..at + consumed, rendered);
                at += consumed;
                continue;
            }
        }
        at += 1;
    }
    let mut row = 0;
    while row < source.len() {
        let opening = source[row].trim();
        let closing = match opening {
            "$$" => Some("$$"),
            "\\[" => Some("\\]"),
            _ => None,
        };
        if !protected.contains(&row) {
            if let Some(closing) = closing {
                if let Some(end) = (row + 1..source.len())
                    .take(128)
                    .find(|i| protected.contains(i) || source[*i].trim() == closing)
                {
                    if !protected.contains(&end) {
                        for index in row..=end {
                            lines[index] = Line::raw(source[index].to_owned());
                        }
                        protected.extend(row..=end);
                        let expression = source[row + 1..end].join(" ");
                        if let Some(rendered) = (!ascii())
                            .then(|| crate::markdown_math::render_math(&expression))
                            .flatten()
                        {
                            protected.extend(row..=end);
                            lines[row] = Line::from(Span::styled(
                                if ascii() { "Math" } else { "∷ Math" },
                                muted(),
                            ));
                            for line in &mut lines[row + 1..=end] {
                                *line = Line::raw("");
                            }
                            if row + 1 < end {
                                lines[row + 1] =
                                    Line::from(Span::styled(rendered, tint(Color::Magenta)));
                            } else {
                                lines[row] =
                                    Line::from(Span::styled(rendered, tint(Color::Magenta)));
                            }
                            row = end + 1;
                            continue;
                        }
                    }
                }
            }
        }
        row += 1;
    }
    let mut wrapped = Vec::new();
    let mut anchors = BTreeMap::new();
    let mut duplicates = HashMap::<String, usize>::new();
    let mut indent = 0;
    for (index, mut line) in lines.into_iter().enumerate() {
        let raw = source.get(index).copied().unwrap_or("");
        let header = raw.trim_start();
        let hashes = header.chars().take_while(|c| *c == '#').count();
        if !protected.contains(&index)
            && (1..=6).contains(&hashes)
            && header.as_bytes().get(hashes) == Some(&b' ')
        {
            let text = line
                .spans
                .iter()
                .map(|s| s.content.as_ref())
                .collect::<String>();
            let base = markdown_heading_slug(text.trim_end_matches('#').trim());
            let count = duplicates.entry(base.clone()).or_default();
            let slug = if *count == 0 {
                base.clone()
            } else {
                format!("{base}-{count}")
            };
            anchors.insert(slug, wrapped.len());
            *count += 1;
        }
        if protected.contains(&index)
            || raw.trim().is_empty()
            || raw.trim_start().starts_with(['#', '>'])
        {
            indent = 0;
            wrapped.push(line);
            continue;
        }
        if let Some(prefix) = list_prefix(raw) {
            indent = prefix;
        } else if indent > 0 {
            let spaces = raw.len() - raw.trim_start_matches(' ').len();
            if spaces < indent {
                line.spans.insert(0, Span::raw(" ".repeat(indent - spaces)));
            }
        }
        wrapped.extend(hanging_lines(
            &line,
            indent,
            if width == 0 { 80 } else { width },
        ));
    }
    if !tagged {
        *browser.markdown_anchor_rows.borrow_mut() = anchors;
    }
    wrapped
}
fn preview_display_lines(browser: &Browser, text: &str) -> Vec<Line<'static>> {
    let rich = browser.preview_rich.as_ref();
    if rich.is_some_and(|p| p.kind == "markdown") {
        let mut lines = markdown_display_lines(browser, text);
        if ascii() || std::env::var_os("NO_COLOR").is_some() {
            for line in &mut lines {
                line.style.fg = None;
                line.style.bg = None;
                for span in &mut line.spans {
                    span.style.fg = None;
                    span.style.bg = None;
                }
            }
        }
        lines
    } else if !ascii() && std::env::var_os("NO_COLOR").is_none() {
        rich.and_then(|p| p.styled.clone())
            .unwrap_or_else(|| preview_lines(text, rich.map(|p| p.kind.as_str()).unwrap_or("text")))
    } else {
        safe_text(text)
            .lines()
            .map(|s| Line::raw(s.to_owned()))
            .collect()
    }
}
fn preview_max_scroll(lines: &[Line<'static>], width: u16, height: u16) -> u16 {
    Paragraph::new(lines.to_vec())
        .wrap(Wrap { trim: false })
        .line_count(width.max(1))
        .saturating_sub(usize::from(height))
        .min(u16::MAX as usize) as u16
}
fn preview_matches(lines: &[Line<'static>], query: &str) -> Vec<(usize, Vec<usize>)> {
    let needle: Vec<_> = query.chars().flat_map(char::to_lowercase).collect();
    if needle.is_empty() {
        return Vec::new();
    }
    let mut result = Vec::new();
    for (row, line) in lines.iter().enumerate() {
        let text: Vec<_> = line
            .spans
            .iter()
            .flat_map(|s| s.content.chars())
            .enumerate()
            .flat_map(|(i, c)| c.to_lowercase().map(move |c| (c, i)))
            .collect();
        // Prefer every contiguous occurrence; otherwise use one fuzzy subsequence per line.
        let mut exact = false;
        for (start, window) in text.windows(needle.len()).enumerate() {
            if window.iter().map(|(c, _)| *c).eq(needle.iter().copied()) {
                result.push((
                    row,
                    text[start..start + needle.len()]
                        .iter()
                        .map(|(_, i)| *i)
                        .collect(),
                ));
                exact = true;
            }
        }
        if !exact {
            let mut positions = Vec::new();
            let mut next = 0;
            for (c, i) in text {
                if c == needle[next] {
                    positions.push(i);
                    next += 1;
                    if next == needle.len() {
                        break;
                    }
                }
            }
            if next == needle.len() {
                result.push((row, positions));
            }
        }
    }
    result
}
fn highlight_preview_matches(lines: &mut [Line<'static>], find: &PreviewFind) {
    let mut matches = find.matches.iter().enumerate().peekable();
    for (row, line) in lines.iter_mut().enumerate() {
        if !matches.peek().is_some_and(|(_, (r, _))| *r == row) {
            continue;
        }
        let mut flags = vec![0u8; line.spans.iter().map(|s| s.content.chars().count()).sum()];
        while matches.peek().is_some_and(|(_, (r, _))| *r == row) {
            let (which, (_, positions)) = matches.next().unwrap();
            for &position in positions {
                if let Some(flag) = flags.get_mut(position) {
                    *flag = (*flag).max(if which == find.selected { 2 } else { 1 });
                }
            }
        }
        let mut spans: Vec<Span<'static>> = Vec::new();
        let mut position = 0;
        for span in std::mem::take(&mut line.spans) {
            for c in span.content.chars() {
                let mut style = span.style;
                if flags[position] > 0 {
                    style = style.add_modifier(Modifier::UNDERLINED);
                    if !ascii() && std::env::var_os("NO_COLOR").is_none() {
                        style = style.fg(if flags[position] == 2 {
                            Color::Cyan
                        } else {
                            Color::Yellow
                        });
                    }
                    if flags[position] == 2 {
                        style = style.add_modifier(Modifier::BOLD);
                    }
                }
                if let Some(last) = spans.last_mut().filter(|last| last.style == style) {
                    last.content.to_mut().push(c);
                } else {
                    spans.push(Span::styled(c.to_string(), style));
                }
                position += 1;
            }
        }
        line.spans = spans;
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
    if browser
        .preview_viewport
        .replace((inner.width, inner.height))
        != (inner.width, inner.height)
        && !browser.preview_find.matches.is_empty()
    {
        browser.preview_find.reveal.set(true);
    }
    let mut lines = preview_display_lines(browser, text);
    let mut find = browser.preview_find.clone();
    find.matches = preview_matches(&lines, &find.query);
    find.selected = find.selected.min(find.matches.len().saturating_sub(1));
    let max = preview_max_scroll(&lines, inner.width, inner.height);
    let mut scroll = browser.preview_scroll.get().min(max);
    if browser.preview_find.reveal.replace(false) {
        if let Some((line, positions)) = find.matches.get(find.selected) {
            let before = Paragraph::new(lines[..*line].to_vec())
                .wrap(Wrap { trim: false })
                .line_count(inner.width.max(1));
            // Prefix line_count uses Ratatui's exact wrapping, including Unicode widths.
            let prefix: String = lines[*line]
                .spans
                .iter()
                .flat_map(|s| s.content.chars())
                .take(positions.first().copied().unwrap_or(0) + 1)
                .collect();
            let within = Paragraph::new(prefix)
                .wrap(Wrap { trim: false })
                .line_count(inner.width.max(1))
                .saturating_sub(1);
            scroll = (before + within).min(usize::from(max)) as u16;
        }
    }
    browser.preview_scroll.set(scroll);
    if max > 0 {
        let position = format!(" {}% ", u32::from(scroll) * 100 / u32::from(max));
        let width = (position.len() as u16).min(area.width.saturating_sub(2));
        if width > 0 && area.height > 0 {
            frame.render_widget(
                Paragraph::new(Span::styled(position, muted())),
                Rect::new(
                    area.right().saturating_sub(width + 1),
                    area.bottom() - 1,
                    width,
                    1,
                ),
            );
        }
    }
    browser.preview_link_cells.borrow_mut().clear();
    if rich.is_some_and(|p| p.kind == "markdown") && !markdown_preview_links(text).is_empty() {
        // Hit-test the unhighlighted presentation. Search underlines are not links.
        let mut buffer = ratatui::buffer::Buffer::empty(inner);
        ratatui::widgets::Widget::render(
            Paragraph::new(browser.markdown_hit_lines.borrow().clone())
                .wrap(Wrap { trim: false })
                .scroll((scroll, 0)),
            inner,
            &mut buffer,
        );
        let mut cells = browser.preview_link_cells.borrow_mut();
        for y in inner.y..inner.bottom() {
            for x in inner.x..inner.right() {
                let cell = &buffer[(x, y)];
                if cell.modifier.contains(Modifier::UNDERLINED) {
                    cells.push((x, y, cell.bg));
                }
            }
        }
    }
    highlight_preview_matches(&mut lines, &find);
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .scroll((scroll, 0)),
        inner,
    );
}

fn browser_entries(b: &Browser) -> Vec<Entry> {
    b.entries
        .iter()
        .filter(|e| {
            (b.show_hidden || !(e.hidden || e.name.starts_with('.')))
                && e.name.to_lowercase().contains(&b.filter.to_lowercase())
        })
        .cloned()
        .collect()
}
fn file_search_matches(rows: &[Entry], query: &str) -> Vec<(usize, Vec<usize>)> {
    if query.is_empty() {
        return Vec::new();
    }
    let lines: Vec<_> = rows
        .iter()
        .map(|e| Line::raw(safe_label(&e.name)))
        .collect();
    let mut matches: Vec<(usize, Vec<usize>)> = Vec::new();
    for (row, positions) in preview_matches(&lines, query) {
        if let Some((_, last_positions)) =
            matches.last_mut().filter(|(last_row, _)| *last_row == row)
        {
            last_positions.extend(positions);
        } else {
            matches.push((row, positions));
        }
    }
    matches
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
// Keep the selected context in traversal order; reserve the deepest two rows
// before showing ancestors so narrow/short panels never hide the selected item.
fn file_context_hierarchy(
    device: &str,
    path: &str,
    entry: Option<&Entry>,
    width: usize,
    height: usize,
) -> Vec<Line<'static>> {
    let safe = safe_label(path);
    let mut parts: Vec<&str> = safe.split('/').filter(|p| !p.is_empty()).collect();
    let folder = parts.pop().unwrap_or("/");
    let required = 2 + usize::from(entry.is_some());
    let parent_count = height.saturating_sub(required).min(parts.len());
    let skipped = parts.len().saturating_sub(parent_count);
    let mut lines = vec![Line::styled(
        fit_label(device, width),
        accent().add_modifier(Modifier::BOLD),
    )];
    let branch = if width < 16 {
        if ascii() {
            ">"
        } else {
            "›"
        }
    } else if ascii() {
        "+- "
    } else {
        "└─ "
    };
    if parent_count > 0 {
        for (depth, parent) in parts.iter().skip(skipped).enumerate() {
            let prefix = if depth == 0 && skipped > 0 {
                if ascii() {
                    ".../"
                } else {
                    "…/"
                }
            } else {
                ""
            };
            let indent = " ".repeat(depth.min(if width < 16 { 1 } else { 3 }));
            lines.push(Line::styled(
                fit_label(&format!("{indent}{prefix}{parent}/"), width),
                muted(),
            ));
        }
    }
    let indent = " ".repeat(if width < 16 { 0 } else { parent_count.min(3) });
    lines.push(Line::styled(
        fit_label(
            &format!(
                "{indent}{branch}{folder}{}",
                if folder == "/" { "" } else { "/" }
            ),
            width,
        ),
        accent().add_modifier(Modifier::BOLD | Modifier::REVERSED),
    ));
    if let Some(entry) = entry {
        lines.push(Line::styled(
            fit_label(
                &format!(
                    "{indent}{}{branch}{}{}",
                    if width < 16 { " " } else { "  " },
                    safe_label(&entry.name),
                    if entry.kind == "directory" { "/" } else { "" }
                ),
                width,
            ),
            Style::default().add_modifier(Modifier::BOLD),
        ));
    }
    lines
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
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(area);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                format!(
                    " {}  ",
                    b.container
                        .as_ref()
                        .map(|c| format!("Container · {}", safe_label(&c.name)))
                        .unwrap_or_else(|| label.into())
                ),
                accent().add_modifier(Modifier::BOLD),
            ),
            Span::styled(identity(device), muted()),
        ])),
        parts[0],
    );
    frame.render_widget(
        Paragraph::new(if b.container.is_some() {
            vec![Line::from(vec![
                Span::styled(" READ-ONLY ", accent().add_modifier(Modifier::BOLD)),
                Span::styled(
                    format!(
                        "hidden {}{}",
                        if b.show_hidden { "shown" } else { "off" },
                        if b.loading { " · loading" } else { "" }
                    ),
                    muted(),
                ),
            ])]
        } else {
            vec![Line::from(vec![
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
            ])]
        }),
        parts[1],
    );
    if let Some(preview) = &b.preview {
        render_preview(frame, parts[2], b, preview, focused);
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
                parts[2],
            );
        } else {
            let matches = file_search_matches(&rows, &b.search);
            let matched_rows: BTreeSet<_> = matches.iter().map(|(row, _)| *row).collect();
            let mut names: Vec<_> = rows
                .iter()
                .enumerate()
                .map(|(index, e)| {
                    let style = if index == b.selected && focused {
                        accent().add_modifier(Modifier::BOLD)
                    } else if index == b.selected {
                        Style::default().add_modifier(Modifier::BOLD)
                    } else if matched_rows.contains(&index) {
                        tint(Color::Yellow).add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
                    } else if e.kind == "directory" {
                        tint(Color::Blue).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default()
                    };
                    Line::from(Span::styled(
                        if b.search.is_empty() {
                            compact_path(&e.name, area.width.saturating_sub(18) as usize)
                        } else {
                            safe_label(&e.name)
                        },
                        style,
                    ))
                })
                .collect();
            let selected_match = matches
                .iter()
                .position(|(i, _)| *i == b.selected)
                .unwrap_or(usize::MAX);
            highlight_preview_matches(
                &mut names,
                &PreviewFind {
                    query: b.search.clone(),
                    matches,
                    selected: selected_match,
                    reveal: std::cell::Cell::new(false),
                },
            );
            let table_rows = rows
                .iter()
                .enumerate()
                .map(|(index, e)| {
                    let clip = clipboard
                        .filter(|c| c.container == b.container)
                        .filter(|c| {
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
                        accent().add_modifier(Modifier::BOLD)
                    } else if selected {
                        Style::default().add_modifier(Modifier::BOLD)
                    } else if e.kind == "directory" {
                        tint(Color::Blue).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default()
                    };
                    let cursor = if selected {
                        if ascii() {
                            "> "
                        } else {
                            "▸ "
                        }
                    } else {
                        "  "
                    };
                    Row::new(vec![
                        Cell::from(marker).style(style.add_modifier(Modifier::BOLD)),
                        Cell::from(match e.kind.as_str() {
                            "directory" => "/",
                            "symlink" => "@",
                            _ => " ",
                        })
                        .style(accent()),
                        // Keep the cursor attached to its filename instead of a remote rail.
                        // ANSI foreground only: preserve light/dark defaults and transparency.
                        Cell::from(Line::from(
                            vec![Span::styled(
                                cursor,
                                if focused { name_style } else { muted() },
                            )]
                            .into_iter()
                            .chain(names[index].spans.clone())
                            .collect::<Vec<_>>(),
                        )),
                        Cell::from(if e.kind == "file" {
                            human_size(e.size)
                        } else {
                            String::new()
                        })
                        .style(if selected && focused {
                            Style::default()
                        } else {
                            muted()
                        }),
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
                .row_highlight_style(Style::default()),
                parts[2],
                &mut state,
            );
        }
    }
    let bottom = if b.preview.is_some() {
        " Preview · j/k scroll · Escape back".into()
    } else if label.starts_with("Destination") && clipboard.is_some() {
        " Enter transfer here · h/l folders".into()
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
    } else if !b.search.is_empty() {
        let matches = file_search_matches(&browser_entries(b), &b.search);
        let current = matches
            .iter()
            .position(|(i, _)| *i == b.selected)
            .map(|i| i + 1)
            .unwrap_or(0);
        format!(
            " / {} · {}/{} · n/N matches{}",
            safe_label(&b.search),
            current,
            matches.len(),
            if b.filter.is_empty() {
                String::new()
            } else {
                format!(" · f {}", safe_label(&b.filter))
            }
        )
    } else if !b.filter.is_empty() {
        format!(" f Filter: {}", safe_label(&b.filter))
    } else {
        if b.container.is_some() {
            " Read-only · . hidden · f filter · n shell".into()
        } else {
            " Space select · v range · . hidden".into()
        }
    };
    frame.render_widget(
        Paragraph::new(bottom).style(if clipboard.is_some() {
            tint(Color::Yellow)
        } else {
            muted()
        }),
        parts[3],
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
fn transfer_spinner(frame: usize, plain: bool) -> &'static str {
    let frames: &[&str] = if plain {
        &["|", "/", "-", "\\"]
    } else {
        &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]
    };
    frames[frame % frames.len()]
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
fn normalized_mac(value: &str) -> Option<String> {
    let bytes: Vec<_> = value.split(':').collect();
    if bytes.len() != 6
        || bytes
            .iter()
            .any(|b| b.len() != 2 || !b.bytes().all(|c| c.is_ascii_hexdigit()))
    {
        return None;
    }
    let normalized = value.to_ascii_lowercase();
    if normalized == "00:00:00:00:00:00" || normalized == "ff:ff:ff:ff:ff:ff" {
        return None;
    }
    Some(normalized)
}
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
    let candidates = app.network_rows();
    let detail = candidates
        .get(app.network_selected)
        .map(|row| app.network_detail(row))
        .unwrap_or_default();
    // Use the installed Ratatui wrapper's count so word wrapping and Unicode match scrolling.
    let detail_lines = Paragraph::new(detail.as_str())
        .wrap(Wrap { trim: false })
        .line_count(inner.width);
    let detail_height = (detail_lines as u16)
        .max(7)
        .min(inner.height.saturating_sub(7));
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(3),
            Constraint::Length(detail_height),
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
        format!("{} · Internet {internet}\nInterfaces: {interfaces}\n{freshness} · automatic SSH-port checks", d.map(|d| identity(&app.devices[d])).unwrap_or_else(|| "unknown".into()))
    }).unwrap_or_else(|| if app.network_loading { "Reading network evidence…".into() } else { "No network observation · select device and refresh".into() });
    frame.render_widget(
        Paragraph::new(summary).wrap(Wrap { trim: false }),
        sections[0],
    );
    if candidates.is_empty() {
        frame.render_widget(
            Paragraph::new("No enrolled devices or observed LAN neighbors"),
            sections[1],
        );
        return;
    }
    let compact = sections[1].width < 45;
    let rows = candidates.iter().map(|row| {
        let depth = row["_depth"].as_u64().unwrap_or(0) as usize;
        let branch = row["_branch"] == true;
        let expanded = app
            .network_expanded
            .contains(row["_key"].as_str().unwrap_or(""));
        let marker = if branch {
            if expanded {
                "v"
            } else {
                ">"
            }
        } else {
            " "
        };
        let state = if let Some(d) = row["_peer"].as_u64() {
            let status = app.peer_status(d as usize);
            if status.contains("checking") {
                "~"
            } else if status.contains("reached") {
                "+"
            } else if status.contains("unavailable") {
                "!"
            } else {
                "?"
            }
        } else {
            match neighbor_ssh(row) {
                "port open" => "+",
                "port closed" => "!",
                _ => "?",
            }
        };
        let label = if row["_peer"].is_u64() {
            safe_label(row["_label"].as_str().unwrap_or("unknown"))
        } else if let Some(d) = row["_known_peer"].as_u64() {
            format!(
                "{} · {}",
                safe_label(&app.devices[d as usize].name),
                safe_label(row["address"].as_str().unwrap_or("unknown"))
            )
        } else if let Some(name) = row["hostname"].as_str() {
            format!(
                "{} · {}",
                safe_label(name),
                safe_label(row["address"].as_str().unwrap_or("unknown"))
            )
        } else {
            safe_label(row["address"].as_str().unwrap_or("unknown"))
        };
        let label = format!("{}{marker} {state} {label}", "  ".repeat(depth));
        if compact {
            return Row::new(vec![Cell::from(label)]);
        }
        let route = if let Some(d) = row["_peer"].as_u64() {
            identity(&app.devices[d as usize])
        } else {
            safe_label(row["interface"].as_str().unwrap_or(""))
        };
        Row::new(vec![Cell::from(label), Cell::from(route)])
    });
    let table = Table::new(
        rows,
        if compact {
            vec![Constraint::Min(1)]
        } else {
            vec![Constraint::Percentage(70), Constraint::Percentage(30)]
        },
    )
    .header(
        Row::new(if compact {
            vec!["Device / LAN neighbor"]
        } else {
            vec!["Device / LAN neighbor", "Execution / link"]
        })
        .style(muted()),
    )
    .row_highlight_style(selected_style());
    let mut state = TableState::default().with_selected(Some(app.network_selected));
    frame.render_stateful_widget(table, sections[1], &mut state);
    if !detail.is_empty() {
        frame.render_widget(
            Paragraph::new(detail).wrap(Wrap { trim: false }).scroll((
                app.network_detail_scroll
                    .min((detail_lines as u16).saturating_sub(detail_height)),
                0,
            )),
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
            if let Some(addresses) = interface["addr_info"].as_array() {
                for address in addresses.iter().take(8) {
                    if let Some(ip) = address["local"].as_str() {
                        lines.push(format!(
                            "    {}/{}",
                            safe_label(ip),
                            address["prefixlen"]
                                .as_u64()
                                .map(|v| v.to_string())
                                .unwrap_or_else(|| "?".into())
                        ));
                    }
                }
            }
        }
    }
    if let Some(routes) = value["routes"]
        .as_array()
        .or_else(|| value["routes"]["data"].as_array())
    {
        lines.push("\nRoutes (observed table)".into());
        for route in routes.iter().take(16) {
            lines.push(format!(
                "  {} via {} · {}{}",
                safe_label(route["dst"].as_str().unwrap_or("default")),
                safe_label(route["gateway"].as_str().unwrap_or("direct")),
                safe_label(route["dev"].as_str().unwrap_or("unknown")),
                route["metric"]
                    .as_u64()
                    .map(|m| format!(" · metric {m}"))
                    .unwrap_or_default()
            ));
        }
        if routes.len() > 16 {
            lines.push(format!("  +{} more routes", routes.len() - 16));
        }
        lines.push("Policy routing / forwarded egress: unverified".into());
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
    let mut show_secret = false;
    let mut toggle_hit = None;
    loop {
        if started.elapsed() > Duration::from_secs(240) {
            return Ok(Answer::Cancel);
        }
        screen.terminal.draw(|f| {
            let area = popup(
                f.area(),
                if trust { 76 } else { 64 },
                if trust { 13 } else { 7 },
            );
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
                let prompt_area = Rect { height: 1, ..inner };
                f.render_widget(
                    Paragraph::new(safe_label(
                        prompt.text.lines().next().unwrap_or("SSH authentication"),
                    )),
                    prompt_area,
                );
                let field = Rect {
                    y: inner.y + 2,
                    height: 1,
                    ..inner
                };
                let shown = if show_secret {
                    safe_label(&secret.0)
                } else {
                    (if ascii() { "*" } else { "•" }).repeat(
                        secret
                            .0
                            .chars()
                            .count()
                            .min(inner.width.saturating_sub(2) as usize),
                    )
                };
                let width = Span::raw(shown.as_str()).width() as u16;
                f.render_widget(
                    Paragraph::new(Line::from(vec![
                        Span::raw("> "),
                        Span::styled(shown, Style::default().fg(Color::Cyan)),
                    ])),
                    field,
                );
                f.set_cursor_position((
                    field.x + (width + 2).min(field.width.saturating_sub(1)),
                    field.y,
                ));
                let footer = Rect {
                    y: inner.y + inner.height.saturating_sub(1),
                    height: 1,
                    ..inner
                };
                let controls = Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([Constraint::Min(1), Constraint::Length(9)])
                    .split(footer);
                f.render_widget(
                    Paragraph::new("Enter OK · Esc Back").style(muted()),
                    controls[0],
                );
                f.render_widget(
                    Paragraph::new(if show_secret { "F2 Hide" } else { "F2 Show" }).style(accent()),
                    controls[1],
                );
                toggle_hit = Some(controls[1]);
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
                        KeyCode::F(2) => show_secret = !show_secret,
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
            Event::Mouse(mouse)
                if !trust
                    && mouse.kind == MouseEventKind::Down(crossterm::event::MouseButton::Left) =>
            {
                if toggle_hit
                    .is_some_and(|rect: Rect| rect.contains((mouse.column, mouse.row).into()))
                {
                    show_secret = !show_secret;
                }
            }
            _ => {}
        }
    }
}

// A physical detent can arrive as identical wheel reports a few microseconds
// apart (verified by a physical input trace). Normalize only that short burst;
// direction, coordinates and modifiers distinguish separate input contexts.
const WHEEL_BURST: Duration = Duration::from_millis(2);
#[derive(Default)]
struct WheelReports {
    last: Option<(MouseEvent, Instant)>,
}
fn wheel_report(input: &Event) -> Option<MouseEvent> {
    match input {
        Event::Mouse(mouse)
            if matches!(
                mouse.kind,
                MouseEventKind::ScrollUp | MouseEventKind::ScrollDown
            ) =>
        {
            Some(*mouse)
        }
        _ => None,
    }
}
impl WheelReports {
    fn accepts(&mut self, input: &Event, observed: Instant) -> bool {
        let Some(mouse) = wheel_report(input) else {
            self.last = None;
            return true;
        };
        if self.last.is_some_and(|(previous, at)| {
            previous == mouse && observed.saturating_duration_since(at) < WHEEL_BURST
        }) {
            return false;
        }
        self.last = Some((mouse, observed));
        true
    }
}
// Timestamp a bounded wheel burst before rendering/backend replies can separate
// duplicate reports. Stop after the first non-wheel event: never pre-read input
// intended for a native terminal that an Enter key is about to open.
fn read_input_batch() -> Result<Vec<(Event, Instant)>> {
    let first = event::read()?;
    let wheel = wheel_report(&first).is_some();
    let mut batch = vec![(first, Instant::now())];
    if wheel {
        let deadline = Instant::now() + WHEEL_BURST;
        for _ in 0..63 {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() || !event::poll(remaining)? {
                break;
            }
            let input = event::read()?;
            let wheel = wheel_report(&input).is_some();
            batch.push((input, Instant::now()));
            if !wheel {
                break;
            }
        }
    }
    Ok(batch)
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
    fn mouse_capture(&mut self, enabled: bool) -> Result<()> {
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
            let mouse = self.mouse_capture(false);
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
        Ok(CheckOutcome::Ready(plan)) => {
            format!("v{} ready; waiting for idle", safe_label(&plan.version))
        }
        Err(_) => "update unavailable; version kept".into(),
    })
}

fn update_notice_kind(result: &Result<crate::update::CheckOutcome>) -> NoticeKind {
    use crate::update::CheckOutcome;
    match result {
        Ok(CheckOutcome::Current | CheckOutcome::Ready(_)) => NoticeKind::Success,
        Ok(CheckOutcome::Offline | CheckOutcome::Unavailable(_) | CheckOutcome::Skipped) => {
            NoticeKind::Warning
        }
        Err(_) => NoticeKind::Error,
    }
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
                    match &task.op {
                        Operation::ContainerFiles { operation, .. } => operation.as_ref(),
                        other => other,
                    },
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
        || notifications::visible(&app.notice)
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
    let inner = Block::default().borders(Borders::ALL).inner(workspace[0]);
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
            app.set_notice(safe_text(&format!(
                "Update restored workspace defaults: {error:#}"
            )));
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
    let mut wheel_reports = WheelReports::default();
    let mut last_refresh = Instant::now();
    let mut last_jobs = Instant::now();
    let mut last_transfer_frame = Instant::now();
    let mut notice_was_animating = false;
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
                app.set_notice(version_notice("checking verified releases..."));
                dirty = true;
            } else if matches!(update_phase, UpdatePhase::Checking) {
                update_report_requested = true;
                app.set_notice(version_notice("checking verified releases..."));
                dirty = true;
            }
        }
        while let Ok(event) = update_rx.try_recv() {
            match event {
                UpdateEvent::Checked(force, result) => {
                    let force = force || std::mem::take(&mut update_report_requested);
                    update_phase = match result {
                        Ok(crate::update::CheckOutcome::Ready(plan)) => {
                            app.set_notice_as(
                                NoticeKind::Success,
                                version_notice(&format!(
                                    "v{} ready; waiting for idle",
                                    safe_label(&plan.version)
                                )),
                            );
                            dirty = true;
                            UpdatePhase::Ready(plan)
                        }
                        other => {
                            if force {
                                app.set_notice_as(
                                    update_notice_kind(&other),
                                    update_check_notice(&other),
                                );
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
                            app.set_notice_as(
                                NoticeKind::Error,
                                version_notice("update could not install; version kept"),
                            );
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
                    app.set_notice(version_notice("installing verified update..."));
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
                        app.set_notice_as(
                            NoticeKind::Warning,
                            safe_text(&format!("Update installed; reopen cx to use it: {error}")),
                        );
                        dirty = true;
                        update_phase = UpdatePhase::Idle;
                    }
                    Err(_) => {
                        app.set_notice_as(NoticeKind::Warning, "Update installed · workspace too large to restore; reopen cx when convenient".into());
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
            app.set_notice(format!("Connecting to {}", safe_label(&target)));
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
                    app.set_notice_as(
                        NoticeKind::Success,
                        format!("Added {}", safe_label(&target)),
                    );
                    app.refresh_work();
                }
                Err(e) => app.set_notice_as(
                    NoticeKind::Error,
                    safe_text(&format!("Enrollment failed: {e:#}")),
                ),
            }
            dirty = true;
        }
        if let Some(device) = app.pending_terminal.take() {
            cleanup_native_preview(&mut screen, &mut native_preview)?;
            screen.suspend()?;
            let result = sessions::login_terminal(&device);
            screen.resume()?;
            last_interaction = Instant::now();
            app.set_notice_as(
                if result.is_err() {
                    NoticeKind::Error
                } else {
                    NoticeKind::Info
                },
                match result {
                    Ok(()) => format!("Returned from {}", identity(&device)),
                    Err(e) => safe_text(&format!("Terminal failed: {e:#}")),
                },
            );
            dirty = true;
        }
        if let Some((device, command)) = app.pending_command.take() {
            cleanup_native_preview(&mut screen, &mut native_preview)?;
            screen.suspend()?;
            let result = sessions::run_command(&device, &command);
            screen.resume()?;
            last_interaction = Instant::now();
            app.set_notice_as(
                if result.is_err() {
                    NoticeKind::Error
                } else {
                    NoticeKind::Info
                },
                match result {
                    Ok(()) => format!("Returned from {}", identity(&device)),
                    Err(e) => safe_text(&format!("Command failed: {e:#}")),
                },
            );
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
            app.set_notice_as(
                if result.is_err() {
                    NoticeKind::Error
                } else {
                    NoticeKind::Info
                },
                match result {
                    Ok(()) => format!(
                        "Returned from {} · session remains on execution host",
                        identity(&app.devices[d])
                    ),
                    Err(e) => safe_text(&format!("Attachment failed: {e:#}")),
                },
            );
            app.refresh_work();
            dirty = true;
        }
        dirty |= app.expire_notice(Instant::now());
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
        // Animate only while work is pending; idle keeps its existing bounded redraw.
        if !app.active_transfer_keys().is_empty()
            && last_transfer_frame.elapsed() >= Duration::from_millis(160)
        {
            app.transfer_frame = app.transfer_frame.wrapping_add(1);
            last_transfer_frame = Instant::now();
            dirty = true;
        }
        let notice_is_animating =
            notifications::animating(app.notice_started, app.notice_deadline, Instant::now());
        dirty |= notice_is_animating || notice_was_animating;
        notice_was_animating = notice_is_animating;
        // Capture wheel reports throughout cx so the emulator does not replace
        // them with accelerated arrow-key bursts. suspend() releases this before
        // handing the terminal to SSH/tmux/native commands.
        screen.mouse_capture(true)?;
        if dirty {
            screen
                .terminal
                .draw(|frame| render_with_native(frame, &app, Some(&mut native_preview)))?;
            dirty = false;
        }
        if event::poll(notifications::poll_interval(
            app.notice_started,
            app.notice_deadline,
            Instant::now(),
        ))? {
            for (input, observed) in read_input_batch()? {
                if !wheel_reports.accepts(&input, observed) {
                    continue;
                }
                match input {
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
                        let handled = app.mouse(mouse, Rect::new(0, 0, size.width, size.height));
                        if handled {
                            last_interaction = Instant::now();
                            dirty = true;
                        }
                    }
                    _ => {}
                }
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
            if let Some(Dialog::Provider(device, _) | Dialog::SessionChooser(device, _)) =
                app.dialog
            {
                app.check_providers(device);
            }
            if app.view == View::Work {
                app.refresh_work();
            } else if app.view == View::Network || app.view == View::Containers {
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
mod tests;

fn browser_scope_key(scope: Option<&ContainerScope>) -> String {
    scope
        .map(|s| format!("{}:{}:{}:{}", s.engine, s.id, s.started_at, s.user))
        .unwrap_or_default()
}

fn file_endpoint_label(device: &Device, scope: &Option<ContainerScope>) -> String {
    let host = identity(device);
    scope
        .as_ref()
        .map(|c| format!("{host} / {}", safe_label(&c.name)))
        .unwrap_or(host)
}
