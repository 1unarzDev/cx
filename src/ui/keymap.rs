//! The single keymap. Key dispatch (`keys.rs`), the help overlay, footer hints,
//! the context menu and the palette's key column all read `BINDINGS`, so a key
//! and its description can never drift apart.
use super::{Action, App, Focus, View};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// What a key, chip, tab or context-menu entry does. Mouse targets carry the
/// same command as the key, so taps and keys share one implementation.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Cmd {
    /// Documentation-only rows (selection motions are read by `navigation`).
    Motion,
    Goto(View),
    NextPanel,
    PrevPanel,
    Palette,
    RunCommand,
    Help,
    Quit,
    /// Enter: attach / open / scope / run, depending on the focused panel.
    Primary,
    /// Esc: back / clear / focus up one level.
    Back,
    Search,
    New,
    Jobs,
    Watch,
    Pin,
    AgentTasks,
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
    /// A palette/workspace action with its own availability (`action_enabled`).
    Do(Action),
    /// A verb of the design's screens (`screens::view_action`): the same as its key
    /// and its tap target.
    View(&'static str),
}

/// Where a binding applies. Lookup walks the active contexts most specific first.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Ctx {
    Global,
    Hub,
    Device,
    FilesList,
    Files,
    Preview,
    Containers,
}
impl Ctx {}

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
const fn code(code: KeyCode) -> K {
    K {
        code,
        ctrl: false,
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
    pub label: &'static str,
}
/// A binding's short name (context-menu rows); the key text and rank are kept in the
/// table as documentation of the footer the design draws.
const fn hint(_key: &'static str, label: &'static str, _rank: u8) -> Option<Hint> {
    Some(Hint { label })
}

pub(super) struct Binding {
    pub ctx: Ctx,
    pub keys: &'static [K],
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
/// A remote device's own screen (the viewer has no address or posture to edit).
fn real_device(app: &App) -> bool {
    app.device
        .checked_sub(1)
        .and_then(|d| app.devices.get(d))
        .is_some_and(|d| d.target.is_some())
}
fn sessions_selected(app: &App) -> bool {
    app.selected_session().is_some()
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

use KeyCode::{
    BackTab, Backspace, Down, End, Enter, Esc, Home, Left, PageDown, PageUp, Right, Tab, Up, F,
};

/// Order matters twice: the first matching binding wins within a context, and
/// help lists bindings in this order.
pub(super) const BINDINGS: &[Binding] = &[
    // Hub (the start view). Device letters, `e` + letter and the slots are read by
    // `screens::screen_key` first; these rows name them for help and the menus.
    bind!(
        Hub,
        [code(Enter)],
        "Enter",
        Cmd::View("open"),
        "Open the selected device / attach the selected session",
        hint("Enter", "Open", 0)
    ),
    bind!(
        Hub,
        [ch(' ')],
        "Space",
        Cmd::View("peek"),
        "Peek the selected session",
        hint("Space", "Peek", 1)
    ),
    bind!(
        Hub,
        [code(Backspace)],
        "Backspace",
        Cmd::View("stop"),
        "Stop the selected session · Backspace again",
        hint("Bksp", "Stop", 2)
    ),
    bind!(
        Hub,
        [code(Tab)],
        "Tab",
        Cmd::View("zone"),
        "Network / sessions zone"
    ),
    // A device's sessions.
    bind!(
        Device,
        [code(Enter)],
        "Enter · a s d g",
        Cmd::View("attach"),
        "Attach the selected session / a slot",
        hint("Enter", "Attach", 0)
    ),
    bind!(
        Device,
        [ch('c')],
        "c",
        Cmd::View("new-claude"),
        "New claude session here, attached",
        hint("c", "Claude", 1)
    ),
    bind!(
        Device,
        [ch('x')],
        "x",
        Cmd::View("new-codex"),
        "New codex session here, attached",
        hint("x", "Codex", 2)
    ),
    bind!(
        Device,
        [ch('n')],
        "n",
        Cmd::View("new-shell"),
        "New shell here, attached",
        hint("n", "Shell", 3)
    ),
    bind!(
        Device,
        [ch(' ')],
        "Space",
        Cmd::View("peek"),
        "Peek the selected session",
        hint("Space", "Peek", 4)
    ),
    bind!(
        Device,
        [ch('f')],
        "f",
        Cmd::View("files"),
        "Files on this device",
        hint("f", "Files", 4)
    ),
    bind!(
        Device,
        [ch('r')],
        "r",
        Cmd::View("retry"),
        "Retry / refresh this device",
        hint("r", "Retry", 5)
    ),
    bind!(
        Device,
        [code(Backspace)],
        "Backspace",
        Cmd::View("stop"),
        "Stop the selected session · Backspace again",
        hint("Bksp", "Stop · twice", 6)
    ),
    bind!(
        Device,
        [ch('w')],
        "w",
        Cmd::Watch,
        "Watch selected session · read-only",
        hint("", "Watch · read-only", 4),
        Some(sessions_selected)
    ),
    bind!(
        Device,
        [ch('p')],
        "p",
        Cmd::Pin,
        "Pin / unpin; keep an ended-work bookmark",
        hint("", "Pin / unpin", 4),
        Some(sessions_selected)
    ),
    bind!(
        Device,
        [ch('t')],
        "t",
        Cmd::AgentTasks,
        "Inspect the session's tasks and subagents",
        hint("", "Agent tasks", 4),
        Some(sessions_selected)
    ),
    bind!(
        Device,
        [ch('b'), ch('B')],
        "b",
        Cmd::ToggleAccess,
        "Toggle this device's access · * means single-direction",
        None,
        Some(real_device)
    ),
    bind!(
        Device,
        [ch('E')],
        "E",
        Cmd::EditDevice,
        "Edit this device's SSH address",
        None,
        Some(real_device)
    ),
    bind!(
        Device,
        [ch('X')],
        "X",
        Cmd::RemoveDevice,
        "Remove this device · confirm",
        None,
        Some(real_device)
    ),
    bind!(
        Preview,
        [ch('j'), ch('k'), code(Down), code(Up), ch(' ')],
        "j/k · Space/PgUp/PgDn",
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
        [code(Esc), code(Left), ch('h')],
        "Esc / h",
        Cmd::Back,
        "Finish search / clear highlights / return to files",
        hint("Esc", "Back", 5)
    ),
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
        [ch('/')],
        "/",
        Cmd::Search,
        "Find in this folder, live matches"
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
        hint("h / l", "Parent / open", 1)
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
        [ch('S')],
        "S",
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
        [ch(':')],
        ":",
        Cmd::RunCommand,
        "Run command in current folder",
        hint("n / :", "Session / command", 5),
        Some(not_files_search)
    ),
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
    bind!(
        Global,
        [ch('/')],
        "/",
        Cmd::Palette,
        "Find anything: sessions, devices, actions, folders"
    ),
    bind!(
        Global,
        [ch('`')],
        "`",
        Cmd::View("hub"),
        "The hub, from any screen (also Esc Esc)"
    ),
    bind!(
        Global,
        [ch('-')],
        "-",
        Cmd::View("last"),
        "Re-attach the last detached session"
    ),
    bind!(
        Global,
        [ch('+')],
        "+",
        Cmd::View("invite"),
        "Invite a device"
    ),
    bind!(
        Global,
        [ch('K')],
        "K",
        Cmd::View("secrets"),
        "Keys and secrets"
    ),
];

/// Contexts for the current focus, most specific first, ending with Global.
pub(super) fn contexts(app: &App) -> Vec<Ctx> {
    let mut ctx = Vec::with_capacity(3);
    match app.view {
        View::Network => ctx.push(Ctx::Hub),
        View::Work if app.device > 0 => ctx.push(Ctx::Device),
        View::Work => {}
        View::Files => {
            if app.browser.as_ref().is_some_and(|b| b.preview.is_some()) {
                ctx.push(Ctx::Preview);
            } else {
                ctx.push(Ctx::FilesList);
            }
            ctx.push(Ctx::Files);
        }
        View::Containers => ctx.push(Ctx::Containers),
        View::Secrets => {}
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
        for binding in BINDINGS.iter().filter(|b| b.ctx == ctx && !b.fold) {
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_dispatchable_binding_has_a_key_or_chord_and_help() {
        for b in BINDINGS {
            assert!(!b.help.is_empty() && !b.label.is_empty());
            assert!(!b.keys.is_empty(), "{}", b.label);
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
    fn no_static_binding_takes_a_device_letter_on_the_hub() {
        // The hub's letters are devices: only `e` (link) and `j k` (motion) are
        // bound there, and `K` (keys) is uppercase.
        for c in 'a'..='z' {
            let key = KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE);
            let taken = BINDINGS
                .iter()
                .filter(|b| matches!(b.ctx, Ctx::Hub | Ctx::Global))
                .any(|b| b.keys.iter().any(|k| k.matches(&key)));
            assert_eq!(taken, "jkgG".contains(c), "{c}");
        }
    }
}
