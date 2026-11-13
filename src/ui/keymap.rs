//! The single keymap for dispatch: key dispatch (`keys.rs`), the context menu and the
//! palette's key column read `BINDINGS`. The design's screens draw their own help and
//! footer chips (`views::mission::views`, from the spec); their taps run the same
//! verbs (`screens::view_action`). The screens' own keys (device letters, slots, the
//! link editor) come first, in `screens::screen_key`.
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
    EditAccess,
    RemoveDevice,
    EditDevice,
    Left,
    Right,
    SessionsHere,
    NextMatch,
    PrevMatch,
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

/// A binding's short name (context-menu rows).
#[derive(Clone, Copy)]
pub(super) struct Hint {
    pub label: &'static str,
}
const fn hint(label: &'static str) -> Option<Hint> {
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
        hint("Open")
    ),
    bind!(
        Hub,
        [ch(' ')],
        "Space",
        Cmd::View("peek"),
        "Peek the selected session",
        hint("Peek")
    ),
    bind!(
        Hub,
        [code(Backspace)],
        "Backspace",
        Cmd::View("stop"),
        "Stop the selected session · Backspace again",
        hint("Stop")
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
        hint("Attach")
    ),
    bind!(
        Device,
        [ch('c')],
        "c",
        Cmd::View("new-claude"),
        "New claude session here, attached",
        hint("Claude")
    ),
    bind!(
        Device,
        [ch('x')],
        "x",
        Cmd::View("new-codex"),
        "New codex session here, attached",
        hint("Codex")
    ),
    bind!(
        Device,
        [ch('n')],
        "n",
        Cmd::View("new-shell"),
        "New shell here, attached",
        hint("Shell")
    ),
    bind!(
        Device,
        [ch(' ')],
        "Space",
        Cmd::View("peek"),
        "Peek the selected session",
        hint("Peek")
    ),
    bind!(
        Device,
        [ch('f')],
        "f",
        Cmd::View("files"),
        "Files on this device",
        hint("Files")
    ),
    bind!(
        Device,
        [ch('r')],
        "r",
        Cmd::View("retry"),
        "Retry / refresh this device",
        hint("Retry")
    ),
    bind!(
        Device,
        [code(Backspace)],
        "Backspace",
        Cmd::View("stop"),
        "Stop the selected session · Backspace again",
        hint("Stop · twice")
    ),
    bind!(
        Device,
        [ch('w')],
        "w",
        Cmd::Watch,
        "Watch selected session · read-only",
        hint("Watch · read-only"),
        Some(sessions_selected)
    ),
    bind!(
        Device,
        [ch('p')],
        "p",
        Cmd::Pin,
        "Pin / unpin; keep an ended-work bookmark",
        hint("Pin / unpin"),
        Some(sessions_selected)
    ),
    bind!(
        Device,
        [ch('t')],
        "t",
        Cmd::AgentTasks,
        "Inspect the session's tasks and subagents",
        hint("Agent tasks"),
        Some(sessions_selected)
    ),
    bind!(
        Device,
        [ch('b')],
        "b",
        Cmd::EditAccess,
        "Edit this viewer's link to the device: key, password, one-way or none",
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
        hint("Scroll"),
        Some(not_pdf_preview)
    ),
    alias!(
        Preview,
        [ch('j'), ch('k'), code(Down), code(Up)],
        "j/k",
        Cmd::Motion,
        "Change PDF page",
        hint("Pages"),
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
        hint("Links"),
        Some(markdown_preview)
    ),
    bind!(
        Preview,
        [ch('/')],
        "/",
        Cmd::PreviewSearch,
        "Search preview text, live fuzzy matches",
        hint("Search")
    ),
    bind!(
        Preview,
        [ch('n')],
        "n / N",
        Cmd::NextMatch,
        "Next / previous match, wrap around",
        hint("Next / previous")
    ),
    alias!(Preview, [ch('N')], "N", Cmd::PrevMatch, "Previous match"),
    bind!(
        Preview,
        [code(Esc), code(Left), ch('h')],
        "Esc / h",
        Cmd::Back,
        "Finish search / clear highlights / return to files",
        hint("Back")
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
        [ch('y')],
        "y",
        Cmd::View("copy-path"),
        "Copy the path to the clipboard (OSC 52)"
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
        hint("Parent / open")
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
        hint("Sessions here"),
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
        hint("Actions")
    ),
    bind!(
        Containers,
        [code(Left), ch('h')],
        "h / l",
        Cmd::Left,
        "Fold / expand",
        hint("Fold / expand")
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
        hint("Open"),
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
        hint("Session / command")
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
        hint("Actions")
    ),
    bind!(
        Global,
        [ch('?'), code(F(1))],
        "? / F1",
        Cmd::Help,
        "Keyboard help",
        hint("Help")
    ),
    bind!(
        Global,
        [ctrl('c')],
        "Ctrl+C",
        Cmd::Quit,
        "Quit cx; work keeps running",
        hint("Quit")
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
        // (`` ` `` is the hidden alias of v0.2.1's hub key, for one release.)
        [ch('H'), ch('`')],
        "H",
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
/// Whether a key is one of cx's global keys (any screen).
pub(super) fn global_key(key: &KeyEvent) -> bool {
    BINDINGS.iter().any(|b| {
        b.ctx == Ctx::Global && b.cmd != Cmd::Motion && b.keys.iter().any(|k| k.matches(key))
    })
}

/// Every key of cx's global bindings, as key events (the yazi layer's shadow test).
#[cfg(test)]
pub(super) fn global_keys() -> Vec<(KeyEvent, &'static str)> {
    BINDINGS
        .iter()
        .filter(|b| b.ctx == Ctx::Global && b.cmd != Cmd::Motion)
        .flat_map(|b| {
            b.keys.iter().map(|k| {
                let mut m = KeyModifiers::NONE;
                if k.ctrl {
                    m |= KeyModifiers::CONTROL;
                }
                if k.alt {
                    m |= KeyModifiers::ALT;
                }
                (KeyEvent::new(k.code, m), b.help)
            })
        })
        .collect()
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
        // The hub's letters are devices: only `e` (link) and `j k gg G` (motion) are
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
