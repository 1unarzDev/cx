//! The Files view as a near-exact yazi clone: yazi's default keymap (plus the user's
//! own prepends from ~/.config/yazi/keymap.toml), which-key panels after prefix keys,
//! tabs that may live on different devices, and `'` + a hub letter to move the view to
//! another device. Listings, previews and transfers are cx's (any device, through the
//! helper); this module turns keys into those actions and the App into a `View`.
pub(super) mod render;
pub(super) mod theme;
mod toml;

use super::*;
use render::{Anchor, Column, Linemode, Mark, ModeKind, Popup, Preview, Row, Status};
use theme::Theme;

/// The theme for this process (the user's yazi theme when there is one).
pub(super) fn theme() -> &'static Theme {
    static THEME: std::sync::OnceLock<Theme> = std::sync::OnceLock::new();
    THEME.get_or_init(|| {
        if cfg!(test) {
            Theme::builtin()
        } else {
            Theme::load()
        }
    })
}

/// What a key runs.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Y {
    Escape,
    Quit,
    CloseTab,
    Arrow(isize),
    /// Fraction of a page (negative: up).
    Page(i32, i32),
    Top,
    Bottom,
    Leave,
    Enter,
    Back,
    Forward,
    Seek(i32),
    Spot,
    Toggle,
    ToggleAll,
    Invert,
    Visual,
    VisualUnset,
    Open,
    OpenInteractive,
    Yank,
    Cut,
    Paste(bool),
    Unyank,
    Trash,
    Delete,
    Create,
    Rename,
    Shell(bool),
    Hidden,
    Search(bool),
    Jump(bool),
    Linemode(Linemode),
    Copy(&'static str),
    Filter,
    Find(bool),
    FindNext(bool),
    Sort(&'static str, bool),
    Goto(&'static str),
    GotoInteractive,
    Follow,
    TabNew,
    TabRename,
    TabSwitch(usize),
    TabStep(isize),
    TabSwap(isize),
    Tasks,
    Help,
    /// `'`: the device panel (handled before the table).
    Devices,
    /// A user prepend that runs a local shell command (`%s` = the chosen files).
    LocalShell(String, bool),
    /// A user prepend cx does not run (a yazi plugin): says so.
    Unsupported(String),
    /// cx's own: the send-to popup (kept from the earlier Files view).
    SendTo,
    /// cx's own: a new agent or shell session in this folder.
    NewSession,
}

pub(super) struct Bind {
    pub on: &'static [&'static str],
    pub run: Y,
    pub desc: &'static str,
}

macro_rules! b {
    ([$($k:expr),*], $run:expr, $desc:expr) => {
        Bind { on: &[$($k),*], run: $run, desc: $desc }
    };
}

/// yazi 26's `[mgr] keymap`, in its order (help and which-key list them so).
/// Differences: `H`/`L` (history) are Alt+Left/Right, as Shift+H is cx's hub key;
/// `w` (tasks) opens cx's transfers; `'` switches device; `s`/`S` run fd/rg in a panel.
pub(super) fn keymap() -> &'static [Bind] {
    use Y::*;
    static MAP: std::sync::OnceLock<Vec<Bind>> = std::sync::OnceLock::new();
    MAP.get_or_init(|| {
        vec![
            b!(
                ["<Esc>"],
                Escape,
                "Exit visual mode, clear selection, or cancel search"
            ),
            b!(
                ["<C-[>"],
                Escape,
                "Exit visual mode, clear selection, or cancel search"
            ),
            b!(["q"], Quit, "Close Files"),
            b!(["Q"], Quit, "Close Files"),
            b!(
                ["<C-c>"],
                CloseTab,
                "Close the current tab, or Files when it is the last"
            ),
            b!(["k"], Arrow(-1), "Previous file"),
            b!(["j"], Arrow(1), "Next file"),
            b!(["<Up>"], Arrow(-1), "Previous file"),
            b!(["<Down>"], Arrow(1), "Next file"),
            b!(["<C-u>"], Page(-1, 2), "Move cursor up half page"),
            b!(["<C-d>"], Page(1, 2), "Move cursor down half page"),
            b!(["<C-b>"], Page(-1, 1), "Move cursor up one page"),
            b!(["<C-f>"], Page(1, 1), "Move cursor down one page"),
            b!(["<S-PageUp>"], Page(-1, 2), "Move cursor up half page"),
            b!(["<S-PageDown>"], Page(1, 2), "Move cursor down half page"),
            b!(["<PageUp>"], Page(-1, 1), "Move cursor up one page"),
            b!(["<PageDown>"], Page(1, 1), "Move cursor down one page"),
            b!(["g", "g"], Top, "Go to top"),
            b!(["G"], Bottom, "Go to bottom"),
            b!(["<Home>"], Top, "Go to top"),
            b!(["<End>"], Bottom, "Go to bottom"),
            b!(["h"], Leave, "Back to the parent directory"),
            b!(["l"], Enter, "Enter the child directory"),
            b!(["<Left>"], Leave, "Back to the parent directory"),
            b!(["<Right>"], Enter, "Enter the child directory"),
            b!(["<A-Left>"], Back, "Back to previous directory"),
            b!(["<A-Right>"], Forward, "Forward to next directory"),
            b!(["K"], Seek(-5), "Seek up 5 units in the preview"),
            b!(["J"], Seek(5), "Seek down 5 units in the preview"),
            b!(["<Tab>"], Spot, "Spot hovered file"),
            b!(["<Space>"], Toggle, "Toggle the current selection state"),
            b!(["<C-a>"], ToggleAll, "Select all files"),
            b!(["<C-r>"], Invert, "Invert selection of all files"),
            b!(["v"], Visual, "Enter visual mode (selection mode)"),
            b!(["V"], VisualUnset, "Enter visual mode (unset mode)"),
            b!(["o"], Open, "Open selected files"),
            b!(["O"], OpenInteractive, "Open selected files interactively"),
            b!(["<Enter>"], Open, "Open selected files"),
            b!(
                ["<S-Enter>"],
                OpenInteractive,
                "Open selected files interactively"
            ),
            b!(["y"], Yank, "Yank selected files (copy)"),
            b!(["x"], Cut, "Yank selected files (cut)"),
            b!(
                ["p"],
                Paste(false),
                "Paste yanked files (here, from any device)"
            ),
            b!(
                ["P"],
                Paste(true),
                "Paste yanked files (overwrite if the destination exists)"
            ),
            b!(["Y"], Unyank, "Cancel the yank status"),
            b!(["X"], Unyank, "Cancel the yank status"),
            b!(["d"], Trash, "Trash selected files"),
            b!(["D"], Delete, "Permanently delete selected files"),
            b!(["a"], Create, "Create a file (ends with / for directories)"),
            b!(["r"], Rename, "Rename selected file(s)"),
            b!([";"], Shell(false), "Run a shell command"),
            b!(
                [":"],
                Shell(true),
                "Run a shell command (block until finishes)"
            ),
            b!(["."], Hidden, "Toggle the visibility of hidden files"),
            b!(["s"], Search(false), "Search files by name via fd"),
            b!(["S"], Search(true), "Search files by content via ripgrep"),
            b!(["<C-s>"], Escape, "Cancel the ongoing search"),
            b!(["z"], Jump(false), "Jump to a file/directory via fzf"),
            b!(["Z"], Jump(true), "Jump to a directory via zoxide"),
            b!(
                ["m", "s"],
                Linemode(render::Linemode::Size),
                "Linemode: size"
            ),
            b!(
                ["m", "p"],
                Linemode(render::Linemode::Permissions),
                "Linemode: permissions"
            ),
            b!(
                ["m", "m"],
                Linemode(render::Linemode::Mtime),
                "Linemode: mtime"
            ),
            b!(
                ["m", "b"],
                Linemode(render::Linemode::Bar),
                "Linemode: size bar and day"
            ),
            b!(
                ["m", "n"],
                Linemode(render::Linemode::None),
                "Linemode: none"
            ),
            b!(["c", "c"], Copy("path"), "Copy file path"),
            b!(["c", "C"], Copy("url"), "Copy file URL"),
            b!(["c", "d"], Copy("dirpath"), "Copy directory path"),
            b!(["c", "D"], Copy("dirurl"), "Copy directory URL"),
            b!(["c", "f"], Copy("filename"), "Copy filename"),
            b!(
                ["c", "n"],
                Copy("name_without_ext"),
                "Copy filename without extension"
            ),
            b!(["f"], Filter, "Filter files"),
            b!(["/"], Find(false), "Find next file"),
            b!(["?"], Find(true), "Find previous file"),
            b!(["n"], FindNext(false), "Next found"),
            b!(["N"], FindNext(true), "Previous found"),
            b!([",", "m"], Sort("mtime", false), "Sort by modified time"),
            b!(
                [",", "M"],
                Sort("mtime", true),
                "Sort by modified time (reverse)"
            ),
            b!([",", "b"], Sort("mtime", false), "Sort by birth time"),
            b!(
                [",", "B"],
                Sort("mtime", true),
                "Sort by birth time (reverse)"
            ),
            b!([",", "e"], Sort("extension", false), "Sort by extension"),
            b!(
                [",", "E"],
                Sort("extension", true),
                "Sort by extension (reverse)"
            ),
            b!(
                [",", "a"],
                Sort("alphabetical", false),
                "Sort alphabetically"
            ),
            b!(
                [",", "A"],
                Sort("alphabetical", true),
                "Sort alphabetically (reverse)"
            ),
            b!([",", "n"], Sort("natural", false), "Sort naturally"),
            b!(
                [",", "N"],
                Sort("natural", true),
                "Sort naturally (reverse)"
            ),
            b!([",", "s"], Sort("size", false), "Sort by size"),
            b!([",", "S"], Sort("size", true), "Sort by size (reverse)"),
            b!([",", "r"], Sort("random", false), "Sort randomly"),
            b!(["g", "h"], Goto("~"), "Go home"),
            b!(["g", "c"], Goto("~/.config"), "Go to ~/.config"),
            b!(["g", "d"], Goto("~/Downloads"), "Go to ~/Downloads"),
            b!(
                ["g", "t"],
                Goto("~/.local/share/Trash/files"),
                "Go to trash bin"
            ),
            b!(["g", "<Space>"], GotoInteractive, "Jump interactively"),
            b!(["g", "f"], Follow, "Follow hovered symlink"),
            b!(["t", "t"], TabNew, "Create a new tab with CWD"),
            b!(["t", "r"], TabRename, "Rename current tab"),
            b!(["1"], TabSwitch(0), "Switch to first tab"),
            b!(["2"], TabSwitch(1), "Switch to second tab"),
            b!(["3"], TabSwitch(2), "Switch to third tab"),
            b!(["4"], TabSwitch(3), "Switch to fourth tab"),
            b!(["5"], TabSwitch(4), "Switch to fifth tab"),
            b!(["6"], TabSwitch(5), "Switch to sixth tab"),
            b!(["7"], TabSwitch(6), "Switch to seventh tab"),
            b!(["8"], TabSwitch(7), "Switch to eighth tab"),
            b!(["9"], TabSwitch(8), "Switch to ninth tab"),
            b!(["["], TabStep(-1), "Switch to previous tab"),
            b!(["]"], TabStep(1), "Switch to next tab"),
            b!(["{"], TabSwap(-1), "Swap current tab with previous tab"),
            b!(["}"], TabSwap(1), "Swap current tab with next tab"),
            b!(["w"], Tasks, "Show task manager (transfers)"),
            b!(["~"], Help, "Open help"),
            b!(["<F1>"], Help, "Open help"),
            b!(
                ["'"],
                Devices,
                "Switch this tab to another device (its hub letter)"
            ),
            b!(["e"], SendTo, "Send selected files to a device (picker)"),
            b!(["T"], NewSession, "New session in this folder"),
        ]
    })
}

/// The user's `[[mgr.prepend_keymap]]` rows cx can run (shown first in help).
fn prepends() -> &'static [(Vec<String>, Y, String)] {
    static P: std::sync::OnceLock<Vec<(Vec<String>, Y, String)>> = std::sync::OnceLock::new();
    P.get_or_init(|| {
        theme()
            .prepends
            .iter()
            .map(|(on, run, desc)| {
                let run = run.trim();
                let y = if run == "open --interactive" {
                    Y::OpenInteractive
                } else if let Some(cmd) = run.strip_prefix("shell") {
                    let cmd = cmd.trim();
                    let block = cmd.starts_with("--block");
                    let cmd = cmd.trim_start_matches("--block").trim();
                    let cmd = cmd.strip_prefix("--").unwrap_or(cmd).trim();
                    let cmd = cmd.trim_matches(|c| c == '\'' || c == '"');
                    Y::LocalShell(cmd.to_owned(), block)
                } else {
                    Y::Unsupported(desc.clone())
                };
                (on.clone(), y, desc.clone())
            })
            .collect()
    })
}

/// A key in yazi's notation (`j`, `<C-u>`, `<Space>`, `<A-Left>`).
pub(super) fn key_name(k: &KeyEvent) -> Option<String> {
    let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
    let alt = k.modifiers.contains(KeyModifiers::ALT);
    let shift = k.modifiers.contains(KeyModifiers::SHIFT);
    let named = |n: &str| {
        let mut p = String::new();
        if ctrl {
            p.push_str("C-");
        }
        if alt {
            p.push_str("A-");
        }
        if shift {
            p.push_str("S-");
        }
        format!("<{p}{n}>")
    };
    Some(match k.code {
        KeyCode::Char(' ') if !ctrl && !alt => "<Space>".into(),
        KeyCode::Char(c) if ctrl || alt => {
            let mut p = String::new();
            if ctrl {
                p.push_str("C-");
            }
            if alt {
                p.push_str("A-");
            }
            format!("<{p}{}>", c.to_ascii_lowercase())
        }
        KeyCode::Char(c) => c.to_string(),
        KeyCode::Enter => named("Enter"),
        KeyCode::Esc => named("Esc"),
        KeyCode::Tab => named("Tab"),
        KeyCode::BackTab => "<BackTab>".into(),
        KeyCode::Backspace => named("Backspace"),
        KeyCode::Delete => named("Delete"),
        KeyCode::Up => named("Up"),
        KeyCode::Down => named("Down"),
        KeyCode::Left => named("Left"),
        KeyCode::Right => named("Right"),
        KeyCode::Home => named("Home"),
        KeyCode::End => named("End"),
        KeyCode::PageUp => named("PageUp"),
        KeyCode::PageDown => named("PageDown"),
        KeyCode::F(n) => named(&format!("F{n}")),
        _ => return None,
    })
}

#[derive(Clone, Debug)]
pub(super) struct Tab {
    pub device: usize,
    pub path: String,
    pub scope: Option<ContainerScope>,
    pub name: Option<String>,
}

#[derive(Clone)]
pub(super) enum YPop {
    Help {
        selected: usize,
        filter: String,
        typing: bool,
    },
    Confirm {
        permanent: bool,
        device: usize,
        entries: Vec<Entry>,
    },
    /// Openers for the hovered file (local), or facts (Tab spot).
    Pick {
        title: String,
        items: Vec<(String, Option<(String, bool)>)>,
        selected: usize,
    },
}

/// The sort the user chose with `,` (None: the helper's order, dirs first A-Z).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Sort {
    pub by: &'static str,
    pub reverse: bool,
}

#[derive(Default)]
pub(super) struct Yazi {
    /// Keys of an unfinished chord (the which-key panel shows its candidates).
    pub pending: Vec<String>,
    pub unset: bool,
    pub tabs: Vec<Tab>,
    pub tab: usize,
    /// The device and folder `''` returns to.
    pub previous: Option<(usize, String)>,
    pub linemode: Linemode,
    pub sort: Option<Sort>,
    pub popup: Option<YPop>,
    /// `/` or `?`: the query and its direction (n repeats it).
    pub find: Option<(String, bool)>,
    pub back: Vec<(usize, String)>,
    pub forward: Vec<(usize, String)>,
    /// The tab being renamed (`t r` reuses the rename input).
    pub renaming_tab: bool,
    /// `a`: the create input is open.
    pub creating: bool,
    /// `g <Space>`: the cd input is open.
    pub cd: bool,
    /// `s`/`S`: a search input is open (true: by content).
    pub search: Option<bool>,
    /// Rows of the current list at the last draw (page moves).
    pub rows: std::cell::Cell<usize>,
    /// Devices whose hover channel has been opened.
    pub warm: BTreeSet<usize>,
    /// The preview column in pixels at the last draw (fitted image previews).
    pub preview_px: std::cell::Cell<(u32, u32)>,
    /// The preview column in cells at the last draw (native image placement).
    pub preview_rect: std::cell::Cell<Rect>,
}

fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

impl App {
    /// Whether keys go to the yazi layer: Files, its list (not an opened preview), no
    /// dialog, input or send-to picker.
    pub(super) fn yazi_active(&self) -> bool {
        self.view == View::Files
            && self.browser.as_ref().is_some_and(|b| b.preview.is_none())
            && self.dialog.is_none()
            && self.input.is_none()
            && !self.send_pick
            && self.command_panel.is_none()
    }

    fn hovered(&self) -> Option<Entry> {
        let b = self.browser.as_ref()?;
        browser_entries(b).get(b.selected).cloned()
    }

    fn chord_candidates(&self, seq: &[String]) -> Vec<(String, String)> {
        if seq.first().map(String::as_str) == Some("'") && seq.len() == 1 {
            let letters = self.hotkeys();
            let mut out: Vec<(String, String)> = self
                .devices
                .iter()
                .enumerate()
                .filter_map(|(i, d)| {
                    let l = *letters.get(&d.id)?;
                    let here = self.browser.as_ref().is_some_and(|b| b.device == i);
                    Some((
                        l.to_string(),
                        format!(
                            "{}{}",
                            safe_label(&d.name),
                            if here { " (here)" } else { "" }
                        ),
                    ))
                })
                .collect();
            out.sort();
            if let Some((d, _)) = &self.yz.previous {
                if let Some(dev) = self.devices.get(*d) {
                    out.insert(
                        0,
                        ("'".into(), format!("Back to {}", safe_label(&dev.name))),
                    );
                }
            }
            return out;
        }
        let mut seen = BTreeSet::new();
        let pre = prepends()
            .iter()
            .map(|(on, _, desc)| (on.as_slice(), desc.as_str()));
        let map = keymap().iter().map(|b| (b.on, b.desc));
        let mut out = Vec::new();
        let mut push = |on: &[&str], desc: &str| {
            if on.len() > seq.len() && on.iter().zip(seq).all(|(a, b)| a == b) {
                let next = on[seq.len()].to_string();
                if seen.insert(next.clone()) {
                    out.push((next, desc.to_string()));
                }
            }
        };
        for (on, desc) in pre {
            let on: Vec<&str> = on.iter().map(String::as_str).collect();
            push(&on, desc);
        }
        for (on, desc) in map {
            push(on, desc);
        }
        out
    }

    /// The binding for a complete key sequence (user prepends win).
    fn binding(seq: &[String]) -> Option<Y> {
        for (on, y, _) in prepends() {
            if on.as_slice() == seq {
                return Some(y.clone());
            }
        }
        keymap()
            .iter()
            .find(|b| b.on.len() == seq.len() && b.on.iter().zip(seq).all(|(a, b)| a == b))
            .map(|b| b.run.clone())
    }

    /// A key in the Files list. Returns false when yazi has nothing for it (cx's own
    /// keys then apply: the palette, the hub key, Shift+H…).
    pub(super) fn yazi_key(&mut self, key: KeyEvent) -> bool {
        if self.yz.popup.is_some() {
            self.yazi_popup_key(key);
            return true;
        }
        let Some(name) = key_name(&key) else {
            return false;
        };
        let mut seq = std::mem::take(&mut self.yz.pending);
        let chord = !seq.is_empty();
        seq.push(name.clone());
        // `'` + a hub letter: this tab moves to that device; `''` goes back.
        if seq.len() == 2 && seq[0] == "'" {
            if let KeyCode::Char(c) = key.code {
                if c == '\'' {
                    if let Some((d, path)) = self.yz.previous.clone() {
                        self.yazi_switch_device(d, Some(path));
                    }
                } else if let Some(d) = self.device_for_letter(c) {
                    self.yazi_switch_device(d, None);
                }
            }
            return true;
        }
        if seq.len() == 1 && seq[0] == "'" {
            self.yz.pending = seq;
            return true;
        }
        if let Some(run) = Self::binding(&seq) {
            // A longer chord sharing this prefix waits (yazi has none for single keys).
            self.yazi_run(run);
            return true;
        }
        if !self.chord_candidates(&seq).is_empty() {
            self.yz.pending = seq;
            return true;
        }
        // Not ours: a pending chord is just cancelled; a lone key goes to cx.
        chord
    }

    fn yazi_popup_key(&mut self, key: KeyEvent) {
        let Some(pop) = self.yz.popup.clone() else {
            return;
        };
        match pop {
            YPop::Confirm {
                permanent,
                device,
                entries,
            } => {
                self.yz.popup = None;
                if matches!(key.code, KeyCode::Char('y' | 'Y') | KeyCode::Enter) {
                    let ops = entries
                        .into_iter()
                        .map(|e| {
                            (
                                device,
                                if permanent {
                                    Operation::Remove {
                                        path: e.path,
                                        expected_identity: e.identity,
                                    }
                                } else {
                                    Operation::Trash {
                                        path: e.path,
                                        expected_identity: e.identity,
                                    }
                                },
                            )
                        })
                        .collect();
                    self.queue_file_actions(ops);
                    if let Some(b) = self.browser.as_mut() {
                        b.marked.clear();
                    }
                }
            }
            YPop::Help {
                selected,
                mut filter,
                typing,
            } => {
                let rows = self.yazi_help_rows(&filter).len();
                let mut sel = selected;
                let mut typing = typing;
                match key.code {
                    KeyCode::Esc if typing => typing = false,
                    KeyCode::Enter if typing => typing = false,
                    KeyCode::Backspace if typing => {
                        filter.pop();
                    }
                    KeyCode::Char(c) if typing => filter.push(c),
                    KeyCode::Esc | KeyCode::Char('q' | '~') | KeyCode::F(1) => {
                        self.yz.popup = None;
                        return;
                    }
                    KeyCode::Char('/') => typing = true,
                    KeyCode::Char('j') | KeyCode::Down => sel += 1,
                    KeyCode::Char('k') | KeyCode::Up => sel = sel.saturating_sub(1),
                    KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        sel += 10
                    }
                    KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        sel = sel.saturating_sub(10)
                    }
                    KeyCode::PageDown => sel += 20,
                    KeyCode::PageUp => sel = sel.saturating_sub(20),
                    KeyCode::Char('G') | KeyCode::End => sel = usize::MAX,
                    KeyCode::Home => sel = 0,
                    _ => {}
                }
                let rows = self.yazi_help_rows(&filter).len().max(rows.min(1));
                self.yz.popup = Some(YPop::Help {
                    selected: sel.min(rows.saturating_sub(1)),
                    filter,
                    typing,
                });
            }
            YPop::Pick {
                title,
                items,
                selected,
            } => match key.code {
                KeyCode::Esc | KeyCode::Char('q') => self.yz.popup = None,
                KeyCode::Char('j') | KeyCode::Down => {
                    self.yz.popup = Some(YPop::Pick {
                        selected: (selected + 1).min(items.len().saturating_sub(1)),
                        title,
                        items,
                    })
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    self.yz.popup = Some(YPop::Pick {
                        selected: selected.saturating_sub(1),
                        title,
                        items,
                    })
                }
                KeyCode::Enter | KeyCode::Char('o' | 'l') => {
                    self.yz.popup = None;
                    if let Some((_, Some((cmd, block)))) = items.get(selected).cloned() {
                        self.run_opener(cmd, block);
                    } else if items.get(selected).is_some_and(|(l, _)| l == "Preview") {
                        if let Some(e) = self.hovered() {
                            let d = self.browser.as_ref().map_or(0, |b| b.device);
                            self.open_preview(d, e.path);
                        }
                    }
                }
                _ => {}
            },
        }
    }

    /// Help rows: user prepends first, then the keymap, filtered.
    pub(super) fn yazi_help_rows(&self, filter: &str) -> Vec<(String, String)> {
        let f = filter.to_lowercase();
        prepends()
            .iter()
            .map(|(on, _, desc)| (on.concat(), desc.clone()))
            .chain(keymap().iter().map(|b| (b.on.concat(), b.desc.to_string())))
            .filter(|(k, d)| {
                f.is_empty() || k.to_lowercase().contains(&f) || d.to_lowercase().contains(&f)
            })
            .collect()
    }

    fn move_cursor(&mut self, to: impl FnOnce(usize, usize) -> usize) {
        let len = self.visible_entries().len();
        if len == 0 {
            return;
        }
        let current = self.browser.as_ref().map_or(0, |b| b.selected);
        let target = to(current, len).min(len - 1) as isize;
        self.move_selection(target - current as isize);
    }

    fn body_rows(&self) -> usize {
        self.yz.rows.get().max(1)
    }

    fn yazi_run(&mut self, run: Y) {
        use Y::*;
        match run {
            Escape => {
                let b = self.browser.as_mut();
                if let Some(b) = b {
                    if b.visual_anchor.is_some() {
                        b.visual_anchor = None;
                        b.visual_base.clear();
                        self.yz.unset = false;
                    } else if !b.marked.is_empty() {
                        b.marked.clear();
                    } else if self.yz.find.is_some() || !b.search.is_empty() {
                        self.yz.find = None;
                        b.search.clear();
                    } else if !b.filter.is_empty() {
                        b.filter.clear();
                    } else {
                        self.back();
                    }
                }
            }
            Quit => self.back(),
            CloseTab => {
                if self.yz.tabs.len() > 1 {
                    self.yz.tabs.remove(self.yz.tab);
                    self.yz.tab = self.yz.tab.min(self.yz.tabs.len() - 1);
                    self.yazi_show_tab();
                } else {
                    self.back();
                }
            }
            Arrow(n) => self.move_selection(n),
            Page(dir, div) => {
                let n = (self.body_rows() / div as usize).max(1) as isize;
                self.move_selection(n * dir as isize);
            }
            Top => self.move_cursor(|_, _| 0),
            Bottom => self.move_cursor(|_, len| len - 1),
            Leave => {
                if let Some(b) = &self.browser {
                    let here = (b.device, b.path.clone());
                    if b.parent.is_some() {
                        self.yz.back.push(here);
                        self.yz.forward.clear();
                    }
                }
                self.parent_directory();
            }
            Enter => {
                if let Some(e) = self.hovered().filter(|e| e.kind == "directory") {
                    self.yazi_cd_entry(e.path);
                }
            }
            Back | Forward => {
                let (from, to) = if run == Back {
                    (&mut self.yz.back, &mut self.yz.forward)
                } else {
                    (&mut self.yz.forward, &mut self.yz.back)
                };
                if let (Some((d, path)), Some(b)) = (from.pop(), self.browser.as_ref()) {
                    to.push((b.device, b.path.clone()));
                    self.open_browser(d, path);
                }
            }
            Seek(n) => {
                if let Some(p) = self.browser.as_ref().and_then(|b| b.peek.as_ref()) {
                    let s = (i64::from(p.preview_scroll.get()) + i64::from(n)).max(0) as u32;
                    p.preview_scroll.set(s.min(doc_max_scroll(p)));
                }
            }
            Spot => self.yazi_spot(),
            Toggle => self.execute(Action::Select),
            ToggleAll | Invert => {
                if let Some(b) = self.browser.as_mut() {
                    let rows: Vec<String> =
                        browser_entries(b).iter().map(|e| e.path.clone()).collect();
                    b.visual_anchor = None;
                    for p in rows {
                        if run == Invert && b.marked.contains(&p) {
                            b.marked.remove(&p);
                        } else if b.marked.len() < 4096 {
                            b.marked.insert(p);
                        }
                    }
                }
            }
            Visual | VisualUnset => {
                self.yz.unset = run == VisualUnset;
                if let Some(b) = self.browser.as_mut() {
                    if b.visual_anchor.take().is_none() {
                        b.visual_anchor = Some(b.selected);
                        b.visual_base = b.marked.clone();
                        if let Some(e) = browser_entries(b).get(b.selected).cloned() {
                            if self.yz.unset {
                                b.marked.remove(&e.path);
                            } else {
                                b.marked.insert(e.path);
                            }
                        }
                    } else {
                        b.visual_base.clear();
                        self.yz.unset = false;
                    }
                }
            }
            Open => {
                if self.destination_active && self.clipboard.is_some() {
                    // The send-to flow's destination: Enter transfers here.
                    self.primary();
                    return;
                }
                let Some(e) = self.hovered() else { return };
                if e.kind == "directory" && self.chosen_entries().len() <= 1 {
                    self.yazi_cd_entry(e.path);
                } else {
                    self.yazi_open(false);
                }
            }
            OpenInteractive => self.yazi_open(true),
            // The header's count shows the yank (yazi says nothing more).
            Yank => {
                self.execute(Action::Copy);
                self.set_notice(String::new());
            }
            Cut => {
                self.execute(Action::Cut);
                self.set_notice(String::new());
            }
            Paste(force) => {
                if self.clipboard.is_none() {
                    return;
                }
                // p never replaces: an existing name gets a numbered copy (yazi's); P
                // overwrites.
                let before = self.conflict;
                self.conflict = if force { 1 } else { 2 };
                self.submit_transfer();
                self.conflict = before;
                self.transfer_drawer = false;
            }
            Unyank => {
                self.clipboard = None;
            }
            Trash | Delete => {
                let entries = self.chosen_entries();
                let Some(b) = self.browser.as_ref() else {
                    return;
                };
                if entries.is_empty() {
                    return;
                }
                if entries.iter().any(|e| e.identity.is_none()) {
                    self.set_notice_as(
                        NoticeKind::Warning,
                        "Still loading this folder · try again in a moment".into(),
                    );
                    return;
                }
                let permanent = run == Delete || b.container.is_some();
                self.yz.popup = Some(YPop::Confirm {
                    permanent,
                    device: b.device,
                    entries,
                });
                self.finish_visual();
            }
            Create => {
                self.yz.creating = true;
                self.execute(Action::Mkdir);
            }
            Rename => self.execute(Action::Rename),
            Shell(_) => self.execute(Action::Command),
            Hidden => self.execute(Action::Hidden),
            Search(content) => {
                self.yz.search = Some(content);
                self.input = Some(Input::Mkdir);
                self.text.clear();
                self.rename_cursor = 0;
            }
            Jump(zoxide) => {
                let local = self
                    .browser
                    .as_ref()
                    .is_some_and(|b| self.devices[b.device].target.is_none());
                if !local {
                    self.set_notice("z and Z jump on this device only · ' switches devices".into());
                    return;
                }
                let tool = if zoxide { "zoxide" } else { "fzf" };
                if !command_exists(tool) {
                    self.set_notice_as(
                        NoticeKind::Warning,
                        format!("{tool} is not installed here"),
                    );
                    return;
                }
                let dir = self
                    .browser
                    .as_ref()
                    .map(|b| b.path.clone())
                    .unwrap_or_default();
                self.pending_jump = Some((zoxide, dir));
            }
            Linemode(m) => self.yz.linemode = m,
            Copy(what) => self.yazi_copy(what),
            Filter => self.execute(Action::Filter),
            Find(prev) => {
                self.yz.find = Some((String::new(), prev));
                self.input = Some(Input::Search);
                self.text.clear();
                self.rename_cursor = 0;
            }
            FindNext(prev) => {
                let dir = self.yz.find.as_ref().map_or(prev, |(_, p)| *p != prev);
                self.next_file_match(if dir { -1 } else { 1 });
            }
            Sort(by, reverse) => {
                self.yz.sort = Some(self::Sort { by, reverse });
                self.yazi_sort();
            }
            Goto(path) => {
                if let Some(d) = self.browser.as_ref().map(|b| b.device) {
                    self.yazi_cd(d, path.to_string());
                }
            }
            GotoInteractive => {
                self.yz.cd = true;
                self.input = Some(Input::Mkdir);
                self.text = self
                    .browser
                    .as_ref()
                    .map(|b| b.display_path.clone())
                    .unwrap_or_default();
                self.rename_cursor = self.text.len();
            }
            Follow => {
                if let Some(e) = self.hovered().filter(|e| e.kind == "symlink") {
                    let d = self.browser.as_ref().map_or(0, |b| b.device);
                    self.yazi_cd(d, e.path);
                }
            }
            TabNew => {
                self.yazi_sync_tab();
                if self.yz.tabs.len() < 9 {
                    let t = self.yz.tabs[self.yz.tab].clone();
                    self.yz
                        .tabs
                        .insert(self.yz.tab + 1, Tab { name: None, ..t });
                    self.yz.tab += 1;
                }
            }
            TabRename => {
                self.yazi_sync_tab();
                self.yz.renaming_tab = true;
                self.input = Some(Input::Rename);
                self.text = self.yz.tabs[self.yz.tab].name.clone().unwrap_or_default();
                self.rename_cursor = self.text.len();
            }
            TabSwitch(n) => {
                self.yazi_sync_tab();
                if n < self.yz.tabs.len() && n != self.yz.tab {
                    self.yz.tab = n;
                    self.yazi_show_tab();
                }
            }
            TabStep(d) => {
                self.yazi_sync_tab();
                let n = self.yz.tabs.len() as isize;
                if n > 1 {
                    self.yz.tab = (self.yz.tab as isize + d).rem_euclid(n) as usize;
                    self.yazi_show_tab();
                }
            }
            TabSwap(d) => {
                self.yazi_sync_tab();
                let n = self.yz.tabs.len() as isize;
                let to = (self.yz.tab as isize + d).rem_euclid(n) as usize;
                self.yz.tabs.swap(self.yz.tab, to);
                self.yz.tab = to;
            }
            Tasks => self.execute(Action::Jobs),
            Help => {
                self.yz.popup = Some(YPop::Help {
                    selected: 0,
                    filter: String::new(),
                    typing: false,
                })
            }
            Devices => {}
            LocalShell(cmd, block) => {
                let local = self
                    .browser
                    .as_ref()
                    .is_some_and(|b| self.devices[b.device].target.is_none());
                let program = cmd.split_whitespace().next().unwrap_or("").to_owned();
                if !local {
                    self.set_notice(format!("{program} runs on this device only"));
                } else if !command_exists(&program) {
                    self.set_notice_as(
                        NoticeKind::Warning,
                        format!("{program} is not installed here"),
                    );
                } else {
                    self.run_opener(cmd, block);
                }
            }
            Unsupported(desc) => {
                self.set_notice(format!("{desc} is a yazi plugin · not in cx"));
            }
            SendTo => self.send_pick = true,
            NewSession => self.session_shortcut(),
        }
    }

    /// After `z`/`Z`: go to the folder picked (a file: its folder, hovering it).
    pub(super) fn yazi_jump_to(&mut self, from: &str, picked: &str) {
        let base = crate::files::decode_path(from).unwrap_or_default();
        let path = base.join(picked);
        let local = self.local_index();
        if path.is_dir() {
            self.yazi_cd(local, path.to_string_lossy().into_owned());
        } else if let Some(parent) = path.parent() {
            self.yazi_cd(local, parent.to_string_lossy().into_owned());
            if let Some(b) = self.browser.as_mut() {
                b.restore_selection = Some(crate::files::encode_path(&path));
            }
        }
    }
    /// Move into a folder entry, recording history.
    fn yazi_cd_entry(&mut self, path: String) {
        if let Some(b) = &self.browser {
            let d = b.device;
            self.yz.back.push((d, b.path.clone()));
            self.yz.forward.clear();
            self.open_browser(d, path);
        }
    }
    fn yazi_cd(&mut self, device: usize, path: String) {
        if let Some(b) = &self.browser {
            self.yz.back.push((b.device, b.path.clone()));
            self.yz.forward.clear();
        }
        self.open_browser(device, path);
    }

    /// Keep the active tab's record in step with the browser.
    pub(super) fn yazi_sync_tab(&mut self) {
        let Some(b) = self.browser.as_ref() else {
            return;
        };
        let here = Tab {
            device: b.device,
            path: b.path.clone(),
            scope: b.container.clone(),
            name: None,
        };
        if self.yz.tabs.is_empty() {
            self.yz.tabs.push(here);
            self.yz.tab = 0;
        } else {
            let i = self.yz.tab.min(self.yz.tabs.len() - 1);
            let t = &mut self.yz.tabs[i];
            t.device = here.device;
            t.path = here.path;
            t.scope = here.scope;
        }
    }
    fn yazi_show_tab(&mut self) {
        if let Some(t) = self.yz.tabs.get(self.yz.tab).cloned() {
            self.open_scoped_browser(t.device, t.path, t.scope);
        }
    }

    /// `'` + letter: this tab shows device `d` at its last folder (or `path`).
    pub(super) fn yazi_switch_device(&mut self, d: usize, path: Option<String>) {
        let Some(b) = self.browser.as_ref() else {
            return;
        };
        if b.device == d && path.as_deref().is_none_or(|p| p == b.path) {
            return;
        }
        self.yz.previous = Some((b.device, b.path.clone()));
        let path = path
            .or_else(|| self.file_locations.get(&d).cloned())
            .unwrap_or_else(|| "~".into());
        let path = self.cached_folder(d, &path).unwrap_or(path);
        self.yazi_sync_tab();
        self.open_host_browser(d, path);
        self.yazi_sync_tab();
    }

    /// On entering Files: list every other reachable device's last folder in the
    /// background, so its helper channel is warm and `'` + its letter paints from memory.
    pub(super) fn yazi_warm_devices(&mut self) {
        let here = self.browser.as_ref().map(|b| b.device);
        for d in 0..self.devices.len() {
            // Not a device whose last check failed (no stray connection attempts).
            if Some(d) == here || self.work.get(d).is_some_and(|w| w.error.is_some()) {
                continue;
            }
            let path = self
                .file_locations
                .get(&d)
                .cloned()
                .unwrap_or_else(|| "~".into());
            if self.cached_folder(d, &path).is_none() {
                self.send_warm(d, Operation::List { path: path.clone() }, false);
            }
            self.yazi_warm_hover(d, path);
        }
    }
    /// Open device `d`'s hover channel (one per device, in parallel with its listing):
    /// a first hover read must not pay for the channel's SSH session and helper start.
    pub(super) fn yazi_warm_hover(&mut self, d: usize, path: String) {
        if self.devices[d].target.is_none() || !self.yz.warm.insert(d) {
            return;
        }
        // The parent when the path names it (it is the next thing the parent column wants).
        let target = path
            .strip_prefix('/')
            .and_then(|_| path.trim_end_matches('/').rsplit_once('/'))
            .map(|(p, _)| {
                if p.is_empty() {
                    "/".to_owned()
                } else {
                    p.to_owned()
                }
            })
            .unwrap_or(path);
        self.send_warm(d, Operation::List { path: target }, true);
    }
    /// The cache key of device `d`'s folder `path` (`~` matches the home it listed).
    fn cached_folder(&self, d: usize, path: &str) -> Option<String> {
        let scope = browser_scope_key(None);
        if self
            .browser_cache
            .contains_key(&(d, path.to_owned(), scope.clone()))
        {
            return Some(path.to_owned());
        }
        (path == "~")
            .then(|| {
                self.browser_cache
                    .iter()
                    .find(|((dev, _, s), b)| *dev == d && *s == scope && b.display_path == "~")
                    .map(|((_, p, _), _)| p.clone())
            })
            .flatten()
    }

    /// Apply the chosen `,` sort to the loaded rows (pages arriving later re-sort).
    pub(super) fn yazi_sort(&mut self) {
        let Some(sort) = self.yz.sort else { return };
        let Some(b) = self.browser.as_mut() else {
            return;
        };
        let keep = browser_entries(b).get(b.selected).map(|e| e.path.clone());
        let natural = |s: &str| -> Vec<(u8, u128, String)> {
            // Digit runs compare as numbers.
            let mut out = Vec::new();
            let mut num = String::new();
            let mut text = String::new();
            for c in s.chars() {
                if c.is_ascii_digit() {
                    if !text.is_empty() {
                        out.push((1, 0, std::mem::take(&mut text)));
                    }
                    num.push(c);
                } else {
                    if !num.is_empty() {
                        out.push((0, num.parse().unwrap_or(u128::MAX), String::new()));
                        num.clear();
                    }
                    text.extend(c.to_lowercase());
                }
            }
            if !num.is_empty() {
                out.push((0, num.parse().unwrap_or(u128::MAX), String::new()));
            }
            if !text.is_empty() {
                out.push((1, 0, text));
            }
            out
        };
        let mut seed: u64 = transport::now() | 1;
        b.entries.sort_by(|x, y| {
            let dirs = (x.kind != "directory").cmp(&(y.kind != "directory"));
            let ord = match sort.by {
                "mtime" => x.modified.cmp(&y.modified),
                "size" => x.size.cmp(&y.size),
                "extension" => {
                    let ext = |n: &str| {
                        n.rsplit_once('.')
                            .map(|(_, e)| e.to_lowercase())
                            .unwrap_or_default()
                    };
                    ext(&x.name)
                        .cmp(&ext(&y.name))
                        .then_with(|| x.name.to_lowercase().cmp(&y.name.to_lowercase()))
                }
                "natural" => natural(&x.name).cmp(&natural(&y.name)),
                "random" => {
                    seed ^= seed << 13;
                    seed ^= seed >> 7;
                    seed ^= seed << 17;
                    (seed % 3).cmp(&1)
                }
                _ => x.name.to_lowercase().cmp(&y.name.to_lowercase()),
            };
            dirs.then(if sort.reverse { ord.reverse() } else { ord })
        });
        if let Some(keep) = keep {
            b.selected = browser_entries(b).position_of(&keep).unwrap_or(0);
        }
    }

    /// `c` + c/C/d/D/f/n: copy through OSC 52 (works over SSH and inside tmux).
    fn yazi_copy(&mut self, what: &str) {
        let Some(b) = self.browser.as_ref() else {
            return;
        };
        let chosen = self.chosen_entries();
        let decode = |p: &str| {
            crate::files::decode_path(p)
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|_| p.to_owned())
        };
        let dir = decode(&b.path);
        let url = |p: &str| {
            let mut out = String::from("file://");
            for &c in p.as_bytes() {
                if c.is_ascii_alphanumeric() || b"/-_.~".contains(&c) {
                    out.push(c as char);
                } else {
                    out.push_str(&format!("%{c:02X}"));
                }
            }
            out
        };
        let lines: Vec<String> = match what {
            "dirpath" => vec![dir.clone()],
            "dirurl" => vec![url(&dir)],
            _ => chosen
                .iter()
                .map(|e| {
                    let full = decode(&e.path);
                    match what {
                        "url" => url(&full),
                        "filename" => e.name.clone(),
                        "name_without_ext" => e
                            .name
                            .rsplit_once('.')
                            .filter(|(stem, _)| !stem.is_empty())
                            .map_or(e.name.clone(), |(s, _)| s.to_owned()),
                        _ => full,
                    }
                })
                .collect(),
        };
        if lines.is_empty() {
            return;
        }
        let what = match what {
            "dirpath" | "dirurl" => "the directory path",
            "filename" | "name_without_ext" => "the name",
            _ => "the path",
        };
        self.copy_text(what, lines.join("\n"));
    }

    /// Tab: the hovered file's facts (yazi's spotter).
    fn yazi_spot(&mut self) {
        let Some(e) = self.hovered() else { return };
        let full = crate::files::decode_path(&e.path)
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|_| e.path.clone());
        let mut items = vec![
            (format!("Path  {}", safe_label(&full)), None),
            (format!("Kind  {}", e.kind), None),
            (format!("Size  {}", render::readable_size(e.size)), None),
        ];
        if let Some(m) = e.mode {
            items.push((
                format!(
                    "Mode  {}",
                    render::perm_string(m, e.kind == "directory", e.kind == "symlink")
                ),
                None,
            ));
        }
        if let Some(t) = e.modified {
            items.push((
                format!("Modified  {}", render::day(t, transport::now() as i64)),
                None,
            ));
        }
        self.yz.popup = Some(YPop::Pick {
            title: safe_label(&e.name),
            items,
            selected: 0,
        });
    }

    /// Enter / o on a file. Locally the user's yazi.toml `[open] rules` decide (the
    /// first opener, or a chooser); on another device the file opens in cx's preview.
    fn yazi_open(&mut self, interactive: bool) {
        let Some(b) = self.browser.as_ref() else {
            return;
        };
        let device = b.device;
        let Some(e) = self.hovered() else { return };
        let local = self.devices[device].target.is_none() && b.container.is_none();
        // Without the user's yazi.toml, Enter opens cx's own preview (any device).
        if !local || (!interactive && !openers::configured()) {
            if e.kind != "directory" {
                self.open_preview(device, e.path);
            }
            return;
        }
        let openers = openers::for_file(&e.name, e.kind == "directory");
        let paths: Vec<String> = self
            .chosen_entries()
            .iter()
            .map(|e| {
                crate::files::decode_path(&e.path)
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_else(|_| e.path.clone())
            })
            .collect();
        let dir = crate::files::decode_path(&b.path)
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        let fill = |run: &str| openers::fill(run, &paths, &dir);
        if interactive || openers.is_empty() {
            let mut items: Vec<(String, Option<(String, bool)>)> = openers
                .iter()
                .map(|o| (o.desc.clone(), Some((fill(&o.run), o.block))))
                .collect();
            items.push(("Preview".into(), None));
            self.yz.popup = Some(YPop::Pick {
                title: "Open with:".into(),
                items,
                selected: 0,
            });
        } else {
            let o = &openers[0];
            let cmd = fill(&o.run);
            self.run_opener(cmd, o.block);
        }
    }

    /// Run an opener on this machine: blocking ones take the terminal (like attach),
    /// the rest start detached.
    fn run_opener(&mut self, cmd: String, block: bool) {
        let dir = self
            .browser
            .as_ref()
            .and_then(|b| crate::files::decode_path(&b.path).ok())
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| "/".into());
        let local = self.local_index();
        if block {
            self.pending_command = Some((
                self.devices[local].clone(),
                RunCommand {
                    directory: dir,
                    command: cmd,
                },
            ));
        } else {
            use std::os::unix::process::CommandExt;
            let spawned = std::process::Command::new("sh")
                .arg("-c")
                .arg(&cmd)
                .current_dir(&dir)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .process_group(0)
                .spawn();
            match spawned {
                Ok(mut child) => {
                    // Reaped off the UI thread; the program outlives cx if it must.
                    thread::spawn(move || child.wait());
                }
                Err(e) => {
                    self.set_notice_as(NoticeKind::Error, safe_text(&format!("Open failed: {e}")))
                }
            }
        }
    }

    /// Enter in the create/cd/search inputs that reuse `Input::Mkdir`.
    pub(super) fn yazi_input_enter(&mut self) -> bool {
        if self.yz.cd {
            self.yz.cd = false;
            self.input = None;
            let path = std::mem::take(&mut self.text);
            if let (false, Some(d)) = (
                path.trim().is_empty(),
                self.browser.as_ref().map(|b| b.device),
            ) {
                self.yazi_cd(d, path.trim().to_owned());
            }
            return true;
        }
        if let Some(content) = self.yz.search.take() {
            self.input = None;
            let q = std::mem::take(&mut self.text);
            if q.is_empty() {
                return true;
            }
            let Some((d, dir)) = self.browser.as_ref().map(|b| (b.device, b.path.clone())) else {
                return true;
            };
            let directory = crate::files::decode_path(&dir)
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or(dir);
            let command = if content {
                format!("rg --files-with-matches --smart-case -- {}", quote(&q))
            } else {
                format!(
                    "fd --hidden --exclude .git -- {} 2>/dev/null || find . -iname {}",
                    quote(&q),
                    quote(&format!("*{q}*"))
                )
            };
            self.pending_command_panel = Some((d, RunCommand { directory, command }));
            return true;
        }
        if self.yz.creating {
            self.yz.creating = false;
            let name = self.text.clone();
            let dir = name.ends_with('/');
            let name = name.trim_end_matches('/');
            if name.is_empty()
                || name == "."
                || name == ".."
                || name.contains('/')
                || name.contains('\0')
            {
                self.set_notice_as(
                    NoticeKind::Warning,
                    "Enter one name (end it with / for a directory)".into(),
                );
                return true;
            }
            self.input = None;
            if let Some(b) = &self.browser {
                let path = join_component(&b.path, name);
                let d = b.device;
                if dir {
                    self.send(d, Operation::Mkdir { path });
                } else {
                    let key = unique_key();
                    self.queue_file_actions(vec![
                        (
                            d,
                            Operation::ReceivePrepare {
                                path,
                                key: key.clone(),
                                source_identity: "cx-create-empty".into(),
                                total: 0,
                                conflict: "skip".into(),
                                mode: 0o644,
                            },
                        ),
                        (
                            d,
                            Operation::ReceiveFinalize {
                                key,
                                sha256: "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855".into(),
                            },
                        ),
                    ]);
                }
            }
            self.text.clear();
            return true;
        }
        false
    }

    /// Enter in the rename input while `t r` renames the tab.
    pub(super) fn yazi_rename_enter(&mut self) -> bool {
        if !std::mem::take(&mut self.yz.renaming_tab) {
            return false;
        }
        let name = std::mem::take(&mut self.text);
        if let Some(t) = self.yz.tabs.get_mut(self.yz.tab) {
            t.name = (!name.trim().is_empty()).then(|| name.trim().to_owned());
        }
        self.input = None;
        true
    }

    /// The Files view to draw, for an area of `width` x `height`.
    pub(super) fn yazi_view(&self, height: u16) -> render::View {
        let mut v = render::View {
            linemode: self.yz.linemode,
            now: transport::now() as i64,
            ..render::View::default()
        };
        let Some(b) = self.browser.as_ref() else {
            return v;
        };
        let device = b.device;
        let scope = browser_scope_key(b.container.as_ref());
        v.cwd = safe_label(&b.display_path);
        if !b.filter.is_empty() {
            v.flags
                .push_str(&format!(" (filter: {})", safe_label(&b.filter)));
        }
        if !b.search.is_empty() {
            v.flags
                .push_str(&format!(" (find: {})", safe_label(&b.search)));
        }
        if self.devices.len() > 1 {
            // Which device this is: its hub letter and name, after the path.
            let letter = self.hotkeys().get(&self.devices[device].id).copied();
            v.flags.push_str(&format!(
                "  {}{}",
                letter.map(|l| format!("{l} ")).unwrap_or_default(),
                safe_label(&self.devices[device].name)
            ));
        }
        v.count = if !b.marked.is_empty() {
            Some((b.marked.len(), Mark::Selected))
        } else {
            self.clipboard.as_ref().map(|c| {
                (
                    c.entries.len(),
                    if c.cut { Mark::Cut } else { Mark::Copied },
                )
            })
        };
        if self.yz.tabs.len() > 1 {
            let letters = self.hotkeys();
            v.tabs = self
                .yz
                .tabs
                .iter()
                .enumerate()
                .map(|(i, tab)| {
                    let (dev, path) = if i == self.yz.tab {
                        (device, b.display_path.as_str())
                    } else {
                        (tab.device, tab.path.as_str())
                    };
                    let name = tab.name.clone().unwrap_or_else(|| {
                        let p = crate::files::decode_path(path)
                            .map(|p| p.to_string_lossy().into_owned())
                            .unwrap_or_else(|_| path.to_owned());
                        p.trim_end_matches('/')
                            .rsplit('/')
                            .next()
                            .filter(|s| !s.is_empty())
                            .unwrap_or("/")
                            .to_owned()
                    });
                    let letter = self
                        .devices
                        .get(dev)
                        .and_then(|d| letters.get(&d.id))
                        .map(|l| format!("{l}:"))
                        .unwrap_or_default();
                    safe_label(&format!("{letter}{name}"))
                })
                .collect();
            v.tab = self.yz.tab;
        }
        let tabs_h = u16::from(v.tabs.len() > 1 && height >= 6);
        let rows = usize::from(height.saturating_sub(2 + tabs_h)).max(1);
        self.yz.rows.set(rows);
        let clip = self.clipboard.as_ref().filter(|c| c.device == device);
        let mark_of = |e: &Entry, marked: &BTreeSet<String>| {
            if marked.contains(&e.path) {
                Mark::Selected
            } else if let Some(c) = clip.filter(|c| c.entries.iter().any(|x| x.path == e.path)) {
                if c.cut {
                    Mark::Cut
                } else {
                    Mark::Copied
                }
            } else {
                Mark::None
            }
        };
        let to_row = |e: &Entry, mark: Mark| Row {
            name: safe_label(&e.name),
            dir: e.kind == "directory",
            link: e.kind == "symlink",
            exec: e.kind == "file" && e.mode.is_some_and(|m| m & 0o111 != 0),
            hidden: e.hidden,
            size: e.size,
            modified: e.modified,
            mode: e.mode,
            mark,
            found: Vec::new(),
            found_pos: None,
        };
        // Current folder, scrolled with yazi's scrolloff (5).
        let all = browser_entries(b);
        let len = all.len();
        let cursor = b.selected.min(len.saturating_sub(1));
        let off = scrolled(b.offset.get(), cursor, len, rows);
        b.offset.set(off);
        // Find (`/`): the matcher's character positions, as byte ranges of the name.
        let found = if b.search.is_empty() {
            None
        } else {
            Some(browser_matches(b))
        };
        let rows_vec: Vec<Row> = (off..(off + rows).min(len))
            .filter_map(|i| all.get(i).map(|e| (i, e)))
            .map(|(i, e)| {
                let mut r = to_row(e, mark_of(e, &b.marked));
                if let Some(m) = found.as_ref() {
                    if let Ok(k) = m.binary_search_by_key(&i, |(row, _)| *row) {
                        let starts: Vec<(usize, char)> = r.name.char_indices().collect();
                        r.found = m[k]
                            .1
                            .iter()
                            .filter_map(|&p| starts.get(p).map(|&(at, c)| (at, at + c.len_utf8())))
                            .collect();
                        r.found_pos = Some((k, m.len()));
                    }
                }
                r
            })
            .collect();
        v.current = Column {
            hovered: (len > 0).then(|| cursor - off),
            empty: (len == 0).then(|| {
                if b.loading && b.entries.is_empty() {
                    "Loading...".into()
                } else {
                    "No items".into()
                }
            }),
            size_max: b.entries.iter().map(|e| e.size).max().unwrap_or(0),
            rows: rows_vec,
        };
        // Parent column, from the listing cache.
        if let Some(parent) = b.parent.as_ref() {
            if let Some(pb) = self
                .browser_cache
                .get(&(device, parent.clone(), scope.clone()))
            {
                let prow = browser_entries(pb);
                let here = prow.position_of(&b.path);
                let hov = here.unwrap_or(0);
                let off = scrolled(pb.offset.get(), hov, prow.len(), rows);
                pb.offset.set(off);
                v.parent = Some(Column {
                    rows: (off..(off + rows).min(prow.len()))
                        .filter_map(|i| prow.get(i))
                        .map(|e| to_row(e, Mark::None))
                        .collect(),
                    hovered: here.map(|h| h - off),
                    empty: None,
                    size_max: 0,
                });
            }
        }
        // Preview column: a hovered folder's listing (cache or peek), or the file body.
        if let Some(e) = all.get(cursor) {
            if e.kind == "directory" {
                let listed = self
                    .browser_cache
                    .get(&(device, e.path.clone(), scope.clone()))
                    .map(|c| (c.entries.clone(), c.selected))
                    .or_else(|| {
                        b.peek
                            .as_ref()
                            .filter(|p| p.path == e.path && !p.loading)
                            .map(|p| (p.entries.clone(), p.selected))
                    });
                v.preview = Some(match listed {
                    Some((entries, sel)) => {
                        let shown: Vec<&Entry> = entries
                            .iter()
                            .filter(|x| b.show_hidden || !x.hidden)
                            .collect();
                        let sel = sel.min(shown.len().saturating_sub(1));
                        let off = scrolled(0, sel, shown.len(), rows);
                        Preview::Folder(Column {
                            hovered: (!shown.is_empty()).then(|| sel - off),
                            empty: shown.is_empty().then(|| "No items".into()),
                            size_max: 0,
                            rows: shown
                                .iter()
                                .skip(off)
                                .take(rows)
                                .map(|x| to_row(x, Mark::None))
                                .collect(),
                        })
                    }
                    None => Preview::Empty,
                });
            } else if b
                .peek
                .as_ref()
                .is_some_and(|p| p.preview.is_some() && p.preview_path.as_ref() == Some(&e.path))
            {
                v.preview = Some(Preview::Body);
            }
        }
        let hovered = all.get(cursor);
        v.status = Status {
            mode: match (b.visual_anchor.is_some(), self.yz.unset) {
                (false, _) => ModeKind::Normal,
                (true, false) => ModeKind::Select,
                (true, true) => ModeKind::Unset,
            },
            size: hovered.map(|e| e.size),
            name: hovered.map(|e| safe_label(&e.name)).unwrap_or_default(),
            perm: hovered.and_then(|e| {
                e.mode
                    .map(|m| render::perm_string(m, e.kind == "directory", e.kind == "symlink"))
            }),
            cursor,
            len,
            progress: self.yazi_progress(),
            // The send-to flow's destination (the earlier popup, kept): Enter pastes.
            message: (self.destination_active && self.clipboard.is_some()).then(|| {
                let n = self.clipboard.as_ref().map_or(0, |c| c.entries.len());
                format!("⏎ transfer here ({n}) · Esc cancels")
            }),
        };
        v.which = if self.yz.pending.is_empty() {
            Vec::new()
        } else {
            self.chord_candidates(&self.yz.pending)
        };
        v.popup = self.yazi_popup_view();
        v
    }

    fn yazi_progress(&self) -> Option<(String, Option<f64>, bool)> {
        let rows = self.job_rows();
        let running: Vec<&Value> = rows
            .iter()
            .map(|(_, j)| j)
            .filter(|j| {
                matches!(
                    j["status"].as_str(),
                    Some("queued" | "running" | "submitting")
                )
            })
            .collect();
        if running.is_empty() {
            return (!self.pending_transfers.is_empty())
                .then(|| ("Starting transfer".into(), None, false));
        }
        let (done, total) = running.iter().fold((0u64, 0u64), |(d, t), j| {
            (
                d + j["bytes"].as_u64().unwrap_or(0),
                t + j["total"].as_u64().unwrap_or(0),
            )
        });
        let verb = if running.iter().all(|j| j["operation"] == "move") {
            "Moving"
        } else {
            "Copying"
        };
        let label = if running.len() == 1 {
            format!("{verb} {}", transfer_name(running[0]))
        } else {
            format!("{verb} {} items", running.len())
        };
        Some((
            label,
            (total > 0).then(|| done as f64 / total as f64),
            false,
        ))
    }

    fn yazi_popup_view(&self) -> Option<Popup> {
        if let Some(mode) = self.input {
            let title = match mode {
                Input::Filter => "Filter:".to_string(),
                Input::Search => match self.yz.find {
                    Some((_, true)) => "Find previous:".into(),
                    _ => "Find next:".into(),
                },
                Input::Rename if self.yz.renaming_tab => "Rename tab:".into(),
                Input::Rename => "Rename:".into(),
                Input::Mkdir if self.yz.cd => "Change directory:".into(),
                Input::Mkdir if self.yz.search == Some(true) => "Search via ripgrep:".into(),
                Input::Mkdir if self.yz.search == Some(false) => "Search via fd:".into(),
                Input::Mkdir => "Create:".into(),
                _ => return None,
            };
            let cursor = self
                .text
                .get(..self.rename_cursor)
                .unwrap_or(&self.text)
                .chars()
                .count();
            let cursor = if matches!(mode, Input::Mkdir) {
                self.text.chars().count()
            } else {
                cursor
            };
            return Some(Popup::Input {
                title,
                value: safe_label(&self.text),
                cursor,
                anchor: if mode == Input::Rename && !self.yz.renaming_tab {
                    Anchor::Hovered
                } else {
                    Anchor::TopCenter
                },
            });
        }
        Some(match self.yz.popup.as_ref()? {
            YPop::Confirm {
                permanent, entries, ..
            } => {
                let n = entries.len();
                let s = if n == 1 { "" } else { "s" };
                let folders = entries.iter().any(|e| e.kind == "directory");
                Popup::Confirm {
                    title: if *permanent {
                        format!("Permanently delete {n} selected file{s}?")
                    } else {
                        format!("Trash {n} selected file{s}?")
                    },
                    body: (*permanent && folders)
                        .then(|| "No undo: folders include their contents.".to_string()),
                    list: entries.iter().map(|e| safe_label(&e.name)).collect(),
                }
            }
            YPop::Help {
                selected, filter, ..
            } => Popup::Help {
                rows: self.yazi_help_rows(filter),
                selected: *selected,
                filter: safe_label(filter),
            },
            YPop::Pick {
                title,
                items,
                selected,
            } => Popup::Pick {
                title: title.clone(),
                items: items.iter().map(|(l, _)| l.clone()).collect(),
                selected: *selected,
            },
        })
    }
}

/// yazi's scroll: keep `scrolloff` (5) rows around the cursor, never past the end.
pub(super) fn scrolled(offset: usize, cursor: usize, len: usize, rows: usize) -> usize {
    if len <= rows {
        return 0;
    }
    let pad = 5.min(rows.saturating_sub(1) / 2);
    let mut off = offset.min(len - rows);
    if cursor < off + pad {
        off = cursor.saturating_sub(pad);
    } else if cursor + pad >= off + rows {
        off = (cursor + pad + 1).saturating_sub(rows);
    }
    off.min(len - rows)
}

fn command_exists(program: &str) -> bool {
    if program.is_empty() || program.contains('/') {
        return !program.is_empty() && std::path::Path::new(program).is_file();
    }
    std::env::var_os("PATH")
        .is_some_and(|paths| std::env::split_paths(&paths).any(|dir| dir.join(program).is_file()))
}

/// The user's `[opener]` and `[open] rules` from yazi.toml (else yazi's defaults).
pub(super) mod openers {
    use super::toml::{self, Value};

    #[derive(Clone, Debug)]
    pub(crate) struct Opener {
        pub run: String,
        pub block: bool,
        pub desc: String,
    }

    fn config() -> &'static Value {
        static C: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
        C.get_or_init(|| {
            if cfg!(test) {
                return Value::Table(Default::default());
            }
            super::Theme::config_dir()
                .map(|d| d.join("yazi.toml"))
                .filter(|p| std::fs::metadata(p).is_ok_and(|m| m.len() < 1 << 20))
                .and_then(|p| std::fs::read_to_string(p).ok())
                .map(|t| toml::parse(&t))
                .unwrap_or(Value::Table(Default::default()))
        })
    }

    /// Whether the user's yazi.toml has `[open] rules` (else cx previews on Enter).
    pub(crate) fn configured() -> bool {
        config()
            .path("open.rules")
            .is_some_and(|r| !r.arr().is_empty())
    }

    /// A rough mime type from the name (yazi asks `file(1)`; this is enough for rules).
    pub(crate) fn mime(name: &str, dir: bool) -> String {
        if dir {
            return "inode/directory".into();
        }
        let ext = name
            .rsplit_once('.')
            .map(|(_, e)| e.to_ascii_lowercase())
            .unwrap_or_default();
        match ext.as_str() {
            "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "ico" | "avif" | "tiff" => {
                format!("image/{ext}")
            }
            "svg" => "image/svg+xml".into(),
            "mp3" | "flac" | "ogg" | "wav" | "m4a" => format!("audio/{ext}"),
            "mp4" | "mkv" | "webm" | "mov" | "avi" => format!("video/{ext}"),
            "pdf" => "application/pdf".into(),
            "zip" => "application/zip".into(),
            "gz" | "tgz" => "application/gzip".into(),
            "tar" => "application/x-tar".into(),
            "xz" => "application/x-xz".into(),
            "bz2" => "application/x-bzip2".into(),
            "7z" => "application/x-7z-compressed".into(),
            "rar" => "application/vnd.rar".into(),
            "json" => "application/json".into(),
            "js" | "mjs" => "application/javascript".into(),
            "xml" => "application/xml".into(),
            "sh" | "bash" | "zsh" => "application/x-shellscript".into(),
            "html" | "htm" => "text/html".into(),
            "" => "text/plain".into(),
            _ => "text/plain".into(),
        }
    }

    /// Glob with `*` and `{a,b}` alternatives (what yazi's rules use).
    pub(crate) fn glob(pattern: &str, text: &str) -> bool {
        if let (Some(open), Some(close)) = (pattern.find('{'), pattern.find('}')) {
            if open < close {
                let (pre, rest) = pattern.split_at(open);
                let alts = &rest[1..close - open];
                let post = &rest[close - open + 1..];
                return alts
                    .split(',')
                    .any(|a| glob(&format!("{pre}{a}{post}"), text));
            }
        }
        fn star(p: &[u8], t: &[u8]) -> bool {
            match p.split_first() {
                None => t.is_empty(),
                Some((b'*', rest)) => (0..=t.len()).any(|i| star(rest, &t[i..])),
                Some((c, rest)) => t.first() == Some(c) && star(rest, &t[1..]),
            }
        }
        star(pattern.as_bytes(), text.as_bytes())
    }

    fn defaults() -> Vec<(&'static str, &'static str, Vec<Opener>)> {
        let o = |run: &str, block: bool, desc: &str| Opener {
            run: run.into(),
            block,
            desc: desc.into(),
        };
        vec![
            (
                "mime",
                "text/*",
                vec![o("${EDITOR:-vi} %s", true, "$EDITOR")],
            ),
            (
                "mime",
                "inode/directory",
                vec![o("${EDITOR:-vi} %s", true, "$EDITOR")],
            ),
            (
                "mime",
                "application/{json,javascript,xml,x-shellscript}",
                vec![o("${EDITOR:-vi} %s", true, "$EDITOR")],
            ),
            (
                "url",
                "*",
                vec![
                    o("xdg-open %s1", false, "Open"),
                    o("${EDITOR:-vi} %s", true, "$EDITOR"),
                ],
            ),
        ]
    }

    /// The openers for a file, first rule that matches.
    pub(crate) fn for_file(name: &str, dir: bool) -> Vec<Opener> {
        let cfg = config();
        let mime = mime(name, dir);
        let named = |key: &str| -> Vec<Opener> {
            cfg.path(&format!("opener.{key}"))
                .map(Value::arr)
                .unwrap_or(&[])
                .iter()
                .filter(|o| {
                    o.get("for")
                        .and_then(Value::str)
                        .is_none_or(|f| f == "unix" || f == "linux")
                })
                .filter_map(|o| {
                    Some(Opener {
                        run: o.get("run")?.str()?.to_owned(),
                        block: o.get("block") == Some(&Value::Bool(true)),
                        desc: o.get("desc").and_then(Value::str).unwrap_or(key).to_owned(),
                    })
                })
                .collect()
        };
        let rules = cfg.path("open.rules").map(Value::arr).unwrap_or(&[]);
        for rule in rules {
            let hit = match (
                rule.get("mime").and_then(Value::str),
                rule.get("url").and_then(Value::str),
            ) {
                (Some(m), _) => glob(m, &mime),
                (_, Some(u)) => glob(u, name) || (u == "*/" && dir),
                _ => false,
            };
            if hit {
                let uses: Vec<&str> = match rule.get("use") {
                    Some(Value::Str(s)) => vec![s.as_str()],
                    Some(v) => v.arr().iter().filter_map(Value::str).collect(),
                    None => Vec::new(),
                };
                let out: Vec<Opener> = uses.iter().flat_map(|u| named(u)).collect();
                if !out.is_empty() {
                    return out;
                }
            }
        }
        defaults()
            .into_iter()
            .find(|(kind, pat, _)| {
                if *kind == "mime" {
                    glob(pat, &mime)
                } else {
                    glob(pat, name)
                }
            })
            .map(|(_, _, o)| o)
            .unwrap_or_default()
    }

    /// `%s` all paths, `%s1` the first, `%d1` its directory (shell-quoted).
    pub(crate) fn fill(run: &str, paths: &[String], dir: &str) -> String {
        let all = paths
            .iter()
            .map(|p| super::quote(p))
            .collect::<Vec<_>>()
            .join(" ");
        let first = paths.first().map(|p| super::quote(p)).unwrap_or_default();
        let d1 = paths
            .first()
            .and_then(|p| std::path::Path::new(p).parent())
            .map(|p| super::quote(&p.to_string_lossy()))
            .unwrap_or_else(|| super::quote(dir));
        run.replace("%s1", &first)
            .replace("%d1", &d1)
            .replace("%s", &all)
            .replace("%S", &all)
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn globs_and_fill() {
            assert!(glob(
                "application/{json,javascript,xml,x-shellscript}",
                "application/json"
            ));
            assert!(glob("text/*", "text/plain"));
            assert!(glob("*.md", "README.md"));
            assert!(!glob("*.md", "README.txt"));
            let s = fill("nvim %s", &["/a b/c".into(), "/d".into()], "/");
            assert_eq!(s, "nvim '/a b/c' '/d'");
            assert_eq!(
                fill("xdg-open %d1", &["/x/y.png".into()], "/"),
                "xdg-open '/x'"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrolloff_keeps_five_rows_around_the_cursor() {
        assert_eq!(scrolled(0, 0, 100, 20), 0);
        assert_eq!(scrolled(0, 14, 100, 20), 0);
        assert_eq!(scrolled(0, 15, 100, 20), 1);
        assert_eq!(scrolled(50, 54, 100, 20), 49);
        assert_eq!(scrolled(0, 99, 100, 20), 80);
        assert_eq!(scrolled(10, 3, 5, 20), 0);
    }

    #[test]
    fn every_default_binding_has_a_unique_sequence() {
        let mut seen = BTreeSet::new();
        for b in keymap() {
            assert!(seen.insert(b.on.concat()), "duplicate {:?}", b.on);
        }
        // `'` is not bound in yazi's default keymap (keymap-default.toml).
        assert!(keymap().iter().filter(|b| b.on == ["'"]).count() == 1);
    }

    #[test]
    fn key_names_follow_yazi() {
        let k = |c, m| KeyEvent::new(c, m);
        assert_eq!(
            key_name(&k(KeyCode::Char('u'), KeyModifiers::CONTROL)).unwrap(),
            "<C-u>"
        );
        assert_eq!(
            key_name(&k(KeyCode::Char(' '), KeyModifiers::NONE)).unwrap(),
            "<Space>"
        );
        assert_eq!(
            key_name(&k(KeyCode::Char('G'), KeyModifiers::SHIFT)).unwrap(),
            "G"
        );
        assert_eq!(
            key_name(&k(KeyCode::Left, KeyModifiers::ALT)).unwrap(),
            "<A-Left>"
        );
        assert_eq!(
            key_name(&k(KeyCode::Enter, KeyModifiers::SHIFT)).unwrap(),
            "<S-Enter>"
        );
    }
}
