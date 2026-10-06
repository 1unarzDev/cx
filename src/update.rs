//! Verified public tagged releases. Call blocking entrypoints on a background worker.
use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::{
        fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt},
        io::AsRawFd,
        process::CommandExt,
    },
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
const REPO: &str = "1unarzDev/cx";
const API: &str = "https://api.github.com/repos/1unarzDev/cx/releases/latest";
const WORKFLOW: &str = "1unarzDev/cx/.github/workflows/release.yml";
const ARCHIVE_LIMIT: u64 = 32 * 1024 * 1024;
const BINARY_LIMIT: u64 = 64 * 1024 * 1024;
const TAR_LIMIT: u64 = BINARY_LIMIT + 1024 * 1024;

#[derive(Debug)]
pub enum CheckOutcome {
    Skipped,
    Current,
    Offline,
    Unavailable(String),
    Ready(UpdatePlan),
}
#[derive(Debug)]
pub struct UpdatePlan {
    pub version: String,
    stage: Stage,
    metadata: Metadata,
    home: PathBuf,
    state: PathBuf,
}
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
struct Metadata {
    version: String,
    digest: String,
    binary_dev: u64,
    binary_ino: u64,
    source_dev: u64,
    source_ino: u64,
}
#[derive(Debug)]
struct Dir(File);
impl Dir {
    fn path(&self, name: &str) -> PathBuf {
        PathBuf::from(format!(
            "/proc/{}/fd/{}",
            std::process::id(),
            self.0.as_raw_fd()
        ))
        .join(name)
    }
    fn child(&self, name: &str, create: bool) -> Result<Self> {
        ensure!(
            !name.contains('/') && name != "." && name != "..",
            "invalid directory"
        );
        let p = self.path(name);
        if create {
            match fs::DirBuilder::new().mode(0o700).create(&p) {
                Ok(()) => (),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => (),
                Err(e) => return Err(e.into()),
            }
        }
        Self::open(&p, true)
    }
    fn open(path: &Path, owned: bool) -> Result<Self> {
        let f = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_DIRECTORY | libc::O_CLOEXEC)
            .open(path)?;
        let m = f.metadata()?;
        if owned {
            ensure!(
                m.uid() == uid() && m.mode() & 0o022 == 0,
                "unsafe update directory"
            );
        }
        Ok(Self(f))
    }
}
fn uid() -> u32 {
    unsafe { libc::geteuid() }
}
// Walk absolute paths with no symlink traversal, then pin all subsequent I/O to directory FDs.
fn absolute_dir(path: &Path) -> Result<Dir> {
    use std::path::Component;
    ensure!(path.is_absolute(), "update path must be absolute");
    let mut d = Dir::open(Path::new("/"), false)?;
    for c in path.components() {
        match c {
            Component::RootDir => (),
            Component::Normal(n) => {
                d = Dir::open(&d.path(n.to_str().context("invalid directory")?), false)?
            }
            _ => bail!("invalid update path"),
        }
    }
    let m = d.0.metadata()?;
    ensure!(
        m.uid() == uid() && m.mode() & 0o022 == 0,
        "unsafe update directory"
    );
    Ok(d)
}
fn private_file(path: &Path, create: bool) -> Result<File> {
    let f = OpenOptions::new()
        .read(true)
        .write(create)
        .create(create)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(path)?;
    let m = f.metadata()?;
    ensure!(
        m.is_file() && m.uid() == uid() && m.nlink() == 1 && m.mode() & 0o077 == 0,
        "unsafe update file"
    );
    Ok(f)
}
fn owned_executable(path: &Path) -> Result<File> {
    let f = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(path)?;
    let m = f.metadata()?;
    ensure!(
        m.is_file()
            && m.uid() == uid()
            && m.nlink() == 1
            && m.mode() & 0o022 == 0
            && m.mode() & 0o100 != 0,
        "unsafe installed executable"
    );
    Ok(f)
}
fn new_file(path: &Path) -> Result<File> {
    Ok(OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)?)
}
struct Lock(File);
impl Drop for Lock {
    fn drop(&mut self) {
        unsafe {
            libc::flock(self.0.as_raw_fd(), libc::LOCK_UN);
        }
    }
}
fn lock(dir: &Dir, name: &str) -> Result<Lock> {
    let f = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(dir.path(name))?;
    let m = f.metadata()?;
    // Existing CLI-created locks may be 0644; only ownership/write access matters.
    ensure!(
        m.is_file() && m.uid() == uid() && m.nlink() == 1 && m.mode() & 0o022 == 0,
        "unsafe maintenance lock"
    );
    ensure!(
        unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0,
        "another cx maintenance operation is active"
    );
    Ok(Lock(f))
}
#[derive(Debug)]
struct Stage {
    dir: Dir,
    parent: Dir,
    name: String,
}
impl Drop for Stage {
    fn drop(&mut self) {
        // Never scan/delete arbitrary interrupted stages or files created by another process.
        for n in ["archive", "cx", "metadata", "cache.tmp"] {
            let _ = fs::remove_file(self.dir.path(n));
        }
        let p = self.parent.path(&self.name);
        if let Ok(m) = fs::symlink_metadata(&p) {
            if m.dev() == self.dir.0.metadata().map(|v| v.dev()).unwrap_or(0)
                && m.ino() == self.dir.0.metadata().map(|v| v.ino()).unwrap_or(0)
            {
                let _ = fs::remove_dir(p);
            }
        }
    }
}
fn stage(parent: Dir) -> Result<Stage> {
    for i in 0..32 {
        let name = format!(
            "stage-{}-{}-{i}",
            std::process::id(),
            SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
        );
        let p = parent.path(&name);
        match fs::DirBuilder::new().mode(0o700).create(&p) {
            Ok(()) => {
                return Ok(Stage {
                    dir: parent.child(&name, false)?,
                    parent,
                    name,
                });
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => (),
            Err(e) => return Err(e.into()),
        }
    }
    bail!("cannot allocate private update stage")
}
fn paths() -> Result<(PathBuf, PathBuf)> {
    let home = PathBuf::from(std::env::var_os("HOME").context("HOME unavailable")?);
    ensure!(home.is_absolute(), "HOME must be absolute");
    let state = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".local/state"))
        .join("cx");
    ensure!(state.is_absolute(), "state path must be absolute");
    Ok((home, state))
}
fn state_dir(home: &Path, state: &Path) -> Result<Dir> {
    // Existing XDG state paths are accepted; default parents can be created privately.
    if state == home.join(".local/state/cx") {
        absolute_dir(home)?
            .child(".local", true)?
            .child("state", true)?
            .child("cx", true)
    } else {
        absolute_dir(state.parent().context("invalid state path")?)?.child("cx", true)
    }
}
pub fn installed_target() -> Result<PathBuf> {
    Ok(paths()?.0.join(".local/bin/cx"))
}
fn numeric(v: &str) -> Result<[u64; 3]> {
    let parts: Vec<_> = v.split('.').collect();
    ensure!(parts.len() == 3 && v.len() <= 62, "invalid stable version");
    let mut result = [0; 3];
    for (i, p) in parts.iter().enumerate() {
        ensure!(
            !p.is_empty()
                && p.bytes().all(|b| b.is_ascii_digit())
                && (p.len() == 1 || !p.starts_with('0')),
            "invalid stable version"
        );
        result[i] = p.parse()?;
    }
    Ok(result)
}
#[derive(Deserialize)]
struct Release {
    tag_name: String,
    draft: bool,
    prerelease: bool,
    assets: Vec<Asset>,
}
#[derive(Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
    size: u64,
}
fn release(bytes: &[u8], current: &str) -> Result<Option<(String, String)>> {
    let r: Release = serde_json::from_slice(bytes).context("invalid release metadata")?;
    ensure!(!r.draft && !r.prerelease, "release is not stable");
    let version = r
        .tag_name
        .strip_prefix('v')
        .context("invalid release tag")?;
    let candidate = numeric(version)?;
    if candidate <= numeric(current)? {
        return Ok(None);
    }
    let name = format!("cx-{}-linux-x86_64.tar.gz", r.tag_name);
    let url = format!(
        "https://github.com/{REPO}/releases/download/{}/{name}",
        r.tag_name
    );
    let assets: Vec<_> = r.assets.iter().filter(|a| a.name == name).collect();
    ensure!(
        assets.len() == 1
            && assets[0].browser_download_url == url
            && assets[0].size > 0
            && assets[0].size <= ARCHIVE_LIMIT,
        "expected release artifact unavailable"
    );
    Ok(Some((version.to_owned(), url)))
}
#[derive(Debug)]
struct Output {
    code: i32,
    bytes: Vec<u8>,
}
#[derive(Clone, Copy, Debug)]
enum Tool {
    Curl,
    Gh,
    Probe,
}
#[cfg(test)]
trait TestBackend {
    fn run(&self, tool: Tool, args: &[String], executable: Option<&Path>) -> Result<Output>;
}
struct Backend<'a> {
    #[cfg(test)]
    test: Option<&'a dyn TestBackend>,
    lifetime: std::marker::PhantomData<&'a ()>,
}
impl Backend<'_> {
    fn system() -> Self {
        Self {
            #[cfg(test)]
            test: None,
            lifetime: std::marker::PhantomData,
        }
    }
    #[cfg(test)]
    fn fixture(test: &dyn TestBackend) -> Backend<'_> {
        Backend {
            test: Some(test),
            lifetime: std::marker::PhantomData,
        }
    }

    fn run(&self, tool: Tool, args: &[String], executable: Option<&Path>) -> Result<Output> {
        #[cfg(test)]
        if let Some(test) = self.test {
            return test.run(tool, args, executable);
        }
        let (program, seconds, cap) = match tool {
            Tool::Curl => (Path::new("/usr/bin/curl"), 35, 256 * 1024),
            Tool::Gh => (Path::new("/usr/bin/gh"), 60, 64 * 1024),
            Tool::Probe => (executable.context("missing executable")?, 3, 1024),
        };
        bounded(program, args, Duration::from_secs(seconds), cap)
    }
}
// Drain a nonblocking pipe in the controlling thread: no detached reader/process remains.
fn bounded(program: &Path, args: &[String], timeout: Duration, cap: usize) -> Result<Output> {
    let mut c = Command::new(program);
    c.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .env("LC_ALL", "C")
        .env("GH_HOST", "github.com")
        .env("GH_PROMPT_DISABLED", "1")
        .env_remove("GH_DEBUG")
        .env_remove("GH_FORCE_TTY");
    unsafe {
        c.pre_exec(|| {
            if libc::setpgid(0, 0) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            let limit = libc::rlimit {
                rlim_cur: ARCHIVE_LIMIT,
                rlim_max: ARCHIVE_LIMIT,
            };
            if libc::setrlimit(libc::RLIMIT_FSIZE, &limit) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = c.spawn().context("update tool unavailable")?;
    let pid = child.id() as i32;
    let mut pipe = child.stdout.take().context("update output unavailable")?;
    let fd = pipe.as_raw_fd();
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    let outcome = (|| {
        ensure!(
            flags >= 0 && unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } >= 0,
            "update output unavailable"
        );
        let deadline = Instant::now() + timeout;
        let mut bytes = Vec::new();
        loop {
            let mut buf = [0; 4096];
            loop {
                match pipe.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        ensure!(bytes.len() + n <= cap, "update output limit exceeded");
                        bytes.extend_from_slice(&buf[..n]);
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(e) => return Err(e.into()),
                }
            }
            if let Some(status) = child.try_wait()? {
                loop {
                    match pipe.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => {
                            ensure!(bytes.len() + n <= cap, "update output limit exceeded");
                            bytes.extend_from_slice(&buf[..n]);
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                        Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                        Err(e) => return Err(e.into()),
                    }
                }
                return Ok(Output {
                    code: status.code().unwrap_or(-1),
                    bytes,
                });
            }
            ensure!(Instant::now() < deadline, "update tool timed out");
            std::thread::sleep(Duration::from_millis(10));
        }
    })();
    unsafe {
        libc::kill(-pid, libc::SIGKILL);
    }
    let _ = child.wait();
    outcome
}
fn curl_args(url: &str) -> Vec<String> {
    [
        "--disable",
        "--silent",
        "--proto",
        "=https",
        "--proto-redir",
        "=https",
        "--connect-timeout",
        "5",
        "--max-time",
        "30",
        "--max-redirs",
        "3",
        "--user-agent",
        "cx-update",
        "--write-out",
        "\n%{http_code}",
        url,
    ]
    .iter()
    .map(|v| v.to_string())
    .collect()
}
fn http(out: Output) -> Result<(u16, Vec<u8>)> {
    ensure!(out.code == 0, "offline");
    let split = out
        .bytes
        .iter()
        .rposition(|b| *b == b'\n')
        .context("invalid HTTP response")?;
    let status = std::str::from_utf8(&out.bytes[split + 1..])?.parse()?;
    Ok((status, out.bytes[..split].to_vec()))
}
#[derive(Serialize, Deserialize)]
struct Cache {
    at: u64,
    failed: bool,
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn read_limited(mut f: File, limit: u64) -> Result<Vec<u8>> {
    ensure!(f.metadata()?.len() <= limit, "update file too large");
    let mut bytes = Vec::new();
    Read::by_ref(&mut f)
        .take(limit + 1)
        .read_to_end(&mut bytes)?;
    ensure!(bytes.len() as u64 <= limit, "update file too large");
    Ok(bytes)
}
pub fn check(force: bool) -> Result<CheckOutcome> {
    if !cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        return Ok(CheckOutcome::Unavailable(
            "Auto-update supports Linux x86_64".into(),
        ));
    }
    let (home, state) = paths()?;
    check_with(
        force,
        &home,
        &state,
        env!("CARGO_PKG_VERSION"),
        &std::env::current_exe()?,
        &Backend::system(),
    )
}
fn check_with(
    force: bool,
    home: &Path,
    state: &Path,
    current: &str,
    source: &Path,
    backend: &Backend,
) -> Result<CheckOutcome> {
    let root = state_dir(home, state)?;
    let _check = match lock(&root, "update-check.lock") {
        Ok(v) => v,
        Err(_) => return Ok(CheckOutcome::Skipped),
    };
    if !force {
        if let Ok(bytes) =
            private_file(&root.path("update-cache.json"), false).and_then(|f| read_limited(f, 1024))
        {
            if let Ok(c) = serde_json::from_slice::<Cache>(&bytes) {
                if c.at <= now() && now() - c.at < if c.failed { 300 } else { 3600 } {
                    return Ok(CheckOutcome::Skipped);
                }
            }
        }
    }
    let result = prepare(home, state, current, source, &root, backend);
    let failed = !matches!(result, Ok(CheckOutcome::Current | CheckOutcome::Ready(_)));
    // In-place cache under check lock avoids replacement of any unowned temporary path.
    let mut cache = private_file(&root.path("update-cache.json"), true)?;
    cache.set_len(0)?;
    cache.write_all(&serde_json::to_vec(&Cache { at: now(), failed })?)?;
    cache.sync_all()?;
    match result {
        Ok(v) => Ok(v),
        Err(_) => Ok(CheckOutcome::Unavailable(
            "Release validation failed; keeping current cx".into(),
        )),
    }
}
fn prepare(
    home: &Path,
    state: &Path,
    current: &str,
    source: &Path,
    root: &Dir,
    backend: &Backend,
) -> Result<CheckOutcome> {
    let out = match backend.run(Tool::Curl, &curl_args(API), None) {
        Ok(v) => v,
        Err(e)
            if e.downcast_ref::<std::io::Error>()
                .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound) =>
        {
            return Ok(CheckOutcome::Unavailable(
                "Install system curl to check releases".into(),
            ))
        }
        Err(_) => return Ok(CheckOutcome::Offline),
    };
    let (status, bytes) = match http(out) {
        Ok(v) => v,
        Err(_) => return Ok(CheckOutcome::Offline),
    };
    if status == 404 {
        return Ok(CheckOutcome::Unavailable(
            "No public tagged release is published".into(),
        ));
    }
    if status != 200 {
        return Ok(CheckOutcome::Offline);
    }
    let Some((version, url)) = release(&bytes, current)? else {
        return Ok(CheckOutcome::Current);
    };
    let s = stage(root.child("update", true)?)?;
    let archive = new_file(&s.dir.path("archive"))?;
    let archive_path = PathBuf::from(format!(
        "/proc/{}/fd/{}",
        std::process::id(),
        archive.as_raw_fd()
    ));
    let mut args = curl_args(&url);
    args.extend([
        "--location".into(),
        "--max-filesize".into(),
        ARCHIVE_LIMIT.to_string(),
        "--output".into(),
        archive_path.to_string_lossy().into_owned(),
    ]);
    let (status, _) = match backend.run(Tool::Curl, &args, None).and_then(http) {
        Ok(v) => v,
        Err(_) => return Ok(CheckOutcome::Offline),
    };
    if status != 200 {
        return Ok(CheckOutcome::Unavailable(
            "Release artifact unavailable".into(),
        ));
    }
    ensure!(
        archive.metadata()?.len() > 0 && archive.metadata()?.len() <= ARCHIVE_LIMIT,
        "invalid archive size"
    );
    archive.sync_all()?;
    let args = vec![
        "attestation".into(),
        "verify".into(),
        archive_path.to_string_lossy().into_owned(),
        "--repo".into(),
        REPO.into(),
        "--signer-workflow".into(),
        WORKFLOW.into(),
        "--source-ref".into(),
        format!("refs/tags/v{version}"),
        "--deny-self-hosted-runners".into(),
        "--hostname".into(),
        "github.com".into(),
        "--limit".into(),
        "10".into(),
    ];
    let archive_digest = digest(&archive)?;
    let verified = match backend.run(Tool::Gh, &args, None) {
        Ok(v) => v,
        Err(_) => {
            return Ok(CheckOutcome::Unavailable(
                "Install gh with attestation verify support and retry".into(),
            ))
        }
    };
    ensure!(verified.code == 0, "release provenance rejected");
    ensure!(
        digest(&archive)? == archive_digest,
        "verified archive changed"
    );
    // Hash before/after verification and extraction prevents changing the verified subject.
    // Archive is never executed. Extract only after the exact repo/workflow/tag policy passes.
    let archive_copy = archive.try_clone()?;
    let binary = extract(archive, &s.dir)?;
    ensure!(
        self::digest(&archive_copy)? == archive_digest,
        "verified archive changed during extraction"
    );
    let m = binary.metadata()?;
    let digest = digest(&binary)?;
    binary.set_permissions(fs::Permissions::from_mode(0o700))?;
    let executable = PathBuf::from(format!(
        "/proc/{}/fd/{}",
        std::process::id(),
        binary.as_raw_fd()
    ));
    let probe = backend.run(Tool::Probe, &["--version".into()], Some(&executable))?;
    ensure!(
        probe.code == 0 && probe.bytes == format!("cx {version}\n").as_bytes(),
        "staged version mismatch"
    );
    ensure!(
        digest == self::digest(&binary)?,
        "staged executable changed"
    );
    let source = fs::metadata(source)?;
    let metadata = Metadata {
        version: version.clone(),
        digest,
        binary_dev: m.dev(),
        binary_ino: m.ino(),
        source_dev: source.dev(),
        source_ino: source.ino(),
    };
    let mut record = new_file(&s.dir.path("metadata"))?;
    record.write_all(&serde_json::to_vec(&metadata)?)?;
    record.sync_all()?;
    binary.sync_all()?;
    s.dir.0.sync_all()?;
    Ok(CheckOutcome::Ready(UpdatePlan {
        version,
        stage: s,
        metadata,
        home: home.into(),
        state: state.into(),
    }))
}
fn digest(f: &File) -> Result<String> {
    use std::os::unix::fs::FileExt;
    let mut hash = Sha256::new();
    let mut offset = 0;
    let mut buf = [0; 32768];
    loop {
        let n = f.read_at(&mut buf, offset)?;
        if n == 0 {
            break;
        }
        offset += n as u64;
        ensure!(offset <= BINARY_LIMIT, "file too large");
        hash.update(&buf[..n]);
    }
    Ok(format!("{:x}", hash.finalize()))
}
fn extract(mut compressed: File, dir: &Dir) -> Result<File> {
    use std::io::{Seek, SeekFrom};
    compressed.seek(SeekFrom::Start(0))?;
    let decoder = flate2::bufread::GzDecoder::new(std::io::BufReader::new(compressed));
    let mut archive = tar::Archive::new(decoder.take(TAR_LIMIT + 1));
    let binary = new_file(&dir.path("cx"))?;
    let mut writer = binary.try_clone()?;
    let mut count = 0;
    let mut expected_tar_size = 0;
    for entry in archive.entries()?.raw(true) {
        let mut entry = entry?;
        count += 1;
        ensure!(
            count == 1
                && entry.header().entry_type().is_file()
                && entry.path_bytes().as_ref() == b"cx",
            "unsafe archive member"
        );
        let size = entry.size();
        ensure!(size > 0 && size <= BINARY_LIMIT, "invalid binary size");
        expected_tar_size = 512 + size.div_ceil(512) * 512 + 1024;
        ensure!(
            entry.header().link_name()?.is_none(),
            "archive link rejected"
        );
        let copied = std::io::copy(&mut entry, &mut writer)?;
        ensure!(copied == size, "truncated archive");
    }
    ensure!(count == 1, "empty archive");
    // Consume bounded tail: validate gzip CRC/truncation and reject concatenated/trailing data.
    let mut bounded = archive.into_inner();
    let mut tail = Vec::new();
    bounded.read_to_end(&mut tail)?;
    ensure!(
        bounded.limit() > 0
            && TAR_LIMIT + 1 - bounded.limit() >= expected_tar_size
            && (TAR_LIMIT + 1 - bounded.limit()) % 512 == 0
            && tail.iter().all(|b| *b == 0),
        "archive expansion limit or extra data"
    );
    let decoder = bounded.into_inner();
    let mut compressed = decoder.into_inner();
    let mut extra = [0; 1];
    ensure!(
        compressed.read(&mut extra)? == 0,
        "trailing compressed data"
    );
    writer.sync_all()?;
    let original = binary.metadata()?;
    let expected_digest = digest(&binary)?;
    drop(writer);
    drop(binary);
    let reopened = private_file(&dir.path("cx"), false)?;
    let m = reopened.metadata()?;
    ensure!(
        m.dev() == original.dev()
            && m.ino() == original.ino()
            && digest(&reopened)? == expected_digest,
        "extracted executable changed"
    );
    Ok(reopened)
}
pub fn install(plan: &UpdatePlan) -> Result<PathBuf> {
    install_with(plan, &std::env::current_exe()?)
}
fn install_with(plan: &UpdatePlan, source: &Path) -> Result<PathBuf> {
    ensure!(uid() != 0, "auto-update refuses privileged installation");
    ensure!(
        plan.version == plan.metadata.version
            && numeric(&plan.version)? > numeric(env!("CARGO_PKG_VERSION"))?,
        "invalid update plan"
    );
    let root = state_dir(&plan.home, &plan.state)?;
    let _maintenance = lock(&root, "maintenance.lock")?;
    let bins = absolute_dir(&plan.home)?
        .child(".local", false)?
        .child("bin", false)?;
    let target = bins.path("cx");
    let old = owned_executable(&target)?;
    let old_m = old.metadata()?;
    ensure!(
        old_m.len() > 0 && old_m.len() <= BINARY_LIMIT,
        "installed executable size invalid"
    );
    let running = fs::metadata(source)?;
    ensure!(
        old_m.dev() == running.dev()
            && old_m.ino() == running.ino()
            && running.dev() == plan.metadata.source_dev
            && running.ino() == plan.metadata.source_ino,
        "installed target differs from running cx"
    );
    ensure!(
        old_m.mode() & 0o200 != 0
            && unsafe {
                libc::faccessat(
                    bins.0.as_raw_fd(),
                    c".".as_ptr(),
                    libc::W_OK,
                    libc::AT_EACCESS,
                )
            } == 0,
        "installed target is not writable"
    );
    let stage_m = fs::symlink_metadata(plan.stage.parent.path(&plan.stage.name))?;
    let held_m = plan.stage.dir.0.metadata()?;
    ensure!(
        stage_m.is_dir()
            && stage_m.uid() == uid()
            && stage_m.mode() & 0o077 == 0
            && stage_m.dev() == held_m.dev()
            && stage_m.ino() == held_m.ino(),
        "private stage replaced"
    );
    let metadata: Metadata = serde_json::from_slice(&read_limited(
        private_file(&plan.stage.dir.path("metadata"), false)?,
        2048,
    )?)?;
    ensure!(metadata == plan.metadata, "staged metadata changed");
    let binary = private_file(&plan.stage.dir.path("cx"), false)?;
    let m = binary.metadata()?;
    ensure!(
        m.dev() == metadata.binary_dev
            && m.ino() == metadata.binary_ino
            && m.mode() & 0o700 == 0o700
            && digest(&binary)? == metadata.digest,
        "staged executable changed"
    );
    // Copy from held descriptor into the destination filesystem, then atomically rename.
    let scratch = stage(Dir(bins.0.try_clone()?))?;
    let mut replacement = new_file(&scratch.dir.path("cx"))?;
    let reader = binary.try_clone()?;
    std::io::copy(&mut reader.take(BINARY_LIMIT + 1), &mut replacement)?;
    ensure!(
        digest(&replacement)? == metadata.digest,
        "replacement digest mismatch"
    );
    replacement.set_permissions(fs::Permissions::from_mode(0o700))?;
    replacement.sync_all()?;
    // Preserve rollback before replacing. Never overwrite an existing rollback file.
    let rollback_name = format!("cx.rollback-{}", plan.metadata.source_ino);
    let rollback = bins.path(&rollback_name);
    let mut backup = new_file(&rollback)?;
    let backup_result = (|| -> Result<()> {
        let reader = old.try_clone()?;
        ensure!(
            std::io::copy(&mut reader.take(BINARY_LIMIT + 1), &mut backup)? == old_m.len(),
            "rollback size changed"
        );
        backup.set_permissions(fs::Permissions::from_mode(0o700))?;
        backup.sync_all()?;
        bins.0.sync_all()?; // Make the rollback directory entry durable before replacement.
        let current = owned_executable(&target)?.metadata()?;
        ensure!(
            current.dev() == old_m.dev() && current.ino() == old_m.ino(),
            "installed target changed"
        );
        fs::rename(scratch.dir.path("cx"), &target)?;
        Ok(())
    })();
    if backup_result.is_err() {
        let _ = fs::remove_file(&rollback);
    }
    backup_result?;
    if bins.0.sync_all().is_err() {
        // A durability failure must not report failure while leaving the new binary installed.
        fs::rename(&rollback, &target)
            .context("installation durability failed; rollback restore failed")?;
        let _ = bins.0.sync_all();
        bail!("installation durability failed; previous executable restored");
    }
    Ok(plan.home.join(".local/bin/cx"))
}

#[cfg(test)]
#[path = "../tests/update_fixtures/mod.rs"]
mod tests;
