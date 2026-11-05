//! The ssh-overlay backend: the lightweight default mesh, built from what cx already has and able to
//! coexist with any VPN (it claims no address range). Members are the enrolled devices; core
//! devices pair automatically (peers.rs, peer_ssh.rs); reachability is the existing routes plus
//! ProxyJump through a reachable peer. New devices join with `cx join <code>` (docs/mesh.md has
//! the protocol and its threat model):
//!
//! 1. `cx mesh invite --backend ssh-overlay` on the viewer makes a direct code (code.rs) holding
//!    its SSH account, reachable addresses, the SHA-256 prefix of its SSH host key and a one-time
//!    token. The token derives an Ed25519 *invite key*; the viewer authorizes only its public half,
//!    as `restrict,command="<cx> join --redeem <id>",expiry-time=...` tagged `cx-join-invite:<id>`.
//! 2. `cx join <code>` on the joiner fetches the viewer's host key, accepts it only when its digest
//!    matches the code (then pins it for the connection, StrictHostKeyChecking=yes), and connects
//!    with the invite key. sshd runs only the redeem command, which burns the invite (single use,
//!    expiry enforced by sshd and by cx), installs the joiner's own key as `cx-managed-peer`
//!    (restricted), pins the joiner's host key, enrolls it as a core device, and answers with the
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
    f.write_all(bytes)?;
    f.sync_all()?;
    fs::rename(&tmp, path)?;
    Ok(())
}

fn write_invites(_l: &Lock, inv: &Invites) -> Result<()> {
    write_private(
        &dir().join("invites.json"),
        &serde_json::to_vec_pretty(inv)?,
    )
}

fn audit(action: &str, detail: &str) {
    let line =
        json!({"at": iso(transport::now()), "actor": "local", "action": action, "detail": detail});
    let path = dir().join("audit.jsonl");
    let _ = (|| -> Result<()> {
        ensure_dir()?;
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
        write_private(&path, text.as_bytes())
    })();
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
        "restrict,command=\"{exe} join --redeem {id}\",expiry-time=\"{t}Z\" {public} cx-join-invite:{id}"
    )
}

fn invite_tag(id: &str) -> String {
    format!(" cx-join-invite:{id}")
}

/// Addresses a joiner may reach this host at, best first: private LAN IPv4, overlay IPv4
/// (CGNAT), then global and ULA IPv6. Loopback and link-local are never offered.
pub fn default_addresses(port: u16) -> Result<Vec<SocketAddr>> {
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
        .map(|ip| SocketAddr::new(ip, port))
        .collect())
}

#[derive(Clone, Debug, Default)]
pub struct InviteOptions {
    pub ttl_hours: Option<f64>,
    pub device_name: Option<String>,
    /// Explicit addresses (`ip` or `ip:port` / `[v6]:port`); default: this host's own.
    pub addresses: Vec<String>,
    pub port: Option<u16>,
    /// The host key to pin, when sshd does not use /etc/ssh/ssh_host_ed25519_key.
    pub host_key_file: Option<PathBuf>,
    /// Also stash the code on the Cloudflare mesh Worker and return a short code.
    pub short: bool,
}

fn parse_address(s: &str, port: u16) -> Result<SocketAddr> {
    if let Ok(a) = s.parse::<SocketAddr>() {
        return Ok(a);
    }
    let ip: IpAddr = s
        .trim_start_matches('[')
        .trim_end_matches(']')
        .parse()
        .with_context(|| format!("invalid address {s:?} (an IP, optionally with :port)"))?;
    Ok(SocketAddr::new(ip, port))
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
    let port = opts.port.unwrap_or(22);
    let addresses = if opts.addresses.is_empty() {
        default_addresses(port)?
    } else {
        opts.addresses
            .iter()
            .map(|a| parse_address(a, port))
            .collect::<Result<_>>()?
    };
    anyhow::ensure!(
        !addresses.is_empty(),
        "no reachable address to put in the code; pass --address"
    );
    let host_key = match &opts.host_key_file {
        Some(p) => {
            peer_ssh::ed25519_public(&fs::read_to_string(p).context("cannot read --host-key")?)?
        }
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
    }
    audit("invite", &format!("{id} until {}", iso(direct.expires)));
    let code = direct.encode();
    let mut invite = Invite {
        code: code.clone(),
        url: None,
        one_liner_sh: format!("cx join {code}"),
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
        invite.one_liner_sh = format!("cx join {host}/{shown}");
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

/// Remove the authorized_keys lines of invites that are no longer pending.
fn sweep(inv: &Invites) -> Result<usize> {
    let dead: Vec<String> = inv
        .invites
        .iter()
        .filter(|(_, r)| r.state != "pending")
        .map(|(k, _)| invite_tag(k))
        .collect();
    if dead.is_empty() {
        return Ok(0);
    }
    peer_ssh::remove_authorized(|l| {
        l.starts_with("restrict,command=") && dead.iter().any(|t| l.ends_with(t.as_str()))
    })
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

fn machine_id() -> String {
    // CX_JOIN_MACHINE_ID: lets the loopback fixture run two identities on one machine.
    std::env::var("CX_JOIN_MACHINE_ID")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| {
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
    peer_ssh::configure(peer, host_key, client_key)?;
    enroll_joined(peer).inspect_err(|_| {
        if !store::devices().is_ok_and(|d| d.iter().any(|d| d.id == peer.id)) {
            let _ = peer_ssh::unpin(&peer.id);
        }
    })
}

/// The forced command (`cx join --redeem <id>`), on the viewer, under sshd: stdin/stdout carry
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
            audit(
                "join-refused",
                &format!("{id}: {}", crate::files::display(&format!("{e:#}"))),
            );
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
    anyhow::ensure!(
        valid_name(&name) && valid_name(&req.host),
        "invalid device name"
    );
    anyhow::ensure!(valid_machine_id(&req.machine_id), "invalid machine id");
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
    }
    audit("join", &format!("{id}: {name} {target}"));
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
                return peer_ssh::ed25519_public(&key);
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

/// `cx join <code>`: redeem an invite on the viewer it names and pair both ways.
pub fn join(text: &str, opts: &JoinOptions) -> Result<Value> {
    let code = Direct::decode(&resolve(text)?)?;
    let now = transport::now();
    anyhow::ensure!(
        !code.expired(now),
        "this join code expired at {}; ask for a new one",
        iso(code.expires)
    );
    let local = store::local_device();
    let name = opts.name.clone().unwrap_or_else(|| local.name.clone());
    anyhow::ensure!(valid_name(&name), "invalid device name; pass --name");
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
            if let Ok(b) = fs::read(self.0.join("invite")) {
                let _ = fs::write(self.0.join("invite"), vec![0u8; b.len()]);
            }
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
                "warning": if addr.port() != 22 { Value::from("the viewer's sshd is not on port 22; cx pairing assumes 22, so add an SSH alias for it") } else { Value::Null },
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
        let pending = if !dir().exists() {
            0
        } else {
            let l = lock()?;
            let mut inv = read_invites(&l)?;
            expire(&mut inv, transport::now());
            inv.invites
                .values()
                .filter(|r| r.state == "pending")
                .count()
        };
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
    fn invite(&self, uses: u32, ttl_hours: f64, device_name: Option<&str>) -> Result<Invite> {
        anyhow::ensure!(uses == 1, "ssh-overlay invites are single use");
        create_invite(&InviteOptions {
            ttl_hours: Some(ttl_hours),
            device_name: device_name.map(str::to_owned),
            ..Default::default()
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
        let removed = store::remove_device(name)?;
        let unpinned = peer_ssh::unpin(&removed.id)?;
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
            n
        };
        audit("revoke", &removed.name);
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
            "restrict,command=\"/home/u/.local/bin/cx join --redeem 0123456789abcdef\",expiry-time=\"202701150800Z\" ssh-ed25519 AAAA cx-join-invite:0123456789abcdef"
        );
        assert!(l.ends_with(&invite_tag("0123456789abcdef")));
    }

    #[test]
    fn addresses_and_names() {
        assert_eq!(
            parse_address("10.0.0.2", 22).unwrap().to_string(),
            "10.0.0.2:22"
        );
        assert_eq!(parse_address("10.0.0.2:2222", 22).unwrap().port(), 2222);
        assert_eq!(parse_address("[fd00::1]:2200", 22).unwrap().port(), 2200);
        assert_eq!(parse_address("fd00::1", 2201).unwrap().port(), 2201);
        assert!(parse_address("host.example", 22).is_err());
        assert!(
            valid_name("lab-arm") && !valid_name("-x") && !valid_name("a b") && !valid_name("")
        );
        assert!(valid_machine_id("0123abc") && !valid_machine_id("a;b"));
    }

    #[test]
    fn short_code_references_resolve_shape() {
        assert!(resolve("mesh.example.org/not-a-code").is_err());
        assert_eq!(resolve("  ABCDE-12345 ").unwrap(), "ABCDE-12345");
    }
}
