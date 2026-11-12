//! The network as a quiet diagram: the root device (this one on the hub) sits in the
//! middle as `◉` and its name, the devices it reaches sit in a column either side of it and
//! devices reached through them in the next column out, each subtree beside its relay.
//! Links are thin box-drawing lines of one weight with rounded corners: a short stub
//! leaves the root, a trunk runs along the column, and a branch enters each label with a
//! clear cell before it. Hue means network (mesh hue, teal for lan, neutral for direct
//! ssh; a line shared by several links is neutral); the selection's route is brighter.
//! The stroke is the access policy, used sparingly: solid where a key is allowed, dashed
//! where only a password is, an arrowhead at the end access reaches where it goes one
//! way, a faint line where nothing crosses (`⊘` mid-run when closed, the device's
//! `✕` when down). Nothing here dispatches.
use super::{fit, tiles, Cx, Size};
use crate::ui::lab::{Access, DeviceStatus, EdgeKind, LabDevice, LabEdge, Tok};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
};
use std::f32::consts::{PI, TAU};

/// What the map highlights.
#[derive(Clone, Copy)]
pub(super) enum Sel<'a> {
    /// A device; `true` when the map has keyboard focus.
    Node(&'a str, bool),
    /// One link (the edge editor): drawn bright, everything else fades.
    Edge(&'a str),
    /// Every link of the root lit; letters are plain labels (inside a device, letters
    /// are slots and verbs).
    Root,
    /// As `Root`, but every letter is live (the send-to picker).
    Targets,
}

/// Label richness, chosen per frame for the whole map.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Mode {
    /// The keycap letter alone.
    Key,
    /// Status dot and name.
    Name,
}

struct Node<'a> {
    d: &'a LabDevice,
    hop: u8,
    /// The link to its parent (None for devices with no routed link).
    up: Option<&'a LabEdge>,
    parent: Option<usize>,
    /// Right of the root (1) or left (-1).
    side: i32,
    label: Rect,
    shown: bool,
}

struct Plan<'a> {
    nodes: Vec<Node<'a>>,
    mode: Mode,
    nmax: usize,
    pill: Rect,
    /// Columns between a parent's column and its children's.
    gap: u16,
    hidden: usize,
}

/// Jump edges are drawn through the hop they use, not as their own line.
fn routed(e: &LabEdge) -> bool {
    !matches!(e.kind, EdgeKind::Jump(_))
}

fn between<'a>(cx: &Cx<'a>, a: &str, b: &str) -> Option<&'a LabEdge> {
    cx.s.edges
        .iter()
        .find(|e| routed(e) && ((e.from == a && e.to == b) || (e.from == b && e.to == a)))
}

pub(super) fn mesh_tok(cx: &Cx, id: &str) -> Tok {
    // Mesh hues are kept apart from teal (lan) and the accent: violet, then blue.
    match cx.s.meshes.iter().position(|m| m.id == id) {
        Some(0) | None => Tok::Mesh,
        Some(_) => Tok::Id(4),
    }
}

/// A link's hue: its mesh, teal for lan, neutral for direct ssh, red when down.
pub(super) fn edge_tok(cx: &Cx, e: &LabEdge) -> Tok {
    if e.up {
        hue_tok(cx, e)
    } else {
        Tok::Err
    }
}

/// The hue of a link's network, up or not.
fn hue_tok(cx: &Cx, e: &LabEdge) -> Tok {
    match &e.kind {
        EdgeKind::Mesh(m) => mesh_tok(cx, m),
        EdgeKind::Lan => Tok::Lan,
        EdgeKind::Direct | EdgeKind::Jump(_) => Tok::Muted,
    }
}

/// The routed path from the viewer to `to` (at most three hops, shortest first), as the
/// links taken; empty for the viewer itself or no route.
pub(super) fn route<'a>(cx: &Cx<'a>, to: &str) -> Vec<&'a LabEdge> {
    route_from(cx, &cx.s.viewer, to)
}

/// The same from any device: only links that are up and open in the direction taken.
pub(super) fn route_from<'a>(cx: &Cx<'a>, from: &str, to: &str) -> Vec<&'a LabEdge> {
    let s = cx.s;
    let mut paths: Vec<(String, Vec<&LabEdge>)> = vec![(from.to_string(), Vec::new())];
    let mut seen = vec![from.to_string()];
    for _ in 0..3 {
        let mut next = Vec::new();
        for (at, path) in &paths {
            for e in s.edges.iter().filter(|e| routed(e) && e.up) {
                let other = if &e.from == at && e.forward != Access::Denied {
                    &e.to
                } else if &e.to == at && e.reverse != Access::Denied {
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

/// Policy shorthand, the arrow pointing the way access is allowed from `from_left`'s
/// side. `short` keeps only the glyph.
pub(super) fn policy(cx: &Cx, e: &LabEdge, from_left: bool, short: bool) -> String {
    if !e.up {
        return if short {
            cx.g("✕", "X").into()
        } else {
            format!("{} down", cx.g("✕", "X"))
        };
    }
    let (to_right, to_left) = if from_left {
        (e.forward, e.reverse)
    } else {
        (e.reverse, e.forward)
    };
    let word = |a: Access| match a {
        Access::Key => "key",
        Access::Password => "pass",
        Access::Denied => "off",
    };
    let (g, w) = match (to_right, to_left) {
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

fn closed(e: &LabEdge) -> bool {
    e.forward == Access::Denied && e.reverse == Access::Denied
}

#[derive(Clone, Copy, PartialEq)]
enum Pat {
    Solid,
    /// Dashed: a password is needed.
    Dash,
    /// Faint: nothing crosses (closed both ways, or down).
    Quiet,
}

/// Stroke of a link (the policy grammar).
fn pattern(e: &LabEdge) -> Pat {
    if !e.up || closed(e) {
        Pat::Quiet
    } else if e.forward == Access::Key || e.reverse == Access::Key {
        Pat::Solid
    } else {
        Pat::Dash
    }
}

// ------------------------------------------------------------------------ layout

/// Spanning tree from `root` over routed links, breadth first, up to `depth` hops.
/// On the hub (depth 3) devices with no routed link join the first column unlinked.
fn tree<'a>(cx: &Cx<'a>, root: &str, depth: usize) -> Vec<Node<'a>> {
    let s = cx.s;
    let idx = |id: &str| s.devices.iter().position(|d| d.id == id).unwrap_or(0);
    let group = |id: &str| {
        s.meshes
            .iter()
            .position(|m| m.members.iter().any(|x| x == id))
            .unwrap_or(s.meshes.len())
    };
    let node = |d, hop, up, parent| Node {
        d,
        hop,
        up,
        parent,
        side: 1,
        label: Rect::default(),
        shown: true,
    };
    let mut nodes: Vec<Node> = Vec::new();
    let mut seen: Vec<&str> = vec![root];
    let mut frontier: Vec<Option<usize>> = vec![None];
    for hop in 1..=depth as u8 {
        let mut next = Vec::new();
        for &p in &frontier {
            let pid = p.map_or(root, |i| nodes[i].d.id.as_str());
            let mut kids: Vec<(&LabDevice, &LabEdge)> = s
                .devices
                .iter()
                .filter(|d| !seen.contains(&d.id.as_str()))
                .filter_map(|d| Some((d, between(cx, pid, &d.id)?)))
                .collect();
            // Mesh members sit together, so their trunk stays short.
            kids.sort_by_key(|(d, _)| (group(&d.id), idx(&d.id)));
            for (d, e) in kids {
                seen.push(&d.id);
                nodes.push(node(d, hop, Some(e), p));
                next.push(Some(nodes.len() - 1));
            }
        }
        if next.is_empty() {
            break;
        }
        frontier = next;
    }
    // A device reached by jumping through a relay hangs from that relay, its line the
    // jump link's (the hop itself is drawn once, as the relay's own link).
    for d in s.devices.iter().filter(|d| !seen.contains(&d.id.as_str())) {
        let jump = s.edges.iter().find_map(|e| match &e.kind {
            EdgeKind::Jump(via) if (e.from == root && e.to == d.id) || (e.to == root && e.from == d.id) => {
                let p = nodes.iter().position(|n| n.d.id == *via)?;
                (usize::from(nodes[p].hop) < depth).then_some((p, e))
            }
            _ => None,
        });
        if let Some((p, e)) = jump {
            let hop = nodes[p].hop + 1;
            nodes.push(node(d, hop, Some(e), Some(p)));
        } else if depth >= 3 {
            nodes.push(node(d, 1, None, None));
        }
    }
    nodes
}

/// The name, with a keycap in front when the hotkey letter is not in it.
fn has_letter(d: &LabDevice) -> Option<usize> {
    d.name
        .chars()
        .position(|c| c.eq_ignore_ascii_case(&d.hotkey))
}

fn label_w(cx: &Cx, d: &LabDevice, mode: Mode, nmax: usize, keys: bool) -> u16 {
    match mode {
        Mode::Key => 3 + u16::from(!matches!(d.status, DeviceStatus::Online | DeviceStatus::Local)),
        Mode::Name => 2 + name_w(cx, d, nmax, keys),
    }
}

/// The status glyph, a space and the name (a keycap with a cell each side).
fn pill_w(cx: &Cx, d: &LabDevice, mode: Mode, keys: bool) -> u16 {
    match mode {
        Mode::Key => 5,
        Mode::Name => 2 + name_w(cx, d, 16, keys),
    }
}

fn plan<'a>(cx: &Cx<'a>, band: Rect, root: &str, depth: usize, keys_only: bool, sel: Sel) -> Plan<'a> {
    let keys = matches!(sel, Sel::Node(..) | Sel::Targets);
    // The ladder, richest first: the drawing takes the most air its area allows (rows
    // three apart only with whole names, then two, then one), then shorter names, then
    // bare keycaps. Within each, the orbit's arc before a flat column, and the widest
    // column gap down to the shortest that keeps a corner.
    // The device view's star and the edge editor's map have the screen to themselves:
    // they spread further apart where the rows allow.
    let roomy = matches!(sel, Sel::Edge(_) | Sel::Root);
    let (sp_max, full): (i32, u16) = match cx.size() {
        Size::L => (if roomy { 4 } else { 2 }, 12),
        Size::M => (if roomy { 3 } else { 2 }, 10),
        Size::S => (2, 8),
        Size::Xs => (2, 6),
    };
    let mut ladder: Vec<(i32, Mode, usize)> = Vec::new();
    if keys_only {
        ladder.extend([(2, Mode::Key, 0), (1, Mode::Key, 0)]);
    } else {
        for sp in (1..=sp_max).rev() {
            let names: &[usize] = if sp > 2 { &[16] } else { &[16, 12, 9] };
            ladder.extend(names.iter().map(|&n| (sp, Mode::Name, n)));
        }
        ladder.push((1, Mode::Key, 0));
    }
    let mut best = None;
    for (sp, mode, nmax) in ladder {
        for arc in [true, false] {
            for gap in (full.min(6)..=full).rev() {
                let p = place(cx, band, tree(cx, root, depth), root, mode, nmax, sp, keys, gap, arc);
                let ok = p.hidden == 0;
                best = Some(p);
                if ok {
                    return best.expect("set");
                }
            }
        }
    }
    best.expect("at least one mode")
}

/// A relay's subtree fans away from the root's row: its first device continues the
/// relay's row, each next one a step further out (`dir`). Returns the outermost row used.
fn fan(i: usize, r: i32, dir: i32, sp: i32, kids: &[Vec<usize>], row: &mut [i32]) -> i32 {
    row[i] = r;
    let mut out = r;
    for (j, &k) in kids[i].iter().enumerate() {
        let at = if j == 0 { r } else { out + dir * sp };
        out = fan(k, at, dir, sp, kids, row);
    }
    out
}

/// Lay the network out as an orbit: the root in the middle, its peers split between its
/// right and left (both sides about as many rows) and, on each side, placed
/// symmetrically about the root's row (an odd one out on the row itself), those nearer
/// the row further out, so their labels trace an arc. A relay's devices fan outward
/// from it, away from the root's row. Every device owns its row on its side, so labels
/// and lines never cross. Labels that leave the band are hidden (counted in `hidden`).
#[allow(clippy::too_many_arguments)]
fn place<'a>(
    cx: &Cx<'a>,
    band: Rect,
    mut nodes: Vec<Node<'a>>,
    root: &str,
    mode: Mode,
    nmax: usize,
    sp: i32,
    keys: bool,
    gap: u16,
    arc: bool,
) -> Plan<'a> {
    let n = nodes.len();
    let kids: Vec<Vec<usize>> = (0..n)
        .map(|i| (0..n).filter(|&k| nodes[k].parent == Some(i)).collect())
        .collect();
    let mut leaves = vec![0i32; n];
    for i in (0..n).rev() {
        leaves[i] = leaves[i].max(1);
        if let Some(p) = nodes[i].parent {
            leaves[p] += leaves[i];
        }
    }
    let tops: Vec<usize> = (0..n).filter(|&i| nodes[i].parent.is_none()).collect();
    // The split that evens the sides' rows (the right one takes a tie: rows are longer
    // to read left to right).
    let total: i32 = tops.iter().map(|&i| leaves[i]).sum();
    let mut right = tops.len();
    let mut acc = 0;
    let mut best = i32::MAX;
    for (k, &i) in tops.iter().enumerate() {
        acc += leaves[i];
        let diff = (2 * acc - total).abs();
        if diff < best || (diff == best && 2 * acc >= total) {
            best = diff;
            right = k + 1;
        }
    }
    let wv: Vec<i32> = nodes
        .iter()
        .map(|nd| i32::from(label_w(cx, nd.d, mode, nmax, keys)))
        .collect();
    let rootd = cx.s.device(root);
    let pw = i32::from(rootd.map_or(5, |d| pill_w(cx, d, mode, keys)));
    let mut row = vec![0i32; n];
    let mut x = vec![0i32; n];
    let gap = i32::from(gap);
    for (side, part) in [(1, &tops[..right]), (-1, &tops[right..])] {
        // An odd one out sits on the root's row when it is a single device; the rest
        // split above and below so both hold about as many rows (the heaviest subtree
        // first, each to the lighter half), keeping their order within a half.
        let mid = (part.len() % 2 == 1)
            .then(|| part.iter().position(|&i| kids[i].is_empty()))
            .flatten()
            .map(|j| part[j]);
        let mut heavy: Vec<usize> = part.iter().copied().filter(|&i| Some(i) != mid).collect();
        heavy.sort_by_key(|&i| -leaves[i]);
        let (mut wu, mut wd) = (0, 0);
        let mut is_up = vec![false; n];
        for &i in &heavy {
            if wu <= wd {
                wu += leaves[i];
                is_up[i] = true;
            } else {
                wd += leaves[i];
            }
        }
        let up: Vec<usize> = part.iter().copied().filter(|&i| is_up[i]).collect();
        let down: Vec<usize> = part.iter().copied().filter(|&i| Some(i) != mid && !is_up[i]).collect();
        let mut below = sp;
        if let Some(c) = mid {
            fan(c, 0, 1, sp, &kids, &mut row);
        }
        for &k in &down {
            below = fan(k, below, 1, sp, &kids, &mut row) + sp;
        }
        let mut above = -sp;
        for &k in up.iter().rev() {
            above = fan(k, above, -1, sp, &kids, &mut row) - sp;
        }
        // The orbit: peers nearer the root's row sit further out, on an ellipse as
        // wide as the side is tall (cells are about twice as tall as wide).
        let reach = part.iter().map(|&i| row[i].abs()).max().unwrap_or(0);
        let lift = |i: usize| -> i32 {
            if !arc || reach == 0 {
                return 0;
            }
            let t = row[i].abs() as f32 / reach as f32;
            (reach.min(10) as f32 * (1.0 - t * t).sqrt()).round() as i32
        };
        // Outward, hop by hop: a device's column follows its relay's label.
        let mut queue: Vec<usize> = part.to_vec();
        let mut k = 0;
        while k < queue.len() {
            let i = queue[k];
            nodes[i].side = side;
            let (port, extra) = match nodes[i].parent {
                None => (if side > 0 { pw } else { 0 }, lift(i)),
                Some(p) => (if side > 0 { x[p] + wv[p] } else { x[p] }, 0),
            };
            x[i] = if side > 0 {
                port + gap + extra
            } else {
                port - gap - extra - wv[i]
            };
            queue.extend(kids[i].iter().copied());
            k += 1;
        }
    }
    // Centre the drawing (not the root: one side often reaches further) in the band,
    // with a two-column gutter at either side; too wide, it keeps its left gutter.
    let (lo, hi, top, bot) = (0..n)
        .map(|i| (x[i], row[i], wv[i]))
        .chain([(0, 0, pw)])
        .fold((i32::MAX, i32::MIN, i32::MAX, i32::MIN), |a, (x, y, w)| {
            (a.0.min(x), a.1.max(x + w), a.2.min(y), a.3.max(y + 1))
        });
    let g = 2;
    let (bx0, bx1) = (i32::from(band.x) + g, i32::from(band.right()) - g);
    let (by0, by1) = (i32::from(band.y), i32::from(band.bottom()));
    let dx = ((bx0 + bx1 - lo - hi) / 2).min(bx1 - hi).max(bx0 - lo);
    let dy = ((by0 + by1 - top - bot) / 2).min(by1 - bot).max(by0 - top);
    for i in 0..n {
        let (lx, ly) = (x[i] + dx, row[i] + dy);
        nodes[i].label = Rect::new(lx.max(0) as u16, ly.max(0) as u16, wv[i] as u16, 1);
        nodes[i].shown = lx >= bx0 && lx + wv[i] <= bx1 && ly >= by0 && ly < by1;
    }
    // A device whose relay is hidden hides too (it would hang from nothing).
    for i in 0..n {
        if nodes[i].parent.is_some_and(|p| !nodes[p].shown) {
            nodes[i].shown = false;
        }
    }
    let pill = Rect::new(dx.max(0) as u16, dy.max(0) as u16, pw as u16, 1);
    let hidden = nodes.iter().filter(|n| !n.shown).count();
    Plan {
        nodes,
        mode,
        nmax,
        pill,
        gap: gap as u16,
        hidden,
    }
}

/// Rows the map wants: a calm spacing for a typical network (one column each side for a
/// device's own neighbourhood).
pub(super) fn want_h(cx: &Cx) -> u16 {
    match cx.size() {
        Size::L => 15,
        Size::M => 13,
        Size::S => 11,
        Size::Xs => 9,
    }
}

/// Rows the hub map's drawing takes in `band` (it sits one row under the band's top).
pub(super) fn drawn_h(cx: &Cx, band: Rect) -> u16 {
    let p = plan(cx, band, &cx.s.viewer, 3, cx.touch(), Sel::Node("", false));
    let ys = p.nodes.iter().filter(|n| n.shown).map(|n| n.label.y).chain([p.pill.y]);
    let (lo, hi) = ys.fold((u16::MAX, 0), |a, y| (a.0.min(y), a.1.max(y)));
    hi + 1 - lo
}

/// The most rows a hub map uses.
pub(super) const MAX_H: u16 = 27;

/// Device ids in selection order: the root first (launch focus, so `⏎` opens this
/// device), then hop by hop, clockwise from the top.
pub(super) fn order(cx: &Cx, band: Rect) -> Vec<String> {
    let root = cx.s.viewer.as_str();
    let p = plan(cx, band, root, 3, cx.touch(), Sel::Node("", false));
    let c = centre(p.pill);
    let clock = |r: Rect| {
        let q = centre(r);
        (((q.1 - c.1) * 2.0).atan2(q.0 - c.0) + PI / 2.0).rem_euclid(TAU)
    };
    let mut v: Vec<&Node> = p.nodes.iter().filter(|n| n.shown).collect();
    v.sort_by(|a, b| {
        (a.hop, clock(a.label))
            .partial_cmp(&(b.hop, clock(b.label)))
            .unwrap()
    });
    std::iter::once(root.to_string())
        .chain(v.into_iter().map(|n| n.d.id.clone()))
        .collect()
}

/// One row of the phone's device tree.
pub(super) struct TreeRow<'a> {
    pub d: &'a LabDevice,
    /// The link to its parent (None for the root and unlinked devices).
    pub edge: Option<&'a LabEdge>,
    /// Per ancestor level below the root: whether that ancestor has later siblings.
    pub guides: Vec<bool>,
    pub last: bool,
}

/// The spanning tree as indented rows, depth first: the root, then each relay followed
/// by the devices reached through it.
pub(super) fn tree_rows<'a>(cx: &Cx<'a>, root: &str) -> Vec<TreeRow<'a>> {
    let nodes = tree(cx, root, 3);
    let mut out = Vec::new();
    if let Some(d) = cx.s.device(root) {
        out.push(TreeRow {
            d,
            edge: None,
            guides: Vec::new(),
            last: false,
        });
    }
    fn walk<'a>(
        nodes: &[Node<'a>],
        p: Option<usize>,
        guides: Vec<bool>,
        out: &mut Vec<TreeRow<'a>>,
    ) {
        let kids: Vec<usize> = (0..nodes.len()).filter(|&k| nodes[k].parent == p).collect();
        for (j, &k) in kids.iter().enumerate() {
            let last = j + 1 == kids.len();
            out.push(TreeRow {
                d: nodes[k].d,
                edge: nodes[k].up,
                guides: guides.clone(),
                last,
            });
            let mut g = guides.clone();
            g.push(!last);
            walk(nodes, Some(k), g, out);
        }
    }
    walk(&nodes, None, Vec::new(), &mut out);
    out
}

// ------------------------------------------------------------------------ lines

const UP: u8 = 1;
const DN: u8 = 2;
const LT: u8 = 4;
const RT: u8 = 8;

/// Box-drawing line cells: per cell, the directions it joins and the links through it.
struct Grid {
    r: Rect,
    bits: Vec<u8>,
    links: Vec<Vec<usize>>,
}

impl Grid {
    fn new(r: Rect) -> Self {
        let n = usize::from(r.width) * usize::from(r.height);
        Self {
            r,
            bits: vec![0; n],
            links: vec![Vec::new(); n],
        }
    }
    fn cell(&self, x: i32, y: i32) -> Option<usize> {
        let (x, y) = (x - i32::from(self.r.x), y - i32::from(self.r.y));
        (x >= 0 && y >= 0 && x < i32::from(self.r.width) && y < i32::from(self.r.height))
            .then(|| (y * i32::from(self.r.width) + x) as usize)
    }
    /// An orthogonal polyline through `pts` (cells), for link `li`.
    fn path(&mut self, pts: &[(i32, i32)], li: usize) {
        let mut cells: Vec<(i32, i32)> = Vec::new();
        for w in pts.windows(2) {
            let (a, b) = (w[0], w[1]);
            let (sx, sy) = ((b.0 - a.0).signum(), (b.1 - a.1).signum());
            let mut q = a;
            loop {
                if cells.last() != Some(&q) {
                    cells.push(q);
                }
                if q == b {
                    break;
                }
                q = (q.0 + sx, q.1 + sy);
            }
        }
        let dir = |a: (i32, i32), b: (i32, i32)| match (b.0 - a.0, b.1 - a.1) {
            (0, -1) => UP,
            (0, 1) => DN,
            (-1, 0) => LT,
            (1, 0) => RT,
            _ => 0,
        };
        for k in 0..cells.len() {
            let c = cells[k];
            let mut b = 0;
            if k > 0 {
                b |= dir(c, cells[k - 1]);
            }
            if let Some(&n) = cells.get(k + 1) {
                b |= dir(c, n);
            }
            if cells.len() == 1 {
                b = LT | RT;
            }
            if let Some(i) = self.cell(c.0, c.1) {
                self.bits[i] |= b;
                if !self.links[i].contains(&li) {
                    self.links[i].push(li);
                }
            }
        }
    }
}

/// The line glyph for a cell's joins. ASCII: `-` `|` and `+` at every corner; a
/// password run is `.` and `:`; a quiet line is drawn plain (its ends say why).
fn line_ch(b: u8, pat: Pat, uni: bool) -> char {
    let flat = b & (UP | DN) == 0;
    let tall = b & (LT | RT) == 0;
    if uni {
        match b {
            _ if flat => {
                if pat == Pat::Dash {
                    '╌'
                } else {
                    '─'
                }
            }
            _ if tall => {
                if pat == Pat::Dash {
                    '╎'
                } else {
                    '│'
                }
            }
            x if x == DN | RT => '╭',
            x if x == DN | LT => '╮',
            x if x == UP | RT => '╰',
            x if x == UP | LT => '╯',
            x if x == UP | DN | RT => '├',
            x if x == UP | DN | LT => '┤',
            x if x == LT | RT | DN => '┬',
            x if x == LT | RT | UP => '┴',
            _ => '┼',
        }
    } else {
        match (pat, flat, tall) {
            (Pat::Dash, true, _) => '.',
            (Pat::Dash, _, true) => ':',
            (_, true, _) => '-',
            (_, _, true) => '|',
            _ => '+',
        }
    }
}

/// A heavy line glyph (the edge editor's edited link). ASCII: `=` and `#` would read
/// as charts, so the plain glyph stays and the accent carries it.
fn heavy(b: u8, uni: bool) -> char {
    if !uni {
        return line_ch(b, Pat::Solid, false);
    }
    match b {
        _ if b & (UP | DN) == 0 => '━',
        _ if b & (LT | RT) == 0 => '┃',
        x if x == DN | RT => '┏',
        x if x == DN | LT => '┓',
        x if x == UP | RT => '┗',
        x if x == UP | LT => '┛',
        x if x == UP | DN | RT => '┣',
        x if x == UP | DN | LT => '┫',
        x if x == LT | RT | DN => '┳',
        x if x == LT | RT | UP => '┻',
        _ => '╋',
    }
}

fn centre(r: Rect) -> (f32, f32) {
    (
        f32::from(r.x) + f32::from(r.width) / 2.0,
        f32::from(r.y) + 0.5,
    )
}

// ------------------------------------------------------------------------- draw

/// Draw the network seen from `root` (columns up to `depth` hops) into `band`. Phones
/// and mini-maps pass `keys_only`: keycaps alone, the names live in the tree below.
pub(super) fn draw(cx: &Cx, buf: &mut Buffer, band: Rect, root: &str, depth: usize, sel: Sel) {
    draw_with(cx, buf, band, root, depth, sel, cx.touch());
}

struct Link<'a> {
    e: &'a LabEdge,
    lit: bool,
    /// Cells: the kid's end, and the first cell of its own branch (the parent's end).
    kid_end: (i32, i32),
    par_end: (i32, i32),
    /// Access flows from the parent toward the kid.
    toward_kid: bool,
    side: i32,
}

pub(super) fn draw_with(
    cx: &Cx,
    buf: &mut Buffer,
    band: Rect,
    root: &str,
    depth: usize,
    sel: Sel,
    keys_only: bool,
) {
    if band.width < 12 || band.height < 3 {
        return;
    }
    let p = plan(cx, band, root, depth, keys_only, sel);
    let mark = cx.hit_mark();
    let idx = |id: &str| p.nodes.iter().position(|n| n.d.id == id);
    // The selection's route back to the root lights up.
    let mut route: Vec<usize> = Vec::new();
    if let Sel::Node(id, _) = sel {
        let mut cur = idx(id);
        while let Some(i) = cur {
            route.push(i);
            cur = p.nodes[i].parent;
        }
    }
    let sel_id = match sel {
        Sel::Node(id, _) => Some(id),
        _ => None,
    };
    let fade_all = matches!(sel, Sel::Edge(_));

    // Links: each device hangs from its relay (or the root) by stub, trunk and branch.
    // A line keeps a clear cell from every label (one more where a selection's
    // brackets may sit).
    let mut grid = Grid::new(band);
    let mut edited = Grid::new(band);
    let mut links: Vec<Link> = Vec::new();
    let anchor = |i: Option<usize>| match i {
        Some(i) => p.nodes[i].label,
        None => p.pill,
    };
    let half = i32::from(p.gap) / 2;
    // A line keeps two cells from its parent's label (a selection's bracket, then air)
    // and three from its device's: the port, then the end.
    let port = |r: Rect, side: i32| {
        if side > 0 {
            i32::from(r.right()) + 2
        } else {
            i32::from(r.x) - 3
        }
    };
    let end_of = |m: &Node| {
        if m.side > 0 {
            i32::from(m.label.x) - 3
        } else {
            i32::from(m.label.right()) + 2
        }
    };
    for (k, n) in p.nodes.iter().enumerate() {
        let (Some(e), true) = (n.up, n.shown) else {
            continue;
        };
        let pa = anchor(n.parent);
        let (py, ky) = (i32::from(pa.y), i32::from(n.label.y));
        let pid = n.parent.map_or(root, |q| p.nodes[q].d.id.as_str());
        let toward_kid = if e.from == pid {
            e.forward != Access::Denied
        } else {
            e.reverse != Access::Denied
        };
        let s = n.side;
        let start = port(pa, s);
        let end = end_of(n);
        // One trunk per relay and side, a short stub out, before the nearest device's
        // branch: the orbit's arc is in the branches' lengths, not in extra trunks.
        let sibs = p
            .nodes
            .iter()
            .filter(|m| m.shown && m.up.is_some() && m.parent == n.parent && m.side == s);
        let near = sibs
            .clone()
            .map(|m| end_of(m) * s)
            .min()
            .unwrap_or(end * s)
            * s;
        let trunk = if s > 0 {
            (start + half - 2).min(near - 1).max(start)
        } else {
            (start - half + 2).max(near + 1).min(start)
        };
        let alone = sibs.count() == 1;
        let li = links.len();
        let pts = [(start, py), (trunk, py), (trunk, ky), (end, ky)];
        grid.path(&pts, li);
        if matches!(sel, Sel::Edge(id) if e.id == id) {
            edited.path(&pts, li);
        }
        let lit = match sel {
            Sel::Node(id, _) => route.contains(&k) || id == pid,
            Sel::Edge(id) => e.id == id,
            Sel::Root | Sel::Targets => n.parent.is_none(),
        };
        let par_end = if trunk == end { (end, ky) } else { (trunk + n.side, ky) };
        links.push(Link {
            e,
            lit,
            kid_end: (end, ky),
            par_end: if py == ky && alone { (start, py) } else { par_end },
            toward_kid,
            side: n.side,
        });
        // Desktop: a 3x1 spot on the branch is the link's tap target (phones tap the
        // tree rows).
        if !cx.touch() {
            let mx = (par_end.0 + end) / 2;
            cx.hit(
                Rect::new((mx - 1).max(0) as u16, ky.max(0) as u16, 3, 1),
                format!("edge:{}", e.id),
            );
        }
    }
    // One style per cell: the link's own where one link runs through it; where several
    // share it, their common hue and stroke, else neutral and solid.
    let style = |tok: Tok, lit: bool, pat: Pat| {
        // A quiet line (closed or down) stays faint even when lit: it is no route.
        let alpha: f32 = match (lit, fade_all) {
            (false, true) => 0.14,
            (true, true) => 1.0,
            _ if pat == Pat::Quiet => 0.3,
            (true, false) => 0.95,
            _ => 0.55,
        };
        // On white a pale hue vanishes: light themes deepen link hues like session
        // inks (`tiles::adapt`), so every link keeps about 3:1 on white.
        if cx.tc() && !cx.dark() && !(fade_all && !lit) && pat != Pat::Quiet {
            let c = super::mix(cx.bg0(), cx.rgb(tok), (alpha + 0.2).min(1.0));
            cx.rgb_fg(tiles::adapt(cx, c, false))
        } else {
            cx.ink(tok, alpha)
        }
    };
    // The edge editor's subject is drawn in the accent, so it reads at a glance.
    let ink = |l: &Link| {
        let tok = match (l.e.up, l.lit && fade_all) {
            (_, true) => Tok::Accent,
            (true, _) => hue_tok(cx, l.e),
            _ => Tok::Muted,
        };
        style(tok, l.lit, pattern(l.e))
    };
    for y in 0..band.height {
        for x in 0..band.width {
            let i = usize::from(y) * usize::from(band.width) + usize::from(x);
            let b = grid.bits[i];
            if b == 0 {
                continue;
            }
            // A cell shared by several links: a lit one's ink, else their common hue and
            // stroke, else neutral and solid.
            let ls: Vec<&Link> = grid.links[i].iter().map(|&k| &links[k]).collect();
            let pat0 = pattern(ls[0].e);
            let pat = if ls.iter().all(|l| pattern(l.e) == pat0) {
                pat0
            } else {
                Pat::Solid
            };
            let st = match ls.iter().find(|l| l.lit) {
                Some(l) => ink(l),
                None if ls.len() == 1 => ink(ls[0]),
                None => {
                    let h0 = hue_tok(cx, ls[0].e);
                    let same = ls.iter().all(|l| hue_tok(cx, l.e) == h0) && ls[0].e.up;
                    style(if same { h0 } else { Tok::Muted }, false, pat)
                }
            };
            let pat = match ls.iter().find(|l| l.lit) {
                Some(l) => pattern(l.e),
                None => pat,
            };
            let (bx, by) = (band.x + x, band.y + y);
            // ASCII has no faint ink: a line nothing crosses is broken (`- - -`).
            let ch = line_ch(b, pat, cx.uni());
            let gapped = !cx.uni() && pat == Pat::Quiet && (if b & (UP | DN) == 0 { bx } else { by }) % 2 == 1;
            let ch = if gapped && matches!(ch, '-' | '|') { ' ' } else { ch };
            // The edge editor's subject is the one heavy stroke, in the accent, so the
            // link being edited reads at a glance (a faded branch from its cell starts
            // a cell later).
            let lb = edited.bits[i];
            let ch = if lb != 0 { heavy(lb, cx.uni()) } else { ch };
            buf[(bx, by)].set_char(ch).set_style(st);
        }
    }
    // Ends: an arrowhead where access goes one way, at the end it reaches; `⊘` at the
    // middle of a link closed both ways (`x` in ASCII).
    for l in &links {
        let st = ink(l);
        let mut put = |at: (i32, i32), g: &str| {
            if at.0 >= 0 && at.1 >= 0 {
                cx.put(buf, at.0 as u16, at.1 as u16, 1, g, st);
            }
        };
        if l.e.up && l.e.one_way() {
            let right = (l.side > 0) == l.toward_kid;
            // A solid triangle meets its stroke; a font arrow floats in its cell.
            let g = match (right, cx.uni()) {
                (true, true) => "▶",
                (false, true) => "◀",
                (true, false) => ">",
                (false, false) => "<",
            };
            put(if l.toward_kid { l.kid_end } else { l.par_end }, g);
        } else if l.e.up && closed(l.e) && cx.uni() {
            // Mid-run, clear of both labels (ASCII breaks the line instead).
            put(((l.par_end.0 + l.kid_end.0) / 2, l.kid_end.1), "⊘");
        }
    }
    // Nodes. Letters are keys on the hub and in the send-to picker only.
    let keys = matches!(sel, Sel::Node(..) | Sel::Targets);
    for n in p.nodes.iter().filter(|n| n.shown) {
        let hot = sel_id == Some(n.d.id.as_str());
        let focus = matches!(sel, Sel::Node(_, true));
        let edge_end = match sel {
            Sel::Edge(id) => cx
                .s
                .edge(id)
                .is_some_and(|e| e.from == n.d.id || e.to == n.d.id),
            _ => false,
        };
        node(cx, buf, &p, n, hot.then_some(focus), keys, fade_all && !edge_end);
    }
    pill(
        cx,
        buf,
        &p,
        root,
        match sel {
            Sel::Node(id, f) if id == root => Some(f),
            _ => None,
        },
        keys,
        fade_all,
    );
    if p.hidden > 0 {
        let t = format!("+{} more", p.hidden);
        let w = super::width(&t, cx.uni());
        let x = band.right().saturating_sub(w + 1);
        let y = band.bottom() - 1;
        cx.put(buf, x, y, w, &t, cx.st(Tok::Muted));
        cx.hit(Rect::new(x, y, w, 1), "palette:devices");
    }
    cx.unclash(mark, matches!(sel, Sel::Node(..)));
}

/// A device's name with its hotkey letter underlined in the accent (letters are keys);
/// where letters are not keys the name is entirely plain (no letter stands out, so
/// nothing suggests a key that does something else there). Returns columns written.
#[allow(clippy::too_many_arguments)]
pub(super) fn name(
    cx: &Cx,
    buf: &mut Buffer,
    x: u16,
    y: u16,
    d: &LabDevice,
    nmax: usize,
    st: Style,
    keys: bool,
) -> u16 {
    let nm = fit(&d.name, nmax, cx.uni());
    if !keys {
        return cx.put(buf, x, y, nmax as u16 + 1, &nm, st);
    }
    let mut w = 0;
    let at = has_letter(d).filter(|&i| i + 1 < nm.chars().count() || nm == d.name);
    if at.is_none() {
        w += cx.dchip(buf, x, y, d, false) + 1;
    }
    let key = cx.st(Tok::Accent).add_modifier(Modifier::BOLD | Modifier::UNDERLINED);
    for (i, ch) in nm.chars().enumerate() {
        let s = ch.to_string();
        let cst = if at == Some(i) { st.patch(key) } else { st };
        w += cx.put(buf, x + w, y, 2, &s, cst);
    }
    w
}

/// One device: its status dot on the side its link arrives, its name; the selection is
/// a tint (brackets without tints), nothing else has a background.
fn node(cx: &Cx, buf: &mut Buffer, p: &Plan, n: &Node, pick: Option<bool>, keys: bool, fade: bool) {
    let d = n.d;
    let r = n.label;
    let up = cx.up(d);
    let bar = |f: bool| {
        if f {
            cx.st(Tok::Accent)
        } else {
            cx.ink(Tok::Muted, 0.9)
        }
    };
    if p.mode == Mode::Key {
        // Keycaps in every map, so the small orbit reads the same everywhere: a faded
        // node is a faint keycap, never a bare letter.
        let st = if !fade {
            cx.chip_style(cx.s.hue(&d.id))
        } else if cx.has_surface() {
            cx.rgb_bg(super::mix(cx.bg0(), cx.rgb(Tok::Faint), 0.55))
                .patch(cx.ink(Tok::Muted, 0.9))
        } else {
            cx.ink(Tok::Muted, 0.6)
        };
        if let Some(f) = pick {
            cx.put(buf, r.x.saturating_sub(1), r.y, 1, cx.g("▕", "["), bar(f));
            cx.put(buf, r.x + 3, r.y, 1, cx.g("▏", "]"), bar(f));
        }
        cx.put(buf, r.x, r.y, 3, &format!(" {} ", d.hotkey), st);
        if !matches!(d.status, DeviceStatus::Online | DeviceStatus::Local) && pick.is_none() {
            let (g, gst) = cx.status(d.status);
            cx.put(buf, r.x + 3, r.y, 1, g, gst);
        }
        // A faded node (edge editor) is scenery, not a target.
        if !fade {
            hit_node(cx, r, d);
        }
        return;
    }
    let outer = Rect::new(r.x.saturating_sub(1), r.y, r.width + 2, 1);
    if let Some(f) = pick {
        brackets(cx, buf, outer, bar(f));
    }
    // The viewer seen from another device is a peer here: its plain dot (`◉` marks
    // the map's root only).
    let status = if d.status == DeviceStatus::Local {
        DeviceStatus::Online
    } else {
        d.status
    };
    let (g, gst) = cx.status(status);
    let gst = if fade { cx.ink(Tok::Faint, 1.0) } else { gst };
    let nst = if let Some(f) = pick {
        cx.ink(Tok::Accent, if f { 1.0 } else { 0.7 }).add_modifier(Modifier::BOLD)
    } else if fade {
        // Muted, not faint: a faded name stays readable (faint is for lines).
        cx.ink(Tok::Muted, 0.85)
    } else if up {
        cx.ink(Tok::Fg, 1.0).add_modifier(Modifier::BOLD)
    } else {
        cx.ink(Tok::Muted, 0.75)
    };
    let (gx, nx) = if n.side < 0 {
        (r.right() - 1, r.x)
    } else {
        (r.x, r.x + 2)
    };
    cx.put(buf, gx, r.y, 1, g, gst);
    name(cx, buf, nx, r.y, d, p.nmax, nst, keys);
    if !fade {
        hit_node(cx, outer, d);
    }
}

fn name_w(cx: &Cx, d: &LabDevice, nmax: usize, keys: bool) -> u16 {
    let nm = fit(&d.name, nmax, cx.uni());
    let at = has_letter(d).filter(|&i| i + 1 < nm.chars().count() || nm == d.name);
    super::width(&nm, cx.uni()) + if at.is_none() && keys { 4 } else { 0 }
}

fn hit_node(cx: &Cx, r: Rect, d: &LabDevice) {
    // Thumb-sized on phones: at least three rows by seven columns.
    let r = if cx.touch() {
        let w = r.width.max(7);
        Rect::new(
            r.x.saturating_sub((w - r.width) / 2),
            r.y.saturating_sub(1),
            w,
            r.height + 2,
        )
    } else {
        r
    };
    cx.hit(r, format!("device:{}", d.id));
}

/// A selection where colour cannot show it: brackets in the cells either side of `r`.
fn brackets(cx: &Cx, buf: &mut Buffer, r: Rect, st: Style) {
    if !cx.uni() || !cx.c.has_color() {
        cx.put(buf, r.x, r.y, 1, "[", st);
        cx.put(buf, r.right() - 1, r.y, 1, "]", st);
    }
}

/// The root: `◉` and its name in bold (hotkey underlined), no background. Selected, it
/// takes the brackets every selected label has.
fn pill(cx: &Cx, buf: &mut Buffer, p: &Plan, root: &str, pick: Option<bool>, keys: bool, fade: bool) {
    let Some(d) = cx.s.device(root) else { return };
    let r = p.pill;
    let bar = |f: bool| {
        if f {
            cx.st(Tok::Accent)
        } else {
            cx.ink(Tok::Muted, 0.9)
        }
    };
    if p.mode == Mode::Key {
        // `◉` and its keycap; bars only when selected, as on every keycap node.
        cx.put(buf, r.x, r.y, 1, cx.g("◉", "@"), cx.st(Tok::Accent));
        cx.put(buf, r.x + 2, r.y, 3, &format!(" {} ", d.hotkey), cx.chip_style(cx.s.hue(&d.id)));
        if let Some(f) = pick {
            cx.put(buf, r.x.saturating_sub(1), r.y, 1, cx.g("▕", "["), bar(f));
            cx.put(buf, r.right(), r.y, 1, cx.g("▏", "]"), bar(f));
        }
    } else {
        // `◉` marks the root on every map unless its status has something to say.
        let (g, gst) = cx.status(d.status);
        let g = if matches!(d.status, DeviceStatus::Online | DeviceStatus::Local) {
            cx.g("◉", "@")
        } else {
            g
        };
        cx.put(buf, r.x, r.y, 1, g, gst);
        let st = if let Some(f) = pick {
            cx.ink(Tok::Accent, if f { 1.0 } else { 0.7 })
        } else {
            cx.ink(Tok::Fg, if fade { 0.85 } else { 1.0 })
        };
        name(cx, buf, r.x + 2, r.y, d, 16, st.add_modifier(Modifier::BOLD), keys);
        if let Some(f) = pick {
            brackets(cx, buf, Rect::new(r.x.saturating_sub(1), r.y, r.width + 2, 1), bar(f));
        }
    }
    let hit = if cx.touch() {
        let w = r.width.max(7);
        Rect::new(
            r.x.saturating_sub((w - r.width) / 2),
            r.y.saturating_sub(1),
            w,
            3,
        )
    } else {
        r
    };
    cx.hit(hit, format!("device:{}", d.id));
}
