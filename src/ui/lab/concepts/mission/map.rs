//! The network schematic, drawn as a star around the root device (this device on the
//! hub): the root is a tinted pill in the middle, the devices it reaches sit on both
//! sides of it and devices reached through them sit further out on their parent's side.
//! The two halves are balanced by height. The left half is laid out as a right half and
//! mirrored through the pill, so one router serves both. Links are box-drawing lines with
//! rounded elbows (`─│╭╮╰╯`, ASCII `-|+`) coloured by network, and their style is the
//! access policy: solid for key, dashed for password, dotted for off or down. Each link
//! carries one chip on its last straight run (policy and rtt), which is also its tap
//! target. No two links cross: they leave the pill from its top and bottom edges, nearest
//! target last, and children leave their parent on a short trunk. Nothing here dispatches.
use super::{fit, mix, ms_text, tiles, Cx, Rgb, Size};
use crate::ui::lab::{Access, DeviceStatus, EdgeKind, LabDevice, LabEdge, Tok};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
};

/// What the schematic highlights.
#[derive(Clone, Copy)]
pub(super) enum Sel<'a> {
    /// A device; `true` when the schematic has keyboard focus.
    Node(&'a str, bool),
    /// One link (the edge editor): it is drawn heavy and everything else fades.
    Edge(&'a str),
    /// Nothing selected, every link of the root lit (the device view's neighbourhood).
    Root,
}

#[derive(Clone, Copy, PartialEq, PartialOrd, Debug)]
enum Mode {
    Chip,
    Name,
    Full,
}

/// A placed device. `x` is in half coordinates (the right half's frame); `flip` nodes are
/// mirrored onto the left of the pill.
struct Node {
    id: String,
    col: usize,
    flip: bool,
    x: u16,
    y: u16,
    w: u16,
}

struct Plan {
    nodes: Vec<Node>,
    /// The root's one-row pill.
    pill: Rect,
    /// Two-row nodes (name, then evidence).
    tall: bool,
    mode: Mode,
    /// The pill's own label richness.
    pmode: Mode,
    /// The pill shows the root's own evidence.
    rich: bool,
    /// "+N" markers: (half x, y, hidden count, flip).
    more: Vec<(u16, u16, usize, bool)>,
}

impl Plan {
    /// Screen x of a half-coordinate x: the left half reflects through the pill's centre.
    fn sx(&self, flip: bool, x: u16) -> u16 {
        if flip {
            (self.pill.x + self.pill.right()).saturating_sub(1 + x)
        } else {
            x
        }
    }
    /// Screen x of a node box's left edge.
    fn left(&self, n: &Node) -> u16 {
        if n.flip {
            self.sx(true, n.x + n.w.saturating_sub(1))
        } else {
            n.x
        }
    }
}

/// Device ids in selection order: column by column, top to bottom, the viewer last.
pub(super) fn order(cx: &Cx, band: Rect) -> Vec<String> {
    let root = cx.s.viewer.as_str();
    let mut nodes = plan(cx, band, root, 3, None).nodes;
    nodes.sort_by_key(|n| (n.col, n.y, !n.flip));
    let mut v: Vec<String> = nodes.into_iter().map(|n| n.id).collect();
    v.push(root.to_string());
    v
}

/// Rows the hub schematic wants: room for its tallest column with air between nodes.
pub(super) fn want_h(cx: &Cx) -> u16 {
    let cols = columns(cx, &cx.s.viewer, 3);
    let most = halves(cx, &cols, None)
        .iter()
        .flat_map(|h| h.iter().map(|c| c.len()))
        .max()
        .unwrap_or(1) as u16;
    most * 4 + 1
}

pub(super) fn mesh_tok(cx: &Cx, id: &str) -> Tok {
    match cx.s.meshes.iter().position(|m| m.id == id) {
        Some(0) | None => Tok::Mesh,
        Some(_) => Tok::Id(6),
    }
}

/// Jump edges are drawn through the hop they use, not as their own line.
fn routed(e: &LabEdge) -> bool {
    !matches!(e.kind, EdgeKind::Jump(_))
}

fn linked(cx: &Cx, a: &str, b: &str) -> bool {
    cx.s.edges
        .iter()
        .any(|e| routed(e) && ((e.from == a && e.to == b) || (e.from == b && e.to == a)))
}

/// A piece of a node's evidence row; `true` when a space separates it from the previous.
type Seg = (String, Style, bool);

/// A node's evidence row: cpu trend, temperature and session pips when it is up, the age
/// of the last sighting when it is not. The rtt rides on the link chip instead.
fn evidence(cx: &Cx, d: &LabDevice) -> Vec<Seg> {
    let mut v: Vec<Seg> = Vec::new();
    if !cx.up(d) {
        let t = if d.status == DeviceStatus::Unknown {
            "not probed".to_string()
        } else {
            format!("seen {}", cx.meta(d))
        };
        v.push((t, cx.ink(Tok::Err, 0.8), false));
        return v;
    }
    if let Some(m) = &d.metrics {
        if cx.size() >= Size::M {
            let h: Vec<Option<f32>> = m.cpu_hist.iter().map(|v| Some(*v)).collect();
            v.push((tiles::spark_str(cx, &h, 6), cx.heat(m.cpu), false));
        } else {
            v.push((format!("{:.0}%", m.cpu * 100.0), cx.heat(m.cpu), false));
        }
        if let Some(t) = m.temp_c {
            let hv = ((t - 35.0) / 50.0).clamp(0.0, 1.0);
            v.push((format!("{t:.0}{}", cx.g("°", "C")), cx.heat(hv), true));
        }
    }
    // Pips sit together, a space after the rest.
    for (i, (t, st)) in tiles::pip_segs(cx, d, 4).into_iter().enumerate() {
        v.push((t, st, i == 0 && !v.is_empty()));
    }
    v
}

fn ev_w(cx: &Cx, segs: &[Seg]) -> u16 {
    segs.iter()
        .map(|(t, _, gap)| super::width(t, cx.uni()) + u16::from(*gap))
        .sum()
}

fn draw_ev(cx: &Cx, buf: &mut Buffer, mut x: u16, y: u16, segs: &[Seg]) -> u16 {
    let x0 = x;
    for (t, st, gap) in segs {
        x += u16::from(*gap);
        x += cx.put(buf, x, y, super::width(t, cx.uni()), t, *st);
    }
    x - x0
}

fn label_w(cx: &Cx, id: &str, mode: Mode, tall: bool, nmax: u16) -> u16 {
    let Some(d) = cx.s.device(id) else { return 0 };
    let name = (d.name.chars().count() as u16).min(nmax) + badges_w(d, mode);
    match mode {
        Mode::Chip => 5,
        Mode::Name => 6 + name,
        Mode::Full if tall => 6 + name.max(ev_w(cx, &evidence(cx, d))),
        Mode::Full => 6 + name,
    }
}

fn badges_w(d: &LabDevice, mode: Mode) -> u16 {
    if mode != Mode::Full || d.meshes.is_empty() {
        0
    } else {
        1 + d.meshes.len() as u16
    }
}

/// The root pill carries its own evidence inline when there is room for it.
fn pill_ev(cx: &Cx, root: &str, mode: Mode, rich: bool) -> Vec<Seg> {
    match cx.s.device(root) {
        Some(d) if rich && mode == Mode::Full && cx.size() >= Size::M && cx.has_surface() => {
            evidence(cx, d)
        }
        _ => Vec::new(),
    }
}

fn pill_w(cx: &Cx, root: &str, mode: Mode, rich: bool) -> u16 {
    let name =
        cx.s.device(root)
            .map_or(4, |d| d.name.chars().count() as u16);
    let ev = ev_w(cx, &pill_ev(cx, root, mode, rich));
    match mode {
        Mode::Chip => 5,
        _ => 8 + name + if ev > 0 { ev + 2 } else { 0 },
    }
}

/// Columns by hop count from `root` (at most `depth`), ordered to avoid crossings.
fn columns(cx: &Cx, root: &str, depth: usize) -> Vec<Vec<String>> {
    let s = cx.s;
    let mut cols: Vec<Vec<String>> = Vec::new();
    let mut seen: Vec<&str> = vec![root];
    let mut frontier: Vec<&str> = vec![root];
    while !frontier.is_empty() && cols.len() < depth {
        let next: Vec<&str> = s
            .devices
            .iter()
            .map(|d| d.id.as_str())
            .filter(|id| !seen.contains(id) && frontier.iter().any(|p| linked(cx, p, id)))
            .collect();
        if next.is_empty() {
            break;
        }
        seen.extend(next.iter().copied());
        cols.push(next.iter().map(|x| x.to_string()).collect());
        frontier = next;
    }
    // On the hub, anything unreached (no routed link at all) joins the last column.
    if depth >= 3 {
        let rest: Vec<String> = s
            .devices
            .iter()
            .map(|d| d.id.clone())
            .filter(|id| !seen.contains(&id.as_str()))
            .collect();
        if !rest.is_empty() {
            let own = cols.len() < 3 && cols.last().is_none_or(|c| c.len() > rest.len() * 2);
            match cols.last_mut() {
                Some(last) if !own => last.extend(rest),
                _ => cols.push(rest),
            }
        }
    }
    // First column grouped by mesh; later columns follow their parents.
    let group = |d: &str| {
        s.meshes
            .iter()
            .position(|m| m.members.iter().any(|x| x == d))
            .unwrap_or(s.meshes.len())
    };
    let idx = |d: &str| s.devices.iter().position(|x| x.id == d).unwrap_or(0);
    if let Some(c) = cols.first_mut() {
        c.sort_by_key(|d| (group(d), idx(d)));
    }
    for k in 1..cols.len() {
        let prev = cols[k - 1].clone();
        let parent = |d: &str| {
            prev.iter()
                .position(|p| linked(cx, p, d))
                .unwrap_or(prev.len())
        };
        cols[k].sort_by_key(|d| (parent(d), idx(d)));
    }
    cols
}

/// Split the columns into a right half and a left half. Every device stays on its
/// parent's side; whole subtrees go to whichever half stays shorter (ties: right).
/// `keep` puts two devices on one side (the peer link being edited).
fn halves(cx: &Cx, cols: &[Vec<String>], keep: Option<(&str, &str)>) -> [Vec<Vec<String>>; 2] {
    let n = cols.len();
    // The subtree root of every device: its first-column ancestor (or itself).
    let mut root_of: Vec<Vec<String>> = Vec::new();
    for (k, c) in cols.iter().enumerate() {
        let r: Vec<String> = c
            .iter()
            .map(|d| {
                k.checked_sub(1)
                    .and_then(|p| {
                        cols[p]
                            .iter()
                            .position(|q| linked(cx, q, d))
                            .map(|i| root_of[p][i].clone())
                    })
                    .unwrap_or_else(|| d.clone())
            })
            .collect();
        root_of.push(r);
    }
    let mut roots: Vec<(String, Vec<u16>)> = Vec::new();
    for k in 0..n {
        for r in &root_of[k] {
            match roots.iter_mut().find(|(id, _)| id == r) {
                Some((_, cnt)) => cnt[k] += 1,
                None => {
                    let mut cnt = vec![0u16; n];
                    cnt[k] = 1;
                    roots.push((r.clone(), cnt));
                }
            }
        }
    }
    roots.sort_by_key(|(_, c)| std::cmp::Reverse(c.iter().sum::<u16>()));
    let mut load = [vec![0u16; n], vec![0u16; n]];
    let mut side: Vec<(String, usize)> = Vec::new();
    for (id, cnt) in &roots {
        let tall = |s: usize| (0..n).map(|k| load[s][k] + cnt[k]).max().unwrap_or(0);
        let total = |s: usize| load[s].iter().sum::<u16>();
        let pick = if (tall(1), total(1)) < (tall(0), total(0)) {
            1
        } else {
            0
        };
        for k in 0..n {
            load[pick][k] += cnt[k];
        }
        side.push((id.clone(), pick));
    }
    let side_of = |id: &str| -> usize {
        let r = (0..n).find_map(|k| cols[k].iter().position(|d| d == id).map(|i| &root_of[k][i]));
        r.and_then(|r| side.iter().find(|(x, _)| x == r).map(|(_, s)| *s))
            .unwrap_or(0)
    };
    let forced = keep.map(|(a, b)| (b.to_string(), side_of(a)));
    let mut out: [Vec<Vec<String>>; 2] = [vec![Vec::new(); n], vec![Vec::new(); n]];
    for k in 0..n {
        for (i, d) in cols[k].iter().enumerate() {
            let mut s = side_of(d);
            if let Some((b, fs)) = &forced {
                if &root_of[k][i] == b || d == b {
                    s = *fs;
                }
            }
            out[s][k].push(d.clone());
        }
    }
    out
}

fn plan(cx: &Cx, band: Rect, root: &str, depth: usize, keep: Option<(&str, &str)>) -> Plan {
    let cols = columns(cx, root, depth);
    let hv = halves(cx, &cols, keep);
    let h = band.height;
    let most = hv
        .iter()
        .flat_map(|c| c.iter().map(|c| c.len()))
        .max()
        .unwrap_or(0) as u16;
    let tall = cx.size() >= Size::S && most > 0 && h + 1 >= most * 3;
    let cap = |n: usize| -> usize {
        let fit = if tall {
            usize::from((h + 1) / 3)
        } else {
            usize::from((h + 1) / 2)
        };
        n.min(fit.max(1))
    };
    // Richest label mode whose columns fit with room for a chip between them.
    let mut mode = Mode::Chip;
    let mut nmax = 8;
    let mut widths: [Vec<u16>; 2] = [Vec::new(), Vec::new()];
    let mut rich = false;
    let mut pmode = Mode::Chip;
    // (node labels, longest name, pill evidence, pill labels, least gap)
    let tries = [
        (Mode::Full, 22, true, Mode::Full, 9),
        (Mode::Full, 16, true, Mode::Full, 9),
        (Mode::Full, 22, false, Mode::Full, 9),
        (Mode::Full, 16, false, Mode::Full, 9),
        (Mode::Name, 22, false, Mode::Name, 9),
        (Mode::Name, 16, false, Mode::Name, 9),
        (Mode::Name, 12, false, Mode::Name, 8),
        (Mode::Name, 10, false, Mode::Name, 7),
        // Names before keycaps: the pill gives up its name first (it is "you").
        (Mode::Name, 10, false, Mode::Chip, 6),
        (Mode::Name, 8, false, Mode::Chip, 5),
        (Mode::Chip, 8, false, Mode::Chip, 5),
    ];
    let mut gaps = 0u16;
    for (m, nm, pr, pm, least) in tries {
        mode = m;
        nmax = nm;
        rich = pr;
        pmode = pm;
        for s in 0..2 {
            widths[s] = hv[s]
                .iter()
                .map(|c| {
                    c.iter()
                        .take(cap(c.len()))
                        .map(|id| label_w(cx, id, m, tall, nmax))
                        .max()
                        .unwrap_or(0)
                })
                .collect();
        }
        gaps = widths.iter().flatten().filter(|w| **w > 0).count() as u16;
        let need =
            2 + pill_w(cx, root, pm, pr) + widths.iter().flatten().sum::<u16>() + least * gaps;
        if need <= band.width {
            break;
        }
    }
    let pw = pill_w(cx, root, pmode, rich).min(band.width);
    let fixed = pw + widths.iter().flatten().sum::<u16>();
    let free = band.width.saturating_sub(fixed);
    // Gaps grow with the width (capped so wide screens get air, not stretched lines);
    // what is left centres the drawing.
    let gap = if gaps == 0 {
        0
    } else {
        (free.saturating_sub(4) / gaps).clamp(4, 24)
    };
    let left_ext: u16 = widths[1].iter().filter(|w| **w > 0).map(|w| w + gap).sum();
    let total = fixed + gap * gaps;
    let margin = band.width.saturating_sub(total) / 2;
    let cy = band.y + h.saturating_sub(1) / 2;
    let pill = Rect::new(band.x + margin + left_ext, cy, pw, 1);

    let mut nodes = Vec::new();
    let mut more = Vec::new();
    let rows = if tall { 2 } else { 1 };
    for (s, half) in hv.iter().enumerate() {
        let mut x = pill.right() + gap;
        for (k, c) in half.iter().enumerate() {
            if c.is_empty() {
                continue;
            }
            let shown = if cap(c.len()) < c.len() {
                cap(c.len()).saturating_sub(1)
            } else {
                c.len()
            };
            let slots = shown + usize::from(shown < c.len());
            let n = slots.max(1) as u16;
            let min_sp = if tall { 3 } else { 2 };
            let sp = if n == 1 {
                0
            } else {
                (h.saturating_sub(rows + 1) / (n - 1)).clamp(
                    min_sp,
                    if !tall {
                        3
                    } else if cx.size() == Size::L {
                        9
                    } else {
                        7
                    },
                )
            };
            let span = (n - 1) * sp + rows;
            // Centre the column on the pill row (odd counts put a node on it).
            let y0 = cy
                .saturating_sub((n - 1) * sp / 2)
                .clamp(band.y, band.bottom().saturating_sub(span).max(band.y));
            for (i, id) in c.iter().take(shown).enumerate() {
                nodes.push(Node {
                    id: id.clone(),
                    col: k + 1,
                    flip: s == 1,
                    x,
                    y: y0 + i as u16 * sp,
                    w: label_w(cx, id, mode, tall, nmax),
                });
            }
            if shown < c.len() {
                more.push((x + 2, y0 + shown as u16 * sp, c.len() - shown, s == 1));
            }
            x += widths[s][k] + gap;
        }
    }
    Plan {
        nodes,
        pill,
        tall,
        mode,
        pmode,
        rich,
        more,
    }
}

const N: u8 = 1;
const E: u8 = 2;
const S: u8 = 4;
const W: u8 = 8;

/// Line styles: the access policy, readable without colour.
const SOLID: u8 = 0;
const DASH: u8 = 1;
const DOT: u8 = 2;
const HEAVY: u8 = 3;

/// Line canvas: per cell the directions a line leaves it, its colour and emphasis.
struct Grid {
    r: Rect,
    bits: Vec<u8>,
    lvl: Vec<u8>,
    tok: Vec<Tok>,
    style: Vec<u8>,
    pulse: Vec<bool>,
}

impl Grid {
    fn new(r: Rect) -> Self {
        let n = usize::from(r.width) * usize::from(r.height);
        Self {
            r,
            bits: vec![0; n],
            lvl: vec![0; n],
            tok: vec![Tok::Faint; n],
            style: vec![SOLID; n],
            pulse: vec![false; n],
        }
    }
    fn at(&self, x: u16, y: u16) -> Option<usize> {
        (x >= self.r.x && y >= self.r.y && x < self.r.right() && y < self.r.bottom()).then(|| {
            usize::from(y - self.r.y) * usize::from(self.r.width) + usize::from(x - self.r.x)
        })
    }
    fn mark(&mut self, x: u16, y: u16, b: u8, lvl: u8, tok: Tok, style: u8) {
        if let Some(i) = self.at(x, y) {
            // A shared trunk takes the brightest, most solid of its links.
            let first = self.bits[i] == 0;
            self.bits[i] |= b;
            if first || lvl > self.lvl[i] || (lvl == self.lvl[i] && style < self.style[i]) {
                self.lvl[i] = lvl;
                self.tok[i] = tok;
                self.style[i] = style;
            }
        }
    }
    /// A polyline through axis-aligned corner points; returns its cells in order.
    fn path(&mut self, pts: &[(u16, u16)], lvl: u8, tok: Tok, style: u8) -> Vec<(u16, u16)> {
        let mut cells: Vec<(u16, u16)> = Vec::new();
        for w in pts.windows(2) {
            let ((x0, y0), (x1, y1)) = (w[0], w[1]);
            let (fwd, back) = if y0 == y1 {
                if x1 >= x0 {
                    (E, W)
                } else {
                    (W, E)
                }
            } else if y1 > y0 {
                (S, N)
            } else {
                (N, S)
            };
            let n = x0.abs_diff(x1).max(y0.abs_diff(y1));
            for k in 0..=n {
                let (x, y) = if y0 == y1 {
                    (if x1 >= x0 { x0 + k } else { x0 - k }, y0)
                } else {
                    (x0, if y1 >= y0 { y0 + k } else { y0 - k })
                };
                let mut b = 0;
                if k < n {
                    b |= fwd;
                }
                if k > 0 {
                    b |= back;
                }
                self.mark(x, y, b, lvl, tok, style);
                if cells.last() != Some(&(x, y)) {
                    cells.push((x, y));
                }
            }
        }
        cells
    }
    fn flush(&self, cx: &Cx, buf: &mut Buffer) {
        for (i, &b) in self.bits.iter().enumerate() {
            if b == 0 {
                continue;
            }
            let x = self.r.x + (i % usize::from(self.r.width)) as u16;
            let y = self.r.y + (i / usize::from(self.r.width)) as u16;
            let horiz = b & (N | S) == 0;
            let vert = b & (E | W) == 0;
            let style = if self.pulse[i] { HEAVY } else { self.style[i] };
            let g = if !cx.uni() {
                match (horiz, vert, style) {
                    (true, _, DASH | DOT) => ".",
                    (_, true, DASH | DOT) => ":",
                    (true, _, HEAVY) => "=",
                    (true, _, _) => "-",
                    (_, true, _) => "|",
                    _ => "+",
                }
            } else {
                let heavy = style == HEAVY;
                match b {
                    _ if horiz => match style {
                        DASH => "┄",
                        DOT => "┈",
                        HEAVY => "━",
                        _ => "─",
                    },
                    _ if vert => match style {
                        DASH => "┆",
                        DOT => "┊",
                        HEAVY => "┃",
                        _ => "│",
                    },
                    x if x == E | S => {
                        if heavy {
                            "┏"
                        } else {
                            "╭"
                        }
                    }
                    x if x == W | S => {
                        if heavy {
                            "┓"
                        } else {
                            "╮"
                        }
                    }
                    x if x == N | E => {
                        if heavy {
                            "┗"
                        } else {
                            "╰"
                        }
                    }
                    x if x == N | W => {
                        if heavy {
                            "┛"
                        } else {
                            "╯"
                        }
                    }
                    x if x == N | E | S => "├",
                    x if x == N | W | S => "┤",
                    x if x == E | W | S => "┬",
                    x if x == N | E | W => "┴",
                    _ => "┼",
                }
            };
            let a = match self.lvl[i] {
                0 => {
                    if cx.dark() {
                        0.32
                    } else {
                        0.45
                    }
                }
                1 => {
                    if cx.dark() {
                        0.85
                    } else {
                        0.95
                    }
                }
                _ => 1.0,
            };
            let mut st = if self.pulse[i] {
                cx.ink(Tok::Fg, 1.0)
            } else {
                cx.ink(self.tok[i], a)
            };
            if self.lvl[i] >= 2 || self.pulse[i] {
                st = st.add_modifier(Modifier::BOLD);
            }
            let c = &mut buf[(x, y)];
            c.set_symbol(g);
            c.set_style(st);
        }
    }
}

fn edge_tok(cx: &Cx, e: &LabEdge) -> Tok {
    if !e.up {
        return Tok::Err;
    }
    match &e.kind {
        EdgeKind::Mesh(m) => mesh_tok(cx, m),
        EdgeKind::Lan => Tok::Lan,
        EdgeKind::Direct | EdgeKind::Jump(_) => Tok::Info,
    }
}

/// Line style of a link: key access anywhere is solid, password only is dashed, nothing
/// allowed (or the link is down) is dotted.
fn line_style(e: &LabEdge) -> u8 {
    let any = |a: Access| e.forward == a || e.reverse == a;
    if !e.up || (e.forward == Access::Denied && e.reverse == Access::Denied) {
        DOT
    } else if any(Access::Key) {
        SOLID
    } else {
        DASH
    }
}

/// Policy shorthand, with the arrow pointing the way access is allowed on screen.
/// `short` keeps only the glyph.
pub(super) fn policy(cx: &Cx, e: &LabEdge, from_left: bool, short: bool) -> String {
    if !e.up {
        return if short {
            cx.g("✕", "x").into()
        } else {
            format!("{} down", cx.g("✕", "x"))
        };
    }
    let (f, r) = (e.forward, e.reverse);
    let (to_right, to_left) = if from_left { (f, r) } else { (r, f) };
    let word = |a: Access| match a {
        Access::Key => "key",
        Access::Password => "pass",
        Access::Denied => "off",
    };
    let (g, w) = match (to_right, to_left) {
        (Access::Denied, Access::Denied) => (cx.g("⊘", "x"), "off".to_string()),
        (a, Access::Denied) => (cx.g("→", "->"), word(a).into()),
        (Access::Denied, a) => (cx.g("←", "<-"), word(a).into()),
        (a, b) if a == b => (cx.g("⇄", "<>"), word(a).into()),
        (a, b) => (cx.g("⇄", "<>"), format!("{}/{}", word(a), word(b))),
    };
    if short {
        g.into()
    } else {
        format!("{g} {w}")
    }
}

/// Chip text for a link run of `len` cells: policy and rtt, then policy, then the glyph.
fn chip_text(cx: &Cx, e: &LabEdge, from_left: bool, len: u16) -> Option<String> {
    let long = policy(cx, e, from_left, false);
    let mut tries = Vec::new();
    if let Some(ms) = e.rtt_ms.filter(|_| e.up && cx.size() >= Size::S) {
        let ms = ms_text(ms);
        tries.push(format!("{long} {ms}"));
        // Two-way and equal: the word alone says it (the line style repeats it).
        if e.forward == e.reverse {
            let word = long.split(' ').nth(1).unwrap_or("");
            tries.push(format!("{word} {ms}"));
        }
    }
    tries.push(long);
    tries.push(policy(cx, e, from_left, true));
    tries
        .into_iter()
        .find(|t| super::width(t, cx.uni()) + 4 <= len)
}

/// Draw the network seen from `root` (columns up to `depth` hops) into `band`.
pub(super) fn draw(cx: &Cx, buf: &mut Buffer, band: Rect, root: &str, depth: usize, sel: Sel) {
    if band.width < 16 || band.height < 3 {
        return;
    }
    let s = cx.s;
    let keep = match sel {
        Sel::Edge(id) => s
            .edge(id)
            .filter(|e| e.from != root && e.to != root)
            .map(|e| (e.from.as_str(), e.to.as_str())),
        _ => None,
    };
    let p = plan(cx, band, root, depth, keep);
    let find = |id: &str| p.nodes.iter().find(|n| n.id == id);
    let rows = if p.tall && p.mode == Mode::Full { 2 } else { 1 };
    let col_of = |id: &str| {
        if id == root {
            Some(0)
        } else {
            find(id).map(|n| n.col)
        }
    };
    let flip_of = |id: &str| find(id).map(|n| n.flip);
    // The selected device's route back to the root lights up with its own links.
    let mut route: Vec<(&str, &str)> = Vec::new();
    if let Sel::Node(id, _) = sel {
        let mut cur = id;
        while let Some(c) = col_of(cur).filter(|c| *c > 0) {
            let parent = if c == 1 {
                Some(root)
            } else {
                p.nodes
                    .iter()
                    .find(|n| n.col == c - 1 && linked(cx, &n.id, cur))
                    .map(|n| n.id.as_str())
            };
            let Some(up) = parent else { break };
            route.push((cur, up));
            cur = up;
        }
    }
    let lit = |e: &LabEdge| match sel {
        Sel::Node(id, _) => {
            e.from == id
                || e.to == id
                || route
                    .iter()
                    .any(|&(a, b)| (e.from == a && e.to == b) || (e.from == b && e.to == a))
        }
        Sel::Edge(id) => e.id == id,
        Sel::Root => e.from == root || e.to == root,
    };
    let edge_mode = matches!(sel, Sel::Edge(_));

    // Mesh hulls: a faint tint behind neighbours in one column that share a mesh.
    if cx.has_surface() && p.mode > Mode::Chip {
        for col in 1..=3 {
            for flip in [false, true] {
                let ns: Vec<&Node> = p
                    .nodes
                    .iter()
                    .filter(|n| n.col == col && n.flip == flip)
                    .collect();
                let mut i = 0;
                while i < ns.len() {
                    let m = s.device(&ns[i].id).and_then(|d| d.meshes.first().cloned());
                    let mut j = i + 1;
                    while j < ns.len()
                        && m.is_some()
                        && s.device(&ns[j].id).and_then(|d| d.meshes.first()) == m.as_ref()
                    {
                        j += 1;
                    }
                    if let (Some(m), true) = (m, j - i >= 2) {
                        let x0 = ns[i..j].iter().map(|n| p.left(n)).min().unwrap_or(1) - 1;
                        let x1 = ns[i..j].iter().map(|n| p.left(n) + n.w).max().unwrap_or(0) + 1;
                        let (y0, y1) = (ns[i].y, ns[j - 1].y + rows);
                        let a = if cx.dark() { 0.06 } else { 0.07 };
                        cx.fill(
                            buf,
                            Rect::new(x0, y0, x1 - x0, y1 - y0),
                            cx.wash(mesh_tok(cx, &m), a),
                        );
                    }
                    i = j;
                }
            }
        }
    }

    let mut grid = Grid::new(band);
    let busy = |e: &LabEdge| {
        s.transfers
            .iter()
            .any(|t| (t.from == e.from && t.to == e.to) || (t.from == e.to && t.to == e.from))
    };
    // Links as (edge, root-side end, outer end, half), in half coordinates.
    let mut links: Vec<(&LabEdge, &str, &str, bool)> = Vec::new();
    for e in s.edges.iter().filter(|e| routed(e)) {
        let (Some(fc), Some(tc)) = (col_of(&e.from), col_of(&e.to)) else {
            continue;
        };
        // Peers in one column are drawn only while that link is being edited.
        if fc == tc && !(edge_mode && lit(e)) {
            continue;
        }
        if fc.abs_diff(tc) > 1 {
            continue;
        }
        let (l, r) = if fc <= tc {
            (e.from.as_str(), e.to.as_str())
        } else {
            (e.to.as_str(), e.from.as_str())
        };
        let Some(flip) = flip_of(r) else { continue };
        if flip_of(l).is_some_and(|f| f != flip) {
            continue;
        }
        links.push((e, l, r, flip));
    }
    let row_of = |id: &str| find(id).map_or(p.pill.y, |n| n.y);
    let right_of = |id: &str| find(id).map_or(p.pill.right(), |n| n.x + n.w + 1);
    // Trunk lanes in each gap start right of the widest label of the column before it.
    let col_right = |c: usize, flip: bool| {
        p.nodes
            .iter()
            .filter(|n| n.col == c && n.flip == flip)
            .map(|n| n.x + n.w)
            .max()
            .unwrap_or(p.pill.right())
    };
    #[allow(clippy::type_complexity)]
    let mut chips: Vec<(
        u16,
        u16,
        u16,
        &LabEdge,
        Tok,
        u8,
        bool,
        Option<(Vec<(u16, u16)>, bool)>,
    )> = Vec::new();
    for (i, &(e, l, r, flip)) in links.iter().enumerate() {
        let (ly, ry) = (row_of(l), row_of(r));
        let Some(rn) = find(r) else { continue };
        let lvl = if lit(e) {
            2
        } else if edge_mode {
            0
        } else {
            1
        };
        let tok = edge_tok(cx, e);
        let style = if edge_mode && lvl >= 2 {
            HEAVY
        } else {
            line_style(e)
        };
        let end = rn.x.saturating_sub(2);
        let same = col_of(l) == col_of(r);
        // Siblings sharing this link's source and half, farthest first: the farthest
        // target bends first so nested elbows never cross.
        let rank = |pred: &dyn Fn(u16) -> bool| -> u16 {
            let mine = ry.abs_diff(ly);
            links
                .iter()
                .enumerate()
                .filter(|(j, (_, l2, r2, f2))| {
                    *l2 == l && *f2 == flip && *j != i && pred(row_of(r2)) && {
                        let d = row_of(r2).abs_diff(ly);
                        d > mine || (d == mine && *j < i)
                    }
                })
                .count() as u16
        };
        let mut stub: Option<(u16, u16, u8)> = None;
        let (pts, run) = if same {
            // A bracket outside the column, between the two labels.
            let xb = col_right(rn.col, flip) + 3;
            let lx = right_of(l);
            let rx = right_of(r);
            (
                vec![(lx, ly), (xb, ly), (xb, ry), (rx, ry)],
                (rx.min(lx), xb, ry),
            )
        } else if l == root {
            let pill = p.pill;
            if ry == ly {
                (vec![(pill.right(), ly), (end, ry)], (pill.right(), end, ry))
            } else {
                // Ports along the pill's top / bottom edge, nearest target outermost.
                let up = ry < ly;
                let k = rank(&|y| (y < ly) == up && y != ly);
                let n_side = links
                    .iter()
                    .filter(|(_, l2, r2, f2)| {
                        *l2 == l && *f2 == flip && (row_of(r2) < ly) == up && row_of(r2) != ly
                    })
                    .count() as u16;
                let room = pill.width.saturating_sub(4) / 4;
                let slot = (n_side.saturating_sub(1) - k.min(n_side.saturating_sub(1))).min(room);
                let px = pill.right().saturating_sub(3 + 2 * slot);
                let py = if up { ly - 1 } else { ly + 1 };
                let mut v = vec![(px, py), (px, ry), (end, ry)];
                if py == ry {
                    v.remove(0);
                }
                // The first cell touches the pill: give it a stub toward it.
                stub = Some((px, py, if up { S } else { N }));
                (v, (px, end, ry))
            }
        } else {
            let lc = col_of(l).unwrap_or(0);
            let base = col_right(lc, flip) + 3;
            let k = rank(&|_| true);
            let lane = links
                .iter()
                .filter(|(_, l2, _, f2)| *f2 == flip && col_of(l2) == Some(lc) && row_of(l2) < ly)
                .count() as u16
                + k;
            let xb = (base + 2 * lane).min(end.saturating_sub(5));
            let sx = right_of(l);
            if ry == ly {
                (vec![(sx, ly), (end, ry)], (xb, end, ry))
            } else {
                (vec![(sx, ly), (xb, ly), (xb, ry), (end, ry)], (xb, end, ry))
            }
        };
        let pts: Vec<(u16, u16)> = pts.iter().map(|&(x, y)| (p.sx(flip, x), y)).collect();
        if let Some((x, y, b)) = stub {
            grid.mark(p.sx(flip, x), y, b, lvl, tok, style);
        }
        let cells = grid.path(&pts, lvl, tok, style);
        let packet = (busy(e) && e.up).then(|| (cells, s.transfers.iter().any(|t| t.to == l)));
        let (rx0, rx1, y) = run;
        let (a, b) = (p.sx(flip, rx0), p.sx(flip, rx1));
        // The root-side end is on the screen's left unless the half is mirrored.
        let from_left = (l == e.from) != flip;
        chips.push((a.min(b), a.max(b), y, e, tok, lvl, from_left, packet));
    }

    // One chip per link, centred on its last straight run.
    let mut placed: Vec<(u16, u16, String, Style, &str)> = Vec::new();
    for (x0, x1, y, e, tok, lvl, from_left, packet) in chips {
        let len = x1.saturating_sub(x0);
        let text = chip_text(cx, e, from_left, len);
        let w = text.as_ref().map_or(0, |t| super::width(t, cx.uni()) + 2);
        let x = x0 + (len.saturating_sub(w) + 1) / 2;
        if let Some((cells, toward_root)) = packet {
            // An active transfer: one bright packet on the unbroken line, clear of the
            // chip, heading for the receiving end (a live build advances it by frame).
            // Straight horizontal cells only (no corners), off the chip.
            let free = |c: &(u16, u16)| {
                let straight = grid.at(c.0, c.1).is_some_and(|i| grid.bits[i] == E | W);
                straight && (c.1 != y || c.0 < x || c.0 >= x + w)
            };
            let starts: Vec<usize> = (0..cells.len().saturating_sub(2))
                .filter(|&j| cells[j..j + 3].iter().all(free))
                .collect();
            if !starts.is_empty() {
                let t = (cx.c.frame as usize * 3) % starts.len();
                let j = starts[if toward_root { starts.len() - 1 - t } else { t }];
                for &(cx_, cy_) in &cells[j..j + 3] {
                    if let Some(ix) = grid.at(cx_, cy_) {
                        grid.pulse[ix] = true;
                    }
                }
            }
        }
        let Some(text) = text else { continue };
        let a = match lvl {
            0 => 0.4,
            1 => 0.9,
            _ => 1.0,
        };
        let ink = if e.up {
            cx.ink(tok, a)
        } else {
            cx.ink(Tok::Err, a)
        };
        let st = if cx.has_surface() {
            cx.rgb_bg(mix(
                cx.bg0(),
                cx.rgb(tok),
                if lvl >= 2 { 0.2 } else { 0.08 },
            ))
            .patch(ink)
        } else {
            ink
        };
        let st = if lvl >= 2 {
            st.add_modifier(Modifier::BOLD)
        } else {
            st
        };
        placed.push((x, y, text, st, e.id.as_str()));
    }
    grid.flush(cx, buf);
    for (x, y, text, st, id) in placed {
        let w = super::width(&text, cx.uni()) + 2;
        let n = cx.put(buf, x, y, w, &format!(" {text} "), st);
        cx.hit(Rect::new(x, y, n, 1), format!("edge:{id}"));
    }

    draw_pill(cx, buf, &p, root);

    let (sel_id, focused) = match sel {
        Sel::Node(id, f) => (Some(id), f),
        _ => (None, false),
    };
    let touch = cx.touch();
    for n in &p.nodes {
        let Some(d) = s.device(&n.id) else { continue };
        let x0 = p.left(n);
        let picked = sel_id == Some(n.id.as_str());
        if picked {
            // One focus language in every tier: an accent bar left of the node (plus a
            // tint where tints exist) and a bold accent name.
            if cx.has_surface() {
                let tint = if focused {
                    cx.st(Tok::AccentBg)
                } else {
                    cx.st(Tok::SelectDimBg)
                };
                cx.fill(
                    buf,
                    Rect::new(x0.saturating_sub(1), n.y, n.w + 2, rows),
                    tint,
                );
            }
            let bar = if focused {
                cx.st(Tok::Accent)
            } else {
                cx.ink(Tok::Muted, 0.9)
            };
            for yy in n.y..n.y + rows {
                cx.put(buf, x0.saturating_sub(1), yy, 1, cx.g("▍", ">"), bar);
            }
        }
        draw_node(cx, buf, n, x0, p.mode, p.tall, picked && focused, edge_mode);
        // Thumb-sized targets on touch screens: at least 3 rows by 7 columns.
        let hit = if touch {
            let w = (n.w + 2).max(7);
            Rect::new(x0.saturating_sub(1), n.y.saturating_sub(1), w, rows + 2)
        } else {
            Rect::new(x0, n.y, n.w, rows)
        };
        cx.hit(hit.intersection(band), format!("device:{}", d.id));
    }

    for &(x, y, m, flip) in &p.more {
        let t = if p.mode == Mode::Chip {
            format!("+{m}")
        } else {
            format!("+{m} more")
        };
        let w = super::width(&t, cx.uni());
        let x = if flip { p.sx(true, x + w - 1) } else { x };
        cx.put(buf, x, y, w, &t, cx.st(Tok::Muted));
        let hit = if touch {
            Rect::new(x.saturating_sub(1), y.saturating_sub(1), w.max(3) + 2, 3)
        } else {
            Rect::new(x, y, w, 1)
        };
        cx.hit(hit.intersection(band), "palette:devices");
    }
}

/// One device: status dot, identity keycap, name and mesh squares, then the evidence row.
/// Mirrored nodes read right to left toward the pill (`name i ●`), so the dot always
/// meets the line.
#[allow(clippy::too_many_arguments)]
fn draw_node(
    cx: &Cx,
    buf: &mut Buffer,
    n: &Node,
    x0: u16,
    mode: Mode,
    tall: bool,
    hot: bool,
    fade: bool,
) {
    let Some(d) = cx.s.device(&n.id) else { return };
    let end = x0 + n.w;
    let (g, gst) = cx.status(d.status);
    let up = cx.up(d);
    let nst = if hot {
        cx.st(Tok::Accent).add_modifier(Modifier::BOLD)
    } else if up && !fade {
        cx.ink(Tok::Fg, 1.0).add_modifier(Modifier::BOLD)
    } else {
        cx.ink(Tok::Muted, 0.9)
    };
    // Name with its mesh squares, as one block.
    let draw_name = |buf: &mut Buffer, x: u16, room: u16| -> u16 {
        let bw = badges_w(d, mode);
        let nm = fit(&d.name, room.saturating_sub(bw) as usize, cx.uni());
        let mut w = cx.put(buf, x, n.y, room, &nm, nst);
        if bw > 0 {
            w += 1;
            w += tiles::mesh_badges(cx, buf, x + w, n.y, d);
        }
        w
    };
    let name_w = |room: u16| {
        let bw = badges_w(d, mode);
        super::width(
            &fit(&d.name, room.saturating_sub(bw) as usize, cx.uni()),
            cx.uni(),
        ) + bw
    };
    let ev = if tall && mode == Mode::Full {
        evidence(cx, d)
    } else {
        Vec::new()
    };
    if !n.flip {
        cx.put(buf, x0, n.y, 1, g, gst);
        let mut x = x0 + 2;
        x += cx.dchip(buf, x, n.y, d, false);
        if mode > Mode::Chip {
            draw_name(buf, x + 1, end.saturating_sub(x + 1));
            if !ev.is_empty() {
                draw_ev(cx, buf, x0 + 6, n.y + 1, &ev);
            }
        }
    } else {
        let dot = end.saturating_sub(1);
        cx.put(buf, dot, n.y, 1, g, gst);
        let kx = dot.saturating_sub(4);
        cx.dchip(buf, kx, n.y, d, false);
        if mode > Mode::Chip {
            let room = kx.saturating_sub(x0 + 1);
            let w = name_w(room);
            draw_name(buf, kx.saturating_sub(1 + w), room);
            if !ev.is_empty() {
                let w = ev_w(cx, &ev);
                draw_ev(cx, buf, kx.saturating_sub(1 + w), n.y + 1, &ev);
            }
        }
    }
}

/// The root device: a one-row tinted capsule with its own evidence inline. Without tints
/// it is a solid accent row (reversed without colour), the same shape as a keycap.
fn draw_pill(cx: &Cx, buf: &mut Buffer, p: &Plan, root: &str) {
    let (pill, mode) = (p.pill, p.pmode);
    let Some(d) = cx.s.device(root) else {
        return;
    };
    let tint: Rgb = mix(
        cx.bg0(),
        cx.rgb(Tok::Accent),
        if cx.dark() { 0.24 } else { 0.18 },
    );
    let (y, x0, x1) = (pill.y, pill.x, pill.right().saturating_sub(1));
    let inner = Rect::new(x0 + 1, y, pill.width.saturating_sub(2), 1);
    let body = if cx.has_surface() {
        cx.rgb_bg(tint)
    } else if cx.c.has_color() {
        Style::default()
            .bg(cx.st(Tok::Accent).fg.unwrap_or_default())
            .fg(ratatui::style::Color::Black)
    } else {
        Style::default().add_modifier(Modifier::REVERSED)
    };
    if cx.uni() || cx.c.has_color() {
        cx.fill(buf, inner, body);
    } else {
        cx.fill(buf, pill, body);
    }
    if cx.uni() {
        let cap = match body.bg {
            Some(c) => Style::default().fg(c),
            None => Style::default(),
        };
        cx.put(buf, x0, y, 1, "▐", cap);
        cx.put(buf, x1, y, 1, "▌", cap);
    }
    let mut x = x0 + 1 + u16::from(mode > Mode::Chip);
    x += cx.dchip(buf, x, y, d, false);
    if mode > Mode::Chip {
        x += 1;
        let ev = pill_ev(cx, root, mode, p.rich);
        let room = x1.saturating_sub(x + if ev.is_empty() { 0 } else { ev_w(cx, &ev) + 2 });
        let st = if cx.has_surface() {
            cx.ink(Tok::Fg, 1.0).patch(body)
        } else {
            body
        };
        x += cx.put(buf, x, y, room, &d.name, st.add_modifier(Modifier::BOLD));
        if !ev.is_empty() {
            let ev: Vec<Seg> = ev
                .into_iter()
                .map(|(t, s, g)| (t, body.patch(s), g))
                .collect();
            draw_ev(cx, buf, x + 2, y, &ev);
        }
    }
    let hit = if cx.touch() {
        Rect::new(pill.x, y.saturating_sub(1), pill.width, 3)
    } else {
        pill
    };
    cx.hit(hit, format!("device:{}", d.id));
}
