//! Drawing the yazi layout from a plain description (`View`): header, optional tab
//! bar, three miller columns (parent 1 : current 4 : preview 3), the status line, and
//! the overlays (which-key, help, input, confirm, pick). It knows nothing of App, so the
//! design lab and the all-size fuzz test draw it from fixtures too.
//!
//! Geometry follows yazi 26 (see ~/.cache/cx-yazi-ref/NOTES.md): at 120 columns the
//! parent list is x1-14, the current column x15-74 with rails on x15 and x74, the
//! preview x75-119.
use super::theme::{Facts, Theme};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::Span,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub(crate) enum Mark {
    #[default]
    None,
    Selected,
    Copied,
    Cut,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Row {
    pub name: String,
    pub dir: bool,
    pub link: bool,
    pub exec: bool,
    pub hidden: bool,
    pub size: u64,
    /// Unix seconds.
    pub modified: Option<i64>,
    pub mode: Option<u32>,
    pub mark: Mark,
    /// Byte ranges of `name` to highlight (find).
    pub found: Vec<(usize, usize)>,
    /// On the hovered match: its place among the matches (yazi's `[i/n]`).
    pub found_pos: Option<(usize, usize)>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Column {
    /// The rows on screen (a window of the folder).
    pub rows: Vec<Row>,
    /// Index into `rows`.
    pub hovered: Option<usize>,
    /// Shown when `rows` is empty: "Loading..." while the listing is on its way
    /// (never on a folder seen before), "No items" once it is known to be empty.
    pub empty: Option<String>,
    /// Largest file size in the folder (the size bar's scale).
    pub size_max: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub(crate) enum Linemode {
    /// cx's default: the size bar, the size and the modified day.
    #[default]
    Bar,
    Size,
    Mtime,
    Permissions,
    None,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Preview {
    Empty,
    Folder(Column),
    /// The app draws the file preview (text, code, image) into the preview rect.
    Body,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub(crate) enum ModeKind {
    #[default]
    Normal,
    Select,
    Unset,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Status {
    pub mode: ModeKind,
    pub size: Option<u64>,
    pub name: String,
    pub perm: Option<String>,
    /// Cursor index and folder length.
    pub cursor: usize,
    pub len: usize,
    /// A running transfer: label and fraction done (None: unknown).
    pub progress: Option<(String, Option<f64>, bool)>,
    /// A one-line message (yazi's notify), shown in the middle of the line.
    pub message: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Anchor {
    TopCenter,
    /// Under the hovered row of the current column.
    Hovered,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Popup {
    Input {
        title: String,
        value: String,
        /// Cursor in characters.
        cursor: usize,
        anchor: Anchor,
    },
    Confirm {
        title: String,
        body: Option<String>,
        list: Vec<String>,
    },
    Help {
        rows: Vec<(String, String)>,
        selected: usize,
        filter: String,
    },
    Pick {
        title: String,
        items: Vec<String>,
        selected: usize,
    },
}

#[derive(Clone, Debug, PartialEq, Default)]
pub(crate) struct View {
    pub cwd: String,
    /// ` (filter: md)` and the like.
    pub flags: String,
    /// Header right: selected, else yanked count.
    pub count: Option<(usize, Mark)>,
    /// Tab labels (shown when there are two or more) and the active one.
    pub tabs: Vec<String>,
    pub tab: usize,
    pub parent: Option<Column>,
    pub current: Column,
    pub preview: Option<Preview>,
    pub status: Status,
    pub which: Vec<(String, String)>,
    pub popup: Option<Popup>,
    pub linemode: Linemode,
    /// Today as unix seconds (relative day labels), for deterministic fixtures.
    pub now: i64,
}

/// What the terminal can show.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Look {
    pub unicode: bool,
    /// Nerd-font icons.
    pub icons: bool,
    /// Rounded caps (U+E0B6/U+E0B4).
    pub pills: bool,
    /// Colours at all (NO_COLOR draws attributes only).
    pub color: bool,
}

/// Where things landed, for the app: the preview rect it draws into, the rows of the
/// current list (mouse), and the text cursor of an input popup.
#[derive(Clone, Debug, Default)]
pub(crate) struct Drawn {
    pub preview: Rect,
    pub current: Rect,
    pub parent: Rect,
    pub cursor: Option<(u16, u16)>,
    pub tabs: Vec<(Rect, usize)>,
}

const CAP_L: &str = "\u{e0b6}";
const CAP_R: &str = "\u{e0b4}";

pub(crate) fn width(s: &str) -> usize {
    Span::raw(s).width()
}
fn char_width(c: char) -> usize {
    let mut b = [0u8; 4];
    width(c.encode_utf8(&mut b))
}

/// Cut `s` to `w` columns with `…`, keeping a file's extension (`conf….toml`).
pub(crate) fn ellipsis(s: &str, w: usize, keep_ext: bool, unicode: bool) -> String {
    if width(s) <= w {
        return s.to_string();
    }
    if w == 0 {
        return String::new();
    }
    let dots = if unicode { "…" } else { "~" };
    let ext = keep_ext
        .then(|| s.rfind('.').filter(|&i| i > 0).map(|i| &s[i..]))
        .flatten()
        .filter(|e| width(e) < w);
    let tail = ext.unwrap_or("");
    let room = w - 1 - width(tail);
    let mut head = String::new();
    let mut used = 0;
    for c in s.chars() {
        let cw = char_width(c);
        if used + cw > room {
            break;
        }
        used += cw;
        head.push(c);
    }
    format!("{head}{dots}{tail}")
}

/// Cut from the left: `…/project/src`.
fn ellipsis_left(s: &str, w: usize, unicode: bool) -> String {
    if width(s) <= w {
        return s.to_string();
    }
    if w == 0 {
        return String::new();
    }
    let mut tail: Vec<char> = Vec::new();
    let mut used = 0;
    for c in s.chars().rev() {
        let cw = char_width(c);
        if used + cw + 1 > w {
            break;
        }
        used += cw;
        tail.push(c);
    }
    tail.reverse();
    format!(
        "{}{}",
        if unicode { "…" } else { "~" },
        tail.into_iter().collect::<String>()
    )
}

/// yazi's `readable_size`: B K M G T, one decimal, `.0` dropped.
pub(crate) fn readable_size(n: u64) -> String {
    const U: [&str; 6] = ["B", "K", "M", "G", "T", "P"];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i < U.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        return format!("{n}B");
    }
    let s = format!("{v:.1}");
    format!("{}{}", s.strip_suffix(".0").unwrap_or(&s), U[i])
}

/// `drwxr-xr-x` from a mode (with the type from `dir`/`link`).
pub(crate) fn perm_string(mode: u32, dir: bool, link: bool) -> String {
    let mut s = String::with_capacity(10);
    s.push(if link {
        'l'
    } else if dir {
        'd'
    } else {
        '-'
    });
    for shift in [6, 3, 0] {
        let bits = (mode >> shift) & 7;
        s.push(if bits & 4 != 0 { 'r' } else { '-' });
        s.push(if bits & 2 != 0 { 'w' } else { '-' });
        s.push(if bits & 1 != 0 { 'x' } else { '-' });
    }
    s
}

/// Seconds east of UTC on this machine at `t` (the viewer's local time).
fn utc_offset(t: i64) -> i64 {
    if cfg!(test) {
        return 0;
    }
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    let tt = t as libc::time_t;
    if unsafe { libc::localtime_r(&tt, &mut tm) }.is_null() {
        return 0;
    }
    tm.tm_gmtoff
}
/// A modified day in local time: `11/12` this year, else `2024`, and `HH:MM` today.
pub(crate) fn day(secs: i64, now: i64) -> String {
    let off = utc_offset(now);
    day_utc(secs + off, now + off)
}
fn day_utc(secs: i64, now: i64) -> String {
    let (y, m, d) = civil(secs);
    let (ny, nm, nd) = civil(now);
    if (y, m, d) == (ny, nm, nd) {
        let t = secs.rem_euclid(86400);
        format!("{:02}:{:02}", t / 3600, t % 3600 / 60)
    } else if y == ny {
        format!("{m:02}/{d:02}")
    } else {
        format!("{y}")
    }
}

/// Days since the epoch to (year, month, day), UTC (Howard Hinnant's algorithm).
fn civil(secs: i64) -> (i64, u32, u32) {
    let z = secs.div_euclid(86400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

struct Pen<'a> {
    buf: &'a mut Buffer,
    look: Look,
}

impl Pen<'_> {
    fn style(&self, s: Style) -> Style {
        let mut s = if self.look.color {
            s
        } else {
            Style {
                fg: None,
                bg: None,
                ..s
            }
        };
        if !self.look.pills && s.add_modifier.contains(Modifier::REVERSED) {
            // Without rounded caps a reversed run is a hard-edged painted block on a
            // transparent terminal: bold (and a bar or `>` on rows) instead.
            s = s
                .remove_modifier(Modifier::REVERSED)
                .add_modifier(Modifier::BOLD);
        }
        s
    }
    /// Write `s` from x, never past `end` (exclusive) or the buffer; returns the next x.
    fn put(&mut self, x: u16, y: u16, end: u16, s: &str, style: Style) -> u16 {
        let area = self.buf.area;
        if y < area.y || y >= area.bottom() || x >= end.min(area.right()) || x < area.x {
            return x;
        }
        let max = (end.min(area.right()) - x) as usize;
        let style = self.style(style);
        self.buf.set_stringn(x, y, s, max, style).0
    }
    fn blank(&mut self, r: Rect) {
        let r = r.intersection(self.buf.area);
        for y in r.top()..r.bottom() {
            for x in r.left()..r.right() {
                self.buf[(x, y)].reset();
            }
        }
    }
    fn style_cells(&mut self, x0: u16, x1: u16, y: u16, s: Style) {
        let area = self.buf.area;
        if y < area.y || y >= area.bottom() {
            return;
        }
        let s = self.style(s);
        for x in x0.max(area.x)..x1.min(area.right()) {
            self.buf[(x, y)].set_style(s);
        }
    }
    fn rounded(&mut self, r: Rect, style: Style, title: Option<(&str, Style, bool)>) {
        let r = r.intersection(self.buf.area);
        if r.width < 2 || r.height < 2 {
            return;
        }
        let u = self.look.unicode;
        let (tl, tr, bl, br, h, v) = if u {
            ("╭", "╮", "╰", "╯", "─", "│")
        } else {
            ("+", "+", "+", "+", "-", "|")
        };
        self.blank(r);
        let right = r.right() - 1;
        let bottom = r.bottom() - 1;
        for x in r.x + 1..right {
            self.put(x, r.y, x + 1, h, style);
            self.put(x, bottom, x + 1, h, style);
        }
        for y in r.y + 1..bottom {
            self.put(r.x, y, r.x + 1, v, style);
            self.put(right, y, right + 1, v, style);
        }
        self.put(r.x, r.y, r.x + 1, tl, style);
        self.put(right, r.y, right + 1, tr, style);
        self.put(r.x, bottom, r.x + 1, bl, style);
        self.put(right, bottom, right + 1, br, style);
        if let Some((t, ts, centred)) = title {
            let t = ellipsis(t, r.width.saturating_sub(2) as usize, false, u);
            let x = if centred {
                r.x + (r.width.saturating_sub(width(&t) as u16)) / 2
            } else {
                r.x + 1
            };
            self.put(x, r.y, right, &t, ts);
        }
    }
}

/// The body columns: parent, current, preview (yazi's 1:4:3 over the full width).
pub(crate) fn columns(body: Rect) -> (Rect, Rect, Rect) {
    let w = u32::from(body.width);
    let a = (w / 8) as u16;
    let b = (w * 5 / 8) as u16;
    (
        Rect::new(body.x, body.y, a, body.height),
        Rect::new(body.x + a, body.y, b - a, body.height),
        Rect::new(body.x + b, body.y, body.width - b, body.height),
    )
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Which {
    Parent,
    Current,
    Preview,
}

fn icon<'t>(t: &'t Theme, look: Look, f: &Facts) -> Option<(&'t str, Option<Color>)> {
    if !look.icons || !look.unicode {
        return None;
    }
    t.icon(f)
}

/// The linemode text right of a row in the current column.
fn linemode(
    row: &Row,
    mode: Linemode,
    size_max: u64,
    now: i64,
    unicode: bool,
    compact: bool,
) -> Vec<(String, u8)> {
    // (text, kind): 0 plain, 1 bar filled, 2 bar empty.
    match mode {
        Linemode::None => Vec::new(),
        Linemode::Size => {
            if row.dir {
                Vec::new()
            } else {
                vec![(readable_size(row.size), 0)]
            }
        }
        Linemode::Mtime => row
            .modified
            .map(|m| vec![(day(m, now), 0)])
            .unwrap_or_default(),
        Linemode::Permissions => row
            .mode
            .map(|m| vec![(perm_string(m, row.dir, row.link), 0)])
            .unwrap_or_default(),
        Linemode::Bar => {
            let mut out = Vec::new();
            if !row.dir {
                // A log-scaled bar (the earlier Files view's): a 1 KB file next to a
                // 1 GB one is still visible; half cells end it.
                let cells = 6usize;
                let v = if size_max <= 1 || row.size == 0 {
                    0.0
                } else {
                    ((row.size as f64).ln() / (size_max as f64).ln()).clamp(0.0, 1.0)
                };
                let halves = ((v * cells as f64 * 2.0).round() as usize)
                    .clamp(usize::from(row.size > 0), cells * 2);
                let (full, half, empty) = if unicode {
                    ("━", "╸", "━")
                } else {
                    ("=", "-", "-")
                };
                let mut bar = full.repeat(halves / 2);
                if halves % 2 == 1 {
                    bar.push_str(half);
                }
                let used = halves.div_ceil(2);
                out.push((bar, 1));
                out.push((empty.repeat(cells - used), 2));
                // A narrow column keeps the bar and the day (the status line has the size).
                if !compact {
                    out.push((format!(" {:>5}", readable_size(row.size)), 0));
                }
            }
            // Folders stay icon + name, as in yazi's default linemode.
            if let Some(m) = row.modified.filter(|_| !row.dir) {
                out.push((format!(" {:>5}", day(m, now)), 3));
            }
            out
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn column(
    p: &mut Pen,
    t: &Theme,
    list: Rect,
    col: &Column,
    which: Which,
    mode: Linemode,
    now: i64,
) {
    let look = p.look;
    if list.width == 0 || list.height == 0 {
        return;
    }
    if col.rows.is_empty() {
        if let Some(text) = &col.empty {
            let y = list.y + list.height / 2;
            let w = width(text) as u16;
            let x = list.x + list.width.saturating_sub(w) / 2;
            p.put(x, y, list.right(), text, t.overall);
        }
        return;
    }
    for (i, row) in col.rows.iter().take(list.height as usize).enumerate() {
        let y = list.y + i as u16;
        let hovered = col.hovered == Some(i);
        let facts = Facts {
            dir: row.dir,
            hovered,
            exec: row.exec,
            link: row.link,
            hidden: row.hidden,
            orphan: false,
        };
        let mut style = t.overall.patch(t.file_style(&row.name, &facts));
        if hovered {
            style = style.patch(match which {
                Which::Current => t.hovered,
                Which::Parent => t.parent_hovered,
                Which::Preview => t.preview_hovered,
            });
        }
        let reversed = hovered && style.add_modifier.contains(Modifier::REVERSED);
        // A pill only with its rounded caps (and the room for them).
        let pill = reversed && look.pills && list.width >= 4;
        if reversed && !pill {
            style = style
                .remove_modifier(Modifier::REVERSED)
                .add_modifier(Modifier::BOLD);
        }
        // Padding: one cell each side; a pill's rounded caps when the terminal draws them.
        let (x0, x1) = (list.x, list.right());
        let cap_style = Style::default().fg(style.fg.unwrap_or(Color::Reset));
        let mut x = x0;
        if pill {
            p.put(x, y, x1, CAP_L, cap_style);
        } else if reversed {
            p.put(x, y, x1, if look.unicode { "▌" } else { ">" }, style);
        } else {
            p.put(x, y, x1, " ", style);
        }
        x += 1;
        if !look.unicode && list.width >= 6 {
            // ASCII: `> name`, the cue a reversed row gives elsewhere.
            p.put(x, y, x1, " ", style);
            x += 1;
        }
        let inner_end = if list.width >= 3 { x1 - 1 } else { x1 };
        if pill {
            p.style_cells(x, inner_end, y, style);
            for cx in x..inner_end {
                p.put(cx, y, cx + 1, " ", style);
            }
        }
        if hovered && !reversed {
            // Underlined (preview) or other non-pill indicators span the whole row.
            p.style_cells(x0, x1, y, style);
        }
        if let Some((text, fg)) = icon(t, look, &facts) {
            let ist = if hovered {
                style
            } else {
                style.patch(Style {
                    fg: fg.or(style.fg),
                    ..Style::default()
                })
            };
            x = p.put(x, y, inner_end, text, ist);
            x = p.put(x, y, inner_end, " ", if hovered { style } else { ist });
        }
        // The linemode (current column only), right-aligned before the padding.
        let lm = if which == Which::Current {
            let room = inner_end.saturating_sub(x) as usize;
            linemode(row, mode, col.size_max, now, look.unicode, room < 48)
        } else {
            Vec::new()
        };
        let lm_w: usize = lm.iter().map(|(s, _)| width(s)).sum();
        let room = inner_end.saturating_sub(x) as usize;
        let lm_w = if lm_w + 4 <= room { lm_w } else { 0 };
        let name_room = room.saturating_sub(if lm_w > 0 { lm_w + 1 } else { 0 });
        let mut name = ellipsis(&row.name, name_room, !row.dir, look.unicode);
        if name != row.name && !row.dir && row.name.contains('.') && x > x0 + 1 {
            // yazi keeps the extension even when that takes the icon's gap (`▯….toml`).
            let wider = ellipsis(&row.name, name_room + 1, true, look.unicode);
            let ext = row.name.rfind('.').map_or("", |i| &row.name[i..]);
            if !name.ends_with(ext) && wider.ends_with(ext) {
                name = wider;
                x -= 1;
            }
        }
        // A cut name keeps the highlights that fall in its kept head.
        let head = if name == row.name {
            name.len()
        } else {
            name.find(['…', '~']).unwrap_or(0)
        };
        let mut found: Vec<(usize, usize)> = row
            .found
            .iter()
            .copied()
            .filter(|&(_, b)| b <= head)
            .collect();
        if found.is_empty() && !row.found.is_empty() {
            // Every match fell in the cut: the whole visible name says it matched.
            found.push((0, head));
        }
        if found.is_empty() {
            x = p.put(x, y, inner_end, &name, style);
        } else {
            let mut last = 0;
            for &(a, b) in &found {
                let (a, b) = (a.min(name.len()), b.min(name.len()));
                if a < last || !name.is_char_boundary(a) || !name.is_char_boundary(b) {
                    continue;
                }
                x = p.put(x, y, inner_end, &name[last..a], style);
                x = p.put(x, y, inner_end, &name[a..b], style.patch(t.find_keyword));
                last = b;
            }
            x = p.put(x, y, inner_end, &name[last..], style);
        }
        if let Some((i, n)) = row.found_pos.filter(|_| hovered) {
            let tag = format!(
                "  [{}/{}]",
                i + 1,
                if n >= 100 {
                    "99+".into()
                } else {
                    n.to_string()
                }
            );
            let lm_start = inner_end.saturating_sub(lm_w as u16 + u16::from(lm_w > 0));
            if x + width(&tag) as u16 <= lm_start {
                p.put(x, y, lm_start, &tag, style.patch(t.find_position));
            }
        }
        if lm_w > 0 {
            let mut lx = inner_end - lm_w as u16;
            for (s, kind) in &lm {
                let st = match kind {
                    1 if !hovered => style.patch(t.which_cand),
                    2 if !hovered => style.patch(t.which_rest).add_modifier(Modifier::DIM),
                    3 if !hovered => style.patch(t.which_rest),
                    _ => style,
                };
                lx = p.put(lx, y, inner_end, s, st);
            }
        }
        if list.width >= 3 {
            if pill {
                p.put(inner_end, y, x1, CAP_R, cap_style);
            } else {
                p.put(inner_end, y, x1, " ", style);
            }
        }
        // Markers: a solid bar on the column's left edge (the rail cell for the
        // current folder), foreground only.
        let marker = match row.mark {
            Mark::None => None,
            Mark::Selected => Some(t.marker_selected),
            Mark::Copied => Some(t.marker_copied),
            Mark::Cut => Some(t.marker_cut),
        };
        if let (Some(c), true) = (marker, which != Which::Preview) {
            let mx = list.x.saturating_sub(1);
            let glyph = if look.unicode { "█" } else { "|" };
            let ms = if look.color {
                Style::default().fg(c)
            } else {
                Style::default().add_modifier(Modifier::BOLD)
            };
            p.put(mx, y, mx + 1, glyph, ms);
        }
    }
}

fn status(p: &mut Pen, t: &Theme, r: Rect, s: &Status) {
    if r.width == 0 {
        return;
    }
    let look = p.look;
    let mode = match s.mode {
        ModeKind::Normal => &t.normal,
        ModeKind::Select => &t.select,
        ModeKind::Unset => &t.unset,
    };
    let pill = Style::default()
        .fg(mode.pill)
        .add_modifier(Modifier::REVERSED)
        .patch(mode.main);
    let cap = Style::default().fg(mode.pill);
    let caps = look.pills && look.unicode;
    let label = match s.mode {
        ModeKind::Normal => " NOR ",
        ModeKind::Select => " SEL ",
        ModeKind::Unset => " UNS ",
    };
    p.style_cells(r.x, r.right(), r.y, t.status);
    // Right block first (it wins the room), then the left fills what is left.
    let pos = format!(" {:>2}/{:<2} ", (s.cursor + 1).min(s.len), s.len);
    let percent = {
        let pc = if s.cursor != 0 && s.len != 0 {
            (s.cursor + 1) * 100 / s.len
        } else {
            0
        };
        match pc {
            0 => " Top ".to_string(),
            100 => " Bot ".to_string(),
            n => format!(" {n:>2}% "),
        }
    };
    let perm = s.perm.clone().unwrap_or_default();
    let right_w = width(&perm) + 1 + width(&percent) + width(&pos) + usize::from(caps);
    let full = right_w + 30 <= r.width as usize;
    let mut rx = r.right();
    if full {
        if caps {
            rx -= 1;
            p.put(rx, r.y, rx + 1, CAP_R, cap);
        }
        rx -= width(&pos) as u16;
        p.put(rx, r.y, r.right(), &pos, pill);
        rx -= width(&percent) as u16;
        p.put(rx, r.y, r.right(), &percent, mode.alt);
        rx -= 1;
        rx -= width(&perm) as u16;
        let mut x = rx;
        for c in perm.chars() {
            let st = match c {
                'r' => t.perm_read,
                'w' => t.perm_write,
                'x' | 's' | 't' => t.perm_exec,
                '-' | '?' => t.perm_sep,
                _ => t.perm_type,
            };
            x = p.put(x, r.y, r.right(), &c.to_string(), st);
        }
    }
    let end = if full {
        rx.saturating_sub(1)
    } else {
        r.right()
    };
    let mut x = r.x;
    if caps {
        x = p.put(x, r.y, end, CAP_L, cap);
    }
    x = p.put(x, r.y, end, label, pill);
    if let Some(size) = s.size {
        x = p.put(x, r.y, end, &format!(" {} ", readable_size(size)), mode.alt);
    }
    if !s.name.is_empty() {
        let room = end.saturating_sub(x + 1) as usize;
        x = p.put(
            x,
            r.y,
            end,
            &format!(" {}", ellipsis(&s.name, room, true, look.unicode)),
            t.status,
        );
    }
    // A transfer's progress or a message: between the name and the right block.
    let mid = s
        .progress
        .as_ref()
        .map(|(label, frac, error)| {
            let pct = frac.map(|f| format!(" {:>3}%", (f * 100.0).round() as u32));
            (
                format!("{label}{}", pct.unwrap_or_default()),
                *frac,
                if *error { t.progress_error } else { t.progress },
            )
        })
        .or_else(|| s.message.clone().map(|m| (m, None, t.status)));
    if let Some((text, frac, st)) = mid {
        let room = end.saturating_sub(x + 2);
        let bar = if frac.is_some() && room >= 30 {
            10u16
        } else {
            0
        };
        let tw = (width(&text) as u16).min(room.saturating_sub(bar + 1));
        if tw >= 4 {
            let start = end.saturating_sub(tw + bar + 2);
            let mut cx = p.put(
                start,
                r.y,
                end,
                &ellipsis(&text, tw as usize, false, look.unicode),
                st,
            );
            if let (Some(f), true) = (frac, bar > 0) {
                cx += 1;
                let filled = ((f.clamp(0.0, 1.0)) * f64::from(bar)).round() as u16;
                let (a, b) = if look.unicode {
                    ("━", "─")
                } else {
                    ("=", "-")
                };
                for i in 0..bar {
                    p.put(
                        cx + i,
                        r.y,
                        end,
                        if i < filled { a } else { b },
                        if i < filled { st } else { t.which_rest },
                    );
                }
            }
        }
    }
}

fn which_key(p: &mut Pen, t: &Theme, area: Rect, bottom: u16, items: &[(String, String)]) {
    if items.is_empty() || area.width < 8 {
        return;
    }
    let cols = t.which_cols.max(1);
    let rows = items.len().div_ceil(cols) as u16;
    let h = (rows + 2).min(bottom.saturating_sub(area.y));
    if h < 3 {
        return;
    }
    let r = Rect::new(area.x + 1, bottom - h, area.width - 2, h);
    p.rounded(r, t.which_border, None);
    let inner = Rect::new(r.x + 1, r.y + 1, r.width.saturating_sub(2), h - 2);
    let iw = u32::from(inner.width);
    let start = |c: usize| inner.x + ((iw * c as u32 * 2 + cols as u32) / (2 * cols as u32)) as u16;
    for (k, (key, desc)) in items.iter().enumerate() {
        let (row, col) = ((k / cols) as u16, k % cols);
        if row >= inner.height {
            break;
        }
        let y = inner.y + row;
        let (x0, x1) = (start(col), start(col + 1).min(inner.right()));
        let kw = width(key) as u16;
        let kx = (x0 + 10).saturating_sub(kw).max(x0);
        let mut x = p.put(kx, y, x1, key, t.which_cand);
        let sep = if p.look.unicode && p.look.icons {
            t.which_sep.clone()
        } else {
            " → ".into()
        };
        let sep = if p.look.unicode { sep } else { " - ".into() };
        x = p.put(x, y, x1, &sep, t.which_sep_style);
        p.put(x, y, x1, desc, t.which_desc);
    }
}

fn popup(
    p: &mut Pen,
    t: &Theme,
    area: Rect,
    current: Rect,
    hovered_y: Option<u16>,
    pop: &Popup,
    d: &mut Drawn,
) {
    let u = p.look.unicode;
    match pop {
        Popup::Input {
            title,
            value,
            cursor,
            anchor,
        } => {
            let w = 50.min(area.width);
            let (x, y) = match anchor {
                Anchor::TopCenter => (area.x + (area.width - w) / 2, area.y + 2),
                Anchor::Hovered => (
                    current.x.min(area.right().saturating_sub(w)),
                    hovered_y.map_or(area.y + 2, |y| y + 1),
                ),
            };
            let y = y.min(area.bottom().saturating_sub(3));
            let r = Rect::new(x, y, w, 3.min(area.height)).intersection(area);
            p.rounded(r, t.input_border, Some((title, t.input_title, false)));
            if r.height >= 3 && r.width > 2 {
                let room = (r.width - 2) as usize;
                // Scroll so the cursor stays visible.
                let chars: Vec<char> = value.chars().collect();
                let cur = (*cursor).min(chars.len());
                let mut start = 0;
                while width(&chars[start..cur].iter().collect::<String>()) >= room && start < cur {
                    start += 1;
                }
                let shown: String = chars[start..].iter().collect();
                p.put(r.x + 1, r.y + 1, r.right() - 1, &shown, t.input_value);
                let cx = r.x + 1 + width(&chars[start..cur].iter().collect::<String>()) as u16;
                d.cursor = Some((cx.min(r.right() - 2), r.y + 1));
            }
        }
        Popup::Confirm { title, body, list } => {
            let w = 70.min(area.width.saturating_sub(2)).max(20.min(area.width));
            let lines = list.len() as u16 + u16::from(body.is_some()) * 2;
            let h = (lines + 5).min(20).min(area.height);
            let r = Rect::new(
                area.x + (area.width - w) / 2,
                area.y + (area.height - h) / 2,
                w,
                h,
            );
            p.rounded(r, t.confirm_border, Some((title, t.confirm_title, true)));
            if r.height < 4 || r.width < 6 {
                return;
            }
            let mut y = r.y + 1;
            let end = r.right() - 1;
            if let Some(b) = body {
                p.put(r.x + 2, y, end, b, t.confirm_body);
                y += 2;
            }
            for item in list {
                if y + 2 >= r.bottom() {
                    break;
                }
                p.put(
                    r.x + 2,
                    y,
                    end,
                    &ellipsis(item, (r.width - 4) as usize, true, u),
                    t.confirm_body,
                );
                y += 1;
            }
            let by = r.bottom() - 2;
            let (yes, no) = &t.btn_labels;
            let bw = (width(yes) + width(no) + 2) as u16;
            let bx = r.x + r.width.saturating_sub(bw) / 2;
            let nx = p.put(bx, by, end, yes, t.btn_yes);
            p.put(nx + 2, by, end, no, t.btn_no);
        }
        Popup::Help {
            rows,
            selected,
            filter,
        } => {
            let w = 80.min(area.width);
            let h = 25.min(area.height);
            let r = Rect::new(
                area.x + (area.width - w) / 2,
                area.y + (area.height - h).div_ceil(2),
                w,
                h,
            );
            p.rounded(r, t.help_border, Some(("mgr.help", t.help_border, true)));
            if r.height < 5 || r.width < 4 {
                return;
            }
            let end = r.right() - 1;
            p.put(r.x + 1, r.y + 1, end, filter, Style::default());
            if !filter.is_empty() {
                d.cursor = Some(((r.x + 1 + width(filter) as u16).min(end - 1), r.y + 1));
            }
            let (l, m, rr) = if u {
                ("├", "─", "┤")
            } else {
                ("+", "-", "+")
            };
            p.put(r.x, r.y + 2, r.x + 1, l, t.help_border);
            for x in r.x + 1..end {
                p.put(x, r.y + 2, x + 1, m, t.help_border);
            }
            p.put(end, r.y + 2, end + 1, rr, t.help_border);
            let body = r.height.saturating_sub(4) as usize;
            let first = selected.saturating_sub(body.saturating_sub(1));
            for (i, (key, desc)) in rows.iter().enumerate().skip(first).take(body) {
                let y = r.y + 3 + (i - first) as u16;
                let hov = i == *selected;
                let base = if hov {
                    t.help_hovered
                } else {
                    Style::default()
                };
                if hov {
                    p.style_cells(r.x + 1, end, y, base);
                    // yazi's: the key column of the hovered row in the key colour.
                    p.style_cells(r.x + 1, (r.x + 21).min(end), y, base.patch(t.help_on));
                }
                let x = p.put(r.x + 1, y, end, key, base.patch(t.help_on));
                let kx = (r.x + 21).max(x + 1);
                p.put(kx, y, end, desc, base);
            }
        }
        Popup::Pick {
            title,
            items,
            selected,
        } => {
            let w = 50.min(area.width);
            let h = (items.len() as u16 + 2).min(9).min(area.height);
            let y = hovered_y
                .map_or(area.y + 2, |y| y + 1)
                .min(area.bottom().saturating_sub(h));
            let r = Rect::new(current.x.min(area.right().saturating_sub(w)), y, w, h);
            p.rounded(r, t.input_border, Some((title, t.input_title, false)));
            let end = r.right().saturating_sub(1);
            let body = h.saturating_sub(2) as usize;
            let first = selected.saturating_sub(body.saturating_sub(1));
            for (i, item) in items.iter().enumerate().skip(first).take(body) {
                let y = r.y + 1 + (i - first) as u16;
                let st = if i == *selected {
                    t.hovered
                } else {
                    t.tab_inactive
                };
                if i == *selected {
                    p.style_cells(r.x + 1, end, y, st);
                }
                p.put(r.x + 1, y, end, &format!(" {item}"), st);
            }
        }
    }
}

/// Draw the whole Files view into `area`.
pub(crate) fn render(v: &View, t: &Theme, look: Look, area: Rect, buf: &mut Buffer) -> Drawn {
    let mut d = Drawn::default();
    let mut p = Pen { buf, look };
    p.blank(area);
    if area.width < 4 || area.height < 3 {
        return d;
    }
    let tabs_h = u16::from(v.tabs.len() > 1 && area.height >= 6);
    let header = Rect::new(area.x, area.y, area.width, 1);
    let status_r = Rect::new(area.x, area.bottom() - 1, area.width, 1);
    let body = Rect::new(
        area.x,
        area.y + 1 + tabs_h,
        area.width,
        area.height.saturating_sub(2 + tabs_h),
    );
    // Header: cwd (cut from the left) and flags; right: selected or yanked count.
    let right = v.count.map(|(n, kind)| {
        (
            format!(" {n} "),
            match kind {
                Mark::Cut => t.count_cut,
                Mark::Copied => t.count_copied,
                _ => t.count_selected,
            },
        )
    });
    let rw = right.as_ref().map_or(0, |(s, _)| width(s) as u16 + 1);
    let left_room = header.width.saturating_sub(rw) as usize;
    let flags_w = width(&v.flags).min(left_room / 2);
    let cwd = ellipsis_left(&v.cwd, left_room - flags_w, look.unicode);
    let x = p.put(header.x, header.y, header.right() - rw, &cwd, t.cwd);
    p.put(x, header.y, header.right() - rw, &v.flags, t.cwd);
    if let Some((s, st)) = &right {
        let x = header.right() - rw;
        p.put(x, header.y, header.right(), s, *st);
    }
    // Tabs.
    if tabs_h == 1 {
        let y = area.y + 1;
        let max = (area.width as usize / v.tabs.len()).max(4);
        let mut x = area.x;
        for (i, label) in v.tabs.iter().enumerate() {
            let text = ellipsis(&format!(" {} {} ", i + 1, label), max, false, look.unicode);
            let st = if i == v.tab {
                t.tab_active
            } else {
                t.tab_inactive
            };
            let start = x;
            x = p.put(x, y, area.right(), &text, st);
            d.tabs.push((Rect::new(start, y, x - start, 1), i));
        }
    }
    let (pr, cr, vr) = columns(body);
    // Parent list: inset one cell on the left (yazi's).
    let parent_list = Rect::new(
        pr.x + pr.width.min(1),
        pr.y,
        pr.width.saturating_sub(1),
        pr.height,
    );
    let rails = cr.width >= 3;
    let current_list = if rails {
        Rect::new(cr.x + 1, cr.y, cr.width - 2, cr.height)
    } else {
        cr
    };
    d.preview = vr;
    d.current = current_list;
    d.parent = parent_list;
    if rails {
        let sym = if look.unicode {
            t.border_symbol.as_str()
        } else {
            "|"
        };
        for y in body.top()..body.bottom() {
            p.put(cr.x, y, cr.x + 1, sym, t.border);
            p.put(cr.right() - 1, y, cr.right(), sym, t.border);
        }
    }
    if let Some(parent) = &v.parent {
        column(
            &mut p,
            t,
            parent_list,
            parent,
            Which::Parent,
            v.linemode,
            v.now,
        );
    }
    column(
        &mut p,
        t,
        current_list,
        &v.current,
        Which::Current,
        v.linemode,
        v.now,
    );
    if let Some(Preview::Folder(c)) = &v.preview {
        column(&mut p, t, vr, c, Which::Preview, v.linemode, v.now);
    }
    status(&mut p, t, status_r, &v.status);
    d
}

/// The which-key panel and popups, drawn after the app's preview body.
pub(crate) fn overlays(
    v: &View,
    t: &Theme,
    look: Look,
    area: Rect,
    buf: &mut Buffer,
    d: &mut Drawn,
) {
    if area.width < 4 || area.height < 3 {
        return;
    }
    let mut p = Pen { buf, look };
    let status_y = area.bottom() - 1;
    let hovered_y = v.current.hovered.map(|i| d.current.y + i as u16);
    if !v.which.is_empty() {
        which_key(&mut p, t, area, status_y.saturating_sub(1), &v.which);
    }
    if let Some(pop) = &v.popup {
        let current = d.current;
        popup(&mut p, t, area, current, hovered_y, pop, d);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_and_days_read_like_yazi() {
        assert_eq!(readable_size(242), "242B");
        assert_eq!(readable_size(4096), "4K");
        assert_eq!(readable_size(4000), "3.9K");
        assert_eq!(readable_size(5 * 1024 * 1024 + 1), "5M");
        assert_eq!(perm_string(0o755, true, false), "drwxr-xr-x");
        assert_eq!(perm_string(0o644, false, false), "-rw-r--r--");
        // 2026-11-12 12:00 UTC.
        let now = 1_794_484_800;
        assert_eq!(day(now - 3600, now), "11:00");
        assert_eq!(day(now - 86400 * 2, now), "11/10");
        assert_eq!(day(now - 86400 * 400, now), "2025");
        assert_eq!(ellipsis("config.toml", 10, true, true), "conf….toml");
        assert_eq!(ellipsis("archive", 4, false, true), "arc…");
    }

    /// Every overlay and linemode at every size from 1x1 to 250x80: no panic.
    #[test]
    fn renders_at_every_size_without_panicking() {
        use super::super::theme::Theme;
        let row = |name: &str, dir: bool, mark: Mark| Row {
            name: name.into(),
            dir,
            size: 4096,
            modified: Some(1_794_484_800),
            mode: Some(0o755),
            mark,
            found: vec![(0, 1)],
            ..Row::default()
        };
        let col = Column {
            rows: vec![
                row("assets", true, Mark::Selected),
                row(
                    "a-very-long-file-name-that-will-not-fit.toml",
                    false,
                    Mark::Copied,
                ),
                row("終わり.txt", false, Mark::Cut),
            ],
            hovered: Some(1),
            empty: None,
            size_max: 1 << 30,
        };
        let popups = [
            None,
            Some(Popup::Input {
                title: "Filter:".into(),
                value: "abc".into(),
                cursor: 3,
                anchor: Anchor::Hovered,
            }),
            Some(Popup::Confirm {
                title: "Trash 3 selected files?".into(),
                body: Some("No undo".into()),
                list: vec!["a".into(); 30],
            }),
            Some(Popup::Help {
                rows: vec![("<C-u>".into(), "Move".into()); 200],
                selected: 150,
                filter: "m".into(),
            }),
            Some(Popup::Pick {
                title: "Open with:".into(),
                items: vec!["nvim".into(); 12],
                selected: 11,
            }),
        ];
        let modes = [
            Linemode::Bar,
            Linemode::Size,
            Linemode::Mtime,
            Linemode::Permissions,
            Linemode::None,
        ];
        let themes = [Theme::builtin()];
        for (i, popup) in popups.iter().enumerate() {
            let v = View {
                cwd: "~/a/very/long/path/that/goes/on".into(),
                flags: " (filter: x)".into(),
                count: Some((3, Mark::Selected)),
                tabs: vec!["a:one".into(), "b:two".into()],
                tab: 1,
                parent: Some(col.clone()),
                current: col.clone(),
                preview: Some(Preview::Folder(col.clone())),
                status: Status {
                    size: Some(5),
                    name: "x".into(),
                    perm: Some("drwxr-xr-x".into()),
                    cursor: 1,
                    len: 3,
                    progress: Some(("Copying x".into(), Some(0.5), false)),
                    ..Status::default()
                },
                which: if i % 2 == 0 {
                    vec![("c".into(), "Copy file path".into()); 7]
                } else {
                    Vec::new()
                },
                popup: popup.clone(),
                linemode: modes[i % modes.len()],
                now: 1_794_484_800,
            };
            for unicode in [true, false] {
                let look = Look {
                    unicode,
                    icons: unicode,
                    pills: unicode,
                    color: unicode,
                };
                // Every width with a few heights, and every height at a few widths.
                let sizes = (1..=250u16)
                    .flat_map(|w| [1, 3, 9, 24, 80].map(|h| (w, h)))
                    .chain(
                        [1u16, 20, 80]
                            .into_iter()
                            .flat_map(|w| (1..=80).map(move |h| (w, h))),
                    );
                for (w, h) in sizes {
                    {
                        let area = Rect::new(0, 0, w, h);
                        let mut buf = Buffer::empty(area);
                        let mut d = render(&v, &themes[0], look, area, &mut buf);
                        overlays(&v, &themes[0], look, area, &mut buf, &mut d);
                    }
                }
            }
        }
    }

    #[test]
    fn columns_split_one_four_three() {
        let (a, b, c) = columns(Rect::new(0, 1, 120, 38));
        assert_eq!((a.width, b.x, b.width, c.x, c.width), (15, 15, 60, 75, 45));
        let (a, b, c) = columns(Rect::new(0, 1, 80, 22));
        assert_eq!((a.width, b.x, c.x), (10, 10, 50));
    }
}
