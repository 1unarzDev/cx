//! Draws a cached [`GraphLayout`] into a ratatui buffer (spec 6.4, 10.3).
//!
//! Colors come from a [`GraphStyle`] provider so the theme module's semantic
//! tokens plug in without this module reading the environment. Geometry is
//! never recomputed here: state, rtt, policy, hotkeys and selection only
//! restyle cells of the cached layout.
use super::layout::{GlyphSet, GraphLayout, Mode, OutlineRow};
use super::model::*;
use super::route::{bit, DOWN, LEFT, RIGHT, UP};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::Widget;

/// Semantic roles the graph asks the theme for (a subset of spec 4.1 tokens).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Role {
    Text,
    Muted,
    Faint,
    Accent,
    /// Background of the selected outline row.
    AccentBg,
    Ok,
    Warn,
    Err,
    Info,
    Mesh,
    Lan,
    /// Easymotion-style hotkey chip on a node.
    Hotkey,
}

/// Style provider; the theme worker implements this over its `Tok` tokens.
pub trait GraphStyle {
    fn style(&self, role: Role) -> Style;
}

/// Spec 4.1 truecolor dark values; `color: false` gives the NO_COLOR substitutes.
#[derive(Clone, Copy, Debug)]
pub struct DefaultStyle {
    pub color: bool,
}
impl GraphStyle for DefaultStyle {
    fn style(&self, role: Role) -> Style {
        let s = Style::default();
        if !self.color {
            return match role {
                Role::Muted | Role::Faint => s.add_modifier(Modifier::DIM),
                Role::Accent => s.add_modifier(Modifier::BOLD),
                Role::AccentBg => s.add_modifier(Modifier::REVERSED),
                Role::Hotkey => s.add_modifier(Modifier::REVERSED | Modifier::BOLD),
                Role::Err => s.add_modifier(Modifier::BOLD),
                _ => s,
            };
        }
        let rgb = |v: u32| Color::Rgb((v >> 16) as u8, (v >> 8) as u8, v as u8);
        match role {
            Role::Text => s,
            Role::Muted => s.fg(rgb(0x8b93a1)),
            Role::Faint => s.fg(rgb(0x4a515c)),
            Role::Accent => s.fg(rgb(0x6cc4d6)),
            Role::AccentBg => s.bg(rgb(0x1d3a42)),
            Role::Ok => s.fg(rgb(0x8fcf9d)),
            Role::Warn => s.fg(rgb(0xe3c275)),
            Role::Err => s.fg(rgb(0xe8837f)),
            Role::Info => s.fg(rgb(0x8fb0e0)),
            Role::Mesh => s.fg(rgb(0xc49cf0)),
            Role::Lan => s.fg(rgb(0x7fc9b4)),
            Role::Hotkey => s
                .fg(rgb(0x171a1e))
                .bg(rgb(0xe3c275))
                .add_modifier(Modifier::BOLD),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Selection {
    pub node: Option<usize>,
    pub edge: Option<usize>,
}

/// Light/heavy box glyphs indexed by `up*27 + right*9 + down*3 + left`
/// with each arm 0 (none), 1 (light) or 2 (heavy).
const BOX: &str =
    " ╴╸╷┐┑╻┒┓╶─╾┌┬┭┎┰┱╺╼━┍┮┯┏┲┳╵┘┙│┤┥╽┧┪└┴┵├┼┽┟╁╅┕┶┷┝┾┿┢╆╈╹┚┛╿┦┩┃┨┫┖┸┹┞╀╃┠╂╉┗┺┻┡╄╇┣╊╋";

/// Arms are `[up, right, down, left]` weights (0..=2).
pub fn box_glyph(arms: [u8; 4], glyphs: GlyphSet) -> char {
    if glyphs == GlyphSet::Ascii {
        let mask = (0..4).filter(|&d| arms[d] > 0).fold(0u8, |m, d| m | bit(d));
        return match mask {
            0 => ' ',
            m if m == LEFT | RIGHT || m == LEFT || m == RIGHT => {
                if arms[1].max(arms[3]) > 1 {
                    '='
                } else {
                    '-'
                }
            }
            m if m == UP | DOWN || m == UP || m == DOWN => '|',
            _ => '+',
        };
    }
    let light_only = arms.iter().all(|&a| a <= 1);
    if light_only {
        match arms {
            [0, 1, 1, 0] => return '╭',
            [0, 0, 1, 1] => return '╮',
            [1, 1, 0, 0] => return '╰',
            [1, 0, 0, 1] => return '╯',
            _ => {}
        }
    }
    let i = arms[0] as usize * 27 + arms[1] as usize * 9 + arms[2] as usize * 3 + arms[3] as usize;
    BOX.chars().nth(i).unwrap_or('+')
}

fn mask_arms(mask: u8, weight: u8) -> [u8; 4] {
    let mut a = [0u8; 4];
    for (d, arm) in a.iter_mut().enumerate() {
        if mask & bit(d) != 0 {
            *arm = weight;
        }
    }
    a
}

pub struct GraphView<'a> {
    pub graph: &'a Graph,
    pub layout: &'a GraphLayout,
    pub style: &'a dyn GraphStyle,
    pub selection: Selection,
    pub glyphs: GlyphSet,
    /// Draw the single-letter hotkey chips.
    pub hotkeys: bool,
}

struct G {
    ascii: bool,
}
impl G {
    fn pick<'a>(&self, uni: &'a str, ascii: &'a str) -> &'a str {
        if self.ascii {
            ascii
        } else {
            uni
        }
    }
}

pub fn state_glyph(state: NodeState, glyphs: GlyphSet) -> &'static str {
    let ascii = glyphs == GlyphSet::Ascii;
    match (state, ascii) {
        (NodeState::Up, false) => "●",
        (NodeState::Stale, false) => "◐",
        (NodeState::Down, false) => "✕",
        (NodeState::Unknown, false) => "◌",
        (NodeState::Up, true) => "+",
        (NodeState::Stale, true) => "~",
        (NodeState::Down, true) => "x",
        (NodeState::Unknown, true) => ".",
    }
}
fn state_role(state: NodeState) -> Role {
    match state {
        NodeState::Up => Role::Ok,
        NodeState::Stale => Role::Warn,
        NodeState::Down => Role::Err,
        NodeState::Unknown => Role::Info,
    }
}

pub fn age(secs: u64) -> String {
    match secs {
        0..=59 => format!("{secs}s"),
        60..=3599 => format!("{}m", secs / 60),
        3600..=86399 => format!("{}h", secs / 3600),
        _ => format!("{}d", secs / 86400),
    }
}

/// Short policy label drawn beside an edge (`None` = unlabeled).
pub fn edge_label(e: &Edge, selected: bool, glyphs: GlyphSet) -> Option<String> {
    let g = G {
        ascii: glyphs == GlyphSet::Ascii,
    };
    let arrow = match e.direction {
        Direction::Both => g.pick("⇄", "<>"),
        Direction::Forward => g.pick("→", "->"),
        Direction::Reverse => g.pick("←", "<-"),
        Direction::Neither => g.pick("⊘", "x"),
    };
    match (e.policy, selected) {
        (Policy::Password, _) => Some(format!("pwd {arrow}")),
        (Policy::OneWay, _) => Some(format!("one-way {arrow}")),
        (Policy::None, _) => Some("none".into()),
        _ if e.kind == EdgeKind::LanObserved => e.evidence.note.clone(),
        (Policy::Key, true) => Some(format!("key {arrow}")),
        (Policy::Unknown, true) => Some("unchecked".into()),
        _ => None,
    }
}

fn edge_role(g: &Graph, e: &Edge) -> Role {
    let down = g
        .node_index(&e.to)
        .is_some_and(|i| g.nodes[i].state == NodeState::Down);
    match e.policy {
        Policy::None => Role::Faint,
        Policy::Password => Role::Warn,
        _ if down => Role::Faint,
        _ => match e.kind {
            EdgeKind::Mesh => Role::Mesh,
            EdgeKind::LanObserved => Role::Lan,
            _ => Role::Muted,
        },
    }
}

/// ASCII mode: terminal-safe text (middle dots become `-`, other non-ASCII `?`).
fn text_for(s: &str, glyphs: GlyphSet) -> std::borrow::Cow<'_, str> {
    if glyphs == GlyphSet::Ascii && !s.is_ascii() {
        s.chars()
            .map(|c| match c {
                '·' => '-',
                c if c.is_ascii() => c,
                _ => '?',
            })
            .collect::<String>()
            .into()
    } else {
        s.into()
    }
}

fn put(buf: &mut Buffer, x: u16, y: u16, s: &str, style: Style) {
    if let Some(c) = buf.cell_mut((x, y)) {
        c.set_symbol(s);
        c.set_style(style);
    }
}
fn put_char(buf: &mut Buffer, x: u16, y: u16, ch: char, style: Style) {
    if let Some(c) = buf.cell_mut((x, y)) {
        c.set_char(ch);
        c.set_style(style);
    }
}

impl Widget for GraphView<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        match self.layout.mode {
            Mode::Outline => self.render_outline(area, buf),
            Mode::Graph => self.render_graph(area, buf),
        }
    }
}

impl GraphView<'_> {
    fn s(&self, role: Role) -> Style {
        self.style.style(role)
    }
    fn g(&self) -> G {
        G {
            ascii: self.glyphs == GlyphSet::Ascii,
        }
    }

    fn render_graph(&self, area: Rect, buf: &mut Buffer) {
        let (w, h) = (
            self.layout.width.min(area.width),
            self.layout.height.min(area.height),
        );
        let clip = Rect::new(area.x, area.y, w, h);
        let at = |x: u16, y: u16| -> Option<(u16, u16)> {
            (x < w && y < h).then_some((area.x + x, area.y + y))
        };
        let g = self.g();
        // 1. Mesh enclosures.
        for e in &self.layout.enclosures {
            let r = e.rect;
            let st = self.s(Role::Mesh);
            let (hz, vt) = if g.ascii { ('.', ':') } else { ('┄', '┆') };
            let corners = if g.ascii {
                ['.', '.', '\'', '\'']
            } else {
                ['╭', '╮', '╰', '╯']
            };
            for x in r.x..r.right() {
                for y in [r.y, r.bottom() - 1] {
                    if let Some((ax, ay)) = at(x, y) {
                        put_char(buf, ax, ay, hz, st);
                    }
                }
            }
            for y in r.y..r.bottom() {
                for x in [r.x, r.right() - 1] {
                    if let Some((ax, ay)) = at(x, y) {
                        put_char(buf, ax, ay, vt, st);
                    }
                }
            }
            for (k, (x, y)) in [
                (r.x, r.y),
                (r.right() - 1, r.y),
                (r.x, r.bottom() - 1),
                (r.right() - 1, r.bottom() - 1),
            ]
            .into_iter()
            .enumerate()
            {
                if let Some((ax, ay)) = at(x, y) {
                    put_char(buf, ax, ay, corners[k], st);
                }
            }
            // Every piece of a split mesh carries the name so it reads as one group.
            {
                let title = format!(" {} {} ", g.pick("◆", "*"), e.mesh);
                if let Some((ax, ay)) = at(r.x + 2, r.y) {
                    let max = r.width.saturating_sub(4) as usize;
                    buf.set_stringn(
                        ax,
                        ay,
                        text_for(&title, self.glyphs),
                        max,
                        st.add_modifier(Modifier::BOLD),
                    );
                }
            }
        }
        // 2. Edges: merge arms per cell, strongest weight wins, selected edge on top.
        let cells = w as usize * h as usize;
        let mut arms: Vec<[u8; 4]> = vec![[0; 4]; cells];
        let mut owner: Vec<Option<(usize, u8)>> = vec![None; cells]; // (edge, count)
        let mut order: Vec<&super::route::Route> = self.layout.routes.iter().collect();
        order.sort_by_key(|r| Some(r.edge) == self.selection.edge);
        let weight = |e: &Edge, sel: bool| -> u8 {
            if sel || e.policy == Policy::Key {
                2
            } else {
                1
            }
        };
        for r in &order {
            let e = &self.graph.edges[r.edge];
            let wgt = weight(e, Some(r.edge) == self.selection.edge);
            for &(x, y, mask) in &r.cells {
                if x >= w || y >= h {
                    continue;
                }
                let i = y as usize * w as usize + x as usize;
                let a = mask_arms(mask, wgt);
                for d in 0..4 {
                    arms[i][d] = arms[i][d].max(a[d]);
                }
                owner[i] = Some((r.edge, owner[i].map_or(1, |o| o.1 + 1)));
            }
        }
        for r in &order {
            let e = &self.graph.edges[r.edge];
            let selected = Some(r.edge) == self.selection.edge;
            let st = if selected {
                self.s(Role::Accent).add_modifier(Modifier::BOLD)
            } else {
                self.s(edge_role(self.graph, e))
            };
            let last = r.cells.len().saturating_sub(1);
            let mid = r.cells.len() / 2;
            for (k, &(x, y, mask)) in r.cells.iter().enumerate() {
                let Some((ax, ay)) = at(x, y) else { continue };
                let i = y as usize * w as usize + x as usize;
                let a = arms[i];
                let single = owner[i].is_some_and(|o| o.1 == 1);
                let mut ch = box_glyph(a, self.glyphs);
                if single && !selected {
                    let horizontal = mask == LEFT | RIGHT;
                    let vertical = mask == UP | DOWN;
                    ch = match (e.policy, g.ascii, horizontal, vertical) {
                        (Policy::Password, false, true, _) => '┄',
                        (Policy::Password, false, _, true) => '┆',
                        (Policy::None, false, true, _) => '┈',
                        (Policy::None, false, _, true) => '┊',
                        (Policy::Password, true, _, true) => ':',
                        (Policy::None, true, true, _) | (Policy::None, true, _, true) => '.',
                        _ => ch,
                    };
                }
                let mut cst = st;
                let arrow_in = k == last && e.direction.forward();
                let arrow_out = k == 0 && e.direction.reverse();
                if arrow_in || arrow_out {
                    let d = if arrow_in {
                        (r.dst_port.2.trailing_zeros() as usize + 2) % 4
                    } else {
                        (r.src_port.2.trailing_zeros() as usize + 2) % 4
                    };
                    ch = arrow_char(d, g.ascii);
                    cst = cst.add_modifier(Modifier::BOLD);
                } else if (k == 0 && e.policy == Policy::OneWay)
                    || (k == mid && e.policy == Policy::None)
                {
                    ch = if g.ascii { 'x' } else { '⊘' };
                }
                if owner[i].map(|o| o.0) == Some(r.edge) || single {
                    put_char(buf, ax, ay, ch, cst);
                } else {
                    // Crossing: draw the merged glyph in the owner's style.
                    put_char(buf, ax, ay, box_glyph(a, self.glyphs), cst);
                }
            }
            if let Some(slot) = r.label {
                if let Some(text) = edge_label(e, selected, self.glyphs) {
                    let len = text.chars().count() as u16;
                    if len <= slot.len {
                        let x = slot.x + (slot.len - len) / 2;
                        if let Some((ax, ay)) = at(x, slot.y) {
                            let lst = if selected {
                                self.s(Role::Accent).add_modifier(Modifier::REVERSED)
                            } else {
                                st.remove_modifier(Modifier::BOLD)
                            };
                            buf.set_stringn(
                                ax,
                                ay,
                                text_for(&text, self.glyphs),
                                (w - x) as usize,
                                lst,
                            );
                        }
                    }
                }
            }
        }
        // 3. Nodes.
        for p in &self.layout.nodes {
            self.render_node(p.node, p.rect, area, clip, buf);
        }
        // Ports: merge the border with the edge arm.
        for r in &self.layout.routes {
            let e = &self.graph.edges[r.edge];
            let selected = Some(r.edge) == self.selection.edge;
            let st = if selected {
                self.s(Role::Accent).add_modifier(Modifier::BOLD)
            } else {
                self.s(edge_role(self.graph, e))
            };
            for (port, node_id) in [(r.src_port, &e.from), (r.dst_port, &e.to)] {
                let (x, y, arm) = port;
                let Some((ax, ay)) = at(x, y) else { continue };
                let Some(ni) = self.graph.node_index(node_id) else {
                    continue;
                };
                let bw = if self.selection.node == Some(ni) {
                    2
                } else {
                    1
                };
                let border = if arm & (LEFT | RIGHT) != 0 {
                    UP | DOWN
                } else {
                    LEFT | RIGHT
                };
                let mut a = mask_arms(border, bw);
                let d = arm.trailing_zeros() as usize;
                a[d] = weight(e, selected);
                let ch = box_glyph(a, self.glyphs);
                put_char(buf, ax, ay, ch, st);
            }
        }
        // 4. Overflow pills.
        for m in &self.layout.more {
            let st = self.s(Role::Faint);
            self.frame(m.rect, area, clip, buf, st, false, true);
            let text = format!("+{} more", m.hidden.len());
            if let Some((ax, ay)) = at(m.rect.x + 2, m.rect.y + 1) {
                buf.set_stringn(
                    ax,
                    ay,
                    &text,
                    m.rect.width.saturating_sub(3) as usize,
                    self.s(Role::Muted),
                );
            }
        }
        // 5. Legend.
        if let Some(row) = self.layout.legend {
            if let Some((ax, ay)) = at(2, row) {
                let items: Vec<(&str, &str, Role)> = if g.ascii {
                    vec![
                        ("=", "key", Role::Muted),
                        ("-", "pwd", Role::Warn),
                        (".", "none", Role::Faint),
                        (">", "access", Role::Muted),
                        ("*", "mesh", Role::Mesh),
                        ("~", "lan", Role::Lan),
                        ("^", "jump", Role::Muted),
                    ]
                } else {
                    vec![
                        ("━", "key", Role::Muted),
                        ("┄", "pwd", Role::Warn),
                        ("┈", "none", Role::Faint),
                        ("▶", "access", Role::Muted),
                        ("◆", "mesh", Role::Mesh),
                        ("~", "lan", Role::Lan),
                        ("↳", "jump", Role::Muted),
                    ]
                };
                let mut x = ax;
                let end = area.x + w;
                for (glyph, text, role) in items {
                    let need = (glyph.chars().count() + text.len() + 3) as u16;
                    if x + need > end {
                        break;
                    }
                    put(buf, x, ay, glyph, self.s(role));
                    buf.set_string(x + 2, ay, text, self.s(Role::Faint));
                    x += need;
                }
            }
        }
    }

    /// Card frame. `dashed` uses dashed sides, `heavy` the selected border.
    #[allow(clippy::too_many_arguments)]
    fn frame(
        &self,
        r: Rect,
        area: Rect,
        clip: Rect,
        buf: &mut Buffer,
        st: Style,
        heavy: bool,
        dashed: bool,
    ) {
        let ascii = self.glyphs == GlyphSet::Ascii;
        let [tl, tr, bl, br, hz, vt] = match (ascii, heavy, dashed) {
            (true, true, _) => ["#", "#", "#", "#", "=", "#"],
            (true, false, false) => ["+", "+", "+", "+", "-", "|"],
            (true, false, true) => [".", ".", "'", "'", ".", ":"],
            (false, true, _) => ["┏", "┓", "┗", "┛", "━", "┃"],
            (false, false, false) => ["╭", "╮", "╰", "╯", "─", "│"],
            (false, false, true) => ["╭", "╮", "╰", "╯", "┄", "┆"],
        };
        let at = |x: u16, y: u16| -> Option<(u16, u16)> {
            let (ax, ay) = (area.x + x, area.y + y);
            (ax < clip.right() && ay < clip.bottom()).then_some((ax, ay))
        };
        let (x1, y1) = (r.right() - 1, r.bottom() - 1);
        for x in r.x..=x1 {
            for (y, sym) in [(r.y, hz), (y1, hz)] {
                if let Some((ax, ay)) = at(x, y) {
                    put(buf, ax, ay, sym, st);
                }
            }
        }
        for y in r.y..=y1 {
            for x in [r.x, x1] {
                if let Some((ax, ay)) = at(x, y) {
                    put(buf, ax, ay, vt, st);
                }
            }
            if y != r.y && y != y1 {
                for x in r.x + 1..x1 {
                    if let Some((ax, ay)) = at(x, y) {
                        put(buf, ax, ay, " ", Style::default());
                    }
                }
            }
        }
        for (x, y, sym) in [(r.x, r.y, tl), (x1, r.y, tr), (r.x, y1, bl), (x1, y1, br)] {
            if let Some((ax, ay)) = at(x, y) {
                put(buf, ax, ay, sym, st);
            }
        }
    }

    fn badge_line(&self, n: &Node) -> Vec<(String, Role)> {
        let g = self.g();
        let mut parts: Vec<(String, Role)> = Vec::new();
        match n.kind {
            NodeKind::Viewer => parts.push((g.pick("⌂ you", "@ you").into(), Role::Muted)),
            NodeKind::Jump => parts.push((g.pick("↳ jump", "^ jump").into(), Role::Muted)),
            NodeKind::LanCluster | NodeKind::Neighbor => {
                if let Some(d) = &n.detail {
                    parts.push((d.clone(), Role::Muted));
                }
                return parts;
            }
            NodeKind::Device => {}
        }
        if n.mesh.is_some() {
            parts.insert(0, (g.pick("◆", "*").into(), Role::Mesh));
        }
        match n.state {
            NodeState::Down => parts.push((
                n.metrics
                    .age_secs
                    .map_or("unreachable".into(), |a| format!("seen {}", age(a))),
                Role::Muted,
            )),
            NodeState::Stale => parts.push((
                n.metrics
                    .age_secs
                    .map_or("stale".into(), |a| format!("{} ago", age(a))),
                Role::Warn,
            )),
            _ => {
                if let Some(rtt) = n.metrics.rtt_ms {
                    parts.push((format!("{rtt}ms"), Role::Muted));
                }
            }
        }
        if let Some(s) = n.metrics.sessions.filter(|&s| s > 0) {
            parts.push((format!("{s} ses"), Role::Muted));
        }
        parts
    }

    fn render_node(&self, i: usize, r: Rect, area: Rect, clip: Rect, buf: &mut Buffer) {
        let n = &self.graph.nodes[i];
        let selected = self.selection.node == Some(i);
        let g = self.g();
        let border = if selected {
            self.s(Role::Accent).add_modifier(Modifier::BOLD)
        } else {
            match n.kind {
                NodeKind::LanCluster | NodeKind::Neighbor => self.s(Role::Lan),
                NodeKind::Viewer => self.s(Role::Muted),
                _ if n.state == NodeState::Down => self.s(Role::Faint),
                _ => self.s(Role::Faint),
            }
        };
        let dashed = matches!(n.kind, NodeKind::LanCluster | NodeKind::Neighbor);
        self.frame(r, area, clip, buf, border, selected, dashed);
        let at = |x: u16, y: u16| -> Option<(u16, u16)> {
            let (ax, ay) = (area.x + x, area.y + y);
            (ax < clip.right() && ay < clip.bottom()).then_some((ax, ay))
        };
        // Content starts two cells in; the
        // last column before the right border stays blank.
        let pad = 2;
        let cx = r.x + pad;
        let room = r.width.saturating_sub(pad + 2) as usize; // dot..last text cell
                                                             // Line 1: state + name (LAN cluster: count + expand hint).
        if let Some((ax, ay)) = at(cx, r.y + 1) {
            let (dot, dot_role) = match n.kind {
                NodeKind::LanCluster => ("~", Role::Lan),
                _ => (state_glyph(n.state, self.glyphs), state_role(n.state)),
            };
            put(buf, ax, ay, dot, self.s(dot_role));
            let name_style = match n.state {
                NodeState::Down => self.s(Role::Muted),
                _ if selected || n.kind == NodeKind::Viewer => {
                    self.s(Role::Text).add_modifier(Modifier::BOLD)
                }
                _ => self.s(Role::Text),
            };
            let mut label = n.label.clone();
            if n.kind == NodeKind::LanCluster {
                let sign = match (n.expanded, g.ascii) {
                    (true, false) => "−",
                    (true, true) => "-",
                    _ => "+",
                };
                label = format!("{label} {sign}");
            }
            let name_room = room.saturating_sub(2);
            if label.chars().count() > name_room {
                let cut: String = label.chars().take(name_room.saturating_sub(1)).collect();
                label = format!("{cut}{}", g.pick("…", "~"));
            }
            buf.set_stringn(
                ax + 2,
                ay,
                text_for(&label, self.glyphs),
                name_room,
                name_style,
            );
        }
        // Line 2: badges and metrics (cards with room only).
        if r.height >= 4 {
            if let Some((ax, ay)) = at(cx, r.y + 2) {
                let mut x = ax;
                let end = ax + room as u16;
                let parts = self.badge_line(n);
                let glued = parts.first().is_some_and(|p| p.0.chars().count() == 1);
                for (k, (text, role)) in parts.into_iter().enumerate() {
                    let joiner = match k {
                        0 => "",
                        1 if glued => " ",
                        _ => g.pick(" · ", "  "),
                    };
                    let need = (joiner.chars().count() + text.chars().count()) as u16;
                    if x + need > end {
                        break;
                    }
                    buf.set_string(x, ay, joiner, self.s(Role::Faint));
                    x += joiner.chars().count() as u16;
                    buf.set_string(x, ay, text_for(&text, self.glyphs), self.s(role));
                    x += text.chars().count() as u16;
                }
            }
        }
        // Hotkey chip on the top border.
        if self.hotkeys {
            if let (Some(k), Some((ax, ay))) = (n.hotkey, at(r.x + 1, r.y)) {
                put_char(buf, ax, ay, k, self.s(Role::Hotkey));
            }
        }
    }

    fn render_outline(&self, area: Rect, buf: &mut Buffer) {
        let rows = &self.layout.outline;
        let scroll = outline_scroll(self.layout, self.selection, area.height);
        let g = self.g();
        for (line, row) in rows.iter().enumerate().skip(scroll) {
            let y = area.y + (line - scroll) as u16;
            if y >= area.bottom() {
                break;
            }
            let selected = self.selection.node == Some(row.node);
            self.render_outline_row(row, area.x, y, area.width, selected, &g, buf);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn render_outline_row(
        &self,
        row: &OutlineRow,
        x0: u16,
        y: u16,
        width: u16,
        selected: bool,
        g: &G,
        buf: &mut Buffer,
    ) {
        let n = &self.graph.nodes[row.node];
        let end = x0 + width;
        let base = if selected {
            self.s(Role::AccentBg)
        } else {
            Style::default()
        };
        if selected {
            for x in x0..end {
                put(buf, x, y, " ", base);
            }
            put(
                buf,
                x0,
                y,
                g.pick("▌", ">"),
                self.s(Role::Accent).patch(base),
            );
        }
        let mut x = x0 + 1;
        let write = |buf: &mut Buffer, x: &mut u16, s: &str, st: Style| {
            if *x >= end {
                return;
            }
            let room = (end - *x) as usize;
            buf.set_stringn(*x, y, text_for(s, self.glyphs), room, st.patch(base));
            *x += s.chars().count().min(room) as u16;
        };
        match (self.hotkeys, n.hotkey) {
            (true, Some(k)) => {
                write(buf, &mut x, &k.to_string(), self.s(Role::Hotkey));
                write(buf, &mut x, " ", Style::default());
            }
            _ => write(buf, &mut x, "  ", Style::default()),
        }
        for &cont in &row.guides {
            write(
                buf,
                &mut x,
                if cont { g.pick("│ ", "| ") } else { "  " },
                self.s(Role::Faint),
            );
        }
        if row.depth > 0 {
            let edge = row.edge.map(|k| &self.graph.edges[k]);
            let conn = if row.last {
                g.pick("└", "`")
            } else {
                g.pick("├", "|")
            };
            let (line, role) = match edge.map(|e| e.policy) {
                Some(Policy::Key) => (g.pick("━━", "=="), Role::Muted),
                Some(Policy::Password) => (g.pick("┄┄", "--"), Role::Warn),
                Some(Policy::None) => (g.pick("┈┈", ".."), Role::Faint),
                _ => (g.pick("──", "--"), Role::Muted),
            };
            let role = match edge {
                Some(e) if e.kind == EdgeKind::Mesh && e.policy == Policy::Key => Role::Mesh,
                Some(e) if e.kind == EdgeKind::LanObserved => Role::Lan,
                _ => role,
            };
            write(buf, &mut x, conn, self.s(Role::Faint));
            let child_is_to = edge.is_some_and(|e| e.to == n.id);
            let head = edge.is_some_and(|e| {
                if child_is_to {
                    e.direction.forward()
                } else {
                    e.direction.reverse()
                }
            });
            let mut s: String = line.chars().take(1).collect();
            s.push_str(if head { g.pick("▶", ">") } else { line });
            write(buf, &mut x, &s, self.s(role));
            write(buf, &mut x, " ", Style::default());
        }
        let (dot, dot_role) = match n.kind {
            NodeKind::LanCluster => ("~", Role::Lan),
            _ => (state_glyph(n.state, self.glyphs), state_role(n.state)),
        };
        write(buf, &mut x, dot, self.s(dot_role));
        write(buf, &mut x, " ", Style::default());
        let label = if n.kind == NodeKind::LanCluster {
            format!(
                "{} {}",
                n.label,
                n.detail
                    .as_deref()
                    .map(|d| format!("({d})"))
                    .unwrap_or_default()
            )
        } else {
            n.label.clone()
        };
        let name_st = if n.state == NodeState::Down {
            self.s(Role::Muted)
        } else if selected || n.kind == NodeKind::Viewer {
            self.s(Role::Text).add_modifier(Modifier::BOLD)
        } else {
            self.s(Role::Text)
        };
        write(buf, &mut x, &label, name_st);
        let mut extras: Vec<(String, Role)> = Vec::new();
        if n.kind == NodeKind::Viewer {
            extras.push((g.pick("⌂ you", "@ you").into(), Role::Muted));
        }
        if let Some(m) = &n.mesh {
            extras.push((format!("{} {m}", g.pick("◆", "*")), Role::Mesh));
        }
        if let Some(e) = row.edge.map(|k| &self.graph.edges[k]) {
            if let Some(l) =
                edge_label(e, true, self.glyphs).filter(|_| e.kind != EdgeKind::LanObserved)
            {
                extras.push((
                    l,
                    if e.policy == Policy::Password {
                        Role::Warn
                    } else {
                        Role::Muted
                    },
                ));
            }
        }
        match n.state {
            NodeState::Down => {
                if let Some(a) = n.metrics.age_secs {
                    extras.push((format!("seen {}", age(a)), Role::Muted));
                }
            }
            _ => {
                if let Some(r) = n.metrics.rtt_ms {
                    extras.push((format!("{r}ms"), Role::Muted));
                }
            }
        }
        for (text, role) in extras {
            write(buf, &mut x, "  ", Style::default());
            write(buf, &mut x, &text, self.s(role));
        }
    }
}

fn arrow_char(dir: usize, ascii: bool) -> char {
    match (dir, ascii) {
        (0, false) => '▲',
        (1, false) => '▶',
        (2, false) => '▼',
        (3, false) => '◀',
        (0, true) => '^',
        (1, true) => '>',
        (2, true) => 'v',
        _ => '<',
    }
}

/// First outline row shown so the selected row stays visible.
pub fn outline_scroll(layout: &GraphLayout, selection: Selection, height: u16) -> usize {
    let Some(k) = selection
        .node
        .and_then(|n| layout.outline.iter().position(|r| r.node == n))
    else {
        return 0;
    };
    k.saturating_sub(height.saturating_sub(1) as usize)
}
