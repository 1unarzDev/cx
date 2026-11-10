//! The network schematic, drawn like a transit map: the root device (this device on the
//! hub) is a tinted pill on the left, the devices it reaches form the next column and the
//! devices reached through them the one after. Links are solid lines with rounded elbows
//! (`─│╭╮╰╯`, ASCII `-|+`), coloured by network, routed so that no two links cross: links
//! leave the pill from its top and bottom edges (the nearest target bends last) and
//! children leave their parent on a short trunk. Every link carries one policy chip on its
//! last straight run (`⇄ → ← ⊘ ✕`), which is also its tap target. Nothing here dispatches.
use super::{fit, mix, Cx, Rgb, Size};
use crate::ui::lab::{Access, DeviceStatus, EdgeKind, LabEdge, Tok};
use ratatui::{buffer::Buffer, layout::Rect, style::Modifier};

/// What the schematic highlights.
#[derive(Clone, Copy)]
pub(super) enum Sel<'a> {
    /// A device; `true` when the schematic has keyboard focus.
    Node(&'a str, bool),
    /// One link (the edge editor): everything else fades.
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

/// A placed device: status dot at `x`, label from `x + 2`, `w` columns wide in total.
struct Node {
    id: String,
    col: usize,
    x: u16,
    y: u16,
    w: u16,
}

struct Plan {
    nodes: Vec<Node>,
    /// The root's one-row pill.
    pill: Rect,
    /// Two-row nodes (name, then evidence and mesh badges).
    tall: bool,
    mode: Mode,
    /// "+N" markers: (x, y, hidden count).
    more: Vec<(u16, u16, usize)>,
}

/// Device ids in selection order: column by column, top to bottom, the viewer last.
pub(super) fn order(cx: &Cx, band: Rect) -> Vec<String> {
    let root = cx.s.viewer.as_str();
    let mut v: Vec<String> = plan(cx, band, root, 3)
        .nodes
        .into_iter()
        .map(|n| n.id)
        .collect();
    v.push(root.to_string());
    v
}

/// Rows the hub schematic wants: room for its tallest column with air between nodes.
pub(super) fn want_h(cx: &Cx) -> u16 {
    let most = columns(cx, &cx.s.viewer, 3)
        .iter()
        .map(|c| c.len())
        .max()
        .unwrap_or(1) as u16;
    most * 3 + 2
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

fn badges_w(cx: &Cx, id: &str) -> u16 {
    cx.s.device(id).map_or(0, |d| d.meshes.len() as u16)
}

/// Room for the cpu sparkline on a tall node's evidence row.
fn spark_w(cx: &Cx, id: &str) -> u16 {
    let live =
        cx.s.device(id)
            .is_some_and(|d| d.metrics.is_some() && cx.up(d));
    if live && cx.size() >= Size::M {
        7
    } else {
        0
    }
}

fn label_w(cx: &Cx, id: &str, mode: Mode, tall: bool, nmax: u16) -> u16 {
    let Some(d) = cx.s.device(id) else { return 0 };
    let name = (d.name.chars().count() as u16).min(nmax);
    let meta = cx.meta(d).chars().count() as u16;
    match mode {
        Mode::Chip => 5,
        Mode::Name => 6 + name,
        Mode::Full if tall => 6 + name.max(meta + 1 + badges_w(cx, id) + spark_w(cx, id)),
        Mode::Full => 7 + name + meta,
    }
}

fn pill_w(cx: &Cx, root: &str, mode: Mode) -> u16 {
    let name =
        cx.s.device(root)
            .map_or(4, |d| d.name.chars().count() as u16);
    match mode {
        Mode::Chip => 5,
        _ => 8 + name,
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

fn plan(cx: &Cx, band: Rect, root: &str, depth: usize) -> Plan {
    let cols = columns(cx, root, depth);
    let h = band.height;
    let most = cols.iter().map(|c| c.len()).max().unwrap_or(0) as u16;
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
    let mut widths: Vec<u16> = Vec::new();
    let tries = [
        (Mode::Full, 22),
        (Mode::Full, 16),
        (Mode::Name, 22),
        (Mode::Name, 16),
        (Mode::Name, 12),
        (Mode::Name, 8),
        (Mode::Chip, 8),
    ];
    for (m, nm) in tries {
        mode = m;
        nmax = nm;
        widths = cols
            .iter()
            .map(|c| {
                c.iter()
                    .take(cap(c.len()))
                    .map(|id| label_w(cx, id, m, tall, nmax))
                    .max()
                    .unwrap_or(0)
                    .max(4)
            })
            .collect();
        let gap = if m == Mode::Chip { 5 } else { 9 };
        let need = 2 + pill_w(cx, root, m) + widths.iter().sum::<u16>() + gap * widths.len() as u16;
        if need <= band.width {
            break;
        }
    }
    let pw = pill_w(cx, root, mode).min(band.width);
    let fixed = pw + widths.iter().sum::<u16>();
    let gaps = widths.len() as u16;
    let free = band.width.saturating_sub(fixed);
    // Gaps grow with the width (capped so wide screens get air, not stretched lines);
    // what is left centres the drawing.
    let gap = if gaps == 0 {
        0
    } else {
        (free.saturating_sub(4) / gaps).clamp(4, 26)
    };
    let margin = free.saturating_sub(gap * gaps) / 2;
    let cy = band.y + h.saturating_sub(1) / 2;
    let pill = Rect::new(band.x + margin, cy, pw, 1);

    let mut nodes = Vec::new();
    let mut more = Vec::new();
    let mut x = pill.right() + gap;
    let rows = if tall { 2 } else { 1 };
    for (k, c) in cols.iter().enumerate() {
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
            (h.saturating_sub(rows) / (n - 1)).clamp(min_sp, if tall { 4 } else { 3 })
        };
        let span = (n - 1) * sp + rows;
        // Centre the column on the pill row (odd counts put a node on it: a straight line).
        let y0 = cy
            .saturating_sub((n - 1) * sp / 2)
            .clamp(band.y, band.bottom().saturating_sub(span).max(band.y));
        for (i, id) in c.iter().take(shown).enumerate() {
            nodes.push(Node {
                id: id.clone(),
                col: k + 1,
                x,
                y: y0 + i as u16 * sp,
                w: label_w(cx, id, mode, tall, nmax),
            });
        }
        if shown < c.len() {
            more.push((x + 2, y0 + shown as u16 * sp, c.len() - shown));
        }
        x += widths.get(k).copied().unwrap_or(0) + gap;
    }
    Plan {
        nodes,
        pill,
        tall,
        mode,
        more,
    }
}

const N: u8 = 1;
const E: u8 = 2;
const S: u8 = 4;
const W: u8 = 8;

/// Line canvas: per cell the directions a line leaves it, its colour and emphasis.
struct Grid {
    r: Rect,
    bits: Vec<u8>,
    lvl: Vec<u8>,
    tok: Vec<Tok>,
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
            pulse: vec![false; n],
        }
    }
    fn at(&self, x: u16, y: u16) -> Option<usize> {
        (x >= self.r.x && y >= self.r.y && x < self.r.right() && y < self.r.bottom()).then(|| {
            usize::from(y - self.r.y) * usize::from(self.r.width) + usize::from(x - self.r.x)
        })
    }
    fn mark(&mut self, x: u16, y: u16, b: u8, lvl: u8, tok: Tok) {
        if let Some(i) = self.at(x, y) {
            self.bits[i] |= b;
            if lvl >= self.lvl[i] {
                self.lvl[i] = lvl;
                self.tok[i] = tok;
            }
        }
    }
    /// A polyline through axis-aligned corner points; returns its cells in order.
    fn path(&mut self, pts: &[(u16, u16)], lvl: u8, tok: Tok) -> Vec<(u16, u16)> {
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
                self.mark(x, y, b, lvl, tok);
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
            let g = if !cx.uni() {
                if horiz {
                    "-"
                } else if vert {
                    "|"
                } else {
                    "+"
                }
            } else if self.pulse[i] && horiz {
                "━"
            } else if self.pulse[i] && vert {
                "┃"
            } else {
                match b {
                    _ if horiz => "─",
                    _ if vert => "│",
                    x if x == E | S => "╭",
                    x if x == W | S => "╮",
                    x if x == N | E => "╰",
                    x if x == N | W => "╯",
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
                        0.38
                    } else {
                        0.5
                    }
                }
                1 => {
                    if cx.dark() {
                        0.78
                    } else {
                        0.9
                    }
                }
                _ => 1.0,
            };
            let mut st = cx.ink(self.tok[i], a);
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
        Access::Password => "pwd",
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

/// Draw the network seen from `root` (columns up to `depth` hops) into `band`.
pub(super) fn draw(cx: &Cx, buf: &mut Buffer, band: Rect, root: &str, depth: usize, sel: Sel) {
    if band.width < 16 || band.height < 3 {
        return;
    }
    let s = cx.s;
    let p = plan(cx, band, root, depth);
    let find = |id: &str| p.nodes.iter().find(|n| n.id == id);
    let rows = if p.tall && p.mode == Mode::Full { 2 } else { 1 };
    let col_of = |id: &str| {
        if id == root {
            Some(0)
        } else {
            find(id).map(|n| n.col)
        }
    };
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
            let ns: Vec<&Node> = p.nodes.iter().filter(|n| n.col == col).collect();
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
                    let x0 = ns[i..j].iter().map(|n| n.x).min().unwrap_or(0) - 1;
                    let x1 = ns[i..j].iter().map(|n| n.x + n.w).max().unwrap_or(0) + 1;
                    let (y0, y1) = (ns[i].y, ns[j - 1].y + rows);
                    let a = if cx.dark() { 0.055 } else { 0.07 };
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

    let mut grid = Grid::new(band);
    let busy = |e: &LabEdge| {
        s.transfers
            .iter()
            .any(|t| (t.from == e.from && t.to == e.to) || (t.from == e.to && t.to == e.from))
    };
    // Links as (edge, left end, right end), left to right.
    let mut links: Vec<(&LabEdge, &str, &str)> = Vec::new();
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
        links.push((e, l, r));
    }
    let row_of = |id: &str| find(id).map_or(p.pill.y, |n| n.y);
    let right_of = |id: &str| find(id).map_or(p.pill.right(), |n| n.x + n.w + 1);
    // Trunk lanes in each gap start right of the widest label of the column before it.
    let col_right = |c: usize| {
        p.nodes
            .iter()
            .filter(|n| n.col == c)
            .map(|n| n.x + n.w)
            .max()
            .unwrap_or(p.pill.right())
    };
    let mut chips: Vec<(u16, u16, u16, String, Tok, u8, String)> = Vec::new();
    for (i, &(e, l, r)) in links.iter().enumerate() {
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
        let end = rn.x.saturating_sub(2);
        let same = col_of(l) == col_of(r);
        // Siblings sharing this link's source and side, farthest first: the farthest
        // target bends first so nested elbows never cross.
        let rank = |pred: &dyn Fn(u16) -> bool| -> u16 {
            let mine = ry.abs_diff(ly);
            links
                .iter()
                .enumerate()
                .filter(|(j, (_, l2, r2))| {
                    *l2 == l && *j != i && pred(row_of(r2)) && {
                        let d = row_of(r2).abs_diff(ly);
                        d > mine || (d == mine && *j < i)
                    }
                })
                .count() as u16
        };
        let (pts, run) = if same {
            // A bracket right of the column, between the two labels.
            let xb = col_right(rn.col) + 3;
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
                // Ports along the pill's top / bottom edge, nearest target rightmost.
                let up = ry < ly;
                let k = rank(&|y| (y < ly) == up && y != ly);
                let n_side = links
                    .iter()
                    .filter(|(_, l2, r2)| *l2 == l && (row_of(r2) < ly) == up && row_of(r2) != ly)
                    .count() as u16;
                let room = pill.width.saturating_sub(4) / 2;
                let slot = (n_side.saturating_sub(1) - k.min(n_side.saturating_sub(1))).min(room);
                let px = pill.right().saturating_sub(3 + 2 * slot);
                let py = if up { ly - 1 } else { ly + 1 };
                let mut v = vec![(px, py), (px, ry), (end, ry)];
                if py == ry {
                    v.remove(0);
                }
                // The first cell touches the pill: give it a stub toward it.
                grid.mark(px, py, if up { S } else { N }, lvl, tok);
                (v, (px, end, ry))
            }
        } else {
            let lc = col_of(l).unwrap_or(0);
            let base = col_right(lc) + 3;
            let k = rank(&|_| true);
            let lane = links
                .iter()
                .filter(|(_, l2, _)| col_of(l2) == Some(lc) && row_of(l2) < ly)
                .count() as u16
                + (k);
            let xb = (base + 2 * lane).min(end.saturating_sub(5));
            let sx = right_of(l);
            if ry == ly {
                (vec![(sx, ly), (end, ry)], (xb, end, ry))
            } else {
                (vec![(sx, ly), (xb, ly), (xb, ry), (end, ry)], (xb, end, ry))
            }
        };
        let cells = grid.path(&pts, lvl, tok);
        if busy(e) && e.up {
            // An active transfer: bright pulses travelling the link (frame-driven).
            let n = cells.len();
            let toward_left = s.transfers.iter().any(|t| t.to == l);
            for (j, &(x, y)) in cells.iter().enumerate() {
                let k = if toward_left { j } else { n - 1 - j };
                if (k + cx.c.frame as usize * 2) % 7 < 2 {
                    if let Some(ix) = grid.at(x, y) {
                        grid.pulse[ix] = true;
                    }
                }
            }
        }
        let (rx0, rx1, y) = run;
        chips.push((rx0, rx1, y, e.id.clone(), tok, lvl, l.to_string()));
    }
    grid.flush(cx, buf);

    // One chip per link, centred on its last straight run.
    for (x0, x1, y, id, tok, lvl, l) in chips {
        let Some(e) = s.edge(&id) else { continue };
        let len = x1.saturating_sub(x0);
        let from_left = l == e.from;
        let long = policy(cx, e, from_left, false);
        let short = policy(cx, e, from_left, true);
        let show_long = lvl >= 2 && cx.size() >= Size::S;
        let text = if show_long && super::width(&long, cx.uni()) + 4 <= len {
            long
        } else if len >= 5 {
            short
        } else {
            continue;
        };
        let w = super::width(&text, cx.uni()) + 2;
        let x = x0 + (len.saturating_sub(w) + 1) / 2;
        let a = match lvl {
            0 => 0.35,
            1 => 0.8,
            _ => 1.0,
        };
        let st = if cx.has_surface() {
            cx.rgb_bg(mix(
                cx.bg0(),
                cx.rgb(tok),
                if lvl >= 2 { 0.22 } else { 0.1 },
            ))
            .patch(cx.ink(tok, a))
        } else {
            cx.ink(tok, a)
        };
        let st = if lvl >= 2 {
            st.add_modifier(Modifier::BOLD)
        } else {
            st
        };
        let n = cx.put(buf, x, y, w, &format!(" {text} "), st);
        cx.hit(Rect::new(x, y, n, 1), format!("edge:{id}"));
    }

    draw_pill(cx, buf, p.pill, p.mode, root);

    let (sel_id, focused) = match sel {
        Sel::Node(id, f) => (Some(id), f),
        _ => (None, false),
    };
    let fade = edge_mode;
    for n in &p.nodes {
        let Some(d) = s.device(&n.id) else { continue };
        let area = Rect::new(n.x, n.y, n.w, rows);
        let picked = sel_id == Some(n.id.as_str());
        if picked && cx.has_surface() {
            let r = Rect::new(n.x.saturating_sub(1), n.y, n.w + 2, rows);
            cx.fill(
                buf,
                r,
                if focused {
                    cx.st(Tok::AccentBg)
                } else {
                    cx.st(Tok::SelectDimBg)
                },
            );
        }
        let (g, gst) = cx.status(d.status);
        cx.put(buf, n.x, n.y, 1, g, gst);
        let mut x = n.x + 2;
        x += cx.dchip(buf, x, n.y, d, false);
        let end = n.x + n.w;
        if p.mode > Mode::Chip {
            x += 1;
            let up = cx.up(d);
            let nst = if up && !fade {
                cx.ink(Tok::Fg, 1.0).add_modifier(Modifier::BOLD)
            } else {
                cx.ink(Tok::Muted, 0.9)
            };
            // Without tints the selected name itself carries the highlight.
            let nst = match (picked && !cx.has_surface(), focused) {
                (true, true) => nst.add_modifier(Modifier::REVERSED),
                (true, false) => nst.add_modifier(Modifier::UNDERLINED),
                _ => nst,
            };
            let meta = cx.meta(d);
            let inline = p.mode == Mode::Full && !p.tall;
            let room = end.saturating_sub(
                x + if inline {
                    meta.chars().count() as u16 + 1
                } else {
                    0
                },
            );
            x += cx.put(
                buf,
                x,
                n.y,
                room,
                &fit(&d.name, room as usize, cx.uni()),
                nst,
            );
            let mst = match d.status {
                DeviceStatus::Down => cx.ink(Tok::Err, 0.85),
                DeviceStatus::Stale => cx.ink(Tok::Warn, 0.9),
                _ => cx.ink(Tok::Muted, 0.95),
            };
            if inline {
                cx.put(buf, x + 1, n.y, end.saturating_sub(x + 1), &meta, mst);
            } else if p.tall && p.mode == Mode::Full {
                let y = n.y + 1;
                let mut mx = n.x + 6;
                mx += cx.put(buf, mx, y, end.saturating_sub(mx), &meta, mst);
                mx += 1;
                mx += super::tiles::mesh_badges(cx, buf, mx, y, d);
                if let Some(m) = d.metrics.as_ref().filter(|_| spark_w(cx, &d.id) > 0) {
                    let v: Vec<Option<f32>> = m.cpu_hist.iter().map(|v| Some(*v)).collect();
                    let x0 = end.saturating_sub(6).max(mx + 1);
                    super::tiles::spark(cx, buf, x0, y, end.saturating_sub(x0), &v, cx.heat(m.cpu));
                }
            }
        }
        // Phones get thumb-sized targets: the row above and below count too.
        let hit = if cx.size() == Size::Xs {
            Rect::new(n.x, n.y.saturating_sub(1).max(band.y), n.w, rows + 2)
        } else {
            area
        };
        cx.hit(hit.intersection(band), format!("device:{}", d.id));
    }

    for &(x, y, m) in &p.more {
        let t = if p.mode == Mode::Chip {
            format!("+{m}")
        } else {
            format!("+{m} more")
        };
        let w = cx.put(
            buf,
            x,
            y,
            band.right().saturating_sub(x),
            &t,
            cx.st(Tok::Muted),
        );
        cx.hit(Rect::new(x, y, w, 1), "palette:devices");
    }
}

/// The root device: a one-row tinted capsule (half-block caps; brackets without tints).
fn draw_pill(cx: &Cx, buf: &mut Buffer, pill: Rect, mode: Mode, root: &str) {
    let Some(d) = cx.s.device(root) else {
        return;
    };
    let tint: Rgb = mix(
        cx.bg0(),
        cx.rgb(Tok::Accent),
        if cx.dark() { 0.24 } else { 0.18 },
    );
    let (y, x0, x1) = (pill.y, pill.x, pill.right().saturating_sub(1));
    if cx.has_surface() {
        cx.fill(
            buf,
            Rect::new(x0 + 1, y, pill.width.saturating_sub(2), 1),
            cx.rgb_bg(tint),
        );
        if cx.uni() {
            cx.put(buf, x0, y, 1, "▐", cx.rgb_fg(tint));
            cx.put(buf, x1, y, 1, "▌", cx.rgb_fg(tint));
        }
    } else {
        let st = cx.st(Tok::Accent).add_modifier(Modifier::BOLD);
        cx.put(buf, x0, y, 1, "(", st);
        cx.put(buf, x1, y, 1, ")", st);
    }
    let mut x = x0 + 1 + u16::from(mode > Mode::Chip);
    x += cx.dchip(buf, x, y, d, false);
    if mode > Mode::Chip {
        x += 1;
        let room = x1.saturating_sub(x);
        let st = cx.ink(Tok::Fg, 1.0).add_modifier(Modifier::BOLD);
        cx.put(
            buf,
            x,
            y,
            room,
            &d.name,
            if cx.has_surface() {
                st.patch(cx.rgb_bg(tint))
            } else {
                st
            },
        );
    }
    let hit = if cx.size() == Size::Xs {
        Rect::new(pill.x, y.saturating_sub(1), pill.width, 3)
    } else {
        pill
    };
    cx.hit(hit, format!("device:{}", d.id));
}
