//! The cloudflare-warp backend: the WARP mesh of mesh/mesh.py, driven through its `--json`
//! contract from the copy embedded in cx (so standalone mesh.py users see no change).
//! Onboarding helpers (`token-url`, `verify`, `zones`) live in mesh.py too, so both get them.
//!
//! The API token is never stored here. It is read, in order, from a [`TokenSource`] (the secrets
//! store once root wires it; until then `$CLOUDFLARE_API_TOKEN`), and passed to mesh.py through
//! its environment or stdin, never argv. mesh.py's own fallback (its mode-600 credentials file)
//! still applies when no source has one.
use super::{ranges, AuditEntry, Invite, IpNet, Member, MeshBackend, Plan, Status};
use anyhow::{bail, Context, Result};
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

pub const ID: &str = "cloudflare-warp";
const SECRET_NAME: &str = "cloudflare-api-token";

/// A token that zeroes its bytes when dropped (stand-in for `zeroize::Zeroizing<String>`).
pub struct Secret(String);
impl Secret {
    pub fn new(s: String) -> Self {
        Self(s)
    }
    pub fn expose(&self) -> &str {
        &self.0
    }
}
impl Drop for Secret {
    fn drop(&mut self) {
        // SAFETY: zero bytes keep the String valid UTF-8.
        unsafe {
            for b in self.0.as_bytes_mut() {
                std::ptr::write_volatile(b, 0);
            }
        }
    }
}
impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Secret(<{} bytes>)", self.0.len())
    }
}

/// Where the Cloudflare token comes from. TODO(root): add a source backed by
/// `crate::secrets::get("cloudflare-api-token")` (Zeroizing<String>) once src/secrets.rs lands,
/// and put it first in [`default_sources`].
pub trait TokenSource {
    fn get(&self, name: &str) -> Option<Secret>;
}

/// `$CLOUDFLARE_API_TOKEN`, what mesh.py itself reads first.
pub struct EnvSource;
impl TokenSource for EnvSource {
    fn get(&self, name: &str) -> Option<Secret> {
        (name == SECRET_NAME)
            .then(|| std::env::var("CLOUDFLARE_API_TOKEN").ok())
            .flatten()
            .filter(|t| !t.trim().is_empty())
            .map(Secret::new)
    }
}

pub fn default_sources() -> Vec<Box<dyn TokenSource>> {
    vec![Box::new(EnvSource)]
}

pub fn token() -> Option<Secret> {
    default_sources().iter().find_map(|s| s.get(SECRET_NAME))
}

/// Run the embedded mesh.py with `--json`; the parsed JSON on success, its error otherwise.
fn mesh_json(args: &[&str], stdin: Option<&[u8]>, timeout: Duration) -> Result<Value> {
    let (ok, v) = mesh_json_status(args, stdin, timeout)?;
    anyhow::ensure!(ok, "mesh.py {} failed", args.first().unwrap_or(&""));
    Ok(v)
}

/// Like [`mesh_json`], but a non-zero exit with a JSON answer (e.g. `verify` of a bad token) is data.
fn mesh_json_status(
    args: &[&str],
    stdin: Option<&[u8]>,
    timeout: Duration,
) -> Result<(bool, Value)> {
    let script = super::unpack()?;
    let mut cmd = Command::new("python3");
    cmd.arg(script).args(args).arg("--json");
    if let Some(t) = token() {
        cmd.env("CLOUDFLARE_API_TOKEN", t.expose());
    }
    let (ok, out) = super::run_bounded(cmd, stdin.unwrap_or_default(), 1024 * 1024, timeout)
        .context("cx mesh needs python3")?;
    let text = String::from_utf8_lossy(&out);
    // The JSON document is the last top-level value mesh.py printed.
    let v: Value = serde_json::from_str(text.trim())
        .with_context(|| format!("mesh.py {} gave no JSON", args.first().unwrap_or(&"")))?;
    if let Some(e) = v.get("error").and_then(Value::as_str) {
        bail!("{}", crate::files::display(e));
    }
    Ok((ok, v))
}

const QUICK: Duration = Duration::from_secs(20);
const SLOW: Duration = Duration::from_secs(300);

/// The dashboard URL that creates a token with the permissions docs/mesh.md lists, pre-filled.
/// `{"url", "permissions": [...prefilled], "manual": [...the user adds by hand]}`; no network.
#[allow(dead_code)] // the UI wave's Secrets onboarding calls this
pub fn onboarding_url(with_domain: bool) -> Result<Value> {
    let mut args = vec!["token-url"];
    if with_domain {
        args.push("--with-domain");
    }
    mesh_json(&args, None, QUICK)
}

/// GET /user/tokens/verify (then the account endpoint) through mesh.py; the token goes on stdin.
/// `{"valid", "status", "id", "expires_on", "kind"}`; the token itself is never echoed.
#[allow(dead_code)] // the UI wave's Secrets onboarding calls this
pub fn verify_token(token: &Secret) -> Result<Value> {
    let input = Secret::new(format!("{}\n", token.expose()));
    Ok(mesh_json_status(
        &["verify", "--token-stdin"],
        Some(input.expose().as_bytes()),
        QUICK,
    )?
    .1)
}

/// The zones the token can see, for a one-click domain choice: `{"zones": [{id, name, status, account}]}`.
#[allow(dead_code)] // the UI wave's Secrets onboarding calls this
pub fn list_zones(token: &Secret) -> Result<Value> {
    let input = Secret::new(format!("{}\n", token.expose()));
    mesh_json(
        &["zones", "--token-stdin"],
        Some(input.expose().as_bytes()),
        QUICK,
    )
}

/// Stash a sealed ssh-overlay invite on the mesh Worker (owner side). `{"url", "expires"}`.
pub fn stash_pair(lookup: &str, sealed: &str, hours: f64) -> Result<Value> {
    let h = format!("{hours}");
    mesh_json(
        &[
            "pair-stash",
            "--lookup",
            lookup,
            "--hours",
            &h,
            "--sealed-stdin",
        ],
        Some(sealed.as_bytes()),
        QUICK,
    )
}

/// Redeem a stashed invite (device side; needs no config or token). Returns the sealed blob.
pub fn redeem_pair(url: &str, lookup: &str) -> Result<String> {
    let v = mesh_json(
        &["pair-redeem", "--url", url, "--lookup", lookup],
        None,
        QUICK,
    )?;
    Ok(v["sealed"]
        .as_str()
        .context("the mesh Worker sent no invite")?
        .to_owned())
}

pub fn config_dir() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config")
        })
        .join("cx")
        .join("mesh")
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Quick {
    pub configured: bool,
    pub provisioned: bool,
    pub name: Option<String>,
    pub endpoint: Option<String>,
    pub mesh_ranges: Vec<IpNet>,
}

fn toml_string(text: &str, key: &str) -> Option<String> {
    text.lines().find_map(|l| {
        let (k, v) = l.split_once('=')?;
        if k.trim() != key {
            return None;
        }
        let v = v.trim();
        let v = v.strip_prefix('"')?;
        Some(v[..v.find('"')?].to_owned())
    })
}

/// Local-only status (no python, no network): config present, and the state file's endpoint.
/// Cheap enough for every network observation.
pub fn quick_status_at(dir: &Path) -> Quick {
    let Ok(text) = std::fs::read_to_string(dir.join("config.toml")) else {
        // Unconfigured: WARP's default range still classifies an observed WARP interface.
        return Quick {
            mesh_ranges: vec![ranges::net(ranges::WARP_V4)],
            ..Quick::default()
        };
    };
    let name = toml_string(&text, "name").filter(|n| !n.is_empty() && !n.contains(['/', '.']));
    let state = name
        .as_ref()
        .and_then(|n| std::fs::read(dir.join(format!("{n}.state.json"))).ok())
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok());
    let endpoint = state
        .as_ref()
        .and_then(|s| s["url"].as_str().map(str::to_owned));
    // [profile] mesh = ["..."]: only quoted CIDRs on that one line (the template writes it so).
    let mut mesh_ranges: Vec<IpNet> = text
        .lines()
        .filter(|l| {
            l.trim_start().starts_with("mesh")
                && l.split_once('=').is_some_and(|(k, _)| k.trim() == "mesh")
        })
        .flat_map(|l| {
            l.split('"')
                .skip(1)
                .step_by(2)
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .filter_map(|s| s.parse().ok())
        .collect();
    if mesh_ranges.is_empty() {
        mesh_ranges.push(ranges::net(ranges::WARP_V4));
    }
    Quick {
        configured: true,
        provisioned: endpoint.is_some(),
        name,
        endpoint,
        mesh_ranges,
    }
}

pub fn quick_status() -> Quick {
    quick_status_at(&config_dir())
}

pub struct CloudflareWarp;

pub fn members_from(d: &Value) -> Vec<Member> {
    let suffix = d["dns_suffix"].as_str().unwrap_or_default();
    let mut out: Vec<Member> = d["enrolled"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|e| {
            let name = e["name"].as_str().unwrap_or_default().to_owned();
            Member {
                dns_name: (!suffix.is_empty() && !name.is_empty())
                    .then(|| format!("{name}{suffix}")),
                name,
                host: e["host"].as_str().unwrap_or_default().to_owned(),
                os: e["os"].as_str().map(str::to_owned),
                overlay_ipv4: e["ipv4"].as_str().map(str::to_owned),
                overlay_ipv6: e["ipv6"].as_str().map(str::to_owned),
                state: e["state"].as_str().unwrap_or("unknown").to_owned(),
                last_seen: e["updated"].as_str().map(str::to_owned),
                role: Some("device".into()),
                via: None,
            }
        })
        .collect();
    for c in d["connectors"].as_array().into_iter().flatten() {
        let name = c["name"].as_str().unwrap_or_default().to_owned();
        out.push(Member {
            dns_name: None,
            host: name.clone(),
            name,
            os: None,
            overlay_ipv4: None,
            overlay_ipv6: None,
            state: c["status"].as_str().unwrap_or("unknown").to_owned(),
            last_seen: None,
            role: Some("connector".into()),
            via: None,
        });
    }
    out
}

fn plan(v: Value, dry_run: bool) -> Plan {
    let steps = v["planned"]
        .as_array()
        .or_else(|| v["would_remove"].as_array())
        .or_else(|| v["removed"].as_array())
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect();
    Plan {
        dry_run,
        applied: !dry_run,
        steps,
        result: v,
    }
}

impl MeshBackend for CloudflareWarp {
    fn id(&self) -> &'static str {
        ID
    }
    fn name(&self) -> &'static str {
        "Cloudflare WARP mesh"
    }
    fn capabilities(&self) -> Vec<&'static str> {
        vec![
            "provision",
            "teardown",
            "invite",
            "revoke",
            "audit",
            "lend",
            "onboarding",
            "pair-rendezvous",
        ]
    }
    fn status(&self) -> Result<Status> {
        let q = quick_status();
        Ok(Status {
            backend: ID.into(),
            available: true,
            provisioned: q.provisioned,
            members: None,
            endpoint: q.endpoint.clone(),
            detail: match (&q.name, q.provisioned) {
                (None, _) => "not set up (cx mesh setup --name N)".into(),
                (Some(n), false) => format!("mesh {n} configured, not deployed"),
                (Some(n), true) => {
                    format!("mesh {n} deployed; members need `cx mesh devices` (network)")
                }
            },
        })
    }
    fn provision(&self, dry_run: bool) -> Result<Plan> {
        anyhow::ensure!(
            quick_status().configured,
            "first setup needs a name: cx mesh setup --name N [--domain D]"
        );
        let mut args = vec!["setup"];
        if dry_run {
            args.push("--dry-run");
        }
        Ok(plan(mesh_json(&args, None, SLOW)?, dry_run))
    }
    fn teardown(&self, dry_run: bool) -> Result<Plan> {
        let mut args = vec!["teardown", "--yes"];
        if dry_run {
            args.push("--dry-run");
        }
        Ok(plan(mesh_json(&args, None, SLOW)?, dry_run))
    }
    fn invite(&self, uses: u32, ttl_hours: f64, device_name: Option<&str>) -> Result<Invite> {
        let (u, h) = (uses.to_string(), ttl_hours.to_string());
        let mut args = vec!["invite", "--uses", &u, "--hours", &h];
        if let Some(n) = device_name {
            args.extend(["--name", n]);
        }
        let d = mesh_json(&args, None, QUICK)?;
        let code = d["code"].as_str().context("no invite code")?.to_owned();
        Ok(Invite {
            url: quick_status().endpoint.map(|e| format!("{e}/{code}")),
            code,
            one_liner_sh: d["sh"].as_str().unwrap_or_default().to_owned(),
            one_liner_ps: d["ps1"].as_str().map(str::to_owned),
            expires: d["expires"].as_str().unwrap_or_default().to_owned(),
            uses: d["uses"].as_u64().unwrap_or(uses as u64) as u32,
            backend: ID.into(),
            short_code: None,
        })
    }
    fn members(&self) -> Result<Vec<Member>> {
        Ok(members_from(&mesh_json(&["devices"], None, QUICK)?))
    }
    fn revoke(&self, name: &str) -> Result<Value> {
        mesh_json(&["revoke", name, "--yes"], None, QUICK)
    }
    fn address_ranges(&self) -> Vec<IpNet> {
        quick_status().mesh_ranges
    }
    fn audit(&self, limit: usize) -> Result<Vec<AuditEntry>> {
        let d = mesh_json(&["audit"], None, QUICK)?;
        Ok(d["audit"]
            .as_array()
            .into_iter()
            .flatten()
            .take(limit)
            .map(|e| AuditEntry {
                at: e["at"].as_str().unwrap_or_default().to_owned(),
                actor: e["actor"].as_str().unwrap_or_default().to_owned(),
                action: e["action"].as_str().unwrap_or_default().to_owned(),
                detail: e["detail"].as_str().unwrap_or_default().to_owned(),
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn quick_status_reads_config_and_state_only() {
        let d = tempfile::tempdir().unwrap();
        let none = quick_status_at(d.path());
        assert!(!none.configured && !none.provisioned && none.endpoint.is_none());
        assert_eq!(none.mesh_ranges, vec![ranges::net(ranges::WARP_V4)]);
        std::fs::write(
            d.path().join("config.toml"),
            "name = \"team\"   # names things\n[profile]\nmesh = [\"100.96.0.0/12\", \"fd00:aa::/64\"]        # carved\n",
        )
        .unwrap();
        let q = quick_status_at(d.path());
        assert!(q.configured && !q.provisioned);
        assert_eq!(q.name.as_deref(), Some("team"));
        assert_eq!(
            q.mesh_ranges
                .iter()
                .map(|r| r.to_string())
                .collect::<Vec<_>>(),
            vec!["100.96.0.0/12", "fd00:aa::/64"]
        );
        std::fs::write(
            d.path().join("team.state.json"),
            r#"{"url":"https://mesh.example.org"}"#,
        )
        .unwrap();
        let q = quick_status_at(d.path());
        assert!(q.provisioned);
        assert_eq!(q.endpoint.as_deref(), Some("https://mesh.example.org"));
    }

    #[test]
    fn maps_the_devices_contract() {
        let d = json!({"dns_suffix": ".mesh.example.org",
            "enrolled": [{"name":"laptop","host":"laptop","os":"linux","ipv4":"100.96.0.2","ipv6":null,"state":"active","updated":"2026-11-01T00:00:00Z"}],
            "connectors": [{"name":"lab","status":"healthy","id":"c1"}], "dns": []});
        let m = members_from(&d);
        assert_eq!(m.len(), 2);
        assert_eq!(m[0].dns_name.as_deref(), Some("laptop.mesh.example.org"));
        assert_eq!(m[0].overlay_ipv4.as_deref(), Some("100.96.0.2"));
        assert_eq!(m[1].role.as_deref(), Some("connector"));
        assert_eq!(m[1].state, "healthy");
    }

    #[test]
    fn secrets_never_print() {
        let s = Secret::new("abc123".into());
        assert_eq!(format!("{s:?}"), "Secret(<6 bytes>)");
        assert!(EnvSource.get("other").is_none());
    }
}
