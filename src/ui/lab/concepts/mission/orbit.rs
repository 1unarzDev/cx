//! The network as an orbit: the root device (this one on the hub) sits in the middle
//! in its capsule, the devices it reaches sit on a ring around it and devices reached
//! through them on the next ring out, inside their parent's sector. Links are thin
//! gentle curves of one weight, drawn with braille sub-cell dots (one slope character
//! per cell in ASCII); the selection's links are a little brighter. Hue means network
//! (mesh hue, teal for lan, neutral for direct ssh); the stroke is the access policy,
//! used sparingly: solid where a key is allowed, dashed where only a password is, an
//! arrowhead where access goes one way, sparse dots where nothing can cross (closed
//! both ways, or down). Mesh members carry a faint wash of their mesh's hue behind
//! their name. Nothing here dispatches.
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
    /// Air around the pill (kept clear of labels).
    halo: Rect,
    hidden: usize,
    /// Session slot letters on the labels (only where letters are keys: the hub).
    pips: bool,
    star: bool,
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

/// Stroke pattern of a link (the policy grammar): nothing crosses a sparse one.
fn pattern(e: &LabEdge) -> Pat {
    if !e.up || closed(e) {
        Pat::Sparse
    } else if e.forward == Access::Key || e.reverse == Access::Key {
        Pat::Solid
    } else {
        Pat::Dash
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
        // Children keep close to their parent: at most 65° each, so siblings on the
        // outer ring keep a clear row between them.
        let span = (a1 - a0).min(inner * 1.13);
        let mut a = (a0 + a1 - span) / 2.0;
        for k in kids {
            let w = nodes[k].weight / inner.max(1.0) * span;
            nodes[k].sec = (a, a + w);
            a += w;
        }
    }
}

fn evidence(cx: &Cx, d: &LabDevice, up: Option<&LabEdge>, pips: bool) -> Vec<(String, Style)> {
    let mut v = Vec::new();
    if !cx.up(d) {
        let t = if d.status == DeviceStatus::Unknown {
            "not probed".to_string()
        } else {
            format!("seen {}", cx.meta(d))
        };
        // Muted: the status mark is the one red cell a down device gets.
        v.push((t, cx.ink(Tok::Muted, 0.85)));
        return v;
    }
    // Quiet text, no bars: how far it is (or what is flowing to it), then its
    // sessions' slot letters.
    if let Some(up) = up {
        let t = transfer_rate(cx, up).or_else(|| up.rtt_ms.map(ms_text));
        let tok = if transfer_rate(cx, up).is_some() { edge_tok(cx, up) } else { Tok::Muted };
        if let Some(t) = t {
            v.push((t, cx.ink(tok, 0.9)));
        }
    }
    if pips {
        v.extend(tiles::pip_segs(cx, d, 4));
    }
    v
}

/// Mesh membership: a faint wash of the mesh's hue behind the name row (split in two
/// for a device in two meshes). Only where tints exist: elsewhere the links' hue says it.
fn wash(cx: &Cx, buf: &mut Buffer, r: Rect, d: &LabDevice) {
    if !cx.has_surface() || d.meshes.is_empty() {
        return;
    }
    let k = if cx.dark() { 0.15 } else { 0.11 };
    let n = d.meshes.len().min(2) as u16;
    for (i, m) in d.meshes.iter().take(2).enumerate() {
        let i = i as u16;
        let x0 = r.x + r.width * i / n;
        let x1 = r.x + r.width * (i + 1) / n;
        // Over whatever lies beneath (the terminal, or a sheet's surface).
        for x in x0..x1.min(buf.area.right()) {
            let under = super::to_rgb(buf[(x, r.y)].bg).unwrap_or(cx.bg0());
            let st = cx.rgb_bg(super::mix(under, cx.rgb(mesh_tok(cx, m)), k));
            cx.fill(buf, Rect::new(x, r.y, 1, 1), st);
        }
    }
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

fn label_w(
    cx: &Cx,
    d: &LabDevice,
    up: Option<&LabEdge>,
    mode: Mode,
    nmax: usize,
    pips: bool,
) -> u16 {
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
                row0.max(2 + ev_w(cx, &evidence(cx, d, up, pips)))
            } else {
                row0
            }
        }
    }
}

/// One cell each side, the status glyph, a space and the name.
fn pill_w(cx: &Cx, d: &LabDevice, mode: Mode, keys: bool) -> u16 {
    match mode {
        Mode::Key => 5,
        _ => 4 + name_w(cx, d, 16, keys),
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
    let mut nodes = tree(cx, root, depth);
    sectors(&mut nodes);
    let star = depth == 1;
    if star {
        // A device's own star: its peers left and right of it, where rows are long (a
        // short band has no room above or below the root), top first on each side.
        let k = nodes.len();
        let right = k.div_ceil(2);
        for (i, n) in nodes.iter_mut().enumerate() {
            let (j, m) = if i < right { (i, right) } else { (i - right, k - right) };
            let t = if m == 1 {
                0.0
            } else {
                j as f32 / (m - 1) as f32 - 0.5
            };
            n.ang = if i < right { t * 1.1 } else { PI - t * 1.1 };
        }
    }
    let opts = Opts {
        star,
        pips: matches!(sel, Sel::Node(..)),
        keys: matches!(sel, Sel::Node(..) | Sel::Targets),
        air: true,
    };
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
    // Every named mode with a clear row between labels first, then without it: names
    // packed closer beat bare keycaps.
    let tries = [true, false].into_iter().flat_map(|air| {
        modes
            .iter()
            .filter(move |(m, _)| !air || *m != Mode::Key)
            .map(move |&(m, n)| (m, n, air))
    });
    for (mode, nmax, air) in tries {
        let p = place(cx, band, &nodes, rings, rootd, mode, nmax, Opts { air, ..opts });
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

#[derive(Clone, Copy)]
struct Opts {
    /// One ring, peers beside the root (a device's LINKS).
    star: bool,
    pips: bool,
    keys: bool,
    /// A clear row above and below every label.
    air: bool,
}

/// Place every label for one mode; overlapping ones are hidden (counted in `hidden`).
#[allow(clippy::too_many_arguments)]
fn place<'a>(
    cx: &Cx<'a>,
    band: Rect,
    nodes: &[Node<'a>],
    rings: u8,
    rootd: Option<&'a LabDevice>,
    mode: Mode,
    nmax: usize,
    o: Opts,
) -> Plan<'a> {
    let rows: u16 = if mode == Mode::Full { 2 } else { 1 };
    let widest = nodes
        .iter()
        .map(|n| label_w(cx, n.d, n.up, mode, nmax, o.pips))
        .max()
        .unwrap_or(4);
    let c = (
        f32::from(band.x) + f32::from(band.width) / 2.0,
        f32::from(band.y) + f32::from(band.height.saturating_sub(rows)) / 2.0,
    );
    let rx = (f32::from(band.width) / 2.0 - f32::from(widest) - 2.0).max(4.0);
    // Radii follow the node count, not the band, past a calm size: the first ring stays
    // within about 30 columns and 9 rows of the root, and big screens give the rows to
    // the session thumbnails instead of spreading a few nodes thin.
    let ry = (f32::from(band.height.saturating_sub(rows)) / 2.0 - 0.5).clamp(1.0, 11.5);
    let rx = if o.star {
        // Beside the root on its row: as long as a chip needs, never the whole width.
        rx.min(30.0)
    } else {
        // Never flatter than about 2.6:1 on screen (a cell is twice as tall as wide).
        rx.min(ry * 5.2)
    };
    let pw = rootd.map_or(5, |d| pill_w(cx, d, mode, o.keys));
    let pill = Rect::new(
        (c.0 - f32::from(pw) / 2.0).round() as u16,
        c.1.round() as u16,
        pw,
        1,
    );
    // Short maps keep only the capsule (a halo would crowd the first ring); a star's
    // peers sit beside the root, so its halo fits any band of five rows that is wide
    // enough to leave its links their length.
    let tight = if o.star {
        band.height < 5 || band.width < 80
    } else {
        band.height < 12
    };
    // The root's own evidence row, centred under the pill.
    let pev = match (mode, rootd) {
        (Mode::Full, Some(d)) => {
            let word = if d.id == cx.s.viewer { 4 } else { 5 };
            let w = word + ev_w(cx, &evidence(cx, d, None, o.pips));
            Rect::new((c.0 - f32::from(w) / 2.0).round() as u16, pill.y + 1, w, 1)
        }
        _ => Rect::default(),
    };
    // The halo rings the capsule and its evidence row.
    let hw = pw.max(pev.width) + 4;
    let halo = if tight {
        Rect::new(pill.x.saturating_sub(1), pill.y, pw + 2, 1)
    } else {
        Rect::new(
            (c.0 - f32::from(hw) / 2.0).round().max(0.0) as u16,
            pill.y.saturating_sub(1),
            hw,
            if mode == Mode::Full { 4 } else { 3 },
        )
    }
    .intersection(band);
    let mut taken: Vec<Rect> = vec![halo, pev];
    let mut out: Vec<Node> = Vec::new();
    let mut hidden = 0;
    for n in nodes {
        // A label that would overlap slides a little along its ring first.
        let mut found = None;
        for nudge in [0.0, 0.2, -0.2, 0.4, -0.4] {
            let ang = n.ang + nudge;
            let (fx, fy) = ring_f(n.hop, rings);
            // The first ring keeps clear of the pill, whatever the band's shape, and
            // where links carry chips, room for one between the halo and the label.
            let clear = if cx.size() >= Size::M { 15.0 } else { 11.0 };
            let rx1 = (rx * fx).max(f32::from(pw) / 2.0 + clear);
            let p = (c.0 + rx1 * ang.cos(), c.1 + ry * fy * ang.sin());
            let w = label_w(cx, n.d, n.up, mode, nmax, o.pips);
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
            // Air: a clear column each side and (when it fits) a clear row above and
            // below, so two labels never read as one block.
            let a = u16::from(o.air);
            let pad = Rect::new(
                label.x.saturating_sub(1),
                label.y.saturating_sub(a),
                label.width + 2,
                rows + 2 * a,
            );
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
    // A star keeps its root centred: it is the subject of the screen.
    let dx = if o.star {
        0
    } else {
        ((i32::from(band.x) + i32::from(band.right()) - lo - hi) / 2)
            .clamp(i32::from(band.x) - lo, i32::from(band.right()) - hi)
    };
    // And vertically: the drawing sits one row under the band's top, so the rows it
    // does not need are all at its foot, where the caller gives them to its content.
    let (top, bot) = out
        .iter()
        .filter(|n| n.shown)
        .map(|n| n.label)
        .chain([halo, pev].into_iter().filter(|r| r.width > 0))
        .fold((i32::MAX, i32::MIN), |(lo, hi), r| {
            (lo.min(i32::from(r.y)), hi.max(i32::from(r.bottom())))
        });
    let dy = ((i32::from(band.y) + i32::from(band.bottom()) - top - bot) / 2)
        .clamp(i32::from(band.y) - top, i32::from(band.bottom()) - bot)
        .min(i32::from(band.y) + 1 - top);
    let mv = |r: &mut Rect| {
        r.x = (i32::from(r.x) + dx).max(0) as u16;
        r.y = (i32::from(r.y) + dy).max(0) as u16;
    };
    let (mut pill, mut pev, mut halo) = (pill, pev, halo);
    for n in out.iter_mut() {
        mv(&mut n.label);
        n.dot.0 = (i32::from(n.dot.0) + dx).max(0) as u16;
        n.dot.1 = (i32::from(n.dot.1) + dy).max(0) as u16;
    }
    mv(&mut pill);
    if pev.width > 0 {
        mv(&mut pev);
    }
    mv(&mut halo);
    let c = (c.0 + dx as f32, c.1 + dy as f32);
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
        pips: o.pips,
        star: o.star,
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

/// The most rows a hub map uses: its capped rings (radius at most 11.5 rows) with
/// two-row labels.
pub(super) const MAX_H: u16 = 27;

/// Device ids in selection order: the root first (launch focus, so `⏎` opens this
/// device), then ring by ring, clockwise from the top.
pub(super) fn order(cx: &Cx, band: Rect) -> Vec<String> {
    let root = cx.s.viewer.as_str();
    let p = plan(cx, band, root, 3, cx.touch(), Sel::Node("", false));
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
    /// Even dashes: a password is needed.
    Dash,
    /// Soft beads: nothing crosses (closed both ways, or down).
    Sparse,
}

impl Pat {
    fn on(self, i: usize) -> bool {
        match self {
            Pat::Solid => true,
            Pat::Dash => i % 9 < 5,
            Pat::Sparse => i % 4 == 0,
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
    fn dot(&mut self, sx: i32, sy: i32, st: Style, pri: u8) {
        let (x, y) = (sx.div_euclid(2), sy.div_euclid(4));
        let Some(i) = self.cell(x, y) else { return };
        if self.mask[i] {
            return;
        }
        const B: [[u8; 4]; 2] = [[0x01, 0x02, 0x04, 0x40], [0x08, 0x10, 0x20, 0x80]];
        self.bits[i] |= B[sx.rem_euclid(2) as usize][sy.rem_euclid(4) as usize];
        if pri >= self.st[i].0 {
            self.st[i] = (pri, st);
        }
    }
    /// A polyline in cell coordinates, stroked with a pattern: braille dots, plus one
    /// ASCII character per cell along a thinned path (see `ascii_path`).
    fn stroke(&mut self, pts: &[P], pat: Pat, st: Style, pri: u8) {
        // The sub-cell dots the curve visits, one dot thick: a dot that only turns a
        // corner between its neighbours (an L step) is dropped, so a diagonal is a
        // clean single chain, never a staircase of pairs.
        let mut path: Vec<(i32, i32)> = Vec::new();
        for w in pts.windows(2) {
            let (a, b) = (w[0], w[1]);
            let (dx, dy) = ((b.0 - a.0) * 2.0, (b.1 - a.1) * 4.0);
            let n = (dx.abs().max(dy.abs()) * 2.0).ceil().max(1.0) as i32;
            for k in 0..n {
                let t = k as f32 / n as f32;
                let q = (
                    ((a.0 * 2.0) + dx * t).floor() as i32,
                    ((a.1 * 4.0) + dy * t).floor() as i32,
                );
                if path.last() == Some(&q) {
                    continue;
                }
                if path.len() >= 2 {
                    let o = path[path.len() - 2];
                    if (q.0 - o.0).abs() <= 1 && (q.1 - o.1).abs() <= 1 {
                        path.pop();
                    }
                }
                path.push(q);
            }
        }
        for (i, &(sx, sy)) in path.iter().enumerate() {
            if pat.on(i) {
                self.dot(sx, sy, st, pri);
            }
        }
        for (ci, (x, y, slope)) in ascii_path(pts).into_iter().enumerate() {
            let ch = ascii_for(pat, ci, slope);
            let Some(j) = self.cell(x, y) else { continue };
            if ch == ' ' || self.mask[j] || (self.asc[j] != ' ' && pri < self.st[j].0) {
                continue;
            }
            self.asc[j] = ch;
            if pri >= self.st[j].0 {
                self.st[j] = (pri, st);
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

/// The cells an ASCII line visits, one per column where it runs mostly sideways and one
/// per row where it runs mostly up or down, each with its slope character from the next
/// cell: `-` / `|` where the line stays in its row / column, `\\` or `/` where it steps
/// on. So a diagonal is one continuous run, never doubled or broken.
fn ascii_path(pts: &[P]) -> Vec<(i32, i32, char)> {
    let (Some(&f), Some(&l)) = (pts.first(), pts.last()) else {
        return Vec::new();
    };
    // One mode for the whole (gently bowed) line.
    let wide = (l.0 - f.0).abs() >= (l.1 - f.1).abs();
    let mut cells: Vec<(i32, i32)> = Vec::new();
    for w in pts.windows(2) {
        let (a, b) = (w[0], w[1]);
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        // Every column (or row) centre the segment crosses.
        let (a0, b0, d) = if wide { (a.0, b.0, dx) } else { (a.1, b.1, dy) };
        if d == 0.0 {
            continue;
        }
        let mut c = (a0.min(b0) - 0.5).ceil();
        let mut seg = Vec::new();
        while c + 0.5 <= a0.max(b0) {
            let t = (c + 0.5 - a0) / d;
            let cell = if wide {
                (c as i32, (a.1 + dy * t).floor() as i32)
            } else {
                ((a.0 + dx * t).floor() as i32, c as i32)
            };
            seg.push(cell);
            c += 1.0;
        }
        if d < 0.0 {
            seg.reverse();
        }
        for cell in seg {
            if cells.last() != Some(&cell) {
                cells.push(cell);
            }
        }
    }
    let mut out: Vec<(i32, i32, char)> = Vec::new();
    for (k, &(x, y)) in cells.iter().enumerate() {
        let c = match cells.get(k + 1) {
            Some(&(_, ny)) if wide && ny == y => '-',
            Some(&(nx, _)) if !wide && nx == x => '|',
            Some(&(nx, ny)) if (nx - x).signum() == (ny - y).signum() => '\\',
            Some(_) => '/',
            None => out.last().map_or(if wide { '-' } else { '|' }, |o| o.2),
        };
        out.push((x, y, c));
    }
    out
}

/// ASCII stand-in for a stroke cell: the slope for solid lines, every other one for
/// dashed ones, `.` beads where nothing crosses.
fn ascii_for(pat: Pat, i: usize, slope: char) -> char {
    match (pat, i % 2) {
        (Pat::Solid, _) | (Pat::Dash, 0) => slope,
        (Pat::Sparse, 0) => '.',
        _ => ' ',
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

fn centre(r: Rect) -> P {
    (
        f32::from(r.x) + f32::from(r.width) / 2.0,
        f32::from(r.y) + 0.5,
    )
}

fn cellp(p: (u16, u16)) -> P {
    (f32::from(p.0) + 0.5, f32::from(p.1) + 0.5)
}

/// An arrowhead for a direction on screen (ASCII: `->` or `<-` by its horizontal sense).
fn arrow(cx: &Cx, dx: f32, dy: f32) -> &'static str {
    let a = (dy * 2.0).atan2(dx);
    let k = ((a / (PI / 4.0)).round() as i32).rem_euclid(8);
    if cx.uni() {
        ["→", "↘", "↓", "↙", "←", "↖", "↑", "↗"][k as usize]
    } else {
        // Two cells: `>`, `^` and `v` already mean selection, alerts and nothing else.
        if dx >= 0.0 {
            "->"
        } else {
            "<-"
        }
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
    let p = plan(cx, band, root, depth, keys_only, sel);
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
            n.label.x.saturating_sub(2),
            n.label.y,
            n.label.width + 4,
            n.label.height,
        ));
    }
    // The capsule plus its selection brackets; ASCII keeps one more clear cell (`..[`
    // would read as one stroke).
    let pad = if cx.uni() { 1 } else { 2 };
    cv.block(Rect::new(
        p.pill.x.saturating_sub(pad),
        p.pill.y,
        p.pill.width + 2 * pad,
        1,
    ));
    // The root's evidence keeps a clear cell each side (a stroke never touches `a s`).
    if p.pev.width > 0 {
        cv.block(Rect::new(
            p.pev.x.saturating_sub(2),
            p.pev.y,
            p.pev.width + 4,
            1,
        ));
    }

    // Links: the tree, plus the selected device's (or edited) peer links.
    let mut arrows: Vec<(Vec<P>, &'static str, Style)> = Vec::new();
    let mut links: Vec<(&LabEdge, P, P, P, bool)> = Vec::new();
    for (i, n) in p.nodes.iter().enumerate() {
        let (Some(e), true) = (n.up, n.shown) else {
            continue;
        };
        if n.parent.is_some_and(|q| !p.nodes[q].shown) {
            continue;
        }
        let b = at(Some(i));
        let from_a = n.parent.map_or(root, |q| p.nodes[q].d.id.as_str()) == e.from;
        if p.star {
            // A star's links leave the capsule's end facing their peer and arrive level
            // with it, so each chip rides on its peer's row.
            let pc = centre(p.pill);
            let x = if b.0 < pc.0 {
                f32::from(p.pill.x) - 1.5
            } else {
                f32::from(p.pill.right()) + 1.5
            };
            let a = (x, pc.1);
            links.push((e, a, b, (a.0 + (b.0 - a.0) * 0.3, b.1), from_a));
        } else {
            let a = at(n.parent);
            links.push((e, a, b, ctrl(a, b, 0.07), from_a));
        }
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
        let out = ctrl(a, b, 0.18);
        let d2 = |q: P| (q.0 - p.c.0).powi(2) + ((q.1 - p.c.1) * 2.0).powi(2);
        let bow = if d2(out) >= d2(m) { 0.18 } else { -0.18 };
        links.push((e, a, b, ctrl(a, b, bow), true));
    }
    for (e, a, b, c, from_a) in links {
        let on = lit(e);
        // A link nothing can cross keeps its network's hue, quieter still: the node at
        // its end carries the one red mark.
        let tok = if e.up { edge_tok(cx, e) } else { hue_tok(cx, e) };
        let quiet = pattern(e) == Pat::Sparse;
        let alpha: f32 = match (on, fade_all) {
            (_, true) if !on => 0.22,
            (true, _) => 0.95,
            _ if quiet => 0.45,
            _ => 0.6,
        };
        // On white a pale hue at 60 % vanishes: light themes deepen link hues like
        // session inks (`tiles::adapt`), so every link keeps about 3:1 on white.
        let st = if cx.tc() && !cx.dark() && !(fade_all && !on) {
            let c = super::mix(cx.bg0(), cx.rgb(tok), (alpha + 0.2).min(1.0));
            cx.rgb_fg(tiles::adapt(cx, c, false))
        } else {
            cx.ink(tok, alpha)
        };
        let pts = curve(a, c, b);
        cv.stroke(&pts, pattern(e), st, if on { 6 } else { 4 });
        // One-way access: a small arrowhead near the end access reaches.
        if e.up && e.one_way() {
            let fwd = (e.forward != Access::Denied) == from_a;
            let ts = if fwd { [0.74, 0.66, 0.58] } else { [0.26, 0.34, 0.42] };
            let d = if fwd { 0.04 } else { -0.04 };
            let (p0, p1) = (bez(a, c, b, ts[0] - d), bez(a, c, b, ts[0] + d));
            // Short links fall back toward the middle, so the head is never masked away.
            let spots = ts.map(|t| bez(a, c, b, t)).to_vec();
            arrows.push((spots, arrow(cx, p1.0 - p0.0, p1.1 - p0.1), st));
        }
        // Desktop: a 3x1 spot on the link's middle is its tap target (phones tap the
        // tree rows).
        if !cx.touch() {
            let q = bez(a, c, b, 0.5);
            let r = Rect::new((q.0 as u16).saturating_sub(1), q.1 as u16, 3, 1);
            cx.hit(r, format!("edge:{}", e.id));
        }
    }
    cv.flush(cx, buf);
    for (spots, g, st) in arrows {
        let gw = super::width(g, cx.uni());
        // ASCII keeps one clear cell each side (`<-` beside a `.` stroke or a bracket
        // would read as the down stroke `-.-`).
        let pad = u16::from(!cx.uni());
        let free = spots
            .iter()
            .map(|q| (q.0 as u16, q.1 as u16))
            .find(|&(x, y)| {
                (x.saturating_sub(pad)..x + gw + pad).all(|xx| {
                    cv.cell(i32::from(xx), i32::from(y))
                        .is_some_and(|i| !cv.mask[i])
                })
            });
        if let Some((x, y)) = free {
            if pad > 0 {
                cx.put(buf, x - 1, y, 1, " ", Style::default());
                cx.put(buf, x + gw, y, 1, " ", Style::default());
            }
            cx.put(buf, x, y, gw, g, st);
        }
    }
    // Nodes.
    // Letters are keys on the hub and in the send-to picker only.
    let keys = matches!(sel, Sel::Node(..) | Sel::Targets);
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
/// keys); where letters are not keys the name is entirely plain (no letter stands out,
/// so nothing suggests a key that does something else there). Returns columns written.
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
        cx.ink(Tok::Id(cx.s.hue(&d.id)), 1.0)
            .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
    };
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
        // Keycaps in every map, so the phone orbit reads the same everywhere: a faded
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
            st,
        );
        if !matches!(d.status, DeviceStatus::Online) && pick.is_none() {
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
    let gst = if fade { cx.ink(Tok::Faint, 1.0) } else { gst };
    let nst = if pick == Some(true) {
        cx.st(Tok::Accent).add_modifier(Modifier::BOLD)
    } else if fade {
        // Muted, not faint: a faded name stays readable (faint is for strokes).
        cx.ink(Tok::Muted, 0.85)
    } else if up {
        cx.ink(Tok::Fg, 1.0).add_modifier(Modifier::BOLD)
    } else {
        cx.ink(Tok::Muted, 0.9)
    };
    let left = n.dot.0 + 1 == r.right() && r.width > 3;
    let nw = r.width - 2;
    // The name row: status dot toward the links, the name, and behind both (one cell
    // of air each side) the wash of its meshes.
    let w = name_w(cx, d, p.nmax, keys) + 2;
    let row = if left {
        Rect::new(r.right() - w, r.y, w, 1)
    } else {
        Rect::new(r.x, r.y, w, 1)
    };
    if pick.is_none() && !fade {
        wash(cx, buf, Rect::new(row.x.saturating_sub(1), r.y, w + 2, 1), d);
    }
    // (Text keeps the cells' background: the wash shows through.)
    let (dx, nx) = if left {
        (r.right() - 1, row.x)
    } else {
        (r.x, r.x + 2)
    };
    cx.put(buf, dx, r.y, 1, g, gst.patch(bg));
    name(cx, buf, nx, r.y, d, p.nmax, nst.patch(bg), keys);
    if r.height > 1 {
        let ev = evidence(cx, d, n.up, p.pips);
        let w = ev_w(cx, &ev);
        let mut x = if left { r.right() - 2 - w } else { r.x + 2 };
        for (t, st) in ev {
            let st = if fade { cx.ink(Tok::Muted, 0.5) } else { st };
            x += cx.put(buf, x, r.y + 1, nw, &t, st.patch(bg)) + 1;
        }
    }
    if fade {
        return;
    }
    hit_node(
        cx,
        Rect::new(r.x.saturating_sub(1), r.y, r.width + 2, r.height),
        d,
    );
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

/// The root: a tinted capsule with its name (hotkey underlined) and, with room, its own
/// evidence beneath, all inside the dotted halo.
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
    let strength = match (pick, cx.dark()) {
        _ if fade => 0.1,
        (Some(true), true) => 0.42,
        (Some(true), false) => 0.32,
        (_, true) => 0.26,
        (_, false) => 0.18,
    };
    // Phone maps (keycaps) draw the root as its keycap between brackets: tinted
    // shoulders beside a keycap read as a render artefact at that size.
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
        // `◉` marks the root on every map (the hub's viewer, a device's own star) unless
        // its status has something to say.
        let (g, gst) = cx.status(d.status);
        let g = if matches!(d.status, DeviceStatus::Online) {
            cx.g("◉", "@")
        } else {
            g
        };
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
        name(cx, buf, r.x + 3, r.y, d, 16, st, keys);
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
        ev.extend(evidence(cx, d, None, p.pips));
        let w = ev_w(cx, &ev);
        let mut x = (centre(r).0 - f32::from(w) / 2.0).round() as u16;
        for (t, st) in ev {
            x += cx.put(buf, x, r.y + 1, w, &t, st) + 1;
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
