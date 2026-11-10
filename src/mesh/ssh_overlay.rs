//! The ssh-overlay backend: the lightweight default mesh, built from what cx already has and able to
//! coexist with any VPN (it claims no address range). Members are the enrolled devices; core
//! devices pair automatically (peers.rs, peer_ssh.rs); reachability is the existing routes plus
//! ProxyJump through a reachable peer. New devices join with `cx mesh pair <code> <host>` (docs/mesh.md
//! has the protocol and its threat model):
//!
//! 1. `cx mesh invite --backend ssh-overlay` on the viewer makes an 11-symbol code (code.rs) and keeps
//!    only its SHA-256, which keys the pairing's HMACs. While an invite is pending the viewer authorizes the
//!    *door key*, a key pair every cx carries (not a secret), as
//!    `restrict,command="<cx> mesh pair --redeem",expiry-time=...` tagged `cx-join-door`.
//! 2. `cx mesh pair <code> <host>` on the joiner reads the viewer's host key (ssh-keyscan), pins it for the
//!    connection, and through the door sends its own keys with HMAC(code key, "join", keys and that host key).
//! 3. The forced command finds the pending invite whose key verifies the HMAC (wrong guesses are rate-limited
//!    and capped), burns it (single use), checks the joiner saw its real host key, installs the joiner's key
//!    as `cx-managed-peer`, enrolls it as core, and answers with its own keys and HMAC(code key, "accept", ...).
//! 4. The joiner verifies that answer (only a holder of the code can make it), then pins and authorizes the
//!    viewer the same way.
use super::{code, iso, AuditEntry, Invite, IpNet, Member, MeshBackend, Plan, Status};
use crate::{model::Device, peer_ssh, store, transport};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    fs,
    io::{BufReader, Read, Write},
    net::{IpAddr, SocketAddr},
    os::unix::fs::{DirBuilderExt, OpenOptionsExt},
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

pub const ID: &str = "ssh-overlay";
/// 2: a short code with HMAC confirmations through the door key (1 carried an invite key in a long code).
const PROTOCOL: u32 = 2;
const MAX_TTL_HOURS: f64 = 24.0 * 7.0;
const DEFAULT_TTL_HOURS: f64 = 24.0;
const MAX_PENDING: usize = 16;
/// Wrong codes: at most this many per window are checked, the rest refused unchecked...
const FAILURES_PER_WINDOW: usize = 3;
const FAILURE_WINDOW_SECS: u64 = 60;
/// ...and this many in all cancel every pending invite.
const MAX_FAILURES: usize = 20;
/// How long the forced command waits for the joiner's request.
const REQUEST_TIMEOUT_SECS: u32 = 30;

/// The door key: the same key pair in every cx, made once by `ssh-keygen -t ed25519`. It is deliberately public. It
/// only reaches the forced redeem command, and only while an invite is pending; the code is what authenticates.
const DOOR_KEY: &str = "-----BEGIN OPENSSH PRIVATE KEY-----
b3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAABAAAAMwAAAAtzc2gtZW
QyNTUxOQAAACDsBFZ4Q0KIf/UObmzcCAQNYE0sDVcdckpSsjeK0AdmkwAAAJDDIbWkwyG1
pAAAAAtzc2gtZWQyNTUxOQAAACDsBFZ4Q0KIf/UObmzcCAQNYE0sDVcdckpSsjeK0Admkw
AAAEAYw9XBWpxOaw//1XztY6zwkqx5VEMeVqbZrauicmzjIewEVnhDQoh/9Q5ubNwIBA1g
TSwNVx1ySlKyN4rQB2aTAAAADGN4LXBhaXItZG9vcgE=
-----END OPENSSH PRIVATE KEY-----
";
const DOOR_PUBLIC: &str =
    "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOwEVnhDQoh/9Q5ubNwIBA1gTSwNVx1ySlKyN4rQB2aT";
const DOOR_TAG: &str = "cx-join-door";

pub struct SshOverlay;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct InviteRecord {
    created: u64,
    expires: u64,
    #[serde(default)]
    name: Option<String>,
    state: String, // pending | redeemed | cancelled | expired
    /// Hex of code::key(code): the code itself is never stored.
    #[serde(default)]
    key: String,
    host_key: String,
    #[serde(default)]
    redeemed_by: Option<String>,
    #[serde(default)]
    redeemed_at: Option<u64>,
}

#[derive(Default, Serialize, Deserialize)]
struct Invites {
    invites: BTreeMap<String, InviteRecord>,
    /// When recent redemptions failed the code check (bounded by MAX_FAILURES).
    #[serde(default)]
    failures: Vec<u64>,
}

fn dir() -> PathBuf {
    store::state_dir().join("mesh").join(ID)
}

fn ensure_dir() -> Result<PathBuf> {
    store::ensure()?;
    let d = dir();
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&d)?;
    use std::os::unix::fs::MetadataExt;
    let m = fs::symlink_metadata(&d)?;
    anyhow::ensure!(
        m.is_dir() && m.uid() == unsafe { libc::geteuid() } && m.mode() & 0o077 == 0,
        "unsafe mesh state directory"
    );
    Ok(d)
}

struct Lock(#[allow(dead_code)] fs::File); // held for its flock
fn lock() -> Result<Lock> {
    use std::os::fd::AsRawFd;
    let f = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(ensure_dir()?.join("lock"))?;
    anyhow::ensure!(
        unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX) } == 0,
        "mesh invite lock unavailable"
    );
    Ok(Lock(f))
}

fn read_invites(_l: &Lock) -> Result<Invites> {
    load_invites()
}

/// The invite state without the lock: it is only ever replaced whole by rename, so a reader sees
/// one complete version (enough for a count; changes go through [`read_invites`]).
fn load_invites() -> Result<Invites> {
    let p = dir().join("invites.json");
    match fs::read(&p) {
        Ok(b) => {
            anyhow::ensure!(b.len() <= 256 * 1024, "invite state too large");
            Ok(serde_json::from_slice(&b)?)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Invites::default()),
        Err(e) => Err(e.into()),
    }
}

fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    let tmp = path.with_extension(format!("{}.tmp", std::process::id()));
    let _ = fs::remove_file(&tmp);
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(&tmp)?;
    let result = (|| -> Result<()> {
        f.write_all(bytes)?;
        f.sync_all()?;
        fs::rename(&tmp, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

/// A private scratch directory under the mesh state dir, removed (with what is in it) on drop.
struct Work(PathBuf);
impl Work {
    fn new(label: &str) -> Result<Work> {
        let path = ensure_dir()?.join(format!("{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::DirBuilder::new().mode(0o700).create(&path)?;
        Ok(Work(path))
    }
}
impl Drop for Work {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn write_invites(_l: &Lock, inv: &Invites) -> Result<()> {
    write_private(
        &dir().join("invites.json"),
        &serde_json::to_vec_pretty(inv)?,
    )
}

/// Append to audit.jsonl (kept to its last 500..1000 entries). The caller holds the lock, so
/// concurrent redeems and invites (separate processes) never lose each other's lines.
fn audit(_l: &Lock, action: &str, detail: &str) {
    let line =
        json!({"at": iso(transport::now()), "actor": "local", "action": action, "detail": detail});
    let path = dir().join("audit.jsonl");
    let old = fs::read_to_string(&path).unwrap_or_default();
    let mut lines: Vec<&str> = old.lines().collect();
    if lines.len() >= 1000 {
        lines.drain(..lines.len() - 499);
    }
    let mut text = lines.join("\n");
    if !text.is_empty() {
        text.push('\n');
    }
    text.push_str(&line.to_string());
    text.push('\n');
    let _ = write_private(&path, text.as_bytes());
}

/// The `cx` the forced command runs: this executable, when its path is plain enough to quote.
fn exe() -> Result<String> {
    let p = std::env::current_exe()?;
    let s = p.to_str().context("cx path is not UTF-8")?.to_owned();
    anyhow::ensure!(
        s.starts_with('/')
            && s.len() < 512
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"/._-+".contains(&b)),
        "cx is installed at a path a forced command cannot quote safely"
    );
    Ok(s)
}

fn door_line(expires: u64, exe: &str) -> String {
    // OpenSSH expiry-time: YYYYMMDDHHMM with Z for UTC (sshd(8) AUTHORIZED_KEYS FILE FORMAT).
    let t: String = iso(expires)
        .chars()
        .filter(|c| c.is_ascii_digit())
        .take(12)
        .collect();
    format!(
        "restrict,command=\"{exe} mesh pair --redeem\",expiry-time=\"{t}Z\" {DOOR_PUBLIC} {DOOR_TAG}"
    )
}

/// A door line (or a protocol-1 invite line, swept the same way).
fn is_door_line(line: &str) -> bool {
    line.starts_with("restrict,command=")
        && (line.ends_with(&format!(" {DOOR_TAG}")) || line.contains(" cx-join-invite:"))
}

/// Authorize the door until the last pending invite expires, or remove it when none is pending.
fn refresh_door(inv: &Invites) -> Result<()> {
    let want = inv
        .invites
        .values()
        .filter(|r| r.state == "pending")
        .map(|r| r.expires)
        .max()
        .map(|e| exe().map(|x| door_line(e, &x)))
        .transpose()?;
    peer_ssh::remove_authorized(|l| is_door_line(l) && Some(l) != want.as_deref())?;
    if let Some(line) = want {
        peer_ssh::authorize_line(&line)?;
    }
    Ok(())
}

/// Addresses a joiner may reach this host at, best first: private LAN IPv4, overlay IPv4
/// (CGNAT), then global and ULA IPv6. Loopback and link-local are never offered.
fn default_addresses() -> Result<Vec<SocketAddr>> {
    let mut ips = crate::peers::local_addresses()?;
    let rank = |ip: &IpAddr| match ip {
        IpAddr::V4(v) if v.is_private() => 0,
        IpAddr::V4(v)
            if super::ranges::net(super::ranges::TAILSCALE_V4).contains(&IpAddr::V4(*v)) =>
        {
            1
        }
        IpAddr::V4(_) => 2,
        IpAddr::V6(v) if v.segments()[0] & 0xfe00 == 0xfc00 => 4,
        IpAddr::V6(_) => 3,
    };
    ips.sort_by_key(|ip| (rank(ip), *ip));
    ips.dedup();
    Ok(ips
        .into_iter()
        .take(3)
        .map(|ip| SocketAddr::new(ip, 22))
        .collect())
}

#[derive(Clone, Debug, Default)]
pub struct InviteOptions {
    pub ttl_hours: Option<f64>,
    pub device_name: Option<String>,
    /// Addresses to suggest to the joiner (`ip`, or `ip:22` / `[v6]:22`); default: this host's own.
    pub addresses: Vec<String>,
    /// The host key sshd presents, when it does not use /etc/ssh/ssh_host_ed25519_key.
    pub host_key_file: Option<PathBuf>,
}

fn parse_address(s: &str) -> Result<SocketAddr> {
    let a = match s.parse::<SocketAddr>() {
        Ok(a) => a,
        Err(_) => SocketAddr::new(
            s.trim_start_matches('[')
                .trim_end_matches(']')
                .parse()
                .with_context(|| format!("invalid address {s:?} (an IP)"))?,
            22,
        ),
    };
    check_port(a.port())?;
    Ok(a)
}

/// cx device targets carry no port, so a pairing is only usable over port 22: anything else would
/// burn the invite for a one-sided pairing. The loopback fixture (debug builds) is the exception.
fn check_port(port: u16) -> Result<()> {
    anyhow::ensure!(
        port == 22 || fixture_machine_id().is_some(),
        "port {port}: cx pairs over SSH port 22 only (device targets carry no port)"
    );
    Ok(())
}

pub(super) fn valid_name(n: &str) -> bool {
    !n.is_empty()
        && n.len() <= 63
        && !n.starts_with('-')
        && n.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
}

/// What the joiner types after the code to reach `addr` as `account`.
fn host_arg(account: &str, addr: &SocketAddr) -> String {
    if addr.port() == 22 {
        format!("{account}@{}", addr.ip())
    } else {
        format!("{account}@{addr}")
    }
}

fn invite_id(key: &[u8; 32]) -> String {
    use sha2::{Digest, Sha256};
    let d = Sha256::new()
        .chain_update(b"cx-pair-v1|id|")
        .chain_update(key)
        .finalize();
    d[..8].iter().map(|b| format!("{b:02x}")).collect()
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|b| format!("{b:02x}")).collect()
}

fn record_key(r: &InviteRecord) -> Option<[u8; 32]> {
    if r.key.len() != 64 {
        return None;
    }
    let mut k = [0u8; 32];
    for (i, out) in k.iter_mut().enumerate() {
        *out = u8::from_str_radix(r.key.get(2 * i..2 * i + 2)?, 16).ok()?;
    }
    Some(k)
}

/// Make an invite: a fresh code, its key recorded, the door open until it expires.
pub fn create_invite(opts: &InviteOptions) -> Result<Invite> {
    let ttl = opts.ttl_hours.unwrap_or(DEFAULT_TTL_HOURS);
    anyhow::ensure!(
        ttl > 0.0 && ttl <= MAX_TTL_HOURS,
        "--hours must be within (0, {MAX_TTL_HOURS}]"
    );
    if let Some(n) = &opts.device_name {
        anyhow::ensure!(valid_name(n), "invalid device name");
    }
    let addresses = if opts.addresses.is_empty() {
        default_addresses().unwrap_or_default()
    } else {
        opts.addresses
            .iter()
            .map(|a| parse_address(a))
            .collect::<Result<_>>()?
    };
    let host_key = match &opts.host_key_file {
        Some(p) => peer_ssh::public_key(&fs::read_to_string(p).context("cannot read --host-key")?)?,
        None => peer_ssh::host_public_key()
            .context("this host has no readable SSH Ed25519 host key; pass --host-key")?,
    };
    let account = store::local_device().account;
    let now = transport::now();
    // Whole minutes, as sshd's expiry-time has them.
    let expires = (now + (ttl * 3600.0) as u64).div_ceil(60) * 60;
    let code = code::new_code()?;
    let key = code::key(&code);
    let id = invite_id(&key);
    exe()?;
    {
        let l = lock()?;
        let mut inv = read_invites(&l)?;
        expire(&mut inv, now);
        let pending = inv
            .invites
            .values()
            .filter(|r| r.state == "pending")
            .count();
        anyhow::ensure!(
            pending < MAX_PENDING,
            "too many pending invites; cancel them with `cx mesh teardown --backend ssh-overlay --yes`"
        );
        if pending == 0 {
            // Wrong guesses count against the invites pending when they were made.
            inv.failures.clear();
        }
        inv.invites.insert(
            id.clone(),
            InviteRecord {
                created: now,
                expires,
                name: opts.device_name.clone(),
                state: "pending".into(),
                key: hex(&key),
                host_key,
                ..Default::default()
            },
        );
        write_invites(&l, &inv)?;
        refresh_door(&inv)?;
        audit(&l, "invite", &format!("{id} until {}", iso(expires)));
    }
    let hosts: Vec<String> = addresses.iter().map(|a| host_arg(&account, a)).collect();
    let shown = code::display(&code);
    let host = hosts
        .first()
        .cloned()
        .unwrap_or_else(|| format!("{account}@<this host>"));
    Ok(Invite {
        code: shown.clone(),
        url: None,
        one_liner_sh: format!("cx mesh pair {shown} {host}"),
        one_liner_ps: None,
        expires: iso(expires),
        uses: 1,
        backend: ID.into(),
        hosts,
    })
}

fn expire(inv: &mut Invites, now: u64) {
    for r in inv.invites.values_mut() {
        if r.state == "pending" && now >= r.expires {
            r.state = "expired".into();
        }
    }
    // Keep history bounded.
    if inv.invites.len() > 64 {
        let mut old: Vec<(u64, String)> = inv
            .invites
            .iter()
            .filter(|(_, r)| r.state != "pending")
            .map(|(k, r)| (r.created, k.clone()))
            .collect();
        old.sort();
        for (_, k) in old.into_iter().take(inv.invites.len() - 64) {
            inv.invites.remove(&k);
        }
    }
}

/// Too many wrong codes just now: refuse without checking.
fn rate_limited(inv: &Invites, now: u64) -> bool {
    inv.failures
        .iter()
        .filter(|&&t| now < t + FAILURE_WINDOW_SECS)
        .count()
        >= FAILURES_PER_WINDOW
}

/// Count a wrong code. The MAX_FAILURES-th cancels every pending invite; returns whether it did.
fn note_failure(inv: &mut Invites, now: u64) -> bool {
    inv.failures.push(now);
    if inv.failures.len() < MAX_FAILURES {
        return false;
    }
    for r in inv.invites.values_mut().filter(|r| r.state == "pending") {
        r.state = "cancelled".into();
    }
    inv.failures.clear();
    true
}

#[derive(Clone, Serialize, Deserialize)]
struct JoinRequest {
    version: u32,
    name: String,
    host: String,
    account: String,
    machine_id: String,
    client_key: String,
    host_key: String,
    /// The viewer host key the joiner reached (and pinned for this connection).
    viewer_host_key: String,
    /// code::mac(key, "join", fields()).
    mac: String,
}

impl JoinRequest {
    fn fields(&self) -> [&[u8]; 7] {
        [
            self.name.as_bytes(),
            self.host.as_bytes(),
            self.account.as_bytes(),
            self.machine_id.as_bytes(),
            self.client_key.as_bytes(),
            self.host_key.as_bytes(),
            self.viewer_host_key.as_bytes(),
        ]
    }
}

#[derive(Default, Serialize, Deserialize)]
struct JoinResponse {
    version: u32,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    viewer: Option<Peer>,
    #[serde(default)]
    host_key: Option<String>,
    /// code::mac(key, "accept", accept_fields(..)).
    #[serde(default)]
    mac: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
struct Peer {
    id: String,
    name: String,
    host: String,
    account: String,
    client_key: String,
}

/// What the viewer's answer authenticates: the request it answers (by its MAC) and everything it sends back.
fn accept_fields<'a>(request_mac: &'a str, viewer: &'a Peer, host_key: &'a str) -> [&'a [u8]; 7] {
    [
        request_mac.as_bytes(),
        viewer.id.as_bytes(),
        viewer.name.as_bytes(),
        viewer.host.as_bytes(),
        viewer.account.as_bytes(),
        viewer.client_key.as_bytes(),
        host_key.as_bytes(),
    ]
}

/// The loopback fixture (tests/mesh_ssh_overlay.py) runs two identities on one machine, and its
/// sshd on a free port. Debug builds only, like CX_ADOPT_REPTYR.
fn fixture_machine_id() -> Option<String> {
    #[cfg(debug_assertions)]
    if let Some(m) = std::env::var("CX_JOIN_MACHINE_ID")
        .ok()
        .filter(|s| !s.is_empty())
    {
        return Some(m);
    }
    None
}

fn machine_id() -> String {
    fixture_machine_id().unwrap_or_else(|| {
        fs::read_to_string("/etc/machine-id")
            .unwrap_or_default()
            .trim()
            .to_owned()
    })
}

fn valid_machine_id(m: &str) -> bool {
    !m.is_empty()
        && m.len() <= 64
        && m.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b))
}

/// The identity a join request carries, as the viewer checks it. The joiner checks its own first,
/// so a request the viewer would refuse never burns an invite.
fn check_identity(name: &str, host: &str, account: &str, machine_id: &str) -> Result<()> {
    anyhow::ensure!(valid_name(name), "invalid device name; pass --name");
    anyhow::ensure!(valid_name(host), "invalid host name");
    anyhow::ensure!(
        valid_machine_id(machine_id),
        "invalid machine id (from /etc/machine-id)"
    );
    anyhow::ensure!(
        transport::valid_target(&format!("{account}@127.0.0.1")),
        "invalid account name"
    );
    Ok(())
}

fn bracket(ip: IpAddr) -> String {
    match ip {
        IpAddr::V6(v) => format!("[{v}]"),
        IpAddr::V4(v) => v.to_string(),
    }
}

/// Refuse a join for an identity this device already enrolled. An invite only ever adds a new
/// device: it never retargets an existing record or changes its access (re-pairing means
/// revoking or removing the old record first).
fn ensure_new(devices: &[Device], id: &str) -> Result<()> {
    if let Some(d) = devices.iter().find(|d| d.id == id) {
        bail!(
            "{} is already enrolled here; an invite only adds new devices (revoke or remove it first to re-pair)",
            crate::files::display(&d.name)
        );
    }
    Ok(())
}

/// Insert a newly joined device as core. An identity already enrolled is refused (see
/// [`ensure_new`]), as is an endpoint enrolled for another identity; a name another device
/// already uses (the local hostname included) gets a `-2`, `-3` suffix. Returns the name the
/// device got.
fn enroll_joined(peer: &Device) -> Result<String> {
    let _guard = store::device_guard()?;
    let mut devices = store::devices()?;
    ensure_new(&devices, &peer.id)?;
    anyhow::ensure!(
        !devices.iter().any(|d| d.target == peer.target),
        "that address is already enrolled for another device"
    );
    let taken = |n: &str| devices.iter().any(|d| d.name == n);
    let mut name = peer.name.clone();
    let mut n = 2;
    while taken(&name) {
        anyhow::ensure!(n < 100, "no free device name");
        name = format!("{}-{n}", peer.name);
        n += 1;
    }
    anyhow::ensure!(devices.len() < 128, "device enrollment limit reached");
    let mut peer = peer.clone();
    peer.name = name.clone();
    devices.push(peer.clone());
    store::save_devices(&devices)?;
    store::set_access(&peer, store::AccessMode::Core)?;
    Ok(name)
}

/// Pin, authorize and enroll a new peer; the pins are undone if enrollment is refused (a race
/// with another enrollment of the same identity or endpoint).
fn pair_new(peer: &Device, host_key: &str, client_key: &str) -> Result<String> {
    ensure_new(&store::devices()?, &peer.id)?;
    store::allow_explicit_enrollment(&peer.id)?;
    // A new core device: no edge records yet, so key access both ways (the edges.rs core default).
    peer_ssh::configure(peer, host_key, client_key, true)?;
    enroll_joined(peer).inspect_err(|_| {
        if !store::devices().is_ok_and(|d| d.iter().any(|d| d.id == peer.id)) {
            let _ = peer_ssh::unpin(&peer.id);
        }
    })
}

/// The forced command (`cx mesh pair --redeem`), on the viewer, under sshd: stdin/stdout carry
/// one request and one response frame. Always answers with a frame, even on refusal.
pub fn redeem() -> Result<()> {
    let stdin = std::io::stdin();
    let mut input = BufReader::new(stdin.lock().take(64 * 1024));
    // Anyone may reach this through the door key: a peer that never sends its request is cut off.
    unsafe { libc::alarm(REQUEST_TIMEOUT_SECS) };
    let request = transport::read_frame::<JoinRequest>(&mut input);
    unsafe { libc::alarm(0) };
    let response = match request.and_then(redeem_request) {
        Ok(r) => r,
        Err(e) => {
            if let Ok(l) = lock() {
                audit(
                    &l,
                    "join-refused",
                    &crate::files::display(&format!("{e:#}")),
                );
            }
            JoinResponse {
                version: PROTOCOL,
                error: Some(format!("{e:#}")),
                ..Default::default()
            }
        }
    };
    let mut out = std::io::stdout().lock();
    transport::frame(&mut out, &response)?;
    out.flush()?;
    Ok(())
}

fn redeem_request(req: JoinRequest) -> Result<JoinResponse> {
    let now = transport::now();
    anyhow::ensure!(
        req.version == PROTOCOL,
        "unsupported join protocol {} (this viewer speaks {PROTOCOL}; update cx on both ends)",
        req.version
    );
    // Find the invite whose key verifies the request, and burn it first: whatever happens next, it
    // cannot be used again.
    let (id, record, key) = {
        let l = lock()?;
        let mut inv = read_invites(&l)?;
        expire(&mut inv, now);
        if rate_limited(&inv, now) {
            write_invites(&l, &inv)?;
            bail!("too many wrong pair codes just now; wait a minute and try again");
        }
        let found = inv.invites.iter().find_map(|(id, r)| {
            let k = record_key(r).filter(|_| r.state == "pending")?;
            code::verify(&k, "join", &req.fields(), &req.mac).then(|| (id.clone(), k))
        });
        let Some((id, key)) = found else {
            let cancelled = note_failure(&mut inv, now);
            write_invites(&l, &inv)?;
            if cancelled {
                let _ = refresh_door(&inv);
                audit(
                    &l,
                    "lockout",
                    &format!("{MAX_FAILURES} wrong codes: pending invites cancelled"),
                );
            }
            bail!("wrong pair code, or it expired or was already used");
        };
        let r = inv.invites.get_mut(&id).context("invite vanished")?;
        r.state = "redeemed".into();
        r.redeemed_at = Some(now);
        let record = r.clone();
        write_invites(&l, &inv)?;
        let _ = refresh_door(&inv);
        (id, record, key)
    };
    // The joiner holds the code but reached another host key: something relayed the connection.
    anyhow::ensure!(
        req.viewer_host_key == record.host_key,
        "the joiner reached a different SSH host key than this host's (a relay in between?); the invite is used up, make a new one"
    );
    let name = record.name.clone().unwrap_or_else(|| req.name.clone());
    check_identity(&name, &req.host, &req.account, &req.machine_id)?;
    let source: IpAddr = std::env::var("SSH_CONNECTION")
        .ok()
        .and_then(|s| s.split_whitespace().next().map(str::to_owned))
        .and_then(|s| s.parse().ok())
        .context("redeem must run under sshd")?;
    let target = format!("{}@{}", req.account, bracket(source));
    anyhow::ensure!(transport::valid_target(&target), "invalid joiner endpoint");
    let me = format!("{}:{}", machine_id(), store::local_device().account);
    let peer_id = format!("{}:{}", req.machine_id, req.account);
    anyhow::ensure!(peer_id != me, "a device cannot join itself");
    let peer = Device {
        id: peer_id,
        name,
        target: Some(target.clone()),
        account: req.account.clone(),
        host: req.host.clone(),
        status: "joined by invite (reverse unverified)".into(),
        observed_at: now,
    };
    let name = pair_new(&peer, &req.host_key, &req.client_key)?;
    {
        let l = lock()?;
        let mut inv = read_invites(&l)?;
        if let Some(r) = inv.invites.get_mut(&id) {
            r.redeemed_by = Some(format!("{name} ({target})"));
        }
        write_invites(&l, &inv)?;
        audit(&l, "join", &format!("{id}: {name} {target}"));
    }
    let identity = peer_ssh::identity()?;
    let local = store::local_device();
    let viewer = Peer {
        id: me,
        name: local.name,
        host: local.host,
        account: local.account,
        client_key: identity["client_key"]
            .as_str()
            .context("no client key")?
            .into(),
    };
    let mac = code::mac(
        &key,
        "accept",
        &accept_fields(&req.mac, &viewer, &record.host_key),
    );
    Ok(JoinResponse {
        version: PROTOCOL,
        error: None,
        viewer: Some(viewer),
        host_key: Some(record.host_key),
        mac: Some(mac),
    })
}

/// `[user@]host[:port]`: the account (default: this one), the host (an IP or a DNS name) and the port (22 except
/// for the loopback fixture).
fn parse_host(text: &str, default_account: &str) -> Result<(String, String, u16)> {
    let (account, rest) = match text.rsplit_once('@') {
        Some((a, h)) => (a, h),
        None => (default_account, text),
    };
    let (host, port) = if let Ok(a) = rest.parse::<SocketAddr>() {
        (a.ip().to_string(), a.port())
    } else if let Ok(ip) = rest
        .trim_start_matches('[')
        .trim_end_matches(']')
        .parse::<IpAddr>()
    {
        (ip.to_string(), 22)
    } else {
        let (h, p) = match rest.rsplit_once(':') {
            Some((h, p)) => (h, p.parse().ok().context("invalid port")?),
            None => (rest, 22),
        };
        anyhow::ensure!(
            !h.is_empty()
                && h.len() <= 253
                && !h.starts_with(['-', '.'])
                && h.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b".-".contains(&b)),
            "invalid host {:?} (an IP or a DNS name)",
            crate::files::display(h)
        );
        (h.to_owned(), p)
    };
    anyhow::ensure!(
        !account.is_empty()
            && !account.starts_with('-')
            && account
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_-.".contains(&b)),
        "invalid account name"
    );
    check_port(port)?;
    Ok((account.to_owned(), host, port))
}

/// The viewer's Ed25519 host key, as `host` presents it now. It is pinned for the join connection and bound into
/// the request's HMAC, so it is only trusted for good once the viewer's answer proves it knows the code.
fn scan_host_key(host: &str, port: u16) -> Result<String> {
    let mut cmd = Command::new("ssh-keyscan");
    cmd.args(["-T", "5", "-t", "ed25519", "-p", &port.to_string(), "--"])
        .arg(host);
    let (_, out) = super::run_bounded(cmd, b"", 16 * 1024, Duration::from_secs(10))?;
    let text = String::from_utf8_lossy(&out);
    for line in text.lines().filter(|l| !l.starts_with('#')) {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() >= 3 && f[1] == "ssh-ed25519" {
            return peer_ssh::public_key(&format!("{} {}", f[1], f[2]));
        }
    }
    bail!("{host} did not answer with an SSH Ed25519 host key")
}

#[derive(Clone, Debug, Default)]
pub struct JoinOptions {
    pub name: Option<String>,
}

/// `cx mesh pair <code> <host>`: redeem an invite on the viewer at `host` and pair both ways.
pub fn join(code_text: &str, host_text: &str, opts: &JoinOptions) -> Result<Value> {
    let code = code::parse(code_text)?;
    let local = store::local_device();
    let (account, host, port) = parse_host(host_text, &local.account)?;
    let name = opts.name.clone().unwrap_or_else(|| local.name.clone());
    check_identity(&name, &local.host, &local.account, &machine_id())?;
    let identity = peer_ssh::identity()?;
    let key = code::key(&code);
    let host_key = scan_host_key(&host, port)?;
    let mut request = JoinRequest {
        version: PROTOCOL,
        name: name.clone(),
        host: local.host.clone(),
        account: local.account.clone(),
        machine_id: machine_id(),
        client_key: identity["client_key"]
            .as_str()
            .context("no client key")?
            .into(),
        host_key: identity["host_key"].as_str().context("no host key")?.into(),
        viewer_host_key: host_key.clone(),
        mac: String::new(),
    };
    request.mac = code::mac(&key, "join", &request.fields());

    let scratch = Work::new("join")?;
    let door = scratch.0.join("door");
    write_private(&door, DOOR_KEY.as_bytes())?;
    let known = scratch.0.join("known_hosts");
    let entry = if port == 22 {
        host.clone()
    } else {
        format!("[{host}]:{port}")
    };
    write_private(&known, format!("{entry} {host_key}\n").as_bytes())?;
    let mut ssh = Command::new("ssh");
    ssh.args(["-F", "none", "-T", "-i"])
        .arg(&door)
        .args([
            "-o",
            "IdentitiesOnly=yes",
            "-o",
            "IdentityAgent=none",
            "-o",
            "BatchMode=yes",
            "-o",
            "StrictHostKeyChecking=yes",
            "-o",
            "HostKeyAlgorithms=ssh-ed25519",
            "-o",
            "GlobalKnownHostsFile=/dev/null",
            "-o",
            "ForwardAgent=no",
            "-o",
            "ForwardX11=no",
            "-o",
            "ClearAllForwardings=yes",
            "-o",
            "ConnectTimeout=10",
            "-o",
            "PasswordAuthentication=no",
            "-o",
            "KbdInteractiveAuthentication=no",
        ])
        .arg("-o")
        .arg(format!("UserKnownHostsFile={}", known.display()))
        .args(["-p", &port.to_string(), "-l", &account, "--"])
        .arg(&host);
    let mut frame = Vec::new();
    transport::frame(&mut frame, &request)?;
    let (_, out) = super::run_bounded(ssh, &frame, 64 * 1024, Duration::from_secs(40))
        .context("the viewer did not answer the join")?;
    drop(scratch);
    let response: JoinResponse = transport::read_frame(&mut std::io::Cursor::new(out))
        .context("the viewer has no pending invite (none made, all expired or used), or cx there is too old")?;
    if let Some(e) = response.error {
        bail!("the viewer refused the join: {}", crate::files::display(&e));
    }
    let (Some(viewer), Some(viewer_host_key), Some(tag)) =
        (response.viewer, response.host_key, response.mac)
    else {
        bail!("the viewer sent an incomplete answer");
    };
    anyhow::ensure!(
        code::verify(
            &key,
            "accept",
            &accept_fields(&request.mac, &viewer, &viewer_host_key),
            &tag
        ) && viewer_host_key == host_key,
        "{host} does not know this code, so it is not the viewer that made it: nothing was pinned. \
         Cancel the invite on the viewer (cx mesh teardown --backend ssh-overlay --yes) and make a new one"
    );
    anyhow::ensure!(
        valid_name(&viewer.name) && valid_name(&viewer.host),
        "invalid viewer name"
    );
    anyhow::ensure!(
        viewer.account == account,
        "viewer account differs from the one joined"
    );
    let target = match host.parse::<IpAddr>() {
        Ok(ip) => format!("{account}@{}", bracket(ip)),
        Err(_) => format!("{account}@{host}"),
    };
    anyhow::ensure!(
        viewer.id.ends_with(&format!(":{}", viewer.account))
            && viewer.id != format!("{}:{}", request.machine_id, request.account),
        "invalid viewer identity"
    );
    let device = Device {
        id: viewer.id.clone(),
        name: viewer.name.clone(),
        target: Some(target.clone()),
        account: viewer.account.clone(),
        host: viewer.host.clone(),
        status: "joined by invite (host key confirmed by code)".into(),
        observed_at: transport::now(),
    };
    let viewer_name = pair_new(&device, &host_key, &viewer.client_key).context(
        "the viewer enrolled this device, but pairing it here failed; revoke this device on the viewer, fix the cause, then join again",
    )?;
    Ok(json!({
        "joined": true,
        "backend": ID,
        "viewer": viewer_name,
        "target": target,
        "port": port,
        "host_key_fingerprint": code::fingerprint(&host_key)?,
        "as": name,
        "next": "cx device sync verifies the pairing in both directions",
    }))
}

impl MeshBackend for SshOverlay {
    fn id(&self) -> &'static str {
        ID
    }
    fn name(&self) -> &'static str {
        "SSH overlay (built-in)"
    }
    fn capabilities(&self) -> Vec<&'static str> {
        vec!["invite", "join", "revoke", "audit", "coexists-with-vpn"]
    }
    fn status(&self) -> Result<Status> {
        let members = self
            .members()?
            .into_iter()
            .filter(|m| m.state != "local")
            .count();
        let now = transport::now();
        let pending = load_invites()?
            .invites
            .values()
            .filter(|r| r.state == "pending" && now < r.expires)
            .count();
        Ok(Status {
            backend: ID.into(),
            available: true,
            provisioned: true,
            members: Some(members),
            endpoint: None,
            detail: format!(
                "{members} enrolled device(s), {pending} pending invite(s); nothing to provision"
            ),
        })
    }
    fn provision(&self, dry_run: bool) -> Result<Plan> {
        Ok(Plan {
            dry_run,
            applied: false,
            steps: vec![
                "nothing to provision: ssh-overlay uses the SSH access cx already has".into(),
            ],
            result: Value::Null,
        })
    }
    fn teardown(&self, dry_run: bool) -> Result<Plan> {
        // Teardown cancels pending invites; enrolled devices are removed one by one with revoke.
        let l = lock()?;
        let mut inv = read_invites(&l)?;
        expire(&mut inv, transport::now());
        let pending: Vec<String> = inv
            .invites
            .iter()
            .filter(|(_, r)| r.state == "pending")
            .map(|(k, _)| k.clone())
            .collect();
        let steps = pending
            .iter()
            .map(|k| format!("cancel invite {k}"))
            .collect();
        let mut closed = 0;
        if !dry_run {
            for k in &pending {
                if let Some(r) = inv.invites.get_mut(k) {
                    r.state = "cancelled".into();
                }
            }
            write_invites(&l, &inv)?;
            closed = peer_ssh::remove_authorized(is_door_line)?;
            audit(
                &l,
                "teardown",
                &format!("{} invite(s) cancelled", pending.len()),
            );
        }
        Ok(Plan {
            dry_run,
            applied: !dry_run,
            steps,
            result: json!({"authorizations_removed": closed}),
        })
    }
    fn members(&self) -> Result<Vec<Member>> {
        let mut out = Vec::new();
        for d in store::devices()? {
            let policy = store::policy(&d)?;
            let host = d
                .target
                .as_deref()
                .and_then(|t| t.split_once('@'))
                .map(|(_, h)| h.trim_start_matches('[').trim_end_matches(']').to_owned());
            let ip = host.as_deref().and_then(|h| h.parse::<IpAddr>().ok());
            let via = d
                .target
                .as_deref()
                .map(store::route)
                .transpose()?
                .unwrap_or_default();
            out.push(Member {
                name: d.name.clone(),
                host: d.host.clone(),
                os: None,
                overlay_ipv4: ip.filter(IpAddr::is_ipv4).map(|i| i.to_string()),
                overlay_ipv6: ip.filter(IpAddr::is_ipv6).map(|i| i.to_string()),
                dns_name: host.filter(|h| h.parse::<IpAddr>().is_err()),
                state: if d.target.is_none() {
                    "local".into()
                } else {
                    d.status.clone()
                },
                last_seen: (d.observed_at > 0).then(|| iso(d.observed_at)),
                role: Some(match policy.access {
                    store::AccessMode::Core => "core".into(),
                    store::AccessMode::Directed => "directed".into(),
                }),
                via: (!via.is_empty()).then_some(via),
            });
        }
        Ok(out)
    }
    fn revoke(&self, name: &str) -> Result<Value> {
        let (removed, unpinned) = peer_ssh::remove_device(name)?;
        let cancelled = {
            let l = lock()?;
            let mut inv = read_invites(&l)?;
            let mut n = 0;
            for r in inv.invites.values_mut() {
                if r.state == "pending" && r.name.as_deref() == Some(removed.name.as_str()) {
                    r.state = "cancelled".into();
                    n += 1;
                }
            }
            write_invites(&l, &inv)?;
            let _ = refresh_door(&inv);
            audit(&l, "revoke", &removed.name);
            n
        };
        Ok(
            json!({"name": removed.name, "id": removed.id, "revoked": ["device", "ssh pins"], "ssh": unpinned, "invites_cancelled": cancelled}),
        )
    }
    fn address_ranges(&self) -> Vec<IpNet> {
        Vec::new()
    }
    fn audit(&self, limit: usize) -> Result<Vec<AuditEntry>> {
        let text = match fs::read_to_string(dir().join("audit.jsonl")) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(e.into()),
        };
        let mut out: Vec<AuditEntry> = text
            .lines()
            .rev()
            .filter_map(|l| serde_json::from_str(l).ok())
            .take(limit)
            .collect();
        out.truncate(limit);
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(created: u64, expires: u64, state: &str) -> InviteRecord {
        InviteRecord {
            created,
            expires,
            state: state.into(),
            ..Default::default()
        }
    }

    #[test]
    fn door_line_is_restricted_forced_and_expiring() {
        let l = door_line(1_800_000_000, "/home/u/.local/bin/cx");
        assert_eq!(
            l,
            format!("restrict,command=\"/home/u/.local/bin/cx mesh pair --redeem\",expiry-time=\"202701150800Z\" {DOOR_PUBLIC} cx-join-door")
        );
        assert!(is_door_line(&l));
        assert!(is_door_line(
            "restrict,command=\"/bin/cx mesh pair --redeem 0123\" ssh-ed25519 AAAA cx-join-invite:0123"
        ));
        assert!(!is_door_line(
            "no-agent-forwarding ssh-ed25519 AAAA cx-managed-peer"
        ));
        assert!(!is_door_line("ssh-ed25519 AAAA me cx-join-door"));
    }

    /// The embedded door key is one OpenSSH reads, and its public half is the one the door line authorizes.
    #[test]
    fn door_key_is_a_valid_openssh_pair() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("door");
        write_private(&p, DOOR_KEY.as_bytes()).unwrap();
        let out = Command::new("ssh-keygen")
            .arg("-y")
            .arg("-f")
            .arg(&p)
            .output()
            .unwrap();
        assert!(out.status.success());
        assert!(String::from_utf8(out.stdout)
            .unwrap()
            .starts_with(DOOR_PUBLIC));
    }

    #[test]
    fn addresses_hosts_ports_and_identities() {
        assert_eq!(
            parse_address("10.0.0.2").unwrap().to_string(),
            "10.0.0.2:22"
        );
        assert_eq!(parse_address("[fd00::1]:22").unwrap().port(), 22);
        assert!(parse_address("host.example").is_err());
        for other in ["10.0.0.2:2222", "[fd00::1]:2200"] {
            let e = parse_address(other).unwrap_err().to_string();
            assert!(e.contains("port 22 only"), "{e}");
        }
        let ok = |t: &str| parse_host(t, "me").unwrap();
        assert_eq!(ok("10.0.0.2"), ("me".into(), "10.0.0.2".into(), 22));
        assert_eq!(ok("alice@10.0.0.2"), ("alice".into(), "10.0.0.2".into(), 22));
        assert_eq!(ok("alice@fd00::1"), ("alice".into(), "fd00::1".into(), 22));
        assert_eq!(ok("[fd00::1]"), ("me".into(), "fd00::1".into(), 22));
        assert_eq!(ok("rover.local"), ("me".into(), "rover.local".into(), 22));
        assert_eq!(ok("a@rover.local:22").1, "rover.local");
        for bad in [
            "-oProxyCommand=x",
            "a@-x",
            "-a@host",
            "a@ho st",
            "a@host:22x",
            "a@",
            "a@.x",
            "10.0.0.2:2222",
            "rover:2200",
        ] {
            assert!(parse_host(bad, "me").is_err(), "{bad}");
        }
        assert_eq!(
            host_arg("a", &"10.0.0.2:22".parse().unwrap()),
            "a@10.0.0.2"
        );
        assert_eq!(
            host_arg("a", &"[::1]:2222".parse().unwrap()),
            "a@[::1]:2222"
        );
        assert!(
            valid_name("lab-arm") && !valid_name("-x") && !valid_name("a b") && !valid_name("")
        );
        assert!(valid_machine_id("0123abc") && !valid_machine_id("a;b"));
        check_identity("rover", "rover", "alice", "0123abc").unwrap();
        // No /etc/machine-id (macOS, minimal containers): refused before any invite is used.
        let e = check_identity("rover", "rover", "alice", "").unwrap_err();
        assert!(e.to_string().contains("machine id"), "{e}");
        assert!(check_identity("rover", "a b", "alice", "0123abc").is_err());
        assert!(check_identity("rover", "rover", "-oProxy", "0123abc").is_err());
    }

    #[test]
    fn expire_marks_and_keeps_history_bounded() {
        let mut inv = Invites::default();
        inv.invites.insert("old".into(), record(1, 100, "pending"));
        expire(&mut inv, 100);
        assert_eq!(inv.invites["old"].state, "expired");
        for i in 0..70u64 {
            let state = if i % 14 == 0 { "pending" } else { "redeemed" };
            inv.invites
                .insert(format!("{i:02}"), record(10 + i, 10_000, state));
        }
        expire(&mut inv, 200);
        assert_eq!(inv.invites.len(), 64);
        assert_eq!(
            inv.invites
                .values()
                .filter(|r| r.state == "pending")
                .count(),
            5,
            "pending invites are never trimmed"
        );
        assert!(
            !inv.invites.contains_key("old"),
            "the oldest history goes first"
        );
    }

    /// At most FAILURES_PER_WINDOW wrong codes a minute are checked, and MAX_FAILURES in all cancel every
    /// pending invite: an online guesser gets at most 20 tries at 50 bits per invite.
    #[test]
    fn wrong_codes_are_rate_limited_and_capped() {
        let mut inv = Invites::default();
        inv.invites.insert("a".into(), record(1, 10_000, "pending"));
        let mut t = 1000;
        for _ in 0..FAILURES_PER_WINDOW {
            assert!(!rate_limited(&inv, t));
            assert!(!note_failure(&mut inv, t));
        }
        assert!(rate_limited(&inv, t));
        assert!(rate_limited(&inv, t + FAILURE_WINDOW_SECS - 1));
        assert!(!rate_limited(&inv, t + FAILURE_WINDOW_SECS));
        let mut cancelled = false;
        while !cancelled {
            t += FAILURE_WINDOW_SECS;
            assert!(!rate_limited(&inv, t));
            cancelled = note_failure(&mut inv, t);
        }
        assert_eq!(inv.invites["a"].state, "cancelled");
        assert!(inv.failures.is_empty());
    }

    #[test]
    fn a_failed_write_leaves_no_temporary_file() {
        let d = tempfile::tempdir().unwrap();
        let target = d.path().join("invites.json");
        fs::create_dir(&target).unwrap();
        fs::write(target.join("x"), "").unwrap();
        assert!(write_private(&target, b"{}").is_err());
        let left: Vec<_> = fs::read_dir(d.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(left, vec![std::ffi::OsString::from("invites.json")]);
    }

    fn scratch_state(d: &Path) -> [(&'static str, PathBuf); 2] {
        [("HOME", d.to_owned()), ("XDG_STATE_HOME", d.join("state"))]
    }

    fn env<'a>(v: &'a [(&'static str, PathBuf)]) -> Vec<(&'static str, &'a Path)> {
        v.iter().map(|(k, p)| (*k, p.as_path())).collect()
    }

    fn private_tempdir() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        fs::set_permissions(
            d.path(),
            std::os::unix::fs::PermissionsExt::from_mode(0o700),
        )
        .unwrap();
        d
    }

    /// Separate processes appending at once (two redeems, an invite during a redeem) keep every
    /// line, and a full log is trimmed to its newest half.
    #[test]
    fn audit_appends_from_concurrent_processes_are_kept() {
        let d = private_tempdir();
        let vars = scratch_state(d.path());
        if !super::super::in_child(
            "mesh::ssh_overlay::tests::audit_appends_from_concurrent_processes_are_kept",
            &env(&vars),
        ) {
            return;
        }
        if let Ok(n) = std::env::var("CX_TEST_AUDIT_WRITER") {
            for i in 0..50 {
                audit(&lock().unwrap(), "join", &format!("{n}.{i}"));
            }
            return;
        }
        let writers: Vec<_> = (0..4)
            .map(|n| {
                Command::new(std::env::current_exe().unwrap())
                    .args([
                        "mesh::ssh_overlay::tests::audit_appends_from_concurrent_processes_are_kept",
                        "--exact",
                    ])
                    .env("CX_TEST_AUDIT_WRITER", n.to_string())
                    .stdout(std::process::Stdio::null())
                    .spawn()
                    .unwrap()
            })
            .collect();
        for mut w in writers {
            assert!(w.wait().unwrap().success());
        }
        let path = dir().join("audit.jsonl");
        assert_eq!(fs::read_to_string(&path).unwrap().lines().count(), 200);
        let full: String = (0..1000).map(|i| format!("{{\"n\":{i}}}\n")).collect();
        fs::write(&path, full).unwrap();
        audit(&lock().unwrap(), "invite", "x");
        let text = fs::read_to_string(&path).unwrap();
        assert_eq!(text.lines().count(), 500);
        assert!(text.starts_with("{\"n\":501}\n") && text.contains("\"invite\""));
    }

    const VIEWER_HOST_KEY: &str =
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl";

    /// A pending invite for `code` (expiring at `expires`), stored as the viewer stores it.
    fn seed_invite(id: &str, code: &str, expires: u64) {
        let l = lock().unwrap();
        let mut inv = read_invites(&l).unwrap();
        inv.invites.insert(
            id.into(),
            InviteRecord {
                created: 1,
                expires,
                state: "pending".into(),
                key: hex(&code::key(code)),
                host_key: VIEWER_HOST_KEY.into(),
                ..Default::default()
            },
        );
        write_invites(&l, &inv).unwrap();
    }

    /// A request with a valid identity, signed with `code`. Its machine id is invalid on purpose ("bad id"), so
    /// a redemption that gets past the code checks stops at the identity check without pairing anything.
    fn request(code: &str) -> JoinRequest {
        let mut r = JoinRequest {
            version: PROTOCOL,
            name: "rover".into(),
            host: "rover".into(),
            account: "alice".into(),
            machine_id: "bad id".into(),
            client_key: VIEWER_HOST_KEY.replace("IOMq", "IOMr"),
            host_key: VIEWER_HOST_KEY.replace("IOMq", "IOMs"),
            viewer_host_key: VIEWER_HOST_KEY.into(),
            mac: String::new(),
        };
        r.mac = code::mac(&code::key(code), "join", &r.fields());
        r
    }

    fn state(id: &str) -> String {
        load_invites().unwrap().invites[id].state.clone()
    }

    /// The viewer's side of the code checks: a wrong code, a tampered key or host key, an expired invite are
    /// refused and counted; the right one burns the invite before anything else (single use).
    #[test]
    fn redeem_checks_the_code_once() {
        let d = private_tempdir();
        let vars = scratch_state(d.path());
        if !super::super::in_child(
            "mesh::ssh_overlay::tests::redeem_checks_the_code_once",
            &env(&vars),
        ) {
            return;
        }
        let now = transport::now();
        let code = code::new_code().unwrap();
        seed_invite("aaaaaaaaaaaaaaaa", &code, now + 3600);
        let err = |r: JoinRequest| format!("{:#}", redeem_request(r).err().unwrap());

        let other = code::new_code().unwrap();
        assert!(err(request(&other)).contains("wrong pair code"));
        let mut tampered = request(&code);
        tampered.client_key = VIEWER_HOST_KEY.replace("IOMq", "IOMt");
        assert!(err(tampered).contains("wrong pair code"), "a swapped key fails the MAC");
        let mut relayed = request(&code);
        relayed.viewer_host_key = VIEWER_HOST_KEY.replace("IOMq", "IOMu");
        assert!(err(relayed).contains("wrong pair code"), "so does a swapped host key");
        assert_eq!(state("aaaaaaaaaaaaaaaa"), "pending");
        assert_eq!(load_invites().unwrap().failures.len(), 3);
        // A fourth try within the minute is refused unchecked, even with the right code.
        assert!(err(request(&code)).contains("wait a minute"));
        assert_eq!(state("aaaaaaaaaaaaaaaa"), "pending");

        {
            let l = lock().unwrap();
            let mut inv = read_invites(&l).unwrap();
            inv.failures.clear();
            write_invites(&l, &inv).unwrap();
        }
        // The right code burns the invite first; this request then fails its identity check, and stays burnt.
        assert!(err(request(&code)).contains("machine id"));
        assert_eq!(state("aaaaaaaaaaaaaaaa"), "redeemed");
        assert!(err(request(&code)).contains("wrong pair code"), "single use");

        // An expired invite is refused even with the right code (sshd's expiry-time is only to the minute).
        let late = code::new_code().unwrap();
        seed_invite("bbbbbbbbbbbbbbbb", &late, now - 1);
        assert!(err(request(&late)).contains("wrong pair code"));
        assert_eq!(state("bbbbbbbbbbbbbbbb"), "expired");

        // The right code over a relay (another host key, MAC still valid) burns the invite and is refused.
        let relay = code::new_code().unwrap();
        seed_invite("cccccccccccccccc", &relay, now + 3600);
        let mut r = request(&relay);
        r.viewer_host_key = VIEWER_HOST_KEY.replace("IOMq", "IOMv");
        r.mac = code::mac(&code::key(&relay), "join", &r.fields());
        {
            let l = lock().unwrap();
            let mut inv = read_invites(&l).unwrap();
            inv.failures.clear();
            write_invites(&l, &inv).unwrap();
        }
        assert!(err(r).contains("different SSH host key"));
        assert_eq!(state("cccccccccccccccc"), "redeemed");
        assert!(store::devices().unwrap().iter().all(|d| d.target.is_none()));
    }

    /// The joiner's check of the answer: only a holder of the code can make it, for this request only.
    #[test]
    fn answers_are_bound_to_the_code_and_the_request() {
        let code = code::new_code().unwrap();
        let key = code::key(&code);
        let viewer = Peer {
            id: "m:alice".into(),
            name: "viewer".into(),
            host: "viewer".into(),
            account: "alice".into(),
            client_key: VIEWER_HOST_KEY.into(),
        };
        let tag = code::mac(&key, "accept", &accept_fields("req", &viewer, VIEWER_HOST_KEY));
        assert!(code::verify(&key, "accept", &accept_fields("req", &viewer, VIEWER_HOST_KEY), &tag));
        assert!(!code::verify(&key, "accept", &accept_fields("other", &viewer, VIEWER_HOST_KEY), &tag));
        assert!(!code::verify(&key, "join", &accept_fields("req", &viewer, VIEWER_HOST_KEY), &tag));
        let mut swapped = viewer.clone();
        swapped.client_key = VIEWER_HOST_KEY.replace("IOMq", "IOMr");
        assert!(!code::verify(&key, "accept", &accept_fields("req", &swapped, VIEWER_HOST_KEY), &tag));
        let wrong = code::key(&code::new_code().unwrap());
        assert!(!code::verify(&wrong, "accept", &accept_fields("req", &viewer, VIEWER_HOST_KEY), &tag));
    }

    /// The joiner refuses an identity the viewer would refuse before it connects, so the invite
    /// is not burnt (and nothing is generated or contacted).
    #[test]
    fn a_join_with_an_invalid_identity_fails_before_connecting() {
        let d = private_tempdir();
        let mut vars = scratch_state(d.path()).to_vec();
        vars.push(("CX_JOIN_MACHINE_ID", PathBuf::from("no;such id")));
        if !super::super::in_child(
            "mesh::ssh_overlay::tests::a_join_with_an_invalid_identity_fails_before_connecting",
            &env(&vars),
        ) {
            return;
        }
        let code = code::new_code().unwrap();
        let started = std::time::Instant::now();
        // TEST-NET-1: never answers, so a missing check would show up as a timeout, not a pass.
        let e = join(&code, "alice@192.0.2.1", &JoinOptions::default()).unwrap_err();
        assert!(e.to_string().contains("machine id"), "{e:#}");
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(
            !store::state_dir().join("peer-ssh").exists(),
            "no identity generated"
        );
        let e = join("ABCD", "alice@192.0.2.1", &JoinOptions::default()).unwrap_err();
        assert!(e.to_string().contains("11 characters"), "{e:#}");
    }
}
