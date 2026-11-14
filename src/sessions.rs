#[path = "sessions/control.rs"]
mod control;
#[path = "sessions/preview.rs"]
mod preview;
use crate::model::{CreateSession, Device, ProcessIdentity, RunCommand, Session, TerminalIdentity};
use anyhow::{bail, Context, Result};
pub use control::watch;
pub use preview::{preview, session_pane_input};
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

/// Set by `cx helper`: sessions it creates were asked for by a viewer on another device (or over SSH), so they
/// never get this device's exposed secrets.
pub static SERVING_HELPER: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);
/// Names (never values) of the variables exposed secrets set in a session, so a cx run inside it
/// keeps them out of tmux's server environment.
const SESSION_SECRETS: &str = "CX_SESSION_SECRETS";

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
const SNAPSHOT_FORMAT: &str = "#{session_id}\t#{pane_pid}\t#{session_created}\t#{pane_id}\t#{pane_dead}\t#{pid}\t#{session_windows}\t#{window_panes}\t#{window_activity}";
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
    /// Unix seconds of the window's last output; empty or zero means unknown.
    activity: Option<u64>,
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
            anyhow::ensure!(columns.len() == 9, "invalid tmux inventory metadata");
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
            // Optional evidence: malformed activity loses its age, not its session.
            let activity = (!columns[8].is_empty()
                && columns[8].bytes().all(|b| b.is_ascii_digit()))
            .then(|| columns[8].parse::<u64>().ok())
            .flatten()
            .filter(|t| *t > 0);
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
                activity,
            })
        })
        .collect()
}
/// Seconds since `activity`, measured on this host; future stamps count as now.
fn idle_seconds(activity: Option<u64>) -> Option<u64> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs();
    activity.map(|t| now.saturating_sub(t))
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
    tracked_title: Option<&str>,
    fallback: String,
) -> String {
    // A proven selected thread can have a title before its directory arrives.
    let selected = native.filter(|metadata| !metadata.current_thread_unknown);
    if let Some(title) = selected.and_then(|metadata| metadata.title.clone()) {
        title
    } else if let Some(title) = tracked_title.filter(|t| !t.is_empty()) {
        // The provider sets its terminal title to the live thread's name even
        // when CX cannot pin the thread's directory, so a project switch in the
        // agent's own control panel still relabels the session.
        title.to_string()
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
        // (`env` is the launcher every pane starts as, `exec env … shell`: a listing
        // while a loaded host still starts the shell must not label it `recent: env`.)
        || ["fish", "bash", "zsh", "sh", "dash", "tmux", "cx", "env"].contains(&command)
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
    // Always resolve a real directory to show. A proven thread gives its own
    // working directory; otherwise fall back to the process's current directory
    // (the launch location) rather than leaving the row "Directory unavailable".
    let directory = native
        .as_ref()
        .filter(|_| !current_thread_unknown)
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
        .unwrap_or(field(managed, target, "#{pane_current_path}")?);
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
        shell_activity(managed, target).unwrap_or(saved_name)
    } else if automatic {
        automatic_provider_name(
            provider,
            native.as_ref(),
            current_thread_unknown,
            tracked_title.as_deref(),
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
                field(managed, target, "#{pane_current_command}")
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
            recent_terminal_output(&field(managed, target, "#{window_activity}").ok()?, now)
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
    let last_output = {
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
        current[0].activity.or(expected.activity)
    };
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
        activity,
        agent_tasks,
        idle_seconds: idle_seconds(last_output),
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
/// A live, interactive shell is a stable terminal session worth listing; a
/// transient or exiting process that briefly owned a pseudo-terminal is not.
fn interactive_shell_leader(pid: u32) -> bool {
    let stat = match fs::read_to_string(format!("/proc/{pid}/stat")) {
        Ok(stat) => stat,
        Err(_) => return false,
    };
    // Skip zombies and dying leaders: they vanish before they can be adopted.
    if stat
        .rsplit_once(')')
        .and_then(|(_, rest)| rest.split_whitespace().next())
        .is_some_and(|state| state == "Z" || state == "X")
    {
        return false;
    }
    let comm = fs::read_to_string(format!("/proc/{pid}/comm")).unwrap_or_default();
    matches!(
        comm.trim(),
        "bash" | "zsh" | "fish" | "sh" | "dash" | "ash" | "ksh" | "mksh" | "tcsh" | "csh"
    )
}

fn external_terminal(terminal: &crate::adoption::Terminal) -> Option<Session> {
    let (host, account, boot_id) = identity();
    let leader = terminal.leader.pid;
    let detected = foreground_provider(leader);
    let provider = detected.as_ref().map(|(p, _)| *p).unwrap_or("shell");
    // List a terminal only when it is a real agent session or a live
    // interactive shell. Anything else that momentarily held a pseudo-terminal
    // would disappear from the list before it could be selected.
    if provider == "shell" && !interactive_shell_leader(leader) {
        return None;
    }
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
            None,
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
        terminal: None,
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
        idle_seconds: None,
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
    // This install, and its replaced copies other viewers may still run after an update.
    let mut executables = vec![];
    if let Ok(exe) = crate::self_exe() {
        let mut replaced = exe.clone().into_os_string();
        replaced.push(" (deleted)");
        executables.extend([exe, replaced.into()]);
    }
    executables.extend(crate::adoption::relay_executables());
    // Exclude the viewer's own terminal and every ancestor's session, so the
    // terminal that launched CX never appears even across a session boundary.
    let mut sessions = vec![unsafe { libc::getsid(0) }];
    let mut pid = std::process::id();
    for _ in 0..32 {
        let Ok(stat) = fs::read_to_string(format!("/proc/{pid}/stat")) else {
            break;
        };
        let Some(rest) = stat.rsplit_once(')').map(|(_, r)| r) else {
            break;
        };
        let fields: Vec<_> = rest.split_whitespace().collect();
        let (Some(ppid), Some(sid)) = (
            fields.get(2).and_then(|v| v.parse::<i32>().ok()),
            fields.get(3).and_then(|v| v.parse::<i32>().ok()),
        ) else {
            break;
        };
        sessions.push(sid);
        if ppid <= 1 {
            break;
        }
        pid = ppid as u32;
    }
    let exclude = crate::adoption::Exclusions {
        executables,
        sessions,
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
    let errors = root.join(format!("{name}.adopt"));
    let leader = &terminal.leader;
    // Build the whole launch command FIRST, so a failure here (e.g. no reachable
    // password prompt on a remote helper) never leaves a record that would hide
    // the terminal from future listings.
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
        // `sudo -A` reads the password through CX's askpass broker and sets
        // SUDO_UID itself; the relay drops to that user after the steal.
        let env = crate::auth::askpass_env().context(
            "moving a terminal here needs a no-prompt relay: run `cx device ptrace <device> enable`",
        )?;
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
    // A leftover record only blocks when its managed session is still alive;
    // a stale one from an interrupted move is cleared and the move proceeds.
    if record.exists() {
        if managed_target(&name)?.is_some() {
            return inspect(true, &name);
        }
        let _ = fs::remove_file(&record);
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
        set_managed_status(&session, viewer_ascii())?;
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
        let Some(session_name) = name.strip_suffix(".json") else {
            continue;
        };
        // Only a live managed session hides its origin terminal. A leftover
        // record from an interrupted move must not hide it forever.
        if let Some(origin) = fs::read(entry.path())
            .ok()
            .and_then(|v| serde_json::from_slice::<Session>(&v).ok())
            .and_then(|s| s.origin)
            .filter(|_| managed_target(session_name).ok().flatten().is_some())
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
            // Command-panel panes belong only in their panel.
            // So does the hidden control-mode session.
            if managed
                && field(true, &observed.pane_id, "#{session_name}")
                    .is_ok_and(|n| valid_pane_name(&n) || n == control::CONTROL_SESSION)
            {
                continue;
            }
            if let Ok(s) = inspect_observed(managed, &observed, &host_identity) {
                seen.insert(runtime);
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
    // A session of its own (so also a group of its own, signalled below) and no
    // controlling terminal: an interactive bash or sh started from a terminal-run cx
    // otherwise takes that terminal for job control from a background group and is
    // stopped by SIGTTOU until the timeout. SAFETY: setsid is async-signal-safe.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
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
    let supports_terminal_color = tmux_at_least(3, 4);
    if !supports_terminal_color && !tmux_at_least(3, 0) {
        config = without_status_mouse(&config);
    }
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
/// Status-line mouse locations are unverified before tmux 3.0, and an unknown
/// key name would fail the whole `source-file`. There the chip stays plain
/// text and Ctrl+\ remains the detach key.
fn without_status_mouse(config: &str) -> String {
    config
        .lines()
        .filter(|line| !line.starts_with("bind-key -n MouseDown1StatusLeft "))
        .map(|line| {
            if line.starts_with("set -g status-left ") {
                "set -g status-left 'detach: Ctrl+\\ #{?pane_in_mode,Scroll: y copy / Esc,- cx} '"
                    .to_owned()
            } else if line.starts_with("set -g status-left-length ") {
                "set -g status-left-length 40".to_owned()
            } else if line.starts_with("set -g window-status-current-format ") {
                "set -g window-status-current-format \"\"".to_owned()
            } else {
                line.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}
/// Bounded `tmux -V` comparison; custom tmux wrappers cannot block setup and
/// an unparseable version counts as old.
fn tmux_at_least(major: u32, minor: u32) -> bool {
    let mut version = Command::new("/bin/sh");
    version.args([
        "-c",
        r#"version=$("$1" -V 2>/dev/null | /usr/bin/head -c 128)
case "$version" in 'tmux '*) version=${version#tmux };; *) exit 1;; esac
major=${version%%.*}; minor=${version#*.}; minor=${minor%%[!0-9]*}
case "$major:$minor" in *[!0-9:]*|:*|*:) exit 1;; esac
[ "$major" -gt "$2" ] || { [ "$major" -eq "$2" ] && [ "$minor" -ge "$3" ]; }
"#,
        "cx-tmux-version",
    ]);
    version
        .arg(tmux_executable())
        .arg(major.to_string())
        .arg(minor.to_string());
    bounded_provider_check(version, Duration::from_millis(200))
}
/// Settings a managed server takes only when it starts (`-f`), never sourced into a
/// running one: an update or attach must leave live panes' server as it was.
fn start_config(mut config: String) -> String {
    if tmux_at_least(3, 3) {
        // Bitmaps (kitty placeholders, iTerm2, sixel) from cx and agents inside a managed
        // pane reach the terminal; this server only, never the user's own tmux.
        config.push_str("\nset -g allow-passthrough on\n");
    }
    config
}
pub fn configure_managed() -> Result<()> {
    let conf = state()?.join("tmux.conf");
    fs::write(&conf, managed_config())?;
    fs::set_permissions(&conf, fs::Permissions::from_mode(0o600))?;
    let mut c = tmux(true)?;
    // The pid names the configured server instance, so attach can skip this.
    c.arg("source-file")
        .arg(conf)
        .args([";", "display-message", "-p", "#{pid}"]);
    if let Ok(pid) = output(c)?.trim().parse() {
        let _ = control::record_configured(pid);
    }
    Ok(())
}

/// Refresh the allowlisted Anthropic variables in tmux's server environment.
/// Tmux keeps a server-wide snapshot, which otherwise survives shell reloads.
/// Variables an exposed secret set in this cx's session ([`SESSION_SECRETS`]) are
/// left as they are: that value is the session's own and never goes to the server.
pub fn refresh_provider_environment() -> Result<()> {
    const VARS: &[&str] = &[
        "ANTHROPIC_API_KEY",
        "ANTHROPIC_AUTH_TOKEN",
        "ANTHROPIC_BASE_URL",
        "ANTHROPIC_CUSTOM_HEADERS",
    ];
    let injected = std::env::var(SESSION_SECRETS).unwrap_or_default();
    for name in VARS
        .iter()
        .filter(|name| !injected.split(',').any(|v| v == **name))
    {
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
fn server_responds(attempts: u32) -> Result<bool> {
    for _ in 0..attempts {
        let mut c = tmux(true)?;
        c.args(["show-options", "-g"]);
        if c.output()?.status.success() {
            return Ok(true);
        }
        thread::sleep(Duration::from_millis(25));
    }
    Ok(false)
}
/// True when the named user unit is loaded but neither running nor starting.
fn stale_unit(unit: &str) -> bool {
    let Ok(out) = Command::new("systemctl")
        .args([
            "--user",
            "show",
            "-p",
            "LoadState",
            "-p",
            "ActiveState",
            "--",
            unit,
        ])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
    else {
        return false;
    };
    unit_is_stale(&String::from_utf8_lossy(&out.stdout))
}
fn unit_is_stale(show: &str) -> bool {
    let field = |key: &str| {
        show.lines()
            .find_map(|l| l.strip_prefix(key)?.strip_prefix('='))
            .unwrap_or("")
    };
    field("LoadState") == "loaded" && matches!(field("ActiveState"), "inactive" | "failed")
}
/// First line of a child's stderr, trimmed to a short printable summary.
fn bounded_stderr(raw: &[u8]) -> String {
    let text = String::from_utf8_lossy(raw);
    let line = text
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    line.chars().filter(|c| !c.is_control()).take(200).collect()
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
    fs::write(&conf, start_config(managed_config()))?;
    fs::set_permissions(&conf, fs::Permissions::from_mode(0o600))?;
    let sock = socket()?;
    // A foreground tmux server in its own user service cannot inherit the metadata service cgroup.
    let bus = PathBuf::from(format!("/run/user/{}/bus", unsafe { libc::geteuid() }));
    if bus.exists() {
        let unit = format!(
            "cx-tmux-{:x}.service",
            Sha256::digest(sock.as_os_str().as_encoded_bytes())
        );
        let run = Command::new("systemd-run")
            .args([
                "--user",
                "--quiet",
                "--collect",
                &format!("--unit={unit}"),
                "--property=Restart=no",
            ])
            .arg(tmux_executable())
            .arg("-u")
            .arg("-D")
            .arg("-S")
            .arg(&sock)
            .arg("-f")
            .arg(&conf)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output()?;
        // An existing owned service is acceptable if its socket responds.
        if server_responds(40)? {
            return configure_managed();
        }
        // A stopped transient unit stays loaded while any member it owns is
        // alive (pane scopes are PartOf it and may outlive the server), so
        // systemd-run refuses the fixed name. Start that same unit instead.
        if stale_unit(&unit) {
            let start = Command::new("systemctl")
                .args(["--user", "start", "--", &unit])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()?;
            if start.success() && server_responds(40)? {
                return configure_managed();
            }
        }
        let detail = bounded_stderr(&run.stderr);
        bail!(
            "persistent tmux user service did not start (systemd-run {}{}); check user-service access",
            run.status,
            if detail.is_empty() {
                String::new()
            } else {
                format!(": {detail}")
            }
        );
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
fn provider_command(provider: &str, yolo: bool, resume: bool, pick: bool) -> Result<String> {
    if !resume && pick {
        bail!("a session picker applies only when resuming");
    }
    // Resume either the most recent session (--last / --continue) or the
    // provider's own interactive picker (codex resume / claude --resume).
    let base = match (provider, resume, pick) {
        ("codex", false, _) => "codex",
        ("codex", true, false) => "codex resume --last",
        ("codex", true, true) => "codex resume",
        ("claude", false, _) => "claude",
        ("claude", true, false) => "claude --continue",
        ("claude", true, true) => "claude --resume",
        // Fall through to the YOLO check so shell + YOLO is still rejected.
        ("shell", false, false) => "shell",
        ("shell", _, _) => {
            bail!("resume is available only for Codex or Claude sessions")
        }
        _ => bail!("unsupported launch profile"),
    };
    let flag = match (provider, yolo) {
        (_, false) => "",
        ("codex", true) => " --dangerously-bypass-approvals-and-sandbox",
        ("claude", true) => " --dangerously-skip-permissions",
        ("shell", true) => bail!("YOLO applies only to Codex or Claude sessions"),
        _ => bail!("unsupported launch profile"),
    };
    Ok(format!("{base}{flag}"))
}
pub fn create_container(
    request: &CreateSession,
    scope: &crate::model::ContainerScope,
    yolo: bool,
) -> Result<Session> {
    anyhow::ensure!(
        request.directory == scope.folder,
        "Container launch directory does not match the selected scope"
    );
    // Validate unsupported shell permissions/history before any container work.
    provider_command(&request.provider, yolo, request.resume, request.resume)?;
    crate::containers::prepare(scope, &request.provider)?;
    create_with_permissions(request, yolo, Some(scope))
}
fn container_run_payload(
    scope: &crate::model::ContainerScope,
    request: &CreateSession,
    yolo: bool,
) -> Result<String> {
    use base64::Engine;
    let value = if request.resume {
        serde_json::json!([scope, request.provider, yolo, true])
    } else {
        serde_json::json!([scope, request.provider, yolo])
    };
    let payload = base64::engine::general_purpose::STANDARD.encode(serde_json::to_vec(&value)?);
    anyhow::ensure!(
        payload.len() < 65536,
        "container launch payload exceeds limit"
    );
    Ok(payload)
}
#[derive(serde::Serialize, serde::Deserialize, PartialEq, Eq)]
struct LaunchIntent {
    provider: String,
    resume: bool,
    #[serde(default)]
    resume_pick: bool,
    yolo: bool,
    directory: String,
    container: Option<crate::model::ContainerScope>,
}
fn read_launch_record<T: serde::de::DeserializeOwned>(path: &std::path::Path) -> Result<T> {
    use std::io::Read;
    use std::os::unix::fs::MetadataExt;
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    let metadata = file.metadata()?;
    anyhow::ensure!(
        metadata.is_file()
            && metadata.uid() == unsafe { libc::geteuid() }
            && metadata.len() <= 65536,
        "unsafe session launch record"
    );
    let mut bytes = Vec::new();
    file.take(65537).read_to_end(&mut bytes)?;
    anyhow::ensure!(bytes.len() <= 65536, "session launch record exceeds limit");
    Ok(serde_json::from_slice(&bytes)?)
}
fn validate_launch_intent(
    root: &std::path::Path,
    name: &str,
    expected: &LaunchIntent,
) -> Result<()> {
    let saved = match read_launch_record::<LaunchIntent>(&root.join(format!("{name}.launch"))) {
        Ok(saved) => saved,
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound) =>
        {
            let original: Session = read_launch_record(&root.join(format!("{name}.json")))?;
            // Old launches never recorded resume selection. Refuse any resumed
            // retry rather than guessing which conversation an old key owns.
            anyhow::ensure!(
                !expected.resume,
                "creation key has no recorded history selection; use a new key"
            );
            LaunchIntent {
                provider: original.provider,
                resume: false,
                resume_pick: false,
                yolo: expected.yolo,
                directory: original.directory,
                container: original.container,
            }
        }
        Err(error) => return Err(error),
    };
    anyhow::ensure!(
        saved == *expected,
        "creation key already belongs to different launch settings; use a new key"
    );
    Ok(())
}
fn create_with_permissions(
    request: &CreateSession,
    yolo: bool,
    container: Option<&crate::model::ContainerScope>,
) -> Result<Session> {
    let mut agent_command =
        provider_command(&request.provider, yolo, request.resume, request.resume_pick)?;
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
    let launch = LaunchIntent {
        provider: request.provider.clone(),
        resume: request.resume,
        resume_pick: if container.is_some() {
            request.resume
        } else {
            request.resume_pick
        },
        yolo,
        directory: container
            .map(|c| c.folder.clone())
            .unwrap_or_else(|| directory.to_string_lossy().into_owned()),
        container: container.cloned(),
    };
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
        validate_launch_intent(&root, &name, &launch)?;
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
    // Secrets marked "expose to sessions" go only to sessions this device's own viewer or CLI starts, never one a
    // helper starts for another device, and not into containers.
    let exposed =
        if container.is_none() && !SERVING_HELPER.load(std::sync::atomic::Ordering::Relaxed) {
            crate::secrets::Secrets::open()?.session_specs()?
        } else {
            Vec::new()
        };
    ensure_server()?;
    refresh_provider_environment()?;
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
        activity: None,
        agent_tasks: vec![],
        idle_seconds: None,
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
    let mut launch_file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(root.join(format!("{name}.launch")))?;
    launch_file.write_all(&serde_json::to_vec(&launch)?)?;
    launch_file.sync_all()?;
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
        let payload = container_run_payload(scope, request, yolo)?;
        launcher = vec![
            crate::self_exe()?.to_str().context("cx path")?.into(),
            "container-run".into(),
            payload,
        ];
    }
    // `cx secrets get --dir STATE VAR=NAME... --exec --` reads the values in the pane and execs the launcher with them in its
    // environment: argv and tmux (options, environment) only ever see names, and the server keeps nothing.
    let mut marker = String::new();
    if !exposed.is_empty() {
        let vars: Vec<&str> = exposed
            .iter()
            .filter_map(|spec| spec.split_once('=').map(|(var, _)| var))
            .collect();
        marker = format!("{SESSION_SECRETS}={} ", quote(&vars.join(",")));
        let mut wrapped = vec![
            crate::self_exe()?.to_str().context("cx path")?.into(),
            "secrets".into(),
            "get".into(),
            "--dir".into(),
            root.to_str().context("state path is not UTF-8")?.into(),
        ];
        wrapped.extend(exposed);
        wrapped.extend(["--exec".into(), "--".into()]);
        wrapped.append(&mut launcher);
        launcher = wrapped;
    }
    let title_tracking = container.is_none()
        && request.provider == "codex"
        && launcher
            .iter()
            .any(|arg| arg.contains("tui.terminal_title="));
    let command = format!(
        "{}exec env {}{marker}CX_VIEWER_THEME=1 CX_TMUX_BIN={} CX_MANAGED_SOCKET={} CX_MANAGED_SESSION={} {}",
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
    set_managed_status(&session, viewer_ascii()).context("terminal unavailable during startup")?;
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

// Codex uses ANSI bright-black (index8) for prompt/answer backgrounds when
// terminal background discovery is unavailable. User palettes may make that an
// accent colour. Scope a neutral fallback to our pane, never the user's terminal.
fn restore_codex_contrast(session: &Session) -> Result<()> {
    if session.external {
        return Ok(());
    }
    let pane = field(true, &session.id, "#{pane_id}")?;
    let mut query = tmux(true)?;
    query.args(["show-options", "-p", "-qv", "-t", &pane, "@cx_codex_dim_bg"]);
    let owned = output(query)?.trim().to_owned();
    let mut query = tmux(true)?;
    query.args([
        "show-options",
        "-p",
        "-A",
        "-qv",
        "-t",
        &pane,
        "pane-colours[8]",
    ]);
    let current = output(query)?.trim().to_owned();
    if session.provider == "codex" {
        if !current.is_empty() && current != owned {
            return Ok(());
        }
        let theme = field(true, &session.id, "#{client_theme}").unwrap_or_default();
        let neutral = if theme.trim() == "light" {
            "#e4e4e4"
        } else {
            "#303030"
        };
        let mut set = tmux(true)?;
        set.args([
            "set-option",
            "-p",
            "-t",
            &pane,
            "pane-colours[8]",
            neutral,
            ";",
            "set-option",
            "-p",
            "-t",
            &pane,
            "@cx_codex_dim_bg",
            neutral,
        ]);
        output(set)?;
    } else if !owned.is_empty() {
        let mut clear = tmux(true)?;
        if current == owned {
            clear.args(["set-option", "-pu", "-t", &pane, "pane-colours[8]", ";"]);
        }
        clear.args(["set-option", "-pu", "-t", &pane, "@cx_codex_dim_bg"]);
        output(clear)?;
    }
    Ok(())
}

fn restore_managed_style(session: &Session) -> Result<()> {
    // Match the private server's terminal defaults, including transparency.
    // Also clear opaque per-window overrides left by older CX versions.
    for option in ["window-style", "window-active-style"] {
        let mut query = tmux(true)?;
        // `-w`: these are window options; tmux < 3.0 rejects them without it.
        query.args(["show-options", "-g", "-w", "-v", option]);
        let style = output(query)?;
        let mut command = tmux(true)?;
        command.args(["set-option", "-w", "-t", &session.id, option, style.trim()]);
        output(command)?;
    }
    Ok(())
}

/// Unicode glyphs are safe: no CX_ASCII, no dumb or console TERM, and a UTF-8
/// locale. LC_ALL > LC_CTYPE > LANG; the first non-empty one decides. With no
/// locale at all (common under systemd/ssh), assume UTF-8: every terminal cx
/// targets is UTF-8, and an explicit `C`/`POSIX` still selects ASCII.
pub(crate) fn unicode_from(env: &dyn Fn(&str) -> Option<String>) -> bool {
    let term = env("TERM").unwrap_or_default().to_ascii_lowercase();
    let locale_utf8 = ["LC_ALL", "LC_CTYPE", "LANG"]
        .iter()
        .find_map(|k| env(k).filter(|v| !v.is_empty()))
        .map(|v| {
            let v = v.to_ascii_lowercase();
            v.contains("utf-8") || v.contains("utf8")
        })
        .unwrap_or(true);
    env("CX_ASCII").is_none() && term != "dumb" && term != "linux" && locale_utf8
}
/// This process's terminal needs ASCII glyphs (the managed tmux status and
/// the attach payload). Environment only, never a terminal probe; the UI's
/// capability detection applies the same rule.
pub(crate) fn viewer_ascii() -> bool {
    !unicode_from(&|k| std::env::var(k).ok())
}

fn set_managed_status(session: &Session, ascii: bool) -> Result<()> {
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
    // The title sits beside the detach chip (status-left/window cell); the
    // right side names the host and the detach key. An ASCII viewer gets
    // ASCII glyphs in the chip and separators. One tmux command list.
    let status = format!(
        "{}@{}{}Ctrl+\\ detach",
        safe(&session.account),
        safe(&session.host),
        if ascii { " | " } else { " · " }
    );
    let title = safe(&session.name);
    let mut command = tmux(true)?;
    command.args([
        "set-option",
        "-t",
        &session.id,
        "status-right",
        &status,
        ";",
        // Read by the status line as `cx · <title>` next to the detach chip.
        "set-option",
        "-t",
        &session.id,
        "@cx_title",
        &title,
        ";",
        "set-option",
        "-t",
        &session.id,
        "@cx_ascii",
        if ascii { "1" } else { "0" },
    ]);
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
    refresh_managed_status_as(id, viewer_ascii())
}
/// `ascii` comes from the viewer: over SSH only TERM reaches the host, so a
/// remote attach carries the viewer's CX_ASCII in its payload.
fn refresh_managed_status_as(id: &str, ascii: bool) -> Result<()> {
    let id = id.strip_prefix('=').unwrap_or(id);
    if !id.starts_with("cx-") || id.len() != 67 || !id[3..].bytes().all(|b| b.is_ascii_hexdigit()) {
        bail!("invalid managed session identity");
    }
    let session = inspect(true, id)?;
    restore_managed_style(&session)?;
    // Older tmux builds may lack pane-colours; optional styling must not
    // prevent reaching a working terminal.
    let _ = restore_codex_contrast(&session);
    set_managed_status(&session, ascii)
}

pub fn quote(s: &str) -> String {
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
    /// The viewer's ASCII switch. Omitted when false so older helpers, which
    /// deny unknown fields, still accept ordinary payloads.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    ascii: bool,
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
        // (The predicate removes the name once it has written its verdict.)
        validate_attach_guard_meta(&self.file, 0..=1)?;
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
    validate_attach_guard_meta(file, 1..=1)
}
fn validate_attach_guard_meta(file: &fs::File, links: std::ops::RangeInclusive<u64>) -> Result<()> {
    use std::os::unix::fs::MetadataExt;
    let metadata = file.metadata()?;
    anyhow::ensure!(
        metadata.is_file()
            && metadata.uid() == unsafe { libc::geteuid() }
            && metadata.mode() & 0o777 == 0o600
            && links.contains(&metadata.nlink())
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
    // Its owner reads the verdict through the descriptor it holds, so the name can go
    // now: an attach can last days and its process can die without unwinding (a closed
    // terminal's watchdog `_exit`, SIGHUP over ssh), which used to leave the file behind.
    let _ = fs::remove_file(attach_guard_path(token)?);
    Ok(())
}
/// Stale attachment leftovers in the state dir: `attach-guard-<64 hex>` files (and
/// v0.2.0's `attach-ticket-<hex>`) older than an hour that no process holds open. Only
/// where /proc tells which files are open; nothing else is ever touched.
pub fn sweep_attach_leftovers() {
    use std::os::unix::fs::MetadataExt;
    const BOUND: Duration = Duration::from_secs(3600);
    let Ok(dir) = state().and_then(|d| Ok(d.canonicalize()?)) else {
        return;
    };
    let Ok(entries) = fs::read_dir(&dir) else {
        return;
    };
    let hex = |h: &str| !h.is_empty() && h.len() <= 64 && h.bytes().all(|b| b.is_ascii_hexdigit());
    let stale: Vec<PathBuf> = entries
        .flatten()
        .filter(|e| {
            let name = e.file_name();
            let name = name.to_string_lossy();
            let ours = name
                .strip_prefix("attach-guard-")
                .is_some_and(valid_attach_guard)
                || name.strip_prefix("attach-ticket-").is_some_and(hex);
            // (DirEntry metadata does not follow a symlink.)
            ours && e.metadata().is_ok_and(|m| {
                m.is_file()
                    && m.uid() == unsafe { libc::geteuid() }
                    && m.modified()
                        .ok()
                        .and_then(|t| t.elapsed().ok())
                        .is_some_and(|age| age > BOUND)
            })
        })
        .map(|e| e.path())
        .collect();
    if stale.is_empty() {
        return;
    }
    // A live owner holds its guard open.
    let Ok(procs) = fs::read_dir("/proc") else {
        return;
    };
    let mut open = std::collections::HashSet::new();
    for p in procs.flatten() {
        if !p
            .file_name()
            .to_string_lossy()
            .bytes()
            .all(|b| b.is_ascii_digit())
        {
            continue;
        }
        for fd in fs::read_dir(p.path().join("fd"))
            .into_iter()
            .flatten()
            .flatten()
        {
            if let Ok(target) = fs::read_link(fd.path()) {
                if target.starts_with(&dir) {
                    open.insert(target);
                }
            }
        }
    }
    for path in stale.iter().filter(|p| !open.contains(*p)) {
        let _ = fs::remove_file(path);
    }
}
fn attach_predicate(executable: &std::path::Path, payload: &str) -> Result<String> {
    // if-shell inherits the server's environment, which may predate this
    // viewer's HOME/XDG settings. Pin only this predicate's state location.
    let root = state()?.canonicalize()?;
    let parent = root.parent().context("state parent unavailable")?;
    let format_quote = |value: &str| quote(value).replace('#', "##");
    Ok(format!(
        "XDG_STATE_HOME={} {} native-attach-check {} '#{{pid}}' '#{{session_id}}' '#{{session_created}}'",
        format_quote(parent.to_str().context("state path is not UTF-8")?),
        format_quote(executable.to_str().context("helper path is not UTF-8")?),
        quote(payload)
    ))
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
    let ascii = request.ascii || viewer_ascii();
    if !request.session.external && !control::ticket_valid(&request.session, ascii) {
        if !control::configured(expected.server_pid, &expected.server_start) {
            configure_managed()?;
        }
        refresh_managed_status_as(&request.session.id, ascii)?;
    }
    if !request.session.external {
        // A focus attach lands live rather than in leftover copy-mode; a
        // read-only watch keeps the scrollback view. Best effort.
        if !request.observe {
            exit_copy_mode(&request.session.id);
        }
    }
    let executable = crate::self_exe()?;
    let predicate = attach_predicate(&executable, &predicate_payload)?;
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
/// Scrolling the wheel puts the pane in tmux copy-mode, which persists across
/// detach. Re-attaching then lands in that read-only scrollback view, which
/// swallows input until it is cancelled — so a focus attach looks like it opened
/// read-only until a key dismisses the mode. Cancel it first so focus lands live.
/// Read-only watches keep the mode. A no-op when the pane is not in a mode or the
/// id is not a managed session; failures never block the attach.
pub fn exit_copy_mode(id: &str) {
    let id = id.strip_prefix('=').unwrap_or(id);
    if !managed_session_name(id) && !valid_pane_name(id) {
        return;
    }
    // One tmux command: the mode test runs inside the server.
    let cancel = format!("send-keys -t {id} -X cancel");
    let args = ["if-shell", "-F", "-t", id, "#{pane_in_mode}", &cancel];
    if let Some(client) = control::running() {
        let _ = client.run(&args);
    } else if let Ok(mut c) = tmux(true) {
        c.args(args);
        let _ = output(c);
    }
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
            ascii: viewer_ascii(),
        })?;
        if device.target.is_some() {
            let routed = crate::edges::interactive_ssh(device)?;
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
        if !observe {
            exit_copy_mode(&session.id);
        }
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
        let mut c = crate::edges::interactive_ssh(device)?;
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
/// A persistent managed session name: `cx-` plus 64 lowercase-hex characters.
fn managed_session_name(id: &str) -> bool {
    id.starts_with("cx-") && id.len() == 67 && id[3..].bytes().all(|b| b.is_ascii_hexdigit())
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
    let mut command = if device.target.is_some() {
        let routed = crate::edges::interactive_ssh(device)?;
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
        let routed = crate::edges::interactive_ssh(device)?;
        let mut command = Command::new("ssh");
        command.arg("-tt").args(routed.get_args());
        // Only base64's fixed alphabet enters the remote shell command.
        command.arg(format!("~/.local/bin/cx native-command '{}'", payload));
        command
    } else {
        let mut command = Command::new(crate::self_exe()?);
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

    /// A running server is re-sourced on update and attach: that config must carry no
    /// setting a v0.2.2 server lacks (compat_021 compares options and keys). Passthrough
    /// for image previews is a start-only setting.
    #[test]
    fn passthrough_is_set_only_when_a_server_starts() {
        let sourced = managed_config();
        assert!(!sourced.contains("allow-passthrough"));
        let started = start_config(sourced.clone());
        assert!(started.starts_with(&sourced));
        assert_eq!(
            started.contains("set -g allow-passthrough on"),
            tmux_at_least(3, 3)
        );
    }

    #[test]
    fn container_launcher_preserves_native_history_picker_intent() {
        use base64::Engine;
        let scope = crate::model::ContainerScope {
            engine: "fixture".into(),
            id: "a".repeat(64),
            name: "fixture".into(),
            user: "tester".into(),
            started_at: "now".into(),
            folder: "/work".into(),
        };
        let request = CreateSession {
            key: "picker".into(),
            directory: scope.folder.clone(),
            provider: "codex".into(),
            name: String::new(),
            resume: true,
            resume_pick: true,
        };
        let value: serde_json::Value = serde_json::from_slice(
            &base64::engine::general_purpose::STANDARD
                .decode(container_run_payload(&scope, &request, true).unwrap())
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            value[3], true,
            "container-run must receive the selected native history picker intent"
        );
        assert_eq!(value[2], true);
    }

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
    else printf '$1\t%s\t100\t%%1\t%s\t%s\t1\t%s\t1700000000\n' "$pane" "$dead" "$server" "$panes"; fi;;
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
        printf '$%s\t%s\t100\t%%1\t%s\t%s\t1\t%s\t1700000000\n' "$selected" "$pane" "$dead" "$server" "$panes";;
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
        if let Ok(payload) = std::env::var("CX_CHECK_PAYLOAD") {
            native_attach_check(
                &payload,
                std::env::var("CX_CHECK_PID").unwrap().parse().unwrap(),
                &std::env::var("CX_CHECK_SESSION").unwrap(),
                &std::env::var("CX_CHECK_CREATED").unwrap(),
            )
            .unwrap();
            return;
        }
        let Ok(case) = std::env::var("CX_INVENTORY_CASE") else {
            return;
        };
        std::env::set_var("CX_INVENTORY_PID", std::process::id().to_string());
        let sessions = list().unwrap();
        match case.as_str() {
            "intent_conflict" => {
                let mut prior = sessions[0].clone();
                let key = "intent-fixture";
                prior.id = format!("cx-{:x}", Sha256::digest(key.as_bytes()));
                prior.directory = std::env::var("HOME").unwrap();
                prior.provider = "shell".into();
                fs::write(
                    state().unwrap().join(format!("{}.json", prior.id)),
                    serde_json::to_vec(&prior).unwrap(),
                )
                .unwrap();
                let request = CreateSession {
                    key: key.into(),
                    directory: prior.directory,
                    provider: "codex".into(),
                    name: String::new(),
                    resume: false,
                    resume_pick: false,
                };
                assert!(
                    create(&request).is_err(),
                    "a creation key for a shell must not silently satisfy an agent launch"
                );
            }
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
                    ascii: false,
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
                // The owner keeps the verdict through its descriptor; the name is gone
                // at once, so an owner that dies mid-attach leaves nothing behind.
                assert!(
                    !guard_path.exists(),
                    "a passed guard must not outlive its check"
                );
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

                let home = PathBuf::from(std::env::var_os("HOME").unwrap());
                let shim = home.join("check-shim");
                fs::write(
                    &shim,
                    r#"#!/bin/sh
export CX_CHECK_PAYLOAD="$2" CX_CHECK_PID="$3" CX_CHECK_SESSION="$4" CX_CHECK_CREATED="$5"
exec "$CX_CHECK_EXEC" --exact sessions::tests::runtime_inventory_fixture_child --nocapture
"#,
                )
                .unwrap();
                fs::set_permissions(&shim, fs::Permissions::from_mode(0o700)).unwrap();
                let mut guard = AttachGuard::new().unwrap();
                let payload = attach_payload(&CheckedAttach {
                    session: session.clone(),
                    observe: false,
                    guard: Some(guard.token.clone()),
                    ascii: false,
                })
                .unwrap();
                let predicate = attach_predicate(&shim, &payload)
                    .unwrap()
                    .replace("#{pid}", &expected.server_pid.to_string())
                    .replace("#{session_id}", &expected.session_id)
                    .replace("#{session_created}", &expected.created);
                let result = Command::new("/bin/sh")
                    .args(["-c", &predicate])
                    .env("CX_CHECK_EXEC", std::env::current_exe().unwrap())
                    .env("HOME", home.join("old-server-home"))
                    .env("XDG_STATE_HOME", home.join("old-server-state"))
                    .output()
                    .unwrap();
                assert!(
                    result.status.success(),
                    "guard predicate must use the initiating helper's state root: {}",
                    String::from_utf8_lossy(&result.stderr)
                );
                assert!(guard.passed().unwrap());
                drop(guard);

                let guard = AttachGuard::new().unwrap();
                let payload = attach_payload(&CheckedAttach {
                    session: session.clone(),
                    observe: true,
                    guard: Some(guard.token.clone()),
                    ascii: false,
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
                        guard: None,
                        ascii: false,
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
    fn creation_key_rejects_changed_provider_intent() {
        inventory_fixture("intent_conflict");
    }

    #[test]
    fn creation_key_binds_history_mode_directory_scope_and_permissions() {
        let fixture = tempfile::tempdir().unwrap();
        let original = LaunchIntent {
            provider: "codex".into(),
            resume: false,
            resume_pick: false,
            yolo: false,
            directory: "/work".into(),
            container: None,
        };
        fs::write(
            fixture.path().join("fixture.launch"),
            serde_json::to_vec(&original).unwrap(),
        )
        .unwrap();
        assert!(validate_launch_intent(fixture.path(), "fixture", &original).is_ok());
        for field in [
            "provider",
            "resume",
            "resume_pick",
            "yolo",
            "directory",
            "container",
        ] {
            let mut value = serde_json::to_value(&original).unwrap();
            value[field] = match field {
                "provider" => serde_json::json!("claude"),
                "directory" => serde_json::json!("/elsewhere"),
                "container" => {
                    serde_json::json!({"engine":"fixture","id":"a".repeat(64),"name":"fixture","user":"tester","started_at":"now","folder":"/work"})
                }
                _ => serde_json::json!(true),
            };
            let changed = serde_json::from_value(value).unwrap();
            assert!(
                validate_launch_intent(fixture.path(), "fixture", &changed).is_err(),
                "{field}"
            );
        }
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
    fn checked_attach_carries_the_viewers_ascii_switch_only_when_set() {
        let session: Session = serde_json::from_value(serde_json::json!({
            "id": "cx-a", "name": "shell", "directory": "/", "provider": "shell",
            "host": "h", "account": "a", "pid": 1, "started": "", "boot_id": "b",
            "external": false, "socket": null
        }))
        .unwrap();
        let request = |ascii| CheckedAttach {
            session: session.clone(),
            observe: false,
            guard: None,
            ascii,
        };
        // Older helpers deny unknown fields: an ordinary payload omits it.
        let plain = serde_json::to_value(request(false)).unwrap();
        assert!(plain.get("ascii").is_none());
        let ascii = serde_json::to_vec(&request(true)).unwrap();
        let parsed: CheckedAttach = serde_json::from_slice(&ascii).unwrap();
        assert!(parsed.ascii);
        let parsed: CheckedAttach = serde_json::from_value(plain).unwrap();
        assert!(!parsed.ascii);
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
    fn startup_sweep_removes_only_stale_unowned_attach_leftovers() {
        crate::store::isolate_test_state();
        let dir = state().unwrap();
        let old = std::time::SystemTime::now() - Duration::from_secs(2 * 3600);
        let make = |name: &str, aged: bool| {
            let path = dir.join(name);
            fs::write(&path, b"ok").unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
            if aged {
                fs::File::options()
                    .write(true)
                    .open(&path)
                    .unwrap()
                    .set_modified(old)
                    .unwrap();
            }
            path
        };
        let stale = make(&format!("attach-guard-{}", "a".repeat(64)), true);
        let ticket = make("attach-ticket-0123abcd", true);
        let fresh = make(&format!("attach-guard-{}", "b".repeat(64)), false);
        let held = make(&format!("attach-guard-{}", "c".repeat(64)), true);
        let _owner = fs::File::open(&held).unwrap();
        let current = make("attach-ticket", true);
        let other = make("attach-guard-notes", true);
        let unrelated = make("devices.json", true);
        sweep_attach_leftovers();
        assert!(!stale.exists() && !ticket.exists(), "stale leftovers stay");
        for kept in [&fresh, &held, &current, &other, &unrelated] {
            assert!(kept.exists(), "{} was removed", kept.display());
        }
    }

    #[test]
    fn terminal_generation_rejects_replaced_or_exited_server() {
        let mut observed = parse_snapshot("$1\t10\t100\t%2\t0\t42\t1\t1\t90")
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
        let valid = "$1\t10\t100\t%2\t0\t42\t1\t1\t90";
        assert_eq!(parse_snapshot(valid).unwrap().len(), 1);
        for invalid in [
            "$1\t10\t100\t%2\t0\t42\t1\t1\t90\textra",
            "$1\t10\t100\t%2\t0\t42\t1\t1",
            "$name\t10\t100\t%2\t0\t42\t1\t1\t90",
            "$1\t10\t100\t%2\t2\t42\t1\t1\t90",
            "$1\t10\t100\t%2\t0\t0\t1\t1\t90",
            "$1\t10\t100\t%2\t0\t42\t0\t1\t90",
            "$1\t10\t100\t%2\t0\t42\t1\t1\t90\nforged",
        ] {
            assert!(parse_snapshot(invalid).is_err());
        }
    }

    #[test]
    fn runtime_snapshot_activity_is_optional_evidence() {
        let row = |activity: &str| {
            parse_snapshot(&format!("$1\t10\t100\t%2\t0\t42\t1\t1\t{activity}"))
                .unwrap()
                .remove(0)
                .activity
        };
        assert_eq!(row("1700000000"), Some(1_700_000_000));
        // Unknown or malformed activity loses its age, never the session.
        for unknown in ["", "0", "-5", "12x", "99999999999999999999999"] {
            assert_eq!(row(unknown), None, "{unknown:?}");
        }
        assert_eq!(idle_seconds(None), None);
        assert_eq!(
            idle_seconds(Some(u64::MAX)),
            Some(0),
            "future stamps are now"
        );
        assert!(idle_seconds(Some(1)).unwrap() > 1_000_000);
    }

    #[test]
    fn runtime_snapshot_accepts_cleared_pid_only_for_dead_pane() {
        let dead = parse_snapshot("$1\t\t100\t%2\t1\t42\t1\t1\t")
            .unwrap()
            .remove(0);
        assert_eq!(dead.pane_pid, 0);
        assert!(dead.ended());
        let expanded = parse_snapshot("$1\t\t100\t%2\t1\t42\t1\t2\t90")
            .unwrap()
            .remove(0);
        assert!(!expanded.ended());
        assert!(parse_snapshot("$1\t\t100\t%2\t0\t42\t1\t1\t90").is_err());
    }

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
    fn only_a_loaded_stopped_tmux_unit_is_restarted() {
        assert!(unit_is_stale("LoadState=loaded\nActiveState=inactive\n"));
        assert!(unit_is_stale("ActiveState=failed\nLoadState=loaded\n"));
        assert!(!unit_is_stale("LoadState=loaded\nActiveState=active\n"));
        assert!(!unit_is_stale("LoadState=loaded\nActiveState=activating\n"));
        assert!(!unit_is_stale(
            "LoadState=not-found\nActiveState=inactive\n"
        ));
        assert!(!unit_is_stale(""));
    }

    #[test]
    fn systemd_run_errors_are_summarized_and_bounded() {
        assert_eq!(
            bounded_stderr(b"\nFailed to start transient service unit: Unit cx-tmux-1.service was already loaded or has a fragment file.\nmore\n"),
            "Failed to start transient service unit: Unit cx-tmux-1.service was already loaded or has a fragment file."
        );
        assert_eq!(bounded_stderr(&[b'x'; 1000]).len(), 200);
        assert_eq!(bounded_stderr(b"a\x1b[31mb"), "a[31mb");
        assert_eq!(bounded_stderr(b""), "");
    }

    #[test]
    fn provider_commands_distinguish_fresh_and_native_resume() {
        assert_eq!(
            provider_command("codex", false, false, false).unwrap(),
            "codex"
        );
        assert_eq!(
            provider_command("codex", false, true, false).unwrap(),
            "codex resume --last"
        );
        assert_eq!(
            provider_command("codex", false, true, true).unwrap(),
            "codex resume"
        );
        assert_eq!(
            provider_command("codex", true, true, false).unwrap(),
            "codex resume --last --dangerously-bypass-approvals-and-sandbox"
        );
        assert_eq!(
            provider_command("claude", false, false, false).unwrap(),
            "claude"
        );
        assert_eq!(
            provider_command("claude", false, true, false).unwrap(),
            "claude --continue"
        );
        assert_eq!(
            provider_command("claude", false, true, true).unwrap(),
            "claude --resume"
        );
        assert_eq!(
            provider_command("claude", true, true, true).unwrap(),
            "claude --resume --dangerously-skip-permissions"
        );
        assert!(provider_command("shell", false, true, false).is_err());
        // A picker only applies when resuming.
        assert!(provider_command("claude", false, false, true).is_err());
        // YOLO never applies to a shell, fresh or not.
        assert_eq!(
            provider_command("shell", false, false, false).unwrap(),
            "shell"
        );
        assert!(provider_command("shell", true, false, false).is_err());
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
            "env",
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
            automatic_provider_name("codex", Some(&named), true, None, fallback.clone()),
            "Current task"
        );
        let unselected = crate::session_titles::NativeMetadata {
            current_thread_unknown: true,
            ..named
        };
        // With no live terminal title, an unknown selection falls back to the
        // provider name rather than reviving a stale workspace label.
        assert_eq!(
            automatic_provider_name("codex", Some(&unselected), true, None, fallback.clone()),
            "codex"
        );
        // The provider's own terminal title (the switched thread's name) relabels
        // the session even when CX cannot pin the thread's directory.
        assert_eq!(
            automatic_provider_name(
                "codex",
                Some(&unselected),
                true,
                Some("project-nebula"),
                fallback.clone()
            ),
            "project-nebula"
        );
        assert_eq!(
            automatic_provider_name("codex", None, true, None, fallback.clone()),
            "codex"
        );
        assert_eq!(
            automatic_provider_name("codex", None, false, None, fallback.clone()),
            fallback
        );
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
            activity: None,
            agent_tasks: vec![],
            idle_seconds: None,
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
    fn managed_detach_is_ctrl_backslash_with_a_tappable_chip() {
        let config = include_str!("../assets/tmux.conf");
        let lines: Vec<_> = config.lines().collect();
        assert!(lines.contains(&"bind-key -n 'C-\\' detach-client"));
        assert!(lines.contains(&"bind-key -n MouseDown1StatusLeft detach-client"));
        // A running server that sourced an older config releases Ctrl+] too.
        assert!(lines.contains(&"unbind-key -n C-]"));
        assert!(!lines
            .iter()
            .any(|l| l.contains("C-]") && l.contains("detach")));
        assert!(config.contains("⏏ detach") && config.contains("[detach]"));
        let legacy = without_status_mouse(config);
        assert!(!legacy.contains("MouseDown1StatusLeft") && !legacy.contains("⏏"));
        assert!(legacy.contains("detach: ") && legacy.contains("'C-\\' detach-client"));
        assert!(!legacy
            .lines()
            .any(|l| !l.starts_with('#') && l.contains("@cx_title")));
        // The clickable status-left is the chip alone: the title and the
        // copy-mode hint live in the window cell, so tapping them never detaches.
        let left = lines
            .iter()
            .find(|l| l.starts_with("set -g status-left '"))
            .unwrap();
        assert_eq!(
            *left,
            "set -g status-left '#[reverse]#{?@cx_ascii, [detach] , ⏏ detach }#[noreverse] '"
        );
        assert!(lines.contains(
            &"set -g window-status-current-format '#{?pane_in_mode,Scroll: y copy / Esc,cx#{?@cx_ascii, | , · }#{@cx_title}}'"
        ));
        // Legacy (tmux < 3.0) status text is ASCII for every viewer.
        let legacy_left = legacy
            .lines()
            .find(|l| l.starts_with("set -g status-left '"))
            .unwrap();
        assert!(legacy_left.is_ascii());
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
