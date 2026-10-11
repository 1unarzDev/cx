use crate::model::Device;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::os::unix::fs::PermissionsExt;
use std::{fs, path::PathBuf};

/// A device's access posture is a local enrollment decision. It is kept out of
/// `Device` so the wire and persisted device schema remains compatible with
/// older helpers. Core devices are approved for bidirectional access; directed
/// devices retain the default viewer-to-device posture.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AccessMode {
    #[default]
    Directed,
    Core,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Availability {
    #[default]
    UsuallyUp,
    UsuallyDown,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct DevicePolicy {
    #[serde(default)]
    pub access: AccessMode,
    #[serde(default)]
    pub availability: Availability,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct PolicyFile {
    #[serde(default)]
    devices: std::collections::BTreeMap<String, DevicePolicy>,
}

const POLICY_LIMIT: usize = 128;
const POLICY_BYTES: u64 = 64 * 1024;
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

/// Remove an enrolled remote device and its local policy/route records.
/// The local execution device is never removable.
pub fn remove_device(name: &str) -> Result<Device> {
    let mut devices = devices()?;
    let index = devices
        .iter()
        .position(|d| {
            d.target.is_some()
                && (d.name == name || d.id == name || d.target.as_deref() == Some(name))
        })
        .context("device not enrolled")?;
    let removed = devices.remove(index);
    save_devices(&devices)?;

    let mut policies = read_policies()?;
    policies.devices.remove(&removed.id);
    write_policies(&policies)?;
    let _ = remove_route(removed.target.as_deref().unwrap_or_default());
    Ok(removed)
}

/// Change only the SSH target for an enrolled device. Its stable machine id,
/// posture and remote configuration remain unchanged.
pub fn update_target(device: &Device, target: &str) -> Result<Device> {
    anyhow::ensure!(device.target.is_some(), "the local device cannot be edited");
    anyhow::ensure!(crate::transport::valid_target(target), "invalid SSH target");
    let mut devices = devices()?;
    anyhow::ensure!(
        !devices
            .iter()
            .any(|d| d.id != device.id && d.target.as_deref() == Some(target)),
        "SSH target is already enrolled for another device"
    );
    let entry = devices
        .iter_mut()
        .find(|d| d.id == device.id)
        .context("device not enrolled")?;
    let previous = entry.target.clone();
    entry.target = Some(target.to_owned());
    entry.status = "unknown".into();
    entry.observed_at = 0;
    let updated = entry.clone();
    save_devices(&devices)?;
    if previous.as_deref() != Some(target) {
        if let Some(previous) = previous {
            let _ = remove_route(&previous);
        }
    }
    Ok(updated)
}

fn policy_path() -> PathBuf {
    state_dir().join("device-policies.json")
}

fn read_policies() -> Result<PolicyFile> {
    let path = policy_path();
    let metadata = match fs::symlink_metadata(&path) {
        Ok(value) => value,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(PolicyFile::default())
        }
        Err(error) => return Err(error.into()),
    };
    use std::os::unix::fs::MetadataExt;
    anyhow::ensure!(
        metadata.is_file()
            && metadata.uid() == unsafe { libc::geteuid() }
            && metadata.mode() & 0o077 == 0
            && metadata.len() <= POLICY_BYTES,
        "unsafe device policy store"
    );
    let policies: PolicyFile = serde_json::from_slice(&fs::read(path)?)?;
    anyhow::ensure!(
        policies.devices.len() <= POLICY_LIMIT,
        "too many device policies"
    );
    Ok(policies)
}

fn write_policies(policies: &PolicyFile) -> Result<()> {
    anyhow::ensure!(
        policies.devices.len() <= POLICY_LIMIT,
        "too many device policies"
    );
    let dir = ensure()?;
    let path = policy_path();
    let temporary = dir.join(format!(
        "device-policies.{}.{}.tmp",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temporary)?;
    let result = (|| -> Result<()> {
        let bytes = serde_json::to_vec_pretty(policies)?;
        anyhow::ensure!(
            bytes.len() <= POLICY_BYTES as usize,
            "device policy store is too large"
        );
        file.write_all(&bytes)?;
        file.sync_all()?;
        fs::rename(&temporary, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

/// Return the effective policy. The local execution device is always core;
/// remote devices default to one-way/direct access for safe migration.
pub fn policy(device: &Device) -> Result<DevicePolicy> {
    if device.target.is_none() {
        return Ok(DevicePolicy {
            access: AccessMode::Core,
            availability: Availability::UsuallyUp,
        });
    }
    Ok(read_policies()?
        .devices
        .get(&device.id)
        .copied()
        .unwrap_or_default())
}

pub fn set_access(device: &Device, access: AccessMode) -> Result<DevicePolicy> {
    anyhow::ensure!(device.target.is_some(), "the local device is always core");
    let mut policies = read_policies()?;
    let entry = policies.devices.entry(device.id.clone()).or_default();
    entry.access = access;
    let updated = *entry;
    write_policies(&policies)?;
    Ok(updated)
}

pub fn set_availability(device: &Device, availability: Availability) -> Result<DevicePolicy> {
    anyhow::ensure!(
        device.target.is_some(),
        "the local device is always available"
    );
    let mut policies = read_policies()?;
    let entry = policies.devices.entry(device.id.clone()).or_default();
    entry.availability = availability;
    let updated = *entry;
    write_policies(&policies)?;
    Ok(updated)
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

fn remove_route(target: &str) -> Result<()> {
    if target.is_empty() || !valid_target(target) {
        return Ok(());
    }
    set_route(target, &[])
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
// ProxyJump inherits -F, but not the outer process's -o options. Put the
// noninteractive policy before the normal configs so every hop fails closed.
fn background_ssh_config() -> Result<PathBuf> {
    const CONFIG: &[u8] = b"Host *\n  BatchMode yes\n  ForwardAgent no\n  PreferredAuthentications publickey,gssapi-keyex,gssapi-with-mic,hostbased,keyboard-interactive,password\n  ConnectTimeout 6\n  ServerAliveInterval 5\n  ServerAliveCountMax 2\nInclude ~/.ssh/config\nInclude /etc/ssh/ssh_config\n";
    let dir = ensure()?;
    let path = dir.join("ssh-background-v2.conf");
    match fs::symlink_metadata(&path) {
        Ok(m) => {
            use std::os::unix::fs::MetadataExt;
            anyhow::ensure!(
                m.is_file()
                    && m.uid() == unsafe { libc::geteuid() }
                    && m.mode() & 0o077 == 0
                    && m.len() <= 4096,
                "unsafe background SSH configuration"
            );
            anyhow::ensure!(
                fs::read(&path)? == CONFIG,
                "background SSH configuration changed"
            );
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            use std::io::Write;
            use std::os::unix::fs::OpenOptionsExt;
            let temporary = dir.join(format!(
                "ssh-background-v2.{}.{}.tmp",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)?
                    .as_nanos()
            ));
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&temporary)?;
            let result = (|| -> Result<()> {
                file.write_all(CONFIG)?;
                file.sync_all()?;
                fs::rename(&temporary, &path)?;
                Ok(())
            })();
            if result.is_err() {
                let _ = fs::remove_file(&temporary);
            }
            result?;
        }
        Err(e) => return Err(e.into()),
    }
    Ok(path)
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
        "-o",
        "PreferredAuthentications=publickey,gssapi-keyex,gssapi-with-mic,hostbased,keyboard-interactive,password",
    ]);
    if !interactive {
        c.args(["-o", "BatchMode=yes"]);
    }
    let hops = route(target)?;
    if !interactive {
        // Covers both cx routes and ProxyJump from the user's SSH configuration.
        c.arg("-F").arg(background_ssh_config()?);
    }
    if !hops.is_empty() {
        c.args(["-J", &hops.join(",")]);
    }
    if interactive && crate::auth::configure(&mut c)? {
        c.stderr(std::process::Stdio::null());
    }
    // A private enrollment master carries authenticated access, never a saved password.
    use sha2::{Digest, Sha256};
    let sockets = ensure()?.join("ssh");
    match std::os::unix::fs::DirBuilderExt::mode(&mut fs::DirBuilder::new(), 0o700).create(&sockets)
    {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error.into()),
    }
    use std::os::unix::fs::MetadataExt;
    let metadata = fs::symlink_metadata(&sockets)?;
    anyhow::ensure!(
        metadata.is_dir()
            && metadata.uid() == unsafe { libc::geteuid() }
            && metadata.mode() & 0o077 == 0,
        "unsafe SSH connection directory"
    );
    let route_identity = format!("{target}:{hops:?}");
    let digest = format!("{:x}", Sha256::digest(route_identity.as_bytes()));
    let socket = sockets.join(format!("{}-%C", &digest[..8]));
    anyhow::ensure!(
        socket.as_os_str().len() + 38 < 104,
        "SSH connection path too long"
    );
    c.arg("-o").arg(format!("ControlPath={}", socket.display()));
    c.args([
        "-o",
        if crate::auth::active() {
            "ControlMaster=auto"
        } else {
            "ControlMaster=no"
        },
    ]);
    if crate::auth::active() {
        c.args(["-o", "ControlPersist=600"]);
    }
    c.arg(target);
    Ok(c)
}
