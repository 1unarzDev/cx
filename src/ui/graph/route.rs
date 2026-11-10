//! Orthogonal edge routing (spec 10.3): A* over free cells with bend, crossing
//! and node-hugging costs, ports spread along node sides, and per-cell arm
//! masks that the renderer merges into box-drawing glyphs.
use super::layout::{Bridge, GraphLayout, Orientation};
use super::model::*;
use ratatui::layout::Rect;
use std::cmp::Reverse;
use std::collections::{BTreeMap, BinaryHeap};

/// Arm bits of a cell mask.
pub const UP: u8 = 1;
pub const RIGHT: u8 = 2;
pub const DOWN: u8 = 4;
pub const LEFT: u8 = 8;

const STEP: u32 = 1;
const BEND: u32 = 4;
const CROSS: u32 = 6;
const HUG: u32 = 2;
const FENCE: u32 = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LabelSlot {
    pub x: u16,
    pub y: u16,
    pub len: u16,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Route {
    /// Index into `Graph::edges`.
    pub edge: usize,
    /// Path cells outside nodes, source end first, with arm masks.
    pub cells: Vec<(u16, u16, u8)>,
    /// Border cell of the source node and the arm that leaves it.
    pub src_port: (u16, u16, u8),
    pub dst_port: (u16, u16, u8),
    pub label: Option<LabelSlot>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Side {
    Top,
    Right,
    Bottom,
    Left,
}
impl Side {
    fn normal_deg(self) -> f64 {
        match self {
            Side::Top => 0.0,
            Side::Right => 90.0,
            Side::Bottom => 180.0,
            Side::Left => 270.0,
        }
    }
    fn outward(self) -> usize {
        match self {
            Side::Top => 0,
            Side::Right => 1,
            Side::Bottom => 2,
            Side::Left => 3,
        }
    }
}

const DX: [i32; 4] = [0, 1, 0, -1];
const DY: [i32; 4] = [-1, 0, 1, 0];
pub fn bit(dir: usize) -> u8 {
    1 << dir
}
fn opposite(dir: usize) -> usize {
    (dir + 2) % 4
}

fn center(r: Rect) -> (f64, f64) {
    (
        r.x as f64 + r.width as f64 / 2.0,
        r.y as f64 + r.height as f64 / 2.0,
    )
}

/// Port slots on a side, from its counter-clockwise end to its clockwise end.
fn slots(r: Rect, side: Side) -> Vec<(u16, u16)> {
    let rows: Vec<u16> = (r.y + 1..r.bottom().saturating_sub(1)).collect();
    let cols: Vec<u16> = (r.x + 2..r.right().saturating_sub(2)).step_by(2).collect();
    match side {
        Side::Right => rows.iter().map(|&y| (r.right() - 1, y)).collect(),
        Side::Left => rows.iter().rev().map(|&y| (r.x, y)).collect(),
        Side::Bottom => cols.iter().rev().map(|&x| (x, r.bottom() - 1)).collect(),
        Side::Top => cols.iter().map(|&x| (x, r.y)).collect(),
    }
}

/// Picks `k` of `n` slots spread evenly (centred for few edges).
fn spread(n: usize, k: usize) -> Vec<usize> {
    if k == 0 || n == 0 {
        return Vec::new();
    }
    if k >= n {
        return (0..k).map(|i| i.min(n - 1)).collect();
    }
    if n == 2 && k == 1 {
        return vec![0];
    }
    (0..k)
        .map(|i| ((2 * i + 1) * n / (2 * k)).min(n - 1))
        .collect()
}

struct End {
    route: usize,
    src: bool,
    other: (f64, f64),
}

struct Grid {
    w: u16,
    h: u16,
    blocked: Vec<bool>,
    hug: Vec<bool>,
    fence: Vec<bool>,
    /// bit 1: horizontal edge, 2: vertical edge, 4: no crossing here.
    occ: Vec<u8>,
    /// Port stub cells (first cell outside a port), owned by one edge so a
    /// route laid earlier never walls off a later edge's port.
    stub: Vec<usize>,
}
const H: u8 = 1;
const V: u8 = 2;
const NOX: u8 = 4;

impl Grid {
    fn idx(&self, x: u16, y: u16) -> usize {
        y as usize * self.w as usize + x as usize
    }
}

/// Routes every edge whose endpoints are both visible and placed.
pub fn route_all(g: &Graph, layout: &GraphLayout, visible: &[bool]) -> Vec<Route> {
    let (w, h) = (layout.width, layout.height);
    let mut grid = Grid {
        w,
        h,
        blocked: vec![false; w as usize * h as usize],
        hug: vec![false; w as usize * h as usize],
        fence: vec![false; w as usize * h as usize],
        occ: vec![0; w as usize * h as usize],
        stub: vec![usize::MAX; w as usize * h as usize],
    };
    let rects: Vec<Rect> = layout
        .nodes
        .iter()
        .map(|p| p.rect)
        .chain(layout.more.iter().map(|m| m.rect))
        .collect();
    for r in &rects {
        for y in r.y..r.bottom().min(h) {
            for x in r.x..r.right().min(w) {
                let i = grid.idx(x, y);
                grid.blocked[i] = true;
            }
        }
        let x0 = r.x.saturating_sub(1);
        let y0 = r.y.saturating_sub(1);
        for y in y0..(r.bottom() + 1).min(h) {
            for x in x0..(r.right() + 1).min(w) {
                let i = grid.idx(x, y);
                grid.hug[i] = true;
            }
        }
    }
    for e in &layout.enclosures {
        let r = e.rect;
        for x in r.x..r.right() {
            for y in [r.y, r.bottom().saturating_sub(1)] {
                if x < w && y < h {
                    let i = grid.idx(x, y);
                    grid.fence[i] = true;
                }
            }
        }
        for y in r.y..r.bottom() {
            for x in [r.x, r.right().saturating_sub(1)] {
                if x < w && y < h {
                    let i = grid.idx(x, y);
                    grid.fence[i] = true;
                }
            }
        }
    }
    let rect_of: BTreeMap<usize, (Rect, usize)> = layout
        .nodes
        .iter()
        .map(|p| (p.node, (p.rect, p.rank)))
        .collect();
    let index: BTreeMap<&str, usize> = g
        .nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.id.as_str(), i))
        .collect();
    // Candidate edges.
    struct Pending {
        edge: usize,
        a: usize,
        b: usize,
    }
    let mut pending: Vec<Pending> = Vec::new();
    for (k, e) in g.edges.iter().enumerate() {
        let (Some(&a), Some(&b)) = (index.get(e.from.as_str()), index.get(e.to.as_str())) else {
            continue;
        };
        if a == b || !visible[a] || !visible[b] {
            continue;
        }
        if rect_of.contains_key(&a) && rect_of.contains_key(&b) {
            pending.push(Pending { edge: k, a, b });
        }
    }
    // Ports: choose a side per edge end, then spread along each side.
    let mut ends: BTreeMap<(usize, u8), Vec<End>> = BTreeMap::new();
    let side_code = |s: Side| s as u8;
    let decode = |c: u8| match c {
        0 => Side::Top,
        1 => Side::Right,
        2 => Side::Bottom,
        _ => Side::Left,
    };
    for (ri, p) in pending.iter().enumerate() {
        for (node, other, src) in [(p.a, p.b, true), (p.b, p.a, false)] {
            let (r, rank) = rect_of[&node];
            let (or, orank) = rect_of[&other];
            let (cx, cy) = center(r);
            let oc = center(or);
            let side = match layout.orientation {
                Orientation::LeftRight => {
                    if orank > rank {
                        Side::Right
                    } else if orank < rank {
                        Side::Left
                    } else if oc.1 >= cy {
                        Side::Bottom
                    } else {
                        Side::Top
                    }
                }
                Orientation::TopBottom => {
                    if orank > rank {
                        Side::Bottom
                    } else if orank < rank {
                        Side::Top
                    } else if oc.0 >= cx {
                        Side::Right
                    } else {
                        Side::Left
                    }
                }
            };
            ends.entry((node, side_code(side))).or_default().push(End {
                route: ri,
                src,
                other: oc,
            });
        }
    }
    // Capacity overflow: move the most extreme ends to the neighbouring side.
    let nodes: Vec<usize> = ends.keys().map(|k| k.0).collect();
    for node in nodes {
        let (r, _) = rect_of[&node];
        let (cx, cy) = center(r);
        for side in [Side::Right, Side::Left, Side::Bottom, Side::Top] {
            let cap = slots(r, side).len();
            loop {
                let list = ends.entry((node, side_code(side))).or_default();
                if list.len() <= cap {
                    break;
                }
                let horizontal = matches!(side, Side::Left | Side::Right);
                let (k, _) = list
                    .iter()
                    .enumerate()
                    .map(|(k, e)| {
                        let d = if horizontal {
                            (e.other.1 - cy).abs() * 2.0
                        } else {
                            (e.other.0 - cx).abs()
                        };
                        (k, d)
                    })
                    .max_by(|a, b| a.1.total_cmp(&b.1).then(b.0.cmp(&a.0)))
                    .unwrap();
                let end = list.remove(k);
                let to = if horizontal {
                    if end.other.1 >= cy {
                        Side::Bottom
                    } else {
                        Side::Top
                    }
                } else if end.other.0 >= cx {
                    Side::Right
                } else {
                    Side::Left
                };
                let target = ends.entry((node, side_code(to))).or_default();
                if target.len() >= slots(r, to).len() {
                    ends.entry((node, side_code(side))).or_default().push(end);
                    break;
                }
                target.push(end);
            }
        }
    }
    let mut src_port: Vec<Option<(u16, u16, usize)>> = vec![None; pending.len()];
    let mut dst_port: Vec<Option<(u16, u16, usize)>> = vec![None; pending.len()];
    for ((node, code), list) in ends.iter_mut() {
        let side = decode(*code);
        let (r, _) = rect_of[node];
        let (cx, cy) = center(r);
        let rel = |e: &End| {
            let dx = e.other.0 - cx;
            let dy = (e.other.1 - cy) * 2.0;
            let theta = dx.atan2(-dy).to_degrees();
            let mut rho = theta - side.normal_deg();
            while rho <= -180.0 {
                rho += 360.0;
            }
            while rho > 180.0 {
                rho -= 360.0;
            }
            rho
        };
        list.sort_by(|a, b| rel(a).total_cmp(&rel(b)).then(a.route.cmp(&b.route)));
        let s = slots(r, side);
        let mut picks = spread(s.len(), list.len());
        // A lone edge on a left/right side uses the top inner row (the name line),
        // so facing ports line up and straight edges stay straight.
        if list.len() == 1 && matches!(side, Side::Left | Side::Right) {
            if let Some(top) = (0..s.len()).min_by_key(|&k| s[k].1) {
                picks = vec![top];
            }
        }
        for (end, slot) in list.iter().zip(picks) {
            let (x, y) = s[slot];
            let port = Some((x, y, side.outward()));
            if end.src {
                src_port[end.route] = port;
            } else {
                dst_port[end.route] = port;
            }
        }
    }
    for (i, p) in pending.iter().enumerate() {
        for port in [src_port[i], dst_port[i]].into_iter().flatten() {
            let (x, y) = (port.0 as i32 + DX[port.2], port.1 as i32 + DY[port.2]);
            if x >= 0 && y >= 0 && x < w as i32 && y < h as i32 {
                let c = grid.idx(x as u16, y as u16);
                grid.stub[c] = if grid.stub[c] == usize::MAX || grid.stub[c] == p.edge {
                    p.edge
                } else {
                    usize::MAX - 1
                };
            }
        }
    }
    // Route shorter edges first.
    let mut order: Vec<usize> = (0..pending.len()).collect();
    let dist = |p: &Pending| {
        let a = center(rect_of[&p.a].0);
        let b = center(rect_of[&p.b].0);
        ((a.0 - b.0).abs() + (a.1 - b.1).abs()) as u32
    };
    order.sort_by_key(|&i| (dist(&pending[i]), pending[i].edge));
    let mut search = Search::new(w as usize * h as usize);
    let mut routes = Vec::new();
    for i in order {
        let (Some(sp), Some(dp)) = (src_port[i], dst_port[i]) else {
            continue;
        };
        if let Some(r) = route_one(&mut grid, &mut search, pending[i].edge, sp, dp) {
            routes.push(r);
        }
    }
    routes.sort_by_key(|r| r.edge);
    place_labels(&grid, &mut routes);
    routes
}

struct Search {
    cost: Vec<u32>,
    parent: Vec<u32>,
    stamp: Vec<u32>,
    gen: u32,
}
impl Search {
    fn new(cells: usize) -> Self {
        Search {
            cost: vec![0; cells * 4],
            parent: vec![0; cells * 4],
            stamp: vec![0; cells * 4],
            gen: 0,
        }
    }
}

fn route_one(
    grid: &mut Grid,
    search: &mut Search,
    edge: usize,
    sp: (u16, u16, usize),
    dp: (u16, u16, usize),
) -> Option<Route> {
    let (w, h) = (grid.w as i32, grid.h as i32);
    let step = |x: u16, y: u16, d: usize| -> Option<(u16, u16)> {
        let nx = x as i32 + DX[d];
        let ny = y as i32 + DY[d];
        (nx >= 0 && ny >= 0 && nx < w && ny < h).then_some((nx as u16, ny as u16))
    };
    let start_dir = sp.2;
    let (sx, sy) = step(sp.0, sp.1, start_dir)?;
    let inward = opposite(dp.2);
    let (tx, ty) = step(dp.0, dp.1, dp.2)?;
    let si = grid.idx(sx, sy);
    let ti = grid.idx(tx, ty);
    if grid.blocked[si] || grid.blocked[ti] || grid.occ[si] != 0 || grid.occ[ti] != 0 {
        return None;
    }
    let windows = {
        let pad = 6i32;
        let x0 = (sx.min(tx) as i32 - pad).max(0);
        let y0 = (sy.min(ty) as i32 - pad).max(0);
        let x1 = (sx.max(tx) as i32 + pad).min(w - 1);
        let y1 = (sy.max(ty) as i32 + pad).min(h - 1);
        [(x0, y0, x1, y1), (0, 0, w - 1, h - 1)]
    };
    for (x0, y0, x1, y1) in windows {
        search.gen = search.gen.wrapping_add(1);
        if search.gen == 0 {
            search.stamp.iter_mut().for_each(|s| *s = 0);
            search.gen = 1;
        }
        let gen = search.gen;
        let heur = |x: u16, y: u16| -> u32 {
            let dx = (x as i32 - tx as i32).unsigned_abs();
            let dy = (y as i32 - ty as i32).unsigned_abs();
            dx + dy + if dx != 0 && dy != 0 { BEND } else { 0 }
        };
        let mut heap: BinaryHeap<Reverse<(u32, u32, u32)>> = BinaryHeap::new();
        let s_state = (si * 4 + start_dir) as u32;
        search.cost[s_state as usize] = 0;
        search.stamp[s_state as usize] = gen;
        search.parent[s_state as usize] = u32::MAX;
        let mut tick = 0u32;
        heap.push(Reverse((heur(sx, sy), tick, s_state)));
        let mut found = None;
        if si == ti {
            if start_dir == inward {
                found = Some(s_state);
            } else {
                continue;
            }
        }
        while let Some(Reverse((_, _, state))) = heap.pop() {
            if found.is_some() {
                break;
            }
            let cell = state as usize / 4;
            let dir = state as usize % 4;
            let g0 = search.cost[state as usize];
            let (x, y) = (
                (cell % grid.w as usize) as u16,
                (cell / grid.w as usize) as u16,
            );
            if cell == ti {
                found = Some(state);
                break;
            }
            // Crossing cells must be passed straight through.
            let crossing = grid.occ[cell] & (H | V) != 0;
            for nd in [dir, (dir + 1) % 4, (dir + 3) % 4] {
                if crossing && nd != dir {
                    continue;
                }
                let Some((nx, ny)) = step(x, y, nd) else {
                    continue;
                };
                if (nx as i32) < x0 || (nx as i32) > x1 || (ny as i32) < y0 || (ny as i32) > y1 {
                    continue;
                }
                let ni = grid.idx(nx, ny);
                if grid.blocked[ni] || (grid.stub[ni] != usize::MAX && grid.stub[ni] != edge) {
                    continue;
                }
                let o = grid.occ[ni];
                let horizontal = nd % 2 == 1;
                let mut c = STEP;
                if o != 0 {
                    if o & NOX != 0
                        || (horizontal && o & H != 0)
                        || (!horizontal && o & V != 0)
                        || ni == ti
                    {
                        continue;
                    }
                    c += CROSS;
                }
                if nd != dir {
                    c += BEND;
                }
                if grid.hug[ni] {
                    c += HUG;
                }
                if grid.fence[ni] {
                    c += FENCE;
                }
                if ni == ti && nd != inward {
                    c += BEND;
                }
                let ns = (ni * 4 + nd) as u32;
                let ng = g0 + c;
                if search.stamp[ns as usize] != gen || ng < search.cost[ns as usize] {
                    search.stamp[ns as usize] = gen;
                    search.cost[ns as usize] = ng;
                    search.parent[ns as usize] = state;
                    tick += 1;
                    heap.push(Reverse((ng + heur(nx, ny), tick, ns)));
                }
            }
        }
        let Some(end) = found else {
            continue;
        };
        // Reconstruct.
        let mut states = vec![end];
        let mut cur = end;
        while search.parent[cur as usize] != u32::MAX {
            cur = search.parent[cur as usize];
            states.push(cur);
        }
        states.reverse();
        let mut cells = Vec::with_capacity(states.len());
        for (k, &st) in states.iter().enumerate() {
            let cell = st as usize / 4;
            let arrive = st as usize % 4;
            let next = if k + 1 < states.len() {
                states[k + 1] as usize % 4
            } else {
                inward
            };
            let mask = bit(opposite(arrive)) | bit(next);
            let (x, y) = (
                (cell % grid.w as usize) as u16,
                (cell / grid.w as usize) as u16,
            );
            cells.push((x, y, mask));
        }
        let last = cells.len() - 1;
        for (k, &(x, y, mask)) in cells.iter().enumerate() {
            let i = grid.idx(x, y);
            let flag = if mask == LEFT | RIGHT {
                H
            } else if mask == UP | DOWN {
                V
            } else {
                H | V | NOX
            };
            grid.occ[i] |= flag;
            if k == 0 || k == last {
                grid.occ[i] |= NOX;
            }
        }
        return Some(Route {
            edge,
            cells,
            src_port: (sp.0, sp.1, bit(start_dir)),
            dst_port: (dp.0, dp.1, bit(dp.2)),
            label: None,
        });
    }
    None
}

/// Label slots: beside the longest straight run when the cells there are free.
fn place_labels(grid: &Grid, routes: &mut [Route]) {
    let mut taken = vec![false; grid.w as usize * grid.h as usize];
    let free = |taken: &[bool], x: u16, y: u16| -> bool {
        if x >= grid.w || y >= grid.h {
            return false;
        }
        let i = grid.idx(x, y);
        !grid.blocked[i] && grid.occ[i] == 0 && !grid.fence[i] && !taken[i]
    };
    for r in routes.iter_mut() {
        // Horizontal runs (excluding the two end cells).
        let cells = &r.cells;
        if cells.len() < 5 {
            continue;
        }
        let inner = &cells[1..cells.len() - 1];
        let mut best: Option<(u16, u16, u16)> = None; // x0, y, len
        let mut k = 0;
        while k < inner.len() {
            let (x, y, m) = inner[k];
            if m & (LEFT | RIGHT) == LEFT | RIGHT {
                let mut j = k;
                while j + 1 < inner.len()
                    && inner[j + 1].1 == y
                    && inner[j + 1].2 & (LEFT | RIGHT) == LEFT | RIGHT
                {
                    j += 1;
                }
                let x0 = inner[k].0.min(inner[j].0);
                let len = (j - k + 1) as u16;
                if best.is_none_or(|b| len > b.2) {
                    best = Some((x0, y, len));
                }
                k = j + 1;
            } else {
                let _ = x;
                k += 1;
            }
        }
        let Some((x0, y, len)) = best else {
            continue;
        };
        if len < 5 {
            continue;
        }
        let slot_len = (len - 2).min(12);
        let sx = x0 + (len - slot_len) / 2;
        for row in [y.checked_sub(1), Some(y + 1)].into_iter().flatten() {
            if (sx..sx + slot_len).all(|x| free(&taken, x, row)) {
                for x in sx..sx + slot_len {
                    taken[grid.idx(x, row)] = true;
                }
                r.label = Some(LabelSlot {
                    x: sx,
                    y: row,
                    len: slot_len,
                });
                break;
            }
        }
    }
}

/// Joins the fragments of every mesh that `layout::enclosures` had to split
/// (another mesh's group interleaves) with dashed bridges routed outside all
/// enclosures and nodes. `None` when some fragment cannot be reached: the
/// caller then treats the graph as not fitting, so a mesh is never shown as
/// unlinked boxes.
pub fn bridge_all(layout: &GraphLayout) -> Option<Vec<Bridge>> {
    let (w, h) = (layout.width, layout.height);
    let mut meshes: Vec<&str> = layout.enclosures.iter().map(|e| e.mesh.as_str()).collect();
    meshes.sort_unstable();
    meshes.dedup();
    let split = meshes
        .iter()
        .any(|m| layout.enclosures.iter().filter(|e| e.mesh == *m).count() > 1);
    if !split {
        return Some(Vec::new());
    }
    let cells = w as usize * h as usize;
    let idx = |x: u16, y: u16| y as usize * w as usize + x as usize;
    // Blocked: nodes, pills, every enclosure (border included), labels, legend.
    let mut blocked = vec![false; cells];
    let mut hug = vec![false; cells];
    let block = |r: Rect, blocked: &mut Vec<bool>| {
        for y in r.y..r.bottom().min(h) {
            for x in r.x..r.right().min(w) {
                blocked[idx(x, y)] = true;
            }
        }
    };
    for r in layout
        .nodes
        .iter()
        .map(|p| p.rect)
        .chain(layout.more.iter().map(|m| m.rect))
    {
        block(r, &mut blocked);
        for y in r.y.saturating_sub(1)..(r.bottom() + 1).min(h) {
            for x in r.x.saturating_sub(1)..(r.right() + 1).min(w) {
                hug[idx(x, y)] = true;
            }
        }
    }
    for e in &layout.enclosures {
        block(e.rect, &mut blocked);
    }
    for r in &layout.routes {
        if let Some(l) = r.label {
            block(Rect::new(l.x, l.y, l.len, 1), &mut blocked);
        }
    }
    if let Some(row) = layout.legend {
        block(Rect::new(0, row, w, 1), &mut blocked);
    }
    let mut occ = vec![0u8; cells];
    let mark = |occ: &mut Vec<u8>, path: &[(u16, u16, u8)]| {
        for &(x, y, m) in path {
            occ[idx(x, y)] |= if m == LEFT | RIGHT {
                H
            } else if m == UP | DOWN {
                V
            } else {
                H | V | NOX
            };
        }
    };
    for r in &layout.routes {
        mark(&mut occ, &r.cells);
        for p in [r.src_port, r.dst_port] {
            occ[idx(p.0, p.1)] |= NOX;
        }
    }
    // Non-corner border cells of an enclosure with their outward direction.
    let border = |r: Rect| -> Vec<(u16, u16, usize)> {
        let mut v = Vec::new();
        if r.width < 3 || r.height < 3 {
            return v;
        }
        for x in r.x + 1..r.right() - 1 {
            v.push((x, r.y, 0));
            v.push((x, r.bottom() - 1, 2));
        }
        for y in r.y + 1..r.bottom() - 1 {
            v.push((r.x, y, 3));
            v.push((r.right() - 1, y, 1));
        }
        v
    };
    let step = |x: u16, y: u16, d: usize| -> Option<(u16, u16)> {
        let nx = x as i32 + DX[d];
        let ny = y as i32 + DY[d];
        (nx >= 0 && ny >= 0 && nx < w as i32 && ny < h as i32).then_some((nx as u16, ny as u16))
    };
    let mut out = Vec::new();
    // Valid only where `stamp == gen` (zeroed allocations stay cheap).
    let mut cost = vec![0u32; cells * 4];
    let mut parent = vec![0u32; cells * 4];
    let mut stamp = vec![0u32; cells * 4];
    let mut gen = 0u32;
    let mut goal_dir = vec![u8::MAX; cells];
    for m in meshes {
        let frags: Vec<usize> = (0..layout.enclosures.len())
            .filter(|&k| layout.enclosures[k].mesh == m)
            .collect();
        if frags.len() < 2 {
            continue;
        }
        let mut connected = vec![frags[0]];
        let mut rest: Vec<usize> = frags[1..].to_vec();
        while !rest.is_empty() {
            // Target: the remaining fragment nearest the connected set.
            let dist = |a: Rect, b: Rect| {
                let dx = (a.x as i32 - b.x as i32).abs();
                let dy = (a.y as i32 - b.y as i32).abs();
                dx + 2 * dy
            };
            let (ti, (_, _, near)) = rest
                .iter()
                .enumerate()
                .map(|(i, &t)| {
                    let (d, c) = connected
                        .iter()
                        .map(|&c| {
                            let d = dist(layout.enclosures[c].rect, layout.enclosures[t].rect);
                            (d, c)
                        })
                        .min()
                        .unwrap();
                    (i, (d, t, c))
                })
                .min_by_key(|x| x.1)
                .unwrap();
            let target = rest.remove(ti);
            let trect = layout.enclosures[target].rect;
            // Goal cells: outside the target, next to a non-corner border cell.
            let mut goal: BTreeMap<usize, (usize, (u16, u16))> = BTreeMap::new();
            for (bx, by, d) in border(trect) {
                if let Some((x, y)) = step(bx, by, d) {
                    let i = idx(x, y);
                    if !blocked[i] && occ[i] & NOX == 0 {
                        goal.entry(i).or_insert((opposite(d), (bx, by)));
                        if goal_dir[i] == u8::MAX {
                            goal_dir[i] = opposite(d) as u8;
                        }
                    }
                }
            }
            let tx = trect.x as i32 + trect.width as i32 / 2;
            let ty = trect.y as i32 + trect.height as i32 / 2;
            let heur = |x: u16, y: u16| -> u32 {
                let dx = ((x as i32 - tx).abs() - trect.width as i32 / 2 - 1).max(0);
                let dy = ((y as i32 - ty).abs() - trect.height as i32 / 2 - 1).max(0);
                (dx + dy) as u32
            };
            // A window around the two fragments first, then the whole area.
            let around = layout.enclosures[near].rect.union(trect);
            let windows = [
                (
                    around.x.saturating_sub(4),
                    around.y.saturating_sub(4),
                    (around.right() + 4).min(w),
                    (around.bottom() + 4).min(h),
                ),
                (0, 0, w, h),
            ];
            let mut start_of: BTreeMap<u32, (u16, u16, usize, usize)> = BTreeMap::new();
            let mut best_goal: Option<(u32, u32)> = None; // (cost, state)
            for (x0, y0, x1, y1) in windows {
                let inside = |x: u16, y: u16| x >= x0 && x < x1 && y >= y0 && y < y1;
                gen += 1;
                let mut heap: BinaryHeap<Reverse<(u32, u32, u32)>> = BinaryHeap::new();
                start_of.clear();
                let mut tick = 0u32;
                for &c in &connected {
                    for (bx, by, d) in border(layout.enclosures[c].rect) {
                        let Some((x, y)) = step(bx, by, d) else {
                            continue;
                        };
                        let i = idx(x, y);
                        if blocked[i] || occ[i] & NOX != 0 || !inside(x, y) {
                            continue;
                        }
                        let st = (i * 4 + d) as u32;
                        if stamp[st as usize] == gen {
                            continue;
                        }
                        stamp[st as usize] = gen;
                        cost[st as usize] = 0;
                        parent[st as usize] = u32::MAX;
                        start_of.insert(st, (bx, by, d, c));
                        tick += 1;
                        heap.push(Reverse((heur(x, y), tick, st)));
                    }
                }
                while let Some(Reverse((f, _, st))) = heap.pop() {
                    if let Some((gc, _)) = best_goal {
                        if f >= gc {
                            break;
                        }
                    }
                    let cell = st as usize / 4;
                    let dir = st as usize % 4;
                    let g0 = cost[st as usize];
                    let (x, y) = ((cell % w as usize) as u16, (cell / w as usize) as u16);
                    if goal_dir[cell] != u8::MAX {
                        let gd = goal_dir[cell] as usize;
                        let gc = g0 + if gd != dir { BEND } else { 0 };
                        if best_goal.is_none_or(|b| gc < b.0) {
                            best_goal = Some((gc, st));
                        }
                    }
                    let crossing = occ[cell] & (H | V) != 0;
                    for nd in [dir, (dir + 1) % 4, (dir + 3) % 4] {
                        if crossing && nd != dir {
                            continue;
                        }
                        let Some((nx, ny)) = step(x, y, nd) else {
                            continue;
                        };
                        let ni = idx(nx, ny);
                        if blocked[ni] || !inside(nx, ny) {
                            continue;
                        }
                        let o = occ[ni];
                        let horizontal = nd % 2 == 1;
                        let mut c = STEP;
                        if o != 0 {
                            if o & NOX != 0
                                || (horizontal && o & H != 0)
                                || (!horizontal && o & V != 0)
                            {
                                continue;
                            }
                            c += CROSS;
                        }
                        if nd != dir {
                            c += BEND;
                        }
                        if hug[ni] {
                            c += HUG;
                        }
                        let ns = ni * 4 + nd;
                        let ng = g0 + c;
                        if stamp[ns] != gen || ng < cost[ns] {
                            stamp[ns] = gen;
                            cost[ns] = ng;
                            parent[ns] = st;
                            tick += 1;
                            heap.push(Reverse((ng + heur(nx, ny), tick, ns as u32)));
                        }
                    }
                }
                if best_goal.is_some() {
                    break;
                }
            }
            for &i in goal.keys() {
                goal_dir[i] = u8::MAX;
            }
            let (_, end) = best_goal?;
            let mut states = vec![end];
            let mut cur = end;
            while parent[cur as usize] != u32::MAX {
                cur = parent[cur as usize];
                states.push(cur);
            }
            states.reverse();
            let (sbx, sby, sd, from) = start_of[&states[0]];
            let end_cell = end as usize / 4;
            let (gd, (gbx, gby)) = goal[&end_cell];
            let mut path = Vec::with_capacity(states.len());
            for (k, &st) in states.iter().enumerate() {
                let cell = st as usize / 4;
                let arrive = st as usize % 4;
                let next = if k + 1 < states.len() {
                    states[k + 1] as usize % 4
                } else {
                    gd
                };
                let (x, y) = ((cell % w as usize) as u16, (cell / w as usize) as u16);
                path.push((x, y, bit(opposite(arrive)) | bit(next)));
            }
            mark(&mut occ, &path);
            for &(x, y, _) in &path {
                blocked[idx(x, y)] = true;
            }
            out.push(Bridge {
                mesh: m.to_string(),
                from,
                to: target,
                cells: path,
                ends: [(sbx, sby, bit(sd)), (gbx, gby, bit(opposite(gd)))],
            });
            connected.push(target);
        }
    }
    Some(out)
}
