//! The cloudflare-warp backend: the WARP mesh of mesh/mesh.py, which `cx mesh` without `--backend`
//! runs unchanged (setup, invite, devices, revoke, audit, teardown). Here only what the backend report
//! and the network observation need: a local status and the ranges, read from mesh.py's config and
//! state files without python or the network.
use super::{ranges, IpNet, MeshBackend, Status};
use std::path::{Path, PathBuf};

pub const ID: &str = "cloudflare-warp";

/// `$XDG_CONFIG_HOME/cx/mesh` (an empty XDG_CONFIG_HOME is unset, as the XDG spec says), as mesh.py
/// reads it.
pub fn config_dir() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
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
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok());
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
    fn status(&self) -> anyhow::Result<Status> {
        Ok(status_from(&quick_status()))
    }
    fn address_ranges(&self) -> Vec<IpNet> {
        quick_status().mesh_ranges
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    /// An empty XDG_CONFIG_HOME is unset: the config (and the credentials `cx secrets migrate` imports)
    /// is under ~/.config, never relative to the working directory.
    #[test]
    fn an_empty_xdg_config_home_means_home() {
        let d = tempfile::tempdir().unwrap();
        if !super::super::in_child(
            "mesh::cloudflare::tests::an_empty_xdg_config_home_means_home",
            &[("HOME", d.path()), ("XDG_CONFIG_HOME", Path::new(""))],
        ) {
            return;
        }
        let home = PathBuf::from(std::env::var_os("HOME").unwrap());
        assert_eq!(config_dir(), home.join(".config/cx/mesh"));
    }
}
