mod files;
mod model;
mod network;
mod sessions;
mod sharing;
mod store;
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
    Update {
        #[arg(long)]
        check: bool,
    },
    #[command(hide = true)]
    Restart {
        state: String,
    },
    Add {
        target: String,
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
        #[arg(long, default_value = "work")]
        name: String,
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
        Operation::Network => network::observe(),
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
    if !transport::valid_target(target) {
        bail!("invalid SSH target")
    };
    let _maintenance = update::maintenance_lock()?;
    let mut c = transport::ssh(target, true)?;
    c.arg("uname -sm; id -un");
    let out = c.output()?;
    if !out.status.success() {
        bail!("SSH access failed; verify or unlock this host with ssh {target}")
    };
    let facts = String::from_utf8(out.stdout)?;
    let lines: Vec<_> = facts.lines().collect();
    let platform = format!("Linux {}", std::env::consts::ARCH);
    if !lines.iter().any(|s| *s == platform) {
        bail!("enrollment requires the same Linux architecture as this viewer; install cx on the host separately for a different architecture")
    };
    let binary = std::env::current_exe()?;
    let bytes = std::fs::read(binary)?;
    let mut c = transport::ssh(target, true)?;
    c.arg("sh -c 'set -eu; umask 077; mkdir -p ~/.local/bin ~/.local/state/cx; exec 9> ~/.local/state/cx/maintenance.lock; flock -n 9 || exit 75; tmp=$(mktemp ~/.local/bin/.cx-install.XXXXXX); trap \"rm -f \\\"$tmp\\\"\" EXIT HUP INT TERM; cat > \"$tmp\"; chmod 700 \"$tmp\"; \"$tmp\" --version >/dev/null; mv \"$tmp\" ~/.local/bin/cx'").stdin(Stdio::piped());
    let mut child = c.spawn()?;
    child
        .stdin
        .take()
        .context("installer stdin")?
        .write_all(&bytes)?;
    if !child.wait()?.success() {
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
    println!(
        "Added {} · {}@{} · {}",
        d.name, d.account, d.host, value["capabilities"]
    );
    Ok(())
}
fn run() -> Result<()> {
    match Cli::parse().command {
        None => ui::run(),
        Some(Cmd::Helper) => helper(),
        Some(Cmd::Restart { state }) => ui::run_restored(Some(&state)),
        Some(Cmd::Update { check }) => {
            match update::check(true)? {
                update::CheckOutcome::Ready(plan) if !check => {
                    let path = update::install(&plan)?;
                    println!("Updated cx to {}", plan.version);
                    if unsafe { libc::isatty(libc::STDIN_FILENO) } != 0 {
                        use std::os::unix::process::CommandExt;
                        return Err(Command::new(path).exec().into());
                    }
                }
                update::CheckOutcome::Ready(plan) => println!("Verified cx {} ready", plan.version),
                update::CheckOutcome::Current => {
                    println!("cx {} is current", env!("CARGO_PKG_VERSION"))
                }
                update::CheckOutcome::Offline => println!(
                    "Update service unreachable · keeping cx {}",
                    env!("CARGO_PKG_VERSION")
                ),
                update::CheckOutcome::Unavailable(message) => {
                    println!("{} · keeping current cx", message)
                }
                update::CheckOutcome::Skipped => {
                    println!("Update check already running · keeping current cx")
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
            if !external {
                sessions::configure_managed()?;
                c.arg("-S").arg(sessions::socket()?);
            }
            let status = c.args(args).status()?;
            if !status.success() {
                bail!("native tmux failed")
            };
            Ok(())
        }
        Some(Cmd::Add { target }) => add(&target),
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
    if let Err(e) = run() {
        eprintln!("cx: {e:#}");
        std::process::exit(1)
    }
}
