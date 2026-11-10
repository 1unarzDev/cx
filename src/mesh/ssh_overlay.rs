//! The ssh-overlay backend: the lightweight default mesh, built from what cx already has and able to
//! coexist with any VPN (it claims no address range). Members are the enrolled devices; core
//! devices pair automatically (peers.rs, peer_ssh.rs); reachability is the existing routes plus
//! ProxyJump through a reachable peer. New devices join with `cx mesh pair <code>` (docs/mesh.md
//! has the protocol and its threat model):
//!
//! 1. `cx mesh invite --backend ssh-overlay` on the viewer makes a direct code (code.rs) holding
//!    its SSH account, reachable addresses, the SHA-256 prefix of its SSH host key and a one-time
//!    token. The token derives an Ed25519 *invite key*; the viewer authorizes only its public half,
//!    as `restrict,command="<cx> mesh pair --redeem <id>",expiry-time=...` tagged
//!    `cx-join-invite:<id>`.
//! 2. `cx mesh pair <code>` on the joiner fetches the viewer's host key, accepts it only when its digest
//!    matches the code (then pins it for the connection, StrictHostKeyChecking=yes), and connects
//!    with the invite key. sshd runs only the redeem command, which burns the invite (single use,
//!    expiry enforced by sshd and by cx), installs the joiner's own key as `cx-managed-peer` (a
//!    full shell as the viewer's account: only agent, port and X11 forwarding and user rc are
//!    disabled), pins the joiner's host key, enrolls it as a core device, and answers with the
//!    viewer's public keys. The joiner pins and authorizes those the same way.
use super::{
    code::Direct, ed25519, iso, AuditEntry, Invite, IpNet, Member, MeshBackend, Plan, Status,
};
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
const PROTOCOL: u32 = 1;
const MAX_TTL_HOURS: f64 = 24.0 * 7.0;
const DEFAULT_TTL_HOURS: f64 = 1.0;
const MAX_PENDING: usize = 16;

pub struct SshOverlay;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct InviteRecord {
    created: u64,
    expires: u64,
    #[serde(default)]
    name: Option<String>,
    state: String, // pending | redeemed | cancelled | failed
    public_key: String,
    host_key: String,
    #[serde(default)]
    redeemed_by: Option<String>,
    #[serde(default)]
    redeemed_at: Option<u64>,
}

#[derive(Default, Serialize, Deserialize)]
struct Invites {
    invites: BTreeMap<String, InviteRecord>,
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

fn invite_line(id: &str, public: &str, expires: u64, exe: &str) -> String {
    // OpenSSH expiry-time: YYYYMMDDHHMM with Z for UTC (sshd(8) AUTHORIZED_KEYS FILE FORMAT).
    let stamp = iso(expires);
    let t: String = stamp
        .chars()
        .filter(|c| c.is_ascii_digit())
        .take(12)
        .collect();
    format!(
        "restrict,command=\"{exe} mesh pair --redeem {id}\",expiry-time=\"{t}Z\" {public} cx-join-invite:{id}"
    )
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
    /// Explicit addresses (`ip`, or `ip:22` / `[v6]:22`); default: this host's own.
    pub addresses: Vec<String>,
    /// The host key to pin, when sshd does not use /etc/ssh/ssh_host_ed25519_key.
    pub host_key_file: Option<PathBuf>,
    /// Also stash the code on the Cloudflare mesh Worker and return a short code.
    pub short: bool,
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
    check_port(&a)?;
    Ok(a)
}

/// cx device targets carry no port, so a pairing is only usable over port 22: anything else would
/// burn the invite for a one-sided pairing. The loopback fixture (debug builds) is the exception.
fn check_port(addr: &SocketAddr) -> Result<()> {
    anyhow::ensure!(
        addr.port() == 22 || fixture_machine_id().is_some(),
        "{addr}: cx pairs over SSH port 22 only (device targets carry no port)"
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

/// Make an invite: authorize the invite key (restricted, forced, expiring) and return the code.
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
        default_addresses()?
    } else {
        opts.addresses
            .iter()
            .map(|a| parse_address(a))
            .collect::<Result<_>>()?
    };
    anyhow::ensure!(
        !addresses.is_empty(),
        "no reachable address to put in the code; pass --address"
    );
    let host_key = match &opts.host_key_file {
        Some(p) => peer_ssh::public_key(&fs::read_to_string(p).context("cannot read --host-key")?)?,
        None => peer_ssh::host_public_key()
            .context("this host has no readable SSH Ed25519 host key; pass --host-key")?,
    };
    let account = store::local_device().account;
    let now = transport::now();
    let direct = Direct::new(&account, addresses, &host_key, (ttl * 3600.0) as u64, now)?;
    let id = direct.invite_id();
    let public = ed25519::openssh_public(&ed25519::public_key(&direct.invite_seed()));
    let exe = exe()?;
    {
        let l = lock()?;
        let mut inv = read_invites(&l)?;
        expire(&mut inv, now);
        let _ = sweep(&inv);
        let pending = inv
            .invites
            .values()
            .filter(|r| r.state == "pending")
            .count();
        anyhow::ensure!(
            pending < MAX_PENDING,
            "too many pending invites; cancel some with `cx mesh teardown --backend ssh-overlay`"
        );
        inv.invites.insert(
            id.clone(),
            InviteRecord {
                created: now,
                expires: direct.expires,
                name: opts.device_name.clone(),
                state: "pending".into(),
                public_key: public.clone(),
                host_key,
                ..Default::default()
            },
        );
        write_invites(&l, &inv)?;
        peer_ssh::authorize_line(&invite_line(&id, &public, direct.expires, &exe))?;
        audit(&l, "invite", &format!("{id} until {}", iso(direct.expires)));
    }
    let code = direct.encode();
    let mut invite = Invite {
        code: code.clone(),
        url: None,
        one_liner_sh: format!("cx mesh pair {code}"),
        one_liner_ps: None,
        expires: iso(direct.expires),
        uses: 1,
        backend: ID.into(),
        short_code: None,
    };
    if opts.short {
        let short = super::code::short_code()?;
        let sealed = super::code::seal(&short, code.as_bytes())?;
        let r = super::cloudflare::stash_pair(&super::code::short_lookup(&short), &sealed, ttl)?;
        let base = r["url"]
            .as_str()
            .context("the mesh Worker returned no URL")?;
        let host = base.trim_start_matches("https://").trim_end_matches('/');
        let shown = super::code::group(&short, 4);
        invite.url = Some(format!("{base}/p/{shown}"));
        invite.short_code = Some(shown.clone());
        invite.one_liner_sh = format!("cx mesh pair {host}/{shown}");
    }
    Ok(invite)
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

/// An invite authorization whose invite is no longer pending, or no longer recorded at all
/// (expire() trims old records, so their lines are found by the absence of a pending one).
fn stale_invite_line(line: &str, inv: &Invites) -> bool {
    line.starts_with("restrict,command=")
        && line
            .rsplit_once(" cx-join-invite:")
            .is_some_and(|(_, id)| inv.invites.get(id).is_none_or(|r| r.state != "pending"))
}

/// Remove the authorized_keys lines of invites that are no longer pending.
fn sweep(inv: &Invites) -> Result<usize> {
    peer_ssh::remove_authorized(|l| stale_invite_line(l, inv))
}

#[derive(Serialize, Deserialize)]
struct JoinRequest {
    version: u32,
    id: String,
    name: String,
    host: String,
    account: String,
    machine_id: String,
    client_key: String,
    host_key: String,
}

#[derive(Serialize, Deserialize)]
struct JoinResponse {
    version: u32,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    viewer: Option<Peer>,
}

#[derive(Serialize, Deserialize)]
struct Peer {
    id: String,
    name: String,
    host: String,
    account: String,
    client_key: String,
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

/// The forced command (`cx mesh pair --redeem <id>`), on the viewer, under sshd: stdin/stdout carry
/// one request and one response frame. Always answers with a frame, even on refusal.
pub fn redeem(id: &str) -> Result<()> {
    let stdin = std::io::stdin();
    let mut input = BufReader::new(stdin.lock().take(64 * 1024));
    let result = (|| -> Result<Peer> {
        anyhow::ensure!(
            id.len() == 16 && id.bytes().all(|b| b.is_ascii_hexdigit()),
            "invalid invite"
        );
        let req: JoinRequest = transport::read_frame(&mut input)?;
        redeem_request(id, req)
    })();
    let response = match result {
        Ok(viewer) => JoinResponse {
            version: PROTOCOL,
            error: None,
            viewer: Some(viewer),
        },
        Err(e) => {
            if let Ok(l) = lock() {
                audit(
                    &l,
                    "join-refused",
                    &format!("{id}: {}", crate::files::display(&format!("{e:#}"))),
                );
            }
            JoinResponse {
                version: PROTOCOL,
                error: Some(format!("{e:#}")),
                viewer: None,
            }
        }
    };
    let mut out = std::io::stdout().lock();
    transport::frame(&mut out, &response)?;
    out.flush()?;
    Ok(())
}

fn redeem_request(id: &str, req: JoinRequest) -> Result<Peer> {
    let now = transport::now();
    // Burn the invite first: whatever happens next, it cannot be used again.
    let record = {
        let l = lock()?;
        let mut inv = read_invites(&l)?;
        expire(&mut inv, now);
        let r = inv.invites.get_mut(id).context("this invite is unknown")?;
        anyhow::ensure!(r.state == "pending", "this invite was already {}", r.state);
        r.state = "redeemed".into();
        r.redeemed_at = Some(now);
        let record = r.clone();
        write_invites(&l, &inv)?;
        let _ = sweep(&inv);
        record
    };
    anyhow::ensure!(
        req.version == PROTOCOL,
        "unsupported join protocol {}",
        req.version
    );
    anyhow::ensure!(req.id == id, "request names another invite");
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
        id: peer_id.clone(),
        name: name.clone(),
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
        if let Some(r) = inv.invites.get_mut(id) {
            r.redeemed_by = Some(format!("{name} ({target})"));
        }
        write_invites(&l, &inv)?;
        audit(&l, "join", &format!("{id}: {name} {target}"));
    }
    let identity = peer_ssh::identity()?;
    let local = store::local_device();
    Ok(Peer {
        id: me,
        name: local.name,
        host: local.host,
        account: local.account,
        client_key: identity["client_key"]
            .as_str()
            .context("no client key")?
            .into(),
    })
}

/// The viewer's host key at `addr`, accepted only when its digest matches the code.
fn verified_host_key(code: &Direct, addr: &SocketAddr) -> Result<String> {
    let mut cmd = Command::new("ssh-keyscan");
    cmd.args([
        "-T",
        "5",
        "-t",
        "ed25519",
        "-p",
        &addr.port().to_string(),
        "--",
    ])
    .arg(addr.ip().to_string());
    let (_, out) = super::run_bounded(cmd, b"", 16 * 1024, Duration::from_secs(10))?;
    let text = String::from_utf8_lossy(&out);
    let mut seen = false;
    for line in text.lines().filter(|l| !l.starts_with('#')) {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() >= 3 && f[1] == "ssh-ed25519" {
            seen = true;
            let key = format!("{} {}", f[1], f[2]);
            if code.matches_host_key(&key) {
                return peer_ssh::public_key(&key);
            }
        }
    }
    if seen {
        bail!("{addr} presented a different SSH host key than the code names: refusing (possible impersonation)")
    }
    bail!("{addr} did not answer with an SSH Ed25519 host key")
}

#[derive(Clone, Debug, Default)]
pub struct JoinOptions {
    pub name: Option<String>,
}

fn resolve(text: &str) -> Result<String> {
    let t = text.trim();
    // A short code comes with the Worker it was stashed on: host[/p]/CODE or https://host/p/CODE.
    if t.contains('/') || t.contains('.') {
        let t = t.trim_start_matches("https://");
        let (host, rest) = t.split_once('/').context("expected <mesh host>/<code>")?;
        let short = super::code::parse_short(rest.trim_start_matches("p/"))?;
        anyhow::ensure!(
            !host.is_empty()
                && host
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b".-:".contains(&b)),
            "invalid mesh host"
        );
        let sealed = super::cloudflare::redeem_pair(
            &format!("https://{host}"),
            &super::code::short_lookup(&short),
        )?;
        let plain = super::code::unseal(&short, &sealed)?;
        return Ok(String::from_utf8(plain)?);
    }
    Ok(t.to_owned())
}

/// `cx mesh pair <code>`: redeem an invite on the viewer it names and pair both ways.
pub fn join(text: &str, opts: &JoinOptions) -> Result<Value> {
    let code = Direct::decode(&resolve(text)?)?;
    let now = transport::now();
    anyhow::ensure!(
        !code.expired(now),
        "this join code expired at {}; ask for a new one",
        iso(code.expires)
    );
    for addr in &code.addresses {
        check_port(addr)?;
    }
    let local = store::local_device();
    let name = opts.name.clone().unwrap_or_else(|| local.name.clone());
    check_identity(&name, &local.host, &local.account, &machine_id())?;
    let identity = peer_ssh::identity()?;
    let request = JoinRequest {
        version: PROTOCOL,
        id: code.invite_id(),
        name: name.clone(),
        host: local.host.clone(),
        account: local.account.clone(),
        machine_id: machine_id(),
        client_key: identity["client_key"]
            .as_str()
            .context("no client key")?
            .into(),
        host_key: identity["host_key"].as_str().context("no host key")?.into(),
    };
    let work = ensure_dir()?.join(format!("join-{}", std::process::id()));
    let _ = fs::remove_dir_all(&work);
    fs::DirBuilder::new().mode(0o700).create(&work)?;
    struct Cleanup(PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(work.clone());
    let seed = code.invite_seed();
    let public = ed25519::public_key(&seed);
    let key_path = work.join("invite");
    write_private(
        &key_path,
        ed25519::openssh_private(
            &seed,
            &public,
            "cx-join",
            u32::from_be_bytes(super::code::random()?),
        )
        .as_bytes(),
    )?;
    let mut failures = Vec::new();
    for addr in &code.addresses {
        let attempt = (|| -> Result<Value> {
            let host_key = verified_host_key(&code, addr)?;
            let known = work.join("known_hosts");
            let host = if addr.port() == 22 {
                addr.ip().to_string()
            } else {
                format!("[{}]:{}", addr.ip(), addr.port())
            };
            write_private(&known, format!("{host} {host_key}\n").as_bytes())?;
            let mut ssh = Command::new("ssh");
            ssh.args(["-F", "none", "-T", "-i"])
                .arg(&key_path)
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
                .args(["-p", &addr.port().to_string(), "-l", &code.account, "--"])
                .arg(addr.ip().to_string());
            let mut frame = Vec::new();
            transport::frame(&mut frame, &request)?;
            let (_, out) = super::run_bounded(ssh, &frame, 64 * 1024, Duration::from_secs(30))
                .context("the viewer did not answer the join")?;
            let response: JoinResponse = transport::read_frame(&mut std::io::Cursor::new(out))
                .context("the viewer refused the invite key (expired, used, or cancelled)")?;
            if let Some(e) = response.error {
                bail!("the viewer refused the join: {}", crate::files::display(&e));
            }
            let viewer = response.viewer.context("the viewer sent no identity")?;
            anyhow::ensure!(
                valid_name(&viewer.name) && valid_name(&viewer.host),
                "invalid viewer name"
            );
            anyhow::ensure!(
                viewer.account == code.account,
                "viewer account differs from the code"
            );
            let target = format!("{}@{}", viewer.account, bracket(addr.ip()));
            let device = Device {
                id: viewer.id.clone(),
                name: viewer.name.clone(),
                target: Some(target.clone()),
                account: viewer.account.clone(),
                host: viewer.host.clone(),
                status: "joined by invite (host key verified by code)".into(),
                observed_at: transport::now(),
            };
            anyhow::ensure!(
                device.id.ends_with(&format!(":{}", device.account))
                    && device.id != request_id(&request),
                "invalid viewer identity"
            );
            let viewer_name = pair_new(&device, &host_key, &viewer.client_key).context(
                "the viewer enrolled this device, but pairing it here failed; revoke this device on the viewer, fix the cause, then join again",
            )?;
            let digest = super::code::host_key_digest(&host_key)?;
            Ok(json!({
                "joined": true,
                "backend": ID,
                "viewer": viewer_name,
                "target": target,
                "port": addr.port(),
                "host_key_sha256_prefix": digest[..super::code::FINGERPRINT_LEN].iter().map(|b| format!("{b:02x}")).collect::<String>(),
                "as": name,
                "next": "cx device sync verifies the pairing in both directions",
            }))
        })();
        match attempt {
            Ok(v) => return Ok(v),
            Err(e) => failures.push(format!("{addr}: {e:#}")),
        }
    }
    bail!("could not join: {}", failures.join("; "))
}

fn request_id(r: &JoinRequest) -> String {
    format!("{}:{}", r.machine_id, r.account)
}

impl MeshBackend for SshOverlay {
    fn id(&self) -> &'static str {
        ID
    }
    fn name(&self) -> &'static str {
        "SSH overlay (built-in)"
    }
    fn capabilities(&self) -> Vec<&'static str> {
        let mut c = vec!["invite", "join", "revoke", "audit", "coexists-with-vpn"];
        if super::cloudflare::quick_status().provisioned {
            c.push("short-code");
        }
        c
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
        if !dry_run {
            for k in &pending {
                if let Some(r) = inv.invites.get_mut(k) {
                    r.state = "cancelled".into();
                }
            }
            write_invites(&l, &inv)?;
            audit(
                &l,
                "teardown",
                &format!("{} invite(s) cancelled", pending.len()),
            );
        }
        let swept = if dry_run { 0 } else { sweep(&inv)? };
        Ok(Plan {
            dry_run,
            applied: !dry_run,
            steps,
            result: json!({"authorizations_removed": swept}),
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
            let _ = sweep(&inv);
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
    fn invite_line_is_restricted_forced_and_expiring() {
        let l = invite_line(
            "0123456789abcdef",
            "ssh-ed25519 AAAA",
            1_800_000_000,
            "/home/u/.local/bin/cx",
        );
        assert_eq!(
            l,
            "restrict,command=\"/home/u/.local/bin/cx mesh pair --redeem 0123456789abcdef\",expiry-time=\"202701150800Z\" ssh-ed25519 AAAA cx-join-invite:0123456789abcdef"
        );
    }

    #[test]
    fn addresses_ports_and_identities() {
        assert_eq!(
            parse_address("10.0.0.2").unwrap().to_string(),
            "10.0.0.2:22"
        );
        assert_eq!(parse_address("10.0.0.2:22").unwrap().port(), 22);
        assert_eq!(parse_address("fd00::1").unwrap().port(), 22);
        assert_eq!(parse_address("[fd00::1]:22").unwrap().port(), 22);
        assert!(parse_address("host.example").is_err());
        // Device targets carry no port: a pairing over another port would be one-sided.
        for other in ["10.0.0.2:2222", "[fd00::1]:2200"] {
            let e = parse_address(other).unwrap_err().to_string();
            assert!(e.contains("port 22 only"), "{e}");
        }
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
    fn short_code_references_resolve_shape() {
        assert!(resolve("mesh.example.org/not-a-code").is_err());
        assert_eq!(resolve("  ABCDE-12345 ").unwrap(), "ABCDE-12345");
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

    #[test]
    fn stale_invite_lines_include_trimmed_records() {
        let mut inv = Invites::default();
        inv.invites.insert("aa".into(), record(1, 100, "pending"));
        inv.invites.insert("bb".into(), record(1, 100, "redeemed"));
        let line = |id: &str| invite_line(id, "ssh-ed25519 AAAA", 100, "/bin/cx");
        assert!(!stale_invite_line(&line("aa"), &inv));
        assert!(stale_invite_line(&line("bb"), &inv));
        // Trimmed from the map by expire(): its line must still be swept.
        assert!(stale_invite_line(&line("cc"), &inv));
        assert!(!stale_invite_line(
            "no-agent-forwarding ssh-ed25519 AAAA cx-managed-peer",
            &inv
        ));
        assert!(!stale_invite_line(
            "ssh-ed25519 AAAA me cx-join-invite:cc",
            &inv
        ));
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

    /// Separate processes appending at once (two redeems, an invite during a redeem) keep every
    /// line, and a full log is trimmed to its newest half.
    #[test]
    fn audit_appends_from_concurrent_processes_are_kept() {
        let d = tempfile::tempdir().unwrap();
        fs::set_permissions(
            d.path(),
            std::os::unix::fs::PermissionsExt::from_mode(0o700),
        )
        .unwrap();
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

    /// cx's own expiry holds even when sshd's minute-rounded expiry-time still lets the key in.
    #[test]
    fn an_expired_invite_is_not_redeemed() {
        let d = tempfile::tempdir().unwrap();
        fs::set_permissions(
            d.path(),
            std::os::unix::fs::PermissionsExt::from_mode(0o700),
        )
        .unwrap();
        let vars = scratch_state(d.path());
        if !super::super::in_child(
            "mesh::ssh_overlay::tests::an_expired_invite_is_not_redeemed",
            &env(&vars),
        ) {
            return;
        }
        let now = transport::now();
        {
            let l = lock().unwrap();
            let mut inv = Invites::default();
            inv.invites.insert(
                "0123456789abcdef".into(),
                record(now - 120, now - 1, "pending"),
            );
            write_invites(&l, &inv).unwrap();
        }
        let req = JoinRequest {
            version: PROTOCOL,
            id: "0123456789abcdef".into(),
            name: "rover".into(),
            host: "rover".into(),
            account: "alice".into(),
            machine_id: "m1".into(),
            client_key: String::new(),
            host_key: String::new(),
        };
        let e = redeem_request("0123456789abcdef", req).err().unwrap();
        assert_eq!(e.to_string(), "this invite was already expired");
        assert!(store::devices().unwrap().iter().all(|d| d.target.is_none()));
    }

    /// The joiner refuses an identity the viewer would refuse before it connects, so the invite
    /// is not burnt (and nothing is generated or contacted).
    #[test]
    fn a_join_with_an_invalid_identity_fails_before_connecting() {
        let d = tempfile::tempdir().unwrap();
        fs::set_permissions(
            d.path(),
            std::os::unix::fs::PermissionsExt::from_mode(0o700),
        )
        .unwrap();
        let mut vars = scratch_state(d.path()).to_vec();
        vars.push(("CX_JOIN_MACHINE_ID", PathBuf::from("no;such id")));
        if !super::super::in_child(
            "mesh::ssh_overlay::tests::a_join_with_an_invalid_identity_fails_before_connecting",
            &env(&vars),
        ) {
            return;
        }
        let key =
            "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl";
        // TEST-NET-1: never answers, so a missing check would show up as a timeout, not a pass.
        let code = Direct::new(
            "alice",
            vec!["192.0.2.1:22".parse().unwrap()],
            key,
            3600,
            transport::now(),
        )
        .unwrap()
        .encode();
        let started = std::time::Instant::now();
        let e = join(&code, &JoinOptions::default()).unwrap_err();
        assert!(e.to_string().contains("machine id"), "{e:#}");
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(
            !store::state_dir().join("peer-ssh").exists(),
            "no identity generated"
        );
    }
}
