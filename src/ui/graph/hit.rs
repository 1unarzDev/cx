//! Per-cell hit map and spatial keyboard navigation (spec 10.4).
use super::layout::{GraphLayout, Mode};
use super::model::Graph;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HitId {
    /// Index into `Graph::nodes`.
    Node(usize),
    /// Index into `Graph::edges`.
    Edge(usize),
    /// Index into `GraphLayout::more`.
    More(usize),
    /// Index into `GraphLayout::enclosures` (title chip only).
    Enclosure(usize),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct HitMap {
    pub width: u16,
    pub height: u16,
    cells: Vec<Option<HitId>>,
}

impl HitMap {
    pub fn new(width: u16, height: u16) -> Self {
        HitMap {
            width,
            height,
            cells: vec![None; width as usize * height as usize],
        }
    }
    /// What sits at (x, y), relative to the graph area.
    pub fn at(&self, x: u16, y: u16) -> Option<HitId> {
        if x >= self.width || y >= self.height {
            return None;
        }
        self.cells[y as usize * self.width as usize + x as usize]
    }
    fn set(&mut self, x: i32, y: i32, id: HitId, overwrite: bool) {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            return;
        }
        let i = y as usize * self.width as usize + x as usize;
        if overwrite || self.cells[i].is_none() {
            self.cells[i] = Some(id);
        }
    }
    /// Priority: nodes and pills, then edge cells, then the enclosure title,
    /// then a 1-cell halo around edges (touch targets).
    pub fn build(g: &Graph, layout: &GraphLayout) -> HitMap {
        let mut m = HitMap::new(layout.width, layout.height);
        if layout.mode == Mode::Outline {
            for (row, r) in layout.outline.iter().enumerate() {
                if row >= layout.height as usize {
                    break;
                }
                for x in 0..layout.width {
                    m.set(x as i32, row as i32, HitId::Node(r.node), true);
                }
            }
            return m;
        }
        let _ = g;
        for p in &layout.nodes {
            for y in p.rect.y..p.rect.bottom() {
                for x in p.rect.x..p.rect.right() {
                    m.set(x as i32, y as i32, HitId::Node(p.node), true);
                }
            }
        }
        for (k, p) in layout.more.iter().enumerate() {
            for y in p.rect.y..p.rect.bottom() {
                for x in p.rect.x..p.rect.right() {
                    m.set(x as i32, y as i32, HitId::More(k), true);
                }
            }
        }
        for r in &layout.routes {
            for &(x, y, _) in &r.cells {
                m.set(x as i32, y as i32, HitId::Edge(r.edge), false);
            }
            if let Some(l) = r.label {
                for x in l.x..l.x + l.len {
                    m.set(x as i32, l.y as i32, HitId::Edge(r.edge), false);
                }
            }
        }
        for (k, e) in layout.enclosures.iter().enumerate() {
            if e.title {
                let len = (e.mesh.chars().count() as u16 + 4).min(e.rect.width.saturating_sub(4));
                for x in e.rect.x + 2..e.rect.x + 2 + len {
                    m.set(x as i32, e.rect.y as i32, HitId::Enclosure(k), false);
                }
            }
        }
        for r in &layout.routes {
            for &(x, y, _) in &r.cells {
                for dy in -1..=1 {
                    for dx in -1..=1 {
                        m.set(x as i32 + dx, y as i32 + dy, HitId::Edge(r.edge), false);
                    }
                }
            }
        }
        m
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dir {
    Up,
    Down,
    Left,
    Right,
}

/// Nearest node in a direction: angle within ±60° of the key's direction,
/// then the shortest (aspect-corrected) distance; ties by smaller angle, then index.
/// In outline mode Up/Down move by row, Left goes to the parent, Right to the first child.
pub fn nearest(layout: &GraphLayout, from: usize, dir: Dir) -> Option<usize> {
    if layout.mode == Mode::Outline {
        let rows = &layout.outline;
        let k = rows.iter().position(|r| r.node == from)?;
        return match dir {
            Dir::Up => k.checked_sub(1).map(|k| rows[k].node),
            Dir::Down => rows.get(k + 1).map(|r| r.node),
            Dir::Left => rows[..k]
                .iter()
                .rev()
                .find(|r| r.depth + 1 == rows[k].depth)
                .map(|r| r.node),
            Dir::Right => rows
                .get(k + 1)
                .filter(|r| r.depth == rows[k].depth + 1)
                .map(|r| r.node),
        };
    }
    let src = layout.placed(from)?;
    let c = |r: ratatui::layout::Rect| {
        (
            r.x as f64 + r.width as f64 / 2.0,
            (r.y as f64 + r.height as f64 / 2.0) * 2.0,
        )
    };
    let (sx, sy) = c(src.rect);
    let (ux, uy) = match dir {
        Dir::Up => (0.0, -1.0),
        Dir::Down => (0.0, 1.0),
        Dir::Left => (-1.0, 0.0),
        Dir::Right => (1.0, 0.0),
    };
    let mut best: Option<(f64, f64, usize)> = None;
    for p in &layout.nodes {
        if p.node == from {
            continue;
        }
        let (x, y) = c(p.rect);
        let (dx, dy) = (x - sx, y - sy);
        let d = (dx * dx + dy * dy).sqrt();
        if d == 0.0 {
            continue;
        }
        let cos = (dx * ux + dy * uy) / d;
        let angle = cos.clamp(-1.0, 1.0).acos().to_degrees();
        if angle > 60.0 + 1e-9 {
            continue;
        }
        let cand = (d, angle, p.node);
        if best.is_none_or(|b| (cand.0, cand.1, cand.2) < (b.0, b.1, b.2)) {
            best = Some(cand);
        }
    }
    best.map(|b| b.2)
}
