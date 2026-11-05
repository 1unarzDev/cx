//! The tailscale backend: read-only adoption of an existing tailnet through
//! `tailscale status --json` when the binary exists. cx never logs in, changes ACLs or
//! routes, or revokes nodes; those stay in Tailscale's own tools.
use super::{ranges, AuditEntry, Invite, IpNet, Member, MeshBackend, Plan, Status};
use anyhow::{bail, Context, Result};
use serde_json::Value;
use std::{
    path::PathBuf,
    process::{Command, Stdio},
    time::Duration,
};

pub const ID: &str = "tailscale";

pub struct Tailscale {
    binary: Option<PathBuf>,
}

/// `tailscale` on PATH, if any (no fixed system paths: what the user's shell would run).
pub fn find() -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|p| {
        std::env::split_paths(&p)
            .map(|d| d.join("tailscale"))
            .find(|f| {
                use std::os::unix::fs::PermissionsExt;
                f.metadata()
                    .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
            })
    })
}

impl Tailscale {
    pub fn new() -> Self {
        Self { binary: find() }
    }
    #[allow(dead_code)] // tests and callers that already located the binary
    pub fn with_binary(binary: Option<PathBuf>) -> Self {
        Self { binary }
    }
    pub fn raw_status(&self) -> Result<Value> {
        let bin = self.binary.as_ref().context("tailscale is not installed")?;
        let mut cmd = Command::new(bin);
        cmd.args(["status", "--json"])
            .env("LC_ALL", "C")
            .stderr(Stdio::null());
        let (ok, out) = super::run_bounded(cmd, b"", 2 * 1024 * 1024, Duration::from_secs(3))?;
        // `tailscale status` exits non-zero when stopped but still prints JSON.
        let v: Value = serde_json::from_slice(&out).with_context(|| {
            if ok {
                "tailscale status gave invalid JSON".to_string()
            } else {
                "tailscale status failed".to_string()
            }
        })?;
        Ok(v)
    }
}

impl Default for Tailscale {
    fn default() -> Self {
        Self::new()
    }
}

fn member(node: &Value, own: bool) -> Member {
    let ips: Vec<&str> = node["TailscaleIPs"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    let dns = node["DNSName"]
        .as_str()
        .map(|s| s.trim_end_matches('.').to_owned())
        .filter(|s| !s.is_empty());
    let online = node["Online"].as_bool();
    Member {
        name: dns
            .as_deref()
            .and_then(|d| d.split('.').next())
            .map(str::to_owned)
            .or_else(|| node["HostName"].as_str().map(str::to_owned))
            .unwrap_or_default(),
        host: node["HostName"].as_str().unwrap_or_default().to_owned(),
        os: node["OS"]
            .as_str()
            .map(str::to_owned)
            .filter(|s| !s.is_empty()),
        overlay_ipv4: ips.iter().find(|i| !i.contains(':')).map(|s| s.to_string()),
        overlay_ipv6: ips.iter().find(|i| i.contains(':')).map(|s| s.to_string()),
        dns_name: dns,
        state: if own {
            "self".into()
        } else {
            match online {
                Some(true) => "online".into(),
                Some(false) => "offline".into(),
                None => "unknown".into(),
            }
        },
        last_seen: node["LastSeen"]
            .as_str()
            .filter(|s| !s.starts_with("0001-"))
            .map(str::to_owned),
        role: Some(if own { "self".into() } else { "peer".into() }),
        via: None,
    }
}

pub fn members_from(status: &Value) -> Vec<Member> {
    let mut out = Vec::new();
    if status["Self"].is_object() {
        out.push(member(&status["Self"], true));
    }
    let mut peers: Vec<Member> = status["Peer"]
        .as_object()
        .into_iter()
        .flat_map(|m| m.values())
        .map(|p| member(p, false))
        .collect();
    peers.sort_by(|a, b| a.name.cmp(&b.name));
    out.extend(peers);
    out
}

impl MeshBackend for Tailscale {
    fn id(&self) -> &'static str {
        ID
    }
    fn name(&self) -> &'static str {
        "Tailscale (read-only)"
    }
    fn capabilities(&self) -> Vec<&'static str> {
        vec!["read-only", "magicdns"]
    }
    fn status(&self) -> Result<Status> {
        if self.binary.is_none() {
            return Ok(Status {
                backend: ID.into(),
                available: false,
                provisioned: false,
                members: None,
                endpoint: None,
                detail: "tailscale is not installed".into(),
            });
        }
        let s = self.raw_status()?;
        let state = s["BackendState"].as_str().unwrap_or("unknown").to_owned();
        let suffix = s["MagicDNSSuffix"]
            .as_str()
            .or_else(|| s["CurrentTailnet"]["MagicDNSSuffix"].as_str())
            .map(str::to_owned);
        Ok(Status {
            backend: ID.into(),
            available: true,
            provisioned: state == "Running",
            members: Some(members_from(&s).len()),
            endpoint: suffix,
            detail: format!("tailscaled reports {state}"),
        })
    }
    fn provision(&self, _dry_run: bool) -> Result<Plan> {
        bail!("tailscale is adopted read-only; use `tailscale up` to join a tailnet")
    }
    fn teardown(&self, _dry_run: bool) -> Result<Plan> {
        bail!("tailscale is adopted read-only; cx does not change it")
    }
    fn invite(&self, _uses: u32, _ttl_hours: f64, _device_name: Option<&str>) -> Result<Invite> {
        bail!("tailscale is adopted read-only; invite devices from the Tailscale admin console")
    }
    fn members(&self) -> Result<Vec<Member>> {
        Ok(members_from(&self.raw_status()?))
    }
    fn revoke(&self, _name: &str) -> Result<Value> {
        bail!("tailscale is adopted read-only; remove devices from the Tailscale admin console")
    }
    fn address_ranges(&self) -> Vec<IpNet> {
        vec![
            ranges::net(ranges::TAILSCALE_V4),
            ranges::net(ranges::TAILSCALE_V6),
        ]
    }
    fn audit(&self, _limit: usize) -> Result<Vec<AuditEntry>> {
        Ok(Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    const STATUS: &str = r#"{"BackendState":"Running","MagicDNSSuffix":"tail1234.ts.net",
      "Self":{"HostName":"laptop","DNSName":"laptop.tail1234.ts.net.","OS":"linux","TailscaleIPs":["100.101.1.2","fd7a:115c:a1e0::1"],"Online":true},
      "Peer":{"nodekey:b":{"HostName":"Lab ARM","DNSName":"lab-arm.tail1234.ts.net.","OS":"linux","TailscaleIPs":["100.88.0.9"],"Online":false,"LastSeen":"2026-11-01T10:00:00Z"},
              "nodekey:a":{"HostName":"phone","DNSName":"phone.tail1234.ts.net.","OS":"iOS","TailscaleIPs":["100.70.0.3"],"Online":true,"LastSeen":"0001-01-01T00:00:00Z"}}}"#;

    #[test]
    fn adopts_a_fake_tailscale_binary() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("tailscale");
        std::fs::write(dir.path().join("status.json"), STATUS).unwrap();
        std::fs::write(
            &bin,
            format!(
                "#!/bin/sh\n[ \"$1 $2\" = \"status --json\" ] || exit 2\ncat '{}'\n",
                dir.path().join("status.json").display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        let t = Tailscale::with_binary(Some(bin));
        let s = t.status().unwrap();
        assert!(s.available && s.provisioned);
        assert_eq!(s.members, Some(3));
        assert_eq!(s.endpoint.as_deref(), Some("tail1234.ts.net"));
        let m = t.members().unwrap();
        assert_eq!(
            m.iter()
                .map(|m| (m.name.as_str(), m.state.as_str()))
                .collect::<Vec<_>>(),
            vec![
                ("laptop", "self"),
                ("lab-arm", "offline"),
                ("phone", "online")
            ]
        );
        assert_eq!(m[0].overlay_ipv6.as_deref(), Some("fd7a:115c:a1e0::1"));
        assert_eq!(m[1].dns_name.as_deref(), Some("lab-arm.tail1234.ts.net"));
        assert_eq!(m[1].last_seen.as_deref(), Some("2026-11-01T10:00:00Z"));
        assert_eq!(m[2].last_seen, None);
        assert!(t.revoke("phone").is_err() && t.invite(1, 1.0, None).is_err());
        assert!(t
            .address_ranges()
            .iter()
            .any(|r| r.to_string() == "100.64.0.0/10"));
    }

    #[test]
    fn absent_binary_is_reported_not_assumed() {
        let s = Tailscale::with_binary(None).status().unwrap();
        assert!(!s.available && !s.provisioned && s.members.is_none());
    }
}
