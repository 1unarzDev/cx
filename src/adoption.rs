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
    zombie: bool,
    group: i32,
    session: i32,
    tty: i64,
    foreground: i32,
    start_ticks: String,
}
fn stat(root: &Path, pid: u32) -> Option<Stat> {
    let text = fs::read_to_string(root.join(pid.to_string()).join("stat")).ok()?;
    let fields: Vec<_> = text.rsplit_once(')')?.1.split_whitespace().collect();
    Some(Stat {
        zombie: *fields.first()? == "Z",
        group: fields.get(2)?.parse().ok()?,
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
            // A terminal led by or currently running CX (another viewer or a
            // CLI invocation) is never adoptable, whatever its session.
            let is_cx = |p: u32| {
                executable(root, p).is_some_and(|exe| exclude.executables.iter().any(|e| *e == exe))
            };
            let cx_here = is_cx(pid) || (stat.foreground > 0 && is_cx(stat.foreground as u32));
            if let Some(index) = pts_index(stat.tty) {
                if !exclude.sessions.contains(&stat.session) && !cx_here {
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

/// The host's Yama ptrace scope (None without Yama: classic same-user ptrace).
pub fn ptrace_scope() -> Option<u8> {
    fs::read_to_string("/proc/sys/kernel/yama/ptrace_scope")
        .ok()?
        .trim()
        .parse()
        .ok()
}

/// Why this host needs cx-reptyr with cap_sys_ptrace (or sudo) to move a terminal.
pub fn scope_note() -> String {
    match ptrace_scope() {
        Some(n) => format!("kernel.yama.ptrace_scope={n}"),
        None => "ptrace of another process tree".into(),
    }
}

/// Locate a relay and the privilege it needs, or say why none is usable.
pub fn reptyr() -> Result<Relay, String> {
    // Debug builds accept a fixture relay so tests never need host privilege.
    #[cfg(debug_assertions)]
    if let Some(path) = std::env::var_os("CX_ADOPT_REPTYR") {
        let path = PathBuf::from(path);
        return (version(&path).as_deref() == Some(REPTYR_VERSION))
            .then_some(Relay {
                path,
                privilege: Privilege::Capability,
            })
            .ok_or_else(|| "fixture cx-reptyr version mismatch".to_string());
    }
    if ptrace_scope() == Some(3) {
        return Err(
            "ptrace is disabled here (kernel.yama.ptrace_scope=3 holds until reboot): \
                    an admin must set it to 1 in /etc/sysctl.d and reboot"
                .into(),
        );
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
        .ok_or_else(|| {
            format!(
                "{} needs cx-reptyr with cap_sys_ptrace: install it once with one sudo password",
                scope_note()
            )
        })
}
fn version(path: &Path) -> Option<String> {
    use std::{io::Read, os::fd::AsRawFd, os::unix::process::CommandExt, process::Stdio};
    // Availability checks must not wait indefinitely for an obsolete/broken
    // relay, nor for a wrapper's descendants retaining its output pipe.
    let spawn = || {
        std::process::Command::new(path)
            .arg("-v")
            .process_group(0)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
    };
    // A relay written just now (an install, a test fixture) can be "text file busy" for a moment:
    // another thread's fork holds a copy of the writer's descriptor until its exec closes it.
    let mut child = spawn();
    for _ in 0..20 {
        match &child {
            Err(e) if e.raw_os_error() == Some(libc::ETXTBSY) => {
                std::thread::sleep(Duration::from_millis(5));
                child = spawn();
            }
            _ => break,
        }
    }
    let mut child = child.ok()?;
    let mut stdout = child.stdout.take()?;
    let fd = stdout.as_raw_fd();
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    let nonblocking =
        flags >= 0 && unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } >= 0;
    let deadline = Instant::now() + Duration::from_millis(250);
    let mut bytes = Vec::new();
    let mut completed = false;
    if nonblocking {
        loop {
            let mut chunk = [0; 129];
            match stdout.read(&mut chunk) {
                Ok(n) => {
                    bytes.extend_from_slice(&chunk[..n]);
                    if bytes.len() > 128 {
                        break;
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
            // Keep the leader unreaped until group cleanup so its PID cannot
            // be recycled into an unrelated process group.
            let mut status: libc::siginfo_t = unsafe { std::mem::zeroed() };
            let observed = unsafe {
                libc::waitid(
                    libc::P_PID,
                    child.id(),
                    &mut status,
                    libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
                )
            };
            if observed == 0 && unsafe { status.si_pid() } != 0 {
                // Drain output written between the read and the exit check.
                if stdout
                    .take(129)
                    .read_to_end(&mut bytes)
                    .is_err_and(|e| e.kind() != std::io::ErrorKind::WouldBlock)
                {
                    break;
                }
                completed = bytes.len() <= 128;
                break;
            }
            if observed != 0 && std::io::Error::last_os_error().raw_os_error() != Some(libc::EINTR)
            {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            if Instant::now() >= deadline {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    unsafe {
        libc::kill(-(child.id() as i32), libc::SIGKILL);
    }
    let status = child.wait().ok()?;
    (completed && status.success()).then(|| String::from_utf8_lossy(&bytes).trim().to_string())
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
/// Whether an adopted terminal has left its previous holder: the session leader
/// is unchanged and the holder (the user's own terminal emulator) no longer holds
/// the master of `index`, or has exited. Only the user's own processes are read:
/// a relay with a file capability, or one that sudo ran and that then dropped to
/// the user, is non-dumpable, so its `/proc/<pid>/fd` is unreadable.
///
/// A holder that exited proves a move only while the leader still has the
/// terminal: had the relay not taken the master, the holder's exit closed it,
/// which hangs the terminal up and leaves the leader's session without one
/// (even when the leader survives the SIGHUP, as reptyr's leader does).
pub fn moved(leader: &ProcessIdentity, holder: &ProcessIdentity, index: u32) -> bool {
    let Some(current) = stat(Path::new("/proc"), leader.pid) else {
        return false;
    };
    if current.start_ticks != leader.start_ticks || pts_index(current.tty) != Some(index) {
        return false;
    }
    if start_ticks(holder.pid).as_deref() != Some(holder.start_ticks.as_str()) {
        return true;
    }
    // An unreadable holder proves nothing.
    fs::read_dir(format!("/proc/{}/fd", holder.pid)).is_ok() && !holds_master(holder.pid, index)
}
/// What became of the window a verified move emptied.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Window {
    /// The same terminal app was reopened attached to the moved session, then
    /// the frozen window was killed.
    Replaced,
    /// The frozen window was closed (or was already gone); nothing replaced it.
    Closed,
    /// Left alone: a multiplexer, a terminal server, or a holder CX cannot verify.
    Kept,
}
impl Window {
    pub fn as_str(self) -> &'static str {
        match self {
            Window::Replaced => "replaced",
            Window::Closed => "closed",
            Window::Kept => "kept",
        }
    }
}
/// What a move's result tells the user about the source window (UI notice, CLI).
pub fn window_note(window: &str) -> Option<&'static str> {
    match window {
        "replaced" => Some("its window was reopened attached to cx"),
        "closed" => Some("its old window was closed"),
        _ => None,
    }
}
/// A replacement window: the command it runs (`cx attach <id>`) and the moved
/// session's leader, whose working directory it opens in.
pub struct Replacement<'a> {
    pub leader: &'a ProcessIdentity,
    pub command: Vec<std::ffi::OsString>,
}

/// Close a single-window terminal emulator whose only terminal was just stolen,
/// first reopening the same emulator attached to the moved session when asked.
///
/// After the steal the holder no longer holds any pseudo-terminal master, so
/// its window is empty. A known standalone terminal emulator is killed outright:
/// some (foot) ignore SIGHUP, and closing the frozen window later would make the
/// emulator signal the moved terminal's process group, killing the work it just
/// handed over. Any other holder (an editor's terminal such as Emacs vterm, VS
/// Code's pty host) only gets SIGHUP, as before, and is never replaced. A
/// multiplexer (tmux, screen), a terminal server (one still holding other
/// masters, or started as a server), a holder whose descriptors cannot be read
/// and reused PIDs are left untouched. A replacement that cannot start (no
/// command form, no display, spawn error) only skips the reopening.
pub fn close_empty_holder(
    holder: &ProcessIdentity,
    kind: &str,
    replace: Option<&Replacement>,
) -> Window {
    if kind != "terminal" {
        return Window::Kept;
    }
    if !alive(holder.pid, &holder.start_ticks) {
        return Window::Closed;
    }
    let root = Path::new("/proc");
    let dir = root.join(holder.pid.to_string());
    if fs::metadata(&dir).map(|m| m.uid()).ok() != Some(unsafe { libc::geteuid() })
        || fs::read_dir(dir.join("fd")).is_err()
        || !masters(root, holder.pid).is_empty()
        || terminal_server(root, holder.pid)
    {
        return Window::Kept;
    }
    let comm = fs::read_to_string(dir.join("comm")).unwrap_or_default();
    let Some((_, form)) = EMULATORS.iter().find(|(name, _)| *name == comm.trim()) else {
        signal(holder.pid, &holder.start_ticks, libc::SIGHUP);
        return Window::Closed;
    };
    let replaced = match (replace, form) {
        (Some(replace), Some(form)) => reopen(holder, form, replace).is_some(),
        _ => false,
    };
    signal(holder.pid, &holder.start_ticks, libc::SIGKILL);
    if replaced {
        Window::Replaced
    } else {
        Window::Closed
    }
}
/// Standalone terminal emulators (by `comm`), whose emptied window may be killed,
/// with the arguments that make a new window run a command (`None`: never reopened).
type Form = Option<&'static [&'static str]>;
const EMULATORS: &[(&str, Form)] = &[
    ("foot", Some(&[])),
    ("kitty", Some(&[])),
    ("alacritty", Some(&["-e"])),
    ("ghostty", Some(&["-e"])),
    ("xterm", Some(&["-e"])),
    ("uxterm", Some(&["-e"])),
    ("urxvt", Some(&["-e"])),
    ("rxvt", Some(&["-e"])),
    ("st", Some(&["-e"])),
    ("konsole", Some(&["-e"])),
    ("tilix", Some(&["-e"])),
    ("wezterm-gui", Some(&["start", "--"])),
    ("xfce4-terminal", Some(&["-x"])),
    ("terminator", Some(&["-x"])),
    ("lxterminal", None),
    ("mate-terminal", None),
    ("sakura", None),
    ("qterminal", None),
    ("terminology", None),
    ("contour", None),
    ("rio", None),
];
/// The holder's own variables a new window needs to reach the same display.
const DISPLAY_KEYS: &[&str] = &[
    "WAYLAND_DISPLAY",
    "DISPLAY",
    "XAUTHORITY",
    "XDG_RUNTIME_DIR",
    "DBUS_SESSION_BUS_ADDRESS",
];
/// Variables kept from CX's own environment: a minimal login environment plus
/// where CX keeps its state, so `cx attach` finds the same managed server.
fn own_key(key: &str) -> bool {
    matches!(
        key,
        "PATH"
            | "HOME"
            | "USER"
            | "LOGNAME"
            | "LANG"
            | "LANGUAGE"
            | "XDG_STATE_HOME"
            | "XDG_CONFIG_HOME"
    ) || key.starts_with("LC_")
}
/// The replacement window's whole environment: `own_key` variables from CX, then
/// only `DISPLAY_KEYS` from the holder's NUL-separated environ. Nothing else (no
/// TERM, no secrets) passes; oversized values are dropped.
pub fn window_env(
    holder_environ: &[u8],
    own: impl Iterator<Item = (std::ffi::OsString, std::ffi::OsString)>,
) -> Vec<(std::ffi::OsString, std::ffi::OsString)> {
    use std::{ffi::OsStr, os::unix::ffi::OsStrExt};
    let mut env: Vec<_> = own
        .filter(|(k, v)| k.to_str().is_some_and(own_key) && v.len() <= 4096)
        .collect();
    for entry in holder_environ.split(|b| *b == 0) {
        let Some(eq) = entry.iter().position(|b| *b == b'=') else {
            continue;
        };
        let (key, value) = (&entry[..eq], &entry[eq + 1..]);
        if DISPLAY_KEYS.iter().any(|k| k.as_bytes() == key) && value.len() <= 4096 {
            env.retain(|(k, _)| k.as_bytes() != key);
            env.push((
                OsStr::from_bytes(key).into(),
                OsStr::from_bytes(value).into(),
            ));
        }
    }
    env
}
/// argv after the program for `form` running `command`.
pub fn emulator_args(form: &[&str], command: &[std::ffi::OsString]) -> Vec<std::ffi::OsString> {
    form.iter()
        .map(Into::into)
        .chain(command.iter().cloned())
        .collect()
}
fn executable_file(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}
/// The emulator's program from its `/proc/<pid>/exe` link. An emulator updated
/// while it ran reads "<path> (deleted)": the new file at that path, else the
/// same name on `PATH`.
pub fn emulator_program(exe: &Path, path: Option<&std::ffi::OsStr>) -> Option<PathBuf> {
    use std::{ffi::OsStr, os::unix::ffi::OsStrExt};
    if executable_file(exe) {
        return Some(exe.to_path_buf());
    }
    let stripped = exe.as_os_str().as_bytes().strip_suffix(b" (deleted)")?;
    let original = Path::new(OsStr::from_bytes(stripped));
    if executable_file(original) {
        return Some(original.to_path_buf());
    }
    let name = original.file_name()?;
    std::env::split_paths(path?)
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_absolute() && executable_file(candidate))
}
/// Whether the user's session bus in `env` accepts connections (for systemd-run).
fn user_bus(env: &[(std::ffi::OsString, std::ffi::OsString)]) -> bool {
    let get = |key: &str| env.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone());
    let from_address = get("DBUS_SESSION_BUS_ADDRESS").and_then(|a| {
        let a = a.to_str()?.to_string();
        let path = a.strip_prefix("unix:path=")?;
        Some(PathBuf::from(path.split(',').next()?))
    });
    let socket =
        from_address.or_else(|| get("XDG_RUNTIME_DIR").map(|d| PathBuf::from(d).join("bus")));
    socket.is_some_and(|s| std::os::unix::net::UnixStream::connect(s).is_ok())
}
/// Open a new window of the holder's emulator running `replace.command`.
/// Returns None (and starts nothing) when it cannot be done safely.
fn reopen(holder: &ProcessIdentity, form: &[&str], replace: &Replacement) -> Option<()> {
    use std::io::Read;
    let dir = PathBuf::from(format!("/proc/{}", holder.pid));
    let path = std::env::var_os("PATH");
    let program = emulator_program(&fs::read_link(dir.join("exe")).ok()?, path.as_deref())?;
    let mut environ = Vec::new();
    fs::File::open(dir.join("environ"))
        .ok()?
        .take(256 * 1024)
        .read_to_end(&mut environ)
        .ok()?;
    // The environment read must be the verified holder's.
    if !alive(holder.pid, &holder.start_ticks) {
        return None;
    }
    let env = window_env(&environ, std::env::vars_os());
    if !env
        .iter()
        .any(|(k, _)| k == "WAYLAND_DISPLAY" || k == "DISPLAY")
    {
        return None;
    }
    let home = env
        .iter()
        .find(|(k, _)| k == "HOME")
        .map(|(_, v)| PathBuf::from(v));
    let cwd = alive(replace.leader.pid, &replace.leader.start_ticks)
        .then(|| fs::read_link(format!("/proc/{}/cwd", replace.leader.pid)).ok())
        .flatten()
        .filter(|d| d.is_dir())
        .or(home.filter(|h| h.is_dir()))
        .unwrap_or_else(|| PathBuf::from("/"));
    let systemd_run = path.as_deref().and_then(|p| {
        std::env::split_paths(p)
            .map(|d| d.join("systemd-run"))
            .find(|c| c.is_absolute() && executable_file(c))
    });
    let scope = systemd_run.filter(|_| user_bus(&env));
    spawn_window(
        &program,
        &emulator_args(form, &replace.command),
        &env,
        &cwd,
        scope.as_deref(),
    )
    .ok()
}
/// Start `program args` detached: its own session, stdio on /dev/null, the given
/// environment only, and (with `scope`, a systemd-run) its own transient user
/// scope, so it is in neither the managed tmux unit nor a viewer pane's cgroup.
/// Never waited on; a thread reaps it whenever it exits.
pub fn spawn_window(
    program: &Path,
    args: &[std::ffi::OsString],
    env: &[(std::ffi::OsString, std::ffi::OsString)],
    cwd: &Path,
    scope: Option<&Path>,
) -> std::io::Result<()> {
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};
    let mut command = match scope {
        Some(systemd_run) => {
            let mut c = Command::new(systemd_run);
            c.args(["--user", "--scope", "--collect", "--quiet", "--"])
                .arg(program);
            c
        }
        None => Command::new(program),
    };
    command
        .args(args)
        .env_clear()
        .envs(env.iter().map(|(k, v)| (k, v)))
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = command.spawn()?;
    std::thread::spawn(move || child.wait());
    Ok(())
}
/// An emulator that serves many windows (gnome-terminal-server, `foot --server`,
/// `kitty --single-instance`, urxvtd) outlives its last window by design.
fn terminal_server(root: &Path, pid: u32) -> bool {
    let name = executable(root, pid)
        .and_then(|e| Some(e.file_name()?.to_string_lossy().into_owned()))
        .unwrap_or_default();
    let args = fs::read(root.join(pid.to_string()).join("cmdline")).unwrap_or_default();
    name.ends_with("-server")
        || name == "urxvtd"
        || args
            .split(|b| *b == 0)
            .take(32)
            .any(|a| [b"--server".as_slice(), b"--single-instance", b"--daemon"].contains(&a))
}

/// Send `sig` to `pid` only while it is still the process that started at
/// `start` (not a zombie): through a pidfd, so a pid reused after the check is
/// never signalled; plain kill() only where pidfds are unsupported.
pub fn signal(pid: u32, start: &str, sig: i32) -> bool {
    let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid as libc::pid_t, 0) } as i32;
    if fd < 0 && std::io::Error::last_os_error().raw_os_error() != Some(libc::ENOSYS) {
        return false;
    }
    let sent = alive(pid, start)
        && if fd < 0 {
            unsafe { libc::kill(pid as i32, sig) == 0 }
        } else {
            let null: *const libc::siginfo_t = std::ptr::null();
            unsafe { libc::syscall(libc::SYS_pidfd_send_signal, fd, sig, null, 0) == 0 }
        };
    if fd >= 0 {
        unsafe { libc::close(fd) };
    }
    sent
}

/// Whether `pid` or any descendant runs `exe` (sudo has handed over to the relay).
pub fn descendant_runs(pid: u32, exe: &Path) -> bool {
    let exe = fs::canonicalize(exe).unwrap_or_else(|_| exe.to_path_buf());
    any_descendant(pid, |p| {
        executable(Path::new("/proc"), p).as_ref() == Some(&exe)
    })
}

fn any_descendant(pid: u32, f: impl Fn(u32) -> bool) -> bool {
    let root = Path::new("/proc");
    let mut stack = vec![pid];
    let mut seen = 0;
    while let Some(current) = stack.pop() {
        seen += 1;
        if seen > 64 {
            break;
        }
        if f(current) {
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

/// Whether `pid` still runs (not a zombie) with the start time `start`.
pub fn alive(pid: u32, start: &str) -> bool {
    stat(Path::new("/proc"), pid).is_some_and(|s| !s.zombie && s.start_ticks == start)
}

/// This user's processes under `roots`, roots included, as (pid, start ticks),
/// bounded. A stop snapshots them first so that descendants which left the
/// terminal (setsid, nohup, SIGHUP ignored) can be ended too, and nothing else.
pub fn descendants(roots: &[u32]) -> Vec<(u32, String)> {
    let uid = unsafe { libc::geteuid() };
    let mut out: Vec<(u32, String)> = vec![];
    let mut stack = roots.to_vec();
    while let Some(pid) = stack.pop() {
        if out.len() >= 512 {
            break;
        }
        // (Never the stopping process itself, should it run inside the session.)
        if pid == std::process::id()
            || out.iter().any(|(p, _)| *p == pid)
            || fs::metadata(format!("/proc/{pid}")).map(|m| m.uid()).ok() != Some(uid)
        {
            continue;
        }
        let Some(start) = start_ticks(pid) else {
            continue;
        };
        out.push((pid, start));
        // Children are listed per thread.
        let Ok(tasks) = fs::read_dir(format!("/proc/{pid}/task")) else {
            continue;
        };
        for task in tasks.flatten().take(256) {
            if let Ok(children) = fs::read_to_string(task.path().join("children")) {
                stack.extend(
                    children
                        .split_whitespace()
                        .filter_map(|c| c.parse::<u32>().ok()),
                );
            }
        }
    }
    out
}

/// This user's processes in process group `group` of session `session`.
pub fn group_members(group: i32, session: i32) -> Vec<(u32, String)> {
    let root = Path::new("/proc");
    let uid = unsafe { libc::geteuid() };
    let Ok(entries) = fs::read_dir(root) else {
        return vec![];
    };
    entries
        .flatten()
        .filter_map(|e| e.file_name().to_str()?.parse::<u32>().ok())
        .take(PROCESS_LIMIT)
        .filter(|pid| {
            fs::metadata(root.join(pid.to_string()))
                .map(|m| m.uid())
                .ok()
                == Some(uid)
        })
        .filter_map(|pid| {
            let s = stat(root, pid)?;
            (s.group == group && s.session == session).then_some((pid, s.start_ticks))
        })
        .collect()
}

/// Send `first` to every snapshot process that still runs with its start time,
/// then SIGKILL whatever is left after `grace`. Returns how many were asked.
pub fn reap(snapshot: &[(u32, String)], first: i32, grace: Duration) -> usize {
    let live = || snapshot.iter().filter(|(p, s)| alive(*p, s));
    let asked = live().filter(|(p, s)| signal(*p, s, first)).count();
    let deadline = Instant::now() + grace;
    while live().next().is_some() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    for (pid, start) in live() {
        signal(*pid, start, libc::SIGKILL);
    }
    asked
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
    fn relay_version_preflight_is_bounded() {
        use std::os::unix::fs::PermissionsExt;
        let fixture = tempfile::tempdir().unwrap();
        let relay = fixture.path().join("cx-reptyr");
        fs::write(
            &relay,
            "#!/bin/sh\nsleep 2\nprintf 'cx-reptyr 1 (reptyr 0.10.0)\\n'\n",
        )
        .unwrap();
        fs::set_permissions(&relay, fs::Permissions::from_mode(0o700)).unwrap();
        let started = Instant::now();
        let result = version(&relay);
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "availability waited for a stalled relay"
        );
        assert!(result.is_none(), "a timed-out relay must not be advertised");
    }

    #[test]
    fn relay_version_does_not_wait_for_wrapper_descendants() {
        use std::os::unix::fs::PermissionsExt;
        let fixture = tempfile::tempdir().unwrap();
        let relay = fixture.path().join("cx-reptyr");
        fs::write(
            &relay,
            format!(
                "#!/bin/sh\nsleep 60 &\necho $! > '{}'/child\nprintf '%s\\n' '{}'\n",
                fixture.path().display(),
                REPTYR_VERSION
            ),
        )
        .unwrap();
        fs::set_permissions(&relay, fs::Permissions::from_mode(0o700)).unwrap();
        // The descendant keeps the output pipe open for a minute: waiting for it (the bug) takes
        // that long, however idle the host; the probe itself takes at most its 250 ms deadline, so
        // these bounds hold however loaded the host is.
        let started = Instant::now();
        assert_eq!(version(&relay).as_deref(), Some(REPTYR_VERSION));
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "the probe waited for its descendant"
        );
        let pid = fs::read_to_string(fixture.path().join("child")).unwrap();
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let running = fs::read_to_string(format!("/proc/{}/stat", pid.trim())).is_ok_and(|s| {
                s.rsplit_once(')')
                    .is_some_and(|(_, fields)| !fields.trim_start().starts_with('Z'))
            });
            if !running {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "probe descendant survived cleanup"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn relay_version_bounds_output_and_requires_success() {
        use std::os::unix::fs::PermissionsExt;
        let fixture = tempfile::tempdir().unwrap();
        let relay = fixture.path().join("cx-reptyr");
        for (script, expected) in [
            (
                format!("printf '%s\\n' '{}'", REPTYR_VERSION),
                Some(REPTYR_VERSION),
            ),
            (format!("printf '%s\\n' '{}'; exit 1", REPTYR_VERSION), None),
            ("while :; do printf '0123456789'; done".into(), None),
        ] {
            fs::write(&relay, format!("#!/bin/sh\n{script}\n")).unwrap();
            fs::set_permissions(&relay, fs::Permissions::from_mode(0o700)).unwrap();
            let started = Instant::now();
            assert_eq!(version(&relay).as_deref(), expected);
            assert!(started.elapsed() < Duration::from_secs(1));
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
    fn a_cx_viewer_terminal_is_never_listed() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        // A separate CX viewer leads its own terminal; it must not be adoptable
        // even though it is not the calling process's session.
        process(root, 40, 40, 136 << 8 | 7, "/home/u/.local/bin/cx");
        process(root, 41, 41, 136 << 8 | 8, "/usr/bin/bash");
        let exclude = Exclusions {
            executables: vec!["/home/u/.local/bin/cx".into()],
            sessions: vec![],
            managed_socket: None,
        };
        let (found, _) = discover(root, uid(), &exclude);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].leader.pid, 41);
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

    #[test]
    fn reaping_ends_only_snapshot_processes_that_outlived_their_session() {
        use std::process::{Command, Stdio};
        // A shell whose setsid'd grandchild ignores SIGHUP and SIGTERM.
        let mut shell = Command::new("sh")
            .args([
                "-c",
                "setsid sh -c 'trap \"\" HUP TERM; sleep 60; :' & sleep 60; :",
            ])
            .stdin(Stdio::null())
            .spawn()
            .unwrap();
        let mut bystander = Command::new("sleep").arg("60").spawn().unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut snapshot = descendants(&[shell.id()]);
        while snapshot.len() < 4 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
            snapshot = descendants(&[shell.id()]);
        }
        assert_eq!(snapshot.len(), 4, "{snapshot:?}");
        assert!(!snapshot.iter().any(|(p, _)| *p == bystander.id()));
        // The session ends (tmux hangs up the pane); the setsid tree survives.
        shell.kill().unwrap();
        shell.wait().unwrap();
        let survivors: Vec<_> = snapshot
            .iter()
            .filter(|(p, s)| alive(*p, s))
            .cloned()
            .collect();
        assert_eq!(survivors.len(), 3, "{survivors:?}");
        // A recycled pid (other start time) is never signalled.
        let mut probe = survivors.clone();
        probe.push((bystander.id(), "1".into()));
        assert_eq!(reap(&probe, libc::SIGTERM, Duration::from_millis(300)), 3);
        let deadline = Instant::now() + Duration::from_secs(3);
        while survivors.iter().any(|(p, s)| alive(*p, s)) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(!survivors.iter().any(|(p, s)| alive(*p, s)));
        assert!(
            bystander.try_wait().unwrap().is_none(),
            "bystander untouched"
        );
        bystander.kill().unwrap();
        bystander.wait().unwrap();
    }

    #[test]
    fn a_move_is_seen_from_holder_and_leader_and_an_exited_holder_needs_the_terminal() {
        use std::os::{fd::AsRawFd, unix::fs::OpenOptionsExt, unix::process::CommandExt};
        let master = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NOCTTY)
            .open("/dev/ptmx")
            .unwrap();
        let mut index: libc::c_uint = 0;
        let unlock: libc::c_int = 0;
        unsafe {
            assert_eq!(
                libc::ioctl(master.as_raw_fd(), libc::TIOCGPTN, &mut index),
                0
            );
            assert_eq!(
                libc::ioctl(master.as_raw_fd(), libc::TIOCSPTLCK, &unlock),
                0
            );
        }
        let slave = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NOCTTY)
            .open(format!("/dev/pts/{index}"))
            .unwrap();
        // A leader that survives the hang-up, as reptyr's (SIGHUP blocked) does.
        let mut child = unsafe {
            std::process::Command::new("sh")
                .args(["-c", "trap '' HUP; sleep 30; :"])
                .stdin(slave)
                .pre_exec(|| {
                    libc::setsid();
                    libc::ioctl(0, libc::TIOCSCTTY, 0);
                    Ok(())
                })
                .spawn()
                .unwrap()
        };
        // (Until its trap is set: SIGHUP's bit in SigIgn.)
        let ignoring = |pid: u32| {
            fs::read_to_string(format!("/proc/{pid}/status"))
                .ok()
                .and_then(|s| {
                    let mask = s.lines().find_map(|l| l.strip_prefix("SigIgn:"))?;
                    u64::from_str_radix(mask.trim(), 16).ok()
                })
                .is_some_and(|m| m & 1 != 0)
        };
        let deadline = Instant::now() + Duration::from_secs(5);
        while !(ignoring(child.id()) && terminal_of(child.id()).and_then(|t| t.1) == Some(index))
            && Instant::now() < deadline
        {
            std::thread::sleep(Duration::from_millis(10));
        }
        let leader = ProcessIdentity {
            pid: child.id(),
            start_ticks: start_ticks(child.id()).unwrap(),
            native_id: None,
        };
        let gone = ProcessIdentity {
            pid: std::process::id(),
            start_ticks: "1".into(),
            native_id: None,
        };
        // Seen from the holder and the leader alone; the relay is never read.
        let me = ProcessIdentity {
            pid: std::process::id(),
            start_ticks: start_ticks(std::process::id()).unwrap(),
            native_id: None,
        };
        assert!(
            !moved(&leader, &me, index),
            "the holder still holds the master"
        );
        assert!(
            !moved(&gone, &gone, index),
            "a changed leader is never a move"
        );
        // The master lives on elsewhere (here: this test, standing in for the relay).
        assert!(
            moved(&leader, &gone, index),
            "the holder exited after the steal"
        );
        // The holder closed the master before any steal: the terminal hangs up.
        drop(master);
        let deadline = Instant::now() + Duration::from_secs(5);
        while moved(&leader, &gone, index) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            alive(leader.pid, &leader.start_ticks),
            "the leader survived the hang-up"
        );
        assert!(
            !moved(&leader, &gone, index),
            "a hung-up terminal was not moved"
        );
        child.kill().unwrap();
        child.wait().unwrap();
    }

    #[test]
    fn only_a_known_emulator_holder_is_killed_others_get_sighup() {
        use std::os::unix::process::ExitStatusExt;
        use std::process::Command;
        let tmp = tempfile::tempdir().unwrap();
        let foot = tmp.path().join("foot");
        fs::copy("/bin/sleep", &foot).unwrap();
        let identity = |c: &std::process::Child| ProcessIdentity {
            pid: c.id(),
            start_ticks: start_ticks(c.id()).unwrap(),
            native_id: None,
        };
        let spawn = |program: &Path, args: &[&str]| {
            // (A copy written just now can be busy while another test's fork holds it.)
            let start = || Command::new(program).args(args).env_clear().spawn();
            let mut child = start();
            for _ in 0..100 {
                match &child {
                    Err(e) if e.raw_os_error() == Some(libc::ETXTBSY) => {
                        std::thread::sleep(Duration::from_millis(10));
                        child = start();
                    }
                    _ => break,
                }
            }
            let child = child.unwrap();
            // (Until the exec has happened: comm names the program.)
            let deadline = Instant::now() + Duration::from_secs(5);
            let want = program.file_name().unwrap().to_str().unwrap().to_string();
            while fs::read_to_string(format!("/proc/{}/comm", child.id()))
                .map(|c| c.trim() != want)
                .unwrap_or(true)
                && Instant::now() < deadline
            {
                std::thread::sleep(Duration::from_millis(5));
            }
            child
        };
        // A known emulator whose window was emptied: SIGKILL.
        let mut emulator = spawn(&foot, &["30"]);
        let id = identity(&emulator);
        assert_eq!(
            close_empty_holder(&id, "tmux", None),
            Window::Kept,
            "multiplexers are never closed"
        );
        let reused = ProcessIdentity {
            start_ticks: "1".into(),
            ..id.clone()
        };
        assert_eq!(
            close_empty_holder(&reused, "terminal", None),
            Window::Closed,
            "a reused pid is never signalled: its window is gone"
        );
        assert_eq!(emulator.try_wait().unwrap(), None);
        // (The holder has no display variables: the replacement is skipped, the close
        // is not. A real display or user bus is never reached from a unit test.)
        let replace = Replacement {
            leader: &id,
            command: vec!["/nonexistent/cx".into(), "attach".into(), "cx-1".into()],
        };
        assert_eq!(
            close_empty_holder(&id, "terminal", Some(&replace)),
            Window::Closed
        );
        assert_eq!(emulator.wait().unwrap().signal(), Some(libc::SIGKILL));
        // Anything else (an editor's terminal, a pty host): only SIGHUP, never replaced.
        let mut other = spawn(Path::new("/bin/sleep"), &["30"]);
        assert_eq!(
            close_empty_holder(&identity(&other), "terminal", Some(&replace)),
            Window::Closed
        );
        assert_eq!(other.wait().unwrap().signal(), Some(libc::SIGHUP));
    }

    #[test]
    fn terminal_servers_are_recognised_by_name_or_argument() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let cases = [
            (
                10,
                "/usr/libexec/gnome-terminal-server",
                "gnome-terminal-server",
                true,
            ),
            (11, "/usr/bin/foot", "foot\0--server", true),
            (12, "/usr/bin/kitty", "kitty\0--single-instance", true),
            (13, "/usr/bin/urxvtd", "urxvtd", true),
            (14, "/usr/bin/foot", "foot\0-e\0claude", false),
        ];
        for (pid, exe, args, _) in cases {
            process(root, pid, pid, 0, exe);
            fs::write(
                root.join(format!("{pid}/cmdline")),
                args.replace("\\0", "\0"),
            )
            .unwrap();
        }
        for (pid, exe, _, server) in cases {
            assert_eq!(terminal_server(root, pid), server, "{exe}");
        }
    }

    #[test]
    fn a_replacement_window_runs_attach_in_each_emulators_command_form() {
        use std::ffi::OsString;
        let command: Vec<OsString> = vec![
            "/home/u/.local/bin/cx".into(),
            "attach".into(),
            "cx-ab".into(),
        ];
        let form = |name: &str| EMULATORS.iter().find(|(n, _)| *n == name).unwrap().1;
        let args = |name: &str| {
            emulator_args(form(name).expect(name), &command)
                .into_iter()
                .map(|a| a.into_string().unwrap())
                .collect::<Vec<_>>()
                .join(" ")
        };
        let tail = "/home/u/.local/bin/cx attach cx-ab";
        for name in ["foot", "kitty"] {
            assert_eq!(args(name), tail, "{name}");
        }
        for name in [
            "alacritty",
            "ghostty",
            "xterm",
            "uxterm",
            "urxvt",
            "rxvt",
            "st",
            "konsole",
            "tilix",
        ] {
            assert_eq!(args(name), format!("-e {tail}"), "{name}");
        }
        assert_eq!(args("wezterm-gui"), format!("start -- {tail}"));
        for name in ["xfce4-terminal", "terminator"] {
            assert_eq!(args(name), format!("-x {tail}"), "{name}");
        }
        // Emulators without a known form are still closed, never reopened.
        for name in [
            "lxterminal",
            "mate-terminal",
            "sakura",
            "qterminal",
            "terminology",
            "contour",
            "rio",
        ] {
            assert!(form(name).is_none(), "{name}");
        }
        assert!(EMULATORS
            .iter()
            .all(|(n, _)| !["emacs", "code", "tmux", "screen", "sshd"].contains(n)));
    }

    #[test]
    fn a_replacement_window_gets_only_display_variables_from_its_holder() {
        use std::ffi::OsString;
        let holder = b"WAYLAND_DISPLAY=wayland-1\0DISPLAY=:0\0XAUTHORITY=/run/user/1/xauth\0\
XDG_RUNTIME_DIR=/run/user/1\0DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1/bus\0\
ANTHROPIC_API_KEY=sk-secret\0TERM=foot\0PATH=/holder/bin\0HOME=/holder\0SSH_AUTH_SOCK=/x\0\
TMUX=/tmp/t,1,0\0broken\0DISPLAY_EXTRA=1";
        let own = [
            ("PATH", "/usr/bin"),
            ("HOME", "/home/u"),
            ("USER", "u"),
            ("LANG", "C.UTF-8"),
            ("LC_CTYPE", "C.UTF-8"),
            ("XDG_STATE_HOME", "/home/u/.state"),
            ("XDG_RUNTIME_DIR", "/run/user/own"),
            ("TERM", "tmux-256color"),
            ("OPENAI_API_KEY", "sk-own"),
            ("CX_MANAGED_SOCKET", "/s"),
            ("TMUX", "/tmp/own"),
        ]
        .map(|(k, v)| (OsString::from(k), OsString::from(v)));
        let mut env: Vec<(String, String)> = window_env(holder, own.into_iter())
            .into_iter()
            .map(|(k, v)| (k.into_string().unwrap(), v.into_string().unwrap()))
            .collect();
        env.sort();
        let expected = [
            ("DBUS_SESSION_BUS_ADDRESS", "unix:path=/run/user/1/bus"),
            ("DISPLAY", ":0"),
            ("HOME", "/home/u"),
            ("LANG", "C.UTF-8"),
            ("LC_CTYPE", "C.UTF-8"),
            ("PATH", "/usr/bin"),
            ("USER", "u"),
            ("WAYLAND_DISPLAY", "wayland-1"),
            ("XAUTHORITY", "/run/user/1/xauth"),
            ("XDG_RUNTIME_DIR", "/run/user/1"),
            ("XDG_STATE_HOME", "/home/u/.state"),
        ]
        .map(|(k, v)| (k.to_string(), v.to_string()));
        assert_eq!(env, expected);
        // Oversized values are dropped.
        let long = format!("DISPLAY={}\0", "x".repeat(5000));
        assert!(window_env(long.as_bytes(), std::iter::empty()).is_empty());
    }

    #[test]
    fn an_updated_emulator_is_found_at_its_path_or_on_path() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let bin = tmp.path().join("bin");
        fs::create_dir(&bin).unwrap();
        let foot = bin.join("foot");
        fs::write(&foot, "#!/bin/sh\n").unwrap();
        fs::set_permissions(&foot, fs::Permissions::from_mode(0o755)).unwrap();
        let path = std::ffi::OsString::from(format!("/nonexistent:{}", bin.display()));
        assert_eq!(emulator_program(&foot, None), Some(foot.clone()));
        let deleted = PathBuf::from(format!("{} (deleted)", foot.display()));
        assert_eq!(emulator_program(&deleted, None), Some(foot.clone()));
        let gone = PathBuf::from("/old/prefix/foot (deleted)");
        assert_eq!(emulator_program(&gone, Some(&path)), Some(foot.clone()));
        assert_eq!(emulator_program(&gone, None), None);
        assert_eq!(
            emulator_program(Path::new("/old/prefix/foot"), Some(&path)),
            None
        );
        fs::set_permissions(&foot, fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(emulator_program(&gone, Some(&path)), None);
    }

    #[test]
    fn a_replacement_window_starts_detached_with_only_its_environment() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let out = tmp.path().join("out");
        let work = tmp.path().join("work");
        fs::create_dir(&work).unwrap();
        let recorder = tmp.path().join("foot");
        fs::write(
            &recorder,
            format!(
                "#!/bin/sh\nstdio=\"$(readlink /proc/$$/fd/0) $(readlink /proc/$$/fd/2)\"\n\
                 {{ echo \"args $*\"; echo \"cwd $(pwd)\"; env | sort; \
                 echo \"stat $(cut -d' ' -f5,6 /proc/$$/stat)\"; echo \"pid $$\"; \
                 echo \"stdio $stdio\"; }} >{}.tmp; mv {0}.tmp {0}\n",
                out.display()
            ),
        )
        .unwrap();
        fs::set_permissions(&recorder, fs::Permissions::from_mode(0o755)).unwrap();
        let env = [
            ("PATH", "/usr/bin:/bin"),
            ("WAYLAND_DISPLAY", "wayland-test"),
        ]
        .map(|(k, v)| (k.into(), v.into()));
        let args = ["/x/cx".into(), "attach".into(), "cx-9".into()];
        let mut started = spawn_window(&recorder, &args, &env, &work, None);
        for _ in 0..100 {
            match &started {
                Err(e) if e.raw_os_error() == Some(libc::ETXTBSY) => {
                    std::thread::sleep(Duration::from_millis(10));
                    started = spawn_window(&recorder, &args, &env, &work, None);
                }
                _ => break,
            }
        }
        started.unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !out.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        let text = fs::read_to_string(&out).unwrap();
        assert!(text.contains("args /x/cx attach cx-9\n"), "{text}");
        assert!(
            text.contains(&format!("cwd {}\n", work.display())),
            "{text}"
        );
        let vars: Vec<_> = text
            .lines()
            .filter(|l| l.contains('=') && !l.starts_with("args") && !l.starts_with("stdio"))
            .map(|l| l.split('=').next().unwrap())
            .filter(|k| !["PWD", "SHLVL", "_", "OLDPWD"].contains(k))
            .collect();
        assert_eq!(vars, ["PATH", "WAYLAND_DISPLAY"], "{text}");
        // Its own session (sid == its pid), stdio on /dev/null.
        let pid = text.lines().find_map(|l| l.strip_prefix("pid ")).unwrap();
        let stat = text.lines().find_map(|l| l.strip_prefix("stat ")).unwrap();
        assert_eq!(stat.split(' ').nth(1), Some(pid), "{text}");
        assert!(text.contains("stdio /dev/null /dev/null\n"), "{text}");
        // A program that cannot start is an error, never a silent success.
        assert!(spawn_window(&tmp.path().join("missing"), &args, &env, &work, None).is_err());
    }
}
