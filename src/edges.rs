//! Per-edge, per-direction access policy (docs/design/ui-revamp.md 10.5).
//!
//! Each record governs one direction `from -> to`:
//! - `key`: `from` may log in to `to` with its cx-managed peer key;
//! - `password`: cx removes `from`'s cx-managed key on `to`; other SSH keys the
//!   user configured may still authenticate, so status never claims otherwise;
//! - `none`: refused; cx opens no SSH over it (transport requests, core sync,
//!   pairing, jumps), except the one enforcement step that removes the viewer's
//!   own key on the receiver.
//!
//! `one-way A B` is a request shorthand stored as `A -> B key` plus `B -> A none`.
//! Records live in `edges.json` (0600, atomic, bounded, under the device lock).
//! Pairs without records keep today's posture, migrated read-only from
//! `device-policies.json`: core devices are `key` both ways, directed devices
//! are `key` viewer -> device and refused device -> viewer. Status reports
//! intent, cx enforcement and observed evidence separately; none implies another.
use crate::{
    model::{Device, Operation},
    peers, store, transport,
};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const LOCAL: &str = "local";
const STORE_BYTES: usize = 256 * 1024;
const RECORD_LIMIT: usize = 1024;
const DETAIL_LIMIT: usize = 200;

/// `one-way` is accepted in requests only; `validate` refuses it in edges.json.
#[derive(
    Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, clap::ValueEnum,
)]
#[serde(rename_all = "snake_case")]
pub enum Policy {
    Key,
    Password,
    OneWay,
    None,
}
impl Policy {
    pub fn label(self) -> &'static str {
        match self {
            Policy::Key => "key",
            Policy::Password => "password",
            Policy::OneWay => "one-way",
            Policy::None => "none",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EdgePolicy {
    pub from: String,
    pub to: String,
    pub policy: Policy,
    pub updated_at: u64,
}

/// Effective access for one direction.
#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Access {
    Key,
    Password,
    Refused,
    /// No record and no migrated posture: cx does not manage this direction.
    Unmanaged,
}
impl Access {
    fn label(self) -> &'static str {
        match self {
            Access::Key => "key",
            Access::Password => "password",
            Access::Refused => "refused",
            Access::Unmanaged => "unmanaged",
        }
    }
}

/// Where a direction's access comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Source {
    Recorded,
    MigratedCore,
    MigratedDirected,
    Unenrolled,
    Unmanaged,
}
impl Source {
    fn label(self) -> &'static str {
        match self {
            Source::Recorded => "edges.json",
            Source::MigratedCore => "migrated core",
            Source::MigratedDirected => "migrated directed",
            Source::Unenrolled => "unenrolled",
            Source::Unmanaged => "unmanaged",
        }
    }
}

/// Latest cx enforcement outcome for one direction.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Enforcement {
    pub from: String,
    pub to: String,
    pub state: String,
    pub ok: bool,
    pub at: u64,
}
/// Authentication evidence for one direction, written by the UI's checks.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Evidence {
    pub from: String,
    pub to: String,
    #[serde(default)]
    pub last_ok: Option<u64>,
    #[serde(default)]
    pub last_failed: Option<u64>,
    #[serde(default)]
    pub detail: Option<String>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct EdgeFile {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    edges: Vec<EdgePolicy>,
    #[serde(default)]
    enforcement: Vec<Enforcement>,
    #[serde(default)]
    evidence: Vec<Evidence>,
}
impl EdgeFile {
    fn validate(&self) -> Result<()> {
        anyhow::ensure!(
            self.edges.len() <= RECORD_LIMIT
                && self.enforcement.len() <= 2 * RECORD_LIMIT
                && self.evidence.len() <= 2 * RECORD_LIMIT,
            "too many edge policy records"
        );
        let mut seen = BTreeSet::new();
        for e in &self.edges {
            anyhow::ensure!(
                valid_id(&e.from)
                    && valid_id(&e.to)
                    && e.from != e.to
                    && e.policy != Policy::OneWay
                    && seen.insert((&e.from, &e.to)),
                "invalid or duplicate edge policy"
            );
        }
        Ok(())
    }
    fn record(&self, from: &str, to: &str) -> Option<&EdgePolicy> {
        self.edges.iter().find(|e| e.from == from && e.to == to)
    }
}
fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 256 && !id.chars().any(char::is_control)
}

/// Everything plan computation needs; pure functions operate on it in tests.
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    devices: Vec<Device>,
    core: BTreeSet<String>,
    file: EdgeFile,
    routes: BTreeMap<String, Vec<String>>,
    now: u64,
}
/// Load one snapshot so a caller's decisions all come from the same files.
pub fn load() -> Result<Snapshot> {
    let devices = store::devices()?;
    let mut core = BTreeSet::new();
    for d in devices.iter().filter(|d| d.target.is_some()) {
        if store::policy(d)?.access == store::AccessMode::Core {
            core.insert(d.id.clone());
        }
    }
    let file = read()?;
    Ok(Snapshot {
        devices,
        core,
        file,
        routes: store::routes()?,
        now: transport::now(),
    })
}

fn read() -> Result<EdgeFile> {
    let file: EdgeFile = store::read_state("edges.json", STORE_BYTES as u64)?.unwrap_or_default();
    file.validate()?;
    Ok(file)
}
fn write(file: &EdgeFile) -> Result<()> {
    file.validate()?;
    store::write_state("edges.json", file, STORE_BYTES)
}

impl Snapshot {
    fn remote(&self, id: &str) -> Option<&Device> {
        self.devices
            .iter()
            .find(|d| d.target.is_some() && d.id == id)
    }
    fn known(&self, id: &str) -> bool {
        id == LOCAL || self.remote(id).is_some()
    }
    fn name(&self, id: &str) -> String {
        if id == LOCAL {
            return self
                .devices
                .iter()
                .find(|d| d.target.is_none())
                .map(|d| d.name.clone())
                .unwrap_or_else(|| LOCAL.into());
        }
        self.remote(id)
            .map(|d| d.name.clone())
            .unwrap_or_else(|| id.into())
    }
    fn by_target(&self, target: &str) -> Option<&str> {
        self.devices
            .iter()
            .find(|d| d.target.as_deref() == Some(target))
            .map(|d| d.id.as_str())
    }
    /// Effective access and where it comes from.
    fn access(&self, from: &str, to: &str) -> (Access, Source) {
        if let Some(r) = self.file.record(from, to) {
            return (
                match r.policy {
                    // One-way is never stored (validate); keep the match total.
                    Policy::Key | Policy::OneWay => Access::Key,
                    Policy::Password => Access::Password,
                    Policy::None => Access::Refused,
                },
                Source::Recorded,
            );
        }
        let core = |id: &str| id == LOCAL || self.core.contains(id);
        match (from == LOCAL, to == LOCAL) {
            _ if !self.known(from) || !self.known(to) => (Access::Unmanaged, Source::Unenrolled),
            (true, false) if core(to) => (Access::Key, Source::MigratedCore),
            (true, false) => (Access::Key, Source::MigratedDirected),
            (false, true) if core(from) => (Access::Key, Source::MigratedCore),
            (false, true) => (Access::Refused, Source::MigratedDirected),
            _ if core(from) && core(to) => (Access::Key, Source::MigratedCore),
            _ => (Access::Unmanaged, Source::Unmanaged),
        }
    }
    /// The records this change writes, applied over the current file.
    fn changed(&self, change: &Change) -> Vec<EdgePolicy> {
        let (f, t, now) = (&change.from, &change.to, self.now);
        let mut edges: Vec<EdgePolicy> = self
            .file
            .edges
            .iter()
            .filter(|e| !((&e.from == f && &e.to == t) || (&e.from == t && &e.to == f)))
            .cloned()
            .collect();
        let record = |from: &str, to: &str, policy| EdgePolicy {
            from: from.into(),
            to: to.into(),
            policy,
            updated_at: now,
        };
        match change.policy {
            Policy::OneWay => {
                edges.push(record(f, t, Policy::Key));
                edges.push(record(t, f, Policy::None));
            }
            Policy::None => {
                edges.push(record(f, t, Policy::None));
                edges.push(record(t, f, Policy::None));
            }
            policy => {
                edges.push(record(f, t, policy));
                // The reverse keeps its own record, or its migrated posture.
                edges.extend(self.file.record(t, f).cloned());
            }
        }
        edges
    }
    /// Whether `from -> to` may use a cx-managed key (core sync and pairing gate):
    /// only recorded edges.json decisions refuse it.
    pub fn key_allowed(&self, from: &str, to: &str) -> bool {
        let (access, source) = self.access(from, to);
        source != Source::Recorded || access == Access::Key
    }
    fn with_edges(&self, edges: Vec<EdgePolicy>) -> Snapshot {
        let mut next = self.clone();
        next.file.edges = edges;
        next
    }
    fn route_nodes(&self, target: &str, hops: &[String]) -> Vec<Option<String>> {
        std::iter::once(Some(LOCAL.to_string()))
            .chain(
                hops.iter()
                    .chain(std::iter::once(&target.to_string()))
                    .map(|t| self.by_target(t).map(str::to_owned)),
            )
            .collect()
    }
    /// The first refused direction a route crosses, if any.
    fn refused_hop(&self, target: &str, hops: &[String]) -> Option<(String, String)> {
        let nodes = self.route_nodes(target, hops);
        nodes.windows(2).find_map(|w| match (&w[0], &w[1]) {
            (Some(a), Some(b)) if self.access(a, b).0 == Access::Refused => {
                Some((a.clone(), b.clone()))
            }
            _ => None,
        })
    }
    fn routes_over(&self, from: &str, to: &str) -> Vec<String> {
        self.routes
            .iter()
            .filter(|(target, hops)| {
                self.route_nodes(target, hops)
                    .windows(2)
                    .any(|w| w[0].as_deref() == Some(from) && w[1].as_deref() == Some(to))
            })
            .map(|(target, _)| target.clone())
            .collect()
    }
}

/// A requested policy for `from -> to` (one-way and none also set the reverse).
#[derive(Clone, Debug, Serialize)]
pub struct Change {
    pub from: String,
    pub to: String,
    pub policy: Policy,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum Step {
    /// Ensure `from`'s cx-managed key is authorized on `to` (needs a verified pin).
    InstallKey { from: String, to: String },
    /// Remove only `from`'s cx-managed line on `to`; user lines stay.
    RemoveKey { from: String, to: String },
}

#[derive(Clone, Debug, Serialize)]
pub struct Plan {
    pub change: Change,
    pub from_name: String,
    pub to_name: String,
    /// Records for this pair before the change; apply refuses if they moved.
    pub basis: Vec<EdgePolicy>,
    /// Records for this pair after the change.
    pub records: Vec<EdgePolicy>,
    pub steps: Vec<Step>,
    /// Human one-liners, e.g. "lab-arm will no longer reach laptop".
    pub consequences: Vec<String>,
    pub refused_routes: Vec<String>,
}

fn pair_records(edges: &[EdgePolicy], a: &str, b: &str) -> Vec<EdgePolicy> {
    let mut v: Vec<_> = edges
        .iter()
        .filter(|e| (e.from == a && e.to == b) || (e.from == b && e.to == a))
        .cloned()
        .collect();
    v.sort_by(|x, y| (&x.from, &x.to).cmp(&(&y.from, &y.to)));
    v
}

fn plural(n: usize, word: &str) -> String {
    if n == 1 {
        format!("1 {word}")
    } else {
        format!("{n} {word}s")
    }
}

fn plan_with(snapshot: &Snapshot, change: Change) -> Result<Plan> {
    anyhow::ensure!(
        valid_id(&change.from) && valid_id(&change.to),
        "invalid edge endpoint"
    );
    anyhow::ensure!(change.from != change.to, "an edge needs two devices");
    anyhow::ensure!(
        snapshot.known(&change.from) && snapshot.known(&change.to),
        "both edge endpoints must be enrolled"
    );
    let edges = snapshot.changed(&change);
    anyhow::ensure!(edges.len() <= RECORD_LIMIT, "edge policy limit reached");
    let next = snapshot.with_edges(edges.clone());
    let (f, t) = (change.from.as_str(), change.to.as_str());
    let mut steps = Vec::new();
    let mut consequences = Vec::new();
    let mut refused_routes = Vec::new();
    for (x, y) in [(f, t), (t, f)] {
        let (before, _) = snapshot.access(x, y);
        let (after, _) = next.access(x, y);
        let (xn, yn) = (snapshot.name(x), snapshot.name(y));
        // Intent is saved before enforcement, so a failed install is retried.
        let failed = snapshot
            .file
            .enforcement
            .iter()
            .any(|e| e.from == x && e.to == y && !e.ok);
        match after {
            Access::Key if before != Access::Key || failed => steps.push(Step::InstallKey {
                from: x.into(),
                to: y.into(),
            }),
            Access::Password | Access::Refused => steps.push(Step::RemoveKey {
                from: x.into(),
                to: y.into(),
            }),
            _ => {}
        }
        if before == after {
            if failed && after == Access::Key {
                consequences.push(format!(
                    "cx will retry installing {xn}'s cx key on {yn}; the last attempt failed"
                ));
            }
            continue;
        }
        consequences.push(match (before, after) {
            (_, Access::Refused) => format!("{xn} will no longer reach {yn}"),
            (_, Access::Password) => format!(
                "cx will remove {xn}'s cx key on {yn}; other SSH keys {xn} has there may still log in"
            ),
            (Access::Password, Access::Key) => {
                format!("{xn} will reach {yn} with its cx key, without a password")
            }
            (_, Access::Key) => format!("{xn} will reach {yn} with its cx key"),
            (_, Access::Unmanaged) => format!("cx will stop managing {xn} → {yn}"),
        });
        if after == Access::Refused {
            let routes = snapshot.routes_over(x, y);
            if !routes.is_empty() {
                let gateway = if x == LOCAL { &yn } else { &xn };
                consequences.push(format!(
                    "{} via {gateway} will be refused",
                    plural(routes.len(), "jump route")
                ));
                refused_routes.extend(routes);
            }
        }
        if after != Access::Key
            && before == Access::Key
            && [x, y]
                .iter()
                .all(|id| *id == LOCAL || snapshot.core.contains(*id))
        {
            consequences.push(format!("core sync will not install {xn}'s key on {yn}"));
        }
    }
    if consequences.is_empty() {
        consequences.push(format!(
            "no access change for {} and {}",
            snapshot.name(f),
            snapshot.name(t)
        ));
    }
    refused_routes.sort();
    refused_routes.dedup();
    // Remote steps run before local ones so the viewer's own key still opens
    // the helper channel it is about to revoke; installs precede removals.
    steps.sort_by_key(|s| {
        let (on, install) = match s {
            Step::InstallKey { to, .. } => (to, true),
            Step::RemoveKey { to, .. } => (to, false),
        };
        (on == LOCAL, !install)
    });
    Ok(Plan {
        from_name: snapshot.name(f),
        to_name: snapshot.name(t),
        basis: pair_records(&snapshot.file.edges, f, t),
        records: pair_records(&edges, f, t),
        change,
        steps,
        consequences,
        refused_routes,
    })
}

/// Preview a change. Reads local state only; no I/O to other devices.
pub fn plan(change: Change) -> Result<Plan> {
    plan_with(&load()?, change)
}

#[derive(Clone, Debug, Serialize)]
pub struct StepResult {
    pub step: Step,
    pub ok: bool,
    pub state: String,
}
#[derive(Clone, Debug, Serialize)]
pub struct Applied {
    pub saved: bool,
    pub enforced: bool,
    pub steps: Vec<StepResult>,
    pub consequences: Vec<String>,
}

fn bounded(text: String) -> String {
    let text: String = text.chars().filter(|c| !c.is_control()).collect();
    if text.len() <= DETAIL_LIMIT {
        return text;
    }
    let mut end = DETAIL_LIMIT;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &text[..end])
}

fn device_for(id: &str) -> Result<Device> {
    if id == LOCAL {
        return Ok(store::local_device());
    }
    store::devices()?
        .into_iter()
        .find(|d| d.target.is_some() && d.id == id)
        .context("edge endpoint is no longer enrolled")
}

fn enforce(step: &Step) -> Result<String> {
    let (from, to, authorize) = match step {
        Step::InstallKey { from, to } => (from, to, true),
        Step::RemoveKey { from, to } => (from, to, false),
    };
    let receiver = device_for(to)?;
    let (peer_id, client_key) = if from == LOCAL {
        match crate::peer_ssh::client_key()? {
            Some(key) => (None, Some(key)),
            None if !authorize => return Ok("cx key absent (no cx key on this viewer)".into()),
            None => bail!("this viewer has no cx key; run core sync first"),
        }
    } else {
        (Some(from.clone()), None)
    };
    let result = peers::checked(
        &receiver,
        Operation::PeerAuthorize {
            peer_id,
            client_key,
            authorize,
        },
        "peer-authorize-v1",
    )
    .map_err(|e| {
        // BatchMode cannot restore a key it needed to log in with.
        let rejected = matches!(
            e.downcast_ref::<transport::ConnectionFailure>(),
            Some(transport::ConnectionFailure::Authentication)
        );
        if authorize && from == LOCAL && rejected {
            e.context(format!(
                "{} rejected this viewer's background SSH; on {} run cx edge set for this viewer, or authorize your own SSH key there, then rerun",
                receiver.name, receiver.name
            ))
        } else {
            e
        }
    })?;
    let installed = result["managed_key_installed"] == true;
    let user = result["user_authorization_present"] == true;
    anyhow::ensure!(
        installed == authorize,
        "helper did not confirm the cx key change"
    );
    Ok(match (installed, user) {
        (true, _) => "cx key installed".into(),
        (false, false) => "cx key absent".into(),
        (false, true) => "cx key absent; a user authorized_keys line for this key remains".into(),
    })
}

/// Save the intent, then enforce each step through the existing helper channels.
/// Never applies networking changes.
pub fn apply(plan: &Plan) -> Result<Applied> {
    let (f, t) = (&plan.change.from, &plan.change.to);
    {
        let _guard = store::device_guard()?;
        let snapshot = load()?;
        anyhow::ensure!(
            pair_records(&snapshot.file.edges, f, t) == plan.basis,
            "edge policy changed since the preview; preview again"
        );
        let mut file = snapshot.file.clone();
        file.version = 1;
        file.edges
            .retain(|e| !((&e.from == f && &e.to == t) || (&e.from == t && &e.to == f)));
        file.edges.extend(plan.records.iter().cloned());
        write(&file)?;
    }
    let mut results = Vec::new();
    for step in &plan.steps {
        // The helper channel uses the user's SSH setup, so a successful step is
        // not evidence for the cx-key direction; no evidence is recorded here.
        let (ok, state) = match enforce(step) {
            Ok(state) => (true, state),
            Err(e) => (false, bounded(format!("failed: {e:#}"))),
        };
        results.push(StepResult {
            step: step.clone(),
            ok,
            state,
        });
    }
    {
        let _guard = store::device_guard()?;
        let mut file = read()?;
        let now = transport::now();
        record_enforcement(&mut file.enforcement, &results, now);
        write(&file)?;
    }
    Ok(Applied {
        saved: true,
        enforced: results.iter().all(|r| r.ok),
        steps: results,
        consequences: plan.consequences.clone(),
    })
}

/// Keep the latest outcome per direction; at the cap, the oldest go first.
fn record_enforcement(records: &mut Vec<Enforcement>, results: &[StepResult], now: u64) {
    for r in results {
        let (from, to) = match &r.step {
            Step::InstallKey { from, to } | Step::RemoveKey { from, to } => (from, to),
        };
        records.retain(|e| !(&e.from == from && &e.to == to));
        records.push(Enforcement {
            from: from.clone(),
            to: to.clone(),
            state: r.state.clone(),
            ok: r.ok,
            at: now,
        });
    }
    let excess = records.len().saturating_sub(2 * RECORD_LIMIT);
    records.drain(..excess);
}

#[derive(Clone, Debug, Serialize)]
pub struct DirectionView {
    pub from: String,
    pub to: String,
    /// Intended access: "key", "password", "refused" or "unmanaged".
    pub access: Access,
    /// "edges.json", "migrated core", "migrated directed", "unenrolled", "unmanaged".
    pub source: String,
    pub enforced: String,
    pub observed: String,
}
#[derive(Clone, Debug, Serialize)]
pub struct EdgeView {
    pub from: String,
    pub to: String,
    pub from_name: String,
    pub to_name: String,
    /// Compact label for the graph: "key ⇄", "key →", "pwd →", "⊘", ...
    pub label: String,
    pub forward: DirectionView,
    pub reverse: DirectionView,
    pub routes_over: usize,
}

fn age(now: u64, at: u64) -> String {
    let s = now.saturating_sub(at);
    match s {
        0..=59 => format!("{s}s ago"),
        60..=3599 => format!("{}m ago", s / 60),
        3600..=86399 => format!("{}h ago", s / 3600),
        _ => format!("{}d ago", s / 86400),
    }
}

fn direction_view(s: &Snapshot, from: &str, to: &str) -> DirectionView {
    let (access, source) = s.access(from, to);
    let enforced = match s
        .file
        .enforcement
        .iter()
        .find(|e| e.from == from && e.to == to)
    {
        Some(e) => format!("{} ({})", e.state, age(s.now, e.at)),
        None if access == Access::Unmanaged => "unmanaged (your SSH setup)".into(),
        None if source == Source::MigratedCore && access == Access::Key => {
            "not applied by cx edge (core sync manages keys)".into()
        }
        None if source == Source::MigratedDirected && access == Access::Key => {
            "unmanaged (your SSH setup)".into()
        }
        None => "not applied".into(),
    };
    let evidence = s
        .file
        .evidence
        .iter()
        .find(|e| e.from == from && e.to == to);
    let observed = match evidence {
        Some(e) => {
            let mut parts = Vec::new();
            if let Some(at) = e.last_ok {
                parts.push(format!("authenticated {}", age(s.now, at)));
            }
            if let Some(at) = e.last_failed {
                parts.push(format!("auth failed {}", age(s.now, at)));
            }
            if parts.is_empty() {
                "not checked".into()
            } else {
                parts.join("; ")
            }
        }
        None => "not checked".into(),
    };
    DirectionView {
        from: from.into(),
        to: to.into(),
        access,
        source: source.label().into(),
        enforced,
        observed,
    }
}

fn label(forward: Access, reverse: Access) -> String {
    let short = |a: Access| match a {
        Access::Key => "key",
        Access::Password => "pwd",
        Access::Refused => "⊘",
        Access::Unmanaged => "ssh",
    };
    match (forward, reverse) {
        (Access::Refused, Access::Refused) => "⊘".into(),
        (a, b) if a == b => format!("{} ⇄", short(a)),
        (a, Access::Refused) => format!("{} →", short(a)),
        (Access::Refused, b) => format!("{} ←", short(b)),
        (a, b) => format!("{} → · {} ←", short(a), short(b)),
    }
}

fn graph_with(s: &Snapshot) -> Vec<EdgeView> {
    let mut pairs: Vec<(String, String)> = Vec::new();
    let mut seen = BTreeSet::new();
    let mut add = |a: &str, b: &str, pairs: &mut Vec<(String, String)>| {
        let key = if a < b { (a, b) } else { (b, a) };
        if seen.insert((key.0.to_string(), key.1.to_string())) {
            pairs.push((a.into(), b.into()));
        }
    };
    let remotes: Vec<&Device> = s.devices.iter().filter(|d| d.target.is_some()).collect();
    for d in &remotes {
        add(LOCAL, &d.id, &mut pairs);
    }
    for e in &s.file.edges {
        if s.known(&e.from) && s.known(&e.to) {
            add(&e.from, &e.to, &mut pairs);
        }
    }
    for (i, a) in remotes.iter().enumerate() {
        for b in &remotes[i + 1..] {
            if s.core.contains(&a.id) && s.core.contains(&b.id) {
                add(&a.id, &b.id, &mut pairs);
            }
        }
    }
    pairs
        .into_iter()
        .map(|(a, b)| {
            let forward = direction_view(s, &a, &b);
            let reverse = direction_view(s, &b, &a);
            EdgeView {
                from_name: s.name(&a),
                to_name: s.name(&b),
                label: label(forward.access, reverse.access),
                routes_over: s.routes_over(&a, &b).len() + s.routes_over(&b, &a).len(),
                from: a,
                to: b,
                forward,
                reverse,
            }
        })
        .collect()
}

/// Every edge the viewer knows, with intent, enforcement and evidence per direction.
/// Reads several state files: load it on refresh or after a policy change and
/// keep it in view state; never call it from render.
pub fn graph() -> Result<Vec<EdgeView>> {
    Ok(graph_with(&load()?))
}

/// Refuse an explicit action (e.g. pairing) only on a recorded edge decision.
/// Migrated postures describe today's behaviour; explicit pairing may change them.
pub fn require_key(from: &str, to: &str, what: &str) -> Result<()> {
    let s = load()?;
    let (access, _) = s.access(from, to);
    anyhow::ensure!(
        s.key_allowed(from, to),
        "edge policy refuses {what}: {} → {} is {}",
        s.name(from),
        s.name(to),
        access.label()
    );
    Ok(())
}

fn check_route_with(s: &Snapshot, target: &str, hops: &[String]) -> Result<()> {
    if let Some((a, b)) = s.refused_hop(target, hops) {
        bail!(
            "edge policy refuses this route: {} → {} is refused",
            s.name(&a),
            s.name(&b)
        );
    }
    Ok(())
}
/// Refuse a jump route (viewer → hops → target) crossing a refused direction.
pub fn check_route(target: &str, hops: &[String]) -> Result<()> {
    check_route_with(&load()?, target, hops)
}
/// Refuse connecting to a device when the viewer's direction or its route is refused.
pub fn guard(device: &Device) -> Result<()> {
    let Some(target) = device.target.as_deref() else {
        return Ok(());
    };
    let s = load()?;
    let hops = s.routes.get(target).cloned().unwrap_or_default();
    check_route_with(&s, target, &hops)
}
/// Resolve a CLI endpoint: "local", the local name, or an enrolled name/id/target.
pub fn resolve(name: &str) -> Result<String> {
    let devices = store::devices()?;
    if name == LOCAL
        || devices
            .iter()
            .any(|d| d.target.is_none() && (d.name == name || d.id == name))
    {
        return Ok(LOCAL.into());
    }
    let matches: Vec<_> = devices
        .iter()
        .filter(|d| {
            d.target.is_some()
                && (d.name == name || d.id == name || d.target.as_deref() == Some(name))
        })
        .collect();
    match matches.as_slice() {
        [d] => Ok(d.id.clone()),
        [] => bail!("device not enrolled: {name}"),
        _ => bail!("ambiguous device name: {name}"),
    }
}

pub fn render_list(edges: &[EdgeView]) -> String {
    let mut out = String::new();
    for e in edges {
        out.push_str(&format!("{} — {}  {}\n", e.from_name, e.to_name, e.label));
        for d in [&e.forward, &e.reverse] {
            let (from, to) = if d.from == e.from {
                (&e.from_name, &e.to_name)
            } else {
                (&e.to_name, &e.from_name)
            };
            out.push_str(&format!(
                "  {from} → {to}: intended {} [{}]; enforced {}; observed {}\n",
                d.access.label(),
                d.source,
                d.enforced,
                d.observed
            ));
        }
        if e.routes_over > 0 {
            out.push_str(&format!(
                "  {} over this edge\n",
                plural(e.routes_over, "jump route")
            ));
        }
    }
    if out.is_empty() {
        out.push_str("no edges: enroll a device with cx add\n");
    }
    out
}

/// "install cx key laptop → lab-arm", named from the plan's endpoints.
fn step_text(plan: &Plan, step: &Step) -> String {
    let (verb, from, to) = match step {
        Step::InstallKey { from, to } => ("install", from, to),
        Step::RemoveKey { from, to } => ("remove", from, to),
    };
    let name = |id: &String| {
        if *id == plan.change.from {
            &plan.from_name
        } else {
            &plan.to_name
        }
    };
    format!("{verb} cx key {} → {}", name(from), name(to))
}

pub fn render_plan(plan: &Plan) -> String {
    let mut out = format!(
        "{} → {}: {}\n",
        plan.from_name,
        plan.to_name,
        plan.change.policy.label()
    );
    for c in &plan.consequences {
        out.push_str(&format!("  • {c}\n"));
    }
    for s in &plan.steps {
        out.push_str(&format!("  step: {}\n", step_text(plan, s)));
    }
    out
}

pub fn render_applied(plan: &Plan, applied: &Applied) -> String {
    let mut out = String::from(if applied.enforced {
        "saved and enforced\n"
    } else {
        "saved; enforcement incomplete\n"
    });
    for r in &applied.steps {
        out.push_str(&format!("  {}: {}\n", step_text(plan, &r.step), r.state));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    fn device(id: &str, name: &str, target: &str) -> Device {
        Device {
            id: id.into(),
            name: name.into(),
            target: Some(target.into()),
            account: "robot".into(),
            host: name.into(),
            status: "unknown".into(),
            observed_at: 0,
        }
    }
    fn snapshot() -> Snapshot {
        let mut local = store::local_device();
        local.name = "laptop".into();
        Snapshot {
            devices: vec![
                local,
                device("m1:robot", "lab-arm", "robot@100.96.0.2"),
                device("m2:robot", "jetson", "robot@192.168.0.7"),
                device("m3:robot", "rover", "rover"),
            ],
            core: ["m1:robot".to_string()].into(),
            file: EdgeFile::default(),
            routes: [
                (
                    "robot@192.168.0.7".to_string(),
                    vec!["robot@100.96.0.2".to_string()],
                ),
                ("rover".to_string(), vec![]),
            ]
            .into(),
            now: 1000,
        }
    }
    fn change(from: &str, to: &str, policy: Policy) -> Change {
        Change {
            from: from.into(),
            to: to.into(),
            policy,
        }
    }

    #[test]
    fn migration_maps_core_and_directed_without_rewriting_device_policies() {
        let s = snapshot();
        assert_eq!(
            s.access(LOCAL, "m1:robot"),
            (Access::Key, Source::MigratedCore)
        );
        assert_eq!(
            s.access("m1:robot", LOCAL),
            (Access::Key, Source::MigratedCore)
        );
        assert_eq!(
            s.access(LOCAL, "m2:robot"),
            (Access::Key, Source::MigratedDirected)
        );
        assert_eq!(
            s.access("m2:robot", LOCAL),
            (Access::Refused, Source::MigratedDirected)
        );
        assert_eq!(s.access("m1:robot", "m2:robot").0, Access::Unmanaged);
        let mut s2 = s.clone();
        s2.core.insert("m2:robot".into());
        assert_eq!(
            s2.access("m1:robot", "m2:robot"),
            (Access::Key, Source::MigratedCore)
        );
        let views = graph_with(&s);
        let arm = views.iter().find(|e| e.to == "m1:robot").unwrap();
        assert_eq!(arm.label, "key ⇄");
        let jet = views.iter().find(|e| e.to == "m2:robot").unwrap();
        assert_eq!(jet.label, "key →");
        assert_eq!(jet.forward.enforced, "unmanaged (your SSH setup)");
        assert_eq!(
            arm.forward.enforced,
            "not applied by cx edge (core sync manages keys)"
        );
        assert_eq!(jet.reverse.access, Access::Refused);
        assert_eq!(jet.reverse.source, "migrated directed");
        // Old device-policies.json content parses unchanged and stays independent.
        let old: store::DevicePolicy =
            serde_json::from_str(r#"{"access":"core","availability":"usually_up"}"#).unwrap();
        assert_eq!(old.access, store::AccessMode::Core);
    }

    #[test]
    fn one_way_plan_installs_forward_removes_reverse_and_refuses_reverse_routes() {
        let s = snapshot();
        let p = plan_with(&s, change(LOCAL, "m1:robot", Policy::OneWay)).unwrap();
        assert_eq!(
            p.steps,
            vec![Step::RemoveKey {
                from: "m1:robot".into(),
                to: LOCAL.into()
            }]
        );
        assert_eq!(
            p.consequences,
            [
                "lab-arm will no longer reach laptop",
                "core sync will not install lab-arm's key on laptop"
            ]
        );
        // Stored as key + none; one-way is never persisted.
        let stored: Vec<_> = p
            .records
            .iter()
            .map(|r| (r.from.as_str(), r.to.as_str(), r.policy))
            .collect();
        assert_eq!(
            stored,
            [
                (LOCAL, "m1:robot", Policy::Key),
                ("m1:robot", LOCAL, Policy::None)
            ]
        );
        let next = s.with_edges(s.changed(&p.change));
        assert_eq!(next.access(LOCAL, "m1:robot").0, Access::Key);
        assert_eq!(next.access("m1:robot", LOCAL).0, Access::Refused);
        // Forward routes via lab-arm still work.
        assert!(check_route_with(&next, "robot@192.168.0.7", &["robot@100.96.0.2".into()]).is_ok());
    }

    #[test]
    fn none_refuses_both_directions_and_routes_over_the_edge() {
        let s = snapshot();
        let p = plan_with(&s, change(LOCAL, "m1:robot", Policy::None)).unwrap();
        assert_eq!(
            p.consequences,
            [
                "laptop will no longer reach lab-arm",
                "1 jump route via lab-arm will be refused",
                "core sync will not install laptop's key on lab-arm",
                "lab-arm will no longer reach laptop",
                "core sync will not install lab-arm's key on laptop",
            ]
        );
        assert_eq!(p.refused_routes, ["robot@192.168.0.7"]);
        // Remote removal first, while the viewer's key still opens the channel.
        assert_eq!(
            p.steps,
            vec![
                Step::RemoveKey {
                    from: LOCAL.into(),
                    to: "m1:robot".into()
                },
                Step::RemoveKey {
                    from: "m1:robot".into(),
                    to: LOCAL.into()
                },
            ]
        );
        let next = s.with_edges(s.changed(&p.change));
        let error = check_route_with(&next, "robot@192.168.0.7", &["robot@100.96.0.2".into()])
            .unwrap_err()
            .to_string();
        assert_eq!(
            error,
            "edge policy refuses this route: laptop → lab-arm is refused"
        );
        assert!(check_route_with(&next, "robot@100.96.0.2", &[]).is_err());
        assert!(check_route_with(&next, "rover", &[]).is_ok());
        // A route between other devices via an unknown hop stays unmanaged.
        assert!(check_route_with(&next, "robot@192.168.0.7", &["robot@172.16.0.1".into()]).is_ok());
    }

    #[test]
    fn password_removes_key_for_that_direction_only_without_claiming_auth_state() {
        let s = snapshot();
        let p = plan_with(&s, change(LOCAL, "m1:robot", Policy::Password)).unwrap();
        assert_eq!(
            p.steps,
            vec![Step::RemoveKey {
                from: LOCAL.into(),
                to: "m1:robot".into()
            }]
        );
        assert_eq!(
            p.consequences[0],
            "cx will remove laptop's cx key on lab-arm; other SSH keys laptop has there may still log in"
        );
        let next = s.with_edges(s.changed(&p.change));
        assert_eq!(next.access("m1:robot", LOCAL).0, Access::Key);
        let view = graph_with(&next);
        let arm = view.iter().find(|e| e.to == "m1:robot").unwrap();
        // Only recorded evidence is reported; intent implies nothing about auth.
        assert_eq!(arm.forward.observed, "not checked");
        assert_eq!(arm.label, "pwd → · key ←");
        // Password then key on a one-way pair keeps the reverse refusal.
        let ow = next.with_edges(next.changed(&change("m1:robot", LOCAL, Policy::OneWay)));
        let back = ow.with_edges(ow.changed(&change("m1:robot", LOCAL, Policy::Password)));
        assert_eq!(back.access(LOCAL, "m1:robot").0, Access::Refused);
        let key = ow.with_edges(ow.changed(&change(LOCAL, "m1:robot", Policy::Key)));
        assert_eq!(key.access(LOCAL, "m1:robot").0, Access::Key);
        assert_eq!(key.access("m1:robot", LOCAL).0, Access::Key);
        assert!(key.file.edges.iter().all(|e| e.policy != Policy::OneWay));
    }

    #[test]
    fn directed_promotion_and_no_change_consequences() {
        let s = snapshot();
        let p = plan_with(&s, change("m2:robot", LOCAL, Policy::Key)).unwrap();
        assert_eq!(p.consequences, ["jetson will reach laptop with its cx key"]);
        assert_eq!(
            p.steps,
            vec![Step::InstallKey {
                from: "m2:robot".into(),
                to: LOCAL.into()
            }]
        );
        let p = plan_with(&s, change(LOCAL, "m1:robot", Policy::Key)).unwrap();
        assert_eq!(p.consequences, ["no access change for laptop and lab-arm"]);
        assert!(p.steps.is_empty());
        assert!(plan_with(&s, change(LOCAL, LOCAL, Policy::Key)).is_err());
        assert!(plan_with(&s, change(LOCAL, "ghost", Policy::Key)).is_err());
    }

    #[test]
    fn routes_plural_and_stored_one_way_or_duplicates_are_invalid() {
        let mut s = snapshot();
        s.routes
            .insert("robot@10.0.0.8".into(), vec!["robot@100.96.0.2".into()]);
        let p = plan_with(&s, change("m1:robot", LOCAL, Policy::OneWay)).unwrap();
        assert!(p
            .consequences
            .contains(&"2 jump routes via lab-arm will be refused".to_string()));
        let mut file = s.file.clone();
        file.edges.push(EdgePolicy {
            from: "m1:robot".into(),
            to: LOCAL.into(),
            policy: Policy::OneWay,
            updated_at: 1,
        });
        assert!(file.validate().is_err());
        file.edges[0].policy = Policy::Key;
        assert!(file.validate().is_ok());
        file.edges.push(file.edges[0].clone());
        assert!(file.validate().is_err());
    }

    #[test]
    fn only_recorded_decisions_gate_key_setup() {
        let s = snapshot();
        // Directed today refuses jetson → laptop, but explicit pairing may promote it.
        assert!(s.key_allowed("m2:robot", LOCAL));
        let one = s.with_edges(s.changed(&change(LOCAL, "m1:robot", Policy::OneWay)));
        assert!(one.key_allowed(LOCAL, "m1:robot"));
        assert!(!one.key_allowed("m1:robot", LOCAL));
        let pwd = s.with_edges(s.changed(&change(LOCAL, "m1:robot", Policy::Password)));
        assert!(!pwd.key_allowed(LOCAL, "m1:robot"));
        assert!(pwd.key_allowed("m1:robot", LOCAL));
    }
    #[test]
    fn key_on_directed_device_keeps_reverse_migrated_and_pairable() {
        let s = snapshot();
        let p = plan_with(&s, change(LOCAL, "m2:robot", Policy::Key)).unwrap();
        assert_eq!(p.consequences, ["no access change for laptop and jetson"]);
        // No hidden reverse refusal: only the requested direction is recorded.
        assert_eq!(p.records.len(), 1);
        let next = s.with_edges(s.changed(&p.change));
        assert_eq!(
            next.access("m2:robot", LOCAL),
            (Access::Refused, Source::MigratedDirected)
        );
        assert!(next.key_allowed("m2:robot", LOCAL));
    }
    #[test]
    fn failed_install_is_replanned_on_rerun() {
        let s = snapshot();
        let pwd = s.with_edges(s.changed(&change(LOCAL, "m1:robot", Policy::Password)));
        let mut key = pwd.with_edges(pwd.changed(&change(LOCAL, "m1:robot", Policy::Key)));
        let install = Step::InstallKey {
            from: LOCAL.into(),
            to: "m1:robot".into(),
        };
        assert!(plan_with(&key, change(LOCAL, "m1:robot", Policy::Key))
            .unwrap()
            .steps
            .is_empty());
        key.file.enforcement.push(Enforcement {
            from: LOCAL.into(),
            to: "m1:robot".into(),
            state: "failed: SSH access unavailable".into(),
            ok: false,
            at: 900,
        });
        let retry = plan_with(&key, change(LOCAL, "m1:robot", Policy::Key)).unwrap();
        assert_eq!(retry.steps, vec![install]);
        assert_eq!(
            retry.consequences,
            ["cx will retry installing laptop's cx key on lab-arm; the last attempt failed"]
        );
    }
    #[test]
    fn enforcement_cap_drops_oldest_and_keeps_newest() {
        let mut records: Vec<Enforcement> = (0..2 * RECORD_LIMIT)
            .map(|i| Enforcement {
                from: format!("d{i}"),
                to: LOCAL.into(),
                state: "old".into(),
                ok: true,
                at: 1,
            })
            .collect();
        let result = StepResult {
            step: Step::RemoveKey {
                from: "new".into(),
                to: LOCAL.into(),
            },
            ok: true,
            state: "fresh".into(),
        };
        record_enforcement(&mut records, &[result], 2);
        assert_eq!(records.len(), 2 * RECORD_LIMIT);
        assert_eq!(records.last().unwrap().state, "fresh");
        assert!(!records.iter().any(|e| e.from == "d0"));
    }
    #[test]
    fn policy_parsing_and_status_strings() {
        use clap::ValueEnum;
        assert_eq!(Policy::from_str("one-way", false).unwrap(), Policy::OneWay);
        assert_eq!(
            Policy::from_str("password", false).unwrap(),
            Policy::Password
        );
        assert!(Policy::from_str("auto", false).is_err());
        assert!(Policy::from_str("PASSWORD", false).is_err());
        let mut s = snapshot();
        s.file.evidence.push(Evidence {
            from: LOCAL.into(),
            to: "m1:robot".into(),
            last_ok: Some(997),
            last_failed: Some(400),
            detail: None,
        });
        s.file.enforcement.push(Enforcement {
            from: "m1:robot".into(),
            to: LOCAL.into(),
            state: "cx key absent".into(),
            ok: true,
            at: 880,
        });
        let v = direction_view(&s, LOCAL, "m1:robot");
        assert_eq!(v.observed, "authenticated 3s ago; auth failed 10m ago");
        let v = direction_view(&s, "m1:robot", LOCAL);
        assert_eq!(v.enforced, "cx key absent (2m ago)");
        assert_eq!(bounded("x".repeat(300)).chars().count(), DETAIL_LIMIT + 1);
        assert_eq!(age(100_000, 0), "1d ago");
    }

    /// EP-3/EP-4: UI attach/terminal/run paths build their ssh through interactive_ssh, which refuses a refused
    /// edge before any ssh runs. Runs in a child with its own HOME and state dir; real state is never read.
    #[test]
    fn interactive_ssh_honours_refused_edges() {
        let home = tempfile::tempdir().unwrap();
        let state = home.path().join("state");
        if !crate::mesh::in_child(
            "edges::tests::interactive_ssh_honours_refused_edges",
            &[("HOME", home.path()), ("XDG_STATE_HOME", &state)],
        ) {
            return;
        }
        let robot = Device {
            id: "m9:robot".into(),
            name: "robot".into(),
            target: Some("robot@192.0.2.9".into()),
            account: "robot".into(),
            host: "robot".into(),
            status: "unknown".into(),
            observed_at: 0,
        };
        store::save_devices(&[store::local_device(), robot.clone()]).unwrap();
        let edge = |policy: &str| {
            let file = serde_json::json!({"version": 1, "edges": [
                {"from": LOCAL, "to": robot.id, "policy": policy, "updated_at": 1}]});
            store::write_state("edges.json", &file, 64 * 1024).unwrap();
        };
        edge("none");
        let refused = store::interactive_ssh(&robot).unwrap_err().to_string();
        assert!(refused.contains("edge policy refuses"), "{refused}");
        assert!(crate::sessions::login_terminal(&robot)
            .unwrap_err()
            .to_string()
            .contains("edge policy refuses"));
        // A route through a refused hop is refused as well.
        let hop = Device {
            id: "m8:robot".into(),
            name: "hop".into(),
            target: Some("robot@192.0.2.8".into()),
            ..robot.clone()
        };
        store::save_devices(&[store::local_device(), robot.clone(), hop.clone()]).unwrap();
        store::set_route("robot@192.0.2.9", &["robot@192.0.2.8".into()]).unwrap();
        let file = serde_json::json!({"version": 1, "edges": [
            {"from": LOCAL, "to": robot.id, "policy": "key", "updated_at": 1},
            {"from": LOCAL, "to": hop.id, "policy": "none", "updated_at": 1}]});
        store::write_state("edges.json", &file, 64 * 1024).unwrap();
        assert!(store::interactive_ssh(&robot).is_err());
        assert!(store::interactive_ssh(&hop).is_err());
        store::set_route("robot@192.0.2.9", &[]).unwrap();
        edge("key");
        let allowed = store::interactive_ssh(&robot).unwrap();
        assert_eq!(allowed.get_program(), "ssh");
        assert!(
            store::interactive_ssh(&store::local_device()).is_err(),
            "local has no SSH target"
        );
    }
}
