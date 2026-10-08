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
const RELEASE_KEY: &str = include_str!("../release-key.pem");
const ARCH: &str = std::env::consts::ARCH;
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
pub struct Lock(File);
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
pub fn maintenance_lock() -> Result<Lock> {
    let (home, state) = paths()?;
    let root = state_dir(&home, &state)?;
    lock(&root, "maintenance.lock")
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
        for n in [
            "archive",
            "signature",
            "release-key.pem",
            "cx",
            "metadata",
            "cache.tmp",
        ] {
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
    release_for_arch(bytes, Some(current), ARCH)
}
fn supported_arch(arch: &str) -> Result<()> {
    ensure!(
        matches!(arch, "x86_64" | "aarch64"),
        "Enrollment supports Linux x86_64 and aarch64"
    );
    Ok(())
}
fn release_for_arch(
    bytes: &[u8],
    current: Option<&str>,
    arch: &str,
) -> Result<Option<(String, String)>> {
    supported_arch(arch)?;
    let r: Release = serde_json::from_slice(bytes).context("invalid release metadata")?;
    ensure!(!r.draft && !r.prerelease, "release is not stable");
    let version = r
        .tag_name
        .strip_prefix('v')
        .context("invalid release tag")?;
    let candidate = numeric(version)?;
    if let Some(current) = current {
        if candidate <= numeric(current)? {
            return Ok(None);
        }
    }
    let name = format!("cx-{}-linux-{arch}.tar.gz", r.tag_name);
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
    OpenSsl,
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
            Tool::OpenSsl => (Path::new("/usr/bin/openssl"), 10, 4096),
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
        .env_remove("OPENSSL_CONF")
        .env_remove("OPENSSL_MODULES");
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
            let mut status: libc::siginfo_t = unsafe { std::mem::zeroed() };
            let observed = unsafe {
                libc::waitid(
                    libc::P_PID,
                    child.id(),
                    &mut status,
                    libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
                )
            };
            if observed != 0 {
                return Err(std::io::Error::last_os_error().into());
            }
            if unsafe { status.si_pid() } != 0 {
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
                    code: if status.si_code == libc::CLD_EXITED {
                        unsafe { status.si_status() }
                    } else {
                        -1
                    },
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
enum ReleaseLookup {
    Found(String, String),
    Current,
    Offline,
    Unavailable(String),
}
fn latest_release(arch: &str, current: Option<&str>, backend: &Backend) -> Result<ReleaseLookup> {
    supported_arch(arch)?;
    let out = match backend.run(Tool::Curl, &curl_args(API), None) {
        Ok(out) => out,
        Err(e)
            if e.downcast_ref::<std::io::Error>()
                .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound) =>
        {
            return Ok(ReleaseLookup::Unavailable(
                "Install system curl to check releases".into(),
            ))
        }
        Err(_) => return Ok(ReleaseLookup::Offline),
    };
    let (status, bytes) = match http(out) {
        Ok(value) => value,
        Err(_) => return Ok(ReleaseLookup::Offline),
    };
    if matches!(status, 403 | 429) {
        // Shared public egress often exhausts GitHub's unauthenticated REST quota.
        // Only an official stable-tag redirect may select the signed artifact.
        let page = format!("https://github.com/{REPO}/releases/latest");
        let mut args = curl_args(&page);
        let at = args.iter().position(|a| a == "--write-out").unwrap() + 1;
        args[at] = "\n%{http_code}\n%{url_effective}".into();
        args.extend([
            "--location".into(),
            "--head".into(),
            "--output".into(),
            "/dev/null".into(),
        ]);
        let response = backend.run(Tool::Curl, &args, None);
        let version = response.ok().and_then(|out| release_redirect(&out).ok());
        let Some(version) = version else {
            return Ok(ReleaseLookup::Unavailable(format!("GitHub API rejected the check (HTTP {status}); official release-page lookup unavailable. Keeping current cx")));
        };
        if let Some(old) = current {
            if numeric(&version)? <= numeric(old)? {
                return Ok(ReleaseLookup::Current);
            }
        }
        let url = format!("https://github.com/{REPO}/releases/download/v{version}/cx-v{version}-linux-{arch}.tar.gz");
        return Ok(ReleaseLookup::Found(version, url));
    }
    if status == 404 {
        return Ok(ReleaseLookup::Unavailable(
            "No public tagged release is published".into(),
        ));
    }
    if status != 200 {
        return Ok(ReleaseLookup::Unavailable(format!(
            "GitHub release metadata returned HTTP {status}; keeping current cx"
        )));
    }
    Ok(match release_for_arch(&bytes, current, arch)? {
        Some((version, url)) => ReleaseLookup::Found(version, url),
        None => ReleaseLookup::Current,
    })
}
fn release_redirect(out: &Output) -> Result<String> {
    ensure!(
        out.code == 0 && out.bytes.len() <= 4096,
        "release-page request failed"
    );
    let text = std::str::from_utf8(&out.bytes)?;
    let fields: Vec<_> = text.lines().filter(|line| !line.is_empty()).collect();
    ensure!(
        fields.len() == 2 && fields[0] == "200",
        "release-page request rejected"
    );
    let prefix = format!("https://github.com/{REPO}/releases/tag/v");
    let version = fields[1]
        .strip_prefix(&prefix)
        .context("release redirect left the official stable tag path")?;
    numeric(version)?;
    Ok(version.into())
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
    if !cfg!(all(
        target_os = "linux",
        any(target_arch = "x86_64", target_arch = "aarch64")
    )) {
        return Ok(CheckOutcome::Unavailable(
            "Auto-update supports Linux x86_64 and aarch64".into(),
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
    let (version, url) = match latest_release(ARCH, Some(current), backend)? {
        ReleaseLookup::Found(version, url) => (version, url),
        ReleaseLookup::Current => return Ok(CheckOutcome::Current),
        ReleaseLookup::Offline => return Ok(CheckOutcome::Offline),
        ReleaseLookup::Unavailable(message) => return Ok(CheckOutcome::Unavailable(message)),
    };
    let (s, binary) = match download_verified(root.child("update", true)?, &url, backend)? {
        ArtifactOutcome::Offline => return Ok(CheckOutcome::Offline),
        ArtifactOutcome::Unavailable(message) => {
            return Ok(CheckOutcome::Unavailable(message.into()))
        }
        ArtifactOutcome::Ready(stage, binary) => (stage, binary),
    };
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
enum ArtifactOutcome {
    Offline,
    Unavailable(&'static str),
    Ready(Stage, File),
}
// Both updater and enrollment share the exact download, pinned signature and bounded extraction path.
fn download_verified(parent: Dir, url: &str, backend: &Backend) -> Result<ArtifactOutcome> {
    let s = stage(parent)?;
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
        Err(_) => return Ok(ArtifactOutcome::Offline),
    };
    if status != 200 {
        return Ok(ArtifactOutcome::Unavailable("Release artifact unavailable"));
    }
    ensure!(
        archive.metadata()?.len() > 0 && archive.metadata()?.len() <= ARCHIVE_LIMIT,
        "invalid archive size"
    );
    archive.sync_all()?;
    // Detached signatures are verified against the public key compiled into this binary.
    let bundle = new_file(&s.dir.path("signature"))?;
    let bundle_path = PathBuf::from(format!(
        "/proc/{}/fd/{}",
        std::process::id(),
        bundle.as_raw_fd()
    ));
    let mut bundle_args = curl_args(&format!("{url}.sig"));
    bundle_args.extend([
        "--location".into(),
        "--max-filesize".into(),
        "8192".into(),
        "--output".into(),
        bundle_path.to_string_lossy().into_owned(),
    ]);
    let (status, _) = match backend.run(Tool::Curl, &bundle_args, None).and_then(http) {
        Ok(value) => value,
        Err(_) => return Ok(ArtifactOutcome::Offline),
    };
    if status != 200 {
        return Ok(ArtifactOutcome::Unavailable(
            "Release signature unavailable; keeping current cx",
        ));
    }
    ensure!(
        bundle.metadata()?.len() > 0 && bundle.metadata()?.len() <= 8192,
        "invalid signature size"
    );
    let mut key = new_file(&s.dir.path("release-key.pem"))?;
    key.write_all(RELEASE_KEY.as_bytes())?;
    key.sync_all()?;
    let key_path = PathBuf::from(format!(
        "/proc/{}/fd/{}",
        std::process::id(),
        key.as_raw_fd()
    ));
    let args = vec![
        "dgst".into(),
        "-sha256".into(),
        "-verify".into(),
        key_path.to_string_lossy().into_owned(),
        "-signature".into(),
        bundle_path.to_string_lossy().into_owned(),
        archive_path.to_string_lossy().into_owned(),
    ];
    let archive_digest = digest(&archive)?;
    let verified = match backend.run(Tool::OpenSsl, &args, None) {
        Ok(v) => v,
        Err(_) => return Ok(ArtifactOutcome::Unavailable("Install OpenSSL and retry")),
    };
    ensure!(verified.code == 0, "release signature rejected");
    ensure!(
        digest(&archive)? == archive_digest,
        "verified archive changed"
    );
    // Hash before/after verification and extraction prevents changing the verified subject.
    // Archive is never executed. Extract only after the exact pinned public key signature passes.
    let archive_copy = archive.try_clone()?;
    let binary = extract(archive, &s.dir)?;
    ensure!(
        self::digest(&archive_copy)? == archive_digest,
        "verified archive changed during extraction"
    );
    Ok(ArtifactOutcome::Ready(s, binary))
}

/// A verified enrollment artifact. Keep this handle alive until remote transfer finishes.
/// Its local file has no execute permission and is never probed or installed by this API.
#[derive(Debug)]
pub struct EnrollmentBinary {
    pub version: String,
    pub arch: String,
    binary: File,
    _stage: Stage,
}
impl EnrollmentBinary {
    /// An FD-pinned path usable by a child transfer process while this handle is alive.
    pub fn path(&self) -> PathBuf {
        PathBuf::from(format!(
            "/proc/{}/fd/{}",
            std::process::id(),
            self.binary.as_raw_fd()
        ))
    }
}
/// Download the latest official stable Linux artifact for enrollment, including foreign architectures.
/// This bypasses local update version/cache checks without changing their behavior.
pub fn obtain_enrollment_binary(arch: &str) -> Result<EnrollmentBinary> {
    supported_arch(arch)?;
    let (home, state) = paths()?;
    obtain_enrollment_with(arch, &home, &state, &Backend::system())
}
fn obtain_enrollment_with(
    arch: &str,
    home: &Path,
    state: &Path,
    backend: &Backend,
) -> Result<EnrollmentBinary> {
    obtain_remote_with(arch, None, home, state, backend)?
        .context("Official stable release unavailable")
}
/// No archive is downloaded when the authenticated remote version is already current.
pub fn obtain_remote_update_binary(arch: &str, current: &str) -> Result<Option<EnrollmentBinary>> {
    let (home, state) = paths()?;
    obtain_remote_with(arch, Some(current), &home, &state, &Backend::system())
}
fn obtain_remote_with(
    arch: &str,
    current: Option<&str>,
    home: &Path,
    state: &Path,
    backend: &Backend,
) -> Result<Option<EnrollmentBinary>> {
    supported_arch(arch)?;
    let (version, url) = match latest_release(arch, current, backend)? {
        ReleaseLookup::Found(version, url) => (version, url),
        ReleaseLookup::Current => return Ok(None),
        ReleaseLookup::Offline => bail!("Official stable release request failed"),
        ReleaseLookup::Unavailable(message) => bail!("{message}"),
    };
    let root = state_dir(home, state)?;
    let (stage, binary) = match download_verified(root.child("enrollment", true)?, &url, backend)? {
        ArtifactOutcome::Offline => bail!("Enrollment artifact download unavailable"),
        ArtifactOutcome::Unavailable(message) => bail!("{message}"),
        ArtifactOutcome::Ready(stage, binary) => (stage, binary),
    };
    ensure!(
        binary.metadata()?.mode() & 0o111 == 0,
        "Enrollment artifact must not execute locally"
    );
    binary.sync_all()?;
    Ok(Some(EnrollmentBinary {
        version,
        arch: arch.into(),
        binary,
        _stage: stage,
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
fn new_rollback(bins: &Dir, source_ino: u64) -> Result<(PathBuf, File)> {
    let base = format!("cx.rollback-{source_ino}");
    for attempt in 0..32 {
        let name = if attempt == 0 {
            base.clone()
        } else {
            format!(
                "{base}-{}-{}-{attempt}",
                std::process::id(),
                SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
            )
        };
        let path = bins.path(&name);
        match new_file(&path) {
            Ok(file) => return Ok((path, file)),
            Err(error)
                if error
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|e| e.kind() == std::io::ErrorKind::AlreadyExists) =>
            {
                ()
            }
            Err(error) => return Err(error),
        }
    }
    bail!("cannot allocate private rollback file")
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
    let (rollback, mut backup) = new_rollback(&bins, plan.metadata.source_ino)?;
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

#[cfg(test)]
mod enrollment_tests {
    use super::*;
    use std::cell::RefCell;

    struct SignedFixture {
        metadata: Vec<u8>,
        archive: Vec<u8>,
        verification: u8,
        synthetic_signature: Option<(PathBuf, Vec<u8>)>,
        calls: RefCell<Vec<&'static str>>,
    }
    fn archive(path: &str, kind: tar::EntryType) -> Vec<u8> {
        let gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        let mut tar = tar::Builder::new(gzip);
        let bytes = b"foreign fixture: never execute";
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(kind);
        header.set_size(bytes.len() as u64);
        header.set_mode(0o755);
        if kind.is_symlink() {
            header.set_link_name("elsewhere").unwrap();
        }
        header.set_cksum();
        tar.append_data(&mut header, path, bytes.as_slice())
            .unwrap();
        tar.into_inner().unwrap().finish().unwrap()
    }
    impl SignedFixture {
        fn new() -> Self {
            let assets = ["x86_64", "aarch64"].map(|arch| serde_json::json!({
                "name":format!("cx-v0.1.0-linux-{arch}.tar.gz"), "size":100,
                "browser_download_url":format!("https://github.com/{REPO}/releases/download/v0.1.0/cx-v0.1.0-linux-{arch}.tar.gz")
            }));
            Self {
                metadata: serde_json::to_vec(&serde_json::json!({
                    "tag_name":"v0.1.0", "draft":false, "prerelease":false, "assets":assets
                }))
                .unwrap(),
                archive: archive("cx", tar::EntryType::Regular),
                verification: 0,
                synthetic_signature: None,
                calls: RefCell::new(Vec::new()),
            }
        }
    }
    impl TestBackend for SignedFixture {
        fn run(&self, tool: Tool, args: &[String], _: Option<&Path>) -> Result<Output> {
            match tool {
                Tool::Probe => panic!("Enrollment must never execute an artifact"),
                Tool::OpenSsl => {
                    self.calls.borrow_mut().push("verify");
                    assert_eq!(&args[..3], &["dgst", "-sha256", "-verify"]);
                    assert_eq!(fs::read_to_string(&args[3])?, RELEASE_KEY);
                    if let Some((test_key, _)) = &self.synthetic_signature {
                        // Test-only tool boundary: production still supplies its compiled trust root.
                        let mut args = args.to_vec();
                        args[3] = test_key.to_string_lossy().into_owned();
                        return Backend::system().run(tool, &args, None);
                    }
                    match self.verification {
                        2 => return Backend::system().run(tool, args, None),
                        3 => fs::write(&args[6], b"changed after verification")?,
                        _ => (),
                    }
                    Ok(Output {
                        code: i32::from(self.verification == 1),
                        bytes: vec![],
                    })
                }
                Tool::Curl => {
                    if let Some(index) = args.iter().position(|s| s == "--output") {
                        let signature = args.iter().any(|s| s.ends_with(".sig"));
                        self.calls.borrow_mut().push(if signature {
                            "signature"
                        } else {
                            "archive"
                        });
                        let signature_bytes = self
                            .synthetic_signature
                            .as_ref()
                            .map(|(_, signature)| signature.as_slice())
                            .unwrap_or(b"synthetic detached signature");
                        fs::write(
                            &args[index + 1],
                            if signature {
                                signature_bytes
                            } else {
                                &self.archive
                            },
                        )?;
                        Ok(Output {
                            code: 0,
                            bytes: b"\n200".to_vec(),
                        })
                    } else {
                        self.calls.borrow_mut().push("metadata");
                        assert!(args.contains(&API.into()));
                        let mut bytes = self.metadata.clone();
                        bytes.extend_from_slice(b"\n200");
                        Ok(Output { code: 0, bytes })
                    }
                }
            }
        }
    }
    fn obtain(home: &Path, fixture: &SignedFixture, arch: &str) -> Result<EnrollmentBinary> {
        obtain_enrollment_with(
            arch,
            home,
            &home.join(".local/state/cx"),
            &Backend::fixture(fixture),
        )
    }
    #[test]
    fn current_remote_checks_never_download_or_execute_artifacts() {
        let home = tempfile::tempdir().unwrap();
        for current in ["0.1.0", "0.2.0"] {
            let fixture = SignedFixture::new();
            assert!(obtain_remote_with(
                "aarch64",
                Some(current),
                home.path(),
                &home.path().join(".local/state/cx"),
                &Backend::fixture(&fixture)
            )
            .unwrap()
            .is_none());
            assert_eq!(*fixture.calls.borrow(), vec!["metadata"]);
        }
    }
    #[test]
    fn latest_both_architectures_are_fd_pinned_nonexecutable_and_cleaned() {
        for arch in ["x86_64", "aarch64"] {
            let home = tempfile::tempdir().unwrap();
            let fixture = SignedFixture::new();
            let binary = obtain(home.path(), &fixture, arch).unwrap();
            assert_eq!(binary.version, "0.1.0");
            assert_eq!(binary.arch, arch);
            let path = binary.path();
            // Keep the artifact inode alive while testing FD closure. Otherwise
            // parallel tests may recycle both the numeric FD and its freed inode.
            let pinned = fs::File::open(&path).unwrap();
            let identity = pinned.metadata().unwrap();
            assert_eq!(fs::read(&path).unwrap(), b"foreign fixture: never execute");
            assert_eq!(fs::metadata(&path).unwrap().mode() & 0o777, 0o600);
            assert_eq!(
                &*fixture.calls.borrow(),
                &["metadata", "archive", "signature", "verify"]
            );
            let stages = home.path().join(".local/state/cx/enrollment");
            assert_eq!(fs::read_dir(&stages).unwrap().count(), 1);
            drop(binary);
            // Parallel tests may reuse the numeric FD immediately; it must no longer identify this artifact.
            assert!(fs::metadata(&path).map_or(true, |m| (m.dev(), m.ino())
                != (identity.dev(), identity.ino())));
            assert_eq!(pinned.metadata().unwrap().nlink(), 0);
            assert_eq!(fs::read_dir(stages).unwrap().count(), 0);
            assert!(!home
                .path()
                .join(".local/state/cx/update-cache.json")
                .exists());
        }
    }
    #[test]
    fn unknown_architecture_rejects_before_network_or_local_state() {
        let home = tempfile::tempdir().unwrap();
        let fixture = SignedFixture::new();
        for arch in ["arm64", "linux-aarch64", "../x86_64", "", "x86_64;false"] {
            assert!(obtain(home.path(), &fixture, arch).is_err());
        }
        assert!(fixture.calls.borrow().is_empty());
        assert!(!home.path().join(".local").exists());
    }
    #[test]
    fn signature_rejection_tampering_and_archive_paths_cleanup() {
        for verification in [1, 2, 3] {
            let home = tempfile::tempdir().unwrap();
            let mut fixture = SignedFixture::new();
            fixture.verification = verification;
            assert!(obtain(home.path(), &fixture, "aarch64").is_err());
            assert_eq!(
                fs::read_dir(home.path().join(".local/state/cx/enrollment"))
                    .unwrap()
                    .count(),
                0
            );
        }
        for (path, kind) in [
            ("nested/cx", tar::EntryType::Regular),
            ("cx", tar::EntryType::Symlink),
        ] {
            let home = tempfile::tempdir().unwrap();
            let mut fixture = SignedFixture::new();
            fixture.archive = archive(path, kind);
            assert!(obtain(home.path(), &fixture, "aarch64").is_err());
            assert_eq!(
                fs::read_dir(home.path().join(".local/state/cx/enrollment"))
                    .unwrap()
                    .count(),
                0
            );
        }
    }
    #[test]
    fn synthetic_signed_archive_uses_real_crypto_without_foreign_execution() {
        let home = tempfile::tempdir().unwrap();
        let keys = tempfile::tempdir().unwrap();
        let private = keys.path().join("private.pem");
        let public = keys.path().join("public.pem");
        let archive = keys.path().join("artifact.tar.gz");
        let signature = keys.path().join("artifact.sig");
        let mut fixture = SignedFixture::new();
        fs::write(&archive, &fixture.archive).unwrap();
        let commands = vec![
            vec![
                "genpkey".into(),
                "-algorithm".into(),
                "RSA".into(),
                "-pkeyopt".into(),
                "rsa_keygen_bits:2048".into(),
                "-out".into(),
                private.to_string_lossy().into_owned(),
            ],
            vec![
                "pkey".into(),
                "-in".into(),
                private.to_string_lossy().into_owned(),
                "-pubout".into(),
                "-out".into(),
                public.to_string_lossy().into_owned(),
            ],
            vec![
                "dgst".into(),
                "-sha256".into(),
                "-sign".into(),
                private.to_string_lossy().into_owned(),
                "-out".into(),
                signature.to_string_lossy().into_owned(),
                archive.to_string_lossy().into_owned(),
            ],
        ];
        for args in commands {
            assert_eq!(
                Backend::system()
                    .run(Tool::OpenSsl, &args, None)
                    .unwrap()
                    .code,
                0
            );
        }
        fixture.synthetic_signature = Some((public, fs::read(signature).unwrap()));
        let artifact = obtain(home.path(), &fixture, "aarch64").unwrap();
        assert_eq!(
            fs::read(artifact.path()).unwrap(),
            b"foreign fixture: never execute"
        );
        drop(artifact);
        fixture.synthetic_signature.as_mut().unwrap().1[0] ^= 1;
        assert!(obtain(home.path(), &fixture, "aarch64").is_err());
    }
    #[test]
    fn metadata_and_state_paths_are_guarded() {
        let home = tempfile::tempdir().unwrap();
        for field in ["name", "browser_download_url", "size"] {
            let mut fixture = SignedFixture::new();
            let mut metadata: serde_json::Value =
                serde_json::from_slice(&fixture.metadata).unwrap();
            metadata["assets"][1][field] = if field == "size" {
                (ARCHIVE_LIMIT + 1).into()
            } else {
                "https://other.example/cx".into()
            };
            fixture.metadata = serde_json::to_vec(&metadata).unwrap();
            assert!(obtain(home.path(), &fixture, "aarch64").is_err());
            assert_eq!(&*fixture.calls.borrow(), &["metadata"]);
        }
        let elsewhere = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(elsewhere.path(), home.path().join(".local")).unwrap();
        let fixture = SignedFixture::new();
        assert!(obtain(home.path(), &fixture, "aarch64").is_err());
        assert_eq!(&*fixture.calls.borrow(), &["metadata"]);
        assert_eq!(fs::read_dir(elsewhere.path()).unwrap().count(), 0);
    }
}
