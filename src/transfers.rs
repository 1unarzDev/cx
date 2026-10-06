//! Durable endpoint/relay transfer workers. The viewer never carries file bytes itself.
use crate::{
    files::{decode_path, display, encode_path},
    model::{Operation, TransferSpec},
    transport,
};
use anyhow::{bail, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
#[cfg(unix)]
use std::os::unix::{
    fs::{OpenOptionsExt, PermissionsExt},
    io::AsRawFd,
    process::CommandExt,
};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};
const CHUNK: u32 = 128 * 1024;
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn valid_key(key: &str) -> Result<()> {
    if key.is_empty()
        || key.len() > 64
        || !key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
    {
        bail!("transfer key must be 1–64 ASCII letters, digits, hyphens or underscores");
    }
    Ok(())
}
fn root() -> Result<PathBuf> {
    let root = crate::store::ensure()?.join("transfers");
    fs::create_dir_all(&root)?;
    let m = fs::symlink_metadata(&root)?;
    if !m.is_dir() || m.is_symlink() {
        bail!("transfer state must be a private directory");
    }
    #[cfg(unix)]
    {
        if m.uid() != unsafe { libc::geteuid() } {
            bail!("transfer state has another owner");
        }
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700))?;
    }
    Ok(root)
}
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    let mut opts = OpenOptions::new();
    opts.read(true);
    #[cfg(unix)]
    opts.custom_flags(libc::O_NOFOLLOW);
    let file = opts.open(path)?;
    if !file.metadata()?.is_file() {
        bail!("state is not a regular file");
    }
    Ok(serde_json::from_reader(file.take(64 * 1024 * 1024))?)
}
fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let temp = path.with_extension(format!("{}.tmp", std::process::id()));
    let mut opts = OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    opts.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    let mut file = opts.open(&temp)?;
    file.write_all(&serde_json::to_vec(value)?)?;
    file.sync_all()?;
    fs::rename(temp, path)?;
    Ok(())
}
struct Lock(File);
impl Lock {
    fn acquire(root: &Path, key: &str) -> Result<Self> {
        let mut opts = OpenOptions::new();
        opts.read(true).write(true).create(true);
        #[cfg(unix)]
        opts.mode(0o600).custom_flags(libc::O_NOFOLLOW);
        let file = opts.open(root.join(format!("{key}.lock")))?;
        #[cfg(unix)]
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            bail!("transfer is already running");
        }
        Ok(Self(file))
    }
}
impl Drop for Lock {
    fn drop(&mut self) {
        #[cfg(unix)]
        unsafe {
            libc::flock(self.0.as_raw_fd(), libc::LOCK_UN);
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Entry {
    source: String,
    destination: String,
    kind: String,
    identity: String,
    size: u64,
    mode: u32,
    target: Option<String>,
    status: String,
    bytes: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Job {
    key: String,
    source_host: String,
    destination_host: String,
    source_path: String,
    destination_path: String,
    route: String,
    status: String,
    bytes: u64,
    total: u64,
    updated: u64,
    error: Option<String>,
    entries: Vec<Entry>,
}
fn job_path(root: &Path, key: &str) -> PathBuf {
    root.join(format!("{key}.job.json"))
}
fn spec_path(root: &Path, key: &str) -> PathBuf {
    root.join(format!("{key}.spec.json"))
}
fn same_spec(a: &TransferSpec, b: &TransferSpec) -> bool {
    a.key == b.key
        && a.source.id == b.source.id
        && a.source.target == b.source.target
        && a.source.account == b.source.account
        && a.destination.id == b.destination.id
        && a.destination.target == b.destination.target
        && a.destination.account == b.destination.account
        && a.source_path == b.source_path
        && a.destination_path == b.destination_path
        && a.conflict == b.conflict
}
fn summary(job: &Job) -> Value {
    json!({"key":job.key,"source_host":job.source_host,"destination_host":job.destination_host,"source_path":job.source_path,"destination_path":job.destination_path,"source_display":display(&decode_path(&job.source_path).map(|p|p.to_string_lossy().into_owned()).unwrap_or_else(|_|job.source_path.clone())),"destination_display":display(&decode_path(&job.destination_path).map(|p|p.to_string_lossy().into_owned()).unwrap_or_else(|_|job.destination_path.clone())),"route":job.route,"status":job.status,"bytes":job.bytes,"total":job.total,"updated":job.updated,"error":job.error,"items":job.entries.len()})
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct DirectRef {
    owner: crate::model::Device,
    last: Value,
}
fn direct_path(root: &Path, key: &str) -> PathBuf {
    root.join(format!("{key}.direct.json"))
}
fn direct_spec(spec: &TransferSpec, owner: &crate::model::Device) -> TransferSpec {
    let mut spec = spec.clone();
    if owner.id == spec.source.id {
        spec.source.target = None;
    } else {
        spec.destination.target = None;
    }
    spec
}
fn direct_result(mut value: Value, owner: &crate::model::Device) -> Value {
    value["route"] = json!(format!("direct on {}@{}", owner.account, owner.host));
    value["worker_host"] = json!(format!("{}@{}", owner.account, owner.host));
    value
}
fn verified_endpoint(value: &Value, device: &crate::model::Device) -> bool {
    value["account"].as_str() == Some(device.account.as_str())
        && value["host"].as_str() == Some(device.host.as_str())
        && (!device.id.contains(':')
            || device
                .id
                .split_once(':')
                .is_some_and(|(id, _)| value["machine_id"].as_str() == Some(id)))
}
pub fn start(spec: &TransferSpec) -> Result<Value> {
    valid_key(&spec.key)?;
    // Resolve relative locations on their actual execution hosts before detaching.
    // systemd's working directory must never silently retarget a CLI copy.
    let mut normalized = spec.clone();
    normalized.source_path = text(&info(&spec.source, &spec.source_path)?, "path")?.to_owned();
    normalized.destination_path =
        text(&info(&spec.destination, &spec.destination_path)?, "path")?.to_owned();
    let spec = &normalized;
    if !matches!(spec.conflict.as_str(), "skip" | "overwrite" | "rename") {
        bail!("invalid conflict policy");
    }
    let root = root()?;
    let _lock = Lock::acquire(&root, &format!("{}-start", spec.key))?;
    let path = spec_path(&root, &spec.key);
    if path.try_exists()? {
        let previous: TransferSpec = read_json(&path)?;
        if !same_spec(&previous, spec) {
            bail!("transfer key belongs to another source or destination");
        }
    } else {
        write_json(&path, spec)?;
    }
    let record = job_path(&root, &spec.key);
    if record.try_exists()? {
        let job: Job = read_json(&record)?;
        if job.status == "complete" || Lock::acquire(&root, &spec.key).is_err() {
            return Ok(summary(&job));
        }
    }
    let direct = direct_path(&root, &spec.key);
    if direct.try_exists()? {
        let mut reference: DirectRef = read_json(&direct)?;
        let result = transport::request(
            &reference.owner,
            Operation::Transfer(direct_spec(spec, &reference.owner)),
        )
        .context("owning endpoint unavailable; existing job remains there")?;
        reference.last = direct_result(result, &reference.owner);
        write_json(&direct, &reference)?;
        return Ok(reference.last);
    }
    if spec.source.target.is_some() && spec.destination.target.is_some() {
        // Prove destination authentication from the actual candidate, without prompts,
        // key copying or agent forwarding. No job is submitted merely on mesh evidence.
        for (candidate, other) in [
            (&spec.source, &spec.destination),
            (&spec.destination, &spec.source),
        ] {
            let probe = transport::request(
                candidate,
                Operation::TransferReachability {
                    destination: other.clone(),
                },
            );
            if probe
                .as_ref()
                .is_ok_and(|value| verified_endpoint(value, other))
            {
                let reference = DirectRef {
                    owner: candidate.clone(),
                    last: json!({"key":spec.key,"status":"submitting","route":format!("direct on {}@{}",candidate.account,candidate.host)}),
                };
                // Record the owning endpoint before submission, so a lost response never
                // silently reroutes a potentially running job to another machine.
                write_json(&direct, &reference)?;
                let result = transport::request(
                    candidate,
                    Operation::Transfer(direct_spec(spec, candidate)),
                )
                .context("endpoint submission response lost; retry reconciles the same job")?;
                let reference = DirectRef {
                    owner: candidate.clone(),
                    last: direct_result(result, candidate),
                };
                write_json(&direct, &reference)?;
                return Ok(reference.last);
            }
        }
    }
    let cancellation = root.join(format!("{}.cancel", spec.key));
    if cancellation.try_exists()? {
        fs::remove_file(cancellation)?;
    }
    let queued = Job {
        key: spec.key.clone(),
        source_host: format!("{}@{}", spec.source.account, spec.source.host),
        destination_host: format!("{}@{}", spec.destination.account, spec.destination.host),
        source_path: spec.source_path.clone(),
        destination_path: spec.destination_path.clone(),
        route: if spec.source.target.is_some() && spec.destination.target.is_some() {
            "relay through worker host"
        } else {
            "worker host is endpoint"
        }
        .into(),
        status: "queued".into(),
        bytes: 0,
        total: 0,
        updated: now(),
        error: None,
        entries: Vec::new(),
    };
    if !record.try_exists()? {
        write_json(&record, &queued)?;
    }
    let executable = std::env::current_exe()?;
    let unit = format!("cx-transfer-{}", spec.key);
    let result = Command::new("systemd-run")
        .args([
            "--user",
            "--quiet",
            "--collect",
            "--unit",
            &unit,
            "--property=Type=exec",
            "--",
        ])
        .arg(&executable)
        .arg("transfer-worker")
        .arg(&path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    let backend = if result.is_ok_and(|s| s.success()) {
        "systemd-user-unit"
    } else {
        // setsid alone cannot escape a service's cgroup. Refuse that unsafe fallback.
        let cgroup = fs::read_to_string("/proc/self/cgroup").unwrap_or_default();
        if cgroup.lines().any(|line| line.contains(".service")) {
            bail!("user service manager unavailable; cannot detach safely from a service cgroup");
        }
        let mut command = Command::new(&executable);
        command
            .arg("transfer-worker")
            .arg(&path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(unix)]
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        command.spawn()?;
        "detached-process (logind persistence unverified)"
    };
    Ok(
        json!({"key":spec.key,"status":"queued","backend":backend,"route":if spec.source.target.is_some() && spec.destination.target.is_some(){"relay through this host"}else{"this host is endpoint"},"source_host":spec.source.host,"destination_host":spec.destination.host}),
    )
}
pub fn jobs() -> Result<Value> {
    let root = root()?;
    let mut values = Vec::new();
    let mut observations: std::collections::HashMap<String, Option<Value>> =
        std::collections::HashMap::new();
    for entry in fs::read_dir(&root)?.take(4096) {
        let path = entry?.path();
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if name.ends_with(".job.json") {
            if let Ok(mut job) = read_json::<Job>(&path) {
                if job.status == "running" && Lock::acquire(&root, &job.key).is_ok() {
                    job.status = "incomplete".into();
                }
                values.push(summary(&job));
            }
        } else if name.ends_with(".direct.json") {
            if let Ok(mut reference) = read_json::<DirectRef>(&path) {
                let key = name.trim_end_matches(".direct.json");
                let observation = observations
                    .entry(reference.owner.id.clone())
                    .or_insert_with(|| {
                        transport::request(&reference.owner, Operation::TransferJobs).ok()
                    });
                match observation {
                    Some(jobs) => {
                        if let Some(job) = jobs["jobs"]
                            .as_array()
                            .and_then(|jobs| jobs.iter().find(|v| v["key"] == key))
                        {
                            reference.last = direct_result(job.clone(), &reference.owner);
                            write_json(&path, &reference)?;
                        }
                        values.push(reference.last);
                    }
                    None => {
                        let mut value = reference.last;
                        value["status"] = json!("unknown");
                        value["error"] =
                            json!("Execution endpoint unavailable; job lifetime is unknown");
                        values.push(value);
                    }
                }
            }
        }
    }
    values.sort_by(|a, b| {
        b["updated"]
            .as_u64()
            .unwrap_or(0)
            .cmp(&a["updated"].as_u64().unwrap_or(0))
    });
    values.truncate(256);
    Ok(json!({"jobs":values}))
}
pub fn cancel(key: &str) -> Result<Value> {
    valid_key(key)?;
    let root = root()?;
    let direct = direct_path(&root, key);
    if direct.try_exists()? {
        let reference: DirectRef = read_json(&direct)?;
        return transport::request(
            &reference.owner,
            Operation::TransferCancel { key: key.into() },
        );
    }
    if !spec_path(&root, key).try_exists()? {
        bail!("unknown transfer");
    }
    let mut opts = OpenOptions::new();
    opts.write(true).create(true);
    #[cfg(unix)]
    opts.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    opts.open(root.join(format!("{key}.cancel")))?.sync_all()?;
    Ok(json!({"key":key,"status":"cancellation_requested"}))
}
fn text<'a>(v: &'a Value, key: &str) -> Result<&'a str> {
    v[key]
        .as_str()
        .with_context(|| format!("missing {key} in file metadata"))
}
fn join(parent: &str, name: &str) -> Result<String> {
    let name = decode_path(name)?;
    if name.components().count() != 1 || name.is_absolute() {
        bail!("invalid remote filename");
    }
    Ok(encode_path(&decode_path(parent)?.join(name)))
}
fn info(device: &crate::model::Device, path: &str) -> Result<Value> {
    transport::request(device, Operation::FileInfo { path: path.into() })
}
fn collect(
    spec: &TransferSpec,
    source: &str,
    destination: &str,
    entries: &mut Vec<Entry>,
    depth: usize,
) -> Result<()> {
    check_cancel(&root()?, &spec.key)?;
    if depth > 64 || entries.len() >= 100000 {
        bail!("copy tree exceeds bounded depth/item limit");
    }
    let metadata = info(&spec.source, source)?;
    let kind = text(&metadata, "kind")?.to_owned();
    if !matches!(kind.as_str(), "file" | "directory" | "symlink") {
        bail!("unsupported source type");
    }
    let identity = text(&metadata, "identity")?.to_owned();
    entries.push(Entry {
        source: source.into(),
        destination: destination.into(),
        kind: kind.clone(),
        identity: identity.clone(),
        size: metadata["size"].as_u64().unwrap_or(0),
        mode: metadata["mode"].as_u64().unwrap_or(0) as u32,
        target: metadata["target"].as_str().map(str::to_owned),
        status: "pending".into(),
        bytes: 0,
    });
    if kind == "directory" {
        let mut offset = 0;
        loop {
            check_cancel(&root()?, &spec.key)?;
            let listing = transport::request(
                &spec.source,
                Operation::ListPage {
                    path: source.into(),
                    offset,
                    limit: 1000,
                },
            )?;
            for child in listing["entries"]
                .as_array()
                .context("invalid directory listing")?
            {
                let childpath = text(child, "path")?;
                let childinfo = info(&spec.source, childpath)?;
                let name = text(&childinfo, "name")?;
                collect(
                    spec,
                    childpath,
                    &join(destination, name)?,
                    entries,
                    depth + 1,
                )?;
            }
            if let Some(next) = listing["next_offset"].as_u64() {
                if next <= offset {
                    bail!("pagination did not advance");
                }
                offset = next;
            } else {
                break;
            }
            if text(&info(&spec.source, source)?, "identity")? != identity {
                bail!("source directory changed during listing");
            }
        }
        if text(&info(&spec.source, source)?, "identity")? != identity {
            bail!("source directory changed during listing");
        }
    }
    Ok(())
}
fn file_key(key: &str, path: &str) -> String {
    format!(
        "{}-{:x}",
        &key[..key.len().min(40)],
        Sha256::digest(path.as_bytes())
    )[..73.min(key.len().min(40) + 65)]
        .to_owned()
}
fn check_cancel(root: &Path, key: &str) -> Result<()> {
    if root.join(format!("{key}.cancel")).try_exists()? {
        bail!("transfer cancelled; partial files retained for explicit resume");
    }
    Ok(())
}
pub fn worker(spec: &TransferSpec) -> Result<()> {
    let result = worker_inner(spec);
    if let Err(error) = &result {
        // Initialization errors are durable too; no phantom queued success.
        if let Ok(root) = root() {
            let record = job_path(&root, &spec.key);
            if let Ok(mut job) = read_json::<Job>(&record) {
                if job.entries.is_empty() {
                    job.status = "failed".into();
                    job.error = Some(display(&format!("{error:#}")));
                    job.updated = now();
                    let _ = write_json(&record, &job);
                }
            }
        }
    }
    result
}
fn worker_inner(spec: &TransferSpec) -> Result<()> {
    valid_key(&spec.key)?;
    let root = root()?;
    let _lock = Lock::acquire(&root, &spec.key)?;
    let record = job_path(&root, &spec.key);
    let mut job = if record.try_exists()? && !read_json::<Job>(&record)?.entries.is_empty() {
        read_json::<Job>(&record)?
    } else {
        let source = info(&spec.source, &spec.source_path)?;
        let source_path = text(&source, "path")?.to_owned();
        let mut destination = spec.destination_path.clone();
        let destination_info = info(&spec.destination, &destination)?;
        destination = text(&destination_info, "path")?.to_owned();
        if destination_info["kind"] == "directory" {
            destination = join(text(&destination_info, "path")?, text(&source, "name")?)?;
        }
        if spec.source.id == spec.destination.id
            || (spec.source.target.is_none() && spec.destination.target.is_none())
            || (spec.source.host == spec.destination.host
                && spec.source.account == spec.destination.account)
        {
            let src = decode_path(text(&source, "path")?)?;
            let dst = decode_path(&destination)?;
            if src == dst || (source["kind"] == "directory" && dst.starts_with(&src)) {
                bail!("destination cannot be the source or inside its tree");
            }
        }
        if source["kind"] == "directory"
            && info(&spec.destination, &destination)?["kind"] != "missing"
            && spec.conflict == "rename"
        {
            let path = decode_path(&destination)?;
            let name = path
                .file_name()
                .context("destination has no name")?
                .to_owned();
            let parent = path.parent().context("destination has no parent")?;
            let mut chosen = None;
            for n in 1..=10000 {
                let mut next = name.clone();
                next.push(format!(".copy-{n}"));
                let next = encode_path(&parent.join(next));
                if info(&spec.destination, &next)?["kind"] == "missing" {
                    chosen = Some(next);
                    break;
                }
            }
            destination = chosen.context("no rename destination available")?;
        }
        let mut entries = Vec::new();
        collect(spec, &source_path, &destination, &mut entries, 0)?;
        let total = entries
            .iter()
            .filter(|e| e.kind == "file")
            .map(|e| e.size)
            .try_fold(0u64, |a, b| a.checked_add(b))
            .context("copy size overflow")?;
        Job {
            key: spec.key.clone(),
            source_host: format!("{}@{}", spec.source.account, spec.source.host),
            destination_host: format!("{}@{}", spec.destination.account, spec.destination.host),
            source_path,
            destination_path: destination,
            route: if spec.source.target.is_some() && spec.destination.target.is_some() {
                "relay through worker host"
            } else {
                "worker host is endpoint"
            }
            .into(),
            status: "running".into(),
            bytes: 0,
            total,
            updated: now(),
            error: None,
            entries,
        }
    };
    if job.status == "complete" {
        return Ok(());
    }
    job.status = "running".into();
    job.error = None;
    write_json(&record, &job)?;
    let mut last_saved = std::time::Instant::now();
    let outcome = (|| -> Result<()> {
        for index in 0..job.entries.len() {
            check_cancel(&root, &spec.key)?;
            let entry = job.entries[index].clone();
            if text(&info(&spec.source, &entry.source)?, "identity")? != entry.identity {
                bail!("source changed since copy began");
            }
            if entry.status == "complete" || entry.status == "skipped" {
                continue;
            }
            match entry.kind.as_str() {
                "directory" => {
                    let current = info(&spec.destination, &entry.destination)?;
                    if current["kind"] == "missing" {
                        transport::request(
                            &spec.destination,
                            Operation::Mkdir {
                                path: entry.destination.clone(),
                            },
                        )?;
                    } else if current["kind"] != "directory" {
                        bail!("directory destination conflicts with a non-directory");
                    }
                    job.entries[index].status = "complete".into();
                }
                "symlink" => {
                    let result = transport::request(
                        &spec.destination,
                        Operation::ReceiveSymlink {
                            key: file_key(&spec.key, &entry.destination),
                            path: entry.destination.clone(),
                            target: entry.target.clone().context("symlink target unavailable")?,
                            conflict: spec.conflict.clone(),
                        },
                    )?;
                    job.entries[index].status = text(&result, "status")?.into();
                }
                "file" => {
                    let key = file_key(&spec.key, &entry.destination);
                    let state = transport::request(
                        &spec.destination,
                        Operation::ReceivePrepare {
                            path: entry.destination.clone(),
                            key: key.clone(),
                            source_identity: entry.identity.clone(),
                            total: entry.size,
                            conflict: spec.conflict.clone(),
                            mode: entry.mode,
                        },
                    )?;
                    if state["status"] == "skipped" {
                        job.entries[index].status = "skipped".into();
                        continue;
                    }
                    let mut offset = state["bytes"]
                        .as_u64()
                        .context("invalid receive checkpoint")?;
                    if offset > entry.size {
                        bail!("receive offset exceeds source size");
                    }
                    let mut hash = Sha256::new();
                    let mut read_offset = 0;
                    while read_offset < offset {
                        check_cancel(&root, &spec.key)?;
                        let chunk = transport::request(
                            &spec.source,
                            Operation::ReadChunk {
                                path: entry.source.clone(),
                                offset: read_offset,
                                limit: (offset - read_offset).min(CHUNK as u64) as u32,
                                identity: entry.identity.clone(),
                            },
                        )?;
                        let bytes = STANDARD.decode(text(&chunk, "data")?)?;
                        if bytes.is_empty() || bytes.len() > CHUNK as usize {
                            bail!("invalid source chunk");
                        }
                        hash.update(&bytes);
                        read_offset += bytes.len() as u64;
                    }
                    if format!("{:x}", hash.clone().finalize()) != text(&state, "prefix_sha256")? {
                        bail!("resume prefix integrity mismatch");
                    }
                    while offset < entry.size {
                        check_cancel(&root, &spec.key)?;
                        let chunk = transport::request(
                            &spec.source,
                            Operation::ReadChunk {
                                path: entry.source.clone(),
                                offset,
                                limit: (entry.size - offset).min(CHUNK as u64) as u32,
                                identity: entry.identity.clone(),
                            },
                        )?;
                        let data = text(&chunk, "data")?;
                        let bytes = STANDARD.decode(data)?;
                        if bytes.is_empty()
                            || bytes.len() > CHUNK as usize
                            || offset + bytes.len() as u64 > entry.size
                        {
                            bail!("invalid source chunk size");
                        }
                        let ack = transport::request(
                            &spec.destination,
                            Operation::ReceiveChunk {
                                key: key.clone(),
                                offset,
                                data: data.into(),
                            },
                        )?;
                        offset += bytes.len() as u64;
                        if ack["bytes"].as_u64() != Some(offset) {
                            bail!("invalid destination acknowledgement");
                        }
                        hash.update(&bytes);
                        job.entries[index].bytes = offset;
                        job.bytes = job.entries.iter().map(|e| e.bytes).sum();
                        job.updated = now();
                        if last_saved.elapsed().as_millis() >= 200 {
                            write_json(&record, &job)?;
                            last_saved = std::time::Instant::now();
                        }
                    }
                    if text(&info(&spec.source, &entry.source)?, "identity")? != entry.identity {
                        bail!("source changed before finalization");
                    }
                    transport::request(
                        &spec.destination,
                        Operation::ReceiveFinalize {
                            key,
                            sha256: format!("{:x}", hash.finalize()),
                        },
                    )?;
                    job.entries[index].status = "complete".into();
                    job.entries[index].bytes = entry.size;
                }
                _ => bail!("unsupported file kind"),
            }
            job.bytes = job.entries.iter().map(|e| e.bytes).sum();
            job.updated = now();
            write_json(&record, &job)?;
        }
        // Metadata-only final validation detects source changes without huge tree hashes.
        for entry in &job.entries {
            check_cancel(&root, &spec.key)?;
            if text(&info(&spec.source, &entry.source)?, "identity")? != entry.identity {
                bail!("source changed before tree completion");
            }
        }
        Ok(())
    })();
    job.updated = now();
    match outcome {
        Ok(()) => job.status = "complete".into(),
        Err(error) => {
            job.status = if root.join(format!("{}.cancel", spec.key)).exists() {
                "cancelled"
            } else {
                "failed"
            }
            .into();
            job.error = Some(display(&format!("{error:#}")));
            write_json(&record, &job)?;
            return Err(error);
        }
    }
    write_json(&record, &job)?;
    Ok(())
}
