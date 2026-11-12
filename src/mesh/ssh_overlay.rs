//! The ssh-overlay backend: the lightweight default mesh, built from what cx already has and able to
//! coexist with any VPN (it claims no address range). Members are the enrolled devices; core
//! devices pair automatically (peers.rs, peer_ssh.rs); reachability is the existing routes plus
//! ProxyJump through a reachable peer. New devices join with `cx join <code> <host[:port]>`
//! (docs/mesh.md has the protocol and its threat model):
//!
//! 1. `cx invite` on the viewer (the hub's `+`) makes an 11-symbol code (code.rs) and, in its
//!    foreground, listens on this host's mesh/LAN addresses until one device joins, the invite expires,
//!    or Ctrl-C. The code and its key live only in that process.
//! 2. `cx join <code> <host[:port]>` on the joiner connects there and sends its own keys, its
//!    identity, the address it dialed and a nonce, with HMAC-SHA256(SHA-256(code), "join", those fields).
//! 3. The viewer checks the HMAC (wrong codes are rate-limited and capped), marks the invite used,
//!    installs the joiner's key as `cx-managed-peer`, enrolls it as core, and answers with its own keys
//!    and HMAC(key, "accept", the nonce and those keys). Then the listener closes.
//! 4. The joiner verifies that answer (only a holder of the code can make it), then pins and authorizes
//!    the viewer the same way.
use super::{code, iso, AuditEntry, Invite, IpNet, Member, MeshBackend, Status};
use crate::{model::Device, peer_ssh, store, transport};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    fs,
    io::{BufReader, Read},
    net::{IpAddr, SocketAddr, TcpListener, TcpStream, ToSocketAddrs},
    os::unix::fs::DirBuilderExt,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

pub const ID: &str = "ssh-overlay";
/// 3: the one-shot invite listener (2 went through a shared door key, 1 carried an invite key in a long code).
const PROTOCOL: u32 = 3;
/// Where `cx invite` listens unless `--port` says otherwise.
pub const DEFAULT_PORT: u16 = 7241;
/// How long an invite stays open (the hub's and the CLI's): the 50-bit code must outlast no offline
/// guessing of an intercepted request (docs/mesh.md, threat model).
pub const DEFAULT_TTL: Duration = Duration::from_secs(30 * 60);
const MAX_TTL_HOURS: f64 = 24.0;
/// The port both sides' sshd must answer on: device targets carry no port.
const SSH_PORT: u16 = 22;
/// audit.jsonl holds at most 1000 short lines.
const AUDIT_LIMIT: u64 = 1024 * 1024;
/// Wrong codes: at most this many per window are checked, the rest refused unchecked...
const FAILURES_PER_WINDOW: usize = 3;
const FAILURE_WINDOW_SECS: u64 = 60;
/// ...and this many in all cancel the invite.
const MAX_FAILURES: usize = 20;
/// A request or answer frame (CX1 framing, transport.rs) is at most this long.
const MAX_FRAME: u64 = 64 * 1024;
/// How long the viewer waits for one connection's request, however slowly it trickles in.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// After answering, how long the rest of a request is drained before the connection closes.
const DRAIN_TIMEOUT: Duration = Duration::from_secs(1);
/// How often an invite with a cancel flag looks at it.
const CANCEL_POLL_MS: u64 = 200;
/// How long the joiner waits for the viewer's answer (the viewer pairs before it answers).
const ANSWER_TIMEOUT: Duration = Duration::from_secs(30);

pub struct SshOverlay;

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

/// Append to audit.jsonl (kept to its last 500..1000 entries), under the lock, so concurrent
/// processes (an invite and a revoke) never lose each other's lines.
fn audit(action: &str, detail: &str) {
    let _ = (|| -> Result<()> {
        let dir = ensure_dir()?;
        let _lock = store::private_lock(&dir, "lock", true)?;
        let line = json!({"at": iso(transport::now()), "actor": "local", "action": action, "detail": detail});
        let old = read_audit()?;
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
        store::atomic_write(&dir, "audit.jsonl", text.as_bytes())
    })();
}

fn read_audit() -> Result<String> {
    let bytes = store::read_private(&dir().join("audit.jsonl"), AUDIT_LIMIT)?;
    Ok(String::from_utf8_lossy(&bytes.unwrap_or_default()).into_owned())
}

/// Where an invite may listen: private LAN IPv4, the CGNAT range overlays use (Tailscale, WARP), IPv6 ULA,
/// and loopback (only when named with --address). Never a public or unspecified address.
fn mesh_or_lan(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v) => {
            v.is_private()
                || v.is_loopback()
                || super::ranges::net(super::ranges::TAILSCALE_V4).contains(&ip)
        }
        IpAddr::V6(v) => v.is_loopback() || v.segments()[0] & 0xfe00 == 0xfc00,
    }
}

/// The addresses to listen on, best first: the named ones, or this host's own mesh/LAN addresses.
fn listen_addresses(named: &[String]) -> Result<Vec<IpAddr>> {
    let mut ips = if named.is_empty() {
        crate::peers::local_addresses()?
            .into_iter()
            .filter(|ip| mesh_or_lan(*ip))
            .collect()
    } else {
        let mut ips = Vec::new();
        for a in named {
            let ip: IpAddr = a
                .trim_start_matches('[')
                .trim_end_matches(']')
                .parse()
                .with_context(|| format!("invalid address {a:?} (an IP)"))?;
            anyhow::ensure!(
                mesh_or_lan(ip),
                "{ip}: an invite listens only on mesh or LAN addresses (private, CGNAT, ULA or loopback)"
            );
            ips.push(ip);
        }
        ips
    };
    // Home and office LANs first; 172.16/12 last of IPv4, as container bridges (docker0) live there.
    let rank = |ip: &IpAddr| match ip {
        IpAddr::V4(v) if v.octets()[0] == 192 => 0,
        IpAddr::V4(v) if v.octets()[0] == 10 => 1,
        IpAddr::V4(v) if v.octets()[0] == 172 => 3,
        IpAddr::V4(_) => 2,
        IpAddr::V6(_) => 4,
    };
    ips.sort_by_key(|ip| (rank(ip), *ip));
    ips.dedup();
    anyhow::ensure!(
        !ips.is_empty(),
        "this host has no mesh or LAN address to listen on; pass --address"
    );
    Ok(ips)
}

#[derive(Clone, Debug, Default)]
pub struct InviteOptions {
    pub ttl_hours: Option<f64>,
    pub device_name: Option<String>,
    /// Addresses to listen on (IPs); default: this host's mesh/LAN addresses.
    pub addresses: Vec<String>,
    pub port: Option<u16>,
    /// Set from another thread to close the invite (the hub's Esc); checked every CANCEL_POLL_MS,
    /// also while a connection is being read.
    pub cancel: Option<Arc<AtomicBool>>,
}

pub(super) fn valid_name(n: &str) -> bool {
    !n.is_empty()
        && n.len() <= 63
        && !n.starts_with('-')
        && n.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|b| format!("{b:02x}")).collect()
}

/// The open invite, in the memory of the `cx mesh invite` serving it: nothing of it is ever stored.
struct Pending {
    key: [u8; 32],
    expires: u64,
    name: Option<String>,
    host_key: String,
    /// Set before anything is acted on: a request with the right code (or the failure cap) uses it up.
    used: bool,
    /// When recent requests failed the code check (bounded by MAX_FAILURES).
    failures: Vec<u64>,
    cancel: Option<Arc<AtomicBool>>,
}

impl Pending {
    /// Too many wrong codes just now: refuse without checking.
    fn rate_limited(&self, now: u64) -> bool {
        self.failures
            .iter()
            .filter(|&&t| now < t + FAILURE_WINDOW_SECS)
            .count()
            >= FAILURES_PER_WINDOW
    }

    /// Count a wrong code. The MAX_FAILURES-th cancels the invite; returns whether it did.
    fn note_failure(&mut self, now: u64) -> bool {
        self.failures.push(now);
        if self.failures.len() >= MAX_FAILURES {
            self.used = true;
        }
        self.used
    }
}

#[derive(Clone, Serialize, Deserialize)]
struct JoinRequest {
    version: u32,
    client_key: String,
    name: String,
    host: String,
    account: String,
    machine_id: String,
    host_key: String,
    /// The viewer address the joiner dialed (`ip:port`).
    viewer: String,
    /// 16 random bytes (hex): what the answer's HMAC is bound to.
    nonce: String,
    /// code::mac(key, "join", fields()).
    mac: String,
}

impl JoinRequest {
    fn fields(&self) -> [&[u8]; 8] {
        [
            self.client_key.as_bytes(),
            self.name.as_bytes(),
            self.host.as_bytes(),
            self.account.as_bytes(),
            self.machine_id.as_bytes(),
            self.host_key.as_bytes(),
            self.viewer.as_bytes(),
            self.nonce.as_bytes(),
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
    host_key: String,
}

/// What the viewer's answer authenticates: the request it answers (by its nonce) and everything it sends back.
fn accept_fields<'a>(nonce: &'a str, viewer: &'a Peer) -> [&'a [u8]; 7] {
    [
        nonce.as_bytes(),
        viewer.id.as_bytes(),
        viewer.name.as_bytes(),
        viewer.host.as_bytes(),
        viewer.account.as_bytes(),
        viewer.client_key.as_bytes(),
        viewer.host_key.as_bytes(),
    ]
}

/// The loopback fixture (tests/mesh_ssh_overlay.py) runs two identities on one machine. Debug builds
/// only, like CX_ADOPT_REPTYR.
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

/// The host key this viewer's sshd presents: /etc/ssh/ssh_host_ed25519_key.pub, as in every other
/// pairing. Debug builds take the loopback fixture's own (its private sshd) from CX_JOIN_HOST_KEY.
fn host_key() -> Result<String> {
    #[cfg(debug_assertions)]
    if let Some(p) = std::env::var_os("CX_JOIN_HOST_KEY").filter(|p| !p.is_empty()) {
        return peer_ssh::public_key(&fs::read_to_string(p)?);
    }
    peer_ssh::host_public_key().context("this host has no readable SSH Ed25519 host key")
}

/// sshd's port on both devices: 22, as pairing enrolls `account@host` with no port. Debug
/// builds take the loopback fixture's stand-in from CX_JOIN_SSH_PORT (CI runners need not run
/// sshd on 22).
fn sshd_port() -> u16 {
    #[cfg(debug_assertions)]
    if let Some(p) = std::env::var("CX_JOIN_SSH_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
    {
        return p;
    }
    SSH_PORT
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
/// so a request the viewer would refuse never uses up an invite.
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
    // Only a join that is enrolled lifts a removal (a refused one keeps the device out of auto-pairing).
    store::allow_explicit_enrollment(&peer.id)?;
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
    // A new core device: no edge records yet, so key access both ways (the edges.rs core default).
    peer_ssh::configure(peer, host_key, client_key, true)?;
    enroll_joined(peer).inspect_err(|_| {
        if !store::devices().is_ok_and(|d| d.iter().any(|d| d.id == peer.id)) {
            let _ = peer_ssh::unpin(&peer.id);
        }
    })
}

/// Reads a socket until a deadline, however slowly the peer sends: each read waits only for what is left.
/// With a cancel flag it looks at the flag every CANCEL_POLL_MS, so a closed invite never waits on a
/// connection.
struct Timed<'a> {
    stream: &'a TcpStream,
    until: Instant,
    cancel: Option<&'a AtomicBool>,
}

impl Read for Timed<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        loop {
            let left = self.until.saturating_duration_since(Instant::now());
            if left.is_zero() || self.cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
                return Err(std::io::ErrorKind::TimedOut.into());
            }
            let slice = match self.cancel {
                Some(_) => left.min(Duration::from_millis(CANCEL_POLL_MS)),
                None => left,
            };
            self.stream.set_read_timeout(Some(slice))?;
            match (&mut &*self.stream).read(buf) {
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) => {}
                r => return r,
            }
        }
    }
}

fn read_bounded<T: serde::de::DeserializeOwned>(
    stream: &TcpStream,
    wait: Duration,
    cancel: Option<&AtomicBool>,
) -> Result<T> {
    let timed = Timed {
        stream,
        until: Instant::now() + wait,
        cancel,
    };
    transport::read_frame(&mut BufReader::new(timed.take(MAX_FRAME)))
}

/// `cx mesh invite --backend ssh-overlay`: open an invite and serve it in the foreground. `ready` gets the
/// invite once the listeners are up (to print it). Returns the join, or an error once the invite is
/// expired or cancelled (a failed handshake with the right code, or too many wrong ones).
pub fn invite(opts: &InviteOptions, ready: impl FnOnce(&Invite) -> Result<()>) -> Result<Value> {
    let ttl = opts.ttl_hours.unwrap_or(DEFAULT_TTL.as_secs_f64() / 3600.0);
    anyhow::ensure!(
        ttl > 0.0 && ttl <= MAX_TTL_HOURS,
        "--hours must be within (0, {MAX_TTL_HOURS}]"
    );
    if let Some(n) = &opts.device_name {
        anyhow::ensure!(valid_name(n), "invalid device name");
    }
    let host_key = host_key()?;
    let port = opts.port.unwrap_or(DEFAULT_PORT);
    anyhow::ensure!(port != 0, "invalid --port 0");
    let mut listeners = Vec::new();
    for ip in listen_addresses(&opts.addresses)? {
        let l = TcpListener::bind((ip, port));
        // A named address must bind; of this host's own, those that can (an IPv6 address may be tentative).
        if l.is_err() && opts.addresses.is_empty() && !listeners.is_empty() {
            continue;
        }
        listeners.push(l.with_context(|| {
            format!(
                "cannot listen on {}:{port} (another invite open? pass --port)",
                bracket(ip)
            )
        })?);
    }
    let hosts: Vec<String> = listeners
        .iter()
        .map(|l| {
            let a = l.local_addr()?;
            Ok(if port == DEFAULT_PORT {
                bracket(a.ip())
            } else {
                format!("{}:{port}", bracket(a.ip()))
            })
        })
        .collect::<Result<_>>()?;
    let code = code::new_code()?;
    let expires = transport::now() + (ttl * 3600.0).ceil() as u64;
    let mut pending = Pending {
        key: code::key(&code),
        expires,
        name: opts.device_name.clone(),
        host_key,
        used: false,
        failures: Vec::new(),
        cancel: opts.cancel.clone(),
    };
    let shown = code::display(&code);
    let invite = Invite {
        one_liner_sh: format!("cx join {shown} {}", hosts[0]),
        code: shown,
        url: None,
        one_liner_ps: None,
        expires: iso(expires),
        uses: 1,
        backend: ID.into(),
        hosts: hosts.clone(),
    };
    audit(
        "invite",
        &format!("until {} on {}", iso(expires), hosts.join(" ")),
    );
    ready(&invite)?;
    let result = serve(&listeners, &mut pending);
    match &result {
        Ok(v) => audit(
            "join",
            &format!(
                "{} {}",
                v["name"].as_str().unwrap_or(""),
                v["target"].as_str().unwrap_or("")
            ),
        ),
        Err(e) => audit("invite-closed", &crate::files::display(&format!("{e:#}"))),
    }
    result
}

/// Accept connections one at a time until one joins, the invite expires or it is cancelled.
fn serve(listeners: &[TcpListener], p: &mut Pending) -> Result<Value> {
    use std::os::fd::AsRawFd;
    loop {
        let now = transport::now();
        anyhow::ensure!(now < p.expires, "the invite expired; nothing joined");
        let cancelled = p.cancel.as_ref().map(|c| c.load(Ordering::Relaxed));
        anyhow::ensure!(
            cancelled != Some(true),
            "the invite was closed; nothing joined"
        );
        let most = if cancelled.is_some() {
            CANCEL_POLL_MS
        } else {
            60_000
        };
        let wait = ((p.expires - now) * 1000).min(most) as libc::c_int;
        let mut fds: Vec<libc::pollfd> = listeners
            .iter()
            .map(|l| libc::pollfd {
                fd: l.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            })
            .collect();
        // SAFETY: fds is a live array of fds.len() pollfd entries.
        if unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, wait) } < 0 {
            let e = std::io::Error::last_os_error();
            if e.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(e).context("invite listener");
        }
        for (l, fd) in listeners.iter().zip(&fds) {
            if fd.revents & libc::POLLIN == 0 {
                continue;
            }
            let Ok((stream, _)) = l.accept() else {
                continue;
            };
            match handle(&stream, p) {
                Ok(Some(joined)) => return Ok(joined),
                Ok(None) => {}
                Err(e) => return Err(e.context("the invite is cancelled; make a new one")),
            }
        }
    }
}

/// One connection: read its request, answer it. `Ok(None)` is a refusal that leaves the invite open;
/// an error means the invite is used up without a join.
fn handle(stream: &TcpStream, p: &mut Pending) -> Result<Option<Value>> {
    stream.set_write_timeout(Some(REQUEST_TIMEOUT))?;
    let local = stream.local_addr()?;
    let source = stream.peer_addr()?.ip();
    let now = transport::now();
    // Read even a request that will be refused unchecked: closing on unread data would reset the
    // connection before the joiner reads why.
    let cancel = p.cancel.clone();
    let cancel = cancel.as_deref();
    // Only a wrong code counts as a failure: a connection that sends nothing readable (a port scan, a
    // health probe) guessed nothing, and one connection at a time with REQUEST_TIMEOUT bounds a flood.
    let outcome = match read_bounded::<JoinRequest>(stream, REQUEST_TIMEOUT, cancel) {
        _ if p.rate_limited(now) => Err(anyhow::anyhow!(
            "too many wrong pair codes just now; wait a minute and try again"
        )),
        Ok(req) => accept(p, &req, local, source, now),
        Err(e) => Err(e.context("unreadable join request")),
    };
    let (response, result) = match outcome {
        Ok((response, joined)) => (response, Ok(Some(joined))),
        Err(e) => {
            let response = JoinResponse {
                version: PROTOCOL,
                error: Some(format!("{e:#}")),
                ..Default::default()
            };
            (response, if p.used { Err(e) } else { Ok(None) })
        }
    };
    let _ = transport::frame(&mut &*stream, &response);
    // Closing on unread bytes (the rest of an oversized request) resets the connection, and a reset
    // can discard the answer before the joiner reads it: end our side, then drain the rest, bounded.
    let _ = stream.shutdown(std::net::Shutdown::Write);
    let rest = Timed {
        stream,
        until: Instant::now() + DRAIN_TIMEOUT,
        cancel,
    };
    let _ = std::io::copy(&mut rest.take(4 * MAX_FRAME), &mut std::io::sink());
    result
}

/// The viewer's checks of a request: the code (wrong ones counted), expiry and single use; then, the invite
/// marked used first, the dialed address and the identity. Returns the device to enroll.
fn check_request(
    p: &mut Pending,
    req: &JoinRequest,
    local: SocketAddr,
    source: IpAddr,
    now: u64,
) -> Result<Device> {
    anyhow::ensure!(now < p.expires && !p.used, "the invite expired or was used");
    if !code::verify(&p.key, "join", &req.fields(), &req.mac) {
        if p.note_failure(now) {
            bail!("{MAX_FAILURES} wrong pair codes");
        }
        bail!("wrong pair code");
    }
    p.used = true;
    anyhow::ensure!(
        req.version == PROTOCOL,
        "unsupported join protocol {} (this viewer speaks {PROTOCOL}; update cx on both ends)",
        req.version
    );
    // The joiner holds the code but dialed another address: something relayed the connection.
    anyhow::ensure!(
        req.viewer.parse::<SocketAddr>().ok() == Some(local),
        "the joiner dialed {} but reached {local} (a relay in between?)",
        crate::files::display(&req.viewer)
    );
    let name = p.name.clone().unwrap_or_else(|| req.name.clone());
    check_identity(&name, &req.host, &req.account, &req.machine_id)?;
    peer_ssh::public_key(&req.client_key)?;
    peer_ssh::public_key(&req.host_key)?;
    let target = format!("{}@{}", req.account, bracket(source));
    anyhow::ensure!(transport::valid_target(&target), "invalid joiner endpoint");
    let id = format!("{}:{}", req.machine_id, req.account);
    anyhow::ensure!(
        id != format!("{}:{}", machine_id(), store::local_device().account),
        "a device cannot join itself"
    );
    Ok(Device {
        id,
        name,
        target: Some(target),
        account: req.account.clone(),
        host: req.host.clone(),
        status: "joined by invite (reverse unverified)".into(),
        observed_at: now,
    })
}

/// A request with the right code: pair the joiner and make the answer.
fn accept(
    p: &mut Pending,
    req: &JoinRequest,
    local: SocketAddr,
    source: IpAddr,
    now: u64,
) -> Result<(JoinResponse, Value)> {
    let device = check_request(p, req, local, source, now)?;
    ensure_sshd(&source.to_string(), sshd_port()).context("the joining device")?;
    let name = pair_new(&device, &req.host_key, &req.client_key)?;
    let identity = peer_ssh::identity()?;
    let me = store::local_device();
    let viewer = Peer {
        id: format!("{}:{}", machine_id(), me.account),
        name: me.name,
        host: me.host,
        account: me.account,
        client_key: identity["client_key"]
            .as_str()
            .context("no client key")?
            .into(),
        host_key: p.host_key.clone(),
    };
    let mac = code::mac(&p.key, "accept", &accept_fields(&req.nonce, &viewer));
    let joined = json!({"joined": true, "backend": ID, "name": name, "target": device.target});
    Ok((
        JoinResponse {
            version: PROTOCOL,
            error: None,
            viewer: Some(viewer),
            mac: Some(mac),
        },
        joined,
    ))
}

/// `host[:port]`: an IP (`[v6]:port` with a port) or a DNS name, and the invite's port (default DEFAULT_PORT).
fn parse_host(text: &str) -> Result<(String, u16)> {
    if let Ok(a) = text.parse::<SocketAddr>() {
        return Ok((a.ip().to_string(), a.port()));
    }
    if let Ok(ip) = text
        .trim_start_matches('[')
        .trim_end_matches(']')
        .parse::<IpAddr>()
    {
        return Ok((ip.to_string(), DEFAULT_PORT));
    }
    let (h, p) = match text.rsplit_once(':') {
        Some((h, p)) => (h, p.parse().ok().context("invalid port")?),
        None => (text, DEFAULT_PORT),
    };
    anyhow::ensure!(
        !h.is_empty()
            && h.len() <= 253
            && !h.starts_with(['-', '.'])
            && h.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b".-".contains(&b)),
        "invalid host {:?} (an IP or a DNS name, then :port if the invite named one)",
        crate::files::display(h)
    );
    anyhow::ensure!(p != 0, "invalid port");
    Ok((h.to_owned(), p))
}

/// The joiner's check of the viewer's answer: only a holder of the code can make it, for this request only.
fn check_answer(key: &[u8; 32], req: &JoinRequest, response: JoinResponse) -> Result<Peer> {
    if let Some(e) = response.error {
        bail!("the viewer refused the join: {}", crate::files::display(&e));
    }
    let (Some(viewer), Some(tag)) = (response.viewer, response.mac) else {
        bail!("the viewer sent an incomplete answer");
    };
    anyhow::ensure!(
        code::verify(key, "accept", &accept_fields(&req.nonce, &viewer), &tag),
        "the answer does not prove it knows this code, so nothing was pinned. Cancel the invite \
         on the viewer (Ctrl-C there) and make a new one"
    );
    anyhow::ensure!(
        valid_name(&viewer.name) && valid_name(&viewer.host),
        "invalid viewer name"
    );
    anyhow::ensure!(
        viewer.id.ends_with(&format!(":{}", viewer.account))
            && viewer.id != format!("{}:{}", req.machine_id, req.account),
        "invalid viewer identity"
    );
    peer_ssh::public_key(&viewer.client_key)?;
    peer_ssh::public_key(&viewer.host_key)?;
    Ok(viewer)
}

fn connect(host: &str, port: u16) -> Result<TcpStream> {
    let mut last = None;
    for a in (host, port)
        .to_socket_addrs()
        .with_context(|| format!("cannot resolve {host}"))?
    {
        match TcpStream::connect_timeout(&a, Duration::from_secs(10)) {
            Ok(s) => return Ok(s),
            Err(e) => last = Some(e),
        }
    }
    Err(match last {
        Some(e) => anyhow::Error::new(e),
        None => anyhow::anyhow!("no address"),
    })
    .with_context(|| format!("no invite is open at {host}:{port} (run cx invite there)"))
}

/// Pairing enrolls `account@host` with no port, so sshd must answer on port 22 there; checked before an
/// invite is used up (a non-22 sshd needs an ssh_config `Host` alias with `Port`, enrolled with `cx add`).
fn ensure_sshd(host: &str, port: u16) -> Result<()> {
    for a in (host, port).to_socket_addrs()? {
        if TcpStream::connect_timeout(&a, Duration::from_secs(5)).is_ok() {
            return Ok(());
        }
    }
    bail!("sshd is not reachable at {host} port {port}; pairing needs sshd on port 22 on both devices")
}

#[derive(Clone, Debug, Default)]
pub struct JoinOptions {
    pub name: Option<String>,
}

/// `cx mesh pair <code> <host[:port]>`: join the invite open at `host` and pair both ways.
pub fn join(code_text: &str, host_text: &str, opts: &JoinOptions) -> Result<Value> {
    let code = code::parse(code_text)?;
    let (host, port) = parse_host(host_text)?;
    let local = store::local_device();
    let name = opts.name.clone().unwrap_or_else(|| local.name.clone());
    check_identity(&name, &local.host, &local.account, &machine_id())?;
    let identity = peer_ssh::identity()?;
    let key = code::key(&code);
    ensure_sshd(&host, sshd_port()).context("the viewer")?;
    let stream = connect(&host, port)?;
    stream.set_write_timeout(Some(ANSWER_TIMEOUT))?;
    let mut request = JoinRequest {
        version: PROTOCOL,
        client_key: identity["client_key"]
            .as_str()
            .context("no client key")?
            .into(),
        name: name.clone(),
        host: local.host.clone(),
        account: local.account.clone(),
        machine_id: machine_id(),
        host_key: identity["host_key"].as_str().context("no host key")?.into(),
        viewer: stream.peer_addr()?.to_string(),
        nonce: hex(&code::random::<16>()?),
        mac: String::new(),
    };
    request.mac = code::mac(&key, "join", &request.fields());
    transport::frame(&mut &stream, &request)?;
    let response = read_bounded(&stream, ANSWER_TIMEOUT, None).context(
        "the viewer did not answer; cancel the invite there (Ctrl-C) and make a new one",
    )?;
    drop(stream);
    let viewer = check_answer(&key, &request, response)?;
    let target = match host.parse::<IpAddr>() {
        Ok(ip) => format!("{}@{}", viewer.account, bracket(ip)),
        Err(_) => format!("{}@{host}", viewer.account),
    };
    anyhow::ensure!(transport::valid_target(&target), "invalid viewer endpoint");
    let device = Device {
        id: viewer.id.clone(),
        name: viewer.name.clone(),
        target: Some(target.clone()),
        account: viewer.account.clone(),
        host: viewer.host.clone(),
        status: "joined by invite (host key confirmed by code)".into(),
        observed_at: transport::now(),
    };
    let viewer_name = pair_new(&device, &viewer.host_key, &viewer.client_key).context(
        "the viewer enrolled this device, but pairing it here failed; revoke this device on the viewer, fix the cause, then join again",
    )?;
    Ok(json!({
        "joined": true,
        "backend": ID,
        "viewer": viewer_name,
        "target": target,
        "host_key_fingerprint": code::fingerprint(&viewer.host_key)?,
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
    fn status(&self) -> Result<Status> {
        let members = self
            .members()?
            .into_iter()
            .filter(|m| m.state != "local")
            .count();
        Ok(Status {
            backend: ID.into(),
            available: true,
            provisioned: true,
            members: Some(members),
            endpoint: None,
            detail: format!("{members} enrolled device(s); nothing to provision"),
        })
    }
    fn address_ranges(&self) -> Vec<IpNet> {
        Vec::new()
    }
}

impl SshOverlay {
    pub fn members(&self) -> Result<Vec<Member>> {
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
    pub fn revoke(&self, name: &str) -> Result<Value> {
        let (removed, unpinned) = peer_ssh::remove_device(name)?;
        audit("revoke", &removed.name);
        Ok(
            json!({"name": removed.name, "id": removed.id, "revoked": ["device", "ssh pins"], "ssh": unpinned}),
        )
    }
    pub fn audit(&self, limit: usize) -> Result<Vec<AuditEntry>> {
        let text = read_audit()?;
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
    use std::{io::Write, path::Path, process::Command};

    const VIEWER_HOST_KEY: &str =
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl";

    fn other_key(c: &str) -> String {
        VIEWER_HOST_KEY.replace("IOMq", &format!("IOM{c}"))
    }

    fn pending(code: &str, expires: u64) -> Pending {
        Pending {
            key: code::key(code),
            expires,
            name: None,
            host_key: VIEWER_HOST_KEY.into(),
            used: false,
            failures: Vec::new(),
            cancel: None,
        }
    }

    const LOCAL: &str = "192.168.1.5:7241";

    /// A request with a valid identity, signed with `code`.
    fn request(code: &str) -> JoinRequest {
        let mut r = JoinRequest {
            version: PROTOCOL,
            client_key: other_key("r"),
            name: "rover".into(),
            host: "rover".into(),
            account: "alice".into(),
            machine_id: "rover-machine".into(),
            host_key: other_key("s"),
            viewer: LOCAL.into(),
            nonce: "00112233445566778899aabbccddeeff".into(),
            mac: String::new(),
        };
        r.mac = code::mac(&code::key(code), "join", &r.fields());
        r
    }

    fn check(p: &mut Pending, r: &JoinRequest, now: u64) -> Result<Device> {
        check_request(
            p,
            r,
            LOCAL.parse().unwrap(),
            "192.168.1.9".parse().unwrap(),
            now,
        )
    }

    #[test]
    fn listens_only_on_mesh_and_lan_addresses() {
        for ip in [
            "10.0.0.2",
            "192.168.1.5",
            "172.16.0.1",
            "100.101.2.3",
            "127.0.0.1",
            "fd7a:115c::1",
            "::1",
        ] {
            assert!(mesh_or_lan(ip.parse().unwrap()), "{ip}");
        }
        for ip in [
            "8.8.8.8",
            "0.0.0.0",
            "::",
            "2001:db8::1",
            "fe80::1",
            "169.254.1.1",
        ] {
            assert!(!mesh_or_lan(ip.parse().unwrap()), "{ip}");
        }
        assert_eq!(
            listen_addresses(&[
                "127.0.0.1".into(),
                "[::1]".into(),
                "172.17.0.1".into(),
                "10.0.0.2".into(),
                "192.168.1.5".into()
            ])
            .unwrap(),
            ["192.168.1.5", "10.0.0.2", "127.0.0.1", "172.17.0.1", "::1"]
                .map(|s| s.parse::<IpAddr>().unwrap())
        );
        let e = listen_addresses(&["0.0.0.0".into()])
            .unwrap_err()
            .to_string();
        assert!(e.contains("mesh or LAN"), "{e}");
        assert!(listen_addresses(&["8.8.8.8".into()]).is_err());
        assert!(listen_addresses(&["host.example".into()]).is_err());
    }

    #[test]
    fn hosts_ports_and_identities() {
        let ok = |t: &str| parse_host(t).unwrap();
        assert_eq!(ok("10.0.0.2"), ("10.0.0.2".into(), DEFAULT_PORT));
        assert_eq!(ok("10.0.0.2:9000"), ("10.0.0.2".into(), 9000));
        assert_eq!(ok("fd00::1"), ("fd00::1".into(), DEFAULT_PORT));
        assert_eq!(ok("[fd00::1]:9000"), ("fd00::1".into(), 9000));
        assert_eq!(ok("rover.local"), ("rover.local".into(), DEFAULT_PORT));
        assert_eq!(ok("rover.local:9000").1, 9000);
        for bad in [
            "-oProxyCommand=x",
            "a@host",
            "ho st",
            "host:22x",
            "",
            ".x",
            "host:0",
            "host:",
        ] {
            assert!(parse_host(bad).is_err(), "{bad}");
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

    /// At most FAILURES_PER_WINDOW wrong codes a minute are checked, and MAX_FAILURES in all cancel the
    /// invite: an online guesser gets at most 20 tries at 50 bits.
    #[test]
    fn wrong_codes_are_rate_limited_and_capped() {
        let code = code::new_code().unwrap();
        let mut p = pending(&code, u64::MAX);
        let wrong = request(&code::new_code().unwrap());
        let mut t = 1000;
        for _ in 0..FAILURES_PER_WINDOW {
            assert!(!p.rate_limited(t));
            let e = check(&mut p, &wrong, t).unwrap_err().to_string();
            assert_eq!(e, "wrong pair code");
        }
        assert!(p.rate_limited(t));
        assert!(p.rate_limited(t + FAILURE_WINDOW_SECS - 1));
        assert!(!p.rate_limited(t + FAILURE_WINDOW_SECS));
        assert!(!p.used, "wrong codes leave the invite open");
        while !p.used {
            t += FAILURE_WINDOW_SECS;
            assert!(!p.rate_limited(t));
            let _ = check(&mut p, &wrong, t);
        }
        assert_eq!(p.failures.len(), MAX_FAILURES);
        let e = check(&mut p, &request(&code), t).unwrap_err().to_string();
        assert!(
            e.contains("was used"),
            "cancelled: even the right code is refused: {e}"
        );
    }

    /// The viewer's side of the code checks: a wrong code or a tampered field is refused and counted; an
    /// expired invite is refused; the right code uses the invite up before anything else (single use).
    #[test]
    fn requests_need_the_code_once_and_in_time() {
        let code = code::new_code().unwrap();
        let mut p = pending(&code, 2000);
        let refused = |p: &mut Pending, r: &JoinRequest| check(p, r, 1000).unwrap_err().to_string();
        assert_eq!(
            refused(&mut p, &request(&code::new_code().unwrap())),
            "wrong pair code"
        );
        let mut tampered = request(&code);
        tampered.client_key = other_key("t");
        assert_eq!(
            refused(&mut p, &tampered),
            "wrong pair code",
            "a swapped key fails the MAC"
        );
        let mut moved = request(&code);
        moved.viewer = "10.0.0.9:7241".into();
        assert_eq!(
            refused(&mut p, &moved),
            "wrong pair code",
            "so does a changed address"
        );
        assert!(!p.used);
        assert_eq!(p.failures.len(), 3);
        p.failures.clear();

        let d = check(&mut p, &request(&code), 1000).unwrap();
        assert!(p.used);
        assert_eq!(
            (d.id.as_str(), d.target.as_deref(), d.name.as_str()),
            ("rover-machine:alice", Some("alice@192.168.1.9"), "rover")
        );
        assert!(
            refused(&mut p, &request(&code)).contains("was used"),
            "single use"
        );

        let mut late = pending(&code, 2000);
        assert!(check(&mut late, &request(&code), 2000)
            .unwrap_err()
            .to_string()
            .contains("expired"));
        assert!(!late.used && late.failures.is_empty());

        // The right code from a relay (the joiner dialed another address) uses the invite up and is refused.
        let mut relayed = pending(&code, 2000);
        let mut r = request(&code);
        r.viewer = "10.0.0.9:7241".into();
        r.mac = code::mac(&code::key(&code), "join", &r.fields());
        assert!(check(&mut relayed, &r, 1000)
            .unwrap_err()
            .to_string()
            .contains("relay"));
        assert!(relayed.used);

        // So does a request the joiner should have refused itself (an invalid identity).
        let mut bad = pending(&code, 2000);
        let mut r = request(&code);
        r.machine_id = "bad id".into();
        r.mac = code::mac(&code::key(&code), "join", &r.fields());
        assert!(check(&mut bad, &r, 1000)
            .unwrap_err()
            .to_string()
            .contains("machine id"));
        assert!(bad.used);
    }

    fn answer(key: &[u8; 32], nonce: &str) -> JoinResponse {
        let viewer = Peer {
            id: "viewer-machine:alice".into(),
            name: "viewer".into(),
            host: "viewer".into(),
            account: "alice".into(),
            client_key: other_key("c"),
            host_key: VIEWER_HOST_KEY.into(),
        };
        let mac = code::mac(key, "accept", &accept_fields(nonce, &viewer));
        JoinResponse {
            version: PROTOCOL,
            error: None,
            viewer: Some(viewer),
            mac: Some(mac),
        }
    }

    /// The joiner's check of the answer: only a holder of the code can make it, for this request only.
    #[test]
    fn forged_answers_are_refused() {
        let code = code::new_code().unwrap();
        let key = code::key(&code);
        let req = request(&code);
        let refused = |r: JoinResponse| format!("{:#}", check_answer(&key, &req, r).err().unwrap());
        let v = check_answer(&key, &req, answer(&key, &req.nonce)).unwrap();
        assert_eq!(v.host_key, VIEWER_HOST_KEY);
        let forged = code::key(&code::new_code().unwrap());
        assert!(refused(answer(&forged, &req.nonce)).contains("nothing was pinned"));
        assert!(
            refused(answer(&key, "another nonce")).contains("nothing was pinned"),
            "replayed"
        );
        let mut swapped = answer(&key, &req.nonce);
        swapped.viewer.as_mut().unwrap().host_key = other_key("h");
        assert!(
            refused(swapped).contains("nothing was pinned"),
            "a swapped host key"
        );
        let mut swapped = answer(&key, &req.nonce);
        swapped.viewer.as_mut().unwrap().client_key = other_key("k");
        assert!(
            refused(swapped).contains("nothing was pinned"),
            "a swapped client key"
        );
        let mut unsigned = answer(&key, &req.nonce);
        unsigned.mac = None;
        assert!(refused(unsigned).contains("incomplete"));
        let e = JoinResponse {
            error: Some("wrong pair code".into()),
            ..Default::default()
        };
        assert!(refused(e).contains("refused the join: wrong pair code"));
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

    /// Separate processes appending at once (an invite during a revoke) keep every line, and a full log is
    /// trimmed to its newest half.
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
                audit("join", &format!("{n}.{i}"));
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
        fs::set_permissions(&path, std::os::unix::fs::PermissionsExt::from_mode(0o600)).unwrap();
        audit("invite", "x");
        let text = fs::read_to_string(&path).unwrap();
        assert_eq!(text.lines().count(), 500);
        assert!(text.starts_with("{\"n\":501}\n") && text.contains("\"invite\""));
    }

    /// The listener over real sockets: a wrong code is refused (and counted) with the invite left open, a
    /// slow or silent connection is cut off (not counted: it guessed nothing), and the listener answers and
    /// closes at expiry.
    #[test]
    fn the_listener_bounds_connections_and_closes_at_expiry() {
        let d = private_tempdir();
        let vars = scratch_state(d.path());
        if !super::super::in_child(
            "mesh::ssh_overlay::tests::the_listener_bounds_connections_and_closes_at_expiry",
            &env(&vars),
        ) {
            return;
        }
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        let code = code::new_code().unwrap();
        let mut p = pending(&code, transport::now() + 4);
        let server = std::thread::spawn(move || {
            let r = serve(&[l], &mut p);
            (r.map_err(|e| e.to_string()), p.failures.len())
        });
        let wrong = request(&code::new_code().unwrap());
        let s = TcpStream::connect(addr).unwrap();
        transport::frame(&mut &s, &wrong).unwrap();
        let r: JoinResponse = read_bounded(&s, Duration::from_secs(5), None).unwrap();
        assert_eq!(r.error.as_deref(), Some("wrong pair code"));
        drop(s);
        // An oversized frame is read only up to MAX_FRAME, and its answer still arrives (the rest is
        // drained, so closing does not reset the connection under it).
        let s = TcpStream::connect(addr).unwrap();
        let mut big = format!("CX1 {}\n", 2 * MAX_FRAME).into_bytes();
        big.resize(big.len() + 2 * MAX_FRAME as usize, b'x');
        let _ = (&s).write_all(&big);
        let r: JoinResponse = read_bounded(&s, Duration::from_secs(5), None).unwrap();
        assert!(r.error.unwrap().contains("unreadable"));
        drop(s);
        // A silent connection is cut off after REQUEST_TIMEOUT.
        let started = Instant::now();
        let s = TcpStream::connect(addr).unwrap();
        let r: JoinResponse = read_bounded(&s, REQUEST_TIMEOUT * 2, None).unwrap();
        assert!(r.error.unwrap().contains("unreadable"));
        let waited = started.elapsed();
        assert!(waited >= REQUEST_TIMEOUT - Duration::from_secs(1) && waited < REQUEST_TIMEOUT * 2);
        // Bare connects (a port scan) are not wrong codes either.
        for _ in 0..FAILURES_PER_WINDOW {
            drop(TcpStream::connect(addr).unwrap());
        }
        let (result, failures) = server.join().unwrap();
        assert_eq!(result.unwrap_err(), "the invite expired; nothing joined");
        assert_eq!(failures, 1, "only the wrong code counts");
        assert!(TcpStream::connect(addr).is_err(), "the port is closed");
    }

    /// The hub's Esc: a set cancel flag closes the listener within a poll, and its port with it, even
    /// while a silent connection is being read (else `+` right after Esc finds the port taken).
    #[test]
    fn a_cancelled_invite_closes_its_listener_promptly() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        let mut p = pending(&code::new_code().unwrap(), transport::now() + 600);
        let cancel = Arc::new(AtomicBool::new(false));
        p.cancel = Some(cancel.clone());
        let server = std::thread::spawn(move || serve(&[l], &mut p).map_err(|e| e.to_string()));
        let silent = TcpStream::connect(addr).unwrap();
        std::thread::sleep(Duration::from_millis(300));
        let started = Instant::now();
        cancel.store(true, Ordering::Relaxed);
        let r = server.join().unwrap();
        assert_eq!(r.unwrap_err(), "the invite was closed; nothing joined");
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "{:?}",
            started.elapsed()
        );
        assert!(TcpListener::bind(addr).is_ok(), "the port is free again");
        drop(silent);
    }

    /// An invite lasts DEFAULT_TTL (the hub's and the CLI's), at most a day, and its line is `cx join`.
    #[test]
    fn an_invite_is_short_lived_and_names_cx_join() {
        let d = private_tempdir();
        let key = d.path().join("host.pub");
        fs::write(&key, format!("{VIEWER_HOST_KEY}\n")).unwrap();
        let mut vars = scratch_state(d.path()).to_vec();
        vars.push(("CX_JOIN_HOST_KEY", key));
        if !super::super::in_child(
            "mesh::ssh_overlay::tests::an_invite_is_short_lived_and_names_cx_join",
            &env(&vars),
        ) {
            return;
        }
        let long = InviteOptions {
            ttl_hours: Some(25.0),
            ..Default::default()
        };
        let e = invite(&long, |_| Ok(())).unwrap_err().to_string();
        assert!(e.contains("--hours must be within (0, 24]"), "{e}");
        let port = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let cancel = Arc::new(AtomicBool::new(false));
        let opts = InviteOptions {
            addresses: vec!["127.0.0.1".into()],
            port: Some(port),
            cancel: Some(cancel.clone()),
            ..Default::default()
        };
        let mut shown = None;
        let r = invite(&opts, |inv| {
            shown = Some(inv.clone());
            cancel.store(true, Ordering::Relaxed);
            Ok(())
        });
        assert!(r.is_err());
        let inv = shown.unwrap();
        assert_eq!(
            inv.one_liner_sh,
            format!("cx join {} 127.0.0.1:{port}", inv.code)
        );
        let now = transport::now();
        assert!(
            inv.expires <= iso(now + DEFAULT_TTL.as_secs())
                && inv.expires > iso(now + DEFAULT_TTL.as_secs() - 60)
        );
    }

    #[test]
    fn pairing_checks_that_sshd_answers() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        ensure_sshd("127.0.0.1", port).unwrap();
        // Port 1 (tcpmux) has no listener; a freed ephemeral port may be taken again by a parallel test.
        let e = ensure_sshd("127.0.0.1", 1).unwrap_err().to_string();
        assert!(
            e.contains("sshd is not reachable at 127.0.0.1 port 1;"),
            "{e}"
        );
    }

    /// A refused join (its address already enrolled for another device) leaves a removed device's
    /// tombstone, so peers.rs still never re-pairs it on its own.
    #[test]
    fn a_refused_join_keeps_a_removal() {
        let d = private_tempdir();
        let vars = scratch_state(d.path());
        if !super::super::in_child(
            "mesh::ssh_overlay::tests::a_refused_join_keeps_a_removal",
            &env(&vars),
        ) {
            return;
        }
        let device = |id: &str, name: &str| Device {
            id: id.into(),
            name: name.into(),
            target: Some("alice@10.0.0.9".into()),
            account: "alice".into(),
            host: name.into(),
            status: String::new(),
            observed_at: 1,
        };
        let x = device("xm:alice", "rover");
        store::save_devices(std::slice::from_ref(&x)).unwrap();
        store::remove_device("rover").unwrap();
        assert!(store::peer_removed(&x.id).unwrap());
        store::save_devices(&[device("ym:alice", "other")]).unwrap();
        let e = pair_new(&x, VIEWER_HOST_KEY, &other_key("c"))
            .unwrap_err()
            .to_string();
        assert!(e.contains("already enrolled for another device"), "{e}");
        assert!(store::peer_removed(&x.id).unwrap(), "tombstone kept");
    }

    /// The joiner refuses an identity the viewer would refuse before it connects, so the invite
    /// is not used up (and nothing is generated or contacted).
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
        let started = Instant::now();
        // TEST-NET-1: never answers, so a missing check would show up as a timeout, not a pass.
        let e = join(&code, "192.0.2.1", &JoinOptions::default()).unwrap_err();
        assert!(e.to_string().contains("machine id"), "{e:#}");
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(
            !store::state_dir().join("peer-ssh").exists(),
            "no identity generated"
        );
        let e = join("ABCD", "192.0.2.1", &JoinOptions::default()).unwrap_err();
        assert!(e.to_string().contains("11 characters"), "{e:#}");
    }
}
