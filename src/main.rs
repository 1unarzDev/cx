mod auth;
mod files;
mod markdown_links;
mod markdown_math;
mod markdown_tables;
mod model;
mod network;
mod session_titles;
mod sessions;
mod sharing;
mod store;
mod syntax_preview;
mod terminal_preview;
mod transfers;
mod transport;
mod ui;
mod update;
use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
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
        #[arg(long)]
        check: bool,
        #[arg(long, hide = true)]
        automatic: bool,
        #[arg(long, hide = true)]
        json: bool,
    },
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
    },
    Devices,
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
        #[arg(long)]
        destination_device: Option<String>,
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
    NativeAttach {
        #[arg(long)]
        external: bool,
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
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
    caps.push("stable-update-v1");
    caps.push("pdf-pages-v1");
    caps.push("network-candidates-v1");
    Ok(
        serde_json::json!({"host":d.host,"account":d.account,"machine_id":machine.trim(),"capabilities":caps,"protocol":1,"persistent_channel":true,"version":env!("CARGO_PKG_VERSION")}),
    )
}
pub fn dispatch(op: Operation) -> Result<serde_json::Value> {
    match op {
        Operation::Info => info(),
        Operation::SetLaunchShell { shell } => sessions::set_launch_shell(&shell),
        Operation::TransferReachability { destination } => {
            transport::request(&destination, Operation::Info)
        }
        Operation::Transfer(spec) => transfers::start(&spec),
        Operation::TransferJobs => transfers::jobs(),
        Operation::TransferCancel { key } => transfers::cancel(&key),
        Operation::TransferRetry { key } => transfers::retry(&key),
        Operation::Sessions => Ok(serde_json::to_value(sessions::list()?)?),
        Operation::Create(ref c) => Ok(serde_json::to_value(sessions::create(c)?)?),
        Operation::StopSession {
            ref id,
            pid,
            ref started,
            ref boot_id,
        } => sessions::stop_shell(id, pid, started, boot_id),
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
fn add(target: &str) -> Result<()> {
    add_via(target, None)
}
fn add_via(target: &str, via: Option<&Device>) -> Result<()> {
    if !transport::valid_target(target) {
        bail!("invalid SSH target")
    };
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
    let result = enroll_target(target);
    if result.is_err() {
        store::set_route(target, &previous)?;
    }
    result
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

fn enroll_target(target: &str) -> Result<()> {
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
    c.arg(format!("sh -c '{script}'")).stdin(Stdio::piped());
    if auth::active() {
        c.stdout(Stdio::null()).stderr(Stdio::null());
    }
    if !enrollment_command(c, Some(bytes))?.status.success() {
        bail!("helper installation failed")
    };
    let mut d = Device {
        id: target.into(),
        name: target.into(),
        target: Some(target.into()),
        account: String::new(),
        host: String::new(),
        status: "unknown".into(),
        observed_at: 0,
    };
    let value = transport::request(&d, Operation::Info)?;
    d.account = value["account"].as_str().unwrap_or("unknown").into();
    d.host = value["host"].as_str().unwrap_or(target).into();
    d.id = format!(
        "{}:{}",
        value["machine_id"].as_str().unwrap_or(&d.host),
        d.account
    );
    d.status = "reachable".into();
    d.observed_at = transport::now();
    let mut ds = store::devices()?;
    if let Some(old) = ds
        .iter_mut()
        .find(|o| o.id == d.id || (o.host == d.host && o.account == d.account))
    {
        *old = d.clone()
    } else {
        ds.push(d.clone())
    };
    store::save_devices(&ds)?;
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
        Some(Cmd::Restart { state }) => ui::run_restored(Some(&state)),
        Some(Cmd::Update {
            check,
            automatic,
            json,
        }) => {
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
                    ("current", format!("cx {} is current", version), None)
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
                    use std::os::unix::process::CommandExt;
                    return Err(Command::new(path).exec().into());
                }
            }
            Ok(())
        }
        Some(Cmd::Copy {
            source,
            destination,
            source_device,
            destination_device,
            cut,
            conflict,
            key,
        }) => {
            let spec = TransferSpec {
                source: device(source_device)?,
                source_path: source,
                destination: device(destination_device)?,
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
        Some(Cmd::Add { target, via }) => {
            let gateway = via.map(|name| device(Some(name))).transpose()?;
            add_via(&target, gateway.as_ref())
        }
        Some(Cmd::LaunchShell { shell, device: d }) => {
            println!(
                "{}",
                transport::request(&device(d)?, Operation::SetLaunchShell { shell })?
            );
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
        }) => {
            let key = key.unwrap_or_else(|| format!("{}-{}", std::process::id(), transport::now()));
            let name = name.unwrap_or_default();
            println!(
                "{}",
                transport::request(
                    &device(d)?,
                    Operation::Create(CreateSession {
                        key,
                        directory,
                        provider,
                        name
                    })
                )?
            );
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
