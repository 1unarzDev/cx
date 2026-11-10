use super::layout::Orientation;
use super::route::{DOWN, LEFT, RIGHT, UP};
use super::*;
use crate::model::Device;
use ratatui::{buffer::Buffer, layout::Rect, widgets::Widget};
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
        device("field-rover", Some("field-rover"), "reachable", 300),
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
        {"address":"192.0.2.10","interface":"enp5s0","observed_at":NOW,"ssh":{"state":"open"}},
        {"address":"192.0.2.11","interface":"enp5s0","observed_at":NOW,"ssh":{"state":"closed"}},
        {"address":"192.0.2.12","interface":"enp5s0","observed_at":NOW,"ssh":{"state":"unknown"}},
        {"address":"192.0.2.13","interface":"enp5s0","observed_at":NOW,"hostname":"printer"},
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
        helper: BTreeMap::from([("field-rover".to_string(), (NOW - 300, false))]),
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
    assert_meshes_connected(g, l);
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

/// Visual-first mesh contract: every mesh is one enclosure, or fragments
/// joined by bridges into one connected region; bridges stay outside nodes
/// and enclosures and start / end on the borders of the fragments they join.
fn assert_meshes_connected(g: &Graph, l: &GraphLayout) {
    let encl = &l.enclosures;
    for p in &l.nodes {
        if let Some(m) = g.nodes[p.node].mesh.as_deref() {
            assert!(
                encl.iter()
                    .any(|e| e.mesh == m && e.rect.union(p.rect) == e.rect),
                "{} outside every {m} enclosure",
                g.nodes[p.node].id
            );
        }
    }
    let on_border = |r: Rect, x: u16, y: u16| {
        x >= r.x
            && x < r.right()
            && y >= r.y
            && y < r.bottom()
            && (x == r.x || x == r.right() - 1 || y == r.y || y == r.bottom() - 1)
    };
    let inside = |r: Rect, x: u16, y: u16| x >= r.x && x < r.right() && y >= r.y && y < r.bottom();
    for b in &l.bridges {
        assert_eq!(encl[b.from].mesh, b.mesh);
        assert_eq!(encl[b.to].mesh, b.mesh);
        assert!(on_border(encl[b.from].rect, b.ends[0].0, b.ends[0].1));
        assert!(on_border(encl[b.to].rect, b.ends[1].0, b.ends[1].1));
        let mut prev = (b.ends[0].0, b.ends[0].1);
        for &(x, y, _) in b
            .cells
            .iter()
            .chain(std::iter::once(&(b.ends[1].0, b.ends[1].1, 0)))
        {
            let d = (x as i32 - prev.0 as i32).abs() + (y as i32 - prev.1 as i32).abs();
            assert_eq!(d, 1, "bridge {} not contiguous at {x},{y}", b.mesh);
            prev = (x, y);
        }
        for &(x, y, _) in &b.cells {
            for p in &l.nodes {
                assert!(!inside(p.rect, x, y), "bridge through node at {x},{y}");
            }
            for e in encl {
                assert!(!inside(e.rect, x, y), "bridge through enclosure at {x},{y}");
            }
        }
    }
    let mut meshes: Vec<&str> = encl.iter().map(|e| e.mesh.as_str()).collect();
    meshes.sort_unstable();
    meshes.dedup();
    for m in meshes {
        let frags: Vec<usize> = (0..encl.len()).filter(|&k| encl[k].mesh == m).collect();
        let mut reached = BTreeSet::from([frags[0]]);
        loop {
            let before = reached.len();
            for b in l.bridges.iter().filter(|b| b.mesh == m) {
                if reached.contains(&b.from) || reached.contains(&b.to) {
                    reached.insert(b.from);
                    reached.insert(b.to);
                }
            }
            if reached.len() == before {
                break;
            }
        }
        assert_eq!(
            reached.len(),
            frags.len(),
            "{m} drawn as unlinked fragments at {}x{}",
            l.width,
            l.height
        );
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
    // Viewer-scoped posture and rtt stay on the viewer's own route edge.
    assert_eq!(
        (peer.policy, peer.direction, peer.evidence.rtt_ms.clone()),
        (Policy::Unknown, Direction::Forward, vec![])
    );
    let own = &g.edges[edge_idx(&g, "tranquility", "lab-arm")];
    assert_eq!(
        (own.policy, own.direction, own.evidence.rtt_ms.clone()),
        (Policy::Key, Direction::Both, vec![12])
    );
    assert_eq!(
        g.edges[edge_idx(&g, "tranquility", "field-rover")].policy,
        Policy::None
    );
    // Neighbour state comes from the bounded SSH port evidence only.
    let open = &g.nodes[idx(&g, "nb:tranquility:enp5s0:192.0.2.10")];
    assert_eq!(open.state, NodeState::Up);
    assert_eq!(open.detail.as_deref(), Some("SSH port open"));
    // A hostname is an unauthenticated hint: the address stays the label.
    let printer = &g.nodes[idx(&g, "nb:tranquility:enp5s0:192.0.2.13")];
    assert_eq!(printer.label, "192.0.2.13");
    assert_eq!(printer.detail.as_deref(), Some("printer? · SSH unknown"));
    let cluster = &g.nodes[idx(&g, "lan:tranquility:enp5s0")];
    assert_eq!(cluster.detail.as_deref(), Some("enp5s0 1 open"));
}

#[test]
fn stale_neighbor_observation_is_unknown() {
    let mut src = fleet_sources();
    src.observations = vec![(
        "tranquility".into(),
        json!({"observed_at": NOW - 1200, "candidates": [{"address":"192.0.2.10",
            "interface":"enp5s0","observed_at":NOW - 1200,"ssh":{"state":"open"}}]}),
    )];
    src.expanded.insert("lan:tranquility:enp5s0".into());
    let g = build(&src);
    let nb = &g.nodes[idx(&g, "nb:tranquility:enp5s0:192.0.2.10")];
    assert_eq!(nb.state, NodeState::Unknown);
    assert_eq!(nb.detail.as_deref(), Some("SSH unknown"));
    let cluster = &g.nodes[idx(&g, "lan:tranquility:enp5s0")];
    assert_eq!(cluster.state, NodeState::Unknown);
    assert_eq!(cluster.detail.as_deref(), Some("enp5s0"));
}

#[test]
fn device_state_uses_typed_evidence_and_exact_store_values() {
    use super::model::device_state;
    assert_eq!(device_state("reachable", NOW, None, NOW), NodeState::Up);
    assert_eq!(device_state("local", NOW, None, NOW), NodeState::Up);
    assert_eq!(device_state("unknown", NOW, None, NOW), NodeState::Unknown);
    assert_eq!(device_state("", 0, None, NOW), NodeState::Unknown);
    // Free-form strings are not evidence, whatever words they contain.
    for s in [
        "broken",
        "setup",
        "update pending",
        "not authenticated",
        "token expired",
    ] {
        assert_eq!(device_state(s, NOW, None, NOW), NodeState::Unknown, "{s}");
    }
    // The add-time "reachable" goes stale on the helper-reached window.
    assert_eq!(
        device_state("reachable", NOW - 500, None, NOW),
        NodeState::Stale
    );
    // A helper check wins over the stored string.
    assert_eq!(
        device_state("reachable", NOW, Some((NOW - 10, false)), NOW),
        NodeState::Down
    );
    assert_eq!(
        device_state("unknown", 0, Some((NOW - 10, true)), NOW),
        NodeState::Up
    );
    assert_eq!(
        device_state("reachable", NOW, Some((NOW - 91, true)), NOW),
        NodeState::Stale
    );
    let g = fleet();
    assert_eq!(g.nodes[idx(&g, "field-rover")].state, NodeState::Down);
}

#[test]
fn labels_are_bounded_and_control_free() {
    let mut src = fleet_sources();
    src.devices[1].name = format!("evil\x1b[31m{}", "x".repeat(80));
    if let Some((_, obs)) = src.observations.first_mut() {
        obs["candidates"][3]["hostname"] = json!("cam\u{202E}gnp.exe");
    }
    src.expanded.insert("lan:tranquility:enp5s0".into());
    let g = build(&src);
    let n = &g.nodes[idx(&g, "innovation")];
    assert!(n.label.chars().count() <= 32);
    assert!(!n.label.chars().any(char::is_control));
    let nb = &g.nodes[idx(&g, "nb:tranquility:enp5s0:192.0.2.13")];
    let detail = nb.detail.as_deref().unwrap();
    assert!(!detail.contains('\u{202E}'), "{detail:?}");
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
fn split_meshes_render_as_one_region_or_linked_fragments() {
    // Room to spare: the LAN neighbours move out of ssh-overlay's band and
    // the mesh is one enclosure spanning ranks 1 and 2.
    let mut src = fleet_sources();
    src.expanded.insert("lan:tranquility:enp5s0".into());
    let mut g = build(&src);
    assign_hotkeys(&mut g, &BTreeMap::new());
    let l = compute(&g, 200, 60, GlyphSet::Unicode, None);
    let ssh: Vec<_> = l
        .enclosures
        .iter()
        .filter(|e| e.mesh == "ssh-overlay")
        .collect();
    assert_eq!(ssh.len(), 1, "{:?}", l.enclosures);
    assert!(l.bridges.is_empty());
    assert_geometry(&g, &l);
    // Shorter: two fragments, joined by exactly one bridge drawn in mesh style.
    let l = compute(&g, 120, 40, GlyphSet::Unicode, None);
    let frags = l
        .enclosures
        .iter()
        .filter(|e| e.mesh == "ssh-overlay")
        .count();
    assert_eq!(frags, 2);
    assert_eq!(l.bridges.len(), 1);
    assert_geometry(&g, &l);
    let (buf, _) = render_text(&g, &l, Selection::default(), GlyphSet::Unicode, true);
    let mesh = DefaultStyle { color: true }.style(Role::Mesh).fg;
    for &(x, y, _) in &l.bridges[0].cells {
        assert_eq!(buf[(x, y)].fg, mesh.unwrap(), "bridge cell {x},{y}");
    }
    // Interleaved meshes (dense fixture): every fragment is linked.
    let g = dense(MAX_DENSE);
    let l = compute(&g, 200, 60, GlyphSet::Unicode, None);
    assert_eq!(l.mode, Mode::Graph);
    for mesh in ["cloudflare", "ssh-overlay"] {
        let frags = l.enclosures.iter().filter(|e| e.mesh == mesh).count();
        let links = l.bridges.iter().filter(|b| b.mesh == mesh).count();
        assert!(
            frags >= 1 && links == frags - 1,
            "{mesh}: {frags} boxes, {links} bridges"
        );
    }
    assert_geometry(&g, &l);
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

/// Hop distance from the viewer over all edges (the layout's rank before folding).
fn hops(g: &Graph) -> Vec<usize> {
    let root = g.viewer().unwrap_or(0);
    let mut d = vec![usize::MAX; g.nodes.len()];
    if g.nodes.is_empty() {
        return d;
    }
    d[root] = 0;
    let mut q = std::collections::VecDeque::from([root]);
    while let Some(a) = q.pop_front() {
        for e in &g.edges {
            let (Some(x), Some(y)) = (g.node_index(&e.from), g.node_index(&e.to)) else {
                continue;
            };
            for (p, c) in [(x, y), (y, x)] {
                if p == a && d[c] == usize::MAX {
                    d[c] = d[a] + 1;
                    q.push_back(c);
                }
            }
        }
    }
    d
}

/// Spec 10.2.5: only ranks >= 3 fold into a pill; a node at hop 1 or 2 is
/// either drawn (with its hotkey badge and hit target) or the outline is used.
pub(super) fn assert_no_shallow_folds(g: &Graph, l: &GraphLayout) {
    let d = hops(g);
    for m in &l.more {
        assert!(m.rank >= 3, "pill at rank {}", m.rank);
        for &i in &m.hidden {
            assert!(
                d[i] >= 3 || g.nodes[i].kind == NodeKind::Neighbor,
                "{} (hop {}) folded at {}x{}",
                g.nodes[i].id,
                d[i],
                l.width,
                l.height
            );
        }
    }
    if l.mode == Mode::Graph {
        for (i, n) in g.nodes.iter().enumerate() {
            if d[i] <= 2 && n.kind != NodeKind::Neighbor {
                let p = l.placed(i).unwrap_or_else(|| panic!("{} not drawn", n.id));
                if let Some(k) = n.hotkey {
                    let _ = k;
                    assert_eq!(l.hits.at(p.rect.x + 1, p.rect.y), Some(HitId::Node(i)));
                }
            }
        }
    }
}

#[test]
fn overflow_never_folds_rank_one_or_two() {
    // A crowded rank 1 does not fit 80x24: outline, never a `+N more` pill.
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
    assert_eq!(l.mode, Mode::Outline);
    assert!(l.more.is_empty());
    // The fixture fleet at 120x24 (rank 1 = 5 cards incl. the password edge to
    // nas): every node stays visible with its hotkey and policy.
    let fleet = fleet();
    for (w, h) in [(120, 24), (80, 24), (200, 24), (100, 20), (160, 22)] {
        let l = compute(&fleet, w, h, GlyphSet::Unicode, None);
        assert_no_shallow_folds(&fleet, &l);
        if l.mode == Mode::Graph {
            assert!(l.more.is_empty(), "{w}x{h}");
            assert!(l
                .route_for(edge_idx(&fleet, "tranquility", "nas"))
                .is_some());
        }
    }
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
    // Without jetson-01 the neighbours need a third rank: at 64x24 two rank
    // columns fit but three do not, so the cluster collapses (step 1 of the
    // overflow ladder) instead of the whole view switching to the outline.
    src.devices.retain(|d| d.id != "jetson-01");
    src.routes.clear();
    src.edge_policies.retain(|p| p.to != "jetson-01");
    for m in src.mesh.iter_mut() {
        m.members.retain(|id| id != "jetson-01");
    }
    let mut g = build(&src);
    assign_hotkeys(&mut g, &BTreeMap::new());
    let small = compute(&g, 64, 24, GlyphSet::Unicode, None);
    assert_eq!(small.mode, Mode::Graph);
    assert!(small
        .nodes
        .iter()
        .all(|p| g.nodes[p.node].kind != NodeKind::Neighbor));
    assert!(small.placed(idx(&g, "lan:tranquility:enp5s0")).is_some());
    assert_geometry(&g, &small);
    assert_no_shallow_folds(&g, &small);
}

#[test]
fn deep_ranks_fold_into_a_pill_at_rank_three() {
    // v -> hop0 -> hop1 -> hop2 -> hop3a -> hop4: six ranks do not fit 95
    // columns or 24 rows; four do once ranks >= 3 fold into one pill.
    let mut src = Sources {
        devices: vec![device("v", None, "local", 1)],
        now: NOW,
        ..Default::default()
    };
    let chain = ["hop0", "hop1", "hop2", "hop3a"];
    // Ranks: v 0, hop0 1, hop1 2, hop2 3, hop3a 4, hop4 5.
    for (k, id) in chain.iter().enumerate() {
        src.devices.push(device(id, Some(id), "reachable", 1));
        if k > 0 {
            src.routes.insert(
                id.to_string(),
                chain[..k].iter().map(|s| s.to_string()).collect(),
            );
        }
    }
    src.devices
        .push(device("hop4", Some("hop4"), "reachable", 1));
    src.routes.insert(
        "hop4".into(),
        ["hop0", "hop1", "hop2", "hop3a"].map(String::from).to_vec(),
    );
    let mut g = build(&src);
    assign_hotkeys(&mut g, &BTreeMap::new());
    let l = compute(&g, 95, 24, GlyphSet::Unicode, None);
    assert_eq!(l.mode, Mode::Graph);
    assert_eq!(l.more.len(), 1);
    assert_eq!(l.more[0].rank, 3);
    let hidden: BTreeSet<&str> = l.more[0]
        .hidden
        .iter()
        .map(|&i| g.nodes[i].id.as_str())
        .collect();
    assert_eq!(hidden, BTreeSet::from(["hop2", "hop3a", "hop4"]));
    assert_eq!(l.nodes.len() + 3, g.nodes.len());
    assert_geometry(&g, &l);
    assert_no_shallow_folds(&g, &l);
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
    // Two-way edge: heads at both ends (spec 10.3), each pointing into its node.
    let first = buf[(r.cells[0].0, r.cells[0].1)].symbol().to_string();
    let last = buf[(r.cells.last().unwrap().0, r.cells.last().unwrap().1)]
        .symbol()
        .to_string();
    assert_eq!((first.as_str(), last.as_str()), ("◀", "▶"));
    // A forward-only edge has a single head at the target.
    let fwd = l
        .route_for(edge_idx(&g, "tranquility", "field-rover"))
        .unwrap();
    assert!(!"▶◀▲▼".contains(buf[(fwd.cells[0].0, fwd.cells[0].1)].symbol()));
    assert_eq!(
        buf[(fwd.cells.last().unwrap().0, fwd.cells.last().unwrap().1)].symbol(),
        "▶"
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
fn outline_drops_whole_tokens_and_keeps_policy_first() {
    let g = fleet();
    let l = compute(&g, 40, 20, GlyphSet::Unicode, None);
    assert_eq!(l.mode, Mode::Outline);
    let (_, text) = render_text(&g, &l, Selection::default(), GlyphSet::Unicode, true);
    let row = |id: &str| -> String {
        text.lines()
            .find(|r| r.contains(&format!(" {id}")))
            .unwrap_or_else(|| panic!("no row {id}:\n{text}"))
            .to_string()
    };
    // The one-way policy outranks the mesh badge on a narrow row.
    assert!(row("jetson-01").contains("one-way →"), "{text}");
    assert!(row("nas").contains("pwd →"), "{text}");
    // Every token is whole: no cut policy words.
    for r in text.lines() {
        for bad in ["↳ o ", "↳ o\n", " ke ", " pw "] {
            assert!(!format!("{r}\n").contains(bad), "cut token in {r:?}");
        }
        let t = r.trim_end();
        for tail in ["↳ o", "↳", " k", " ke", " p", " pw", " one-", "◆ ssh-"] {
            assert!(!t.ends_with(tail), "cut token in {r:?}");
        }
    }
}

#[test]
fn legend_lists_only_what_is_drawn() {
    let g = fleet();
    let l = compute(&g, 120, 40, GlyphSet::Unicode, None);
    let items = render::legend_items(&g, &l, Selection::default(), GlyphSet::Unicode);
    assert!(!items.contains(&"none"), "{items:?}");
    assert!(items.contains(&"pwd") && items.contains(&"key") && items.contains(&"mesh"));
    // The dense fixture has no LAN and no labelled jump edge.
    let d = dense(MAX_DENSE);
    let l = compute(&d, 200, 60, GlyphSet::Unicode, None);
    assert_eq!(l.mode, Mode::Graph);
    let items = render::legend_items(&d, &l, Selection::default(), GlyphSet::Unicode);
    assert_eq!(items, vec!["key", "pwd", "access", "mesh"]);
    // Plain key jump edges carry no label unless selected.
    let jump = d
        .edges
        .iter()
        .position(|e| e.kind == EdgeKind::Jump && e.policy == Policy::Key)
        .unwrap();
    assert_eq!(
        render::edge_label(&d.edges[jump], false, GlyphSet::Unicode),
        None
    );
    assert!(render::edge_label(&d.edges[jump], true, GlyphSet::Unicode)
        .unwrap()
        .starts_with('↳'));
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
fn many_expanded_neighbors_collapse_instead_of_leaving_graph_mode() {
    let mut src = fleet_sources();
    let cands: Vec<_> = (0..20)
        .map(|k| {
            json!({"address": format!("192.0.2.{}", 10 + k), "interface": "enp5s0",
            "observed_at": NOW, "ssh": {"state": "open"}})
        })
        .collect();
    src.observations = vec![("tranquility".into(), json!({"candidates": cands}))];
    src.expanded.insert("lan:tranquility:enp5s0".into());
    let mut g = build(&src);
    assign_hotkeys(&mut g, &BTreeMap::new());
    assert!(g.nodes.len() > layout::MAX_GRAPH_NODES);
    for (w, h) in [(80, 24), (120, 40), (200, 60)] {
        let l = compute(&g, w, h, GlyphSet::Unicode, None);
        assert_eq!(l.mode, Mode::Graph, "{w}x{h}");
        assert!(l.placed(idx(&g, "lan:tranquility:enp5s0")).is_some());
        assert_geometry(&g, &l);
    }
}

#[test]
fn outline_hits_follow_scroll() {
    let g = fleet();
    let l = compute(&g, 40, 3, GlyphSet::Unicode, None);
    assert_eq!(l.mode, Mode::Outline);
    assert!(l.outline.len() > 3);
    let last = l.outline.last().unwrap().node;
    let sel = Selection {
        node: Some(last),
        edge: None,
    };
    // The per-cell map has no outline rows; only `hit_at` knows the scroll.
    assert_eq!(l.hits.at(0, 0), None);
    assert_eq!(hit_at(&l, sel, 0, 2), Some(HitId::Node(last)));
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

/// Viewer + `n - 1` devices over up to five hop ranks, two meshes, mixed policies.
fn dense(n: usize) -> Graph {
    let mut src = Sources {
        devices: vec![device("viewer", None, "local", 1)],
        now: NOW,
        ..Default::default()
    };
    let mut hops: Vec<Vec<String>> = Vec::new();
    let mut members = [Vec::new(), Vec::new()];
    for k in 0..n - 1 {
        let id = format!("dev-{k:02}");
        if k % 7 == 3 {
            src.helper.insert(id.clone(), (NOW - 1, false));
        }
        src.devices.push(device(&id, Some(&id), "reachable", 1));
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
    assert_eq!(g.nodes.len(), n);
    g
}

/// The largest graph `compute` still draws as a graph (`MAX_GRAPH_NODES`).
const MAX_DENSE: usize = layout::MAX_GRAPH_NODES;

const PERF_SIZES: [(u16, u16); 3] = [(200, 60), (120, 40), (80, 24)];

#[test]
fn dense_graphs_take_the_shipped_compute_paths() {
    let g = dense(MAX_DENSE);
    for (w, h) in PERF_SIZES {
        let l = compute(&g, w, h, GlyphSet::Unicode, None);
        if l.mode == Mode::Graph {
            assert_geometry(&g, &l);
            assert_no_shallow_folds(&g, &l);
        }
    }
    let l = compute(&g, 200, 60, GlyphSet::Unicode, None);
    assert_eq!(l.mode, Mode::Graph);
    assert_eq!(l.nodes.len(), MAX_DENSE, "all placed, none folded");
    assert_eq!(l.routes.len(), g.edges.len(), "all edges routed");
    let thirty = dense(30);
    for (w, h) in PERF_SIZES {
        assert_eq!(
            compute(&thirty, w, h, GlyphSet::Unicode, None).mode,
            Mode::Outline
        );
    }
}

/// Spec 10.4 target: layout + routing <= 2 ms in debug builds. Wall-clock, so
/// it runs alone: `cargo test --locked graph_layout_perf -- --ignored --test-threads=1`.
#[test]
#[ignore]
fn graph_layout_perf() {
    let mut worst = std::time::Duration::ZERO;
    for (n, (w, h)) in [MAX_DENSE, 30]
        .into_iter()
        .flat_map(|n| PERF_SIZES.map(|s| (n, s)))
    {
        let g = dense(n);
        let mut best = std::time::Duration::MAX;
        for _ in 0..15 {
            let t = std::time::Instant::now();
            let l = compute(&g, w, h, GlyphSet::Unicode, None);
            best = best.min(t.elapsed());
            std::hint::black_box(l);
        }
        eprintln!("graph perf: {n} nodes at {w}x{h}: best {best:?}");
        worst = worst.max(best);
    }
    assert!(
        worst <= std::time::Duration::from_millis(2),
        "layout+route took {worst:?}"
    );
}

/// Regression for the centring-shift `clamp(min > max)` panic: all small
/// heights at widths around the size-class boundaries, plus the short-wide
/// areas that used to panic, in both glyph sets, for every fixture graph.
#[test]
fn every_area_lays_out_renders_and_hits_without_panic() {
    let mut lan = fleet_sources();
    lan.expanded.insert("lan:tranquility:enp5s0".into());
    let mut lan = build(&lan);
    assign_hotkeys(&mut lan, &BTreeMap::new());
    let mut empty = build(&Sources {
        now: NOW,
        ..Default::default()
    });
    assign_hotkeys(&mut empty, &BTreeMap::new());
    let mut one = build(&Sources {
        devices: vec![device("viewer", None, "local", 1)],
        now: NOW,
        ..Default::default()
    });
    assign_hotkeys(&mut one, &BTreeMap::new());
    let mut dense = dense(MAX_DENSE);
    assign_hotkeys(&mut dense, &BTreeMap::new());
    let graphs = [
        ("fleet", fleet()),
        ("lan-expanded", lan),
        ("empty", empty),
        ("one-node", one),
        ("dense", dense),
    ];
    let widths = [0, 1, 16, 59, 60, 61, 99, 100, 101, 159, 160, 161, 400];
    let areas = widths
        .iter()
        .flat_map(|&w| (0..=30u16).chain([40, 60, 100]).map(move |h| (w, h)))
        .chain([(120, 24), (200, 24), (80, 24)]);
    for (w, h) in areas {
        for (name, g) in &graphs {
            for glyphs in [GlyphSet::Unicode, GlyphSet::Ascii] {
                sweep_one(name, g, w, h, glyphs);
            }
        }
    }
}

fn sweep_one(name: &str, g: &Graph, w: u16, h: u16, glyphs: GlyphSet) {
    let l = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        compute(g, w, h, glyphs, None)
    }))
    .unwrap_or_else(|_| panic!("{name}: compute panicked at {w}x{h} {glyphs:?}"));
    assert_eq!((l.width, l.height), (w, h), "{name} {w}x{h}");
    if l.mode == Mode::Graph {
        assert_geometry(g, &l);
    }
    assert_no_shallow_folds(g, &l);
    let sel = Selection {
        node: g.nodes.first().map(|_| 0),
        edge: g.edges.first().map(|_| 0),
    };
    let area = Rect::new(0, 0, w, h);
    let mut buf = Buffer::empty(area);
    let style = DefaultStyle { color: true };
    GraphView {
        graph: g,
        layout: &l,
        style: &style,
        selection: sel,
        glyphs,
        hotkeys: true,
    }
    .render(area, &mut buf);
    for (x, y) in [
        (0, 0),
        (w / 2, h / 2),
        (w.saturating_sub(1), h.saturating_sub(1)),
        (w, h),
    ] {
        if let Some(HitId::Node(n)) = hit_at(&l, sel, x, y) {
            assert!(n < g.nodes.len(), "{name} {w}x{h}");
        }
    }
    for from in 0..g.nodes.len().min(3) {
        for d in [Dir::Up, Dir::Down, Dir::Left, Dir::Right] {
            if let Some(n) = nearest(&l, from, d) {
                assert!(n < g.nodes.len() && n != from, "{name} {w}x{h}");
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Snapshots
// ---------------------------------------------------------------------------

fn snapshot_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/ui/graph/snapshots")
}

/// Compares against the committed text snapshot (`CX_UPDATE_SNAPSHOTS=1`
/// rewrites it).
fn check_snapshot(name: &str, text: &str) {
    let path = snapshot_dir().join(format!("{name}.txt"));
    if std::env::var_os("CX_UPDATE_SNAPSHOTS").is_some() {
        std::fs::create_dir_all(snapshot_dir()).unwrap();
        std::fs::write(&path, text).unwrap();
    }
    let want = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("missing snapshot {path:?}; run with CX_UPDATE_SNAPSHOTS=1"));
    assert_eq!(text, want, "snapshot {name} differs:\n{text}");
}

/// One smoke snapshot: the visual system is still open (design studio), so
/// glyph-exact output is pinned once and the rest is checked geometrically.
#[test]
fn fleet_snapshot() {
    let g = fleet();
    let l = compute(&g, 120, 40, GlyphSet::Unicode, None);
    let sel = Selection {
        node: Some(idx(&g, "tranquility")),
        edge: None,
    };
    let (_, text) = render_text(&g, &l, sel, GlyphSet::Unicode, true);
    check_snapshot("fleet-120x40", &text);
    // Short-and-wide terminals (these used to panic in the centring shift).
    for (w, h) in [(120, 24), (200, 24)] {
        let l = compute(&g, w, h, GlyphSet::Unicode, None);
        assert_eq!(l.mode, Mode::Graph, "{w}x{h}");
        assert_geometry(&g, &l);
    }
}
