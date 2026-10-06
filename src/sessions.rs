use crate::model::{CreateSession, Device, ProcessIdentity, RunCommand, Session};
use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Write,
    os::unix::{
        fs::{OpenOptionsExt, PermissionsExt},
        io::AsRawFd,
    },
    path::PathBuf,
    process::{Command, Stdio},
    thread,
    time::Duration,
};

fn state() -> Result<PathBuf> {
    let root = crate::store::ensure()?;
    fs::create_dir_all(&root)?;
    // Refuse foreign ownership and symlinks before changing permissions.
    use std::os::unix::fs::MetadataExt;
    let meta = fs::symlink_metadata(&root)?;
    if !meta.is_dir() || meta.uid() != unsafe { libc::geteuid() } {
        bail!("unsafe cx state directory");
    }
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700))?;
    Ok(root)
}
pub fn socket() -> Result<PathBuf> {
    Ok(state()?.join("managed.sock"))
}
fn tmux_executable() -> PathBuf {
    let user = PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/bin/tmux");
    if user.is_file() {
        user
    } else {
        PathBuf::from("tmux")
    }
}
fn tmux(managed: bool) -> Result<Command> {
    let mut c = Command::new(tmux_executable());
    // SSH often omits locale variables. cx itself requires a Unicode terminal;
    // keep tmux from replacing Unicode merely because the remote locale is C.
    c.arg("-u");
    if managed {
        c.arg("-S").arg(socket()?);
    }
    Ok(c)
}
fn output(mut c: Command) -> Result<String> {
    let o = c
        .output()
        .context("tmux unavailable; install tmux on the execution device")?;
    if !o.status.success() {
        bail!(
            "tmux operation failed: {}",
            String::from_utf8_lossy(&o.stderr).trim()
        );
    }
    let text = String::from_utf8_lossy(&o.stdout);
    Ok(text.strip_suffix('\n').unwrap_or(&text).to_string())
}
fn field(managed: bool, id: &str, fmt: &str) -> Result<String> {
    let mut c = tmux(managed)?;
    c.args(["display-message", "-p", "-t", id, fmt]);
    output(c)
}
fn identity() -> (String, String, String) {
    let host = fs::read_to_string("/proc/sys/kernel/hostname")
        .unwrap_or_else(|_| "unknown".into())
        .trim()
        .to_string();
    let user = unsafe {
        let p = libc::getpwuid(libc::geteuid());
        if p.is_null() {
            "unknown".into()
        } else {
            std::ffi::CStr::from_ptr((*p).pw_name)
                .to_string_lossy()
                .into_owned()
        }
    };
    let boot = fs::read_to_string("/proc/sys/kernel/random/boot_id")
        .unwrap_or_default()
        .trim()
        .to_string();
    (host, user, boot)
}
fn ids(managed: bool) -> Result<Vec<String>> {
    let mut c = tmux(managed)?;
    c.args(["list-sessions", "-F", "#{session_id}"]);
    let out = c.output()?;
    if !out.status.success() {
        let e = String::from_utf8_lossy(&out.stderr);
        if e.contains("no server running")
            || e.contains("No such file")
            || e.contains("Connection refused")
        {
            return Ok(vec![]);
        }
        bail!("cannot list tmux sessions: {}", e.trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|s| s.starts_with('$') && s[1..].bytes().all(|b| b.is_ascii_digit()))
        .map(str::to_owned)
        .collect())
}
fn process_identity(pane: u32, provider: &str) -> Option<ProcessIdentity> {
    let mut queue = std::collections::VecDeque::from([(pane, 0)]);
    let mut visited = 0;
    while let Some((pid, depth)) = queue.pop_front() {
        visited += 1;
        if visited > 24 {
            return None;
        }
        let comm = fs::read_to_string(format!("/proc/{pid}/comm"))
            .ok()
            .unwrap_or_default();
        if provider == "shell" || comm.trim() == provider {
            let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
            let start_ticks = stat
                .rsplit_once(')')?
                .1
                .split_whitespace()
                .nth(19)?
                .to_owned();
            // Retain only allowlisted identity values; never serialize other environment data.
            use std::io::Read;
            let native_id = fs::File::open(format!("/proc/{pid}/environ"))
                .ok()
                .and_then(|f| {
                    let mut bytes = Vec::new();
                    f.take(65536).read_to_end(&mut bytes).ok()?;
                    bytes.split(|b| *b == 0).find_map(|entry| {
                        let at = entry.iter().position(|b| *b == b'=')?;
                        let (k, v) = entry.split_at(at);
                        if ![
                            b"CODEX_SESSION_ID".as_slice(),
                            b"CODEX_THREAD_ID".as_slice(),
                            b"CLAUDE_SESSION_ID".as_slice(),
                        ]
                        .contains(&k)
                        {
                            return None;
                        }
                        let id = std::str::from_utf8(&v[1..]).ok()?;
                        if id.len() > 256
                            || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
                        {
                            return None;
                        }
                        Some(id.into())
                    })
                });
            return Some(ProcessIdentity {
                pid,
                start_ticks,
                native_id,
            });
        }
        if depth < 3 {
            if let Ok(children) = fs::read_to_string(format!("/proc/{pid}/task/{pid}/children")) {
                for child in children.split_whitespace().take(8) {
                    if let Ok(pid) = child.parse::<u32>() {
                        queue.push_back((pid, depth + 1));
                    }
                }
            }
        }
    }
    None
}
fn running_provider(pid: u32) -> Option<&'static str> {
    let executable = fs::read_link(format!("/proc/{pid}/exe")).ok()?;
    // Claude's native installer uses a version-numbered executable behind claude.
    if executable.parent()?.file_name()?.to_str()? == "versions"
        && executable.parent()?.parent()?.file_name()?.to_str()? == "claude"
        && executable
            .file_name()?
            .to_str()?
            .bytes()
            .all(|b| b.is_ascii_digit() || b == b'.')
    {
        return Some("claude");
    }
    match executable.file_name()?.to_str()? {
        "claude" => Some("claude"),
        "codex" => Some("codex"),
        "node" | "nodejs" | "bun" => {
            // Only examine the executable/script arguments, never prompts or tool inputs.
            use std::io::{BufRead, BufReader, Read};
            let file = fs::File::open(format!("/proc/{pid}/cmdline")).ok()?;
            let mut reader = BufReader::with_capacity(128, file.take(4096));
            let mut script = Vec::new();
            reader.read_until(0, &mut script).ok()?;
            script.clear();
            reader.read_until(0, &mut script).ok()?;
            if script.pop()? != 0 {
                return None;
            }
            if script.ends_with(b"/@anthropic-ai/claude-code/cli.js") {
                Some("claude")
            } else if script.ends_with(b"/@openai/codex/bin/codex.js") {
                Some("codex")
            } else {
                None
            }
        }
        _ => None,
    }
}
fn terminal_group(pid: u32) -> Option<(i32, i64, i32)> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let fields: Vec<_> = stat
        .rsplit_once(')')?
        .1
        .split_whitespace()
        .take(6)
        .collect();
    Some((
        fields.get(2)?.parse().ok()?,
        fields.get(4)?.parse().ok()?,
        fields.get(5)?.parse().ok()?,
    ))
}
fn foreground_provider(pane: u32) -> Option<(&'static str, ProcessIdentity)> {
    let (_, terminal, foreground) = terminal_group(pane)?;
    if terminal == 0 || foreground <= 0 {
        return None;
    }
    let mut queue = std::collections::VecDeque::from([(pane, 0)]);
    let mut visited = 0;
    while let Some((pid, depth)) = queue.pop_front() {
        visited += 1;
        if visited > 64 {
            break;
        }
        if let Some((group, tty, _)) = terminal_group(pid) {
            if group == foreground && tty == terminal {
                if let Some(provider) = running_provider(pid) {
                    let identity = process_identity(pid, provider)
                        .or_else(|| process_identity(pid, "shell"))?;
                    return Some((provider, identity));
                }
            }
        }
        if depth < 6 {
            if let Ok(children) = fs::read_to_string(format!("/proc/{pid}/task/{pid}/children")) {
                for child in children.split_whitespace().take(16) {
                    if let Ok(child) = child.parse() {
                        queue.push_back((child, depth + 1));
                    }
                }
            }
        }
    }
    None
}
fn inspect(managed: bool, id: &str) -> Result<Session> {
    let (host, account, boot_id) = identity();
    let name = field(managed, id, "#{session_name}")?;
    let record = state()?.join(format!("{name}.json"));
    let prior: Option<Session> = if managed
        && name.len() == 67
        && name.starts_with("cx-")
        && name[3..].bytes().all(|b| b.is_ascii_hexdigit())
    {
        fs::read(&record)
            .ok()
            .and_then(|v| serde_json::from_slice(&v).ok())
    } else {
        None
    };
    let pid = field(managed, id, "#{pane_pid}")?.parse().unwrap_or(0);
    let process_start = fs::read_to_string(format!("/proc/{pid}/stat"))
        .ok()
        .and_then(|v| {
            v.rsplit_once(')')
                .and_then(|(_, rest)| rest.split_whitespace().nth(19).map(str::to_owned))
        })
        .unwrap_or_else(|| "unknown".into());
    let started = format!(
        "{}:{process_start}",
        field(managed, id, "#{session_created}")?
    );
    let original_provider = prior
        .as_ref()
        .map(|s| s.provider.as_str())
        .unwrap_or("shell");
    let detected = foreground_provider(pid);
    let provider = detected
        .as_ref()
        .map(|(provider, _)| *provider)
        .unwrap_or(original_provider);
    let process = detected
        .map(|(_, process)| process)
        .or_else(|| process_identity(pid, original_provider));
    Ok(Session {
        id: if managed { name.clone() } else { id.into() },
        name: prior.as_ref().map(|s| s.name.clone()).unwrap_or(name),
        directory: field(managed, id, "#{pane_current_path}")?,
        provider: provider.into(),
        host,
        account,
        pid,
        started,
        boot_id,
        external: !managed,
        process,
        launcher: prior.as_ref().and_then(|s| s.launcher.clone()),
        socket: if managed {
            Some(socket()?.to_string_lossy().into_owned())
        } else {
            None
        },
    })
}
pub fn list() -> Result<Vec<Session>> {
    let mut result = vec![];
    for managed in [true, false] {
        for id in ids(managed)? {
            if let Ok(s) = inspect(managed, &id) {
                result.push(s);
            }
        }
    }
    result.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(result)
}
pub fn set_launch_shell(name: &str) -> Result<serde_json::Value> {
    if !["fish", "bash", "zsh", "sh"].contains(&name) {
        bail!("choose Fish, Bash, Zsh or sh");
    }
    let path = ["/usr/bin", "/bin", "/usr/local/bin"]
        .into_iter()
        .map(|p| PathBuf::from(p).join(name))
        .find(|p| p.is_file())
        .context("selected shell not installed on execution host")?;
    let root = state()?;
    let temp = root.join("launcher.tmp");
    let mut f = fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&temp)?;
    f.write_all(&serde_json::to_vec(&path.to_string_lossy())?)?;
    f.sync_all()?;
    fs::rename(temp, root.join("launcher.json"))?;
    Ok(serde_json::json!({"launcher":path,"applies_to":"new sessions only"}))
}
/// Use exactly the execution host's selected launch profile for both checking and starting.
pub fn launch_shell() -> Result<String> {
    let shell = fs::read(state()?.join("launcher.json"))
        .ok()
        .and_then(|v| serde_json::from_slice::<String>(&v).ok())
        .unwrap_or_else(|| std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into()));
    let basename = std::path::Path::new(&shell)
        .file_name()
        .and_then(|v| v.to_str())
        .unwrap_or("");
    if !["fish", "bash", "zsh", "sh", "dash"].contains(&basename) {
        bail!("unsupported login shell: choose Fish, Bash, Zsh or sh");
    }
    Ok(shell)
}

/// This is runtime availability, not provider authentication or model-route health.
/// No output from shell configuration or providers enters metadata/logs.
pub fn available_providers() -> Result<Vec<&'static str>> {
    let shell = launch_shell()?;
    let mut available = Vec::new();
    for (provider, probe) in [("claude", "claude --version"), ("codex", "codex --version")] {
        if provider_available(&shell, probe) {
            available.push(provider);
        }
    }
    Ok(available)
}

fn provider_available(shell: &str, probe: &str) -> bool {
    let mut command = Command::new(shell);
    let shell_name = std::path::Path::new(shell)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    // Interactive startup is needed for wrappers, but probes do not own a PTY.
    // Disable inherited monitor mode before the version command so ordinary
    // background children remain in the disposable probe's process group.
    let fixed_probe = if ["bash", "zsh", "sh", "dash"].contains(&shell_name) {
        format!("set +m; {probe}")
    } else {
        probe.into()
    };
    command.args(["-l", "-i", "-c", &fixed_probe]);
    bounded_provider_check(command, Duration::from_secs(3))
}

// Fish runs init commands after its normal configuration. These tmux-supported
// resets restore the viewer palette/defaults without editing any shell config.
const FISH_VIEWER_PALETTE: &str = r"printf '%b' '\e]104\e\\' '\e]110\e\\' '\e]111\e\\'";

fn bounded_provider_check(mut command: Command, timeout: Duration) -> bool {
    use std::os::unix::process::CommandExt;
    command.process_group(0);
    let Ok(mut child) = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return false;
    };
    let deadline = std::time::Instant::now() + timeout;
    // Observe exit without reaping: the zombie pins the owned group ID until
    // descendants have been terminated, preventing a recycled-PID signal race.
    let completed = loop {
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
            break true;
        }
        if observed != 0 && std::io::Error::last_os_error().raw_os_error() != Some(libc::EINTR) {
            // Do not signal a group if ownership of its leader cannot be established.
            let _ = child.kill();
            let _ = child.wait();
            return false;
        }
        if std::time::Instant::now() >= deadline {
            break false;
        }
        thread::sleep(Duration::from_millis(10));
    };
    unsafe {
        libc::kill(-(child.id() as i32), libc::SIGKILL);
    }
    child
        .wait()
        .is_ok_and(|status| completed && status.success())
}

fn bounded_terminfo(mut command: Command) -> bool {
    let Ok(mut child) = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return false;
    };
    let deadline = std::time::Instant::now() + Duration::from_millis(200);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) if std::time::Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(5));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
        }
    }
}
fn managed_config() -> String {
    let terminal = ["tmux-256color", "tmux", "screen-256color"]
        .into_iter()
        .find(|name| {
            let mut command = Command::new("infocmp");
            command.arg(name);
            bounded_terminfo(command)
        })
        .unwrap_or("screen-256color");
    let mut config = include_str!("../assets/tmux.conf").replace(
        "set -g default-terminal 'tmux-256color'",
        &format!("set -g default-terminal '{terminal}'"),
    );
    // Reuse the bounded process-group probe; custom tmux wrappers cannot block setup.
    let mut version = Command::new("/bin/sh");
    version.args([
        "-c",
        r#"version=$("$1" -V 2>/dev/null | /usr/bin/head -c 128)
case "$version" in 'tmux '*) version=${version#tmux };; *) exit 1;; esac
major=${version%%.*}; minor=${version#*.}; minor=${minor%%[!0-9]*}
case "$major:$minor" in *[!0-9:]*|:*|*:) exit 1;; esac
[ "$major" -gt 3 ] || { [ "$major" -eq 3 ] && [ "$minor" -ge 4 ]; }
"#,
        "cx-tmux-version",
    ]);
    version.arg(tmux_executable());
    let supports_terminal_color = bounded_provider_check(version, Duration::from_millis(200));
    if !supports_terminal_color {
        // Older tmux rejects `terminal`, opening an error pager instead of the shell.
        config = config.replace("fg=terminal,bg=terminal", "fg=default,bg=default");
    }
    config
}
pub fn configure_managed() -> Result<()> {
    let conf = state()?.join("tmux.conf");
    fs::write(&conf, managed_config())?;
    fs::set_permissions(&conf, fs::Permissions::from_mode(0o600))?;
    let mut c = tmux(true)?;
    c.arg("source-file").arg(conf);
    output(c)?;
    Ok(())
}
fn ensure_server() -> Result<()> {
    if !ids(true)?.is_empty() {
        return configure_managed();
    }
    let root = state()?;
    let conf = root.join("tmux.conf");
    fs::write(&conf, managed_config())?;
    fs::set_permissions(&conf, fs::Permissions::from_mode(0o600))?;
    let sock = socket()?;
    // A foreground tmux server in its own user service cannot inherit the metadata service cgroup.
    let bus = PathBuf::from(format!("/run/user/{}/bus", unsafe { libc::geteuid() }));
    if bus.exists() {
        let unit = format!(
            "--unit=cx-tmux-{:x}",
            Sha256::digest(sock.as_os_str().as_encoded_bytes())
        );
        let status = Command::new("systemd-run")
            .args([
                "--user",
                "--quiet",
                "--collect",
                &unit,
                "--property=Restart=no",
            ])
            .arg(tmux_executable())
            .arg("-u")
            .arg("-D")
            .arg("-S")
            .arg(&sock)
            .arg("-f")
            .arg(&conf)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()?;
        // An existing owned service is acceptable if its socket responds.
        for _ in 0..40 {
            let mut c = tmux(true)?;
            c.args(["show-options", "-g", "exit-empty"]);
            if c.output()?.status.success() {
                return Ok(());
            }
            thread::sleep(Duration::from_millis(25));
        }
        bail!("persistent tmux user service did not start (systemd-run {status}); check user-service access");
    }
    let cgroup = fs::read_to_string("/proc/self/cgroup").unwrap_or_default();
    if cgroup.contains(".service") {
        bail!("user services unavailable; refusing to put sessions in the helper service cgroup");
    }
    let mut c = tmux(true)?;
    c.arg("-f").arg(conf).arg("start-server");
    output(c)?;
    Ok(())
}
pub fn create(request: &CreateSession) -> Result<Session> {
    if request.key.is_empty() || request.key.len() > 1024 {
        bail!("creation key must be 1–1024 bytes");
    }
    if !["shell", "claude", "codex"].contains(&request.provider.as_str()) {
        bail!("unsupported launch profile");
    }
    let directory = fs::canonicalize(crate::files::decode_path(&request.directory)?)
        .context("execution directory unavailable")?;
    if !directory.is_dir() {
        bail!("execution location is not a directory");
    }
    let root = state()?;
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .mode(0o600)
        .open(root.join("sessions.lock"))?;
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX) } != 0 {
        bail!("session lock unavailable");
    }
    let name = format!("cx-{:x}", Sha256::digest(request.key.as_bytes()));
    let mut exists = tmux(true)?;
    exists.args(["has-session", "-t", &format!("={name}")]);
    if exists.output()?.status.success() {
        return inspect(true, &name);
    }
    if root.join(format!("{name}.json")).exists() {
        bail!("original session has ended; use a new creation key to start replacement work");
    }
    let shell = launch_shell()?;
    if request.provider != "shell"
        && !provider_available(
            &shell,
            match request.provider.as_str() {
                "claude" => "claude --version",
                _ => "codex --version",
            },
        )
    {
        bail!(
            "{} is not launchable on this execution device; choose an available profile",
            request.provider
        );
    }
    ensure_server()?;
    let (host, account, boot_id) = identity();
    let pending = Session {
        id: name.clone(),
        name: request.name.clone(),
        directory: directory.to_string_lossy().into_owned(),
        provider: request.provider.clone(),
        host,
        account,
        pid: 0,
        started: String::new(),
        boot_id,
        external: false,
        socket: Some(socket()?.to_string_lossy().into_owned()),
        launcher: Some(shell.clone()),
        process: None,
    };
    // Persist intent before starting: an interrupted helper response must never lose the launcher identity.
    let mut intent = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(root.join(format!("{name}.json")))?;
    intent.write_all(&serde_json::to_vec(&pending)?)?;
    intent.sync_all()?;
    // A single safely quoted shell-command works on tmux 2.7 as well as newer
    // versions; multi-argument new-session and -e were added later.
    let mut launcher = vec![shell.clone()];
    if std::path::Path::new(&shell)
        .file_name()
        .is_some_and(|name| name == "fish")
    {
        launcher.extend(["--init-command".into(), FISH_VIEWER_PALETTE.into()]);
    }
    launcher.extend(["-l".into()]);
    if request.provider != "shell" {
        launcher.extend(["-i".into(), "-c".into(), request.provider.clone()]);
    }
    let command = format!(
        "exec env CX_VIEWER_THEME=1 {}",
        launcher
            .iter()
            .map(|arg| quote(arg))
            .collect::<Vec<_>>()
            .join(" ")
    );
    let mut c = tmux(true)?;
    c.args(["new-session", "-d", "-s", &name, "-n", "work", "-c"])
        .arg(directory)
        .arg(command);
    output(c)?;
    let mut session = inspect(true, &name)?;
    session.name = request.name.clone();
    session.provider = request.provider.clone();
    let temp = root.join(format!("{name}.tmp"));
    let mut f = fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .mode(0o600)
        .open(&temp)?;
    f.write_all(&serde_json::to_vec(&session)?)?;
    f.sync_all()?;
    fs::rename(temp, root.join(format!("{name}.json")))?;
    let safe = |v: &str| {
        v.chars()
            .filter(|c| !c.is_control() && *c != '#')
            .take(50)
            .collect::<String>()
    };
    let status = format!(
        "{}@{} | {}",
        safe(&session.account),
        safe(&session.host),
        safe(&session.name)
    );
    let mut c = tmux(true)?;
    c.args(["set-option", "-t", &name, "status-right", &status]);
    output(c)?;
    Ok(session)
}
fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}
pub fn attach(device: &Device, session: &Session, observe: bool) -> Result<()> {
    if std::env::var_os("TMUX").is_some() {
        bail!("cx attachment from inside tmux requires leaving the outer client first; protected outer sessions are kept intact");
    }
    if session.external {
        if !session.id.starts_with('$') || !session.id[1..].bytes().all(|b| b.is_ascii_digit()) {
            bail!("invalid external session identity");
        }
    } else if !session.id.starts_with("cx-")
        || session.id.len() != 67
        || !session.id[3..].bytes().all(|b| b.is_ascii_hexdigit())
    {
        bail!("invalid managed session identity");
    }
    if !session.external && device.target.is_none() {
        configure_managed()?;
    }
    let mut args = vec![
        "attach-session".to_string(),
        "-t".into(),
        if session.external {
            session.id.clone()
        } else {
            format!("={}", session.id)
        },
    ];
    if observe {
        args.extend(["-r".into(), "-f".into(), "ignore-size".into()]);
    }
    let mut c = if let Some(target) = &device.target {
        if target.starts_with('-') || target.bytes().any(|b| b.is_ascii_whitespace() || b < 32) {
            bail!("invalid SSH target");
        }
        let mut c = Command::new("ssh");
        c.args(["-t", "-o", "ForwardAgent=no", "--", target]);
        let prefix = if session.external {
            "~/.local/bin/cx native-attach --external".to_string()
        } else {
            "~/.local/bin/cx native-attach".to_string()
        };
        c.arg(format!(
            "{prefix} {}",
            args.iter().map(|a| quote(a)).collect::<Vec<_>>().join(" ")
        ));
        c
    } else {
        let mut c = tmux(!session.external)?;
        c.args(args);
        c
    };
    let result = c.status().context("native terminal attachment failed")?;
    if !result.success() {
        bail!("native terminal attachment ended with {result}");
    }
    Ok(())
}

/// Run a one-shot native terminal command, never retrying or recording its text.
pub fn run_command(device: &Device, request: &RunCommand) -> Result<()> {
    use base64::Engine;
    validate_command(request)?;
    let payload = base64::engine::general_purpose::STANDARD.encode(serde_json::to_vec(request)?);
    if payload.len() > 32768 {
        bail!("command payload is too large");
    }
    let mut command = if let Some(target) = &device.target {
        if target.is_empty()
            || target.starts_with('-')
            || target.chars().any(|c| c.is_whitespace() || c.is_control())
        {
            bail!("invalid SSH target");
        }
        let mut command = Command::new("ssh");
        command.args(["-tt", "-o", "ForwardAgent=no", "--", target]);
        // Only base64's fixed alphabet enters the remote shell command.
        command.arg(format!("~/.local/bin/cx native-command '{}'", payload));
        command
    } else {
        let mut command = Command::new(std::env::current_exe()?);
        command.args(["native-command", &payload]);
        command
    };
    let result = foreground_command(&mut command)?;
    if !result.success() {
        bail!("native command connection ended with {result}; command was not retried");
    }
    Ok(())
}

fn validate_command(request: &RunCommand) -> Result<()> {
    if request.command.trim().is_empty()
        || request.command.len() > 8192
        || request.command.contains('\0')
    {
        bail!("command must be nonempty, contain no NUL, and be at most 8192 bytes");
    }
    if request.directory.len() > 16384 || request.directory.contains('\0') {
        bail!("invalid command directory");
    }
    Ok(())
}

/// Native helper endpoint: shell startup/wrappers belong to the execution host.
pub fn execute_command(request: &RunCommand) -> Result<()> {
    use std::io::Read;
    validate_command(request)?;
    let directory = fs::canonicalize(crate::files::decode_path(&request.directory)?)
        .context("command directory is unavailable")?;
    if !directory.is_dir() {
        bail!("command directory is not a folder");
    }
    let shell = launch_shell()?;
    let mut command = Command::new(&shell);
    if std::path::Path::new(&shell)
        .file_name()
        .is_some_and(|s| s == "fish")
    {
        command.args(["--init-command", FISH_VIEWER_PALETTE]);
    }
    command
        .args(["-l", "-i", "-c", &request.command])
        .current_dir(&directory)
        .env("CX_VIEWER_THEME", "1");
    let (host, account, _) = identity();
    let safe = |s: &str| {
        s.chars()
            .filter(|c| !c.is_control())
            .take(200)
            .collect::<String>()
    };
    println!(
        "{}@{}  {}",
        safe(&account),
        safe(&host),
        safe(&directory.to_string_lossy())
    );
    std::io::stdout().flush()?;
    let result = foreground_command(&mut command)?;
    println!("\r\nCommand ended: {result}. Enter to return to cx.");
    std::io::stdout().flush()?;
    // Read the controlling terminal rather than accidentally consuming metadata stdin.
    let mut tty = fs::OpenOptions::new().read(true).open("/dev/tty")?;
    let mut byte = [0];
    loop {
        match tty.read(&mut byte) {
            Ok(0) => break,
            Ok(_) if byte[0] == b'\n' || byte[0] == b'\r' => break,
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

/// Own only this child's process group; restore foreground ownership and terminal
/// modes even when a command exits after changing stty. Blocking SIGTTOU on this
/// thread avoids changing the viewer's global signal handlers.
fn foreground_command(command: &mut Command) -> Result<std::process::ExitStatus> {
    use std::os::unix::process::CommandExt;
    let tty = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty")
        .context("native commands require a controlling terminal")?;
    let fd = tty.as_raw_fd();
    let mut modes: libc::termios = unsafe { std::mem::zeroed() };
    // A freshly exec'd native helper can run before its viewer has handed off
    // the tty. Wait without reading or changing it; never steal another group's
    // foreground ownership ourselves.
    let deadline = std::time::Instant::now() + Duration::from_secs(1);
    let previous = loop {
        let group = unsafe { libc::tcgetpgrp(fd) };
        if group < 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        if group == unsafe { libc::getpgrp() } {
            break group;
        }
        if std::time::Instant::now() >= deadline {
            bail!("native command viewer must own the foreground terminal");
        }
        thread::sleep(Duration::from_millis(2));
    };
    if unsafe { libc::tcgetattr(fd, &mut modes) } != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    struct Restore {
        fd: i32,
        pgrp: libc::pid_t,
        modes: libc::termios,
    }
    impl Drop for Restore {
        fn drop(&mut self) {
            unsafe {
                let mut mask: libc::sigset_t = std::mem::zeroed();
                let mut old: libc::sigset_t = std::mem::zeroed();
                libc::sigemptyset(&mut mask);
                libc::sigaddset(&mut mask, libc::SIGTTOU);
                libc::pthread_sigmask(libc::SIG_BLOCK, &mask, &mut old);
                libc::tcsetpgrp(self.fd, self.pgrp);
                libc::tcsetattr(self.fd, libc::TCSANOW, &self.modes);
                libc::pthread_sigmask(libc::SIG_SETMASK, &old, std::ptr::null_mut());
            }
        }
    }
    let restore = Restore {
        fd,
        pgrp: previous,
        modes,
    };
    command.process_group(0);
    let mut child = command.spawn().context("native command could not start")?;
    let pid = child.id() as libc::pid_t;
    // A child racing this handoff may stop on SIGTTIN; resume only its own group.
    if unsafe { libc::tcsetpgrp(fd, pid) } != 0 {
        let error = std::io::Error::last_os_error();
        unsafe {
            libc::kill(-pid, libc::SIGKILL);
        }
        let _ = child.wait();
        return Err(error.into());
    }
    unsafe {
        libc::kill(-pid, libc::SIGCONT);
    }
    let result = child.wait().context("native command wait failed");
    drop(restore);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    fn assert_probe_child_stopped(pid: &str) {
        let deadline = std::time::Instant::now() + Duration::from_millis(250);
        loop {
            let stat = fs::read_to_string(format!("/proc/{}/stat", pid.trim()));
            if stat.is_err()
                || stat
                    .unwrap()
                    .rsplit_once(')')
                    .unwrap()
                    .1
                    .trim_start()
                    .starts_with('Z')
            {
                return;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "probe child still executing"
            );
            thread::sleep(Duration::from_millis(5));
        }
    }
    #[test]
    fn command_validation_is_bounded() {
        for text in [
            "".into(),
            "   ".into(),
            "bad\0text".into(),
            "x".repeat(8193),
        ] {
            assert!(validate_command(&RunCommand {
                directory: "/tmp".into(),
                command: text
            })
            .is_err());
        }
        assert!(validate_command(&RunCommand {
            directory: "~".into(),
            command: "printf '%s' 'quotes; spaces ☃'".into()
        })
        .is_ok());
    }

    #[test]
    fn native_command_fixture() {
        let Ok(payload) = std::env::var("CX_COMMAND_PTY_FIXTURE") else {
            return;
        };
        let request: RunCommand = serde_json::from_str(&payload).unwrap();
        execute_command(&request).unwrap();
    }

    #[test]
    fn bash_monitor_mode_is_disabled_for_noninteractive_probe_work() {
        let fixture = tempfile::tempdir().unwrap();
        let pid_path = fixture.path().join("child.pid");
        let mut command = Command::new("/bin/bash");
        command.args([
            "--noprofile",
            "--norc",
            "-i",
            "-m",
            "-c",
            "set +m; sleep 5 & echo $! > \"$1\"; wait",
            "probe",
        ]);
        command.arg(&pid_path);
        let start = std::time::Instant::now();
        assert!(!bounded_provider_check(command, Duration::from_millis(150)));
        assert!(start.elapsed() < Duration::from_secs(1));
        let pid = fs::read_to_string(pid_path).unwrap();
        assert_probe_child_stopped(&pid);
    }
    #[test]
    fn exited_probe_wrappers_do_not_leave_background_work() {
        for exit in ["0", "1"] {
            let fixture = tempfile::tempdir().unwrap();
            let pid_path = fixture.path().join("child.pid");
            let mut command = Command::new("/bin/sh");
            command.args(["-c", "sleep 5 & echo $! > \"$1\"; exit \"$2\"", "probe"]);
            command.arg(&pid_path).arg(exit);
            assert_eq!(
                bounded_provider_check(command, Duration::from_millis(150)),
                exit == "0"
            );
            let pid = fs::read_to_string(pid_path).unwrap();
            assert_probe_child_stopped(&pid);
        }
    }
    #[test]
    fn provider_probe_requires_success_and_bounds_wrappers() {
        let mut good = Command::new("sh");
        good.args(["-c", "exit 0"]);
        assert!(bounded_provider_check(good, Duration::from_millis(150)));
        let mut broken = Command::new("sh");
        broken.args(["-c", "echo broken >&2; exit 1"]);
        assert!(!bounded_provider_check(broken, Duration::from_millis(150)));
        let mut stalled = Command::new("sh");
        stalled.args(["-c", "sleep 5 & wait"]);
        let start = std::time::Instant::now();
        assert!(!bounded_provider_check(stalled, Duration::from_millis(150)));
        assert!(start.elapsed() < Duration::from_secs(1));
    }
    #[test]
    fn fish_palette_reset_is_after_config_before_command() {
        if !std::path::Path::new("/usr/bin/fish").exists() {
            return;
        }
        let mut command = Command::new("/usr/bin/fish");
        command.args([
            "--no-config",
            "--init-command",
            FISH_VIEWER_PALETTE,
            "-c",
            "printf AFTER",
        ]);
        let output = command.output().unwrap();
        assert!(output.status.success());
        assert_eq!(
            output.stdout,
            b"\x1b]104\x1b\\\x1b]110\x1b\\\x1b]111\x1b\\AFTER"
        );
    }
    #[test]
    fn managed_launcher_preserves_fish_palette_quoting() {
        if !std::path::Path::new("/usr/bin/fish").exists() {
            return;
        }
        let config = include_str!("../assets/tmux.conf");
        let parser = config
            .lines()
            .find_map(|line| {
                line.strip_prefix("set -g default-shell '")
                    .and_then(|s| s.strip_suffix('\''))
            })
            .expect("managed launcher must select its command parser");
        let args = [
            "/usr/bin/fish",
            "--no-config",
            "--init-command",
            FISH_VIEWER_PALETTE,
            "-c",
            "printf AFTER",
        ];
        let command = format!(
            "exec env CX_VIEWER_THEME=1 {}",
            args.iter()
                .map(|arg| quote(arg))
                .collect::<Vec<_>>()
                .join(" ")
        );
        let output = Command::new(parser)
            .args(["-c", &command])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            output.stdout,
            b"\x1b]104\x1b\\\x1b]110\x1b\\\x1b]111\x1b\\AFTER"
        );
    }
    #[test]
    fn stalled_terminfo_is_bounded_and_reaped() {
        let fixture = tempfile::tempdir().unwrap();
        let pid_path = fixture.path().join("probe-pid");
        let mut command = Command::new("sh");
        command.args(["-c", "echo $$ > \"$1\"; exec sleep 5", "probe"]);
        command.arg(&pid_path);
        let start = std::time::Instant::now();
        assert!(!bounded_terminfo(command));
        assert!(start.elapsed() < Duration::from_secs(1));
        let pid = fs::read_to_string(pid_path).unwrap();
        assert!(!PathBuf::from(format!("/proc/{}", pid.trim())).exists());
    }
}
