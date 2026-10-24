use crate::model::{CreateSession, Device, ProcessIdentity, RunCommand, Session, TerminalIdentity};
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
    Ok(snapshot(managed)?.into_iter().map(|s| s.id).collect())
}

// Only tmux-generated numeric fields enter this framing. Names and paths may
// contain tabs/newlines and must remain separate reads, never inventory records.
const SNAPSHOT_FORMAT: &str = "#{session_id}\t#{pane_pid}\t#{session_created}\t#{pane_id}\t#{pane_dead}\t#{pid}\t#{session_windows}\t#{window_panes}";
#[derive(Debug)]
struct SessionSnapshot {
    id: String,
    pane_pid: u32,
    created: String,
    pane_id: String,
    dead: bool,
    server_pid: u32,
    server_start: Option<String>,
    windows: u32,
    panes: u32,
}
impl SessionSnapshot {
    fn ended(&self) -> bool {
        // An expanded session may have live work outside its selected dead pane.
        self.dead && self.windows == 1 && self.panes == 1
    }
}
fn parse_snapshot(raw: &str) -> Result<Vec<SessionSnapshot>> {
    let number = |s: &str| -> Result<u32> {
        anyhow::ensure!(
            !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()),
            "invalid tmux numeric metadata"
        );
        Ok(s.parse()?)
    };
    raw.lines()
        .map(|line| {
            let columns: Vec<_> = line.split('\t').collect();
            anyhow::ensure!(columns.len() == 8, "invalid tmux inventory metadata");
            let id = columns[0]
                .strip_prefix('$')
                .context("invalid tmux session identity")?;
            number(id)?;
            let pane = columns[3]
                .strip_prefix('%')
                .context("invalid tmux pane identity")?;
            number(pane)?;
            anyhow::ensure!(
                !columns[2].is_empty() && columns[2].bytes().all(|b| b.is_ascii_digit()),
                "invalid tmux creation time"
            );
            let dead = number(columns[4])?;
            let server_pid = number(columns[5])?;
            let windows = number(columns[6])?;
            let panes = number(columns[7])?;
            anyhow::ensure!(
                dead <= 1 && server_pid > 0 && windows > 0 && panes > 0,
                "invalid tmux runtime metadata"
            );
            Ok(SessionSnapshot {
                id: columns[0].into(),
                // tmux clears pane_pid after an exited pane is retained.
                pane_pid: if dead == 1 && columns[1].is_empty() {
                    0
                } else {
                    number(columns[1])?
                },
                created: columns[2].into(),
                pane_id: columns[3].into(),
                dead: dead == 1,
                server_pid,
                server_start: None,
                windows,
                panes,
            })
        })
        .collect()
}
fn snapshot(managed: bool) -> Result<Vec<SessionSnapshot>> {
    let mut c = tmux(managed)?;
    c.args(["list-sessions", "-F", SNAPSHOT_FORMAT]);
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
    let mut rows = parse_snapshot(
        std::str::from_utf8(&out.stdout).context("invalid tmux inventory encoding")?,
    )?;
    let mut generations = std::collections::BTreeMap::new();
    for row in &mut rows {
        row.server_start = generations
            .entry(row.server_pid)
            .or_insert_with(|| process_start_ticks(row.server_pid))
            .clone();
    }
    Ok(rows)
}
fn process_start_ticks(pid: u32) -> Option<String> {
    use std::io::Read;
    let mut bytes = Vec::new();
    fs::File::open(format!("/proc/{pid}/stat"))
        .ok()?
        .take(4097)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() > 4096 {
        return None;
    }
    parse_process_start_ticks(pid, std::str::from_utf8(&bytes).ok()?)
}
fn parse_process_start_ticks(pid: u32, stat: &str) -> Option<String> {
    let (prefix, rest) = stat.rsplit_once(')')?;
    if prefix.split_once(" (")?.0.parse::<u32>().ok()? != pid {
        return None;
    }
    let fields: Vec<_> = rest.split_whitespace().take(20).collect();
    if matches!(*fields.first()?, "Z" | "X" | "x") {
        return None;
    }
    let ticks = *fields.get(19)?;
    if ticks.is_empty() || !ticks.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some(ticks.into())
}
fn terminal_identity(
    observed: &SessionSnapshot,
    current_start: Option<String>,
) -> Result<Option<TerminalIdentity>> {
    let Some(start) = &observed.server_start else {
        return Ok(None);
    };
    anyhow::ensure!(
        current_start.as_ref() == Some(start),
        "tmux server changed during observation"
    );
    Ok(Some(TerminalIdentity {
        server_pid: observed.server_pid,
        server_start: start.clone(),
        session_id: observed.id.clone(),
        created: observed.created.clone(),
    }))
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
    // These optional command labels are accepted only as three strictly bounded
    // executable names. Malformed metadata loses its label, not its session.
    let labels = field(
        managed,
        id,
        "#{pane_current_command}\t#{@cx_last_command}\t#{@cx_recent_command}",
    )
    .ok()?;
    let mut labels = labels.split('\t');
    let current = shell_command_label(labels.next()?);
    let last = shell_command_label(labels.next()?);
    let recent = shell_command_label(labels.next()?);
    if labels.next().is_some() {
        return None;
    }
    if let Some(name) = current {
        if managed && recent.as_ref() != Some(&name) {
            let mut c = tmux(true).ok()?;
            c.args(["set-option", "-t", id, "@cx_recent_command", &name]);
            // Best effort; this metadata cannot prevent listing a running terminal.
            let _ = output(c);
        }
        return Some(format!("running {name}"));
    }
    if managed {
        if let Some(name) = last {
            return Some(format!("last: {name}"));
        }
        return recent.map(|name| format!("recent: {name}"));
    }
    None
}

fn inspect(managed: bool, id: &str) -> Result<Session> {
    let mut rows = parse_snapshot(&field(managed, id, SNAPSHOT_FORMAT)?)?;
    anyhow::ensure!(rows.len() == 1, "session metadata unavailable");
    let mut observed = rows.remove(0);
    observed.server_start = process_start_ticks(observed.server_pid);
    inspect_observed(managed, &observed, &identity())
}
fn inspect_observed(
    managed: bool,
    observed: &SessionSnapshot,
    host_identity: &(String, String, String),
) -> Result<Session> {
    let (host, account, boot_id) = host_identity.clone();
    // Pin metadata reads to the observed pane; selection can change during refresh.
    let target = observed.pane_id.as_str();
    let name = field(managed, target, "#{session_name}")?;
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
    let pid = observed.pane_pid;
    let process_start = fs::read_to_string(format!("/proc/{pid}/stat"))
        .ok()
        .and_then(|v| {
            v.rsplit_once(')')
                .and_then(|(_, rest)| rest.split_whitespace().nth(19).map(str::to_owned))
        })
        .unwrap_or_else(|| "unknown".into());
    let started = format!("{}:{process_start}", observed.created);
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
        .map(Ok)
        .unwrap_or_else(|| field(managed, target, "#{pane_current_path}"))?;
    let saved_name = prior
        .as_ref()
        .map(|s| current_session_name(s, provider, &directory))
        .unwrap_or_else(|| name.clone());
    let automatic = prior.as_ref().is_some_and(automatically_named);
    let display_name = if automatic && provider == "shell" {
        shell_activity(managed, target).unwrap_or(saved_name)
    } else if automatic {
        process
            .as_ref()
            .and_then(|p| crate::session_titles::native_title(provider, p.pid))
            .unwrap_or(saved_name)
    } else {
        saved_name
    };
    {
        let expected = observed;
        // A pane can move between sessions while enrichment is in flight. Never
        // return its new metadata under the old attachment/ownership identity.
        let current = parse_snapshot(&field(managed, target, SNAPSHOT_FORMAT)?)?;
        anyhow::ensure!(
            current.len() == 1
                && current[0].id == expected.id
                && current[0].pane_pid == expected.pane_pid
                && current[0].server_pid == expected.server_pid
                && current[0].created == expected.created
                && !current[0].ended(),
            "session changed during observation"
        );
    }
    let terminal = terminal_identity(observed, process_start_ticks(observed.server_pid))?;
    Ok(Session {
        terminal,
        id: if managed { name } else { observed.id.clone() },
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
        container: prior.as_ref().and_then(|s| s.container.clone()),
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
    let host_identity = identity();
    let mut seen = std::collections::BTreeSet::new();
    for managed in [true, false] {
        for observed in snapshot(managed)? {
            let runtime = (
                observed.server_pid,
                observed.server_start.clone(),
                observed.id.clone(),
                observed.created.clone(),
            );
            if observed.ended() || seen.contains(&runtime) {
                continue;
            }
            if let Ok(s) = inspect_observed(managed, &observed, &host_identity) {
                seen.insert(runtime);
                result.push(s);
            }
        }
    }
    // Labels change as commands and native names change; identity ordering stays stable.
    result.sort_by(|a, b| (a.external, &a.id).cmp(&(b.external, &b.id)));
    Ok(result)
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
    let agent_command = provider_command(&request.provider, yolo, request.resume)?;
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
    ensure_server()?;
    let (host, account, boot_id) = identity();
    let pending = Session {
        terminal: None,
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
    let command = format!(
        "exec env CX_VIEWER_THEME=1 CX_TMUX_BIN={} CX_MANAGED_SOCKET={} CX_MANAGED_SESSION={} {}",
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
    set_managed_status(&inspect(true, id)?)
}

fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}
const ATTACH_PAYLOAD_LIMIT: usize = 32768;
const ATTACH_REFUSED: &str = "Session changed or ended; refresh before attaching";
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct CheckedAttach {
    session: Session,
    observe: bool,
    #[serde(default)]
    guard: Option<String>,
}
fn attach_payload(request: &CheckedAttach) -> Result<String> {
    use base64::Engine;
    let payload = base64::engine::general_purpose::STANDARD.encode(serde_json::to_vec(request)?);
    anyhow::ensure!(
        payload.len() <= ATTACH_PAYLOAD_LIMIT,
        "attachment metadata exceeds limit"
    );
    Ok(payload)
}
fn parse_attach_payload(payload: &str) -> Result<CheckedAttach> {
    use base64::Engine;
    anyhow::ensure!(
        payload.len() <= ATTACH_PAYLOAD_LIMIT,
        "attachment metadata exceeds limit"
    );
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(payload)
        .context("invalid attachment metadata")?;
    let request: CheckedAttach =
        serde_json::from_slice(&bytes).context("invalid attachment metadata")?;
    let terminal = request
        .session
        .terminal
        .as_ref()
        .context("terminal identity unavailable")?;
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    anyhow::ensure!(
        terminal.server_pid > 0
            && digits(&terminal.server_start)
            && digits(&terminal.created)
            && terminal.session_id.strip_prefix('$').is_some_and(digits)
            && !request.session.boot_id.is_empty(),
        "invalid terminal identity"
    );
    if request.session.external {
        anyhow::ensure!(
            request.session.id == terminal.session_id,
            "invalid external terminal identity"
        );
    } else {
        anyhow::ensure!(
            request.session.id.len() == 67
                && request.session.id.starts_with("cx-")
                && request.session.id[3..]
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit()),
            "invalid managed terminal identity"
        );
    }
    if let Some(token) = &request.guard {
        anyhow::ensure!(valid_attach_guard(token), "invalid attachment guard");
    }
    Ok(request)
}
fn valid_attach_guard(token: &str) -> bool {
    token.len() == 64 && token.bytes().all(|b| b.is_ascii_hexdigit())
}
fn attach_guard_path(token: &str) -> Result<PathBuf> {
    anyhow::ensure!(valid_attach_guard(token), "invalid attachment guard");
    Ok(state()?.join(format!("attach-guard-{token}")))
}
struct AttachGuard {
    token: String,
    path: PathBuf,
    file: fs::File,
}
impl AttachGuard {
    fn new() -> Result<Self> {
        use std::io::Read;
        let mut random = [0u8; 32];
        fs::File::open("/dev/urandom")?.read_exact(&mut random)?;
        let token: String = random.iter().map(|v| format!("{v:02x}")).collect();
        let path = attach_guard_path(&token)?;
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&path)?;
        Ok(Self { token, path, file })
    }
    fn passed(&mut self) -> Result<bool> {
        use std::io::{Read, Seek, SeekFrom};
        validate_attach_guard_file(&self.file)?;
        self.file.seek(SeekFrom::Start(0))?;
        let mut bytes = Vec::new();
        (&mut self.file).take(3).read_to_end(&mut bytes)?;
        Ok(bytes == b"ok")
    }
}
impl Drop for AttachGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}
fn validate_attach_guard_file(file: &fs::File) -> Result<()> {
    use std::os::unix::fs::MetadataExt;
    let metadata = file.metadata()?;
    anyhow::ensure!(
        metadata.is_file()
            && metadata.uid() == unsafe { libc::geteuid() }
            && metadata.mode() & 0o777 == 0o600
            && metadata.nlink() == 1
            && metadata.len() <= 2,
        "invalid attachment guard file"
    );
    Ok(())
}
fn verify_attach_generation(
    request: &CheckedAttach,
    server_pid: u32,
    session_id: &str,
    created: &str,
) -> Result<()> {
    let expected = request
        .session
        .terminal
        .as_ref()
        .context("terminal identity unavailable")?;
    anyhow::ensure!(
        server_pid == expected.server_pid
            && session_id == expected.session_id
            && created == expected.created
            && identity().2 == request.session.boot_id
            && process_start_ticks(server_pid).as_ref() == Some(&expected.server_start),
        ATTACH_REFUSED
    );
    Ok(())
}
/// Called only by the synchronous if-shell predicate on the attaching connection.
pub fn native_attach_check(
    payload: &str,
    server_pid: u32,
    session_id: &str,
    created: &str,
) -> Result<()> {
    let request = parse_attach_payload(payload)?;
    verify_attach_generation(&request, server_pid, session_id, created)?;
    let token = request
        .guard
        .as_deref()
        .context("attachment guard unavailable")?;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(attach_guard_path(token)?)?;
    validate_attach_guard_file(&file)?;
    anyhow::ensure!(
        file.metadata()?.len() == 0,
        "attachment guard already consumed"
    );
    file.write_all(b"ok")?;
    file.sync_all()?;
    Ok(())
}
pub fn native_attach_checked(payload: &str) -> Result<()> {
    let mut request = parse_attach_payload(payload)?;
    anyhow::ensure!(request.guard.is_none(), "attachment guard is internal");
    let expected = request
        .session
        .terminal
        .clone()
        .context("terminal identity unavailable")?;
    verify_attach_generation(
        &request,
        expected.server_pid,
        &expected.session_id,
        &expected.created,
    )?;
    let mut guard = AttachGuard::new()?;
    request.guard = Some(guard.token.clone());
    let predicate_payload = attach_payload(&request)?;
    if !request.session.external {
        configure_managed()?;
        refresh_managed_status(&request.session.id)?;
    }
    let executable = std::env::current_exe()?;
    let predicate = format!(
        "{} native-attach-check {} '#{{pid}}' '#{{session_id}}' '#{{session_created}}'",
        quote(executable.to_str().context("helper path is not UTF-8")?),
        quote(&predicate_payload)
    );
    // All interpolated target components are validated numeric values. The
    // guard runs on this connection's server; never reconnect after checking it.
    let attach = format!(
        "attach-session -t {}{}",
        quote(&expected.session_id),
        if request.observe {
            " -r -f ignore-size"
        } else {
            ""
        }
    );
    let mut command = tmux(!request.session.external)?;
    if let Some(terminal) = attach_terminal()? {
        command.env("TERM", terminal);
    }
    command.args([
        "if-shell",
        "-t",
        &expected.session_id,
        &predicate,
        &attach,
        &format!("display-message -p {}", quote(ATTACH_REFUSED)),
    ]);
    let status = command
        .status()
        .context("checked terminal attachment failed")?;
    anyhow::ensure!(status.success() && guard.passed()?, ATTACH_REFUSED);
    Ok(())
}
pub fn attach(device: &Device, session: &Session, observe: bool) -> Result<()> {
    if std::env::var_os("TMUX").is_some() {
        bail!("cx attachment from inside tmux requires leaving the outer client first; protected outer sessions are kept intact");
    }
    if session.terminal.is_some() {
        let payload = attach_payload(&CheckedAttach {
            session: session.clone(),
            observe,
            guard: None,
        })?;
        if let Some(target) = &device.target {
            let routed = crate::store::ssh(target, true)?;
            let mut command = Command::new("ssh");
            command.arg("-t").args(routed.get_args());
            command.arg(format!(
                "~/.local/bin/cx native-attach-checked {}",
                quote(&payload)
            ));
            anyhow::ensure!(
                command
                    .status()
                    .context("checked remote attachment failed")?
                    .success(),
                ATTACH_REFUSED
            );
            return Ok(());
        }
        return native_attach_checked(&payload);
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

    fn inventory_fixture(case: &str) {
        let fixture = tempfile::tempdir().unwrap();
        let bin = fixture.path().join(".local/bin");
        fs::create_dir_all(&bin).unwrap();
        let mock = bin.join("tmux");
        // This executable simulates tmux metadata only; no tmux server is started.
        fs::write(&mock, r#"#!/bin/sh
managed=0
op=
for arg do
    [ "$arg" = -S ] && managed=1
    case "$arg" in list-sessions|display-message|set-option|attach-session|if-shell|source-file) op=$arg;; esac
    last=$arg
done
printf '%s\n' "$op" >> "$HOME/tmux-ops"
server=42
[ "$CX_INVENTORY_CASE" = distinct ] && [ "$managed" = 0 ] && server=43
[ "$CX_INVENTORY_CASE" = identity ] && server=$CX_INVENTORY_PID
[ "$CX_INVENTORY_CASE" = checked ] && server=$CX_INVENTORY_PID
pane=${CX_INVENTORY_PANE:-$CX_INVENTORY_PID}
dead=0
[ "$CX_INVENTORY_CASE" = dead ] && dead=1
panes=1
[ "$CX_INVENTORY_CASE" = expanded ] && { dead=1; panes=2; }
[ "$dead" = 1 ] && pane=
case "$op" in
list-sessions)
    if [ "$last" = '#{session_id}' ]; then printf '$1\n'
    else printf '$1\t%s\t100\t%%1\t%s\t%s\t1\t%s\n' "$pane" "$dead" "$server" "$panes"; fi;;
display-message)
    case "$last" in
    '#{session_name}') printf 'cx-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n';;
    '#{pane_pid}') printf '%s\n' "$pane";;
    '#{session_created}') printf '100\n';;
    '#{pane_current_path}') printf '/same-project\n';;
    '#{pane_current_command}') printf 'bash\n';;
    '#{pane_dead}') printf '%s\n' "$dead";;
    '#{pid}') printf '%s\n' "$server";;
    '#{session_id}'*)
        selected=1
        [ "$CX_INVENTORY_CASE" = moved ] && selected=2
        printf '$%s\t%s\t100\t%%1\t%s\t%s\t1\t%s\n' "$selected" "$pane" "$dead" "$server" "$panes";;
    *) printf '\n';;
    esac;;
esac
"#).unwrap();
        fs::set_permissions(&mock, fs::Permissions::from_mode(0o700)).unwrap();
        let result = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "sessions::tests::runtime_inventory_fixture_child",
                "--nocapture",
            ])
            .env("HOME", fixture.path())
            .env("XDG_STATE_HOME", fixture.path().join("state"))
            .env("CX_INVENTORY_CASE", case)
            .env_remove("TMUX")
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
    }

    #[test]
    fn runtime_inventory_fixture_child() {
        let Ok(case) = std::env::var("CX_INVENTORY_CASE") else {
            return;
        };
        std::env::set_var("CX_INVENTORY_PID", std::process::id().to_string());
        let sessions = list().unwrap();
        match case.as_str() {
            "legacy_attach" => {
                let mut session = sessions[0].clone();
                session.id = "$1".into();
                session.external = true;
                session.socket = None;
                session.terminal = None;
                attach(&crate::store::local_device(), &session, false).unwrap();
                let ops = fs::read_to_string(
                    PathBuf::from(std::env::var_os("HOME").unwrap()).join("tmux-ops"),
                )
                .unwrap();
                assert!(ops.lines().any(|op| op == "attach-session"));
                assert!(!ops
                    .lines()
                    .any(|op| op == "if-shell" || op == "source-file"));
            }
            "checked" => {
                let mut session = sessions[0].clone();
                session.id = "$1".into();
                session.external = true;
                session.socket = None;
                assert!(session.terminal.is_some());
                let device = crate::store::local_device();
                assert!(attach(&device, &session, false).is_err(), "a tmux command that never passes the generation guard must not report successful attachment");
                let ops = fs::read_to_string(
                    PathBuf::from(std::env::var_os("HOME").unwrap()).join("tmux-ops"),
                )
                .unwrap();
                assert!(ops.lines().any(|op| op == "if-shell"));
                assert!(
                    !ops.lines().any(|op| op == "source-file"),
                    "external servers must not be reconfigured"
                );
                assert!(
                    !fs::read_dir(state().unwrap()).unwrap().any(|entry| entry
                        .unwrap()
                        .file_name()
                        .to_string_lossy()
                        .starts_with("attach-guard-")),
                    "failed attachment must clean its guard"
                );
                let expected = session.terminal.clone().unwrap();
                let mut guard = AttachGuard::new().unwrap();
                let guard_path = guard.path.clone();
                let payload = attach_payload(&CheckedAttach {
                    session: session.clone(),
                    observe: false,
                    guard: Some(guard.token.clone()),
                })
                .unwrap();
                assert!(native_attach_check(
                    &payload,
                    expected.server_pid + 1,
                    &expected.session_id,
                    &expected.created
                )
                .is_err());
                assert!(native_attach_check(
                    &payload,
                    expected.server_pid,
                    "$999",
                    &expected.created
                )
                .is_err());
                assert!(native_attach_check(
                    &payload,
                    expected.server_pid,
                    &expected.session_id,
                    "0"
                )
                .is_err());
                assert!(!guard.passed().unwrap());
                native_attach_check(
                    &payload,
                    expected.server_pid,
                    &expected.session_id,
                    &expected.created,
                )
                .unwrap();
                assert!(guard.passed().unwrap());
                assert!(
                    native_attach_check(
                        &payload,
                        expected.server_pid,
                        &expected.session_id,
                        &expected.created
                    )
                    .is_err(),
                    "guard cannot be replayed"
                );
                drop(guard);
                assert!(!guard_path.exists());

                let guard = AttachGuard::new().unwrap();
                let payload = attach_payload(&CheckedAttach {
                    session: session.clone(),
                    observe: true,
                    guard: Some(guard.token.clone()),
                })
                .unwrap();
                let destination = state().unwrap().join("guard-symlink-target");
                fs::write(&destination, b"preserve").unwrap();
                fs::remove_file(&guard.path).unwrap();
                std::os::unix::fs::symlink(&destination, &guard.path).unwrap();
                assert!(native_attach_check(
                    &payload,
                    expected.server_pid,
                    &expected.session_id,
                    &expected.created
                )
                .is_err());
                assert_eq!(fs::read(&destination).unwrap(), b"preserve");
                drop(guard);

                let mut stale = session;
                stale.boot_id = "different-boot".into();
                assert!(native_attach_checked(
                    &attach_payload(&CheckedAttach {
                        session: stale,
                        observe: false,
                        guard: None
                    })
                    .unwrap()
                )
                .is_err());
            }
            "identity" => {
                assert_eq!(sessions.len(), 1);
                let terminal = sessions[0]
                    .terminal
                    .clone()
                    .expect("verified tmux server must expose terminal generation");
                assert_eq!(terminal.server_pid, std::process::id());
                assert_eq!(terminal.session_id, "$1");
                let fresh = inspect(true, &sessions[0].id).unwrap();
                assert_eq!(
                    fresh.terminal.as_ref(),
                    Some(&terminal),
                    "creation inspection and refresh must produce the same key"
                );
                std::env::set_var("CX_INVENTORY_PANE", "99999999");
                let changed = list().unwrap();
                assert_ne!(changed[0].pid, sessions[0].pid);
                assert_eq!(
                    changed[0].terminal.as_ref(),
                    Some(&terminal),
                    "active pane changes must preserve terminal generation"
                );
            }
            "alias" => {
                assert_eq!(
                    sessions.len(),
                    1,
                    "one tmux runtime reached through two socket aliases must have one row"
                );
                assert!(
                    !sessions[0].external,
                    "managed ownership must win over its external alias"
                );
            }
            "distinct" => assert_eq!(
                sessions.len(),
                2,
                "same folder/title/session number on distinct servers is distinct work"
            ),
            "dead" => assert!(
                sessions.is_empty(),
                "retained dead panes are not running shells or agents"
            ),
            "expanded" => assert_eq!(
                sessions.len(),
                1,
                "a selected dead pane must not hide other work in an expanded session"
            ),
            "moved" => assert!(
                sessions.is_empty(),
                "a pane moved to another session must not inherit its previous attachment identity"
            ),
            _ => panic!("unknown fixture"),
        }
    }

    #[test]
    fn runtime_inventory_deduplicates_server_aliases() {
        inventory_fixture("alias");
    }

    #[test]
    fn runtime_inventory_preserves_distinct_servers() {
        inventory_fixture("distinct");
    }

    #[test]
    fn runtime_inventory_omits_dead_panes() {
        inventory_fixture("dead");
    }

    #[test]
    fn runtime_inventory_preserves_expanded_sessions() {
        inventory_fixture("expanded");
    }

    #[test]
    fn runtime_inventory_rejects_moved_pane_metadata() {
        inventory_fixture("moved");
    }

    #[test]
    fn runtime_inventory_terminal_identity_survives_pane_change() {
        inventory_fixture("identity");
    }

    #[test]
    fn checked_attach_requires_guard_success_and_preserves_external_config() {
        inventory_fixture("checked");
    }

    #[test]
    fn checked_attach_preserves_old_helper_legacy_path() {
        inventory_fixture("legacy_attach");
    }

    #[test]
    fn checked_attach_metadata_is_bounded_and_guard_tokens_are_fixed() {
        assert!(parse_attach_payload(&"A".repeat(ATTACH_PAYLOAD_LIMIT + 1)).is_err());
        assert!(parse_attach_payload("not base64").is_err());
        for token in ["", "../guard", "/tmp/guard", "$(false)"] {
            assert!(!valid_attach_guard(token));
        }
        assert!(valid_attach_guard(&"a".repeat(64)));
        assert!(!valid_attach_guard(&"g".repeat(64)));
    }

    #[test]
    fn terminal_generation_rejects_replaced_or_exited_server() {
        let mut observed = parse_snapshot("$1\t10\t100\t%2\t0\t42\t1\t1")
            .unwrap()
            .remove(0);
        observed.server_start = Some("700".into());
        let first = terminal_identity(&observed, Some("700".into()))
            .unwrap()
            .unwrap();
        assert!(terminal_identity(&observed, Some("701".into())).is_err());
        assert!(terminal_identity(&observed, None).is_err());
        observed.server_start = Some("701".into());
        let replacement = terminal_identity(&observed, Some("701".into()))
            .unwrap()
            .unwrap();
        assert_ne!(
            replacement, first,
            "reused server PID/session number must produce a new generation"
        );
        observed.server_start = None;
        assert!(
            terminal_identity(&observed, Some("701".into()))
                .unwrap()
                .is_none(),
            "one observation cannot establish a stable generation"
        );
    }

    #[test]
    fn server_start_ticks_require_matching_live_process_metadata() {
        let stat = format!(
            "42 (name ) with (parens)) S {} 700",
            vec!["0"; 18].join(" ")
        );
        assert_eq!(parse_process_start_ticks(42, &stat).as_deref(), Some("700"));
        assert!(parse_process_start_ticks(43, &stat).is_none());
        assert!(parse_process_start_ticks(42, &stat.replace(") S ", ") Z ")).is_none());
        assert!(parse_process_start_ticks(42, &stat.replace("700", "unknown")).is_none());
        assert!(parse_process_start_ticks(42, "42 (short) S 0").is_none());
        assert!(process_start_ticks(std::process::id()).is_some());
    }

    #[test]
    fn runtime_snapshot_rejects_malformed_identity_and_framing() {
        let valid = "$1\t10\t100\t%2\t0\t42\t1\t1";
        assert_eq!(parse_snapshot(valid).unwrap().len(), 1);
        for invalid in [
            "$1\t10\t100\t%2\t0\t42\t1\t1\textra",
            "$1\t10\t100\t%2\t0\t42\t1",
            "$name\t10\t100\t%2\t0\t42\t1\t1",
            "$1\t10\t100\t%2\t2\t42\t1\t1",
            "$1\t10\t100\t%2\t0\t0\t1\t1",
            "$1\t10\t100\t%2\t0\t42\t0\t1",
            "$1\t10\t100\t%2\t0\t42\t1\t1\nforged",
        ] {
            assert!(parse_snapshot(invalid).is_err());
        }
    }

    #[test]
    fn runtime_snapshot_accepts_cleared_pid_only_for_dead_pane() {
        let dead = parse_snapshot("$1\t\t100\t%2\t1\t42\t1\t1")
            .unwrap()
            .remove(0);
        assert_eq!(dead.pane_pid, 0);
        assert!(dead.ended());
        let expanded = parse_snapshot("$1\t\t100\t%2\t1\t42\t1\t2")
            .unwrap()
            .remove(0);
        assert!(!expanded.ended());
        assert!(parse_snapshot("$1\t\t100\t%2\t0\t42\t1\t1").is_err());
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
    fn generated_session_names_follow_provider_workspace_and_keep_custom_names() {
        let mut session = Session {
            terminal: None,
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
