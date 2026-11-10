//! The network as an orbit: the root device (this one on the hub) sits in the middle
//! inside a dotted halo, the devices it reaches sit on a ring around it and devices
//! reached through them on the next ring out, inside their parent's sector. Mesh
//! membership is a soft dotted arc around its members with the mesh named on it. Links
//! are gentle curves drawn with braille sub-cell dots (one slope character per cell in
//! ASCII). Hue means network (mesh hue, teal for lan, neutral for direct ssh) and red
//! for a link that is down; the stroke is the access policy: solid where a key is
//! allowed, dotted where only a password is, an arrowhead where access goes one way,
//! dash-dot when the link is down. Nothing here dispatches.
use super::{fit, ms_text, tiles, Cx, Size};
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
    /// One link (the edge editor): drawn heavy, everything else fades.
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
    /// Name, then a row of evidence (cpu trend and %, temperature, session pips).
    Full,
}

type P = (f32, f32);

struct Node<'a> {
    d: &'a LabDevice,
    hop: u8,
    /// The link to its parent (None for devices with no routed link).
    up: Option<&'a LabEdge>,
    parent: Option<usize>,
    ang: f32,
    /// Sector bounds (children divide it).
    sec: (f32, f32),
    weight: f32,
    /// Where its links meet it (the status dot, or the keycap's middle).
    dot: (u16, u16),
    label: Rect,
    shown: bool,
}

struct Plan<'a> {
    nodes: Vec<Node<'a>>,
    rings: u8,
    c: P,
    rx: f32,
    ry: f32,
    mode: Mode,
    nmax: usize,
    pill: Rect,
    /// The root's evidence row (empty unless two-row labels).
    pev: Rect,
    /// The halo around the pill (also kept clear of labels).
    halo: Rect,
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
    if !e.up {
        return Tok::Err;
    }
    match &e.kind {
        EdgeKind::Mesh(m) => mesh_tok(cx, m),
        EdgeKind::Lan => Tok::Lan,
        EdgeKind::Direct | EdgeKind::Jump(_) => Tok::Muted,
    }
}

/// The routed path from the viewer to `to` (at most three hops, shortest first), as the
/// links taken; empty for the viewer itself or no route.
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

/// Stroke pattern of a link (the policy grammar).
fn pattern(e: &LabEdge) -> Pat {
    if !e.up {
        Pat::DashDot
    } else if closed(e) {
        Pat::Sparse
    } else if e.forward == Access::Key || e.reverse == Access::Key {
        Pat::Solid
    } else {
        Pat::Dot
    }
}

// ------------------------------------------------------------------------ layout

/// Spanning tree from `root` over routed links, breadth first, up to `depth` hops.
/// On the hub (depth 3) devices with no routed link join the outer ring unlinked.
fn tree<'a>(cx: &Cx<'a>, root: &str, depth: usize) -> Vec<Node<'a>> {
    let s = cx.s;
    let idx = |id: &str| s.devices.iter().position(|d| d.id == id).unwrap_or(0);
    let group = |id: &str| {
        s.meshes
            .iter()
            .position(|m| m.members.iter().any(|x| x == id))
            .unwrap_or(s.meshes.len())
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
            // Mesh members sit together, so their arc stays short.
            kids.sort_by_key(|(d, _)| (group(&d.id), idx(&d.id)));
            for (d, e) in kids {
                seen.push(&d.id);
                nodes.push(Node {
                    d,
                    hop,
                    up: Some(e),
                    parent: p,
                    ang: 0.0,
                    sec: (0.0, 0.0),
                    weight: 1.0,
                    dot: (0, 0),
                    label: Rect::default(),
                    shown: true,
                });
                next.push(Some(nodes.len() - 1));
            }
        }
        if next.is_empty() {
            break;
        }
        frontier = next;
    }
    if depth >= 3 {
        let hop = nodes.iter().map(|n| n.hop).max().unwrap_or(1);
        for d in s.devices.iter().filter(|d| !seen.contains(&d.id.as_str())) {
            nodes.push(Node {
                d,
                hop,
                up: None,
                parent: None,
                ang: 0.0,
                sec: (0.0, 0.0),
                weight: 1.0,
                dot: (0, 0),
                label: Rect::default(),
                shown: true,
            });
        }
    }
    nodes
}

/// Sectors: every first-ring subtree gets a share of the circle by its size; children
/// divide their parent's sector. The largest subtree faces right, where rows are long.
fn sectors(nodes: &mut [Node]) {
    for i in (0..nodes.len()).rev() {
        if let Some(p) = nodes[i].parent {
            let w = nodes[i].weight;
            nodes[p].weight += w;
        }
    }
    let top: Vec<usize> = (0..nodes.len())
        .filter(|&i| nodes[i].parent.is_none())
        .collect();
    let total: f32 = top.iter().map(|&i| nodes[i].weight).sum::<f32>().max(1.0);
    // Start so the heaviest subtree is centred at 0 (right); equal ones sit on the
    // diagonals rather than straight above and below the pill.
    let heavy = top
        .iter()
        .copied()
        .max_by(|&a, &b| nodes[a].weight.total_cmp(&nodes[b].weight))
        .unwrap_or(0);
    let mut acc = 0.0;
    let mut start = 0.0;
    for &i in &top {
        if i == heavy {
            start = -(acc + nodes[i].weight / 2.0) / total * TAU;
        }
        acc += nodes[i].weight;
    }
    if top.iter().all(|&i| nodes[i].weight == 1.0) {
        start = -PI / 2.0 + PI / top.len().max(1) as f32;
        if top.len() == 2 {
            start = 0.0;
        }
    }
    let mut a = start;
    for &i in &top {
        let w = nodes[i].weight / total * TAU;
        nodes[i].sec = (a, a + w);
        a += w;
    }
    for i in 0..nodes.len() {
        let (a0, a1) = nodes[i].sec;
        nodes[i].ang = (a0 + a1) / 2.0;
        let kids: Vec<usize> = (0..nodes.len())
            .filter(|&k| nodes[k].parent == Some(i))
            .collect();
        let inner = nodes[i].weight - 1.0;
        // Children keep close to their parent: at most 50° each.
        let span = (a1 - a0).min(inner * 0.87);
        let mut a = (a0 + a1 - span) / 2.0;
        for k in kids {
            let w = nodes[k].weight / inner.max(1.0) * span;
            nodes[k].sec = (a, a + w);
            a += w;
        }
    }
}

fn evidence(cx: &Cx, d: &LabDevice) -> Vec<(String, Style)> {
    let mut v = Vec::new();
    if !cx.up(d) {
        let t = if d.status == DeviceStatus::Unknown {
            "not probed".to_string()
        } else {
            format!("seen {}", cx.meta(d))
        };
        v.push((t, cx.ink(Tok::Err, 0.85)));
        return v;
    }
    if let Some(m) = &d.metrics {
        let h: Vec<Option<f32>> = m.cpu_hist.iter().map(|v| Some(*v)).collect();
        v.push((tiles::spark_str(cx, &h, 5), cx.heat(m.cpu)));
        v.push((format!("{:.0}%", m.cpu * 100.0), cx.ink(Tok::Muted, 1.0)));
        if let Some(t) = m.temp_c.filter(|_| cx.size() >= Size::L) {
            let hv = ((t - 35.0) / 50.0).clamp(0.0, 1.0);
            v.push((format!("{t:.0}{}", cx.g("°", "C")), cx.heat(hv)));
        }
    }
    for (t, st) in tiles::pip_segs(cx, d, 4) {
        v.push((t, st));
    }
    v
}

fn ev_w(cx: &Cx, v: &[(String, Style)]) -> u16 {
    v.iter()
        .map(|(t, _)| super::width(t, cx.uni()) + 1)
        .sum::<u16>()
        .saturating_sub(1)
}

/// The name, with a keycap in front when the hotkey letter is not in it.
fn has_letter(d: &LabDevice) -> Option<usize> {
    d.name
        .chars()
        .position(|c| c.eq_ignore_ascii_case(&d.hotkey))
}

fn label_w(cx: &Cx, d: &LabDevice, mode: Mode, nmax: usize) -> u16 {
    match mode {
        Mode::Key => 3 + u16::from(!matches!(d.status, DeviceStatus::Online)),
        _ => {
            let n = d.name.chars().count().min(nmax) as u16;
            let cap = if has_letter(d).is_some_and(|i| i < nmax.saturating_sub(1)) {
                0
            } else {
                4
            };
            let row0 = 2 + cap + n;
            if mode == Mode::Full {
                row0.max(2 + ev_w(cx, &evidence(cx, d)))
            } else {
                row0
            }
        }
    }
}

fn pill_w(d: &LabDevice, mode: Mode) -> u16 {
    match mode {
        Mode::Key => 5,
        _ => 4 + d.name.chars().count().min(16) as u16 + 1,
    }
}

fn plan<'a>(cx: &Cx<'a>, band: Rect, root: &str, depth: usize, keys_only: bool) -> Plan<'a> {
    let mut nodes = tree(cx, root, depth);
    sectors(&mut nodes);
    let rings = nodes.iter().map(|n| n.hop).max().unwrap_or(1);
    let rootd = cx.s.device(root);
    let modes: &[(Mode, usize)] = if keys_only {
        &[(Mode::Key, 0)]
    } else if band.height >= 9 && cx.size() >= Size::M {
        &[
            (Mode::Full, 16),
            (Mode::Full, 12),
            (Mode::Name, 16),
            (Mode::Name, 12),
            (Mode::Name, 9),
            (Mode::Key, 0),
        ]
    } else {
        &[
            (Mode::Name, 16),
            (Mode::Name, 12),
            (Mode::Name, 9),
            (Mode::Key, 0),
        ]
    };
    let mut best: Option<Plan> = None;
    for &(mode, nmax) in modes {
        let p = place(cx, band, &nodes, rings, rootd, mode, nmax);
        let ok = p.hidden == 0;
        best = Some(p);
        if ok {
            break;
        }
    }
    let mut p = best.expect("at least one mode");
    // Nodes carry over the placement of the chosen mode.
    for (n, q) in nodes.iter_mut().zip(p.nodes.drain(..)) {
        n.dot = q.dot;
        n.label = q.label;
        n.shown = q.shown;
        n.ang = q.ang;
    }
    p.nodes = nodes;
    p
}

/// Place every label for one mode; overlapping ones are hidden (counted in `hidden`).
fn place<'a>(
    cx: &Cx<'a>,
    band: Rect,
    nodes: &[Node<'a>],
    rings: u8,
    rootd: Option<&'a LabDevice>,
    mode: Mode,
    nmax: usize,
) -> Plan<'a> {
    let rows: u16 = if mode == Mode::Full { 2 } else { 1 };
    let widest = nodes
        .iter()
        .map(|n| label_w(cx, n.d, mode, nmax))
        .max()
        .unwrap_or(4);
    let c = (
        f32::from(band.x) + f32::from(band.width) / 2.0,
        f32::from(band.y) + f32::from(band.height.saturating_sub(rows)) / 2.0,
    );
    let rx = (f32::from(band.width) / 2.0 - f32::from(widest) - 2.0).max(4.0);
    let ry = (f32::from(band.height.saturating_sub(rows)) / 2.0 - 0.5).max(1.0);
    // Never flatter than about 2.6:1 on screen (a cell is twice as tall as wide).
    let rx = rx.min(ry * 5.2);
    let pw = rootd.map_or(5, |d| pill_w(d, mode));
    let pill = Rect::new(
        (c.0 - f32::from(pw) / 2.0).round() as u16,
        c.1.round() as u16,
        pw,
        1,
    );
    // Tiny maps keep only the capsule (a halo would crowd the first ring).
    let halo = if mode == Mode::Key && band.height < 9 {
        Rect::new(pill.x.saturating_sub(1), pill.y, pw + 2, 1)
    } else {
        Rect::new(
            pill.x.saturating_sub(2),
            pill.y.saturating_sub(1),
            pw + 4,
            if mode == Mode::Full { 4 } else { 3 },
        )
    }
    .intersection(band);
    // The root's own evidence row, centred under the pill.
    let pev = match (mode, rootd) {
        (Mode::Full, Some(d)) => {
            let w = 4 + ev_w(cx, &evidence(cx, d));
            Rect::new((c.0 - f32::from(w) / 2.0).round() as u16, pill.y + 1, w, 1)
        }
        _ => Rect::default(),
    };
    let mut taken: Vec<Rect> = vec![halo, pev];
    let mut out: Vec<Node> = Vec::new();
    let mut hidden = 0;
    for n in nodes {
        // A label that would overlap slides a little along its ring first.
        let mut found = None;
        for nudge in [0.0, 0.2, -0.2, 0.4, -0.4] {
            let ang = n.ang + nudge;
            let (fx, fy) = ring_f(n.hop, rings);
            // The first ring keeps clear of the pill, whatever the band's shape.
            let rx1 = (rx * fx).max(f32::from(pw) / 2.0 + 7.0);
            let p = (c.0 + rx1 * ang.cos(), c.1 + ry * fy * ang.sin());
            let w = label_w(cx, n.d, mode, nmax);
            let (px, py) = (p.0.round() as i32, p.1.round() as i32);
            let cos = ang.cos();
            let x0 = if mode == Mode::Key {
                px - 1
            } else if cos > 0.3 {
                px
            } else if cos < -0.3 {
                px - i32::from(w) + 1
            } else {
                px - i32::from(w) / 2
            };
            let x0 = x0.clamp(i32::from(band.x), i32::from(band.right()) - i32::from(w));
            let y0 = py.clamp(
                i32::from(band.y),
                i32::from(band.bottom()) - i32::from(rows),
            );
            let label = Rect::new(x0.max(0) as u16, y0.max(0) as u16, w, rows);
            let dot = if mode == Mode::Key {
                (label.x + 1, label.y)
            } else if cos < -0.3 {
                (label.right() - 1, label.y)
            } else {
                (label.x, label.y)
            };
            let pad = Rect::new(label.x.saturating_sub(1), label.y, label.width + 2, rows);
            let clash = taken.iter().any(|t| t.intersects(pad)) || !band.contains(label.into());
            if !clash || nudge == -0.4 {
                found = Some((ang, label, dot, pad, clash));
                if !clash {
                    break;
                }
            }
        }
        let Some((ang, label, dot, pad, clash)) = found else {
            continue;
        };
        if clash {
            hidden += 1;
        } else {
            taken.push(pad);
        }
        out.push(Node {
            d: n.d,
            hop: n.hop,
            up: n.up,
            parent: n.parent,
            ang,
            sec: n.sec,
            weight: n.weight,
            dot,
            label,
            shown: !clash,
        });
    }
    // Centre the drawing, not the root: outer rings are often one-sided.
    let shown = out.iter().filter(|n| n.shown).map(|n| n.label);
    let (lo, hi) = shown
        .chain([halo])
        .fold((i32::MAX, i32::MIN), |(lo, hi), r| {
            (lo.min(i32::from(r.x)), hi.max(i32::from(r.right())))
        });
    let dx = ((i32::from(band.x) + i32::from(band.right()) - lo - hi) / 2)
        .clamp(i32::from(band.x) - lo, i32::from(band.right()) - hi);
    let mv = |r: &mut Rect| r.x = (i32::from(r.x) + dx).max(0) as u16;
    let (mut pill, mut pev, mut halo) = (pill, pev, halo);
    for n in out.iter_mut() {
        mv(&mut n.label);
        n.dot.0 = (i32::from(n.dot.0) + dx).max(0) as u16;
    }
    mv(&mut pill);
    if pev.width > 0 {
        mv(&mut pev);
    }
    mv(&mut halo);
    let c = (c.0 + dx as f32, c.1);
    Plan {
        nodes: out,
        rings,
        c,
        rx,
        ry,
        mode,
        nmax,
        pill,
        pev,
        halo,
        hidden,
    }
}

/// Ring radii as fractions of the band's (x, y) radii: the first ring keeps clear of the
/// pill and sits tall, outer rings spread wide, where rows are long.
fn ring_f(hop: u8, rings: u8) -> (f32, f32) {
    if rings <= 1 {
        return (1.0, 1.0);
    }
    let t = f32::from(hop.max(1) - 1) / f32::from(rings - 1);
    (0.5 + 0.5 * t, 0.78 + 0.22 * t)
}

/// Rows the map wants: two rings of two-row labels at a calm spacing (one ring for a
/// device's own neighbourhood).
pub(super) fn want_h(cx: &Cx) -> u16 {
    match cx.size() {
        Size::L => 21,
        Size::M => 17,
        Size::S => 11,
        Size::Xs => 9,
    }
}

/// Device ids in selection order: the root first (launch focus, so `⏎` opens this
/// device), then ring by ring, clockwise from the top.
pub(super) fn order(cx: &Cx, band: Rect) -> Vec<String> {
    let root = cx.s.viewer.as_str();
    let p = plan(cx, band, root, 3, cx.touch());
    let mut v: Vec<&Node> = p.nodes.iter().filter(|n| n.shown).collect();
    let clock = |a: f32| (a + PI / 2.0).rem_euclid(TAU);
    v.sort_by(|a, b| {
        (a.hop, clock(a.ang))
            .partial_cmp(&(b.hop, clock(b.ang)))
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

// ------------------------------------------------------------------------ canvas

#[derive(Clone, Copy, PartialEq)]
enum Pat {
    Solid,
    Dot,
    DashDot,
    /// Soft beads: closed links.
    Sparse,
    /// Mesh arcs: a faint continuous orbit (`'` beads in ASCII, never a link stroke).
    Orbit,
    /// The root's halo: dots (`'` beads in ASCII).
    Halo,
}

impl Pat {
    fn on(self, i: usize) -> bool {
        match self {
            Pat::Solid => true,
            Pat::Dot => i % 3 == 0,
            Pat::DashDot => matches!(i % 9, 0..=3 | 6),
            Pat::Sparse => i % 4 == 0,
            Pat::Orbit => true,
            Pat::Halo => i % 3 == 0,
        }
    }
}

/// Braille sub-cell canvas (2x4 dots per cell), one style per cell decided by the
/// highest-priority stroke; in ASCII, one character per cell.
struct Canvas {
    r: Rect,
    bits: Vec<u8>,
    st: Vec<(u8, Style)>,
    asc: Vec<char>,
    mask: Vec<bool>,
}

impl Canvas {
    fn new(r: Rect) -> Self {
        let n = usize::from(r.width) * usize::from(r.height);
        Self {
            r,
            bits: vec![0; n],
            st: vec![(0, Style::default()); n],
            asc: vec![' '; n],
            mask: vec![false; n],
        }
    }
    fn cell(&self, x: i32, y: i32) -> Option<usize> {
        let (x, y) = (x - i32::from(self.r.x), y - i32::from(self.r.y));
        (x >= 0 && y >= 0 && x < i32::from(self.r.width) && y < i32::from(self.r.height))
            .then(|| (y * i32::from(self.r.width) + x) as usize)
    }
    fn block(&mut self, r: Rect) {
        for y in r.top()..r.bottom() {
            for x in r.left()..r.right() {
                if let Some(i) = self.cell(i32::from(x), i32::from(y)) {
                    self.mask[i] = true;
                }
            }
        }
    }
    /// One dot at sub-cell coordinates.
    fn dot(&mut self, sx: i32, sy: i32, st: Style, pri: u8, asc: char) {
        let (x, y) = (sx.div_euclid(2), sy.div_euclid(4));
        let Some(i) = self.cell(x, y) else { return };
        if self.mask[i] {
            return;
        }
        const B: [[u8; 4]; 2] = [[0x01, 0x02, 0x04, 0x40], [0x08, 0x10, 0x20, 0x80]];
        self.bits[i] |= B[sx.rem_euclid(2) as usize][sy.rem_euclid(4) as usize];
        if pri >= self.st[i].0 {
            self.st[i] = (pri, st);
            if asc != ' ' {
                self.asc[i] = asc;
            }
        }
    }
    /// A polyline in cell coordinates, stroked with a pattern.
    fn stroke(&mut self, pts: &[P], pat: Pat, st: Style, pri: u8, thick: bool) {
        let mut last = (i32::MIN, i32::MIN);
        let mut lastc = (i32::MIN, i32::MIN);
        let (mut i, mut ci) = (0usize, 0usize);
        for w in pts.windows(2) {
            let (a, b) = (w[0], w[1]);
            let (dx, dy) = ((b.0 - a.0) * 2.0, (b.1 - a.1) * 4.0);
            let n = (dx.abs().max(dy.abs()) * 1.5).ceil().max(1.0) as i32;
            for k in 0..n {
                let t = k as f32 / n as f32;
                let (sx, sy) = (
                    ((a.0 * 2.0) + dx * t).floor() as i32,
                    ((a.1 * 4.0) + dy * t).floor() as i32,
                );
                if (sx, sy) == last {
                    continue;
                }
                last = (sx, sy);
                i += 1;
                let cell = (sx.div_euclid(2), sy.div_euclid(4));
                let mut asc = ' ';
                if cell != lastc {
                    lastc = cell;
                    ci += 1;
                    asc = ascii_for(pat, ci, dx, dy / 2.0);
                }
                if pat.on(i) {
                    self.dot(sx, sy, st, pri, asc);
                    if thick {
                        if dx.abs() > dy.abs() {
                            self.dot(sx, sy + 1, st, pri, ' ');
                        } else {
                            self.dot(sx + 1, sy, st, pri, ' ');
                        }
                    }
                } else if asc != ' ' && matches!(pat, Pat::Dot | Pat::DashDot) {
                    // ASCII cells still need their character when the dot falls in a gap.
                    if let Some(j) = self.cell(cell.0, cell.1) {
                        if !self.mask[j] && pri >= self.st[j].0 {
                            self.st[j] = (pri, st);
                            self.asc[j] = asc;
                            self.bits[j] |= 0;
                        }
                    }
                }
            }
        }
    }
    fn flush(&self, cx: &Cx, buf: &mut Buffer) {
        for y in 0..self.r.height {
            for x in 0..self.r.width {
                let i = usize::from(y) * usize::from(self.r.width) + usize::from(x);
                if self.mask[i] {
                    continue;
                }
                let (bx, by) = (self.r.x + x, self.r.y + y);
                if cx.uni() {
                    if self.bits[i] == 0 {
                        continue;
                    }
                    let ch = char::from_u32(0x2800 + u32::from(self.bits[i])).unwrap_or(' ');
                    buf[(bx, by)].set_char(ch).set_style(self.st[i].1);
                } else if self.asc[i] != ' ' {
                    buf[(bx, by)].set_char(self.asc[i]).set_style(self.st[i].1);
                }
            }
        }
    }
}

/// ASCII stand-in for a stroke cell: the slope for solid lines, `.` for dotted ones,
/// `'` beads for mesh arcs and halos.
fn ascii_for(pat: Pat, i: usize, dx: f32, dy: f32) -> char {
    let slope = if dx.abs() > dy.abs() * 2.0 {
        '-'
    } else if dy.abs() > dx.abs() * 2.0 {
        '|'
    } else if (dx > 0.0) == (dy > 0.0) {
        '\\'
    } else {
        '/'
    };
    match pat {
        Pat::Solid => slope,
        Pat::Dot => '.',
        Pat::DashDot => {
            if i % 3 == 2 {
                '.'
            } else {
                slope
            }
        }
        Pat::Sparse => {
            if i % 2 == 0 {
                '.'
            } else {
                ' '
            }
        }
        Pat::Orbit | Pat::Halo => {
            if i % 2 == 0 {
                '\''
            } else {
                ' '
            }
        }
    }
}

fn bez(a: P, c: P, b: P, t: f32) -> P {
    let u = 1.0 - t;
    (
        u * u * a.0 + 2.0 * u * t * c.0 + t * t * b.0,
        u * u * a.1 + 2.0 * u * t * c.1 + t * t * b.1,
    )
}

/// A gentle bow: the control point sits off the chord's middle by `k` of its length
/// (on screen, where a cell is twice as tall as wide).
fn ctrl(a: P, b: P, k: f32) -> P {
    let (dx, dy) = (b.0 - a.0, (b.1 - a.1) * 2.0);
    let m = ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0);
    (m.0 - dy * k, m.1 + dx * k / 2.0)
}

fn curve(a: P, c: P, b: P) -> Vec<P> {
    (0..=32).map(|i| bez(a, c, b, i as f32 / 32.0)).collect()
}

fn ellipse(c: P, rx: f32, ry: f32, a0: f32, a1: f32) -> Vec<P> {
    let n = ((a1 - a0).abs() * rx.max(ry) * 1.5).ceil().max(8.0) as usize;
    (0..=n)
        .map(|i| {
            let a = a0 + (a1 - a0) * i as f32 / n as f32;
            (c.0 + rx * a.cos(), c.1 + ry * a.sin())
        })
        .collect()
}

fn centre(r: Rect) -> P {
    (
        f32::from(r.x) + f32::from(r.width) / 2.0,
        f32::from(r.y) + 0.5,
    )
}

fn cellp(p: (u16, u16)) -> P {
    (f32::from(p.0) + 0.5, f32::from(p.1) + 0.5)
}

/// An arrowhead for a direction on screen.
fn arrow(cx: &Cx, dx: f32, dy: f32) -> &'static str {
    let a = (dy * 2.0).atan2(dx);
    let k = ((a / (PI / 4.0)).round() as i32).rem_euclid(8);
    if cx.uni() {
        ["→", "↘", "↓", "↙", "←", "↖", "↑", "↗"][k as usize]
    } else {
        [">", "v", "v", "v", "<", "^", "^", "^"][k as usize]
    }
}

// ------------------------------------------------------------------------- draw

/// Draw the network seen from `root` (rings up to `depth` hops) into `band`. Phones and
/// mini-maps pass `keys_only`: keycaps alone, the names live in the tree below.
pub(super) fn draw(cx: &Cx, buf: &mut Buffer, band: Rect, root: &str, depth: usize, sel: Sel) {
    draw_with(cx, buf, band, root, depth, sel, cx.touch());
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
    let s = cx.s;
    let p = plan(cx, band, root, depth, keys_only);
    let mark = cx.hit_mark();
    let idx = |id: &str| p.nodes.iter().position(|n| n.d.id == id);
    let at = |i: Option<usize>| match i {
        Some(i) => cellp(p.nodes[i].dot),
        None => centre(p.pill),
    };
    // The selection's route back to the root lights up with its own links.
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
    let lit = |e: &LabEdge| match sel {
        Sel::Node(id, _) => {
            e.from == id
                || e.to == id
                || route
                    .iter()
                    .any(|&i| p.nodes[i].up.is_some_and(|u| u.id == e.id))
        }
        Sel::Edge(id) => e.id == id,
        Sel::Root | Sel::Targets => e.from == root || e.to == root,
    };
    let fade_all = matches!(sel, Sel::Edge(_));

    let mut cv = Canvas::new(band);
    for n in p.nodes.iter().filter(|n| n.shown) {
        cv.block(Rect::new(
            n.label.x.saturating_sub(1),
            n.label.y,
            n.label.width + 2,
            n.label.height,
        ));
    }
    cv.block(Rect::new(p.pill.x, p.pill.y, p.pill.width, 1));
    cv.block(p.pev);

    // Mesh arcs: a soft dotted arc just outside its members, named in its hue.
    let mut mesh_names: Vec<(Vec<P>, String, Tok, String)> = Vec::new();
    if p.mode != Mode::Key {
        for m in &s.meshes {
            let mem: Vec<&Node> = p
                .nodes
                .iter()
                .filter(|n| n.shown && m.members.contains(&n.d.id))
                .collect();
            if mem.len() < 2 {
                continue;
            }
            let ring = mem.iter().map(|n| n.hop).max().unwrap_or(1);
            let (fx, fy) = ring_f(ring, p.rings);
            // The smallest arc over every member: the complement of the widest gap.
            let mut angs: Vec<f32> = mem.iter().map(|n| n.ang.rem_euclid(TAU)).collect();
            angs.sort_by(f32::total_cmp);
            let k = angs.len();
            // A gap holding a device outside the mesh must stay open, so the arc never
            // seems to include it; then the widest gap.
            let others: Vec<f32> = p
                .nodes
                .iter()
                .filter(|n| n.shown && n.hop <= ring && !m.members.contains(&n.d.id))
                .map(|n| n.ang.rem_euclid(TAU))
                .collect();
            let (gap_at, _) = (0..k)
                .map(|i| {
                    let (g0, len) = (angs[i], (angs[(i + 1) % k] - angs[i]).rem_euclid(TAU));
                    let len = if k == 1 { TAU } else { len };
                    let holds = others.iter().any(|&o| (o - g0).rem_euclid(TAU) < len);
                    (i, (holds, len))
                })
                .max_by(|a, b| a.1 .0.cmp(&b.1 .0).then(a.1 .1.total_cmp(&b.1 .1)))
                .unwrap_or((0, (false, 0.0)));
            let a0 = angs[(gap_at + 1) % k] - 0.2;
            let mut a1 = angs[gap_at] + 0.2;
            if a1 < a0 {
                a1 += TAU;
            }
            // Halfway out to the next ring, or just outside the last one.
            let (rxm, rym) = if ring < p.rings {
                let (nx, ny) = ring_f(ring + 1, p.rings);
                (p.rx * (fx + nx) / 2.0 + 1.0, p.ry * (fy + ny) / 2.0 + 0.5)
            } else {
                (p.rx * fx + 4.0, p.ry * fy + 1.4)
            };
            let tok = mesh_tok(cx, &m.id);
            let pts = ellipse(p.c, rxm, rym, a0, a1);
            // A faint continuous orbit, so it never reads as a dotted (password) link.
            let a = if fade_all { 0.2 } else { 0.36 };
            cv.stroke(&pts, Pat::Orbit, cx.ink(tok, a), 1, false);
            // The name rides just outside the arc, at its middle or the nearest free spot.
            let spots = [0.5, 0.35, 0.65, 0.2, 0.8]
                .map(|t| {
                    let a = a0 + (a1 - a0) * t;
                    (p.c.0 + (rxm + 1.0) * a.cos(), p.c.1 + (rym + 0.8) * a.sin())
                })
                .to_vec();
            mesh_names.push((spots, m.name.clone(), tok, m.id.clone()));
        }
    }

    // The halo around the root.
    let pc = centre(p.pill);
    let hy = if p.mode == Mode::Full { 1.7 } else { 1.25 };
    let hrx = f32::from(p.pill.width) / 2.0 + 2.2;
    let halo = ellipse(
        (pc.0, pc.1 + if p.mode == Mode::Full { 0.5 } else { 0.0 }),
        hrx,
        hy,
        0.0,
        TAU,
    );
    if p.halo.height > 1 {
        cv.stroke(&halo, Pat::Halo, cx.ink(Tok::Accent, 0.8), 2, false);
    }

    // Links: the tree, plus the selected device's (or edited) peer links.
    let mut chips: Vec<(&LabEdge, P, bool)> = Vec::new();
    let mut arrows: Vec<(P, &'static str, Style)> = Vec::new();
    let mut links: Vec<(&LabEdge, P, P, f32, bool)> = Vec::new();
    for (i, n) in p.nodes.iter().enumerate() {
        let (Some(e), true) = (n.up, n.shown) else {
            continue;
        };
        if n.parent.is_some_and(|q| !p.nodes[q].shown) {
            continue;
        }
        let a = at(n.parent);
        let b = at(Some(i));
        let from_a = n.parent.map_or(root, |q| p.nodes[q].d.id.as_str()) == e.from;
        links.push((e, a, b, 0.1, from_a));
    }
    for e in s.edges.iter().filter(|e| routed(e) && lit(e)) {
        let (Some(i), Some(j)) = (idx(&e.from), idx(&e.to)) else {
            continue;
        };
        let tree_link = p.nodes[i].up.is_some_and(|u| u.id == e.id)
            || p.nodes[j].up.is_some_and(|u| u.id == e.id);
        if tree_link || !p.nodes[i].shown || !p.nodes[j].shown {
            continue;
        }
        let (a, b) = (at(Some(i)), at(Some(j)));
        // Bow away from the pill, so a peer link rounds the outside of the ring.
        let m = ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0);
        let out = ctrl(a, b, 0.3);
        let d2 = |q: P| (q.0 - p.c.0).powi(2) + ((q.1 - p.c.1) * 2.0).powi(2);
        let bow = if d2(out) >= d2(m) { 0.3 } else { -0.3 };
        links.push((e, a, b, bow, true));
    }
    for (e, a, b, bow, from_a) in links {
        let on = lit(e);
        let tok = edge_tok(cx, e);
        let alpha = match (on, fade_all) {
            (true, _) => 1.0,
            (false, true) => 0.25,
            (false, false) => 0.62,
        };
        let st = if on {
            cx.ink(tok, alpha).add_modifier(Modifier::BOLD)
        } else {
            cx.ink(tok, alpha)
        };
        let c = ctrl(a, b, bow);
        let pts = curve(a, c, b);
        let heavy = fade_all && on;
        cv.stroke(&pts, pattern(e), st, if on { 6 } else { 4 }, heavy);
        // One-way access: an arrowhead two thirds of the way, pointing the way allowed.
        if e.up && e.one_way() {
            let fwd = (e.forward != Access::Denied) == from_a;
            let (t0, t1) = if fwd { (0.62, 0.68) } else { (0.38, 0.32) };
            let (p0, p1) = (bez(a, c, b, t0), bez(a, c, b, t1));
            arrows.push((p0, arrow(cx, p1.0 - p0.0, p1.1 - p0.1), st));
        }
        chips.push((e, bez(a, c, b, 0.5), on));
    }
    cv.flush(cx, buf);
    for (q, g, st) in arrows {
        let (x, y) = (q.0 as u16, q.1 as u16);
        if band.contains((x, y).into())
            && !cv.mask[cv.cell(i32::from(x), i32::from(y)).unwrap_or(0)]
        {
            cx.put(buf, x, y, 1, g, st);
        }
    }

    // Chips on the links' middles: the rtt (with the policy word at L), or what is
    // wrong. Below 100 columns only closed and down links carry one.
    let mut taken: Vec<Rect> = p
        .nodes
        .iter()
        .filter(|n| n.shown)
        .map(|n| n.label)
        .chain([p.pill, p.pev])
        .collect();
    for (e, q, on) in chips {
        let text = chip_text(cx, e, p.mode);
        let Some(text) = text else {
            let r = Rect::new(q.0 as u16, q.1 as u16, 1, 1);
            cx.hit(
                Rect::new(r.x.saturating_sub(1), r.y, 3, 1),
                format!("edge:{}", e.id),
            );
            continue;
        };
        let w = super::width(&text, cx.uni()) + 2;
        let r = Rect::new(
            (q.0 - f32::from(w) / 2.0).max(f32::from(band.x)) as u16,
            q.1 as u16,
            w,
            1,
        );
        if !band.contains(r.into())
            || r.right() > band.right()
            || taken.iter().any(|t| t.intersects(r))
        {
            continue;
        }
        taken.push(r);
        let tok = if !e.up {
            Tok::Err
        } else if closed(e) {
            Tok::Muted
        } else {
            edge_tok(cx, e)
        };
        let bg = if cx.has_surface() {
            cx.wash(tok, if on { 0.16 } else { 0.07 })
        } else {
            Style::default()
        };
        let fg = cx.ink(
            if e.up { Tok::Fg } else { Tok::Err },
            if on { 1.0 } else { 0.8 },
        );
        cx.fill(buf, r, bg);
        cx.put(buf, r.x + 1, r.y, w - 2, &text, fg.patch(bg));
        cx.hit(r, format!("edge:{}", e.id));
    }

    // Mesh names on their arcs.
    for (spots, name, tok, id) in mesh_names {
        let w = super::width(&name, cx.uni());
        let free = spots.iter().find_map(|q| {
            let x = (q.0 - f32::from(w) / 2.0).round().max(f32::from(band.x)) as u16;
            let y =
                q.1.round()
                    .clamp(f32::from(band.y), f32::from(band.bottom() - 1)) as u16;
            let r = Rect::new(x, y, w, 1);
            let pad = Rect::new(r.x.saturating_sub(1), r.y, r.width + 2, 1);
            (r.right() <= band.right() && !taken.iter().any(|t| t.intersects(pad))).then_some(r)
        });
        let Some(r) = free else { continue };
        let (x, y) = (r.x, r.y);
        cx.put(
            buf,
            x,
            y,
            w,
            &name,
            cx.ink(tok, 0.9).add_modifier(Modifier::ITALIC),
        );
        cx.hit(r, format!("mesh:{id}"));
        taken.push(r);
    }

    // Nodes.
    let keys = !matches!(sel, Sel::Root);
    for n in p.nodes.iter().filter(|n| n.shown) {
        let hot = sel_id == Some(n.d.id.as_str());
        let focus = matches!(sel, Sel::Node(_, true));
        let edge_end = match sel {
            Sel::Edge(id) => s
                .edge(id)
                .is_some_and(|e| e.from == n.d.id || e.to == n.d.id),
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
    );
    if p.hidden > 0 {
        let t = format!("+{} more", p.hidden);
        let w = super::width(&t, cx.uni());
        let x = band.right().saturating_sub(w + 1);
        let y = band.bottom() - 1;
        cx.put(buf, x, y, w, &t, cx.st(Tok::Muted));
        cx.hit(Rect::new(x, y, w, 1), "palette:devices");
    }
    cx.unclash(mark);
}

fn chip_text(cx: &Cx, e: &LabEdge, mode: Mode) -> Option<String> {
    if !e.up {
        return Some(if mode == Mode::Key || cx.size() <= Size::S {
            cx.g("✕", "x").to_string()
        } else {
            format!("{} down", cx.g("✕", "x"))
        });
    }
    if closed(e) {
        return Some(policy(
            cx,
            e,
            true,
            mode == Mode::Key || cx.size() <= Size::S,
        ));
    }
    if cx.size() <= Size::S || mode == Mode::Key {
        return None;
    }
    let rate = transfer_rate(cx, e);
    let ms = e.rtt_ms.map(ms_text);
    let num = rate.or(ms)?;
    Some(if cx.size() >= Size::L {
        let word = match (e.forward, e.reverse) {
            (Access::Key, _) | (_, Access::Key) => "key",
            _ => "pass",
        };
        format!("{word} {num}")
    } else {
        num
    })
}

/// An active transfer over this link: its rate, with an arrow toward the receiver.
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

/// A device's name with its hotkey letter underlined in the device hue (letters are
/// keys); where letters are not keys the name is plain. Returns columns written.
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
    let mut w = 0;
    let at = has_letter(d).filter(|&i| i + 1 < nm.chars().count() || nm == d.name);
    if at.is_none() {
        w += if keys {
            cx.dchip(buf, x, y, d, false)
        } else {
            cx.put(
                buf,
                x,
                y,
                3,
                &format!(" {} ", d.hotkey),
                cx.ink(Tok::Id(cx.s.hue(&d.id)), 1.0),
            )
        };
        w += 1;
    }
    let key = cx.ink(Tok::Id(cx.s.hue(&d.id)), 1.0).add_modifier(
        Modifier::BOLD
            | if keys {
                Modifier::UNDERLINED
            } else {
                Modifier::empty()
            },
    );
    for (i, ch) in nm.chars().enumerate() {
        let s = ch.to_string();
        let cst = if at == Some(i) { st.patch(key) } else { st };
        w += cx.put(buf, x + w, y, 2, &s, cst);
    }
    w
}

fn node(cx: &Cx, buf: &mut Buffer, p: &Plan, n: &Node, pick: Option<bool>, keys: bool, fade: bool) {
    let d = n.d;
    let r = n.label;
    let up = cx.up(d);
    if p.mode == Mode::Key {
        let st = if fade {
            cx.ink(Tok::Muted, 0.6)
        } else {
            cx.chip_style(cx.s.hue(&d.id))
        };
        if let Some(f) = pick {
            let bar = if f {
                cx.st(Tok::Accent)
            } else {
                cx.ink(Tok::Muted, 0.9)
            };
            cx.put(buf, r.x.saturating_sub(1), r.y, 1, cx.g("▕", "["), bar);
            cx.put(buf, r.x + 3, r.y, 1, cx.g("▏", "]"), bar);
        }
        cx.put(
            buf,
            r.x,
            r.y,
            3,
            &format!(" {} ", d.hotkey),
            if keys || fade {
                st
            } else {
                cx.ink(Tok::Id(cx.s.hue(&d.id)), 1.0)
                    .add_modifier(Modifier::BOLD)
            },
        );
        if !matches!(d.status, DeviceStatus::Online) && pick.is_none() {
            let (g, gst) = cx.status(d.status);
            cx.put(buf, r.x + 3, r.y, 1, g, gst);
        }
        hit_node(cx, r, d);
        return;
    }
    let bg = match pick {
        Some(true) if cx.has_surface() => cx.st(Tok::AccentBg),
        Some(false) if cx.has_surface() => cx.st(Tok::SelectDimBg),
        _ => Style::default(),
    };
    if pick.is_some() {
        cx.fill(
            buf,
            Rect::new(r.x.saturating_sub(1), r.y, r.width + 2, r.height),
            bg,
        );
        let bar = if pick == Some(true) {
            cx.st(Tok::Accent)
        } else {
            cx.ink(Tok::Muted, 0.9)
        };
        let (l, rr) = (r.x.saturating_sub(2), r.right() + 1);
        if r.height == 1 {
            cx.put(buf, l, r.y, 1, cx.g("▕", "["), bar);
            cx.put(buf, rr, r.y, 1, cx.g("▏", "]"), bar);
        } else {
            cx.put(buf, l, r.y, 1, cx.g("╭", "["), bar);
            cx.put(buf, l, r.bottom() - 1, 1, cx.g("╰", "["), bar);
            cx.put(buf, rr, r.y, 1, cx.g("╮", "]"), bar);
            cx.put(buf, rr, r.bottom() - 1, 1, cx.g("╯", "]"), bar);
        }
    }
    let (g, gst) = cx.status(d.status);
    let nst = if pick == Some(true) {
        cx.st(Tok::Accent).add_modifier(Modifier::BOLD)
    } else if up && !fade {
        cx.ink(Tok::Fg, 1.0).add_modifier(Modifier::BOLD)
    } else {
        cx.ink(Tok::Muted, 0.9)
    };
    let left = n.dot.0 + 1 == r.right() && r.width > 3;
    let nw = r.width - 2;
    if left {
        cx.put(buf, r.right() - 1, r.y, 1, g, gst.patch(bg));
        let w = name_w(cx, d, p.nmax);
        name(
            cx,
            buf,
            r.right() - 2 - w,
            r.y,
            d,
            p.nmax,
            nst.patch(bg),
            keys,
        );
    } else {
        cx.put(buf, r.x, r.y, 1, g, gst.patch(bg));
        name(cx, buf, r.x + 2, r.y, d, p.nmax, nst.patch(bg), keys);
    }
    if r.height > 1 {
        let ev = evidence(cx, d);
        let w = ev_w(cx, &ev);
        let mut x = if left { r.right() - 2 - w } else { r.x + 2 };
        for (t, st) in ev {
            let st = if fade { cx.ink(Tok::Muted, 0.5) } else { st };
            x += cx.put(buf, x, r.y + 1, nw, &t, st.patch(bg)) + 1;
        }
    }
    hit_node(
        cx,
        Rect::new(r.x.saturating_sub(1), r.y, r.width + 2, r.height),
        d,
    );
}

fn name_w(cx: &Cx, d: &LabDevice, nmax: usize) -> u16 {
    let nm = fit(&d.name, nmax, cx.uni());
    let at = has_letter(d).filter(|&i| i + 1 < nm.chars().count() || nm == d.name);
    super::width(&nm, cx.uni()) + if at.is_none() { 4 } else { 0 }
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

/// The root: a tinted capsule with its name (hotkey underlined) and, with room, its own
/// evidence beneath, all inside the dotted halo.
fn pill(cx: &Cx, buf: &mut Buffer, p: &Plan, root: &str, pick: Option<bool>, keys: bool) {
    let Some(d) = cx.s.device(root) else { return };
    let r = p.pill;
    let strength = match (pick, cx.dark()) {
        (Some(true), true) => 0.42,
        (Some(true), false) => 0.32,
        (_, true) => 0.26,
        (_, false) => 0.18,
    };
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
        let (g, gst) = cx.status(d.status);
        let gst = if cx.has_surface() {
            gst.patch(body)
        } else {
            body
        };
        cx.put(buf, r.x + 1, r.y, 1, g, gst);
        let st = if cx.has_surface() {
            cx.ink(Tok::Fg, 1.0)
                .patch(body)
                .add_modifier(Modifier::BOLD)
        } else {
            body.add_modifier(Modifier::BOLD)
        };
        name(cx, buf, r.x + 3, r.y, d, 16, st, keys && cx.has_surface());
    }
    if let Some(f) = pick {
        let bar = if f {
            cx.st(Tok::Accent)
        } else {
            cx.ink(Tok::Muted, 0.9)
        };
        cx.put(buf, r.x.saturating_sub(1), r.y, 1, cx.g("▕", "["), bar);
        cx.put(buf, r.right(), r.y, 1, cx.g("▏", "]"), bar);
    }
    if p.mode == Mode::Full {
        let mut ev = vec![(
            if d.id == cx.s.viewer { "you" } else { "here" }.to_string(),
            cx.st(Tok::Accent),
        )];
        ev.extend(evidence(cx, d));
        let w = ev_w(cx, &ev);
        let mut x = (centre(r).0 - f32::from(w) / 2.0).round() as u16;
        for (t, st) in ev {
            x += cx.put(buf, x, r.y + 1, w, &t, st) + 1;
        }
    }
    let hit = if cx.touch() {
        Rect::new(r.x, r.y.saturating_sub(1), r.width, 3)
    } else {
        r
    };
    cx.hit(hit, format!("device:{}", d.id));
}
