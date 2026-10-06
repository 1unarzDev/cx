//! Filesystem operations shared by the local viewer and SSH helper.
//! Copy is synchronous and durable: it does not claim to outlive its helper.
use crate::model::Operation;
use anyhow::{bail, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
#[cfg(unix)]
use std::os::unix::{
    ffi::{OsStrExt, OsStringExt},
    fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    io::AsRawFd,
};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
const PATH_PREFIX: &str = "cx-bytes:";
const PREVIEW_LIMIT: usize = 32 * 1024;
const LIST_LIMIT: usize = 1000;

/// Escape all terminal controls, including C1 controls. Metadata must not render escapes.
pub fn display(text: &str) -> String {
    text.chars()
        .flat_map(|c| {
            if c.is_control() {
                c.escape_default().collect::<Vec<_>>()
            } else {
                vec![c]
            }
        })
        .collect()
}
fn encode_path(path: &Path) -> String {
    #[cfg(unix)]
    {
        let bytes = path.as_os_str().as_bytes();
        match path.to_str() {
            Some(s) if !s.starts_with(PATH_PREFIX) => s.to_owned(),
            _ => format!("{PATH_PREFIX}{}", STANDARD.encode(bytes)),
        }
    }
    #[cfg(not(unix))]
    {
        path.to_string_lossy().to_string()
    }
}
fn decode_path(path: &str) -> Result<PathBuf> {
    if path.len() > 16384 {
        bail!("path is too long");
    }
    if let Some(encoded) = path.strip_prefix(PATH_PREFIX) {
        let bytes = STANDARD.decode(encoded).context("invalid encoded path")?;
        if bytes.contains(&0) {
            bail!("path contains NUL");
        }
        #[cfg(unix)]
        {
            return Ok(PathBuf::from(std::ffi::OsString::from_vec(bytes)));
        }
        #[cfg(not(unix))]
        {
            return Ok(PathBuf::from(String::from_utf8(bytes)?));
        }
    }
    if path.contains('\0') {
        bail!("path contains NUL");
    }
    Ok(PathBuf::from(path))
}
fn absolute(path: &Path) -> Result<PathBuf> {
    Ok(if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    })
}
pub fn handle(op: &Operation) -> Result<Value> {
    match op {
        Operation::List { path } => list(&decode_path(path)?),
        Operation::Preview { path } => preview(&decode_path(path)?),
        Operation::Mkdir { path } => {
            let path = absolute(&decode_path(path)?)?;
            fs::create_dir(&path).context("create directory")?;
            Ok(json!({"path":encode_path(&path)}))
        }
        Operation::Copy {
            source,
            destination,
            conflict,
            key,
        } => copy_file(
            &job_root()?,
            &decode_path(source)?,
            &decode_path(destination)?,
            conflict,
            key,
        ),
        Operation::Jobs => jobs(&job_root()?),
        Operation::Cancel { key } => cancel(&job_root()?, key),
        _ => bail!("unsupported filesystem operation"),
    }
}
fn list(path: &Path) -> Result<Value> {
    let path = fs::canonicalize(path).context("open directory")?;
    let mut entries = Vec::new();
    let mut truncated = false;
    let mut budget = 0;
    for entry in fs::read_dir(&path)? {
        let entry = entry?;
        let metadata = fs::symlink_metadata(entry.path())?;
        let name = display(&entry.file_name().to_string_lossy());
        let opaque = encode_path(&entry.path());
        budget += name.len() + opaque.len() + 128;
        if entries.len() == LIST_LIMIT || budget > 512 * 1024 {
            truncated = true;
            break;
        }
        let kind = if metadata.is_symlink() {
            "symlink"
        } else if metadata.is_dir() {
            "directory"
        } else if metadata.is_file() {
            "file"
        } else {
            "other"
        };
        entries.push(json!({"name":name,"path":opaque,"kind":kind,"size":metadata.len()}));
    }
    entries.sort_by(|a, b| {
        let group = |v: &Value| if v["kind"] == "directory" { 0 } else { 1 };
        group(a)
            .cmp(&group(b))
            .then_with(|| a["name"].as_str().cmp(&b["name"].as_str()))
    });
    Ok(
        json!({"path":encode_path(&path),"display_path":display(&path.to_string_lossy()),"parent":path.parent().map(encode_path),"entries":entries,"truncated":truncated}),
    )
}
fn open_read(path: &Path) -> Result<File> {
    let mut opts = OpenOptions::new();
    opts.read(true);
    #[cfg(unix)]
    opts.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    let f = opts
        .open(path)
        .context("open regular file (symlinks are not followed)")?;
    if !f.metadata()?.is_file() {
        bail!("only regular files are supported");
    }
    Ok(f)
}
fn preview(path: &Path) -> Result<Value> {
    let file = open_read(path)?;
    let mut bytes = Vec::new();
    file.take((PREVIEW_LIMIT + 1) as u64)
        .read_to_end(&mut bytes)?;
    let truncated = bytes.len() > PREVIEW_LIMIT;
    bytes.truncate(PREVIEW_LIMIT);
    let binary = bytes.contains(&0);
    // Preserve ordinary line breaks and tabs, escape all other terminal controls.
    let text: String = if binary {
        "Binary file — preview unavailable".into()
    } else {
        String::from_utf8_lossy(&bytes)
            .chars()
            .flat_map(|c| {
                if c == '\n' || c == '\t' {
                    vec![c]
                } else if c.is_control() {
                    c.escape_default().collect()
                } else {
                    vec![c]
                }
            })
            .collect()
    };
    Ok(json!({"path":encode_path(path),"text":text,"truncated":truncated,"binary":binary}))
}
fn job_root() -> Result<PathBuf> {
    let base = match std::env::var_os("XDG_STATE_HOME") {
        Some(v) if Path::new(&v).is_absolute() => PathBuf::from(v),
        _ => PathBuf::from(std::env::var_os("HOME").context("HOME unavailable")?)
            .join(".local/state"),
    };
    let path = base.join("cx/jobs");
    fs::create_dir_all(&path)?;
    let metadata = fs::symlink_metadata(&path)?;
    if !metadata.is_dir() || metadata.is_symlink() {
        bail!("job directory is not a private directory");
    }
    #[cfg(unix)]
    {
        if metadata.uid() != unsafe { libc::geteuid() } {
            bail!("job directory has another owner");
        }
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(path)
}
fn validate_key(key: &str) -> Result<()> {
    if key.is_empty()
        || key.len() > 80
        || !key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        bail!("job key must contain 1–80 ASCII letters, digits, hyphens or underscores");
    }
    Ok(())
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct Identity {
    size: u64,
    modified: u128,
    device: u64,
    inode: u64,
}
fn identity(metadata: &fs::Metadata) -> Result<Identity> {
    #[cfg(unix)]
    let (device, inode) = (metadata.dev(), metadata.ino());
    #[cfg(not(unix))]
    let (device, inode) = (0, 0);
    Ok(Identity {
        size: metadata.len(),
        modified: metadata.modified()?.duration_since(UNIX_EPOCH)?.as_nanos(),
        device,
        inode,
    })
}
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Job {
    key: String,
    source: String,
    destination: String,
    conflict: String,
    status: String,
    #[serde(default)]
    requested_destination: String,
    #[serde(default)]
    partial_created: bool,
    bytes: u64,
    total: u64,
    identity: Identity,
    sha256: Option<String>,
    error: Option<String>,
    updated: u64,
}
fn save_job(root: &Path, job: &Job) -> Result<()> {
    let tmp = root.join(format!("{}.{}.tmp", job.key, std::process::id()));
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    let mut f = options.open(&tmp)?;
    f.write_all(&serde_json::to_vec(job)?)?;
    f.sync_all()?;
    fs::rename(tmp, root.join(format!("{}.json", job.key)))?;
    Ok(())
}
fn load_job(root: &Path, key: &str) -> Result<Option<Job>> {
    let path = root.join(format!("{key}.json"));
    match open_read(&path) {
        Ok(f) => Ok(Some(serde_json::from_reader(f.take(32768))?)),
        Err(e)
            if e.downcast_ref::<std::io::Error>()
                .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound) =>
        {
            Ok(None)
        }
        Err(e) => Err(e),
    }
}
fn jobs(root: &Path) -> Result<Value> {
    let mut values = Vec::new();
    for entry in fs::read_dir(root)?.take(4096) {
        let path = entry?.path();
        if path.extension().is_some_and(|v| v == "json") {
            if let Ok(f) = open_read(&path) {
                if let Ok(mut job) = serde_json::from_reader::<_, Job>(f.take(32768)) {
                    // A stale running record never claims that its helper is alive.
                    if job.status == "running" || job.status == "finalizing" {
                        if Lock::acquire(root, &job.key).is_ok() {
                            job.status = "incomplete".into();
                        }
                    }
                    values.push(job);
                }
            }
        }
    }
    values.sort_by(|a, b| b.updated.cmp(&a.updated).then(a.key.cmp(&b.key)));
    values.truncate(256);
    Ok(json!({"jobs":values}))
}
fn cancel(root: &Path, key: &str) -> Result<Value> {
    validate_key(key)?;
    if load_job(root, key)?.is_none() {
        bail!("job does not exist");
    }
    let mut options = OpenOptions::new();
    options.write(true).create(true);
    #[cfg(unix)]
    options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    options
        .open(root.join(format!("{key}.cancel")))?
        .sync_all()?;
    Ok(json!({"key":key,"status":"cancellation_requested"}))
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
            bail!("job is already running");
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
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn hash_prefix(file: &mut File, length: u64) -> Result<Sha256> {
    file.seek(SeekFrom::Start(0))?;
    let mut remaining = length;
    let mut hash = Sha256::new();
    let mut buf = [0; 64 * 1024];
    while remaining > 0 {
        let count = file.read(&mut buf[..remaining.min(64 * 1024) as usize])?;
        if count == 0 {
            bail!("partial file is shorter than recorded progress");
        }
        hash.update(&buf[..count]);
        remaining -= count as u64;
    }
    Ok(hash)
}
fn copy_file(
    root: &Path,
    source: &Path,
    destination: &Path,
    conflict: &str,
    key: &str,
) -> Result<Value> {
    validate_key(key)?;
    if !matches!(conflict, "skip" | "overwrite" | "rename") {
        bail!("conflict must be skip, overwrite or rename");
    }
    let _lock = Lock::acquire(root, key)?;
    let source = absolute(source)?;
    let mut requested = absolute(destination)?;
    if requested.is_dir() {
        requested = requested.join(source.file_name().context("source has no filename")?);
    }
    if let Some(mut previous) = load_job(root, key)? {
        if previous.source != encode_path(&source)
            || previous.requested_destination != encode_path(&requested)
            || previous.conflict != conflict
        {
            bail!("idempotency key belongs to another copy");
        }
        if previous.status == "complete" || previous.status == "skipped" {
            return Ok(serde_json::to_value(previous)?);
        }
        let partial = decode_path(&previous.destination)?
            .parent()
            .context("destination has no parent")?
            .join(format!(".cx-{key}.partial"));
        if previous.sha256.is_some() && !partial.try_exists()? {
            let mut final_file = open_read(&decode_path(&previous.destination)?)?;
            if final_file.metadata()?.len() != previous.total {
                bail!("finalized destination changed before reconciliation");
            }
            let hash = format!(
                "{:x}",
                hash_prefix(&mut final_file, previous.total)?.finalize()
            );
            if previous.sha256.as_deref() != Some(hash.as_str()) {
                bail!("finalized destination integrity mismatch");
            }
            previous.status = "complete".into();
            previous.error = None;
            previous.updated = now();
            save_job(root, &previous)?;
            return Ok(serde_json::to_value(previous)?);
        }
    }
    let mut input = open_read(&source)?;
    let initial = identity(&input.metadata()?)?;
    let mut job = if let Some(previous) = load_job(root, key)? {
        if previous.source != encode_path(&source) || previous.conflict != conflict {
            bail!("idempotency key belongs to another copy");
        }
        // destination can differ only for our rename policy, which records the chosen suffix.
        if (if previous.requested_destination.is_empty() {
            &previous.destination
        } else {
            &previous.requested_destination
        }) != &encode_path(&requested)
        {
            bail!("idempotency key belongs to another destination");
        }
        if previous.status == "complete" || previous.status == "skipped" {
            return Ok(serde_json::to_value(previous)?);
        }
        if previous.identity != initial {
            bail!("source changed; start a new job instead of resuming");
        }
        previous
    } else {
        let mut destination = requested.clone();
        if destination.try_exists()? {
            if conflict == "skip" {
                let job = Job {
                    key: key.into(),
                    source: encode_path(&source),
                    destination: encode_path(&destination),
                    conflict: conflict.into(),
                    requested_destination: encode_path(&requested),
                    partial_created: false,
                    status: "skipped".into(),
                    bytes: 0,
                    total: initial.size,
                    identity: initial,
                    sha256: None,
                    error: None,
                    updated: now(),
                };
                save_job(root, &job)?;
                return Ok(serde_json::to_value(job)?);
            }
            if conflict == "rename" {
                let name = destination
                    .file_name()
                    .context("destination has no filename")?
                    .to_os_string();
                let parent = destination.parent().context("destination has no parent")?;
                let mut found = None;
                for n in 1..=10000 {
                    let mut candidate = name.clone();
                    candidate.push(format!(".copy-{n}"));
                    let p = parent.join(candidate);
                    if !p.try_exists()? {
                        found = Some(p);
                        break;
                    }
                }
                destination = found.context("no available rename destination")?;
            }
        }
        Job {
            key: key.into(),
            source: encode_path(&source),
            destination: encode_path(&destination),
            conflict: conflict.into(),
            requested_destination: encode_path(&requested),
            partial_created: false,
            status: "pending".into(),
            bytes: 0,
            total: initial.size,
            identity: initial,
            sha256: None,
            error: None,
            updated: now(),
        }
    };
    let destination = decode_path(&job.destination)?;
    if let Ok(m) = fs::symlink_metadata(&destination) {
        if !m.is_file() {
            bail!("destination must be a regular file; symlinks are not overwritten");
        }
        if identity(&m)? == job.identity {
            bail!("source and destination are the same file");
        }
    }
    let parent = destination.parent().context("destination has no parent")?;
    if !parent.is_dir() {
        bail!("destination directory does not exist");
    }
    let partial = parent.join(format!(".cx-{key}.partial"));
    let cancellation = root.join(format!("{key}.cancel"));
    if cancellation.try_exists()? {
        fs::remove_file(&cancellation)?;
    }
    job.status = "running".into();
    job.error = None;
    job.updated = now();
    save_job(root, &job)?;
    let result = (|| -> Result<()> {
        let mut options = OpenOptions::new();
        options.read(true).write(true);
        #[cfg(unix)]
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
        // Do not adopt a preexisting partial for a job which has never recorded progress.
        if !job.partial_created {
            options.create_new(true);
        } else {
            options.create(false);
        }
        let mut output = options.open(&partial).context("open owned partial file")?;
        job.partial_created = true;
        save_job(root, &job)?;
        if !output.metadata()?.is_file() {
            bail!("partial is not a regular file");
        }
        if job.bytes > job.total || output.metadata()?.len() < job.bytes {
            bail!("partial file has invalid size");
        }
        output.set_len(job.bytes)?;
        let mut hash = hash_prefix(&mut input, job.bytes)?;
        let partial_hash = hash_prefix(&mut output, job.bytes)?;
        if hash.clone().finalize() != partial_hash.finalize() {
            bail!("partial integrity mismatch; refusing resume");
        }
        output.seek(SeekFrom::Start(job.bytes))?;
        let mut buf = [0; 64 * 1024];
        let mut last_saved = std::time::Instant::now();
        loop {
            if cancellation.try_exists()? {
                bail!("copy cancelled; partial retained for explicit resume");
            }
            let count = input.read(&mut buf)?;
            if count == 0 {
                break;
            }
            output.write_all(&buf[..count])?;
            hash.update(&buf[..count]);
            job.bytes += count as u64;
            if job.bytes > job.total {
                bail!("source grew while copying");
            }
            if last_saved.elapsed().as_millis() >= 200 {
                output.sync_data()?;
                job.updated = now();
                save_job(root, &job)?;
                last_saved = std::time::Instant::now();
            }
        }
        if identity(&input.metadata()?)? != job.identity
            || identity(&fs::metadata(&source)?)? != job.identity
        {
            bail!("source changed while copying");
        }
        if job.bytes != job.total {
            bail!("source size changed while copying");
        }
        #[cfg(unix)]
        fs::set_permissions(
            &partial,
            fs::Permissions::from_mode(input.metadata()?.permissions().mode() & 0o777),
        )?;
        output.sync_all()?;
        job.sha256 = Some(format!("{:x}", hash.finalize()));
        job.status = "finalizing".into();
        save_job(root, &job)?;
        // hard_link provides an atomic no-clobber finalize for skip/rename.
        if conflict == "overwrite" {
            fs::rename(&partial, &destination)?;
        } else {
            fs::hard_link(&partial, &destination)
                .context("destination appeared before finalization")?;
            fs::remove_file(&partial)?;
        }
        if let Ok(directory) = File::open(parent) {
            directory.sync_all()?;
        }
        Ok(())
    })();
    match result {
        Ok(()) => {
            job.status = "complete".into();
        }
        Err(error) => {
            job.status = if cancellation.exists() {
                "cancelled"
            } else {
                "failed"
            }
            .into();
            job.error = Some(display(&format!("{error:#}")));
            job.updated = now();
            save_job(root, &job)?;
            return Err(error);
        }
    }
    job.updated = now();
    save_job(root, &job)?;
    Ok(serde_json::to_value(job)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    #[test]
    fn hostile_preview_and_listing() {
        let d = tempdir().unwrap();
        let p = d.path().join("- hostile\u{1b}[2J.txt");
        fs::write(&p, b"hello\x1b[2J\nworld\x07").unwrap();
        let value = preview(&p).unwrap();
        assert!(!value["text"].as_str().unwrap().contains('\u{1b}'));
        assert!(value["text"].as_str().unwrap().contains("\\u{1b}"));
        let value = list(d.path()).unwrap();
        assert!(!value["entries"][0]["name"]
            .as_str()
            .unwrap()
            .contains('\u{1b}'));
        assert_eq!(
            decode_path(value["entries"][0]["path"].as_str().unwrap()).unwrap(),
            p
        );
    }
    #[test]
    fn binary_and_bounded_preview() {
        let d = tempdir().unwrap();
        let p = d.path().join("data");
        fs::write(&p, vec![b'x'; 100000]).unwrap();
        assert_eq!(preview(&p).unwrap()["truncated"], true);
        fs::write(&p, b"\0").unwrap();
        assert_eq!(preview(&p).unwrap()["binary"], true);
    }
    #[test]
    fn copy_conflicts_and_idempotency() {
        let d = tempdir().unwrap();
        let state = d.path().join("jobs");
        fs::create_dir(&state).unwrap();
        let s = d.path().join("source");
        let t = d.path().join("target");
        fs::write(&s, b"abcdef").unwrap();
        let job = copy_file(&state, &s, &t, "skip", "first").unwrap();
        assert_eq!(job["status"], "complete");
        assert_eq!(job["bytes"], 6);
        assert_eq!(fs::read(&t).unwrap(), b"abcdef");
        assert_eq!(
            copy_file(&state, &s, &t, "skip", "first").unwrap()["status"],
            "complete"
        );
        fs::write(&s, b"second").unwrap();
        assert_eq!(
            copy_file(&state, &s, &t, "skip", "second").unwrap()["status"],
            "skipped"
        );
        assert_eq!(fs::read(&t).unwrap(), b"abcdef");
        copy_file(&state, &s, &t, "rename", "third").unwrap();
        assert_eq!(fs::read(d.path().join("target.copy-1")).unwrap(), b"second");
        copy_file(&state, &s, &t, "overwrite", "fourth").unwrap();
        assert_eq!(fs::read(t).unwrap(), b"second");
    }
    #[test]
    fn same_file_and_symlink_rejected() {
        let d = tempdir().unwrap();
        let s = d.path().join("source");
        fs::write(&s, b"content").unwrap();
        assert!(copy_file(d.path(), &s, &s, "overwrite", "same").is_err());
        #[cfg(unix)]
        {
            let link = d.path().join("link");
            std::os::unix::fs::symlink(&s, &link).unwrap();
            assert!(preview(&link).is_err());
            assert!(copy_file(d.path(), &link, &d.path().join("dest"), "skip", "link").is_err());
        }
    }
    #[test]
    fn resume_validates_partial_and_source() {
        let d = tempdir().unwrap();
        let s = d.path().join("source");
        let t = d.path().join("target");
        fs::write(&s, b"abcdef").unwrap();
        let job = Job {
            key: "resume".into(),
            source: encode_path(&s),
            destination: encode_path(&t),
            conflict: "skip".into(),
            requested_destination: encode_path(&t),
            partial_created: true,
            status: "failed".into(),
            bytes: 3,
            total: 6,
            identity: identity(&fs::metadata(&s).unwrap()).unwrap(),
            sha256: None,
            error: None,
            updated: now(),
        };
        save_job(d.path(), &job).unwrap();
        fs::write(d.path().join(".cx-resume.partial"), b"abcZZZ").unwrap();
        copy_file(d.path(), &s, &t, "skip", "resume").unwrap();
        assert_eq!(fs::read(t).unwrap(), b"abcdef");
        let mut job = job;
        job.key = "corrupt".into();
        save_job(d.path(), &job).unwrap();
        fs::write(d.path().join(".cx-corrupt.partial"), b"bad").unwrap();
        assert!(
            copy_file(d.path(), &s, &d.path().join("target"), "skip", "corrupt")
                .unwrap_err()
                .to_string()
                .contains("integrity")
        );
        fs::write(&s, b"changed").unwrap();
        assert!(
            copy_file(d.path(), &s, &d.path().join("target"), "skip", "corrupt")
                .unwrap_err()
                .to_string()
                .contains("source changed")
        );
    }
    #[test]
    fn keys_and_non_utf8_roundtrip() {
        assert!(validate_key("../oops").is_err());
        assert!(validate_key("safe-1_2").is_ok());
        #[cfg(unix)]
        {
            let p = PathBuf::from(std::ffi::OsString::from_vec(b"odd-\xff".to_vec()));
            assert_eq!(decode_path(&encode_path(&p)).unwrap(), p);
        }
    }
    #[test]
    fn cancellation_resume_and_concurrent_lock() {
        let d = tempdir().unwrap();
        let source = d.path().join("source");
        let target = d.path().join("target");
        File::create(&source)
            .unwrap()
            .set_len(32 * 1024 * 1024)
            .unwrap();
        let root = d.path().to_path_buf();
        let worker_root = root.clone();
        let worker_source = source.clone();
        let worker_target = target.clone();
        let worker = std::thread::spawn(move || {
            copy_file(
                &worker_root,
                &worker_source,
                &worker_target,
                "skip",
                "cancel",
            )
        });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !root.join("cancel.json").exists() {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert!(Lock::acquire(&root, "cancel").is_err());
        cancel(&root, "cancel").unwrap();
        assert!(worker.join().unwrap().is_err());
        assert_eq!(
            load_job(&root, "cancel").unwrap().unwrap().status,
            "cancelled"
        );
        assert!(!target.exists());
        assert_eq!(
            copy_file(&root, &source, &target, "skip", "cancel").unwrap()["status"],
            "complete"
        );
        assert_eq!(fs::metadata(target).unwrap().len(), 32 * 1024 * 1024);
    }
    #[test]
    fn finalize_reconciliation_and_wrong_rename_destination() {
        let d = tempdir().unwrap();
        let source = d.path().join("source");
        let target = d.path().join("target");
        fs::write(&source, b"content").unwrap();
        copy_file(d.path(), &source, &target, "rename", "reconcile").unwrap();
        let mut job = load_job(d.path(), "reconcile").unwrap().unwrap();
        job.status = "finalizing".into();
        save_job(d.path(), &job).unwrap();
        fs::remove_file(&source).unwrap();
        assert_eq!(
            copy_file(d.path(), &source, &target, "rename", "reconcile").unwrap()["status"],
            "complete"
        );
        assert!(copy_file(
            d.path(),
            &source,
            &d.path().join("other"),
            "rename",
            "reconcile"
        )
        .is_err());
    }
    #[test]
    fn directory_destination_and_opaque_parent() {
        let d = tempdir().unwrap();
        let source = d.path().join("source");
        fs::write(&source, b"content").unwrap();
        let out = d.path().join("out");
        fs::create_dir(&out).unwrap();
        copy_file(d.path(), &source, &out, "skip", "into-dir").unwrap();
        assert_eq!(fs::read(out.join("source")).unwrap(), b"content");
        assert_eq!(
            decode_path(list(&out).unwrap()["parent"].as_str().unwrap()).unwrap(),
            d.path()
        );
    }
    #[test]
    fn directory_copy_is_honestly_unsupported() {
        let d = tempdir().unwrap();
        assert!(copy_file(
            d.path(),
            d.path(),
            &d.path().join("copy"),
            "skip",
            "directory"
        )
        .is_err());
    }
}
