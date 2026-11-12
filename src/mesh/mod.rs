//! `cx mesh`: runs the standalone mesh util (mesh/mesh.py, docs/mesh.md) from a copy embedded in this binary,
//! so it works wherever cx is installed; the util itself needs only python3 and no cx. `cx claude` runs its
//! neighbour mesh/claude.py (docs/claude.md), whose codes the same Worker serves.
//!
//! The mesh is a pluggable feature: [`MeshBackend`] has three implementations, listed by
//! [`backends`] (`cx mesh backends`):
//! - `ssh-overlay` (ssh_overlay.rs): the built-in default over plain SSH, coexisting with any VPN;
//!   devices join with `cx mesh pair <code> <host>` while `cx mesh invite` listens (code.rs, the protocol
//!   and threat model in docs/mesh.md).
//! - `cloudflare-warp` (cloudflare.rs): the WARP mesh of mesh.py, through its `--json` contract.
//! - `tailscale` (tailscale.rs): read-only adoption of `tailscale status --json`.
//!
//! [`overlay`] feeds network.rs from local evidence only: which interfaces sit in overlay ranges,
//! and range collisions (ranges.rs) between overlays and LANs.
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    io::{Read, Write},
    path::PathBuf,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

pub mod cloudflare;
pub mod code;
pub mod ranges;
pub mod ssh_overlay;
pub mod tailscale;

pub use ranges::IpNet;

const FILES: &[(&str, &str)] = &[
    ("mesh.py", include_str!("../../mesh/mesh.py")),
    ("claude.py", include_str!("../../mesh/claude.py")),
    (
        "worker/worker.js",
        include_str!("../../mesh/worker/worker.js"),
    ),
    (
        "worker/schema.sql",
        include_str!("../../mesh/worker/schema.sql"),
    ),
    ("worker/join.sh", include_str!("../../mesh/worker/join.sh")),
    (
        "worker/leave.sh",
        include_str!("../../mesh/worker/leave.sh"),
    ),
    (
        "worker/join.ps1",
        include_str!("../../mesh/worker/join.ps1"),
    ),
    (
        "worker/leave.ps1",
        include_str!("../../mesh/worker/leave.ps1"),
    ),
    (
        "worker/claude.sh",
        include_str!("../../mesh/worker/claude.sh"),
    ),
    (
        "worker/claude.ps1",
        include_str!("../../mesh/worker/claude.ps1"),
    ),
];

fn unpack() -> Result<PathBuf> {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
        .context("HOME is not set")?;
    let dir = base
        .join("cx")
        .join(format!("mesh-{}", env!("CARGO_PKG_VERSION")));
    for (name, body) in FILES {
        let path = dir.join(name);
        if std::fs::read_to_string(&path).ok().as_deref() != Some(*body) {
            std::fs::create_dir_all(path.parent().unwrap())?;
            std::fs::write(&path, body)?;
        }
    }
    Ok(dir.join("mesh.py"))
}

/// `cx mesh ARGS`: `backends`, `ranges`, `pair` and any subcommand with `--backend ID` are handled
/// here; everything else goes to mesh.py unchanged (its CLI and `--json` contract are frozen).
pub fn run(args: &[String]) -> Result<()> {
    if args.first().map(String::as_str) == Some("pair") {
        return pair(&args[1..]);
    }
    if args.first().map(String::as_str) == Some("backends") {
        let json = args.iter().any(|a| a == "--json");
        let r = report();
        if json {
            println!("{}", serde_json::to_string_pretty(&r)?);
        } else {
            print_report(&r);
        }
        return Ok(());
    }
    if args.first().map(String::as_str) == Some("ranges") {
        let r = report();
        println!(
            "{}",
            serde_json::to_string_pretty(
                &json!({"claims": r["claims"], "collisions": r["collisions"]})
            )?
        );
        return Ok(());
    }
    if args
        .iter()
        .any(|a| a == "--backend" || a.starts_with("--backend="))
    {
        return run_backend(args);
    }
    python(unpack()?, args, MESH_API)
}

pub fn run_claude(args: &[String]) -> Result<()> {
    python(unpack()?.with_file_name("claude.py"), args, CLAUDE_API)
}

/// mesh.py subcommands that call the Cloudflare API and so get the stored token. `join` and `leave` are left out
/// on purpose: they pipe a script fetched from a (possibly foreign) URL into `sh`, which inherits the environment.
const MESH_API: &[&str] = &[
    "setup", "invite", "devices", "deploy", "audit", "profile", "revoke", "teardown",
];
/// claude.py subcommands; each builds a Mesh for the Access API. claude.py runs the configured `gateway` command
/// with CLOUDFLARE_API_TOKEN removed from its environment.
const CLAUDE_API: &[&str] = &["lend", "revoke", "list"];

fn python(script: PathBuf, args: &[String], api: &[&str]) -> Result<()> {
    let mut command = std::process::Command::new("python3");
    command.arg(&script).args(args);
    if subcommand(args).is_some_and(|s| api.contains(&s)) && !names_config(args) {
        inject_token(&mut command)?;
    }
    let status = command
        .status()
        .context("cx mesh and cx claude need python3")?;
    std::process::exit(status.code().unwrap_or(1));
}

/// The subcommand argparse will pick, read conservatively: only the util's own leading options (`--json`,
/// `--config V`, `--config=V`) are skipped; any other leading option (an abbreviation such as `--conf`, `-h`,
/// `--`) gives None, so an argv this cannot read never gets the token.
fn subcommand(args: &[String]) -> Option<&str> {
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--json" => {}
            "--config" => {
                it.next()?;
            }
            a if a.starts_with("--config=") => {}
            a if a.starts_with('-') => return None,
            a => return Some(a),
        }
    }
    None
}

/// Whether argv picks a mesh config with `--config`. The stored token belongs to the default mesh (the one
/// `cx secrets migrate` imports from); another config keeps the `credentials` file next to it.
fn names_config(args: &[String]) -> bool {
    args.iter()
        .any(|a| a == "--config" || a.starts_with("--config="))
}

/// Whether the cloudflare-warp backend's own mesh.py call (always the default config) gets the stored token.
fn mesh_api(subcommand: &str) -> bool {
    MESH_API.contains(&subcommand)
}

/// Hand a stored `cloudflare.api_token` (docs/secrets.md) to the util as $CLOUDFLARE_API_TOKEN, the variable it
/// already reads first, so mesh.py stays standalone. An explicit $CLOUDFLARE_API_TOKEN wins; with no stored token
/// the util keeps its own lookup (credentials file, then prompt). A stored token that cannot be read here is an
/// error: falling through would prompt and save the token in plaintext again.
fn inject_token(command: &mut std::process::Command) -> Result<()> {
    if std::env::var_os("CLOUDFLARE_API_TOKEN").is_some() {
        return Ok(());
    }
    let name = crate::secrets::CLOUDFLARE_TOKEN;
    let token = crate::secrets::Secrets::open()
        .and_then(|s| s.try_get(name))
        .with_context(|| {
            format!(
                "stored {name} is unavailable here; export CLOUDFLARE_API_TOKEN or run \
                 `cx secrets get {name} --exec -- cx mesh ...` where it is reachable"
            )
        })?;
    if let Some(token) = token {
        command.env("CLOUDFLARE_API_TOKEN", token.expose());
    }
    Ok(())
}

// ---- the backend contract

/// Cheap, local status: what a backend can say without the network, unless noted.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Status {
    pub backend: String,
    /// The backend's tooling exists here (binary, python3, ...).
    pub available: bool,
    /// The mesh exists (deployed / running); not a claim about any member's reachability.
    pub provisioned: bool,
    /// None when counting needs the network (cloudflare-warp: use members()).
    pub members: Option<usize>,
    pub endpoint: Option<String>,
    pub detail: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Plan {
    pub dry_run: bool,
    pub applied: bool,
    pub steps: Vec<String>,
    pub result: Value,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Invite {
    pub backend: String,
    /// What the new device types (with `cx mesh pair <code> <host>` for ssh-overlay).
    pub code: String,
    pub url: Option<String>,
    pub one_liner_sh: String,
    pub one_liner_ps: Option<String>,
    /// RFC 3339 UTC.
    pub expires: String,
    pub uses: u32,
    /// ssh-overlay: where the invite listens (`ip` or `ip:port`), best first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hosts: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Member {
    pub name: String,
    pub host: String,
    pub os: Option<String>,
    pub overlay_ipv4: Option<String>,
    pub overlay_ipv6: Option<String>,
    pub dns_name: Option<String>,
    /// Evidence as the backend reports it (e.g. "active", "online", a cx device status).
    pub state: String,
    pub last_seen: Option<String>,
    /// "core" / "directed" (ssh-overlay), "device" / "connector" (warp), "self" / "peer" (tailscale).
    pub role: Option<String>,
    /// ssh-overlay: the ProxyJump hops of the device's route, when it has one.
    pub via: Option<Vec<String>>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct AuditEntry {
    pub at: String,
    pub actor: String,
    pub action: String,
    pub detail: String,
}

pub trait MeshBackend {
    fn id(&self) -> &'static str;
    fn name(&self) -> &'static str;
    /// Feature flags: "invite", "join", "revoke", "audit", "provision", "teardown", "lend",
    /// "onboarding", "read-only", "short-code", "coexists-with-vpn", ...
    fn capabilities(&self) -> Vec<&'static str>;
    fn status(&self) -> Result<Status>;
    fn provision(&self, dry_run: bool) -> Result<Plan>;
    fn teardown(&self, dry_run: bool) -> Result<Plan>;
    fn members(&self) -> Result<Vec<Member>>;
    fn revoke(&self, name: &str) -> Result<Value>;
    /// The ranges this overlay assigns addresses from (empty: it claims none).
    fn address_ranges(&self) -> Vec<IpNet>;
    fn audit(&self, limit: usize) -> Result<Vec<AuditEntry>>;
}

/// Every backend, the default (ssh-overlay) first.
pub fn backends() -> Vec<Box<dyn MeshBackend>> {
    vec![
        Box::new(ssh_overlay::SshOverlay),
        Box::new(cloudflare::CloudflareWarp),
        Box::new(tailscale::Tailscale::new()),
    ]
}

pub fn backend(id: &str) -> Result<Box<dyn MeshBackend>> {
    backends()
        .into_iter()
        .find(|b| b.id() == id)
        .with_context(|| format!("unknown mesh backend {id:?} (cx mesh backends lists them)"))
}

/// RFC 3339 UTC for unix seconds (civil-from-days; no time crate).
pub fn iso(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

/// Run a child with `input` on stdin; stdout bounded to `limit`; killed at `timeout`.
/// stderr is discarded (it may carry banners or secrets-adjacent noise).
///
/// The child leads its own process group, so a timeout kills everything it spawned (mesh.py's
/// curl, wrangler, ...), not only the direct child. Being outside the terminal's group, it would
/// miss a Ctrl+C aimed at cx, so on Linux it also gets SIGTERM when cx goes away.
pub(crate) fn run_bounded(
    mut cmd: Command,
    input: &[u8],
    limit: usize,
    timeout: Duration,
) -> Result<(bool, Vec<u8>)> {
    use std::os::unix::process::CommandExt;
    cmd.process_group(0);
    #[cfg(target_os = "linux")]
    // SAFETY: prctl is async-signal-safe and touches no memory of the parent.
    unsafe {
        cmd.pre_exec(|| {
            libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
            Ok(())
        });
    }
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let mut stdin = child.stdin.take().context("no stdin")?;
    let input = input.to_vec();
    // Not joined: a group member that holds stdin without reading would block it; the group kill
    // below closes the pipe and ends the thread.
    std::thread::spawn(move || {
        let _ = stdin.write_all(&input);
    });
    let mut stdout = child.stdout.take().context("no stdout")?;
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut b = Vec::new();
        let r = (&mut stdout).take(limit as u64 + 1).read_to_end(&mut b);
        let _ = tx.send((r, b));
    });
    let deadline = Instant::now() + timeout;
    let group = child.id() as libc::pid_t;
    // SAFETY: plain syscall; the group id is the child's pid (process_group(0) above). After the
    // leader is reaped the id stays reserved while any member of the group remains.
    let kill_group = || unsafe {
        libc::killpg(group, libc::SIGKILL);
    };
    let status = loop {
        if let Some(s) = child.try_wait()? {
            break s;
        }
        if Instant::now() >= deadline {
            kill_group();
            let _ = child.kill();
            let _ = child.wait();
            bail!("timed out");
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    // Whatever the child left running in its group (a lingering helper still holding stdout)
    // goes with it, so the output is complete and nothing outlives the call.
    kill_group();
    let (r, b) = rx.recv_timeout(Duration::from_secs(2))?;
    r?;
    anyhow::ensure!(b.len() <= limit, "output too large");
    Ok((status.success(), b))
}

// ---- reports for the CLI, the UI and network.rs

fn backend_value(b: &dyn MeshBackend) -> Value {
    let status = b.status();
    json!({
        "id": b.id(),
        "name": b.name(),
        "capabilities": b.capabilities(),
        "ranges": b.address_ranges(),
        "status": status.as_ref().ok(),
        "error": status.as_ref().err().map(|e| crate::files::display(&format!("{e:#}"))),
    })
}

fn interfaces_json() -> Value {
    crate::network::bounded_command("ip", &["-j", "address", "show"])
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(Value::Array(Vec::new()))
}

/// The overlay an interface address belongs to. WARP's 100.96/12 sits inside Tailscale's
/// 100.64/10, so the interface name decides first (when its overlay claims the address), then
/// the most specific range.
fn attribute(claims: &[ranges::Claim], ifname: &str, ip: &std::net::IpAddr) -> Option<String> {
    let n = ifname.to_ascii_lowercase();
    let hint = if n.starts_with("tailscale") || n == "utun-tailscale" {
        Some(tailscale::ID)
    } else if n.contains("warp") {
        Some(cloudflare::ID)
    } else {
        None
    };
    hint.filter(|h| claims.iter().any(|c| c.owner == *h && c.range.contains(ip)))
        .map(str::to_owned)
        .or_else(|| ranges::classify(claims, ip).map(|c| c.owner.clone()))
}

/// Claims of every overlay that is present here (configured, installed, or with an interface in
/// its ranges), plus the observed LANs.
fn claims(backends: &[Value], interfaces: &Value) -> (Vec<ranges::Claim>, Vec<ranges::Claim>) {
    let mut all = Vec::new();
    let mut present = Vec::new();
    for b in backends {
        let id = b["id"].as_str().unwrap_or_default().to_owned();
        if b["status"]["provisioned"] == true
            || b["status"]["available"] == true && id != ssh_overlay::ID
        {
            present.push(id.clone());
        }
        for r in b["ranges"].as_array().into_iter().flatten() {
            if let Some(range) = r.as_str().and_then(|s| s.parse::<IpNet>().ok()) {
                all.push(ranges::Claim {
                    owner: id.clone(),
                    range,
                });
            }
        }
    }
    // An interface address in overlay ranges makes that overlay observed.
    for item in interfaces.as_array().into_iter().flatten() {
        let ifname = item["ifname"].as_str().unwrap_or_default();
        for a in item["addr_info"].as_array().into_iter().flatten() {
            let Some(ip) = a["local"].as_str().and_then(|s| s.parse().ok()) else {
                continue;
            };
            if let Some(owner) = attribute(&all, ifname, &ip) {
                if !present.contains(&owner) {
                    present.push(owner);
                }
            }
        }
    }
    let overlays: Vec<_> = all
        .into_iter()
        .filter(|c| present.contains(&c.owner))
        .collect();
    let lans = ranges::lans_from_ip_json(interfaces, &overlays);
    (overlays, lans)
}

/// `cx mesh backends --json`: each backend with its status, plus range claims and collisions.
pub fn report() -> Value {
    report_with(&backends(), &interfaces_json())
}

pub fn report_with(list: &[Box<dyn MeshBackend>], interfaces: &Value) -> Value {
    let values: Vec<Value> = list.iter().map(|b| backend_value(b.as_ref())).collect();
    let (overlays, lans) = claims(&values, interfaces);
    let all: Vec<_> = overlays.iter().chain(lans.iter()).cloned().collect();
    json!({
        "default": ssh_overlay::ID,
        "backends": values,
        "claims": all,
        "collisions": ranges::collisions(&all),
    })
}

/// network.rs: the `overlay` field's state and the details behind it, from `ip -j address`.
/// state: "observed" (an interface address sits in an overlay range), "configured" (a WARP mesh
/// is deployed, no interface seen), or "none".
///
/// Every network observation (local and remote helpers) runs this, so it uses local evidence
/// only and never a backend's status(): WARP's config and state files (cloudflare::quick_status)
/// and `tailscale` on PATH, with no subprocess and no lock. ssh-overlay claims no range, so it
/// takes no part; `cx mesh backends` reports the full status.
pub fn overlay(interfaces: &Value) -> (String, Value) {
    let q = cloudflare::quick_status();
    let present = [
        json!({"id": cloudflare::ID, "ranges": q.mesh_ranges,
               "status": {"available": q.configured, "provisioned": q.provisioned}}),
        json!({"id": tailscale::ID, "ranges": tailscale::ranges(),
               "status": {"available": tailscale::find().is_some(), "provisioned": false}}),
    ];
    overlay_with(&present, interfaces)
}

/// [`overlay`] for backends given as `{id, ranges, status: {available, provisioned}}`.
fn overlay_with(values: &[Value], interfaces: &Value) -> (String, Value) {
    let (overlays, lans) = claims(values, interfaces);
    let mut on = Vec::new();
    for item in interfaces.as_array().into_iter().flatten() {
        for a in item["addr_info"].as_array().into_iter().flatten() {
            let Some(ip) = a["local"].as_str().and_then(|s| s.parse().ok()) else {
                continue;
            };
            let ifname = item["ifname"].as_str().unwrap_or_default();
            if let Some(owner) = attribute(&overlays, ifname, &ip) {
                on.push(json!({"ifname": ifname, "address": ip.to_string(), "backend": owner}));
            }
        }
    }
    let configured = values.iter().any(|b| {
        b["status"]["provisioned"] == true && b["ranges"].as_array().is_some_and(|r| !r.is_empty())
    });
    let state = if !on.is_empty() {
        "observed"
    } else if configured {
        "configured"
    } else {
        "none"
    };
    let all: Vec<_> = overlays.iter().chain(lans.iter()).cloned().collect();
    (
        state.into(),
        json!({"state": state, "backends": values, "interfaces": on, "claims": all, "collisions": ranges::collisions(&all)}),
    )
}

fn print_report(r: &Value) {
    for b in r["backends"].as_array().into_iter().flatten() {
        let s = &b["status"];
        let detail = s["detail"].as_str().or(b["error"].as_str()).unwrap_or("");
        let mark = if s["provisioned"] == true {
            "●"
        } else if s["available"] == true {
            "○"
        } else {
            "·"
        };
        println!(
            "{mark} {:16} {:28} {detail}",
            b["id"].as_str().unwrap_or(""),
            b["name"].as_str().unwrap_or("")
        );
    }
    for c in r["collisions"].as_array().into_iter().flatten() {
        println!("! {}", c["detail"].as_str().unwrap_or(""));
    }
}

/// `cx mesh SUB --backend ID ...`: the part of `cx mesh` handled here. Unknown flags are refused.
#[derive(clap::Parser)]
#[command(name = "cx mesh", no_binary_name = true)]
struct BackendArgs {
    /// status, members, invite, revoke, audit, ranges, setup or teardown
    #[arg(default_value = "status")]
    sub: String,
    /// revoke: the device
    device: Option<String>,
    #[arg(long)]
    backend: String,
    #[arg(long)]
    hours: Option<f64>,
    #[arg(long)]
    uses: Option<u32>,
    #[arg(long)]
    name: Option<String>,
    /// ssh-overlay invite: an address to listen on (repeatable; default: this host's mesh/LAN addresses).
    #[arg(long)]
    address: Vec<String>,
    /// ssh-overlay invite: the port to listen on.
    #[arg(long)]
    port: Option<u16>,
    /// ssh-overlay invite: the host key to pin, when sshd does not use the default one.
    #[arg(long)]
    host_key: Option<PathBuf>,
    #[arg(long)]
    limit: Option<usize>,
    #[arg(long)]
    dry_run: bool,
    #[arg(long)]
    yes: bool,
    #[arg(long)]
    json: bool,
}

fn run_backend(args: &[String]) -> Result<()> {
    use clap::Parser;
    let a = BackendArgs::try_parse_from(args).unwrap_or_else(|e| e.exit());
    if a.sub == "invite" && a.backend == ssh_overlay::ID {
        return ssh_overlay_invite(&a);
    }
    let out = backend_command(&a)?;
    if a.json {
        println!("{}", serde_json::to_string_pretty(&out)?);
    } else if a.sub == "invite" {
        println!(
            "On the new device:  {}",
            out["one_liner_sh"].as_str().unwrap_or("")
        );
        if let Some(ps) = out["one_liner_ps"].as_str() {
            println!("Windows:            {ps}");
        }
        println!(
            "expires {} ({} use)",
            out["expires"].as_str().unwrap_or(""),
            out["uses"]
        );
    } else {
        println!("{}", serde_json::to_string_pretty(&out)?);
    }
    Ok(())
}

/// `cx mesh invite --backend ssh-overlay`: print the invite, then serve it in the foreground until one
/// device joins, it expires, or Ctrl-C.
fn ssh_overlay_invite(a: &BackendArgs) -> Result<()> {
    anyhow::ensure!(
        a.uses.is_none_or(|u| u == 1),
        "ssh-overlay invites are single use"
    );
    let opts = ssh_overlay::InviteOptions {
        ttl_hours: a.hours,
        device_name: a.name.clone(),
        addresses: a.address.clone(),
        port: a.port,
        host_key_file: a.host_key.clone(),
        cancel: None,
    };
    let joined = ssh_overlay::invite(&opts, |inv| {
        let mut out = std::io::stdout().lock();
        if a.json {
            writeln!(out, "{}", serde_json::to_string(inv)?)?;
        } else {
            writeln!(
                out,
                "Pair code {} (single use, until {})",
                inv.code, inv.expires
            )?;
            writeln!(out, "On the new device:  {}", inv.one_liner_sh)?;
            if inv.hosts.len() > 1 {
                writeln!(out, "Also listening on:  {}", inv.hosts[1..].join(" "))?;
            }
            writeln!(out, "Waiting for it; Ctrl-C cancels the invite.")?;
        }
        out.flush()?;
        Ok(())
    })?;
    if a.json {
        println!("{}", serde_json::to_string(&joined)?);
    } else {
        println!(
            "joined: {} ({})",
            joined["name"].as_str().unwrap_or(""),
            joined["target"].as_str().unwrap_or("")
        );
    }
    Ok(())
}

fn backend_command(a: &BackendArgs) -> Result<Value> {
    let b = backend(&a.backend)?;
    anyhow::ensure!(
        a.device.is_none() || a.sub == "revoke",
        "cx mesh {}: unexpected argument {:?}",
        a.sub,
        a.device.as_deref().unwrap_or_default()
    );
    Ok(match a.sub.as_str() {
        "status" => serde_json::to_value(b.status()?)?,
        "members" | "devices" => serde_json::to_value(b.members()?)?,
        "audit" => serde_json::to_value(b.audit(a.limit.unwrap_or(50))?)?,
        "ranges" => serde_json::to_value(b.address_ranges())?,
        "setup" | "provision" => serde_json::to_value(b.provision(a.dry_run)?)?,
        "teardown" => {
            anyhow::ensure!(
                a.dry_run || a.yes,
                "not confirmed (pass --yes, or --dry-run to preview)"
            );
            serde_json::to_value(b.teardown(a.dry_run)?)?
        }
        "revoke" => {
            let name = a.device.as_deref().context("revoke NAME")?;
            anyhow::ensure!(a.yes, "not confirmed (pass --yes)");
            b.revoke(name)?
        }
        "invite" => serde_json::to_value(match b.id() {
            cloudflare::ID => cloudflare::invite(
                a.uses.unwrap_or(1),
                a.hours.unwrap_or(24.0),
                a.name.as_deref(),
            )?,
            _ => bail!("tailscale is adopted read-only; invite devices from the Tailscale admin console"),
        })?,
        other => bail!("cx mesh {other} --backend: use status, members, invite, revoke, audit, ranges, setup or teardown"),
    })
}

/// `cx mesh pair <code> <host[:port]> [--name N]`: join an invite `cx mesh invite --backend ssh-overlay` serves.
#[derive(clap::Parser)]
#[command(name = "cx mesh pair", no_binary_name = true)]
struct PairArgs {
    /// The code `cx mesh invite --backend ssh-overlay` printed.
    code: String,
    /// Where that invite listens: an IP or DNS name, with `:port` if it printed one.
    host: String,
    /// The name this device gets on the viewer (default: the hostname).
    #[arg(long)]
    name: Option<String>,
    #[arg(long)]
    json: bool,
}

fn pair(args: &[String]) -> Result<()> {
    use clap::Parser;
    let a = PairArgs::try_parse_from(args).unwrap_or_else(|e| e.exit());
    let r = ssh_overlay::join(
        &a.code,
        &a.host,
        &ssh_overlay::JoinOptions {
            name: a.name.clone(),
        },
    )?;
    if a.json {
        println!("{}", serde_json::to_string_pretty(&r)?);
    } else {
        println!(
            "joined {} as {} (host key verified against the code)",
            r["target"].as_str().unwrap_or(""),
            r["as"].as_str().unwrap_or("")
        );
        println!("next: {}", r["next"].as_str().unwrap_or(""));
    }
    Ok(())
}

/// Tests that need HOME, XDG dirs or PATH of their own run again as a child of this test binary,
/// with `env` set there, so this process's environment (shared by every test thread) is untouched.
/// Returns true in the child, which then does the work; in the parent it runs the child, asserts
/// that it passed, and returns false.
#[cfg(test)]
pub(crate) fn in_child(test: &str, env: &[(&str, &std::path::Path)]) -> bool {
    if std::env::var_os("CX_TEST_CHILD").is_some() {
        return true;
    }
    let mut cmd = Command::new(std::env::current_exe().unwrap());
    cmd.args([test, "--exact", "--test-threads=1"])
        .env("CX_TEST_CHILD", "1");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success() && text.contains("1 passed"),
        "{test} failed in its child:\n{text}{}",
        String::from_utf8_lossy(&out.stderr)
    );
    false
}

#[cfg(test)]
mod tests {
    #[test]
    fn iso_formats_utc() {
        assert_eq!(super::iso(0), "1970-01-01T00:00:00Z");
        assert_eq!(super::iso(1_800_000_000), "2027-01-15T08:00:00Z");
        assert_eq!(super::iso(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(super::iso(1_793_836_799), "2026-11-04T23:59:59Z");
    }

    /// A backend with one range; `provisioned` is its evidence (available and provisioned alike).
    struct Fake(&'static str, &'static str, bool);
    impl super::MeshBackend for Fake {
        fn id(&self) -> &'static str {
            self.0
        }
        fn name(&self) -> &'static str {
            self.0
        }
        fn capabilities(&self) -> Vec<&'static str> {
            vec![]
        }
        fn status(&self) -> anyhow::Result<super::Status> {
            Ok(super::Status {
                backend: self.0.into(),
                available: self.2,
                provisioned: self.2,
                ..Default::default()
            })
        }
        fn provision(&self, _: bool) -> anyhow::Result<super::Plan> {
            unimplemented!()
        }
        fn teardown(&self, _: bool) -> anyhow::Result<super::Plan> {
            unimplemented!()
        }
        fn members(&self) -> anyhow::Result<Vec<super::Member>> {
            Ok(vec![])
        }
        fn revoke(&self, _: &str) -> anyhow::Result<serde_json::Value> {
            unimplemented!()
        }
        fn address_ranges(&self) -> Vec<super::IpNet> {
            vec![super::ranges::net(self.1)]
        }
        fn audit(&self, _: usize) -> anyhow::Result<Vec<super::AuditEntry>> {
            Ok(vec![])
        }
    }

    fn values(list: &[Fake]) -> Vec<serde_json::Value> {
        list.iter().map(|b| super::backend_value(b)).collect()
    }

    #[test]
    fn report_flags_overlay_lan_collisions() {
        let interfaces = serde_json::json!([
            {"ifname":"wlan0","addr_info":[{"local":"100.70.3.4","prefixlen":10}]},
            {"ifname":"eth0","addr_info":[{"local":"192.168.7.2","prefixlen":24}]}
        ]);
        let list: Vec<Box<dyn super::MeshBackend>> =
            vec![Box::new(Fake("fake", "100.64.0.0/10", true))];
        let r = super::report_with(&list, &interfaces);
        assert_eq!(r["backends"][0]["id"], "fake");
        let owners: Vec<_> = r["claims"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["owner"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(
            owners,
            vec!["fake", "lan:eth0"],
            "an address inside an overlay range is the overlay's, not a LAN"
        );
        assert!(r["collisions"].as_array().unwrap().is_empty());
    }

    #[test]
    fn observed_interfaces_pick_their_overlay() {
        let list: Vec<Box<dyn super::MeshBackend>> = vec![
            Box::new(Fake("cloudflare-warp", "100.96.0.0/12", false)),
            Box::new(Fake("tailscale", "100.64.0.0/10", false)),
        ];
        let owners = |interfaces: serde_json::Value| -> Vec<String> {
            let r = super::report_with(&list, &interfaces);
            r["claims"]
                .as_array()
                .unwrap()
                .iter()
                .map(|c| c["owner"].as_str().unwrap().to_owned())
                .collect()
        };
        assert!(
            owners(serde_json::json!([])).is_empty(),
            "nothing installed, nothing observed: no claims"
        );
        assert_eq!(
            owners(
                serde_json::json!([{"ifname":"CloudflareWARP","addr_info":[{"local":"100.96.0.5","prefixlen":32}]}])
            ),
            vec!["cloudflare-warp"]
        );
        // A tailnet address inside WARP's /12 is still Tailscale's on tailscale0.
        assert_eq!(
            owners(
                serde_json::json!([{"ifname":"tailscale0","addr_info":[{"local":"100.100.1.1","prefixlen":32}]}])
            ),
            vec!["tailscale"]
        );
        let both = super::report_with(
            &list,
            &serde_json::json!([
                {"ifname":"CloudflareWARP","addr_info":[{"local":"100.96.0.5","prefixlen":32}]},
                {"ifname":"tailscale0","addr_info":[{"local":"100.70.1.1","prefixlen":32}]}
            ]),
        );
        assert_eq!(both["collisions"][0]["kind"], "nested");
        assert_eq!(both["collisions"][0]["a"]["owner"], "cloudflare-warp");
    }

    #[test]
    fn network_overlay_interfaces_use_the_interface_name_first() {
        let list = values(&[
            Fake("cloudflare-warp", "100.96.0.0/12", true),
            Fake("tailscale", "100.64.0.0/10", true),
        ]);
        let (state, v) = super::overlay_with(
            &list,
            &serde_json::json!([
                {"ifname":"tailscale0","addr_info":[{"local":"100.100.1.1","prefixlen":32}]},
                {"ifname":"CloudflareWARP","addr_info":[{"local":"100.96.0.5","prefixlen":32}]},
                {"ifname":"wg9","addr_info":[{"local":"100.97.0.9","prefixlen":32}]}
            ]),
        );
        assert_eq!(state, "observed");
        let by: Vec<(String, String)> = v["interfaces"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| {
                (
                    i["ifname"].as_str().unwrap().to_owned(),
                    i["backend"].as_str().unwrap().to_owned(),
                )
            })
            .collect();
        assert_eq!(
            by,
            vec![
                ("tailscale0".into(), "tailscale".into()),
                ("CloudflareWARP".into(), "cloudflare-warp".into()),
                // No name hint: the most specific range decides.
                ("wg9".into(), "cloudflare-warp".into()),
            ]
        );
    }

    #[test]
    fn available_but_unprovisioned_backends_without_interfaces_claim_nothing() {
        // No evidence for WARP: no claim, no collision.
        let list = values(&[
            Fake("cloudflare-warp", "100.96.0.0/12", false),
            Fake("tailscale", "100.64.0.0/10", true),
        ]);
        let (state, v) = super::overlay_with(&list, &serde_json::json!([]));
        assert_eq!(state, "configured");
        assert!(v["collisions"].as_array().unwrap().is_empty(), "{v}");
        assert_eq!(v["claims"][0]["owner"], "tailscale");
        assert_eq!(v["claims"].as_array().unwrap().len(), 1);
    }

    /// The network observation runs no backend status: a hung `tailscale` on PATH is never
    /// called and a held ssh-overlay invite lock never waited on.
    #[test]
    fn network_overlay_runs_no_subprocess_and_takes_no_lock() {
        let d = tempfile::tempdir().unwrap();
        let home = d.path();
        let path =
            std::path::PathBuf::from(format!("{}:/usr/bin:/bin", home.join("bin").display()));
        if !super::in_child(
            "mesh::tests::network_overlay_runs_no_subprocess_and_takes_no_lock",
            &[
                ("HOME", home),
                ("XDG_STATE_HOME", &home.join("state")),
                ("XDG_CONFIG_HOME", &home.join("config")),
                ("PATH", &path),
            ],
        ) {
            return;
        }
        let home = std::path::PathBuf::from(std::env::var_os("HOME").unwrap());
        let bin = home.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let fake = bin.join("tailscale");
        std::fs::write(
            &fake,
            format!(
                "#!/bin/sh\ntouch {}\nsleep 30\n",
                home.join("called").display()
            ),
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mesh = home.join("state/cx/mesh/ssh-overlay");
        std::fs::create_dir_all(&mesh).unwrap();
        for p in [
            home.join("state"),
            home.join("state/cx"),
            home.join("state/cx/mesh"),
            mesh.clone(),
        ] {
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let held = std::fs::File::create(mesh.join("lock")).unwrap();
        use std::os::fd::AsRawFd;
        assert_eq!(unsafe { libc::flock(held.as_raw_fd(), libc::LOCK_EX) }, 0);
        let started = std::time::Instant::now();
        let (state, v) = super::overlay(&serde_json::json!([]));
        assert!(
            started.elapsed() < std::time::Duration::from_secs(1),
            "{:?}",
            started.elapsed()
        );
        assert!(!home.join("called").exists(), "tailscale was run");
        assert_eq!(state, "none", "{v}");
        assert_eq!(
            v["backends"][1]["status"]["available"], true,
            "found on PATH"
        );
    }

    fn parse(args: &[&str]) -> Result<super::BackendArgs, clap::Error> {
        use clap::Parser;
        super::BackendArgs::try_parse_from(args)
    }

    #[test]
    fn backend_flags_parse_anywhere_and_unknown_ones_are_refused() {
        let a = parse(&["revoke", "--backend", "x", "--yes", "rover"]).unwrap();
        assert_eq!(
            (a.sub.as_str(), a.device.as_deref(), a.yes),
            ("revoke", Some("rover"), true)
        );
        let a = parse(&["--backend", "ssh-overlay", "status"]).unwrap();
        assert_eq!(a.sub, "status");
        assert_eq!(parse(&["--backend=ssh-overlay"]).unwrap().sub, "status");
        let a = parse(&[
            "invite",
            "--backend",
            "x",
            "--hours=2",
            "--address",
            "10.0.0.2",
            "--address",
            "10.0.0.3",
        ])
        .unwrap();
        assert_eq!((a.hours, a.address.len()), (Some(2.0), 2));
        // A typo (`--hour`) must not silently fall back to the default lifetime.
        assert!(parse(&["invite", "--backend", "x", "--hour", "48"]).is_err());
        assert!(parse(&["status", "--backend", "x", "--bogus"]).is_err());
        assert!(parse(&["invite", "--backend", "x", "--hours", "many"]).is_err());
        let a = parse(&["status", "--backend", "ssh-overlay", "stray"]).unwrap();
        assert!(super::backend_command(&a)
            .unwrap_err()
            .to_string()
            .contains("unexpected argument"));
    }

    #[test]
    fn timeout_kills_the_whole_process_group() {
        let d = tempfile::tempdir().unwrap();
        let pidfile = d.path().join("grandchild");
        let mut cmd = std::process::Command::new("sh");
        cmd.arg("-c")
            .arg("sleep 30 & echo $! > \"$1\"; wait")
            .arg("sh")
            .arg(&pidfile);
        let started = std::time::Instant::now();
        let e = super::run_bounded(cmd, b"", 1024, std::time::Duration::from_millis(300));
        assert_eq!(e.unwrap_err().to_string(), "timed out");
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
        let pid: i32 = std::fs::read_to_string(&pidfile)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        // The grandchild is gone (reaped by init shortly after SIGKILL).
        let mut alive = true;
        for _ in 0..200 {
            let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap_or_default();
            if stat.is_empty() || stat.contains(") Z ") {
                alive = false;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(!alive, "grandchild {pid} survived the timeout");
    }

    #[test]
    fn normal_exit_ends_lingering_group_members_and_never_waits_on_stdin() {
        let d = tempfile::tempdir().unwrap();
        let pidfile = d.path().join("grandchild");
        let mut cmd = std::process::Command::new("sh");
        cmd.arg("-c")
            .arg("sleep 20 & echo $! > \"$1\"; echo hi")
            .arg("sh")
            .arg(&pidfile);
        let started = std::time::Instant::now();
        let (ok, out) =
            super::run_bounded(cmd, b"", 1024, std::time::Duration::from_secs(10)).unwrap();
        assert!(ok && out == b"hi\n", "{out:?}");
        assert!(started.elapsed() < std::time::Duration::from_millis(1500));
        let pid: i32 = std::fs::read_to_string(&pidfile)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        let mut alive = true;
        for _ in 0..200 {
            let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap_or_default();
            if stat.is_empty() || stat.contains(") Z ") {
                alive = false;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(!alive, "grandchild {pid} outlived the call");
        // A group member holding stdin without reading it, with more input than a pipe buffers.
        let mut cmd = std::process::Command::new("sh");
        cmd.arg("-c").arg("sleep 8 0<&0 >/dev/null & exit 0");
        let started = std::time::Instant::now();
        let input = vec![b'x'; 1024 * 1024];
        let (ok, _) =
            super::run_bounded(cmd, &input, 1024, std::time::Duration::from_secs(2)).unwrap();
        assert!(ok);
        assert!(started.elapsed() < std::time::Duration::from_secs(3));
    }

    #[test]
    fn embeds_every_asset_the_util_uploads() {
        let py = super::FILES[0].1;
        for (name, _) in super::FILES[2..].iter() {
            let base = name.trim_start_matches("worker/");
            assert!(py.contains(base), "mesh.py does not reference {base}");
        }
    }

    fn sub(args: &[&str]) -> Option<String> {
        let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        super::subcommand(&args).map(str::to_string)
    }

    #[test]
    fn token_goes_only_to_api_subcommands() {
        assert_eq!(sub(&["devices", "--json"]).as_deref(), Some("devices"));
        assert_eq!(
            sub(&["--json", "--config", "c.toml", "audit"]).as_deref(),
            Some("audit")
        );
        assert_eq!(sub(&["--config=c.toml", "setup"]).as_deref(), Some("setup"));
        // --config's value is never taken for the subcommand: argparse runs `join` here.
        assert_eq!(
            sub(&["--config", "devices", "join", "x"]).as_deref(),
            Some("join")
        );
        // Options argparse might expand (abbreviations) or anything unreadable: no subcommand, no token.
        assert_eq!(sub(&["--conf", "devices", "join", "x"]), None);
        assert_eq!(sub(&["-h"]), None);
        assert_eq!(sub(&["--", "devices"]), None);
        assert_eq!(sub(&["--config"]), None);
        assert_eq!(sub(&[]), None);
        let args = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(super::names_config(&args(&[
            "--config", "c.toml", "devices"
        ])));
        assert!(super::names_config(&args(&[
            "--json",
            "--config=c.toml",
            "audit"
        ])));
        assert!(!super::names_config(&args(&["--json", "devices"])));
        for s in ["join", "leave"] {
            assert!(!super::MESH_API.contains(&s));
            assert!(!super::CLAUDE_API.contains(&s));
        }
        // The cloudflare-warp backend's API calls get the token; join and leave do not need it.
        for s in ["devices", "revoke", "audit"] {
            assert!(super::mesh_api(s), "{s}");
        }
        for s in ["join", "leave"] {
            assert!(!super::mesh_api(s), "{s}");
        }
        // Every allowlisted name is a real subcommand of the embedded utils.
        for s in super::MESH_API {
            assert!(
                super::FILES[0]
                    .1
                    .contains(&format!("sp.add_parser(\"{s}\")")),
                "{s}"
            );
        }
        for s in super::CLAUDE_API {
            assert!(
                super::FILES[1]
                    .1
                    .contains(&format!("sp.add_parser(\"{s}\"")),
                "{s}"
            );
        }
    }
}
