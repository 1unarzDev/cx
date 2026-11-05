//! Layered (Sugiyama-lite) layout for the Network hub (spec 10.2).
//!
//! Rank = BFS hop distance from the viewer; order within a rank = barycenter
//! sweeps seeded by the previous layout's order; mesh members are kept
//! contiguous and wrapped in an enclosure; coordinates come from an isotonic
//! (pool-adjacent-violators) fit to each node's neighbours. Overflow collapses
//! LAN clusters, then folds ranks >= 3 and long ranks into `+N more` pills, and
//! finally falls back to the outline view.
use super::hit::HitMap;
use super::model::*;
use super::route::{self, Route};
use ratatui::layout::Rect;
use std::collections::{BTreeMap, VecDeque};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SizeClass {
    XS,
    S,
    M,
    L,
}
impl SizeClass {
    pub fn for_width(w: u16) -> Self {
        match w {
            0..=59 => SizeClass::XS,
            60..=99 => SizeClass::S,
            100..=159 => SizeClass::M,
            _ => SizeClass::L,
        }
    }
    /// Node card size (width, height) including its border.
    pub fn node_size(self) -> (u16, u16) {
        match self {
            SizeClass::L => (20, 4),
            SizeClass::M => (18, 4),
            SizeClass::S | SizeClass::XS => (17, 3),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GlyphSet {
    Unicode,
    Ascii,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CacheKey {
    pub topology: u64,
    pub width: u16,
    pub height: u16,
    pub class: SizeClass,
    pub glyphs: GlyphSet,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Graph,
    Outline,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Orientation {
    /// Ranks are columns (L/M/S when they fit).
    LeftRight,
    /// Ranks are row bands (narrow areas with many ranks).
    TopBottom,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Placed {
    pub node: usize,
    pub rect: Rect,
    pub rank: usize,
}

/// A `+N more` pill standing in for hidden nodes.
#[derive(Clone, Debug, PartialEq)]
pub struct More {
    pub rect: Rect,
    pub rank: usize,
    pub hidden: Vec<usize>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Enclosure {
    pub mesh: String,
    pub rect: Rect,
    /// The first enclosure of a mesh carries the title.
    pub title: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct OutlineRow {
    pub node: usize,
    pub depth: usize,
    /// Edge from the parent row.
    pub edge: Option<usize>,
    /// Per ancestor level below the root: whether a `│` guide continues.
    pub guides: Vec<bool>,
    pub last: bool,
}

#[derive(Clone, Debug)]
pub struct GraphLayout {
    pub key: CacheKey,
    pub mode: Mode,
    pub orientation: Orientation,
    pub class: SizeClass,
    pub width: u16,
    pub height: u16,
    pub nodes: Vec<Placed>,
    pub more: Vec<More>,
    pub enclosures: Vec<Enclosure>,
    pub routes: Vec<Route>,
    pub outline: Vec<OutlineRow>,
    /// node id -> (rank, position) for seeding the next layout.
    pub order: BTreeMap<String, (usize, usize)>,
    /// Row reserved for the legend, if any.
    pub legend: Option<u16>,
    pub hits: HitMap,
}

impl GraphLayout {
    pub fn placed(&self, node: usize) -> Option<&Placed> {
        self.nodes.iter().find(|p| p.node == node)
    }
    pub fn route_for(&self, edge: usize) -> Option<&Route> {
        self.routes.iter().find(|r| r.edge == edge)
    }
}

/// Graphs with more visible nodes than this use the outline (spec 6.4).
pub const MAX_GRAPH_NODES: usize = 24;
const LAN_EXTRA_RANK: usize = 1;

/// Computes the layout for an area of `width` x `height` cells.
pub fn compute(
    g: &Graph,
    width: u16,
    height: u16,
    glyphs: GlyphSet,
    prev: Option<&GraphLayout>,
) -> GraphLayout {
    let class = SizeClass::for_width(width);
    let key = CacheKey {
        topology: g.topology_hash(),
        width,
        height,
        class,
        glyphs,
    };
    if class != SizeClass::XS && g.nodes.len() <= MAX_GRAPH_NODES {
        if let Some(l) = graph_with_overflow(g, key, prev) {
            return l;
        }
    }
    outline_layout(g, key)
}

/// Graph mode with the overflow ladder; `None` when even the folded graph does not fit.
pub(super) fn graph_with_overflow(
    g: &Graph,
    key: CacheKey,
    prev: Option<&GraphLayout>,
) -> Option<GraphLayout> {
    let has_neighbors = g.nodes.iter().any(|n| n.kind == NodeKind::Neighbor);
    let ladder: &[(bool, bool, bool)] = &[
        (false, false, false),
        (true, false, false),
        (true, true, false),
        (true, true, true),
    ];
    for &(hide_neighbors, fold_deep, trim) in ladder {
        if hide_neighbors && !has_neighbors && !fold_deep {
            continue;
        }
        if let Some(l) = try_graph(g, key, prev, hide_neighbors, fold_deep, trim) {
            return Some(l);
        }
    }
    None
}

fn neighbors_of(g: &Graph, visible: &[bool]) -> Vec<Vec<usize>> {
    let mut adj = vec![Vec::new(); g.nodes.len()];
    let index: BTreeMap<&str, usize> = g
        .nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.id.as_str(), i))
        .collect();
    for e in &g.edges {
        if let (Some(&a), Some(&b)) = (index.get(e.from.as_str()), index.get(e.to.as_str())) {
            if a != b && visible[a] && visible[b] {
                adj[a].push(b);
                adj[b].push(a);
            }
        }
    }
    for a in adj.iter_mut() {
        a.sort_unstable();
        a.dedup();
    }
    adj
}

fn bfs_ranks(g: &Graph, adj: &[Vec<usize>], visible: &[bool]) -> Vec<usize> {
    let root = g.viewer().unwrap_or(0);
    let mut rank = vec![usize::MAX; g.nodes.len()];
    let mut q = VecDeque::new();
    if !g.nodes.is_empty() {
        rank[root] = 0;
        q.push_back(root);
    }
    while let Some(a) = q.pop_front() {
        for &b in &adj[a] {
            if rank[b] == usize::MAX {
                rank[b] = rank[a] + 1;
                q.push_back(b);
            }
        }
    }
    for (i, r) in rank.iter_mut().enumerate() {
        if *r == usize::MAX && visible[i] {
            *r = 1;
        }
    }
    let _ = LAN_EXTRA_RANK;
    rank
}

/// Default ordering key for nodes without a previous position.
fn default_key<'a>(
    g: &'a Graph,
    i: usize,
    mesh_rank: &BTreeMap<&'a str, usize>,
) -> (u8, usize, &'a str, NodeKind, &'a str, &'a str) {
    let n = &g.nodes[i];
    let kind_order = match n.kind {
        NodeKind::Viewer | NodeKind::Device | NodeKind::Jump => 0,
        NodeKind::LanCluster => 2,
        NodeKind::Neighbor => 3,
    };
    match n.mesh.as_deref() {
        Some(m) => (
            kind_order.min(1),
            mesh_rank.get(m).copied().unwrap_or(0),
            m,
            n.kind,
            n.label.as_str(),
            n.id.as_str(),
        ),
        None => (
            kind_order.max(1),
            usize::MAX,
            "",
            n.kind,
            n.label.as_str(),
            n.id.as_str(),
        ),
    }
}

fn try_graph(
    g: &Graph,
    key: CacheKey,
    prev: Option<&GraphLayout>,
    hide_neighbors: bool,
    fold_deep: bool,
    trim: bool,
) -> Option<GraphLayout> {
    let (w, h) = (key.width, key.height);
    let class = key.class;
    let n = g.nodes.len();
    let mut visible: Vec<bool> = g
        .nodes
        .iter()
        .map(|x| !(hide_neighbors && x.kind == NodeKind::Neighbor))
        .collect();
    let adj0 = neighbors_of(g, &visible);
    let rank = bfs_ranks(g, &adj0, &visible);
    let mut more: Vec<(usize, Vec<usize>)> = Vec::new(); // (rank, hidden)
    if fold_deep {
        let deep: Vec<usize> = (0..n).filter(|&i| visible[i] && rank[i] >= 3).collect();
        if !deep.is_empty() {
            for &i in &deep {
                visible[i] = false;
            }
            more.push((3, deep));
        }
    }
    let adj = neighbors_of(g, &visible);
    let max_rank = (0..n)
        .filter(|&i| visible[i])
        .map(|i| rank[i])
        .chain(more.iter().map(|m| m.0))
        .max()
        .unwrap_or(0);
    let ranks_n = max_rank + 1;
    let (nw, nh) = class.node_size();

    // Orientation and rank-axis spacing.
    const MX: u16 = 3;
    let lr_need = ranks_n as u16 * nw + (ranks_n.saturating_sub(1) as u16) * 6 + 2 * MX;
    let orientation = if lr_need <= w {
        Orientation::LeftRight
    } else {
        let tb_need = ranks_n as u16 * nh + ranks_n.saturating_sub(1) as u16 * 4 + 2;
        if tb_need > h {
            return None;
        }
        Orientation::TopBottom
    };
    let legend = (matches!(class, SizeClass::L | SizeClass::M) && h >= 16).then(|| h - 1);
    let (lo, hi) = match orientation {
        Orientation::LeftRight => (1u16, h.saturating_sub(1 + legend.is_some() as u16)),
        Orientation::TopBottom => (MX, w.saturating_sub(MX)),
    };
    if hi <= lo {
        return None;
    }
    // Rank lists.
    let mut lists: Vec<Vec<usize>> = vec![Vec::new(); ranks_n];
    for i in 0..n {
        if visible[i] {
            lists[rank[i]].push(i);
        }
    }
    let mut mesh_rank: BTreeMap<&str, usize> = BTreeMap::new();
    for i in (0..n).filter(|&i| visible[i]) {
        if let Some(m) = g.nodes[i].mesh.as_deref() {
            let e = mesh_rank.entry(m).or_insert(usize::MAX);
            *e = (*e).min(rank[i]);
        }
    }
    let prev_pos = |i: usize, r: usize| -> Option<usize> {
        prev.and_then(|p| p.order.get(&g.nodes[i].id))
            .filter(|(pr, _)| *pr == r)
            .map(|(_, pos)| *pos)
    };
    for (r, list) in lists.iter_mut().enumerate() {
        list.sort_by(|&a, &b| {
            match (prev_pos(a, r), prev_pos(b, r)) {
                (Some(x), Some(y)) => x.cmp(&y),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => std::cmp::Ordering::Equal,
            }
            .then_with(|| default_key(g, a, &mesh_rank).cmp(&default_key(g, b, &mesh_rank)))
        });
    }
    // Barycenter sweeps (4 down/up), ties keep the current (seeded) order.
    let mut pos = vec![0f64; n];
    let set_pos = |lists: &Vec<Vec<usize>>, pos: &mut Vec<f64>| {
        for list in lists {
            for (p, &i) in list.iter().enumerate() {
                pos[i] = p as f64;
            }
        }
    };
    set_pos(&lists, &mut pos);
    for sweep in 0..4 {
        let down = sweep % 2 == 0;
        let order: Vec<usize> = if down {
            (1..ranks_n).collect()
        } else {
            (0..ranks_n.saturating_sub(1)).rev().collect()
        };
        for r in order {
            let adjacent = if down { r - 1 } else { r + 1 };
            let bary: Vec<(usize, f64)> = lists[r]
                .iter()
                .map(|&i| {
                    let ns: Vec<f64> = adj[i]
                        .iter()
                        .filter(|&&j| rank[j] == adjacent && visible[j])
                        .map(|&j| pos[j])
                        .collect();
                    let b = if ns.is_empty() {
                        pos[i]
                    } else {
                        ns.iter().sum::<f64>() / ns.len() as f64
                    };
                    (i, b)
                })
                .collect();
            let mut sorted = bary.clone();
            sorted.sort_by(|a, b| a.1.total_cmp(&b.1).then(pos[a.0].total_cmp(&pos[b.0])));
            lists[r] = sorted.into_iter().map(|x| x.0).collect();
            for (p, &i) in lists[r].iter().enumerate() {
                pos[i] = p as f64;
            }
        }
    }
    // Mesh contiguity: stable partition, groups ordered by first member.
    for list in lists.iter_mut() {
        let mut groups: Vec<(Option<&str>, Vec<usize>)> = Vec::new();
        for &i in list.iter() {
            let m = g.nodes[i].mesh.as_deref();
            match m.and_then(|m| groups.iter_mut().find(|(k, _)| *k == Some(m))) {
                Some(group) => group.1.push(i),
                None => groups.push((m, vec![i])),
            }
        }
        *list = groups.into_iter().flat_map(|g| g.1).collect();
    }
    // Items: node indices, and `n + k` for pill k.
    let mut items: Vec<Vec<usize>> = lists;
    let mut pills: Vec<(usize, Vec<usize>)> = Vec::new();
    for (r, hidden) in more {
        items[r].push(n + pills.len());
        pills.push((r, hidden));
    }
    let (ext_o, tight_gap, pad_o) = match orientation {
        Orientation::LeftRight => (nh, 1u16, 1u16),
        Orientation::TopBottom => (nw, 2u16, 2u16),
    };
    let mesh_of = |it: usize| -> Option<&str> {
        if it < n {
            g.nodes[it].mesh.as_deref()
        } else {
            None
        }
    };
    let gap_with = |base: u16, a: usize, b: usize| -> u16 {
        match (mesh_of(a), mesh_of(b)) {
            (Some(x), Some(y)) if x == y => base,
            (x, y) => base + pad_o * (x.is_some() as u16 + y.is_some() as u16),
        }
    };
    let extent_with = |base: u16, list: &[usize]| -> u16 {
        let mut t = 0u16;
        for (k, &it) in list.iter().enumerate() {
            t = t.saturating_add(ext_o);
            if k > 0 {
                t = t.saturating_add(gap_with(base, list[k - 1], it));
            }
        }
        t
    };
    let room = hi - lo;
    // Breathing room between cards on large areas when every rank still fits.
    let base_gap = if orientation == Orientation::LeftRight
        && matches!(class, SizeClass::L | SizeClass::M)
        && items.iter().all(|l| extent_with(tight_gap + 1, l) <= room)
    {
        tight_gap + 1
    } else {
        tight_gap
    };
    let gap = |a: usize, b: usize| gap_with(base_gap, a, b);
    let extent = |list: &[usize]| extent_with(base_gap, list);
    for r in 0..items.len() {
        if extent(&items[r]) <= room {
            continue;
        }
        if !trim {
            return None;
        }
        // Keep the longest prefix that fits next to a pill.
        let mut keep = items[r].clone();
        let mut hidden = Vec::new();
        while !keep.is_empty() && extent(&keep) + base_gap + ext_o > room {
            let x = keep.pop().unwrap();
            if x < n {
                hidden.push(x);
            } else {
                hidden.extend(pills[x - n].1.iter().copied());
                pills[x - n].1.clear();
            }
        }
        if keep.is_empty() {
            return None;
        }
        for &x in &hidden {
            visible[x] = false;
        }
        hidden.sort_unstable();
        keep.push(n + pills.len());
        pills.push((r, hidden));
        items[r] = keep;
    }
    let pills_live: Vec<bool> = pills.iter().map(|p| !p.1.is_empty()).collect();
    for list in items.iter_mut() {
        list.retain(|&it| it < n || pills_live[it - n]);
    }
    // Rank-axis coordinates.
    let ranks_n = items.len();
    let rank_coord: Vec<u16> = match orientation {
        Orientation::LeftRight => {
            let free = w.saturating_sub(2 * MX + ranks_n as u16 * nw);
            let g = if ranks_n > 1 {
                (free / (ranks_n as u16 - 1)).clamp(6, 28)
            } else {
                0
            };
            let total = ranks_n as u16 * nw + (ranks_n.saturating_sub(1) as u16) * g;
            let start = (w.saturating_sub(total) / 2).max(MX.min(w.saturating_sub(total)));
            (0..ranks_n).map(|r| start + r as u16 * (nw + g)).collect()
        }
        Orientation::TopBottom => {
            let avail = h.saturating_sub(2 + legend.is_some() as u16);
            let free = avail.saturating_sub(ranks_n as u16 * nh);
            let g = if ranks_n > 1 {
                (free / (ranks_n as u16 - 1)).clamp(4, 8)
            } else {
                0
            };
            let total = ranks_n as u16 * nh + (ranks_n.saturating_sub(1) as u16) * g;
            let start = 1 + avail.saturating_sub(total) / 2;
            (0..ranks_n).map(|r| start + r as u16 * (nh + g)).collect()
        }
    };
    // Order-axis coordinates: ranks by descending size, isotonic fit to desired centres.
    let total_items = n + pills.len();
    let mut center: Vec<Option<f64>> = vec![None; total_items];
    let mut rank_order: Vec<usize> = (0..ranks_n).collect();
    rank_order.sort_by(|&a, &b| items[b].len().cmp(&items[a].len()).then(a.cmp(&b)));
    let mid = (lo as f64 + hi as f64) / 2.0;
    let mut start_o: Vec<u16> = vec![0; total_items];
    for &r in &rank_order {
        let list = &items[r];
        if list.is_empty() {
            continue;
        }
        let desired: Vec<f64> = list
            .iter()
            .map(|&it| {
                if it >= n {
                    return f64::NAN;
                }
                let mesh = g.nodes[it].mesh.as_deref();
                let peers: Vec<f64> = (0..n)
                    .filter(|&j| {
                        j != it
                            && rank[j] != r
                            && visible[j]
                            && mesh.is_some()
                            && g.nodes[j].mesh.as_deref() == mesh
                    })
                    .filter_map(|j| center[j])
                    .collect();
                let src = if peers.is_empty() {
                    adj[it].iter().filter_map(|&j| center[j]).collect()
                } else {
                    peers
                };
                if src.is_empty() {
                    f64::NAN
                } else {
                    src.iter().sum::<f64>() / src.len() as f64
                }
            })
            .collect();
        let total = extent(list) as f64;
        // Unconstrained items follow their neighbours in the list, else the middle.
        let any = desired.iter().any(|d| !d.is_nan());
        let mut offsets = Vec::with_capacity(list.len());
        let mut acc = 0f64;
        for (k, &it) in list.iter().enumerate() {
            if k > 0 {
                acc += gap(list[k - 1], it) as f64;
            }
            offsets.push(acc);
            acc += ext_o as f64;
        }
        let targets: Vec<f64> = (0..list.len())
            .map(|k| {
                let d = desired[k];
                let c = if d.is_nan() {
                    if any {
                        f64::NAN
                    } else {
                        mid - total / 2.0 + offsets[k] + ext_o as f64 / 2.0
                    }
                } else {
                    d
                };
                c - ext_o as f64 / 2.0 - offsets[k]
            })
            .collect();
        let q = isotonic(&targets);
        let qmax = hi as f64 - total;
        for (k, &it) in list.iter().enumerate() {
            let v = q[k].clamp(lo as f64, qmax.max(lo as f64)).round() + offsets[k];
            start_o[it] = v as u16;
            center[it] = Some(v + ext_o as f64 / 2.0);
        }
    }
    // Centre the whole composition along the order axis (enclosure pads included).
    {
        let live: Vec<usize> = items.iter().flatten().copied().collect();
        if let (Some(min), Some(max)) = (
            live.iter().map(|&it| start_o[it]).min(),
            live.iter().map(|&it| start_o[it] + ext_o).max(),
        ) {
            let pad = |it: usize| if mesh_of(it).is_some() { pad_o } else { 0 };
            let min_p = live
                .iter()
                .map(|&it| start_o[it].saturating_sub(pad(it)))
                .min()
                .unwrap_or(min);
            let max_p = live
                .iter()
                .map(|&it| start_o[it] + ext_o + pad(it))
                .max()
                .unwrap_or(max);
            let (blo, bhi) = (lo.saturating_sub(1), hi + 1);
            let want = (blo as i32 + bhi as i32 - min_p as i32 - max_p as i32) / 2;
            let shift = want.clamp(lo as i32 - min as i32, hi as i32 - max as i32);
            for &it in &live {
                start_o[it] = (start_o[it] as i32 + shift) as u16;
            }
        }
    }
    let rect_of = |r: usize, it: usize| -> Rect {
        match orientation {
            Orientation::LeftRight => Rect::new(rank_coord[r], start_o[it], nw, nh),
            Orientation::TopBottom => Rect::new(start_o[it], rank_coord[r], nw, nh),
        }
    };
    let mut placed = Vec::new();
    let mut pills_out = Vec::new();
    let mut order = BTreeMap::new();
    for (r, list) in items.iter().enumerate() {
        for (p, &it) in list.iter().enumerate() {
            let rect = rect_of(r, it);
            if rect.right() > w || rect.bottom() > h {
                return None;
            }
            if it < n {
                placed.push(Placed {
                    node: it,
                    rect,
                    rank: r,
                });
                order.insert(g.nodes[it].id.clone(), (r, p));
            } else {
                pills_out.push(More {
                    rect,
                    rank: r,
                    hidden: pills[it - n].1.clone(),
                });
            }
        }
    }
    placed.sort_by_key(|p| p.node);
    let enclosures = enclosures(g, &placed, &pills_out, w, h);
    let mut layout = GraphLayout {
        key,
        mode: Mode::Graph,
        orientation,
        class,
        width: w,
        height: h,
        nodes: placed,
        more: pills_out,
        enclosures,
        routes: Vec::new(),
        outline: Vec::new(),
        order,
        legend,
        hits: HitMap::new(w, h),
    };
    layout.routes = route::route_all(g, &layout, &visible);
    layout.hits = HitMap::build(g, &layout);
    Some(layout)
}

/// Pool-adjacent-violators: the non-decreasing sequence closest (L2) to `t`.
/// NaN targets carry no weight and take their block's value.
pub(super) fn isotonic(t: &[f64]) -> Vec<f64> {
    // blocks: (sum, weight, len)
    let mut blocks: Vec<(f64, f64, usize)> = Vec::new();
    for &x in t {
        let (s, wgt) = if x.is_nan() { (0.0, 0.0) } else { (x, 1.0) };
        blocks.push((s, wgt, 1));
        while blocks.len() >= 2 {
            let b = blocks[blocks.len() - 1];
            let a = blocks[blocks.len() - 2];
            let mean = |x: (f64, f64, usize)| if x.1 > 0.0 { Some(x.0 / x.1) } else { None };
            let merge = match (mean(a), mean(b)) {
                (Some(ma), Some(mb)) => ma > mb,
                // Weightless blocks merge into their neighbours.
                _ => true,
            };
            if !merge {
                break;
            }
            blocks.pop();
            let last = blocks.last_mut().unwrap();
            *last = (a.0 + b.0, a.1 + b.1, a.2 + b.2);
        }
    }
    let mut out = Vec::with_capacity(t.len());
    for (s, wgt, len) in blocks {
        let v = if wgt > 0.0 { s / wgt } else { 0.0 };
        out.extend(std::iter::repeat_n(v, len));
    }
    out
}

fn expand(r: Rect, w: u16, h: u16) -> Rect {
    let x = r.x.saturating_sub(2);
    let y = r.y.saturating_sub(1);
    let right = (r.right() + 2).min(w);
    let bottom = (r.bottom() + 1).min(h);
    Rect::new(x, y, right - x, bottom - y)
}

fn enclosures(g: &Graph, placed: &[Placed], pills: &[More], w: u16, h: u16) -> Vec<Enclosure> {
    let mut meshes: Vec<&str> = placed
        .iter()
        .filter_map(|p| g.nodes[p.node].mesh.as_deref())
        .collect();
    meshes.sort_unstable();
    meshes.dedup();
    let mut out: Vec<Enclosure> = Vec::new();
    for m in meshes {
        let members: Vec<&Placed> = placed
            .iter()
            .filter(|p| g.nodes[p.node].mesh.as_deref() == Some(m))
            .collect();
        let others: Vec<Rect> = placed
            .iter()
            .filter(|p| g.nodes[p.node].mesh.as_deref() != Some(m))
            .map(|p| p.rect)
            .chain(pills.iter().map(|p| p.rect))
            .collect();
        let bbox = members
            .iter()
            .map(|p| p.rect)
            .reduce(|a, b| a.union(b))
            .unwrap();
        let whole = expand(bbox, w, h);
        let clean = !others.iter().any(|o| o.intersects(whole))
            && !out.iter().any(|e| e.rect.intersects(whole));
        if clean {
            out.push(Enclosure {
                mesh: m.into(),
                rect: whole,
                title: true,
            });
            continue;
        }
        // Per-rank runs of consecutive members.
        let mut by_rank: BTreeMap<usize, Vec<Rect>> = BTreeMap::new();
        for p in &members {
            by_rank.entry(p.rank).or_default().push(p.rect);
        }
        let mut first = true;
        for (_, mut rects) in by_rank {
            rects.sort_by_key(|r| (r.y, r.x));
            let mut run: Option<Rect> = None;
            for r in rects {
                run = Some(match run {
                    Some(acc)
                        if !others
                            .iter()
                            .any(|o| o.intersects(expand(acc.union(r), w, h))) =>
                    {
                        acc.union(r)
                    }
                    Some(acc) => {
                        out.push(Enclosure {
                            mesh: m.into(),
                            rect: expand(acc, w, h),
                            title: std::mem::take(&mut first),
                        });
                        r
                    }
                    None => r,
                });
            }
            if let Some(acc) = run {
                out.push(Enclosure {
                    mesh: m.into(),
                    rect: expand(acc, w, h),
                    title: std::mem::take(&mut first),
                });
            }
        }
    }
    out
}

fn outline_layout(g: &Graph, key: CacheKey) -> GraphLayout {
    let mut layout = GraphLayout {
        key,
        mode: Mode::Outline,
        orientation: Orientation::TopBottom,
        class: key.class,
        width: key.width,
        height: key.height,
        nodes: Vec::new(),
        more: Vec::new(),
        enclosures: Vec::new(),
        routes: Vec::new(),
        outline: outline_rows(g),
        order: BTreeMap::new(),
        legend: None,
        hits: HitMap::new(key.width, key.height),
    };
    for (p, row) in layout.outline.iter().enumerate() {
        layout
            .order
            .insert(g.nodes[row.node].id.clone(), (row.depth, p));
    }
    layout.hits = HitMap::build(g, &layout);
    layout
}

/// Spanning tree from the viewer (BFS parents), rendered depth-first.
pub fn outline_rows(g: &Graph) -> Vec<OutlineRow> {
    let n = g.nodes.len();
    if n == 0 {
        return Vec::new();
    }
    let index: BTreeMap<&str, usize> = g
        .nodes
        .iter()
        .enumerate()
        .map(|(i, x)| (x.id.as_str(), i))
        .collect();
    let mut adj: Vec<Vec<(usize, usize)>> = vec![Vec::new(); n];
    for (k, e) in g.edges.iter().enumerate() {
        if let (Some(&a), Some(&b)) = (index.get(e.from.as_str()), index.get(e.to.as_str())) {
            adj[a].push((b, k));
            adj[b].push((a, k));
        }
    }
    let root = g.viewer().unwrap_or(0);
    let mut parent: Vec<Option<(usize, usize)>> = vec![None; n];
    let mut seen = vec![false; n];
    let mut children: Vec<Vec<(usize, Option<usize>)>> = vec![Vec::new(); n];
    seen[root] = true;
    let mut q = VecDeque::from([root]);
    while let Some(a) = q.pop_front() {
        let mut next: Vec<(usize, usize)> =
            adj[a].iter().copied().filter(|(b, _)| !seen[*b]).collect();
        next.sort_by(|x, y| {
            let (p, r) = (&g.nodes[x.0], &g.nodes[y.0]);
            (
                p.kind == NodeKind::LanCluster,
                p.kind,
                &p.mesh,
                &p.label,
                &p.id,
            )
                .cmp(&(
                    r.kind == NodeKind::LanCluster,
                    r.kind,
                    &r.mesh,
                    &r.label,
                    &r.id,
                ))
        });
        next.dedup_by_key(|x| x.0);
        for (b, k) in next {
            if !seen[b] {
                seen[b] = true;
                parent[b] = Some((a, k));
                children[a].push((b, Some(k)));
                q.push_back(b);
            }
        }
    }
    for i in 0..n {
        if !seen[i] {
            children[root].push((i, None));
        }
    }
    let mut rows = Vec::new();
    fn walk(
        i: usize,
        edge: Option<usize>,
        depth: usize,
        guides: Vec<bool>,
        last: bool,
        children: &[Vec<(usize, Option<usize>)>],
        rows: &mut Vec<OutlineRow>,
    ) {
        rows.push(OutlineRow {
            node: i,
            depth,
            edge,
            guides: guides.clone(),
            last,
        });
        let kids = &children[i];
        for (k, &(c, e)) in kids.iter().enumerate() {
            let mut gd = guides.clone();
            if depth > 0 {
                gd.push(!last);
            }
            walk(c, e, depth + 1, gd, k + 1 == kids.len(), children, rows);
        }
    }
    walk(root, None, 0, Vec::new(), true, &children, &mut rows);
    rows
}
