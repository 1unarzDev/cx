//! Dedicated SSH identity and verified pins for automatically paired core peers.
//! Private keys never leave this host; caller must authenticate peer metadata.
use crate::{model::Device, store};
use anyhow::{bail, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::{
        fd::AsRawFd,
        unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    },
    path::{Path, PathBuf},
    process::Command,
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
        fs::create_dir_all(path)?;
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
    if !path.try_exists()? {
        return Ok(String::new());
    }
    let mut s = String::new();
    open(path, false)?.take(MAX + 1).read_to_string(&mut s)?;
    anyhow::ensure!(s.len() as u64 <= MAX, "SSH file too large");
    Ok(s)
}
struct Locked {
    root: PathBuf,
    _lock: File,
}
impl Locked {
    fn new(root: PathBuf) -> Result<Self> {
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
        let mut f = open(&self.root.join(name), false)?;
        f.set_len(0)?;
        f.write_all(text.as_bytes())?;
        f.sync_all()?;
        Ok(())
    }
    fn identity(&self) -> Result<String> {
        let key = self.root.join("id_ed25519");
        if let Ok(meta) = fs::symlink_metadata(&key) {
            anyhow::ensure!(
                !meta.file_type().is_symlink(),
                "unsafe SSH private key symlink"
            );
        }
        if !key.try_exists()? {
            // Reserve the filename safely; ssh-keygen refuses existing files, so generate
            // in a private staging directory while holding the per-host lock.
            let staging = self.root.join("generate");
            fs::create_dir(&staging)
                .context("unfinished SSH identity generation; inspect peer-ssh/generate")?;
            fs::set_permissions(&staging, fs::Permissions::from_mode(0o700))?;
            let output = Command::new("ssh-keygen")
                .args(["-q", "-t", "ed25519", "-N", "", "-C", "cx-peer"])
                .arg("-f")
                .arg(staging.join("identity"))
                .output()?;
            if !output.status.success() {
                let _ = fs::remove_dir_all(&staging);
                bail!("cannot generate CX peer SSH identity");
            }
            fs::rename(staging.join("identity"), &key)?;
            fs::rename(
                staging.join("identity.pub"),
                self.root.join("id_ed25519.pub"),
            )?;
            fs::remove_dir(&staging)?;
        }
        let private = open(&key, false)?;
        anyhow::ensure!(
            private.metadata()?.mode() & 0o077 == 0,
            "unsafe private key permissions"
        );
        // Derive from private key, rather than trusting an independently replaced .pub.
        let output = Command::new("ssh-keygen")
            .args(["-y", "-P", "", "-f"])
            .arg(&key)
            .output()?;
        anyhow::ensure!(output.status.success(), "cannot read CX peer SSH identity");
        public_key(std::str::from_utf8(&output.stdout)?)
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
pub fn configure(peer: &Device, host_key: &str, authorized_key: &str) -> Result<Value> {
    let home = std::env::var_os("HOME").context("HOME is unavailable")?;
    configure_at(
        root(),
        &PathBuf::from(home).join(".ssh"),
        peer,
        host_key,
        authorized_key,
    )
}
fn configure_at(
    root: PathBuf,
    ssh: &Path,
    peer: &Device,
    host_key: &str,
    authorized_key: &str,
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
    let authorized = read(&ssh.join("authorized_keys"))?;
    let _ = read(&locked.root.join("known_hosts"))?;
    let _ = locked.identity()?;
    let present = authorized.lines().any(|line| {
        let fields: Vec<_> = line.split_whitespace().collect();
        fields
            .windows(2)
            .any(|w| format!("{} {}", w[0], w[1]) == authorized_key)
    });
    if !present {
        let prefix = if !authorized.is_empty() && !authorized.ends_with('\n') {
            "\n"
        } else {
            ""
        };
        let addition = format!("{prefix}no-agent-forwarding,no-port-forwarding,no-X11-forwarding,no-user-rc {authorized_key} cx-managed-peer\n");
        anyhow::ensure!(
            (authorized.len() + addition.len()) as u64 <= MAX,
            "authorized_keys too large"
        );
        // Append only: unrelated keys and their options remain byte-for-byte intact.
        let mut file = open(&ssh.join("authorized_keys"), true)?;
        file.write_all(addition.as_bytes())?;
        file.sync_all()?;
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
    Ok(json!({"configured": true, "target": target}))
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
        configure_at(root.clone(), &ssh, &peer(), &key(1), &key(2)).unwrap();
        let authorized = fs::read_to_string(ssh.join("authorized_keys")).unwrap();
        assert!(authorized.starts_with(&old));
        assert!(authorized.contains(
            "no-agent-forwarding,no-port-forwarding,no-X11-forwarding,no-user-rc ssh-ed25519"
        ));
        configure_at(root.clone(), &ssh, &peer(), &key(1), &key(2)).unwrap();
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
        assert!(configure_at(root.clone(), &ssh, &peer(), &key(4), &key(2)).is_err());
        assert_eq!(fs::read(root.join("known_hosts")).unwrap(), before);
        let mut other = peer();
        other.id = "other:alice".into();
        assert!(configure_at(root, &ssh, &other, &key(1), &key(2)).is_err());
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
            &key(2)
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
            &key(2)
        )
        .is_err());
        symlink(&ssh, temp.path().join("alias")).unwrap();
        assert!(Locked::new(temp.path().join("alias/subdir")).is_err());
    }
}
