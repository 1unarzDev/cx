//! Linux NetworkManager sharing transactions. No global route/firewall/forwarding writes.
//! Integrator must hold a per-host maintenance lock and schedule `expire` host-side.
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::{fs, net::Ipv4Addr, path::Path, process::Command};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Output {
    pub interface: String,
    pub nm_device_path: String,
    pub isolated: bool,
    pub ethernet: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Observation {
    pub management: Option<String>,
    pub effective_upstream: String,
    pub upstream_ready: bool,
    pub observed_at: u64,
    pub occupied_subnets: Vec<String>,
    pub outputs: Vec<Output>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Source {
    Automatic,
    Pinned(String),
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Profile {
    pub uuid: String,
    pub name: String,
    pub interface: String,
    pub address: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Plan {
    pub id: String,
    pub effective_upstream: String,
    pub device_paths: Vec<String>,
    pub profiles: Vec<Profile>,
    pub timeout: u32,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum State {
    Applying,
    AwaitingConfirmation,
    Enabled,
    Degraded,
    Off,
    RolledBack,
    RollbackFailed,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Ledger {
    pub plan: Plan,
    pub state: State,
    pub checkpoint: Option<String>,
    pub expires_at: u64,
    pub note: Option<String>,
}
fn iface(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 15
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-' || b == b'.')
        && s != "lo"
}
fn object(s: &str, kind: &str) -> bool {
    s.strip_prefix(&format!("/org/freedesktop/NetworkManager/{kind}/"))
        .is_some_and(|v| !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit()))
}
fn subnet(s: &str) -> Result<(u32, u32)> {
    let (ip, bits) = s.split_once('/').context("subnet needs prefix")?;
    let bits: u32 = bits.parse()?;
    if bits > 32 {
        bail!("invalid IPv4 prefix");
    }
    let mask = if bits == 0 {
        0
    } else {
        u32::MAX << (32 - bits)
    };
    let ip = u32::from(ip.parse::<Ipv4Addr>()?);
    Ok((ip & mask, (ip & mask) | !mask))
}
fn overlap(a: (u32, u32), b: (u32, u32)) -> bool {
    a.0 <= b.1 && b.0 <= a.1
}
fn uuid(id: &str, index: usize) -> String {
    use sha2::{Digest, Sha256};
    let hash = format!(
        "{:x}",
        Sha256::digest(format!("cx-share:{id}:{index}").as_bytes())
    );
    format!(
        "{}-{}-4{}-8{}-{}",
        &hash[..8],
        &hash[8..12],
        &hash[13..16],
        &hash[17..20],
        &hash[20..32]
    )
}
pub fn preview(id: &str, source: Source, obs: &Observation, now: u64) -> Result<Plan> {
    if id.is_empty() || id.len() > 40 || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        bail!("invalid sharing transaction id");
    }
    if let Source::Pinned(_) = source {
        bail!("pinned uplink unsupported: NetworkManager shared mode follows host routing; verified policy-routing backend required");
    }
    if !iface(&obs.effective_upstream)
        || !obs.upstream_ready
        || now.saturating_sub(obs.observed_at) > 30
        || obs.observed_at > now
    {
        bail!("refresh upstream evidence before sharing");
    }
    if obs.outputs.is_empty() || obs.outputs.len() > 8 {
        bail!("choose 1–8 isolated wired outputs");
    }
    let mut occupied = obs
        .occupied_subnets
        .iter()
        .map(|s| subnet(s))
        .collect::<Result<Vec<_>>>()?;
    let mut profiles = vec![];
    let mut device_paths = vec![];
    for (i, o) in obs.outputs.iter().enumerate() {
        if !iface(&o.interface) || !object(&o.nm_device_path, "Devices") {
            bail!("invalid output identity");
        }
        if o.interface == obs.effective_upstream || obs.management.as_ref() == Some(&o.interface) {
            bail!("management/upstream interface cannot be an output");
        }
        if !o.isolated || !o.ethernet {
            bail!("only explicitly isolated wired outputs supported; established LAN/static robots and Wi-Fi need additional policy");
        }
        if profiles
            .iter()
            .any(|p: &Profile| p.interface == o.interface)
            || device_paths.contains(&o.nm_device_path)
        {
            bail!("duplicate output");
        }
        let address = (1..=254)
            .map(|n| format!("10.42.{n}.1/24"))
            .find(|s| !occupied.iter().any(|v| overlap(subnet(s).unwrap(), *v)))
            .context("no non-overlapping sharing subnet available")?;
        occupied.push(subnet(&address)?);
        device_paths.push(o.nm_device_path.clone());
        profiles.push(Profile {
            uuid: uuid(id, i),
            name: format!("cx-share-{id}-{}", o.interface),
            interface: o.interface.clone(),
            address,
        });
    }
    Ok(Plan {
        id: id.into(),
        effective_upstream: obs.effective_upstream.clone(),
        device_paths,
        profiles,
        timeout: 60,
    })
}
/// Complete allowlist; no user-provided property names, gateways or firewall commands.
pub fn profile_args(p: &Profile) -> Vec<String> {
    [
        "--wait",
        "15",
        "connection",
        "add",
        "type",
        "ethernet",
        "ifname",
        &p.interface,
        "con-name",
        &p.name,
        "connection.uuid",
        &p.uuid,
        "connection.autoconnect",
        "no",
        "ipv4.method",
        "shared",
        "ipv4.addresses",
        &p.address,
        "ipv4.never-default",
        "yes",
        "ipv6.method",
        "disabled",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}
pub trait Backend {
    fn checkpoint(&mut self, paths: &[String], timeout: u32) -> Result<String>;
    fn create(&mut self, p: &Profile) -> Result<()>;
    fn up(&mut self, p: &Profile) -> Result<()>;
    /// Verify downstream DHCP/DNS and egress externally; NM activation alone is insufficient.
    fn verified(&mut self, plan: &Plan) -> Result<bool>;
    fn confirm(&mut self, checkpoint: &str) -> Result<()>;
    fn rollback(&mut self, checkpoint: &str) -> Result<()>;
    fn remove(&mut self, p: &Profile) -> Result<()>;
}
fn validate_profile(p: &Profile) -> Result<()> {
    if !iface(&p.interface)
        || !p.name.starts_with("cx-share-")
        || p.name.len() > 100
        || !p
            .name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.')
    {
        bail!("invalid owned sharing profile");
    }
    if p.uuid.len() != 36
        || p.uuid.bytes().enumerate().any(|(i, b)| {
            if [8, 13, 18, 23].contains(&i) {
                b != b'-'
            } else {
                !b.is_ascii_hexdigit()
            }
        })
    {
        bail!("invalid profile UUID");
    }
    let range = subnet(&p.address)?;
    let allowed = subnet("10.42.0.0/16")?;
    if !p.address.ends_with(".1/24") || range.0 < allowed.0 || range.1 > allowed.1 {
        bail!("sharing address outside owned allocation policy");
    }
    Ok(())
}
fn save(path: &Path, ledger: &Ledger) -> Result<()> {
    use std::{io::Write, os::unix::fs::OpenOptionsExt};
    let parent = path.parent().context("ledger parent absent")?;
    use std::os::unix::fs::MetadataExt;
    let meta = fs::symlink_metadata(parent)?;
    if !meta.is_dir() || meta.mode() & 0o077 != 0 {
        bail!("ledger directory must already exist with private permissions");
    }
    let tmp = path.with_extension("tmp");
    let mut f = fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .mode(0o600)
        .open(&tmp)?;
    f.write_all(&serde_json::to_vec(ledger)?)?;
    f.sync_all()?;
    fs::rename(tmp, path)?;
    fs::File::open(parent)?.sync_all()?;
    Ok(())
}
pub fn begin(backend: &mut impl Backend, plan: Plan, path: &Path, now: u64) -> Result<Ledger> {
    if path.exists() {
        bail!("transaction already recorded; reconcile instead of repeating");
    }
    if plan.profiles.is_empty()
        || plan.profiles.len() > 8
        || plan.profiles.len() != plan.device_paths.len()
        || plan.timeout != 60
        || !iface(&plan.effective_upstream)
        || plan.device_paths.iter().any(|p| !object(p, "Devices"))
    {
        bail!("invalid transaction scope");
    }
    for p in &plan.profiles {
        validate_profile(p)?;
    }
    let mut ledger = Ledger {
        expires_at: now + u64::from(plan.timeout),
        plan,
        state: State::Applying,
        checkpoint: None,
        note: None,
    };
    // Persist the exact owned UUIDs before creating resources; lost responses cannot orphan unnamed profiles.
    save(path, &ledger)?;
    let checkpoint = backend.checkpoint(&ledger.plan.device_paths, ledger.plan.timeout)?;
    ledger.checkpoint = Some(checkpoint);
    save(path, &ledger)?;
    let apply = (|| -> Result<()> {
        for p in &ledger.plan.profiles {
            backend.create(p)?;
            backend.up(p)?;
        }
        Ok(())
    })();
    if let Err(e) = apply {
        ledger.note = Some(e.to_string());
        rollback(backend, &mut ledger, path)?;
        return Err(e);
    }
    ledger.state = State::AwaitingConfirmation;
    save(path, &ledger)?;
    Ok(ledger)
}
pub fn confirm(
    backend: &mut impl Backend,
    ledger: &mut Ledger,
    path: &Path,
    now: u64,
) -> Result<()> {
    if ledger.state != State::AwaitingConfirmation || now >= ledger.expires_at {
        bail!("sharing confirmation expired or not pending");
    }
    if !backend.verified(&ledger.plan)? {
        bail!("downstream DHCP/DNS/egress remains unverified; checkpoint will roll back");
    }
    backend.confirm(ledger.checkpoint.as_deref().context("missing checkpoint")?)?;
    ledger.state = State::Enabled;
    ledger.checkpoint = None;
    save(path, ledger)
}
pub fn rollback(backend: &mut impl Backend, ledger: &mut Ledger, path: &Path) -> Result<()> {
    let mut errors = vec![];
    if let Some(c) = ledger.checkpoint.as_ref() {
        match backend.rollback(c) {
            Err(e) => errors.push(e.to_string()),
            Ok(()) => {}
        }
    }
    for p in &ledger.plan.profiles {
        if let Err(e) = backend.remove(p) {
            errors.push(e.to_string());
        }
    }
    ledger.state = if errors.is_empty() {
        State::RolledBack
    } else {
        State::RollbackFailed
    };
    ledger.note = if errors.is_empty() {
        ledger.note.take()
    } else {
        Some(errors.join("; "))
    };
    save(path, ledger)?;
    if ledger.state == State::RollbackFailed {
        bail!(
            "owned-resource rollback incomplete: {}",
            ledger.note.as_deref().unwrap_or("unknown")
        );
    }
    Ok(())
}
pub fn disable(backend: &mut impl Backend, ledger: &mut Ledger, path: &Path) -> Result<()> {
    if matches!(
        ledger.state,
        State::Applying | State::AwaitingConfirmation | State::RollbackFailed
    ) {
        rollback(backend, ledger, path)?;
    } else {
        for p in &ledger.plan.profiles {
            backend.remove(p)?;
        }
    }
    ledger.state = State::Off;
    ledger.checkpoint = None;
    save(path, ledger)
}
pub fn observe_state(ledger: &mut Ledger, path: &Path, upstream_ready: bool) -> Result<()> {
    if matches!(ledger.state, State::Enabled | State::Degraded) {
        ledger.state = if upstream_ready {
            State::Enabled
        } else {
            State::Degraded
        };
        save(path, ledger)?;
    }
    Ok(())
}

/// Run in a host-side watchdog service independent of the viewer, under the same maintenance lock.
pub fn expire(backend: &mut impl Backend, path: &Path, now: u64) -> Result<bool> {
    let mut ledger: Ledger = serde_json::from_slice(&fs::read(path)?)?;
    if matches!(
        ledger.state,
        State::Applying | State::AwaitingConfirmation | State::RollbackFailed
    ) && now >= ledger.expires_at
    {
        rollback(backend, &mut ledger, path)?;
        return Ok(true);
    }
    Ok(false)
}

/// Narrow production backend. Activation remains unconfirmed until an integrator supplies
/// independently measured downstream DHCP/DNS/egress evidence. No root shell or sudo.
pub struct NetworkManager {
    pub downstream_verified: bool,
}
impl NetworkManager {
    fn run(program: &str, args: &[String]) -> Result<String> {
        let out = Command::new(program).args(args).output()?;
        if !out.status.success() {
            bail!(
                "NetworkManager operation rejected: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        Ok(String::from_utf8(out.stdout)?.trim().into())
    }
    fn nm(args: &[&str]) -> Result<String> {
        Self::run(
            "nmcli",
            &args.iter().map(|v| (*v).into()).collect::<Vec<_>>(),
        )
    }
    fn call(method: &str, sig: &str, args: Vec<String>) -> Result<String> {
        let mut fixed = vec![
            "--system".into(),
            "call".into(),
            "org.freedesktop.NetworkManager".into(),
            "/org/freedesktop/NetworkManager".into(),
            "org.freedesktop.NetworkManager".into(),
            method.into(),
            sig.into(),
        ];
        fixed.extend(args);
        Self::run("busctl", &fixed)
    }
    fn owned(p: &Profile) -> Result<bool> {
        let name = Self::nm(&["-g", "connection.id", "connection", "show", "uuid", &p.uuid])?;
        Ok(name == p.name && name.starts_with("cx-share-"))
    }
}
impl Backend for NetworkManager {
    fn checkpoint(&mut self, paths: &[String], timeout: u32) -> Result<String> {
        if paths.is_empty()
            || paths.iter().any(|p| !object(p, "Devices"))
            || !(15..=120).contains(&timeout)
        {
            bail!("invalid checkpoint scope");
        }
        let mut args = vec![paths.len().to_string()];
        args.extend(paths.to_vec());
        args.extend([timeout.to_string(), "0".into()]);
        let out = Self::call("CheckpointCreate", "aouu", args)?;
        let value = out
            .split('"')
            .nth(1)
            .context("invalid checkpoint response")?;
        if !object(value, "Checkpoint") {
            bail!("invalid checkpoint object");
        }
        Ok(value.into())
    }
    fn create(&mut self, p: &Profile) -> Result<()> {
        validate_profile(p)?;
        Self::run("nmcli", &profile_args(p))?;
        Ok(())
    }
    fn up(&mut self, p: &Profile) -> Result<()> {
        validate_profile(p)?;
        if !Self::owned(p)? {
            bail!("profile ownership mismatch");
        }
        Self::nm(&[
            "--wait",
            "15",
            "connection",
            "up",
            "uuid",
            &p.uuid,
            "ifname",
            &p.interface,
        ])?;
        Ok(())
    }
    fn verified(&mut self, plan: &Plan) -> Result<bool> {
        if !self.downstream_verified {
            return Ok(false);
        }
        let route = Self::run(
            "ip",
            &["-json", "route", "get", "1.1.1.1"]
                .iter()
                .map(|v| (*v).into())
                .collect::<Vec<_>>(),
        )?;
        let value: serde_json::Value = serde_json::from_str(&route)?;
        Ok(value
            .get(0)
            .and_then(|v| v.get("dev"))
            .and_then(|v| v.as_str())
            == Some(plan.effective_upstream.as_str()))
    }
    fn confirm(&mut self, c: &str) -> Result<()> {
        if !object(c, "Checkpoint") {
            bail!("invalid checkpoint");
        }
        Self::call("CheckpointDestroy", "o", vec![c.into()])?;
        Ok(())
    }
    fn rollback(&mut self, c: &str) -> Result<()> {
        if !object(c, "Checkpoint") {
            bail!("invalid checkpoint");
        }
        let out = Self::call("CheckpointRollback", "o", vec![c.into()])?;
        let fields = out.split_whitespace().collect::<Vec<_>>();
        if fields.get(0) != Some(&"a{su}") || fields.get(1).is_none() {
            bail!("invalid rollback response");
        }
        for pair in fields[2..].chunks(2) {
            if pair.len() != 2 || pair[1] != "0" {
                bail!("checkpoint rollback reported failure");
            }
        }
        Ok(())
    }
    fn remove(&mut self, p: &Profile) -> Result<()> {
        validate_profile(p)?;
        let listing = Self::nm(&["-g", "UUID", "connection", "show"])?;
        if !listing.lines().any(|v| v == p.uuid) {
            return Ok(());
        }
        if !Self::owned(p)? {
            bail!("refusing removal of unowned profile");
        }
        Self::nm(&["connection", "delete", "uuid", &p.uuid])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Default)]
    struct Fake {
        created: Vec<String>,
        removed: Vec<String>,
        rolled: usize,
        fail_second: bool,
        proof: bool,
    }
    impl Backend for Fake {
        fn checkpoint(&mut self, _: &[String], _: u32) -> Result<String> {
            Ok("/org/freedesktop/NetworkManager/Checkpoint/1".into())
        }
        fn create(&mut self, p: &Profile) -> Result<()> {
            if self.fail_second && self.created.len() == 1 {
                bail!("injected failure");
            }
            self.created.push(p.uuid.clone());
            Ok(())
        }
        fn up(&mut self, _: &Profile) -> Result<()> {
            Ok(())
        }
        fn verified(&mut self, _: &Plan) -> Result<bool> {
            Ok(self.proof)
        }
        fn confirm(&mut self, _: &str) -> Result<()> {
            Ok(())
        }
        fn rollback(&mut self, _: &str) -> Result<()> {
            self.rolled += 1;
            Ok(())
        }
        fn remove(&mut self, p: &Profile) -> Result<()> {
            self.removed.push(p.uuid.clone());
            Ok(())
        }
    }
    fn observation() -> Observation {
        Observation {
            management: Some("uplink1".into()),
            effective_upstream: "uplink1".into(),
            upstream_ready: true,
            observed_at: 100,
            occupied_subnets: vec!["10.42.1.0/24".into(), "192.168.0.0/24".into()],
            outputs: (1..=2)
                .map(|i| Output {
                    interface: format!("output{i}"),
                    nm_device_path: format!("/org/freedesktop/NetworkManager/Devices/{i}"),
                    isolated: true,
                    ethernet: true,
                })
                .collect(),
        }
    }
    #[test]
    fn preview_scope_and_rejections() {
        let obs = observation();
        let p = preview("fixture", Source::Automatic, &obs, 100).unwrap();
        assert_eq!(p.profiles[0].address, "10.42.2.1/24");
        assert_eq!(p.profiles[1].address, "10.42.3.1/24");
        for v in &p.profiles {
            let args = profile_args(v);
            assert!(!args
                .iter()
                .any(|s| s.contains("gateway") || s.contains("route") || s == "sudo"));
            assert!(args.windows(2).any(|p| p == ["ipv6.method", "disabled"]));
            assert!(args.windows(2).any(|p| p == ["ipv4.never-default", "yes"]));
        }
        assert!(preview("fixture", Source::Pinned("uplink2".into()), &obs, 100).is_err());
        assert!(preview("fixture", Source::Automatic, &obs, 131).is_err());
        let mut protected = obs.clone();
        protected.outputs[0].interface = "uplink1".into();
        assert!(preview("fixture", Source::Automatic, &protected, 100).is_err());
        protected = obs.clone();
        protected.outputs[0].isolated = false;
        assert!(preview("fixture", Source::Automatic, &protected, 100).is_err());
    }
    #[test]
    fn partial_failure_compensates_exact_owned_resources() {
        let dir = tempfile::tempdir().unwrap();
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let path = dir.path().join("ledger.json");
        let plan = preview("fixture", Source::Automatic, &observation(), 100).unwrap();
        let mut b = Fake {
            fail_second: true,
            ..Default::default()
        };
        assert!(begin(&mut b, plan.clone(), &path, 100).is_err());
        assert_eq!(b.rolled, 1);
        assert_eq!(
            b.removed,
            plan.profiles
                .iter()
                .map(|p| p.uuid.clone())
                .collect::<Vec<_>>()
        );
        let ledger: Ledger = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        assert_eq!(ledger.state, State::RolledBack);
    }
    #[test]
    fn expiration_and_confirmation_are_explicit() {
        let dir = tempfile::tempdir().unwrap();
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let path = dir.path().join("ledger.json");
        let plan = preview("fixture", Source::Automatic, &observation(), 100).unwrap();
        let mut b = Fake::default();
        let mut l = begin(&mut b, plan.clone(), &path, 100).unwrap();
        assert!(confirm(&mut b, &mut l, &path, 110).is_err());
        assert!(!expire(&mut b, &path, 159).unwrap());
        assert!(expire(&mut b, &path, 160).unwrap());
        assert!(begin(&mut b, plan.clone(), &path, 170).is_err());
        let second = dir.path().join("second.json");
        let mut l = begin(&mut b, plan, &second, 100).unwrap();
        b.proof = true;
        confirm(&mut b, &mut l, &second, 110).unwrap();
        assert_eq!(l.state, State::Enabled);
        assert!(!expire(&mut b, &second, 1000).unwrap());
    }
}
