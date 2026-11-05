//! Native workspace. Remote work runs off the input/render thread; attachment owns the terminal.
use crate::{
    model::{ContainerScope, CreateSession, Device, Operation, RunCommand, Session},
    sessions, store, transport,
};
mod container_agents;
mod filters;
mod rendering;
mod session_preview;
mod workspaces;
#[cfg(test)]
use rendering::render;
use rendering::render_with_native;
mod menus;
mod navigation;
mod notifications;
mod panels;
mod theme;
use filters::score as fuzzy_score;
use menus::{container_action_labels, render_session_chooser};
use navigation::{Motion as Navigation, Target};
use notifications::Kind as NoticeKind;
use theme::*;
mod preview;
use preview::*;
mod files_view;
use files_view::*;
mod network_view;
use network_view::*;
mod runtime;
pub(crate) use runtime::sweep_stale_restarts;
use runtime::*;
mod apply;
mod dialogs;
use dialogs::*;
mod input;
pub use runtime::{run, run_restored};

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
        let app = Self {
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
        };
        app
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
    device_id: Option<String>,
    execution_target: Option<Option<String>>,
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
    PinSession,
    FolderSessions,
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
    Reset,
    Quit,
}
const ACTIONS: &[(Action, &str)] = &[
    (Action::PinSession, "Pin / unpin selected session · p"),
    (Action::FolderSessions, "Sessions in selected folder · s"),
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
    (
        Action::Reset,
        "Reset managed tmux / agent sessions · confirm",
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
    ContainerPermissions(usize, ContainerScope, String, bool),
    DestinationScope(usize, Vec<crate::containers::Container>),
    ContainerConfirm(usize, crate::containers::Container, String),
    Device(ChooseDevice),
    Provider(usize, Option<String>),
    Permissions(usize, String, String, bool, bool),
    SessionPicker(usize, String, Option<ContainerScope>, String, Vec<Session>),
    PinnedEnded(usize, Session),
    Jobs,
    AgentTasks(String, Session),
    Links(Vec<crate::markdown_links::Link>),
    Delete(usize, Vec<Entry>),
    StopShell(usize, Session),
    PendingExit(usize),
    Neighbor(usize, Value),
    Peer(usize),
    RemoveDevice(usize),
    EditDevice(usize),
    Reset,
}

/// Live view of a command running in a managed pane, shown in a floating panel.
struct CommandPanel {
    device: usize,
    id: String,
    cols: u16,
    rows: u16,
    /// Raw styled lines (SGR intact) from the last capture.
    lines: Vec<String>,
    alive: bool,
    status: Option<i32>,
    /// A capture request is outstanding.
    inflight: bool,
    last_poll: std::time::Instant,
    /// The command, for the panel title.
    label: String,
}
struct App {
    session_preview: session_preview::Snapshot,
    devices: Vec<Device>,
    policies: Vec<store::DevicePolicy>,
    device: usize,
    focus: Focus,
    view: View,
    work: Vec<Cached>,
    pins: Vec<store::SessionPin>,
    selected: usize,
    side_selected: usize,
    search: String,
    input: Option<Input>,
    text: String,
    palette_selected: usize,
    navigation: navigation::State,
    device_filter: String,
    job_filter: String,
    /// The committed filter text captured when a filter edit begins, so Esc can
    /// cancel back to it (empty when there was none) instead of leaving the
    /// half-typed filter applied. Taken on cancel; overwritten on each new edit.
    filter_restore: Option<String>,
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
    container_profiles: HashMap<(usize, String), container_agents::Profiles>,
    container_profile_loading: BTreeSet<(usize, String)>,
    containers: HashMap<usize, Vec<crate::containers::Container>>,
    container_errors: HashMap<usize, String>,
    containers_loading: BTreeSet<usize>,
    rebuilding: BTreeSet<(usize, String)>,
    container_selected: usize,
    container_collapsed: BTreeSet<usize>,
    file_locations: HashMap<usize, String>,
    generation: u64,
    creating: bool,
    device_generation_floor: u64,
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
    pending_edit: Option<usize>,
    clipboard: Option<Clipboard>,
    rename_target: Option<(usize, Entry)>,
    rename_cursor: usize,
    file_queue: VecDeque<(usize, Operation)>,
    file_busy: bool,
    file_errors: Vec<String>,
    transfer_drawer: bool,
    jobs: HashMap<usize, Value>,
    pending_attach: Option<(usize, Session, bool)>,
    attach_check: Option<(usize, Session, bool, u64)>,
    command_target: Option<(usize, String)>,
    pending_command: Option<(Device, RunCommand)>,
    pending_command_panel: Option<(usize, RunCommand)>,
    command_panel: Option<CommandPanel>,
    /// A command panel to attach full-screen on the next loop turn.
    pending_panel_expand: Option<(usize, Session)>,
    command_panel_label: Option<String>,
    pending_ptrace: Option<usize>,
    pending_adoption: Option<(Device, Session, Instant)>,
    pending_terminal: Option<Device>,
    pending_add: Option<String>,
    quit: bool,
    force_update: bool,
    pending_requests: std::cell::Cell<usize>,
    panels: std::cell::RefCell<Vec<(Focus, bool, Rect)>>,
    session_hits: std::cell::RefCell<Vec<(Rect, usize)>>,
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
        let policies = devices
            .iter()
            .map(|device| store::policy(device).unwrap_or_default())
            .collect();
        let work = devices
            .iter()
            .map(|_| Cached {
                sessions: vec![],
                loading: false,
                error: None,
                fetched: 0,
            })
            .collect();
        let mut app = Self {
            session_preview: session_preview::Snapshot::default(),
            devices,
            policies,
            device: 0,
            focus: Focus::Workspace,
            view: View::Work,
            work,
            pins: Vec::new(),
            selected: 0,
            side_selected: 0,
            search: String::new(),
            input: None,
            text: String::new(),
            palette_selected: 0,
            device_filter: String::new(),
            job_filter: String::new(),
            filter_restore: None,
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
            container_profiles: HashMap::new(),
            container_profile_loading: BTreeSet::new(),
            containers: HashMap::new(),
            container_errors: HashMap::new(),
            containers_loading: BTreeSet::new(),
            rebuilding: BTreeSet::new(),
            container_selected: 0,
            container_collapsed: BTreeSet::new(),
            file_locations: HashMap::new(),
            generation: 0,
            creating: false,
            device_generation_floor: 0,
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
            pending_edit: None,
            clipboard: None,
            rename_target: None,
            rename_cursor: 0,
            file_queue: VecDeque::new(),
            file_busy: false,
            file_errors: Vec::new(),
            transfer_drawer: false,
            jobs: HashMap::new(),
            pending_attach: None,
            attach_check: None,
            command_target: None,
            pending_command: None,
            pending_command_panel: None,
            command_panel: None,
            pending_panel_expand: None,
            command_panel_label: None,
            pending_ptrace: None,
            pending_adoption: None,
            pending_terminal: None,
            pending_add: None,
            quit: false,
            force_update: false,
            pending_requests: std::cell::Cell::new(0),
            panels: std::cell::RefCell::new(Vec::new()),
            session_hits: std::cell::RefCell::new(Vec::new()),
            tx,
        };
        if let Ok(message) = std::env::var("CX_STARTUP_NOTICE") {
            app.set_notice_as(NoticeKind::Info, safe_text(&message));
        }
        app
    }
    fn device_policy(&self, device: usize) -> store::DevicePolicy {
        self.policies.get(device).copied().unwrap_or_default()
    }
    fn background_checks_enabled(&self, device: usize) -> bool {
        device < self.devices.len()
    }
    fn toggle_access(&mut self) {
        let Some(device) = self.device.checked_sub(1) else {
            self.set_notice_as(
                NoticeKind::Warning,
                "Select one remote device before changing its access posture".into(),
            );
            return;
        };
        let current = self.device_policy(device);
        let next = match current.access {
            store::AccessMode::Core => store::AccessMode::Directed,
            store::AccessMode::Directed => store::AccessMode::Core,
        };
        match store::set_access(&self.devices[device], next) {
            Ok(policy) => {
                if let Some(slot) = self.policies.get_mut(device) {
                    *slot = policy;
                }
                let message = match next {
                    store::AccessMode::Core => {
                        "Core posture saved; connections will be configured automatically"
                    }
                    store::AccessMode::Directed => "Viewer-to-device access only",
                };
                self.set_notice_as(
                    NoticeKind::Success,
                    format!("{message} · {}", identity(&self.devices[device])),
                );
            }
            Err(error) => self.set_notice_as(
                NoticeKind::Error,
                safe_text(&format!("Access posture unchanged: {error:#}")),
            ),
        }
    }
    fn set_availability(&mut self, availability: store::Availability) {
        let Some(device) = self.device.checked_sub(1) else {
            self.set_notice_as(
                NoticeKind::Warning,
                "The local device is always available".into(),
            );
            return;
        };
        match store::set_availability(&self.devices[device], availability) {
            Ok(policy) => {
                if let Some(slot) = self.policies.get_mut(device) {
                    *slot = policy;
                }
                let message = if availability == store::Availability::UsuallyDown {
                    "Device marked down · background checks paused"
                } else {
                    "Device marked up · background checks enabled"
                };
                self.set_notice_as(
                    NoticeKind::Success,
                    format!("{message} · {}", identity(&self.devices[device])),
                );
            }
            Err(error) => self.set_notice_as(
                NoticeKind::Error,
                safe_text(&format!("Availability unchanged: {error:#}")),
            ),
        }
    }
    // These collections use device vector indices. Clear them as one unit
    // whenever an endpoint changes or removing a device shifts those indices.
    fn invalidate_device_views(&mut self) {
        self.browser = None;
        self.other_browser = None;
        self.destination_active = false;
        self.browser_cache.clear();
        self.container_locations.clear();
        self.containers.clear();
        self.container_errors.clear();
        self.containers_loading.clear();
        self.rebuilding.clear();
        self.container_collapsed.clear();
        self.container_selected = 0;
        self.file_locations.clear();
        self.providers.clear();
        self.provider_loading.clear();
        self.network.clear();
        self.peer_evidence.clear();
        self.peer_checks.clear();
        self.peer_inflight.clear();
        self.network_inflight.clear();
        self.network_candidates_inflight.clear();
        self.network_refresh_queue.clear();
        self.network_jump_routes.clear();
        self.network_loading = false;
        self.neighbor_probes.clear();
        self.neighbor_queue.clear();
        self.neighbor_evidence.clear();
        self.jobs.clear();
        self.pending_attach = None;
        self.attach_check = None;
        self.pending_adoption = None;
        self.container_profiles.clear();
        self.container_profile_loading.clear();
        self.pending_command = None;
        self.pending_terminal = None;
        self.clipboard = None;
        self.creating = false;
        self.dialog = None;
        self.command_target = None;
        self.rename_target = None;
        self.network_add_target = None;
        self.generation += 1;
        for work in &mut self.work {
            work.loading = false;
        }
    }

    fn replace_device_endpoint(&mut self, device: usize, updated: Device) {
        self.devices[device] = updated;
        self.invalidate_device_views();
        self.work[device] = Cached {
            sessions: vec![],
            loading: false,
            error: None,
            fetched: 0,
        };
        self.file_queue.retain(|(d, _)| *d != device);
    }
    fn remove_device_state(&mut self, index: usize) {
        self.devices.remove(index);
        self.policies.remove(index);
        self.work.remove(index);
        self.invalidate_device_views();
        self.file_queue.retain(|(d, _)| *d != index);
        for (d, _) in &mut self.file_queue {
            if *d > index {
                *d -= 1;
            }
        }
        self.device = self.device.min(self.devices.len());
        self.view = View::Work;
        self.focus = Focus::Devices;
    }

    fn reconcile_devices(&mut self, updated: Vec<Device>) -> bool {
        let selected = self
            .device
            .checked_sub(1)
            .and_then(|i| self.devices.get(i))
            .map(|d| d.id.clone());
        let mut changed = false;
        for i in (0..self.devices.len()).rev() {
            if !updated.iter().any(|d| d.id == self.devices[i].id) {
                self.remove_device_state(i);
                changed = true;
            }
        }
        for device in updated {
            let policy = store::policy(&device).unwrap_or_default();
            if let Some(i) = self.devices.iter().position(|d| d.id == device.id) {
                if self.devices[i].target != device.target {
                    self.replace_device_endpoint(i, device.clone());
                    changed = true;
                }
                changed |= self.devices[i].name != device.name || self.policies[i] != policy;
                self.devices[i] = device;
                self.policies[i] = policy;
            } else {
                self.devices.push(device);
                self.policies.push(policy);
                self.work.push(Cached {
                    sessions: vec![],
                    loading: false,
                    error: None,
                    fetched: 0,
                });
                changed = true;
            }
        }
        self.device = selected
            .and_then(|id| self.devices.iter().position(|d| d.id == id))
            .map_or(0, |i| i + 1);
        changed
    }

    fn remove_selected_device(&mut self) {
        if self.file_busy
            || !self.file_queue.is_empty()
            || self.creating
            || !self.rebuilding.is_empty()
        {
            self.set_notice_as(
                NoticeKind::Warning,
                "Finish submitting work before removing a device".into(),
            );
            return;
        }
        let Some(index) = self.device.checked_sub(1) else {
            self.set_notice_as(
                NoticeKind::Warning,
                "The local device cannot be removed".into(),
            );
            return;
        };
        let device = self.devices[index].clone();
        match store::remove_device(&device.id) {
            Ok(_) => {
                self.forget_device_index(index);
                self.set_notice_as(
                    NoticeKind::Success,
                    format!("Removed {} from this CX viewer", safe_label(&device.name)),
                );
            }
            Err(error) => self.set_notice_as(
                NoticeKind::Error,
                safe_text(&format!("Device was not removed: {error:#}")),
            ),
        }
    }
    fn forget_device_index(&mut self, index: usize) {
        // All navigation/action indexes belong to the old device vector. Rebuild
        // that state together; queued tasks keep their captured execution Device,
        // but their replies must never address the newly shifted indexes.
        let mut devices = self.devices.clone();
        devices.remove(index);
        let mut next = Self::new(devices, self.tx.clone());
        next.generation = self.generation + 1;
        next.device_generation_floor = next.generation;
        next.pending_requests.set(self.pending_requests.get());
        next.pins = std::mem::take(&mut self.pins);
        self.work.remove(index);
        next.work = std::mem::take(&mut self.work);
        for work in &mut next.work {
            work.loading = false;
        }
        next.submitted = std::mem::take(&mut self.submitted);
        next.submitted_clipboards = std::mem::take(&mut self.submitted_clipboards);
        next.watched_jobs = std::mem::take(&mut self.watched_jobs);
        next.device = self.device.min(next.devices.len());
        next.focus = Focus::Devices;
        *self = next;
    }
    fn focus_label(&self) -> &'static str {
        if self.help {
            return "Help";
        }
        if let Some(dialog) = &self.dialog {
            return match dialog {
                Dialog::SessionPicker(..) => "Sessions here",
                Dialog::PinnedEnded(..) => "Pinned work",
                Dialog::DevcontainerUp(..) => "Start devcontainer workspace",
                Dialog::ContainerActions(..) => "Container actions",
                Dialog::ContainerProvider(..) => "Container session",
                Dialog::ContainerPermissions(..) => "Container permissions",
                Dialog::DestinationScope(..) => "Transfer destination",
                Dialog::ContainerConfirm(..) => "Container confirmation",
                Dialog::Links(_) => "Links",
                Dialog::Device(_) => "Device picker",
                Dialog::SessionChooser(..) => "New session",
                Dialog::Provider(..) => "Provider",
                Dialog::Permissions(..) => "Session permissions",
                Dialog::Jobs => {
                    if self.dialog_detail_focus {
                        "Transfer details"
                    } else {
                        "Transfers"
                    }
                }
                Dialog::AgentTasks(..) => "Agent tasks",
                Dialog::Delete(..) => "Delete",
                Dialog::StopShell(..) => "Stop session",
                Dialog::PendingExit(_) => "Pending actions",
                Dialog::Neighbor(..) => "Neighbor actions",
                Dialog::Peer(_) => "Device actions",
                Dialog::RemoveDevice(_) => "Remove device",
                Dialog::EditDevice(_) => "Edit device",
                Dialog::Reset => "Reset managed sessions",
            };
        }
        if let Some(input) = self.input {
            return match input {
                Input::Search | Input::PreviewSearch => "Search",
                Input::Palette => "Actions",
                Input::Command => "Run command",
                Input::Add => {
                    if self.pending_edit.is_some() {
                        "Edit SSH address"
                    } else {
                        "Add by SSH address"
                    }
                }
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
            if self.background_checks_enabled(index) && !self.work[index].loading {
                self.work[index].loading = self.send(index, Operation::Sessions);
                if !self.work[index].loading {
                    self.set_notice_as(
                        NoticeKind::Warning,
                        "Refresh queue busy · retry shortly".into(),
                    );
                }
                // Discover outside-CX terminals for the UI when the device
                // supports adoption; `cx sessions` itself stays managed-only.
                if self
                    .providers
                    .get(&index)
                    .is_some_and(|(caps, _)| caps.iter().any(|c| c == "adopt-session-v1"))
                {
                    self.send(index, Operation::ExternalSessions);
                }
            }
        }
        for index in 0..self.devices.len() {
            if self.background_checks_enabled(index) {
                self.check_providers(index);
            }
        }
    }
    fn open_startup_directory(&mut self, directory: std::path::PathBuf) {
        if let Some(local) = self.devices.iter().position(|d| d.target.is_none()) {
            self.device = local + 1;
            self.open_host_browser(local, crate::files::encode_path(&directory));
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
        self.session_rows_for(&self.search)
    }
    fn session_rows_for(&self, search: &str) -> Vec<(usize, &Session)> {
        let mut seen = BTreeSet::new();
        let saved: Vec<_> = self
            .pins
            .iter()
            .filter_map(|p| {
                self.devices
                    .iter()
                    .position(|d| workspaces::device_matches(d, &p.device))
                    .map(|d| (d, &p.session))
            })
            .collect();
        let pinned: BTreeSet<_> = saved
            .iter()
            .map(|(d, s)| (*d, workspaces::runtime_key(s)))
            .collect();
        let live = self
            .work
            .iter()
            .enumerate()
            .flat_map(|(d, w)| w.sessions.iter().map(move |s| (d, s)));
        let mut rows: Vec<_> = live
            .chain(saved)
            .filter(|(d, _)| self.device == 0 || self.device == d + 1)
            .filter_map(|(d, s)| {
                let key = workspaces::runtime_key(s);
                let is_pinned = pinned.contains(&(d, key.clone()));
                if !seen.insert((d, key)) {
                    return None;
                }
                fuzzy_score(
                    search,
                    &format!(
                        "{} {} {} {} {} {} {}",
                        s.name,
                        s.directory,
                        s.host,
                        s.account,
                        s.provider,
                        menus::session_label(self, d, s),
                        s.container.as_ref().map(|c| c.name.as_str()).unwrap_or("")
                    ),
                )
                .map(|score| (is_pinned, score, d, s))
            })
            .collect();
        rows.sort_by(|(p, a, i, x), (q, b, j, y)| {
            q.cmp(p).then_with(|| b.cmp(a)).then_with(|| {
                (x.directory.as_str(), *i, x.id.as_str(), &x.socket).cmp(&(
                    y.directory.as_str(),
                    *j,
                    y.id.as_str(),
                    &y.socket,
                ))
            })
        });
        rows.into_iter().map(|(_, _, d, s)| (d, s)).collect()
    }
    fn reconcile_session_selection(&mut self, identity: Option<(usize, String)>) {
        if let Some((device, id)) = identity {
            self.selected = self
                .session_rows()
                .iter()
                .position(|(d, s)| *d == device && workspaces::runtime_key(s) == id)
                .unwrap_or_else(|| self.session_rows().len());
        } else {
            self.selected = self.selected.min(self.session_rows().len());
        }
        if let Some(Dialog::AgentTasks(device_id, session)) = &self.dialog {
            let updated = self
                .devices
                .iter()
                .position(|d| &d.id == device_id)
                .and_then(|d| self.work[d].sessions.iter().find(|s| s.id == session.id))
                .cloned();
            self.dialog = updated.map(|session| Dialog::AgentTasks(device_id.clone(), session));
            let count = match &self.dialog {
                Some(Dialog::AgentTasks(_, s)) => s.agent_tasks.len(),
                _ => 0,
            };
            self.dialog_selected = self.dialog_selected.min(count.saturating_sub(1));
        }
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
                if session.container.is_some() || session.directory.is_empty() {
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
    pub(super) fn host_session_choices(&self, device: usize) -> Vec<&'static menus::SessionChoice> {
        let available = self.provider_choices(device);
        menus::HOST
            .iter()
            .filter(|choice| available.contains(&choice.provider))
            .collect()
    }
    /// Attach to a tmux session, or move a terminal started outside CX into
    /// managed tmux first when the device has a verified relay.
    fn adopt_external_session(&mut self, device: usize, session: Session) {
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
    /// Route a key to the open command panel: Esc closes it, Ctrl+O expands it
    /// full-screen, everything else is delivered to the running command.
    fn command_panel_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if key.code == KeyCode::Esc {
            if let Some(panel) = self.command_panel.take() {
                self.send(panel.device, Operation::PaneClose { id: panel.id });
            }
            return;
        }
        if ctrl && key.code == KeyCode::Char('o') {
            if let Some(panel) = self.command_panel.as_ref() {
                if panel.alive {
                    self.pending_panel_expand = Some((panel.device, command_panel_session(panel)));
                }
            }
            return;
        }
        let alive = self.command_panel.as_ref().is_some_and(|p| p.alive);
        if !alive {
            // A finished command only accepts closing or expanding to scroll.
            return;
        }
        if let Some(bytes) = encode_key(key) {
            let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
            let (id, device) = self
                .command_panel
                .as_ref()
                .map(|p| (p.id.clone(), p.device))
                .unwrap();
            self.send(device, Operation::PaneInput { id, hex });
        }
    }
    fn check_providers(&mut self, device: usize) {
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
            Action::Observe | Action::PinSession => {
                self.view == View::Work && self.selected_session().is_some()
            }
            Action::FolderSessions => {
                self.view == View::Files
                    && self.browser.as_ref().is_some_and(|b| b.preview.is_none())
            }
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
            if self.browser_cache.len() >= 64 {
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
            if self.background_checks_enabled(d)
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
            resume: false,
            resume_pick: false,
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
            if !self.background_checks_enabled(d) {
                continue;
            }
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
                    if !self.background_checks_enabled(d) {
                        continue;
                    }
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
                    if self.background_checks_enabled(d)
                        && (self.device == 0 || self.device == d + 1)
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
        let rows = self.scoped_sessions(d, &directory, None, &provider);
        if !rows.is_empty() || container_agents::has_history(None, &provider) {
            self.dialog = Some(Dialog::SessionPicker(d, directory, None, provider, rows));
            self.dialog_selected = 0;
            self.refresh_sessions_on(d);
        } else {
            self.create_at(d, directory, provider);
        }
    }
    fn create_at(&mut self, d: usize, directory: String, provider: String) {
        if provider != "shell" {
            self.dialog = Some(Dialog::Permissions(d, directory, provider, false, false));
            self.dialog_selected = 0;
            return;
        }
        self.create_permission_session(d, directory, provider, false, false, false);
    }
    fn create_permission_session(
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
    fn key(&mut self, key: KeyEvent) {
        if key.kind == KeyEventKind::Release {
            return;
        }
        if key.code != KeyCode::Enter {
            self.pending_adoption = None;
        }
        if self.pane_focused() {
            self.session_pane_key(key);
            return;
        }
        // A command panel captures input (including Ctrl+C) while it is open.
        if self.command_panel.is_some() {
            self.command_panel_key(key);
            return;
        }
        if self.attach_check.take().is_some() || self.pending_attach.take().is_some() {
            self.pending_attach = None;
            self.set_notice("Attachment cancelled".into());
            if key.code == KeyCode::Esc {
                return;
            }
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
            self.input_key(key, mode);
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
                if !self.observed(device, &session) {
                    self.set_notice("This pinned runtime has ended; no process was stopped".into());
                    return;
                }
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
            KeyCode::Char('s') if self.view == View::Work && self.focus == Focus::Workspace => {
                if let Some((device, session)) = self.selected_session() {
                    self.dialog =
                        Some(Dialog::AgentTasks(self.devices[device].id.clone(), session));
                    self.dialog_selected = 0;
                    self.dialog_scroll = 0;
                    self.dialog_detail_focus = false;
                }
            }
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
            KeyCode::Char('b' | 'B') if self.focus == Focus::Devices => self.toggle_access(),
            KeyCode::Char('x' | 'X') if self.focus == Focus::Devices && self.device > 0 => {
                self.dialog_selected = 0;
                self.dialog = Some(Dialog::RemoveDevice(self.device - 1));
            }
            KeyCode::Char('e' | 'E') if self.focus == Focus::Devices && self.device > 0 => {
                self.dialog_selected = 0;
                self.dialog = Some(Dialog::EditDevice(self.device - 1));
            }
            KeyCode::Char('p') if self.view == View::Work && self.focus == Focus::Workspace => {
                self.toggle_pin()
            }
            KeyCode::Char('s') if self.view == View::Files && self.focus == Focus::Workspace => {
                self.sessions_here()
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
                                if !self.focus_session_pane() {
                                    self.open_session(d, s, false);
                                }
                            } else if empty_work_can_create(self) {
                                self.session_shortcut();
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
        if self.scroll_session_preview(mouse) {
            return true;
        }
        if self.pane_focused() {
            return true;
        }
        if self.view == View::Work
            && !self.help
            && self.dialog.is_none()
            && self.input.is_none()
            && matches!(
                mouse.kind,
                MouseEventKind::Moved | MouseEventKind::Down(event::MouseButton::Left)
            )
        {
            let target = self
                .session_hits
                .borrow()
                .iter()
                .find(|(rect, _)| {
                    rect.contains(ratatui::layout::Position::new(mouse.column, mouse.row))
                })
                .map(|(_, index)| *index);
            if let Some(index) = target {
                if self.selected != index {
                    self.attach_check = None;
                    self.pending_attach = None;
                    self.pending_adoption = None;
                    self.selected = index;
                }
                self.focus = Focus::Workspace;
                return true;
            }
        }
        if matches!(
            mouse.kind,
            MouseEventKind::Down(_) | MouseEventKind::ScrollUp | MouseEventKind::ScrollDown
        ) {
            self.attach_check = None;
            self.pending_attach = None;
        }
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
/// Terminals discovered outside any tmux server have no attachable tmux id.
fn outside_tmux(s: &Session) -> bool {
    s.external && s.id.starts_with("ext-")
}
/// Rows that came from the separate external-terminal discovery, not the
/// managed session list (which includes default-tmux `$N` rows).
fn is_discovered_external(s: &Session) -> bool {
    s.external && s.id.starts_with("ext-")
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
fn session_activity_current(cache: &Cached) -> bool {
    cache.error.is_none()
        && cache.fetched > 0
        && transport::now().saturating_sub(cache.fetched) <= 5
}
fn session_busy(session: &Session) -> bool {
    matches!(
        session.activity.as_deref(),
        Some("running" | "working" | "busy" | "output")
    )
}

#[cfg(test)]
mod tests;

fn browser_scope_key(scope: Option<&ContainerScope>) -> String {
    scope
        .map(|s| format!("{}:{}:{}:{}", s.engine, s.id, s.started_at, s.user))
        .unwrap_or_default()
}

fn session_folder_label(path: &str) -> String {
    if path.is_empty() {
        "Directory unavailable".into()
    } else {
        safe_label(path)
    }
}

fn file_endpoint_label(device: &Device, scope: &Option<ContainerScope>) -> String {
    let host = identity(device);
    scope
        .as_ref()
        .map(|c| format!("{host} / {}", safe_label(&c.name)))
        .unwrap_or(host)
}

/// Encode a key for delivery to a command pane, or None if it has no bytes.
fn encode_key(key: KeyEvent) -> Option<Vec<u8>> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let mut bytes = match key.code {
        KeyCode::Char(c) => {
            if ctrl {
                let b = (c.to_ascii_uppercase() as u8).wrapping_sub(0x40);
                if b < 0x20 || b == 0x7f {
                    vec![b]
                } else {
                    return None;
                }
            } else {
                let mut buf = [0u8; 4];
                c.encode_utf8(&mut buf).as_bytes().to_vec()
            }
        }
        KeyCode::Esc => vec![0x1b],
        KeyCode::Enter => vec![b'\r'],
        KeyCode::Backspace => vec![0x7f],
        KeyCode::Tab => vec![b'\t'],
        KeyCode::Up => b"\x1b[A".to_vec(),
        KeyCode::Down => b"\x1b[B".to_vec(),
        KeyCode::Right => b"\x1b[C".to_vec(),
        KeyCode::Left => b"\x1b[D".to_vec(),
        KeyCode::Home => b"\x1b[H".to_vec(),
        KeyCode::End => b"\x1b[F".to_vec(),
        KeyCode::PageUp => b"\x1b[5~".to_vec(),
        KeyCode::PageDown => b"\x1b[6~".to_vec(),
        KeyCode::Delete => b"\x1b[3~".to_vec(),
        _ => return None,
    };
    if key.modifiers.contains(KeyModifiers::ALT) {
        bytes.insert(0, 0x1b);
    }
    Some(bytes)
}
/// Inner panel size for a command panel, bounded for readability.
fn command_panel_size(area: ratatui::layout::Size) -> (u16, u16) {
    let cols = area.width.saturating_sub(6).clamp(20, 200);
    let rows = area.height.saturating_sub(6).clamp(4, 120);
    (cols, rows)
}
/// A minimal Session describing a command pane, for full-screen expand.
fn command_panel_session(panel: &CommandPanel) -> Session {
    Session {
        id: panel.id.clone(),
        name: panel.label.clone(),
        directory: String::new(),
        provider: "shell".into(),
        host: String::new(),
        account: String::new(),
        pid: 0,
        started: String::new(),
        boot_id: String::new(),
        external: false,
        socket: None,
        launcher: None,
        process: None,
        container: None,
        activity: None,
        agent_tasks: vec![],
        origin: None,
        terminal: None,
    }
}
