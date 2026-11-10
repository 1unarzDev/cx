//! `cx orphans`: find this user's stray cx processes and stale restart handoffs, and
//! optionally stop them. A process is stray only on evidence that nothing can use it:
//! its terminal is gone, or it was reparented to init (its session/ssh parent died).
//! Transfer workers are intentionally detached and are reported, never stopped.
use anyhow::Result;
use serde_json::{json, Value};
use std::{fs, path::Path, time::Duration};

const SPINNING: f64 = 25.0;

#[derive(Debug, Clone, PartialEq)]
pub struct Proc {
    pub pid: i32,
    pub ppid: i32,
    pub state: char,
    pub cpu_ticks: u64,
    pub start_ticks: u64,
    pub args: Vec<String>,
    pub exe_deleted: bool,
    /// Target of fd 0, if readable.
    pub stdin: Option<String>,
    /// Name of the parent; orphans are reparented to init or a user-manager subreaper.
    pub parent_comm: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Finding {
    pub proc: Proc,
    pub role: &'static str,
    pub orphaned: bool,
    pub terminal_gone: bool,
    pub cpu_percent: f64,
    pub stray: bool,
}

pub fn role(args: &[String]) -> &'static str {
    match args.get(1).map(String::as_str) {
        None | Some("restart") => "viewer",
        Some("helper") | Some("container-helper") => "helper",
        Some("transfer-worker") => "transfer-worker",
        // `cx mesh invite`: an ssh-overlay invite listens until it expires; it is meant for a terminal.
        Some("mesh") if args.get(2).map(String::as_str) == Some("invite") => "invite",
        Some(a) if a.starts_with("native-") => "native",
        Some(_) => "command",
    }
}

fn terminal_gone(stdin: Option<&str>) -> bool {
    stdin.is_some_and(|t| {
        t.ends_with(" (deleted)") || (t.starts_with("/dev/pts/") && !Path::new(t).exists())
    })
}

fn live_terminal(stdin: Option<&str>) -> bool {
    stdin.is_some_and(|t| {
        (t.starts_with("/dev/pts/") || t.starts_with("/dev/tty")) && Path::new(t).exists()
    })
}

/// Pure classification so it can be tested without real processes.
pub fn classify(proc: Proc, cpu_percent: f64, own_lineage: &[i32]) -> Finding {
    let role = role(&proc.args);
    let orphaned = proc.ppid == 1 || proc.parent_comm == "systemd";
    let gone = terminal_gone(proc.stdin.as_deref());
    // Viewers and helpers are useless once orphaned; one-shot commands (an update
    // finishing after its ssh dropped) only when their terminal is gone or they spin.
    let evidence = match role {
        "transfer-worker" => false,
        // A viewer still on a live terminal is in use even if its launcher exited.
        "viewer" => gone || (orphaned && !live_terminal(proc.stdin.as_deref())),
        // An invite nobody watches any more is an open listener for nothing.
        "helper" | "invite" => gone || orphaned,
        _ => gone || (orphaned && cpu_percent >= SPINNING),
    };
    let stray = evidence && !own_lineage.contains(&proc.pid);
    Finding {
        proc,
        role,
        orphaned,
        terminal_gone: gone,
        cpu_percent,
        stray,
    }
}

fn read_proc(pid: i32) -> Option<Proc> {
    let base = Path::new("/proc").join(pid.to_string());
    let stat = fs::read_to_string(base.join("stat")).ok()?;
    // comm may contain spaces/parens; fields resume after the last ')'.
    let (head, rest) = stat.rsplit_once(')')?;
    if !head.ends_with("(cx") {
        return None;
    }
    let fields: Vec<&str> = rest.split_whitespace().collect();
    let state = fields.first()?.chars().next()?;
    let ppid = fields.get(1)?.parse().ok()?;
    let utime: u64 = fields.get(11)?.parse().ok()?;
    let stime: u64 = fields.get(12)?.parse().ok()?;
    let start_ticks = fields.get(19)?.parse().ok()?;
    let args: Vec<String> = fs::read(base.join("cmdline"))
        .ok()?
        .split(|b| *b == 0)
        .filter(|a| !a.is_empty())
        .map(|a| String::from_utf8_lossy(a).into_owned())
        .collect();
    let exe = fs::read_link(base.join("exe")).ok()?;
    let exe = exe.to_string_lossy();
    let name = exe.trim_end_matches(" (deleted)");
    if !name.ends_with("/cx") {
        return None;
    }
    Some(Proc {
        pid,
        ppid,
        state,
        cpu_ticks: utime + stime,
        start_ticks,
        args,
        exe_deleted: exe.ends_with(" (deleted)"),
        stdin: fs::read_link(base.join("fd/0"))
            .ok()
            .map(|p| p.to_string_lossy().into_owned()),
        parent_comm: fs::read_to_string(format!("/proc/{ppid}/comm"))
            .map(|c| c.trim().to_owned())
            .unwrap_or_default(),
    })
}

fn own_cx_processes() -> Vec<Proc> {
    use std::os::unix::fs::MetadataExt;
    let uid = unsafe { libc::geteuid() };
    let Ok(entries) = fs::read_dir("/proc") else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|e| e.metadata().is_ok_and(|m| m.uid() == uid))
        .filter_map(|e| e.file_name().to_str()?.parse::<i32>().ok())
        .filter(|pid| *pid != std::process::id() as i32)
        .filter_map(read_proc)
        .collect()
}

fn lineage() -> Vec<i32> {
    let mut pids = vec![std::process::id() as i32];
    let mut pid = pids[0];
    for _ in 0..64 {
        let Some(ppid) = fs::read_to_string(format!("/proc/{pid}/stat"))
            .ok()
            .and_then(|s| {
                s.rsplit_once(')')?
                    .1
                    .split_whitespace()
                    .nth(1)?
                    .parse()
                    .ok()
            })
        else {
            break;
        };
        if ppid <= 1 {
            break;
        }
        pids.push(ppid);
        pid = ppid;
    }
    pids
}

/// Sample every cx process twice to measure CPU use, then classify.
pub fn scan(sample: Duration) -> Vec<Finding> {
    let first = own_cx_processes();
    std::thread::sleep(sample);
    let hz = unsafe { libc::sysconf(libc::_SC_CLK_TCK) }.max(1) as f64;
    let own = lineage();
    own_cx_processes()
        .into_iter()
        .map(|now| {
            let cpu = first
                .iter()
                .find(|p| p.pid == now.pid && p.start_ticks == now.start_ticks)
                .map(|p| {
                    now.cpu_ticks.saturating_sub(p.cpu_ticks) as f64
                        / hz
                        / sample.as_secs_f64().max(0.001)
                        * 100.0
                })
                .unwrap_or(0.0);
            classify(now, cpu, &own)
        })
        .collect()
}

fn stale_handoffs(directory: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(directory) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_str()?.to_owned();
            let old = e
                .metadata()
                .and_then(|m| m.modified())
                .is_ok_and(|t| t.elapsed().is_ok_and(|a| a > Duration::from_secs(600)));
            (name.starts_with("viewer-restart-") && name.ends_with(".json") && old).then_some(name)
        })
        .collect()
}

/// Stop one process only if it is still the same process (pid and start time).
fn stop(proc: &Proc) -> bool {
    let same = || read_proc(proc.pid).is_some_and(|p| p.start_ticks == proc.start_ticks);
    if !same() {
        return true;
    }
    unsafe { libc::kill(proc.pid, libc::SIGTERM) };
    for _ in 0..30 {
        std::thread::sleep(Duration::from_millis(100));
        if !same() {
            return true;
        }
    }
    if same() {
        unsafe { libc::kill(proc.pid, libc::SIGKILL) };
        std::thread::sleep(Duration::from_millis(200));
    }
    !same()
}

pub fn run(clean: bool, as_json: bool) -> Result<()> {
    let findings = scan(Duration::from_secs(1));
    let state = crate::store::state_dir();
    let handoffs = stale_handoffs(&state);
    let mut stopped = Vec::new();
    let mut removed = 0;
    if clean {
        for f in findings.iter().filter(|f| f.stray) {
            if stop(&f.proc) {
                stopped.push(f.proc.pid);
            }
        }
        removed = crate::ui::sweep_stale_restarts(&state, Duration::from_secs(600));
    }
    let rows: Vec<Value> = findings
        .iter()
        .map(|f| {
            json!({
                "pid": f.proc.pid, "ppid": f.proc.ppid, "state": f.proc.state.to_string(),
                "role": f.role, "args": f.proc.args.iter().skip(1).take(1).collect::<Vec<_>>(),
                "cpu_percent": (f.cpu_percent * 10.0).round() / 10.0,
                "orphaned": f.orphaned, "terminal_gone": f.terminal_gone,
                "old_binary": f.proc.exe_deleted, "stray": f.stray,
                "stopped": stopped.contains(&f.proc.pid),
            })
        })
        .collect();
    if as_json {
        println!(
            "{}",
            json!({"processes": rows, "stale_handoffs": handoffs, "removed_handoffs": removed})
        );
        return Ok(());
    }
    let strays = findings.iter().filter(|f| f.stray).count();
    for f in &findings {
        let mut flags = Vec::new();
        if f.orphaned {
            flags.push("orphaned");
        }
        if f.terminal_gone {
            flags.push("terminal-gone");
        }
        if f.cpu_percent >= SPINNING {
            flags.push("spinning");
        }
        if f.proc.exe_deleted {
            flags.push("old-binary");
        }
        if f.stray {
            flags.push(if stopped.contains(&f.proc.pid) {
                "STOPPED"
            } else {
                "STRAY"
            });
        }
        println!(
            "{:>8} ppid {:>8} {:<15} {:>5.1}% {} {}",
            f.proc.pid,
            f.proc.ppid,
            f.role,
            f.cpu_percent,
            f.proc.args.get(1).map(String::as_str).unwrap_or("(viewer)"),
            flags.join(" ")
        );
    }
    println!(
        "{} cx processes, {} stray, {} stale restart handoffs{}",
        findings.len(),
        strays,
        handoffs.len(),
        if clean {
            format!("; stopped {}, removed {} handoffs", stopped.len(), removed)
        } else if strays + handoffs.len() > 0 {
            "; run `cx orphans --clean` to stop strays and remove handoffs".into()
        } else {
            String::new()
        }
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn proc(pid: i32, ppid: i32, args: &[&str], stdin: Option<&str>) -> Proc {
        Proc {
            pid,
            ppid,
            state: 'R',
            cpu_ticks: 0,
            start_ticks: 1,
            args: args.iter().map(|a| a.to_string()).collect(),
            exe_deleted: false,
            stdin: stdin.map(Into::into),
            parent_comm: if ppid == 1 {
                "systemd".into()
            } else {
                "fish".into()
            },
        }
    }
    #[test]
    fn strays_need_evidence_and_daemons_are_kept() {
        let live_tty = "/dev/null";
        let gone = "/dev/pts/987654 (deleted)";
        // Viewer in a live terminal with a living parent: kept.
        assert!(!classify(proc(10, 9, &["cx"], Some(live_tty)), 60.0, &[]).stray);
        // Restarted viewer whose terminal is gone: stray even with a subreaper parent.
        let f = classify(
            proc(
                11,
                500,
                &["cx", "restart", "viewer-restart-1.json"],
                Some(gone),
            ),
            0.0,
            &[],
        );
        assert!(f.stray && f.terminal_gone && f.role == "viewer");
        // Orphaned viewer still on a live terminal: kept; without one: stray.
        assert!(!classify(proc(17, 1, &["cx"], Some("/dev/tty")), 0.0, &[]).stray);
        assert!(classify(proc(18, 1, &["cx"], Some("/dev/null")), 0.0, &[]).stray);
        // Orphaned helper (ssh session died): stray.
        assert!(classify(proc(12, 1, &["cx", "helper"], Some("pipe:[1]")), 0.0, &[]).stray);
        // An invite listener reparented to init (its shell went away): stray; one in its terminal: kept.
        let invite = ["cx", "mesh", "invite", "--backend", "ssh-overlay"];
        let f = classify(proc(19, 1, &invite, Some("pipe:[1]")), 0.0, &[]);
        assert!(f.stray && f.role == "invite");
        assert!(!classify(proc(20, 9, &invite, Some(live_tty)), 0.0, &[]).stray);
        // Detached transfer worker: never stray.
        assert!(
            !classify(
                proc(13, 1, &["cx", "transfer-worker", "/x"], None),
                90.0,
                &[]
            )
            .stray
        );
        // An orphaned one-shot command is kept unless it spins.
        assert!(!classify(proc(15, 1, &["cx", "update"], None), 1.0, &[]).stray);
        assert!(classify(proc(16, 1, &["cx", "update"], None), 60.0, &[]).stray);
        // Never ourselves or an ancestor.
        assert!(!classify(proc(14, 1, &["cx"], Some(gone)), 0.0, &[14]).stray);
    }
    #[test]
    fn scan_sees_this_test_binary_is_not_cx() {
        // The test harness binary is not named cx, so it is never listed.
        assert!(scan(Duration::from_millis(10))
            .iter()
            .all(|f| f.proc.pid != std::process::id() as i32));
    }
}
