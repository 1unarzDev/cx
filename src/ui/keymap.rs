//! The single keymap. Key dispatch (`keys.rs`), the help overlay, footer hints,
//! the context menu and the palette's key column all read `BINDINGS`, so a key
//! and its description can never drift apart.
use super::{local_only, Action, App, Focus, View};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// What a key, chip, tab or context-menu entry does. Mouse targets carry the
/// same command as the key, so taps and keys share one implementation.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Cmd {
    /// Documentation-only rows (selection motions are read by `navigation`).
    Motion,
    Goto(View),
    PrevView,
    NextView,
    NextPanel,
    PrevPanel,
    FocusDir(i8, i8),
    Palette,
    RunCommand,
    Help,
    Quit,
    /// Enter: attach / open / scope / run, depending on the focused panel.
    Primary,
    /// Esc: back / clear / focus up one level.
    Back,
    Search,
    Filter,
    New,
    Jobs,
    AddDevice,
    Watch,
    /// Type into the selected session's live preview.
    TypeInline,
    /// Zoom the live preview over the list (Esc returns).
    Zoom,
    Pin,
    Stop,
    AgentTasks,
    Refresh,
    ToggleAccess,
    RemoveDevice,
    EditDevice,
    Left,
    Right,
    SessionsHere,
    NextMatch,
    PrevMatch,
    ClearClipboard,
    ClearMarks,
    PreviewLinks,
    PreviewSearch,
    DetailDown,
    DetailUp,
    /// A palette/workspace action with its own availability (`action_enabled`).
    Do(Action),
}

/// Where a binding applies. Lookup walks the active contexts most specific first.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Ctx {
    Global,
    Sessions,
    Devices,
    Actions,
    FilesList,
    Files,
    Preview,
    Network,
    Containers,
}
impl Ctx {
    pub(super) fn title(self) -> &'static str {
        match self {
            Ctx::Global => "Everywhere",
            Ctx::Sessions => "Sessions",
            Ctx::Devices => "Devices",
            Ctx::Actions => "Actions panel",
            Ctx::FilesList => "Files",
            Ctx::Files => "Files and previews",
            Ctx::Preview => "File preview",
            Ctx::Network => "Network",
            Ctx::Containers => "Containers",
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct K {
    code: KeyCode,
    ctrl: bool,
    alt: bool,
}
const fn ch(c: char) -> K {
    K {
        code: KeyCode::Char(c),
        ctrl: false,
        alt: false,
    }
}
const fn ctrl(c: char) -> K {
    K {
        code: KeyCode::Char(c),
        ctrl: true,
        alt: false,
    }
}
const fn alt(c: char) -> K {
    K {
        code: KeyCode::Char(c),
        ctrl: false,
        alt: true,
    }
}
const fn code(code: KeyCode) -> K {
    K {
        code,
        ctrl: false,
        alt: false,
    }
}
const fn ctrl_code(code: KeyCode) -> K {
    K {
        code,
        ctrl: true,
        alt: false,
    }
}
impl K {
    fn matches(&self, key: &KeyEvent) -> bool {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        if ctrl != self.ctrl || alt != self.alt {
            return false;
        }
        match (self.code, key.code) {
            // Shift+Tab arrives as BackTab with or without SHIFT.
            (KeyCode::BackTab, KeyCode::BackTab) => true,
            (KeyCode::Tab, KeyCode::Tab) => !key.modifiers.contains(KeyModifiers::SHIFT),
            (KeyCode::Tab, KeyCode::BackTab) => false,
            (KeyCode::Char(a), KeyCode::Char(b)) if self.ctrl => a.eq_ignore_ascii_case(&b),
            (a, b) => a == b,
        }
    }
}

/// A footer chip: key text and a short label. Rank orders chips (lower first).
#[derive(Clone, Copy)]
pub(super) struct Hint {
    pub key: &'static str,
    pub label: &'static str,
    pub rank: u8,
}
const fn hint(key: &'static str, label: &'static str, rank: u8) -> Option<Hint> {
    Some(Hint { key, label, rank })
}

pub(super) struct Binding {
    pub ctx: Ctx,
    pub keys: &'static [K],
    /// `g` followed by this character (the goto chord).
    pub chord: Option<char>,
    /// Key text for help and the context menu.
    pub label: &'static str,
    pub cmd: Cmd,
    pub help: &'static str,
    pub hint: Option<Hint>,
    /// Extra availability beyond the context; `None` means always.
    pub when: Option<fn(&App) -> bool>,
    /// A second key of a row already described in help (e.g. Alt+2 under
    /// "Alt+1..4"); dispatched, but not listed again.
    pub fold: bool,
}

macro_rules! bind {
    ($ctx:ident, [$($k:expr),*], $label:expr, $cmd:expr, $help:expr) => {
        bind!($ctx, [$($k),*], $label, $cmd, $help, None, None)
    };
    ($ctx:ident, [$($k:expr),*], $label:expr, $cmd:expr, $help:expr, $hint:expr) => {
        bind!($ctx, [$($k),*], $label, $cmd, $help, $hint, None)
    };
    ($ctx:ident, [$($k:expr),*], $label:expr, $cmd:expr, $help:expr, $hint:expr, $when:expr) => {
        Binding {
            ctx: Ctx::$ctx,
            keys: &[$($k),*],
            chord: None,
            label: $label,
            cmd: $cmd,
            help: $help,
            hint: $hint,
            when: $when,
            fold: false,
        }
    };
}
/// A dispatch-only alias of the row above it in help.
macro_rules! alias {
    ($ctx:ident, [$($k:expr),*], $label:expr, $cmd:expr, $help:expr) => {
        Binding { fold: true, ..bind!($ctx, [$($k),*], $label, $cmd, $help) }
    };
    ($ctx:ident, [$($k:expr),*], $label:expr, $cmd:expr, $help:expr, $hint:expr, $when:expr) => {
        Binding { fold: true, ..bind!($ctx, [$($k),*], $label, $cmd, $help, $hint, $when) }
    };
}
macro_rules! goto {
    ($c:expr, $label:expr, $view:ident, $help:expr) => {
        Binding {
            ctx: Ctx::Global,
            keys: &[],
            chord: Some($c),
            label: $label,
            cmd: Cmd::Goto(View::$view),
            help: $help,
            hint: None,
            when: None,
            fold: false,
        }
    };
}

fn files_search(app: &App) -> bool {
    app.browser.as_ref().is_some_and(|b| !b.search.is_empty())
}
fn not_files_search(app: &App) -> bool {
    !(app.view == View::Files && app.focus == Focus::Workspace && files_search(app))
}
fn markdown_preview(app: &App) -> bool {
    app.markdown_preview_active()
}
fn pdf_preview(app: &App) -> bool {
    app.pdf_preview_active()
}
fn not_pdf_preview(app: &App) -> bool {
    !pdf_preview(app)
}
fn real_device(app: &App) -> bool {
    app.device > 0
}
fn work_local_only(app: &App) -> bool {
    app.view == View::Work && local_only(app)
}
fn list_filter(app: &App) -> bool {
    app.view != View::Files || app.focus == Focus::Devices
}
fn sessions_selected(app: &App) -> bool {
    app.selected_session().is_some()
}
fn sessions_empty(app: &App) -> bool {
    app.selected_session().is_none()
}
/// False on the Keys/Secrets placeholder body, where Enter and `/` have
/// nothing to act on (the device rail keeps its own keys).
fn not_placeholder(app: &App) -> bool {
    !(app.view == View::Secrets && app.focus == Focus::Workspace)
}
fn destination_paste(app: &App) -> bool {
    app.destination_active && app.clipboard.is_some()
}
fn not_destination_paste(app: &App) -> bool {
    !destination_paste(app)
}

use KeyCode::{BackTab, Down, End, Enter, Esc, Home, Left, PageDown, PageUp, Right, Tab, Up, F};

/// Order matters twice: the first matching binding wins within a context, and
/// help lists bindings in this order.
pub(super) const BINDINGS: &[Binding] = &[
    // Sessions list.
    bind!(
        Sessions,
        [code(Enter)],
        "Enter",
        Cmd::Primary,
        "Attach the selected session",
        hint("Enter", "Attach", 0),
        Some(sessions_selected)
    ),
    bind!(
        Sessions,
        [code(Enter)],
        "Enter",
        Cmd::Primary,
        "Start a new session here",
        hint("Enter/n", "New session", 0),
        Some(sessions_empty)
    ),
    bind!(
        Sessions,
        [ch('i')],
        "i",
        Cmd::TypeInline,
        "Type into the live preview · Ctrl+\\ leaves",
        hint("i", "Type", 1),
        Some(sessions_selected)
    ),
    bind!(
        Sessions,
        [ch('z')],
        "z",
        Cmd::Zoom,
        "Zoom the live preview · z / Esc back",
        hint("z", "Zoom", 1),
        Some(sessions_selected)
    ),
    bind!(
        Sessions,
        [ch('w')],
        "w",
        Cmd::Watch,
        "Watch selected session · read-only",
        hint("w", "Watch · read-only", 2),
        Some(sessions_selected)
    ),
    bind!(
        Sessions,
        [ch('p')],
        "p",
        Cmd::Pin,
        "Pin / unpin; keep an ended-work bookmark",
        hint("p", "Pin / unpin", 3),
        Some(sessions_selected)
    ),
    bind!(
        Sessions,
        [ch('s')],
        "s",
        Cmd::AgentTasks,
        "Inspect selected session's tasks and subagents",
        hint("s", "Agent tasks", 4),
        Some(sessions_selected)
    ),
    bind!(
        Sessions,
        [ch('d')],
        "d",
        Cmd::Stop,
        "Stop selected CX-managed session · confirm",
        hint("d", "Stop · confirm", 5),
        Some(sessions_selected)
    ),
    // Devices rail.
    bind!(
        Devices,
        [code(Enter)],
        "Enter",
        Cmd::Primary,
        "Scope the view to this device",
        hint("Enter", "Open", 0)
    ),
    bind!(
        Devices,
        [ch('r')],
        "r",
        Cmd::Refresh,
        "Refresh the selected device (moving never refreshes)",
        hint("r", "Refresh", 1)
    ),
    bind!(
        Devices,
        [ch('b'), ch('B')],
        "b",
        Cmd::ToggleAccess,
        "Toggle selected remote device access · * means single-direction"
    ),
    bind!(
        Devices,
        [ch('e'), ch('E')],
        "e",
        Cmd::EditDevice,
        "Edit the selected device's SSH address",
        None,
        Some(real_device)
    ),
    bind!(
        Devices,
        [ch('x'), ch('X')],
        "x",
        Cmd::RemoveDevice,
        "Remove the selected device · confirm",
        None,
        Some(real_device)
    ),
    bind!(
        Devices,
        [ch('/')],
        "/ · f",
        Cmd::Filter,
        "Filter devices",
        hint("/", "Filter", 2)
    ),
    bind!(
        Devices,
        [code(Right), ch('l')],
        "→ / l",
        Cmd::Right,
        "Focus the workspace"
    ),
    // Actions panel.
    bind!(
        Actions,
        [code(Enter)],
        "Enter",
        Cmd::Primary,
        "Run the selected action",
        hint("Enter", "Run", 0)
    ),
    // File preview (checked before Files).
    bind!(
        Preview,
        [ch('j'), ch('k'), code(Down), code(Up)],
        "j/k · PgUp/PgDn",
        Cmd::Motion,
        "Scroll preview",
        hint("j/k", "Scroll", 0),
        Some(not_pdf_preview)
    ),
    alias!(
        Preview,
        [ch('j'), ch('k'), code(Down), code(Up)],
        "j/k",
        Cmd::Motion,
        "Change PDF page",
        hint("j/k", "Pages", 0),
        Some(pdf_preview)
    ),
    bind!(
        Preview,
        [ch('g'), ch('G')],
        "gg / G",
        Cmd::Motion,
        "Preview top / bottom (first/last PDF page)"
    ),
    bind!(
        Preview,
        [ch('o')],
        "o / click link",
        Cmd::PreviewLinks,
        "Inspect links; Enter opens viewer browser",
        hint("o", "Links", 1),
        Some(markdown_preview)
    ),
    bind!(
        Preview,
        [ch('/')],
        "/",
        Cmd::PreviewSearch,
        "Search preview text, live fuzzy matches",
        hint("/", "Search", 2)
    ),
    bind!(
        Preview,
        [ch('n')],
        "n / N",
        Cmd::NextMatch,
        "Next / previous match, wrap around",
        hint("n / N", "Next / previous", 3)
    ),
    alias!(Preview, [ch('N')], "N", Cmd::PrevMatch, "Previous match"),
    bind!(
        Preview,
        [code(Esc)],
        "Esc",
        Cmd::Back,
        "Finish search / clear highlights / return to files",
        hint("Esc", "Back", 5)
    ),
    // File list verbs (no preview open).
    bind!(
        FilesList,
        [code(Enter)],
        "Enter",
        Cmd::Primary,
        "Transfer the clipboard into this folder",
        hint("Enter", "Transfer here", 0),
        Some(destination_paste)
    ),
    bind!(
        FilesList,
        [ch('n')],
        "n / N",
        Cmd::NextMatch,
        "Next / previous matching file",
        hint("n / N", "Next / previous", 4),
        Some(files_search)
    ),
    alias!(
        FilesList,
        [ch('N')],
        "N",
        Cmd::PrevMatch,
        "Previous matching file",
        None,
        Some(files_search)
    ),
    bind!(
        FilesList,
        [ch(' ')],
        "Space",
        Cmd::Do(Action::Select),
        "Select and advance",
        hint("Space", "Select", 1),
        Some(not_destination_paste)
    ),
    bind!(
        FilesList,
        [ch('v')],
        "v",
        Cmd::Do(Action::Visual),
        "Visual range selection"
    ),
    bind!(
        FilesList,
        [ch('u')],
        "u",
        Cmd::ClearMarks,
        "Clear selection"
    ),
    bind!(
        FilesList,
        [ch('.')],
        ".",
        Cmd::Do(Action::Hidden),
        "Show / hide hidden files"
    ),
    bind!(
        FilesList,
        [ch('f')],
        "f",
        Cmd::Do(Action::Filter),
        "Filter names"
    ),
    bind!(
        FilesList,
        [ch('c'), ch('y')],
        "c / y",
        Cmd::Do(Action::Copy),
        "Copy, then choose another folder or device",
        hint("c / x", "Copy / cut", 2),
        Some(not_destination_paste)
    ),
    bind!(
        FilesList,
        [ch('x')],
        "x",
        Cmd::Do(Action::Cut),
        "Cut, then choose another folder or device"
    ),
    bind!(
        FilesList,
        [ch('p')],
        "p",
        Cmd::Do(Action::Paste),
        "Paste here",
        hint("p / t", "Paste / transfer", 3),
        Some(not_destination_paste)
    ),
    bind!(
        FilesList,
        [ch('Y')],
        "Y",
        Cmd::ClearClipboard,
        "Clear clipboard · files unchanged"
    ),
    bind!(
        FilesList,
        [ch('t')],
        "t",
        Cmd::Do(Action::TransferTo),
        "Transfer to… choose device, folder, then Enter"
    ),
    bind!(
        FilesList,
        [ch('T')],
        "T",
        Cmd::Do(Action::Jobs),
        "Transfer jobs and results",
        hint("T", "Progress", 4),
        Some(destination_paste)
    ),
    bind!(
        FilesList,
        [ch('r')],
        "r",
        Cmd::Do(Action::Rename),
        "Rename the selected file",
        hint("r / d", "Rename / delete", 6),
        Some(not_destination_paste)
    ),
    bind!(
        FilesList,
        [ch('d')],
        "d",
        Cmd::Do(Action::Delete),
        "Delete · confirm permanently"
    ),
    bind!(
        FilesList,
        [ch('M')],
        "M",
        Cmd::Do(Action::Mkdir),
        "New folder"
    ),
    bind!(
        FilesList,
        [ch('o')],
        "o",
        Cmd::Do(Action::Conflict),
        "Cycle copy conflict policy",
        hint("o", "Conflict policy", 3),
        Some(destination_paste)
    ),
    // Files and previews.
    bind!(
        Files,
        [code(Enter)],
        "Enter",
        Cmd::Primary,
        "Open folder / preview file"
    ),
    bind!(
        Files,
        [ch('n')],
        "n",
        Cmd::New,
        "New session in this folder"
    ),
    bind!(
        Files,
        [code(Left), ch('h')],
        "← / h",
        Cmd::Left,
        "Parent directory",
        hint("h / l", "Parent / open", 1),
        Some(destination_paste)
    ),
    alias!(
        Files,
        [code(Right), ch('l')],
        "→ / l",
        Cmd::Right,
        "Enter directory"
    ),
    bind!(
        Files,
        [ch('s')],
        "s",
        Cmd::SessionsHere,
        "Sessions in selected / current folder",
        hint("s", "Sessions here", 0),
        Some(not_destination_paste)
    ),
    bind!(
        Files,
        [code(Esc)],
        "Esc",
        Cmd::Back,
        "Clear selection / filter / search, then back to Sessions"
    ),
    // Network.
    bind!(
        Network,
        [code(Enter)],
        "Enter",
        Cmd::Primary,
        "Open / connect",
        hint("Enter", "Open / connect", 1)
    ),
    bind!(
        Network,
        [code(Left), ch('h')],
        "h / l",
        Cmd::Left,
        "Fold / expand",
        hint("h / l", "Fold / expand", 2)
    ),
    alias!(Network, [code(Right), ch('l')], "l", Cmd::Right, "Expand"),
    bind!(
        Network,
        [ch('J')],
        "J / K",
        Cmd::DetailDown,
        "Scroll details",
        hint("J / K", "Scroll details", 3)
    ),
    alias!(Network, [ch('K')], "K", Cmd::DetailUp, "Scroll details up"),
    // Containers.
    bind!(
        Containers,
        [code(Enter)],
        "Enter",
        Cmd::Primary,
        "Container actions",
        hint("Enter", "Actions", 0)
    ),
    bind!(
        Containers,
        [code(Left), ch('h')],
        "h / l",
        Cmd::Left,
        "Fold / expand",
        hint("h / l", "Fold / expand", 2)
    ),
    alias!(
        Containers,
        [code(Right), ch('l')],
        "l",
        Cmd::Right,
        "Expand"
    ),
    // Everywhere.
    bind!(
        Global,
        [code(Up), code(Down), ch('j'), ch('k')],
        "↑↓ / j k",
        Cmd::Motion,
        "Move selection (counts: 5j)",
        None
    ),
    bind!(
        Global,
        [ch('g'), ch('G'), code(Home), code(End)],
        "gg / G",
        Cmd::Motion,
        "First / last row",
        None
    ),
    bind!(
        Global,
        [code(PageUp), code(PageDown)],
        "PgUp / PgDn",
        Cmd::Motion,
        "Page",
        None
    ),
    goto!('s', "g s", Work, "Go to Sessions"),
    goto!('f', "g f", Files, "Go to Files"),
    goto!('n', "g n", Network, "Go to Network"),
    goto!('k', "g k", Secrets, "Go to Keys / Secrets"),
    bind!(
        Global,
        [ch('[')],
        "[ / ]",
        Cmd::PrevView,
        "Previous / next view"
    ),
    alias!(Global, [ch(']')], "]", Cmd::NextView, "Next view"),
    bind!(
        Global,
        [alt('1')],
        "Alt+1..4",
        Cmd::Goto(View::Work),
        "Go to view 1-4 (Sessions, Files, Network, Keys)"
    ),
    alias!(
        Global,
        [alt('2')],
        "Alt+2",
        Cmd::Goto(View::Files),
        "Go to Files"
    ),
    alias!(
        Global,
        [alt('3')],
        "Alt+3",
        Cmd::Goto(View::Network),
        "Go to Network"
    ),
    alias!(
        Global,
        [alt('4')],
        "Alt+4",
        Cmd::Goto(View::Secrets),
        "Go to Keys / Secrets"
    ),
    bind!(
        Global,
        [code(Tab)],
        "Tab / Shift+Tab",
        Cmd::NextPanel,
        "Next / previous panel"
    ),
    alias!(
        Global,
        [code(BackTab)],
        "Shift+Tab",
        Cmd::PrevPanel,
        "Previous panel"
    ),
    bind!(
        Global,
        [ctrl_code(Left)],
        "Ctrl+arrows",
        Cmd::FocusDir(-1, 0),
        "Move panel focus by direction"
    ),
    alias!(
        Global,
        [ctrl_code(Right)],
        "Ctrl+→",
        Cmd::FocusDir(1, 0),
        "Focus panel to the right"
    ),
    alias!(
        Global,
        [ctrl_code(Up)],
        "Ctrl+↑",
        Cmd::FocusDir(0, -1),
        "Focus panel above"
    ),
    alias!(
        Global,
        [ctrl_code(Down)],
        "Ctrl+↓",
        Cmd::FocusDir(0, 1),
        "Focus panel below"
    ),
    bind!(
        Global,
        [code(Left), ch('h')],
        "← / h",
        Cmd::Left,
        "Focus devices"
    ),
    alias!(
        Global,
        [code(Right), ch('l')],
        "→ / l",
        Cmd::Right,
        "Focus the workspace"
    ),
    bind!(
        Global,
        [code(Enter)],
        "Enter",
        Cmd::Primary,
        "Open",
        hint("Enter", "Open", 0),
        Some(not_placeholder)
    ),
    bind!(
        Global,
        [code(Esc)],
        "Esc",
        Cmd::Back,
        "Back / clear current mode"
    ),
    bind!(
        Global,
        [ch('/')],
        "/",
        Cmd::Search,
        "Search",
        hint("/", "Search", 7),
        Some(not_placeholder)
    ),
    bind!(
        Global,
        [ch('f')],
        "f",
        Cmd::Filter,
        "Filter the list",
        None,
        Some(list_filter)
    ),
    bind!(
        Global,
        [ch('a')],
        "a",
        Cmd::AddDevice,
        "Add by SSH address",
        hint("a", "Add by SSH address", 2),
        Some(work_local_only)
    ),
    bind!(
        Global,
        [ch('n')],
        "n",
        Cmd::New,
        "New session in current folder",
        hint("n", "New session", 1),
        Some(not_files_search)
    ),
    bind!(
        Global,
        [ch(':')],
        ":",
        Cmd::RunCommand,
        "Run command in current folder",
        hint("n / :", "Session / command", 5),
        Some(not_files_search)
    ),
    // `:` always runs; only its footer chip yields to the search's `n / N`.
    alias!(
        Global,
        [ch(':')],
        ":",
        Cmd::RunCommand,
        "Run command in current folder",
        None,
        None
    ),
    bind!(
        Global,
        [ch('T')],
        "T",
        Cmd::Jobs,
        "Transfers · progress / cancel / retry"
    ),
    bind!(
        Global,
        [ctrl('p')],
        "Ctrl+P",
        Cmd::Palette,
        "Actions, sessions, devices and folders",
        hint("Ctrl P", "Actions", 8)
    ),
    bind!(
        Global,
        [ch('?'), code(F(1))],
        "? / F1",
        Cmd::Help,
        "Keyboard help",
        hint("?", "Help", 9)
    ),
    bind!(
        Global,
        [ctrl('c')],
        "Ctrl+C",
        Cmd::Quit,
        "Quit cx; work keeps running",
        hint("Ctrl C", "Quit", 10)
    ),
];

/// Contexts for the current focus, most specific first, ending with Global.
pub(super) fn contexts(app: &App) -> Vec<Ctx> {
    let mut ctx = Vec::with_capacity(3);
    match app.focus {
        Focus::Devices => ctx.push(Ctx::Devices),
        Focus::Actions => ctx.push(Ctx::Actions),
        Focus::Workspace => match app.view {
            View::Work => ctx.push(Ctx::Sessions),
            View::Files => {
                if app.browser.as_ref().is_some_and(|b| b.preview.is_some()) {
                    ctx.push(Ctx::Preview);
                } else {
                    ctx.push(Ctx::FilesList);
                }
                ctx.push(Ctx::Files);
            }
            View::Network => ctx.push(Ctx::Network),
            View::Containers => ctx.push(Ctx::Containers),
            View::Secrets => {}
        },
    }
    ctx.push(Ctx::Global);
    ctx
}

fn available(app: &App, binding: &Binding) -> bool {
    binding.when.is_none_or(|when| when(app))
}

fn winner(app: &App, key: &KeyEvent) -> Option<&'static Binding> {
    for ctx in contexts(app) {
        for binding in BINDINGS.iter().filter(|b| b.ctx == ctx) {
            if binding.keys.iter().any(|k| k.matches(key)) && available(app, binding) {
                return Some(binding);
            }
        }
    }
    None
}
/// The command a key runs in the current context, if any.
pub(super) fn resolve(app: &App, key: &KeyEvent) -> Option<Cmd> {
    winner(app, key).map(|b| b.cmd)
}
/// True when this binding is the one its first key actually runs right now.
fn effective(app: &App, binding: &'static Binding) -> bool {
    let Some(k) = binding.keys.first() else {
        return binding.chord.is_some();
    };
    let mut modifiers = KeyModifiers::NONE;
    if k.ctrl {
        modifiers |= KeyModifiers::CONTROL;
    }
    if k.alt {
        modifiers |= KeyModifiers::ALT;
    }
    winner(app, &KeyEvent::new(k.code, modifiers)).is_some_and(|w| std::ptr::eq(w, binding))
}

/// The goto chord `g <c>`, if `c` names a view.
pub(super) fn chord(c: char) -> Option<Cmd> {
    BINDINGS.iter().find(|b| b.chord == Some(c)).map(|b| b.cmd)
}

/// Footer chips for the focused context, ranked. A key or label already
/// claimed by a more specific context is not repeated.
pub(super) fn hints(app: &App) -> Vec<(Hint, Cmd)> {
    let mut out: Vec<(Hint, Cmd)> = Vec::new();
    for ctx in contexts(app) {
        for binding in BINDINGS.iter().filter(|b| b.ctx == ctx) {
            let Some(hint) = binding.hint else { continue };
            // Combined chips ("n / :") yield when any of their keys already
            // has a chip of its own.
            let parts = |key: &'static str| key.split(" / ");
            if !effective(app, binding)
                || out.iter().any(|(h, _)| {
                    h.label == hint.label || parts(h.key).any(|a| parts(hint.key).any(|b| a == b))
                })
            {
                continue;
            }
            out.push((hint, binding.cmd));
        }
    }
    out.sort_by_key(|(hint, _)| hint.rank);
    out
}

/// Help sections: the active contexts first (with their available bindings),
/// then every other context, so the overlay is complete but leads with what applies.
pub(super) fn help_sections(app: &App) -> Vec<(&'static str, Vec<(&'static str, &'static str)>)> {
    let active = contexts(app);
    // Key labels a more specific active context already explains; the generic
    // Everywhere row for the same key (e.g. Enter "Open") is folded (P15).
    let specific: Vec<&'static str> = active
        .iter()
        .filter(|ctx| **ctx != Ctx::Global)
        .flat_map(|ctx| BINDINGS.iter().filter(move |b| b.ctx == *ctx))
        .filter(|b| available(app, b))
        .map(|b| b.label)
        .collect();
    let mut order = active.clone();
    for ctx in [
        Ctx::Sessions,
        Ctx::Devices,
        Ctx::FilesList,
        Ctx::Files,
        Ctx::Preview,
        Ctx::Network,
        Ctx::Containers,
        Ctx::Actions,
    ] {
        if !order.contains(&ctx) {
            order.push(ctx);
        }
    }
    order
        .into_iter()
        .map(|ctx| {
            let mut rows: Vec<(&'static str, &'static str)> = Vec::new();
            for binding in BINDINGS.iter().filter(|b| b.ctx == ctx && !b.fold) {
                if active.contains(&ctx) && !available(app, binding) {
                    continue;
                }
                if ctx == Ctx::Global && specific.contains(&binding.label) {
                    continue;
                }
                // One row per key label; the first description wins.
                if rows.iter().any(|(label, _)| *label == binding.label) {
                    continue;
                }
                // Secondary halves of combined rows ("N" after "n / N") are folded.
                if rows.iter().any(|(label, _)| {
                    label.contains(" / ") && label.split(" / ").any(|p| p == binding.label)
                }) {
                    continue;
                }
                rows.push((binding.label, binding.help));
            }
            (ctx.title(), rows)
        })
        .filter(|(_, rows)| !rows.is_empty())
        .collect()
}

/// The short name of a binding: its footer-chip label, else the help text
/// up to its first clause (context menu rows must fit a narrow box).
pub(super) fn short_label(binding: &'static Binding) -> &'static str {
    if let Some(hint) = binding.hint {
        return hint.label;
    }
    let help = binding.help;
    let end = [" · ", "; ", " (", ", "]
        .iter()
        .filter_map(|sep| help.find(sep))
        .min()
        .unwrap_or(help.len());
    &help[..end]
}

/// Context-menu entries for the focused selection: the same commands as its keys.
pub(super) fn context_menu(app: &App) -> Vec<(&'static str, &'static str, Cmd)> {
    let mut out: Vec<(&'static str, &'static str, Cmd)> = Vec::new();
    for ctx in contexts(app) {
        if ctx == Ctx::Global {
            break;
        }
        for binding in BINDINGS.iter().filter(|b| b.ctx == ctx) {
            if matches!(
                binding.cmd,
                Cmd::Motion | Cmd::Back | Cmd::Left | Cmd::Right
            ) || !available(app, binding)
                || out.iter().any(|(_, _, c)| *c == binding.cmd)
            {
                continue;
            }
            if let Cmd::Do(action) = binding.cmd {
                if !app.action_enabled(action) {
                    continue;
                }
            }
            out.push((binding.label, short_label(binding), binding.cmd));
        }
    }
    for cmd in [Cmd::New, Cmd::Palette] {
        if let Some(b) = BINDINGS
            .iter()
            .find(|b| b.ctx == Ctx::Global && b.cmd == cmd)
        {
            if available(app, b) && !out.iter().any(|(_, _, c)| *c == cmd) {
                out.push((b.label, short_label(b), cmd));
            }
        }
    }
    out
}

/// Key text for a command in the current context (palette key column).
pub(super) fn key_for(app: &App, cmd: Cmd) -> Option<&'static str> {
    let active = contexts(app);
    BINDINGS
        .iter()
        .filter(|b| b.cmd == cmd && active.contains(&b.ctx))
        .find(|b| available(app, b))
        .or_else(|| BINDINGS.iter().find(|b| b.cmd == cmd))
        .map(|b| b.label)
}

/// The four top-level views in tab order.
pub(super) const TABS: [(View, &str, &str); 4] = [
    (View::Work, "Sessions", "Sess"),
    (View::Files, "Files", "Files"),
    (View::Network, "Network", "Net"),
    (View::Secrets, "Keys", "Keys"),
];
pub(super) fn tab_index(view: View) -> usize {
    match view {
        View::Work | View::Containers => 0,
        View::Files => 1,
        View::Network => 2,
        View::Secrets => 3,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_dispatchable_binding_has_a_key_or_chord_and_help() {
        for b in BINDINGS {
            assert!(!b.help.is_empty() && !b.label.is_empty());
            assert!(!b.keys.is_empty() || b.chord.is_some(), "{}", b.label);
        }
    }
    #[test]
    fn ctrl_hjkl_no_longer_moves_focus() {
        for c in ['h', 'j', 'k', 'l'] {
            let key = KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL);
            assert!(BINDINGS
                .iter()
                .all(|b| !b.keys.iter().any(|k| k.matches(&key))));
        }
    }
    #[test]
    fn goto_chords_cover_the_four_views() {
        assert_eq!(chord('s'), Some(Cmd::Goto(View::Work)));
        assert_eq!(chord('f'), Some(Cmd::Goto(View::Files)));
        assert_eq!(chord('n'), Some(Cmd::Goto(View::Network)));
        assert_eq!(chord('k'), Some(Cmd::Goto(View::Secrets)));
        assert_eq!(chord('x'), None);
    }
}
