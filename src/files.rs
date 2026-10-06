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
pub(crate) fn encode_path(path: &Path) -> String {
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
pub(crate) fn decode_path(path: &str) -> Result<PathBuf> {
    if path == "~" {
        return Ok(PathBuf::from(
            std::env::var_os("HOME").context("HOME unavailable")?,
        ));
    }
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
        Operation::ListPage {
            path,
            offset,
            limit,
        } => list_page(&decode_path(path)?, *offset, *limit),
        Operation::Preview { path } => preview(&decode_path(path)?),
        Operation::Mkdir { path } => {
            let path = absolute(&decode_path(path)?)?;
            let anchor = Anchor::parent(&path)?;
            let name = Anchor::cstr(&anchor.name)?;
            if unsafe { libc::mkdirat(anchor.dir.as_raw_fd(), name.as_ptr(), 0o777) } != 0 {
                return Err(std::io::Error::last_os_error()).context("create anchored directory");
            }
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
        Operation::SetPermissions {
            path,
            mode,
            expected_identity,
        } => set_permissions(&decode_path(path)?, *mode, expected_identity.as_deref()),
        Operation::FileInfo { path } => file_info(&decode_path(path)?),
        Operation::ReadChunk {
            path,
            offset,
            limit,
            identity,
        } => read_chunk(&decode_path(path)?, *offset, *limit, identity),
        Operation::ReceivePrepare {
            path,
            key,
            source_identity,
            total,
            conflict,
            mode,
        } => receive_prepare(
            &decode_path(path)?,
            key,
            source_identity,
            *total,
            conflict,
            *mode,
        ),
        Operation::ReceiveChunk { key, offset, data } => receive_chunk(key, *offset, data),
        Operation::ReceiveFinalize { key, sha256 } => receive_finalize(key, sha256),
        Operation::ReceiveSymlink {
            path,
            target,
            conflict,
            key,
        } => receive_symlink(&decode_path(path)?, &decode_path(target)?, conflict, key),
        _ => bail!("unsupported filesystem operation"),
    }
}
fn list(path: &Path) -> Result<Value> {
    list_page(path, 0, LIST_LIMIT as u32)
}
fn list_page(path: &Path, offset: u64, limit: u32) -> Result<Value> {
    if limit == 0 || limit > LIST_LIMIT as u32 || offset > 100000 {
        bail!("listing pagination exceeds bounds");
    }
    let path = fs::canonicalize(path).context("open directory")?;
    let mut entries = Vec::new();
    let mut truncated = false;
    let mut budget = 0;
    for entry in fs::read_dir(&path)?.skip(offset as usize) {
        let entry = entry?;
        let metadata = fs::symlink_metadata(entry.path())?;
        let name = display(&entry.file_name().to_string_lossy());
        let opaque = encode_path(&entry.path());
        budget += name.len() + opaque.len() + 128;
        if entries.len() == limit as usize || budget > 512 * 1024 {
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
        json!({"path":encode_path(&path),"display_path":display(&path.to_string_lossy()),"parent":path.parent().map(encode_path),"next_offset":if truncated{Some(offset+entries.len() as u64)}else{None},"entries":entries,"truncated":truncated}),
    )
}
fn open_read(path: &Path) -> Result<File> {
    // Reject special files before opening them: opening a device can itself have
    // hardware side effects, even if we never read any bytes.
    if !fs::symlink_metadata(path)?.is_file() {
        bail!("only regular files are supported; devices and symlinks are not opened");
    }
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

// Keep every destination mutation anchored to a directory descriptor. Traversing
// ancestors with O_NOFOLLOW rejects redirection through a replaced symlink.
#[cfg(unix)]
struct Anchor {
    dir: File,
    name: std::ffi::OsString,
}
#[cfg(unix)]
impl Anchor {
    fn cstr(value: &std::ffi::OsStr) -> Result<std::ffi::CString> {
        Ok(std::ffi::CString::new(value.as_bytes())?)
    }
    fn parent(path: &Path) -> Result<Self> {
        use std::os::unix::io::FromRawFd;
        let path = absolute(path)?;
        let parent = path.parent().context("destination has no parent")?;
        let mut dir = File::open("/")?;
        for part in parent.components() {
            let name = match part {
                std::path::Component::RootDir | std::path::Component::CurDir => continue,
                std::path::Component::Normal(name) => name,
                std::path::Component::ParentDir => std::ffi::OsStr::new(".."),
                _ => bail!("unsupported path prefix"),
            };
            let name = Self::cstr(name)?;
            let fd = unsafe {
                libc::openat(
                    dir.as_raw_fd(),
                    name.as_ptr(),
                    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                )
            };
            if fd < 0 {
                return Err(std::io::Error::last_os_error())
                    .context("destination ancestor is unavailable or a symlink");
            }
            dir = unsafe { File::from_raw_fd(fd) };
        }
        Ok(Self {
            dir,
            name: path
                .file_name()
                .context("destination has no filename")?
                .to_owned(),
        })
    }
    fn parent_identity(&self) -> Result<(u64, u64)> {
        let m = self.dir.metadata()?;
        Ok((m.dev(), m.ino()))
    }
    fn open(&self, name: &std::ffi::OsStr, flags: i32) -> Result<File> {
        use std::os::unix::io::FromRawFd;
        let name = Self::cstr(name)?;
        let fd = unsafe {
            libc::openat(
                self.dir.as_raw_fd(),
                name.as_ptr(),
                flags | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
                0o600,
            )
        };
        if fd < 0 {
            return Err(std::io::Error::last_os_error()).context("open anchored regular file");
        }
        let file = unsafe { File::from_raw_fd(fd) };
        if !file.metadata()?.is_file() {
            bail!("anchored entry is not a regular file");
        }
        Ok(file)
    }
    fn mode(&self, name: &std::ffi::OsStr) -> Result<Option<u32>> {
        let name = Self::cstr(name)?;
        let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
        if unsafe {
            libc::fstatat(
                self.dir.as_raw_fd(),
                name.as_ptr(),
                stat.as_mut_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            )
        } != 0
        {
            let e = std::io::Error::last_os_error();
            if e.kind() == std::io::ErrorKind::NotFound {
                return Ok(None);
            }
            return Err(e.into());
        }
        Ok(Some(unsafe { stat.assume_init() }.st_mode as u32))
    }
    fn rename(&self, from: &std::ffi::OsStr, to: &std::ffi::OsStr) -> Result<()> {
        let from = Self::cstr(from)?;
        let to = Self::cstr(to)?;
        if unsafe {
            libc::renameat(
                self.dir.as_raw_fd(),
                from.as_ptr(),
                self.dir.as_raw_fd(),
                to.as_ptr(),
            )
        } != 0
        {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok(())
    }
    fn link(&self, from: &std::ffi::OsStr, to: &std::ffi::OsStr) -> Result<()> {
        let from = Self::cstr(from)?;
        let to = Self::cstr(to)?;
        if unsafe {
            libc::linkat(
                self.dir.as_raw_fd(),
                from.as_ptr(),
                self.dir.as_raw_fd(),
                to.as_ptr(),
                0,
            )
        } != 0
        {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok(())
    }
    fn unlink(&self, name: &std::ffi::OsStr) -> Result<()> {
        let name = Self::cstr(name)?;
        if unsafe { libc::unlinkat(self.dir.as_raw_fd(), name.as_ptr(), 0) } != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok(())
    }
    fn symlink(&self, name: &std::ffi::OsStr, target: &Path) -> Result<()> {
        let name = Self::cstr(name)?;
        let target = Self::cstr(target.as_os_str())?;
        if unsafe { libc::symlinkat(target.as_ptr(), self.dir.as_raw_fd(), name.as_ptr()) } != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok(())
    }
    fn readlink(&self, name: &std::ffi::OsStr) -> Result<PathBuf> {
        let name = Self::cstr(name)?;
        let mut bytes = vec![0u8; 16384];
        let count = unsafe {
            libc::readlinkat(
                self.dir.as_raw_fd(),
                name.as_ptr(),
                bytes.as_mut_ptr().cast(),
                bytes.len(),
            )
        };
        if count < 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        if count as usize == bytes.len() {
            bail!("symlink target exceeds limit");
        }
        bytes.truncate(count as usize);
        Ok(PathBuf::from(std::ffi::OsString::from_vec(bytes)))
    }
}
fn set_permissions(path: &Path, mode: u32, expected: Option<&str>) -> Result<Value> {
    use std::os::unix::io::FromRawFd;
    if mode > 0o777 {
        bail!("only ordinary rwx permissions are supported");
    }
    let anchor = Anchor::parent(path)?;
    let name = Anchor::cstr(&anchor.name)?;
    let fd = unsafe {
        libc::openat(
            anchor.dir.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    let file = unsafe { File::from_raw_fd(fd) };
    let metadata = file.metadata()?;
    if !metadata.is_dir() && !metadata.is_file() {
        bail!("permissions apply only to regular files and directories");
    }
    if let Some(expected) = expected {
        if expected.len() > 2048 {
            bail!("expected identity too large");
        }
        let expected: Identity = serde_json::from_str(expected)?;
        let current = identity(&metadata)?;
        if current.device != expected.device || current.inode != expected.inode {
            bail!("destination entry changed; refusing chmod of another object");
        }
    }
    file.set_permissions(fs::Permissions::from_mode(mode))?;
    Ok(json!({"path":encode_path(path),"mode":mode}))
}
const CHUNK_LIMIT: usize = 128 * 1024;
fn file_info(path: &Path) -> Result<Value> {
    let path = absolute(path)?;
    let metadata = match fs::symlink_metadata(&path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let path = if let Some(parent) = path.parent() {
                match fs::canonicalize(parent) {
                    Ok(parent) => parent.join(
                        path.file_name()
                            .context("missing destination has no filename")?,
                    ),
                    Err(_) => path,
                }
            } else {
                path
            };
            return Ok(json!({"path":encode_path(&path),"kind":"missing"}));
        }
        Err(e) => return Err(e.into()),
    };
    let path = if metadata.is_symlink() {
        path.parent()
            .and_then(|p| fs::canonicalize(p).ok())
            .map(|parent| parent.join(path.file_name().unwrap()))
            .unwrap_or(path)
    } else {
        fs::canonicalize(path)?
    };
    let kind = if metadata.is_symlink() {
        "symlink"
    } else if metadata.is_file() {
        "file"
    } else if metadata.is_dir() {
        "directory"
    } else {
        "other"
    };
    #[cfg(unix)]
    let mode = metadata.permissions().mode() & 0o777;
    #[cfg(not(unix))]
    let mode = 0;
    let token = serde_json::to_string(&identity(&metadata)?)?;
    Ok(
        json!({"path":encode_path(&path),"name":path.file_name().map(|p|encode_path(Path::new(p))),"kind":kind,"size":metadata.len(),"identity":token,"mode":mode,"target":if metadata.is_symlink(){Some(encode_path(&fs::read_link(&path)?))}else{None}}),
    )
}
fn read_chunk(path: &Path, offset: u64, limit: u32, expected: &str) -> Result<Value> {
    if limit == 0 || limit as usize > CHUNK_LIMIT {
        bail!("chunk limit must be 1–131072 bytes");
    }
    let mut file = open_read(path)?;
    let before = serde_json::to_string(&identity(&file.metadata()?)?)?;
    if before != expected {
        bail!("source changed before read");
    }
    if offset > file.metadata()?.len() {
        bail!("offset exceeds source size");
    }
    file.seek(SeekFrom::Start(offset))?;
    let mut bytes = Vec::new();
    (&mut file).take(limit as u64).read_to_end(&mut bytes)?;
    if serde_json::to_string(&identity(&file.metadata()?)?)? != expected
        || serde_json::to_string(&identity(&fs::metadata(path)?)?)? != expected
    {
        bail!("source changed during read");
    }
    Ok(json!({"data":STANDARD.encode(&bytes),"bytes":bytes.len(),"offset":offset}))
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Receive {
    #[serde(default)]
    parent_identity: Option<(u64, u64)>,
    key: String,
    requested: String,
    path: String,
    source_identity: String,
    total: u64,
    bytes: u64,
    conflict: String,
    mode: u32,
    status: String,
    sha256: Option<String>,
}
fn receive_root() -> Result<PathBuf> {
    let root = job_root()?.join("receives");
    fs::create_dir_all(&root)?;
    let metadata = fs::symlink_metadata(&root)?;
    if !metadata.is_dir() || metadata.is_symlink() {
        bail!("receive state is not a directory");
    }
    #[cfg(unix)]
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700))?;
    Ok(root)
}
fn receive_load(root: &Path, key: &str) -> Result<Option<Receive>> {
    validate_key(key)?;
    let path = root.join(format!("{key}.json"));
    if !path.try_exists()? {
        return Ok(None);
    }
    Ok(Some(serde_json::from_reader(
        open_read(&path)?.take(32768),
    )?))
}
fn receive_save(root: &Path, r: &Receive) -> Result<()> {
    let path = root.join(format!("{}.{}.tmp", r.key, std::process::id()));
    let mut opts = OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    opts.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    let mut file = opts.open(&path)?;
    file.write_all(&serde_json::to_vec(r)?)?;
    file.sync_all()?;
    fs::rename(path, root.join(format!("{}.json", r.key)))?;
    Ok(())
}
fn receive_anchor(r: &Receive) -> Result<Anchor> {
    let a = Anchor::parent(&decode_path(&r.path)?)?;
    if r.parent_identity != Some(a.parent_identity()?) {
        bail!("destination parent changed; refusing redirected writes");
    }
    Ok(a)
}
fn partial_name(r: &Receive) -> std::ffi::OsString {
    format!(".cx-{}.partial", r.key).into()
}
fn receive_state(r: &Receive) -> Result<Value> {
    let prefix = if r.status == "receiving" {
        let a = receive_anchor(r)?;
        let mut f = a.open(&partial_name(r), libc::O_RDONLY)?;
        format!("{:x}", hash_prefix(&mut f, r.bytes)?.finalize())
    } else {
        r.sha256.clone().unwrap_or_default()
    };
    Ok(
        json!({"key":r.key,"path":r.path,"bytes":r.bytes,"total":r.total,"status":r.status,"prefix_sha256":prefix}),
    )
}
fn receive_prepare(
    path: &Path,
    key: &str,
    source_identity: &str,
    total: u64,
    conflict: &str,
    mode: u32,
) -> Result<Value> {
    validate_key(key)?;
    if source_identity.len() > 2048 {
        bail!("source identity too large");
    }
    if !matches!(conflict, "skip" | "overwrite" | "rename") {
        bail!("invalid conflict policy");
    }
    let root = receive_root()?;
    let _lock = Lock::acquire(&root, key)?;
    let requested = absolute(path)?;
    if let Some(r) = receive_load(&root, key)? {
        if r.requested != encode_path(&requested)
            || r.source_identity != source_identity
            || r.total != total
            || r.conflict != conflict
            || r.mode != (mode & 0o777)
        {
            bail!("receive key belongs to another file");
        }
        if r.status == "receiving" {
            let anchor = receive_anchor(&r)?;
            let file = anchor.open(&partial_name(&r), libc::O_RDWR)?;
            if file.metadata()?.len() < r.bytes {
                bail!("partial file shorter than recorded progress");
            }
            file.set_len(r.bytes)?;
        }
        return receive_state(&r);
    }
    let mut path = requested.clone();
    if let Ok(m) = fs::symlink_metadata(&path) {
        if conflict == "skip" {
            let r = Receive {
                parent_identity: Some(Anchor::parent(&path)?.parent_identity()?),
                key: key.into(),
                requested: encode_path(&requested),
                path: encode_path(&path),
                source_identity: source_identity.into(),
                total,
                bytes: 0,
                conflict: conflict.into(),
                mode: mode & 0o777,
                status: "skipped".into(),
                sha256: None,
            };
            receive_save(&root, &r)?;
            return receive_state(&r);
        }
        if conflict == "rename" {
            let name = path
                .file_name()
                .context("destination has no filename")?
                .to_owned();
            let parent = path.parent().context("destination has no parent")?;
            let mut found = None;
            for n in 1..=10000 {
                let mut candidate = name.clone();
                candidate.push(format!(".copy-{n}"));
                let p = parent.join(candidate);
                if fs::symlink_metadata(&p).is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound)
                {
                    found = Some(p);
                    break;
                }
            }
            path = found.context("no available rename destination")?;
        } else if !m.is_file() {
            bail!("only regular destinations can be overwritten");
        }
    }
    if !path.parent().context("destination has no parent")?.is_dir() {
        bail!("destination directory does not exist");
    }
    let r = Receive {
        parent_identity: Some(Anchor::parent(&path)?.parent_identity()?),
        key: key.into(),
        requested: encode_path(&requested),
        path: encode_path(&path),
        source_identity: source_identity.into(),
        total,
        bytes: 0,
        conflict: conflict.into(),
        mode: mode & 0o777,
        status: "receiving".into(),
        sha256: None,
    };
    let anchor = receive_anchor(&r)?;
    anchor
        .open(
            &partial_name(&r),
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL,
        )?
        .sync_all()?;
    receive_save(&root, &r)?;
    receive_state(&r)
}
fn receive_chunk(key: &str, offset: u64, data: &str) -> Result<Value> {
    validate_key(key)?;
    if data.len() > CHUNK_LIMIT * 4 / 3 + 4 {
        bail!("chunk too large");
    }
    let bytes = STANDARD.decode(data)?;
    if bytes.len() > CHUNK_LIMIT {
        bail!("chunk too large");
    }
    let root = receive_root()?;
    let _lock = Lock::acquire(&root, key)?;
    let mut r = receive_load(&root, key)?.context("unknown receive key")?;
    if r.status != "receiving" {
        bail!("receive is not accepting data");
    }
    if offset
        .checked_add(bytes.len() as u64)
        .is_none_or(|end| end > r.total)
    {
        bail!("chunk exceeds source length");
    }
    let anchor = receive_anchor(&r)?;
    let mut file = anchor.open(&partial_name(&r), libc::O_RDWR)?;
    if offset < r.bytes {
        if offset + bytes.len() as u64 > r.bytes {
            bail!("chunk overlaps checkpoint");
        }
        file.seek(SeekFrom::Start(offset))?;
        let mut existing = vec![0; bytes.len()];
        file.read_exact(&mut existing)?;
        if existing != bytes {
            bail!("retry data differs from written chunk");
        }
    } else {
        if offset != r.bytes {
            bail!("chunk offset is not next recorded byte");
        }
        file.set_len(r.bytes)?;
        file.seek(SeekFrom::Start(r.bytes))?;
        file.write_all(&bytes)?;
        file.sync_data()?;
        r.bytes += bytes.len() as u64;
        receive_save(&root, &r)?;
    }
    Ok(json!({"key":key,"bytes":r.bytes,"status":r.status}))
}
fn receive_finalize(key: &str, checksum: &str) -> Result<Value> {
    if checksum.len() != 64 || !checksum.bytes().all(|b| b.is_ascii_hexdigit()) {
        bail!("invalid SHA-256 checksum");
    }
    let root = receive_root()?;
    validate_key(key)?;
    let _lock = Lock::acquire(&root, key)?;
    let mut r = receive_load(&root, key)?.context("unknown receive key")?;
    if r.status == "complete" {
        if r.sha256.as_deref() != Some(checksum) {
            bail!("finalize checksum changed");
        }
        return receive_state(&r);
    }
    if r.status == "skipped" {
        return receive_state(&r);
    }
    if r.bytes != r.total {
        bail!("partial is not complete");
    }
    let anchor = receive_anchor(&r)?;
    let partial = partial_name(&r);
    if r.status == "finalizing" && anchor.mode(&partial)?.is_none() {
        let mut file = anchor.open(&anchor.name, libc::O_RDONLY)?;
        if file.metadata()?.len() != r.total
            || format!("{:x}", hash_prefix(&mut file, r.total)?.finalize()) != checksum
        {
            bail!("destination integrity mismatch on reconciliation");
        }
    } else {
        let mut file = anchor.open(&partial, libc::O_RDONLY)?;
        if file.metadata()?.len() != r.total
            || format!("{:x}", hash_prefix(&mut file, r.total)?.finalize()) != checksum
        {
            bail!("partial integrity mismatch");
        }
        #[cfg(unix)]
        file.set_permissions(fs::Permissions::from_mode(r.mode))?;
        file.sync_all()?;
        r.sha256 = Some(checksum.into());
        r.status = "finalizing".into();
        receive_save(&root, &r)?;
        if r.conflict == "overwrite" {
            if anchor
                .mode(&anchor.name)?
                .is_some_and(|mode| mode & libc::S_IFMT != libc::S_IFREG)
            {
                bail!("destination changed to a non-regular entry");
            }
            anchor.rename(&partial, &anchor.name)?;
        } else {
            anchor
                .link(&partial, &anchor.name)
                .context("destination appeared before finalization")?;
            anchor.unlink(&partial)?;
        }
        anchor.dir.sync_all()?;
    }
    r.status = "complete".into();
    r.sha256 = Some(checksum.into());
    receive_save(&root, &r)?;
    receive_state(&r)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct SymlinkReceive {
    #[serde(default)]
    parent_identity: Option<(u64, u64)>,
    key: String,
    requested: String,
    path: String,
    target: String,
    conflict: String,
    status: String,
}
fn receive_symlink(path: &Path, target: &Path, conflict: &str, key: &str) -> Result<Value> {
    valid_symlink_request(conflict, key)?;
    #[cfg(not(unix))]
    {
        let _ = (path, target);
        bail!("symlink copy currently requires Unix");
    }
    #[cfg(unix)]
    {
        let root = receive_root()?;
        let _lock = Lock::acquire(&root, key)?;
        let requested = absolute(path)?;
        let record = root.join(format!("{key}.symlink.json"));
        let mut r = if record.try_exists()? {
            let r: SymlinkReceive = serde_json::from_reader(open_read(&record)?.take(32768))?;
            if r.requested != encode_path(&requested)
                || r.target != encode_path(target)
                || r.conflict != conflict
            {
                bail!("symlink key belongs to another copy");
            }
            r
        } else {
            let mut chosen = requested.clone();
            let mut status = "pending";
            if let Ok(m) = fs::symlink_metadata(&chosen) {
                if conflict == "skip" {
                    status = "skipped";
                } else if conflict == "rename" {
                    let name = chosen.file_name().context("no filename")?.to_owned();
                    let parent = chosen.parent().context("no parent")?;
                    let mut found = None;
                    for n in 1..=10000 {
                        let mut candidate = name.clone();
                        candidate.push(format!(".copy-{n}"));
                        let p = parent.join(candidate);
                        if fs::symlink_metadata(&p)
                            .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound)
                        {
                            found = Some(p);
                            break;
                        }
                    }
                    chosen = found.context("no available rename destination")?;
                } else if m.is_dir() {
                    bail!("cannot overwrite a directory with a symlink");
                }
            }
            SymlinkReceive {
                parent_identity: Some(Anchor::parent(&chosen)?.parent_identity()?),
                key: key.into(),
                requested: encode_path(&requested),
                path: encode_path(&chosen),
                target: encode_path(target),
                conflict: conflict.into(),
                status: status.into(),
            }
        };
        let save = |r: &SymlinkReceive| -> Result<()> {
            let tmp = root.join(format!("{key}.{}.symlink.tmp", std::process::id()));
            let mut opts = OpenOptions::new();
            opts.write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW);
            let mut file = opts.open(&tmp)?;
            file.write_all(&serde_json::to_vec(r)?)?;
            file.sync_all()?;
            fs::rename(tmp, &record)?;
            Ok(())
        };
        if r.status == "complete" || r.status == "skipped" {
            return Ok(json!({"path":r.path,"status":r.status}));
        }
        save(&r)?;
        let anchor = Anchor::parent(&decode_path(&r.path)?)?;
        if r.parent_identity != Some(anchor.parent_identity()?) {
            bail!("symlink destination parent changed");
        }
        let temp: std::ffi::OsString = format!(".cx-{key}.symlink.partial").into();
        if r.status == "finalizing" && anchor.mode(&temp)?.is_none() {
            if anchor.readlink(&anchor.name)? != target {
                bail!("finalized symlink target changed");
            }
        } else {
            if let Some(mode) = anchor.mode(&temp)? {
                if mode & libc::S_IFMT != libc::S_IFLNK || anchor.readlink(&temp)? != target {
                    bail!("symlink partial does not belong to this job");
                }
            } else {
                anchor.symlink(&temp, target)?;
            }
            r.status = "finalizing".into();
            save(&r)?;
            if conflict == "overwrite" {
                if anchor
                    .mode(&anchor.name)?
                    .is_some_and(|mode| mode & libc::S_IFMT == libc::S_IFDIR)
                {
                    bail!("destination changed to a directory");
                }
                anchor.rename(&temp, &anchor.name)?;
            } else {
                anchor.link(&temp, &anchor.name)?;
                anchor.unlink(&temp)?;
            }
            anchor.dir.sync_all()?;
        }
        r.status = "complete".into();
        save(&r)?;
        Ok(json!({"path":r.path,"status":r.status}))
    }
}
fn valid_symlink_request(conflict: &str, key: &str) -> Result<()> {
    validate_key(key)?;
    if !matches!(conflict, "skip" | "overwrite" | "rename") {
        bail!("invalid conflict policy");
    }
    Ok(())
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
    #[cfg(unix)]
    fn anchored_directory_swap_never_redirects_writes() {
        let d = tempdir().unwrap();
        let selected = d.path().join("selected");
        let outside = d.path().join("outside");
        let moved = d.path().join("moved");
        fs::create_dir(&selected).unwrap();
        fs::create_dir(&outside).unwrap();
        let anchor = Anchor::parent(&selected.join("file")).unwrap();
        fs::rename(&selected, &moved).unwrap();
        std::os::unix::fs::symlink(&outside, &selected).unwrap();
        anchor
            .open(&anchor.name, libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL)
            .unwrap()
            .write_all(b"owned")
            .unwrap();
        assert!(!outside.join("file").exists());
        assert_eq!(fs::read(moved.join("file")).unwrap(), b"owned");
        assert!(Anchor::parent(&selected.join("later")).is_err());
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
