//! Network graph model (spec 10.1), the builder that assembles it from cx's
//! existing data sources, and stable single-letter node hotkeys.
//!
//! The builder takes plain data ([`Sources`]) so tests feed fixtures and the UI
//! feeds `store::devices()`, routes, device policies, the access graph and
//! network observations. Optional inputs from the edge-policy store
//! ([`EdgePolicyInput`]) and mesh backends ([`MeshMembership`]) override the
//! defaults derived from per-device policy. Nothing here performs I/O.
use crate::model::Device;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::hash::{Hash, Hasher};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum NodeKind {
    Viewer,
    Device,
    /// An SSH jump hop that is not an enrolled device.
    Jump,
    /// LAN neighbours seen by one observer on one interface (collapsible).
    LanCluster,
    /// One LAN neighbour (only present while its cluster is expanded).
    Neighbor,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum NodeState {
    Up,
    Down,
    Unknown,
    /// Was up, but the evidence is older than [`FRESH_SECS`].
    Stale,
}

/// Evidence older than this turns `Up` into `Stale` (the window
/// `App::peer_status` uses for "helper reached").
pub const FRESH_SECS: u64 = 90;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Metrics {
    pub rtt_ms: Option<u32>,
    pub load: Option<f32>,
    pub sessions: Option<u32>,
    /// Seconds since the last observation, when known.
    pub age_secs: Option<u64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    pub id: String,
    pub label: String,
    pub kind: NodeKind,
    pub state: NodeState,
    /// Mesh backend id (e.g. `cloudflare`, `ssh-overlay`), drawn as an enclosure.
    pub mesh: Option<String>,
    pub hotkey: Option<char>,
    pub metrics: Metrics,
    /// Short secondary text: interface of a LAN cluster, hostname hint and
    /// SSH port evidence for a neighbour.
    pub detail: Option<String>,
    /// Neighbour count of a LAN cluster.
    pub count: Option<u32>,
    /// LAN cluster: expanded into neighbour leaves.
    pub expanded: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum EdgeKind {
    SshDirect,
    Mesh,
    Jump,
    LanObserved,
}

/// Per-edge access policy (spec 10.5).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Policy {
    Key,
    Password,
    OneWay,
    None,
    Unknown,
}

/// Which way access is proven or allowed, relative to `from -> to`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Direction {
    Forward,
    Reverse,
    Both,
    Neither,
}
impl Direction {
    pub fn forward(self) -> bool {
        matches!(self, Direction::Forward | Direction::Both)
    }
    pub fn reverse(self) -> bool {
        matches!(self, Direction::Reverse | Direction::Both)
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Evidence {
    /// Last authenticated SSH check along this edge (unix seconds).
    pub authenticated_at: Option<u64>,
    pub rtt_ms: Vec<u32>,
    pub note: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Edge {
    pub from: String,
    pub to: String,
    pub kind: EdgeKind,
    pub policy: Policy,
    pub direction: Direction,
    pub evidence: Evidence,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Graph {
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
}

impl Graph {
    pub fn node_index(&self, id: &str) -> Option<usize> {
        self.nodes.iter().position(|n| n.id == id)
    }
    pub fn viewer(&self) -> Option<usize> {
        self.nodes.iter().position(|n| n.kind == NodeKind::Viewer)
    }
    /// Node index for a hotkey press on the hub.
    pub fn node_for_hotkey(&self, key: char) -> Option<usize> {
        self.nodes.iter().position(|n| n.hotkey == Some(key))
    }
    /// Hash of everything that moves geometry: node ids, kinds, mesh membership,
    /// cluster expansion and edge endpoints/kinds. State, metrics, labels,
    /// policy and hotkeys only restyle cached cells, so they are excluded.
    /// In-process cache key only; never persisted.
    pub fn topology_hash(&self) -> u64 {
        let mut h = std::hash::DefaultHasher::new();
        for n in &self.nodes {
            (&n.id, n.kind, &n.mesh, n.expanded).hash(&mut h);
        }
        self.nodes.len().hash(&mut h);
        for e in &self.edges {
            (&e.from, &e.to, e.kind).hash(&mut h);
        }
        h.finish()
    }
}

// ---------------------------------------------------------------------------
// Builder
// ---------------------------------------------------------------------------

/// One directed policy record (from the edge-policy store, `edges.json`).
#[derive(Clone, Debug, PartialEq)]
pub struct EdgePolicyInput {
    pub from: String,
    pub to: String,
    pub policy: Policy,
}

/// Mesh backend membership (from the mesh backends). Members are device ids or names.
#[derive(Clone, Debug, PartialEq)]
pub struct MeshMembership {
    pub backend: String,
    pub members: Vec<String>,
}

/// Plain inputs for [`build`]. Every field is optional data; nothing is read here.
#[derive(Clone, Debug, Default)]
pub struct Sources {
    /// `store::devices()`; the device with `target: None` is the viewer.
    pub devices: Vec<Device>,
    /// routes.json: target -> ordered jump hops.
    pub routes: BTreeMap<String, Vec<String>>,
    /// device-policies.json by device id.
    pub policies: BTreeMap<String, crate::store::DevicePolicy>,
    /// Directed access graph (`access.rs`), used for evidence and extra peer edges.
    pub access: Option<crate::access::Graph>,
    /// Network observations: (observer device id, `network::observe()` or
    /// `network::candidates()` JSON).
    pub observations: Vec<(String, Value)>,
    /// Optional per-edge policy records; override device-policy defaults.
    pub edge_policies: Vec<EdgePolicyInput>,
    /// Optional mesh membership.
    pub mesh: Vec<MeshMembership>,
    /// Helper checks by device id: (checked at, success), as `App::peer_evidence`.
    pub helper: BTreeMap<String, (u64, bool)>,
    /// Viewer-side metrics by device id (rtt, sessions, load).
    pub metrics: BTreeMap<String, Metrics>,
    /// LAN cluster ids the user expanded.
    pub expanded: BTreeSet<String>,
    pub now: u64,
}

const LABEL_MAX: usize = 32;

/// [`crate::ui::safe_label`] (control and bidi characters replaced), bounded.
fn safe_label(s: &str) -> String {
    let out: String = crate::ui::safe_label(s).chars().take(LABEL_MAX).collect();
    if out.is_empty() {
        "?".into()
    } else {
        out
    }
}

/// Device state from the typed helper check (`(checked at, success)`) when
/// there is one, else from the exact status values the store writes
/// (`local`, `reachable`; anything else is unknown).
pub fn device_state(
    status: &str,
    observed_at: u64,
    helper: Option<(u64, bool)>,
    now: u64,
) -> NodeState {
    let (at, success) = match helper {
        Some(check) => check,
        None if matches!(status, "local" | "reachable") => (observed_at, true),
        None => return NodeState::Unknown,
    };
    if !success {
        NodeState::Down
    } else if at == 0 || now.saturating_sub(at) > FRESH_SECS {
        NodeState::Stale
    } else {
        NodeState::Up
    }
}

/// Assembles the hub graph. Deterministic for equal inputs.
pub fn build(src: &Sources) -> Graph {
    let mut g = Graph::default();
    let mut by_ref: HashMap<String, String> = HashMap::new(); // name/target/id -> node id
    let mut mesh_of: HashMap<String, String> = HashMap::new();
    for m in &src.mesh {
        for member in &m.members {
            mesh_of
                .entry(member.clone())
                .or_insert_with(|| safe_label(&m.backend));
        }
    }
    let lookup_mesh = |d: &Device| mesh_of.get(&d.id).or_else(|| mesh_of.get(&d.name)).cloned();
    // Viewer first, then remote devices sorted by name for determinism.
    let viewer = src.devices.iter().find(|d| d.target.is_none());
    let viewer_id = viewer
        .map(|d| d.id.clone())
        .unwrap_or_else(|| "viewer".into());
    {
        let (label, mesh, metrics) = match viewer {
            Some(d) => (
                safe_label(&d.name),
                lookup_mesh(d),
                src.metrics.get(&d.id).cloned().unwrap_or_default(),
            ),
            None => ("this device".into(), None, Metrics::default()),
        };
        g.nodes.push(Node {
            id: viewer_id.clone(),
            label,
            kind: NodeKind::Viewer,
            state: NodeState::Up,
            mesh,
            hotkey: None,
            metrics,
            detail: None,
            count: None,
            expanded: false,
        });
        if let Some(d) = viewer {
            by_ref.insert(d.name.clone(), viewer_id.clone());
            by_ref.insert(d.id.clone(), viewer_id.clone());
        }
    }
    let mut remotes: Vec<&Device> = src.devices.iter().filter(|d| d.target.is_some()).collect();
    remotes.sort_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
    remotes.dedup_by(|a, b| a.id == b.id);
    for d in &remotes {
        let mut metrics = src.metrics.get(&d.id).cloned().unwrap_or_default();
        if metrics.age_secs.is_none() && d.observed_at > 0 {
            metrics.age_secs = Some(src.now.saturating_sub(d.observed_at));
        }
        g.nodes.push(Node {
            id: d.id.clone(),
            label: safe_label(&d.name),
            kind: NodeKind::Device,
            state: device_state(
                &d.status,
                d.observed_at,
                src.helper.get(&d.id).copied(),
                src.now,
            ),
            mesh: lookup_mesh(d),
            hotkey: None,
            metrics,
            detail: None,
            count: None,
            expanded: false,
        });
        by_ref.insert(d.id.clone(), d.id.clone());
        by_ref.insert(d.name.clone(), d.id.clone());
        if let Some(t) = &d.target {
            by_ref.entry(t.clone()).or_insert_with(|| d.id.clone());
        }
    }
    let mut seen: BTreeSet<(String, String, EdgeKind)> = BTreeSet::new();
    let mut push_edge = |g: &mut Graph, from: &str, to: &str, kind: EdgeKind| {
        if from == to || !seen.insert((from.into(), to.into(), kind)) {
            return;
        }
        g.edges.push(Edge {
            from: from.into(),
            to: to.into(),
            kind,
            policy: Policy::Unknown,
            direction: Direction::Forward,
            evidence: Evidence::default(),
        });
    };
    // Route edges: viewer -> hop1 -> ... -> device. Viewer-scoped evidence
    // (device policy, rtt) belongs to the last edge of each route only.
    let mut route_last: BTreeSet<(String, String)> = BTreeSet::new();
    for d in &remotes {
        let target = d.target.as_deref().unwrap_or_default();
        let hops = src.routes.get(target).cloned().unwrap_or_default();
        let mut prev = viewer_id.clone();
        for (i, hop) in hops.iter().enumerate() {
            let id = match by_ref.get(hop) {
                Some(id) => id.clone(),
                None => {
                    let id = format!("jump:{hop}");
                    if g.node_index(&id).is_none() {
                        g.nodes.push(Node {
                            id: id.clone(),
                            label: safe_label(hop),
                            kind: NodeKind::Jump,
                            state: NodeState::Unknown,
                            mesh: None,
                            hotkey: None,
                            metrics: Metrics::default(),
                            detail: None,
                            count: None,
                            expanded: false,
                        });
                    }
                    by_ref.insert(hop.clone(), id.clone());
                    id
                }
            };
            push_edge(
                &mut g,
                &prev,
                &id,
                if i == 0 {
                    EdgeKind::SshDirect
                } else {
                    EdgeKind::Jump
                },
            );
            prev = id;
        }
        route_last.insert((prev.clone(), d.id.clone()));
        push_edge(
            &mut g,
            &prev,
            &d.id,
            if hops.is_empty() {
                EdgeKind::SshDirect
            } else {
                EdgeKind::Jump
            },
        );
    }
    // Extra allowed peer edges from the access graph (e.g. core pairs).
    if let Some(access) = &src.access {
        let resolve = |id: &str| -> Option<String> {
            by_ref.get(id).cloned().or_else(|| {
                access
                    .nodes
                    .iter()
                    .find(|n| n.id == id)
                    .and_then(|n| by_ref.get(&n.target).cloned())
            })
        };
        for e in &access.edges {
            if !e.allowed {
                continue;
            }
            if let (Some(a), Some(b)) = (resolve(&e.from), resolve(&e.to)) {
                let exists = g
                    .edges
                    .iter()
                    .any(|x| (x.from == a && x.to == b) || (x.from == b && x.to == a));
                if !exists {
                    push_edge(&mut g, &a, &b, EdgeKind::SshDirect);
                }
            }
        }
        for e in g.edges.iter_mut() {
            let matches = |x: &crate::access::Edge| {
                resolve(&x.from).as_deref() == Some(e.from.as_str())
                    && resolve(&x.to).as_deref() == Some(e.to.as_str())
            };
            for x in access.edges.iter().chain(access.grants.iter()) {
                if matches(x) {
                    e.evidence.authenticated_at =
                        e.evidence.authenticated_at.max(x.authenticated_at);
                    if !x.allowed {
                        e.policy = Policy::None;
                        e.direction = Direction::Neither;
                    }
                }
            }
        }
    }
    // Default policy from device policy (Core = key both ways, Directed = key forward).
    let last_edge = |e: &Edge| route_last.contains(&(e.from.clone(), e.to.clone()));
    for e in g.edges.iter_mut() {
        if e.policy != Policy::Unknown || !last_edge(e) {
            continue;
        }
        if let Some(p) = src.policies.get(&e.to) {
            e.policy = Policy::Key;
            e.direction = match p.access {
                crate::store::AccessMode::Core => Direction::Both,
                crate::store::AccessMode::Directed => Direction::Forward,
            };
        } else if g
            .nodes
            .iter()
            .any(|n| n.id == e.to && n.kind == NodeKind::Device)
        {
            e.policy = Policy::Key;
        }
    }
    // Explicit per-edge records win.
    let resolve = |s: &str| by_ref.get(s).cloned().unwrap_or_else(|| s.to_string());
    let records: BTreeMap<(String, String), Policy> = src
        .edge_policies
        .iter()
        .map(|r| ((resolve(&r.from), resolve(&r.to)), r.policy))
        .collect();
    for e in g.edges.iter_mut() {
        let fwd = records.get(&(e.from.clone(), e.to.clone())).copied();
        let rev = records.get(&(e.to.clone(), e.from.clone())).copied();
        if fwd.is_none() && rev.is_none() {
            continue;
        }
        let allows = |p: Option<Policy>| matches!(p, Some(Policy::Key | Policy::Password));
        if let Some(p) = fwd {
            e.policy = p;
        }
        e.direction = match (fwd, allows(fwd) || fwd == Some(Policy::OneWay), allows(rev)) {
            (Some(Policy::OneWay), _, _) => Direction::Forward,
            (_, true, true) => Direction::Both,
            (_, true, false) => Direction::Forward,
            (_, false, true) => Direction::Reverse,
            (_, false, false) => {
                if fwd.is_none() {
                    e.direction
                } else {
                    Direction::Neither
                }
            }
        };
    }
    // Edges inside one mesh travel over it.
    let mesh: HashMap<&str, &str> = g
        .nodes
        .iter()
        .filter_map(|n| n.mesh.as_deref().map(|m| (n.id.as_str(), m)))
        .collect();
    for e in g.edges.iter_mut() {
        if e.kind == EdgeKind::SshDirect
            && mesh.contains_key(e.from.as_str())
            && mesh.get(e.from.as_str()) == mesh.get(e.to.as_str())
        {
            e.kind = EdgeKind::Mesh;
        }
    }
    // Rtt evidence on the last edge into a device.
    for e in g.edges.iter_mut().filter(|e| last_edge(e)) {
        if let Some(rtt) = src.metrics.get(&e.to).and_then(|m| m.rtt_ms) {
            e.evidence.rtt_ms.push(rtt);
        }
    }
    add_lan(&mut g, src, &by_ref, &mut push_edge);
    g
}

/// (address, interface, hostname hint, SSH port evidence) per observed row.
fn neighbor_rows(v: &Value, now: u64) -> Vec<(String, String, Option<String>, &'static str)> {
    let mut out = Vec::new();
    let rows = v["candidates"]
        .as_array()
        .or_else(|| v["neighbors"]["data"].as_array());
    for r in rows.into_iter().flatten().take(512) {
        let Some(addr) = r["address"].as_str().or_else(|| r["dst"].as_str()) else {
            continue;
        };
        let iface = r["interface"]
            .as_str()
            .or_else(|| r["dev"].as_str())
            .unwrap_or("lan");
        let ssh = crate::ui::network_view::neighbor_ssh(r, now);
        let host = r["hostname"].as_str().map(safe_label);
        out.push((safe_label(addr), safe_label(iface), host, ssh));
    }
    out
}

fn add_lan(
    g: &mut Graph,
    src: &Sources,
    by_ref: &HashMap<String, String>,
    push_edge: &mut impl FnMut(&mut Graph, &str, &str, EdgeKind),
) {
    let known_hosts: BTreeSet<&str> = src
        .devices
        .iter()
        .filter(|d| d.target.is_some())
        .map(|d| d.host.as_str())
        .collect();
    let mut observations: Vec<&(String, Value)> = src.observations.iter().collect();
    observations.sort_by(|a, b| a.0.cmp(&b.0));
    for (observer, value) in observations {
        let Some(observer) = by_ref.get(observer).cloned() else {
            continue;
        };
        let mut groups: BTreeMap<String, Vec<_>> = BTreeMap::new();
        for (addr, iface, host, ssh) in neighbor_rows(value, src.now) {
            if known_hosts.contains(addr.as_str()) {
                continue;
            }
            groups.entry(iface).or_default().push((addr, host, ssh));
        }
        for (iface, mut rows) in groups {
            rows.sort();
            rows.dedup_by(|a, b| a.0 == b.0);
            let id = format!("lan:{observer}:{iface}");
            let expanded = src.expanded.contains(&id);
            let open = rows.iter().filter(|r| r.2 == "port open").count();
            g.nodes.push(Node {
                id: id.clone(),
                label: format!("{} LAN", rows.len()),
                kind: NodeKind::LanCluster,
                state: if open > 0 {
                    NodeState::Up
                } else {
                    NodeState::Unknown
                },
                mesh: None,
                hotkey: None,
                metrics: Metrics::default(),
                detail: Some(if open > 0 {
                    format!("{iface} {open} open")
                } else {
                    iface.clone()
                }),
                count: Some(rows.len() as u32),
                expanded,
            });
            push_edge(g, &observer, &id, EdgeKind::LanObserved);
            if let Some(e) = g.edges.last_mut().filter(|e| e.to == id) {
                e.evidence.note = Some(iface.clone());
            }
            if expanded {
                for (addr, host, ssh) in rows {
                    let nid = format!("nb:{observer}:{iface}:{addr}");
                    // Names are unauthenticated hints: the address is the label.
                    let detail = match host {
                        Some(h) => format!("{h}? · SSH {ssh}"),
                        None => format!("SSH {ssh}"),
                    };
                    g.nodes.push(Node {
                        id: nid.clone(),
                        label: addr,
                        kind: NodeKind::Neighbor,
                        state: match ssh {
                            "port open" => NodeState::Up,
                            "port closed" => NodeState::Down,
                            _ => NodeState::Unknown,
                        },
                        mesh: None,
                        hotkey: None,
                        metrics: Metrics::default(),
                        detail: Some(detail),
                        count: None,
                        expanded: false,
                    });
                    push_edge(g, &id, &nid, EdgeKind::LanObserved);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Hotkeys
// ---------------------------------------------------------------------------

/// Letters the Network hub already binds (spec section 8): vim motion `hjkl`,
/// `g`/`G` go, `n`/`N` new, `f` filter, `s` sessions of node, `e` enrol,
/// `r` refresh, `a` add, plus `q` kept free for quit conventions.
pub const RESERVED_KEYS: &str = "hjklgGnNfsearq";

fn eligible(n: &Node) -> bool {
    matches!(n.kind, NodeKind::Viewer | NodeKind::Device | NodeKind::Jump)
}

/// Assigns stable single-letter hotkeys. `memory` is the map returned by the
/// previous call: a node keeps its letter while it exists and the letter stays
/// free; a node that disappears keeps a soft claim so it gets its letter back
/// when it returns. Returns the memory for the next frame.
pub fn assign_hotkeys(g: &mut Graph, memory: &BTreeMap<String, char>) -> BTreeMap<String, char> {
    let reserved: BTreeSet<char> = RESERVED_KEYS.chars().collect();
    let mut used: BTreeSet<char> = BTreeSet::new();
    let mut out: BTreeMap<String, char> = BTreeMap::new();
    for n in g.nodes.iter_mut() {
        n.hotkey = None;
    }
    // 1. Keep remembered letters of present nodes.
    for n in g.nodes.iter_mut().filter(|n| eligible(n)) {
        if let Some(&c) = memory.get(&n.id) {
            if c.is_ascii_alphabetic() && !reserved.contains(&c) && used.insert(c) {
                n.hotkey = Some(c);
                out.insert(n.id.clone(), c);
            }
        }
    }
    // Soft claims of absent nodes: only taken if nothing else is free.
    let present: BTreeSet<String> = g.nodes.iter().map(|n| n.id.clone()).collect();
    let soft: BTreeSet<char> = memory
        .iter()
        .filter(|(id, c)| !present.contains(id.as_str()) && !used.contains(c))
        .map(|(_, c)| *c)
        .collect();
    // 2. New nodes in a deterministic order: viewer, devices, jumps; by label.
    let mut order: Vec<usize> = (0..g.nodes.len())
        .filter(|&i| eligible(&g.nodes[i]) && g.nodes[i].hotkey.is_none())
        .collect();
    order.sort_by(|&a, &b| {
        let (x, y) = (&g.nodes[a], &g.nodes[b]);
        x.kind
            .cmp(&y.kind)
            .then(x.label.cmp(&y.label))
            .then(x.id.cmp(&y.id))
    });
    let reserved_ref = &reserved;
    let letters_of = |label: &str| -> Vec<char> {
        label
            .chars()
            .filter(|c| c.is_ascii_alphabetic())
            .map(|c| c.to_ascii_lowercase())
            .collect()
    };
    // Pass 1: the name's initial when free. Pass 2: other letters of the name,
    // then any free lowercase letter, then uppercase.
    let mut rest = Vec::new();
    for i in order {
        let initial = letters_of(&g.nodes[i].label).first().copied();
        match initial
            .filter(|c| !reserved_ref.contains(c) && !used.contains(c) && !soft.contains(c))
        {
            Some(c) => {
                used.insert(c);
                g.nodes[i].hotkey = Some(c);
                out.insert(g.nodes[i].id.clone(), c);
            }
            None => rest.push(i),
        }
    }
    for i in rest {
        let letters = letters_of(&g.nodes[i].label);
        let candidates: Vec<char> = letters
            .iter()
            .copied()
            .chain('a'..='z')
            .chain(letters.iter().map(|c| c.to_ascii_uppercase()))
            .chain('A'..='Z')
            .collect();
        let free = |c: &char| !reserved.contains(c) && !used.contains(c);
        let pick = candidates
            .iter()
            .copied()
            .find(|c| free(c) && !soft.contains(c))
            .or_else(|| candidates.iter().copied().find(|c| free(c)));
        if let Some(c) = pick {
            used.insert(c);
            g.nodes[i].hotkey = Some(c);
            out.insert(g.nodes[i].id.clone(), c);
        }
    }
    for (id, c) in memory {
        if !present.contains(id.as_str()) && !used.contains(c) && !out.values().any(|v| v == c) {
            out.insert(id.clone(), *c);
        }
    }
    out
}
