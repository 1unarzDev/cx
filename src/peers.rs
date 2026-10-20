//! Explicit reciprocal enrollment over existing authenticated SSH routes.
use crate::{
    model::{Device, Operation},
    store, transport,
};
use anyhow::{bail, Context, Result};

fn identity(info: &serde_json::Value) -> Result<String> {
    let machine = info["machine_id"]
        .as_str()
        .filter(|s| !s.is_empty())
        .context("helper did not report machine identity")?;
    let account = info["account"]
        .as_str()
        .filter(|s| !s.is_empty())
        .context("helper did not report account")?;
    Ok(format!("{machine}:{account}"))
}

fn validate(peer: &Device) -> Result<()> {
    let target = peer
        .target
        .as_deref()
        .context("peer requires an explicit account@host endpoint")?;
    let (account, host) = target
        .split_once('@')
        .context("use a portable account@host endpoint, not a local SSH alias")?;
    anyhow::ensure!(
        transport::valid_target(target)
            && !account.is_empty()
            && !host.is_empty()
            && !host.contains('@')
            && account == peer.account,
        "invalid peer endpoint/account"
    );
    for value in [&peer.id, &peer.name, &peer.host, &peer.account] {
        anyhow::ensure!(
            !value.is_empty() && value.len() <= 256 && !value.chars().any(char::is_control),
            "invalid peer identity"
        );
    }
    anyhow::ensure!(peer.id != "local", "remote peer requires stable identity");
    Ok(())
}

pub fn enroll(mut peer: Device) -> Result<serde_json::Value> {
    validate(&peer)?;
    anyhow::ensure!(
        identity(&crate::info()?)? != peer.id,
        "cannot enroll this device as its own peer"
    );
    // Nothing is persisted until reverse authentication and identity are proven.
    let observed = transport::fresh_info(peer.target.as_deref().context("missing endpoint")?).context("reverse SSH unavailable; configure the existing SSH route/trust/authentication outside CX and retry pairing")?;
    anyhow::ensure!(
        identity(&observed)? == peer.id,
        "reverse helper identity mismatch"
    );
    // Serialize concurrent pairing helpers without holding a lock during SSH.
    use std::os::unix::fs::OpenOptionsExt;
    let lock = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(store::ensure()?.join("peer-enrollment.lock"))?;
    anyhow::ensure!(lock.metadata()?.is_file(), "unsafe pairing lock");
    anyhow::ensure!(
        unsafe { libc::flock(std::os::fd::AsRawFd::as_raw_fd(&lock), libc::LOCK_EX) } == 0,
        "pairing lock unavailable"
    );
    let mut devices = store::devices()?;
    anyhow::ensure!(
        !devices
            .iter()
            .any(|d| d.id != peer.id && (d.target == peer.target || d.name == peer.name)),
        "peer endpoint or name is already enrolled for another identity"
    );
    peer.status = "reachable".into();
    peer.observed_at = transport::now();
    if let Some(existing) = devices.iter_mut().find(|d| d.id == peer.id) {
        // Preserve receiver-local aliases/routes and explicit one-way policies.
        let policy = store::policy(existing)?;
        anyhow::ensure!(
            policy.access == store::AccessMode::Core,
            "existing peer is directed; promote it locally before pairing"
        );
        existing.status = peer.status.clone();
        existing.observed_at = peer.observed_at;
    } else {
        anyhow::ensure!(devices.len() < 128, "device enrollment limit reached");
        devices.push(peer.clone());
        store::set_access(&peer, store::AccessMode::Core)?;
    }
    store::save_devices(&devices)?;
    Ok(
        serde_json::json!({"registered": true, "id": peer.id, "reverse_verified_at": peer.observed_at}),
    )
}

pub fn pair(device: &Device, return_target: &str) -> Result<serde_json::Value> {
    anyhow::ensure!(device.target.is_some(), "select a remote device");
    let local_info = crate::info()?;
    let mut local = store::local_device();
    local.id = identity(&local_info)?;
    local.target = Some(return_target.to_owned());
    validate(&local)?;
    let remote = transport::fresh_info(device.target.as_deref().context("missing endpoint")?)?;
    anyhow::ensure!(
        identity(&remote)? == device.id,
        "remote helper identity mismatch; re-enroll the correct device"
    );
    if !remote["capabilities"]
        .as_array()
        .is_some_and(|c| c.iter().any(|c| c == "peer-enroll-v1"))
    {
        bail!("remote helper does not support pairing; update that helper first");
    }
    let local_id = local.id.clone();
    let result = transport::request(device, Operation::PeerEnroll { peer: local })?;
    anyhow::ensure!(
        result["registered"] == true && result["id"].as_str() == Some(local_id.as_str()),
        "remote helper did not confirm enrollment"
    );
    store::set_access(device, store::AccessMode::Core)?;
    Ok(
        serde_json::json!({"device":device.name,"bidirectional":true,"verified_at":transport::now(),"return_target":return_target}),
    )
}
