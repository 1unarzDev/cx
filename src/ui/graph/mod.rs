//! Pure network-graph core for the Network hub (design spec 10.1-10.4).
//!
//! * [`model`]: `Graph`/`Node`/`Edge`, the builder over plain [`model::Sources`], hotkeys.
//! * [`layout`]: layered layout, mesh enclosures, size classes, overflow, outline.
//! * [`route`]: orthogonal A* edge routing and port spreading.
//! * [`render`]: the ratatui widget ([`render::GraphView`]).
//! * [`hit`]: per-cell hit map and nearest-node-in-direction navigation.
//!
//! No I/O happens here; the UI builds [`model::Sources`] from the store and
//! observations, keeps a [`GraphCache`], and draws `GraphView` each frame.
// The UI wave wires this into the Network view; until then only tests use it.
#![cfg_attr(not(test), allow(dead_code, unused_imports))]

pub mod hit;
pub mod layout;
pub mod model;
pub mod render;
pub mod route;

pub use hit::{nearest, Dir, HitId};
pub use layout::{compute, CacheKey, GlyphSet, GraphLayout, Mode, SizeClass};
pub use model::{
    assign_hotkeys, build, Direction, EdgeKind, EdgePolicyInput, Graph, MeshMembership, Metrics,
    NodeKind, NodeState, Policy, Sources,
};
pub use render::{DefaultStyle, GraphView, Role, Selection};

use std::collections::BTreeMap;

/// Per-view cache: the layout (keyed by topology hash, area, size class and
/// glyph set) and the hotkey memory. Status, rtt, policy and selection
/// changes reuse the cached geometry.
#[derive(Default)]
pub struct GraphCache {
    layout: Option<GraphLayout>,
    hotkeys: BTreeMap<String, char>,
    /// Number of layout computations (for tests and perf counters).
    pub computes: u64,
}

impl GraphCache {
    /// Assigns stable hotkeys to `graph` (call once per rebuilt graph).
    pub fn assign_hotkeys(&mut self, graph: &mut Graph) {
        // Bounded by the letter pool: one entry per free letter (at most 38).
        self.hotkeys = assign_hotkeys(graph, &self.hotkeys);
    }
    /// The layout for `graph` in an area of `width` x `height`; recomputed only
    /// when the cache key changes, seeded by the previous layout's order.
    pub fn layout(
        &mut self,
        graph: &Graph,
        width: u16,
        height: u16,
        glyphs: GlyphSet,
    ) -> &GraphLayout {
        let key = CacheKey {
            topology: graph.topology_hash(),
            width,
            height,
            class: SizeClass::for_width(width),
            glyphs,
        };
        if self.layout.as_ref().map(|l| l.key) != Some(key) {
            let next = compute(graph, width, height, glyphs, self.layout.as_ref());
            self.computes += 1;
            self.layout = Some(next);
        }
        self.layout.as_ref().unwrap()
    }
}

/// Hit test at (x, y) relative to the graph area, accounting for outline scroll.
pub fn hit_at(layout: &GraphLayout, selection: Selection, x: u16, y: u16) -> Option<HitId> {
    if layout.mode == Mode::Outline {
        let scroll = render::outline_scroll(layout, selection, layout.height);
        let row = layout.outline.get(scroll + y as usize)?;
        return (x < layout.width).then_some(HitId::Node(row.node));
    }
    layout.hits.at(x, y)
}

#[cfg(test)]
mod tests;
