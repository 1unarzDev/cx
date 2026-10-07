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
        let e = e.trim();
        if e.starts_with("no server running on ")
            || (e.starts_with("error connecting to ")
                && (e.ends_with("(No such file or directory)")
                    || e.ends_with("(Connection refused)")))
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
// Codex's working root can differ from its process cwd (notably with -C).
// Retain only that allowlisted path; arguments may otherwise contain prompts.
fn codex_directory_arg(args: &[Vec<u8>]) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStringExt;
    let mut args = args.iter().skip(1);
    let mut directory = None;
    while let Some(arg) = args.next() {
        if arg == b"--" {
            break;
        }
        let path = if arg == b"-C" || arg == b"--cd" {
            args.next()?.as_slice()
        } else if let Some(path) = arg.strip_prefix(b"--cd=") {
            path
        } else if let Some(path) = arg.strip_prefix(b"-C") {
            path
        } else {
            // Skip option values so a config value or image named -C cannot
            // masquerade as the workspace. Unknown flags fail closed.
            if [
                b"-c".as_slice(),
                b"--config",
                b"-m",
                b"--model",
                b"-p",
                b"--profile",
                b"-s",
                b"--sandbox",
                b"-a",
                b"--ask-for-approval",
                b"-i",
                b"--image",
                b"--add-dir",
                b"--enable",
                b"--disable",
                b"--remote",
                b"--remote-auth-token-env",
                b"--local-provider",
            ]
            .contains(&arg.as_slice())
            {
                args.next()?;
            } else if arg.starts_with(b"-")
                && !arg.contains(&b'=')
                && ![
                    b"--last".as_slice(),
                    b"--all",
                    b"--search",
                    b"--no-alt-screen",
                    b"--no-daemon",
                    b"--worktree",
                    b"--oss",
                    b"--strict-config",
                    b"--approve-for-me",
                    b"--dangerously-bypass-approvals-and-sandbox",
                    b"--dangerously-bypass-hook-trust",
                ]
                .contains(&arg.as_slice())
            {
                return None;
            }
            continue;
        };
        if path.is_empty() {
            return None;
        }
        directory = Some(PathBuf::from(std::ffi::OsString::from_vec(path.to_vec())));
    }
    directory
}

fn provider_directory(provider: &str, process: &ProcessIdentity) -> Option<String> {
    use std::io::Read;
    // Guard PID reuse; never retarget a session to a replacement process.
    let start = || -> Option<String> {
        let stat = fs::read_to_string(format!("/proc/{}/stat", process.pid)).ok()?;
        Some(stat.rsplit_once(')')?.1.split_whitespace().nth(19)?.into())
    };
    if start()? != process.start_ticks {
        return None;
    }
    let cwd = fs::read_link(format!("/proc/{}/cwd", process.pid)).ok()?;
    let directory = if provider == "codex" {
        let mut bytes = Vec::new();
        fs::File::open(format!("/proc/{}/cmdline", process.pid))
            .ok()?
            .take(16385)
            .read_to_end(&mut bytes)
            .ok()?;
        if bytes.len() <= 16384 && bytes.last() == Some(&0) {
            let args: Vec<_> = bytes[..bytes.len() - 1]
                .split(|b| *b == 0)
                .map(<[u8]>::to_vec)
                .collect();
            codex_directory_arg(&args)
                .map(|path| {
                    if path.is_absolute() {
                        path
                    } else {
                        cwd.join(path)
                    }
                })
                .unwrap_or(cwd)
        } else {
            cwd
        }
    } else {
        cwd
    };
    if start()? != process.start_ticks {
        return None;
    }
    // Directory is metadata only: it is never a shell command or prompt.
    Some(directory.canonicalize().ok()?.to_str()?.to_owned())
}

fn default_session_name(provider: &str, directory: &str) -> String {
    let folder = directory
        .rsplit('/')
        .find(|s| !s.is_empty())
        .unwrap_or("root");
    let folder: String = folder
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}') {
                '�'
            } else {
                c
            }
        })
        .collect();
    format!("{provider} · {folder}")
}
fn current_session_name(prior: &Session, provider: &str, directory: &str) -> String {
    // Refresh the exact cx-generated title convention; preserve other names.
    if prior.name == default_session_name(&prior.provider, &prior.directory) {
        default_session_name(provider, directory)
    } else {
        prior.name.clone()
    }
}

fn shell_command_label(command: &str) -> Option<String> {
    // Command name only: never arguments, shell history or terminal contents.
    let command = command.trim();
    if command.is_empty()
        || command.len() > 64
        || !command
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._+-".contains(&b))
        || ["fish", "bash", "zsh", "sh", "dash", "tmux", "cx"].contains(&command)
    {
        return None;
    }
    Some(command.into())
}
fn shell_activity(managed: bool, id: &str) -> Option<String> {
    let current = field(managed, id, "#{pane_current_command}")
        .ok()
        .and_then(|name| shell_command_label(&name));
    if let Some(name) = current {
        if managed {
            let mut c = tmux(true).ok()?;
            c.args(["set-option", "-p", "-t", id, "@cx_recent_command", &name]);
            // Best effort; this metadata cannot prevent listing a running terminal.
            let _ = output(c);
        }
        return Some(format!("running {name}"));
    }
    if managed {
        if let Some(name) = field(true, id, "#{@cx_last_command}")
            .ok()
            .and_then(|name| shell_command_label(&name))
        {
            return Some(format!("last: {name}"));
        }
        return field(true, id, "#{@cx_recent_command}")
            .ok()
            .and_then(|name| shell_command_label(&name))
            .map(|name| format!("recent: {name}"));
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
    let directory = process
        .as_ref()
        .filter(|_| provider != "shell")
        .and_then(|process| provider_directory(provider, process))
        .unwrap_or(field(managed, id, "#{pane_current_path}")?);
    let saved_name = prior
        .as_ref()
        .map(|s| current_session_name(s, provider, &directory))
        .unwrap_or_else(|| name.clone());
    let automatic = prior
        .as_ref()
        .is_some_and(|s| s.name == default_session_name(&s.provider, &s.directory));
    let display_name = if automatic && provider == "shell" {
        shell_activity(managed, id).unwrap_or(saved_name)
    } else if automatic {
        process
            .as_ref()
            .and_then(|p| crate::session_titles::native_title(provider, p.pid))
            .unwrap_or(saved_name)
    } else {
        saved_name
    };
    Ok(Session {
        id: if managed { name } else { id.into() },
        name: display_name,
        directory,
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
    // Labels change as commands and native names change; identity ordering stays stable.
    result.sort_by(|a, b| (a.external, &a.id).cmp(&(b.external, &b.id)));
    Ok(result)
}
/// Stop only the exact cx-owned shell shown to the user at confirmation time.
/// Never target personal/external tmux sessions or silently stop a provider takeover.
pub fn stop_shell(id: &str, pid: u32, started: &str, boot_id: &str) -> Result<serde_json::Value> {
    if id.len() != 67
        || !id.starts_with("cx-")
        || !id[3..].bytes().all(|b| b.is_ascii_hexdigit())
        || pid == 0
        || started.is_empty()
        || started.contains("unknown")
    {
        bail!("invalid managed shell identity");
    }
    let current_boot = identity().2;
    if boot_id.is_empty() || current_boot.is_empty() || boot_id != current_boot {
        bail!("execution device restarted; refresh sessions before stopping work");
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
    let record: Session = serde_json::from_slice(
        &fs::read(root.join(format!("{id}.json"))).context("shell ownership record unavailable")?,
    )?;
    if record.id != id
        || record.external
        || record.provider != "shell"
        || record.pid != pid
        || record.started != started
        || record.boot_id != boot_id
    {
        bail!("shell ownership or runtime identity changed; refresh sessions");
    }
    let mut target = None;
    for session_id in ids(true)? {
        if field(true, &session_id, "#{session_name}")? == id {
            target = Some(session_id);
            break;
        }
    }
    let Some(target) = target else {
        return Ok(serde_json::json!({"status": "already_stopped"}));
    };
    // Refuse expanded terminals: stopping a whole session must not kill unshown work.
    let mut panes = tmux(true)?;
    panes.args(["list-panes", "-s", "-t", &target, "-F", "#{pane_id}"]);
    if output(panes)?.lines().count() != 1 {
        bail!("shell has multiple panes; stop it from its terminal instead");
    }
    let current = inspect(true, &target)?;
    if current.id != id
        || current.pid != pid
        || current.started != started
        || current.boot_id != boot_id
        || current.provider != "shell"
    {
        bail!("session changed or is running an agent; refresh sessions");
    }
    reject_provider_descendants(pid)?;
    // Check again immediately before submitting an atomic tmux identity/shape guard.
    let current = inspect(true, &target)?;
    if current.pid != pid || current.started != started || current.provider != "shell" {
        bail!("session changed while confirming; refresh sessions");
    }
    reject_provider_descendants(pid)?;
    let created = started
        .split_once(':')
        .context("invalid process start identity")?
        .0;
    if !created.bytes().all(|b| b.is_ascii_digit()) || created.is_empty() {
        bail!("invalid process start identity");
    }
    let runtime_guard =
        format!("#{{&&:#{{==:#{{pane_pid}},{pid}}},#{{==:#{{session_created}},{created}}}}}");
    let shape_guard = "#{&&:#{==:#{session_windows},1},#{==:#{window_panes},1}}";
    // tmux checks its current command again in the same command queue as the kill.
    // This narrows (but cannot completely eliminate) concurrent provider-start races.
    let provider_guard =
        "#{&&:#{!=:#{pane_current_command},claude},#{!=:#{pane_current_command},codex}}";
    let guard = format!(
        "#{{&&:#{{&&:{runtime_guard},{shape_guard}}},#{{&&:{provider_guard},#{{==:#{{session_name}},{id}}}}}}}"
    );
    let mut stop = tmux(true)?;
    stop.args([
        "if-shell",
        "-F",
        "-t",
        &target,
        &guard,
        &format!("kill-session -t {target}"),
        "display-message -p identity_changed",
    ]);
    let result = output(stop)?;
    if result.contains("identity_changed") {
        bail!("session changed while confirming; refresh sessions");
    }
    // Keep the original record as a creation/idempotency tombstone.
    if ids(true)?.iter().any(|session| session == &target) {
        bail!("shell stop was not confirmed; refresh sessions before retrying");
    }
    Ok(serde_json::json!({"status": "stopped"}))
}

fn reject_provider_descendants(pane: u32) -> Result<()> {
    let mut queue = std::collections::VecDeque::from([(pane, 0)]);
    let mut visited = 0;
    while let Some((pid, depth)) = queue.pop_front() {
        visited += 1;
        if visited > 128 || depth > 12 {
            bail!("shell process tree exceeds safe stop limits; stop it from its terminal");
        }
        if running_provider(pid).is_some() {
            bail!("shell is running an agent; stop it from its terminal instead");
        }
        let children = match fs::read_to_string(format!("/proc/{pid}/task/{pid}/children")) {
            Ok(children) => children,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => bail!("cannot verify shell processes; stop it from its terminal"),
        };
        for child in children.split_whitespace() {
            queue.push_back((child.parse::<u32>()?, depth + 1));
        }
    }
    Ok(())
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
// resets restore pane defaults without editing shell config. They are safe only
// inside managed tmux: on a native viewer terminal they erase its dynamic theme.
const FISH_VIEWER_PALETTE: &str = r"printf '%b' '\e]104\e\\' '\e]110\e\\' '\e]111\e\\'";
// Installed only in newly created managed Fish shells; no personal startup file changes.
// Token parsing happens in-process and persists only a conservative first command name.
const FISH_COMMAND_LABEL: &str = r#"
function __cx_command_label --on-event fish_preexec
    set -l token (string split -m 1 ' ' -- (string trim -- $argv[1]))[1]
    set -l name (string replace -r '^.*/' '' -- $token)
    if string match -qr '^[a-zA-Z0-9_][a-zA-Z0-9_.+-]{0,63}$' -- $name
        switch $name
            case fish bash zsh sh dash tmux cx
                return
        end
        command timeout 0.2s "$CX_TMUX_BIN" -u -S "$CX_MANAGED_SOCKET" set-option -p -t "$TMUX_PANE" @cx_last_command "$name" >/dev/null 2>&1
    end
end
"#;

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
    let display_name = if request.name.is_empty() {
        default_session_name(&request.provider, &directory.to_string_lossy())
    } else {
        request.name.clone()
    };
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
        name: display_name.clone(),
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
        launcher.extend([
            "--init-command".into(),
            format!("{FISH_VIEWER_PALETTE}; {FISH_COMMAND_LABEL}"),
        ]);
    }
    launcher.extend(["-l".into()]);
    if request.provider != "shell" {
        launcher.extend(["-i".into(), "-c".into(), request.provider.clone()]);
    }
    let command = format!(
        "exec env CX_VIEWER_THEME=1 CX_TMUX_BIN={} CX_MANAGED_SOCKET={} {}",
        quote(
            tmux_executable()
                .to_str()
                .context("tmux path is not UTF-8")?
        ),
        quote(socket()?.to_str().context("socket path is not UTF-8")?),
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
    session.name = display_name;
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
    set_managed_status(&session)?;
    Ok(session)
}
fn set_managed_status(session: &Session) -> Result<()> {
    let safe = |value: &str| {
        value
            .chars()
            .filter(|c| {
                !c.is_control()
                    && *c != '#'
                    && !matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
            })
            .take(50)
            .collect::<String>()
    };
    let status = format!(
        "{}@{} | {}",
        safe(&session.account),
        safe(&session.host),
        safe(&session.name)
    );
    let mut command = tmux(true)?;
    command.args(["set-option", "-t", &session.id, "status-right", &status]);
    output(command)?;
    Ok(())
}
pub fn refresh_managed_status(id: &str) -> Result<()> {
    let id = id.strip_prefix('=').unwrap_or(id);
    if !id.starts_with("cx-") || id.len() != 67 || !id[3..].bytes().all(|b| b.is_ascii_hexdigit()) {
        bail!("invalid managed session identity");
    }
    set_managed_status(&inspect(true, id)?)
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
        refresh_managed_status(&session.id)?;
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
        let mut c = crate::store::ssh(target, true)?;
        // Insert PTY options before the destination already owned by ssh().
        let ssh_args = c.get_args().map(|a| a.to_os_string()).collect::<Vec<_>>();
        let mut with_pty = Command::new("ssh");
        with_pty.arg("-t").args(ssh_args);
        c = with_pty;
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
        let routed = crate::store::ssh(target, true)?;
        let mut command = Command::new("ssh");
        command.arg("-tt").args(routed.get_args());
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
    // This shell writes directly to the viewing terminal. Palette resets here
    // would erase its dynamic colors/background; only managed tmux panes may
    // use FISH_VIEWER_PALETTE (tmux scopes those changes to the pane).
    let fish = std::path::Path::new(&shell)
        .file_name()
        .is_some_and(|s| s == "fish");
    // One-shot commands retain interactive shell startup and functions, but
    // their processes share the launcher's owned group. Otherwise a login
    // shell can ignore suspend or strand a stopped job in a separate group.
    let posix = std::path::Path::new(&shell)
        .file_name()
        .is_some_and(|s| s == "sh" || s == "dash");
    let script = if posix {
        // POSIX batch commands use login startup and default native signals.
        // Interactive dash otherwise returns to a prompt after interruption.
        r#"command set +m; command eval "$1""#
    } else if fish {
        "function __cx_suspend --on-signal TSTP; command kill -STOP $fish_pid; end; status job-control none; eval $argv[1]; exit $status"
    } else {
        r#"builtin set +m
builtin trap 'command kill -STOP $$' TSTP
(builtin trap - INT QUIT TSTP TTIN TTOU; builtin eval "$1") < /dev/tty &
__cx_child=$!
builtin wait "$__cx_child"
__cx_status=$?
while command kill -0 "$__cx_child" 2>/dev/null; do
    builtin wait "$__cx_child"
    __cx_status=$?
done
exit "$__cx_status""#
    };
    if posix {
        command.args(["-l", "-c", script]);
    } else {
        command.args(["-l", "-i", "-c", script]);
    }
    if !fish {
        command.arg("cx-command");
    }
    command
        .arg(&request.command)
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
    impl Restore {
        fn now(&self) {
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
    impl Drop for Restore {
        fn drop(&mut self) {
            self.now();
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
    use std::os::unix::process::ExitStatusExt;
    loop {
        let mut status = 0;
        let waited = unsafe { libc::waitpid(pid, &mut status, libc::WUNTRACED) };
        if waited < 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error).context("native command wait failed");
        }
        if libc::WIFSTOPPED(status) {
            // Preserve an application's current modes for an explicit resume.
            let mut child_modes = modes;
            unsafe {
                libc::tcgetattr(fd, &mut child_modes);
            }
            restore.now();
            let resume = stopped_command_choice(fd, pid)?;
            if resume {
                if unsafe { libc::tcsetattr(fd, libc::TCSANOW, &child_modes) } != 0
                    || unsafe { libc::tcsetpgrp(fd, pid) } != 0
                {
                    return Err(std::io::Error::last_os_error().into());
                }
                unsafe {
                    libc::kill(-pid, libc::SIGCONT);
                }
            } else {
                // Cancel is explicit: terminate only the group created above.
                unsafe {
                    libc::kill(-pid, libc::SIGKILL);
                }
            }
        } else if libc::WIFEXITED(status) || libc::WIFSIGNALED(status) {
            drop(restore);
            return Ok(std::process::ExitStatus::from_raw(status));
        }
    }
}

/// A suspended command is never silently discarded. The helper takes back its
/// tty, offers two single-key choices, and treats Ctrl+C as explicit Cancel.
fn stopped_command_choice(fd: i32, pid: libc::pid_t) -> Result<bool> {
    let mut saved: libc::termios = unsafe { std::mem::zeroed() };
    if unsafe { libc::tcgetattr(fd, &mut saved) } != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    struct Modes {
        fd: i32,
        saved: libc::termios,
    }
    impl Drop for Modes {
        fn drop(&mut self) {
            unsafe {
                libc::tcsetattr(self.fd, libc::TCSANOW, &self.saved);
            }
        }
    }
    let _restore = Modes { fd, saved };
    let mut input = saved;
    input.c_lflag &= !(libc::ICANON | libc::ECHO | libc::ISIG);
    input.c_cc[libc::VMIN] = 1;
    input.c_cc[libc::VTIME] = 0;
    if unsafe { libc::tcsetattr(fd, libc::TCSANOW, &input) } != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    println!("\r\nCommand suspended · [r] Resume  [c] Cancel");
    std::io::stdout().flush()?;
    loop {
        let mut key = 0u8;
        let read = unsafe { libc::read(fd, (&mut key as *mut u8).cast(), 1) };
        if read < 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            bail!("suspended command {pid} could not read choice: {error}");
        }
        if read == 0 {
            bail!("terminal closed; command {pid} remains suspended");
        }
        match key {
            b'r' | b'R' => {
                println!("Resume");
                return Ok(true);
            }
            b'c' | b'C' | 3 => {
                println!("Cancel");
                return Ok(false);
            }
            _ => {}
        }
    }
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
    fn shell_labels_never_include_arguments_or_terminal_controls() {
        assert_eq!(shell_command_label("cargo"), Some("cargo".into()));
        assert_eq!(shell_command_label("python3"), Some("python3".into()));
        for name in [
            "bash",
            "fish",
            "curl -H secret",
            "TOKEN=secret",
            "evil\x1b[31m",
            "$(secret)",
            "",
        ] {
            assert!(shell_command_label(name).is_none(), "{name:?}");
        }
    }
    #[test]
    fn codex_workspace_argument_ignores_prompt_and_config_values() {
        let parse = |args: &[&str]| {
            codex_directory_arg(
                &args
                    .iter()
                    .map(|s| s.as_bytes().to_vec())
                    .collect::<Vec<_>>(),
            )
        };
        assert_eq!(
            parse(&["codex", "-C", "/projects/cx"]),
            Some("/projects/cx".into())
        );
        assert_eq!(
            parse(&["codex", "resume", "--last", "--cd=/projects/cx"]),
            Some("/projects/cx".into())
        );
        assert_eq!(
            parse(&["codex", "-C../quoted ' ☃"]),
            Some("../quoted ' ☃".into())
        );
        assert_eq!(
            parse(&["codex", "-c", "--cd=/misleading", "-C", "/correct"]),
            Some("/correct".into())
        );
        assert_eq!(parse(&["codex", "--", "-C", "/prompt"]), None);
        assert_eq!(parse(&["codex", "--future-option", "-C", "/unknown"]), None);
        assert_eq!(parse(&["codex", "-C"]), None);
    }

    #[test]
    fn provider_directory_rejects_pid_reuse() {
        let process = ProcessIdentity {
            pid: std::process::id(),
            start_ticks: "wrong".into(),
            native_id: None,
        };
        assert_eq!(provider_directory("claude", &process), None);
    }

    #[test]
    fn generated_session_names_follow_provider_workspace_and_keep_custom_names() {
        let mut session = Session {
            id: "fixture".into(),
            name: "shell · scaling-law".into(),
            directory: "/projects/scaling-law".into(),
            provider: "shell".into(),
            host: "host".into(),
            account: "account".into(),
            pid: 1,
            started: "start".into(),
            boot_id: "boot".into(),
            external: false,
            socket: None,
            launcher: None,
            process: None,
        };
        assert_eq!(
            current_session_name(&session, "codex", "/projects/cx"),
            "codex · cx"
        );
        assert_eq!(
            current_session_name(&session, "shell", "/projects/new"),
            "shell · new"
        );
        session.name = "My important work".into();
        assert_eq!(
            current_session_name(&session, "codex", "/projects/cx"),
            "My important work"
        );
        assert_eq!(default_session_name("shell", "/"), "shell · root");
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
