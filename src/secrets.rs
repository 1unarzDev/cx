//! Secrets: one interface over the OS keyring and an age-encrypted file (docs/secrets.md).
//!
//! * [`Secrets::list`] returns metadata only (never values) from `secrets.json` and never contacts a backend; it
//!   still reads the index from disk on each call.
//! * [`Secrets::get`] returns a [`SecretValue`], which zeroizes on drop and has no `Display` and a redacted `Debug`.
//! * [`Secrets::set`] stores in the OS keyring when one answers within [`KEYRING_TIMEOUT`], else in the age file.
//! * [`Secrets::inject`] puts values into one child's environment, never into argv.
//!
//! Backends: the keyring is the Secret Service over a pure-Rust zbus connection on Linux (no libdbus, so the static
//! musl release keeps working) and the Keychain on macOS. Every keyring call runs on its own thread with a bounded
//! wait; a call that hangs marks the keyring unusable for the rest of the process. The fallback is
//! `$XDG_STATE_HOME/cx/secrets.age`, encrypted to an X25519 identity in `secrets.key` (0600, in the 0700 state
//! dir). That protects like an SSH private key: against other users and against copies of `secrets.age` alone,
//! not against this user's own processes or root.
use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, OnceLock};
use std::time::Duration;
use zeroize::Zeroizing;

/// Largest value accepted (bytes).
pub const VALUE_LIMIT: usize = 16 * 1024;
/// Longest wait for one OS keyring call before falling back.
pub const KEYRING_TIMEOUT: Duration = Duration::from_secs(3);
const MAX_SECRETS: usize = 256;
const STORE_LIMIT: u64 = 8 * 1024 * 1024;
const INDEX_LIMIT: u64 = 512 * 1024;
const KEYRING_SERVICE: &str = "cx";
const INDEX: &str = "secrets.json";
const STORE: &str = "secrets.age";
const IDENTITY: &str = "secrets.key";
const LOCK: &str = "secrets.lock";

/// The stored Cloudflare token `cx mesh` and `cx claude` hand to mesh.py.
pub const CLOUDFLARE_TOKEN: &str = "cloudflare.api_token";
/// Names whose default variable is not the derived one. Add a row only when something in cx consumes it.
const WELL_KNOWN: &[(&str, &str)] = &[(CLOUDFLARE_TOKEN, "CLOUDFLARE_API_TOKEN")];

/// The variable [`Secrets::inject`] sets for `name` unless the spec says `VAR=NAME`.
fn default_env_var(name: &str) -> String {
    WELL_KNOWN
        .iter()
        .find(|(n, _)| *n == name)
        .map_or_else(|| derived_env_var(name), |(_, var)| var.to_string())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Backend {
    Keyring,
    File,
}

impl Backend {
    pub fn as_str(self) -> &'static str {
        match self {
            Backend::Keyring => "keyring",
            Backend::File => "file",
        }
    }
}

/// Everything known about a secret except its value.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretMeta {
    pub name: String,
    /// Unix seconds.
    pub created: u64,
    pub updated: u64,
    pub last_used: Option<u64>,
    pub backend: Backend,
    /// Value length in characters, for the masked "•••• (40)" display.
    pub length: usize,
}

/// A secret value: zeroized on drop, no `Display`, `Debug` prints only its length.
pub struct SecretValue(Zeroizing<String>);

impl SecretValue {
    pub fn new(value: Zeroizing<String>) -> SecretValue {
        SecretValue(value)
    }
    pub fn expose(&self) -> &str {
        &self.0
    }
    pub fn len(&self) -> usize {
        self.0.chars().count()
    }
}

impl std::fmt::Debug for SecretValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SecretValue(<redacted, {} chars>)", self.len())
    }
}

/// A place values live. Implementations must never put a value in an error message.
pub trait Vault: Send + Sync + 'static {
    fn get(&self, name: &str) -> Result<Option<SecretValue>>;
    fn set(&self, name: &str, value: &str) -> Result<()>;
    /// Removing a missing entry is not an error.
    fn delete(&self, name: &str) -> Result<()>;
}

/// The OS keyring via the `keyring` crate (service "cx", user = secret name).
pub struct OsKeyring;

fn keyring_error(e: keyring::Error) -> anyhow::Error {
    // BadEncoding carries the stored bytes, so variants are mapped instead of displayed.
    use keyring::Error as E;
    match e {
        E::PlatformFailure(inner) => anyhow::anyhow!("OS keyring failed: {inner}"),
        E::NoStorageAccess(_) => anyhow::anyhow!("OS keyring is locked or not accessible"),
        E::BadEncoding(_) => anyhow::anyhow!("OS keyring entry is not valid UTF-8"),
        E::TooLong(..) | E::Invalid(..) => anyhow::anyhow!("OS keyring rejected the entry"),
        E::Ambiguous(_) => anyhow::anyhow!("OS keyring holds several matching entries"),
        _ => anyhow::anyhow!("OS keyring error"),
    }
}

impl Vault for OsKeyring {
    fn get(&self, name: &str) -> Result<Option<SecretValue>> {
        let entry = keyring::Entry::new(KEYRING_SERVICE, name).map_err(keyring_error)?;
        match entry.get_password() {
            Ok(v) => Ok(Some(SecretValue(Zeroizing::new(v)))),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(keyring_error(e)),
        }
    }
    fn set(&self, name: &str, value: &str) -> Result<()> {
        keyring::Entry::new(KEYRING_SERVICE, name)
            .map_err(keyring_error)?
            .set_password(value)
            .map_err(keyring_error)
    }
    fn delete(&self, name: &str) -> Result<()> {
        match keyring::Entry::new(KEYRING_SERVICE, name)
            .map_err(keyring_error)?
            .delete_credential()
        {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(keyring_error(e)),
        }
    }
}

/// Wraps a vault so every call returns within `timeout`. After one timeout the vault reports itself unavailable
/// without trying again (the stuck thread is left to finish on its own).
#[derive(Clone)]
pub struct TimedVault {
    inner: Arc<dyn Vault>,
    timeout: Duration,
    hung: Arc<AtomicBool>,
}

impl TimedVault {
    /// `hung` is shared by every vault of this process (see `process_keyring_hung`).
    pub fn new(inner: Arc<dyn Vault>, timeout: Duration, hung: Arc<AtomicBool>) -> TimedVault {
        TimedVault {
            inner,
            timeout,
            hung,
        }
    }
    pub fn is_hung(&self) -> bool {
        self.hung.load(Ordering::SeqCst)
    }
    fn call<T: Send + 'static>(
        &self,
        f: impl FnOnce(&dyn Vault) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        ensure!(
            !self.is_hung(),
            "OS keyring stopped answering earlier in this process"
        );
        let (tx, rx) = mpsc::channel();
        let inner = self.inner.clone();
        std::thread::Builder::new()
            .name("cx-keyring".into())
            .spawn(move || {
                let _ = tx.send(f(&*inner));
            })
            .context("start keyring thread")?;
        match rx.recv_timeout(self.timeout) {
            Ok(result) => result,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                self.hung.store(true, Ordering::SeqCst);
                bail!(
                    "OS keyring did not answer within {} ms",
                    self.timeout.as_millis()
                )
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => bail!("OS keyring call failed"),
        }
    }
    pub fn get(&self, name: &str) -> Result<Option<SecretValue>> {
        let name = name.to_string();
        self.call(move |v| v.get(&name))
    }
    pub fn set(&self, name: &str, value: &str) -> Result<()> {
        let name = name.to_string();
        let value = Zeroizing::new(value.to_string());
        self.call(move |v| v.set(&name, &value))
    }
    pub fn delete(&self, name: &str) -> Result<()> {
        let name = name.to_string();
        self.call(move |v| v.delete(&name))
    }
}

/// Whether an OS keyring might be reachable from this session. On Linux that needs a session bus; headless robots
/// and plain SSH logins usually have none, and then the age file is used without trying.
pub fn keyring_reachable() -> bool {
    if cfg!(target_os = "macos") {
        return true;
    }
    if std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_some_and(|v| !v.is_empty()) {
        return true;
    }
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(|d| PathBuf::from(d).join("bus"))
        .and_then(|p| fs::symlink_metadata(p).ok())
        .is_some_and(|m| {
            use std::os::unix::fs::FileTypeExt;
            m.file_type().is_socket()
        })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Keyring when it answers, else the age file.
    Auto,
    /// Never touch the keyring (`CX_SECRETS_BACKEND=file`).
    FileOnly,
    /// Store only in the keyring (`CX_SECRETS_BACKEND=keyring`).
    KeyringOnly,
}

type Map = BTreeMap<String, Zeroizing<String>>;

/// The age-encrypted fallback file and its identity.
struct FileVault {
    dir: PathBuf,
}

impl FileVault {
    fn store(&self) -> PathBuf {
        self.dir.join(STORE)
    }
    fn identity_path(&self) -> PathBuf {
        self.dir.join(IDENTITY)
    }

    fn identity(&self, create: bool) -> Result<Option<age::x25519::Identity>> {
        use age::secrecy::ExposeSecret;
        let path = self.identity_path();
        match fs::symlink_metadata(&path) {
            Ok(m) => {
                ensure!(
                    m.file_type().is_file() && m.uid() == unsafe { libc::geteuid() },
                    "{} is not a regular file owned by this user; refusing to use it",
                    path.display()
                );
                ensure!(
                    m.mode() & 0o077 == 0,
                    "{} is readable by other users; run chmod 600 on it",
                    path.display()
                );
                let mut text = Zeroizing::new(String::new());
                fs::File::open(&path)?
                    .take(4096)
                    .read_to_string(&mut text)
                    .with_context(|| format!("read {}", path.display()))?;
                let line = text
                    .lines()
                    .find(|l| l.starts_with("AGE-SECRET-KEY-"))
                    .unwrap_or("");
                let id = line
                    .parse::<age::x25519::Identity>()
                    .map_err(|_| anyhow::anyhow!("{} is not an age identity", path.display()))?;
                Ok(Some(id))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                if fs::symlink_metadata(self.store()).is_ok() {
                    bail!(
                        "{} exists but its identity {} is missing; the file cannot be decrypted",
                        self.store().display(),
                        path.display()
                    );
                }
                if !create {
                    return Ok(None);
                }
                let id = age::x25519::Identity::generate();
                let body = Zeroizing::new(format!(
                    "# cx secrets identity: decrypts secrets.age. Protect it like an SSH private key.\n# public key: {}\n{}\n",
                    id.to_public(),
                    id.to_string().expose_secret()
                ));
                atomic_write(&self.dir, IDENTITY, body.as_bytes())?;
                Ok(Some(id))
            }
            Err(e) => Err(e).with_context(|| format!("inspect {}", path.display())),
        }
    }

    fn corrupt(&self) -> String {
        format!(
            "{} is unreadable (corrupt, truncated or not encrypted to {}); it was left untouched. Move it aside to start an empty store",
            self.store().display(),
            self.identity_path().display()
        )
    }

    fn read(&self) -> Result<Map> {
        let path = self.store();
        let m = match fs::symlink_metadata(&path) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                // Still validate an existing identity, so a bad key is reported before the first write.
                self.identity(false)?;
                return Ok(Map::new());
            }
            Err(e) => return Err(e).with_context(|| format!("inspect {}", path.display())),
        };
        ensure!(
            m.file_type().is_file() && m.uid() == unsafe { libc::geteuid() },
            "{} is not a regular file owned by this user",
            path.display()
        );
        ensure!(
            m.len() <= STORE_LIMIT,
            "{} exceeds its size limit",
            path.display()
        );
        let id = self.identity(false)?.context("secrets identity missing")?;
        let bytes = fs::read(&path).with_context(|| format!("read {}", path.display()))?;
        let decryptor =
            age::Decryptor::new(&bytes[..]).map_err(|_| anyhow::anyhow!(self.corrupt()))?;
        let mut reader = decryptor
            .decrypt(std::iter::once(&id as &dyn age::Identity))
            .map_err(|_| anyhow::anyhow!(self.corrupt()))?;
        let mut plain = Zeroizing::new(Vec::new());
        reader
            .read_to_end(&mut plain)
            .map_err(|_| anyhow::anyhow!(self.corrupt()))?;
        #[derive(Deserialize)]
        struct Body {
            version: u32,
            secrets: Map,
        }
        let body: Body =
            serde_json::from_slice(&plain).map_err(|_| anyhow::anyhow!(self.corrupt()))?;
        ensure!(
            body.version == 1,
            "{} has unsupported version",
            path.display()
        );
        Ok(body.secrets)
    }

    fn write(&self, map: &Map) -> Result<()> {
        let id = self.identity(true)?.context("secrets identity missing")?;
        let recipient = id.to_public();
        #[derive(Serialize)]
        struct Body<'a> {
            version: u32,
            secrets: &'a Map,
        }
        let plain = Zeroizing::new(serde_json::to_vec(&Body {
            version: 1,
            secrets: map,
        })?);
        let encryptor =
            age::Encryptor::with_recipients(std::iter::once(&recipient as &dyn age::Recipient))
                .map_err(|_| anyhow::anyhow!("age encryption setup failed"))?;
        let mut out = Vec::with_capacity(plain.len() + 256);
        let mut w = encryptor.wrap_output(&mut out)?;
        w.write_all(&plain)?;
        w.finish()?;
        atomic_write(&self.dir, STORE, &out)
    }
}

/// Write `name` in `dir` by temp file (0600, create_new) + fsync + rename + directory fsync.
fn atomic_write(dir: &Path, name: &str, bytes: &[u8]) -> Result<()> {
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

/// Refuse a state file that is not a regular file owned by this user without group/other bits (the checks
/// store.rs applies to its private files). `file` was opened with O_NOFOLLOW, so a symlink never gets here.
fn ensure_private(file: &fs::File, path: &Path) -> Result<()> {
    let m = file.metadata()?;
    ensure!(
        m.is_file() && m.uid() == unsafe { libc::geteuid() } && m.mode() & 0o077 == 0,
        "{} is not a private file owned by this user; refusing to use it",
        path.display()
    );
    Ok(())
}

struct Lock(fs::File);

impl Lock {
    fn take(dir: &Path) -> Result<Lock> {
        use std::os::fd::AsRawFd;
        let path = dir.join(LOCK);
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&path)
            .context("open secrets lock")?;
        ensure_private(&file, &path)?;
        ensure!(
            unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } == 0,
            "lock secrets store"
        );
        Ok(Lock(file))
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        use std::os::fd::AsRawFd;
        unsafe { libc::flock(self.0.as_raw_fd(), libc::LOCK_UN) };
    }
}

#[derive(Default, Serialize, Deserialize)]
struct Index {
    version: u32,
    secrets: BTreeMap<String, SecretMeta>,
    /// Names whose value moved from the OS keyring to the file while the keyring could not be reached: the old
    /// keyring copy is deleted by a later set/delete once the keyring answers.
    #[serde(default, skip_serializing_if = "std::collections::BTreeSet::is_empty")]
    stale_keyring: std::collections::BTreeSet<String>,
}

/// A secrets store rooted at one directory.
pub struct Secrets {
    dir: PathBuf,
    keyring: Option<TimedVault>,
    file: FileVault,
    mode: Mode,
}

impl std::fmt::Debug for Secrets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Secrets")
            .field("dir", &self.dir)
            .field("keyring", &self.keyring.is_some())
            .field("mode", &self.mode)
            .finish()
    }
}

fn process_keyring_hung() -> Arc<AtomicBool> {
    static HUNG: OnceLock<Arc<AtomicBool>> = OnceLock::new();
    HUNG.get_or_init(|| Arc::new(AtomicBool::new(false)))
        .clone()
}

pub fn validate_name(name: &str) -> Result<()> {
    let ok = (1..=64).contains(&name.len())
        && name.as_bytes()[0].is_ascii_alphanumeric()
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._-".contains(&b));
    ensure!(
        ok,
        "secret names are 1-64 characters of a-z, 0-9, '.', '_' and '-', starting with a letter or digit"
    );
    Ok(())
}

fn derived_env_var(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect()
}

fn validate_env_var(var: &str) -> Result<()> {
    let ok = !var.is_empty()
        && var.len() <= 128
        && !var.as_bytes()[0].is_ascii_digit()
        && var.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_');
    ensure!(ok, "invalid environment variable name {var:?}");
    Ok(())
}

impl Secrets {
    /// The store in cx's state dir, with the keyring as `CX_SECRETS_BACKEND` (auto | file | keyring) allows.
    pub fn open() -> Result<Secrets> {
        let dir = crate::store::ensure()?;
        let mode = match std::env::var("CX_SECRETS_BACKEND").as_deref() {
            Ok("file") => Mode::FileOnly,
            Ok("keyring") => Mode::KeyringOnly,
            Ok("auto") | Ok("") | Err(_) => Mode::Auto,
            Ok(other) => bail!("CX_SECRETS_BACKEND must be auto, file or keyring, not {other:?}"),
        };
        let keyring = (mode != Mode::FileOnly && keyring_reachable())
            .then(|| TimedVault::new(Arc::new(OsKeyring), KEYRING_TIMEOUT, process_keyring_hung()));
        Ok(Secrets::at(dir, keyring, mode))
    }

    /// A store in `dir` (which the caller has made 0700) with an optional keyring; used by tests with fakes.
    pub fn at(dir: PathBuf, keyring: Option<TimedVault>, mode: Mode) -> Secrets {
        Secrets {
            file: FileVault { dir: dir.clone() },
            dir,
            keyring: if mode == Mode::FileOnly {
                None
            } else {
                keyring
            },
            mode,
        }
    }

    fn read_index(&self) -> Result<Index> {
        let path = self.dir.join(INDEX);
        let file = match fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(&path)
        {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Index {
                    version: 1,
                    ..Index::default()
                })
            }
            Err(e) => return Err(e).with_context(|| format!("read {}", path.display())),
        };
        ensure_private(&file, &path)?;
        let mut bytes = Vec::new();
        file.take(INDEX_LIMIT + 1).read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() as u64 <= INDEX_LIMIT,
            "{} exceeds its size limit",
            path.display()
        );
        let index: Index = serde_json::from_slice(&bytes).map_err(|_| {
            anyhow::anyhow!(
                "{} is corrupt; it was left untouched (it holds names only, no values)",
                path.display()
            )
        })?;
        ensure!(
            index.version == 1,
            "{} has unsupported version",
            path.display()
        );
        Ok(index)
    }

    fn write_index(&self, index: &Index) -> Result<()> {
        atomic_write(&self.dir, INDEX, &serde_json::to_vec_pretty(index)?)
    }

    /// Metadata of every secret, by name. Reads only the index: no backend is contacted.
    pub fn list(&self) -> Result<Vec<SecretMeta>> {
        Ok(self.read_index()?.secrets.into_values().collect())
    }

    pub fn meta(&self, name: &str) -> Result<Option<SecretMeta>> {
        Ok(self.read_index()?.secrets.remove(name))
    }

    fn keyring(&self) -> Result<&TimedVault> {
        self.keyring
            .as_ref()
            .context("this secret is in the OS keyring, which is not reachable from this session")
    }

    /// The value, or None when no secret has this name.
    pub fn try_get(&self, name: &str) -> Result<Option<SecretValue>> {
        validate_name(name)?;
        let Some(meta) = self.meta(name)? else {
            return Ok(None);
        };
        let value = match meta.backend {
            Backend::Keyring => self.keyring()?.get(name)?,
            Backend::File => self.file.read()?.remove(name).map(SecretValue::new),
        };
        let value = value.with_context(|| {
            format!(
                "secret {name} is listed but missing from its {} backend",
                meta.backend.as_str()
            )
        })?;
        let t = crate::transport::now();
        if meta.last_used.is_none_or(|u| t.saturating_sub(u) >= 60) {
            let _ = self.touch(name, t);
        }
        Ok(Some(value))
    }

    fn touch(&self, name: &str, t: u64) -> Result<()> {
        let _lock = Lock::take(&self.dir)?;
        let mut index = self.read_index()?;
        if let Some(m) = index.secrets.get_mut(name) {
            m.last_used = Some(t);
            self.write_index(&index)?;
        }
        Ok(())
    }

    pub fn get(&self, name: &str) -> Result<SecretValue> {
        self.try_get(name)?
            .with_context(|| format!("no secret named {name}; `cx secrets list` shows them"))
    }

    /// Store `value` under `name`, replacing any previous value (whose `created` is kept).
    pub fn set(&self, name: &str, value: &str) -> Result<SecretMeta> {
        validate_name(name)?;
        ensure!(!value.is_empty(), "refusing to store an empty secret");
        ensure!(
            value.len() <= VALUE_LIMIT,
            "secret exceeds {VALUE_LIMIT} bytes"
        );
        ensure!(
            !value.contains('\0'),
            "secret contains a NUL byte, which no environment variable can carry"
        );
        let _lock = Lock::take(&self.dir)?;
        let mut index = self.read_index()?;
        let previous = index.secrets.get(name).cloned();
        if previous.is_none() {
            ensure!(index.secrets.len() < MAX_SECRETS, "too many secrets");
        }
        // Read the file first so a corrupt store is reported before anything changes.
        let mut map = self.file.read()?;
        let mut keyring_error = None;
        let backend = match (&self.keyring, self.mode) {
            (Some(k), _) => match k.set(name, value) {
                Ok(()) => Backend::Keyring,
                Err(e) if self.mode == Mode::Auto => {
                    keyring_error = Some(e);
                    Backend::File
                }
                Err(e) => return Err(e),
            },
            (None, Mode::KeyringOnly) => {
                bail!("CX_SECRETS_BACKEND=keyring but no OS keyring is reachable from this session")
            }
            (None, _) => Backend::File,
        };
        match backend {
            Backend::File => {
                map.insert(name.to_string(), Zeroizing::new(value.to_string()));
                self.file.write(&map)?;
            }
            Backend::Keyring => {
                if map.remove(name).is_some() {
                    self.file.write(&map)?;
                }
            }
        }
        // Leaving the keyring strands the old copy there; a keyring call that failed or timed out just now may
        // still land later (a timed-out thread keeps running). Remember both so a later sweep deletes them.
        if backend == Backend::Keyring {
            index.stale_keyring.remove(name);
        } else if keyring_error.is_some()
            || previous.as_ref().map(|p| p.backend) == Some(Backend::Keyring)
        {
            index.stale_keyring.insert(name.to_string());
        }
        let t = crate::transport::now();
        let meta = SecretMeta {
            name: name.to_string(),
            created: previous.as_ref().map_or(t, |p| p.created),
            updated: t,
            last_used: previous.and_then(|p| p.last_used),
            backend,
            length: value.chars().count(),
        };
        index.version = 1;
        index.secrets.insert(name.to_string(), meta.clone());
        if keyring_error.is_none() {
            self.sweep_stale_keyring(&mut index);
        }
        self.write_index(&index)?;
        Ok(meta)
    }

    /// Remove a secret from its backend and the index.
    pub fn delete(&self, name: &str) -> Result<SecretMeta> {
        validate_name(name)?;
        let _lock = Lock::take(&self.dir)?;
        let mut index = self.read_index()?;
        let meta = index
            .secrets
            .get(name)
            .cloned()
            .with_context(|| format!("no secret named {name}"))?;
        match meta.backend {
            Backend::Keyring => self.keyring()?.delete(name)?,
            Backend::File => {
                let mut map = self.file.read()?;
                if map.remove(name).is_some() {
                    self.file.write(&map)?;
                }
            }
        }
        index.secrets.remove(name);
        self.sweep_stale_keyring(&mut index);
        self.write_index(&index)?;
        Ok(meta)
    }

    /// Delete stranded keyring copies (see `Index::stale_keyring`) when a keyring is reachable, stopping at the
    /// first failure so an unresponsive keyring costs at most one bounded wait. Caller holds the lock and writes
    /// the index afterwards.
    fn sweep_stale_keyring(&self, index: &mut Index) {
        let Some(k) = &self.keyring else { return };
        let names: Vec<String> = index.stale_keyring.iter().cloned().collect();
        for name in names {
            // A name whose current value lives in the keyring is not stale.
            if index.secrets.get(&name).map(|m| m.backend) == Some(Backend::Keyring) {
                index.stale_keyring.remove(&name);
                continue;
            }
            if k.is_hung() || k.delete(&name).is_err() {
                return;
            }
            index.stale_keyring.remove(&name);
        }
    }

    /// Names with a stranded copy still in the OS keyring.
    #[cfg(test)]
    fn stale_keyring(&self) -> Result<Vec<String>> {
        Ok(self.read_index()?.stale_keyring.into_iter().collect())
    }

    /// Set each secret as an environment variable of `cmd` only. A spec is `NAME` (the well-known variable, else
    /// the upper-cased name) or `VAR=NAME`; two specs may not set the same variable. All values are fetched
    /// before any is applied.
    pub fn inject(&self, specs: &[&str], cmd: &mut std::process::Command) -> Result<Vec<String>> {
        let mut values = Vec::new();
        for spec in specs {
            let (var, name) = match spec.split_once('=') {
                Some((var, name)) => (Some(var.to_string()), name),
                None => (None, *spec),
            };
            let var = var.unwrap_or_else(|| default_env_var(name));
            validate_env_var(&var)?;
            ensure!(
                values.iter().all(|(v, _)| *v != var),
                "two secrets would set {var}; name one with VAR=NAME"
            );
            values.push((var, self.get(name)?));
        }
        let vars = values.iter().map(|(v, _)| v.clone()).collect();
        for (var, value) in &values {
            cmd.env(var, value.expose());
        }
        Ok(vars)
    }
}

// ---- CLI -----------------------------------------------------------------------------------------------------

#[derive(clap::Subcommand)]
pub enum Cli {
    /// Names, backends and dates; never values.
    List {
        #[arg(long)]
        json: bool,
    },
    /// Store a value read from stdin (or a no-echo prompt on a terminal); values are never taken from argv.
    Set { name: String },
    /// Hand a value to a program: write it to an inherited descriptor, or run a command with it in its env.
    Get {
        /// Secret names; with --exec each may be VAR=NAME to choose the variable.
        #[arg(required = true)]
        names: Vec<String>,
        /// Write the single value (no newline) to this already-open descriptor; refused if it is a terminal.
        #[arg(long, conflicts_with = "exec", required_unless_present = "exec")]
        fd: Option<i32>,
        /// Run the command after `--` with the secrets as environment variables.
        #[arg(long, requires = "command")]
        exec: bool,
        #[arg(last = true)]
        command: Vec<String>,
    },
    /// Remove a secret.
    Rm { name: String },
    /// Import the plaintext `cx mesh` token ($XDG_CONFIG_HOME/cx/mesh/credentials) as cloudflare.api_token.
    Migrate {
        /// After a verified import, delete the plaintext file.
        #[arg(long)]
        delete_plaintext: bool,
    },
}

fn date(t: u64) -> String {
    let age = crate::transport::now().saturating_sub(t);
    match age {
        0..=59 => "now".into(),
        60..=3599 => format!("{}m", age / 60),
        3600..=86399 => format!("{}h", age / 3600),
        _ => format!("{}d", age / 86400),
    }
}

/// Read a value from stdin, or from /dev/tty with echo off when stdin is a terminal.
fn read_value(name: &str) -> Result<Zeroizing<String>> {
    let mut bytes = Zeroizing::new(Vec::new());
    if unsafe { libc::isatty(libc::STDIN_FILENO) } != 0 {
        prompt_no_echo(&format!("Value for {name} (not echoed): "), &mut bytes)?;
    } else {
        std::io::stdin()
            .take(VALUE_LIMIT as u64 + 3)
            .read_to_end(&mut bytes)?;
    }
    while matches!(bytes.last(), Some(b'\n' | b'\r')) {
        bytes.pop();
    }
    ensure!(
        bytes.len() <= VALUE_LIMIT,
        "secret exceeds {VALUE_LIMIT} bytes"
    );
    let text = std::str::from_utf8(&bytes).map_err(|_| anyhow::anyhow!("secret is not UTF-8"))?;
    Ok(Zeroizing::new(text.to_string()))
}

/// The terminal mode a running prompt must put back, for the signal handler. Written before the
/// handler is installed and only read by it.
static PROMPT_TTY: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(-1);
static mut PROMPT_MODE: std::mem::MaybeUninit<libc::termios> = std::mem::MaybeUninit::uninit();
const PROMPT_SIGNALS: [libc::c_int; 4] = [libc::SIGINT, libc::SIGTERM, libc::SIGHUP, libc::SIGQUIT];

extern "C" fn restore_tty_and_die(sig: libc::c_int) {
    // Only async-signal-safe calls: tcsetattr, sigaction (via signal) and raise.
    let fd = PROMPT_TTY.load(Ordering::SeqCst);
    if fd >= 0 {
        unsafe { libc::tcsetattr(fd, libc::TCSANOW, std::ptr::addr_of!(PROMPT_MODE).cast()) };
    }
    unsafe {
        libc::signal(sig, libc::SIG_DFL);
        libc::raise(sig);
    }
}

/// Read one line from /dev/tty with echo off. The terminal is put in non-canonical mode with ISIG off, so Ctrl-C
/// (and Ctrl-\) arrive as bytes and cancel the prompt here, Ctrl-D ends the value like Enter, and the mode is
/// always restored on the way out; SIGINT/SIGTERM/SIGHUP/SIGQUIT from elsewhere restore it in a handler before the
/// process dies as it would have.
fn prompt_no_echo(label: &str, out: &mut Vec<u8>) -> Result<()> {
    use std::os::fd::AsRawFd;
    let mut tty = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty")
        .context("open the terminal for a no-echo prompt")?;
    let fd = tty.as_raw_fd();
    let mut old: libc::termios = unsafe { std::mem::zeroed() };
    ensure!(
        unsafe { libc::tcgetattr(fd, &mut old) } == 0,
        "read terminal mode"
    );
    let mut quiet = old;
    quiet.c_lflag &= !(libc::ECHO | libc::ICANON | libc::ISIG | libc::IEXTEN);
    quiet.c_cc[libc::VMIN] = 1;
    quiet.c_cc[libc::VTIME] = 0;

    struct Restore {
        fd: i32,
        old: libc::termios,
        previous: Vec<(libc::c_int, libc::sigaction)>,
    }
    impl Drop for Restore {
        fn drop(&mut self) {
            unsafe { libc::tcsetattr(self.fd, libc::TCSAFLUSH, &self.old) };
            for (sig, action) in &self.previous {
                unsafe { libc::sigaction(*sig, action, std::ptr::null_mut()) };
            }
            PROMPT_TTY.store(-1, Ordering::SeqCst);
        }
    }
    unsafe { std::ptr::addr_of_mut!(PROMPT_MODE).write(std::mem::MaybeUninit::new(old)) };
    PROMPT_TTY.store(fd, Ordering::SeqCst);
    let mut restore = Restore {
        fd,
        old,
        previous: Vec::new(),
    };
    for sig in PROMPT_SIGNALS {
        let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
        action.sa_sigaction =
            restore_tty_and_die as extern "C" fn(libc::c_int) as libc::sighandler_t;
        unsafe { libc::sigemptyset(&mut action.sa_mask) };
        let mut previous: libc::sigaction = unsafe { std::mem::zeroed() };
        if unsafe { libc::sigaction(sig, &action, &mut previous) } == 0 {
            restore.previous.push((sig, previous));
        }
    }
    ensure!(
        unsafe { libc::tcsetattr(fd, libc::TCSAFLUSH, &quiet) } == 0,
        "turn off terminal echo"
    );
    tty.write_all(label.as_bytes())?;
    tty.flush()?;
    let (intr, quit, eof, erase, kill) = (
        old.c_cc[libc::VINTR],
        old.c_cc[libc::VQUIT],
        old.c_cc[libc::VEOF],
        old.c_cc[libc::VERASE],
        old.c_cc[libc::VKILL],
    );
    let mut byte = [0u8; 1];
    let cancelled = loop {
        if tty.read(&mut byte)? == 0 {
            break false;
        }
        match byte[0] {
            b'\n' | b'\r' => break false,
            b if b == intr || b == quit => break true,
            b if b == eof => break false,
            b if b == erase || b == 0x7f || b == 0x08 => {
                // Drop one whole character: its UTF-8 continuation bytes, then its lead byte.
                while out.pop().is_some_and(|c| c & 0xc0 == 0x80) {}
            }
            b if b == kill => out.clear(),
            b => {
                out.push(b);
                ensure!(
                    out.len() <= VALUE_LIMIT + 2,
                    "secret exceeds {VALUE_LIMIT} bytes"
                );
            }
        }
    };
    let _ = tty.write_all(b"\n");
    drop(restore);
    if cancelled {
        out.clear();
        bail!("cancelled; nothing stored");
    }
    Ok(())
}

fn write_fd(fd: i32, value: &SecretValue) -> Result<()> {
    use std::os::fd::FromRawFd;
    ensure!(
        fd >= 1,
        "--fd must name an open output descriptor (1 or above)"
    );
    ensure!(
        unsafe { libc::fcntl(fd, libc::F_GETFD) } != -1,
        "descriptor {fd} is not open"
    );
    ensure!(
        unsafe { libc::isatty(fd) } == 0,
        "refusing to write a secret to a terminal; redirect descriptor {fd} or use --exec"
    );
    let mut file = std::mem::ManuallyDrop::new(unsafe { fs::File::from_raw_fd(fd) });
    file.write_all(value.expose().as_bytes())?;
    file.flush()?;
    Ok(())
}

/// The plaintext token file `cx mesh setup` writes (mesh/mesh.py `Mesh.creds`).
pub fn mesh_credentials_path() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .map(|c| c.join("cx/mesh/credentials"))
}

fn migrate(secrets: &Secrets, delete_plaintext: bool) -> Result<()> {
    let name = CLOUDFLARE_TOKEN;
    let path = mesh_credentials_path().context("HOME is not set")?;
    let mut text = Zeroizing::new(String::new());
    match fs::File::open(&path) {
        Ok(f) => {
            f.take(VALUE_LIMIT as u64 + 2).read_to_string(&mut text)?;
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            println!("nothing to migrate: {} does not exist", path.display());
            return Ok(());
        }
        Err(e) => return Err(e).with_context(|| format!("read {}", path.display())),
    }
    let token = text.trim();
    if token.is_empty() {
        println!("nothing to migrate: {} is empty", path.display());
        return Ok(());
    }
    match secrets.try_get(name)? {
        Some(existing) if existing.expose() == token => {
            println!("{name} already holds the token from {}", path.display())
        }
        Some(_) => bail!(
            "{name} already holds a different token; nothing changed. Run `cx secrets set {name}` to choose one"
        ),
        None => {
            let meta = secrets.set(name, token)?;
            println!(
                "imported {} into {name} ({} backend)",
                path.display(),
                meta.backend.as_str()
            );
        }
    }
    if delete_plaintext {
        let stored = secrets.get(name)?;
        ensure!(
            stored.expose() == token,
            "stored token does not match the plaintext; kept {}",
            path.display()
        );
        fs::remove_file(&path).with_context(|| format!("remove {}", path.display()))?;
        println!(
            "deleted {}; `cx mesh` passes the stored token to mesh.py, and standalone mesh.py needs $CLOUDFLARE_API_TOKEN",
            path.display()
        );
    } else {
        println!(
            "the plaintext copy is still at {}; re-run with --delete-plaintext to remove it",
            path.display()
        );
    }
    Ok(())
}

pub fn run_cli(command: Cli) -> Result<()> {
    let secrets = Secrets::open()?;
    match command {
        Cli::List { json } => {
            let all = secrets.list()?;
            if json {
                println!("{}", serde_json::to_string_pretty(&all)?);
            } else if all.is_empty() {
                println!("no secrets; `cx secrets set NAME` stores one from stdin");
            } else {
                println!(
                    "{:<28} {:<8} {:<14} {:>7} {:>9}",
                    "NAME", "BACKEND", "VALUE", "CREATED", "LAST USED"
                );
                for m in all {
                    println!(
                        "{:<28} {:<8} {:<14} {:>7} {:>9}",
                        m.name,
                        m.backend.as_str(),
                        format!("•••••••• ({})", m.length),
                        date(m.created),
                        m.last_used.map(date).unwrap_or_else(|| "never".into())
                    );
                }
            }
        }
        Cli::Set { name } => {
            validate_name(&name)?;
            let value = read_value(&name)?;
            let meta = secrets.set(&name, &value)?;
            eprintln!(
                "stored {} ({} chars) in the {} backend",
                meta.name,
                meta.length,
                meta.backend.as_str()
            );
        }
        Cli::Get {
            names,
            fd,
            exec,
            command,
        } => {
            if let Some(fd) = fd {
                ensure!(names.len() == 1, "--fd writes exactly one secret");
                return write_fd(fd, &secrets.get(&names[0])?);
            }
            ensure!(
                exec && !command.is_empty(),
                "--exec needs a command after --"
            );
            use std::os::unix::process::CommandExt;
            let mut child = std::process::Command::new(&command[0]);
            child.args(&command[1..]);
            let specs: Vec<&str> = names.iter().map(String::as_str).collect();
            secrets.inject(&specs, &mut child)?;
            let err = child.exec();
            bail!("run {}: {err}", command[0]);
        }
        Cli::Rm { name } => {
            let meta = secrets.delete(&name)?;
            eprintln!(
                "removed {} from the {} backend",
                meta.name,
                meta.backend.as_str()
            );
        }
        Cli::Migrate { delete_plaintext } => migrate(&secrets, delete_plaintext)?,
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::Mutex;

    const CANARY: &str = "cx-canary-7f3a9d1e-DO-NOT-LOG";

    fn temp() -> (tempfile::TempDir, PathBuf) {
        let t = tempfile::tempdir().unwrap();
        let dir = t.path().join("state");
        fs::create_dir(&dir).unwrap();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).unwrap();
        (t, dir)
    }

    fn file_store(dir: &Path) -> Secrets {
        Secrets::at(dir.to_path_buf(), None, Mode::Auto)
    }

    #[derive(Default)]
    struct FakeKeyring {
        map: Mutex<BTreeMap<String, String>>,
        fail: AtomicBool,
        hang: AtomicBool,
    }

    impl Vault for FakeKeyring {
        fn get(&self, name: &str) -> Result<Option<SecretValue>> {
            if self.hang.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_secs(2));
            }
            ensure!(!self.fail.load(Ordering::SeqCst), "fake keyring locked");
            Ok(self
                .map
                .lock()
                .unwrap()
                .get(name)
                .map(|v| SecretValue::new(Zeroizing::new(v.clone()))))
        }
        fn set(&self, name: &str, value: &str) -> Result<()> {
            if self.hang.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_secs(2));
            }
            ensure!(!self.fail.load(Ordering::SeqCst), "fake keyring locked");
            self.map
                .lock()
                .unwrap()
                .insert(name.to_string(), value.to_string());
            Ok(())
        }
        fn delete(&self, name: &str) -> Result<()> {
            ensure!(!self.fail.load(Ordering::SeqCst), "fake keyring locked");
            self.map.lock().unwrap().remove(name);
            Ok(())
        }
    }

    fn keyring_store(dir: &Path, fake: &Arc<FakeKeyring>, timeout: Duration) -> Secrets {
        let vault = TimedVault::new(
            fake.clone() as Arc<dyn Vault>,
            timeout,
            Arc::new(AtomicBool::new(false)),
        );
        Secrets::at(dir.to_path_buf(), Some(vault), Mode::Auto)
    }

    fn mode(path: &Path) -> u32 {
        fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn file_backend_round_trip_with_private_modes() {
        let (_t, dir) = temp();
        let s = file_store(&dir);
        assert!(s.list().unwrap().is_empty());
        let meta = s.set("cloudflare.api_token", "tok-1").unwrap();
        assert_eq!(meta.backend, Backend::File);
        assert_eq!(meta.length, 5);
        s.set("robot.wifi", "pässwörd").unwrap();
        assert_eq!(s.get("cloudflare.api_token").unwrap().expose(), "tok-1");
        assert_eq!(s.get("robot.wifi").unwrap().expose(), "pässwörd");
        let names: Vec<_> = s.list().unwrap().into_iter().map(|m| m.name).collect();
        assert_eq!(names, ["cloudflare.api_token", "robot.wifi"]);
        assert!(s.meta("robot.wifi").unwrap().unwrap().last_used.is_some());
        assert_eq!(s.meta("robot.wifi").unwrap().unwrap().length, 8);
        let created = s.meta("robot.wifi").unwrap().unwrap().created;
        s.set("robot.wifi", "second").unwrap();
        assert_eq!(s.get("robot.wifi").unwrap().expose(), "second");
        assert_eq!(s.meta("robot.wifi").unwrap().unwrap().created, created);
        for f in [STORE, IDENTITY, INDEX, LOCK] {
            assert_eq!(mode(&dir.join(f)), 0o600, "{f}");
        }
        // A fresh handle (another process) decrypts with the same identity.
        assert_eq!(
            file_store(&dir).get("robot.wifi").unwrap().expose(),
            "second"
        );
        s.delete("robot.wifi").unwrap();
        assert!(s.try_get("robot.wifi").unwrap().is_none());
        assert!(s.delete("robot.wifi").is_err());
        assert_eq!(s.list().unwrap().len(), 1);
    }

    #[test]
    fn writes_replace_by_rename_and_leave_no_temporaries() {
        let (_t, dir) = temp();
        let s = file_store(&dir);
        s.set("a", "one").unwrap();
        let old = dir.join("old-link");
        fs::hard_link(dir.join(STORE), &old).unwrap();
        let before = fs::read(&old).unwrap();
        s.set("a", "two").unwrap();
        // The old inode is untouched: the new content arrived by rename, not by rewriting in place.
        assert_eq!(fs::read(&old).unwrap(), before);
        assert_ne!(fs::read(dir.join(STORE)).unwrap(), before);
        fs::remove_file(&old).unwrap();
        let leftovers: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .filter(|n| n.ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
        assert_eq!(s.get("a").unwrap().expose(), "two");
    }

    #[test]
    fn corrupt_store_is_reported_and_never_overwritten() {
        let (_t, dir) = temp();
        let s = file_store(&dir);
        s.set("a", "one").unwrap();
        let mut bytes = fs::read(dir.join(STORE)).unwrap();
        let n = bytes.len();
        bytes[n - 10] ^= 0xff;
        fs::write(dir.join(STORE), &bytes).unwrap();
        let err = format!("{:#}", s.get("a").unwrap_err());
        assert!(err.contains("unreadable"), "{err}");
        let err = format!("{:#}", s.set("b", "two").unwrap_err());
        assert!(err.contains("left untouched"), "{err}");
        assert_eq!(fs::read(dir.join(STORE)).unwrap(), bytes);
        assert!(s.meta("b").unwrap().is_none());
        // Not-age garbage and a truncated file are refused the same way.
        for junk in [&b"not age at all"[..], &bytes[..40]] {
            fs::write(dir.join(STORE), junk).unwrap();
            assert!(s.get("a").is_err());
        }
        // A corrupt index holds no values and is reported, not replaced.
        fs::write(dir.join(INDEX), b"{").unwrap();
        let err = format!("{:#}", s.list().unwrap_err());
        assert!(err.contains("corrupt"), "{err}");
        assert!(s.set("c", "x").is_err());
        assert_eq!(fs::read(dir.join(INDEX)).unwrap(), b"{");
    }

    #[test]
    fn index_and_lock_must_be_private_files() {
        let (_t, dir) = temp();
        let s = file_store(&dir);
        s.set("a", "one").unwrap();
        // A planted symlink is refused, not followed (the lock would otherwise create its target).
        fs::remove_file(dir.join(LOCK)).unwrap();
        std::os::unix::fs::symlink(dir.join("elsewhere"), dir.join(LOCK)).unwrap();
        assert!(s.set("b", "two").is_err());
        assert!(!dir.join("elsewhere").exists());
        fs::remove_file(dir.join(LOCK)).unwrap();
        let planted = dir.join("planted.json");
        fs::rename(dir.join(INDEX), &planted).unwrap();
        std::os::unix::fs::symlink(&planted, dir.join(INDEX)).unwrap();
        assert!(s.list().is_err());
        fs::remove_file(dir.join(INDEX)).unwrap();
        fs::rename(&planted, dir.join(INDEX)).unwrap();
        // So is an index or lock others can write.
        fs::set_permissions(dir.join(INDEX), fs::Permissions::from_mode(0o666)).unwrap();
        let err = format!("{:#}", s.list().unwrap_err());
        assert!(err.contains("not a private file"), "{err}");
        fs::set_permissions(dir.join(INDEX), fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(s.list().unwrap().len(), 1);
        s.set("b", "two").unwrap();
        fs::set_permissions(dir.join(LOCK), fs::Permissions::from_mode(0o666)).unwrap();
        assert!(s.set("c", "three").is_err());
    }

    #[test]
    fn identity_must_be_private_and_present() {
        let (_t, dir) = temp();
        let s = file_store(&dir);
        s.set("a", "one").unwrap();
        fs::set_permissions(dir.join(IDENTITY), fs::Permissions::from_mode(0o644)).unwrap();
        let err = format!("{:#}", s.get("a").unwrap_err());
        assert!(err.contains("chmod 600"), "{err}");
        fs::remove_file(dir.join(IDENTITY)).unwrap();
        let err = format!("{:#}", s.set("b", "two").unwrap_err());
        assert!(err.contains("missing"), "{err}");
        assert!(
            !dir.join(IDENTITY).exists(),
            "must not mint a new identity over an old store"
        );
    }

    #[test]
    fn validates_names_and_values() {
        let (_t, dir) = temp();
        let s = file_store(&dir);
        for bad in ["", "Upper", ".dot", "a b", "x/y", &"a".repeat(65)] {
            assert!(s.set(bad, "v").is_err(), "{bad:?}");
        }
        assert!(s.set("a", "").is_err());
        assert!(s.set("a", "x\0y").is_err());
        assert!(s.set("a", &"x".repeat(VALUE_LIMIT + 1)).is_err());
        // The index records only what something reads: no kind or scope taxonomy.
        s.set("a", "v").unwrap();
        let meta = serde_json::to_value(s.meta("a").unwrap().unwrap()).unwrap();
        let keys: Vec<_> = meta.as_object().unwrap().keys().cloned().collect();
        assert_eq!(
            keys,
            [
                "backend",
                "created",
                "last_used",
                "length",
                "name",
                "updated"
            ]
        );
    }

    #[test]
    fn inject_sets_env_of_that_child_only() {
        let (_t, dir) = temp();
        let s = file_store(&dir);
        s.set("cloudflare.api_token", "cf-v").unwrap();
        s.set("robot.wifi-pass", "wifi-v").unwrap();
        let mut cmd = std::process::Command::new("/bin/sh");
        cmd.args([
            "-c",
            "printf '%s|%s|%s' \"$CLOUDFLARE_API_TOKEN\" \"$ROBOT_WIFI_PASS\" \"$OTHER\"",
        ]);
        let vars = s
            .inject(
                &[
                    "cloudflare.api_token",
                    "robot.wifi-pass",
                    "OTHER=robot.wifi-pass",
                ],
                &mut cmd,
            )
            .unwrap();
        assert_eq!(vars, ["CLOUDFLARE_API_TOKEN", "ROBOT_WIFI_PASS", "OTHER"]);
        let out = cmd.output().unwrap();
        assert_eq!(String::from_utf8(out.stdout).unwrap(), "cf-v|wifi-v|wifi-v");
        assert!(std::env::var_os("ROBOT_WIFI_PASS").is_none());
        let mut cmd = std::process::Command::new("true");
        assert!(s.inject(&["missing"], &mut cmd).is_err());
        assert!(s.inject(&["1BAD=robot.wifi-pass"], &mut cmd).is_err());
        // Two specs setting one variable would let the second silently shadow the first.
        assert!(s
            .inject(
                &[
                    "cloudflare.api_token",
                    "CLOUDFLARE_API_TOKEN=robot.wifi-pass"
                ],
                &mut cmd
            )
            .is_err());
        assert!(
            cmd.get_envs().next().is_none(),
            "nothing applied after a failure"
        );
        // Only the well-known name maps to the Cloudflare variable; any other name gets its own.
        s.set("foo", "other").unwrap();
        let mut cmd = std::process::Command::new("true");
        assert_eq!(
            s.inject(&["cloudflare.api_token", "foo"], &mut cmd)
                .unwrap(),
            ["CLOUDFLARE_API_TOKEN", "FOO"]
        );
    }

    #[test]
    fn keyring_preferred_with_file_fallback_and_migration_between() {
        let (_t, dir) = temp();
        let fake = Arc::new(FakeKeyring::default());
        let s = keyring_store(&dir, &fake, Duration::from_secs(2));
        let meta = s.set("a", "in-keyring").unwrap();
        assert_eq!(meta.backend, Backend::Keyring);
        assert_eq!(fake.map.lock().unwrap().get("a").unwrap(), "in-keyring");
        assert!(
            !dir.join(STORE).exists(),
            "keyring values never reach the file"
        );
        assert_eq!(s.get("a").unwrap().expose(), "in-keyring");

        fake.fail.store(true, Ordering::SeqCst);
        let meta = s.set("b", "in-file").unwrap();
        assert_eq!(meta.backend, Backend::File);
        assert_eq!(s.get("b").unwrap().expose(), "in-file");
        assert!(
            s.get("a").is_err(),
            "keyring entry unreachable while locked"
        );

        fake.fail.store(false, Ordering::SeqCst);
        // b moves to the keyring and leaves the file.
        assert_eq!(s.set("b", "moved").unwrap().backend, Backend::Keyring);
        assert!(file_store(&dir).file.read().unwrap().is_empty());
        s.delete("a").unwrap();
        assert!(!fake.map.lock().unwrap().contains_key("a"));

        let strict = Secrets::at(dir.clone(), None, Mode::KeyringOnly);
        assert!(strict.set("c", "v").is_err());
    }

    #[test]
    fn stranded_keyring_copy_is_deleted_once_the_keyring_answers() {
        let (_t, dir) = temp();
        let fake = Arc::new(FakeKeyring::default());
        let s = keyring_store(&dir, &fake, Duration::from_secs(2));
        assert_eq!(s.set("a", "old").unwrap().backend, Backend::Keyring);

        // Another session without a keyring replaces the value: it lands in the file, the old copy is noted.
        let headless = file_store(&dir);
        assert_eq!(headless.set("a", "new").unwrap().backend, Backend::File);
        assert_eq!(headless.stale_keyring().unwrap(), ["a"]);
        assert_eq!(fake.map.lock().unwrap().get("a").unwrap(), "old");
        assert_eq!(headless.get("a").unwrap().expose(), "new");

        // A keyring that is reachable but failing: the value still goes to the file, the note stays, and b is
        // noted too in case the failed call landed after all.
        fake.fail.store(true, Ordering::SeqCst);
        assert_eq!(s.set("b", "x").unwrap().backend, Backend::File);
        assert_eq!(s.stale_keyring().unwrap(), ["a", "b"]);
        assert!(fake.map.lock().unwrap().contains_key("a"));

        // Once the keyring answers, the next write removes the stranded copy; the file value is untouched.
        fake.fail.store(false, Ordering::SeqCst);
        s.set("c", "y").unwrap();
        assert!(!fake.map.lock().unwrap().contains_key("a"));
        assert!(s.stale_keyring().unwrap().is_empty());
        assert_eq!(s.get("a").unwrap().expose(), "new");
        assert_eq!(s.meta("a").unwrap().unwrap().backend, Backend::File);

        // Moving back into the keyring clears the note without deleting the fresh keyring value.
        headless.set("d", "1").unwrap();
        s.set("d", "2").unwrap();
        let headless_again = file_store(&dir);
        s.set("e", "k").unwrap();
        headless_again.set("e", "f").unwrap();
        assert_eq!(headless_again.stale_keyring().unwrap(), ["e"]);
        s.set("e", "k2").unwrap();
        assert!(s.stale_keyring().unwrap().is_empty());
        assert_eq!(fake.map.lock().unwrap().get("e").unwrap(), "k2");
        s.delete("e").unwrap();
        assert!(!fake.map.lock().unwrap().contains_key("e"));
    }

    #[test]
    fn hung_keyring_is_bounded_and_then_skipped() {
        let (_t, dir) = temp();
        let fake = Arc::new(FakeKeyring::default());
        fake.hang.store(true, Ordering::SeqCst);
        let s = keyring_store(&dir, &fake, Duration::from_millis(150));
        let start = std::time::Instant::now();
        let meta = s.set("a", "v").unwrap();
        assert_eq!(meta.backend, Backend::File);
        assert!(
            start.elapsed() < Duration::from_secs(1),
            "{:?}",
            start.elapsed()
        );
        let start = std::time::Instant::now();
        assert_eq!(s.set("b", "w").unwrap().backend, Backend::File);
        assert!(
            start.elapsed() < Duration::from_millis(100),
            "a hung keyring is not retried"
        );
        assert!(s.keyring.as_ref().unwrap().is_hung());

        // The timed-out call keeps running and lands in the keyring after the value went to the file. It is noted
        // as stale, so once a keyring answers, `rm` (or any later write) deletes that late copy too.
        fake.hang.store(false, Ordering::SeqCst);
        assert_eq!(s.stale_keyring().unwrap(), ["a", "b"]);
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !fake.map.lock().unwrap().contains_key("a") {
            assert!(
                std::time::Instant::now() < deadline,
                "late write never landed"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        let later = keyring_store(&dir, &fake, Duration::from_secs(2));
        later.delete("a").unwrap();
        assert!(
            fake.map.lock().unwrap().is_empty(),
            "late keyring copy left behind"
        );
        assert!(later.stale_keyring().unwrap().is_empty());
        assert_eq!(later.get("b").unwrap().expose(), "w");
    }

    #[test]
    fn values_never_reach_logs_metadata_or_disk_in_clear() {
        let (_t, dir) = temp();
        let fake = Arc::new(FakeKeyring::default());
        let k = keyring_store(&dir, &fake, Duration::from_secs(2));
        let f = file_store(&dir);
        let mut seen = Vec::new();
        k.set("kr", CANARY).unwrap();
        f.set("fl", CANARY).unwrap();
        let v = f.get("fl").unwrap();
        seen.push(format!("{v:?}"));
        seen.push(format!("{:?}", k.get("kr").unwrap()));
        seen.push(format!("{k:?} {f:?}"));
        seen.push(format!("{:?}", f.list().unwrap()));
        seen.push(serde_json::to_string(&f.list().unwrap()).unwrap());
        seen.push(format!(
            "{:#}",
            f.set("fl", &format!("{CANARY}\0")).unwrap_err()
        ));
        seen.push(format!("{:#}", f.set("Bad", CANARY).unwrap_err()));
        fake.fail.store(true, Ordering::SeqCst);
        seen.push(format!("{:#}", k.get("kr").unwrap_err()));
        let mut bytes = fs::read(dir.join(INDEX)).unwrap();
        bytes.extend(fs::read(dir.join(STORE)).unwrap());
        bytes.extend(fs::read(dir.join(IDENTITY)).unwrap());
        for text in &seen {
            assert!(!text.contains(CANARY), "leaked in {text}");
            assert!(!text.contains(&CANARY[..12]), "partial leak in {text}");
        }
        assert!(!bytes.windows(CANARY.len()).any(|w| w == CANARY.as_bytes()));
        assert!(seen[0].contains("redacted"));
    }
}
