//! `cx mesh`: runs the standalone mesh util (mesh/mesh.py, docs/mesh.md) from a copy embedded in this binary,
//! so it works wherever cx is installed; the util itself needs only python3 and no cx. `cx claude` runs its
//! neighbour mesh/claude.py (docs/claude.md), whose codes the same Worker serves.
//!
//! The mesh is a pluggable feature: [`MeshBackend`] has three implementations, listed by
//! [`backends`] (`cx mesh backends`):
//! - `ssh-overlay` (ssh_overlay.rs): the built-in default over plain SSH, coexisting with any VPN;
//!   devices join with `cx join <code>` (code.rs, the protocol and threat model in docs/mesh.md).
//! - `cloudflare-warp` (cloudflare.rs): the WARP mesh of mesh.py, through its `--json` contract.
//! - `tailscale` (tailscale.rs): read-only adoption of `tailscale status --json`.
//!
//! [`overlay`] feeds network.rs: which backends exist, which interfaces sit in their ranges, and
//! range collisions (ranges.rs) between overlays and LANs.
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
mod ed25519;
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

/// `cx mesh ARGS`: `backends`, `ranges` and any subcommand with `--backend ID` are handled here;
/// everything else goes to mesh.py unchanged (its CLI and `--json` contract are frozen).
pub fn run(args: &[String]) -> Result<()> {
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
    python(unpack()?, args)
}

pub fn run_claude(args: &[String]) -> Result<()> {
    python(unpack()?.with_file_name("claude.py"), args)
}

fn python(script: PathBuf, args: &[String]) -> Result<()> {
    let status = std::process::Command::new("python3")
        .arg(&script)
        .args(args)
        .status()
        .context("cx mesh and cx claude need python3")?;
    std::process::exit(status.code().unwrap_or(1));
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
    /// What the new device types or pastes (`cx join <code>` for ssh-overlay).
    pub code: String,
    pub url: Option<String>,
    pub one_liner_sh: String,
    pub one_liner_ps: Option<String>,
    /// RFC 3339 UTC.
    pub expires: String,
    pub uses: u32,
    /// ssh-overlay with a mesh Worker: the 12-character code redeemable through `url`.
    pub short_code: Option<String>,
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
    fn invite(&self, uses: u32, ttl_hours: f64, device_name: Option<&str>) -> Result<Invite>;
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
    let writer = std::thread::spawn(move || {
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
    let status = loop {
        if let Some(s) = child.try_wait()? {
            break s;
        }
        if Instant::now() >= deadline {
            // SAFETY: plain syscall; the group id is the child's pid (process_group(0) above).
            unsafe {
                libc::killpg(child.id() as libc::pid_t, libc::SIGKILL);
            }
            let _ = child.kill();
            let _ = child.wait();
            bail!("timed out");
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let _ = writer.join();
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
/// state: "observed" (an interface address sits in an overlay range), "configured" (a backend
/// with ranges is provisioned, no interface seen), or "none". ssh-overlay claims no range, so it
/// is listed but never makes the state.
pub fn overlay(interfaces: &Value) -> (String, Value) {
    overlay_with(&backends(), interfaces)
}

pub fn overlay_with(list: &[Box<dyn MeshBackend>], interfaces: &Value) -> (String, Value) {
    let values: Vec<Value> = list.iter().map(|b| backend_value(b.as_ref())).collect();
    let (overlays, lans) = claims(&values, interfaces);
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

struct Flags<'a> {
    args: &'a [String],
}
impl<'a> Flags<'a> {
    fn has(&self, f: &str) -> bool {
        self.args.iter().any(|a| a == f)
    }
    fn all(&self, f: &str) -> Vec<&'a str> {
        let mut out = Vec::new();
        let eq = format!("{f}=");
        let mut i = 0;
        while i < self.args.len() {
            if self.args[i] == f {
                if let Some(v) = self.args.get(i + 1) {
                    out.push(v.as_str());
                }
                i += 2;
                continue;
            }
            if let Some(v) = self.args[i].strip_prefix(&eq) {
                out.push(v);
            }
            i += 1;
        }
        out
    }
    fn one(&self, f: &str) -> Option<&'a str> {
        self.all(f).into_iter().last()
    }
    /// Positional arguments after the subcommand (values of known flags skipped).
    fn positional(&self) -> Vec<&'a str> {
        const VALUED: &[&str] = &[
            "--backend",
            "--hours",
            "--uses",
            "--name",
            "--address",
            "--port",
            "--host-key",
            "--limit",
        ];
        let mut out = Vec::new();
        let mut i = 1;
        while i < self.args.len() {
            let a = self.args[i].as_str();
            if VALUED.contains(&a) {
                i += 2;
                continue;
            }
            if !a.starts_with("--") {
                out.push(a);
            }
            i += 1;
        }
        out
    }
}

fn run_backend(args: &[String]) -> Result<()> {
    let f = Flags { args };
    let id = f.one("--backend").context("--backend needs a value")?;
    let b = backend(id)?;
    let sub = args.first().map(String::as_str).unwrap_or("status");
    let hours = f
        .one("--hours")
        .map(str::parse::<f64>)
        .transpose()
        .context("--hours")?;
    let out: Value = match sub {
        "status" => serde_json::to_value(b.status()?)?,
        "members" | "devices" => serde_json::to_value(b.members()?)?,
        "audit" => serde_json::to_value(b.audit(f.one("--limit").map(str::parse).transpose()?.unwrap_or(50))?)?,
        "ranges" => serde_json::to_value(b.address_ranges())?,
        "setup" | "provision" => serde_json::to_value(b.provision(f.has("--dry-run"))?)?,
        "teardown" => {
            let dry = f.has("--dry-run");
            anyhow::ensure!(dry || f.has("--yes"), "not confirmed (pass --yes, or --dry-run to preview)");
            serde_json::to_value(b.teardown(dry)?)?
        }
        "revoke" => {
            let name = *f.positional().first().context("revoke NAME")?;
            anyhow::ensure!(f.has("--yes"), "not confirmed (pass --yes)");
            b.revoke(name)?
        }
        "invite" if b.id() == ssh_overlay::ID => {
            let opts = ssh_overlay::InviteOptions {
                ttl_hours: hours,
                device_name: f.one("--name").map(str::to_owned),
                addresses: f.all("--address").into_iter().map(str::to_owned).collect(),
                port: f.one("--port").map(str::parse).transpose().context("--port")?,
                host_key_file: f.one("--host-key").map(PathBuf::from),
                short: f.has("--short"),
            };
            anyhow::ensure!(f.one("--uses").is_none_or(|u| u == "1"), "ssh-overlay invites are single use");
            serde_json::to_value(ssh_overlay::create_invite(&opts)?)?
        }
        "invite" => {
            let uses = f.one("--uses").map(str::parse).transpose().context("--uses")?.unwrap_or(1);
            serde_json::to_value(b.invite(uses, hours.unwrap_or(24.0), f.one("--name"))?)?
        }
        other => bail!("cx mesh {other} --backend: use status, members, invite, revoke, audit, ranges, setup or teardown"),
    };
    if f.has("--json") {
        println!("{}", serde_json::to_string_pretty(&out)?);
    } else if sub == "invite" {
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

/// `cx join <code> [--name N]`, and the viewer's forced command `cx join --redeem <id>`.
pub fn join(
    code: Option<&str>,
    name: Option<&str>,
    redeem: Option<&str>,
    json: bool,
) -> Result<()> {
    if let Some(id) = redeem {
        return ssh_overlay::redeem(id);
    }
    let code = code.context("cx join <code>")?;
    let r = ssh_overlay::join(
        code,
        &ssh_overlay::JoinOptions {
            name: name.map(str::to_owned),
        },
    )?;
    if json {
        println!("{}", serde_json::to_string_pretty(&r)?);
    } else {
        println!(
            "joined {} as {} (host key verified against the code)",
            r["target"].as_str().unwrap_or(""),
            r["as"].as_str().unwrap_or("")
        );
        if let Some(w) = r["warning"].as_str() {
            println!("note: {w}");
        }
        println!("next: {}", r["next"].as_str().unwrap_or(""));
    }
    Ok(())
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

    struct Fake;
    impl super::MeshBackend for Fake {
        fn id(&self) -> &'static str {
            "fake"
        }
        fn name(&self) -> &'static str {
            "Fake"
        }
        fn capabilities(&self) -> Vec<&'static str> {
            vec![]
        }
        fn status(&self) -> anyhow::Result<super::Status> {
            Ok(super::Status {
                backend: "fake".into(),
                available: true,
                provisioned: true,
                ..Default::default()
            })
        }
        fn provision(&self, _: bool) -> anyhow::Result<super::Plan> {
            unimplemented!()
        }
        fn teardown(&self, _: bool) -> anyhow::Result<super::Plan> {
            unimplemented!()
        }
        fn invite(&self, _: u32, _: f64, _: Option<&str>) -> anyhow::Result<super::Invite> {
            unimplemented!()
        }
        fn members(&self) -> anyhow::Result<Vec<super::Member>> {
            Ok(vec![])
        }
        fn revoke(&self, _: &str) -> anyhow::Result<serde_json::Value> {
            unimplemented!()
        }
        fn address_ranges(&self) -> Vec<super::IpNet> {
            vec![super::ranges::net("100.64.0.0/10")]
        }
        fn audit(&self, _: usize) -> anyhow::Result<Vec<super::AuditEntry>> {
            Ok(vec![])
        }
    }

    #[test]
    fn report_flags_overlay_lan_collisions() {
        let interfaces = serde_json::json!([
            {"ifname":"wlan0","addr_info":[{"local":"100.70.3.4","prefixlen":10}]},
            {"ifname":"eth0","addr_info":[{"local":"192.168.7.2","prefixlen":24}]}
        ]);
        let list: Vec<Box<dyn super::MeshBackend>> = vec![Box::new(Fake)];
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

    struct Ranged(&'static str, &'static str);
    impl super::MeshBackend for Ranged {
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
            Ok(super::Status::default())
        }
        fn provision(&self, _: bool) -> anyhow::Result<super::Plan> {
            unimplemented!()
        }
        fn teardown(&self, _: bool) -> anyhow::Result<super::Plan> {
            unimplemented!()
        }
        fn invite(&self, _: u32, _: f64, _: Option<&str>) -> anyhow::Result<super::Invite> {
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

    #[test]
    fn observed_interfaces_pick_their_overlay() {
        let list: Vec<Box<dyn super::MeshBackend>> = vec![
            Box::new(Ranged("cloudflare-warp", "100.96.0.0/12")),
            Box::new(Ranged("tailscale", "100.64.0.0/10")),
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

    /// Ranged, but with real evidence (provisioned), like a configured WARP and a running tailnet.
    struct Present(&'static str, &'static str);
    impl super::MeshBackend for Present {
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
                available: true,
                provisioned: true,
                ..Default::default()
            })
        }
        fn provision(&self, _: bool) -> anyhow::Result<super::Plan> {
            unimplemented!()
        }
        fn teardown(&self, _: bool) -> anyhow::Result<super::Plan> {
            unimplemented!()
        }
        fn invite(&self, _: u32, _: f64, _: Option<&str>) -> anyhow::Result<super::Invite> {
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

    #[test]
    fn network_overlay_interfaces_use_the_interface_name_first() {
        let list: Vec<Box<dyn super::MeshBackend>> = vec![
            Box::new(Present("cloudflare-warp", "100.96.0.0/12")),
            Box::new(Present("tailscale", "100.64.0.0/10")),
        ];
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
        // Ranged reports available=false, provisioned=false: no evidence, no claim, no collision.
        let list: Vec<Box<dyn super::MeshBackend>> = vec![
            Box::new(Ranged("cloudflare-warp", "100.96.0.0/12")),
            Box::new(Present("tailscale", "100.64.0.0/10")),
        ];
        let (state, v) = super::overlay_with(&list, &serde_json::json!([]));
        assert_eq!(state, "configured");
        assert!(v["collisions"].as_array().unwrap().is_empty(), "{v}");
        assert_eq!(v["claims"][0]["owner"], "tailscale");
        assert_eq!(v["claims"].as_array().unwrap().len(), 1);
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
    fn embeds_every_asset_the_util_uploads() {
        let py = super::FILES[0].1;
        for (name, _) in super::FILES[2..].iter() {
            let base = name.trim_start_matches("worker/");
            assert!(py.contains(base), "mesh.py does not reference {base}");
        }
    }
}
