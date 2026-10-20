//! Terminal sessions started outside CX.
//!
//! Discovery is read-only and bounded: it reads `/proc` for this user's
//! processes, never terminal contents, arguments beyond executable names or
//! environment data. Each terminal is identified by its session leader and the
//! process holding its pseudo-terminal master, which adoption later re-verifies.
use crate::model::ProcessIdentity;
use std::{
    collections::HashMap,
    fs,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

const PROCESS_LIMIT: usize = 4096;
const FD_LIMIT: usize = 256;
const BUDGET: Duration = Duration::from_millis(150);

/// One pseudo-terminal session owned by this user and not managed by CX.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Terminal {
    pub index: u32,
    pub leader: Process,
    /// Foreground process group of the terminal, from the leader's stat.
    pub foreground: i32,
    pub holder: Option<Holder>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Process {
    pub pid: u32,
    pub start_ticks: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Holder {
    pub process: Process,
    pub kind: &'static str,
}
impl Process {
    pub fn identity(&self) -> ProcessIdentity {
        ProcessIdentity {
            pid: self.pid,
            start_ticks: self.start_ticks.clone(),
            native_id: None,
        }
    }
}
impl Terminal {
    pub fn tty(&self) -> String {
        format!("/dev/pts/{}", self.index)
    }
}

struct Stat {
    session: i32,
    tty: i64,
    foreground: i32,
    start_ticks: String,
}
fn stat(root: &Path, pid: u32) -> Option<Stat> {
    let text = fs::read_to_string(root.join(pid.to_string()).join("stat")).ok()?;
    let fields: Vec<_> = text.rsplit_once(')')?.1.split_whitespace().collect();
    Some(Stat {
        session: fields.get(3)?.parse().ok()?,
        tty: fields.get(4)?.parse().ok()?,
        foreground: fields.get(5)?.parse().ok()?,
        start_ticks: (*fields.get(19)?).into(),
    })
}
/// Unix98 pseudo-terminal index from a `tty_nr`; other terminals are ignored.
pub fn pts_index(tty: i64) -> Option<u32> {
    let major = (tty >> 8) & 0xfff;
    let minor = (tty & 0xff) | ((tty >> 12) & 0xfff00);
    (136..=143)
        .contains(&major)
        .then(|| ((major - 136) * 256 + minor) as u32)
}
fn executable(root: &Path, pid: u32) -> Option<PathBuf> {
    fs::read_link(root.join(pid.to_string()).join("exe")).ok()
}
/// Classify the master holder by executable name only.
pub fn holder_kind(exe: &Path) -> &'static str {
    let name = exe.file_name().and_then(|n| n.to_str()).unwrap_or("");
    if name == "tmux" {
        "tmux"
    } else if name == "screen" || name == "SCREEN" {
        "screen"
    } else if name.starts_with("sshd") {
        "sshd"
    } else {
        "terminal"
    }
}
/// Pseudo-terminal indexes whose master is open in `pid`.
fn masters(root: &Path, pid: u32) -> Vec<u32> {
    let base = root.join(pid.to_string());
    let Ok(entries) = fs::read_dir(base.join("fd")) else {
        return vec![];
    };
    let mut found = vec![];
    for entry in entries.flatten().take(FD_LIMIT) {
        let Ok(target) = fs::read_link(entry.path()) else {
            continue;
        };
        if target != Path::new("/dev/ptmx") && target != Path::new("/dev/pts/ptmx") {
            continue;
        }
        let info = fs::read_to_string(base.join("fdinfo").join(entry.file_name()));
        if let Some(index) = info.ok().and_then(|info| {
            info.lines()
                .find_map(|l| l.strip_prefix("tty-index:"))
                .and_then(|v| v.trim().parse().ok())
        }) {
            found.push(index);
        }
    }
    found
}

/// Holders whose terminals belong to CX and must never be offered for adoption.
pub struct Exclusions {
    /// The CX executable, including adoption relays launched through it.
    pub executables: Vec<PathBuf>,
    /// Sessions of the calling process (the viewer's own terminal).
    pub sessions: Vec<i32>,
    /// Managed tmux socket; its server holds every managed pane.
    pub managed_socket: Option<PathBuf>,
}

fn managed_server(root: &Path, pid: u32, socket: &Path) -> bool {
    let Ok(bytes) = fs::read(root.join(pid.to_string()).join("cmdline")) else {
        return false;
    };
    let args: Vec<&[u8]> = bytes.split(|b| *b == 0).take(16).collect();
    args.windows(2)
        .any(|w| w[0] == b"-S" && w[1] == socket.as_os_str().as_encoded_bytes())
}

/// Scan `root` (normally `/proc`) for this user's pseudo-terminal sessions.
/// The boolean reports whether the scan completed within its bounds.
pub fn discover(root: &Path, uid: u32, exclude: &Exclusions) -> (Vec<Terminal>, bool) {
    let started = Instant::now();
    let mut complete = true;
    let mut leaders = vec![];
    let mut holders: HashMap<u32, Holder> = HashMap::new();
    let mut excluded = std::collections::HashSet::new();
    let Ok(entries) = fs::read_dir(root) else {
        return (vec![], false);
    };
    let mut pids: Vec<u32> = entries
        .flatten()
        .filter_map(|e| e.file_name().to_str()?.parse().ok())
        .collect();
    pids.sort_unstable();
    if pids.len() > PROCESS_LIMIT {
        pids.truncate(PROCESS_LIMIT);
        complete = false;
    }
    for pid in pids {
        if started.elapsed() > BUDGET {
            complete = false;
            break;
        }
        let dir = root.join(pid.to_string());
        if fs::metadata(&dir).map(|m| m.uid()).ok() != Some(uid) {
            continue;
        }
        let Some(stat) = stat(root, pid) else {
            continue;
        };
        if stat.session == pid as i32 {
            if let Some(index) = pts_index(stat.tty) {
                if !exclude.sessions.contains(&stat.session) {
                    leaders.push(Terminal {
                        index,
                        leader: Process {
                            pid,
                            start_ticks: stat.start_ticks.clone(),
                        },
                        foreground: stat.foreground,
                        holder: None,
                    });
                }
            }
        }
        let indexes = masters(root, pid);
        if indexes.is_empty() {
            continue;
        }
        let exe = executable(root, pid).unwrap_or_default();
        let is_cx = exclude.executables.iter().any(|e| *e == exe);
        let is_managed = exclude
            .managed_socket
            .as_deref()
            .is_some_and(|s| holder_kind(&exe) == "tmux" && managed_server(root, pid, s));
        for index in indexes {
            if is_cx || is_managed {
                excluded.insert(index);
            } else {
                holders.entry(index).or_insert_with(|| Holder {
                    process: Process {
                        pid,
                        start_ticks: stat.start_ticks.clone(),
                    },
                    kind: holder_kind(&exe),
                });
            }
        }
    }
    leaders.retain(|t| !excluded.contains(&t.index));
    for terminal in &mut leaders {
        terminal.holder = holders.get(&terminal.index).cloned();
    }
    leaders.sort_by_key(|t| t.index);
    (leaders, complete)
}

/// Stable row identity: a reused PID or terminal number yields a new id.
pub fn session_id(boot_id: &str, terminal: &Terminal) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(format!(
        "{boot_id}:{}:{}:{}",
        terminal.index, terminal.leader.pid, terminal.leader.start_ticks
    ));
    format!("ext-{:x}", digest)[..20].to_string()
}

/// Root-owned install location; `cx device ptrace` places cx-reptyr here.
pub const REPTYR_PATH: &str = "/usr/local/libexec/cx/cx-reptyr";
pub const REPTYR_VERSION: &str = "cx-reptyr 1 (reptyr 0.10.0)";
const CAP_SYS_PTRACE: u32 = 19;

/// Why adoption by terminal steal is unavailable, or the verified relay.
/// How CX gains the privilege to steal a terminal with a relay.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Privilege {
    /// A root-owned relay carrying cap_sys_ptrace; no prompt per move.
    Capability,
    /// A verified relay run through `sudo -A`; prompts for a password.
    Sudo,
}
pub struct Relay {
    pub path: PathBuf,
    pub privilege: Privilege,
}

/// User-installed relay for the sudo path, mirroring the portable tmux location.
fn user_reptyr() -> Option<PathBuf> {
    let path =
        PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/bin/cx-reptyr");
    (version(&path).as_deref() == Some(REPTYR_VERSION)).then_some(path)
}

/// Locate a relay and the privilege it needs, or say why none is usable.
pub fn reptyr() -> Result<Relay, &'static str> {
    // Debug builds accept a fixture relay so tests never need host privilege.
    #[cfg(debug_assertions)]
    if let Some(path) = std::env::var_os("CX_ADOPT_REPTYR") {
        let path = PathBuf::from(path);
        return (version(&path).as_deref() == Some(REPTYR_VERSION))
            .then_some(Relay {
                path,
                privilege: Privilege::Capability,
            })
            .ok_or("fixture cx-reptyr version mismatch");
    }
    if fs::read_to_string("/proc/sys/kernel/yama/ptrace_scope").is_ok_and(|s| s.trim() == "3") {
        return Err("ptrace is disabled on this host (Yama scope 3)");
    }
    let path = PathBuf::from(REPTYR_PATH);
    // A root-owned capability install needs no prompt. Any ancestor being
    // writable by others, a symlink or a non-root owner disqualifies it.
    let rooted =
        std::iter::successors(Some(path.as_path()), |p| p.parent()).try_fold(true, |ok, p| {
            let meta = fs::symlink_metadata(p).ok()?;
            Some(
                ok && meta.uid() == 0 && meta.mode() & 0o022 == 0 && !meta.file_type().is_symlink(),
            )
        });
    if rooted == Some(true)
        && version(&path).as_deref() == Some(REPTYR_VERSION)
        && ptrace_capability(&path)
    {
        return Ok(Relay {
            path,
            privilege: Privilege::Capability,
        });
    }
    // Otherwise a user-owned relay works through one sudo prompt per move.
    user_reptyr()
        .map(|path| Relay {
            path,
            privilege: Privilege::Sudo,
        })
        .ok_or("install cx-reptyr (cx device ptrace enable) to move terminals")
}
fn version(path: &Path) -> Option<String> {
    let out = std::process::Command::new(path)
        .arg("-v")
        .stdin(std::process::Stdio::null())
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}
/// File capability grants effective+permitted CAP_SYS_PTRACE on a mount that honors it.
fn ptrace_capability(path: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let Ok(c_path) = std::ffi::CString::new(path.as_os_str().as_bytes()) else {
        return false;
    };
    let mut fs_stat: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c_path.as_ptr(), &mut fs_stat) } != 0
        || fs_stat.f_flag & libc::ST_NOSUID != 0
    {
        return false;
    }
    let mut data = [0u8; 24];
    let n = unsafe {
        libc::getxattr(
            c_path.as_ptr(),
            c"security.capability".as_ptr(),
            data.as_mut_ptr().cast(),
            data.len(),
        )
    };
    file_capability_grants_ptrace(&data[..n.max(0) as usize])
}
/// Parse `vfs_cap_data`: magic/flags word, then permitted/inheritable pairs.
pub fn file_capability_grants_ptrace(data: &[u8]) -> bool {
    let word = |i: usize| {
        data.get(i..i + 4)
            .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
    };
    let (Some(magic), Some(permitted)) = (word(0), word(4)) else {
        return false;
    };
    let effective = magic & 1 == 1;
    matches!(magic & 0xff00_0000, 0x0100_0000 | 0x0200_0000 | 0x0300_0000)
        && effective
        && permitted & (1 << CAP_SYS_PTRACE) != 0
}

/// Whether `pid` currently holds the master of pseudo-terminal `index`.
pub fn holds_master(pid: u32, index: u32) -> bool {
    masters(Path::new("/proc"), pid).contains(&index)
}
/// Close a single-window terminal emulator whose only terminal was just stolen.
///
/// After the steal the holder no longer holds any pseudo-terminal master, so
/// its window is empty; SIGHUP closes it. A multiplexer (tmux, screen) or a
/// server emulator (gnome-terminal, konsole) still holds other masters and is
/// left untouched, as are reused PIDs.
pub fn close_empty_holder(holder: &ProcessIdentity, kind: &str) -> bool {
    if kind != "terminal" {
        return false;
    }
    let root = Path::new("/proc");
    if start_ticks(holder.pid).as_deref() != Some(holder.start_ticks.as_str()) {
        return false;
    }
    if fs::metadata(root.join(holder.pid.to_string()))
        .map(|m| m.uid())
        .ok()
        != Some(unsafe { libc::geteuid() })
    {
        return false;
    }
    if !masters(root, holder.pid).is_empty() {
        return false;
    }
    unsafe { libc::kill(holder.pid as i32, libc::SIGHUP) == 0 }
}

/// Whether `pid` or any descendant holds the master of `index`. A sudo relay's
/// pane child is `sudo`, with cx-reptyr (holding the master) beneath it.
pub fn descendant_holds_master(pid: u32, index: u32) -> bool {
    let root = Path::new("/proc");
    let mut stack = vec![pid];
    let mut seen = 0;
    while let Some(current) = stack.pop() {
        seen += 1;
        if seen > 64 {
            break;
        }
        if masters(root, current).contains(&index) {
            return true;
        }
        if let Ok(children) =
            fs::read_to_string(root.join(format!("{current}/task/{current}/children")))
        {
            stack.extend(
                children
                    .split_whitespace()
                    .filter_map(|c| c.parse::<u32>().ok()),
            );
        }
    }
    false
}
/// Start ticks of a live process, for PID-reuse checks.
pub fn start_ticks(pid: u32) -> Option<String> {
    stat(Path::new("/proc"), pid).map(|s| s.start_ticks)
}
/// Foreground process group and terminal index of a session leader.
pub fn terminal_of(pid: u32) -> Option<(i32, Option<u32>)> {
    stat(Path::new("/proc"), pid).map(|s| (s.foreground, pts_index(s.tty)))
}
/// Effective capabilities of a process, from its status.
pub fn effective_capabilities(pid: u32) -> Option<u64> {
    fs::read_to_string(format!("/proc/{pid}/status"))
        .ok()?
        .lines()
        .find_map(|l| l.strip_prefix("CapEff:"))
        .and_then(|v| u64::from_str_radix(v.trim(), 16).ok())
}
/// Relay executables whose stolen masters belong to adopted CX sessions.
pub fn relay_executables() -> Vec<PathBuf> {
    let mut found = vec![PathBuf::from(REPTYR_PATH)];
    found.extend(user_reptyr());
    #[cfg(debug_assertions)]
    if let Some(path) = std::env::var_os("CX_ADOPT_REPTYR") {
        found.extend(fs::canonicalize(path));
    }
    found
}

/// Ask every process group of an adopted terminal session to exit.
///
/// The leader identity is re-verified first, and only this user's processes
/// whose session and terminal both match are signalled. reptyr makes the
/// leader ignore SIGHUP, so termination uses SIGTERM.
pub fn terminate(leader: &ProcessIdentity, index: u32) -> usize {
    let root = Path::new("/proc");
    let uid = unsafe { libc::geteuid() };
    let Some(current) = stat(root, leader.pid) else {
        return 0;
    };
    if current.start_ticks != leader.start_ticks
        || current.session != leader.pid as i32
        || pts_index(current.tty) != Some(index)
    {
        return 0;
    }
    let mut groups = std::collections::BTreeSet::new();
    let Ok(entries) = fs::read_dir(root) else {
        return 0;
    };
    for pid in entries
        .flatten()
        .filter_map(|e| e.file_name().to_str()?.parse::<u32>().ok())
        .take(PROCESS_LIMIT)
    {
        if fs::metadata(root.join(pid.to_string()))
            .map(|m| m.uid())
            .ok()
            != Some(uid)
        {
            continue;
        }
        let Ok(text) = fs::read_to_string(root.join(pid.to_string()).join("stat")) else {
            continue;
        };
        let Some((_, rest)) = text.rsplit_once(')') else {
            continue;
        };
        let fields: Vec<_> = rest.split_whitespace().collect();
        let (Some(group), Some(session), Some(tty)) = (
            fields.get(2).and_then(|v| v.parse::<i32>().ok()),
            fields.get(3).and_then(|v| v.parse::<i32>().ok()),
            fields.get(4).and_then(|v| v.parse::<i64>().ok()),
        ) else {
            continue;
        };
        if session == leader.pid as i32 && pts_index(tty) == Some(index) && group > 0 {
            groups.insert(group);
        }
    }
    for group in &groups {
        unsafe { libc::kill(-group, libc::SIGTERM) };
    }
    groups.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    fn process(root: &Path, pid: u32, session: u32, tty: i64, exe: &str) {
        let dir = root.join(pid.to_string());
        fs::create_dir_all(dir.join("fd")).unwrap();
        fs::create_dir_all(dir.join("fdinfo")).unwrap();
        fs::write(
            dir.join("stat"),
            format!(
                "{pid} (x y) S 1 {pid} {session} {tty} {pid} 0 0 0 0 0 0 0 0 0 0 0 0 0 {}00 0",
                pid
            ),
        )
        .unwrap();
        symlink(exe, dir.join("exe")).unwrap();
    }
    fn master(root: &Path, pid: u32, fd: u32, index: u32) {
        let dir = root.join(pid.to_string());
        symlink("/dev/ptmx", dir.join("fd").join(fd.to_string())).unwrap();
        fs::write(
            dir.join("fdinfo").join(fd.to_string()),
            format!("pos:\t0\nflags:\t02\ntty-index:\t{index}\n"),
        )
        .unwrap();
    }
    fn uid() -> u32 {
        unsafe { libc::geteuid() }
    }
    fn none() -> Exclusions {
        Exclusions {
            executables: vec![],
            sessions: vec![],
            managed_socket: None,
        }
    }

    #[test]
    fn decodes_unix98_pseudo_terminals_only() {
        assert_eq!(pts_index(136 << 8 | 3), Some(3));
        assert_eq!(pts_index(137 << 8 | 4), Some(260));
        assert_eq!(pts_index(4 << 8 | 1), None);
        assert_eq!(pts_index(0), None);
    }

    #[test]
    fn pairs_leaders_with_master_holders_and_classifies_them() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        process(root, 10, 1, 0, "/usr/bin/foot");
        master(root, 10, 6, 3);
        process(root, 11, 11, 136 << 8 | 3, "/usr/bin/bash");
        process(root, 20, 1, 0, "/usr/bin/tmux");
        master(root, 20, 9, 4);
        process(root, 21, 21, 136 << 8 | 4, "/usr/bin/zsh");
        process(root, 31, 31, 136 << 8 | 5, "/usr/bin/bash");
        // A non-leader on a terminal is part of that session, not a new row.
        process(root, 12, 11, 136 << 8 | 3, "/usr/bin/claude");
        let (found, complete) = discover(root, uid(), &none());
        assert!(complete);
        assert_eq!(found.len(), 3);
        assert_eq!(found[0].tty(), "/dev/pts/3");
        assert_eq!(found[0].leader.pid, 11);
        assert_eq!(found[0].holder.as_ref().unwrap().kind, "terminal");
        assert_eq!(found[0].holder.as_ref().unwrap().process.pid, 10);
        assert_eq!(found[1].holder.as_ref().unwrap().kind, "tmux");
        // An unreadable master holder leaves the row browse-only.
        assert!(found[2].holder.is_none());
    }

    #[test]
    fn excludes_managed_relays_and_the_viewer_terminal() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        process(root, 10, 1, 0, "/usr/bin/tmux");
        fs::write(
            root.join("10/cmdline"),
            b"tmux\0-u\0-S\0/state/cx/managed.sock\0",
        )
        .unwrap();
        master(root, 10, 5, 1);
        process(root, 11, 11, 136 << 8 | 1, "/usr/bin/bash");
        process(root, 20, 1, 0, "/home/u/.local/bin/cx");
        master(root, 20, 7, 2);
        process(root, 21, 21, 136 << 8 | 2, "/usr/bin/bash");
        process(root, 31, 31, 136 << 8 | 3, "/usr/bin/bash");
        let exclude = Exclusions {
            executables: vec!["/home/u/.local/bin/cx".into()],
            sessions: vec![31],
            managed_socket: Some("/state/cx/managed.sock".into()),
        };
        let (found, _) = discover(root, uid(), &exclude);
        assert!(found.is_empty(), "{found:?}");
    }

    #[test]
    fn ids_change_when_a_terminal_or_pid_is_reused() {
        let t = Terminal {
            index: 3,
            leader: Process {
                pid: 11,
                start_ticks: "100".into(),
            },
            foreground: 11,
            holder: None,
        };
        let a = session_id("boot", &t);
        assert!(a.starts_with("ext-") && a.len() == 20);
        assert_eq!(a, session_id("boot", &t));
        let mut reused = t.clone();
        reused.leader.start_ticks = "200".into();
        assert_ne!(a, session_id("boot", &reused));
        assert_ne!(a, session_id("other-boot", &t));
    }

    #[test]
    fn file_capability_must_be_effective_ptrace() {
        // setcap cap_sys_ptrace+ep: VFS_CAP_REVISION_2 | effective, permitted bit 19.
        let mut data = vec![];
        data.extend(0x0200_0001u32.to_le_bytes());
        data.extend((1u32 << 19).to_le_bytes());
        data.extend([0u8; 16]);
        assert!(file_capability_grants_ptrace(&data));
        let mut not_effective = data.clone();
        not_effective[0] = 0;
        assert!(!file_capability_grants_ptrace(&not_effective));
        let mut other = data.clone();
        other[4..8].copy_from_slice(&(1u32 << 21).to_le_bytes());
        assert!(!file_capability_grants_ptrace(&other));
        assert!(!file_capability_grants_ptrace(&[]));
    }
}
