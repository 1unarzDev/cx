//! Read-only network observation. No discovery scan or interface mutation is implicit.
use anyhow::{bail, Result};
use serde_json::{json, Value};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::{
        fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
        io::AsRawFd,
    },
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
const OUTPUT_LIMIT: usize = 256 * 1024;
/// Execute an allowlisted observation with a strict wall clock and output bound.
fn bounded_command(program: &str, args: &[&str]) -> Result<String> {
    let mut child = Command::new(program)
        .args(args)
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let stdout = child.stdout.take().unwrap();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = stdout
            .take((OUTPUT_LIMIT + 1) as u64)
            .read_to_end(&mut bytes);
        let _ = tx.send((result, bytes));
    });
    let deadline = Instant::now() + Duration::from_secs(2);
    let outcome = loop {
        if let Some(status) = child.try_wait()? {
            break Ok(status);
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            break Err(anyhow::anyhow!("observation timed out"));
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let status = outcome?;
    let (read, bytes) = rx.recv_timeout(Duration::from_millis(200))?;
    read?;
    if bytes.len() > OUTPUT_LIMIT {
        bail!("observation exceeded output limit");
    }
    if !status.success() {
        bail!("observation unavailable");
    }
    Ok(String::from_utf8(bytes)?)
}
fn observation(program: &str, args: &[&str]) -> Value {
    match bounded_command(program, args) {
        Ok(value) => match serde_json::from_str::<Value>(&value) {
            Ok(v) => json!({"state":"observed","data":v}),
            Err(_) => json!({"state":"unknown","reason":"invalid structured output"}),
        },
        Err(e) => json!({"state":"unknown","reason":crate::files::display(&e.to_string())}),
    }
}
pub fn observe() -> Result<Value> {
    #[cfg(not(target_os = "linux"))]
    return Ok(
        json!({"backend":"unsupported","sharing":"unsupported","reason":"network observation currently supports Linux","observed_at":timestamp()}),
    );
    #[cfg(target_os = "linux")]
    {
        // Snapshots are read-only; public ICMP evidence is shared across helper processes.
        let mut interfaces = observation("ip", &["-j", "address", "show"]);
        if let Some(items) = interfaces.get_mut("data").and_then(Value::as_array_mut) {
            items.retain(|v| {
                v["ifname"] != "lo"
                    && !v["ifname"].as_str().is_some_and(|n| {
                        n.starts_with("veth") || n.starts_with("docker") || n.starts_with("br-")
                    })
            });
            for item in items.iter_mut() {
                if let Some(name) = item.get("ifname").and_then(Value::as_str) {
                    let display = crate::files::display(name);
                    item["display_name"] = json!(display);
                }
            }
        }
        let routes = observation("ip", &["-j", "route", "show"]);
        let neighbors = observation("ip", &["-j", "neigh", "show"]);
        let network_manager = match bounded_command("nmcli", &["--version"]) {
            Ok(version) => {
                json!({"state":"installed","version":crate::files::display(version.trim()),"sharing":"not implemented","reason":"No mutation occurs through observation"})
            }
            Err(_) => {
                json!({"state":"unavailable","sharing":"unsupported","reason":"NetworkManager capability unverified"})
            }
        };
        Ok(
            json!({"backend":"linux-iproute2","observed_at":timestamp(),"interfaces":interfaces,"routes":routes,"neighbors":neighbors,"network_manager":network_manager,"internet":internet_observation(),"overlay":"unknown","sharing":{"state":"unsupported","reason":"Transactional sharing backend not yet implemented"}}),
        )
    }
}
const INTERNET_TTL: u64 = 60;
const ICMP_TARGETS: [&str; 2] = ["1.1.1.1", "8.8.8.8"];
const COVERAGE:&str="Public IPv4 ICMP reachability via this host's ordinary routes and VPN policy. Does not establish DNS, HTTPS, provider health, or connectivity on another device. ICMP failures do not establish loss of internet access.";
#[derive(Clone, Debug)]
enum Probe {
    Reply,
    Failed,
    Missing,
    Deadline,
    Unavailable,
}
fn ping(target: &str) -> Probe {
    // Numeric allowlist only, no interface binding, shell, DNS or privilege change.
    if !ICMP_TARGETS.contains(&target) {
        return Probe::Unavailable;
    }
    match bounded_command("ping", &["-n", "-c", "1", "-W", "1", "-w", "2", target]) {
        Ok(_) => Probe::Reply,
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound) =>
        {
            Probe::Missing
        }
        Err(error) if error.to_string().contains("timed out") => Probe::Deadline,
        Err(error) if error.downcast_ref::<std::io::Error>().is_some() => Probe::Unavailable,
        Err(_) => Probe::Failed,
    }
}
fn evidence(now: u64, probe: &mut impl FnMut(&str) -> Probe) -> Value {
    let mut attempts = Vec::new();
    let mut reachable = false;
    for target in ICMP_TARGETS {
        let result = probe(target);
        let state = match result {
            Probe::Reply => {
                reachable = true;
                "reply"
            }
            Probe::Failed => "not_confirmed",
            Probe::Missing => "ping_unavailable",
            Probe::Deadline => "deadline",
            Probe::Unavailable => "permission_or_execution_unavailable",
        };
        attempts.push(json!({"target":target,"result":state}));
    }
    json!({"state":if reachable{"reachable"}else{"unknown"},"label":if reachable{"Public ICMP reachable"}else{"Internet access not confirmed"},"observed_at":now,"method":"icmp_ipv4","coverage":COVERAGE,"confidence":if reachable{"direct reply for at least one allowlisted public IPv4 target"}else{"no positive ICMP evidence"},"attempts":attempts,"cached":false,"stale":false})
}
fn unknown(reason: &str) -> Value {
    json!({"state":"unknown","label":"Internet access not confirmed","observed_at":null,"method":"icmp_ipv4","coverage":COVERAGE,"confidence":"no recent evidence","reason":reason,"cached":false,"stale":true})
}
fn cache_root() -> Result<PathBuf> {
    let base = std::env::var_os("XDG_STATE_HOME")
        .filter(|p| Path::new(p).is_absolute())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/state")
        });
    let cx = base.join("cx");
    fs::create_dir_all(&cx)?;
    check_directory(&cx)?;
    let path = cx.join("observations");
    let mut builder = fs::DirBuilder::new();
    use std::os::unix::fs::DirBuilderExt;
    builder.mode(0o700);
    match builder.create(&path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e.into()),
    }
    check_directory(&path)?;
    Ok(path)
}
fn private_directory(path: &Path) -> Result<File> {
    let mut opts = OpenOptions::new();
    opts.read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_DIRECTORY);
    let file = opts.open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_dir() || metadata.uid() != unsafe { libc::geteuid() } {
        bail!("cache directory is not privately owned");
    }
    if metadata.permissions().mode() & 0o077 != 0 {
        file.set_permissions(fs::Permissions::from_mode(0o700))?;
    }
    Ok(file)
}
fn check_directory(path: &Path) -> Result<()> {
    private_directory(path)?;
    Ok(())
}
fn private_file(path: &Path, write: bool) -> Result<File> {
    let mut opts = OpenOptions::new();
    opts.read(true)
        .write(write)
        .create(write)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK);
    let file = opts.open(path)?;
    let m = file.metadata()?;
    if !m.is_file() || m.uid() != unsafe { libc::geteuid() } || m.permissions().mode() & 0o077 != 0
    {
        bail!("cache file is not private");
    }
    Ok(file)
}
struct ProbeLock(File);
impl Drop for ProbeLock {
    fn drop(&mut self) {
        unsafe {
            libc::flock(self.0.as_raw_fd(), libc::LOCK_UN);
        }
    }
}
fn cached(root: &Path) -> Option<Value> {
    let file = private_file(&root.join("internet.json"), false).ok()?;
    if file.metadata().ok()?.len() > 8192 {
        return None;
    }
    let value: Value = serde_json::from_reader(file.take(8193)).ok()?;
    if !matches!(value["state"].as_str(), Some("reachable" | "unknown"))
        || value["method"] != "icmp_ipv4"
        || value["observed_at"].as_u64().is_none()
    {
        return None;
    }
    Some(value)
}
fn recent(value: &Value, now: u64) -> bool {
    value["observed_at"]
        .as_u64()
        .is_some_and(|observed| observed <= now && now - observed < INTERNET_TTL)
}
fn shared_observation(
    root: &Path,
    now: u64,
    probe: &mut impl FnMut(&str) -> Probe,
) -> Result<Value> {
    let directory = private_directory(root)?;
    // Pin every cache/lock operation to the opened directory, so replacing the
    // path while a probe runs cannot redirect writes or split its lock.
    let anchored = PathBuf::from(format!("/proc/self/fd/{}", directory.as_raw_fd()));
    let root = anchored.as_path();
    if let Some(mut value) = cached(root).filter(|v| recent(v, now)) {
        value["cached"] = json!(true);
        return Ok(value);
    }
    let lock = private_file(&root.join("internet.lock"), true)?;
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        let mut value =
            cached(root).unwrap_or_else(|| unknown("another helper is checking reachability"));
        value["previous_state"] = value["state"].clone();
        value["state"] = json!("unknown");
        value["label"] = json!("Reachability checking");
        value["stale"] = json!(true);
        value["cached"] = json!(true);
        value["refreshing"] = json!(true);
        return Ok(value);
    }
    let _lock = ProbeLock(lock);
    // Recheck after locking: another helper may have completed since the first read.
    if let Some(mut value) = cached(root).filter(|v| recent(v, now)) {
        value["cached"] = json!(true);
        return Ok(value);
    }
    let value = evidence(now, probe);
    let temp = root.join(format!("internet.{}.tmp", std::process::id()));
    let mut options = OpenOptions::new();
    options
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    let result = (|| -> Result<()> {
        let mut file = options.open(&temp)?;
        file.write_all(&serde_json::to_vec(&value)?)?;
        file.sync_all()?;
        fs::rename(&temp, root.join("internet.json"))?;
        directory.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    let mut value = value;
    if result.is_err() {
        value["cache_warning"] = json!("Reachability cache could not be persisted");
    }
    Ok(value)
}
fn internet_observation() -> Value {
    match cache_root().and_then(|root| shared_observation(&root, timestamp(), &mut ping)) {
        Ok(value) => value,
        Err(_) => unknown("private reachability cache unavailable; automatic probes withheld"),
    }
}
fn timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn command_failure_is_unknown() {
        assert_eq!(
            observation("cx-nonexistent-observer", &[])["state"],
            "unknown"
        );
    }
    #[test]
    fn malformed_output_is_unknown() {
        assert_eq!(observation("printf", &["not-json"])["state"], "unknown");
    }
    #[test]
    fn observation_timeout_is_bounded() {
        let started = Instant::now();
        assert!(bounded_command("sleep", &["5"]).is_err());
        assert!(started.elapsed() < Duration::from_secs(3));
    }
    #[test]
    fn observation_output_is_bounded() {
        assert!(bounded_command("head", &["-c", "1048576", "/dev/zero"]).is_err());
    }
    #[test]
    fn probe_success_has_limited_coverage() {
        let mut calls = 0;
        let value = evidence(100, &mut |_| {
            calls += 1;
            if calls == 1 {
                Probe::Reply
            } else {
                Probe::Failed
            }
        });
        assert_eq!(calls, 2);
        assert_eq!(value["state"], "reachable");
        assert_eq!(value["observed_at"], 100);
        assert!(value["coverage"]
            .as_str()
            .unwrap()
            .contains("Does not establish DNS"));
    }
    #[test]
    fn all_failures_are_unknown() {
        for result in [
            Probe::Failed,
            Probe::Missing,
            Probe::Deadline,
            Probe::Unavailable,
        ] {
            let value = evidence(100, &mut |_| result.clone());
            assert_eq!(value["state"], "unknown");
            assert_eq!(value["attempts"].as_array().unwrap().len(), 2);
        }
    }
    #[test]
    fn cached_and_concurrent_probes_coalesce() {
        let root = tempfile::tempdir().unwrap();
        fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let mut calls = 0;
        let fresh = shared_observation(root.path(), 100, &mut |_| {
            calls += 1;
            Probe::Reply
        })
        .unwrap();
        assert_eq!(fresh["cached"], false);
        assert_eq!(calls, 2);
        let value = shared_observation(root.path(), 159, &mut |_| {
            panic!("fresh cache must not probe")
        })
        .unwrap();
        assert_eq!(value["cached"], true);
        let lock = private_file(&root.path().join("internet.lock"), true).unwrap();
        assert_eq!(
            unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
            0
        );
        let value = shared_observation(root.path(), 160, &mut |_| {
            panic!("concurrent refresh must not probe")
        })
        .unwrap();
        assert_eq!(value["state"], "unknown");
        assert_eq!(value["previous_state"], "reachable");
        assert_eq!(value["observed_at"], 100);
        assert_eq!(value["refreshing"], true);
        drop(ProbeLock(lock));
        let value = shared_observation(root.path(), 160, &mut |_| Probe::Failed).unwrap();
        assert_eq!(value["observed_at"], 160);
        assert_eq!(value["state"], "unknown");
    }
    #[test]
    fn overlapping_helpers_have_one_probe_pair() {
        let root = tempfile::tempdir().unwrap();
        fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let path = root.path().to_owned();
        let worker_path = path.clone();
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            let mut count = 0;
            shared_observation(&worker_path, 100, &mut |_| {
                count += 1;
                if count == 1 {
                    started_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                }
                Probe::Reply
            })
            .unwrap()
        });
        started_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        let competing = shared_observation(&path, 100, &mut |_| {
            panic!("overlapping helper must not probe")
        })
        .unwrap();
        assert_eq!(competing["refreshing"], true);
        release_tx.send(()).unwrap();
        assert_eq!(worker.join().unwrap()["state"], "reachable");
        assert_eq!(
            shared_observation(&path, 101, &mut |_| panic!(
                "completed cache must be reused"
            ))
            .unwrap()["cached"],
            true
        );
    }
    #[test]
    fn future_corrupt_and_symlink_caches_are_not_trusted() {
        let root = tempfile::tempdir().unwrap();
        fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
        shared_observation(root.path(), 200, &mut |_| Probe::Reply).unwrap();
        let value = shared_observation(root.path(), 100, &mut |_| Probe::Failed).unwrap();
        assert_eq!(value["state"], "unknown");
        fs::write(root.path().join("internet.json"), b"broken").unwrap();
        assert!(cached(root.path()).is_none());
        let target = root.path().join("target");
        fs::write(&target, b"private").unwrap();
        fs::remove_file(root.path().join("internet.lock")).unwrap();
        std::os::unix::fs::symlink(&target, root.path().join("internet.lock")).unwrap();
        assert!(shared_observation(root.path(), 300, &mut |_| panic!(
            "unsafe cache must not probe"
        ))
        .is_err());
        assert_eq!(fs::read(target).unwrap(), b"private");
    }
}
