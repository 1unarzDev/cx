//! The network as a quiet diagram: the root device (this one on the hub) sits in the
//! middle in its capsule, the devices it reaches sit in a column either side of it and
//! devices reached through them in the next column out, each subtree beside its relay.
//! Links are thin box-drawing lines of one weight with rounded corners: a short stub
//! leaves the root, a trunk runs along the column, and a branch enters each label with a
//! clear cell before it. Hue means network (mesh hue, teal for lan, neutral for direct
//! ssh; a line shared by several links is neutral); the selection's route is brighter.
//! The stroke is the access policy, used sparingly: solid where a key is allowed, dashed
//! where only a password is, an arrowhead at the end access reaches where it goes one
//! way, a faint line where nothing crosses (`⊘` at its inner end when closed, the device's
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
            cx.g("✕", "x").into()
        } else {
            format!("{} down", cx.g("✕", "x"))
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
    if depth >= 3 {
        for d in s.devices.iter().filter(|d| !seen.contains(&d.id.as_str())) {
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

/// One cell each side, the status glyph, a space and the name.
fn pill_w(cx: &Cx, d: &LabDevice, mode: Mode, keys: bool) -> u16 {
    match mode {
        Mode::Key => 5,
        Mode::Name => 4 + name_w(cx, d, 16, keys),
    }
}

fn plan<'a>(cx: &Cx<'a>, band: Rect, root: &str, depth: usize, keys_only: bool, sel: Sel) -> Plan<'a> {
    let keys = matches!(sel, Sel::Node(..) | Sel::Targets);
    let modes: &[(Mode, usize)] = if keys_only {
        &[(Mode::Key, 0)]
    } else {
        &[(Mode::Name, 16), (Mode::Name, 12), (Mode::Name, 9), (Mode::Key, 0)]
    };
    // Every named mode with a clear row between labels first, then packed: names
    // packed closer beat bare keycaps. (An even spacing keeps two peers symmetric
    // about their relay.)
    // Big screens try a wider spacing first (still even); columns draw closer (down
    // to the shortest stub that keeps a corner) before a name is cut.
    let spacings: &[i32] = if cx.size() == Size::L { &[4, 2, 1] } else { &[2, 1] };
    let full: u16 = match cx.size() {
        Size::L => 12,
        Size::M => 10,
        Size::S => 8,
        Size::Xs => 6,
    };
    let mut best = None;
    for &sp in spacings {
        for &(mode, nmax) in modes {
            if sp > 1 && mode == Mode::Key && !keys_only {
                continue;
            }
            for gap in (full.min(6)..=full).rev() {
                let p = place(cx, band, tree(cx, root, depth), root, mode, nmax, sp, keys, gap);
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

/// Lay the tree out as two mirrored columns sets: the root's subtrees split between its
/// right and left so both sides hold about as many rows, every device one row (two with
/// with air), a relay on the middle row of its children, the columns of one hop aligned.
/// Labels that leave the band are hidden (counted in `hidden`).
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
    // Rows: leaves one after another, a relay on its children's middle row.
    let wv: Vec<i32> = nodes
        .iter()
        .map(|nd| i32::from(label_w(cx, nd.d, mode, nmax, keys)))
        .collect();
    let mut row = vec![0i32; n];
    fn rows(i: usize, kids: &[Vec<usize>], next: &mut i32, sp: i32, row: &mut [i32]) {
        if kids[i].is_empty() {
            row[i] = *next;
            *next += sp;
        } else {
            for &k in &kids[i] {
                rows(k, kids, next, sp, row);
            }
            let (a, b) = (row[kids[i][0]], row[*kids[i].last().expect("kids")]);
            row[i] = (a + b).div_euclid(2);
        }
    }
    let rootd = cx.s.device(root);
    let pw = rootd.map_or(5, |d| pill_w(cx, d, mode, keys));
    let c = (
        i32::from(band.x) + i32::from(band.width) / 2,
        i32::from(band.y) + i32::from(band.height) / 2,
    );
    let pill = (c.0 - i32::from(pw) / 2, c.1);
    // Positions first (x, y, width), then the whole drawing moves into the band.
    let mut pos = vec![(0i32, 0i32, 0i32); n];
    for (side, part) in [(1, &tops[..right]), (-1, &tops[right..])] {
        let mut next = 0;
        for &i in part {
            rows(i, &kids, &mut next, sp, &mut row);
        }
        let h = (next - sp + 1).max(1);
        // Every device of this side, its subtrees included.
        let mut all: Vec<usize> = part.to_vec();
        let mut k = 0;
        while k < all.len() {
            all.extend(kids[all[k]].iter().copied());
            k += 1;
        }
        let hops = all.iter().map(|&i| nodes[i].hop).max().unwrap_or(1);
        let mut x = if side > 0 {
            pill.0 + i32::from(pw) + i32::from(gap)
        } else {
            pill.0 - i32::from(gap)
        };
        // Columns per hop, as wide as their widest label.
        for hop in 1..=hops {
            let col: Vec<usize> = all.iter().copied().filter(|&i| nodes[i].hop == hop).collect();
            let col_w = col.iter().map(|&i| wv[i]).max().unwrap_or(0);
            for i in col {
                let lx = if side > 0 { x } else { x - wv[i] };
                pos[i] = (lx, c.1 + row[i] - (h - 1) / 2, wv[i]);
                nodes[i].side = side;
            }
            x += side * (col_w + i32::from(gap));
        }
    }
    // Centre the drawing, not the root (one side often reaches further), and sit it
    // one row under the band's top: the rows it does not need are all at its foot,
    // where the caller gives them to its content. Too wide, it keeps its left edge.
    let (lo, hi, top, bot) = pos
        .iter()
        .copied()
        .chain([(pill.0, pill.1, i32::from(pw))])
        .fold((i32::MAX, i32::MIN, i32::MAX, i32::MIN), |a, (x, y, w)| {
            (a.0.min(x), a.1.max(x + w), a.2.min(y), a.3.max(y + 1))
        });
    let (bx0, bx1) = (i32::from(band.x), i32::from(band.right()));
    let (by0, by1) = (i32::from(band.y), i32::from(band.bottom()));
    let dx = ((bx0 + bx1 - lo - hi) / 2).min(bx1 - 1 - hi).max(bx0 + 1 - lo);
    let dy = ((by0 + by1 - top - bot) / 2)
        .min(by1 - bot)
        .max(by0 - top)
        .min(by0 + 1 - top);
    for (i, &(x, y, w)) in pos.iter().enumerate() {
        let (x, y) = (x + dx, y + dy);
        nodes[i].label = Rect::new(x.max(0) as u16, y.max(0) as u16, w as u16, 1);
        // (A clear column at either edge: nothing butts into the screen's side.)
        nodes[i].shown = x > bx0 && x + w < bx1 && y >= by0 && y < by1;
    }
    // A device whose relay is hidden hides too (it would hang from nothing).
    for i in 0..n {
        if nodes[i].parent.is_some_and(|p| !nodes[p].shown) {
            nodes[i].shown = false;
        }
    }
    let pill = Rect::new((pill.0 + dx).max(0) as u16, (pill.1 + dy).max(0) as u16, pw, 1);
    let hidden = nodes.iter().filter(|n| !n.shown).count();
    Plan {
        nodes,
        mode,
        nmax,
        pill,
        gap,
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
/// password run is `=` (unbroken); quiet lines are `.` and `:`.
fn line_ch(b: u8, pat: Pat, uni: bool) -> char {
    let flat = b & (UP | DN) == 0;
    let tall = b & (LT | RT) == 0;
    if uni {
        match b {
            _ if flat => {
                if pat == Pat::Dash {
                    '┄'
                } else {
                    '─'
                }
            }
            _ if tall => {
                if pat == Pat::Dash {
                    '┆'
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
            (Pat::Quiet, true, _) => '.',
            (Pat::Quiet, _, true) => ':',
            (Pat::Dash, true, _) => '=',
            (_, true, _) => '-',
            (_, _, true) => '|',
            _ => '+',
        }
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
    let mut links: Vec<Link> = Vec::new();
    let anchor = |i: Option<usize>| match i {
        Some(i) => p.nodes[i].label,
        None => p.pill,
    };
    let half = i32::from(p.gap) / 2;
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
        // An arrowhead into the label keeps one more clear cell before it, where the
        // columns are far enough apart.
        let clear = 3 + i32::from(e.up && e.one_way() && toward_kid && half >= 4);
        let (start, end, trunk) = if n.side > 0 {
            let end = i32::from(n.label.x) - clear;
            (i32::from(pa.right()) + 2, end, end - half + clear - 1)
        } else {
            let end = i32::from(n.label.right()) + clear - 1;
            (i32::from(pa.x) - 3, end, end + half - clear + 1)
        };
        // Close columns: the stub starts a cell nearer its relay, so it keeps a run.
        let start = if n.side > 0 {
            start.min(trunk - 1)
        } else {
            start.max(trunk + 1)
        };
        let trunk = if n.side > 0 {
            trunk.clamp(start.min(end), end)
        } else {
            trunk.clamp(end, start.max(end))
        };
        let li = links.len();
        grid.path(&[(start, py), (trunk, py), (trunk, ky), (end, ky)], li);
        let lit = match sel {
            Sel::Node(id, _) => route.contains(&k) || id == pid,
            Sel::Edge(id) => e.id == id,
            Sel::Root | Sel::Targets => n.parent.is_none(),
        };
        let alone = p
            .nodes
            .iter()
            .filter(|m| m.shown && m.up.is_some() && m.parent == n.parent && m.side == n.side)
            .count()
            == 1;
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
            (false, true) => 0.2,
            (true, true) => 0.95,
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
            buf[(bx, by)].set_char(line_ch(b, pat, cx.uni())).set_style(st);
        }
    }
    // Ends: an arrowhead where access goes one way, at the end it reaches; `⊘` at the
    // inner end of a link closed both ways (`/` in ASCII).
    for l in &links {
        let st = ink(l);
        let mut put = |at: (i32, i32), g: &str| {
            if at.0 >= 0 && at.1 >= 0 {
                cx.put(buf, at.0 as u16, at.1 as u16, 1, g, st);
            }
        };
        if l.e.up && l.e.one_way() {
            let right = (l.side > 0) == l.toward_kid;
            let g = match (right, cx.uni()) {
                (true, true) => "→",
                (false, true) => "←",
                (true, false) => ">",
                (false, false) => "<",
            };
            put(if l.toward_kid { l.kid_end } else { l.par_end }, g);
        } else if l.e.up && closed(l.e) {
            // At the inner end, away from the device's own status dot.
            put(l.par_end, cx.g("⊘", "/"));
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
    // On a solid capsule (no tints) the letter keeps the capsule's ink.
    let key = if st.bg.is_some() && !cx.has_surface() {
        st.add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
    } else {
        cx.st(Tok::Accent)
            .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
    };
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
    let bg = match pick {
        Some(true) if cx.has_surface() => cx.st(Tok::AccentBg),
        Some(false) if cx.has_surface() => cx.st(Tok::SelectDimBg),
        _ => Style::default(),
    };
    let outer = Rect::new(r.x.saturating_sub(1), r.y, r.width + 2, 1);
    if let Some(f) = pick {
        if cx.has_surface() {
            cx.fill(buf, outer, bg);
        } else {
            cx.put(buf, outer.x, r.y, 1, "[", bar(f));
            cx.put(buf, r.right(), r.y, 1, "]", bar(f));
        }
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
    let nst = if pick == Some(true) {
        cx.st(Tok::Accent).add_modifier(Modifier::BOLD)
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
    cx.put(buf, gx, r.y, 1, g, gst.patch(bg));
    name(cx, buf, nx, r.y, d, p.nmax, nst.patch(bg), keys);
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

/// The capsule that marks the root (hub, device star, onboarding): an accent tint, a
/// solid accent row in 16 colours, reversed without colour; in ASCII its padding cells
/// are `(` `)`. Returns the body style.
pub(super) fn capsule(cx: &Cx, buf: &mut Buffer, r: Rect, strength: f32) -> Style {
    let body = if cx.has_surface() {
        cx.rgb_bg(super::mix(cx.bg0(), cx.rgb(Tok::Accent), strength))
    } else if cx.c.has_color() {
        Style::default()
            .bg(cx.st(Tok::Accent).fg.unwrap_or_default())
            .fg(ratatui::style::Color::Black)
    } else {
        Style::default().add_modifier(Modifier::REVERSED)
    };
    cx.fill(buf, r, body);
    if !cx.uni() {
        cx.put(buf, r.x, r.y, 1, "(", body);
        cx.put(buf, r.right() - 1, r.y, 1, ")", body);
    }
    body
}

/// The root: a tinted capsule with `◉` and its name (hotkey underlined). Selected, the
/// tint deepens (brackets where there are no tints: in ASCII they replace the capsule's
/// parentheses).
fn pill(cx: &Cx, buf: &mut Buffer, p: &Plan, root: &str, pick: Option<bool>, keys: bool, fade: bool) {
    let Some(d) = cx.s.device(root) else { return };
    let r = p.pill;
    let strength = match (pick, cx.dark()) {
        _ if fade => 0.1,
        (Some(true), true) => 0.42,
        (Some(true), false) => 0.32,
        (_, true) => 0.26,
        (_, false) => 0.18,
    };
    // Keycap maps draw the root as its keycap between brackets: tinted shoulders
    // beside a keycap read as a render artefact at that size.
    let key_root = p.mode == Mode::Key && cx.has_surface();
    let body = if key_root {
        Style::default()
    } else {
        capsule(cx, buf, r, strength)
    };
    let pick = if key_root { pick.or(Some(false)) } else { pick };
    if p.mode == Mode::Key {
        cx.put(
            buf,
            r.x + 1,
            r.y,
            3,
            &format!(" {} ", d.hotkey),
            cx.chip_style(cx.s.hue(&d.id)),
        );
    } else {
        // `◉` marks the root on every map unless its status has something to say.
        let (g, gst) = cx.status(d.status);
        let g = if matches!(d.status, DeviceStatus::Online | DeviceStatus::Local) {
            cx.g("◉", "@")
        } else {
            g
        };
        let gst = if cx.has_surface() { gst.patch(body) } else { body };
        cx.put(buf, r.x + 1, r.y, 1, g, gst);
        let st = if cx.has_surface() {
            cx.ink(Tok::Fg, 1.0).patch(body).add_modifier(Modifier::BOLD)
        } else {
            body.add_modifier(Modifier::BOLD)
        };
        name(cx, buf, r.x + 3, r.y, d, 16, st, keys);
    }
    if let Some(f) = pick.filter(|_| key_root || !cx.has_surface()) {
        let bar = if f {
            cx.st(Tok::Accent)
        } else {
            cx.ink(Tok::Muted, 0.9)
        };
        if cx.uni() {
            cx.put(buf, r.x.saturating_sub(1), r.y, 1, "▕", bar);
            cx.put(buf, r.right(), r.y, 1, "▏", bar);
        } else {
            cx.put(buf, r.x, r.y, 1, "[", body);
            cx.put(buf, r.right() - 1, r.y, 1, "]", body);
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
