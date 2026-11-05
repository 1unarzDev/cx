//! Dedicated SSH identity and verified pins for automatically paired core peers.
//! Private keys never leave this host; caller must authenticate peer metadata.
use crate::{model::Device, store};
use anyhow::{Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::{
        fd::AsRawFd,
        unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt},
    },
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};
const MAX: u64 = 1024 * 1024;
#[derive(Default, Serialize, Deserialize)]
struct Pins {
    endpoints: BTreeMap<String, Pin>,
}
#[derive(Serialize, Deserialize)]
struct Pin {
    device_id: String,
    host: String,
    key: String,
    client_key: String,
}
fn owned_dir(path: &Path) -> Result<()> {
    // Reject symlink traversal even when an ancestor points at an owned directory.
    for ancestor in path.ancestors() {
        if let Ok(meta) = fs::symlink_metadata(ancestor) {
            anyhow::ensure!(
                !meta.file_type().is_symlink(),
                "symlink in SSH directory path"
            );
        }
    }
    if !path.exists() {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(path)?;
    }
    let m = fs::symlink_metadata(path)?;
    anyhow::ensure!(
        m.is_dir() && m.uid() == unsafe { libc::geteuid() },
        "unsafe SSH directory"
    );
    anyhow::ensure!(
        m.mode() & 0o022 == 0,
        "SSH directory is writable by other users"
    );
    Ok(())
}
fn private_dir(path: &Path) -> Result<()> {
    owned_dir(path)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    Ok(())
}
fn open(path: &Path, append: bool) -> Result<File> {
    let f = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .append(append)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)?;
    let m = f.metadata()?;
    anyhow::ensure!(
        m.is_file()
            && m.uid() == unsafe { libc::geteuid() }
            && m.nlink() == 1
            && m.mode() & 0o022 == 0
            && m.len() <= MAX,
        "unsafe SSH file"
    );
    Ok(f)
}
fn read(path: &Path) -> Result<String> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(String::new()),
        Err(error) => return Err(error.into()),
        Ok(_) => {}
    }
    let mut s = String::new();
    open(path, false)?.take(MAX + 1).read_to_string(&mut s)?;
    anyhow::ensure!(s.len() as u64 <= MAX, "SSH file too large");
    Ok(s)
}

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Never buffer unbounded command output or block forever on key generation.
fn keygen(command: &mut Command, timeout: Duration) -> Result<Vec<u8>> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = command.spawn().context("cannot start ssh-keygen")?;
    let result = (|| -> Result<Vec<u8>> {
        let mut stdout = child.stdout.take().context("missing ssh-keygen output")?;
        let fd = stdout.as_raw_fd();
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        anyhow::ensure!(
            flags >= 0 && unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } >= 0,
            "cannot bound ssh-keygen output"
        );
        let deadline = Instant::now() + timeout;
        let mut output = Vec::new();
        loop {
            let mut bytes = [0; 1024];
            loop {
                match stdout.read(&mut bytes) {
                    Ok(0) => break,
                    Ok(n) => {
                        anyhow::ensure!(output.len() + n <= 4096, "ssh-keygen output too large");
                        output.extend_from_slice(&bytes[..n]);
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(error) => return Err(error.into()),
                }
            }
            if let Some(status) = child.try_wait()? {
                anyhow::ensure!(status.success(), "ssh-keygen failed");
                // Drain bytes written between the previous read and observed exit.
                loop {
                    match stdout.read(&mut bytes) {
                        Ok(0) => break,
                        Ok(n) => {
                            anyhow::ensure!(
                                output.len() + n <= 4096,
                                "ssh-keygen output too large"
                            );
                            output.extend_from_slice(&bytes[..n]);
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                        Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                        Err(error) => return Err(error.into()),
                    }
                }
                return Ok(output);
            }
            anyhow::ensure!(Instant::now() < deadline, "ssh-keygen timed out");
            std::thread::sleep(Duration::from_millis(10));
        }
    })();
    if result.is_err() {
        let _ = child.kill();
        let _ = child.wait();
    }
    result
}
fn private_key(path: &Path) -> Result<String> {
    let private = open(path, false)?;
    anyhow::ensure!(
        private.metadata()?.mode() & 0o077 == 0,
        "unsafe private key permissions"
    );
    let output = keygen(
        Command::new("ssh-keygen")
            .args(["-y", "-P", "", "-f"])
            .arg(path),
        Duration::from_secs(10),
    )?;
    public_key(std::str::from_utf8(&output)?)
}
struct Locked {
    root: PathBuf,
    _lock: File,
}
impl Locked {
    fn new(root: PathBuf) -> Result<Self> {
        // v0.1.75 could leave an empty group-writable root under a private state
        // directory. Recover only that harmless shape, never existing key material.
        if let Ok(metadata) = fs::symlink_metadata(&root) {
            if metadata.is_dir()
                && metadata.uid() == unsafe { libc::geteuid() }
                && metadata.mode() & 0o022 != 0
            {
                for ancestor in root.ancestors() {
                    if let Ok(meta) = fs::symlink_metadata(ancestor) {
                        anyhow::ensure!(
                            !meta.file_type().is_symlink(),
                            "symlink in SSH directory path"
                        );
                    }
                }
                let parent =
                    fs::symlink_metadata(root.parent().context("missing SSH state parent")?)?;
                anyhow::ensure!(
                    parent.is_dir()
                        && parent.uid() == unsafe { libc::geteuid() }
                        && parent.mode() & 0o077 == 0
                        && fs::read_dir(&root)?.next().is_none(),
                    "unsafe existing SSH state directory"
                );
                fs::set_permissions(&root, fs::Permissions::from_mode(0o700))?;
            }
        }
        private_dir(&root)?;
        let f = open(&root.join("lock"), false)?;
        anyhow::ensure!(
            unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX) } == 0,
            "cannot lock peer SSH setup"
        );
        Ok(Self { root, _lock: f })
    }
    fn pins(&self) -> Result<Pins> {
        let text = read(&self.root.join("peers.json"))?;
        if text.is_empty() {
            Ok(Pins::default())
        } else {
            Ok(serde_json::from_str(&text)?)
        }
    }
    fn write(&self, name: &str, text: &str) -> Result<()> {
        anyhow::ensure!(text.len() as u64 <= MAX, "SSH file too large");
        anyhow::ensure!(
            !name.is_empty() && !name.contains('/') && name != "." && name != "..",
            "invalid managed SSH filename"
        );
        let destination = self.root.join(name);
        // Validate any existing file, including dangling symlinks, before replacement.
        let _ = read(&destination)?;
        let (temporary, mut file) = (0..64)
            .find_map(|_| {
                let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
                let path =
                    self.root
                        .join(format!(".{name}.{}.{}.tmp", std::process::id(), sequence));
                match OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(0o600)
                    .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
                    .open(&path)
                {
                    Ok(file) => Some(Ok((path, file))),
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => None,
                    Err(error) => Some(Err(error)),
                }
            })
            .context("cannot reserve managed SSH temporary file")??;
        let result = (|| -> Result<()> {
            file.write_all(text.as_bytes())?;
            file.sync_all()?;
            fs::rename(&temporary, &destination)?;
            File::open(&self.root)?.sync_all()?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }
    fn identity(&self) -> Result<String> {
        let key = self.root.join("id_ed25519");
        match fs::symlink_metadata(&key) {
            Ok(_) => return private_key(&key),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        // A completed staging private key survives interruption and is reused.
        // Unknown contents are left alone; neither retries nor cleanup remove them.
        let staging = self.root.join("generate");
        private_dir(&staging)?;
        let staged_key = staging.join("identity");
        match fs::symlink_metadata(&staged_key) {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                anyhow::ensure!(
                    fs::symlink_metadata(staging.join("identity.pub"))
                        .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound),
                    "staged public key has no private key; refusing identity rotation"
                );
                keygen(
                    Command::new("ssh-keygen")
                        .args(["-q", "-t", "ed25519", "-N", "", "-C", "cx-peer"])
                        .arg("-f")
                        .arg(&staged_key),
                    Duration::from_secs(10),
                )?;
            }
            Err(error) => return Err(error.into()),
        }
        let public = private_key(&staged_key)
            .context("staged SSH key is incomplete; preserved without rotation")?;
        open(&staged_key, false)?.sync_all()?;
        // The lock serializes CX writers. Existing final paths are never replaced.
        let source = std::ffi::CString::new(staged_key.as_os_str().as_encoded_bytes())?;
        let destination = std::ffi::CString::new(key.as_os_str().as_encoded_bytes())?;
        anyhow::ensure!(
            unsafe {
                libc::syscall(
                    libc::SYS_renameat2,
                    libc::AT_FDCWD,
                    source.as_ptr(),
                    libc::AT_FDCWD,
                    destination.as_ptr(),
                    libc::RENAME_NOREPLACE,
                )
            } == 0,
            "cannot install staged SSH identity without replacing existing file"
        );
        File::open(&self.root)?.sync_all()?;
        // This is only a convenience public copy; derive the authoritative value
        // from the private key on every call, including after an interrupted write.
        self.write("id_ed25519.pub", &format!("{public}\n"))?;
        Ok(public)
    }
}
fn root() -> PathBuf {
    store::state_dir().join("peer-ssh")
}
fn public_key(input: &str) -> Result<String> {
    anyhow::ensure!(
        input.len() <= 1024 && !input.contains(['\r', '\0']) && input.trim().lines().count() == 1,
        "invalid Ed25519 public key"
    );
    let mut fields = input.split_whitespace();
    anyhow::ensure!(
        fields.next() == Some("ssh-ed25519"),
        "expected Ed25519 public key"
    );
    let encoded = fields.next().context("missing Ed25519 public key")?;
    let bytes = STANDARD
        .decode(encoded)
        .context("invalid public key encoding")?;
    anyhow::ensure!(
        bytes.len() == 51
            && &bytes[..4] == [0, 0, 0, 11]
            && &bytes[4..15] == b"ssh-ed25519"
            && &bytes[15..19] == [0, 0, 0, 32],
        "invalid Ed25519 key data"
    );
    Ok(format!("ssh-ed25519 {}", STANDARD.encode(bytes)))
}
fn endpoint(target: &str) -> Result<(&str, &str)> {
    anyhow::ensure!(target.len() <= 320, "SSH endpoint too long");
    let (account, host) = target
        .split_once('@')
        .context("pairing requires account@hostname")?;
    anyhow::ensure!(
        !account.is_empty()
            && !account.starts_with('-')
            && account
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b)),
        "invalid SSH account"
    );
    if host.starts_with('[') && host.ends_with(']') {
        host[1..host.len() - 1]
            .parse::<std::net::Ipv6Addr>()
            .context("invalid IPv6 SSH host")?;
    } else {
        anyhow::ensure!(
            !host.is_empty()
                && !host.starts_with(['-', '.'])
                && host
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b".-".contains(&b)),
            "invalid SSH hostname"
        );
    }
    Ok((account, host))
}
/// Public metadata authenticated through the existing CX helper connection.
pub fn identity() -> Result<Value> {
    let locked = Locked::new(root())?;
    let host = fs::symlink_metadata("/etc/ssh/ssh_host_ed25519_key.pub")?;
    anyhow::ensure!(
        host.is_file() && host.len() <= 1024 && host.uid() == 0 && host.mode() & 0o022 == 0,
        "unsafe SSH host public key"
    );
    let host_key = public_key(&fs::read_to_string("/etc/ssh/ssh_host_ed25519_key.pub")?)?;
    Ok(json!({"client_key": locked.identity()?, "host_key": host_key}))
}
pub fn configure(
    peer: &Device,
    host_key: &str,
    authorized_key: &str,
    authorize: bool,
) -> Result<Value> {
    let home = std::env::var_os("HOME").context("HOME is unavailable")?;
    configure_at(
        root(),
        &PathBuf::from(home).join(".ssh"),
        peer,
        host_key,
        authorized_key,
        authorize,
    )
}
fn managed_line(key: &str) -> String {
    format!(
        "no-agent-forwarding,no-port-forwarding,no-X11-forwarding,no-user-rc {key} cx-managed-peer"
    )
}
fn names_key(line: &str, key: &str) -> bool {
    !line.trim_start().starts_with('#')
        && line
            .split_whitespace()
            .collect::<Vec<_>>()
            .windows(2)
            .any(|w| format!("{} {}", w[0], w[1]) == key)
}
fn bare(line: &str) -> &str {
    let line = line.strip_suffix('\n').unwrap_or(line);
    line.strip_suffix('\r').unwrap_or(line)
}
/// Append the exact restricted cx-managed line once. Returns whether it was added.
fn authorize_managed(ssh: &Path, key: &str) -> Result<bool> {
    let authorized = read(&ssh.join("authorized_keys"))?;
    let managed = managed_line(key);
    let mut present = false;
    for line in authorized.lines().filter(|line| names_key(line, key)) {
        anyhow::ensure!(line == managed, "existing managed-key authorization has different restrictions; refusing to override it");
        present = true;
    }
    if present {
        return Ok(false);
    }
    let prefix = if !authorized.is_empty() && !authorized.ends_with('\n') {
        "\n"
    } else {
        ""
    };
    let addition = format!("{prefix}{managed}\n");
    anyhow::ensure!(
        (authorized.len() + addition.len()) as u64 <= MAX,
        "authorized_keys too large"
    );
    // Append only: unrelated keys and their options remain byte-for-byte intact.
    let mut file = open(&ssh.join("authorized_keys"), true)?;
    file.write_all(addition.as_bytes())?;
    file.sync_all()?;
    Ok(true)
}
/// Remove only exact cx-managed lines for `key`. Every other line, including a
/// user's own line for the same key, stays byte-for-byte. Returns
/// (removed managed lines, user line for this key remains).
fn revoke_managed(ssh: &Path, key: &str) -> Result<(usize, bool)> {
    let path = ssh.join("authorized_keys");
    let text = read(&path)?;
    let managed = managed_line(key);
    let mut removed = 0;
    let mut user = false;
    let mut kept = String::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        if bare(line) == managed {
            removed += 1;
            continue;
        }
        user |= names_key(bare(line), key);
        kept.push_str(line);
    }
    if removed == 0 {
        return Ok((0, user));
    }
    let mode = open(&path, false)?.metadata()?.mode() & 0o777;
    let (temporary, mut file) = (0..64)
        .find_map(|_| {
            let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let path = ssh.join(format!(
                ".authorized_keys.cx.{}.{}.tmp",
                std::process::id(),
                sequence
            ));
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
                .open(&path)
            {
                Ok(file) => Some(Ok((path, file))),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => None,
                Err(error) => Some(Err(error)),
            }
        })
        .context("cannot reserve authorized_keys temporary file")??;
    let result = (|| -> Result<()> {
        file.write_all(kept.as_bytes())?;
        file.set_permissions(fs::Permissions::from_mode(mode))?;
        file.sync_all()?;
        fs::rename(&temporary, &path)?;
        File::open(ssh)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result?;
    Ok((removed, user))
}
/// Change the cx-managed authorization for one pinned peer. The key is the
/// explicit client key when given, otherwise the pinned key for `peer_id`.
/// Authorizing requires an existing verified pin and `consent` for the pin's
/// device id; revoking never does and never creates peer SSH state.
pub fn authorization(
    peer_id: Option<&str>,
    client_key: Option<&str>,
    authorize: bool,
    consent: impl Fn(&str) -> Result<()>,
) -> Result<Value> {
    let home = std::env::var_os("HOME").context("HOME is unavailable")?;
    authorization_at(
        root(),
        &PathBuf::from(home).join(".ssh"),
        peer_id,
        client_key,
        authorize,
        consent,
    )
}
fn authorization_at(
    root: PathBuf,
    ssh: &Path,
    peer_id: Option<&str>,
    client_key: Option<&str>,
    authorize: bool,
    consent: impl Fn(&str) -> Result<()>,
) -> Result<Value> {
    anyhow::ensure!(
        peer_id.is_some() || client_key.is_some(),
        "select a peer by identity or client key"
    );
    if let Some(id) = peer_id {
        anyhow::ensure!(
            !id.is_empty() && id.len() <= 256 && !id.chars().any(char::is_control),
            "invalid peer identity"
        );
    }
    let client_key = client_key.map(public_key).transpose()?;
    // Held until return so authorized_keys changes serialize with configure.
    let locked = if root.try_exists()? {
        Some(Locked::new(root)?)
    } else {
        None
    };
    let pins = match &locked {
        Some(locked) => locked.pins()?,
        None => Pins::default(),
    };
    let matching = |pin: &&Pin| {
        peer_id.is_none_or(|id| pin.device_id == id)
            && client_key.as_ref().is_none_or(|key| &pin.client_key == key)
    };
    let pinned = pins.endpoints.values().find(matching);
    let key = client_key.or_else(|| pinned.map(|pin| pin.client_key.clone()));
    let Some(key) = key else {
        anyhow::ensure!(
            !authorize,
            "no verified cx pairing for this peer; run core sync first"
        );
        return Ok(json!({"pinned": false, "managed_key_installed": false,
            "user_authorization_present": false, "changed": false}));
    };
    let changed = if authorize {
        let pin = pinned.context("no verified cx pairing for this peer; run core sync first")?;
        consent(&pin.device_id)?;
        private_dir(ssh)?;
        authorize_managed(ssh, &key)?
    } else if ssh.try_exists()? {
        owned_dir(ssh)?;
        revoke_managed(ssh, &key)?.0 > 0
    } else {
        false
    };
    let text = if ssh.try_exists()? {
        read(&ssh.join("authorized_keys"))?
    } else {
        String::new()
    };
    let managed = managed_line(&key);
    let installed = text.lines().any(|line| line == managed);
    let user = text
        .lines()
        .any(|line| line != managed && names_key(line, &key));
    Ok(
        json!({"pinned": pinned.is_some(), "managed_key_installed": installed,
        "user_authorization_present": user, "changed": changed}),
    )
}
fn configure_at(
    root: PathBuf,
    ssh: &Path,
    peer: &Device,
    host_key: &str,
    authorized_key: &str,
    authorize: bool,
) -> Result<Value> {
    let target = peer.target.as_deref().context("cannot pair local device")?;
    let (account, host) = endpoint(target)?;
    anyhow::ensure!(
        account == peer.account
            && peer.id != "local"
            && peer.id.len() <= 256
            && peer.id.ends_with(&format!(":{}", peer.account))
            && !peer.id.chars().any(char::is_control),
        "invalid peer identity"
    );
    let host_key = public_key(host_key)?;
    let authorized_key = public_key(authorized_key)?;
    let locked = Locked::new(root)?;
    let mut pins = locked.pins()?;
    for (existing_target, pin) in &pins.endpoints {
        anyhow::ensure!(
            pin.device_id != peer.id || (pin.key == host_key && pin.client_key == authorized_key),
            "peer SSH identity keys changed; verify identity before re-pairing"
        );
        anyhow::ensure!(
            pin.host != host || pin.key == host_key,
            "peer SSH host key changed; verify identity before re-pairing"
        );
        anyhow::ensure!(
            existing_target != target || pin.device_id == peer.id,
            "SSH endpoint belongs to another device"
        );
    }
    // Validate all destinations before making changes.
    private_dir(ssh)?;
    let _ = read(&ssh.join("authorized_keys"))?;
    let _ = read(&locked.root.join("known_hosts"))?;
    let _ = locked.identity()?;
    if authorize {
        authorize_managed(ssh, &authorized_key)?;
    } else {
        // One-way: this host may reach the peer, but the peer may not log in here.
        revoke_managed(ssh, &authorized_key)?;
    }
    pins.endpoints.insert(
        target.to_owned(),
        Pin {
            device_id: peer.id.clone(),
            host: host.to_owned(),
            key: host_key,
            client_key: authorized_key,
        },
    );
    let hosts: BTreeMap<_, _> = pins.endpoints.values().map(|p| (&p.host, &p.key)).collect();
    let known = hosts
        .into_iter()
        .map(|(host, key)| {
            format!(
                "{} {key}\n",
                host.trim_start_matches('[').trim_end_matches(']')
            )
        })
        .collect::<String>();
    locked.write("known_hosts", &known)?;
    locked.write("peers.json", &serde_json::to_string(&pins)?)?;
    Ok(json!({"configured": true, "target": target, "authorized": authorize}))
}
/// This host's cx public client key, if one was ever generated. Never creates one.
pub fn client_key() -> Result<Option<String>> {
    if !root().try_exists()? {
        return Ok(None);
    }
    let locked = Locked::new(root())?;
    match fs::symlink_metadata(locked.root.join("id_ed25519")) {
        Ok(_) => Ok(Some(private_key(&locked.root.join("id_ed25519"))?)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}
/// Only exact endpoints enrolled by authenticated pairing receive this identity.
pub fn options(target: &str) -> Result<Option<(PathBuf, PathBuf)>> {
    if !root().try_exists()? {
        return Ok(None);
    }
    let locked = Locked::new(root())?;
    if !locked.pins()?.endpoints.contains_key(target) {
        return Ok(None);
    }
    let _ = endpoint(target)?;
    let private = open(&locked.root.join("id_ed25519"), false)?;
    anyhow::ensure!(
        private.metadata()?.mode() & 0o077 == 0,
        "unsafe private key permissions"
    );
    let _ = read(&locked.root.join("known_hosts"))?;
    Ok(Some((
        locked.root.join("id_ed25519"),
        locked.root.join("known_hosts"),
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn key(value: u8) -> String {
        let mut data = vec![0, 0, 0, 11];
        data.extend_from_slice(b"ssh-ed25519");
        data.extend_from_slice(&[0, 0, 0, 32]);
        data.extend_from_slice(&[value; 32]);
        format!("ssh-ed25519 {}", STANDARD.encode(data))
    }
    fn peer() -> Device {
        Device {
            id: "machine:alice".into(),
            name: "peer".into(),
            target: Some("alice@peer.example".into()),
            account: "alice".into(),
            host: "peer".into(),
            status: "unknown".into(),
            observed_at: 0,
        }
    }
    #[test]
    fn recovers_only_empty_legacy_root_beneath_private_parent() {
        let temp = tempfile::tempdir().unwrap();
        fs::set_permissions(temp.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let root = temp.path().join("legacy");
        fs::create_dir(&root).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o775)).unwrap();
        let locked = Locked::new(root.clone()).unwrap();
        assert_eq!(fs::metadata(&root).unwrap().mode() & 0o777, 0o700);
        drop(locked);
        fs::set_permissions(&root, fs::Permissions::from_mode(0o775)).unwrap();
        assert!(Locked::new(root.clone()).is_err()); // lock file exists: not empty
        assert_eq!(fs::metadata(&root).unwrap().mode() & 0o777, 0o775);
        fs::remove_file(root.join("lock")).unwrap();
        fs::set_permissions(temp.path(), fs::Permissions::from_mode(0o755)).unwrap();
        assert!(Locked::new(root).is_err()); // parent no longer private
    }

    #[test]
    fn validates_public_key_wire_format_and_endpoint() {
        assert!(public_key(&key(1)).is_ok());
        assert!(public_key("ssh-ed25519 AAAA").is_err());
        assert!(public_key(&format!("{}\n{}", key(1), key(2))).is_err());
        assert!(endpoint("alice@[2001:db8::1]").is_ok());
        for target in [
            "alias",
            "alice@-option",
            "alice@host:22",
            "alice@host\ncommand",
            "alice@host@evil",
            "-x@host",
        ] {
            assert!(endpoint(target).is_err(), "{target}");
        }
    }
    #[test]
    fn commented_keys_are_not_authorization_and_restricted_keys_are_preserved() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("peer-ssh");
        let ssh = temp.path().join("ssh");
        private_dir(&ssh).unwrap();
        let comment = format!("# {} disabled\n", key(2));
        fs::write(ssh.join("authorized_keys"), &comment).unwrap();
        configure_at(root.clone(), &ssh, &peer(), &key(1), &key(2), true).unwrap();
        let authorized = fs::read_to_string(ssh.join("authorized_keys")).unwrap();
        assert!(authorized.starts_with(&comment));
        assert!(authorized.contains("no-user-rc ssh-ed25519"));
        let restricted = format!("command=\"false\" {} restricted\n", key(2));
        fs::write(ssh.join("authorized_keys"), &restricted).unwrap();
        assert!(configure_at(root, &ssh, &peer(), &key(1), &key(2), true).is_err());
        assert_eq!(
            fs::read_to_string(ssh.join("authorized_keys")).unwrap(),
            restricted
        );
    }

    #[test]
    fn pairing_preserves_keys_and_is_idempotent_with_private_files() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("peer-ssh");
        let ssh = temp.path().join("ssh");
        private_dir(&ssh).unwrap();
        let old = format!("restrict {} unrelated\n", key(3));
        fs::write(ssh.join("authorized_keys"), &old).unwrap();
        fs::set_permissions(
            ssh.join("authorized_keys"),
            fs::Permissions::from_mode(0o644),
        )
        .unwrap();
        configure_at(root.clone(), &ssh, &peer(), &key(1), &key(2), true).unwrap();
        let authorized = fs::read_to_string(ssh.join("authorized_keys")).unwrap();
        assert!(authorized.starts_with(&old));
        assert!(authorized.contains(
            "no-agent-forwarding,no-port-forwarding,no-X11-forwarding,no-user-rc ssh-ed25519"
        ));
        configure_at(root.clone(), &ssh, &peer(), &key(1), &key(2), true).unwrap();
        assert_eq!(
            fs::read_to_string(ssh.join("authorized_keys")).unwrap(),
            authorized
        );
        assert_eq!(
            fs::metadata(root.join("id_ed25519")).unwrap().mode() & 0o777,
            0o600
        );
        assert_eq!(fs::metadata(&root).unwrap().mode() & 0o777, 0o700);
        assert_eq!(
            fs::read_to_string(root.join("known_hosts")).unwrap(),
            format!("peer.example {}\n", key(1))
        );
        let before = fs::read(root.join("known_hosts")).unwrap();
        assert!(configure_at(root.clone(), &ssh, &peer(), &key(4), &key(2), true).is_err());
        assert_eq!(fs::read(root.join("known_hosts")).unwrap(), before);
        let mut other = peer();
        other.id = "other:alice".into();
        assert!(configure_at(root, &ssh, &other, &key(1), &key(2), true).is_err());
    }
    #[test]
    fn edge_revocation_removes_only_cx_managed_lines_and_preserves_user_lines() {
        let ok = |_: &str| -> Result<()> { Ok(()) };
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("peer-ssh");
        let ssh = temp.path().join("ssh");
        private_dir(&ssh).unwrap();
        let user = format!(
            "# keep me\nrestrict {} unrelated\nfrom=\"10.0.0.1\" {} my-own-line\n",
            key(3),
            key(2)
        );
        fs::write(ssh.join("authorized_keys"), &user).unwrap();
        fs::set_permissions(
            ssh.join("authorized_keys"),
            fs::Permissions::from_mode(0o640),
        )
        .unwrap();
        // A user's own line for the same key blocks cx from adding a differing one.
        assert!(configure_at(root.clone(), &ssh, &peer(), &key(1), &key(2), true).is_err());
        fs::write(
            ssh.join("authorized_keys"),
            format!("restrict {} unrelated\n", key(3)),
        )
        .unwrap();
        configure_at(root.clone(), &ssh, &peer(), &key(1), &key(2), true).unwrap();
        let installed =
            authorization_at(root.clone(), &ssh, Some("machine:alice"), None, true, ok).unwrap();
        assert_eq!(installed["managed_key_installed"], true);
        assert_eq!(installed["pinned"], true);
        assert_eq!(installed["changed"], false);
        let mut text = fs::read_to_string(ssh.join("authorized_keys")).unwrap();
        text.push_str(&format!(
            "from=\"10.0.0.1\" {} my-own-line\n# trailing comment\n",
            key(2)
        ));
        fs::write(ssh.join("authorized_keys"), &text).unwrap();
        fs::set_permissions(
            ssh.join("authorized_keys"),
            fs::Permissions::from_mode(0o640),
        )
        .unwrap();
        let revoked =
            authorization_at(root.clone(), &ssh, Some("machine:alice"), None, false, ok).unwrap();
        assert_eq!(revoked["changed"], true);
        assert_eq!(revoked["managed_key_installed"], false);
        assert_eq!(revoked["user_authorization_present"], true);
        let after = fs::read_to_string(ssh.join("authorized_keys")).unwrap();
        assert_eq!(
            after,
            text.replace(&format!("{}\n", managed_line(&key(2))), "")
        );
        assert!(after.contains("my-own-line") && after.contains("unrelated"));
        assert_eq!(
            fs::metadata(ssh.join("authorized_keys")).unwrap().mode() & 0o777,
            0o640
        );
        // Idempotent; revoking an unknown peer never errors.
        let again =
            authorization_at(root.clone(), &ssh, Some("machine:alice"), None, false, ok).unwrap();
        assert_eq!(again["changed"], false);
        let unknown =
            authorization_at(root.clone(), &ssh, Some("nobody:x"), None, false, ok).unwrap();
        assert_eq!(unknown["pinned"], false);
        assert!(authorization_at(root.clone(), &ssh, Some("nobody:x"), None, true, ok).is_err());
        // Re-authorizing a pinned peer by client key restores exactly one managed line.
        fs::write(
            ssh.join("authorized_keys"),
            format!("restrict {} unrelated\n", key(3)),
        )
        .unwrap();
        // The receiver's consent is checked for the pin's device id, even when
        // the request selects the peer only by client key.
        let refuse = |id: &str| -> Result<()> {
            assert_eq!(id, "machine:alice");
            anyhow::bail!("receiver keeps this peer directed")
        };
        assert!(authorization_at(root.clone(), &ssh, None, Some(&key(2)), true, refuse).is_err());
        assert!(!fs::read_to_string(ssh.join("authorized_keys"))
            .unwrap()
            .contains("cx-managed-peer"));
        let restored = authorization_at(root.clone(), &ssh, None, Some(&key(2)), true, ok).unwrap();
        assert_eq!(restored["managed_key_installed"], true);
        authorization_at(root.clone(), &ssh, None, Some(&key(2)), true, ok).unwrap();
        let text = fs::read_to_string(ssh.join("authorized_keys")).unwrap();
        assert_eq!(text.matches("cx-managed-peer").count(), 1);
        // One-way configure pins the peer without authorizing it, and revokes.
        configure_at(root.clone(), &ssh, &peer(), &key(1), &key(2), false).unwrap();
        let text = fs::read_to_string(ssh.join("authorized_keys")).unwrap();
        assert!(!text.contains("cx-managed-peer") && text.contains("unrelated"));
        assert!(fs::read_to_string(root.join("known_hosts"))
            .unwrap()
            .contains(&key(1)));
        assert!(authorization_at(root, &ssh, None, Some("not a key"), false, ok).is_err());
        // Revoking on a host that never paired creates no peer SSH state.
        let fresh = temp.path().join("never-paired");
        let none =
            authorization_at(fresh.clone(), &ssh, Some("nobody:x"), None, false, ok).unwrap();
        assert_eq!(none["changed"], false);
        assert!(!fresh.exists());
        assert!(authorization_at(fresh.clone(), &ssh, None, Some(&key(2)), true, ok).is_err());
        assert!(!fresh.exists());
    }

    #[test]
    fn refuses_symlink_and_shared_writable_files() {
        use std::os::unix::fs::symlink;
        let temp = tempfile::tempdir().unwrap();
        let ssh = temp.path().join("ssh");
        private_dir(&ssh).unwrap();
        let destination = temp.path().join("unrelated");
        fs::write(&destination, "untouched").unwrap();
        symlink(&destination, ssh.join("authorized_keys")).unwrap();
        assert!(configure_at(
            temp.path().join("peer-ssh"),
            &ssh,
            &peer(),
            &key(1),
            &key(2),
            true
        )
        .is_err());
        assert_eq!(fs::read_to_string(&destination).unwrap(), "untouched");
        fs::remove_file(ssh.join("authorized_keys")).unwrap();
        fs::write(ssh.join("authorized_keys"), "").unwrap();
        fs::set_permissions(
            ssh.join("authorized_keys"),
            fs::Permissions::from_mode(0o666),
        )
        .unwrap();
        assert!(configure_at(
            temp.path().join("peer-ssh"),
            &ssh,
            &peer(),
            &key(1),
            &key(2),
            true
        )
        .is_err());
        symlink(&ssh, temp.path().join("alias")).unwrap();
        assert!(Locked::new(temp.path().join("alias/subdir")).is_err());
    }
    #[test]
    fn atomic_replacement_preserves_old_reader_and_orphan_staging() {
        use std::os::unix::fs::symlink;
        let temp = tempfile::tempdir().unwrap();
        let locked = Locked::new(temp.path().join("peer-ssh")).unwrap();
        locked.write("known_hosts", "old complete pin\n").unwrap();
        let mut old_reader = File::open(locked.root.join("known_hosts")).unwrap();
        // Simulate a crash leaving an unpublished temporary file.
        let orphan = locked.root.join(".known_hosts.crashed.tmp");
        fs::write(&orphan, "partial new pin").unwrap();
        locked.write("known_hosts", "new complete pin\n").unwrap();
        let mut old = String::new();
        old_reader.read_to_string(&mut old).unwrap();
        assert_eq!(old, "old complete pin\n");
        assert_eq!(
            read(&locked.root.join("known_hosts")).unwrap(),
            "new complete pin\n"
        );
        assert_eq!(fs::read_to_string(orphan).unwrap(), "partial new pin");
        assert_eq!(
            fs::metadata(locked.root.join("known_hosts"))
                .unwrap()
                .mode()
                & 0o777,
            0o600
        );
        assert!(locked
            .write("known_hosts", &"x".repeat(MAX as usize + 1))
            .is_err());
        assert_eq!(
            read(&locked.root.join("known_hosts")).unwrap(),
            "new complete pin\n"
        );
        let dangling = locked.root.join("peers.json");
        symlink(locked.root.join("missing"), &dangling).unwrap();
        assert!(locked.write("peers.json", "{}").is_err());
        assert!(fs::symlink_metadata(dangling)
            .unwrap()
            .file_type()
            .is_symlink());
    }
    #[test]
    fn interrupted_identity_generation_reuses_private_key_without_cleanup() {
        let temp = tempfile::tempdir().unwrap();
        let locked = Locked::new(temp.path().join("peer-ssh")).unwrap();
        let staging = locked.root.join("generate");
        private_dir(&staging).unwrap();
        fs::write(staging.join("unrelated"), "preserve").unwrap();
        keygen(
            Command::new("ssh-keygen")
                .args(["-q", "-t", "ed25519", "-N", "", "-f"])
                .arg(staging.join("identity")),
            Duration::from_secs(10),
        )
        .unwrap();
        let public = private_key(&staging.join("identity")).unwrap();
        let private = fs::read(staging.join("identity")).unwrap();
        // Public staging file exists, as after a kill before final-key promotion.
        assert_eq!(locked.identity().unwrap(), public);
        assert_eq!(fs::read(locked.root.join("id_ed25519")).unwrap(), private);
        assert_eq!(
            fs::read_to_string(staging.join("unrelated")).unwrap(),
            "preserve"
        );
        fs::remove_file(locked.root.join("id_ed25519.pub")).unwrap();
        // A crash after promotion but before the convenience public copy is harmless.
        assert_eq!(locked.identity().unwrap(), public);
        assert_eq!(fs::read(locked.root.join("id_ed25519")).unwrap(), private);
    }
    #[test]
    fn empty_staging_retries_and_incomplete_key_fails_without_rotation() {
        let temp = tempfile::tempdir().unwrap();
        let locked = Locked::new(temp.path().join("peer-ssh")).unwrap();
        private_dir(&locked.root.join("generate")).unwrap();
        let first = locked.identity().unwrap();
        assert_eq!(first, locked.identity().unwrap());
        let other = Locked::new(temp.path().join("other-peer-ssh")).unwrap();
        private_dir(&other.root.join("generate")).unwrap();
        let path = other.root.join("generate/identity");
        fs::write(&path, "partial private key").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        assert!(other.identity().is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "partial private key");
        assert!(!other.root.join("id_ed25519").exists());
    }
    #[test]
    fn concurrent_configuration_serializes_and_keeps_one_identity() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("peer-ssh");
        let ssh = temp.path().join("ssh");
        // Real startup races include concurrent directory creation.
        let handles: Vec<_> = (0..6)
            .map(|_| {
                let root = root.clone();
                let ssh = ssh.clone();
                std::thread::spawn(move || {
                    configure_at(root.clone(), &ssh, &peer(), &key(1), &key(2), true).unwrap();
                    Locked::new(root).unwrap().identity().unwrap()
                })
            })
            .collect();
        let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        assert!(results.iter().all(|r| r == &results[0]));
        let contents = read(&ssh.join("authorized_keys")).unwrap();
        assert_eq!(contents.lines().count(), 1);
        assert_eq!(
            Locked::new(root.clone())
                .unwrap()
                .pins()
                .unwrap()
                .endpoints
                .len(),
            1
        );
        assert_eq!(read(&root.join("known_hosts")).unwrap().lines().count(), 1);
    }
    #[test]
    fn keygen_time_and_output_are_bounded() {
        let start = Instant::now();
        assert!(keygen(Command::new("sleep").arg("10"), Duration::from_millis(30)).is_err());
        assert!(start.elapsed() < Duration::from_secs(2));
        assert!(keygen(
            Command::new("head").args(["-c", "5000", "/dev/zero"]),
            Duration::from_secs(2)
        )
        .is_err());
    }
}
