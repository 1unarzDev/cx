use crate::model::Device;
use anyhow::{Context, Result};
use std::os::unix::fs::PermissionsExt;
use std::{fs, path::PathBuf};
pub fn state_dir() -> PathBuf {
    std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/state")
        })
        .join("cx")
}
pub fn ensure() -> Result<PathBuf> {
    let p = state_dir();
    fs::create_dir_all(&p)?;
    use std::os::unix::fs::MetadataExt;
    let m = fs::symlink_metadata(&p)?;
    anyhow::ensure!(
        m.is_dir() && m.uid() == unsafe { libc::geteuid() },
        "unsafe cx state directory"
    );
    fs::set_permissions(&p, fs::Permissions::from_mode(0o700))?;
    Ok(p)
}
pub fn local_device() -> Device {
    Device {
        id: "local".into(),
        name: std::fs::read_to_string("/proc/sys/kernel/hostname")
            .unwrap_or_else(|_| "local".into())
            .trim()
            .into(),
        target: None,
        account: std::env::var("USER").unwrap_or_else(|_| "unknown".into()),
        host: std::fs::read_to_string("/proc/sys/kernel/hostname")
            .unwrap_or_else(|_| "local".into())
            .trim()
            .into(),
        status: "local".into(),
        observed_at: 0,
    }
}
pub fn devices() -> Result<Vec<Device>> {
    let p = state_dir().join("devices.json");
    if !p.exists() {
        return Ok(vec![local_device()]);
    }
    let mut d: Vec<Device> = serde_json::from_slice(&fs::read(p)?)?;
    if !d.iter().any(|d| d.target.is_none()) {
        d.insert(0, local_device())
    }
    Ok(d)
}
pub fn save_devices(d: &[Device]) -> Result<()> {
    let p = ensure()?.join("devices.json");
    let tmp = p.with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_vec_pretty(d)?)?;
    fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600))?;
    fs::rename(tmp, p).context("save enrolled devices")
}

// Transport routes are local enrollment decisions, separate from peer observations.
// No discovered hostname is allowed to install a key or change this map.
const ROUTE_LIMIT: usize = 128;
fn validate_route(target: &str, hops: &[String]) -> Result<()> {
    anyhow::ensure!(valid_target(target), "invalid route target");
    anyhow::ensure!(hops.len() <= 4, "at most four SSH jumps are supported");
    let mut unique = std::collections::HashSet::new();
    for hop in hops {
        anyhow::ensure!(
            valid_target(hop) && hop != target && unique.insert(hop),
            "invalid or cyclic SSH jump route"
        );
    }
    Ok(())
}
fn routes() -> Result<std::collections::BTreeMap<String, Vec<String>>> {
    let path = state_dir().join("routes.json");
    let metadata = match fs::symlink_metadata(&path) {
        Ok(value) => value,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Default::default()),
        Err(e) => return Err(e.into()),
    };
    use std::os::unix::fs::MetadataExt;
    anyhow::ensure!(
        metadata.is_file()
            && metadata.uid() == unsafe { libc::geteuid() }
            && metadata.mode() & 0o077 == 0
            && metadata.len() <= 65536,
        "unsafe SSH route store"
    );
    let result: std::collections::BTreeMap<String, Vec<String>> =
        serde_json::from_slice(&fs::read(path)?)?;
    anyhow::ensure!(result.len() <= ROUTE_LIMIT, "too many SSH routes");
    for (target, hops) in &result {
        validate_route(target, hops)?;
    }
    Ok(result)
}
pub fn route(target: &str) -> Result<Vec<String>> {
    Ok(routes()?.remove(target).unwrap_or_default())
}
pub fn set_route(target: &str, hops: &[String]) -> Result<()> {
    validate_route(target, hops)?;
    let mut routes = routes()?;
    if hops.is_empty() {
        routes.remove(target);
    } else {
        routes.insert(target.into(), hops.to_vec());
    }
    anyhow::ensure!(routes.len() <= ROUTE_LIMIT, "too many SSH routes");
    use std::os::unix::fs::OpenOptionsExt;
    let dir = ensure()?;
    let path = dir.join("routes.json");
    let temporary = dir.join(format!(
        "routes.{}.{}.tmp",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temporary)?;
    use std::io::Write;
    let result = (|| -> Result<()> {
        file.write_all(&serde_json::to_vec(&routes)?)?;
        file.sync_all()?;
        fs::rename(&temporary, &path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}
pub fn route_via(target: &str, via: &Device) -> Result<Vec<String>> {
    let Some(gateway) = &via.target else {
        return Ok(Vec::new());
    };
    let mut hops = route(gateway)?;
    hops.push(gateway.clone());
    validate_route(target, &hops)?;
    Ok(hops)
}
#[cfg(test)]
mod route_tests {
    use super::*;
    #[test]
    fn jump_routes_are_bounded_validated_and_acyclic() {
        assert!(validate_route("robot@192.168.0.2", &["tranquility".into()]).is_ok());
        for hops in [
            vec!["robot".into()],
            vec!["gateway".into(), "gateway".into()],
            vec!["-oProxyCommand=evil".into()],
            vec!["a".into(), "b".into(), "c".into(), "d".into(), "e".into()],
        ] {
            assert!(validate_route("robot", &hops).is_err());
        }
    }
}

pub fn valid_target(s: &str) -> bool {
    !s.is_empty()
        && s.len() < 256
        && !s.starts_with('-')
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"@._-:[]".contains(&b))
}
pub fn ssh(target: &str, interactive: bool) -> Result<std::process::Command> {
    if !valid_target(target) {
        anyhow::bail!("invalid SSH target")
    };
    let mut c = std::process::Command::new("ssh");
    c.args([
        "-o",
        "ConnectTimeout=6",
        "-o",
        "ServerAliveInterval=5",
        "-o",
        "ServerAliveCountMax=2",
        "-o",
        "ForwardAgent=no",
    ]);
    if !interactive {
        c.args(["-o", "BatchMode=yes"]);
    }
    let hops = route(target)?;
    if !hops.is_empty() {
        c.args(["-J", &hops.join(",")]);
    }
    c.arg(target);
    Ok(c)
}
