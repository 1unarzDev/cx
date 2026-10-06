use crate::model::Device;
use anyhow::{Context, Result};
use std::os::unix::fs::PermissionsExt;
use std::{fs, path::PathBuf};
pub fn state_dir() -> PathBuf {
    std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/state")
        })
        .join("cx")
}
pub fn ensure() -> Result<PathBuf> {
    let p = state_dir();
    fs::create_dir_all(&p)?;
    use std::os::unix::fs::MetadataExt;
    let m = fs::symlink_metadata(&p)?;
    anyhow::ensure!(
        m.is_dir() && m.uid() == unsafe { libc::geteuid() },
        "unsafe cx state directory"
    );
    fs::set_permissions(&p, fs::Permissions::from_mode(0o700))?;
    Ok(p)
}
pub fn local_device() -> Device {
    Device {
        id: "local".into(),
        name: std::fs::read_to_string("/proc/sys/kernel/hostname")
            .unwrap_or_else(|_| "local".into())
            .trim()
            .into(),
        target: None,
        account: std::env::var("USER").unwrap_or_else(|_| "unknown".into()),
        host: std::fs::read_to_string("/proc/sys/kernel/hostname")
            .unwrap_or_else(|_| "local".into())
            .trim()
            .into(),
        status: "local".into(),
        observed_at: 0,
    }
}
pub fn devices() -> Result<Vec<Device>> {
    let p = state_dir().join("devices.json");
    if !p.exists() {
        return Ok(vec![local_device()]);
    }
    let mut d: Vec<Device> = serde_json::from_slice(&fs::read(p)?)?;
    if !d.iter().any(|d| d.target.is_none()) {
        d.insert(0, local_device())
    }
    Ok(d)
}
pub fn save_devices(d: &[Device]) -> Result<()> {
    let p = ensure()?.join("devices.json");
    let tmp = p.with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_vec_pretty(d)?)?;
    fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600))?;
    fs::rename(tmp, p).context("save enrolled devices")
}
