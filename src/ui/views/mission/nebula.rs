//! The hub's map: a modular nebula in the round-67 style. This device sits in the
//! middle (`◉` and its name, bold, inside a faint dotted halo); the devices it reaches
//! sit on quiet dotted orbit rings around it, one ring per hop. Devices group into
//! clusters by the network of their first hop (a mesh, the viewer's lan, a relay's
//! site, or plain direct links). Each cluster owns an arc of the orbit; when the whole
//! network does not fit, every cluster but the selection's folds into one node with a
//! count and its members' letters, and blooms open when the selection enters it.
//! Links are bowed dotted braille curves; a cluster's links leave the root as one
//! bundle. Beside each device: a small activity sparkline and its latency in plain dim
//! text. Nothing paints a cell background.
//!
//! The two halves are one code path: every position is computed for the right half
//! and the left half is its mirror (cells, braille dots and labels), so a symmetric
//! network draws as an exact mirror image.
use super::{fit, orbit, spatial, tiles, Cx, Size};
use crate::ui::views::{Access, DeviceStatus, EdgeKind, LabDevice, LabEdge, Tok};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
};
use std::f32::consts::PI;

type P = (f32, f32);

#[derive(Clone, Copy, PartialEq, Debug)]
enum Mode {
    /// Name, then a row of evidence (sparkline, cpu, latency).
    Full,
    /// Name with its evidence after it, on one row.
    Line,
    /// Status dot and name.
    Name,
    /// The letter alone.
    Key,
}

/// A cell rectangle in signed coordinates (a layout may try positions off screen).
#[derive(Clone, Copy, PartialEq, Debug, Default)]
struct R {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
}

impl R {
    fn hits(&self, o: &R) -> bool {
        self.x < o.x + o.w && o.x < self.x + self.w && self.y < o.y + o.h && o.y < self.y + self.h
    }
    fn pad(&self, dx: i32) -> R {
        R {
            x: self.x - dx,
            w: self.w + 2 * dx,
            ..*self
        }
    }
    fn rect(&self) -> Rect {
        Rect::new(
            self.x.max(0) as u16,
            self.y.max(0) as u16,
            self.w.max(0) as u16,
            self.h.max(0) as u16,
        )
    }
}

/// A cluster: a mesh, the viewer's lan, a relay's site, or the direct links.
struct Cl {
    key: String,
    name: String,
    tok: Tok,
    /// Its devices (node indexes), first hops first.
    members: Vec<usize>,
    side: i32,
    open: bool,
    phi: f32,
    /// The folded node's dot and label (right-half coordinates).
    dot: (i32, i32),
    lab: R,
}

struct N<'a> {
    d: &'a LabDevice,
    hop: u8,
    up: Option<&'a LabEdge>,
    parent: Option<usize>,
    cl: usize,
    phi: f32,
    /// Dot and label in right-half coordinates (the left half mirrors them).
    dot: (i32, i32),
    lab: R,
}

struct Plan<'a> {
    nodes: Vec<N<'a>>,
    cls: Vec<Cl>,
    mode: Mode,
    nmax: usize,
    /// The mirror axis in half cells: cell `x` mirrors to `a - 1 - x`.
    a: i32,
    root: R,
    rx: f32,
    ry: f32,
    /// Half the root label's width plus the air before the first ring.
    r0: f32,
    hops: u8,
    /// Devices that could not be placed at all (none unless the band is tiny).
    hidden: usize,
}

/// Ring radius factors by hop.
const RING: [f32; 3] = [1.0, 1.6, 2.15];

fn ring(hop: u8) -> f32 {
    RING[usize::from(hop.clamp(1, 3)) - 1]
}

impl Plan<'_> {
    fn ax(&self) -> f32 {
        self.a as f32 / 2.0
    }
    fn cy(&self) -> f32 {
        self.root.y as f32 + 0.5
    }
    /// A right-half point as drawn on `side` (and back: the mirror is its own inverse).
    fn mirror(&self, side: i32, p: P) -> P {
        if side < 0 {
            (self.a as f32 - p.0, p.1)
        } else {
            p
        }
    }
    fn open(&self, i: usize) -> bool {
        self.cls[self.nodes[i].cl].open
    }
    fn side(&self, i: usize) -> i32 {
        self.cls[self.nodes[i].cl].side
    }
}

// ------------------------------------------------------------------------ layout

fn leaves(nodes: &[N], kids: &[Vec<usize>], i: usize) -> f32 {
    kids[i]
        .iter()
        .map(|&k| leaves(nodes, kids, k))
        .sum::<f32>()
        .max(1.0)
}

/// Group the spanning tree into clusters by the network of each first hop.
fn cluster<'a>(cx: &Cx<'a>, nodes: &mut [N<'a>]) -> Vec<Cl> {
    let n = nodes.len();
    let kids: Vec<Vec<usize>> = (0..n)
        .map(|i| (0..n).filter(|&k| nodes[k].parent == Some(i)).collect())
        .collect();
    let mut cls: Vec<Cl> = Vec::new();
    for i in 0..n {
        let top = {
            let mut t = i;
            while let Some(p) = nodes[t].parent {
                t = p;
            }
            t
        };
        let key = match nodes[top].up.map(|e| &e.kind) {
            Some(EdgeKind::Mesh(m)) => m.clone(),
            Some(EdgeKind::Lan) => "lan".into(),
            Some(_) if leaves(nodes, &kids, top) >= 2.0 => format!("site:{}", nodes[top].d.id),
            _ => "direct".into(),
        };
        let at = match cls.iter().position(|c| c.key == key) {
            Some(at) => at,
            None => {
                let (name, tok) = if let Some(m) = cx.s.meshes.iter().find(|m| m.id == key) {
                    (m.name.clone(), orbit::mesh_tok(cx, &m.id))
                } else if key == "lan" {
                    ("lan".into(), Tok::Lan)
                } else if key.starts_with("site:") {
                    (format!("via {}", nodes[top].d.name), Tok::Muted)
                } else if let Some(EdgeKind::Mesh(m)) = nodes[top].up.map(|e| &e.kind) {
                    (m.clone(), orbit::mesh_tok(cx, m))
                } else {
                    ("direct".into(), Tok::Muted)
                };
                cls.push(Cl {
                    key,
                    name,
                    tok,
                    members: Vec::new(),
                    side: 1,
                    open: true,
                    phi: 0.0,
                    dot: (0, 0),
                    lab: R::default(),
                });
                cls.len() - 1
            }
        };
        nodes[i].cl = at;
        cls[at].members.push(i);
    }
    // Sides by size (stable whatever is selected): the heaviest cluster right, each
    // next one on the lighter side.
    let mut by: Vec<usize> = (0..cls.len()).collect();
    by.sort_by_key(|&c| std::cmp::Reverse(cls[c].members.len()));
    let (mut wr, mut wl) = (0, 0);
    for c in by {
        let w = cls[c].members.len();
        if wr <= wl {
            wr += w;
            cls[c].side = 1;
        } else {
            wl += w;
            cls[c].side = -1;
        }
    }
    cls
}

#[derive(Clone, Copy)]
struct Try {
    mode: Mode,
    nmax: usize,
    ry: f32,
    rx: f32,
}

/// Lay the map out in `band` (only its width and at most its height are used, so a
/// band of the drawing's own height lays out the same).
fn plan<'a>(cx: &Cx<'a>, band: Rect, root: &str, sel: &str) -> Plan<'a> {
    let mut nodes = nodes_of(orbit::tree(cx, root, 3));
    let mut cls = cluster(cx, &mut nodes);
    let focus = nodes.iter().find(|n| n.d.id == sel).map(|n| n.cl);
    let foldable = cls
        .iter()
        .enumerate()
        .any(|(c, cl)| Some(c) != focus && cl.members.len() > 1);
    let cap: f32 = match cx.size() {
        Size::L => 6.0,
        Size::M => 5.0,
        Size::S => 4.0,
        Size::Xs => 3.0,
    };
    let mut ladder: Vec<(Mode, usize)> = Vec::new();
    if !cx.touch() {
        if cx.size() >= Size::M {
            ladder.extend([(Mode::Full, 16), (Mode::Full, 12)]);
        }
        ladder.extend([
            (Mode::Line, 16),
            (Mode::Line, 12),
            (Mode::Name, 16),
            (Mode::Name, 12),
            (Mode::Name, 9),
        ]);
    }
    ladder.push((Mode::Key, 0));
    let mut last = None;
    // A small network shows whole when it fits; a larger one is a nebula: every
    // cluster but the selection's folded.
    let folds: &[bool] = match (foldable, nodes.len() > 9) {
        (false, _) => &[false],
        (true, true) => &[true],
        (true, false) => &[false, true],
    };
    for &(mode, nmax) in &ladder {
        for &fold in folds {
            let mut ry = cap;
            while ry >= 1.0 {
                for k in [3.0, 2.4] {
                    let t = Try {
                        mode,
                        nmax,
                        ry,
                        rx: ry * k,
                    };
                    for (c, cl) in cls.iter_mut().enumerate() {
                        cl.open = !fold || Some(c) == focus || cl.members.len() < 2;
                    }
                    if let Some(p) = place(cx, band, root, &mut nodes, &mut cls, t) {
                        return finish(nodes, cls, p);
                    }
                    last = Some(t);
                }
                ry -= 0.5;
            }
        }
    }
    // Nothing fits (a tiny band): the smallest try, devices outside the band hidden.
    let t = last.expect("a try");
    let p = place_any(cx, band, root, &mut nodes, &mut cls, t);
    finish(nodes, cls, p)
}

struct Placed {
    mode: Mode,
    nmax: usize,
    a: i32,
    root: R,
    rx: f32,
    ry: f32,
    r0: f32,
    hops: u8,
    hidden: usize,
}

fn finish<'a>(nodes: Vec<N<'a>>, cls: Vec<Cl>, p: Placed) -> Plan<'a> {
    Plan {
        nodes,
        cls,
        mode: p.mode,
        nmax: p.nmax,
        a: p.a,
        root: p.root,
        rx: p.rx,
        ry: p.ry,
        r0: p.r0,
        hops: p.hops,
        hidden: p.hidden,
    }
}

fn place<'a>(
    cx: &Cx<'a>,
    band: Rect,
    root: &str,
    nodes: &mut [N<'a>],
    cls: &mut [Cl],
    t: Try,
) -> Option<Placed> {
    let p = layout(cx, band, root, nodes, cls, t, false);
    (p.hidden == 0).then_some(p)
}

fn place_any<'a>(
    cx: &Cx<'a>,
    band: Rect,
    root: &str,
    nodes: &mut [N<'a>],
    cls: &mut [Cl],
    t: Try,
) -> Placed {
    layout(cx, band, root, nodes, cls, t, true)
}

/// One try: angles, then positions on the rings, labels pushed apart along their side,
/// then the drawing centred in the band. `hidden` counts labels left outside it.
fn layout<'a>(
    cx: &Cx<'a>,
    band: Rect,
    root: &str,
    nodes: &mut [N<'a>],
    cls: &mut [Cl],
    t: Try,
    force: bool,
) -> Placed {
    let n = nodes.len();
    let kids: Vec<Vec<usize>> = (0..n)
        .map(|i| (0..n).filter(|&k| nodes[k].parent == Some(i)).collect())
        .collect();
    // Angles, per side, from the top (0) to the bottom (π); clusters keep air between.
    for side in [1, -1] {
        let on: Vec<usize> = (0..cls.len()).filter(|&c| cls[c].side == side).collect();
        let weights: Vec<f32> = (0..cls.len())
            .map(|c| {
                if cls[c].open {
                    cls[c]
                        .members
                        .iter()
                        .filter(|&&i| nodes[i].parent.is_none())
                        .map(|&i| leaves(nodes, &kids, i))
                        .sum::<f32>()
                        .max(1.0)
                } else {
                    1.0
                }
            })
            .collect();
        let weight = |c: usize| weights[c];
        let gap = 0.6;
        let total: f32 =
            on.iter().map(|&c| weight(c)).sum::<f32>() + gap * on.len().saturating_sub(1) as f32;
        let unit = PI / (total + 0.5).max(1.0);
        let mut at = 0.25 * unit;
        for &c in &on {
            let w = weight(c);
            cls[c].phi = at + w * unit / 2.0;
            if cls[c].open {
                fn spread(i: usize, a: f32, unit: f32, kids: &[Vec<usize>], nodes: &mut [N]) {
                    let l = leaves(nodes, kids, i);
                    nodes[i].phi = a + l * unit / 2.0;
                    let mut b = a;
                    for &k in &kids[i] {
                        spread(k, b, unit, kids, nodes);
                        b += leaves(nodes, kids, k) * unit;
                    }
                }
                let mut a = at;
                let tops: Vec<usize> = cls[c]
                    .members
                    .iter()
                    .copied()
                    .filter(|&i| nodes[i].parent.is_none())
                    .collect();
                for i in tops {
                    spread(i, a, unit, &kids, nodes);
                    a += leaves(nodes, &kids, i) * unit;
                }
            }
            at += (w + gap) * unit;
        }
    }
    let rows = if t.mode == Mode::Full { 2 } else { 1 };
    let rootd = cx.s.device(root);
    let rw = rootd.map_or(1, |d| root_w(cx, d, t.mode));
    let root_r = R {
        x: i32::from(band.x) + (i32::from(band.width) - rw) / 2,
        y: 0,
        w: rw,
        h: if t.mode == Mode::Full { 2 } else { 1 },
    };
    let a = 2 * root_r.x + root_r.w;
    let ax = a as f32 / 2.0;
    let r0 = rw as f32 / 2.0 + 3.0;
    let xmin = (a + 1) / 2 + 1;
    let hops = nodes.iter().map(|n| n.hop).max().unwrap_or(1);
    let pos = |phi: f32, hop: u8| -> (i32, i32) {
        let m = ring(hop);
        let x = ax + phi.sin() * (r0 + t.rx * m);
        let y = 0.5 - phi.cos() * t.ry * m;
        (
            ((x - 0.5).round() as i32).max(xmin),
            (y - 0.5).round() as i32,
        )
    };
    for i in 0..n {
        let open = cls[nodes[i].cl].open;
        if !open {
            continue;
        }
        let dot = pos(nodes[i].phi, nodes[i].hop);
        let w = dev_w(cx, nodes[i].d, nodes[i].up, t.mode, t.nmax);
        nodes[i].dot = dot;
        nodes[i].lab = R {
            x: dot.0,
            y: dot.1,
            w,
            h: rows,
        };
    }
    for cl in cls.iter_mut().filter(|c| !c.open) {
        let dot = pos(cl.phi, 1);
        cl.dot = dot;
        cl.lab = R {
            x: dot.0,
            y: dot.1,
            w: cl_w(cx, cl, nodes, t.mode),
            h: rows,
        };
    }
    // Push labels apart along their side: outward (up above the root's row, down
    // below it), inner rings first. Each side alone, so the halves stay mirrors.
    let halo = R {
        x: root_r.x - 2,
        y: root_r.y - u8::from(t.mode != Mode::Key) as i32,
        w: root_r.w + 4,
        h: root_r.h + 2 * u8::from(t.mode != Mode::Key) as i32,
    };
    for side in [1, -1] {
        // (index, folded cluster?)
        let mut items: Vec<(usize, bool)> = (0..n)
            .filter(|&i| cls[nodes[i].cl].open && cls[nodes[i].cl].side == side)
            .map(|i| (i, false))
            .chain(
                (0..cls.len())
                    .filter(|&c| !cls[c].open && cls[c].side == side)
                    .map(|c| (c, true)),
            )
            .collect();
        let key = |&(i, f): &(usize, bool)| -> (u8, i32) {
            if f {
                (1, (cls[i].dot.1).abs())
            } else {
                (nodes[i].hop, nodes[i].dot.1.abs())
            }
        };
        items.sort_by_key(key);
        let mut taken: Vec<R> = vec![halo];
        for (i, f) in items {
            let mut lab = if f { cls[i].lab } else { nodes[i].lab };
            let dir = if lab.y < 0 { -1 } else { 1 };
            let mut tries = 0;
            while taken.iter().any(|r| r.hits(&lab.pad(1))) && tries < 60 {
                lab.y += dir;
                tries += 1;
            }
            taken.push(lab);
            if f {
                cls[i].lab = lab;
                cls[i].dot.1 = lab.y;
            } else {
                nodes[i].lab = lab;
                nodes[i].dot.1 = lab.y;
            }
        }
    }
    // Vertical extent, then centre it in the band (the root's row moves with it).
    let labs: Vec<(R, i32)> = (0..n)
        .filter(|&i| cls[nodes[i].cl].open)
        .map(|i| (nodes[i].lab, cls[nodes[i].cl].side))
        .chain(cls.iter().filter(|c| !c.open).map(|c| (c.lab, c.side)))
        .collect();
    let top = labs
        .iter()
        .map(|(r, _)| r.y)
        .chain([halo.y])
        .min()
        .unwrap_or(0);
    let bot = labs
        .iter()
        .map(|(r, _)| r.y + r.h)
        .chain([halo.y + halo.h])
        .max()
        .unwrap_or(1);
    let dy = i32::from(band.y) + (i32::from(band.height) - (bot - top)).max(0) / 2 - top;
    for nd in nodes.iter_mut() {
        nd.dot.1 += dy;
        nd.lab.y += dy;
    }
    for cl in cls.iter_mut() {
        cl.dot.1 += dy;
        cl.lab.y += dy;
    }
    let root_r = R {
        y: root_r.y + dy,
        ..root_r
    };
    let inside = |r: R| {
        r.x >= i32::from(band.x)
            && r.x + r.w <= i32::from(band.right())
            && r.y >= i32::from(band.y)
            && r.y + r.h <= i32::from(band.bottom())
    };
    let mirror = |side: i32, r: R| {
        if side < 0 {
            R {
                x: a - r.x - r.w,
                ..r
            }
        } else {
            r
        }
    };
    let tall = bot - top > i32::from(band.height);
    let mut hidden = labs
        .iter()
        .filter(|(r, s)| !inside(mirror(*s, R { y: r.y + dy, ..*r })))
        .count();
    if tall && !force {
        hidden = hidden.max(1);
    }
    Placed {
        mode: t.mode,
        nmax: t.nmax,
        a,
        root: root_r,
        rx: t.rx,
        ry: t.ry,
        r0,
        hops,
        hidden,
    }
}

// ------------------------------------------------------------------------ labels

fn root_w(cx: &Cx, d: &LabDevice, mode: Mode) -> i32 {
    match mode {
        Mode::Key => 3,
        _ => i32::from(2 + orbit::name_w(cx, d, 16, true)),
    }
}

fn ev_w(cx: &Cx, v: &[(String, Style)]) -> i32 {
    v.iter()
        .map(|(t, _)| i32::from(super::width(t, cx.uni())) + 1)
        .sum::<i32>()
        .saturating_sub(1)
}

/// The quiet row under a device: activity sparkline, cpu and the latency of its link
/// (what is wrong instead when it is down).
fn evidence(cx: &Cx, d: &LabDevice, up: Option<&LabEdge>) -> Vec<(String, Style)> {
    let mut v = Vec::new();
    if !cx.up(d) {
        let t = if d.status == DeviceStatus::Unknown {
            "not probed".to_string()
        } else if d.observed_secs == crate::ui::views::NEVER_SEEN {
            "never seen".to_string()
        } else {
            format!("seen {}", super::fmt::age(d.observed_secs))
        };
        v.push((t, cx.ink(Tok::Err, 0.8)));
        return v;
    }
    if let Some(m) = &d.metrics {
        let h: Vec<Option<f32>> = m.cpu_hist.iter().map(|v| Some(*v)).collect();
        v.push((tiles::spark_str(cx, &h, 5), cx.spark_heat(m.cpu)));
        v.push((format!("{:.0}%", m.cpu * 100.0), cx.ink(Tok::Muted, 0.9)));
    }
    if let Some(ms) = up.and_then(|e| e.rtt_ms) {
        v.push((super::ms_text(ms), cx.ink(Tok::Muted, 0.75)));
    }
    v
}

fn dev_w(cx: &Cx, d: &LabDevice, up: Option<&LabEdge>, mode: Mode, nmax: usize) -> i32 {
    match mode {
        Mode::Key => 1 + i32::from(!cx.up(d)),
        Mode::Name => 2 + i32::from(orbit::name_w(cx, d, nmax, true)),
        Mode::Line => {
            let ev = evidence(cx, d, up);
            2 + i32::from(orbit::name_w(cx, d, nmax, true))
                + if ev.is_empty() { 0 } else { 2 + ev_w(cx, &ev) }
        }
        Mode::Full => (2 + i32::from(orbit::name_w(cx, d, nmax, true)))
            .max(2 + ev_w(cx, &evidence(cx, d, up))),
    }
}

fn cl_count(cl: &Cl) -> String {
    format!("+{}", cl.members.len())
}

fn cl_w(cx: &Cx, cl: &Cl, nodes: &[N], mode: Mode) -> i32 {
    let count = cl_count(cl).len() as i32;
    match mode {
        Mode::Key => 1 + count,
        _ => {
            let nm = i32::from(super::width(&fit(&cl.name, 16, cx.uni()), cx.uni()));
            let row0 = 2 + nm + 1 + count;
            if mode == Mode::Full {
                row0.max(
                    2 + 2 * cl.members.len().min(8) as i32 - 1
                        + 2 * i32::from(cl.members.iter().any(|&i| !cx.up(nodes[i].d))),
                )
            } else {
                row0
            }
        }
    }
}

// ------------------------------------------------------------------------ canvas

#[derive(Clone, Copy, PartialEq)]
enum Pat {
    Solid,
    /// Key links: a dot every other step.
    Dots,
    /// Password links.
    Sparse,
    /// Nothing crosses (closed or down).
    Quiet,
    /// Rings and the halo.
    Ring,
}

impl Pat {
    fn on(self, i: usize) -> bool {
        match self {
            Pat::Solid => true,
            Pat::Dots => i % 2 == 0,
            Pat::Sparse => i % 4 == 0,
            Pat::Quiet => i % 6 == 0,
            Pat::Ring => i % 5 == 0,
        }
    }
}

fn pattern(e: &LabEdge) -> Pat {
    if !e.up || orbit::closed(e) {
        Pat::Quiet
    } else if e.forward == Access::Key || e.reverse == Access::Key {
        Pat::Dots
    } else {
        Pat::Sparse
    }
}

/// Braille sub-cell canvas (2x4 dots per cell), one style per cell from its
/// highest-priority stroke; ASCII keeps one character per cell. Strokes are given in
/// right-half coordinates with the side they are drawn on: the left half's dots are the
/// mirror of what the same stroke draws on the right.
struct Canvas {
    r: Rect,
    a: i32,
    bits: Vec<u8>,
    st: Vec<(u8, Style)>,
    asc: Vec<char>,
    mask: Vec<bool>,
}

impl Canvas {
    fn new(r: Rect, a: i32) -> Self {
        let n = usize::from(r.width) * usize::from(r.height);
        Self {
            r,
            a,
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
    fn block(&mut self, r: R) {
        for y in r.y..r.y + r.h {
            for x in r.x..r.x + r.w {
                if let Some(i) = self.cell(x, y) {
                    self.mask[i] = true;
                }
            }
        }
    }
    fn free(&self, x: i32, y: i32) -> bool {
        self.cell(x, y).is_some_and(|i| !self.mask[i])
    }
    fn dot(&mut self, sx: i32, sy: i32, st: Style, pri: u8) {
        let Some(i) = self.cell(sx.div_euclid(2), sy.div_euclid(4)) else {
            return;
        };
        if self.mask[i] {
            return;
        }
        const B: [[u8; 4]; 2] = [[0x01, 0x02, 0x04, 0x40], [0x08, 0x10, 0x20, 0x80]];
        self.bits[i] |= B[sx.rem_euclid(2) as usize][sy.rem_euclid(4) as usize];
        if pri >= self.st[i].0 {
            self.st[i] = (pri, st);
        }
    }
    /// The sub-cell dots a polyline (cell coordinates) passes, in order.
    fn raster(pts: &[P]) -> Vec<(i32, i32)> {
        let mut out: Vec<(i32, i32)> = Vec::new();
        for w in pts.windows(2) {
            let (a, b) = (w[0], w[1]);
            let (dx, dy) = ((b.0 - a.0) * 2.0, (b.1 - a.1) * 4.0);
            let n = (dx.abs().max(dy.abs()) * 1.5).ceil().max(1.0) as i32;
            for k in 0..n {
                let t = k as f32 / n as f32;
                let q = (
                    (a.0 * 2.0 + dx * t).floor() as i32,
                    (a.1 * 4.0 + dy * t).floor() as i32,
                );
                if out.last() != Some(&q) {
                    out.push(q);
                }
            }
        }
        out
    }
    /// Stroke `pts` (right-half coordinates) on `side`, from dot `skip` on.
    fn stroke(&mut self, pts: &[P], side: i32, pat: Pat, st: Style, pri: u8) {
        let a2 = 2 * self.a - 1;
        let flip = |sx: i32| if side < 0 { a2 - sx } else { sx };
        for (i, (sx, sy)) in Self::raster(pts).into_iter().enumerate() {
            if pat.on(i) {
                self.dot(flip(sx), sy, st, pri);
            }
        }
        for (ci, (x, y, slope)) in ascii_path(pts).into_iter().enumerate() {
            let (x, slope) = if side < 0 {
                (
                    self.a - 1 - x,
                    match slope {
                        '/' => '\\',
                        '\\' => '/',
                        c => c,
                    },
                )
            } else {
                (x, slope)
            };
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
    /// Copy `o`'s dots into cells empty here with no dot of ours beside them (rings and
    /// the halo never touch a link).
    fn merge_clear(&mut self, o: &Canvas) {
        let w = i32::from(self.r.width);
        let h = i32::from(self.r.height);
        for i in 0..self.bits.len() {
            if o.bits[i] == 0 {
                continue;
            }
            let (x, y) = (i as i32 % w, i as i32 / w);
            let near = (-1..=1).any(|dy| {
                (-1..=1).any(|dx| {
                    let (nx, ny) = (x + dx, y + dy);
                    nx >= 0 && ny >= 0 && nx < w && ny < h && self.bits[(ny * w + nx) as usize] != 0
                })
            });
            if !near {
                self.bits[i] = o.bits[i];
                self.st[i] = o.st[i];
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

/// The cells an ASCII line visits (one per column where it runs sideways, one per row
/// where it runs up or down), each with its slope character.
fn ascii_path(pts: &[P]) -> Vec<(i32, i32, char)> {
    let (Some(&f), Some(&l)) = (pts.first(), pts.last()) else {
        return Vec::new();
    };
    let wide = (l.0 - f.0).abs() >= (l.1 - f.1).abs() * 2.0;
    let mut cells: Vec<(i32, i32)> = Vec::new();
    for w in pts.windows(2) {
        let (a, b) = (w[0], w[1]);
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let (a0, b0, d) = if wide { (a.0, b.0, dx) } else { (a.1, b.1, dy) };
        if d == 0.0 {
            continue;
        }
        let mut c = (a0.min(b0) - 0.5).ceil();
        let mut seg = Vec::new();
        while c + 0.5 <= a0.max(b0) {
            let t = (c + 0.5 - a0) / d;
            seg.push(if wide {
                (c as i32, (a.1 + dy * t).floor() as i32)
            } else {
                ((a.0 + dx * t).floor() as i32, c as i32)
            });
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

fn ascii_for(pat: Pat, i: usize, slope: char) -> char {
    match pat {
        Pat::Solid => slope,
        Pat::Dots | Pat::Sparse => '.',
        Pat::Quiet => {
            if i % 2 == 0 {
                '.'
            } else {
                ' '
            }
        }
        Pat::Ring => ' ',
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
fn bow(a: P, b: P, k: f32) -> P {
    let (dx, dy) = (b.0 - a.0, (b.1 - a.1) * 2.0);
    let m = ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0);
    (m.0 - dy * k, m.1 + dx * k / 2.0)
}

fn curve(a: P, c: P, b: P) -> Vec<P> {
    (0..=40).map(|i| bez(a, c, b, i as f32 / 40.0)).collect()
}

/// The right half of an ellipse, top to bottom.
fn half_ellipse(c: P, rx: f32, ry: f32) -> Vec<P> {
    let n = ((rx.max(ry * 2.0)) * 4.0).ceil().max(12.0) as usize;
    (0..=n)
        .map(|i| {
            let a = PI * i as f32 / n as f32;
            (c.0 + rx * a.sin(), c.1 - ry * a.cos())
        })
        .collect()
}

fn arrow(cx: &Cx, dx: f32, dy: f32) -> &'static str {
    let a = (dy * 2.0).atan2(dx);
    let k = ((a / (PI / 4.0)).round() as i32).rem_euclid(8);
    if cx.uni() {
        ["→", "↘", "↓", "↙", "←", "↖", "↑", "↗"][k as usize]
    } else if dx >= 0.0 {
        ">"
    } else {
        "<"
    }
}

fn ease(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    // Ease in-out (cubic).
    if t < 0.5 {
        4.0 * t * t * t
    } else {
        1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
    }
}

fn lerp(a: P, b: P, t: f32) -> P {
    (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)
}

// ------------------------------------------------------------------------- scene

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Root,
    Dev(usize),
    Fold(usize),
}

/// One thing drawn: the root, a device or a folded cluster, at its dot (screen).
#[derive(Clone)]
struct It<'a> {
    key: String,
    kind: Kind,
    d: Option<&'a LabDevice>,
    side: i32,
    dot: P,
    w: i32,
    h: i32,
    /// The item its link comes from, the link, and the cluster angle when the link
    /// is part of a bundle leaving the root.
    from: Option<usize>,
    edge: Option<&'a LabEdge>,
    bundle: Option<f32>,
    alpha: f32,
    ghost: bool,
    group: usize,
}

fn scene<'a>(p: &Plan<'a>, root: Option<&'a LabDevice>) -> Vec<It<'a>> {
    let mut v: Vec<It> = vec![It {
        key: root.map_or(String::new(), |d| d.id.clone()),
        kind: Kind::Root,
        d: root,
        side: 1,
        dot: (p.ax(), p.cy()),
        w: p.root.w,
        h: p.root.h,
        from: None,
        edge: None,
        bundle: None,
        alpha: 1.0,
        ghost: false,
        group: usize::MAX,
    }];
    let cell = |side: i32, d: (i32, i32)| p.mirror(side, (d.0 as f32 + 0.5, d.1 as f32 + 0.5));
    let mut at: Vec<Option<usize>> = vec![None; p.nodes.len()];
    // Parents come before their devices in the tree.
    for (i, n) in p.nodes.iter().enumerate() {
        if !p.open(i) {
            continue;
        }
        let cl = &p.cls[n.cl];
        let tops = cl
            .members
            .iter()
            .filter(|&&m| p.nodes[m].parent.is_none() && p.nodes[m].up.is_some())
            .count();
        let side = p.side(i);
        at[i] = Some(v.len());
        v.push(It {
            key: n.d.id.clone(),
            kind: Kind::Dev(i),
            d: Some(n.d),
            side,
            dot: cell(side, n.dot),
            w: n.lab.w,
            h: n.lab.h,
            from: if n.up.is_some() {
                Some(n.parent.and_then(|q| at[q]).unwrap_or(0))
            } else {
                None
            },
            edge: n.up,
            bundle: (n.parent.is_none() && tops > 1).then_some(cl.phi),
            alpha: 1.0,
            ghost: false,
            group: n.cl,
        });
    }
    for (c, cl) in p.cls.iter().enumerate().filter(|(_, c)| !c.open) {
        let first = cl.members.first().map(|&m| &p.nodes[m]);
        v.push(It {
            key: format!("cl:{}", cl.key),
            kind: Kind::Fold(c),
            d: None,
            side: cl.side,
            dot: cell(cl.side, cl.dot),
            w: cl.lab.w,
            h: cl.lab.h,
            from: first.and_then(|n| n.up).map(|_| 0),
            edge: first.and_then(|n| n.up),
            bundle: None,
            alpha: 1.0,
            ghost: false,
            group: c,
        });
    }
    v
}

/// The scene part-way from `prev` to `cur`: shared items glide, new ones grow out of
/// the folded node they were in and fade in, gone ones shrink into theirs and fade out.
fn tween<'a>(prev: &[It<'a>], cur: Vec<It<'a>>, t: f32, pcls: &Plan, ccls: &Plan) -> Vec<It<'a>> {
    let fold_of = |p: &Plan, id: &str| -> Option<String> {
        let n = p.nodes.iter().find(|n| n.d.id == id)?;
        let cl = &p.cls[n.cl];
        (!cl.open).then(|| format!("cl:{}", cl.key))
    };
    let find = |v: &[It], key: &str| v.iter().position(|x| x.key == key);
    let mut out = cur.clone();
    for it in out.iter_mut() {
        let start = find(prev, &it.key)
            .or_else(|| fold_of(pcls, &it.key).and_then(|k| find(prev, &k)))
            .or_else(|| match it.kind {
                // A cluster folding: from its members' midpoint.
                Kind::Fold(c) => ccls.cls[c]
                    .members
                    .iter()
                    .find_map(|m| find(prev, &ccls.nodes[*m].d.id)),
                _ => None,
            });
        match start {
            Some(s) => {
                let shared = prev[s].key == it.key;
                it.dot = lerp(prev[s].dot, it.dot, t);
                if !shared {
                    it.alpha = t;
                }
            }
            None => it.alpha = t,
        }
    }
    for g in prev.iter().filter(|g| find(&cur, &g.key).is_none()) {
        let to = fold_of(ccls, &g.key)
            .and_then(|k| find(&cur, &k))
            .map_or(g.dot, |j| cur[j].dot);
        let mut g = g.clone();
        g.dot = lerp(g.dot, to, t);
        g.alpha = 1.0 - t;
        g.ghost = true;
        g.from = None;
        out.push(g);
    }
    out
}

/// A label's cells on screen from its dot: right of it on the right half, left of it on
/// the left (the root is centred on its dot).
fn label_rect(it: &It) -> R {
    let (x, y) = (it.dot.0.floor() as i32, it.dot.1.floor() as i32);
    match it.kind {
        Kind::Root => R {
            x: (it.dot.0 - it.w as f32 / 2.0).round() as i32,
            y,
            w: it.w,
            h: it.h,
        },
        _ if it.side < 0 => R {
            x: x + 1 - it.w,
            y,
            w: it.w,
            h: it.h,
        },
        _ => R {
            x,
            y,
            w: it.w,
            h: it.h,
        },
    }
}

// ------------------------------------------------------------------------- draw

/// Rows the map's drawing takes in `band`: the most any selection needs (a cluster
/// blooming never moves the rows under the map).
pub(super) fn height(cx: &Cx, band: Rect) -> u16 {
    let root = cx.s.viewer.as_str();
    let mut ids = vec![root.to_string()];
    let tree = orbit::tree(cx, root, 3);
    let mut nodes = nodes_of(tree);
    for cl in cluster(cx, &mut nodes) {
        ids.extend(cl.members.first().map(|&m| nodes[m].d.id.clone()));
    }
    ids.iter()
        .map(|sel| rows_of(&plan(cx, band, root, sel)))
        .max()
        .unwrap_or(1) as u16
}

fn nodes_of(tree: Vec<orbit::Node>) -> Vec<N> {
    tree.into_iter()
        .map(|t| N {
            d: t.d,
            hop: t.hop,
            up: t.up,
            parent: t.parent,
            cl: 0,
            phi: 0.0,
            dot: (0, 0),
            lab: R::default(),
        })
        .collect()
}

fn rows_of(p: &Plan) -> i32 {
    let labs = p
        .nodes
        .iter()
        .enumerate()
        .filter(|(i, _)| p.open(*i))
        .map(|(_, n)| n.lab)
        .chain(p.cls.iter().filter(|c| !c.open).map(|c| c.lab));
    let halo = i32::from(p.mode != Mode::Key);
    let (lo, hi) = labs.fold(
        (p.root.y - halo, p.root.y + p.root.h + halo),
        |(lo, hi), r| (lo.min(r.y), hi.max(r.y + r.h)),
    );
    hi - lo
}

/// Device ids in a stable order, whatever is selected or folded: the root, then the
/// right half top to bottom, then the left half top to bottom, cluster by cluster.
pub(super) fn order(cx: &Cx, _band: Rect) -> Vec<String> {
    order_ids(cx)
}

fn order_ids(cx: &Cx) -> Vec<String> {
    let root = cx.s.viewer.as_str();
    let mut nodes = nodes_of(orbit::tree(cx, root, 3));
    let cls = cluster(cx, &mut nodes);
    let mut out = vec![root.to_string()];
    for side in [1, -1] {
        for cl in cls.iter().filter(|c| c.side == side) {
            out.extend(cl.members.iter().map(|&m| nodes[m].d.id.clone()));
        }
    }
    out
}

/// Draw the hub's map in `band`, `sel` selected (`focus`: the map has the keys).
pub(super) fn draw(cx: &Cx, buf: &mut Buffer, band: Rect, sel: &str, focus: bool) {
    if band.width < 12 || band.height < 1 {
        return;
    }
    let s = cx.s;
    let root = s.viewer.as_str();
    let rootd = s.device(root);
    let p = plan(cx, band, root, sel);
    let mo = cx.c.motion;
    let from = mo
        .from
        .and_then(|i| s.devices.get(usize::from(i)))
        .map(|d| d.id.as_str())
        .filter(|f| *f != sel && mo.moving());
    let t = ease(mo.at());
    let mut items = scene(&p, rootd);
    if let Some(f) = from {
        let q = plan(cx, band, root, f);
        let prev = scene(&q, rootd);
        if prev.iter().map(|x| &x.key).ne(items.iter().map(|x| &x.key))
            || prev.iter().zip(&items).any(|(a, b)| a.dot != b.dot)
        {
            items = tween(&prev, items, t, &q, &p);
        }
    }
    let mark = cx.hit_mark();
    let sel_item = items.iter().position(|x| !x.ghost && x.key == sel);
    // The selection's route to the root lights up.
    let mut lit: Vec<usize> = Vec::new();
    let mut cur = sel_item;
    while let Some(i) = cur {
        lit.push(i);
        cur = items[i].from.filter(|&f| f != i);
        if cur == Some(0) {
            break;
        }
    }

    let mut cv = Canvas::new(band, p.a);
    for it in &items {
        let r = label_rect(it);
        let pad = if it.kind == Kind::Root { 2 } else { 1 };
        if it.alpha > 0.3 {
            cv.block(r.pad(pad));
        }
    }
    let rl = |side: i32, q: P| p.mirror(side, q);
    let mut arrows: Vec<(Vec<P>, &'static str, Style)> = Vec::new();
    let mut taps: Vec<(P, &LabEdge)> = Vec::new();
    for (i, it) in items.iter().enumerate() {
        let (Some(f), Some(e)) = (it.from, it.edge) else {
            continue;
        };
        if it.ghost {
            continue;
        }
        let on = lit.contains(&i);
        let a = rl(it.side, items[f].dot);
        let b = rl(it.side, it.dot);
        // A bundle: each device's curve is drawn toward its cluster's fork (most of
        // the way), so a cluster's links leave the root together and fan out.
        let c = match it.bundle {
            Some(phi) => {
                let fork = (
                    p.ax() + phi.sin() * (p.r0 * 0.6 + p.rx * 0.45),
                    p.cy() - phi.cos() * p.ry * 0.45,
                );
                lerp(bow(a, b, 0.12), fork, 0.7)
            }
            None => bow(a, b, 0.12),
        };
        let pts = curve(a, c, b);
        // A bundle's branches wear their cluster's hue until lit.
        let tok = if it.bundle.is_some() && !on {
            p.cls[it.group].tok
        } else {
            orbit::edge_tok(cx, e)
        };
        let alpha = if on { 1.0 } else { 0.55 } * it.alpha.max(0.2);
        let mut st = cx.ink(tok, alpha);
        if on {
            st = st.add_modifier(Modifier::BOLD);
        }
        let pat = if on && pattern(e) == Pat::Dots {
            Pat::Solid
        } else {
            pattern(e)
        };
        cv.stroke(&pts, it.side, pat, st, if on { 6 } else { 4 });
        let screen = |q: P| p.mirror(it.side, q);
        if e.up && e.one_way() && (on || it.bundle.is_none()) {
            // Access flows from the parent's end toward this device, or back.
            let parent_id = items[f].d.map(|d| d.id.as_str()).unwrap_or(root);
            let fwd = (e.forward != Access::Denied) == (parent_id == e.from);
            let (t0, t1) = if fwd { (0.62, 0.68) } else { (0.38, 0.32) };
            let (p0, p1) = (screen(bez(a, c, b, t0)), screen(bez(a, c, b, t1)));
            let spots = [t0, 0.5, (t0 + 0.5) / 2.0]
                .map(|t| screen(bez(a, c, b, t)))
                .to_vec();
            arrows.push((spots, arrow(cx, p1.0 - p0.0, p1.1 - p0.1), st));
        }
        taps.push((screen(bez(a, c, b, 0.5)), e));
    }
    // Rings and the halo: quiet dots that never touch a link or a label.
    if cx.uni() && p.mode != Mode::Key {
        let mut rc = Canvas::new(band, p.a);
        rc.mask.clone_from(&cv.mask);
        let faint = cx.ink(Tok::Faint, if cx.dark() { 0.75 } else { 0.9 });
        let c = (p.ax(), p.cy());
        let mut ellipses = vec![(p.root.w as f32 / 2.0 + 2.5, 1.6)];
        if cx.size() >= Size::M {
            for h in 1..=p.hops.min(2) {
                ellipses.push((p.r0 + p.rx * ring(h), p.ry * ring(h)));
            }
        }
        for (rx, ry) in ellipses {
            let pts = half_ellipse(c, rx, ry);
            for side in [1, -1] {
                rc.stroke(&pts, side, Pat::Ring, faint, 1);
            }
        }
        cv.merge_clear(&rc);
    }
    // The highlight's glide: a short bright run along the way between the two.
    if let (Some(f), true) = (from, t < 1.0) {
        let a = items
            .iter()
            .find(|x| x.key == f)
            .or_else(|| {
                let n = p.nodes.iter().find(|n| n.d.id == f)?;
                let k = format!("cl:{}", p.cls[n.cl].key);
                items.iter().find(|x| x.key == k)
            })
            .map(|x| x.dot);
        let b = sel_item.map(|i| items[i].dot);
        if let (Some(a), Some(b)) = (a, b) {
            let c = lerp(lerp(a, b, 0.5), (p.ax(), p.cy()), 0.35);
            let pts = curve(a, c, b);
            let dots = Canvas::raster(&pts);
            let head = (t * dots.len() as f32) as usize;
            let st = cx.st(Tok::Accent).add_modifier(Modifier::BOLD);
            for &(sx, sy) in dots.iter().take(head + 1).skip(head.saturating_sub(5)) {
                cv.dot(sx, sy, st, 9);
            }
        }
    }
    cv.flush(cx, buf);
    for (spots, g, st) in arrows {
        let gw = i32::from(super::width(g, cx.uni()));
        if let Some(q) = spots.iter().find(|q| {
            let (x, y) = (q.0.floor() as i32, q.1.floor() as i32);
            (x..x + gw).all(|xx| cv.free(xx, y))
        }) {
            cx.put(buf, q.0 as u16, q.1 as u16, gw as u16, g, st);
        }
    }
    if !cx.touch() {
        for (q, e) in taps {
            let r = Rect::new((q.0 as u16).saturating_sub(1), q.1 as u16, 3, 1);
            if cv.free(i32::from(r.x) + 1, i32::from(r.y)) {
                cx.hit(r, format!("edge:{}", e.id));
            }
        }
    }
    // Labels, then targets and moves.
    let mut nav: Vec<(String, Rect, usize, P)> = Vec::new();
    for it in &items {
        let r = label_rect(it);
        let pick = (Some(it.key.as_str()) == Some(sel) && !it.ghost).then_some(focus);
        let was = from == Some(it.key.as_str()) && !it.ghost;
        let fade = if pick.is_some() {
            t
        } else if was {
            1.0 - t
        } else {
            0.0
        };
        label(cx, buf, &p, it, r, pick, fade);
        if it.ghost || it.alpha < 0.5 {
            continue;
        }
        let target = match it.kind {
            Kind::Root | Kind::Dev(_) => it.key.clone(),
            // A folded cluster opens on its first device.
            Kind::Fold(c) => p.cls[c]
                .members
                .first()
                .map(|&m| p.nodes[m].d.id.clone())
                .unwrap_or_default(),
        };
        let rr = r.rect().intersection(cx.area);
        let rr =
            Rect::new(rr.x.saturating_sub(1), rr.y, rr.width + 2, rr.height).intersection(cx.area);
        cx.nav(rr, format!("device:{target}"));
        cx.hit(rr, format!("device:{target}"));
        nav.push((target, rr, it.group, it.dot));
    }
    if p.hidden > 0 {
        let t = format!("+{} more", p.hidden);
        let w = super::width(&t, cx.uni());
        let x = band.right().saturating_sub(w + 1);
        let y = band.bottom() - 1;
        cx.put(buf, x, y, w, &t, cx.st(Tok::Muted));
        cx.hit(Rect::new(x, y, w, 1), "palette:devices");
    }
    let spots: Vec<spatial::Item> = nav
        .iter()
        .map(|(_, _, g, at)| spatial::Item { at: *at, group: *g })
        .collect();
    let mv = spatial::moves(&spots);
    cx.set_moves(
        nav.iter()
            .zip(mv)
            .map(|((id, ..), m)| {
                (
                    format!("device:{id}"),
                    m.map(|t| t.map(|j| format!("device:{}", nav[j].0))),
                )
            })
            .collect(),
    );
    cx.unclash(mark, true);
}

/// Mix the accent in by `k` (the focus fade), else the plain style.
fn faded(cx: &Cx, base: Tok, base_a: f32, k: f32) -> Style {
    if k <= 0.0 {
        return cx.ink(base, base_a);
    }
    if cx.tc() {
        let c = super::mix(
            super::mix(cx.bg0(), cx.rgb(base), base_a),
            cx.rgb(Tok::Accent),
            k,
        );
        cx.rgb_fg(c)
    } else if k >= 0.5 {
        cx.st(Tok::Accent)
    } else {
        cx.ink(base, base_a)
    }
}

fn label(cx: &Cx, buf: &mut Buffer, p: &Plan, it: &It, r: R, pick: Option<bool>, fade: f32) {
    let area = cx.area;
    if r.x < i32::from(area.x)
        || r.y < i32::from(area.y)
        || r.x + r.w > i32::from(area.right())
        || r.y + r.h > i32::from(area.bottom())
    {
        return;
    }
    let (x, y, w) = (r.x as u16, r.y as u16, r.w as u16);
    let left = it.side < 0 && it.kind != Kind::Root;
    let a = it.alpha.clamp(0.0, 1.0);
    let sel_k = match pick {
        Some(true) => fade,
        Some(false) => fade * 0.7,
        None => fade,
    };
    // Brackets where colour cannot show the selection.
    if pick.is_some() && (!cx.uni() || !cx.c.has_color()) {
        let st = cx.st(Tok::Accent);
        cx.put(buf, x.saturating_sub(1), y, 1, "[", st);
        cx.put(buf, x + w, y, 1, "]", st);
    }
    match it.kind {
        Kind::Root => {
            let Some(d) = it.d else { return };
            if p.mode == Mode::Key {
                cx.put(buf, x, y, 1, cx.g("◉", "@"), cx.st(Tok::Accent));
                let st = faded(cx, Tok::Fg, 1.0, sel_k)
                    .add_modifier(Modifier::BOLD | Modifier::UNDERLINED);
                cx.put(buf, x + 2, y, 1, &d.hotkey.to_string(), st);
                return;
            }
            let (g, gst) = cx.status(d.status);
            let g = if matches!(d.status, DeviceStatus::Online | DeviceStatus::Local) {
                cx.g("◉", "@")
            } else {
                g
            };
            cx.put(buf, x, y, 1, g, gst);
            let st = faded(cx, Tok::Fg, 1.0, sel_k).add_modifier(Modifier::BOLD);
            orbit::name(cx, buf, x + 2, y, d, 16, st, true);
            if p.mode == Mode::Full {
                let mut ev = vec![("you".to_string(), cx.st(Tok::Accent))];
                ev.extend(evidence(cx, d, None));
                let ew = ev_w(cx, &ev);
                let mut xx = (it.dot.0 - ew as f32 / 2.0).round().max(0.0) as u16;
                for (t, st) in ev {
                    xx += cx.put(buf, xx, y + 1, ew as u16, &t, st) + 1;
                }
            }
        }
        Kind::Dev(i) => {
            let Some(d) = it.d else { return };
            let up = cx.up(d);
            let status = if d.status == DeviceStatus::Local {
                DeviceStatus::Online
            } else {
                d.status
            };
            let (g, gst) = cx.status(status);
            let gst = if a < 1.0 {
                gst.patch(cx.ink(Tok::Faint, 1.0))
            } else {
                gst
            };
            if p.mode == Mode::Key {
                let hue = Tok::Id(cx.s.hue(&d.id));
                let st =
                    faded(cx, hue, a, sel_k).add_modifier(Modifier::BOLD | Modifier::UNDERLINED);
                let (lx, gx) = if left { (x + w - 1, x) } else { (x, x + 1) };
                cx.put(buf, lx, y, 1, &d.hotkey.to_string(), st);
                if !up {
                    cx.put(buf, gx, y, 1, g, gst);
                }
                return;
            }
            let base = if up {
                (Tok::Fg, a)
            } else {
                (Tok::Muted, 0.75 * a)
            };
            let mut nst = faded(cx, base.0, base.1, sel_k);
            if up || pick.is_some() {
                nst = nst.add_modifier(Modifier::BOLD);
            }
            let nw = orbit::name_w(cx, d, p.nmax, true);
            let (gx, nx) = if left {
                (x + w - 1, (x + w).saturating_sub(2 + nw))
            } else {
                (x, x + 2)
            };
            cx.put(buf, gx, y, 1, g, gst);
            orbit::name(cx, buf, nx, y, d, p.nmax, nst, true);
            if (p.mode == Mode::Full && r.h > 1) || p.mode == Mode::Line {
                let ev = evidence(cx, d, p.nodes[i].up);
                let ew = ev_w(cx, &ev) as u16;
                let (mut xx, y) = match (p.mode, left) {
                    (Mode::Line, true) => (x, y),
                    (Mode::Line, false) => (x + w - ew, y),
                    (_, true) => ((x + w).saturating_sub(2 + ew), y + 1),
                    (_, false) => (x + 2, y + 1),
                };
                for (t, st) in ev {
                    let st = if a < 1.0 { cx.ink(Tok::Faint, 1.0) } else { st };
                    xx += cx.put(buf, xx, y, ew, &t, st) + 1;
                }
            }
        }
        Kind::Fold(c) => {
            let cl = &p.cls[c];
            let gl = cx.g("◍", "*");
            let gst = cx.ink(cl.tok, a);
            let count = cl_count(cl);
            let cst = cx.ink(Tok::Fg, a).add_modifier(Modifier::BOLD);
            if p.mode == Mode::Key {
                let (gx, nx) = if left { (x + w - 1, x) } else { (x, x + 1) };
                cx.put(buf, gx, y, 1, gl, gst);
                cx.put(buf, nx, y, w - 1, &count, cst);
                return;
            }
            let nm = fit(&cl.name, 16, cx.uni());
            let nw = super::width(&nm, cx.uni());
            let cw = count.len() as u16;
            let nst = cx.ink(cl.tok, 0.9 * a);
            if left {
                cx.put(buf, x + w - 1, y, 1, gl, gst);
                let nx = (x + w).saturating_sub(2 + nw);
                cx.put(buf, nx, y, nw, &nm, nst);
                cx.put(buf, nx.saturating_sub(cw + 1), y, cw, &count, cst);
            } else {
                cx.put(buf, x, y, 1, gl, gst);
                cx.put(buf, x + 2, y, nw, &nm, nst);
                cx.put(buf, x + 3 + nw, y, cw, &count, cst);
            }
            if p.mode == Mode::Full && r.h > 1 {
                // Its members' letters (keys that open it on them), downs counted.
                let mut segs: Vec<(String, Style)> = cl
                    .members
                    .iter()
                    .take(8)
                    .map(|&m| {
                        let d = p.nodes[m].d;
                        let st = if cx.up(d) {
                            cx.ink(Tok::Id(cx.s.hue(&d.id)), 0.9 * a)
                        } else {
                            cx.ink(Tok::Muted, 0.6 * a)
                        };
                        (d.hotkey.to_string(), st)
                    })
                    .collect();
                if cl.members.iter().any(|&m| !cx.up(p.nodes[m].d)) {
                    segs.push((cx.g("✕", "!").into(), cx.ink(Tok::Err, 0.8 * a)));
                }
                let ew = ev_w(cx, &segs) as u16;
                let mut xx = if left {
                    (x + w).saturating_sub(2 + ew)
                } else {
                    x + 2
                };
                for (t, st) in segs {
                    xx += cx.put(buf, xx, y + 1, 2, &t, st) + 1;
                }
            }
        }
    }
}
