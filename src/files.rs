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
#[path = "file_preview.rs"]
mod file_preview;

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
        Operation::PreviewPage { path, page } => {
            let path = decode_path(path)?;
            file_preview::render_page(&path, open_read(&path)?, *page)
        }
        Operation::Rename {
            path,
            name,
            expected_identity,
        } => rename_entry(&decode_path(path)?, name, expected_identity.as_deref()),
        Operation::Remove {
            path,
            expected_identity,
        } => remove_entry(&decode_path(path)?, expected_identity.as_deref()),
        Operation::RemoveEmptyDirectory { path, identity } => {
            remove_empty_directory(&decode_path(path)?, identity)
        }
        Operation::Move {
            path,
            destination,
            expected_identity,
        } => move_entry(
            &decode_path(path)?,
            &decode_path(destination)?,
            expected_identity.as_deref(),
        ),
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
/// First-page size for `Operation::List`; continuation pages accept up to this many too.
/// Old helpers reject `ListPage` limits above 1000, so clients only ask for more after
/// seeing `"order"` in a reply (the field this helper version adds).
pub(crate) const FIRST_PAGE_LIMIT: usize = 4000;
/// The largest `ListPage` offset any helper version accepts.
pub(crate) const MAX_LIST_OFFSET: u64 = 100_000;
/// Serialized bytes of entries per page. The 1 MiB frame also carries three paths and the
/// response envelope (and may be nested inside a container relay), so stay well under it.
const LIST_BYTES: usize = 768 * 1024;
/// One read_dir pass never holds more names than this; old helpers stopped at `MAX_LIST_OFFSET`.
const SNAPSHOT_ENTRIES: usize = 131_072;
/// Bound on snapshots and total names kept for continuation pages.
const SNAPSHOT_CACHE: usize = 8;
const SNAPSHOT_CACHE_ENTRIES: usize = 262_144;
const SNAPSHOT_TTL: std::time::Duration = std::time::Duration::from_secs(60);
/// Order of `entries` across all pages: directories first, then display name, then raw name.
/// This is the UI's own sort key, so a client that sees it may append pages without re-sorting.
pub(crate) const LIST_ORDER: &str = "directories-first-name";

/// Which directory a snapshot lists. Its times are not part of it: a continuation page
/// comes from its first page's snapshot even after the directory changed, so the pages
/// never duplicate or skip a row (the next open or refresh rereads).
#[derive(Clone, PartialEq, Eq)]
struct SnapshotKey {
    path: PathBuf,
    device: u64,
    inode: u64,
}
struct SnapshotEntry {
    name: std::ffi::OsString,
    display: String,
    directory: bool,
}
struct Snapshot {
    key: SnapshotKey,
    taken: std::time::Instant,
    entries: std::sync::Arc<Vec<SnapshotEntry>>,
    incomplete: bool,
}
static SNAPSHOTS: std::sync::Mutex<Vec<Snapshot>> = std::sync::Mutex::new(Vec::new());

fn snapshot_key(path: &Path) -> Result<SnapshotKey> {
    let m = fs::metadata(path).context("open directory")?;
    if !m.is_dir() {
        bail!("open directory: not a directory");
    }
    Ok(SnapshotKey {
        path: path.to_path_buf(),
        device: m.dev(),
        inode: m.ino(),
    })
}
/// One read_dir pass, sorted in the UI's order. Uses the dirent type only (no stat per entry,
/// except where the filesystem reports DT_UNKNOWN and std falls back to lstat).
fn read_snapshot(key: SnapshotKey) -> Result<Snapshot> {
    let mut entries = Vec::new();
    let mut incomplete = false;
    for entry in fs::read_dir(&key.path).context("open directory")? {
        if entries.len() == SNAPSHOT_ENTRIES {
            incomplete = true;
            break;
        }
        let entry = entry?;
        let name = entry.file_name();
        let directory = match entry.file_type() {
            Ok(t) => t.is_dir(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e.into()),
        };
        entries.push(SnapshotEntry {
            display: display(&name.to_string_lossy()),
            name,
            directory,
        });
    }
    entries.sort_unstable_by(|a, b| {
        (!a.directory, a.display.as_str(), a.name.as_os_str()).cmp(&(
            !b.directory,
            b.display.as_str(),
            b.name.as_os_str(),
        ))
    });
    Ok(Snapshot {
        key,
        taken: std::time::Instant::now(),
        entries: std::sync::Arc::new(entries),
        incomplete,
    })
}
type Snapshotted = (std::sync::Arc<Vec<SnapshotEntry>>, bool);
/// A fresh snapshot for offset 0 (every open or refresh rereads); continuation pages reuse
/// the snapshot their first page came from, or get None when it is gone (expired, evicted,
/// a restarted helper, a replaced directory): paging a new snapshot at the old offset
/// would duplicate or skip rows.
fn snapshot(path: &Path, offset: u64) -> Result<Option<Snapshotted>> {
    let key = snapshot_key(path)?;
    let mut cache = SNAPSHOTS.lock().unwrap_or_else(|e| e.into_inner());
    cache.retain(|s| s.taken.elapsed() < SNAPSHOT_TTL);
    if offset > 0 {
        let Some(i) = cache.iter().position(|s| s.key == key) else {
            return Ok(None);
        };
        let s = cache.remove(i);
        let found = (s.entries.clone(), s.incomplete);
        cache.push(s);
        return Ok(Some(found));
    }
    cache.retain(|s| s.key.path != key.path);
    drop(cache);
    let s = read_snapshot(key)?;
    let found = (s.entries.clone(), s.incomplete);
    let mut cache = SNAPSHOTS.lock().unwrap_or_else(|e| e.into_inner());
    cache.retain(|c| c.key.path != s.key.path);
    cache.push(s);
    while cache.len() > SNAPSHOT_CACHE
        || cache.iter().map(|s| s.entries.len()).sum::<usize>() > SNAPSHOT_CACHE_ENTRIES
    {
        cache.remove(0);
    }
    Ok(Some(found))
}
/// The last page a client can ask for was served: nothing will read the snapshot again.
fn forget_snapshot(path: &Path) {
    let mut cache = SNAPSHOTS.lock().unwrap_or_else(|e| e.into_inner());
    cache.retain(|s| s.key.path != path);
}
/// Keys, punctuation, the longest kind, a u64 size, a bool and the comma.
const ENTRY_JSON_OVERHEAD: usize = 128;
/// Serialized length of `text` as a JSON string, quotes included.
fn json_string_len(text: &str) -> usize {
    2 + text
        .chars()
        .map(|c| match c {
            '"' | '\\' | '\n' | '\r' | '\t' | '\u{8}' | '\u{c}' => 2,
            c if (c as u32) < 0x20 => 6,
            c => c.len_utf8(),
        })
        .sum::<usize>()
}
fn list(path: &Path) -> Result<Value> {
    list_page(path, 0, FIRST_PAGE_LIMIT as u32)
}
fn list_page(path: &Path, offset: u64, limit: u32) -> Result<Value> {
    if limit == 0 || limit > FIRST_PAGE_LIMIT as u32 || offset > MAX_LIST_OFFSET {
        bail!("listing pagination exceeds bounds");
    }
    let path = fs::canonicalize(path).context("open directory")?;
    let Some((snapshot, incomplete)) = snapshot(&path, offset)? else {
        // The client lists again from offset 0 (older clients see an empty last page).
        return Ok(json!({"restart":true,"entries":[],"next_offset":null,"truncated":true}));
    };
    let mut entries = Vec::new();
    let mut bytes = 0;
    let mut index = (offset as usize).min(snapshot.len());
    while index < snapshot.len() && entries.len() < limit as usize {
        let item = &snapshot[index];
        let child = path.join(&item.name);
        // Size and identity are reported, so each returned entry needs exactly one lstat.
        let metadata = match fs::symlink_metadata(&child) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                index += 1;
                continue;
            }
            Err(e) => return Err(e.into()),
        };
        let kind = if metadata.is_symlink() {
            "symlink"
        } else if metadata.is_dir() {
            "directory"
        } else if metadata.is_file() {
            "file"
        } else {
            "other"
        };
        let rename_name = item
            .name
            .to_str()
            .filter(|s| !s.chars().any(char::is_control));
        let hidden = item.name.as_bytes().first() == Some(&b'.');
        let opaque = encode_path(&child);
        let identity = serde_json::to_string(&identity(&metadata)?)?;
        // Upper bound of the entry's serialized size without serializing it twice.
        let size = ENTRY_JSON_OVERHEAD
            + json_string_len(&item.display)
            + json_string_len(&opaque)
            + json_string_len(&identity)
            + rename_name.map_or(4, json_string_len);
        let entry = json!({"name":item.display,"path":opaque,"kind":kind,"size":metadata.len(),
            "hidden":hidden,"rename_name":rename_name,"identity":identity});
        if !entries.is_empty() && bytes + size > LIST_BYTES {
            break;
        }
        bytes += size;
        entries.push(entry);
        index += 1;
    }
    let next_offset = (index < snapshot.len()).then_some(index as u64);
    if next_offset.is_none_or(|next| next > MAX_LIST_OFFSET) {
        forget_snapshot(&path);
    }
    Ok(json!({
        "path":encode_path(&path),
        "display_path":display(&path.to_string_lossy()),
        "parent":path.parent().map(encode_path),
        "next_offset":next_offset,
        "entries":entries,
        "truncated":next_offset.is_some() || incomplete,
        "order":LIST_ORDER,
        "total":snapshot.len(),
        "incomplete":incomplete,
    }))
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
/// The helper's preview text normalization (lossy UTF-8, CR/LF, tabs, escaped controls),
/// so viewer-streamed continuations match the first preview window exactly.
pub(crate) fn preview_text(bytes: &[u8]) -> String {
    file_preview::safe_text(bytes)
}
/// The helper's text preview kind for a name: "markdown", "code" or "text".
pub(crate) fn preview_kind(path: &Path) -> &'static str {
    file_preview::text_kind(path)
}
fn preview(path: &Path) -> Result<Value> {
    file_preview::render(path, open_read(path)?)
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
// Name mutations remain relative to an open parent, never a shell command.
fn mutation_stat(anchor: &Anchor, name: &std::ffi::OsStr) -> Result<libc::stat> {
    let name = Anchor::cstr(name)?;
    let mut stat = std::mem::MaybeUninit::uninit();
    if unsafe {
        libc::fstatat(
            anchor.dir.as_raw_fd(),
            name.as_ptr(),
            stat.as_mut_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    } != 0
    {
        return Err(std::io::Error::last_os_error()).context("inspect selected entry");
    }
    Ok(unsafe { stat.assume_init() })
}
fn check_mutation_identity(stat: &libc::stat, expected: Option<&str>) -> Result<()> {
    if let Some(expected) = expected {
        if expected.len() > 2048 {
            bail!("expected identity too large");
        }
        let expected: Identity = serde_json::from_str(expected)?;
        let modified =
            (stat.st_mtime as u128).saturating_mul(1_000_000_000) + stat.st_mtime_nsec as u128;
        if stat.st_dev as u64 != expected.device
            || stat.st_ino as u64 != expected.inode
            || stat.st_size as u64 != expected.size
            || modified != expected.modified
        {
            bail!("selected entry changed; refresh before trying again");
        }
    }
    Ok(())
}
fn rename_noreplace(anchor: &Anchor, from: &std::ffi::OsStr, to: &std::ffi::OsStr) -> Result<()> {
    rename_between(anchor, from, anchor, to)
}
fn rename_between(
    anchor: &Anchor,
    from: &std::ffi::OsStr,
    destination: &Anchor,
    to: &std::ffi::OsStr,
) -> Result<()> {
    let from = Anchor::cstr(from)?;
    let to = Anchor::cstr(to)?;
    #[cfg(target_os = "linux")]
    if unsafe {
        libc::syscall(
            libc::SYS_renameat2,
            anchor.dir.as_raw_fd(),
            from.as_ptr(),
            destination.dir.as_raw_fd(),
            to.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    } == 0
    {
        return Ok(());
    }
    #[cfg(not(target_os = "linux"))]
    bail!("safe no-overwrite rename requires Linux");
    #[cfg(target_os = "linux")]
    Err(std::io::Error::last_os_error()).context("rename without overwriting")
}
fn protected_mutation(path: &Path, anchor: &Anchor) -> Result<()> {
    // Reject dot components rather than letting file_name normalize them away.
    if path
        .as_os_str()
        .as_bytes()
        .split(|b| *b == b'/')
        .any(|p| p == b"." || p == b"..")
    {
        bail!("dot components are not valid mutation paths");
    }
    let selected = mutation_stat(anchor, &anchor.name)?;
    for protected in [
        Some(std::env::current_dir()?),
        std::env::var_os("HOME").map(PathBuf::from),
    ]
    .into_iter()
    .flatten()
    {
        if let Ok(m) = fs::metadata(protected) {
            if selected.st_dev as u64 == m.dev() && selected.st_ino as u64 == m.ino() {
                bail!("refusing to mutate the current directory or home directory");
            }
        }
    }
    Ok(())
}
fn rename_entry(path: &Path, name: &str, expected: Option<&str>) -> Result<Value> {
    let destination_name = PathBuf::from(name);
    let bytes = destination_name.as_os_str().as_bytes();
    if bytes.is_empty() || bytes.contains(&b'/') || bytes == b"." || bytes == b".." {
        bail!("rename requires one filename, not a path");
    }
    let path = absolute(path)?;
    let anchor = Anchor::parent(&path)?;
    protected_mutation(&path, &anchor)?;
    let destination = path
        .parent()
        .context("missing parent")?
        .join(destination_name);
    move_entry(&path, &destination, expected)
}
fn move_entry(path: &Path, destination: &Path, expected: Option<&str>) -> Result<Value> {
    let path = absolute(path)?;
    let destination = absolute(destination)?;
    let source = Anchor::parent(&path)?;
    let target = Anchor::parent(&destination)?;
    protected_mutation(&path, &source)?;
    if destination
        .as_os_str()
        .as_bytes()
        .split(|b| *b == b'/')
        .any(|p| p == b"." || p == b"..")
    {
        bail!("dot components are not valid mutation paths");
    }
    let before = mutation_stat(&source, &source.name)?;
    check_mutation_identity(&before, expected)?;
    rename_between(&source, &source.name, &target, &target.name)?;
    let claimed = mutation_stat(&target, &target.name)?;
    if before.st_dev != claimed.st_dev
        || before.st_ino != claimed.st_ino
        || check_mutation_identity(&claimed, expected).is_err()
    {
        let restored = rename_between(&target, &target.name, &source, &source.name);
        bail!(
            "selected entry changed during move; restore {}",
            if restored.is_ok() {
                "complete"
            } else {
                "failed; inspect destination"
            }
        );
    }
    source.dir.sync_all()?;
    target.dir.sync_all()?;
    Ok(json!({"path":encode_path(&destination),"moved":true}))
}
/// Tree moves remove only an empty directory with the original device/inode.
/// Child deletion changes directory timestamps, so the full selection identity is inappropriate here.
fn remove_empty_directory(path: &Path, expected: &str) -> Result<Value> {
    let path = absolute(path)?;
    let anchor = Anchor::parent(&path)?;
    protected_mutation(&path, &anchor)?;
    let original: Identity = serde_json::from_str(expected)?;
    let initial = mutation_stat(&anchor, &anchor.name)?;
    anyhow::ensure!(
        initial.st_mode & libc::S_IFMT == libc::S_IFDIR
            && initial.st_dev == original.device
            && initial.st_ino == original.inode,
        "Move directory changed; remaining source retained"
    );
    let quarantine = std::ffi::OsString::from(format!(
        ".cx-empty-{}-{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
    ));
    rename_noreplace(&anchor, &anchor.name, &quarantine)?;
    let claimed = mutation_stat(&anchor, &quarantine)?;
    let same = claimed.st_dev == initial.st_dev
        && claimed.st_ino == initial.st_ino
        && claimed.st_mode & libc::S_IFMT == libc::S_IFDIR;
    let name = Anchor::cstr(&quarantine)?;
    let removed = same
        && unsafe { libc::unlinkat(anchor.dir.as_raw_fd(), name.as_ptr(), libc::AT_REMOVEDIR) }
            == 0;
    if !removed {
        let restored = rename_noreplace(&anchor, &quarantine, &anchor.name);
        bail!(
            "Move directory is changed, busy or nonempty; source retained (restore: {})",
            if restored.is_ok() {
                "complete"
            } else {
                "inspect hidden .cx-empty entry"
            }
        );
    }
    anchor.dir.sync_all()?;
    Ok(json!({"path":encode_path(&path),"removed":true}))
}
fn remove_entry(path: &Path, expected: Option<&str>) -> Result<Value> {
    let path = absolute(path)?;
    let anchor = Anchor::parent(&path)?;
    protected_mutation(&path, &anchor)?;
    let initial = mutation_stat(&anchor, &anchor.name)?;
    check_mutation_identity(&initial, expected)?;
    // Claim the exact selected name first. A changed object is restored, never deleted.
    let quarantine = std::ffi::OsString::from(format!(
        ".cx-delete-{}-{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
    ));
    rename_noreplace(&anchor, &anchor.name, &quarantine)?;
    let claimed = mutation_stat(&anchor, &quarantine)?;
    if claimed.st_dev != initial.st_dev
        || claimed.st_ino != initial.st_ino
        || check_mutation_identity(&claimed, expected).is_err()
    {
        let restored = rename_noreplace(&anchor, &quarantine, &anchor.name);
        bail!(
            "selected entry changed; refusing deletion (restore: {})",
            if restored.is_ok() {
                "complete"
            } else {
                "failed; inspect hidden .cx-delete entry"
            }
        );
    }
    let mut budget = 100_000usize;
    let result = remove_anchored(&anchor, &quarantine, initial.st_dev, 0, &mut budget);
    if let Err(error) = result {
        let restored = rename_noreplace(&anchor, &quarantine, &anchor.name);
        bail!(
            "deletion incomplete: {error:#}; remaining entry {}",
            if restored.is_ok() {
                "restored to original name"
            } else {
                "retained under hidden .cx-delete name"
            }
        );
    }
    anchor.dir.sync_all()?;
    Ok(json!({"path":encode_path(&path),"removed":true}))
}
fn remove_anchored(
    anchor: &Anchor,
    name: &std::ffi::OsStr,
    device: libc::dev_t,
    depth: usize,
    budget: &mut usize,
) -> Result<()> {
    use std::os::unix::io::FromRawFd;
    if depth > 64 || *budget == 0 {
        bail!("deletion exceeds depth or entry limit");
    }
    *budget -= 1;
    let before = mutation_stat(anchor, name)?;
    if before.st_dev != device {
        bail!("refusing to cross a filesystem boundary");
    }
    let cname = Anchor::cstr(name)?;
    let directory = before.st_mode & libc::S_IFMT == libc::S_IFDIR;
    if directory {
        // RESOLVE_NO_XDEV also rejects bind mounts on the same filesystem.
        // Older kernels fail safely instead of falling back to unsafe traversal.
        #[repr(C)]
        struct OpenHow {
            flags: u64,
            mode: u64,
            resolve: u64,
        }
        let how = OpenHow {
            flags: (libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC) as u64,
            mode: 0,
            resolve: 1 | 4,
        };
        let fd = unsafe {
            libc::syscall(
                libc::SYS_openat2,
                anchor.dir.as_raw_fd(),
                cname.as_ptr(),
                &how,
                std::mem::size_of::<OpenHow>(),
            )
        } as i32;
        if fd < 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        let dir = unsafe { File::from_raw_fd(fd) };
        let m = dir.metadata()?;
        if m.ino() != before.st_ino as u64 || m.dev() != before.st_dev as u64 {
            bail!("directory changed during deletion");
        }
        let child = Anchor {
            dir,
            name: std::ffi::OsString::new(),
        };
        // /proc exposes the already-open descriptor; no selected path is retraversed.
        for entry in fs::read_dir(format!("/proc/self/fd/{}", child.dir.as_raw_fd()))? {
            remove_anchored(&child, &entry?.file_name(), device, depth + 1, budget)?;
        }
    }
    let after = mutation_stat(anchor, name)?;
    if after.st_dev != before.st_dev || after.st_ino != before.st_ino {
        bail!("entry changed during deletion");
    }
    if unsafe {
        libc::unlinkat(
            anchor.dir.as_raw_fd(),
            cname.as_ptr(),
            if directory { libc::AT_REMOVEDIR } else { 0 },
        )
    } != 0
    {
        return Err(std::io::Error::last_os_error()).context("remove selected entry");
    }
    Ok(())
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
    /// transport::MAX_MESSAGE; files.rs is also compiled into integration tests without transport.
    const FRAME_LIMIT: usize = 1024 * 1024;
    /// The pre-snapshot helper listing (readdir order, skip(offset), lstat each), kept to prove
    /// the new pages contain exactly what the old pages plus the UI's sort produced.
    fn old_list_page(path: &Path, offset: u64, limit: u32) -> Value {
        let path = fs::canonicalize(path).unwrap();
        let mut entries = Vec::new();
        let mut truncated = false;
        let mut budget = 0;
        for entry in fs::read_dir(&path).unwrap().skip(offset as usize) {
            let entry = entry.unwrap();
            let metadata = fs::symlink_metadata(entry.path()).unwrap();
            let name = display(&entry.file_name().to_string_lossy());
            let opaque = encode_path(&entry.path());
            budget += name.len() + opaque.len() + entry.file_name().as_bytes().len() + 256;
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
            let filename = entry.file_name();
            let rename_name = filename
                .to_str()
                .filter(|s| !s.chars().any(char::is_control));
            let hidden = filename.as_bytes().first() == Some(&b'.');
            entries.push(json!({"name":name,"path":opaque,"kind":kind,"size":metadata.len(),
                "hidden":hidden,"rename_name":rename_name,"identity":serde_json::to_string(&identity(&metadata).unwrap()).unwrap()}));
        }
        json!({"next_offset":if truncated{Some(offset+entries.len() as u64)}else{None},"entries":entries})
    }
    /// Pages the way the UI does: List, then ListPage limit 1000 from each next_offset.
    fn all_pages(path: &Path, first: impl Fn() -> Value, limit: u32) -> (Vec<Value>, usize) {
        let mut page = first();
        let mut entries = Vec::new();
        let mut requests = 1;
        loop {
            entries.extend(page["entries"].as_array().unwrap().iter().cloned());
            let Some(offset) = page["next_offset"].as_u64() else {
                break;
            };
            page = handle(&Operation::ListPage {
                path: encode_path(path),
                offset,
                limit,
            })
            .unwrap_or_else(|_| old_list_page(path, offset, limit));
            requests += 1;
        }
        (entries, requests)
    }
    fn ui_sort(entries: &mut [Value]) {
        entries.sort_by(|a, b| {
            (a["kind"] != "directory", a["name"].as_str())
                .cmp(&(b["kind"] != "directory", b["name"].as_str()))
        });
    }
    #[test]
    fn twenty_thousand_entry_listing_is_fast_sorted_and_matches_old_pages() {
        let d = tempdir().unwrap();
        for i in 0..20_000u32 {
            let name = format!("entry-{:05}-{}", (i * 7919) % 20_000, i % 3);
            if i % 97 == 0 {
                fs::create_dir(d.path().join(&name)).unwrap();
            } else {
                fs::write(d.path().join(&name), vec![b'x'; (i % 5) as usize]).unwrap();
            }
        }
        std::os::unix::fs::symlink("entry-00000-0", d.path().join("link")).unwrap();
        fs::write(d.path().join(".hidden"), b"h").unwrap();
        // Old helper: readdir order with skip(offset), 1000 per page, sorted by the UI after.
        let started = std::time::Instant::now();
        let mut old = Vec::new();
        let mut offset = 0;
        loop {
            let page = old_list_page(d.path(), offset, 1000);
            old.extend(page["entries"].as_array().unwrap().iter().cloned());
            match page["next_offset"].as_u64() {
                Some(next) => offset = next,
                None => break,
            }
        }
        let old_time = started.elapsed();
        ui_sort(&mut old);
        let started = std::time::Instant::now();
        let (new, requests) = all_pages(d.path(), || list(d.path()).unwrap(), 1000);
        let new_time = started.elapsed();
        eprintln!(
            "20002-entry listing: old {old_time:?} (21 requests), new {new_time:?} ({requests} requests)"
        );
        assert_eq!(new.len(), 20_002);
        assert_eq!(new, old, "pages arrive pre-sorted with the same contents");
        let mut resorted = new.clone();
        ui_sort(&mut resorted);
        assert_eq!(
            resorted, new,
            "the UI's sort is a no-op on the helper's order"
        );
        assert!(new_time < std::time::Duration::from_secs(1), "{new_time:?}");
        let first = list(d.path()).unwrap();
        assert_eq!(first["order"], LIST_ORDER);
        assert_eq!(first["total"], 20_002);
        assert!(first["entries"].as_array().unwrap().len() > 1000);
        assert!(serde_json::to_vec(&first).unwrap().len() < FRAME_LIMIT);
        assert!(requests < 21);
    }
    #[test]
    fn listing_pages_fit_the_frame_with_long_names() {
        let d = tempdir().unwrap();
        let deep = d.path().join("d".repeat(200)).join("e".repeat(200));
        fs::create_dir_all(&deep).unwrap();
        for i in 0..5000 {
            fs::write(deep.join(format!("{i:05}{}\u{7f}", "n".repeat(240))), b"").unwrap();
        }
        let (entries, requests) =
            all_pages(&deep, || list(&deep).unwrap(), FIRST_PAGE_LIMIT as u32);
        assert_eq!(entries.len(), 5000);
        assert!(requests > 1, "byte budget split the listing");
        let mut offset = 0;
        loop {
            let page = list_page(&deep, offset, FIRST_PAGE_LIMIT as u32).unwrap();
            let response = crate::model::Response {
                version: 1,
                id: "x".repeat(64),
                result: Some(page.clone()),
                error: None,
            };
            assert!(serde_json::to_vec(&response).unwrap().len() < FRAME_LIMIT / 4 * 3 + 64 * 1024);
            let bytes: usize = page["entries"]
                .as_array()
                .unwrap()
                .iter()
                .map(|e| serde_json::to_string(e).unwrap().len() + 1)
                .sum();
            assert!(bytes <= LIST_BYTES, "{bytes}");
            match page["next_offset"].as_u64() {
                Some(next) => {
                    assert!(next > offset);
                    offset = next
                }
                None => break,
            }
        }
    }
    #[test]
    fn json_size_estimate_bounds_serialized_entries() {
        for text in [
            "plain",
            "q\"b\\s",
            "c\u{1}\u{1f}\n\t\u{7f}",
            "\u{e9}\u{1f600}",
            "",
        ] {
            assert_eq!(
                json_string_len(text),
                serde_json::to_string(text).unwrap().len()
            );
        }
        let d = tempdir().unwrap();
        fs::write(d.path().join("\"\\\u{1}\u{1b}[2J\n"), b"x").unwrap();
        fs::create_dir(d.path().join("dir")).unwrap();
        let page = list(d.path()).unwrap();
        for e in page["entries"].as_array().unwrap() {
            let estimate = ENTRY_JSON_OVERHEAD
                + json_string_len(e["name"].as_str().unwrap())
                + json_string_len(e["path"].as_str().unwrap())
                + json_string_len(e["identity"].as_str().unwrap())
                + e["rename_name"].as_str().map_or(4, json_string_len);
            assert!(serde_json::to_string(e).unwrap().len() + 1 <= estimate);
        }
    }
    fn names(page: &Value) -> Vec<String> {
        page["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["name"].as_str().unwrap().to_owned())
            .collect()
    }
    #[test]
    fn continuation_pages_never_duplicate_or_skip_rows_when_the_directory_changes() {
        let d = tempdir().unwrap();
        for i in 0..1500 {
            fs::write(d.path().join(format!("f{i:04}")), b"").unwrap();
        }
        let first = list_page(d.path(), 0, 1000).unwrap();
        assert_eq!(first["next_offset"], 1000);
        // Sorts first: a reread at offset 1000 would repeat f0999.
        fs::write(d.path().join("a-new"), b"").unwrap();
        fs::remove_file(d.path().join("f1499")).unwrap();
        let rest = list_page(d.path(), 1000, 1000).unwrap();
        let mut all = names(&first);
        all.extend(names(&rest));
        let want: Vec<_> = (0..1499).map(|i| format!("f{i:04}")).collect();
        assert_eq!(
            all, want,
            "one snapshot; the removed entry is skipped, not an error"
        );
        assert!(rest["next_offset"].is_null());
        // Opening again always rereads.
        assert_eq!(list(d.path()).unwrap()["total"], 1500);
        assert!(list_page(d.path(), 0, FIRST_PAGE_LIMIT as u32 + 1).is_err());
        assert!(list_page(d.path(), MAX_LIST_OFFSET + 1, 10).is_err());
    }
    #[test]
    fn a_continuation_without_its_snapshot_asks_for_a_restart_and_the_last_page_frees_it() {
        let d = tempdir().unwrap();
        for i in 0..1500 {
            fs::write(d.path().join(format!("f{i:04}")), b"").unwrap();
        }
        let path = fs::canonicalize(d.path()).unwrap();
        let cached = || SNAPSHOTS.lock().unwrap().iter().any(|s| s.key.path == path);
        let restart = list_page(d.path(), 1000, 1000).unwrap();
        assert_eq!(restart["restart"], true);
        assert_eq!(restart["entries"], json!([]));
        assert!(restart["next_offset"].is_null());
        list_page(d.path(), 0, 1000).unwrap();
        assert!(cached());
        let rest = list_page(d.path(), 1000, 1000).unwrap();
        assert_eq!(rest["entries"].as_array().unwrap().len(), 500);
        assert!(!cached(), "nothing pages past the last page");
        // A listing that fits one page keeps no snapshot either.
        list(d.path()).unwrap();
        assert!(!cached());
    }
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
