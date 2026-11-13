//! `mission`: "mission control". A calm, dense-but-airy dashboard. The hub is the
//! network as an orbit around this device (rings by hop, meshes as faint orbits, curved
//! links whose stroke is the access policy) over the selected device's vitals, one row
//! of live session thumbnails and an alert ticker that pairs each problem with its keys.
//! One grammar everywhere: a device letter, then a verb or a session slot.
//! See docs/design/concept-mission.md.
use crate::ui::views::{
    caps::{index_rgb, named_index, rgb_to_16, rgb_to_256, XTERM16},
    fmt, ColorLevel, DeviceStatus, LabCaps, LabDevice, LabSession, LabState, LabTarget, LabView,
    SessionState, Theme, Tok,
};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
};
use std::cell::RefCell;

mod hub;
mod orbit;
mod tiles;
mod views;

/// What one production frame drew: its tap targets and the hub's device order (the
/// map's or the phone tree's), so keys move through exactly what is on screen.
pub struct Drawn {
    pub targets: Vec<LabTarget>,
    pub order: Vec<String>,
    /// Each map node (`device:<id>`) and session tile (`session:<d>:<id>`) where it was
    /// drawn, so the arrows move between them as they sit on screen.
    pub nav: Vec<LabTarget>,
}

/// The production entry point: one draw that also returns the frame's targets.
pub fn render(
    view: &LabView,
    state: &LabState,
    caps: &LabCaps,
    area: Rect,
    buf: &mut Buffer,
) -> Drawn {
    let cx = Cx::new(state, caps, area);
    cx.draw(view, buf);
    Drawn {
        targets: cx.hits.into_inner(),
        order: cx.order.into_inner(),
        nav: cx.nav.into_inner(),
    }
}

/// The palette over whatever screen is already in `buf` (dimmed under it); returns its
/// targets, which replace the screen's (only the sheet is live).
pub fn render_palette(
    state: &LabState,
    caps: &LabCaps,
    area: Rect,
    buf: &mut Buffer,
    query: &str,
    cursor: usize,
) -> Vec<LabTarget> {
    let cx = Cx::new(state, caps, area);
    if area.width < 20 || area.height < 6 {
        return Vec::new();
    }
    cx.dim(buf, area);
    views::palette(&cx, buf, area, query, cursor);
    cx.hits.into_inner()
}

/// Session ids in hub order (what needs you first), as the hub's tiles rank them.
pub fn by_attention(state: &LabState) -> Vec<String> {
    let caps = LabCaps::default();
    let cx = Cx::new(state, &caps, Rect::default());
    cx.by_attention().iter().map(|x| x.id.clone()).collect()
}

pub(super) type Rgb = (u8, u8, u8);

/// Width class: phone, small, medium, large.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub(super) enum Size {
    Xs,
    S,
    M,
    L,
}

/// Session slot keys inside a device (home row; `f` stays "files", `j k` stay motion as
/// in files and every list; `h l` stay fold keys as in files). Sessions past the fourth
/// are reached by the arrows, `Tab` or `/`.
pub const SLOTS: [char; 4] = ['a', 's', 'd', 'g'];

/// A pill's rounded caps (powerline extra symbols, drawn by kitty, ghostty and wezterm).
pub const PILL_L: &str = "\u{e0b6}";
pub const PILL_R: &str = "\u{e0b4}";

/// Render context: fixture, capabilities and the tap targets recorded while drawing.
pub(super) struct Cx<'a> {
    pub s: &'a LabState,
    pub c: &'a LabCaps,
    pub area: Rect,
    hits: RefCell<Vec<LabTarget>>,
    /// The hub's device order as drawn (see `Drawn`).
    pub order: RefCell<Vec<String>>,
    /// Where each navigable item was drawn (see `Drawn::nav`).
    nav: RefCell<Vec<LabTarget>>,
}

impl<'a> Cx<'a> {
    fn new(s: &'a LabState, c: &'a LabCaps, area: Rect) -> Self {
        Self {
            s,
            c,
            area,
            hits: RefCell::new(Vec::new()),
            order: RefCell::new(Vec::new()),
            nav: RefCell::new(Vec::new()),
        }
    }

    fn draw(&self, view: &LabView, buf: &mut Buffer) {
        if self.area.width < 20 || self.area.height < 6 {
            self.put(
                buf,
                self.area.x,
                self.area.y,
                self.area.width,
                "cx",
                self.st(Tok::Accent),
            );
            return;
        }
        views::draw(self, view, buf);
    }

    pub fn size(&self) -> Size {
        match self.area.width {
            0..=59 => Size::Xs,
            60..=99 => Size::S,
            100..=159 => Size::M,
            _ => Size::L,
        }
    }

    /// Touch-first layouts (phones up to about 70 columns): bigger targets, thumb footer.
    pub fn touch(&self) -> bool {
        self.area.width < 70
    }

    pub fn uni(&self) -> bool {
        self.c.unicode
    }
    pub fn g(&self, u: &'static str, a: &'static str) -> &'static str {
        self.c.glyph(u, a)
    }
    /// The Enter key's label (`ret` in ASCII).
    pub fn ret(&self) -> &'static str {
        self.key_label("⏎")
    }
    /// A key's label, one set everywhere (footer, help, chips, palette): `␣ ⏎ ⌫`, and
    /// in ASCII the words `spc ret bksp`.
    pub fn key_label<'k>(&self, k: &'k str) -> &'k str {
        if self.uni() {
            return k;
        }
        match k {
            "␣" => "spc",
            "⏎" => "ret",
            "⌫" => "bksp",
            "↑↓" => "up/dn",
            other => other,
        }
    }
    pub fn st(&self, t: Tok) -> Style {
        // 16 colours: brightblack is the background in some palettes (Solarized Dark's
        // base03), so evidence ink is the default foreground, dimmed; brightblack stays
        // for faint strokes and beads only.
        if self.c.color == ColorLevel::C16 && matches!(t, Tok::Muted) {
            return Style::default().add_modifier(Modifier::DIM);
        }
        self.c.tok(t)
    }
    pub fn tc(&self) -> bool {
        matches!(self.c.color, ColorLevel::Truecolor | ColorLevel::C256)
    }
    pub fn dark(&self) -> bool {
        self.c.theme == Theme::Dark
    }

    pub fn fg0(&self) -> Rgb {
        if self.dark() {
            (0xd5, 0xd9, 0xe0)
        } else {
            (0x24, 0x29, 0x2f)
        }
    }
    pub fn bg0(&self) -> Rgb {
        if self.dark() {
            (0x12, 0x15, 0x19)
        } else {
            (0xfa, 0xfa, 0xf8)
        }
    }
    /// A token's truecolor value for this theme (used for blending).
    pub fn rgb(&self, t: Tok) -> Rgb {
        let mut tc = *self.c;
        tc.color = ColorLevel::Truecolor;
        match tc.color(t) {
            Some(Color::Rgb(r, g, b)) => (r, g, b),
            _ => self.fg0(),
        }
    }
    /// An arbitrary rgb at this terminal's color level.
    pub fn col(&self, c: Rgb) -> Option<Color> {
        match self.c.color {
            ColorLevel::Truecolor => Some(Color::Rgb(c.0, c.1, c.2)),
            ColorLevel::C256 => Some(Color::Indexed(rgb_to_256(c.0, c.1, c.2))),
            ColorLevel::C16 => Some(crate::ui::views::caps::indexed_16(rgb_to_16(c.0, c.1, c.2))),
            ColorLevel::None => None,
        }
    }
    /// Foreground: token blended toward the background by `a` (1.0 = full). In 16 colors
    /// and NO_COLOR, blends become the token itself (dimmed when faint).
    pub fn ink(&self, t: Tok, a: f32) -> Style {
        if self.tc() {
            let c = mix(self.bg0(), self.rgb(t), a);
            Style::default().fg(self.col(c).unwrap_or(Color::Reset))
        } else {
            let s = self.st(t);
            if a < 0.55 {
                s.add_modifier(Modifier::DIM)
            } else {
                s
            }
        }
    }
    /// Background tint: none. Terminals are often transparent, and a painted cell reads
    /// as a slab over the wallpaper: hierarchy is ink, weight, rules and glyphs.
    pub fn wash(&self, _t: Tok, _a: f32) -> Style {
        Style::default()
    }
    pub fn rgb_fg(&self, c: Rgb) -> Style {
        match self.col(c) {
            Some(c) => Style::default().fg(c),
            None => Style::default(),
        }
    }
    pub fn rgb_bg(&self, c: Rgb) -> Style {
        match self.col(c) {
            Some(c) => Style::default().bg(c),
            None => Style::default(),
        }
    }
    /// Panels and tiles have no surface of their own (see `wash`).
    pub fn surface(&self) -> Style {
        Style::default()
    }
    pub fn has_surface(&self) -> bool {
        false
    }

    /// Value 0..1 to a calm ok → warn → err ramp.
    /// Sparkline and chart fill: the heat ramp, but on a light theme calm values are a
    /// half tint, so the bars never outweigh the names; only warm values keep full hue.
    pub fn spark_heat(&self, v: f32) -> Style {
        if self.tc() && !self.dark() && v < 0.6 {
            let c = mix(self.rgb(Tok::Ok), self.rgb(Tok::Warn), v.max(0.0) / 0.6);
            self.rgb_fg(mix(self.bg0(), c, 0.5))
        } else {
            self.heat(v)
        }
    }
    pub fn heat(&self, v: f32) -> Style {
        let v = v.clamp(0.0, 1.0);
        if self.tc() {
            let (a, b, t) = if v < 0.6 {
                (self.rgb(Tok::Ok), self.rgb(Tok::Warn), v / 0.6)
            } else {
                (self.rgb(Tok::Warn), self.rgb(Tok::Err), (v - 0.6) / 0.4)
            };
            self.rgb_fg(mix(a, b, t))
        } else if v < 0.6 {
            self.st(Tok::Ok)
        } else if v < 0.85 {
            self.st(Tok::Warn)
        } else {
            self.st(Tok::Err)
        }
    }

    pub fn hit(&self, r: Rect, action: impl Into<String>) {
        let r = r.intersection(self.area);
        if r.width > 0 && r.height > 0 {
            self.hits.borrow_mut().push(LabTarget {
                area: r,
                action: action.into(),
            });
        }
    }

    /// Record a navigable item's place (a node or a tile; see `Drawn::nav`).
    pub fn nav(&self, r: Rect, action: String) {
        self.nav.borrow_mut().push(LabTarget { area: r, action });
    }

    /// Number of targets recorded so far (see `unclash`).
    pub fn hit_mark(&self) -> usize {
        self.hits.borrow().len()
    }
    /// Make the targets recorded since `from` disjoint: where two overlap, the larger one
    /// gives up the overlapping rows (or columns, when the overlap spans its full height),
    /// so a thumb tap between two nodes is never ambiguous. Repeats until no pair
    /// overlaps (a trim can change which of two targets is the larger).
    pub fn unclash(&self, from: usize, covered: bool) {
        let mut v = self.hits.borrow_mut();
        let area = |r: Rect| u32::from(r.width) * u32::from(r.height);
        for _ in 0..8 {
            let mut changed = false;
            let n = v.len();
            for i in from..n {
                for j in from..n {
                    let (a, b) = (v[i].area, v[j].area);
                    if i == j || area(a) < area(b) || (area(a) == area(b) && i > j) {
                        continue;
                    }
                    let o = a.intersection(b);
                    if o.width == 0 || o.height == 0 {
                        continue;
                    }
                    // Trim `a` (the larger) on the side where the overlap sits.
                    let mut r = a;
                    if o.height == a.height {
                        if o.x == a.x {
                            r.x = o.right().min(a.right());
                            r.width = a.right() - r.x;
                        } else {
                            r.width = o.x - a.x;
                        }
                    } else if o.y == a.y {
                        r.y = o.bottom().min(a.bottom());
                        r.height = a.bottom() - r.y;
                    } else {
                        r.height = o.y - a.y;
                    }
                    v[i].area = r;
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        v.retain(|t| t.area.width > 0 && t.area.height > 0);
        if !self.touch() {
            return;
        }
        // Touch: a trimmed target grows back toward 3 rows by 7 columns into free cells.
        // One that still cannot is dropped when `covered` (the phone hub, whose device
        // tree rows are the thumb targets); elsewhere the lint names it.
        let ok = |r: Rect| r.height >= 3 && r.width >= 7;
        for i in from..v.len() {
            for _ in 0..8 {
                let r = v[i].area;
                if ok(r) {
                    break;
                }
                let mut tries = Vec::new();
                if r.height < 3 {
                    tries.push(Rect::new(r.x, r.y + r.height, r.width, 1));
                    tries.push(Rect::new(r.x, r.y.saturating_sub(1), r.width, 1));
                }
                if r.width < 7 {
                    tries.push(Rect::new(r.x + r.width, r.y, 1, r.height));
                    tries.push(Rect::new(r.x.saturating_sub(1), r.y, 1, r.height));
                }
                let free = |g: Rect| {
                    g.intersection(self.area) == g
                        && !r.intersects(g)
                        && (from..v.len()).all(|j| j == i || !v[j].area.intersects(g))
                };
                match tries.into_iter().find(|g| free(*g)) {
                    Some(g) => v[i].area = r.union(g),
                    None => break,
                }
            }
        }
        if covered {
            let keep: Vec<LabTarget> = v.drain(from..).filter(|t| ok(t.area)).collect();
            v.extend(keep);
        } else if cfg!(test) {
            // (The lab's lint; the app's stderr is the screen.)
            for t in v[from..].iter().filter(|t| !ok(t.area)) {
                eprintln!("lint touch target {} under 3x7", t.action);
            }
        }
    }
    /// Drop every target recorded so far: an overlay (palette, sheet) owns the taps, so
    /// nothing under its scrim stays live.
    pub fn clear_hits(&self) {
        self.hits.borrow_mut().clear();
    }

    /// The focused selection's tint (`accent.bg`); nothing without tints, where the bar
    /// and a bold accent name carry the selection (never a reversed block).
    pub fn sel_bg(&self) -> Style {
        if self.has_surface() {
            self.st(Tok::AccentBg)
        } else {
            Style::default()
        }
    }
    /// The selection bar down the left of `h` rows: `▌` (ASCII `>` on the first row and
    /// `|` below, so a column of `>` never reads as shell prompts).
    pub fn bar(&self, buf: &mut Buffer, x: u16, y: u16, h: u16, st: Style) {
        self.bar_in(buf, x, y, h, st, true);
    }
    /// The bar of a selection in a focused or unfocused zone: without colour the two
    /// would differ only by ink, so ASCII marks the unfocused one `|` on every row and
    /// heads the focused one with `>` (`:` is a password line on the map).
    pub fn bar_in(&self, buf: &mut Buffer, x: u16, y: u16, h: u16, st: Style, focused: bool) {
        for yy in y..y + h {
            let a = match (focused, yy == y) {
                (false, _) => "|",
                (true, true) => ">",
                (true, false) => "|",
            };
            self.put(buf, x, yy, 1, self.g("▌", a), st);
        }
    }

    /// Write text clipped to `max` columns and the frame; returns columns written.
    /// ASCII mode transliterates fixture text so nothing non-ASCII leaks.
    pub fn put(&self, buf: &mut Buffer, x: u16, y: u16, max: u16, s: &str, st: Style) -> u16 {
        let a = buf.area;
        if y < a.y || y >= a.bottom() || x < a.x || x >= a.right() || max == 0 {
            return 0;
        }
        let max = max.min(a.right() - x);
        let text;
        let s = if self.uni() {
            s
        } else {
            text = asciify(s);
            &text
        };
        let (nx, _) = buf.set_stringn(x, y, s, usize::from(max), st);
        nx.saturating_sub(x)
    }
    /// Text right-aligned so it ends at `right` (exclusive).
    pub fn put_r(&self, buf: &mut Buffer, right: u16, y: u16, s: &str, st: Style) -> u16 {
        let w = width(s, self.uni());
        let x = right.saturating_sub(w).max(buf.area.x);
        self.put(buf, x, y, right.saturating_sub(x), s, st)
    }
    pub fn fill(&self, buf: &mut Buffer, r: Rect, st: Style) {
        let r = r.intersection(buf.area);
        for y in r.top()..r.bottom() {
            for x in r.left()..r.right() {
                let c = &mut buf[(x, y)];
                c.reset();
                c.set_style(st);
            }
        }
    }
    /// Patch a style over existing cells (keeps symbols).
    pub fn paint(&self, buf: &mut Buffer, r: Rect, st: Style) {
        let r = r.intersection(buf.area);
        for y in r.top()..r.bottom() {
            for x in r.left()..r.right() {
                buf[(x, y)].set_style(st);
            }
        }
    }
    /// Fade everything under an overlay: a scrim that keeps the picture (colours pulled
    /// toward the background); below 256 colours the backdrop is cleared.
    pub fn dim(&self, buf: &mut Buffer, r: Rect) {
        let r = r.intersection(buf.area);
        let bg0 = self.bg0();
        for y in r.top()..r.bottom() {
            for x in r.left()..r.right() {
                let c = &mut buf[(x, y)];
                if self.tc() {
                    let fg = to_rgb(c.fg).unwrap_or(self.fg0());
                    // One level for every ink (names, strokes, tile text, bold or screen-authored).
                    c.fg = self.col(mix(bg0, fg, 0.17)).unwrap_or(Color::Reset);
                    // No cell behind a backdrop (a dimmed pill or diff line would be a
                    // slab over a transparent terminal), and a pill's caps go with it.
                    c.bg = Color::Reset;
                    if matches!(c.symbol(), PILL_L | PILL_R) {
                        c.set_symbol(" ");
                    }
                    c.modifier = Modifier::empty();
                } else {
                    // No blending: clear the backdrop rather than let it bleed around
                    // the overlay as half-legible fragments.
                    c.reset();
                }
            }
        }
    }

    /// Identity keycap ` i ` in the device hue; returns its width.
    pub fn dchip(&self, buf: &mut Buffer, x: u16, y: u16, d: &LabDevice, narrow: bool) -> u16 {
        let st = self.chip_style(self.s.hue(&d.id));
        let k = d.hotkey.to_string();
        if narrow {
            self.put(buf, x, y, 1, &k, st)
        } else {
            self.keycap(buf, x, y, 3, &k, st)
        }
    }
    /// Key chips as pills: where the terminal draws rounded powerline caps (ui::caps),
    /// in 256 colours or more, and not on a phone.
    pub fn pills(&self) -> bool {
        self.c.pills && self.uni() && self.tc() && !self.touch()
    }
    /// A keycap ` k ` (the key's width + 2), its label from `key_label`: bold `st` ink
    /// with nothing behind it; or, with `pills`, a pill in yazi's style: the rounded
    /// caps in the key's colour and the key on that colour. A pill is the only cell
    /// background cx's chrome paints, and only between its two caps (the caps' own
    /// cells stay transparent around the round).
    pub fn keycap(&self, buf: &mut Buffer, x: u16, y: u16, max: u16, k: &str, st: Style) -> u16 {
        let k = self.key_label(k);
        let w = width(k, self.uni()) + 2;
        match st
            .fg
            .filter(|_| self.pills() && w <= max.min(buf.area.right().saturating_sub(x)))
        {
            Some(c) => {
                let on = self.col(self.bg0()).unwrap_or(Color::Reset);
                let cap = Style::default().fg(c);
                self.put(buf, x, y, 1, PILL_L, cap);
                let kst = Style::default().fg(on).bg(c).add_modifier(Modifier::BOLD);
                self.put(buf, x + 1, y, w - 2, k, kst);
                self.put(buf, x + w - 1, y, 1, PILL_R, cap);
                w
            }
            None => self.put(buf, x, y, max, &format!(" {k} "), st),
        }
    }
    /// A device letter: bold in the device's hue, no cell behind it.
    pub fn chip_style(&self, hue: u8) -> Style {
        self.st(Tok::Id(hue)).add_modifier(Modifier::BOLD)
    }
    /// Neutral keycap for verbs (` ⏎ `, ` n `): the letter in bold accent ink, no
    /// cell behind it (the label beside it is muted, so the key leads).
    pub fn key_style(&self) -> Style {
        self.st(Tok::Accent).add_modifier(Modifier::BOLD)
    }

    pub fn status(&self, s: DeviceStatus) -> (&'static str, Style) {
        match s {
            DeviceStatus::Local => (self.g("◉", "@"), self.st(Tok::Accent)),
            // ASCII: online is the normal case and carries no mark (`o` is a key there).
            DeviceStatus::Online => (self.g("●", " "), self.st(Tok::Ok)),
            // (`o` a hollow dot, `!` down: `~` starts paths, `x` is the codex key.)
            DeviceStatus::Stale => (self.g("○", "o"), self.st(Tok::Warn)),
            DeviceStatus::Down => (self.g("✕", "!"), self.st(Tok::Err)),
            DeviceStatus::Unknown => (self.g("◌", "?"), self.st(Tok::Muted)),
        }
    }
    pub fn up(&self, d: &LabDevice) -> bool {
        matches!(
            d.status,
            DeviceStatus::Local | DeviceStatus::Online | DeviceStatus::Stale
        )
    }
    /// Short evidence for a device: rtt to the viewer, else the age of the last sighting.
    pub fn meta(&self, d: &LabDevice) -> String {
        if d.id == self.s.viewer {
            return "you".into();
        }
        if d.observed_secs == crate::ui::views::NEVER_SEEN && d.status == DeviceStatus::Down {
            return "never seen".into();
        }
        if !matches!(d.status, DeviceStatus::Online | DeviceStatus::Stale) {
            return if d.status == DeviceStatus::Unknown {
                "?".into()
            } else {
                fmt::age(d.observed_secs)
            };
        }
        let e = self.s.edges.iter().find(|e| {
            (e.from == self.s.viewer && e.to == d.id) || (e.to == self.s.viewer && e.from == d.id)
        });
        match e.and_then(|e| e.rtt_ms) {
            Some(ms) => ms_text(ms),
            None => fmt::age(d.observed_secs),
        }
    }

    pub fn sstate(&self, x: &LabSession) -> (&'static str, &'static str, Style) {
        match x.state {
            SessionState::AwaitingInput => (self.g("◆", "^"), "needs you", self.st(Tok::Warn)),
            // ASCII: live work is the normal case and carries no mark, as an online
            // device (`*` stays the agents' own bullet); what needs you is an alert, `^`.
            SessionState::Working => (self.g("●", " "), "working", self.st(Tok::Accent)),
            SessionState::Running => (self.g("●", " "), "running", self.st(Tok::Ok)),
            SessionState::Idle => (self.g("○", "-"), "idle", self.st(Tok::Muted)),
            // No current evidence (the device is unreachable or the listing is old).
            SessionState::Unknown => (self.g("◌", "?"), "unknown", self.st(Tok::Muted)),
            SessionState::Exited => (self.g("·", "."), "exited", self.ink(Tok::Faint, 1.0)),
        }
    }
    pub fn prov(&self, x: &LabSession) -> (&'static str, Tok) {
        match x.provider {
            crate::ui::views::Provider::Claude => ("claude", Tok::Claude),
            crate::ui::views::Provider::Codex => ("codex", Tok::Codex),
            crate::ui::views::Provider::Shell => ("shell", Tok::Shell),
        }
    }
    /// The session's slot key within its device (stable creation order).
    pub fn slot(&self, x: &LabSession) -> Option<char> {
        let i = self.s.sessions_on(&x.device).position(|y| y.id == x.id)?;
        SLOTS.get(i).copied()
    }
    /// Hub order: what needs you first, then live work, then by recency.
    pub fn by_attention(&self) -> Vec<&'a LabSession> {
        let mut v: Vec<&LabSession> = self.s.sessions.iter().collect();
        let rank = |x: &LabSession| match x.state {
            SessionState::AwaitingInput => 0,
            SessionState::Working => 1,
            SessionState::Running => 2,
            SessionState::Idle => 3,
            SessionState::Unknown => 4,
            SessionState::Exited => 5,
        };
        v.sort_by_key(|x| (rank(x), x.last_activity_secs));
        v
    }

    /// Small-caps section label (`NETWORK`, plain letters so it copies and reads aloud
    /// as a word); an accent bar `▍` when focused (ASCII `[NETWORK]`: `>` is the
    /// selection mark only).
    pub fn label(
        &self,
        buf: &mut Buffer,
        x: u16,
        y: u16,
        max: u16,
        text: &str,
        focused: bool,
    ) -> u16 {
        let (bar, st) = if focused {
            (
                self.g("▍", "  "),
                self.st(Tok::Accent).add_modifier(Modifier::BOLD),
            )
        } else {
            (self.g(" ", "  "), self.ink(Tok::Muted, 0.8))
        };
        if focused && !self.uni() {
            let t = format!("[{text}]");
            return self.put(buf, x, y, max, &t, st);
        }
        let mut w = self.put(buf, x, y, max, bar, self.st(Tok::Accent));
        w += self.put(buf, x + w, y, max.saturating_sub(w), text, st);
        w
    }

    /// A faint horizontal rule from `x0` to `x1` (exclusive).
    pub fn rule(&self, buf: &mut Buffer, x0: u16, x1: u16, y: u16) {
        let st = self.ink(Tok::Faint, if self.dark() { 0.8 } else { 0.7 });
        for x in x0..x1 {
            self.put(buf, x, y, 1, self.g("─", "-"), st);
        }
    }
    /// A faint vertical separator (tiles without tints). ASCII leaves the gutter blank:
    /// `|` is the selection bar's continuation and means nothing else.
    pub fn vsep(&self, buf: &mut Buffer, x: u16, y: u16, h: u16) {
        if !self.uni() {
            return;
        }
        let st = self.ink(Tok::Faint, 1.0);
        for yy in y..y + h {
            self.put(buf, x, yy, 1, self.g("│", "|"), st);
        }
    }

    /// A chip key's drawn width: ` k ` (an address `@t a` is one keycap per key).
    pub fn key_w(&self, k: &str) -> u16 {
        if k.contains('@') {
            k.split(' ').count() as u16 * 3
        } else {
            width(self.key_label(k), self.uni()) + 2
        }
    }
    /// A row of key chips (` ⏎ open  ␣ peek `); records taps. Returns end x. A key
    /// written `@t a` is an address: the device letter `t` in its hue, then `a`.
    pub fn chips(
        &self,
        buf: &mut Buffer,
        x: u16,
        y: u16,
        right: u16,
        items: &[(&str, &str, &str)],
    ) -> u16 {
        self.chips_h(buf, x, y, right, items, 1)
    }
    /// Chips whose tap targets extend `th - 1` rows upward (phone footers).
    pub fn chips_h(
        &self,
        buf: &mut Buffer,
        x: u16,
        y: u16,
        right: u16,
        items: &[(&str, &str, &str)],
        th: u16,
    ) -> u16 {
        let mut cx = x;
        for (k, label, action) in items {
            let kw = self.key_w(k);
            let lw = width(label, self.uni());
            let need = kw + if lw > 0 { lw + 1 } else { 0 };
            if cx + need > right {
                break;
            }
            let x0 = cx;
            if k.contains('@') {
                // An address: one keycap per key, a device letter in its hue.
                for t in k.split(' ') {
                    let (c, st) = match t.strip_prefix('@') {
                        Some(c) => {
                            let hue = self.s.devices.iter().find(|d| c.starts_with(d.hotkey));
                            (
                                c,
                                hue.map_or(self.key_style(), |d| {
                                    self.chip_style(self.s.hue(&d.id))
                                }),
                            )
                        }
                        None => (t, self.key_style()),
                    };
                    cx += self.keycap(buf, cx, y, 3, c, st);
                }
            } else {
                cx += self.keycap(buf, cx, y, kw, k, self.key_style());
            }
            if lw > 0 {
                cx += 1;
                cx += self.put(buf, cx, y, lw, label, self.st(Tok::Muted));
            }
            // A chip with no action only teaches its key (`j k move`): no tap target.
            if !action.is_empty() {
                let top = y.saturating_sub(th - 1);
                self.hit(Rect::new(x0, top, cx - x0, y + 1 - top), *action);
            }
            cx += 2;
        }
        cx
    }
}

pub(super) fn mix(a: Rgb, b: Rgb, t: f32) -> Rgb {
    let t = t.clamp(0.0, 1.0);
    let f = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round() as u8;
    (f(a.0, b.0), f(a.1, b.1), f(a.2, b.2))
}

/// Any ratatui color as rgb (None for the terminal default).
pub(super) fn to_rgb(c: Color) -> Option<Rgb> {
    match c {
        Color::Reset => None,
        Color::Rgb(r, g, b) => Some((r, g, b)),
        c => named_index(c).map(|i| {
            if i < 16 {
                // A calm palette rather than raw xterm primaries.
                const CALM: [Rgb; 16] = [
                    (0x2a, 0x2f, 0x37),
                    (0xe0, 0x6c, 0x75),
                    (0x98, 0xc3, 0x79),
                    (0xe5, 0xc0, 0x7b),
                    (0x61, 0xaf, 0xef),
                    (0xc6, 0x78, 0xdd),
                    (0x56, 0xb6, 0xc2),
                    (0xab, 0xb2, 0xbf),
                    (0x5c, 0x63, 0x70),
                    (0xe8, 0x83, 0x7f),
                    (0xa8, 0xd8, 0x8a),
                    (0xf0, 0xd0, 0x8a),
                    (0x7f, 0xbf, 0xff),
                    (0xd8, 0x9a, 0xf0),
                    (0x7f, 0xd8, 0xe8),
                    (0xe6, 0xe6, 0xe6),
                ];
                let _ = XTERM16;
                CALM[usize::from(i)]
            } else {
                index_rgb(i)
            }
        }),
    }
}

pub(super) fn width(s: &str, unicode: bool) -> u16 {
    if unicode {
        ratatui::text::Span::raw(s).width() as u16
    } else {
        s.chars().count() as u16
    }
}

pub(super) fn ms_text(ms: f32) -> String {
    if ms < 10.0 {
        format!("{ms:.1}ms")
    } else {
        format!("{ms:.0}ms")
    }
}

/// Truncate to `w` columns with one ellipsis (a trailing `…` in the source is dropped
/// first, so a cut never shows two).
pub(super) fn fit(s: &str, w: usize, unicode: bool) -> String {
    if s.chars().count() <= w {
        return s.to_string();
    }
    if w == 0 {
        return String::new();
    }
    // ASCII marks a cut with `..` (`~` means degraded there).
    let mark = if unicode || w < 3 { 1 } else { 2 };
    let mut head: String = s.chars().take(w - mark).collect();
    // The text's own ellipsis inside the kept part: cut there, so a cut never shows two.
    if let Some(i) = head.find('…') {
        head.truncate(i);
    }
    let head = head.trim_end_matches(['…', '.', ' ', '·', '(']);
    format!(
        "{head}{}",
        match (unicode, mark) {
            (true, _) => "…",
            (false, 1) => ".",
            _ => "..",
        }
    )
}

/// ASCII stand-ins for the fixture text's few non-ASCII characters. Each keeps one
/// meaning: `..` is a cut, `->` an arrow, `>` stays the selection mark.
pub(super) fn asciify(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            c if c.is_ascii() => out.push(c),
            '•' | '●' | '✻' | '✶' | '✳' | '∗' => out.push('*'),
            '·' => out.push('-'),
            '…' => out.push_str(".."),
            '─' | '━' | '╌' => out.push('-'),
            // Box sides are blank: `|` is the focused selection bar only.
            '│' | '┃' => out.push(' '),
            // An agent's own prompt bar is blank: `|` and `:` are cx's selection bars.
            '▌' | '▍' | '▎' => out.push(' '),
            '╭' | '╮' | '╰' | '╯' | '┌' | '┐' | '└' | '┘' => out.push('-'),
            // A screen's own text is not rewritten: what has no ASCII stand-in of the
            // same meaning is a neutral `?` (`v` and `^` are cx marks).
            '→' => out.push_str("->"),
            '←' => out.push_str("<-"),
            '⏎' => out.push_str("ret"),
            '⎿' => out.push('L'),
            '°' => out.push('C'),
            _ => out.push('?'),
        }
    }
    out
}

/// Rect helpers.
pub(super) fn inset(r: Rect, dx: u16, dy: u16) -> Rect {
    Rect::new(
        r.x + dx.min(r.width / 2),
        r.y + dy.min(r.height / 2),
        r.width.saturating_sub(2 * dx),
        r.height.saturating_sub(2 * dy),
    )
}
