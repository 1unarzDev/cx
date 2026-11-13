//! Native workspace. Remote work runs off the input/render thread; attachment owns the terminal.
use crate::{
    model::{ContainerScope, CreateSession, Device, Operation, RunCommand, Session},
    sessions, store, transport,
};
mod activity;
mod container_agents;
mod filters;
mod rendering;
mod session_preview;
mod workspaces;
#[cfg(test)]
use rendering::render;
use rendering::render_with_native;
mod caps;
mod hitmap;
mod keymap;
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
mod file_index;
use file_index::{Entries, ViewIndex, VisibleRows};
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
mod actions;
mod containers_view;
mod hotkeys;
mod input;
mod keys;
#[cfg(test)]
mod lab;
mod launch;
mod screens;
mod viewmodel;
mod views;
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
        Block, Borders, Cell, Clear, List, ListItem, Padding, Paragraph, Row, Table, TableState,
        Wrap,
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

#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
enum View {
    Work,
    Files,
    Network,
    Containers,
    /// Keys / secrets inventory (placeholder until the secrets wave).
    Secrets,
}
#[derive(Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
enum Focus {
    /// (A handoff from an older cx may name its removed sidebar panels.)
    #[serde(other)]
    Workspace,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Input {
    Search,
    PreviewSearch,
    Palette,
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
    /// Modification time (Unix seconds), from the identity the helper sends.
    #[serde(default)]
    modified: Option<i64>,
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
    /// The helper sent only the start of the file.
    truncated: bool,
    page: u32,
    pages: Option<u32>,
    /// A streamed text continuation, prepared off the UI thread (`ReadChunk` replies).
    chunk: Option<PreviewChunk>,
}
impl RichPreview {
    fn from_value(value: &Value) -> Self {
        let mut preview = Self::plain(value);
        if preview.kind == "code" {
            preview.styled = Some(crate::syntax_preview::highlight(
                &safe_text(value["text"].as_str().unwrap_or("")),
                value["path"].as_str().unwrap_or(""),
            ));
        }
        preview
    }
    /// Everything but source highlighting, which the request worker delivers after the
    /// plain first paint (`Highlight` replies; see `runtime::start_task_workers`).
    fn plain(value: &Value) -> Self {
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
            truncated: value["truncated"] == true,
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
                Some("markdown") => Some(preview_lines(
                    value["text"].as_str().unwrap_or(""),
                    "markdown",
                )),
                _ => None,
            },
            chunk: None,
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
    entries: Entries,
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
    /// Wrapped preview layout, keyed on (revision, width, source); see `preview::DocCache`.
    #[serde(skip)]
    doc: std::cell::RefCell<Option<DocCache>>,
    #[serde(skip)]
    preview_path: Option<String>,
    #[serde(skip)]
    preview_requested_page: u32,
    #[serde(skip)]
    preview_pending_page: Option<u32>,
    preview_scroll: std::cell::Cell<u32>,
    #[serde(skip)]
    preview_viewport: std::cell::Cell<(u16, u16)>,
    #[serde(skip)]
    preview_link_cells: std::cell::RefCell<Vec<(u16, u16, Color)>>,
    #[serde(skip)]
    markdown_anchor_rows: std::cell::RefCell<BTreeMap<String, usize>>,
    #[serde(skip)]
    preview_find: PreviewFind,
    #[serde(skip)]
    preview_history: Vec<(String, u32, PreviewFind)>,
    #[serde(skip)]
    preview_restore: Option<(u32, PreviewFind)>,
    #[serde(skip)]
    preview_anchor: Option<String>,
    #[serde(default)]
    restore_selection: Option<String>,
    /// Derived listing caches (visible rows, lowercase names, search matches).
    #[serde(skip)]
    index: std::cell::RefCell<ViewIndex>,
    /// Entries the helper reported for the whole directory (`total`), when known.
    #[serde(skip)]
    total: Option<u64>,
    /// Child folders whose first listing page is being prefetched.
    #[serde(skip)]
    prefetch: BTreeSet<String>,
    /// Selected path and when it was first highlighted (prefetch/peek debounce).
    #[serde(skip)]
    settle: Option<(String, Instant, bool)>,
    /// Inline preview of the highlighted file on wide layouts (preview fields only).
    #[serde(skip)]
    peek: Option<Box<Browser>>,
    /// Whether the last render had room for the inline preview.
    #[serde(skip)]
    peek_shown: std::cell::Cell<bool>,
    /// The raw reply behind a loaded peek, so Enter can open it without a re-read.
    #[serde(skip)]
    peek_value: Option<Value>,
    /// Peek previews still in flight, as (path, request generation). Their replies are
    /// consumed by the peek (or dropped) and never reach the full preview.
    #[serde(skip)]
    peek_inflight: Vec<(String, u64)>,
    /// Streaming state for text previews larger than the helper's first window.
    #[serde(skip)]
    stream: Option<PreviewStream>,
}
impl Browser {
    fn new(device: usize, path: String) -> Self {
        Self {
            container: None,
            device,
            display_path: path.clone(),
            path,
            parent: None,
            entries: Entries::default(),
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
            doc: std::cell::RefCell::new(None),
            preview_path: None,
            preview_requested_page: 1,
            preview_pending_page: None,
            preview_scroll: std::cell::Cell::new(0),
            preview_viewport: std::cell::Cell::new((0, 0)),
            preview_link_cells: std::cell::RefCell::new(Vec::new()),
            markdown_anchor_rows: std::cell::RefCell::new(BTreeMap::new()),
            preview_find: PreviewFind::default(),
            preview_history: Vec::new(),
            preview_restore: None,
            preview_anchor: None,
            restore_selection: None,
            index: Default::default(),
            total: None,
            prefetch: BTreeSet::new(),
            settle: None,
            peek: None,
            peek_shown: std::cell::Cell::new(false),
            peek_value: None,
            peek_inflight: Vec::new(),
            stream: None,
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
    Files,
    New,
    Destination,
    TransferTo,
    Conflict,
    Jobs,
    Command,
    Observe,
    SendCtrlBackslash,
    PinSession,
    FolderSessions,
    Refresh,
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
    (
        Action::SendCtrlBackslash,
        "Send Ctrl+\\ to the last session · re-attaches it",
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
    PendingExit(usize),
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
    session_list: activity::State,
    devices: Vec<Device>,
    policies: Vec<store::DevicePolicy>,
    device: usize,
    focus: Focus,
    view: View,
    work: Vec<Cached>,
    pins: Vec<store::SessionPin>,
    selected: usize,
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
    /// Pointer press in progress (tap / long-press / scrollbar drag).
    press: Option<hitmap::Press>,
    /// Right-click / long-press menu: the focused selection's keymap commands.
    context_menu: Option<hitmap::ContextMenu>,
    /// Palette picks, most recent first (in-memory MRU ranking).
    palette_mru: VecDeque<String>,
    /// Hub: the selected node (view-model device id) and whether the sessions zone
    /// has focus; the device order the last frame drew.
    hub_device: Option<String>,
    hub_sessions: bool,
    /// A device's LINKS zone has focus, on the neighbour (view-model id) it selects.
    device_link: Option<String>,
    hub_order: std::cell::RefCell<Vec<String>>,
    /// Where the last frame drew each map node and session tile (`views::mission::Drawn::nav`).
    nav: std::cell::RefCell<Vec<(Rect, String)>>,
    /// Device letters (`hotkeys::assign`), by device id.
    hotkeys: std::cell::RefCell<BTreeMap<String, char>>,
    /// `␣`: the selected session's peek.
    peek: bool,
    /// `⌫` asked about this session (identity captured when asked).
    stop_armed: Option<(usize, Session)>,
    /// Files: the send-to picker is up.
    send_pick: bool,
    /// `e` was pressed: the next device letter opens that link.
    edge_pending: bool,
    edge: Option<screens::EdgeEdit>,
    edge_job: Option<mpsc::Receiver<std::result::Result<String, String>>>,
    edge_views: Vec<crate::edges::EdgeView>,
    secret_list: Vec<crate::secrets::SecretMeta>,
    secret_selected: usize,
    /// The keys screen's masked entry (set, rotate, a new secret's name first).
    secret_entry: Option<screens::SecretEntry>,
    secret_job: Option<mpsc::Receiver<std::result::Result<String, String>>>,
    /// The hub's open invite (`+`), served off the UI thread until use, expiry or Esc.
    invite: Option<screens::InviteRun>,
    /// The listener thread of the last invite, joined by the next one's thread.
    invite_thread: Option<std::thread::JoinHandle<()>>,
    /// Text for the system clipboard (OSC 52), written by the event loop.
    copy_out: Option<String>,
    /// The last session detached from, for `-`.
    last_session: Option<(usize, String)>,
    last_esc: Option<Instant>,
    onboarding_dismissed: bool,
    /// Where the open file preview was drawn (native images go there).
    preview_rect: std::cell::Cell<Option<Rect>>,
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
            session_list: activity::State::default(),
            devices,
            policies,
            device: 0,
            focus: Focus::Workspace,
            view: View::Network,
            work,
            pins: Vec::new(),
            selected: 0,
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
            press: None,
            context_menu: None,
            palette_mru: VecDeque::new(),
            hub_device: None,
            hub_sessions: false,
            device_link: None,
            hub_order: Default::default(),
            nav: Default::default(),
            hotkeys: Default::default(),
            peek: false,
            stop_armed: None,
            send_pick: false,
            edge_pending: false,
            edge: None,
            edge_job: None,
            edge_views: Vec::new(),
            secret_list: Vec::new(),
            secret_selected: 0,
            secret_entry: None,
            secret_job: None,
            invite: None,
            invite_thread: None,
            copy_out: None,
            last_session: None,
            last_esc: None,
            onboarding_dismissed: false,
            preview_rect: std::cell::Cell::new(None),
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
    /// `b` (and the edit menu's access entry): the edge editor for the
    /// viewer's link to this device, the one place access policy is set.
    fn edit_access(&mut self) {
        let Some(device) = self.device.checked_sub(1) else {
            self.set_notice_as(
                NoticeKind::Warning,
                "Select one remote device before changing its access".into(),
            );
            return;
        };
        self.open_edge(self.local_index(), device);
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
        self.view = View::Network;
        self.focus = Focus::Workspace;
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
        match crate::peer_ssh::remove_device(&device.id) {
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
        next.focus = Focus::Workspace;
        *self = next;
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
    fn refresh(&mut self) {
        if self.view != View::Network {
            for d in 0..self.devices.len() {
                self.providers.remove(&d);
                self.check_providers(d);
            }
        }
        match self.view {
            View::Secrets => {}
            View::Containers => self.refresh_containers(),
            View::Work => self.refresh_work(),
            View::Files => self.refresh_browser(),
            // The hub: every device's sessions (its evidence), the edge graph and,
            // on this explicit refresh only, the network observation behind it.
            View::Network => {
                for d in 0..self.work.len().min(self.devices.len()) {
                    if (self.device == 0 || self.device == d + 1) && !self.work[d].loading {
                        self.work[d].loading = self.send(d, Operation::Sessions);
                    }
                }
                self.reload_edges();
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
            if ctrl && ('4'..='7').contains(&c) {
                // Legacy terminals send Ctrl+\ ] ^ _ as 0x1c-0x1f, which
                // crossterm reports as Ctrl+4..7. Ctrl+] must reach the app.
                vec![c as u8 - b'4' + 0x1c]
            } else if ctrl && matches!(c, ' ' | '2' | '@') {
                vec![0]
            } else if ctrl {
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
        idle_seconds: None,
        origin: None,
        terminal: None,
    }
}
