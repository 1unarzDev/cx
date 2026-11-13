//! The yazi look. When `~/.config/yazi/theme.toml` exists on this machine its colours,
//! icons and keymap prepends are reused (read once, never written: no new cx config);
//! otherwise yazi's built-in defaults apply.
//!
//! Transparency (decision 2026-11-12): a background is painted only where the theme
//! sets one for that element and it is not the theme's own app background (which is
//! the terminal's colour). Pills (mode, hovered row) are drawn REVERSED over their
//! colour, the way yazi does; markers become a solid foreground glyph.
use super::toml::{self, Value};
use ratatui::style::{Color, Modifier, Style};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Is {
    Dir,
    Hovered,
    Exec,
    Link,
    Orphan,
    Hidden,
    Dummy,
}

/// One `[icon] conds` rule: `if` as alternatives (`|`) of conjunctions (`&`) of
/// possibly negated facts.
#[derive(Clone, Debug)]
pub(crate) struct IconCond {
    any: Vec<Vec<(bool, Is)>>,
    pub text: String,
    pub fg: Option<Color>,
}

#[derive(Clone, Debug)]
pub(crate) struct FileRule {
    /// `*` (any), `*/` (folders), or `*.ext`-style suffix globs.
    url: Option<String>,
    mime: Option<&'static str>,
    is: Option<Is>,
    pub style: Style,
}

#[derive(Clone, Debug)]
pub(crate) struct Mode {
    /// The pill's colour (the theme's `*_main.bg`), drawn reversed.
    pub pill: Color,
    pub main: Style,
    pub alt: Style,
}

#[derive(Clone, Debug)]
pub(crate) struct Theme {
    /// Read from the user's yazi config (else built-in).
    pub from_user: bool,
    pub overall: Style,
    pub cwd: Style,
    pub find_keyword: Style,
    pub find_position: Style,
    pub symlink_target: Style,
    pub marker_copied: Color,
    pub marker_cut: Color,
    pub marker_selected: Color,
    pub count_copied: Style,
    pub count_cut: Style,
    pub count_selected: Style,
    pub border: Style,
    pub border_symbol: String,
    pub tab_active: Style,
    pub tab_inactive: Style,
    pub normal: Mode,
    pub select: Mode,
    pub unset: Mode,
    pub hovered: Style,
    pub parent_hovered: Style,
    pub preview_hovered: Style,
    pub status: Style,
    pub perm_sep: Style,
    pub perm_type: Style,
    pub perm_read: Style,
    pub perm_write: Style,
    pub perm_exec: Style,
    pub progress: Style,
    pub progress_error: Style,
    pub which_border: Style,
    pub which_cols: usize,
    pub which_cand: Style,
    pub which_rest: Style,
    pub which_desc: Style,
    pub which_sep: String,
    pub which_sep_style: Style,
    pub confirm_border: Style,
    pub confirm_title: Style,
    pub confirm_body: Style,
    pub btn_yes: Style,
    pub btn_no: Style,
    pub btn_labels: (String, String),
    pub input_border: Style,
    pub input_title: Style,
    pub input_value: Style,
    pub help_border: Style,
    pub help_on: Style,
    pub help_hovered: Style,
    pub filetype: Vec<FileRule>,
    pub icons: Vec<IconCond>,
    /// The user's `[[mgr.prepend_keymap]]` rows: (keys, run, desc).
    pub prepends: Vec<(Vec<String>, String, String)>,
}

pub(crate) fn color(name: &str) -> Option<Color> {
    let n = name.trim().to_ascii_lowercase();
    if let Some(hex) = n.strip_prefix('#') {
        if hex.len() == 6 {
            let v = u32::from_str_radix(hex, 16).ok()?;
            return Some(Color::Rgb((v >> 16) as u8, (v >> 8) as u8, v as u8));
        }
        return None;
    }
    Some(match n.as_str() {
        "black" => Color::Black,
        "red" => Color::Red,
        "green" => Color::Green,
        "yellow" => Color::Yellow,
        "blue" => Color::Blue,
        "magenta" => Color::Magenta,
        "cyan" => Color::Cyan,
        "white" => Color::White,
        "gray" => Color::Gray,
        "darkgray" => Color::DarkGray,
        "lightred" => Color::LightRed,
        "lightgreen" => Color::LightGreen,
        "lightyellow" => Color::LightYellow,
        "lightblue" => Color::LightBlue,
        "lightmagenta" => Color::LightMagenta,
        "lightcyan" => Color::LightCyan,
        "reset" => Color::Reset,
        _ => return None,
    })
}

/// A theme style table; `bg` is kept only when it is not the app background.
fn style(v: Option<&Value>, app_bg: Option<Color>) -> Option<Style> {
    let v = v?;
    let mut s = Style::default();
    if let Some(c) = v.get("fg").and_then(Value::str).and_then(color) {
        s = s.fg(c);
    }
    if let Some(c) = v.get("bg").and_then(Value::str).and_then(color) {
        if Some(c) != app_bg && c != Color::Reset {
            s = s.bg(c);
        }
    }
    for (key, m) in [
        ("bold", Modifier::BOLD),
        ("italic", Modifier::ITALIC),
        ("underline", Modifier::UNDERLINED),
        ("reversed", Modifier::REVERSED),
        ("dim", Modifier::DIM),
        ("blink", Modifier::SLOW_BLINK),
        ("hidden", Modifier::HIDDEN),
        ("crossed", Modifier::CROSSED_OUT),
    ] {
        if v.get(key) == Some(&Value::Bool(true)) {
            s = s.add_modifier(m);
        }
    }
    Some(s)
}

fn fg(c: Color) -> Style {
    Style::default().fg(c)
}

fn is(word: &str) -> Option<Is> {
    Some(match word.trim() {
        "dir" => Is::Dir,
        "hovered" => Is::Hovered,
        "exec" => Is::Exec,
        "link" => Is::Link,
        "orphan" => Is::Orphan,
        "hidden" => Is::Hidden,
        "dummy" => Is::Dummy,
        _ => return None,
    })
}

fn icon_cond(expr: &str, text: &str, fg: Option<Color>) -> Option<IconCond> {
    let any = expr
        .split('|')
        .map(|alt| {
            alt.split('&')
                .map(|w| {
                    let w = w.trim();
                    match w.strip_prefix('!') {
                        Some(rest) => is(rest).map(|f| (true, f)),
                        None => is(w).map(|f| (false, f)),
                    }
                })
                .collect::<Option<Vec<_>>>()
        })
        .collect::<Option<Vec<_>>>()?;
    Some(IconCond {
        any,
        text: text.into(),
        fg,
    })
}

/// What an entry is, for icon and filetype rules.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Facts {
    pub dir: bool,
    pub hovered: bool,
    pub exec: bool,
    pub link: bool,
    pub orphan: bool,
    pub hidden: bool,
}
impl Facts {
    fn has(&self, f: Is) -> bool {
        match f {
            Is::Dir => self.dir,
            Is::Hovered => self.hovered,
            Is::Exec => self.exec,
            Is::Link => self.link,
            Is::Orphan => self.orphan,
            Is::Hidden => self.hidden,
            Is::Dummy => false,
        }
    }
}

/// A coarse mime class from the extension (the default theme colours by mime).
fn mime_class(name: &str) -> Option<&'static str> {
    let ext = name.rsplit_once('.')?.1.to_ascii_lowercase();
    Some(match ext.as_str() {
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "ico" | "svg" | "avif" | "tiff" => {
            "image"
        }
        "mp3" | "flac" | "ogg" | "wav" | "m4a" | "mp4" | "mkv" | "webm" | "mov" | "avi" => "media",
        "zip" | "rar" | "7z" | "tar" | "gz" | "tgz" | "xz" | "zst" | "bz2" | "lzma" | "cpio" => {
            "archive"
        }
        "pdf" | "doc" | "rtf" => "document",
        _ => return None,
    })
}

impl Theme {
    /// The icon text and colour for an entry (first matching rule).
    pub(crate) fn icon(&self, f: &Facts) -> Option<(&str, Option<Color>)> {
        self.icons
            .iter()
            .find(|c| {
                c.any
                    .iter()
                    .any(|all| all.iter().all(|(neg, fact)| f.has(*fact) != *neg))
            })
            .map(|c| (c.text.as_str(), c.fg))
    }
    /// The name style for an entry (first matching filetype rule).
    pub(crate) fn file_style(&self, name: &str, f: &Facts) -> Style {
        self.filetype
            .iter()
            .find(|r| {
                r.is.is_none_or(|i| f.has(i))
                    && r.mime.is_none_or(|m| !f.dir && mime_class(name) == Some(m))
                    && r.url.as_deref().is_none_or(|u| match u {
                        "*" => true,
                        "*/" => f.dir,
                        glob => glob
                            .strip_prefix('*')
                            .is_some_and(|suffix| !f.dir && name.ends_with(suffix)),
                    })
            })
            .map(|r| r.style)
            .unwrap_or_default()
    }

    /// yazi's built-in dark theme, minus the grey backgrounds (transparent).
    pub(crate) fn builtin() -> Self {
        let mode = |c: Color| Mode {
            pill: c,
            main: Style::default().add_modifier(Modifier::BOLD),
            alt: fg(c),
        };
        let rev = Style::default().add_modifier(Modifier::REVERSED);
        let dir = |hovered: bool, text: &str, fgc: Color| IconCond {
            any: vec![if hovered {
                vec![(false, Is::Dir), (false, Is::Hovered)]
            } else {
                vec![(false, Is::Dir)]
            }],
            text: text.into(),
            fg: Some(fgc),
        };
        let blue = Color::Rgb(0x03, 0xa9, 0xf4);
        Theme {
            from_user: false,
            overall: Style::default(),
            cwd: fg(Color::Cyan),
            find_keyword: fg(Color::Yellow)
                .add_modifier(Modifier::BOLD | Modifier::ITALIC | Modifier::UNDERLINED),
            find_position: fg(Color::Magenta).add_modifier(Modifier::BOLD | Modifier::ITALIC),
            symlink_target: Style::default().add_modifier(Modifier::ITALIC),
            marker_copied: Color::LightGreen,
            marker_cut: Color::LightRed,
            marker_selected: Color::LightYellow,
            count_copied: fg(Color::LightGreen),
            count_cut: fg(Color::LightRed),
            count_selected: fg(Color::LightYellow),
            border: fg(Color::DarkGray),
            border_symbol: "│".into(),
            tab_active: fg(Color::Blue).add_modifier(Modifier::BOLD | Modifier::REVERSED),
            tab_inactive: fg(Color::Blue),
            normal: mode(Color::Blue),
            select: mode(Color::Red),
            unset: mode(Color::Red),
            hovered: rev,
            parent_hovered: rev,
            preview_hovered: Style::default().add_modifier(Modifier::UNDERLINED),
            status: Style::default(),
            perm_sep: fg(Color::DarkGray),
            perm_type: fg(Color::Green),
            perm_read: fg(Color::Yellow),
            perm_write: fg(Color::Red),
            perm_exec: fg(Color::Cyan),
            progress: fg(Color::Green),
            progress_error: fg(Color::Red),
            which_border: fg(Color::Blue),
            which_cols: 3,
            which_cand: fg(Color::LightCyan),
            which_rest: fg(Color::DarkGray),
            which_desc: fg(Color::LightMagenta),
            which_sep: "  ".into(),
            which_sep_style: fg(Color::DarkGray),
            confirm_border: fg(Color::Blue),
            confirm_title: fg(Color::Blue),
            confirm_body: Style::default(),
            btn_yes: rev,
            btn_no: Style::default(),
            btn_labels: ("  [Y]es  ".into(), "  (N)o  ".into()),
            input_border: fg(Color::Blue),
            input_title: Style::default(),
            input_value: Style::default(),
            help_border: fg(Color::Blue),
            help_on: fg(Color::Cyan),
            help_hovered: rev.add_modifier(Modifier::BOLD),
            filetype: [
                (None, Some("image"), None, fg(Color::Yellow)),
                (None, Some("media"), None, fg(Color::Magenta)),
                (None, Some("archive"), None, fg(Color::Red)),
                (None, Some("document"), None, fg(Color::Cyan)),
                (Some("*"), None, Some(Is::Exec), fg(Color::Green)),
                (Some("*/"), None, None, fg(Color::Blue)),
            ]
            .into_iter()
            .map(|(url, mime, is, style)| FileRule {
                url: url.map(str::to_owned),
                mime,
                is,
                style,
            })
            .collect(),
            icons: vec![
                IconCond {
                    any: vec![vec![(false, Is::Link)]],
                    text: "\u{f481}".into(),
                    fg: Some(Color::Rgb(0x9e, 0x9e, 0x9e)),
                },
                dir(true, "\u{e5fe}", blue),
                dir(false, "\u{e5ff}", blue),
                IconCond {
                    any: vec![vec![(false, Is::Exec)]],
                    text: "\u{f489}".into(),
                    fg: Some(Color::Rgb(0x8b, 0xc3, 0x4a)),
                },
                IconCond {
                    any: vec![vec![(true, Is::Dir)]],
                    text: "\u{f15b}".into(),
                    fg: None,
                },
            ],
            prepends: Vec::new(),
        }
    }

    /// The built-in theme overlaid with a theme.toml document (and keymap prepends).
    pub(crate) fn from_docs(theme: &Value, keymap: Option<&Value>) -> Self {
        let mut t = Theme::builtin();
        t.from_user = true;
        let app_bg = theme
            .path("app.overall.bg")
            .and_then(Value::str)
            .and_then(color);
        let st = |path: &str| style(theme.path(path), app_bg);
        let col = |path: &str| {
            theme
                .path(path)
                .and_then(|v| v.get("fg"))
                .and_then(Value::str)
                .and_then(color)
        };
        macro_rules! set {
            ($field:ident, $path:expr) => {
                if let Some(s) = st($path) {
                    t.$field = s;
                }
            };
        }
        if let Some(s) = st("app.overall") {
            // Only its foreground: the default text colour.
            t.overall = Style::default().fg(s.fg.unwrap_or(Color::Reset));
            if s.fg.is_none() {
                t.overall = Style::default();
            }
        }
        set!(cwd, "mgr.cwd");
        set!(find_keyword, "mgr.find_keyword");
        set!(find_position, "mgr.find_position");
        set!(symlink_target, "mgr.symlink_target");
        set!(count_copied, "mgr.count_copied");
        set!(count_cut, "mgr.count_cut");
        set!(count_selected, "mgr.count_selected");
        set!(border, "mgr.border_style");
        set!(tab_active, "tabs.active");
        set!(tab_inactive, "tabs.inactive");
        set!(hovered, "indicator.current");
        set!(parent_hovered, "indicator.parent");
        set!(preview_hovered, "indicator.preview");
        // yazi 26 draws the hovered help row's desc in the hovered indicator's colour.
        if let Some(c) = t.hovered.fg {
            t.help_hovered = t.help_hovered.fg(c);
        }
        set!(status, "status.overall");
        set!(perm_sep, "status.perm_sep");
        set!(perm_type, "status.perm_type");
        set!(perm_read, "status.perm_read");
        set!(perm_write, "status.perm_write");
        set!(perm_exec, "status.perm_exec");
        set!(progress, "status.progress_normal");
        set!(progress_error, "status.progress_error");
        set!(which_border, "which.border");
        set!(which_cand, "which.cand");
        set!(which_rest, "which.rest");
        set!(which_desc, "which.desc");
        set!(which_sep_style, "which.separator_style");
        set!(confirm_border, "confirm.border");
        set!(confirm_title, "confirm.title");
        set!(confirm_body, "confirm.body");
        set!(btn_yes, "confirm.btn_yes");
        set!(btn_no, "confirm.btn_no");
        set!(input_border, "input.border");
        set!(input_title, "input.title");
        set!(input_value, "input.value");
        for (field, path) in [
            (&mut t.marker_copied, "mgr.marker_copied"),
            (&mut t.marker_cut, "mgr.marker_cut"),
            (&mut t.marker_selected, "mgr.marker_selected"),
        ] {
            if let Some(c) = col(path) {
                *field = c;
            }
        }
        if let Some(s) = theme.path("mgr.border_symbol").and_then(Value::str) {
            t.border_symbol = s.into();
        }
        if let Some(s) = theme.path("which.separator").and_then(Value::str) {
            t.which_sep = s.into();
        }
        if let Some(Value::Int(n)) = theme.path("which.cols") {
            t.which_cols = (*n).clamp(1, 3) as usize;
        }
        if let [Value::Str(y), Value::Str(n)] =
            theme.path("confirm.btn_labels").map_or(&[][..], Value::arr)
        {
            t.btn_labels = (y.clone(), n.clone());
        }
        for (mode, name) in [
            (&mut t.normal, "normal"),
            (&mut t.select, "select"),
            (&mut t.unset, "unset"),
        ] {
            let main = theme.path(&format!("mode.{name}_main"));
            if let Some(c) = main
                .and_then(|v| v.get("bg"))
                .and_then(Value::str)
                .and_then(color)
            {
                mode.pill = c;
            }
            if let Some(s) = style(main, app_bg) {
                // The pill is drawn reversed over `pill`: only the attributes stay.
                mode.main = Style::default().add_modifier(s.add_modifier);
            }
            if let Some(s) = style(theme.path(&format!("mode.{name}_alt")), app_bg) {
                mode.alt = s;
            }
        }
        let rules = theme.path("filetype.rules").map_or(&[][..], Value::arr);
        if !rules.is_empty() {
            t.filetype = rules
                .iter()
                .filter_map(|r| {
                    let mime = match r.get("mime").and_then(Value::str) {
                        None => None,
                        Some(m) if m.contains("image") => Some("image"),
                        Some(m) if m.contains("audio") || m.contains("video") => Some("media"),
                        Some(m) if m.contains("zip") => Some("archive"),
                        Some(m) if m.contains("pdf") => Some("document"),
                        Some(_) => return None,
                    };
                    Some(FileRule {
                        url: r.get("url").and_then(Value::str).map(str::to_owned),
                        mime,
                        is: r.get("is").and_then(Value::str).and_then(is),
                        style: style(Some(r), app_bg).unwrap_or_default(),
                    })
                })
                .collect();
        }
        let conds = theme.path("icon.conds").map_or(&[][..], Value::arr);
        if !conds.is_empty() {
            t.icons = conds
                .iter()
                .filter_map(|c| {
                    icon_cond(
                        c.get("if")?.str()?,
                        c.get("text")?.str()?,
                        c.get("fg").and_then(Value::str).and_then(color),
                    )
                })
                .collect();
        }
        if let Some(k) = keymap {
            t.prepends = k
                .path("mgr.prepend_keymap")
                .map_or(&[][..], Value::arr)
                .iter()
                .filter_map(|row| {
                    let on = match row.get("on")? {
                        Value::Str(s) => vec![s.clone()],
                        v => v
                            .arr()
                            .iter()
                            .filter_map(|k| k.str().map(str::to_owned))
                            .collect(),
                    };
                    let run = match row.get("run")? {
                        Value::Str(s) => s.clone(),
                        v => v.arr().first()?.str()?.to_owned(),
                    };
                    let desc = row
                        .get("desc")
                        .and_then(Value::str)
                        .unwrap_or(&run)
                        .to_owned();
                    Some((on, run, desc))
                })
                .collect();
        }
        t
    }

    /// The user's yazi config directory on this machine, if any (`$YAZI_CONFIG_HOME`,
    /// else `$XDG_CONFIG_HOME/yazi`, else `~/.config/yazi`).
    pub(crate) fn config_dir() -> Option<std::path::PathBuf> {
        if let Some(d) = std::env::var_os("YAZI_CONFIG_HOME").filter(|d| !d.is_empty()) {
            return Some(d.into());
        }
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .filter(|d| !d.is_empty())
            .map(std::path::PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|h| std::path::Path::new(&h).join(".config"))
            })?;
        Some(base.join("yazi"))
    }

    /// The theme for this process: the user's yazi theme if present, read once.
    pub(crate) fn load() -> Self {
        let read = |name: &str| -> Option<Value> {
            let path = Self::config_dir()?.join(name);
            let meta = std::fs::metadata(&path).ok()?;
            // A theme file is small; anything huge is not one.
            if !meta.is_file() || meta.len() > 1 << 20 {
                return None;
            }
            Some(toml::parse(&std::fs::read_to_string(path).ok()?))
        };
        match read("theme.toml") {
            Some(theme) => {
                if let Some(p) = theme.path("mgr.syntect_theme").and_then(Value::str) {
                    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
                    let path = match (p.strip_prefix("~/"), home) {
                        (Some(rest), Some(h)) => h.join(rest),
                        _ => std::path::PathBuf::from(p),
                    };
                    crate::syntax_preview::use_theme(&path);
                }
                Theme::from_docs(&theme, read("keymap.toml").as_ref())
            }
            None => {
                let mut t = Theme::builtin();
                if let Some(k) = read("keymap.toml") {
                    t.prepends =
                        Theme::from_docs(&Value::Table(Default::default()), Some(&k)).prepends;
                }
                t
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const USER: &str = r##"
[app]
overall = { fg = "#b4c7ed", bg = "#0d0e12" }
[mgr]
cwd = { fg = "#ebdcff" }
marker_selected = { fg = "#ebdcff", bg = "#ebdcff" }
count_selected = { fg = "#e3e5ef", bg = "#0d0e12" }
border_style  = { fg = "#e3e5ef" }
[mode]
normal_main = { fg = "#0d0e12", bg = "#b4c7ed", bold = true }
normal_alt  = { fg = "#b4c7ed", bg = "#0d0e12" }
[indicator]
current = { reversed = true, fg = "#b4c7ed" }
[filetype]
rules = [ { url = "*/", fg = "#b4c7ed"}, { url = "*", fg = "#b4c7ed"} ]
[icon]
conds = [
	{ if = "dir & hovered", text = "", fg = "#b4c7ed"},
	{ if = "dir", text = "", fg = "#b4c7ed"},
	{ if = "!dir", text = "", fg = "#b4c7ed"}
]
"##;

    #[test]
    fn user_theme_colours_without_its_app_background() {
        let t = Theme::from_docs(&toml::parse(USER), None);
        let blue = Color::Rgb(0xb4, 0xc7, 0xed);
        assert_eq!(t.cwd.fg, Some(Color::Rgb(0xeb, 0xdc, 0xff)));
        // The app background is the terminal's: never painted.
        assert_eq!(t.count_selected.bg, None);
        assert_eq!(t.normal.alt.bg, None);
        assert_eq!(t.normal.pill, blue);
        assert!(t.normal.main.add_modifier.contains(Modifier::BOLD));
        assert_eq!(t.marker_selected, Color::Rgb(0xeb, 0xdc, 0xff));
        assert_eq!(t.hovered.fg, Some(blue));
        let dir = Facts {
            dir: true,
            hovered: true,
            ..Facts::default()
        };
        assert_eq!(t.icon(&dir).map(|i| i.0), Some(""));
        let file = Facts::default();
        assert_eq!(t.icon(&file).map(|i| i.0), Some(""));
        assert_eq!(t.file_style("a.rs", &file).fg, Some(blue));
    }

    #[test]
    fn builtin_paints_no_backgrounds() {
        let t = Theme::builtin();
        for s in [
            t.normal.alt,
            t.tab_inactive,
            t.status,
            t.hovered,
            t.count_selected,
        ] {
            assert_eq!(s.bg, None);
        }
        let png = Facts::default();
        assert_eq!(t.file_style("a.png", &png).fg, Some(Color::Yellow));
        let dir = Facts {
            dir: true,
            ..Facts::default()
        };
        assert_eq!(t.file_style("x", &dir).fg, Some(Color::Blue));
    }
}
