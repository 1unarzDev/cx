//! The network schematic, drawn as a star around the root device (this device on the
//! hub): the root is a tinted pill in the middle, the devices it reaches sit on both
//! sides of it and devices reached through them sit further out on their parent's side.
//! The two halves are balanced by height. The left half is laid out as a right half and
//! mirrored through the pill, so one router serves both. Links are box-drawing lines with
//! rounded elbows (`─│╭╮╰╯`, ASCII `-|+`) coloured by network, and their style is the
//! access policy: solid for key, double for password, dotted for off or down. Each link
//! carries one chip on its last straight run (policy and rtt), which is also its tap
//! target. No two links cross: they leave the pill from its top and bottom edges, nearest
//! target last, and children leave their parent on a short trunk. Peer links among the
//! outer devices are faint secondary strokes (brackets, or arcs over the pill) where they
//! cross nothing. Nothing here dispatches.
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
    /// Its letters are labels, not keys: inside a device, letters are slots and verbs.
    Root,
    /// As `Root`, but every letter is live (the send-to picker: a letter sends there).
    Targets,
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
    /// Label richness (keycap-only maps still name their outermost column).
    mode: Mode,
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
    /// Longest pill name (shortened with an ellipsis beyond it).
    pmax: u16,
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

/// Device ids in selection order: the viewer first (launch focus, so `⏎` opens this
/// device), then column by column, top to bottom.
pub(super) fn order(cx: &Cx, band: Rect) -> Vec<String> {
    let root = cx.s.viewer.as_str();
    let mut nodes = plan(cx, band, root, 3, None).nodes;
    nodes.sort_by_key(|n| (n.col, n.y, !n.flip));
    std::iter::once(root.to_string())
        .chain(nodes.into_iter().map(|n| n.id))
        .collect()
}

/// Rows the hub schematic needs: its tallest column of two-row cards at a calm spacing,
/// plus one pad row above and below (where peer links arc over and under the pill).
pub(super) fn want_h(cx: &Cx) -> u16 {
    let cols = columns(cx, &cx.s.viewer, 3);
    let most = halves(cx, &cols, None)
        .iter()
        .flat_map(|h| h.iter().map(|c| c.len()))
        .max()
        .unwrap_or(1) as u16;
    let sp = match cx.size() {
        Size::L => 6,
        Size::M => 5,
        _ => 3,
    };
    (most.max(1) - 1) * sp + 2 + 2
}

/// The routed path from the viewer to `to` (at most three hops, shortest first), as the
/// devices passed and the links taken; empty for the viewer itself or no route.
pub(super) fn route<'a>(cx: &Cx<'a>, to: &str) -> Vec<&'a LabEdge> {
    let s = cx.s;
    let mut paths: Vec<(String, Vec<&LabEdge>)> = vec![(s.viewer.clone(), Vec::new())];
    let mut seen = vec![s.viewer.clone()];
    for _ in 0..3 {
        let mut next = Vec::new();
        for (at, path) in &paths {
            for e in s.edges.iter().filter(|e| routed(e) && e.up) {
                let other = if &e.from == at {
                    &e.to
                } else if &e.to == at {
                    &e.from
                } else {
                    continue;
                };
                if seen.contains(other) {
                    continue;
                }
                let mut p = path.clone();
                p.push(e);
                if other == to {
                    return p;
                }
                seen.push(other.clone());
                next.push((other.clone(), p));
            }
        }
        paths = next;
    }
    Vec::new()
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
    // Slot letters, one space apart (two after the temperature).
    for (i, (t, st)) in tiles::pip_segs(cx, d, 4).into_iter().enumerate() {
        if i == 0 && !v.is_empty() {
            v.push((String::new(), st, true));
        }
        v.push((t, st, !v.is_empty()));
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

fn pill_w(cx: &Cx, root: &str, mode: Mode, rich: bool, nmax: u16) -> u16 {
    let name =
        cx.s.device(root)
            .map_or(4, |d| (d.name.chars().count() as u16).min(nmax));
    let ev = ev_w(cx, &pill_ev(cx, root, mode, rich));
    let badges = cx.s.device(root).map_or(0, |d| badges_w(d, mode));
    match mode {
        Mode::Chip => 5,
        _ => 8 + name + badges + if ev > 0 { ev + 2 } else { 0 },
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
    // Richest label mode whose columns fit. Chips ride only at M+ (there the least gap
    // leaves a link room for `⇄ pass`); below that the line style carries the policy and
    // a gap needs only a few line cells. The pill is the hero: it keeps its full name
    // until nothing else fits, and keycap-only maps still name their outermost column,
    // whose names grow away from the pill.
    // Least gaps: next to the pill (its links leave from ports above and below it, so
    // their runs are already long) and between outer columns (a trunk lane, then the run).
    let (least0, least1) = if cx.size() >= Size::M {
        (9, 15)
    } else {
        (4, 4)
    };
    let gap_n = |w: &[Vec<u16>; 2]| {
        let mut n = (0u16, 0u16);
        for half in w {
            for (k, x) in half.iter().enumerate() {
                if *x > 0 {
                    if k == 0 {
                        n.0 += 1;
                    } else {
                        n.1 += 1;
                    }
                }
            }
        }
        n
    };
    let mut mode = Mode::Chip;
    let mut nmax = 8;
    let mut outer = false;
    let mut widths: [Vec<u16>; 2] = [Vec::new(), Vec::new()];
    let mut rich = false;
    let mut pmode = Mode::Chip;
    // (node labels, longest name, pill evidence, pill labels, longest pill name, names
    // on the outermost column in keycap mode).
    let tries = [
        (Mode::Full, 22, true, Mode::Full, 22, false),
        (Mode::Full, 16, true, Mode::Full, 22, false),
        (Mode::Full, 22, false, Mode::Full, 22, false),
        (Mode::Full, 16, false, Mode::Full, 22, false),
        (Mode::Full, 12, false, Mode::Full, 22, false),
        (Mode::Name, 22, false, Mode::Name, 22, false),
        (Mode::Name, 16, false, Mode::Name, 22, false),
        (Mode::Name, 12, false, Mode::Name, 22, false),
        (Mode::Name, 10, false, Mode::Name, 22, false),
        (Mode::Name, 8, false, Mode::Name, 22, false),
        (Mode::Chip, 8, false, Mode::Name, 22, true),
        (Mode::Chip, 6, false, Mode::Name, 22, true),
        (Mode::Chip, 5, false, Mode::Name, 22, true),
        (Mode::Chip, 5, false, Mode::Name, 6, true),
        (Mode::Chip, 8, false, Mode::Name, 6, false),
        (Mode::Chip, 8, false, Mode::Chip, 0, false),
    ];
    let mut gaps = (0u16, 0u16);
    let mut pmax = 0;
    for (m, nm, pr, pm, px, out) in tries {
        mode = m;
        nmax = nm;
        rich = pr;
        pmode = pm;
        pmax = px;
        outer = out;
        for s in 0..2 {
            let last = hv[s].iter().rposition(|c| !c.is_empty());
            widths[s] = hv[s]
                .iter()
                .enumerate()
                .map(|(k, c)| {
                    let m = if outer && Some(k) == last {
                        Mode::Name
                    } else {
                        m
                    };
                    c.iter()
                        .take(cap(c.len()))
                        .map(|id| label_w(cx, id, m, tall, nmax))
                        .max()
                        .unwrap_or(0)
                })
                .collect();
        }
        gaps = gap_n(&widths);
        let need = 2
            + pill_w(cx, root, pm, pr, px)
            + widths.iter().flatten().sum::<u16>()
            + least0 * gaps.0
            + least1 * gaps.1;
        if need <= band.width {
            break;
        }
    }
    let pw = pill_w(cx, root, pmode, rich, pmax).min(band.width);
    let fixed = pw + widths.iter().flatten().sum::<u16>();
    let free = band.width.saturating_sub(fixed);
    // Gaps grow with the width (capped so wide screens get air, not stretched lines);
    // what is left centres the drawing.
    let n = gaps.0 + gaps.1;
    let extra = free.saturating_sub(least0 * gaps.0 + least1 * gaps.1) / n.max(1);
    let gap = |k: usize| {
        let least = if k == 0 { least0 } else { least1 };
        (least + extra).min(24.max(least))
    };
    let gsum = |half: &[u16]| -> u16 {
        half.iter()
            .enumerate()
            .filter(|(_, w)| **w > 0)
            .map(|(k, w)| w + gap(k))
            .sum()
    };
    let left_ext = gsum(&widths[1]);
    let total = pw + gsum(&widths[0]) + left_ext;
    let margin = band.width.saturating_sub(total) / 2;
    let rows = if tall && mode == Mode::Full { 2 } else { 1 };
    // Node spacing: the rows left after one pad row above and below, shared out, capped
    // so a tall band gives air rather than stretched lines.
    let (min_sp, max_sp) = match (rows == 2, cx.size()) {
        (false, _) => (2, 3),
        (true, Size::L) => (3, 7),
        (true, _) => (3, 5),
    };
    let spacing = |n: u16| -> u16 {
        if n <= 1 {
            return 0;
        }
        let sp = (h.saturating_sub(rows + 2) / (n - 1)).clamp(min_sp, max_sp);
        // One-row cards in an even column: an even spacing puts the pill row exactly
        // between them.
        if rows == 1 && n % 2 == 0 && sp % 2 == 1 {
            if (n - 1) * (sp + 1) + rows + 2 <= h || sp == min_sp {
                sp + 1
            } else {
                sp - 1
            }
        } else {
            sp
        }
    };
    let span = |n: u16| (n.max(1) - 1) * spacing(n) + rows;
    // The tallest column decides where the pill row sits; the drawing is centred.
    let tallest = hv
        .iter()
        .flat_map(|c| {
            c.iter().map(|c| {
                let k = cap(c.len());
                (k + usize::from(k < c.len())) as u16
            })
        })
        .map(span)
        .max()
        .unwrap_or(1);
    let top = band.y + (h.saturating_sub(tallest) + 1) / 2;
    let cy = top + tallest.saturating_sub(1) / 2;
    let pill = Rect::new(band.x + margin + left_ext, cy, pw, 1);

    let mut nodes = Vec::new();
    let mut more = Vec::new();
    for (s, half) in hv.iter().enumerate() {
        let mut x = pill.right() + gap(0);
        let last = half.iter().rposition(|c| !c.is_empty());
        for (k, c) in half.iter().enumerate() {
            if c.is_empty() {
                continue;
            }
            let m = if outer && Some(k) == last {
                Mode::Name
            } else {
                mode
            };
            let shown = if cap(c.len()) < c.len() {
                cap(c.len()).saturating_sub(1)
            } else {
                c.len()
            };
            let slots = shown + usize::from(shown < c.len());
            let n = slots.max(1) as u16;
            let sp = spacing(n);
            // Centre the column on the pill row (odd counts put a node on it).
            let y0 = cy
                .saturating_sub((span(n) - 1) / 2)
                .clamp(band.y, band.bottom().saturating_sub(span(n)).max(band.y));
            for (i, id) in c.iter().take(shown).enumerate() {
                nodes.push(Node {
                    id: id.clone(),
                    col: k + 1,
                    flip: s == 1,
                    x,
                    y: y0 + i as u16 * sp,
                    w: label_w(cx, id, m, tall, nmax),
                    mode: m,
                });
            }
            if shown < c.len() {
                more.push((x + 2, y0 + shown as u16 * sp, c.len() - shown, s == 1));
            }
            x += widths[s][k] + gap(k + 1);
        }
    }
    Plan {
        nodes,
        pill,
        tall,
        mode,
        pmode,
        rich,
        pmax,
        more,
    }
}

const N: u8 = 1;
const E: u8 = 2;
const S: u8 = 4;
const W: u8 = 8;

/// Line styles: the access policy, readable without colour. Password is a double line
/// and off a dotted one, so the three never look alike in any font (unlike two dash
/// counts); the edited link is heavy.
const SOLID: u8 = 0;
const DOUBLE: u8 = 1;
const DOT: u8 = 2;
const HEAVY: u8 = 3;

/// Line canvas: per cell the directions a line leaves it, its colour and emphasis.
struct Grid {
    r: Rect,
    bits: Vec<u8>,
    lvl: Vec<u8>,
    tok: Vec<Tok>,
    style: Vec<u8>,
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
            let style = self.style[i];
            let g = if !cx.uni() {
                // A double vertical has no ASCII form: `|` (the `=` runs carry it).
                match (horiz, vert, style) {
                    (true, _, DOUBLE) => "=",
                    (true, _, DOT) => ".",
                    (_, true, DOT) => ":",
                    (true, _, _) => "-",
                    (_, true, _) => "|",
                    _ => "+",
                }
            } else {
                // Corners: rounded, heavy or double, by style.
                let pick =
                    |round: &'static str, heavy: &'static str, double: &'static str| match style {
                        HEAVY => heavy,
                        DOUBLE => double,
                        _ => round,
                    };
                match b {
                    _ if horiz => match style {
                        DOUBLE => "═",
                        DOT => "┈",
                        HEAVY => "━",
                        _ => "─",
                    },
                    _ if vert => match style {
                        DOUBLE => "║",
                        DOT => "┊",
                        HEAVY => "┃",
                        _ => "│",
                    },
                    x if x == E | S => pick("╭", "┏", "╔"),
                    x if x == W | S => pick("╮", "┓", "╗"),
                    x if x == N | E => pick("╰", "┗", "╚"),
                    x if x == N | W => pick("╯", "┛", "╝"),
                    x if x == N | E | S => "├",
                    x if x == N | W | S => "┤",
                    x if x == E | W | S => "┬",
                    x if x == N | E | W => "┴",
                    _ => "┼",
                }
            };
            // Resting lines stay at >= 3:1 on a light background.
            let a: f32 = match self.lvl[i] {
                0 => {
                    if cx.dark() {
                        0.32
                    } else {
                        0.5
                    }
                }
                1 => 0.85,
                _ => 1.0,
            };
            let mut st = if !cx.dark() && cx.tc() {
                // On cream, the hue is deepened so even resting strokes stay above 3:1.
                let a = a.max(0.8);
                cx.rgb_fg(mix(mix(cx.bg0(), cx.rgb(self.tok[i]), a), (0, 0, 0), 0.3))
            } else {
                cx.ink(self.tok[i], a)
            };
            if self.lvl[i] >= 2 {
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
        // Hue means mesh and nothing else: lan and direct ssh links are neutral (the
        // device view names their transport).
        EdgeKind::Lan | EdgeKind::Direct | EdgeKind::Jump(_) => Tok::Muted,
    }
}

/// Line style of a link: key access anywhere is solid, password only is double, nothing
/// allowed (or the link is down) is dotted.
fn line_style(e: &LabEdge) -> u8 {
    let any = |a: Access| e.forward == a || e.reverse == a;
    if !e.up || (e.forward == Access::Denied && e.reverse == Access::Denied) {
        DOT
    } else if any(Access::Key) {
        SOLID
    } else {
        DOUBLE
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
        // ASCII `x` means down; a closed link says `off` instead.
        (Access::Denied, Access::Denied) => (cx.g("⊘", "off"), "off".to_string()),
        (a, Access::Denied) => (cx.g("→", "->"), word(a).into()),
        (Access::Denied, a) => (cx.g("←", "<-"), word(a).into()),
        (a, b) if a == b => (cx.g("⇄", "<>"), word(a).into()),
        (a, b) => (cx.g("⇄", "<>"), format!("{}/{}", word(a), word(b))),
    };
    if short || g == w {
        g.into()
    } else {
        format!("{g} {w}")
    }
}

/// Chip text for a link run of `len` cells, one grammar at every size: direction glyph,
/// policy word, then the rtt (or an active transfer's rate) when it fits, with at least
/// one line cell either side. Below 100 columns only links where nothing is allowed get
/// a chip (`⊘`, `✕`); the line style and the header legend carry the rest there.
fn chip_text(cx: &Cx, e: &LabEdge, from_left: bool, len: u16) -> Option<String> {
    let long = policy(cx, e, from_left, false);
    if cx.size() <= Size::S {
        let short = policy(cx, e, from_left, true);
        let closed = !e.up || (e.forward == Access::Denied && e.reverse == Access::Denied);
        return (closed && len >= 5).then_some(short);
    }
    let mut tries = Vec::new();
    if let Some(rate) = transfer_rate(cx, e) {
        tries.push(format!("{long} {rate}"));
    } else if let Some(ms) = e.rtt_ms.filter(|_| e.up) {
        tries.push(format!("{long} {}", ms_text(ms)));
    }
    tries.push(long);
    tries
        .into_iter()
        // Padded by a tinted space each side, plus a line cell each side.
        .find(|t| super::width(t, cx.uni()) + 4 <= len)
}

/// An active transfer over this link: its rate, with an arrow toward the receiver in
/// Unicode (`↓38M/s` when it comes to this device).
fn transfer_rate(cx: &Cx, e: &LabEdge) -> Option<String> {
    let t =
        cx.s.transfers
            .iter()
            .find(|t| (t.from == e.from && t.to == e.to) || (t.from == e.to && t.to == e.from))?;
    let rate = super::fmt::bytes(t.rate_hist.last().copied().unwrap_or(0.0) as u64);
    let arrow = if t.to == cx.s.viewer { "↓" } else { "↑" };
    Some(if cx.uni() {
        format!("{arrow}{rate}/s")
    } else {
        format!("{rate}/s")
    })
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
    let mark = cx.hit_mark();
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
        Sel::Root | Sel::Targets => e.from == root || e.to == root,
    };
    let edge_mode = matches!(sel, Sel::Edge(_));

    let mut grid = Grid::new(band);
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
    // Peer links (both ends one hop or more out, in one column) are the rest of the mesh.
    // They appear only while one end is selected (lit, with a chip), arcing over the pill
    // through the top pad row (or under it) when both ends are the outermost cards of
    // their sides; anything else would cross a line and is left to the device view.
    let rows_n = rows;
    #[allow(clippy::type_complexity)]
    let mut peers: Vec<(&LabEdge, Vec<(u16, u16)>, (u16, u16, u16), bool, Option<u8>)> = Vec::new();
    for e in s
        .edges
        .iter()
        .filter(|e| routed(e) && e.from != root && e.to != root)
    {
        if edge_mode || !(lit(e) && matches!(sel, Sel::Node(..))) {
            continue;
        }
        let (Some(a), Some(b)) = (find(&e.from), find(&e.to)) else {
            continue;
        };
        if a.col != b.col {
            continue;
        }
        let dot = |n: &Node| {
            if n.flip {
                p.left(n) + n.w - 1
            } else {
                n.x
            }
        };
        if a.flip == b.flip {
            continue;
        } else {
            let edge_of = |n: &Node, top: bool| {
                p.nodes
                    .iter()
                    .filter(|m| m.flip == n.flip && m.col == n.col)
                    .all(|m| if top { m.y >= n.y } else { m.y <= n.y })
            };
            let over = if edge_of(a, true) && edge_of(b, true) {
                true
            } else if edge_of(a, false) && edge_of(b, false) {
                false
            } else {
                continue;
            };
            let y = if over {
                match a.y.min(b.y).checked_sub(1) {
                    Some(y) if y >= band.y => y,
                    _ => continue,
                }
            } else {
                let y = a.y.max(b.y) + rows_n;
                if y >= band.bottom() {
                    continue;
                }
                y
            };
            let (l, r) = if dot(a) < dot(b) { (a, b) } else { (b, a) };
            let leg = |n: &Node| if over { n.y - 1 } else { n.y + 1 };
            let pts = vec![(dot(l), leg(l)), (dot(l), y), (dot(r), y), (dot(r), leg(r))];
            let from_left = l.id == e.from;
            let stub = if over { S } else { N };
            peers.push((
                e,
                pts,
                (dot(l) + 1, dot(r).saturating_sub(1), y),
                from_left,
                Some(stub),
            ));
        }
    }
    let mut chips: Vec<(u16, u16, u16, &LabEdge, Tok, u8, bool)> = Vec::new();
    for (i, &(e, l, r, flip)) in links.iter().enumerate() {
        let (ly, ry) = (row_of(l), row_of(r));
        let Some(rn) = find(r) else { continue };
        let busy = !edge_mode && e.up && transfer_rate(cx, e).is_some();
        let lvl = if lit(e) || busy {
            2
        } else if edge_mode {
            0
        } else {
            1
        };
        let tok = edge_tok(cx, e);
        // The edited link, and a link carrying a transfer (motion on the map always
        // means traffic; a live build pulses it at 2-4 Hz), are heavy. ASCII keeps the
        // policy style there (bold marks it).
        let style = if ((edge_mode && lvl >= 2) || busy) && cx.uni() {
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
            let base = col_right(lc, flip) + 2;
            let k = rank(&|_| true);
            let lane = links
                .iter()
                .filter(|(_, l2, _, f2)| *f2 == flip && col_of(l2) == Some(lc) && row_of(l2) < ly)
                .count() as u16
                + k;
            // Never into the names: at worst one blank cell after the widest label.
            let xb = (base + 2 * lane).min(end.saturating_sub(5)).max(base - 1);
            let sx = right_of(l);
            if ry == ly {
                // The chip stays clear of sibling trunks leaving this run.
                (vec![(sx, ly), (end, ry)], (xb.max(sx), end, ry))
            } else {
                (vec![(sx, ly), (xb, ly), (xb, ry), (end, ry)], (xb, end, ry))
            }
        };
        let pts: Vec<(u16, u16)> = pts.iter().map(|&(x, y)| (p.sx(flip, x), y)).collect();
        if let Some((x, y, b)) = stub {
            grid.mark(p.sx(flip, x), y, b, lvl, tok, style);
        }
        grid.path(&pts, lvl, tok, style);
        let (rx0, rx1, y) = run;
        let (a, b) = (p.sx(flip, rx0), p.sx(flip, rx1));
        // The root-side end is on the screen's left unless the half is mirrored.
        let from_left = (l == e.from) != flip;
        chips.push((a.min(b), a.max(b), y, e, tok, lvl, from_left));
    }

    for (e, pts, (x0, x1, y), from_left, stub) in peers {
        let lvl = 2;
        let tok = edge_tok(cx, e);
        let style = line_style(e);
        let cells = grid.path(&pts, lvl, tok, style);
        // An arc's legs end on the cards' status dots.
        if let (Some(b), Some(&f), Some(&l)) = (stub, cells.first(), cells.last()) {
            for (x, y) in [f, l] {
                grid.mark(x, y, b, lvl, tok, style);
            }
        }
        chips.push((x0, x1, y, e, tok, lvl, from_left));
    }

    // One chip per link, centred on its last straight run.
    let mut placed: Vec<(u16, u16, String, Style, &str)> = Vec::new();
    for (x0, x1, y, e, tok, lvl, from_left) in chips {
        let len = x1.saturating_sub(x0);
        let text = chip_text(cx, e, from_left, len);
        // Padded with a tinted space each side, centred between the corner (`x0`) and
        // the run's end with at least one line cell on both sides.
        let w = text.as_ref().map_or(0, |t| super::width(t, cx.uni()) + 2);
        let x = x0 + 2 + len.saturating_sub(2 + w) / 2;
        let Some(text) = text else {
            // No chip (phones): the run itself is the tap target, three rows tall.
            if len >= 2 {
                let r = Rect::new(x0 + 1, y.saturating_sub(1), len - 1, 3);
                cx.hit(r.intersection(band), format!("edge:{}", e.id));
            }
            continue;
        };
        let a: f32 = match lvl {
            0 => 0.4,
            1 => 0.9,
            _ => 1.0,
        };
        let ink = if !cx.dark() && cx.tc() {
            // Light theme: the hue at full strength, deepened, on a pale tint (>= 4.5:1).
            cx.rgb_fg(mix(mix(cx.bg0(), cx.rgb(tok), a.max(0.8)), (0, 0, 0), 0.42))
        } else {
            cx.ink(tok, a)
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
        let text = format!(" {text} ");
        placed.push((x, y, text, st, e.id.as_str()));
    }
    grid.flush(cx, buf);
    for (x, y, text, st, id) in placed {
        let w = super::width(&text, cx.uni());
        let n = cx.put(buf, x, y, w, &text, st);
        cx.hit(Rect::new(x, y, n, 1), format!("edge:{id}"));
    }

    let (sel_id, focused) = match sel {
        Sel::Node(id, f) => (Some(id), f),
        _ => (None, false),
    };
    draw_pill(cx, buf, &p, root, (sel_id == Some(root)).then_some(focused));
    let touch = cx.touch();
    let keys = matches!(sel, Sel::Node(..) | Sel::Targets);
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
        draw_node(cx, buf, n, x0, p.tall, picked && focused, keys, edge_mode);
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
    cx.unclash(mark);
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
    tall: bool,
    hot: bool,
    keys: bool,
    fade: bool,
) {
    let Some(d) = cx.s.device(&n.id) else { return };
    let mode = n.mode;
    // Where letters are not keys the identity is plain coloured text, so the screen never
    // shows a keycap that does something else when pressed.
    let ident = |buf: &mut Buffer, x: u16| {
        if keys {
            cx.dchip(buf, x, n.y, d, false);
        } else {
            let st = cx
                .ink(Tok::Id(cx.s.hue(&d.id)), 1.0)
                .add_modifier(Modifier::BOLD);
            cx.put(buf, x + 1, n.y, 1, &d.hotkey.to_string(), st);
        }
    };

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
        let x = x0 + 5;
        ident(buf, x0 + 2);
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
        ident(buf, kx);
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
fn draw_pill(cx: &Cx, buf: &mut Buffer, p: &Plan, root: &str, pick: Option<bool>) {
    let (pill, mode) = (p.pill, p.pmode);
    let Some(d) = cx.s.device(root) else {
        return;
    };
    let strength = match (pick, cx.dark()) {
        (Some(true), true) => 0.38,
        (Some(true), false) => 0.3,
        (_, true) => 0.24,
        (_, false) => 0.18,
    };
    let tint: Rgb = mix(cx.bg0(), cx.rgb(Tok::Accent), strength);
    // Selected: the same accent bar every other selection uses, left of the capsule.
    if let Some(f) = pick {
        let bar = if f {
            cx.st(Tok::Accent)
        } else {
            cx.ink(Tok::Muted, 0.9)
        };
        if pill.x > cx.area.x {
            cx.put(buf, pill.x - 1, pill.y, 1, cx.g("▍", ">"), bar);
        }
    }
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
        let bw = badges_w(d, mode);
        let room = x1.saturating_sub(x + bw + if ev.is_empty() { 0 } else { ev_w(cx, &ev) + 2 });
        let st = if cx.has_surface() {
            cx.ink(Tok::Fg, 1.0).patch(body)
        } else {
            body
        };
        let name = fit(&d.name, usize::from(p.pmax.min(room)), cx.uni());
        x += cx.put(buf, x, y, room, &name, st.add_modifier(Modifier::BOLD));
        if bw > 0 {
            x += 1;
            let bx = x;
            x += tiles::mesh_badges(cx, buf, x, y, d);
            cx.paint(
                buf,
                Rect::new(bx, y, x - bx, 1),
                body.bg.map_or(Style::default(), |c| Style::default().bg(c)),
            );
        }
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
