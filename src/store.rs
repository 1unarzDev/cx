use crate::model::Device;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::{
    fs,
    path::{Path, PathBuf},
};

/// A bookmark retains a runtime reference; it never authorizes restarting it.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionPin {
    pub device: Device,
    pub session: crate::model::Session,
    pub pinned_at: u64,
}

/// Open `path` for reading with O_NOFOLLOW and return its bytes, or None when it does not exist. The file must be a
/// regular file owned by this user with no group/other bits and at most `limit` bytes; anything else is refused.
pub(crate) fn read_private(path: &Path, limit: u64) -> Result<Option<Vec<u8>>> {
    use std::io::Read;
    let file = match fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
    {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e).with_context(|| format!("read {}", path.display())),
    };
    ensure_private(&file, path)?;
    let m = file.metadata()?;
    anyhow::ensure!(
        m.len() <= limit,
        "{} exceeds its size limit",
        path.display()
    );
    // Sized up front so a value read here is not left behind in a reallocated buffer.
    let mut bytes = Vec::with_capacity(m.len() as usize + 1);
    file.take(limit + 1).read_to_end(&mut bytes)?;
    anyhow::ensure!(
        bytes.len() as u64 <= limit,
        "{} exceeds its size limit",
        path.display()
    );
    Ok(Some(bytes))
}

/// Refuse a state file that is not a regular file owned by this user without group/other bits. `file` was opened
/// with O_NOFOLLOW, so a symlink never gets here.
fn ensure_private(file: &fs::File, path: &Path) -> Result<()> {
    let m = file.metadata()?;
    anyhow::ensure!(
        m.is_file() && m.uid() == unsafe { libc::geteuid() } && m.mode() & 0o077 == 0,
        "{} is not a private file owned by this user; refusing to use it",
        path.display()
    );
    Ok(())
}

/// Write `name` in `dir`: a 0600 create_new temporary, fsync, rename over the old file, then fsync the directory.
pub(crate) fn atomic_write(dir: &Path, name: &str, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    let temporary = dir.join(format!(
        ".{name}.{}.{}.tmp",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let result = (|| -> Result<()> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&temporary, dir.join(name))?;
        if let Ok(d) = fs::File::open(dir) {
            let _ = d.sync_all();
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result.with_context(|| format!("write {}", dir.join(name).display()))
}

/// An exclusive flock on a private lock file in `dir`, held until dropped. `wait` false fails at once when busy.
pub(crate) struct PrivateLock(#[allow(dead_code)] fs::File);
pub(crate) fn private_lock(dir: &Path, name: &str, wait: bool) -> Result<Option<PrivateLock>> {
    use std::os::fd::AsRawFd;
    let path = dir.join(name);
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(&path)
        .with_context(|| format!("open {}", path.display()))?;
    ensure_private(&file, &path)?;
    let op = if wait {
        libc::LOCK_EX
    } else {
        libc::LOCK_EX | libc::LOCK_NB
    };
    if unsafe { libc::flock(file.as_raw_fd(), op) } == 0 {
        return Ok(Some(PrivateLock(file)));
    }
    let error = std::io::Error::last_os_error();
    anyhow::ensure!(
        !wait && error.kind() == std::io::ErrorKind::WouldBlock,
        "lock {}: {error}",
        path.display()
    );
    Ok(None)
}

fn private_state_name(name: &str) -> Result<()> {
    anyhow::ensure!(
        matches!(
            name,
            "session-pins.json" | "ui-navigation.json" | "edges.json"
        ),
        "invalid private state name"
    );
    Ok(())
}
pub fn read_state<T: serde::de::DeserializeOwned>(name: &str, limit: u64) -> Result<Option<T>> {
    read_state_at(&state_dir(), name, limit)
}
fn read_state_at<T: serde::de::DeserializeOwned>(
    dir: &Path,
    name: &str,
    limit: u64,
) -> Result<Option<T>> {
    private_state_name(name)?;
    read_private(&dir.join(name), limit)?
        .map(|bytes| serde_json::from_slice(&bytes).map_err(Into::into))
        .transpose()
}
pub fn write_state<T: Serialize>(name: &str, value: &T, limit: usize) -> Result<()> {
    write_state_at(&ensure()?, name, value, limit)
}
fn write_state_at<T: Serialize>(dir: &Path, name: &str, value: &T, limit: usize) -> Result<()> {
    private_state_name(name)?;
    let bytes = serde_json::to_vec(value)?;
    anyhow::ensure!(bytes.len() <= limit, "private state exceeds limit");
    atomic_write(dir, name, &bytes)
}

pub fn session_pins() -> Result<Vec<SessionPin>> {
    let pins: Vec<SessionPin> = read_state("session-pins.json", 512 * 1024)?.unwrap_or_default();
    anyhow::ensure!(pins.len() <= 128, "too many pinned sessions");
    Ok(pins)
}

/// Merge a single explicit pin change under a private lock so simultaneous viewers do not lose pins.
pub fn change_session_pin(
    pin: &SessionPin,
    remove: bool,
    matches: impl Fn(&SessionPin) -> bool,
) -> Result<Vec<SessionPin>> {
    let dir = ensure()?;
    let _lock = private_lock(&dir, "session-pins.lock", false)?.context("pins busy; retry")?;
    let mut pins = session_pins()?;
    pins.retain(|p| !matches(p));
    if !remove {
        anyhow::ensure!(pins.len() < 128, "pin limit reached");
        pins.push(pin.clone());
    }
    for pin in &mut pins {
        pin.session.activity = None;
        pin.session.agent_tasks.clear();
    }
    write_state_at(&dir, "session-pins.json", &pins, 512 * 1024)?;
    Ok(pins)
}

#[cfg(test)]
mod ui_state_tests {
    use super::*;
    #[test]
    fn ui_state_survives_reopen_and_refuses_links_permissions_and_oversize() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().unwrap();
        let value = serde_json::json!({"runtime":"ended","folder":"/project"});
        write_state_at(dir.path(), "session-pins.json", &value, 1024).unwrap();
        assert_eq!(
            read_state_at::<serde_json::Value>(dir.path(), "session-pins.json", 1024).unwrap(),
            Some(value.clone())
        );
        assert!(read_state_at::<serde_json::Value>(dir.path(), "session-pins.json", 2).is_err());
        assert!(write_state_at(dir.path(), "session-pins.json", &value, 2).is_err());
        let path = dir.path().join("session-pins.json");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(read_state_at::<serde_json::Value>(dir.path(), "session-pins.json", 1024).is_err());
        fs::remove_file(&path).unwrap();
        symlink("outside", &path).unwrap();
        assert!(read_state_at::<serde_json::Value>(dir.path(), "session-pins.json", 1024).is_err());
        assert!(write_state_at(dir.path(), "../elsewhere", &value, 1024).is_err());
        // Edge policy shares the same hardened private-state path.
        write_state_at(dir.path(), "edges.json", &value, 1024).unwrap();
        assert_eq!(
            fs::metadata(dir.path().join("edges.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert_eq!(
            read_state_at::<serde_json::Value>(dir.path(), "edges.json", 1024).unwrap(),
            Some(value)
        );
    }

    #[test]
    fn private_lock_refuses_links_and_shared_files_and_reports_busy() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().unwrap();
        let held = private_lock(dir.path(), "x.lock", true).unwrap().unwrap();
        assert_eq!(
            fs::metadata(dir.path().join("x.lock"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        // flock is per open file description: a second open in this process is refused while held.
        assert!(private_lock(dir.path(), "x.lock", false).unwrap().is_none());
        drop(held);
        assert!(private_lock(dir.path(), "x.lock", false).unwrap().is_some());
        fs::set_permissions(dir.path().join("x.lock"), fs::Permissions::from_mode(0o660)).unwrap();
        assert!(private_lock(dir.path(), "x.lock", true).is_err());
        symlink(dir.path().join("target"), dir.path().join("y.lock")).unwrap();
        assert!(private_lock(dir.path(), "y.lock", true).is_err());
        assert!(
            !dir.path().join("target").exists(),
            "a lock never creates a link's target"
        );
        // atomic_write leaves only the named 0600 file behind.
        atomic_write(dir.path(), "z.json", b"{}").unwrap();
        assert_eq!(
            read_private(&dir.path().join("z.json"), 2)
                .unwrap()
                .unwrap(),
            b"{}"
        );
        assert!(read_private(&dir.path().join("z.json"), 1).is_err());
        assert!(read_private(&dir.path().join("missing"), 1)
            .unwrap()
            .is_none());
        assert!(fs::read_dir(dir.path()).unwrap().all(|e| !e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".tmp")));
    }
}

/// A device's access posture is a local enrollment decision. It is kept out of
/// `Device` so the wire and persisted device schema remains compatible with
/// older helpers. Core records local intent, not proof of reverse access; directed
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
    #[serde(default)]
    removed: std::collections::BTreeSet<String>,
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
// One cross-process lock protects inventory and policy read-modify-write operations.
thread_local! { static DEVICE_LOCK_DEPTH: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }
pub struct DeviceGuard {
    _file: Option<fs::File>,
    _thread: std::marker::PhantomData<std::rc::Rc<()>>,
}
impl Drop for DeviceGuard {
    fn drop(&mut self) {
        DEVICE_LOCK_DEPTH.with(|depth| depth.set(depth.get() - 1));
    }
}
pub fn device_guard() -> Result<DeviceGuard> {
    use std::os::{
        fd::AsRawFd,
        unix::fs::{MetadataExt, OpenOptionsExt},
    };
    let file = if DEVICE_LOCK_DEPTH.with(|depth| depth.get() == 0) {
        let file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(ensure()?.join("peer-enrollment.lock"))?;
        let metadata = file.metadata()?;
        anyhow::ensure!(
            metadata.is_file()
                && metadata.uid() == unsafe { libc::geteuid() }
                && metadata.nlink() == 1,
            "unsafe device lock"
        );
        anyhow::ensure!(
            unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } == 0,
            "device lock unavailable"
        );
        Some(file)
    } else {
        None
    };
    DEVICE_LOCK_DEPTH.with(|depth| depth.set(depth.get() + 1));
    Ok(DeviceGuard {
        _file: file,
        _thread: std::marker::PhantomData,
    })
}
pub fn save_devices(d: &[Device]) -> Result<()> {
    let _guard = device_guard()?;
    let p = ensure()?.join("devices.json");
    let tmp = p.with_extension(format!(
        "{}.{}.tmp",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    ));
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&tmp)?;
    file.write_all(&serde_json::to_vec_pretty(d)?)?;
    file.sync_all()?;
    fs::rename(tmp, p).context("save enrolled devices")
}

/// Remove an enrolled remote device and its local policy/route records.
/// The local execution device is never removable.
pub fn remove_device(name: &str) -> Result<Device> {
    let _guard = device_guard()?;
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
    policies.removed.insert(removed.id.clone());
    write_policies(&policies)?;
    let _ = remove_route(removed.target.as_deref().unwrap_or_default());
    Ok(removed)
}

/// Change only the SSH target for an enrolled device. Its stable machine id,
/// posture and remote configuration remain unchanged.
pub fn update_target(device: &Device, target: &str) -> Result<Device> {
    let _guard = device_guard()?;
    anyhow::ensure!(device.target.is_some(), "the local device cannot be edited");
    anyhow::ensure!(valid_target(target), "invalid SSH target");
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
        policies.devices.len() <= POLICY_LIMIT && policies.removed.len() <= POLICY_LIMIT,
        "too many device policies"
    );
    Ok(policies)
}

fn write_policies(policies: &PolicyFile) -> Result<()> {
    anyhow::ensure!(
        policies.devices.len() <= POLICY_LIMIT && policies.removed.len() <= POLICY_LIMIT,
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
/// Every remote device's recorded policy by id, from one read (devices without a record use the default).
pub fn policies() -> Result<std::collections::BTreeMap<String, DevicePolicy>> {
    Ok(read_policies()?.devices)
}
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

pub fn peer_removed(id: &str) -> Result<bool> {
    Ok(read_policies()?.removed.contains(id))
}
pub fn allow_explicit_enrollment(id: &str) -> Result<()> {
    let _guard = device_guard()?;
    let mut policies = read_policies()?;
    policies.removed.remove(id);
    write_policies(&policies)
}

pub fn set_access(device: &Device, access: AccessMode) -> Result<DevicePolicy> {
    let _guard = device_guard()?;
    anyhow::ensure!(device.target.is_some(), "the local device is always core");
    let mut policies = read_policies()?;
    let entry = policies.devices.entry(device.id.clone()).or_default();
    entry.access = access;
    let updated = *entry;
    write_policies(&policies)?;
    Ok(updated)
}

pub fn set_availability(device: &Device, availability: Availability) -> Result<DevicePolicy> {
    let _guard = device_guard()?;
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
pub(crate) fn routes() -> Result<std::collections::BTreeMap<String, Vec<String>>> {
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
    if let Some((identity, known_hosts)) = crate::peer_ssh::options(target)? {
        let (account, hostname) = target.split_once('@').context("invalid managed endpoint")?;
        let hostname = hostname.trim_start_matches('[').trim_end_matches(']');
        let trust_path = known_hosts
            .to_str()
            .context("invalid managed trust path")?
            .replace('\\', "\\\\")
            .replace('"', "\\\"");
        c.arg("-i").arg(identity);
        c.arg("-o")
            .arg(format!("UserKnownHostsFile=\"{trust_path}\""));
        for option in [
            "IdentitiesOnly=yes",
            "StrictHostKeyChecking=yes",
            "GlobalKnownHostsFile=/dev/null",
            "HostKeyAlgorithms=ssh-ed25519",
            "CheckHostIP=no",
            "ProxyJump=none",
            "ProxyCommand=none",
            "Port=22",
            "ControlPath=none",
            "ControlMaster=no",
        ] {
            c.args(["-o", option]);
        }
        for option in [
            format!("HostName={hostname}"),
            format!("HostKeyAlias={hostname}"),
            format!("User={account}"),
        ] {
            c.arg("-o").arg(option);
        }
    }
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
