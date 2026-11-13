//! Arrow and `h j k l` moves between items drawn on a plane (the hub's nebula).
//!
//! From an item, a direction picks the nearest item inside a cone of ±45° around it,
//! the angle weighed over the distance; only when that cone is empty does it widen to
//! ±80°. An item in the same group (a bloomed cluster) wins over one outside it, so the
//! keys move inside a cluster and leave it at its edge. Moves are paired first: the
//! cone relation is symmetric, so A→right=B is matched with B→left=A, best pairs first,
//! and those moves reverse exactly. Where several items share one neighbour (three
//! nodes left of the root), the ones left over still move there (one way), and going
//! back lands on the neighbour's own pair, which leads straight back again.
//! A move never goes against its key (right never lands further left).

/// One navigable item: its centre in cells and its group (cluster).
pub(super) struct Item {
    pub at: (f32, f32),
    pub group: usize,
}

#[cfg(test)]
pub(super) const LEFT: usize = 0;
pub(super) const RIGHT: usize = 1;
#[cfg(test)]
pub(super) const UP: usize = 2;
pub(super) const DOWN: usize = 3;
// (Directions as `moves` indexes them: left, right, up, down.)
const DIRS: [(f32, f32); 4] = [(-1.0, 0.0), (1.0, 0.0), (0.0, -1.0), (0.0, 1.0)];

pub(super) fn opposite(d: usize) -> usize {
    d ^ 1
}

/// (other group, score) of `b` seen from `a` toward `d`, inside a cone of `cone`
/// degrees; None outside it. A cell is about twice as tall as wide, so rows count
/// double.
fn rank(items: &[Item], a: usize, b: usize, d: usize, cone: f32) -> Option<(bool, f32)> {
    let (p, q) = (items[a].at, items[b].at);
    let v = (q.0 - p.0, (q.1 - p.1) * 2.0);
    let (dx, dy) = DIRS[d];
    let along = v.0 * dx + v.1 * dy;
    if along <= 1e-3 {
        return None;
    }
    let side = (v.0 * dy - v.1 * dx).abs();
    let angle = side.atan2(along);
    if angle > cone.to_radians() + 1e-4 {
        return None;
    }
    let dist = (v.0 * v.0 + v.1 * v.1).sqrt();
    Some((items[a].group != items[b].group, dist * (1.0 + 4.0 * angle)))
}

/// For every item, where each direction (`LEFT RIGHT UP DOWN`) moves.
pub(super) fn moves(items: &[Item]) -> Vec<[Option<usize>; 4]> {
    let n = items.len();
    let mut mv = vec![[None; 4]; n];
    for d in [RIGHT, DOWN] {
        let o = opposite(d);
        for cone in [45.0, 80.0] {
            // Pairs, best first: a toward d, b toward o (the same cone, mirrored).
            let mut pairs: Vec<(bool, f32, usize, usize)> = Vec::new();
            for a in 0..n {
                for b in 0..n {
                    if a == b || mv[a][d].is_some() || mv[b][o].is_some() {
                        continue;
                    }
                    if let Some((x, s)) = rank(items, a, b, d, cone) {
                        pairs.push((x, s, a, b));
                    }
                }
            }
            pairs.sort_by(|p, q| {
                (p.0, p.1, p.2, p.3)
                    .partial_cmp(&(q.0, q.1, q.2, q.3))
                    .unwrap()
            });
            for (_, _, a, b) in pairs {
                if mv[a][d].is_none() && mv[b][o].is_none() {
                    mv[a][d] = Some(b);
                    mv[b][o] = Some(a);
                }
            }
            // The rest: one way to the best item in the cone (either direction).
            for dir in [d, o] {
                for a in 0..n {
                    if mv[a][dir].is_some() {
                        continue;
                    }
                    let mut best: Option<((bool, f32), usize)> = None;
                    for b in (0..n).filter(|&b| b != a) {
                        if let Some(r) = rank(items, a, b, dir, cone) {
                            if best.is_none_or(|(k, _)| r < k) {
                                best = Some((r, b));
                            }
                        }
                    }
                    mv[a][dir] = best.map(|(_, b)| b);
                }
            }
        }
    }
    mv
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(v: &[(f32, f32, usize)]) -> Vec<Item> {
        v.iter()
            .map(|&(x, y, g)| Item {
                at: (x, y),
                group: g,
            })
            .collect()
    }

    #[test]
    fn a_star_pairs_the_nearest_and_never_strands_the_rest() {
        // The root and three nodes to its left, one above it.
        let it = at(&[
            (50.0, 10.0, 0),
            (30.0, 6.0, 1),
            (30.0, 10.0, 1),
            (30.0, 14.0, 1),
            (50.0, 4.0, 2),
        ]);
        let mv = moves(&it);
        assert_eq!(mv[0][LEFT], Some(2));
        assert_eq!(mv[2][RIGHT], Some(0));
        // The lower one reaches the root too (one way: back leads to the root's pair);
        // the upper one is better aligned with the node above the root.
        assert_eq!(mv[3][RIGHT], Some(0));
        assert_eq!(mv[1][RIGHT], Some(4));
        assert_eq!(mv[0][UP], Some(4));
        assert_eq!(mv[4][DOWN], Some(0));
        // Inside the left group, up and down walk it.
        assert_eq!(mv[2][UP], Some(1));
        assert_eq!(mv[2][DOWN], Some(3));
    }

    #[test]
    fn the_cone_widens_only_when_empty() {
        let it = at(&[(10.0, 10.0, 0), (20.0, 18.0, 0), (40.0, 10.5, 0)]);
        let mv = moves(&it);
        // Right: the far node inside the cone, not the near one 58° off it (32° from
        // straight down, so down's).
        assert_eq!(mv[0][RIGHT], Some(2));
        assert_eq!(mv[0][DOWN], Some(1));
        let it = at(&[(10.0, 10.0, 0), (30.0, 13.0, 0)]);
        let mv = moves(&it);
        // 30 columns right and 6 (doubled) rows down: 17°, right; down widens to it.
        assert_eq!(mv[0][RIGHT], Some(1));
        assert_eq!(mv[0][DOWN], Some(1));
    }
}
