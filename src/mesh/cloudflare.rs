//! The cloudflare-warp backend: the WARP mesh of mesh/mesh.py, driven through its `--json`
//! contract from the copy embedded in cx (so standalone mesh.py users see no change).
//! Onboarding helpers (`token-url`, `verify`, `zones`) live in mesh.py, reached through `cx mesh`.
//!
//! The API token is never handled here: mesh.py inherits cx's environment and reads
//! `$CLOUDFLARE_API_TOKEN` itself, else its mode-600 credentials file.
use super::{ranges, AuditEntry, Invite, IpNet, Member, MeshBackend, Plan, Status};
use anyhow::{bail, Context, Result};
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

pub const ID: &str = "cloudflare-warp";
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

/// `[profile] mesh = [...]` of mesh.py's config: the quoted CIDRs of that array, which may span
/// lines, with `#` comments ignored (tomllib's reading of the template's form).
fn toml_mesh_ranges(text: &str) -> Vec<IpNet> {
    let mut array = String::new();
    let mut open = false;
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or_default();
        if !open {
            let Some((k, v)) = line.split_once('=') else {
                continue;
            };
            if k.trim() != "mesh" || !v.trim_start().starts_with('[') {
                continue;
            }
            open = true;
            array.push_str(v);
        } else {
            array.push_str(line);
        }
        if array.contains(']') {
            break;
        }
    }
    let array = array.split(']').next().unwrap_or_default();
    array
        .split('"')
        .skip(1)
        .step_by(2)
        .filter_map(|s| s.parse().ok())
        .collect()
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
    let mut mesh_ranges = toml_mesh_ranges(&text);
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

/// `mesh.py invite`: a WARP join code with its one-liners.
pub fn invite(uses: u32, ttl_hours: f64, device_name: Option<&str>) -> Result<Invite> {
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

/// `available` is evidence, not tooling: a mesh config exists here. An unconfigured host makes
/// no claim on WARP's range (an observed WARP interface still does, through `claims`).
fn status_from(q: &Quick) -> Status {
    Status {
        backend: ID.into(),
        available: q.configured,
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
        Ok(status_from(&quick_status()))
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
    fn members(&self) -> Result<Vec<Member>> {
        Ok(members_from(&mesh_json(&["devices"], None, QUICK)?))
    }
    fn revoke(&self, name: &str) -> Result<Value> {
        // A name reaches mesh.py as argv and the Worker as a path segment: no flags, no dots-only.
        anyhow::ensure!(
            super::ssh_overlay::valid_name(name) && name.bytes().any(|b| b != b'.'),
            "invalid device name"
        );
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
    fn revoke_refuses_flag_like_and_path_like_names() {
        for bad in ["-h", "--yes", "..", ".", "a/b", "", "a b"] {
            let e = CloudflareWarp.revoke(bad).unwrap_err();
            assert_eq!(e.to_string(), "invalid device name", "{bad:?}");
        }
    }

    #[test]
    fn available_means_a_config_exists() {
        let d = tempfile::tempdir().unwrap();
        assert!(!status_from(&quick_status_at(d.path())).available);
        std::fs::write(d.path().join("config.toml"), "name = \"team\"\n").unwrap();
        let s = status_from(&quick_status_at(d.path()));
        assert!(s.available && !s.provisioned);
    }

    #[test]
    fn mesh_ranges_read_like_tomllib() {
        let d = tempfile::tempdir().unwrap();
        let ranges = |text: &str| {
            std::fs::write(d.path().join("config.toml"), text).unwrap();
            quick_status_at(d.path())
                .mesh_ranges
                .iter()
                .map(|r| r.to_string())
                .collect::<Vec<_>>()
        };
        // An array edited across lines, with comments (one holding a quote).
        assert_eq!(
            ranges("name = \"team\"\n[profile]\nmesh = [  # the \"carved\" ranges\n  \"10.77.0.0/16\",\n  # \"10.1.0.0/16\",\n  \"fd00:bb::/64\",\n]\nexcludes = [\"192.168.0.0/16\"]\n"),
            vec!["10.77.0.0/16", "fd00:bb::/64"]
        );
        assert_eq!(
            ranges("[profile]\nmesh_dns = [\"10.9.0.0/16\"]\nmesh = []\n"),
            vec![ranges::WARP_V4],
            "an empty array, and another key that starts with mesh, fall back to the default"
        );
    }
}
