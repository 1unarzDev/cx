mod access;
mod adoption;
mod auth;
mod containers;
mod files;
mod markdown_links;
mod markdown_math;
mod markdown_tables;
mod model;
mod network;
mod peer_ssh;
mod peers;
mod session_titles;
mod sessions;
mod sharing;
mod sharing_lan;
mod store;
mod syntax_preview;
mod terminal_preview;
mod transfers;
mod transport;
mod ui;
mod update;
use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use model::*;
use std::{
    io::{BufReader, Write},
    process::{Command, Stdio},
};
#[derive(Parser)]
#[command(version, about = "A terminal workspace across your devices")]
struct Cli {
    #[command(subcommand)]
    command: Option<Cmd>,
}
#[derive(Subcommand)]
enum Cmd {
    #[command(hide = true)]
    NativeWheel {
        pane: String,
        direction: String,
        x: u16,
        y: u16,
        client: u32,
    },
    #[command(hide = true)]
    NativeCommand {
        payload: String,
    },
    Update {
        /// Update an enrolled SSH device, using this viewer's internet if needed.
        #[arg(long, conflicts_with_all = ["check", "automatic"])]
        device: Option<String>,
        #[arg(long)]
        check: bool,
        #[arg(long, hide = true)]
        automatic: bool,
        #[arg(long, hide = true)]
        json: bool,
    },
    /// Restart CX's managed tmux server and provider sessions on this device.
    Reset,
    #[command(hide = true)]
    Restart {
        state: String,
    },
    Add {
        target: String,
        #[arg(
            long,
            help = "Connect through an enrolled device using OpenSSH ProxyJump"
        )]
        via: Option<String>,
        #[arg(
            long,
            help = "Enroll an immutable/minimal Linux host without requiring optional terminal/PDF utilities"
        )]
        minimal: bool,
    },
    Devices,
    /// Change a locally approved device access posture or availability hint.
    Device {
        #[command(subcommand)]
        command: DeviceCommand,
    },
    /// Open an ordinary login terminal; exit returns without requiring tmux.
    Terminal {
        #[arg(long)]
        device: Option<String>,
    },
    /// Preview preserving robot-LAN sharing; never applies networking changes.
    SharePlan {
        id: String,
        #[arg(long)]
        observation: std::path::PathBuf,
    },
    /// Review directed access policy and an approved recursive SSH path.
    Access {
        from: String,
        to: String,
        #[arg(long)]
        graph: std::path::PathBuf,
        /// Install this approved path for an already enrolled destination.
        #[arg(long)]
        install_route: bool,
    },
    LaunchShell {
        shell: String,
        #[arg(long)]
        device: Option<String>,
    },
    Sessions {
        #[arg(long)]
        device: Option<String>,
    },
    New {
        #[arg(long)]
        device: Option<String>,
        #[arg(long, default_value = "shell")]
        provider: String,
        #[arg(long)]
        directory: String,
        #[arg(long)]
        key: Option<String>,
        #[arg(long)]
        name: Option<String>,
        /// Bypass agent approvals (and Codex sandbox) for this new session.
        #[arg(long)]
        yolo: bool,
    },
    Attach {
        id: String,
        #[arg(long)]
        device: Option<String>,
        #[arg(long)]
        observe: bool,
    },
    Files {
        path: String,
        #[arg(long)]
        device: Option<String>,
    },
    Network {
        #[arg(long)]
        device: Option<String>,
    },
    Copy {
        source: String,
        destination: String,
        #[arg(long)]
        source_device: Option<String>,
        /// Full container ID on the source device.
        #[arg(long)]
        source_container: Option<String>,
        #[arg(long)]
        destination_device: Option<String>,
        /// Full container ID on the destination device.
        #[arg(long)]
        destination_container: Option<String>,
        #[arg(
            long,
            help = "Move source after verified copy; atomic on the same filesystem"
        )]
        cut: bool,
        #[arg(long, default_value = "rename")]
        conflict: String,
        #[arg(long)]
        key: Option<String>,
    },
    Jobs,
    Cancel {
        key: String,
    },
    #[command(hide = true)]
    TransferWorker {
        spec: String,
    },
    #[command(hide = true)]
    Helper,
    #[command(hide = true)]
    ContainerRun {
        payload: String,
    },
    #[command(hide = true)]
    ContainerShell {
        payload: String,
    },
    #[command(hide = true)]
    ContainerHelper,
    #[command(hide = true)]
    ContainerProcessStop {
        id: String,
    },
    #[command(hide = true)]
    ContainerWheelOwner {
        id: String,
    },
    /// Inspect Docker/devcontainer entries on an enrolled device.
    Containers {
        #[arg(long)]
        device: Option<String>,
    },
    /// Start a persistent shell or agent inside a selected container.
    ContainerNew {
        id: String,
        #[arg(long)]
        device: Option<String>,
        #[arg(long, default_value = "shell")]
        provider: String,
        #[arg(long)]
        yolo: bool,
        #[arg(long)]
        directory: Option<String>,
    },
    /// Start a workspace through its Dev Containers configuration and lifecycle hooks.
    DevcontainerUp {
        workspace: String,
        #[arg(long)]
        device: Option<String>,
        #[arg(long)]
        yes: bool,
    },
    /// Recreate a devcontainer using its host workspace configuration.
    DevcontainerRebuild {
        id: String,
        #[arg(long)]
        device: Option<String>,
        #[arg(long)]
        yes: bool,
    },
    ContainerStart {
        id: String,
        #[arg(long)]
        device: Option<String>,
    },
    ContainerStop {
        id: String,
        #[arg(long)]
        device: Option<String>,
    },
    /// Explicitly opt a non-devcontainer into terminal/files/lifecycle access.
    ContainerAccess {
        id: String,
        #[arg(long)]
        device: Option<String>,
        #[arg(long)]
        disable: bool,
    },
    ContainerFiles {
        id: String,
        #[arg(long)]
        device: Option<String>,
        #[arg(long, default_value = "/")]
        path: String,
        #[arg(long)]
        preview: bool,
    },
    #[command(hide = true)]
    NativeAttach {
        #[arg(long)]
        external: bool,
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
}

#[derive(Subcommand)]
enum DeviceCommand {
    /// Change the enrolled SSH alias or user@host without changing identity or posture.
    Edit { name: String, target: String },
    /// Remove an enrolled device from this viewer.
    Remove { name: String },
    /// Reconcile automatic SSH setup and enrollment for core devices.
    Sync,
    /// Verify reverse SSH and enroll this device on the remote helper.
    Pair {
        name: String,
        #[arg(long)]
        return_target: String,
    },
    /// Promote an enrolled device to the core network posture.
    Promote { name: String },
    /// Demote an enrolled device to viewer-to-device access.
    Demote { name: String },
    /// Set whether refreshes should expect this device to be online.
    Availability {
        name: String,
        #[arg(value_enum)]
        state: AvailabilityState,
    },
    /// Show the effective access and availability policy for a device.
    Policy { name: String },
    /// Manage the guarded ptrace relay that moves outside terminals into CX.
    Ptrace {
        name: String,
        #[arg(value_enum)]
        action: PtraceAction,
        /// Locally built cx-reptyr instead of the signed release artifact.
        #[arg(long)]
        binary: Option<std::path::PathBuf>,
    },
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub(crate) enum PtraceAction {
    /// Install the user-local relay so moves work through one sudo prompt each.
    Install,
    /// Remove the user-local relay.
    Uninstall,
    /// Grant cap_sys_ptrace to a root-owned relay so moves need no prompt.
    Enable,
    /// Remove the privileged capability relay.
    Disable,
    Status,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum AvailabilityState {
    Up,
    Down,
}
fn info() -> Result<serde_json::Value> {
    let d = store::local_device();
    let machine = std::fs::read_to_string("/etc/machine-id").unwrap_or_default();
    let mut caps = Vec::new();
    let home = std::path::PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    for tool in ["tmux"] {
        let local = home.join(".local/bin").join(tool);
        let path = if local.is_file() {
            local
        } else {
            std::path::PathBuf::from("/usr/bin").join(tool)
        };
        use std::os::unix::fs::PermissionsExt;
        if path
            .metadata()
            .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        {
            caps.push(tool)
        }
    }
    caps.extend(sessions::available_providers()?);
    caps.push("native-command-v1");
    caps.push("stop-session-v1");
    caps.push("stop-agent-session-v1");
    caps.push("session-yolo-v1");
    caps.push("containers-v1");
    caps.push("container-transfers-v1");
    caps.push("devcontainer-rebuild-v1");
    caps.push("stable-update-v1");
    caps.push("pdf-pages-v1");
    caps.push("network-candidates-v1");
    caps.push("peer-enroll-v1");
    caps.push("peer-bootstrap-v1");
    caps.push("adopt-session-v1");
    let relay = adoption::reptyr();
    if relay.is_ok() {
        caps.push("adopt-reptyr-v1");
    }
    let (needs_sudo, unavailable) = match &relay {
        Ok(r) => (r.privilege == adoption::Privilege::Sudo, None),
        Err(e) => (false, Some(*e)),
    };
    Ok(
        serde_json::json!({"host":d.host,"account":d.account,"machine_id":machine.trim(),"capabilities":caps,"protocol":1,"persistent_channel":true,"version":env!("CARGO_PKG_VERSION"),"adoption_unavailable":unavailable,"adoption_needs_password":needs_sudo}),
    )
}
pub fn dispatch(op: Operation) -> Result<serde_json::Value> {
    match op {
        Operation::Info => info(),
        Operation::PeerEnroll {
            peer,
            replace_stale,
        } => peers::enroll(peer, replace_stale),
        Operation::PeerIdentity => peers::bootstrap_identity(),
        Operation::PeerConfigure {
            peer,
            host_key,
            client_key,
        } => peers::configure(&peer, &host_key, &client_key),
        Operation::Containers => containers::list(),
        Operation::ContainerInspect { id } => Ok(serde_json::to_value(containers::inspect(&id)?)?),
        Operation::DevcontainerUp { workspace } => containers::up(&workspace),
        Operation::DevcontainerRebuild { scope } => containers::rebuild(&scope),
        Operation::ContainerAccess {
            id,
            engine,
            enabled,
        } => containers::set_access(&id, &engine, enabled),
        Operation::ContainerLifecycle {
            id,
            engine,
            started_at,
            action,
        } => containers::lifecycle(&id, &engine, &started_at, &action),
        Operation::ContainerFiles { scope, operation } => containers::files(&scope, &operation),
        Operation::ContainerFileAction { scope, operation } => {
            containers::file_action(&scope, &operation)
        }
        Operation::ContainerCreate {
            scope,
            request,
            yolo,
        } => Ok(serde_json::to_value(sessions::create_container(
            &request, &scope, yolo,
        )?)?),
        Operation::SetLaunchShell { shell } => sessions::set_launch_shell(&shell),
        Operation::TransferReachability { destination } => {
            transport::request(&destination, Operation::Info)
        }
        Operation::Transfer(spec) => {
            anyhow::ensure!(
                spec.source_container.is_none() && spec.destination_container.is_none(),
                "Container transfer requires scoped_transfer; update the submitting CX helper"
            );
            transfers::start(&spec)
        }
        Operation::ScopedTransfer(spec) => transfers::start(&spec),
        Operation::TransferJobs => transfers::jobs(),
        Operation::TransferCancel { key } => transfers::cancel(&key),
        Operation::TransferRetry { key } => transfers::retry(&key),
        Operation::Sessions => Ok(serde_json::to_value(sessions::list()?)?),
        Operation::ExternalSessions => Ok(serde_json::to_value(sessions::external_list()?)?),
        Operation::AdoptSession {
            ref id,
            ref origin,
            ref boot_id,
        } => Ok(serde_json::to_value(sessions::adopt(id, origin, boot_id)?)?),
        Operation::Create(ref c) => Ok(serde_json::to_value(sessions::create(c)?)?),
        Operation::CreateYolo(ref c) => Ok(serde_json::to_value(sessions::create_yolo(c)?)?),
        Operation::StopAgentSession {
            id,
            pid,
            started,
            boot_id,
            provider,
        } => sessions::stop_agent(&id, pid, &started, &boot_id, &provider),
        Operation::StopSession {
            ref id,
            pid,
            ref started,
            ref boot_id,
        } => sessions::stop_shell(id, pid, started, boot_id),
        Operation::CommandPanel {
            ref request,
            cols,
            rows,
        } => sessions::command_panel(request, cols, rows),
        Operation::CapturePane { ref id, cols, rows } => sessions::capture_pane(id, cols, rows),
        Operation::PaneInput { ref id, ref hex } => sessions::pane_input(id, hex),
        Operation::PaneClose { ref id } => sessions::pane_close(id),
        Operation::Network => network::observe(),
        Operation::NetworkCandidates => network::candidates(),
        Operation::ProbeCandidate { address, interface } => {
            network::probe_candidate(&address, interface.as_deref())
        }
        Operation::Copy {
            source,
            destination,
            conflict,
            key,
        } => transfers::start(&TransferSpec {
            source_container: None,
            destination_container: None,
            source: store::local_device(),
            source_path: source,
            destination: store::local_device(),
            destination_path: destination,
            cut: false,
            source_identity: None,
            conflict,
            key,
        }),
        Operation::Jobs => transfers::jobs(),
        Operation::Cancel { key } => transfers::cancel(&key),
        ref o => files::handle(o),
    }
}
fn helper() -> Result<()> {
    use std::io::BufRead;
    let mut input = BufReader::new(std::io::stdin().lock());
    let mut output = std::io::stdout().lock();
    loop {
        if input.fill_buf()?.is_empty() {
            return Ok(());
        }
        let req: Request = transport::read_frame(&mut input)?;
        let result = if req.version != 1 {
            Err(anyhow::anyhow!("unsupported helper protocol"))
        } else {
            dispatch(req.op)
        };
        let response = match result {
            Ok(v) => Response {
                version: 1,
                id: req.id,
                result: Some(v),
                error: None,
            },
            Err(e) => Response {
                version: 1,
                id: req.id,
                result: None,
                error: Some(format!("{e:#}")),
            },
        };
        transport::frame(&mut output, &response)?;
    }
}
fn device(name: Option<String>) -> Result<Device> {
    let ds = store::devices()?;
    if let Some(n) = name {
        ds.into_iter()
            .find(|d| d.name == n || d.id == n || d.target.as_deref() == Some(&n))
            .context("device not enrolled; use cx add")
    } else {
        Ok(store::local_device())
    }
}

/// Install, remove or report the guarded relay on a device. Installation is an
/// explicit privileged step: one sudo prompt on that device, never automatic.
pub(crate) fn device_ptrace(
    name: &str,
    action: PtraceAction,
    binary: Option<std::path::PathBuf>,
) -> Result<serde_json::Value> {
    let device = if name == "local" {
        store::local_device()
    } else {
        enrolled_device(name)?
    };
    let label = device.name.clone();
    let status = |device: &Device| -> Result<serde_json::Value> {
        let info = transport::request(device, Operation::Info)?;
        let enabled = info["capabilities"]
            .as_array()
            .is_some_and(|c| c.iter().any(|c| c == "adopt-reptyr-v1"));
        Ok(serde_json::json!({
            "device": label,
            "adoption": if !enabled {
                "unavailable"
            } else if info["adoption_needs_password"] == serde_json::json!(true) {
                "enabled (asks for a password per move)"
            } else {
                "enabled (no prompt)"
            },
            "reason": info["adoption_unavailable"],
        }))
    };
    let script = include_str!("../scripts/enable-ptrace.sh");
    let sudo = |args: &[&str]| -> Result<()> {
        let quoted: Vec<String> = args.iter().map(|a| sessions::quote(a)).collect();
        let command = format!(
            "sudo sh -c {} sh {}",
            sessions::quote(script),
            quoted.join(" ")
        );
        let ok = if let Some(target) = &device.target {
            let routed = store::ssh(target, true)?;
            let mut c = Command::new("ssh");
            c.arg("-tt").args(routed.get_args()).arg(command);
            c.status()?.success()
        } else {
            Command::new("sh")
                .arg("-c")
                .arg(command)
                .status()?
                .success()
        };
        anyhow::ensure!(
            ok,
            "privileged step failed or was cancelled; nothing else changed"
        );
        Ok(())
    };
    // Verified relay bytes for this device's architecture.
    let relay_bytes = |device: &Device| -> Result<Vec<u8>> {
        let arch = match &device.target {
            None => std::env::consts::ARCH.to_string(),
            Some(target) => {
                let mut c = transport::ssh(target, true)?;
                c.arg("uname -sm");
                let out = enrollment_command(c, None)?;
                match String::from_utf8_lossy(&out.stdout).trim() {
                    "Linux x86_64" => "x86_64".into(),
                    "Linux aarch64" => "aarch64".into(),
                    _ => bail!("cx-reptyr supports Linux x86_64 and ARM64 only"),
                }
            }
        };
        match &binary {
            Some(path) => Ok(std::fs::read(path)?),
            None => Ok(std::fs::read(
                update::obtain_reptyr_binary(&arch, env!("CARGO_PKG_VERSION"))?.path(),
            )?),
        }
    };
    match action {
        PtraceAction::Status => status(&device),
        PtraceAction::Uninstall => {
            if let Some(target) = &device.target {
                let mut c = transport::ssh(target, true)?;
                c.arg("rm -f \"$HOME/.local/bin/cx-reptyr\"");
                enrollment_command(c, None)?;
            } else {
                let _ = std::fs::remove_file(
                    std::path::PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
                        .join(".local/bin/cx-reptyr"),
                );
            }
            status(&device)
        }
        PtraceAction::Install => {
            // Unprivileged: the relay only becomes capable when run through sudo
            // per move. Verify the exact version where it is installed.
            let bytes = relay_bytes(&device)?;
            if let Some(target) = &device.target {
                let mut c = transport::ssh(target, true)?;
                c.arg("set -eu; umask 077; d=\"$HOME/.local/bin\"; mkdir -p \"$d\"; f=\"$d/cx-reptyr\"; cat > \"$f.new\"; chmod 755 \"$f.new\"; \"$f.new\" -v; mv -f \"$f.new\" \"$f\"")
                    .stdin(Stdio::piped());
                let out = enrollment_command(c, Some(bytes))?;
                anyhow::ensure!(
                    out.status.success()
                        && String::from_utf8_lossy(&out.stdout).trim() == adoption::REPTYR_VERSION,
                    "cx-reptyr did not verify on {label}; nothing installed"
                );
            } else {
                use std::os::unix::fs::OpenOptionsExt;
                let dir = std::path::PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
                    .join(".local/bin");
                std::fs::create_dir_all(&dir)?;
                let staged = dir.join("cx-reptyr.new");
                let _ = std::fs::remove_file(&staged);
                let mut f = std::fs::OpenOptions::new()
                    .create_new(true)
                    .write(true)
                    .mode(0o755)
                    .open(&staged)?;
                std::io::Write::write_all(&mut f, &bytes)?;
                f.sync_all()?;
                drop(f);
                let out = Command::new(&staged)
                    .arg("-v")
                    .stdin(Stdio::null())
                    .output()?;
                anyhow::ensure!(
                    String::from_utf8_lossy(&out.stdout).trim() == adoption::REPTYR_VERSION,
                    "cx-reptyr version mismatch; nothing installed"
                );
                std::fs::rename(&staged, dir.join("cx-reptyr"))?;
            }
            status(&device)
        }
        PtraceAction::Disable => {
            sudo(&["disable"])?;
            status(&device)
        }
        PtraceAction::Enable => {
            let bytes = relay_bytes(&device)?;
            use sha2::Digest;
            let sha = format!("{:x}", sha2::Sha256::digest(&bytes));
            // Stage as the user and verify the exact version there, never as root.
            let stage = if device.target.is_none() {
                use std::os::unix::fs::OpenOptionsExt;
                let tools = store::ensure()?.join("tools");
                std::fs::create_dir_all(&tools)?;
                let stage = tools.join("cx-reptyr.stage");
                let _ = std::fs::remove_file(&stage);
                let mut f = std::fs::OpenOptions::new()
                    .create_new(true)
                    .write(true)
                    .mode(0o700)
                    .open(&stage)?;
                std::io::Write::write_all(&mut f, &bytes)?;
                f.sync_all()?;
                drop(f);
                let out = Command::new(&stage)
                    .arg("-v")
                    .stdin(Stdio::null())
                    .output()?;
                anyhow::ensure!(
                    String::from_utf8_lossy(&out.stdout).trim() == adoption::REPTYR_VERSION,
                    "cx-reptyr version mismatch; nothing installed"
                );
                stage
                    .to_str()
                    .context("state path is not UTF-8")?
                    .to_string()
            } else {
                let mut c = transport::ssh(device.target.as_deref().unwrap(), true)?;
                c.arg("set -eu; umask 077; d=\"${XDG_STATE_HOME:-$HOME/.local/state}/cx/tools\"; mkdir -p \"$d\"; f=\"$d/cx-reptyr.stage\"; rm -f \"$f\"; cat > \"$f\"; chmod 700 \"$f\"; printf '%s\\n' \"$f\"; \"$f\" -v")
                    .stdin(Stdio::piped());
                let out = enrollment_command(c, Some(bytes))?;
                let text = String::from_utf8_lossy(&out.stdout).into_owned();
                let mut lines = text.lines();
                let stage = lines.next().unwrap_or_default().to_string();
                anyhow::ensure!(
                    out.status.success() && lines.next() == Some(adoption::REPTYR_VERSION),
                    "cx-reptyr did not verify on {label}; nothing installed"
                );
                stage
            };
            eprintln!(
                "Installing cx-reptyr on {label} with cap_sys_ptrace. It moves terminals into CX \
                 only for processes fully owned by the user who runs it. Any program running as \
                 you on {label} could use it to take over your other terminals. Undo with: \
                 cx device ptrace {name} disable"
            );
            sudo(&["enable", &stage, &sha])?;
            status(&device)
        }
    }
}

fn enrolled_device(name: &str) -> Result<Device> {
    device(Some(name.to_owned()))
}

fn device_policy_report(name: &str) -> Result<serde_json::Value> {
    let device = enrolled_device(name)?;
    let policy = store::policy(&device)?;
    Ok(serde_json::json!({
        "id": device.id,
        "name": device.name,
        "target": device.target,
        "access": policy.access,
        "availability": policy.availability,
    }))
}
fn add(target: &str) -> Result<()> {
    add_via(target, None)
}
fn add_via(target: &str, via: Option<&Device>) -> Result<()> {
    add_via_mode(target, via, true)
}
fn add_via_mode(target: &str, via: Option<&Device>, minimal: bool) -> Result<()> {
    if !transport::valid_target(target) {
        bail!("invalid SSH target")
    };
    let resolved = if let Some(gateway) = via {
        if target
            .rsplit_once('@')
            .is_some_and(|(_, host)| host.parse::<std::net::IpAddr>().is_err())
        {
            let candidates = transport::request(gateway, Operation::NetworkCandidates)?;
            candidate_target(target, &candidates)?.unwrap_or_else(|| target.into())
        } else {
            target.into()
        }
    } else {
        target.into()
    };
    let target = resolved.as_str();
    let _maintenance = update::maintenance_lock()?;
    let previous = store::route(target)?;
    let next = if let Some(via) = via {
        store::route_via(target, via)?
    } else {
        previous.clone()
    };
    anyhow::ensure!(
        previous == next || !store::devices()?.iter().any(|d| d.target.as_deref() == Some(target)),
        "This target is already enrolled through another route. Use distinct SSH aliases for devices on overlapping networks; the existing device was not changed."
    );
    store::set_route(target, &next)?;
    let result = enroll_target(target, minimal);
    if result.is_err() {
        store::set_route(target, &previous)?;
    }
    result
}
// A name is a routing hint on the selected observer, not SSH identity evidence.
fn candidate_target(target: &str, snapshot: &serde_json::Value) -> Result<Option<String>> {
    let Some((account, host)) = target.rsplit_once('@') else {
        return Ok(None);
    };
    let normalized = |name: &str| {
        name.trim_end_matches('.')
            .trim_end_matches(".local")
            .to_ascii_lowercase()
    };
    let Some(rows) = snapshot["candidates"].as_array() else {
        return Ok(None);
    };
    let mut matches: Vec<_> = rows
        .iter()
        .filter(|row| {
            row["hostname"]
                .as_str()
                .is_some_and(|name| normalized(name) == normalized(host))
        })
        .filter_map(|row| {
            let address = row["address"].as_str()?.parse::<std::net::IpAddr>().ok()?;
            if matches!(address, std::net::IpAddr::V6(ip) if ip.is_unicast_link_local()) {
                return None;
            }
            Some((address, row["lladdr"].as_str()))
        })
        .collect();
    matches.sort_by_key(|(ip, _)| (!ip.is_ipv4(), *ip));
    matches.dedup_by_key(|(ip, _)| *ip);
    if matches.len() > 1 {
        let mac = matches[0].1;
        anyhow::ensure!(mac.is_some() && matches.iter().all(|(_, other)| *other == mac),
            "This hostname matches multiple devices. Select its address under the gateway in Network.");
    }
    Ok(matches.first().map(|(ip, _)| format!("{account}@{ip}")))
}
#[cfg(test)]
mod robot_target_tests {
    use super::*;
    #[test]
    fn observer_names_choose_numeric_ipv4_without_granting_ssh_identity() {
        let mut rows = serde_json::json!({"candidates":[
            {"hostname":"blastoise-odroid.local","address":"192.168.0.152","lladdr":"same"},
            {"hostname":"blastoise-odroid.local","address":"192.168.0.147","lladdr":"same"}
        ]});
        assert_eq!(
            candidate_target("roboboat@blastoise-odroid.local", &rows)
                .unwrap()
                .as_deref(),
            Some("roboboat@192.168.0.147")
        );
        assert_eq!(
            candidate_target("roboboat@blastoise-odroid", &rows)
                .unwrap()
                .as_deref(),
            Some("roboboat@192.168.0.147")
        );
        rows["candidates"][1]["lladdr"] = serde_json::json!("different");
        assert!(candidate_target("roboboat@blastoise-odroid.local", &rows).is_err());
        assert!(candidate_target("roboboat@unknown", &rows)
            .unwrap()
            .is_none());
    }
}
// Bound enrollment I/O even if a remote host stops consuming the binary.
fn enrollment_command(
    mut command: Command,
    bytes: Option<Vec<u8>>,
) -> Result<std::process::Output> {
    use std::{
        io::Read,
        os::unix::process::CommandExt,
        thread,
        time::{Duration, Instant},
    };
    if bytes.is_none() {
        command.stdin(Stdio::null());
    }
    command
        .process_group(0)
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = command.spawn()?;
    let mut stdout = child.stdout.take().context("enrollment output")?;
    let reader = thread::spawn(move || {
        let mut output = Vec::new();
        let _ = stdout.by_ref().take(65536).read_to_end(&mut output);
        output
    });
    let writer = bytes.map(|bytes| {
        let mut input = child.stdin.take().expect("installer input");
        thread::spawn(move || input.write_all(&bytes))
    });
    let started = Instant::now();
    let status = loop {
        if auth::cancelled() || started.elapsed() > Duration::from_secs(300) {
            unsafe {
                libc::kill(-(child.id() as i32), libc::SIGKILL);
            }
            let _ = child.kill();
            let _ = child.wait();
            bail!("Enrollment cancelled or timed out; host keys are only accepted with explicit approval");
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {}
            Err(error) => {
                unsafe {
                    libc::kill(-(child.id() as i32), libc::SIGKILL);
                }
                let _ = child.kill();
                let _ = child.wait();
                return Err(error.into());
            }
        }
        thread::sleep(Duration::from_millis(25));
    };
    // Descendants must release stdin/stdout before joining either I/O thread.
    unsafe {
        libc::kill(-(child.id() as i32), libc::SIGKILL);
    }
    if let Some(writer) = writer {
        writer
            .join()
            .map_err(|_| anyhow::anyhow!("installer input failed"))??;
    }
    let stdout = reader
        .join()
        .map_err(|_| anyhow::anyhow!("enrollment output failed"))?;
    Ok(std::process::Output {
        status,
        stdout,
        stderr: Vec::new(),
    })
}

fn enroll_target(target: &str, _minimal: bool) -> Result<()> {
    let mut c = transport::ssh(target, true)?;
    c.arg("uname -sm; id -un");
    let out = enrollment_command(c, None)?;
    if !out.status.success() {
        bail!("SSH access failed; verify or unlock this host with ssh {target}")
    };
    let facts = String::from_utf8(out.stdout)?;
    let lines: Vec<_> = facts.lines().collect();
    let remote_arch = lines
        .iter()
        .find_map(|line| match *line {
            "Linux x86_64" => Some("x86_64"),
            "Linux aarch64" => Some("aarch64"),
            _ => None,
        })
        .context("Execution hosts currently require Linux x86_64 or ARM64")?;
    // Foreign executables are verified against the release key, never run here.
    // Keep the stage handle alive until the remote installer finishes.
    let enrollment = if remote_arch != std::env::consts::ARCH {
        Some(update::obtain_enrollment_binary(remote_arch)?)
    } else {
        None
    };
    let binary = if let Some(artifact) = &enrollment {
        artifact.path()
    } else {
        std::env::current_exe()?
    };
    let bytes = std::fs::read(binary)?;
    let mut c = transport::ssh(target, true)?;
    let script = include_str!("../scripts/enroll-helper.sh").replace('\'', "'\"'\"'");
    c.arg(format!("CX_ENROLL_MINIMAL=1 sh -c '{script}'"))
        .stdin(Stdio::piped());
    if auth::active() {
        c.stdout(Stdio::null()).stderr(Stdio::null());
    }
    if !enrollment_command(c, Some(bytes))?.status.success() {
        bail!("helper installation failed")
    };
    // Existing system or user tmux is retained; provision only when absent.
    let mut probe = transport::ssh(target, true)?;
    probe.arg("PATH=\"$HOME/.local/bin:$PATH\"; export PATH; if command -v tmux >/dev/null 2>&1; then timeout 10 tmux -V; else exit 77; fi");
    let available = enrollment_command(probe, None)?;
    if available.status.code() == Some(77) {
        let version = enrollment
            .as_ref()
            .map(|a| a.version.as_str())
            .unwrap_or(env!("CARGO_PKG_VERSION"));
        let tool = update::obtain_tmux_binary(remote_arch, version).context(
            "Helper installed, but portable tmux is unavailable; retry adding this device",
        )?;
        let script = include_str!("../scripts/enroll-tmux.sh").replace('\'', "'\"'\"'");
        let mut install = transport::ssh(target, true)?;
        install
            .arg(format!("sh -c '{script}'"))
            .stdin(Stdio::piped());
        if !enrollment_command(install, Some(std::fs::read(tool.path())?))?
            .status
            .success()
        {
            bail!("Helper installed, but tmux installation failed; existing tools were preserved. Retry adding this device")
        }
    } else if !available.status.success() || !available.stdout.starts_with(b"tmux ") {
        bail!("Helper installed, but the existing remote tmux could not run; repair it and retry adding this device")
    }
    let mut d = Device {
        id: target.into(),
        name: target.into(),
        target: Some(target.into()),
        account: String::new(),
        host: String::new(),
        status: "unknown".into(),
        observed_at: 0,
    };
    // Installation already chose the helper binary; verify without starting a redundant
    // background updater that could outlive this short enrollment command.
    let value = transport::probe_info(target)?;
    d.account = value["account"].as_str().unwrap_or("unknown").into();
    d.host = value["host"].as_str().unwrap_or(target).into();
    if target.contains('@') {
        d.name = d.host.clone();
    }
    d.id = format!(
        "{}:{}",
        value["machine_id"].as_str().unwrap_or(&d.host),
        d.account
    );
    d.status = "reachable".into();
    d.observed_at = transport::now();
    let enrollment_guard = store::device_guard()?;
    let mut ds = store::devices()?;
    let existing = ds
        .iter()
        .find(|entry| entry.id == d.id)
        .map(|entry| store::policy(entry))
        .transpose()?
        .is_some_and(|policy| policy.access == store::AccessMode::Directed);
    if let Some(old) = ds
        .iter_mut()
        .find(|o| o.id == d.id || (o.host == d.host && o.account == d.account))
    {
        *old = d.clone()
    } else {
        ds.push(d.clone())
    };
    store::allow_explicit_enrollment(&d.id)?;
    store::save_devices(&ds)?;
    // Core enrollment automatically reconciles verified SSH access. Users can demote
    // them later with `cx device demote` or the device-panel toggle.
    if !existing {
        let _ = store::set_access(&d, store::AccessMode::Core)?;
    }
    drop(enrollment_guard);
    let setup = peers::sync();
    if !auth::active() {
        match setup {
            Ok(report) if report["state"] == "ready" => {
                eprintln!("Core device connections verified")
            }
            _ => eprintln!("Device enrolled; core setup is pending and will retry in CX"),
        }
    }
    if !auth::active() {
        println!(
            "Added {} · {}@{} · {}",
            d.name, d.account, d.host, value["capabilities"]
        );
    }
    Ok(())
}
fn run() -> Result<()> {
    match Cli::parse().command {
        None => ui::run(),
        Some(Cmd::NativeCommand { payload }) => {
            use base64::Engine;
            anyhow::ensure!(payload.len() <= 32768, "Command payload too large");
            let bytes = base64::engine::general_purpose::STANDARD.decode(payload)?;
            let command: RunCommand = serde_json::from_slice(&bytes)?;
            sessions::execute_command(&command)
        }
        Some(Cmd::Helper) => helper(),
        Some(Cmd::ContainerHelper) => {
            containers::container_home()?;
            helper()
        }
        Some(Cmd::ContainerShell { payload }) => {
            use base64::Engine;
            anyhow::ensure!(
                payload.len() < 65536,
                "container shell payload exceeds limit"
            );
            let (provider, yolo, folder, id): (String, bool, String, String) =
                serde_json::from_slice(
                    &base64::engine::general_purpose::STANDARD.decode(payload)?,
                )?;
            containers::local_shell(&provider, yolo, &folder, &id)
        }
        Some(Cmd::ContainerProcessStop { id }) => {
            println!("{}", containers::local_stop(&id)?);
            Ok(())
        }
        Some(Cmd::ContainerWheelOwner { id }) => {
            println!("{}", containers::local_wheel_owner(&id)?);
            Ok(())
        }
        Some(Cmd::ContainerRun { payload }) => {
            use base64::Engine;
            anyhow::ensure!(payload.len() < 65536, "container launch exceeds limit");
            let (scope, provider, yolo): (ContainerScope, String, bool) = serde_json::from_slice(
                &base64::engine::general_purpose::STANDARD.decode(payload)?,
            )?;
            containers::run(&scope, &provider, yolo)
        }
        Some(Cmd::Containers { device: d }) => {
            println!(
                "{}",
                transport::request(&device(d)?, Operation::Containers)?
            );
            Ok(())
        }
        Some(Cmd::DevcontainerUp {
            workspace,
            device: d,
            yes,
        }) => {
            anyhow::ensure!(yes, "Workspace startup can run configuration hooks and Compose services. Review its config and pass --yes to continue");
            println!(
                "{}",
                transport::request(&device(d)?, Operation::DevcontainerUp { workspace })?
            );
            Ok(())
        }
        Some(Cmd::DevcontainerRebuild { id, device: d, yes }) => {
            anyhow::ensure!(yes, "Rebuild replaces the container and its writable layer, ends its sessions, and runs workspace hooks. Back up container-only files, then pass --yes to proceed");
            let d = device(d)?;
            let c: containers::Container = serde_json::from_value(transport::request(
                &d,
                Operation::ContainerInspect { id },
            )?)?;
            println!(
                "{}",
                transport::request(&d, Operation::DevcontainerRebuild { scope: c.scope() })?
            );
            Ok(())
        }
        Some(Cmd::ContainerStart { id, device: d }) => container_lifecycle_cli(id, d, "start"),
        Some(Cmd::ContainerStop { id, device: d }) => container_lifecycle_cli(id, d, "stop"),
        Some(Cmd::ContainerAccess {
            id,
            device: d,
            disable,
        }) => {
            println!(
                "{}",
                transport::request(
                    &device(d.clone())?,
                    Operation::ContainerAccess {
                        engine: serde_json::from_value::<containers::Container>(
                            transport::request(
                                &device(d.clone())?,
                                Operation::ContainerInspect { id: id.clone() }
                            )?
                        )?
                        .engine,
                        id,
                        enabled: !disable
                    }
                )?
            );
            Ok(())
        }
        Some(Cmd::ContainerFiles {
            id,
            device: d,
            path,
            preview,
        }) => {
            let d = device(d)?;
            let c: containers::Container = serde_json::from_value(transport::request(
                &d,
                Operation::ContainerInspect { id },
            )?)?;
            let op = if preview {
                Operation::Preview { path }
            } else {
                Operation::List { path }
            };
            println!(
                "{}",
                transport::request(
                    &d,
                    Operation::ContainerFiles {
                        scope: c.scope(),
                        operation: Box::new(op)
                    }
                )?
            );
            Ok(())
        }
        Some(Cmd::ContainerNew {
            id,
            device: d,
            provider,
            yolo,
            directory,
        }) => {
            let d = device(d)?;
            let c: containers::Container = serde_json::from_value(transport::request(
                &d,
                Operation::ContainerInspect { id },
            )?)?;
            let mut scope = c.scope();
            if let Some(folder) = directory {
                scope.folder = folder;
            }
            let request = CreateSession {
                key: format!("container-{}-{}", std::process::id(), transport::now()),
                directory: scope.folder.clone(),
                provider,
                name: String::new(),
                resume: false,
                resume_pick: false,
            };
            println!(
                "{}",
                transport::request(
                    &d,
                    Operation::ContainerCreate {
                        scope,
                        request,
                        yolo
                    }
                )?
            );
            Ok(())
        }
        Some(Cmd::Restart { state }) => ui::run_restored(Some(&state)),
        Some(Cmd::Update {
            device: selected,
            check,
            automatic,
            json,
        }) => {
            if let Some(selected) = selected {
                let host = device(Some(selected))?;
                let outcome = transport::update_device(&host)?;
                if json {
                    println!("{}", outcome);
                } else {
                    println!(
                        "{} · {} · cx {}",
                        host.name,
                        outcome["state"].as_str().unwrap_or("unavailable"),
                        outcome["version"].as_str().unwrap_or("unknown")
                    );
                }
                return Ok(());
            }
            let mut version = env!("CARGO_PKG_VERSION").to_string();
            let (state, message, launch) = match update::check(!automatic)? {
                update::CheckOutcome::Ready(plan) if !check => {
                    let path = update::install(&plan)?;
                    version = plan.version.clone();
                    ("updated", format!("Updated cx to {}", version), Some(path))
                }
                update::CheckOutcome::Ready(plan) => {
                    version = plan.version;
                    ("ready", format!("Verified cx {} ready", version), None)
                }
                update::CheckOutcome::Current => {
                    // Keep the interactive command consistent with the update path:
                    // after reporting the result, return to the normal workspace.
                    (
                        "current",
                        format!("cx {} is current", version),
                        Some(std::env::current_exe()?),
                    )
                }
                update::CheckOutcome::Offline => (
                    "offline",
                    format!("Update service unreachable · keeping cx {}", version),
                    None,
                ),
                update::CheckOutcome::Unavailable(message) => (
                    "unavailable",
                    format!("{} · keeping current cx", message),
                    None,
                ),
                update::CheckOutcome::Skipped => (
                    "busy",
                    "Update check deferred · keeping current cx".into(),
                    None,
                ),
            };
            if json {
                println!("{}", serde_json::json!({"state":state,"version":version}));
            } else {
                println!("{message}");
            }
            if !json && !automatic && unsafe { libc::isatty(libc::STDIN_FILENO) } != 0 {
                if let Some(path) = launch {
                    if state == "current" {
                        std::env::set_var(
                            "CX_STARTUP_NOTICE",
                            format!("cx {} is already up to date", version),
                        );
                    }
                    use std::os::unix::process::CommandExt;
                    return Err(Command::new(path).exec().into());
                }
            }
            Ok(())
        }
        Some(Cmd::Reset) => {
            println!("{}", serde_json::to_string_pretty(&sessions::reset()?)?);
            Ok(())
        }
        Some(Cmd::Copy {
            source,
            destination,
            source_device,
            destination_device,
            source_container,
            destination_container,
            cut,
            conflict,
            key,
        }) => {
            let source_device = device(source_device)?;
            let destination_device = device(destination_device)?;
            let resolve_scope = |device: &model::Device,
                                 id: Option<String>|
             -> Result<Option<model::ContainerScope>> {
                id.map(|id| {
                    let c: containers::Container = serde_json::from_value(transport::request(
                        device,
                        Operation::ContainerInspect { id },
                    )?)?;
                    Ok(c.scope())
                })
                .transpose()
            };
            let spec = TransferSpec {
                source_container: resolve_scope(&source_device, source_container)?,
                destination_container: resolve_scope(&destination_device, destination_container)?,
                source: source_device,
                source_path: source,
                destination: destination_device,
                destination_path: destination,
                cut,
                source_identity: None,
                conflict,
                key: key
                    .unwrap_or_else(|| format!("copy-{}-{}", std::process::id(), transport::now())),
            };
            println!("{}", transfers::start(&spec)?);
            Ok(())
        }
        Some(Cmd::Jobs) => {
            println!("{}", transfers::jobs()?);
            Ok(())
        }
        Some(Cmd::Cancel { key }) => {
            println!("{}", transfers::cancel(&key)?);
            Ok(())
        }
        Some(Cmd::TransferWorker { spec }) => {
            use std::io::Read;
            use std::os::unix::fs::OpenOptionsExt;
            let f = std::fs::OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NOFOLLOW)
                .open(spec)?;
            let s: TransferSpec = serde_json::from_reader(f.take(1024 * 1024))?;
            transfers::worker(&s)
        }
        Some(Cmd::NativeWheel {
            pane,
            direction,
            x,
            y,
            client,
        }) => sessions::native_wheel(&pane, &direction, x, y, client),
        Some(Cmd::NativeAttach { external, args }) => {
            if args.first().map(String::as_str) != Some("attach-session") {
                bail!("invalid native operation")
            };
            let p = std::path::PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
                .join(".local/bin/tmux");
            let mut c = Command::new(if p.is_file() {
                p
            } else {
                std::path::PathBuf::from("tmux")
            });
            c.arg("-u");
            if let Some(terminal) = sessions::attach_terminal()? {
                c.env("TERM", terminal);
            }
            if external {
                c.env_remove("TMUX").env_remove("TMUX_PANE");
            }
            if !external {
                sessions::configure_managed()?;
                let target = args
                    .windows(2)
                    .find(|pair| pair[0] == "-t")
                    .map(|pair| pair[1].as_str())
                    .context("native attachment requires a session target")?;
                sessions::refresh_managed_status(target)?;
                c.arg("-S").arg(sessions::socket()?);
            }
            let status = c.args(args).status()?;
            if !status.success() {
                bail!("native tmux failed")
            };
            Ok(())
        }
        Some(Cmd::Add {
            target,
            via,
            minimal,
        }) => {
            let gateway = via.map(|name| device(Some(name))).transpose()?;
            add_via_mode(&target, gateway.as_ref(), minimal)
        }
        Some(Cmd::LaunchShell { shell, device: d }) => {
            println!(
                "{}",
                transport::request(&device(d)?, Operation::SetLaunchShell { shell })?
            );
            Ok(())
        }
        Some(Cmd::SharePlan { id, observation }) => {
            use std::io::Read;
            let mut bytes = Vec::new();
            std::fs::File::open(observation)?
                .take(256 * 1024 + 1)
                .read_to_end(&mut bytes)?;
            anyhow::ensure!(
                bytes.len() <= 256 * 1024,
                "sharing observation exceeds limit"
            );
            let observation: sharing_lan::Observation = serde_json::from_slice(&bytes)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&sharing_lan::preview(
                    &id,
                    observation,
                    transport::now()
                )?)?
            );
            Ok(())
        }
        Some(Cmd::Access {
            from,
            to,
            graph,
            install_route,
        }) => {
            use std::io::Read;
            let mut bytes = Vec::new();
            std::fs::File::open(graph)?
                .take(256 * 1024 + 1)
                .read_to_end(&mut bytes)?;
            anyhow::ensure!(bytes.len() <= 256 * 1024, "access policy exceeds limit");
            let graph: access::Graph = serde_json::from_slice(&bytes)?;
            let connection = graph.connection(&from, &to, transport::now())?;
            if install_route {
                let _maintenance = update::maintenance_lock()?;
                anyhow::ensure!(
                    store::devices()?
                        .iter()
                        .any(|d| d.target.as_deref() == Some(&connection.target)),
                    "enroll and verify destination identity before installing its route"
                );
                store::set_route(&connection.target, &connection.jumps)?;
            }
            println!("{}", serde_json::to_string_pretty(&connection)?);
            Ok(())
        }
        Some(Cmd::Terminal { device: d }) => sessions::login_terminal(&device(d)?),
        Some(Cmd::Device { command }) => {
            match command {
                DeviceCommand::Edit { name, target } => {
                    let device = enrolled_device(&name)?;
                    let updated = store::update_target(&device, &target)?;
                    println!("{}", serde_json::to_string_pretty(&updated)?);
                }
                DeviceCommand::Sync => {
                    println!("{}", serde_json::to_string_pretty(&peers::sync()?)?);
                }
                DeviceCommand::Pair {
                    name,
                    return_target,
                } => {
                    let device = enrolled_device(&name)?;
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&peers::pair(&device, &return_target)?)?
                    );
                }
                DeviceCommand::Ptrace {
                    name,
                    action,
                    binary,
                } => {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&device_ptrace(&name, action, binary)?)?
                    );
                }
                DeviceCommand::Promote { name } => {
                    let device = enrolled_device(&name)?;
                    let policy = store::set_access(&device, store::AccessMode::Core)?;
                    let setup = peers::sync()?;
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&serde_json::json!({
                            "device": device.name,
                            "access": policy.access,
                            "availability": policy.availability,
                            "bidirectional": setup["state"] == "ready",
                            "setup": setup,
                        }))?
                    );
                }
                DeviceCommand::Remove { name } => {
                    let removed = store::remove_device(&name)?;
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&serde_json::json!({
                            "removed": removed.name,
                            "id": removed.id,
                        }))?
                    );
                }
                DeviceCommand::Demote { name } => {
                    let device = enrolled_device(&name)?;
                    let policy = store::set_access(&device, store::AccessMode::Directed)?;
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&serde_json::json!({
                            "device": device.name,
                            "access": policy.access,
                            "availability": policy.availability,
                            "bidirectional": false,
                        }))?
                    );
                }
                DeviceCommand::Availability { name, state } => {
                    let device = enrolled_device(&name)?;
                    let availability = match state {
                        AvailabilityState::Up => store::Availability::UsuallyUp,
                        AvailabilityState::Down => store::Availability::UsuallyDown,
                    };
                    let policy = store::set_availability(&device, availability)?;
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&serde_json::json!({
                            "device": device.name,
                            "access": policy.access,
                            "availability": policy.availability,
                            "automatic_refresh": policy.availability == store::Availability::UsuallyUp,
                        }))?
                    );
                }
                DeviceCommand::Policy { name } => {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&device_policy_report(&name)?)?
                    );
                }
            }
            Ok(())
        }
        Some(Cmd::Devices) => {
            println!("{}", serde_json::to_string_pretty(&store::devices()?)?);
            Ok(())
        }
        Some(Cmd::Sessions { device: d }) => {
            println!("{}", transport::request(&device(d)?, Operation::Sessions)?);
            Ok(())
        }
        Some(Cmd::New {
            device: d,
            provider,
            directory,
            key,
            name,
            yolo,
        }) => {
            let key = key.unwrap_or_else(|| format!("{}-{}", std::process::id(), transport::now()));
            let name = name.unwrap_or_default();
            anyhow::ensure!(
                !yolo || provider != "shell",
                "YOLO applies only to Codex or Claude sessions"
            );
            let spec = CreateSession {
                key,
                directory,
                provider,
                name,
                resume: false,
                resume_pick: false,
            };
            let op = if yolo {
                Operation::CreateYolo(spec)
            } else {
                Operation::Create(spec)
            };
            println!("{}", transport::request(&device(d)?, op)?);
            Ok(())
        }
        Some(Cmd::Attach {
            id,
            device: d,
            observe,
        }) => {
            let d = device(d)?;
            let list: Vec<Session> =
                serde_json::from_value(transport::request(&d, Operation::Sessions)?)?;
            let s = list
                .iter()
                .find(|s| s.id == id || s.name == id)
                .context("session not found")?;
            sessions::attach(&d, s, observe)
        }
        Some(Cmd::Files { path, device: d }) => {
            println!(
                "{}",
                transport::request(&device(d)?, Operation::List { path })?
            );
            Ok(())
        }
        Some(Cmd::Network { device: d }) => {
            println!("{}", transport::request(&device(d)?, Operation::Network)?);
            Ok(())
        }
    }
}
fn main() {
    if auth::is_askpass_client() {
        std::process::exit(if auth::askpass_client().is_ok() { 0 } else { 1 });
    }
    if let Err(e) = run() {
        eprintln!("cx: {e:#}");
        std::process::exit(1)
    }
}

#[cfg(test)]
mod enrollment_tests {
    use super::*;
    #[test]
    fn exited_installer_cannot_leave_writer_or_reader_blocked() {
        let mut command = Command::new("sh");
        command
            .args(["-c", "sleep 30 & exit 0"])
            .stdin(Stdio::piped());
        let started = std::time::Instant::now();
        // Exceeds a pipe buffer; an inherited descendant never consumes it.
        let result = enrollment_command(command, Some(vec![0u8; 1024 * 1024]));
        assert!(result.is_err());
        assert!(started.elapsed() < std::time::Duration::from_secs(3));
    }
}

fn container_lifecycle_cli(id: String, target: Option<String>, action: &str) -> Result<()> {
    let device = device(target)?;
    let c: containers::Container = serde_json::from_value(transport::request(
        &device,
        Operation::ContainerInspect { id },
    )?)?;
    println!(
        "{}",
        transport::request(
            &device,
            Operation::ContainerLifecycle {
                id: c.id,
                engine: c.engine,
                started_at: c.started_at,
                action: action.into()
            }
        )?
    );
    Ok(())
}
