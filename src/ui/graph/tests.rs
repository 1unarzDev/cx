use super::layout::{graph_with_overflow, Orientation};
use super::route::{DOWN, LEFT, RIGHT, UP};
use super::*;
use crate::model::Device;
use ratatui::{buffer::Buffer, layout::Rect, style::Color, widgets::Widget};
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};

const NOW: u64 = 1_800_000_000;

fn device(id: &str, target: Option<&str>, status: &str, age: u64) -> Device {
    Device {
        id: id.into(),
        name: id.into(),
        target: target.map(Into::into),
        account: "robot".into(),
        host: target.unwrap_or("localhost").into(),
        status: status.into(),
        observed_at: NOW - age,
    }
}

/// Viewer + 5 devices (one jump host), cloudflare x2, ssh-overlay x3, a one-way
/// edge, a password edge, a down device and a LAN cluster with 4 neighbours.
pub(super) fn fleet_sources() -> Sources {
    let devices = vec![
        device("tranquility", None, "local", 1),
        device("innovation", Some("innovation"), "reachable", 3),
        device("lab-arm", Some("lab-arm"), "reachable", 3),
        device("jetson-01", Some("robot@jetson-01"), "reachable", 20),
        device("field-rover", Some("field-rover"), "unreachable", 300),
        device("nas", Some("nas"), "reachable", 30),
    ];
    let mut routes = BTreeMap::new();
    routes.insert("robot@jetson-01".to_string(), vec!["lab-arm".to_string()]);
    let mut policies = BTreeMap::new();
    for id in ["innovation", "lab-arm", "jetson-01", "field-rover", "nas"] {
        policies.insert(
            id.to_string(),
            crate::store::DevicePolicy {
                access: if id == "innovation" || id == "lab-arm" {
                    crate::store::AccessMode::Core
                } else {
                    crate::store::AccessMode::Directed
                },
                availability: Default::default(),
            },
        );
    }
    let candidates = json!({"observed_at": NOW, "candidates": [
        {"address":"192.0.2.10","interface":"enp5s0","ssh":{"state":"open"}},
        {"address":"192.0.2.11","interface":"enp5s0","ssh":{"state":"closed"}},
        {"address":"192.0.2.12","interface":"enp5s0","ssh":{"state":"unknown"}},
        {"address":"192.0.2.13","interface":"enp5s0","hostname":"printer"},
    ]});
    let mut metrics = BTreeMap::new();
    for (id, rtt, ses) in [
        ("tranquility", None, 2),
        ("innovation", Some(8), 3),
        ("lab-arm", Some(12), 1),
        ("jetson-01", Some(31), 0),
        ("nas", Some(3), 0),
    ] {
        metrics.insert(
            id.to_string(),
            Metrics {
                rtt_ms: rtt,
                sessions: Some(ses),
                ..Default::default()
            },
        );
    }
    Sources {
        devices,
        routes,
        policies,
        access: None,
        observations: vec![("tranquility".into(), candidates)],
        edge_policies: vec![
            EdgePolicyInput {
                from: "tranquility".into(),
                to: "nas".into(),
                policy: Policy::Password,
            },
            EdgePolicyInput {
                from: "lab-arm".into(),
                to: "jetson-01".into(),
                policy: Policy::OneWay,
            },
        ],
        mesh: vec![
            MeshMembership {
                backend: "cloudflare".into(),
                members: vec!["tranquility".into(), "innovation".into()],
            },
            MeshMembership {
                backend: "ssh-overlay".into(),
                members: vec!["lab-arm".into(), "jetson-01".into(), "field-rover".into()],
            },
        ],
        metrics,
        expanded: BTreeSet::new(),
        now: NOW,
    }
}

pub(super) fn fleet() -> Graph {
    let mut g = build(&fleet_sources());
    assign_hotkeys(&mut g, &BTreeMap::new());
    g
}

fn idx(g: &Graph, id: &str) -> usize {
    g.node_index(id).unwrap_or_else(|| panic!("no node {id}"))
}

fn edge_idx(g: &Graph, from: &str, to: &str) -> usize {
    g.edges
        .iter()
        .position(|e| e.from == from && e.to == to)
        .unwrap_or_else(|| panic!("no edge {from}->{to}"))
}

const SIZES: [(u16, u16); 4] = [(80, 24), (120, 40), (200, 60), (40, 20)];

fn assert_geometry(g: &Graph, l: &GraphLayout) {
    let rects: Vec<Rect> = l
        .nodes
        .iter()
        .map(|p| p.rect)
        .chain(l.more.iter().map(|m| m.rect))
        .collect();
    for (a, ra) in rects.iter().enumerate() {
        assert!(
            ra.right() <= l.width && ra.bottom() <= l.height,
            "{ra:?} out of area"
        );
        for rb in rects.iter().skip(a + 1) {
            assert!(!ra.intersects(*rb), "overlap {ra:?} {rb:?}");
        }
    }
    for r in &l.routes {
        assert!(!r.cells.is_empty());
        for &(x, y, _) in &r.cells {
            assert!(x < l.width && y < l.height);
            for rect in &rects {
                assert!(
                    !(x >= rect.x && x < rect.right() && y >= rect.y && y < rect.bottom()),
                    "edge {} passes through node {rect:?} at {x},{y}",
                    r.edge
                );
            }
        }
        // Contiguous 4-connected path from port to port.
        let mut prev = (r.src_port.0, r.src_port.1);
        for &(x, y, _) in r
            .cells
            .iter()
            .chain(std::iter::once(&(r.dst_port.0, r.dst_port.1, 0)))
        {
            let d = (x as i32 - prev.0 as i32).abs() + (y as i32 - prev.1 as i32).abs();
            assert_eq!(d, 1, "edge {} not contiguous at {x},{y}", r.edge);
            prev = (x, y);
        }
        // Ports sit on the endpoint borders.
        let e = &g.edges[r.edge];
        for (port, id) in [(r.src_port, &e.from), (r.dst_port, &e.to)] {
            let p = l.placed(idx(g, id)).unwrap();
            let (x, y) = (port.0, port.1);
            let on_border = (x == p.rect.x
                || x == p.rect.right() - 1
                || y == p.rect.y
                || y == p.rect.bottom() - 1)
                && x >= p.rect.x
                && x < p.rect.right()
                && y >= p.rect.y
                && y < p.rect.bottom();
            assert!(on_border, "port {port:?} not on {id}");
        }
    }
    // Two routes never run along the same cell in the same axis.
    let mut seen: BTreeMap<(u16, u16), u8> = BTreeMap::new();
    for r in &l.routes {
        for &(x, y, m) in &r.cells {
            let axis = if m == LEFT | RIGHT {
                1
            } else if m == UP | DOWN {
                2
            } else {
                3
            };
            let prev = seen.entry((x, y)).or_insert(0);
            assert!(*prev & axis == 0, "edges overlap at {x},{y}");
            *prev |= axis;
        }
    }
}

// ---------------------------------------------------------------------------
// Builder
// ---------------------------------------------------------------------------

#[test]
fn builder_assembles_fixture_fleet() {
    let g = fleet();
    let kinds: Vec<(String, NodeKind)> = g.nodes.iter().map(|n| (n.id.clone(), n.kind)).collect();
    assert_eq!(kinds[0], ("tranquility".into(), NodeKind::Viewer));
    assert_eq!(
        g.nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Device)
            .count(),
        5
    );
    let lan = &g.nodes[idx(&g, "lan:tranquility:enp5s0")];
    assert_eq!(lan.kind, NodeKind::LanCluster);
    assert_eq!(lan.count, Some(4));
    assert!(!g.nodes.iter().any(|n| n.kind == NodeKind::Neighbor));
    // Jump route: viewer -> lab-arm -> jetson-01.
    let j = &g.edges[edge_idx(&g, "lab-arm", "jetson-01")];
    assert_eq!(j.kind, EdgeKind::Jump);
    assert_eq!(j.policy, Policy::OneWay);
    assert_eq!(j.direction, Direction::Forward);
    assert!(!g
        .edges
        .iter()
        .any(|e| e.from == "tranquility" && e.to == "jetson-01"));
    let pwd = &g.edges[edge_idx(&g, "tranquility", "nas")];
    assert_eq!(pwd.policy, Policy::Password);
    let core = &g.edges[edge_idx(&g, "tranquility", "innovation")];
    assert_eq!(
        (core.kind, core.policy, core.direction),
        (EdgeKind::Mesh, Policy::Key, Direction::Both)
    );
    assert_eq!(g.nodes[idx(&g, "field-rover")].state, NodeState::Down);
    assert_eq!(g.nodes[idx(&g, "field-rover")].metrics.age_secs, Some(300));
    assert_eq!(
        g.nodes[idx(&g, "innovation")].mesh.as_deref(),
        Some("cloudflare")
    );
    assert_eq!(
        g.nodes[idx(&g, "jetson-01")].mesh.as_deref(),
        Some("ssh-overlay")
    );
}

#[test]
fn builder_creates_jump_nodes_expands_lan_and_reads_access_graph() {
    let mut src = fleet_sources();
    src.routes
        .insert("nas".into(), vec!["bastion.example".into()]);
    src.expanded.insert("lan:tranquility:enp5s0".into());
    src.access = Some(crate::access::Graph {
        revision: 1,
        nodes: vec![],
        edges: vec![
            crate::access::Edge {
                from: "innovation".into(),
                to: "lab-arm".into(),
                allowed: true,
                authenticated_at: Some(NOW - 5),
            },
            crate::access::Edge {
                from: "tranquility".into(),
                to: "field-rover".into(),
                allowed: false,
                authenticated_at: None,
            },
        ],
        grants: vec![],
    });
    let g = build(&src);
    let jump = &g.nodes[idx(&g, "jump:bastion.example")];
    assert_eq!(jump.kind, NodeKind::Jump);
    edge_idx(&g, "tranquility", "jump:bastion.example");
    assert_eq!(
        g.edges[edge_idx(&g, "jump:bastion.example", "nas")].kind,
        EdgeKind::Jump
    );
    assert_eq!(
        g.nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Neighbor)
            .count(),
        4
    );
    let peer = &g.edges[edge_idx(&g, "innovation", "lab-arm")];
    assert_eq!(peer.evidence.authenticated_at, Some(NOW - 5));
    assert_eq!(
        g.edges[edge_idx(&g, "tranquility", "field-rover")].policy,
        Policy::None
    );
    // Neighbour state comes from the bounded SSH port evidence only.
    let open = &g.nodes[idx(&g, "nb:tranquility:enp5s0:192.0.2.10")];
    assert_eq!(open.state, NodeState::Up);
    assert_eq!(
        g.nodes[idx(&g, "nb:tranquility:enp5s0:192.0.2.13")].label,
        "printer"
    );
}

#[test]
fn status_strings_map_to_evidence_states() {
    assert_eq!(state_from_status("reachable", NOW, NOW), NodeState::Up);
    assert_eq!(state_from_status("unreachable", NOW, NOW), NodeState::Down);
    assert_eq!(state_from_status("checking", NOW, NOW), NodeState::Unknown);
    assert_eq!(state_from_status("", 0, NOW), NodeState::Unknown);
    assert_eq!(
        state_from_status("reachable", NOW - 3600, NOW),
        NodeState::Stale
    );
    assert_eq!(state_from_status("refused", NOW, NOW), NodeState::Down);
}
use super::model::state_from_status;

#[test]
fn labels_are_bounded_and_control_free() {
    let mut src = fleet_sources();
    src.devices[1].name = format!("evil\x1b[31m{}", "x".repeat(80));
    let g = build(&src);
    let n = &g.nodes[idx(&g, "innovation")];
    assert!(n.label.chars().count() <= 32);
    assert!(!n.label.chars().any(char::is_control));
}

// ---------------------------------------------------------------------------
// Hotkeys
// ---------------------------------------------------------------------------

#[test]
fn hotkeys_avoid_reserved_keys_prefer_initials_and_stay_stable() {
    let mut g = build(&fleet_sources());
    let mem = assign_hotkeys(&mut g, &BTreeMap::new());
    let keys: BTreeMap<&str, char> = g
        .nodes
        .iter()
        .filter_map(|n| n.hotkey.map(|k| (n.id.as_str(), k)))
        .collect();
    let reserved: BTreeSet<char> = model::RESERVED_KEYS.chars().collect();
    for k in keys.values() {
        assert!(!reserved.contains(k));
    }
    let unique: BTreeSet<char> = keys.values().copied().collect();
    assert_eq!(unique.len(), keys.len());
    assert_eq!(
        keys.len(),
        6,
        "viewer and devices get keys, LAN clusters do not"
    );
    assert_eq!(keys["tranquility"], 't');
    assert_eq!(keys["innovation"], 'i');
    assert_eq!(keys["field-rover"], 'd');
    assert_eq!(keys["jetson-01"], 'o');
    assert_eq!(keys["lab-arm"], 'b');
    assert_eq!(keys["nas"], 'c');
    // Adding a node and removing another keeps everyone else's letter.
    let mut src = fleet_sources();
    src.devices.retain(|d| d.id != "nas");
    src.devices
        .push(device("ithaca", Some("ithaca"), "reachable", 1));
    let mut g2 = build(&src);
    let mem2 = assign_hotkeys(&mut g2, &mem);
    for n in g2.nodes.iter().filter(|n| n.hotkey.is_some()) {
        if let Some(&k) = keys.get(n.id.as_str()) {
            assert_eq!(n.hotkey, Some(k), "{} changed", n.id);
        }
    }
    let ithaca = g2.nodes[idx(&g2, "ithaca")].hotkey.unwrap();
    assert_ne!(ithaca, keys["nas"], "absent node keeps a soft claim");
    // The removed node gets its letter back when it returns.
    let mut g3 = build(&fleet_sources());
    assign_hotkeys(&mut g3, &mem2);
    assert_eq!(g3.nodes[idx(&g3, "nas")].hotkey, Some(keys["nas"]));
    assert_eq!(g.node_for_hotkey('t'), Some(0));
}

#[test]
fn hotkeys_fall_back_to_uppercase_then_pool() {
    let mut src = fleet_sources();
    for k in 0..40 {
        src.devices.push(device(
            &format!("tt-{k}"),
            Some(&format!("tt{k}")),
            "reachable",
            1,
        ));
    }
    let mut g = build(&src);
    assign_hotkeys(&mut g, &BTreeMap::new());
    let keys: Vec<char> = g.nodes.iter().filter_map(|n| n.hotkey).collect();
    let unique: BTreeSet<char> = keys.iter().copied().collect();
    assert_eq!(unique.len(), keys.len());
    assert!(keys.contains(&'T'));
    // 52 letters minus 14 reserved = 38 keys; the rest stay unassigned.
    assert_eq!(keys.len(), 52 - model::RESERVED_KEYS.len());
}

// ---------------------------------------------------------------------------
// Layout
// ---------------------------------------------------------------------------

#[test]
fn layout_is_deterministic_and_valid_at_all_sizes() {
    let g = fleet();
    for (w, h) in SIZES {
        let a = compute(&g, w, h, GlyphSet::Unicode, None);
        let b = compute(&g, w, h, GlyphSet::Unicode, None);
        assert_eq!(a.nodes, b.nodes);
        assert_eq!(a.routes, b.routes);
        assert_eq!(a.enclosures, b.enclosures);
        assert_eq!(a.outline, b.outline);
        if a.mode == Mode::Graph {
            assert_geometry(&g, &a);
            assert_eq!(a.routes.len(), g.edges.len(), "all edges routed at {w}x{h}");
        }
    }
    assert_eq!(
        compute(&g, 40, 20, GlyphSet::Unicode, None).mode,
        Mode::Outline
    );
    for (w, h) in [(80, 24), (120, 40), (200, 60)] {
        let l = compute(&g, w, h, GlyphSet::Unicode, None);
        assert_eq!(l.mode, Mode::Graph, "{w}x{h}");
        assert_eq!(l.orientation, Orientation::LeftRight);
    }
}

#[test]
fn ranks_follow_hops_and_lan_sits_beside_its_observer() {
    let g = fleet();
    let l = compute(&g, 200, 60, GlyphSet::Unicode, None);
    let rank = |id: &str| l.placed(idx(&g, id)).unwrap().rank;
    assert_eq!(rank("tranquility"), 0);
    assert_eq!(rank("innovation"), 1);
    assert_eq!(rank("lab-arm"), 1);
    assert_eq!(rank("jetson-01"), 2);
    assert_eq!(rank("lan:tranquility:enp5s0"), 1);
}

#[test]
fn mesh_members_are_contiguous_and_enclosed() {
    let g = fleet();
    for (w, h) in [(80, 24), (120, 40), (200, 60)] {
        let l = compute(&g, w, h, GlyphSet::Unicode, None);
        for mesh in ["cloudflare", "ssh-overlay"] {
            let boxes: Vec<&layout::Enclosure> =
                l.enclosures.iter().filter(|e| e.mesh == mesh).collect();
            assert!(!boxes.is_empty(), "{mesh} at {w}x{h}");
            assert_eq!(boxes.iter().filter(|e| e.title).count(), 1);
            for p in &l.nodes {
                let member = g.nodes[p.node].mesh.as_deref() == Some(mesh);
                let inside = boxes.iter().any(|e| e.rect.union(p.rect) == e.rect);
                assert_eq!(
                    member, inside,
                    "{} vs {mesh} at {w}x{h}",
                    g.nodes[p.node].id
                );
            }
        }
        // One enclosure per mesh in the fixture: membership reads as one group.
        assert_eq!(l.enclosures.len(), 2, "{w}x{h}: {:?}", l.enclosures);
    }
}

#[test]
fn layout_is_stable_under_small_topology_changes() {
    let g = fleet();
    let before = compute(&g, 120, 40, GlyphSet::Unicode, None);
    let mut src = fleet_sources();
    src.devices
        .push(device("aurora", Some("aurora"), "reachable", 1));
    let mut g2 = build(&src);
    assign_hotkeys(&mut g2, &BTreeMap::new());
    let after = compute(&g2, 120, 40, GlyphSet::Unicode, Some(&before));
    // Every surviving node keeps its rank and relative order within the rank.
    for r in 0..3 {
        let order = |l: &GraphLayout, g: &Graph| -> Vec<String> {
            let mut v: Vec<(usize, String)> = l
                .order
                .iter()
                .filter(|(id, (rank, _))| *rank == r && g.node_index(id).is_some())
                .map(|(id, (_, p))| (*p, id.clone()))
                .collect();
            v.sort();
            v.into_iter().map(|x| x.1).collect()
        };
        let a = order(&before, &g);
        let b: Vec<String> = order(&after, &g2)
            .into_iter()
            .filter(|id| id != "aurora")
            .collect();
        assert_eq!(a, b, "rank {r}");
    }
    // Ranks and columns do not move.
    for p in &before.nodes {
        let id = &g.nodes[p.node].id;
        let q = after.placed(idx(&g2, id)).unwrap();
        assert_eq!((p.rank, p.rect.x), (q.rank, q.rect.x), "{id}");
    }
    // Removing aurora again restores the original layout exactly.
    let back = compute(&g, 120, 40, GlyphSet::Unicode, Some(&after));
    assert_eq!(back.nodes, before.nodes);
    assert_eq!(back.routes, before.routes);
}

#[test]
fn overflow_folds_into_more_pills_without_overlap() {
    let mut src = fleet_sources();
    for k in 0..9 {
        src.devices.push(device(
            &format!("node-{k:02}"),
            Some(&format!("n{k}")),
            "reachable",
            1,
        ));
    }
    let mut g = build(&src);
    assign_hotkeys(&mut g, &BTreeMap::new());
    let l = compute(&g, 80, 24, GlyphSet::Unicode, None);
    assert_eq!(l.mode, Mode::Graph);
    assert!(!l.more.is_empty(), "rank 1 folds into +N more");
    let hidden: usize = l.more.iter().map(|m| m.hidden.len()).sum();
    assert_eq!(hidden + l.nodes.len(), g.nodes.len());
    assert_geometry(&g, &l);
    // More than 24 nodes: outline.
    for k in 9..30 {
        src.devices.push(device(
            &format!("node-{k:02}"),
            Some(&format!("n{k}")),
            "reachable",
            1,
        ));
    }
    let g = build(&src);
    assert_eq!(
        compute(&g, 200, 60, GlyphSet::Unicode, None).mode,
        Mode::Outline
    );
}

#[test]
fn expanded_lan_cluster_collapses_first_when_space_runs_out() {
    let mut src = fleet_sources();
    src.expanded.insert("lan:tranquility:enp5s0".into());
    let mut g = build(&src);
    assign_hotkeys(&mut g, &BTreeMap::new());
    let big = compute(&g, 200, 60, GlyphSet::Unicode, None);
    assert_eq!(big.mode, Mode::Graph);
    let shown = big
        .nodes
        .iter()
        .filter(|p| g.nodes[p.node].kind == NodeKind::Neighbor)
        .count();
    assert_eq!(shown, 4);
    assert_geometry(&g, &big);
    let small = compute(&g, 80, 18, GlyphSet::Unicode, None);
    assert_eq!(small.mode, Mode::Graph);
    assert!(small
        .nodes
        .iter()
        .all(|p| g.nodes[p.node].kind != NodeKind::Neighbor));
    assert_geometry(&g, &small);
}

#[test]
fn narrow_wide_graphs_switch_to_row_bands() {
    // Five ranks do not fit 70 columns side by side.
    let mut src = Sources {
        devices: vec![device("v", None, "local", 1)],
        now: NOW,
        ..Default::default()
    };
    let mut prev: Vec<String> = Vec::new();
    for k in 0..4 {
        let id = format!("hop{k}");
        src.devices.push(device(&id, Some(&id), "reachable", 1));
        if !prev.is_empty() {
            src.routes.insert(id.clone(), prev.clone());
        }
        prev.push(id);
    }
    let g = build(&src);
    let l = compute(&g, 70, 40, GlyphSet::Unicode, None);
    assert_eq!(l.mode, Mode::Graph);
    assert_eq!(l.orientation, Orientation::TopBottom);
    assert_geometry(&g, &l);
}

#[test]
fn isotonic_fit_is_monotone_and_centred() {
    let r = layout_isotonic(&[5.0, 1.0, 3.0]);
    assert!(r.windows(2).all(|w| w[0] <= w[1]));
    assert!((r.iter().sum::<f64>() - 9.0).abs() < 1e-9);
    let r = layout_isotonic(&[f64::NAN, 2.0, f64::NAN]);
    assert_eq!(r, vec![2.0, 2.0, 2.0]);
}
fn layout_isotonic(t: &[f64]) -> Vec<f64> {
    layout::isotonic(t)
}

// ---------------------------------------------------------------------------
// Routing and glyphs
// ---------------------------------------------------------------------------

#[test]
fn box_glyphs_merge_by_mask_and_weight() {
    use render::box_glyph;
    let u = GlyphSet::Unicode;
    assert_eq!(box_glyph([1, 1, 1, 0], u), '├');
    assert_eq!(box_glyph([1, 1, 1, 1], u), '┼');
    assert_eq!(box_glyph([2, 2, 2, 2], u), '╋');
    assert_eq!(box_glyph([1, 2, 1, 0], u), '┝');
    assert_eq!(box_glyph([1, 0, 1, 2], u), '┥');
    assert_eq!(box_glyph([0, 2, 0, 2], u), '━');
    assert_eq!(box_glyph([0, 1, 1, 0], u), '╭');
    assert_eq!(box_glyph([0, 2, 2, 0], u), '┏');
    assert_eq!(box_glyph([2, 1, 2, 1], u), '╂');
    assert_eq!(box_glyph([1, 2, 1, 2], u), '┿');
    let a = GlyphSet::Ascii;
    assert_eq!(box_glyph([0, 2, 0, 2], a), '=');
    assert_eq!(box_glyph([0, 1, 0, 1], a), '-');
    assert_eq!(box_glyph([1, 0, 1, 0], a), '|');
    assert_eq!(box_glyph([1, 1, 1, 0], a), '+');
}

fn render_text(
    g: &Graph,
    l: &GraphLayout,
    sel: Selection,
    glyphs: GlyphSet,
    color: bool,
) -> (Buffer, String) {
    let area = Rect::new(0, 0, l.width, l.height);
    let mut buf = Buffer::empty(area);
    let style = DefaultStyle { color };
    GraphView {
        graph: g,
        layout: l,
        style: &style,
        selection: sel,
        glyphs,
        hotkeys: true,
    }
    .render(area, &mut buf);
    let mut text = String::new();
    for y in 0..l.height {
        let row: String = (0..l.width)
            .map(|x| buf[(x, y)].symbol().to_string())
            .collect();
        text.push_str(row.trim_end());
        text.push('\n');
    }
    (buf, text)
}

#[test]
fn ports_merge_with_node_borders_and_arrows_point_into_targets() {
    let g = fleet();
    let l = compute(&g, 120, 40, GlyphSet::Unicode, None);
    let (buf, _) = render_text(&g, &l, Selection::default(), GlyphSet::Unicode, true);
    let r = l
        .route_for(edge_idx(&g, "tranquility", "innovation"))
        .unwrap();
    let port = buf[(r.src_port.0, r.src_port.1)].symbol().to_string();
    assert!("┝┥┯┷├┤┬┴".contains(&port), "port glyph {port}");
    // Two-way edge: arrowheads at both ends.
    let first = buf[(r.cells[0].0, r.cells[0].1)].symbol().to_string();
    let last = buf[(r.cells.last().unwrap().0, r.cells.last().unwrap().1)]
        .symbol()
        .to_string();
    assert!(
        "▶◀▲▼".contains(&first) && "▶◀▲▼".contains(&last),
        "{first} {last}"
    );
    // Password edge is dashed, one-way has a single head and ⊘ at the source end.
    let pwd = l.route_for(edge_idx(&g, "tranquility", "nas")).unwrap();
    assert!(pwd
        .cells
        .iter()
        .any(|&(x, y, _)| "┄┆".contains(buf[(x, y)].symbol())));
    let one = l.route_for(edge_idx(&g, "lab-arm", "jetson-01")).unwrap();
    assert_eq!(buf[(one.cells[0].0, one.cells[0].1)].symbol(), "⊘");
    assert!(
        "▶◀▲▼".contains(buf[(one.cells.last().unwrap().0, one.cells.last().unwrap().1)].symbol())
    );
}

#[test]
fn ascii_mode_uses_only_ascii() {
    let g = fleet();
    for (w, h) in SIZES {
        let l = compute(&g, w, h, GlyphSet::Ascii, None);
        let sel = Selection {
            node: Some(idx(&g, "lab-arm")),
            edge: Some(edge_idx(&g, "tranquility", "nas")),
        };
        let (_, text) = render_text(&g, &l, sel, GlyphSet::Ascii, false);
        assert!(text.is_ascii(), "{w}x{h}:\n{text}");
    }
}

#[test]
fn style_changes_do_not_invalidate_the_cache() {
    let mut cache = GraphCache::default();
    let mut g = fleet();
    cache.assign_hotkeys(&mut g);
    cache.layout(&g, 120, 40, GlyphSet::Unicode);
    g.nodes[1].state = NodeState::Down;
    g.nodes[1].metrics.rtt_ms = Some(99);
    g.edges[0].policy = Policy::None;
    cache.layout(&g, 120, 40, GlyphSet::Unicode);
    assert_eq!(cache.computes, 1);
    cache.layout(&g, 121, 40, GlyphSet::Unicode);
    cache.layout(&g, 121, 40, GlyphSet::Ascii);
    assert_eq!(cache.computes, 3);
    g.nodes[1].mesh = None;
    cache.layout(&g, 121, 40, GlyphSet::Ascii);
    assert_eq!(cache.computes, 4);
}

// ---------------------------------------------------------------------------
// Hit testing and navigation
// ---------------------------------------------------------------------------

#[test]
fn hit_map_resolves_nodes_edges_and_halo() {
    let g = fleet();
    let l = compute(&g, 120, 40, GlyphSet::Unicode, None);
    let p = l.placed(idx(&g, "lab-arm")).unwrap();
    for y in p.rect.y..p.rect.bottom() {
        for x in p.rect.x..p.rect.right() {
            assert_eq!(l.hits.at(x, y), Some(HitId::Node(p.node)));
        }
    }
    let k = edge_idx(&g, "tranquility", "nas");
    let r = l.route_for(k).unwrap();
    let mid = r.cells[r.cells.len() / 2];
    assert_eq!(l.hits.at(mid.0, mid.1), Some(HitId::Edge(k)));
    // Halo: some free neighbour of the path resolves to the edge.
    let halo = r.cells.iter().any(|&(x, y, _)| {
        [(0i32, -1i32), (0, 1), (-1, 0), (1, 0)]
            .iter()
            .any(|(dx, dy)| {
                let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                nx >= 0
                    && ny >= 0
                    && !r.cells.iter().any(|c| c.0 as i32 == nx && c.1 as i32 == ny)
                    && l.hits.at(nx as u16, ny as u16) == Some(HitId::Edge(k))
            })
    });
    assert!(halo);
    assert_eq!(hit_at(&l, Selection::default(), 0, 0), l.hits.at(0, 0));
    assert_eq!(l.hits.at(500, 500), None);
}

#[test]
fn nearest_node_in_direction() {
    let g = fleet();
    let l = compute(&g, 200, 60, GlyphSet::Unicode, None);
    let v = idx(&g, "tranquility");
    let right = nearest(&l, v, Dir::Right).unwrap();
    assert_eq!(l.placed(right).unwrap().rank, 1);
    assert_eq!(nearest(&l, v, Dir::Left), None);
    let lab = idx(&g, "lab-arm");
    let jet = idx(&g, "jetson-01");
    assert_eq!(nearest(&l, lab, Dir::Right), Some(jet));
    assert_eq!(
        l.placed(nearest(&l, jet, Dir::Left).unwrap()).unwrap().rank,
        1
    );
    // Down from the top of rank 1 walks the column.
    let mut col: Vec<&layout::Placed> = l.nodes.iter().filter(|p| p.rank == 1).collect();
    col.sort_by_key(|p| p.rect.y);
    assert_eq!(nearest(&l, col[0].node, Dir::Down), Some(col[1].node));
    assert_eq!(nearest(&l, col[1].node, Dir::Up), Some(col[0].node));
    // Outline navigation.
    let o = compute(&g, 40, 20, GlyphSet::Unicode, None);
    let first = o.outline[1].node;
    assert_eq!(nearest(&o, v, Dir::Down), Some(first));
    assert_eq!(nearest(&o, first, Dir::Left), Some(v));
    assert_eq!(nearest(&o, first, Dir::Up), Some(v));
}

#[test]
fn outline_tree_covers_every_node_once() {
    let g = fleet();
    let l = compute(&g, 40, 20, GlyphSet::Unicode, None);
    let ids: BTreeSet<usize> = l.outline.iter().map(|r| r.node).collect();
    assert_eq!(ids.len(), g.nodes.len());
    assert_eq!(l.outline.len(), g.nodes.len());
    assert_eq!(l.outline[0].node, idx(&g, "tranquility"));
    let jet = l
        .outline
        .iter()
        .find(|r| r.node == idx(&g, "jetson-01"))
        .unwrap();
    assert_eq!(jet.depth, 2);
    assert_eq!(hit_at(&l, Selection::default(), 3, 0), Some(HitId::Node(0)));
}

// ---------------------------------------------------------------------------
// Performance
// ---------------------------------------------------------------------------

/// Viewer + 29 devices over five hop ranks, two meshes, mixed policies.
fn thirty_nodes() -> Graph {
    let mut src = Sources {
        devices: vec![device("viewer", None, "local", 1)],
        now: NOW,
        ..Default::default()
    };
    let mut hops: Vec<Vec<String>> = Vec::new();
    let mut members = [Vec::new(), Vec::new()];
    for k in 0..29usize {
        let id = format!("dev-{k:02}");
        let status = if k % 7 == 3 {
            "unreachable"
        } else {
            "reachable"
        };
        src.devices.push(device(&id, Some(&id), status, 1));
        let route = if k >= 6 {
            let mut r = hops[k - 6].clone();
            r.push(format!("dev-{:02}", k - 6));
            r
        } else {
            Vec::new()
        };
        if !route.is_empty() {
            src.routes.insert(id.clone(), route.clone());
        }
        hops.push(route);
        if k % 3 != 2 {
            members[(k / 6) % 2].push(id.clone());
        }
        if k % 4 == 1 {
            src.edge_policies.push(EdgePolicyInput {
                from: "viewer".into(),
                to: id.clone(),
                policy: Policy::Password,
            });
        }
    }
    src.mesh = vec![
        MeshMembership {
            backend: "cloudflare".into(),
            members: members[0].clone(),
        },
        MeshMembership {
            backend: "ssh-overlay".into(),
            members: members[1].clone(),
        },
    ];
    let g = build(&src);
    assert_eq!(g.nodes.len(), 30);
    g
}

#[test]
fn layout_and_routing_30_nodes_within_budget() {
    let g = thirty_nodes();
    let key = CacheKey {
        topology: g.topology_hash(),
        width: 200,
        height: 60,
        class: SizeClass::L,
        glyphs: GlyphSet::Unicode,
    };
    let mut best = std::time::Duration::MAX;
    let mut layout = None;
    for _ in 0..7 {
        let t = std::time::Instant::now();
        let l = graph_with_overflow(&g, key, None).expect("graph mode");
        best = best.min(t.elapsed());
        layout = Some(l);
    }
    let l = layout.unwrap();
    assert_geometry(&g, &l);
    assert_eq!(l.nodes.len(), 30, "all 30 nodes placed, none folded");
    assert_eq!(l.routes.len(), g.edges.len(), "all edges routed");
    let budget = if cfg!(debug_assertions) { 10 } else { 2 };
    eprintln!(
        "graph-core perf: {} nodes, {} routes, best {:?} (budget {budget} ms)",
        l.nodes.len(),
        l.routes.len(),
        best
    );
    assert!(
        best <= std::time::Duration::from_millis(budget),
        "layout+route took {best:?}"
    );
}

// ---------------------------------------------------------------------------
// Snapshots and evidence
// ---------------------------------------------------------------------------

fn color_hex(c: Color) -> Option<String> {
    match c {
        Color::Rgb(r, g, b) => Some(format!("{r:02x}{g:02x}{b:02x}")),
        Color::Reset => None,
        other => Some(format!("{other:?}").to_lowercase()),
    }
}

fn style_dump(buf: &Buffer, w: u16, h: u16) -> serde_json::Value {
    let mut rows = Vec::new();
    for y in 0..h {
        let mut row = Vec::new();
        for x in 0..w {
            let c = &buf[(x, y)];
            let m = c.modifier;
            use ratatui::style::Modifier;
            row.push(json!([
                c.symbol(),
                color_hex(c.fg),
                color_hex(c.bg),
                m.contains(Modifier::BOLD),
                m.contains(Modifier::DIM),
                m.contains(Modifier::REVERSED)
            ]));
        }
        rows.push(serde_json::Value::Array(row));
    }
    json!({"cols": w, "rows": h, "cells": rows})
}

fn snapshot_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/ui/graph/snapshots")
}

/// Compares against the committed text snapshot (`CX_UPDATE_SNAPSHOTS=1`
/// rewrites it) and, with `CX_GRAPH_EVIDENCE=<dir>`, writes text + style dumps
/// for `tests/graph_render_png.py`.
fn check_snapshot(name: &str, buf: &Buffer, text: &str, w: u16, h: u16) {
    let path = snapshot_dir().join(format!("{name}.txt"));
    if std::env::var_os("CX_UPDATE_SNAPSHOTS").is_some() {
        std::fs::create_dir_all(snapshot_dir()).unwrap();
        std::fs::write(&path, text).unwrap();
    }
    if let Some(dir) = std::env::var_os("CX_GRAPH_EVIDENCE") {
        let dir = std::path::PathBuf::from(dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(format!("{name}.txt")), text).unwrap();
        std::fs::write(
            dir.join(format!("{name}.json")),
            serde_json::to_vec(&style_dump(buf, w, h)).unwrap(),
        )
        .unwrap();
    }
    let want = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("missing snapshot {path:?}; run with CX_UPDATE_SNAPSHOTS=1"));
    assert_eq!(text, want, "snapshot {name} differs:\n{text}");
}

#[test]
fn fleet_snapshots() {
    let g = fleet();
    for (w, h) in SIZES {
        let l = compute(&g, w, h, GlyphSet::Unicode, None);
        let sel = Selection {
            node: Some(idx(&g, "tranquility")),
            edge: None,
        };
        let (buf, text) = render_text(&g, &l, sel, GlyphSet::Unicode, true);
        check_snapshot(&format!("fleet-{w}x{h}"), &buf, &text, w, h);
    }
    // Variants: ASCII + NO_COLOR at 80x24, a selected edge, an expanded LAN cluster.
    let l = compute(&g, 80, 24, GlyphSet::Ascii, None);
    let sel = Selection {
        node: Some(idx(&g, "tranquility")),
        edge: None,
    };
    let (buf, text) = render_text(&g, &l, sel, GlyphSet::Ascii, false);
    check_snapshot("fleet-ascii-80x24", &buf, &text, 80, 24);
    let l = compute(&g, 120, 40, GlyphSet::Unicode, None);
    let sel = Selection {
        node: Some(idx(&g, "lab-arm")),
        edge: Some(edge_idx(&g, "lab-arm", "jetson-01")),
    };
    let (buf, text) = render_text(&g, &l, sel, GlyphSet::Unicode, true);
    check_snapshot("fleet-edge-selected-120x40", &buf, &text, 120, 40);
    let mut src = fleet_sources();
    src.expanded.insert("lan:tranquility:enp5s0".into());
    let mut g2 = build(&src);
    assign_hotkeys(&mut g2, &BTreeMap::new());
    let l = compute(&g2, 200, 60, GlyphSet::Unicode, None);
    let sel = Selection {
        node: Some(idx(&g2, "lan:tranquility:enp5s0")),
        edge: None,
    };
    let (buf, text) = render_text(&g2, &l, sel, GlyphSet::Unicode, true);
    check_snapshot("fleet-lan-expanded-200x60", &buf, &text, 200, 60);
    // Dense 30-node graph (the perf fixture) in forced graph mode at L.
    let g3 = thirty_nodes();
    let key = CacheKey {
        topology: g3.topology_hash(),
        width: 200,
        height: 60,
        class: SizeClass::L,
        glyphs: GlyphSet::Unicode,
    };
    let l = graph_with_overflow(&g3, key, None).unwrap();
    let (buf, text) = render_text(&g3, &l, Selection::default(), GlyphSet::Unicode, true);
    check_snapshot("dense-30-200x60", &buf, &text, 200, 60);
}
