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
    } else {
        // An inherited managed client must not redirect external discovery to
        // the managed socket. Use the same default server as remote helpers.
        c.env_remove("TMUX").env_remove("TMUX_PANE");
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
    let out = c.output().context(
        "Persistent sessions require tmux on the execution device; use SSH terminal or Files",
    )?;
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

fn launch_native_title(managed: bool, id: &str, process: &ProcessIdentity) -> Option<String> {
    use std::io::Read;
    if !managed {
        return None;
    }
    let mut bytes = Vec::new();
    fs::File::open(format!("/proc/{}/environ", process.pid))
        .ok()?
        .take(65537)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() > 65536 || bytes.last() != Some(&0) {
        return None;
    }
    let expected = format!("CX_MANAGED_SESSION={id}");
    let entries = bytes.split(|b| *b == 0).collect::<Vec<_>>();
    if !entries.contains(&b"CX_CODEX_TITLE=thread-name-v1".as_slice())
        || !entries.contains(&expected.as_bytes())
    {
        return None;
    }
    // The launcher clears inherited titles before Codex starts. A missing or
    // pending native title must not enable the older ancestry fallback.
    let title = field(managed, id, "#{pane_title}").unwrap_or_default();
    Some(if title == "CX awaiting native title" {
        String::new()
    } else {
        title
    })
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

fn workspace_session_name(provider: &str, directory: &str) -> String {
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
fn default_session_name(provider: &str, directory: &str) -> String {
    if provider == "shell" {
        "New shell".into()
    } else {
        workspace_session_name(provider, directory)
    }
}

fn legacy_generated_shell_name(prior: &Session) -> bool {
    // Older cx UIs sent name="shell-{key}". Verify the creation-key hash;
    // resemblance to the old shape alone must never override a custom name.
    prior.provider == "shell"
        && prior.name.strip_prefix("shell-").is_some_and(|key| {
            !key.is_empty() && prior.id == format!("cx-{:x}", Sha256::digest(key.as_bytes()))
        })
}

fn automatically_named(prior: &Session) -> bool {
    prior.name.trim().is_empty()
        || prior.name == default_session_name(&prior.provider, &prior.directory)
        || prior.name == workspace_session_name(&prior.provider, &prior.directory)
        || legacy_generated_shell_name(prior)
}

fn current_session_name(prior: &Session, provider: &str, directory: &str) -> String {
    // Refresh blank or cx-generated labels; preserve explicitly chosen names.
    if automatically_named(prior) {
        default_session_name(provider, directory)
    } else {
        prior.name.clone()
    }
}

fn automatic_provider_name(
    provider: &str,
    native: Option<&crate::session_titles::NativeMetadata>,
    directory_unknown: bool,
    fallback: String,
) -> String {
    // A proven selected thread can have a title before its directory arrives.
    // Conversely an unknown selection must not revive an old workspace/title.
    let selected = native.filter(|metadata| !metadata.current_thread_unknown);
    if let Some(title) = selected.and_then(|metadata| metadata.title.clone()) {
        title
    } else if directory_unknown {
        provider.into()
    } else {
        fallback
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
            c.args(["set-option", "-t", id, "@cx_recent_command", &name]);
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

fn normalized_activity(status: Option<&str>) -> Option<&'static str> {
    match status? {
        "running" | "working" | "busy" => Some("running"),
        "waiting" | "waiting_for_input" => Some("waiting"),
        "idle" => Some("idle"),
        "output" => Some("output"),
        _ => None,
    }
}

fn recent_terminal_output(timestamp: &str, now: u64) -> Option<String> {
    let observed = timestamp.parse::<u64>().ok()?;
    (observed > 0 && observed <= now && now - observed <= 2).then(|| "output".into())
}

fn inspect(managed: bool, id: &str) -> Result<Session> {
    anyhow::ensure!(
        field(managed, id, "#{pane_dead}")? == "0",
        "session terminal has exited; refresh sessions"
    );
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
    // An adopted terminal's processes are not pane descendants: the pane runs
    // the relay. Follow the original session leader while its identity holds.
    let adopted = prior.as_ref().and_then(|s| s.origin.as_ref()).filter(|o| {
        crate::adoption::start_ticks(o.leader.pid).as_deref() == Some(o.leader.start_ticks.as_str())
    });
    let probe = adopted.map(|o| o.leader.pid).unwrap_or(pid);
    let detected = foreground_provider(probe);
    let provider = detected
        .as_ref()
        .map(|(provider, _)| *provider)
        .unwrap_or(original_provider);
    let process = detected
        .map(|(_, process)| process)
        .or_else(|| process_identity(probe, original_provider));
    // Native metadata follows the live provider thread when its control panel
    // switches projects; argv and process cwd retain only the launch location.
    let tracked_title = process
        .as_ref()
        .filter(|_| provider == "codex")
        .and_then(|p| launch_native_title(managed, &name, p));
    let native = process
        .as_ref()
        .filter(|_| provider != "shell")
        .and_then(|p| {
            crate::session_titles::native_metadata_with_title(
                provider,
                p.pid,
                tracked_title.as_deref(),
            )
        });
    // A controlled launch must prove the selected thread and its directory.
    // Missing SQLite, scan races or a pending title cannot revive launch cwd.
    let current_thread_unknown = native.as_ref().is_some_and(|m| m.current_thread_unknown)
        || (tracked_title.is_some()
            && native.as_ref().and_then(|m| m.directory.as_ref()).is_none());
    let directory = if current_thread_unknown {
        String::new()
    } else {
        native
            .as_ref()
            .and_then(|m| m.directory.clone())
            .or_else(|| {
                process
                    .as_ref()
                    .filter(|_| provider != "shell")
                    .and_then(|process| provider_directory(provider, process))
            })
            .or_else(|| {
                let o = adopted?;
                provider_directory("shell", &o.leader)
            })
            .unwrap_or(field(managed, id, "#{pane_current_path}")?)
    };
    let saved_name = prior
        .as_ref()
        .map(|s| current_session_name(s, provider, &directory))
        .unwrap_or_else(|| name.clone());
    let automatic = prior.as_ref().is_some_and(automatically_named);
    let adopted_command = adopted.and_then(|o| {
        let (foreground, _) = crate::adoption::terminal_of(o.leader.pid)?;
        (foreground > 0 && foreground as u32 != o.leader.pid)
            .then(|| fs::read_to_string(format!("/proc/{foreground}/comm")).ok())
            .flatten()
            .and_then(|c| shell_command_label(&c))
    });
    let display_name = if adopted.is_some() && provider == "shell" {
        adopted_command
            .as_ref()
            .map(|c| format!("running {c}"))
            .unwrap_or(saved_name)
    } else if automatic && provider == "shell" {
        shell_activity(managed, id).unwrap_or(saved_name)
    } else if automatic {
        automatic_provider_name(
            provider,
            native.as_ref(),
            current_thread_unknown,
            saved_name,
        )
    } else {
        saved_name
    };
    let native_activity = native.as_ref().and_then(|m| m.activity.as_deref());
    let activity = normalized_activity(native_activity)
        .map(str::to_owned)
        .or_else(|| {
            if provider == "shell" && adopted.is_some() {
                Some(
                    if adopted_command.is_some() {
                        "running"
                    } else {
                        "idle"
                    }
                    .into(),
                )
            } else if provider == "shell"
                && prior.as_ref().and_then(|s| s.container.as_ref()).is_none()
            {
                field(managed, id, "#{pane_current_command}")
                    .ok()
                    .map(|command| {
                        if shell_command_label(&command).is_some() {
                            "running"
                        } else {
                            "idle"
                        }
                        .into()
                    })
            } else {
                None
            }
        });
    // This indicates recent terminal output, never inferred model work. Native
    // waiting/idle/running status is stronger evidence and takes precedence.
    let activity = activity.or_else(|| {
        if provider != "shell" || prior.as_ref().and_then(|s| s.container.as_ref()).is_some() {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .ok()?
                .as_secs();
            recent_terminal_output(&field(managed, id, "#{window_activity}").ok()?, now)
        } else {
            None
        }
    });
    let agent_tasks = native
        .map(|m| {
            m.agents
                .into_iter()
                .map(|a| crate::model::AgentTask {
                    id: a.id,
                    name: a.name,
                    task: a.task,
                    status: a.status,
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(Session {
        id: if managed { name } else { id.into() },
        name: display_name,
        directory: prior
            .as_ref()
            .and_then(|s| s.container.as_ref())
            .map(|c| c.folder.clone())
            .unwrap_or(directory),
        provider: provider.into(),
        host,
        account,
        pid,
        started,
        boot_id,
        external: !managed,
        process,
        activity,
        agent_tasks,
        origin: prior.as_ref().and_then(|s| s.origin.clone()),
        container: prior.as_ref().and_then(|s| s.container.clone()),
        launcher: prior.as_ref().and_then(|s| s.launcher.clone()),
        socket: if managed {
            Some(socket()?.to_string_lossy().into_owned())
        } else {
            None
        },
    })
}
// Parse only the Docker exec invocation prefix; never inspect its command or
// arguments. Unknown daemon/context options fail closed rather than associating
// a terminal with a different engine's similarly named container.
fn docker_exec_target(args: &[Vec<u8>]) -> Option<String> {
    let mut args = args.iter().skip(1);
    if args.next()?.as_slice() != b"exec" {
        return None;
    }
    while let Some(arg) = args.next() {
        if arg == b"--" {
            return docker_target_name(args.next()?);
        }
        if [
            b"-u".as_slice(),
            b"--user",
            b"-w",
            b"--workdir",
            b"-e",
            b"--env",
            b"--env-file",
            b"--detach-keys",
        ]
        .contains(&arg.as_slice())
        {
            args.next()?;
        } else if [
            b"-i".as_slice(),
            b"-t",
            b"-it",
            b"-ti",
            b"--interactive",
            b"--tty",
            b"--privileged",
        ]
        .contains(&arg.as_slice())
        {
        } else if [
            b"--user=".as_slice(),
            b"--workdir=",
            b"--env=",
            b"--env-file=",
            b"--detach-keys=",
        ]
        .iter()
        .any(|prefix| arg.starts_with(prefix))
        {
        } else if arg.starts_with(b"-") {
            return None;
        } else {
            return docker_target_name(arg);
        }
    }
    None
}
fn docker_target_name(arg: &[u8]) -> Option<String> {
    let value = std::str::from_utf8(arg).ok()?;
    if value.is_empty()
        || value.len() > 256
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
    {
        return None;
    }
    Some(value.into())
}
fn foreground_container(pane: u32) -> Option<String> {
    use std::io::Read;
    let (_, terminal, foreground) = terminal_group(pane)?;
    let mut queue = std::collections::VecDeque::from([(pane, 0)]);
    let mut seen = std::collections::HashSet::new();
    while let Some((pid, depth)) = queue.pop_front() {
        if !seen.insert(pid) {
            continue;
        }
        if seen.len() > 64 {
            return None;
        }
        if terminal_group(pid).is_some_and(|(group, tty, _)| group == foreground && tty == terminal)
            && fs::read_link(format!("/proc/{pid}/exe"))
                .ok()
                .is_some_and(|exe| exe.file_name().is_some_and(|name| name == "docker"))
        {
            let mut bytes = Vec::new();
            fs::File::open(format!("/proc/{pid}/cmdline"))
                .ok()?
                .take(4097)
                .read_to_end(&mut bytes)
                .ok()?;
            if bytes.len() > 4096 || bytes.last() != Some(&0) {
                return None;
            }
            let args = bytes[..bytes.len() - 1]
                .split(|b| *b == 0)
                .take(64)
                .map(<[u8]>::to_vec)
                .collect::<Vec<_>>();
            return docker_exec_target(&args);
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
fn recovered_container(
    target: &str,
    inventory: &[crate::containers::Container],
) -> Option<crate::model::ContainerScope> {
    let mut matches = inventory.iter().filter(|c| {
        c.state == "running"
            && (c.devcontainer || c.allowed)
            && (c.id == target || c.name == target)
    });
    let candidate = matches.next()?;
    if matches.next().is_some() {
        return None;
    }
    Some(candidate.scope())
}

fn container_scope_current(
    scope: &crate::model::ContainerScope,
    inventory: &[crate::containers::Container],
    complete: bool,
) -> bool {
    match inventory.iter().find(|c| c.id == scope.id) {
        Some(c) => {
            c.engine == scope.engine
                && c.started_at == scope.started_at
                && c.state == "running"
                && c.user == scope.user
                && (c.devcontainer || c.allowed)
        }
        // A partial observation never establishes that a missing instance died.
        None => !complete,
    }
}

/// Session row for a terminal started outside CX. Metadata only: adoption
/// re-verifies leader, holder and terminal before changing anything.
fn external_terminal(terminal: &crate::adoption::Terminal) -> Option<Session> {
    let (host, account, boot_id) = identity();
    let leader = terminal.leader.pid;
    let detected = foreground_provider(leader);
    let provider = detected.as_ref().map(|(p, _)| *p).unwrap_or("shell");
    let process = detected
        .map(|(_, process)| process)
        .or_else(|| process_identity(leader, "shell"));
    let native = process
        .as_ref()
        .filter(|_| provider != "shell")
        .and_then(|p| crate::session_titles::native_metadata_with_title(provider, p.pid, None));
    let current_thread_unknown = native.as_ref().is_some_and(|m| m.current_thread_unknown);
    let foreground = (terminal.foreground > 0 && terminal.foreground as u32 != leader)
        .then(|| {
            fs::read_to_string(format!("/proc/{}/comm", terminal.foreground))
                .ok()
                .and_then(|c| shell_command_label(&c))
        })
        .flatten();
    // A discovered terminal always has a real working directory to show. When
    // the native workspace can't be proven (daemon-backed Codex), fall back to
    // the leader's actual cwd rather than leaving it blank.
    let directory = native
        .as_ref()
        .filter(|_| !current_thread_unknown)
        .and_then(|m| m.directory.clone())
        .or_else(|| {
            let p = process.as_ref()?;
            provider_directory(provider, p)
        })
        .or_else(|| {
            provider_directory(
                "shell",
                &ProcessIdentity {
                    pid: leader,
                    start_ticks: terminal.leader.start_ticks.clone(),
                    native_id: None,
                },
            )
        })
        .unwrap_or_default();
    let name = if provider == "shell" {
        foreground
            .as_ref()
            .map(|c| format!("running {c}"))
            .unwrap_or_else(|| workspace_session_name("shell", &directory))
    } else {
        automatic_provider_name(
            provider,
            native.as_ref(),
            current_thread_unknown,
            workspace_session_name(provider, &directory),
        )
    };
    let activity = normalized_activity(native.as_ref().and_then(|m| m.activity.as_deref()))
        .map(str::to_owned)
        .or_else(|| {
            (provider == "shell").then(|| {
                if foreground.is_some() {
                    "running"
                } else {
                    "idle"
                }
                .into()
            })
        });
    let agent_tasks = native
        .map(|m| {
            m.agents
                .into_iter()
                .map(|a| crate::model::AgentTask {
                    id: a.id,
                    name: a.name,
                    task: a.task,
                    status: a.status,
                })
                .collect()
        })
        .unwrap_or_default();
    Some(Session {
        id: crate::adoption::session_id(&boot_id, terminal),
        name,
        directory,
        provider: provider.into(),
        host,
        account,
        pid: leader,
        started: format!("ext:{}", terminal.leader.start_ticks),
        boot_id,
        external: true,
        socket: None,
        launcher: None,
        process,
        container: None,
        activity,
        agent_tasks,
        origin: Some(origin(terminal)),
    })
}
fn origin(terminal: &crate::adoption::Terminal) -> crate::model::ExternalOrigin {
    crate::model::ExternalOrigin {
        kind: terminal
            .holder
            .as_ref()
            .map(|h| h.kind)
            .unwrap_or("unknown")
            .into(),
        tty: terminal.tty(),
        leader: terminal.leader.identity(),
        holder: terminal.holder.as_ref().map(|h| h.process.identity()),
    }
}
fn external_terminals() -> Vec<crate::adoption::Terminal> {
    let mut executables: Vec<_> = std::env::current_exe().into_iter().collect();
    executables.extend(crate::adoption::relay_executables());
    let exclude = crate::adoption::Exclusions {
        executables,
        sessions: vec![unsafe { libc::getsid(0) }],
        managed_socket: socket().ok(),
    };
    crate::adoption::discover(
        std::path::Path::new("/proc"),
        unsafe { libc::geteuid() },
        &exclude,
    )
    .0
}

fn session_lock(root: &std::path::Path) -> Result<fs::File> {
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
    Ok(lock)
}

/// Move a terminal started outside CX into the managed tmux server.
///
/// The guarded relay steals the terminal's master from its current holder, so
/// the running processes, their PIDs, threads and child tools are untouched;
/// only where the terminal is displayed changes. Every identity CX observed is
/// re-verified under the session lock, and failure leaves the original holder.
pub fn adopt(id: &str, origin: &crate::model::ExternalOrigin, boot_id: &str) -> Result<Session> {
    let (_, _, boot) = identity();
    anyhow::ensure!(
        !boot.is_empty() && boot == boot_id,
        "execution device restarted; refresh sessions"
    );
    let relay = crate::adoption::reptyr().map_err(|e| anyhow::anyhow!(e))?;
    let root = state()?;
    let _lock = session_lock(&root)?;
    let name = format!(
        "cx-{:x}",
        Sha256::digest(format!(
            "adopt:{boot}:{}:{}:{}",
            origin.tty, origin.leader.pid, origin.leader.start_ticks
        ))
    );
    let mut exists = tmux(true)?;
    exists.args(["has-session", "-t", &format!("={name}")]);
    if exists.output().is_ok_and(|o| o.status.success()) {
        return inspect(true, &name);
    }
    let terminal = external_terminals()
        .into_iter()
        .find(|t| {
            t.tty() == origin.tty
                && t.leader.pid == origin.leader.pid
                && t.leader.start_ticks == origin.leader.start_ticks
        })
        .context("terminal changed or closed; refresh sessions")?;
    anyhow::ensure!(
        id == crate::adoption::session_id(&boot, &terminal)
            || (id.starts_with('$') && id[1..].bytes().all(|b| b.is_ascii_digit())),
        "invalid external session identity"
    );
    let holder = terminal
        .holder
        .clone()
        .context("this terminal's owner is not accessible; it cannot be moved")?;
    anyhow::ensure!(
        origin.holder.as_ref().is_some_and(
            |h| h.pid == holder.process.pid && h.start_ticks == holder.process.start_ticks
        ),
        "terminal owner changed; refresh sessions"
    );
    let snapshot = external_terminal(&terminal).context("terminal changed; refresh sessions")?;
    ensure_server()?;
    let record = root.join(format!("{name}.json"));
    if record.exists() {
        bail!("this terminal was already moved once; refresh sessions");
    }
    let mut pending = snapshot.clone();
    pending.id = name.clone();
    pending.external = false;
    pending.socket = Some(socket()?.to_string_lossy().into_owned());
    pending.origin = Some(origin.clone());
    let mut intent = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(&record)?;
    intent.write_all(&serde_json::to_vec(&pending)?)?;
    intent.sync_all()?;
    let errors = root.join(format!("{name}.adopt"));
    let leader = &terminal.leader;
    // `sudo -A` reads the password through CX's askpass broker and sets SUDO_UID
    // itself; the relay drops to that user after the steal. A capability relay
    // needs no prompt. Askpass values are environment, never arguments.
    let mut prefix = format!(
        "env CX_VIEWER_THEME=1 CX_TMUX_BIN={} CX_MANAGED_SOCKET={} CX_MANAGED_SESSION={} ",
        quote(
            tmux_executable()
                .to_str()
                .context("tmux path is not UTF-8")?
        ),
        quote(socket()?.to_str().context("socket path is not UTF-8")?),
        quote(&name),
    );
    if relay.privilege == crate::adoption::Privilege::Sudo {
        let env = crate::auth::askpass_env()
            .context("password entry is unavailable; move terminals from the CX interface")?;
        for (key, value) in env {
            prefix.push_str(&format!("{}={} ", key, quote(&value)));
        }
        prefix.push_str("sudo -A -p 'sudo password to move this terminal into CX: ' ");
    }
    let command = format!(
        "exec {}{} -T {} {} {} {} {} {} 2>{}",
        prefix,
        quote(relay.path.to_str().context("relay path is not UTF-8")?),
        leader.pid,
        quote(&leader.start_ticks),
        leader.pid,
        quote(&leader.start_ticks),
        holder.process.pid,
        quote(&holder.process.start_ticks),
        quote(errors.to_str().context("state path is not UTF-8")?),
    );
    let directory = if snapshot.directory.is_empty() {
        std::env::var("HOME").unwrap_or_else(|_| "/".into())
    } else {
        snapshot.directory.clone()
    };
    let mut c = tmux(true)?;
    c.args(["new-session", "-d", "-s", &name, "-n", "work", "-c"])
        .arg(&directory)
        .arg(command);
    if let Err(error) = output(c) {
        let _ = fs::remove_file(&record);
        return Err(error);
    }
    // Success is observed, not assumed: the relay holds the master, the
    // previous holder no longer does, and the relay dropped its capabilities.
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    // tmux prints nothing for a session that no longer exists, so only an
    // explicit "0" proves a live pane.
    let relay_pid = loop {
        let dead = field(true, &name, "#{pane_dead}").map_or(true, |d| d != "0");
        let pane = field(true, &name, "#{pane_pid}")
            .ok()
            .and_then(|p| p.parse::<u32>().ok());
        if let Some(pane) = pane.filter(|_| !dead) {
            // The master now lives under the pane (the relay, possibly beneath
            // sudo) and no longer in the previous holder.
            let moved = crate::adoption::holds_master(pane, terminal.index)
                || crate::adoption::descendant_holds_master(pane, terminal.index);
            if moved && !crate::adoption::holds_master(holder.process.pid, terminal.index) {
                break pane;
            }
        } else {
            let reason = fs::read_to_string(&errors).unwrap_or_default();
            let _ = fs::remove_file(&errors);
            let _ = fs::remove_file(&record);
            let intact = crate::adoption::holds_master(holder.process.pid, terminal.index);
            bail!(
                "could not move this terminal{}: {}",
                if intact { "; it is unchanged" } else { "" },
                reason
                    .lines()
                    .rfind(|l| !l.trim().is_empty())
                    .unwrap_or("relay exited")
                    .trim_start_matches("cx-reptyr: ")
            );
        }
        if std::time::Instant::now() > deadline {
            // Never kill a relay mid-steal: its ptrace state must unwind itself.
            bail!("moving this terminal is taking unusually long; refresh sessions");
        }
        thread::sleep(Duration::from_millis(25));
    };
    let _ = fs::remove_file(&errors);
    let temp = root.join(format!("{name}.tmp"));
    let finish = || -> Result<Session> {
        let mut session = inspect(true, &name)?;
        session.pid = relay_pid;
        session.origin = Some(origin.clone());
        let mut f = fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .mode(0o600)
            .open(&temp)?;
        f.write_all(&serde_json::to_vec(&session)?)?;
        f.sync_all()?;
        fs::rename(&temp, &record)?;
        set_managed_status(&session)?;
        Ok(session)
    };
    let outcome = finish();
    if outcome.is_ok() {
        // The source window is now empty; close it if it was single-window.
        if let Some(h) = origin.holder.as_ref() {
            crate::adoption::close_empty_holder(h, &origin.kind);
        }
    }
    outcome.map_err(|error| {
        // The moved terminal ended immediately (its relay exits with it).
        if field(true, &name, "#{pane_dead}").map_or(true, |d| d != "0") {
            let _ = fs::remove_file(&temp);
            let _ = fs::remove_file(&record);
            anyhow::anyhow!("the terminal closed while it was being moved")
        } else {
            error.context("terminal unavailable after moving")
        }
    })
}

fn adopted_index(origin: &crate::model::ExternalOrigin) -> Result<u32> {
    origin
        .tty
        .strip_prefix("/dev/pts/")
        .and_then(|v| v.parse().ok())
        .context("invalid adopted terminal")
}
/// Managed tmux id of a named session, if it is running.
fn managed_target(name: &str) -> Result<Option<String>> {
    for session_id in ids(true)? {
        if field(true, &session_id, "#{session_name}")? == name {
            return Ok(Some(session_id));
        }
    }
    Ok(None)
}
/// Stop an adopted terminal: its processes first, while the terminal is live,
/// then the relay. Closing the relay hangs up interactive shells with EOF.
fn stop_adopted(
    name: &str,
    relay: u32,
    origin: &crate::model::ExternalOrigin,
) -> Result<serde_json::Value> {
    let index = adopted_index(origin)?;
    let target = managed_target(name)?;
    if let Some(target) = &target {
        anyhow::ensure!(
            field(true, target, "#{pane_pid}")? == relay.to_string(),
            "session runtime identity changed; refresh sessions"
        );
    }
    let groups = crate::adoption::terminate(&origin.leader, index);
    if let Some(target) = target {
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while std::time::Instant::now() < deadline
            && field(true, &target, "#{pane_dead}").is_ok_and(|d| d == "0")
            && crate::adoption::start_ticks(origin.leader.pid).as_deref()
                == Some(origin.leader.start_ticks.as_str())
        {
            thread::sleep(Duration::from_millis(50));
        }
        let mut kill = tmux(true)?;
        kill.args(["kill-session", "-t", &target]);
        // The relay may already have exited with the terminal.
        let _ = kill.output();
    }
    Ok(serde_json::json!({"status": "stopped", "adopted_groups": groups}))
}

/// Leaders of terminals already moved into CX, from managed session records,
/// so a moved terminal is never also offered as external.
fn adopted_origins() -> Vec<(u32, String)> {
    let Ok(root) = state() else {
        return vec![];
    };
    let Ok(entries) = fs::read_dir(&root) else {
        return vec![];
    };
    let mut out = vec![];
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if !(name.starts_with("cx-") && name.ends_with(".json")) {
            continue;
        }
        if let Some(origin) = fs::read(entry.path())
            .ok()
            .and_then(|v| serde_json::from_slice::<Session>(&v).ok())
            .and_then(|s| s.origin)
        {
            out.push((origin.leader.pid, origin.leader.start_ticks));
        }
    }
    out
}

/// Terminals started outside CX: bare terminals, SSH logins and screen. These
/// are shown only in the UI for adoption, never in the managed session list, so
/// the `cx sessions` contract and scripts stay unchanged. tmux-held panes are
/// omitted here: the default server's panes already appear via `list()`.
pub fn external_list() -> Result<Vec<Session>> {
    let adopted = adopted_origins();
    let mut rows = vec![];
    for terminal in external_terminals() {
        if terminal.holder.as_ref().is_some_and(|h| h.kind == "tmux") {
            continue;
        }
        if adopted.iter().any(|(pid, start)| {
            *pid == terminal.leader.pid && *start == terminal.leader.start_ticks
        }) {
            continue;
        }
        if let Some(session) = external_terminal(&terminal) {
            rows.push(session);
        }
    }
    Ok(rows)
}

pub fn list() -> Result<Vec<Session>> {
    let mut result = vec![];
    for managed in [true, false] {
        for id in ids(managed)? {
            // Command-panel panes are transient and shown only in their panel.
            if managed && field(true, &id, "#{session_name}").is_ok_and(|n| valid_pane_name(&n)) {
                continue;
            }
            if let Ok(s) = inspect(managed, &id) {
                result.push(s);
            }
        }
    }
    // Reconcile saved container scopes on every listing, even without a newly
    // observed Docker exec. Otherwise stale JSON records resurrect removed rows.
    // Inventory is also queried when a live foreground Docker exec is observed.
    // Exact names/full IDs are revalidated against a fresh engine observation;
    // no helper deployment, execution inside containers or process reparenting.
    let candidates = result
        .iter()
        .filter(|s| s.container.is_none())
        .any(|s| foreground_container(s.pid).is_some());
    if candidates || result.iter().any(|s| s.container.is_some()) {
        if let Ok(value) = crate::containers::list() {
            if let Ok(inventory) = serde_json::from_value::<Vec<crate::containers::Container>>(
                value["containers"].clone(),
            ) {
                let complete = value["complete"].as_bool() == Some(true)
                    && value["truncated"].as_bool() != Some(true);
                result.retain(|s| {
                    s.container
                        .as_ref()
                        .is_none_or(|scope| container_scope_current(scope, &inventory, complete))
                });
                for session in result.iter_mut().filter(|s| s.container.is_none()) {
                    let tmux_row = session
                        .origin
                        .as_ref()
                        .is_none_or(|_| session.id.starts_with('$'));
                    let Some(target) = foreground_container(session.pid) else {
                        continue;
                    };
                    if let Some(scope) = recovered_container(&target, &inventory) {
                        session.directory = scope.folder.clone();
                        session.container = Some(scope);
                        // Docker's attached client alone does not prove an inner
                        // command is busy; preserve unknown instead of spinning.
                        let now = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs();
                        if tmux_row {
                            session.activity = recent_terminal_output(
                                &field(!session.external, &session.id, "#{window_activity}")
                                    .unwrap_or_default(),
                                now,
                            );
                        }
                    }
                }
            }
        }
    }
    // Labels change as commands and native names change; identity ordering stays stable.
    result.sort_by(|a, b| (a.external, &a.id).cmp(&(b.external, &b.id)));
    Ok(result)
}

/// Restart CX's managed tmux server and its child provider processes.
///
/// The default tmux server and any externally-created sessions are deliberately
/// untouched. Killing the managed server is the safe provider reset boundary:
/// Claude and Codex launched inside its panes are descendants of that server.
pub fn reset() -> Result<serde_json::Value> {
    let sessions = ids(true)?;
    // Adopted terminals are not descendants of the server; ask them to exit
    // before their relays close, rather than leaving them on a dead terminal.
    let root = state()?;
    for id in &sessions {
        let Ok(name) = field(true, id, "#{session_name}") else {
            continue;
        };
        let record: Option<Session> = fs::read(root.join(format!("{name}.json")))
            .ok()
            .and_then(|v| serde_json::from_slice(&v).ok());
        if let Some(origin) = record.and_then(|r| r.origin) {
            if let Ok(index) = adopted_index(&origin) {
                crate::adoption::terminate(&origin.leader, index);
            }
        }
    }
    let mut kill = tmux(true)?;
    kill.arg("kill-server");
    let status = kill
        .status()
        .context("managed tmux reset failed; tmux is unavailable")?;
    if !status.success() && !sessions.is_empty() {
        bail!("managed tmux reset failed")
    }
    ensure_server()?;
    configure_managed()?;
    refresh_provider_environment()?;
    Ok(serde_json::json!({
        "status": "reset",
        "managed_sessions": sessions.len(),
        "provider_processes": "restarted with managed tmux",
        "external_tmux": "untouched"
    }))
}

/// Stop the exact CX-owned runtime shown at confirmation; preserve external sessions.
pub fn stop_shell(id: &str, pid: u32, started: &str, boot_id: &str) -> Result<serde_json::Value> {
    stop_managed(id, pid, started, boot_id, "shell")
}
pub fn stop_agent(
    id: &str,
    pid: u32,
    started: &str,
    boot_id: &str,
    provider: &str,
) -> Result<serde_json::Value> {
    anyhow::ensure!(
        ["codex", "claude"].contains(&provider),
        "unsupported agent stop profile"
    );
    stop_managed(id, pid, started, boot_id, provider)
}
fn stop_managed(
    id: &str,
    pid: u32,
    started: &str,
    boot_id: &str,
    provider: &str,
) -> Result<serde_json::Value> {
    if id.len() != 67
        || !id.starts_with("cx-")
        || !id[3..].bytes().all(|b| b.is_ascii_hexdigit())
        || pid == 0
        || started.is_empty()
        || started.contains("unknown")
    {
        bail!("invalid managed session identity");
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
        &fs::read(root.join(format!("{id}.json")))
            .context("session ownership record unavailable")?,
    )?;
    if record.id != id
        || record.external
        || !["shell", "codex", "claude"].contains(&record.provider.as_str())
        || (provider == "shell" && record.provider != "shell")
        || record.pid != pid
        || record.started != started
        || record.boot_id != boot_id
    {
        bail!("session ownership or runtime identity changed; refresh sessions");
    }
    if let Some(origin) = record.origin.as_ref() {
        return stop_adopted(id, pid, origin);
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
        bail!("session has multiple panes; stop it from its terminal instead");
    }
    let current = inspect(true, &target)?;
    if current.id != id
        || current.pid != pid
        || current.started != started
        || current.boot_id != boot_id
        || current.provider != provider
    {
        bail!("session provider or identity changed; refresh sessions");
    }
    if provider == "shell" {
        reject_provider_descendants(pid)?;
    }
    // Check again immediately before submitting an atomic tmux identity/shape guard.
    let current = inspect(true, &target)?;
    if current.pid != pid || current.started != started || current.provider != provider {
        bail!("session changed while confirming; refresh sessions");
    }
    if provider == "shell" {
        reject_provider_descendants(pid)?;
    }
    if let Some(scope) = &record.container {
        crate::containers::stop_terminal(scope, id)?;
        if !ids(true)?.iter().any(|session| session == &target) {
            return Ok(serde_json::json!({"status":"stopped"}));
        }
    }
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
    let provider_guard = if provider == "shell" {
        provider_guard
    } else {
        "1"
    };
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
        bail!("session stop was not confirmed; refresh sessions before retrying");
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

/// Probe in the same selected login shell as the eventual launcher. Never
/// assume version support or edit provider configuration; older CLIs fall back.
fn codex_supports_no_daemon(shell: &str) -> bool {
    provider_available(
        shell,
        "codex --help 2>/dev/null | command head -c 65536 | command grep -Fq -- '  --no-daemon'",
    )
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
        command timeout 0.2s "$CX_TMUX_BIN" -u -S "$CX_MANAGED_SOCKET" set-option -t "$CX_MANAGED_SESSION" @cx_last_command "$name" >/dev/null 2>&1
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
/// Preserve supported terminal types; older/immutable hosts may lack a viewer's
/// vendor terminfo. Use an installed standard type without changing OS databases.
pub fn attach_terminal() -> Result<Option<String>> {
    let supported = |name: &str| {
        !name.is_empty()
            && name.len() <= 128
            && name.as_bytes()[0].is_ascii_alphanumeric()
            && name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-+._".contains(&b))
            && {
                let mut c = Command::new("infocmp");
                c.arg(name);
                bounded_terminfo(c)
            }
    };
    if std::env::var("TERM")
        .ok()
        .is_some_and(|name| supported(&name))
    {
        return Ok(None);
    }
    ["xterm-256color", "xterm", "screen-256color", "vt100"]
        .into_iter()
        .find(|name| supported(name))
        .map(|name| Some(name.into()))
        .context("Execution device has no standard terminal description; install terminfo")
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
        // Old tmux lacks mouse event coordinates in run-shell format expansion.
        // Preserve native requested mouse input and guarded ordinary scrollback.
        config = config.lines().map(|line| {
            if line.starts_with("bind-key -n WheelUpPane ") {
                "bind-key -n WheelUpPane if-shell -F -t = '#{||:#{mouse_any_flag},#{&&:#{pane_in_mode},#{!=:#{history_size},0}}}' 'send-keys -M' \"if-shell -F -t = '#{!=:#{history_size},0}' 'copy-mode -e -t = ; send-keys -X -t = scroll-up'\"".to_owned()
            } else if line.starts_with("bind-key -n WheelDownPane ") {
                "bind-key -n WheelDownPane if-shell -F -t = '#{||:#{mouse_any_flag},#{&&:#{pane_in_mode},#{!=:#{history_size},0}}}' 'send-keys -M'".to_owned()
            } else if line.contains("WheelUpPane") && line.starts_with("bind-key -T copy-mode") {
                line.split(" if-shell").next().unwrap().to_owned() + " if-shell -F -t = '#{!=:#{history_size},0}' 'send-keys -X -N 5 scroll-up'"
            } else if line.contains("WheelDownPane") && line.starts_with("bind-key -T copy-mode") {
                line.split(" if-shell").next().unwrap().to_owned() + " if-shell -F -t = '#{!=:#{history_size},0}' 'send-keys -X -N 5 scroll-down'"
            } else { line.to_owned() }
        }).collect::<Vec<_>>().join("\n");
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

/// Refresh the allowlisted Anthropic variables in tmux's server environment.
/// Tmux keeps a server-wide snapshot, which otherwise survives shell reloads.
pub fn refresh_provider_environment() -> Result<()> {
    const VARS: &[&str] = &[
        "ANTHROPIC_API_KEY",
        "ANTHROPIC_AUTH_TOKEN",
        "ANTHROPIC_BASE_URL",
        "ANTHROPIC_CUSTOM_HEADERS",
    ];
    for name in VARS {
        let mut command = tmux(true)?;
        command.args(["set-environment", "-g"]);
        match std::env::var(name) {
            Ok(value) => {
                command.arg(*name).arg(value);
            }
            Err(_) => {
                command.args(["-u", *name]);
            }
        };
        command
            .status()
            .with_context(|| format!("refreshing managed tmux variable {name}"))?;
    }
    Ok(())
}
fn ensure_server() -> Result<()> {
    // An owned server may persist after its last session ends. New binaries
    // must reload its configuration too, rather than reusing old key bindings.
    let mut existing = tmux(true)?;
    existing.args(["show-options", "-g"]);
    if existing.output()?.status.success() {
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
            c.args(["show-options", "-g"]);
            if c.output()?.status.success() {
                return configure_managed();
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
    create_with_permissions(request, false, None)
}
pub fn create_yolo(request: &CreateSession) -> Result<Session> {
    create_with_permissions(request, true, None)
}
fn provider_command(provider: &str, yolo: bool, resume: bool) -> Result<String> {
    match (provider, yolo, resume) {
        ("codex", true, true) => {
            Ok("codex resume --last --dangerously-bypass-approvals-and-sandbox".into())
        }
        ("codex", false, true) => Ok("codex resume --last".into()),
        ("claude", true, true) => Ok("claude --continue --dangerously-skip-permissions".into()),
        ("claude", false, true) => Ok("claude --continue".into()),
        ("shell", _, true) => bail!("resume is available only for Codex or Claude sessions"),
        ("codex", true, false) => Ok("codex --dangerously-bypass-approvals-and-sandbox".into()),
        ("claude", true, false) => Ok("claude --dangerously-skip-permissions".into()),
        ("shell", true, false) => bail!("YOLO applies only to Codex or Claude sessions"),
        ("shell" | "codex" | "claude", false, false) => Ok(provider.into()),
        _ => bail!("unsupported launch profile"),
    }
}
pub fn create_container(
    request: &CreateSession,
    scope: &crate::model::ContainerScope,
    yolo: bool,
) -> Result<Session> {
    crate::containers::prepare(scope, &request.provider)?;
    create_with_permissions(request, yolo, Some(scope))
}
fn create_with_permissions(
    request: &CreateSession,
    yolo: bool,
    container: Option<&crate::model::ContainerScope>,
) -> Result<Session> {
    let mut agent_command = provider_command(&request.provider, yolo, request.resume)?;
    if request.key.is_empty() || request.key.len() > 1024 {
        bail!("creation key must be 1–1024 bytes");
    }
    if !["shell", "claude", "codex"].contains(&request.provider.as_str()) {
        bail!("unsupported launch profile");
    }
    let execution_directory = if container.is_some() {
        std::env::var("HOME").unwrap_or_else(|_| "/".into())
    } else {
        request.directory.clone()
    };
    let directory = fs::canonicalize(crate::files::decode_path(&execution_directory)?)
        .context("execution directory unavailable")?;
    if !directory.is_dir() {
        bail!("execution location is not a directory");
    }
    let display_name = if request.name.is_empty() {
        if let Some(scope) = container {
            format!("{} · {}", scope.name, request.provider)
        } else {
            default_session_name(&request.provider, &directory.to_string_lossy())
        }
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
    let permissions_file = root.join(format!("{name}.permissions"));
    if exists.output()?.status.success() {
        let previous = match fs::read_to_string(&permissions_file) {
            Ok(value) => value,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => "default".into(),
            Err(error) => return Err(error).context("session permissions unavailable"),
        };
        if previous != if yolo { "yolo" } else { "default" } {
            bail!("creation key already belongs to a different permission mode; use a new key");
        }
        let current = inspect(true, &name)?;
        anyhow::ensure!(
            current.container.as_ref() == container,
            "creation key already belongs to a different execution scope; use a new key"
        );
        return Ok(current);
    }
    if root.join(format!("{name}.json")).exists() {
        bail!("original session has ended; use a new creation key to start replacement work");
    }
    let shell = launch_shell()?;
    if container.is_none()
        && request.provider != "shell"
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
    if container.is_none() && request.provider == "codex" && codex_supports_no_daemon(&shell) {
        agent_command.push_str(" --no-daemon --config 'tui.terminal_title=[\"thread-name\"]'");
    }
    ensure_server()?;
    refresh_provider_environment()?;
    let (host, account, boot_id) = identity();
    let pending = Session {
        id: name.clone(),
        name: display_name.clone(),
        directory: container
            .map(|s| s.folder.clone())
            .unwrap_or_else(|| directory.to_string_lossy().into_owned()),
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
        activity: None,
        agent_tasks: vec![],
        origin: None,
        container: container.cloned(),
    };
    // Persist intent before starting: an interrupted helper response must never lose the launcher identity.
    let mut intent = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(root.join(format!("{name}.json")))?;
    intent.write_all(&serde_json::to_vec(&pending)?)?;
    intent.sync_all()?;
    let mut permissions = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(permissions_file)?;
    permissions.write_all(if yolo { b"yolo" } else { b"default" })?;
    permissions.sync_all()?;
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
        launcher.extend(["-i".into(), "-c".into(), agent_command]);
    }
    if let Some(scope) = container {
        use base64::Engine;
        let payload = base64::engine::general_purpose::STANDARD.encode(serde_json::to_vec(&(
            scope,
            &request.provider,
            yolo,
        ))?);
        launcher = vec![
            std::env::current_exe()?.to_str().context("cx path")?.into(),
            "container-run".into(),
            payload,
        ];
    }
    let title_tracking = container.is_none()
        && request.provider == "codex"
        && launcher
            .iter()
            .any(|arg| arg.contains("tui.terminal_title="));
    let command = format!(
        "{}exec env {}CX_VIEWER_THEME=1 CX_TMUX_BIN={} CX_MANAGED_SOCKET={} CX_MANAGED_SESSION={} {}",
        if title_tracking { "printf '\\033]0;CX awaiting native title\\007'; " } else { "" },
        if title_tracking { "CX_CODEX_TITLE=thread-name-v1 " } else { "" },
        quote(
            tmux_executable()
                .to_str()
                .context("tmux path is not UTF-8")?
        ),
        quote(socket()?.to_str().context("socket path is not UTF-8")?),
        quote(&name),
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
    let mut session = inspect(true, &name).context("terminal unavailable during startup")?;
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
    set_managed_status(&session).context("terminal unavailable during startup")?;
    wait_for_first_screen(&name).context("terminal unavailable during startup")?;
    Ok(session)
}

/// Creation already runs on the helper worker. Keep the viewer visible while
/// the detached terminal initializes, without sending input or guessing prompts.
/// Silent programs must remain attachable, so this is a bounded presentation wait.
fn wait_for_first_screen(id: &str) -> Result<()> {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        if field(true, id, "#{pane_dead}")? == "1" {
            bail!("terminal exited during startup; inspect the session before retrying");
        }
        let mut capture = tmux(true)?;
        // Only inspect the first 40 visible rows, never history. The text is
        // discarded immediately and never stored, logged or used as metadata.
        capture.args(["capture-pane", "-p", "-t", id, "-S", "0", "-E", "39"]);
        let visible = output(capture)?;
        if visible
            .chars()
            .any(|c| !c.is_whitespace() && !c.is_control())
            || std::time::Instant::now() >= deadline
        {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(80));
    }
}

fn restore_managed_style(session: &Session) -> Result<()> {
    // Match the private server's terminal defaults, including transparency.
    // Also clear opaque per-window overrides left by older CX versions.
    for option in ["window-style", "window-active-style"] {
        let mut query = tmux(true)?;
        query.args(["show-options", "-g", "-v", option]);
        let style = output(query)?;
        let mut command = tmux(true)?;
        command.args(["set-option", "-w", "-t", &session.id, option, style.trim()]);
        output(command)?;
    }
    Ok(())
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
// Codex can spawn an editor in its own foreground process group. A provider
// ancestor is not enough: another process on this PTY must retain input.
// Conservatively reject same-group children even with redirected stdin; they
// may open /dev/tty independently.
pub(crate) fn wheel_codex_owner(pane: u32) -> Option<ProcessIdentity> {
    let (provider, identity) = foreground_provider(pane)?;
    if provider != "codex" {
        return None;
    }
    let (_, terminal, foreground) = terminal_group(pane)?;
    let mut queue = std::collections::VecDeque::from([(pane, 0)]);
    let mut parents = std::collections::HashMap::new();
    let mut readers = Vec::new();
    let mut visited = 0;
    let mut seen = std::collections::HashSet::new();
    let mut tasks = 0;
    while let Some((pid, depth)) = queue.pop_front() {
        if !seen.insert(pid) {
            continue;
        }
        visited += 1;
        if visited > 64 || depth > 6 {
            return None;
        }
        if pid != pane
            && terminal_group(pid)
                .is_some_and(|(group, tty, _)| group == foreground && tty == terminal)
            && running_provider(pid) != Some("codex")
        {
            readers.push(pid);
        }
        // /task/PID/children lists only leader-spawned children. Editors may
        // be created by a worker thread, so inspect all bounded task lists.
        use std::io::Read;
        for task in fs::read_dir(format!("/proc/{pid}/task")).ok()? {
            tasks += 1;
            if tasks > 256 {
                return None;
            }
            let mut children = String::new();
            fs::File::open(task.ok()?.path().join("children"))
                .ok()?
                .take(8193)
                .read_to_string(&mut children)
                .ok()?;
            if children.len() > 8192 {
                return None;
            }
            for child in children.split_whitespace() {
                let child = child.parse().ok()?;
                parents.insert(child, pid);
                queue.push_back((child, depth + 1));
                if queue.len() > 64 {
                    return None;
                }
            }
        }
    }
    // Fish/Node launchers waiting above the native provider do not own input.
    let mut ancestors = std::collections::HashSet::new();
    let mut pid = identity.pid;
    while let Some(parent) = parents.get(&pid) {
        ancestors.insert(*parent);
        pid = *parent;
    }
    if readers.iter().any(|pid| !ancestors.contains(pid)) {
        return None;
    }
    Some(identity)
}

/// Wheel fallback belongs only to our managed terminal. Never synthesize prompt
/// history arrows or send mouse input to an unrelated foreground editor/shell.
pub fn native_wheel(pane: &str, direction: &str, x: u16, y: u16, client: u32) -> Result<()> {
    if !pane.starts_with('%')
        || pane.len() > 12
        || !pane[1..].bytes().all(|c| c.is_ascii_digit())
        || pane.len() == 1
        || !matches!(direction, "up" | "down")
    {
        bail!("invalid managed wheel event");
    }
    let mut command = tmux(true)?;
    command.args(["display-message", "-p", "-t", pane,
        "#{session_name} #{pane_pid} #{alternate_on} #{history_size} #{pane_in_mode} #{pane_left} #{pane_top} #{pane_width} #{pane_height} #{mouse_any_flag}"]);
    let snapshot = output(command)?;
    let fields: Vec<_> = snapshot.split_whitespace().collect();
    if fields.len() != 10
        || fields[0].len() != 67
        || !fields[0].starts_with("cx-")
        || !fields[0][3..].bytes().all(|c| c.is_ascii_hexdigit())
    {
        return Ok(());
    }
    // A fallback tmux subprocess must not bypass the originating viewer's
    // read-only/no-resize attachment policy. Detached/stale events are discarded.
    let mut clients = tmux(true)?;
    clients.args([
        "list-clients",
        "-t",
        fields[0],
        "-F",
        "#{client_pid} #{client_readonly}",
    ]);
    if !output(clients)?
        .lines()
        .any(|line| line == format!("{client} 0"))
    {
        return Ok(());
    }
    let pid: u32 = fields[1].parse()?;
    let mode = fields[4] == "1";
    let history: u32 = fields[3].parse()?;
    let container_record = fs::read(state()?.join(format!("{}.json", fields[0])))
        .ok()
        .and_then(|b| serde_json::from_slice::<Session>(&b).ok())
        .filter(|s| !s.external && s.pid == pid && s.id == fields[0]);
    let owner = || {
        wheel_codex_owner(pid).or_else(|| {
            container_record
                .as_ref()
                .and_then(|s| s.container.as_ref())
                .and_then(|scope| crate::containers::wheel_owner(scope, fields[0]))
        })
    };
    let provider = owner();
    if fields[2] == "1" && fields[9] == "0" && provider.is_some() {
        let left: u16 = fields[5].parse()?;
        let top: u16 = fields[6].parse()?;
        let width: u16 = fields[7].parse()?;
        let height: u16 = fields[8].parse()?;
        let (Some(column), Some(row)) = (x.checked_sub(left), y.checked_sub(top)) else {
            return Ok(());
        };
        if column >= width || row >= height {
            return Ok(());
        }
        // Recheck the foreground group and start identity immediately before
        // dispatch. A provider launched inside a shell is still recognized, but
        // suspended Codex and its foreground editors receive no fabricated input.
        if owner().map(|p| (p.pid, p.start_ticks)) != provider.map(|p| (p.pid, p.start_ticks)) {
            return Ok(());
        }
        if mode {
            if history > 0 {
                return Ok(());
            }
            let mut cancel = tmux(true)?;
            cancel.args(["send-keys", "-t", pane, "-X", "cancel"]);
            output(cancel)?;
        }
        let event = format!(
            "\x1b[<{};{};{}M",
            if direction == "up" { 64 } else { 65 },
            column + 1,
            row + 1
        );
        let mut send = tmux(true)?;
        send.args(["send-keys", "-t", pane, "-l", &event]);
        output(send)?;
    } else if direction == "up" && history > 0 && !mode {
        let mut copy = tmux(true)?;
        copy.args([
            "copy-mode",
            "-e",
            "-t",
            pane,
            ";",
            "send-keys",
            "-t",
            pane,
            "-X",
            "scroll-up",
        ]);
        output(copy)?;
    }
    Ok(())
}

pub fn refresh_managed_status(id: &str) -> Result<()> {
    let id = id.strip_prefix('=').unwrap_or(id);
    if !id.starts_with("cx-") || id.len() != 67 || !id[3..].bytes().all(|b| b.is_ascii_hexdigit()) {
        bail!("invalid managed session identity");
    }
    let session = inspect(true, id)?;
    restore_managed_style(&session)?;
    set_managed_status(&session)
}

pub fn quote(s: &str) -> String {
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
    } else if !(session.id.starts_with("cx-")
        && session.id.len() == 67
        && session.id[3..].bytes().all(|b| b.is_ascii_hexdigit()))
        && !valid_pane_name(&session.id)
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
        if let Some(terminal) = attach_terminal()? {
            c.env("TERM", terminal);
        }
        c.args(args);
        c
    };
    let result = c.status().context("native terminal attachment failed")?;
    if !result.success() {
        bail!("native terminal attachment ended with {result}");
    }
    Ok(())
}

/// Open an ordinary login terminal without requiring tmux or the CX helper.
/// Exiting the shell restores the viewer through the existing native handoff.
/// Transient managed pane names for command panels; never recorded or listed.
fn command_pane_name() -> Result<String> {
    use std::io::Read;
    let mut bytes = [0u8; 16];
    fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(format!(
        "cxcmd-{}",
        bytes.iter().map(|b| format!("{b:02x}")).collect::<String>()
    ))
}
fn valid_pane_name(id: &str) -> bool {
    id.len() == 38 && id.starts_with("cxcmd-") && id[6..].bytes().all(|b| b.is_ascii_hexdigit())
}
fn clamp_size(cols: u16, rows: u16) -> (u16, u16) {
    (cols.clamp(20, 500), rows.clamp(4, 300))
}

/// Start a command in a managed pane kept alive after exit so its final output
/// and status remain capturable. The pane is transient: no ownership record,
/// hidden from the session list, and closed when its panel closes.
pub fn command_panel(request: &RunCommand, cols: u16, rows: u16) -> Result<serde_json::Value> {
    validate_command(request)?;
    let directory = fs::canonicalize(crate::files::decode_path(&request.directory)?)
        .context("command directory is unavailable")?;
    if !directory.is_dir() {
        bail!("command directory is not a folder");
    }
    let (cols, rows) = clamp_size(cols, rows);
    ensure_server()?;
    let shell = launch_shell()?;
    let fish = std::path::Path::new(&shell)
        .file_name()
        .is_some_and(|s| s == "fish");
    // The command runs under a login shell on the pane's own tty, so colors and
    // interactive prompts work. Output and status persist via remain-on-exit.
    let invocation = if fish {
        // fish -c has no $0 slot, so the command is argv[1] directly.
        format!(
            "exec {} -l -c {} {}",
            quote(&shell),
            quote("eval $argv[1]"),
            quote(&request.command)
        )
    } else {
        format!(
            "exec {} -l -c {} cx-command {}",
            quote(&shell),
            quote(r#"eval "$1""#),
            quote(&request.command)
        )
    };
    let name = command_pane_name()?;
    let mut c = tmux(true)?;
    c.args(["new-session", "-d", "-s", &name])
        .arg("-x")
        .arg(cols.to_string())
        .arg("-y")
        .arg(rows.to_string())
        .args(["-n", "work", "-c"])
        .arg(&directory)
        .arg(&invocation);
    output(c)?;
    // Keep the dead pane and its status; scope the option to this session.
    let mut opt = tmux(true)?;
    opt.args(["set-option", "-t", &name, "remain-on-exit", "on"]);
    let _ = output(opt);
    let pid = field(true, &name, "#{pane_pid}")?.parse().unwrap_or(0);
    Ok(serde_json::json!({
        "id": name,
        "socket": socket()?.to_string_lossy(),
        "pid": pid,
        "cols": cols,
        "rows": rows,
    }))
}

/// Capture the styled screen, liveness and exit status of a command pane.
pub fn capture_pane(id: &str, cols: u16, rows: u16) -> Result<serde_json::Value> {
    anyhow::ensure!(valid_pane_name(id), "invalid command pane");
    let (cols, rows) = clamp_size(cols, rows);
    // Match the pane to the panel so wrapping and cursor line up.
    let mut resize = tmux(true)?;
    resize
        .args(["resize-window", "-t", id, "-x"])
        .arg(cols.to_string())
        .arg("-y")
        .arg(rows.to_string());
    let _ = output(resize);
    let dead = field(true, id, "#{pane_dead}").map_or(true, |d| d != "0");
    // tmux can report pane_dead before pane_dead_status is populated. When the
    // pane has just died, wait briefly for the exit code so the panel shows it
    // instead of settling on "finished".
    let mut status = None;
    if dead {
        for attempt in 0..15 {
            if let Some(code) = field(true, id, "#{pane_dead_status}")
                .ok()
                .filter(|s| !s.is_empty())
                .and_then(|s| s.parse::<i32>().ok())
            {
                status = Some(code);
                break;
            }
            if attempt < 14 {
                thread::sleep(Duration::from_millis(20));
            }
        }
    }
    let mut capture = tmux(true)?;
    capture.args(["capture-pane", "-e", "-p", "-t", id]);
    let screen = output(capture).unwrap_or_default();
    Ok(serde_json::json!({
        "id": id,
        "lines": screen.split('\n').collect::<Vec<_>>(),
        "alive": !dead,
        "status": status,
        "cols": cols,
        "rows": rows,
    }))
}

/// Deliver hex-encoded bytes to a running command pane.
pub fn pane_input(id: &str, hex: &str) -> Result<serde_json::Value> {
    anyhow::ensure!(valid_pane_name(id), "invalid command pane");
    anyhow::ensure!(
        !hex.is_empty()
            && hex.len() <= 8192
            && hex.len() % 2 == 0
            && hex.bytes().all(|b| b.is_ascii_hexdigit()),
        "invalid key data"
    );
    if field(true, id, "#{pane_dead}").map_or(true, |d| d != "0") {
        return Ok(serde_json::json!({"delivered": false}));
    }
    let mut c = tmux(true)?;
    c.args(["send-keys", "-t", id, "-H"]);
    for pair in hex.as_bytes().chunks(2) {
        c.arg(std::str::from_utf8(pair)?);
    }
    output(c)?;
    Ok(serde_json::json!({"delivered": true}))
}

/// Close a command pane, ending the command if it is still running.
pub fn pane_close(id: &str) -> Result<serde_json::Value> {
    anyhow::ensure!(valid_pane_name(id), "invalid command pane");
    let mut c = tmux(true)?;
    c.args(["kill-session", "-t", id]);
    let _ = c.output();
    Ok(serde_json::json!({"closed": true}))
}

pub fn login_terminal(device: &Device) -> Result<()> {
    let mut command = if let Some(target) = &device.target {
        let routed = crate::store::ssh(target, true)?;
        let mut command = Command::new("ssh");
        command.arg("-tt").args(routed.get_args());
        command
    } else {
        let mut command = Command::new(launch_shell()?);
        command.arg("-l");
        command
    };
    let status = foreground_command(&mut command)?;
    if !status.success() {
        bail!("Login terminal ended with {status}; connection was not retried");
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

    #[test]
    fn terminal_output_indicator_expires_and_rejects_future_or_invalid_time() {
        assert_eq!(
            recent_terminal_output("100", 102).as_deref(),
            Some("output")
        );
        assert_eq!(recent_terminal_output("100", 103), None);
        assert_eq!(recent_terminal_output("104", 103), None);
        assert_eq!(recent_terminal_output("0", 1), None);
        assert_eq!(recent_terminal_output("unknown", 103), None);
    }
    #[test]
    fn saved_container_scope_reconciles_explicit_evidence_and_complete_absence() {
        let c: crate::containers::Container = serde_json::from_value(serde_json::json!({
            "engine":"fixture", "id":"a".repeat(64), "name":"workspace", "image":"fixture", "state":"running",
            "started_at":"one", "devcontainer":true,"evidence":"label", "user":"dev", "folder":"/workspace", "workspace":null,
            "config":null,"network":"bridge", "networks":[],"ports":{},"allowed":true
        })).unwrap();
        let scope = c.scope();
        assert!(container_scope_current(&scope, &[c.clone()], true));
        assert!(
            container_scope_current(&scope, &[], false),
            "partial absence must preserve uncertainty"
        );
        assert!(
            !container_scope_current(&scope, &[], true),
            "complete absence must prune"
        );
        for change in ["stopped", "restart", "engine", "user", "access"] {
            let mut changed = c.clone();
            match change {
                "stopped" => changed.state = "exited".into(),
                "restart" => changed.started_at = "two".into(),
                "engine" => changed.engine = "different".into(),
                "user" => changed.user = "other".into(),
                _ => {
                    changed.devcontainer = false;
                    changed.allowed = false;
                }
            }
            assert!(
                !container_scope_current(&scope, &[changed], false),
                "explicit {change} evidence must prune even partial discovery"
            );
        }
    }

    #[test]
    fn docker_exec_recovery_reads_only_exact_invocation_prefix() {
        let parse = |args: &[&str]| {
            docker_exec_target(
                &args
                    .iter()
                    .map(|s| s.as_bytes().to_vec())
                    .collect::<Vec<_>>(),
            )
        };
        assert_eq!(
            parse(&[
                "docker",
                "exec",
                "-it",
                "--user",
                "dev",
                "--workdir=/projects/cx",
                "workspace",
                "bash",
                "secret prompt"
            ]),
            Some("workspace".into())
        );
        assert_eq!(
            parse(&["docker", "exec", "--", "workspace", "sh"]),
            Some("workspace".into())
        );
        for args in [
            vec!["docker", "--context", "other", "exec", "workspace", "sh"],
            vec!["docker", "exec", "--future-option", "workspace", "sh"],
            vec!["docker", "exec", "-d", "workspace", "sh"],
            vec!["docker", "exec", "--user"],
            vec!["docker", "exec", "evil;command", "sh"],
        ] {
            assert_eq!(parse(&args), None);
        }
    }
    #[test]
    fn activity_requires_known_native_status() {
        for status in ["working", "busy", "running"] {
            assert_eq!(normalized_activity(Some(status)), Some("running"));
        }
        assert_eq!(
            normalized_activity(Some("waiting_for_input")),
            Some("waiting")
        );
        assert_eq!(normalized_activity(Some("idle")), Some("idle"));
        assert_eq!(normalized_activity(None), None);
        assert_eq!(normalized_activity(Some("unrecognized")), None);
    }
    #[test]
    fn recovered_container_requires_running_allowed_exact_unique_identity() {
        let id = "a".repeat(64);
        let c: crate::containers::Container = serde_json::from_value(serde_json::json!({
            "engine":"fixture", "id":id, "name":"workspace", "image":"fixture", "state":"running",
            "started_at":"one", "devcontainer":true,"evidence":"label", "user":"dev", "folder":"/workspace", "workspace":null,
            "config":null,"network":"bridge", "networks":[],"ports":{},"allowed":true
        })).unwrap();
        assert!(recovered_container("workspace", &[c.clone()]).is_some());
        assert!(recovered_container(&id, &[c.clone()]).is_some());
        assert!(recovered_container(&id[..12], &[c.clone()]).is_none());
        assert!(recovered_container("workspace", &[c.clone(), c.clone()]).is_none());
        let mut stopped = c.clone();
        stopped.state = "exited".into();
        assert!(recovered_container("workspace", &[stopped]).is_none());
        let mut service = c;
        service.devcontainer = false;
        service.allowed = false;
        assert!(recovered_container("workspace", &[service]).is_none());
    }

    #[test]
    fn provider_commands_distinguish_fresh_and_native_resume() {
        assert_eq!(provider_command("codex", false, false).unwrap(), "codex");
        assert_eq!(
            provider_command("codex", false, true).unwrap(),
            "codex resume --last"
        );
        assert_eq!(
            provider_command("codex", true, true).unwrap(),
            "codex resume --last --dangerously-bypass-approvals-and-sandbox"
        );
        assert_eq!(provider_command("claude", false, false).unwrap(), "claude");
        assert_eq!(
            provider_command("claude", false, true).unwrap(),
            "claude --continue"
        );
        assert_eq!(
            provider_command("claude", true, true).unwrap(),
            "claude --continue --dangerously-skip-permissions"
        );
        assert!(provider_command("shell", false, true).is_err());
    }
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
    fn unknown_provider_directory_is_not_a_session_title() {
        let fallback = "codex · old-project".to_owned();
        let named = crate::session_titles::NativeMetadata {
            title: Some("Current task".into()),
            ..Default::default()
        };
        assert_eq!(
            automatic_provider_name("codex", Some(&named), true, fallback.clone()),
            "Current task"
        );
        let unselected = crate::session_titles::NativeMetadata {
            current_thread_unknown: true,
            ..named
        };
        assert_eq!(
            automatic_provider_name("codex", Some(&unselected), true, fallback.clone()),
            "codex"
        );
        assert_eq!(
            automatic_provider_name("codex", None, true, fallback.clone()),
            "codex"
        );
        assert_eq!(
            automatic_provider_name("codex", None, false, fallback.clone()),
            fallback
        );
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
            activity: None,
            agent_tasks: vec![],
            origin: None,
            container: None,
        };
        assert_eq!(
            current_session_name(&session, "codex", "/projects/cx"),
            "codex · cx"
        );
        assert_eq!(
            current_session_name(&session, "shell", "/projects/new"),
            "New shell"
        );
        for blank in ["", "  \t "] {
            session.name = blank.into();
            assert_eq!(
                current_session_name(&session, "codex", "/projects/cx"),
                "codex · cx"
            );
        }
        session.name = "My important work".into();
        assert_eq!(
            current_session_name(&session, "codex", "/projects/cx"),
            "My important work"
        );
        assert_eq!(default_session_name("shell", "/"), "New shell");
        session.name = "shell-123456-654321".into();
        session.id = format!("cx-{:x}", Sha256::digest(b"123456-654321"));
        assert_eq!(
            current_session_name(&session, "shell", "/projects/cx"),
            "New shell"
        );
        session.id = "unrelated-id".into();
        assert_eq!(
            current_session_name(&session, "shell", "/projects/cx"),
            "shell-123456-654321"
        );
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
