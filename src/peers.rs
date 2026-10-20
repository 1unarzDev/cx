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

pub fn enroll(mut peer: Device, replace_stale: bool) -> Result<serde_json::Value> {
    validate(&peer)?;
    anyhow::ensure!(
        !store::peer_removed(&peer.id)?,
        "peer was removed locally; explicit re-enrollment is required"
    );
    anyhow::ensure!(
        identity(&crate::info()?)? != peer.id,
        "cannot enroll this device as its own peer"
    );
    // Nothing is persisted until reverse authentication and identity are proven.
    let observed = transport::probe_info(peer.target.as_deref().context("missing endpoint")?).context("reverse SSH unavailable; configure the existing SSH route/trust/authentication outside CX and retry pairing")?;
    anyhow::ensure!(
        identity(&observed)? == peer.id,
        "reverse helper identity mismatch"
    );
    // Probe preserved aliases outside the inventory lock, then compare the snapshot.
    let snapshot = store::devices()?.into_iter().find(|d| d.id == peer.id);
    let mut adopt_portable = false;
    if let Some(existing) = &snapshot {
        if existing.target != peer.target {
            let target = existing
                .target
                .as_deref()
                .context("existing peer has no SSH endpoint")?;
            match transport::probe_info(target) {
                Ok(observed) => anyhow::ensure!(identity(&observed)? == peer.id, "existing receiver endpoint identity mismatch"),
                Err(_) if replace_stale => adopt_portable = true,
                Err(error) => return Err(error.context("existing receiver endpoint is unavailable; edit it on the receiver before pairing")),
            }
        }
    }
    let _guard = store::device_guard()?;
    anyhow::ensure!(
        !store::peer_removed(&peer.id)?,
        "peer was removed while reverse SSH was being verified"
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
        anyhow::ensure!(
            snapshot
                .as_ref()
                .is_some_and(|old| old.target == existing.target),
            "peer endpoint changed during verification; retry"
        );
        if adopt_portable {
            existing.target = peer.target.clone();
        }
        existing.status = peer.status.clone();
        existing.observed_at = peer.observed_at;
    } else {
        anyhow::ensure!(snapshot.is_none(), "peer removed during verification");
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
    let local_id = local.id.clone();
    let result = transport::verified_request(
        device.target.as_deref().context("missing endpoint")?,
        Operation::PeerEnroll {
            peer: local,
            replace_stale: false,
        },
        |remote| {
            anyhow::ensure!(
                identity(remote)? == device.id,
                "remote helper identity mismatch; re-enroll the correct device"
            );
            if !remote["capabilities"]
                .as_array()
                .is_some_and(|c| c.iter().any(|c| c == "peer-enroll-v1"))
            {
                bail!("remote helper does not support pairing; update that helper first");
            }
            Ok(())
        },
    )?;
    anyhow::ensure!(
        result["registered"] == true && result["id"].as_str() == Some(local_id.as_str()),
        "remote helper did not confirm enrollment"
    );
    store::set_access(device, store::AccessMode::Core)?;
    Ok(
        serde_json::json!({"device":device.name,"bidirectional":true,"verified_at":transport::now(),"return_target":return_target}),
    )
}

/// Metadata and public keys only; private keys never leave their owning device.
pub fn bootstrap_identity() -> Result<serde_json::Value> {
    let mut value = crate::peer_ssh::identity()?;
    value
        .as_object_mut()
        .context("invalid key metadata")?
        .extend(
            crate::info()?
                .as_object()
                .context("invalid device metadata")?
                .clone(),
        );
    value["observed_source"] = std::env::var("SSH_CONNECTION")
        .ok()
        .and_then(|s| s.split_whitespace().next().map(str::to_owned))
        .and_then(|s| s.parse::<std::net::IpAddr>().ok())
        .map(|s| s.to_string())
        .into();
    Ok(value)
}

pub fn configure(peer: &Device, host_key: &str, client_key: &str) -> Result<serde_json::Value> {
    validate(peer)?;
    let _guard = store::device_guard()?;
    anyhow::ensure!(
        !store::peer_removed(&peer.id)?,
        "peer was removed locally; explicit re-enrollment is required"
    );
    anyhow::ensure!(
        identity(&crate::info()?)? != peer.id,
        "cannot configure self as peer"
    );
    if let Some(existing) = store::devices()?.iter().find(|d| d.id == peer.id) {
        anyhow::ensure!(
            store::policy(existing)?.access == store::AccessMode::Core,
            "receiver keeps this peer directed"
        );
    }
    crate::peer_ssh::configure(peer, host_key, client_key)
}

fn local_addresses() -> Result<Vec<std::net::IpAddr>> {
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
    let mut head = std::ptr::null_mut();
    anyhow::ensure!(
        unsafe { libc::getifaddrs(&mut head) } == 0,
        "cannot observe local addresses"
    );
    let mut cursor = head;
    let mut ips = Vec::new();
    while !cursor.is_null() {
        let entry = unsafe { &*cursor };
        if !entry.ifa_addr.is_null() {
            let address = unsafe { &*entry.ifa_addr };
            let ip = match address.sa_family as i32 {
                libc::AF_INET => {
                    let addr = unsafe { &*(entry.ifa_addr as *const libc::sockaddr_in) };
                    Some(IpAddr::V4(Ipv4Addr::from(
                        addr.sin_addr.s_addr.to_ne_bytes(),
                    )))
                }
                libc::AF_INET6 => {
                    let addr = unsafe { &*(entry.ifa_addr as *const libc::sockaddr_in6) };
                    Some(IpAddr::V6(Ipv6Addr::from(addr.sin6_addr.s6_addr)))
                }
                _ => None,
            };
            if let Some(ip) = ip {
                if !ip.is_loopback()
                    && !ip.is_unspecified()
                    && !match ip {
                        IpAddr::V4(ip) => ip.is_link_local(),
                        IpAddr::V6(ip) => ip.is_unicast_link_local(),
                    }
                {
                    ips.push(ip);
                }
            }
        }
        cursor = entry.ifa_next;
    }
    unsafe { libc::freeifaddrs(head) };
    Ok(ips)
}

fn portable_target(device: &Device) -> Result<String> {
    let target = device.target.as_deref().context("missing SSH target")?;
    anyhow::ensure!(transport::valid_target(target), "invalid SSH target");
    anyhow::ensure!(
        store::route(target)?.is_empty(),
        "automatic core setup does not export viewer-local CX jump routes"
    );
    let config = crate::network::bounded_command(
        "ssh",
        &["-G", "-o", "BatchMode=yes", "-o", "ForwardAgent=no", target],
    )?;
    let field = |key: &str| {
        config
            .lines()
            .find_map(|line| line.strip_prefix(&format!("{key} ")))
            .map(str::trim)
    };
    anyhow::ensure!(
        field("port") == Some("22"),
        "automatic core setup requires direct SSH port 22"
    );
    for key in ["proxyjump", "proxycommand"] {
        anyhow::ensure!(
            field(key).is_none_or(|v| v == "none"),
            "automatic core setup does not copy viewer-local SSH jump routes"
        );
    }
    anyhow::ensure!(
        field("user") == Some(device.account.as_str()),
        "resolved SSH account does not match enrolled identity"
    );
    let host = field("hostname").context("SSH did not resolve a hostname")?;
    let result = if let Ok(ip) = host
        .trim_start_matches('[')
        .trim_end_matches(']')
        .parse::<std::net::Ipv6Addr>()
    {
        format!("{}@[{ip}]", device.account)
    } else {
        format!("{}@{host}", device.account)
    };
    anyhow::ensure!(
        transport::valid_target(&result),
        "invalid resolved SSH endpoint"
    );
    Ok(result)
}

fn checked(device: &Device, operation: Operation) -> Result<serde_json::Value> {
    match device.target.as_deref() {
        None => crate::dispatch(operation),
        Some(target) => transport::verified_request(target, operation, |info| {
            anyhow::ensure!(
                identity(info)? == device.id,
                "enrolled helper identity mismatch"
            );
            anyhow::ensure!(
                info["capabilities"]
                    .as_array()
                    .is_some_and(|caps| caps.iter().any(|c| c == "peer-bootstrap-v1")),
                "helper update needed for automatic core setup"
            );
            Ok(())
        }),
    }
}

#[derive(Clone)]
struct CoreNode {
    connection: Device,
    peer: Device,
    metadata: serde_json::Value,
}

/// One bounded coordinator per viewer host. UI retries pending/offline edges.
pub fn sync() -> Result<serde_json::Value> {
    use serde_json::json;
    use std::os::{fd::AsRawFd, unix::fs::OpenOptionsExt};
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(store::ensure()?.join("peer-sync.lock"))?;
    anyhow::ensure!(lock.metadata()?.is_file(), "unsafe core sync lock");
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Ok(json!({"state":"busy","pending":[],"ready":[]}));
    }
    let devices = store::devices()?;
    let candidates = devices
        .iter()
        .filter(|d| d.target.is_some())
        .filter_map(|d| match store::policy(d) {
            Ok(p) if p.access == store::AccessMode::Core => Some(Ok(d.clone())),
            Ok(_) => None,
            Err(e) => Some(Err(e)),
        })
        .collect::<Result<Vec<_>>>()?;
    anyhow::ensure!(
        candidates.len() <= 16,
        "automatic core setup supports at most 16 remote core devices"
    );
    let mut pending = Vec::new();
    let mut nodes = Vec::new();
    let mut source = None;
    let ips = local_addresses()?;
    for device in candidates {
        let result = (|| -> Result<CoreNode> {
            let target = portable_target(&device)?;
            let metadata = checked(&device, Operation::PeerIdentity)?;
            anyhow::ensure!(
                identity(&metadata)? == device.id,
                "helper metadata identity mismatch"
            );
            if let Some(ip) = metadata["observed_source"]
                .as_str()
                .and_then(|ip| ip.parse::<std::net::IpAddr>().ok())
            {
                if ips.contains(&ip) {
                    source.get_or_insert(ip);
                }
            }
            let mut peer = device.clone();
            peer.target = Some(target);
            validate(&peer)?;
            Ok(CoreNode {
                connection: device.clone(),
                peer,
                metadata,
            })
        })();
        match result {
            Ok(node) => nodes.push(node),
            Err(e) => pending.push(json!({"device":device.name,"reason":format!("{e:#}")})),
        }
    }
    if !nodes.is_empty() {
        if let Some(source) = source {
            let connection = store::local_device();
            let metadata = bootstrap_identity()?;
            let mut peer = connection.clone();
            peer.id = identity(&metadata)?;
            peer.target = Some(match source {
                std::net::IpAddr::V6(ip) => format!("{}@[{ip}]", peer.account),
                _ => format!("{}@{source}", peer.account),
            });
            nodes.insert(
                0,
                CoreNode {
                    connection,
                    peer,
                    metadata,
                },
            );
        } else {
            pending.push(json!({"device":"local","reason":"no verified direct return address; existing SSH route may use a gateway"}));
        }
    }
    let selected = |node: &CoreNode| -> Result<()> {
        if node.connection.target.is_none() {
            return Ok(());
        }
        let current = store::devices()?
            .into_iter()
            .find(|d| d.id == node.connection.id)
            .context("core device was removed during setup")?;
        anyhow::ensure!(
            current.target == node.connection.target
                && store::policy(&current)?.access == store::AccessMode::Core
                && !store::peer_removed(&current.id)?,
            "core device changed during setup"
        );
        Ok(())
    };
    let mut ready = Vec::new();
    for a in 0..nodes.len() {
        for b in a + 1..nodes.len() {
            let mut first = nodes[a].clone();
            let mut second = nodes[b].clone();
            // Each receiver uses the viewer address observed on its own authenticated
            // connection, so LAN and mesh interfaces can coexist.
            if first.connection.target.is_none() {
                if let Some(ip) = second.metadata["observed_source"]
                    .as_str()
                    .and_then(|s| s.parse::<std::net::IpAddr>().ok())
                    .filter(|ip| ips.contains(ip))
                {
                    first.peer.target = Some(match ip {
                        std::net::IpAddr::V6(ip) => format!("{}@[{ip}]", first.peer.account),
                        _ => format!("{}@{ip}", first.peer.account),
                    });
                }
            }
            if second.connection.target.is_none() {
                if let Some(ip) = first.metadata["observed_source"]
                    .as_str()
                    .and_then(|s| s.parse::<std::net::IpAddr>().ok())
                    .filter(|ip| ips.contains(ip))
                {
                    second.peer.target = Some(match ip {
                        std::net::IpAddr::V6(ip) => format!("{}@[{ip}]", second.peer.account),
                        _ => format!("{}@{ip}", second.peer.account),
                    });
                }
            }
            let result = (|| -> Result<()> {
                for (receiver, sender) in [(&first, &second), (&second, &first)] {
                    selected(receiver)?;
                    selected(sender)?;
                    checked(
                        &receiver.connection,
                        Operation::PeerConfigure {
                            peer: sender.peer.clone(),
                            host_key: sender.metadata["host_key"]
                                .as_str()
                                .context("missing verified host key")?
                                .into(),
                            client_key: sender.metadata["client_key"]
                                .as_str()
                                .context("missing public client key")?
                                .into(),
                        },
                    )?;
                }
                for (receiver, sender) in [(&first, &second), (&second, &first)] {
                    selected(receiver)?;
                    selected(sender)?;
                    checked(
                        &receiver.connection,
                        Operation::PeerEnroll {
                            peer: sender.peer.clone(),
                            replace_stale: true,
                        },
                    )?;
                }
                Ok(())
            })();
            match result {
                Ok(()) => ready.push(
                    json!({"from":first.peer.name,"to":second.peer.name,"bidirectional":true}),
                ),
                Err(e) => pending.push(
                    json!({"from":first.peer.name,"to":second.peer.name,"reason":format!("{e:#}")}),
                ),
            }
        }
    }
    Ok(
        json!({"state":if pending.is_empty(){"ready"}else{"pending"},"pending":pending,"ready":ready}),
    )
}
