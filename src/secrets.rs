//! Secrets: one interface over the OS keyring and a private fallback file (docs/secrets.md).
//!
//! * [`Secrets::list`] returns metadata only (never values) from `secrets.json` and never contacts a backend; it
//!   still reads the index from disk on each call.
//! * [`Secrets::get`] returns a [`SecretValue`], which zeroizes on drop and has no `Display` and a redacted `Debug`.
//! * [`Secrets::set`] stores in the OS keyring when one answers within [`KEYRING_TIMEOUT`], else in the fallback file.
//! * [`Secrets::inject`] puts values into one child's environment, never into argv.
//!
//! Backends: the keyring is the Secret Service through libsecret's `secret-tool` on Linux, values on its stdin and
//! stdout only (no D-Bus stack in this binary, so the static musl release keeps working). macOS uses the file:
//! `security add-generic-password` takes the password only in argv or at a prompt. Every keyring call is one process
//! killed at a bounded wait; a call that hangs marks the keyring unusable for the rest of the process. The fallback is
//! `$XDG_STATE_HOME/cx/secrets.values.json`: the values in plain JSON, 0600 in the 0700 state dir. That is how
//! `~/.ssh/id_ed25519` without a passphrase is stored: other users are kept out by file modes, this user's own
//! processes and root are not, and a copy of the file is a copy of the values.
use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
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
const STORE: &str = "secrets.values.json";
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

/// The OS keyring through libsecret's `secret-tool` (attributes `service=cx name=<secret>`). Values go only
/// through its stdin and stdout, never argv, and never into an error message. Each call is one bounded
/// process, killed with its whole process group at KEYRING_TIMEOUT; after one timeout the keyring is not
/// tried again in this process.
pub struct SecretTool;

static KEYRING_HUNG: AtomicBool = AtomicBool::new(false);

fn secret_tool(args: &[&str], input: &[u8]) -> Result<(bool, Zeroizing<Vec<u8>>)> {
    ensure!(
        !KEYRING_HUNG.load(Ordering::SeqCst),
        "OS keyring stopped answering earlier in this process"
    );
    let mut cmd = std::process::Command::new("secret-tool");
    cmd.args(args);
    let (ok, out) =
        crate::mesh::run_bounded(cmd, input, VALUE_LIMIT, KEYRING_TIMEOUT).map_err(|e| {
            if e.to_string() == "timed out" {
                KEYRING_HUNG.store(true, Ordering::SeqCst);
                return anyhow::anyhow!(
                    "OS keyring did not answer within {} ms",
                    KEYRING_TIMEOUT.as_millis()
                );
            }
            anyhow::anyhow!("secret-tool failed: {e}")
        })?;
    Ok((ok, Zeroizing::new(out)))
}

impl SecretTool {
    pub fn is_hung(&self) -> bool {
        KEYRING_HUNG.load(Ordering::SeqCst)
    }
    pub fn get(&self, name: &str) -> Result<Option<SecretValue>> {
        let (ok, out) = secret_tool(&["lookup", "service", KEYRING_SERVICE, "name", name], b"")?;
        // A missing entry exits 1 with no output (as does a keyring that cannot be reached).
        if !ok && out.is_empty() {
            return Ok(None);
        }
        ensure!(ok, "OS keyring lookup failed");
        let value = std::str::from_utf8(&out).context("OS keyring entry is not valid UTF-8")?;
        Ok(Some(SecretValue(Zeroizing::new(value.to_owned()))))
    }
    pub fn set(&self, name: &str, value: &str) -> Result<()> {
        let label = format!("--label=cx: {name}");
        let args = ["store", &label, "service", KEYRING_SERVICE, "name", name];
        let (ok, _) = secret_tool(&args, value.as_bytes())?;
        ensure!(
            ok,
            "OS keyring refused the entry (locked or no Secret Service?)"
        );
        Ok(())
    }
    /// Removing a missing entry is not an error.
    pub fn delete(&self, name: &str) -> Result<()> {
        let (ok, _) = secret_tool(&["clear", "service", KEYRING_SERVICE, "name", name], b"")?;
        ensure!(ok, "OS keyring delete failed");
        Ok(())
    }
}

/// Whether an OS keyring might be reachable from this session: on Linux, a session bus and `secret-tool` on PATH.
/// Headless robots and plain SSH logins usually have no bus, and then the fallback file is used without trying.
/// macOS always uses the file: `security add-generic-password` takes the password only in argv or at a prompt.
pub fn keyring_reachable() -> bool {
    if !cfg!(target_os = "linux") {
        return false;
    }
    let bus = std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_some_and(|v| !v.is_empty())
        || std::env::var_os("XDG_RUNTIME_DIR")
            .map(|d| PathBuf::from(d).join("bus"))
            .and_then(|p| fs::symlink_metadata(p).ok())
            .is_some_and(|m| {
                use std::os::unix::fs::FileTypeExt;
                m.file_type().is_socket()
            });
    bus && std::env::var_os("PATH").is_some_and(|path| {
        std::env::split_paths(&path).any(|d| {
            use std::os::unix::fs::PermissionsExt;
            fs::metadata(d.join("secret-tool"))
                .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        })
    })
}

type Map = BTreeMap<String, Zeroizing<String>>;

/// The fallback file: `{"version": 1, "secrets": {name: value}}`, 0600 in the 0700 state dir.
struct FileVault {
    dir: PathBuf,
}

impl FileVault {
    fn corrupt(&self) -> anyhow::Error {
        anyhow::anyhow!(
            "{} is unreadable (corrupt or truncated); it was left untouched. Move it aside to start an empty store",
            self.dir.join(STORE).display()
        )
    }

    fn read(&self) -> Result<Map> {
        let path = self.dir.join(STORE);
        let Some(bytes) = crate::store::read_private(&path, STORE_LIMIT)? else {
            return Ok(Map::new());
        };
        let bytes = Zeroizing::new(bytes);
        #[derive(Deserialize)]
        struct Body {
            version: u32,
            secrets: Map,
        }
        let body: Body = serde_json::from_slice(&bytes).map_err(|_| self.corrupt())?;
        ensure!(
            body.version == 1,
            "{} has unsupported version",
            path.display()
        );
        Ok(body.secrets)
    }

    fn write(&self, map: &Map) -> Result<()> {
        #[derive(Serialize)]
        struct Body<'a> {
            version: u32,
            secrets: &'a Map,
        }
        // Sized up front so the serialized values are not left behind in a reallocated buffer.
        let size: usize = map.iter().map(|(k, v)| k.len() + 2 * v.len() + 8).sum();
        let mut plain = Zeroizing::new(Vec::with_capacity(size + 64));
        serde_json::to_writer(
            &mut *plain,
            &Body {
                version: 1,
                secrets: map,
            },
        )?;
        crate::store::atomic_write(&self.dir, STORE, &plain)
    }
}

fn lock(dir: &std::path::Path) -> Result<crate::store::PrivateLock> {
    crate::store::private_lock(dir, LOCK, true)?.context("lock secrets store")
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
    keyring: Option<SecretTool>,
    file: FileVault,
}

impl std::fmt::Debug for Secrets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Secrets")
            .field("dir", &self.dir)
            .field("keyring", &self.keyring.is_some())
            .finish()
    }
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
    /// The store in cx's state dir, with the keyring as `CX_SECRETS_BACKEND` (auto | file) allows.
    pub fn open() -> Result<Secrets> {
        let dir = crate::store::ensure()?;
        let file_only = match std::env::var("CX_SECRETS_BACKEND").as_deref() {
            Ok("file") => true,
            Ok("auto") | Ok("") | Err(_) => false,
            Ok(other) => bail!("CX_SECRETS_BACKEND must be auto or file, not {other:?}"),
        };
        let keyring = (!file_only && keyring_reachable()).then_some(SecretTool);
        Ok(Secrets::at(dir, keyring))
    }

    /// A store in `dir` (which the caller has made 0700) with an optional keyring.
    pub fn at(dir: PathBuf, keyring: Option<SecretTool>) -> Secrets {
        Secrets {
            file: FileVault { dir: dir.clone() },
            dir,
            keyring,
        }
    }

    fn read_index(&self) -> Result<Index> {
        let path = self.dir.join(INDEX);
        let Some(bytes) = crate::store::read_private(&path, INDEX_LIMIT)? else {
            return Ok(Index {
                version: 1,
                ..Index::default()
            });
        };
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
        crate::store::atomic_write(&self.dir, INDEX, &serde_json::to_vec_pretty(index)?)
    }

    /// Metadata of every secret, by name. Reads only the index: no backend is contacted.
    pub fn list(&self) -> Result<Vec<SecretMeta>> {
        Ok(self.read_index()?.secrets.into_values().collect())
    }

    pub fn meta(&self, name: &str) -> Result<Option<SecretMeta>> {
        Ok(self.read_index()?.secrets.remove(name))
    }

    fn keyring(&self) -> Result<&SecretTool> {
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
        let _lock = lock(&self.dir)?;
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
        let _lock = lock(&self.dir)?;
        let mut index = self.read_index()?;
        let previous = index.secrets.get(name).cloned();
        if previous.is_none() {
            ensure!(index.secrets.len() < MAX_SECRETS, "too many secrets");
        }
        // Read the file first so a corrupt store is reported before anything changes.
        let mut map = self.file.read()?;
        let mut keyring_error = None;
        let backend = match &self.keyring {
            Some(k) => match k.set(name, value) {
                Ok(()) => Backend::Keyring,
                Err(e) => {
                    keyring_error = Some(e);
                    Backend::File
                }
            },
            None => Backend::File,
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
        // still have landed (a D-Bus write may go out before the kill). Remember both so a later sweep deletes them.
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
        let _lock = lock(&self.dir)?;
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

fn migrate(secrets: &Secrets, delete_plaintext: bool) -> Result<()> {
    let name = CLOUDFLARE_TOKEN;
    // The plaintext token file `cx mesh setup` writes (mesh/mesh.py `Mesh.creds`).
    let path = crate::mesh::cloudflare::config_dir().join("credentials");
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
    use std::path::Path;

    const CANARY: &str = "cx-canary-7f3a9d1e-DO-NOT-LOG";

    fn temp() -> (tempfile::TempDir, PathBuf) {
        let t = tempfile::tempdir().unwrap();
        let dir = t.path().join("state");
        fs::create_dir(&dir).unwrap();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).unwrap();
        (t, dir)
    }

    fn file_store(dir: &Path) -> Secrets {
        Secrets::at(dir.to_path_buf(), None)
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
        for f in [STORE, INDEX, LOCK] {
            assert_eq!(mode(&dir.join(f)), 0o600, "{f}");
        }
        assert_eq!(mode(&dir), 0o700);
        // Exactly these files, no key file: the fallback is a plain private file.
        let mut names: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        assert_eq!(names, [INDEX, LOCK, STORE]);
        // A fresh handle (another process) reads the same file.
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
        let good = fs::read(dir.join(STORE)).unwrap();
        let wrong_version = String::from_utf8(good.clone())
            .unwrap()
            .replace("\"version\":1", "\"version\":2");
        // Truncated, not JSON, invalid UTF-8, and a version this build does not know: all refused, none replaced.
        for junk in [
            &good[..good.len() / 2],
            &b"not json at all"[..],
            &[0xff, 0xfe, b'{'][..],
            wrong_version.as_bytes(),
        ] {
            fs::write(dir.join(STORE), junk).unwrap();
            assert!(s.get("a").is_err());
            let err = format!("{:#}", s.set("b", "two").unwrap_err());
            assert!(
                err.contains("left untouched") || err.contains("unsupported version"),
                "{err}"
            );
            assert_eq!(fs::read(dir.join(STORE)).unwrap(), junk);
            assert!(s.meta("b").unwrap().is_none());
        }
        // A corrupt index holds no values and is reported, not replaced.
        fs::write(dir.join(STORE), &good).unwrap();
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
    fn value_file_must_be_private_and_is_never_followed() {
        let (_t, dir) = temp();
        let s = file_store(&dir);
        s.set("a", "one").unwrap();
        // Readable by others: refused for reads and writes, and left as it is.
        fs::set_permissions(dir.join(STORE), fs::Permissions::from_mode(0o644)).unwrap();
        let err = format!("{:#}", s.get("a").unwrap_err());
        assert!(err.contains("not a private file"), "{err}");
        assert!(s.set("b", "two").is_err());
        assert_eq!(mode(&dir.join(STORE)), 0o644);
        fs::set_permissions(dir.join(STORE), fs::Permissions::from_mode(0o600)).unwrap();
        // A planted symlink is refused, never read through, and a write never lands in its target.
        let elsewhere = dir.join("elsewhere.json");
        fs::rename(dir.join(STORE), &elsewhere).unwrap();
        std::os::unix::fs::symlink(&elsewhere, dir.join(STORE)).unwrap();
        assert!(s.get("a").is_err());
        assert!(s.set("b", "two").is_err());
        let before = fs::read(&elsewhere).unwrap();
        assert!(fs::symlink_metadata(dir.join(STORE))
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(fs::read(&elsewhere).unwrap(), before);
        fs::remove_file(dir.join(STORE)).unwrap();
        fs::rename(&elsewhere, dir.join(STORE)).unwrap();
        // Oversized: refused before it is parsed.
        let big = vec![b' '; STORE_LIMIT as usize + 1];
        let good = fs::read(dir.join(STORE)).unwrap();
        fs::write(dir.join(STORE), &big).unwrap();
        let err = format!("{:#}", s.get("a").unwrap_err());
        assert!(err.contains("size limit"), "{err}");
        fs::write(dir.join(STORE), &good).unwrap();
        assert_eq!(s.get("a").unwrap().expose(), "one");
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

    /// A fake `secret-tool` that keeps values in files and records its argv; `CX_FAKE_MODE=hang` makes it hang,
    /// and a `fail` file in its vault makes it fail (a locked keyring).
    const FAKE_SECRET_TOOL: &str = r#"#!/bin/sh
PATH=/usr/bin:/bin
d="$CX_FAKE_VAULT"
printf '%s\n' "$*" >> "$d/argv"
if [ "$CX_FAKE_MODE" = hang ]; then echo $$ > "$d/pid"; exec sleep 30; fi
[ -e "$d/fail" ] && exit 1
op=$1
for a; do name=$a; done
case $op in
store) cat > "$d/v.$name" ;;
lookup) [ -f "$d/v.$name" ] || exit 1; cat "$d/v.$name" ;;
clear) rm -f "$d/v.$name" ;;
*) exit 2 ;;
esac
"#;

    /// Runs `test` in a child with its own state, a fake session bus and `secret-tool` on PATH as `tool`
    /// (none when `tool` is None). Returns whether this is the child, and the vault dir.
    fn with_fake_tool(test: &str, tool: Option<&str>, mode: &str) -> (bool, PathBuf) {
        if std::env::var_os("CX_TEST_CHILD").is_some() {
            return (true, PathBuf::from(std::env::var("CX_FAKE_VAULT").unwrap()));
        }
        let t = tempfile::tempdir().unwrap();
        let root = t.path();
        fs::set_permissions(root, fs::Permissions::from_mode(0o700)).unwrap();
        let bin = root.join("bin");
        let vault = root.join("vault");
        fs::create_dir(&bin).unwrap();
        fs::create_dir(&vault).unwrap();
        if let Some(body) = tool {
            fs::write(bin.join("secret-tool"), body).unwrap();
            fs::set_permissions(bin.join("secret-tool"), fs::Permissions::from_mode(0o700))
                .unwrap();
        }
        let vars = [
            ("HOME", root.to_path_buf()),
            ("XDG_STATE_HOME", root.join("state")),
            ("PATH", bin.clone()),
            (
                "DBUS_SESSION_BUS_ADDRESS",
                PathBuf::from("unix:path=/nonexistent/cx-fake-bus"),
            ),
            ("CX_SECRETS_BACKEND", PathBuf::from("auto")),
            ("CX_FAKE_VAULT", vault.clone()),
            ("CX_FAKE_MODE", PathBuf::from(mode)),
        ];
        let env: Vec<(&str, &Path)> = vars.iter().map(|(k, v)| (*k, v.as_path())).collect();
        assert!(!crate::mesh::in_child(test, &env));
        // The parent checks what the child left: the argv the tool saw never holds the value.
        let argv = fs::read_to_string(vault.join("argv")).unwrap_or_default();
        assert!(!argv.contains(CANARY), "a value reached argv: {argv}");
        (false, vault)
    }

    #[test]
    fn secret_tool_gets_values_on_stdin_never_in_argv() {
        let (child, vault) = with_fake_tool(
            "secrets::tests::secret_tool_gets_values_on_stdin_never_in_argv",
            Some(FAKE_SECRET_TOOL),
            "",
        );
        if !child {
            return;
        }
        assert!(keyring_reachable());
        let s = Secrets::open().unwrap();
        assert_eq!(s.set("robot.x", CANARY).unwrap().backend, Backend::Keyring);
        assert_eq!(fs::read_to_string(vault.join("v.robot.x")).unwrap(), CANARY);
        assert_eq!(s.get("robot.x").unwrap().expose(), CANARY);
        s.delete("robot.x").unwrap();
        assert!(!vault.join("v.robot.x").exists());
        assert!(!fs::read_to_string(crate::store::state_dir().join(STORE))
            .unwrap_or_default()
            .contains(CANARY));
        let argv = fs::read_to_string(vault.join("argv")).unwrap();
        let calls: Vec<&str> = argv.lines().collect();
        assert_eq!(
            calls[0],
            "store --label=cx: robot.x service cx name robot.x"
        );
        assert!(calls.contains(&"lookup service cx name robot.x"), "{argv}");
        assert!(calls.contains(&"clear service cx name robot.x"), "{argv}");
        assert!(!argv.contains(CANARY));
    }

    #[test]
    fn a_missing_secret_tool_means_the_file() {
        let (child, vault) = with_fake_tool(
            "secrets::tests::a_missing_secret_tool_means_the_file",
            None,
            "",
        );
        if !child {
            return;
        }
        assert!(!keyring_reachable());
        let s = Secrets::open().unwrap();
        assert_eq!(s.set("robot.x", CANARY).unwrap().backend, Backend::File);
        assert_eq!(s.get("robot.x").unwrap().expose(), CANARY);
        assert!(!vault.join("argv").exists());
    }

    /// A hung `secret-tool` is given up on and killed at KEYRING_TIMEOUT (the value goes to the file), and not
    /// tried again in this process.
    #[test]
    fn a_hung_secret_tool_falls_back_and_is_killed() {
        let (child, vault) = with_fake_tool(
            "secrets::tests::a_hung_secret_tool_falls_back_and_is_killed",
            Some(FAKE_SECRET_TOOL),
            "hang",
        );
        if !child {
            return;
        }
        let s = Secrets::open().unwrap();
        let start = std::time::Instant::now();
        assert_eq!(s.set("robot.x", CANARY).unwrap().backend, Backend::File);
        let took = start.elapsed();
        assert!(
            took >= KEYRING_TIMEOUT && took < KEYRING_TIMEOUT + Duration::from_secs(1),
            "{took:?}"
        );
        assert!(s.keyring.as_ref().unwrap().is_hung());
        assert_eq!(s.get("robot.x").unwrap().expose(), CANARY);
        // Not tried again: the next value goes to the file at once. Both are noted in case a write landed.
        let start = std::time::Instant::now();
        assert_eq!(s.set("robot.y", "w").unwrap().backend, Backend::File);
        assert!(start.elapsed() < Duration::from_millis(500));
        assert_eq!(s.stale_keyring().unwrap(), ["robot.x", "robot.y"]);
        let pid: i32 = fs::read_to_string(vault.join("pid"))
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        let deadline = std::time::Instant::now() + Duration::from_millis(500);
        while unsafe { libc::kill(pid, 0) } == 0 {
            assert!(
                std::time::Instant::now() < deadline,
                "secret-tool {pid} left running"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(
            fs::read_to_string(vault.join("argv"))
                .unwrap()
                .lines()
                .count(),
            1,
            "not retried"
        );
    }

    /// Values in the vault of the fake `secret-tool`, by name.
    fn kept(vault: &Path, name: &str) -> Option<String> {
        fs::read_to_string(vault.join(format!("v.{name}"))).ok()
    }

    #[test]
    fn keyring_preferred_with_file_fallback_and_migration_between() {
        let (child, vault) = with_fake_tool(
            "secrets::tests::keyring_preferred_with_file_fallback_and_migration_between",
            Some(FAKE_SECRET_TOOL),
            "",
        );
        if !child {
            return;
        }
        let dir = crate::store::ensure().unwrap();
        let s = Secrets::open().unwrap();
        let meta = s.set("a", "in-keyring").unwrap();
        assert_eq!(meta.backend, Backend::Keyring);
        assert_eq!(kept(&vault, "a").as_deref(), Some("in-keyring"));
        assert!(
            !dir.join(STORE).exists(),
            "keyring values never reach the file"
        );
        assert_eq!(s.get("a").unwrap().expose(), "in-keyring");

        fs::write(vault.join("fail"), "").unwrap();
        let meta = s.set("b", "in-file").unwrap();
        assert_eq!(meta.backend, Backend::File);
        assert_eq!(s.get("b").unwrap().expose(), "in-file");
        assert!(
            s.get("a").is_err(),
            "keyring entry unreachable while locked"
        );

        fs::remove_file(vault.join("fail")).unwrap();
        // b moves to the keyring and leaves the file.
        assert_eq!(s.set("b", "moved").unwrap().backend, Backend::Keyring);
        assert!(file_store(&dir).file.read().unwrap().is_empty());
        s.delete("a").unwrap();
        assert!(kept(&vault, "a").is_none());
    }

    #[test]
    fn stranded_keyring_copy_is_deleted_once_the_keyring_answers() {
        let (child, vault) = with_fake_tool(
            "secrets::tests::stranded_keyring_copy_is_deleted_once_the_keyring_answers",
            Some(FAKE_SECRET_TOOL),
            "",
        );
        if !child {
            return;
        }
        let dir = crate::store::ensure().unwrap();
        let s = Secrets::open().unwrap();
        assert_eq!(s.set("a", "old").unwrap().backend, Backend::Keyring);

        // Another session without a keyring replaces the value: it lands in the file, the old copy is noted.
        let headless = file_store(&dir);
        assert_eq!(headless.set("a", "new").unwrap().backend, Backend::File);
        assert_eq!(headless.stale_keyring().unwrap(), ["a"]);
        assert_eq!(kept(&vault, "a").as_deref(), Some("old"));
        assert_eq!(headless.get("a").unwrap().expose(), "new");

        // A keyring that is reachable but failing: the value still goes to the file, the note stays, and b is
        // noted too in case the failed call landed after all.
        fs::write(vault.join("fail"), "").unwrap();
        assert_eq!(s.set("b", "x").unwrap().backend, Backend::File);
        assert_eq!(s.stale_keyring().unwrap(), ["a", "b"]);
        assert!(kept(&vault, "a").is_some());

        // Once the keyring answers, the next write removes the stranded copy; the file value is untouched.
        fs::remove_file(vault.join("fail")).unwrap();
        s.set("c", "y").unwrap();
        assert!(kept(&vault, "a").is_none());
        assert!(s.stale_keyring().unwrap().is_empty());
        assert_eq!(s.get("a").unwrap().expose(), "new");
        assert_eq!(s.meta("a").unwrap().unwrap().backend, Backend::File);

        // Moving back into the keyring clears the note without deleting the fresh keyring value.
        headless.set("d", "1").unwrap();
        s.set("d", "2").unwrap();
        s.set("e", "k").unwrap();
        headless.set("e", "f").unwrap();
        assert_eq!(headless.stale_keyring().unwrap(), ["e"]);
        s.set("e", "k2").unwrap();
        assert!(s.stale_keyring().unwrap().is_empty());
        assert_eq!(kept(&vault, "e").as_deref(), Some("k2"));
        s.delete("e").unwrap();
        assert!(kept(&vault, "e").is_none());
    }

    #[test]
    fn values_never_reach_logs_metadata_or_the_index() {
        let (child, vault) = with_fake_tool(
            "secrets::tests::values_never_reach_logs_metadata_or_the_index",
            Some(FAKE_SECRET_TOOL),
            "",
        );
        if !child {
            return;
        }
        let dir = crate::store::ensure().unwrap();
        let k = Secrets::open().unwrap();
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
        fs::write(vault.join("fail"), "").unwrap();
        seen.push(format!("{:#}", k.get("kr").unwrap_err()));
        let index = fs::read(dir.join(INDEX)).unwrap();
        for text in &seen {
            assert!(!text.contains(CANARY), "leaked in {text}");
            assert!(!text.contains(&CANARY[..12]), "partial leak in {text}");
        }
        assert!(!index.windows(12).any(|w| w == &CANARY.as_bytes()[..12]));
        // The fallback file holds the file-backend value in the clear (protected by its 0600 mode, like an
        // unencrypted SSH key) and never the keyring-backend one.
        let store = String::from_utf8(fs::read(dir.join(STORE)).unwrap()).unwrap();
        assert_eq!(store.matches(CANARY).count(), 1);
        assert_eq!(mode(&dir.join(STORE)), 0o600);
        assert!(seen[0].contains("redacted"));
    }
}
