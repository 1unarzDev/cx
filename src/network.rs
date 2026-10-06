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
    // RHEL-family installations place ip outside ordinary users' PATH.
    let executable = if program == "ip" {
        ["/usr/sbin/ip", "/sbin/ip", "/usr/bin/ip"]
            .into_iter()
            .find(|path| Path::new(path).is_file())
            .unwrap_or(program)
    } else {
        program
    };
    let mut child = Command::new(executable)
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
// Candidate observation is deliberately separate from observe(): it never invokes ICMP,
// DNS, authentication, or a connection. Neighbor-cache entries are hints, not devices.
#[derive(Clone, Debug)]
struct Candidate {
    address: std::net::IpAddr,
    interface: String,
    interface_index: u32,
    link_state: String,
}
impl Candidate {
    fn value(&self) -> Value {
        json!({"address":self.address.to_string(),"hostname":null,
            "interface":self.interface,"link_state":self.link_state,"source":"neighbor",
            "ssh":{"state":"unknown"},"internet":{"state":"unknown"}})
    }
}
fn candidate_interface(name: &str) -> bool {
    !name.is_empty()
        && name.len() < libc::IFNAMSIZ
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.-:".contains(&b))
        && name != "lo"
        && ![
            "veth", "docker", "br-", "virbr", "cni", "podman", "lxc", "flannel",
        ]
        .iter()
        .any(|prefix| name.starts_with(prefix))
}
fn candidate_address(address: std::net::IpAddr) -> bool {
    match address {
        std::net::IpAddr::V4(ip) => {
            !ip.is_loopback()
                && !ip.is_unspecified()
                && !ip.is_multicast()
                && ip != std::net::Ipv4Addr::BROADCAST
        }
        std::net::IpAddr::V6(ip) => {
            !ip.is_loopback()
                && !ip.is_unspecified()
                && !ip.is_multicast()
                && ip.to_ipv4_mapped().is_none()
        }
    }
}
fn parse_candidates(
    addresses: &Value,
    neighbors: &Value,
    routes: &Value,
) -> Result<Vec<Candidate>> {
    use std::collections::{BTreeMap, BTreeSet};
    let interfaces = addresses
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("invalid address snapshot"))?;
    let neighbors = neighbors
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("invalid neighbor snapshot"))?;
    // Route observations are passive context only, never Internet evidence. Requiring
    // structured output avoids accepting a partial or corrupted iproute2 snapshot.
    routes
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("invalid route snapshot"))?;
    let mut allowed = BTreeMap::new();
    let mut local = BTreeSet::new();
    for interface in interfaces {
        if let Some(items) = interface["addr_info"].as_array() {
            for item in items {
                for field in ["local", "broadcast"] {
                    if let Some(ip) = item[field]
                        .as_str()
                        .and_then(|s| s.parse::<std::net::IpAddr>().ok())
                    {
                        local.insert(ip);
                    }
                }
            }
        }
        let Some(name) = interface["ifname"].as_str() else {
            continue;
        };
        let Some(index) = interface["ifindex"]
            .as_u64()
            .and_then(|i| u32::try_from(i).ok())
            .filter(|i| *i > 0)
        else {
            continue;
        };
        let kind = interface["linkinfo"]["info_kind"].as_str().unwrap_or("");
        if !candidate_interface(name) || matches!(kind, "veth" | "dummy") {
            continue;
        }
        if interface["flags"]
            .as_array()
            .is_some_and(|flags| flags.iter().any(|f| f == "LOOPBACK"))
        {
            continue;
        }
        allowed.insert(name, index);
    }
    let mut rows = BTreeMap::new();
    for row in neighbors {
        let Some(interface) = row["dev"].as_str() else {
            continue;
        };
        let Some(&index) = allowed.get(interface) else {
            continue;
        };
        let Some(address) = row["dst"]
            .as_str()
            .and_then(|s| s.parse::<std::net::IpAddr>().ok())
        else {
            continue;
        };
        if !candidate_address(address) || local.contains(&address) {
            continue;
        }
        let mut states: Vec<String> = row["state"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .or_else(|| row["state"].as_str().map(|s| vec![s.to_owned()]))
            .unwrap_or_default();
        states.retain(|s| {
            matches!(
                s.as_str(),
                "REACHABLE"
                    | "STALE"
                    | "DELAY"
                    | "PROBE"
                    | "PERMANENT"
                    | "NOARP"
                    | "INCOMPLETE"
                    | "FAILED"
                    | "NONE"
            )
        });
        states.sort();
        states.dedup();
        let state = if states.is_empty() {
            "unknown".to_owned()
        } else {
            states.join(",")
        };
        let key = (address, interface.to_owned());
        // Duplicate rows with differing evidence resolve deterministically.
        rows.entry(key)
            .and_modify(|c: &mut Candidate| {
                if state < c.link_state {
                    c.link_state = state.clone();
                }
            })
            .or_insert(Candidate {
                address,
                interface: interface.to_owned(),
                interface_index: index,
                link_state: state,
            });
    }
    // Keep the helper response well below the shared 1 MiB framing limit.
    if rows.len() > 1024 {
        bail!("neighbor candidate snapshot exceeds 1024 entries");
    }
    Ok(rows.into_values().collect())
}
fn candidate_snapshot() -> Result<Vec<Candidate>> {
    #[cfg(not(target_os = "linux"))]
    bail!("neighbor observation currently supports Linux");
    #[cfg(target_os = "linux")]
    {
        let addresses =
            serde_json::from_str(&bounded_command("ip", &["-j", "-d", "address", "show"])?)?;
        let neighbors = serde_json::from_str(&bounded_command("ip", &["-j", "neigh", "show"])?)?;
        let routes = serde_json::from_str(&bounded_command("ip", &["-j", "route", "show"])?)?;
        parse_candidates(&addresses, &neighbors, &routes)
    }
}
/// Passive neighbor-cache hints. Names are absent because discovery does not perform DNS.
/// Identity is (numeric address, interface); an address may occur on several links.
pub fn candidates() -> Result<Value> {
    let rows = candidate_snapshot()?;
    Ok(
        json!({"observed_at":timestamp(),"candidates":rows.iter().map(Candidate::value).collect::<Vec<_>>()}),
    )
}
fn select_candidate(
    rows: &[Candidate],
    address: &str,
    interface: Option<&str>,
) -> Result<Candidate> {
    // Accept numeric IP only; neither hostname resolution nor caller-controlled scopes.
    let ip: std::net::IpAddr = address
        .parse()
        .map_err(|_| anyhow::anyhow!("candidate address must be a numeric IP without a scope"))?;
    if interface.is_some_and(|name| !candidate_interface(name)) {
        bail!("invalid candidate interface")
    }
    let mut matches = rows
        .iter()
        .filter(|c| c.address == ip && interface.is_none_or(|name| c.interface == name));
    let candidate = matches.next().ok_or_else(|| {
        anyhow::anyhow!("address and interface are not in the current neighbor candidates")
    })?;
    if matches.next().is_some() {
        bail!("candidate address occurs on multiple interfaces; select an interface")
    }
    Ok(candidate.clone())
}
/// Explicitly test one current neighbor's TCP port 22. A successful connect does not
/// establish an SSH service, trusted host key, authenticated identity, or Internet access.
/// No application payload is sent. All socket operations stay bound to the observed link.
pub fn probe_candidate(address: &str, interface: Option<&str>) -> Result<Value> {
    let candidate = select_candidate(&candidate_snapshot()?, address, interface)?;
    let outcome = connect_candidate(&candidate);
    let mut value = candidate.value();
    value["observed_at"] = json!(timestamp());
    value["ssh"] = match outcome {
        Ok(()) => {
            json!({"state":"tcp_reachable","port":22,"authenticated":false,"method":"tcp_connect","timeout_ms":750})
        }
        Err(error) => {
            json!({"state":if error.downcast_ref::<std::io::Error>().is_some_and(|e| e.kind() == std::io::ErrorKind::ConnectionRefused) {"closed"} else {"unknown"},"port":22,"authenticated":false,"method":"tcp_connect","timeout_ms":750,"reason":crate::files::display(&error.to_string())})
        }
    };
    Ok(value)
}
#[cfg(not(target_os = "linux"))]
fn connect_candidate(_: &Candidate) -> Result<()> {
    bail!("candidate probing currently supports Linux")
}
#[cfg(target_os = "linux")]
fn connect_candidate(candidate: &Candidate) -> Result<()> {
    use std::os::fd::{FromRawFd, OwnedFd};
    let deadline = Instant::now() + Duration::from_millis(750);
    let family = if candidate.address.is_ipv4() {
        libc::AF_INET
    } else {
        libc::AF_INET6
    };
    let raw = unsafe {
        libc::socket(
            family,
            libc::SOCK_STREAM | libc::SOCK_NONBLOCK | libc::SOCK_CLOEXEC,
            0,
        )
    };
    if raw < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    let socket = unsafe { OwnedFd::from_raw_fd(raw) };
    let name = std::ffi::CString::new(candidate.interface.as_str())?;
    // Fail closed if device binding is unavailable (including permission failure).
    if unsafe {
        libc::setsockopt(
            socket.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_BINDTODEVICE,
            name.as_ptr().cast(),
            name.as_bytes_with_nul().len() as libc::socklen_t,
        )
    } != 0
    {
        return Err(std::io::Error::last_os_error().into());
    }
    // Recheck the interface index after snapshot collection to avoid recycled names.
    if unsafe { libc::if_nametoindex(name.as_ptr()) } != candidate.interface_index {
        bail!("candidate interface changed since observation");
    }
    let result = match candidate.address {
        std::net::IpAddr::V4(ip) => {
            let addr = libc::sockaddr_in {
                sin_family: libc::AF_INET as _,
                sin_port: 22u16.to_be(),
                sin_addr: libc::in_addr {
                    s_addr: u32::from_ne_bytes(ip.octets()),
                },
                sin_zero: [0; 8],
            };
            unsafe {
                libc::connect(
                    raw,
                    (&addr as *const libc::sockaddr_in).cast(),
                    std::mem::size_of_val(&addr) as _,
                )
            }
        }
        std::net::IpAddr::V6(ip) => {
            let addr = libc::sockaddr_in6 {
                sin6_family: libc::AF_INET6 as _,
                sin6_port: 22u16.to_be(),
                sin6_flowinfo: 0,
                sin6_addr: libc::in6_addr {
                    s6_addr: ip.octets(),
                },
                sin6_scope_id: if ip.is_unicast_link_local() {
                    candidate.interface_index
                } else {
                    0
                },
            };
            unsafe {
                libc::connect(
                    raw,
                    (&addr as *const libc::sockaddr_in6).cast(),
                    std::mem::size_of_val(&addr) as _,
                )
            }
        }
    };
    if result == 0 {
        return Ok(());
    }
    let error = std::io::Error::last_os_error();
    if error.raw_os_error() != Some(libc::EINPROGRESS) {
        return Err(error.into());
    }
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            bail!("TCP connect deadline exceeded")
        }
        let mut poll = libc::pollfd {
            fd: raw,
            events: libc::POLLOUT,
            revents: 0,
        };
        let ready = unsafe { libc::poll(&mut poll, 1, remaining.as_millis().max(1) as i32) };
        if ready < 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error.into());
        }
        if ready == 0 {
            bail!("TCP connect deadline exceeded")
        }
        let mut error: libc::c_int = 0;
        let mut length = std::mem::size_of_val(&error) as libc::socklen_t;
        if unsafe {
            libc::getsockopt(
                raw,
                libc::SOL_SOCKET,
                libc::SO_ERROR,
                (&mut error as *mut libc::c_int).cast(),
                &mut length,
            )
        } != 0
        {
            return Err(std::io::Error::last_os_error().into());
        }
        if error != 0 {
            return Err(std::io::Error::from_raw_os_error(error).into());
        }
        return Ok(());
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

#[cfg(test)]
mod candidate_tests {
    use super::*;
    fn addresses() -> Value {
        json!([
            {"ifname":"eth0","ifindex":2,"flags":["UP"],"addr_info":[{"local":"192.0.2.1","broadcast":"192.0.2.255"}]},
            {"ifname":"eth1","ifindex":3,"addr_info":[]},
            {"ifname":"lo","ifindex":1,"flags":["LOOPBACK"]},
            {"ifname":"veth0","ifindex":4},
            {"ifname":"renamed-container","ifindex":5,"linkinfo":{"info_kind":"veth"}},
            {"ifname":"eth\u{1b}[2J","ifindex":6},
            {"ifname":"eth\u{fffd}","ifindex":7}
        ])
    }
    #[test]
    fn candidates_are_numeric_sorted_deduplicated_and_unknown() {
        let neighbors = json!([
            {"dst":"192.0.2.100","dev":"eth0","state":["STALE"]},
            {"dst":"fe80::2","dev":"eth0","state":["REACHABLE"]},
            {"dst":"192.0.2.9","dev":"eth0","state":["REACHABLE"]},
            {"dst":"192.0.2.9","dev":"eth0","state":["STALE"]},
            {"dst":"fe80:0:0:0:0:0:0:2","dev":"eth0","state":["REACHABLE"]},
            {"dst":"192.0.2.9","dev":"eth1","state":["FAILED"]}
        ]);
        let rows = parse_candidates(&addresses(), &neighbors, &json!([])).unwrap();
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[0].address.to_string(), "192.0.2.9");
        assert_eq!(rows[0].interface, "eth0");
        assert_eq!(rows[1].interface, "eth1");
        assert_eq!(rows[2].address.to_string(), "192.0.2.100");
        assert_eq!(rows[3].address.to_string(), "fe80::2");
        let reversed = Value::Array(
            neighbors
                .as_array()
                .unwrap()
                .iter()
                .rev()
                .cloned()
                .collect(),
        );
        let reverse_rows = parse_candidates(&addresses(), &reversed, &json!([])).unwrap();
        assert_eq!(
            rows.iter().map(Candidate::value).collect::<Vec<_>>(),
            reverse_rows
                .iter()
                .map(Candidate::value)
                .collect::<Vec<_>>()
        );
        for row in rows {
            let value = row.value();
            assert_eq!(value["ssh"]["state"], "unknown");
            assert_eq!(value["internet"]["state"], "unknown");
            assert_eq!(value["source"], "neighbor");
            assert_eq!(value["hostname"], Value::Null);
        }
    }
    #[test]
    fn unsafe_and_non_neighbor_destinations_are_excluded() {
        let mut neighbors = Vec::new();
        for address in [
            "127.0.0.2",
            "::1",
            "0.0.0.0",
            "::",
            "224.0.0.1",
            "ff02::1",
            "255.255.255.255",
            "192.0.2.1",
            "192.0.2.255",
            "::ffff:192.0.2.9",
            "hostname",
            "fe80::2%eth0",
        ] {
            neighbors.push(json!({"dst":address,"dev":"eth0"}));
        }
        for interface in [
            "lo",
            "veth0",
            "renamed-container",
            "docker0",
            "eth\u{1b}[2J",
            "eth\u{fffd}",
            "absent",
        ] {
            neighbors.push(json!({"dst":"192.0.2.9","dev":interface}));
        }
        assert!(
            parse_candidates(&addresses(), &json!(neighbors), &json!([]))
                .unwrap()
                .is_empty()
        );
    }
    #[test]
    fn probe_selection_is_fresh_interface_specific_and_numeric() {
        let rows = parse_candidates(
            &addresses(),
            &json!([
                {"dst":"fe80::2","dev":"eth0"},
                {"dst":"fe80::2","dev":"eth1"},
                {"dst":"192.0.2.9","dev":"eth0"}
            ]),
            &json!([]),
        )
        .unwrap();
        for address in [
            "192.0.2.10",
            "localhost",
            "-oProxyCommand=x",
            "192.0.2.9:22",
            "fe80::2%eth0",
            "::ffff:192.0.2.9",
        ] {
            assert!(select_candidate(&rows, address, None).is_err());
        }
        assert!(select_candidate(&rows, "fe80::2", None).is_err());
        assert!(select_candidate(&rows, "192.0.2.9", Some("eth1")).is_err());
        assert!(select_candidate(&rows, "192.0.2.9", Some("eth0\0oops")).is_err());
        assert!(select_candidate(&[], "192.0.2.9", Some("eth0")).is_err());
        let selected = select_candidate(&rows, "fe80::2", Some("eth1")).unwrap();
        assert_eq!(selected.interface_index, 3);
        assert_eq!(
            select_candidate(&rows, "192.0.2.9", None)
                .unwrap()
                .interface,
            "eth0"
        );
    }
    #[test]
    fn response_candidate_count_is_bounded() {
        let neighbors: Vec<_> = (0..1025u32)
            .map(|i| json!({"dst":std::net::Ipv4Addr::from(0x0a000001 + i).to_string(),"dev":"eth0"}))
            .collect();
        assert!(parse_candidates(&addresses(), &json!(neighbors), &json!([])).is_err());
    }
    #[test]
    fn structured_snapshots_and_state_sanitization() {
        for (a, n, r) in [
            (json!(null), json!([]), json!([])),
            (addresses(), json!({}), json!([])),
            (addresses(), json!([]), json!("route")),
        ] {
            assert!(parse_candidates(&a, &n, &r).is_err());
        }
        let rows = parse_candidates(
            &addresses(),
            &json!([{"dst":"192.0.2.9","dev":"eth0","state":["\u{1b}[2J","unknown custom state"]}]),
            &json!([]),
        )
        .unwrap();
        assert_eq!(rows[0].link_state, "unknown");
    }
}
