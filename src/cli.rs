//! Plain-terminal session commands: `cx claude|codex|shell` start a managed session
//! and attach, `cx kill` stops one, `cx adopt` moves an outside terminal into CX.
//! The same operations the UI sends; nothing here is UI-only.
use crate::model::{CreateSession, Device, Operation, Session};
use crate::{sessions, transport};
use anyhow::{bail, Context, Result};
use std::io::{BufRead, IsTerminal, Write};

#[derive(clap::Args, Debug, Default)]
pub struct LaunchArgs {
    /// Folder to start in: this folder by default (`~` on another device).
    pub dir: Option<String>,
    /// Start on this enrolled device.
    #[arg(short, long)]
    pub device: Option<String>,
    /// Bypass approvals (and Codex's sandbox); remembered as this agent's mode.
    #[arg(short, long, conflicts_with = "normal")]
    pub yolo: bool,
    /// Ask for approvals; remembered as this agent's mode.
    #[arg(short, long)]
    pub normal: bool,
    /// Continue the latest conversation in that folder.
    #[arg(short, long)]
    pub resume: bool,
    /// Choose which conversation to resume.
    #[arg(short, long, conflicts_with = "resume")]
    pub pick: bool,
}

/// `cx claude …`: lend/revoke/list stay the gateway lending commands (docs/claude.md);
/// anything else starts Claude.
#[derive(clap::Parser)]
#[command(
    name = "cx claude",
    about = "Start Claude in this folder (or DIR, or on --device) and attach",
    after_help = "Lend your Claude Code gateway: cx claude lend|revoke|list (docs/claude.md)"
)]
struct ClaudeCli {
    #[command(flatten)]
    launch: LaunchArgs,
}

pub fn claude(args: Vec<String>) -> Result<()> {
    if matches!(
        args.first().map(String::as_str),
        Some("lend" | "revoke" | "list")
    ) {
        return crate::mesh::run_claude(&args);
    }
    let cli = <ClaudeCli as clap::Parser>::try_parse_from(
        std::iter::once("cx claude".to_string()).chain(args),
    )
    .unwrap_or_else(|e| e.exit());
    launch("claude", cli.launch)
}

/// Create a session on the device and attach to it. A remote attach whose connection
/// drops reattaches the same session (Ctrl+C gives up), as the viewer does.
pub fn launch(provider: &str, a: LaunchArgs) -> Result<()> {
    let d = crate::device(a.device.clone())?;
    let agent = provider != "shell";
    anyhow::ensure!(
        agent || !(a.yolo || a.normal || a.resume || a.pick),
        "--yolo, --normal, --resume and --pick apply only to claude and codex"
    );
    let yolo = if a.yolo || a.normal {
        let _ = store::set_launch_yolo(&d.id, provider, a.yolo);
        a.yolo
    } else {
        agent
            && store::launch_yolo()
                .unwrap_or_default()
                .contains(&(d.id.clone(), provider.to_string()))
    };
    let spec = CreateSession {
        key: format!("{}-{}", std::process::id(), transport::now()),
        directory: directory(&d, a.dir.as_deref())?,
        provider: provider.into(),
        name: String::new(),
        resume: a.resume || a.pick,
        resume_pick: a.pick,
    };
    let op = if yolo {
        Operation::CreateYolo(spec)
    } else {
        Operation::Create(spec)
    };
    let session: Session = serde_json::from_value(transport::request(&d, op)?)?;
    attach(&d, &session)
}

/// The folder to send: local paths resolve here (relative to this folder, `~` to
/// HOME); a remote device gets `~`, `~/…` or an absolute path as typed.
fn directory(d: &Device, dir: Option<&str>) -> Result<String> {
    if d.target.is_some() {
        let dir = dir.unwrap_or("~");
        anyhow::ensure!(
            dir == "~" || dir.starts_with("~/") || dir.starts_with('/'),
            "on another device give an absolute folder, ~ or ~/…"
        );
        return Ok(dir.into());
    }
    let cwd = std::env::current_dir().context("this folder is unavailable")?;
    let path = match dir {
        None => cwd,
        Some("~") => home()?,
        Some(p) if p.starts_with("~/") => home()?.join(&p[2..]),
        Some(p) => cwd.join(p),
    };
    let path = std::fs::canonicalize(&path)
        .with_context(|| format!("{} is not a folder", path.display()))?;
    anyhow::ensure!(path.is_dir(), "{} is not a folder", path.display());
    Ok(crate::files::encode_path(&path))
}
fn home() -> Result<std::path::PathBuf> {
    Ok(std::env::var_os("HOME").context("HOME unavailable")?.into())
}

fn attach(d: &Device, s: &Session) -> Result<()> {
    let mut delay = 1;
    loop {
        match sessions::attach(d, s, false) {
            Err(e) if e.is::<sessions::Dropped>() => {
                eprintln!("cx: connection lost, reconnecting in {delay} s… (Ctrl+C gives up)");
                std::thread::sleep(std::time::Duration::from_secs(delay));
                delay = (delay * 2).min(8);
            }
            other => return other,
        }
    }
}

/// Every session on the device, managed and outside-CX agent terminals alike.
fn listed(d: &Device) -> Result<Vec<Session>> {
    let mut all: Vec<Session> =
        serde_json::from_value(transport::request(d, Operation::Sessions)?)?;
    if let Ok(v) = transport::request(d, Operation::ExternalSessions) {
        all.extend(serde_json::from_value::<Vec<Session>>(v).unwrap_or_default());
    }
    Ok(all)
}

fn label(s: &Session) -> String {
    let dir = crate::files::decode_path(&s.directory)
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| s.directory.clone());
    let tty = s
        .origin
        .as_ref()
        .map(|o| format!(" {}", o.tty))
        .unwrap_or_default();
    crate::ui::safe_label(&format!("{} · {} · {}{tty}", s.name, s.provider, dir))
}

fn confirm(question: &str) -> Result<bool> {
    if !std::io::stdin().is_terminal() {
        bail!("not a terminal: pass --yes to confirm");
    }
    eprint!("{question} [y/N] ");
    std::io::stderr().flush()?;
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line)?;
    Ok(matches!(line.trim(), "y" | "Y" | "yes"))
}

/// `cx kill SESSION`: by id or name, the stop ⌫ sends, after a confirmation.
pub fn kill(session: &str, device: Option<String>, yes: bool) -> Result<()> {
    let d = crate::device(device)?;
    let all = listed(&d)?;
    let matches: Vec<&Session> = all
        .iter()
        .filter(|s| s.id == session || s.name == session)
        .collect();
    let s = match matches.as_slice() {
        [s] => *s,
        [] => bail!("no session {session:?} (see `cx sessions`)"),
        _ => bail!("{session:?} names several sessions: use its id"),
    };
    if !yes && !confirm(&format!("stop {}?", label(s)))? {
        bail!("cancelled");
    }
    let (op, cap) = sessions::stop_operation(s)?;
    if d.target.is_some() {
        let info = transport::request(&d, Operation::Info)?;
        anyhow::ensure!(
            info["capabilities"]
                .as_array()
                .is_some_and(|c| c.iter().any(|c| c == cap)),
            "{} needs a newer cx to stop this session (`cx update --device {}`)",
            d.name,
            d.name
        );
    }
    transport::request(&d, op)?;
    eprintln!("stopped {}", label(s));
    Ok(())
}

/// `cx adopt [PID|TTY]`: move an agent terminal started outside CX into CX and attach.
/// Without an argument, choose from the list. A sudo password, if the move needs one,
/// is asked here with echo off.
pub fn adopt(terminal: Option<String>, device: Option<String>) -> Result<()> {
    let d = crate::device(device)?;
    let all: Vec<Session> =
        serde_json::from_value(transport::request(&d, Operation::ExternalSessions)?)?;
    let all: Vec<Session> = all.into_iter().filter(|s| s.origin.is_some()).collect();
    let s = match terminal {
        Some(t) => {
            let tty = if t.starts_with("/dev/") {
                t.clone()
            } else {
                format!("/dev/pts/{t}")
            };
            all.into_iter()
                .find(|s| {
                    let o = s.origin.as_ref().expect("filtered");
                    s.pid.to_string() == t || o.leader.pid.to_string() == t || o.tty == tty
                })
                .with_context(|| format!("no terminal outside CX matches {t:?}"))?
        }
        None => {
            anyhow::ensure!(!all.is_empty(), "no agent terminals outside CX");
            for (i, s) in all.iter().enumerate() {
                eprintln!("{:>3}  {}", i + 1, label(s));
            }
            if !std::io::stdin().is_terminal() {
                bail!("not a terminal: give a pid or tty");
            }
            eprint!("move which? ");
            std::io::stderr().flush()?;
            let mut line = String::new();
            std::io::stdin().lock().read_line(&mut line)?;
            let i: usize = line.trim().parse().context("not a number")?;
            all.into_iter()
                .nth(i.wrapping_sub(1))
                .context("no such terminal")?
        }
    };
    let op = Operation::AdoptSession {
        id: s.id.clone(),
        origin: s.origin.clone().expect("filtered"),
        boot_id: s.boot_id.clone(),
    };
    let (tx, rx) = std::sync::mpsc::channel();
    let target = d.clone();
    crate::auth::with_broker(
        move || {
            let _ = tx.send(transport::request(&target, op));
            Ok(())
        },
        |prompt| {
            let mut value = Vec::new();
            crate::secrets::prompt_no_echo(&format!("{} ", prompt.text.trim()), &mut value)?;
            Ok(crate::auth::Answer::Submit(String::from_utf8(value)?))
        },
    )?;
    let moved: Session = serde_json::from_value(rx.recv().context("the move did not finish")??)?;
    eprintln!("moved {} into CX", label(&moved));
    attach(&d, &moved)
}

// TEMP until the launch worker's store API lands.
mod store {
    pub fn launch_yolo() -> anyhow::Result<std::collections::BTreeSet<(String, String)>> {
        Ok(Default::default())
    }
    pub fn set_launch_yolo(_: &str, _: &str, _: bool) -> anyhow::Result<()> {
        Ok(())
    }
}
