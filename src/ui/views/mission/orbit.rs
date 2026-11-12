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
use crate::ui::views::{Access, DeviceStatus, EdgeKind, LabDevice, LabEdge, Tok};
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
    /// The orbit's line from the parent's port to this device (empty: the column
    /// layout's stub, trunk and branch).
    pts: Vec<(i32, i32)>,
}

struct Plan<'a> {
    nodes: Vec<Node<'a>>,
    mode: Mode,
    nmax: usize,
    pill: Rect,
    /// Rows between a device and the next.
    sp: i32,
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
            cx.g("✕", "!").into()
        } else {
            format!("{} down", cx.g("✕", "!"))
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
        pts: Vec::new(),
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
            EdgeKind::Jump(via)
                if (e.from == root && e.to == d.id) || (e.to == root && e.from == d.id) =>
            {
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
        Mode::Key => {
            3 + u16::from(!matches!(
                d.status,
                DeviceStatus::Online | DeviceStatus::Local
            ))
        }
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

fn plan<'a>(
    cx: &Cx<'a>,
    band: Rect,
    root: &str,
    depth: usize,
    keys_only: bool,
    sel: Sel,
) -> Plan<'a> {
    let keys = matches!(sel, Sel::Node(..) | Sel::Targets);
    // The ladder, richest first: the drawing takes the most air its area allows (rows
    // three apart only with whole names, then two, then one), then shorter names, then
    // bare keycaps. Within each, the orbit's arc before a flat column, and the widest
    // column gap down to the shortest that keeps a corner.
    // One spacing scale at every size and in every view: the drawing is centred in its
    // area, never stretched to fill it.
    // The edge editor's map is the screen's subject: one step up the same scale.
    let sp_max = match cx.size() {
        Size::L if matches!(sel, Sel::Edge(_)) => 6,
        Size::M if matches!(sel, Sel::Edge(_)) => 4,
        Size::L => 4,
        Size::M => 3,
        Size::S | Size::Xs => 2,
    };
    // Rows three apart suit a small network only; a dense one at that spacing reads
    // as a stretched wireframe.
    let sp_max = if tree(cx, root, depth).len() > 8 {
        sp_max.min(2)
    } else {
        sp_max
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
    // A small network is an orbit, sized to its area; a dense one falls back to columns.
    if tree(cx, root, depth).len() <= 8 {
        let modes: &[(Mode, usize)] = if keys_only {
            &[(Mode::Key, 0)]
        } else {
            &[
                (Mode::Name, 16),
                (Mode::Name, 12),
                (Mode::Name, 9),
                (Mode::Key, 0),
            ]
        };
        // The drawing takes up to about two thirds of a wide band (all but the gutters
        // of a narrow one); rings stop growing where they would only add air.
        let w = i32::from(band.width);
        let lim = if w >= 100 { w * 7 / 10 } else { w - 8 };
        let edge = matches!(sel, Sel::Edge(_));
        // (The edge editor's map is the screen's subject: one step up.)
        let cap = match (depth, cx.size()) {
            (_, Size::L) if edge => 9,
            (_, Size::M) if edge => 6,
            (1, Size::L) => 6,
            (1, _) => 4,
            (_, Size::L) => 7,
            (_, Size::M) => 5,
            _ => 4,
        };
        for &(mode, nmax) in modes {
            for ry in (2..=cap).rev() {
                // About round (a cell is twice as tall as wide), wider where the band is.
                for rx in (2 * ry..=4 * ry).rev().step_by(2) {
                    let nodes = tree(cx, root, depth);
                    if let Some(p) =
                        radial(cx, band, nodes, root, mode, nmax, keys, ry, rx, lim, edge)
                    {
                        return p;
                    }
                }
            }
        }
    }
    let mut best = None;
    for (sp, mode, nmax) in ladder {
        // Columns between a label and the next follow the rows (a cell is about twice
        // as tall as wide), so arms across and arms down read as one length.
        let full = (3 * sp as u16 + 4).clamp(8, 22);
        for arc in [true, false] {
            for gap in (4..=full).rev() {
                // The edge editor's subject keeps two rows of stroke at the least.
                let arm = if matches!(sel, Sel::Edge(_)) { 3 } else { 2 };
                let p = place(
                    cx,
                    band,
                    tree(cx, root, depth),
                    root,
                    mode,
                    nmax,
                    sp,
                    keys,
                    gap,
                    arc,
                    arm,
                );
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

/// Lay the network out as an orbit around the root, mirror-balanced: in a small network
/// the heaviest subtree whose link reads both ways sits straight above the root (the
/// next below it), its devices spreading out on wings a row further out, left and right;
/// the other peers sit in a column either side of the root (both sides about as many
/// rows), symmetric about its row, those nearer the row further out, so their labels
/// trace an arc, and a relay's devices fan outward from it. Every device owns its row on
/// its side, so labels and lines never cross. The root sits at the band's centre when
/// the drawing allows, else the drawing does. Labels that leave the band are hidden.
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
    arm: i32,
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
    // Above and below the root: an arrowhead reads along a row, so only links that are
    // not one way; only in a small network (a dense one needs the columns' rows).
    let mut vert: Vec<usize> = Vec::new();
    // (An even count only: three peers as a cross leave one arm empty.)
    if (4..=6).contains(&tops.len()) {
        let mut c: Vec<usize> = tops
            .iter()
            .copied()
            .filter(|&i| nodes[i].up.is_some_and(|e| !(e.up && e.one_way())))
            .collect();
        c.sort_by_key(|&i| -leaves[i]);
        c.truncate(2);
        vert = c;
    }
    let side_tops: Vec<usize> = tops.iter().copied().filter(|i| !vert.contains(i)).collect();
    // The split that evens the sides' rows (the right one takes a tie: rows are longer
    // to read left to right).
    let total: i32 = side_tops.iter().map(|&i| leaves[i]).sum();
    let mut right = side_tops.len();
    let mut acc = 0;
    let mut best = i32::MAX;
    for (k, &i) in side_tops.iter().enumerate() {
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
    let mut reach = 0;
    let mut lifts = vec![0i32; n];
    for (side, part) in [(1, &side_tops[..right]), (-1, &side_tops[right..])] {
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
        let down: Vec<usize> = part
            .iter()
            .copied()
            .filter(|&i| Some(i) != mid && !is_up[i])
            .collect();
        let mut below = sp;
        let mut lo = 0;
        if let Some(c) = mid {
            lo = fan(c, 0, 1, sp, &kids, &mut row);
        }
        for &k in &down {
            lo = fan(k, below, 1, sp, &kids, &mut row);
            below = lo + sp;
        }
        let mut above = -sp;
        let mut hi = 0;
        for &k in up.iter().rev() {
            hi = fan(k, above, -1, sp, &kids, &mut row);
            above = hi - sp;
        }
        reach = reach.max(lo).max(-hi);
        // The orbit: peers nearer the root's row sit further out, on an ellipse as
        // wide as the side is tall (cells are about twice as tall as wide).
        let r = part.iter().map(|&i| row[i].abs()).max().unwrap_or(0);
        for &i in part {
            nodes[i].side = side;
            if arc && r > 0 {
                let t = row[i].abs() as f32 / r as f32;
                lifts[i] = (r.min(10) as f32 * (1.0 - t * t).sqrt()).round() as i32;
            }
        }
    }
    // Straight above (then below) the root, clear of the columns; its devices on its
    // own row, split left and right, so one straight rule runs through it.
    for (k, &v) in vert.iter().enumerate() {
        let dir = if k == 0 { -1 } else { 1 };
        row[v] = dir * (reach + sp.max(arm));
        nodes[v].side = 0;
        let ks = &kids[v];
        let half = ks.len() / 2;
        for (side, part) in [(-1, &ks[..half]), (1, &ks[half..])] {
            let mut at = row[v];
            for &c in part {
                nodes[c].side = side;
                at = fan(c, at, dir, sp, &kids, &mut row) + dir * sp;
            }
        }
    }
    // Outward, hop by hop (a relay comes before the devices reached through it): a
    // device's column follows its relay's label, and takes its relay's side.
    for i in 0..n {
        let side = match nodes[i].parent {
            Some(p) if nodes[p].side != 0 => nodes[p].side,
            _ => nodes[i].side,
        };
        nodes[i].side = side;
        x[i] = match (nodes[i].parent, side) {
            (None, 0) => (pw - wv[i]) / 2,
            (None, s) if s > 0 => pw + gap + lifts[i],
            (None, _) => -gap - lifts[i] - wv[i],
            (Some(p), s) if s > 0 => x[p] + wv[p] + gap,
            (Some(p), _) => x[p] - gap - wv[i],
        };
    }
    // The root at the centre of the band where the drawing fits around it, else the
    // drawing centred, a two-column gutter either side; too wide, it keeps its left
    // gutter. Rows: the drawing centred.
    let (lo, hi, top, bot) = (0..n)
        .map(|i| (x[i], row[i], wv[i]))
        .chain([(0, 0, pw)])
        .fold((i32::MAX, i32::MIN, i32::MAX, i32::MIN), |a, (x, y, w)| {
            (a.0.min(x), a.1.max(x + w), a.2.min(y), a.3.max(y + 1))
        });
    let g = 2;
    let (bx0, bx1) = (i32::from(band.x) + g, i32::from(band.right()) - g);
    let (by0, by1) = (i32::from(band.y), i32::from(band.bottom()));
    let on_root = (bx0 + bx1 - pw) / 2;
    let dx = if on_root + lo >= bx0 && on_root + hi <= bx1 {
        on_root
    } else {
        ((bx0 + bx1 - lo - hi) / 2).min(bx1 - hi).max(bx0 - lo)
    };
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
        sp,
        gap: gap as u16,
        hidden,
    }
}

/// The orbit: the root at the centre, each subtree given a share of the circle by its
/// devices (a radial tree), its device in the middle of its share on ring one, the
/// devices reached through it further out on ring two. The heaviest relay sits straight
/// above the root and the rest split between the two sides by weight, so the drawing is
/// mirror-balanced. A line leaves its parent along the parent's row and turns once,
/// with a rounded corner, straight into the device's dot. `None` when labels or lines
/// would collide or leave the band (the caller tries a smaller orbit).
#[allow(clippy::too_many_arguments)]
fn radial<'a>(
    cx: &Cx<'a>,
    band: Rect,
    mut nodes: Vec<Node<'a>>,
    root: &str,
    mode: Mode,
    nmax: usize,
    keys: bool,
    ry: i32,
    rx: i32,
    lim: i32,
    edge: bool,
) -> Option<Plan<'a>> {
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
    let mut tops: Vec<usize> = (0..n).filter(|&i| nodes[i].parent.is_none()).collect();
    if tops.is_empty() {
        return None;
    }
    // On the axis: the heaviest relay (an arrowhead reads along a row, so not a one-way
    // link), or the first device when the count is odd; the rest by weight to the
    // lighter side, the right first, each side ordered away from the top.
    tops.sort_by_key(|&i| (-leaves[i], nodes[i].up.is_some_and(|e| e.one_way())));
    let relay = leaves[tops[0]] > 1;
    let anchor = (relay || tops.len() % 2 == 1).then(|| tops.remove(0));
    // (One-way links pair up first, so the one left over for the bottom of the axis
    // reads both ways.)
    tops.sort_by_key(|&i| (-leaves[i], !nodes[i].up.is_some_and(|e| e.one_way())));
    let (mut rs, mut ls, mut wr, mut wl) = (Vec::new(), Vec::new(), 0, 0);
    for i in tops {
        if wr <= wl {
            wr += leaves[i];
            rs.push(i);
        } else {
            wl += leaves[i];
            ls.push(i);
        }
    }
    let ring: Vec<usize> = anchor
        .into_iter()
        .chain(rs)
        .chain(ls.into_iter().rev())
        .collect();
    let total: i32 = ring.iter().map(|&i| leaves[i]).sum();
    let unit = TAU / total as f32;
    // Clockwise from the top: an axis device's share is centred on it, else the shares
    // meet there.
    let mut at = -anchor.map_or(0.0, |a| leaves[a] as f32 * unit / 2.0);
    let mut theta = vec![0f32; n];
    fn spread(i: usize, a: f32, unit: f32, kids: &[Vec<usize>], leaves: &[i32], theta: &mut [f32]) {
        theta[i] = a + leaves[i] as f32 * unit / 2.0;
        let mut b = a;
        for &k in &kids[i] {
            spread(k, b, unit, kids, leaves, theta);
            b += leaves[k] as f32 * unit;
        }
    }
    for &i in &ring {
        spread(i, at, unit, &kids, &leaves, &mut theta);
        at += leaves[i] as f32 * unit;
    }
    let wv: Vec<i32> = nodes
        .iter()
        .map(|nd| i32::from(label_w(cx, nd.d, mode, nmax, keys)))
        .collect();
    let pw = i32::from(cx.s.device(root).map_or(5, |d| pill_w(cx, d, mode, keys)));
    let pill = Rect::new(0, 0, pw as u16, 1);
    // Root-relative cells, shifted into the band at the end (labels as signed x, y, w).
    let mut lab: Vec<(i32, i32, i32)> = vec![(0, 0, 0); n];
    let root_lab = (-pw / 2, 0, pw);
    let port = |l: (i32, i32, i32), s: i32| if s > 0 { l.0 + l.2 + 2 } else { l.0 - 3 };
    let order: Vec<usize> = {
        let mut v: Vec<usize> = (0..n).collect();
        v.sort_by_key(|&i| nodes[i].hop);
        v
    };
    for &i in &order {
        let ring_f = [1.0, 1.75, 2.4][usize::from(nodes[i].hop.clamp(1, 3)) - 1];
        let (sn, cs) = theta[i].sin_cos();
        let par = nodes[i].parent;
        let pl = par.map_or(root_lab, |p| lab[p]);
        let pside = par.map_or(0, |p| nodes[p].side);
        let side = if pside == 0 && sn.abs() < 0.15 {
            0
        } else if pside != 0 {
            pside
        } else if sn > 0.0 {
            1
        } else {
            -1
        };
        nodes[i].side = side;
        // (Rounded the same way either side of the axis, so mirrored devices match.)
        let snap = |v: f32| v.signum() * (v.abs() + 1e-3).round();
        // (The ellipse is measured from the root's label ends, not its middle.)
        let mut px = snap(rx as f32 * ring_f * sn) as i32 + snap(sn) as i32 * (pw / 2);
        let mut py = snap(-(ry as f32) * ring_f * cs) as i32;
        // A row apart reads as a kink: level with the parent on a small orbit, else two
        // rows from it.
        let dy = py - pl.1;
        if dy.abs() == 1 {
            py = if ry <= 3 { pl.1 } else { pl.1 + 2 * dy };
        }
        // The edited link keeps two rows of stroke where the orbit has room.
        if edge && side == 0 && ry >= 3 && (py - pl.1).abs() < 4 {
            py = pl.1 + 4 * (py - pl.1).signum();
        }
        let w = wv[i];
        let x = match side {
            0 => {
                px = pl.0 + pl.2 / 2;
                px - w / 2
            }
            s => {
                // Clear of the parent: three cells of line at the least (five when the
                // line runs straight in, as the end keeps two cells of air).
                let min = port(pl, s) + s * if py == pl.1 { 5 } else { 2 };
                px = if s > 0 { px.max(min) } else { px.min(min) };
                // A keycap takes its line on its letter.
                if mode == Mode::Key {
                    px - 1
                } else if s > 0 {
                    px
                } else {
                    px - w + 1
                }
            }
        };
        lab[i] = (x, py, w);
        let start = port(pl, side);
        nodes[i].pts = match side {
            0 => {
                let dir = (py - pl.1).signum();
                vec![(px, pl.1 + dir), (px, py - dir)]
            }
            s if py == pl.1 => vec![(start, py), (if s > 0 { x - 3 } else { x + w + 2 }, py)],
            _ => vec![(start, pl.1), (px, pl.1), (px, py - (py - pl.1).signum())],
        };
    }
    // Labels keep a cell either side for a selection's brackets, so two never touch.
    let boxes: Vec<(i32, i32, i32)> = lab.iter().copied().chain([root_lab]).collect();
    let hit = |b: &(i32, i32, i32), x: i32, y: i32| y == b.1 && x >= b.0 - 1 && x <= b.0 + b.2;
    for a in 0..boxes.len() {
        for b in a + 1..boxes.len() {
            let (p, q) = (boxes[a], boxes[b]);
            if p.1 == q.1 && p.0 - 2 < q.0 + q.2 && q.0 - 2 < p.0 + p.2 {
                return None;
            }
        }
    }
    // Lines cross no label, and links from different parents share no cell.
    let mut owner: std::collections::HashMap<(i32, i32), Option<usize>> = Default::default();
    for node in nodes.iter().take(n) {
        let pts = &node.pts;
        if pts.is_empty() || node.up.is_none() {
            continue;
        }
        for w in pts.windows(2) {
            let (a, b) = (w[0], w[1]);
            if (b.0 - a.0) * (b.1 - a.1) != 0 {
                return None;
            }
            let (sx, sy) = ((b.0 - a.0).signum(), (b.1 - a.1).signum());
            let mut q = a;
            loop {
                if boxes.iter().any(|bx| hit(bx, q.0, q.1)) {
                    return None;
                }
                let o = owner.entry(q).or_insert(node.parent);
                if *o != node.parent {
                    return None;
                }
                if q == b {
                    break;
                }
                q = (q.0 + sx, q.1 + sy);
            }
        }
    }
    // Centred in the band, a four-column gutter either side.
    let (lo, hi, top, bot) = boxes
        .iter()
        .fold((i32::MAX, i32::MIN, i32::MAX, i32::MIN), |a, b| {
            (
                a.0.min(b.0),
                a.1.max(b.0 + b.2),
                a.2.min(b.1),
                a.3.max(b.1 + 1),
            )
        });
    let g = 4;
    if hi - lo > lim.min(i32::from(band.width) - 2 * g) || bot - top > i32::from(band.height) {
        return None;
    }
    let dx = i32::from(band.x) + (i32::from(band.width) - (hi - lo)) / 2 - lo;
    let dy = i32::from(band.y) + (i32::from(band.height) - (bot - top)) / 2 - top;
    for i in 0..n {
        let (x, y, w) = lab[i];
        nodes[i].label = Rect::new((x + dx) as u16, (y + dy) as u16, w as u16, 1);
        for q in nodes[i].pts.iter_mut() {
            *q = (q.0 + dx, q.1 + dy);
        }
    }
    Some(Plan {
        nodes,
        mode,
        nmax,
        pill: Rect::new((root_lab.0 + dx) as u16, dy as u16, pill.width, 1),
        sp: ry,
        gap: rx as u16,
        hidden: 0,
    })
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
pub(super) fn drawn_h(cx: &Cx, band: Rect, sel: Sel) -> u16 {
    height(cx, band, &cx.s.viewer, 3, sel)
}

/// Rows the map seen from `root` takes in `band`.
pub(super) fn height(cx: &Cx, band: Rect, root: &str, depth: usize, sel: Sel) -> u16 {
    let p = plan(cx, band, root, depth, cx.touch(), sel);
    let ys = p
        .nodes
        .iter()
        .filter(|n| n.shown)
        .map(|n| n.label.y)
        .chain([p.pill.y]);
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
    /// A vertical run between `y0` and `y1`, for link `li`.
    fn vline(&mut self, x: i32, y0: i32, y1: i32, li: usize) {
        for y in y0.min(y1)..=y0.max(y1) {
            if let Some(i) = self.cell(x, y) {
                self.bits[i] |= UP | DN;
                self.links[i].push(li);
            }
        }
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

/// The line glyph for a cell's joins. ASCII: `-` `|`, corners `,` above and `'`
/// below, a junction its trunk's `|` or `-` (`+` is the invite key); a password run is
/// `.` and `:`; a quiet line is drawn plain (its ends say why).
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
            _ if b == DN | RT || b == DN | LT => ',',
            _ if b == UP | RT || b == UP | LT => '\'',
            _ if b & (UP | DN) == UP | DN => '|',
            _ => '-',
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
    /// One cell from the kid's end toward the parent.
    back: (i32, i32),
    /// Where an arrowhead pointing at the kid goes.
    kid_arrow: (i32, i32),
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
    if band.width < 12 || band.height < 1 {
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
        let lit = match sel {
            Sel::Node(id, _) => route.contains(&k) || id == pid,
            Sel::Edge(id) => e.id == id,
            Sel::Root | Sel::Targets => n.parent.is_none(),
        };
        if !n.pts.is_empty() {
            let li = links.len();
            let (a, b) = (n.pts[0], n.pts[n.pts.len() - 1]);
            let this = matches!(sel, Sel::Edge(id) if e.id == id);
            for g in [Some(&mut grid), this.then_some(&mut edited)]
                .into_iter()
                .flatten()
            {
                // (A one-cell run straight up or down is still vertical.)
                if s == 0 {
                    g.vline(a.0, a.1, b.1, li);
                } else {
                    g.path(&n.pts, li);
                }
            }
            let prev = n.pts[n.pts.len() - 2];
            let back = ((prev.0 - b.0).signum(), (prev.1 - b.1).signum());
            // A line that turns takes its arrowhead on its run along the parent's row,
            // a cell before the corner.
            let kid_arrow = if n.pts.len() == 3 {
                (prev.0 - s, prev.1)
            } else {
                b
            };
            links.push(Link {
                e,
                lit,
                kid_end: b,
                par_end: a,
                back,
                kid_arrow,
                toward_kid,
                side: s,
            });
            if !cx.touch() {
                let (mx, my) = if n.pts.len() == 3 {
                    ((a.0 + prev.0) / 2, a.1)
                } else {
                    ((a.0 + b.0) / 2, (a.1 + b.1) / 2)
                };
                let r = Rect::new((mx - 1).max(0) as u16, my.max(0) as u16, 3, 1);
                cx.hit(r, format!("edge:{}", e.id));
            }
            continue;
        }
        if s == 0 {
            // Straight up or down from the root's middle to the label's.
            let cxr = i32::from(pa.x) + i32::from(pa.width) / 2;
            let dir = (ky - py).signum();
            let (a, b) = ((cxr, py + dir), (cxr, ky - dir));
            let li = links.len();
            if (b.1 - a.1) * dir >= 0 {
                grid.vline(cxr, a.1, b.1, li);
                if matches!(sel, Sel::Edge(id) if e.id == id) {
                    edited.vline(cxr, a.1, b.1, li);
                }
            }
            links.push(Link {
                e,
                lit,
                kid_end: b,
                par_end: a,
                back: (0, -dir),
                kid_arrow: b,
                toward_kid,
                side: 0,
            });
            if !cx.touch() {
                cx.hit(
                    Rect::new(
                        (cxr - 1).max(0) as u16,
                        ((a.1 + b.1) / 2).max(0) as u16,
                        3,
                        1,
                    ),
                    format!("edge:{}", e.id),
                );
            }
            continue;
        }
        let start = port(pa, s);
        let end = end_of(n);
        // One trunk per relay and side, a short stub out, before the nearest device's
        // branch: the orbit's arc is in the branches' lengths, not in extra trunks.
        let sibs = p
            .nodes
            .iter()
            .filter(|m| m.shown && m.up.is_some() && m.parent == n.parent && m.side == s);
        let near = sibs.clone().map(|m| end_of(m) * s).min().unwrap_or(end * s) * s;
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
        let par_end = if trunk == end {
            (end, ky)
        } else {
            (trunk + n.side, ky)
        };
        links.push(Link {
            e,
            lit,
            kid_end: (end, ky),
            par_end: if py == ky && alone {
                (start, py)
            } else {
                par_end
            },
            back: (-n.side, 0),
            kid_arrow: (end, ky),
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
        // Everything else recedes to one grey (16 colours cannot fade a hue).
        let tok = match (l.e.up, l.lit, fade_all) {
            (_, true, true) => Tok::Accent,
            (_, false, true) => Tok::Muted,
            (true, _, _) => hue_tok(cx, l.e),
            _ => Tok::Muted,
        };
        style(tok, l.lit, pattern(l.e))
    };
    wash(cx, buf, &p, root, band, fade_all);
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
                    style(if same && !fade_all { h0 } else { Tok::Muted }, false, pat)
                }
            };
            let pat = match ls.iter().find(|l| l.lit) {
                Some(l) => pattern(l.e),
                None => pat,
            };
            let (bx, by) = (band.x + x, band.y + y);
            let ch = line_ch(b, pat, cx.uni());
            // The edge editor's subject keeps the one weight, in the accent (ASCII `#`,
            // a mark no other line uses), so the link being edited reads at a glance.
            let lb = edited.bits[i];
            let ch = if lb != 0 && !cx.uni() { '#' } else { ch };
            buf[(bx, by)].set_char(ch).set_style(st);
        }
    }
    // Ends: an arrowhead where access goes one way, at the end it reaches; a link closed
    // both ways stops short of its device (no glyph on the stroke).
    for l in &links {
        let st = ink(l);
        let mut put = |at: (i32, i32), g: &str| {
            if at.0 >= 0 && at.1 >= 0 {
                cx.put(buf, at.0 as u16, at.1 as u16, 1, g, st);
            }
        };
        if l.e.up && l.e.one_way() && l.side != 0 {
            let right = (l.side > 0) == l.toward_kid;
            // A solid triangle meets its stroke; a font arrow floats in its cell.
            let g = match (right, cx.uni()) {
                (true, true) => "▶",
                (false, true) => "◀",
                (true, false) => ">",
                (false, false) => "<",
            };
            put(if l.toward_kid { l.kid_arrow } else { l.par_end }, g);
        } else if l.e.up && closed(l.e) {
            // Two cells of air before the device: the line does not reach it.
            let (x, y) = l.kid_end;
            // (One cell where the line turns into the dot, so its corner stays.)
            put((x, y), " ");
            if l.back.1 == 0 || l.side == 0 {
                put((x + l.back.0, y + l.back.1), " ");
            }
        }
    }
    // Nodes. Letters are keys on the hub and in the send-to picker only.
    let keys = matches!(sel, Sel::Node(..) | Sel::Targets);
    for n in p.nodes.iter().filter(|n| n.shown) {
        let hot = sel_id == Some(n.d.id.as_str());
        let focus = matches!(sel, Sel::Node(_, true));
        let edge_end = match sel {
            Sel::Edge(id) => {
                cx.s.edge(id)
                    .is_some_and(|e| e.from == n.d.id || e.to == n.d.id)
            }
            _ => false,
        };
        node(
            cx,
            buf,
            &p,
            n,
            hot.then_some(focus),
            keys,
            fade_all && !edge_end,
        );
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

/// A mesh the root is not in, drawn as one group: a quiet wash of its hue behind its
/// members, where they sit together (no other label inside it). The root's own meshes
/// are its lines' hues.
fn wash(cx: &Cx, buf: &mut Buffer, p: &Plan, root: &str, band: Rect, faded: bool) {
    if !cx.tc() {
        return;
    }
    for m in
        cx.s.meshes
            .iter()
            .filter(|m| !m.members.iter().any(|x| x == root))
    {
        let inside = |n: &&Node| n.shown && m.members.contains(&n.d.id);
        let mut it = p.nodes.iter().filter(inside).map(|n| n.label);
        let Some(first) = it.next() else { continue };
        let (bb, count) = it.fold((first, 1), |(a, k), r| (a.union(r), k + 1));
        // Two cells of air either side, symmetric about the root's axis (the group's
        // relay sits on it), on the members' own rows only.
        let axis = i32::from(p.pill.x) * 2 + i32::from(p.pill.width);
        let half = (axis - 2 * i32::from(bb.x)).max(2 * i32::from(bb.right()) - axis) / 2 + 2;
        let x0 = (axis / 2 - half).max(i32::from(band.x));
        let bb = Rect::new(x0 as u16, bb.y, (2 * half) as u16, bb.height).intersection(band);
        let other = p
            .nodes
            .iter()
            .any(|n| n.shown && !inside(&n) && n.label.intersects(bb));
        if count < 2 || other || p.pill.intersects(bb) {
            continue;
        }
        // (Fainter still where the map is scenery, as in the edge editor.)
        let a = if cx.dark() { 0.07 } else { 0.06 } * if faded { 0.5 } else { 1.0 };
        let st = cx.wash(mesh_tok(cx, &m.id), a);
        for y in bb.top()..bb.bottom() {
            for x in bb.left()..bb.right() {
                buf[(x, y)].set_style(st);
            }
        }
    }
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
    let key = cx
        .st(Tok::Accent)
        .add_modifier(Modifier::BOLD | Modifier::UNDERLINED);
    for (i, ch) in nm.chars().enumerate() {
        // ASCII has no underline: the letter sits in parentheses, `(t)ranquility`.
        let s = if at == Some(i) && !cx.uni() {
            format!("({ch})")
        } else {
            ch.to_string()
        };
        let cst = if at == Some(i) { st.patch(key) } else { st };
        w += cx.put(buf, x + w, y, 3, &s, cst);
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
        cx.ink(Tok::Accent, if f { 1.0 } else { 0.7 })
            .add_modifier(Modifier::BOLD)
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
    super::width(&nm, cx.uni())
        + match (at, keys) {
            (None, true) => 4,
            (Some(_), true) if !cx.uni() => 2,
            _ => 0,
        }
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

/// A selection where colour cannot show it: brackets in the cells either side of `r`
/// (only a selection; the viewer is `@` and bare everywhere else).
fn brackets(cx: &Cx, buf: &mut Buffer, r: Rect, st: Style) {
    if !cx.uni() || !cx.c.has_color() {
        cx.put(buf, r.x, r.y, 1, "[", st);
        cx.put(buf, r.right() - 1, r.y, 1, "]", st);
    }
}

/// The root: `◉` and its name in bold (hotkey underlined), no background. Selected, it
/// takes the brackets every selected label has.
fn pill(
    cx: &Cx,
    buf: &mut Buffer,
    p: &Plan,
    root: &str,
    pick: Option<bool>,
    keys: bool,
    fade: bool,
) {
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
        cx.put(
            buf,
            r.x + 2,
            r.y,
            3,
            &format!(" {} ", d.hotkey),
            cx.chip_style(cx.s.hue(&d.id)),
        );
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
        name(
            cx,
            buf,
            r.x + 2,
            r.y,
            d,
            16,
            st.add_modifier(Modifier::BOLD),
            keys,
        );
        if let Some(f) = pick {
            brackets(
                cx,
                buf,
                Rect::new(r.x.saturating_sub(1), r.y, r.width + 2, 1),
                bar(f),
            );
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
