//! Native structured metadata: never read transcripts or infer identity from cwd/recency.
//! Claude exposes a PID/start-tick record; Codex uses exact open writer IDs.
//! Metadata is optional and version-sensitive. Unknown association returns `None`.
use serde_json::Value;
use std::collections::BTreeSet;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const MAX_RECORD: usize = 64 * 1024;
const INDEX_TAIL: u64 = 8 * 1024 * 1024;
const MAX_TITLE: usize = 96;

/// Provider-authored structured metadata, associated with one exact live process.
#[derive(Clone, Debug, Default)]
pub struct NativeMetadata {
    pub title: Option<String>,
    pub directory: Option<String>,
    pub activity: Option<String>,
    pub agents: Vec<NativeAgentTask>,
}

#[derive(Clone, Debug)]
pub struct NativeAgentTask {
    pub id: String,
    pub name: String,
    pub task: Option<String>,
    pub status: String,
}

pub fn native_title(provider: &str, pid: u32) -> Option<String> {
    native_metadata(provider, pid)?.title
}

pub fn native_directory(provider: &str, pid: u32) -> Option<String> {
    native_metadata(provider, pid)?.directory
}

pub fn native_metadata(provider: &str, pid: u32) -> Option<NativeMetadata> {
    #[cfg(target_os = "linux")]
    {
        let start = process_start(pid)?;
        let result = match provider {
            "claude" => claude_metadata(pid, start),
            "codex" => codex_metadata(pid),
            _ => None,
        };
        // Discard a result if the PID disappeared or was reused while reading metadata.
        if process_start(pid)? == start {
            result
        } else {
            None
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (provider, pid);
        None
    }
}

fn title(value: &str) -> Option<String> {
    // Remove control sequences, including their printable payload, rather than
    // stripping ESC alone and exposing misleading fragments in the workspace.
    let mut out = String::new();
    let mut chars = value.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            match chars.next() {
                Some('[') => {
                    for c in chars.by_ref() {
                        if ('@'..='~').contains(&c) {
                            break;
                        }
                    }
                }
                Some(']') | Some('P') | Some('_') | Some('^') => {
                    while let Some(c) = chars.next() {
                        if c == '\x07' || (c == '\x1b' && chars.peek() == Some(&'\\')) {
                            if c == '\x1b' {
                                chars.next();
                            }
                            break;
                        }
                    }
                }
                _ => {}
            }
        } else if c.is_control() || matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}') {
            if c.is_whitespace() {
                out.push(' ');
            }
        } else {
            out.push(c);
        }
    }
    let compact = out.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.is_empty() {
        return None;
    }
    let mut visible: String = compact.chars().take(MAX_TITLE).collect();
    if compact.chars().count() > MAX_TITLE {
        visible.push('…');
    }
    Some(visible)
}

#[cfg(target_os = "linux")]
fn process_start(pid: u32) -> Option<u64> {
    use std::os::unix::fs::MetadataExt;
    let path = PathBuf::from(format!("/proc/{pid}"));
    if fs::metadata(&path).ok()?.uid() != unsafe { libc::geteuid() } {
        return None;
    }
    let raw = fs::read_to_string(path.join("stat")).ok()?;
    start_ticks(&raw)
}

fn start_ticks(raw: &str) -> Option<u64> {
    // comm may contain spaces or ')'; split after its final closing parenthesis.
    raw.get(raw.rfind(')')? + 2..)?
        .split_whitespace()
        .nth(19)?
        .parse()
        .ok()
}

#[cfg(target_os = "linux")]
fn private_file(path: &Path) -> Option<File> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .ok()?;
    let meta = file.metadata().ok()?;
    if !meta.is_file() || meta.uid() != unsafe { libc::geteuid() } {
        return None;
    }
    Some(file)
}

#[cfg(target_os = "linux")]
fn claude_metadata(pid: u32, start: u64) -> Option<NativeMetadata> {
    let home = PathBuf::from(std::env::var_os("HOME")?);
    let record = read_record(&home.join(".claude/sessions").join(format!("{pid}.json")))?;
    if let Some(domain) = record.get("pidDomain") {
        let machine = fs::read_to_string("/etc/machine-id").ok()?;
        let namespace = fs::read_link(format!("/proc/{pid}/ns/pid")).ok()?;
        let expected = format!("linux:{}:{}", machine.trim(), namespace.to_str()?);
        if domain.as_str()? != expected {
            return None;
        }
    }
    let mut metadata = claude_record_metadata(&record, pid, start)?;
    let id = record.get("sessionId")?.as_str()?;
    metadata.agents = claude_tasks(&home.join(".claude/tasks").join(id));
    Some(metadata)
}

#[cfg(target_os = "linux")]
fn read_record(path: &Path) -> Option<Value> {
    let file = private_file(path)?;
    let mut bytes = Vec::new();
    file.take(MAX_RECORD as u64 + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() > MAX_RECORD {
        return None;
    }
    serde_json::from_slice(&bytes).ok()
}

fn directory(value: Option<&Value>) -> Option<String> {
    let s = value?.as_str()?;
    // Paths must remain exact, not be transformed by display sanitization.
    (s.len() <= 4096 && Path::new(s).is_absolute() && !s.chars().any(char::is_control))
        .then(|| s.to_owned())
}

fn claude_record_metadata(record: &Value, pid: u32, start: u64) -> Option<NativeMetadata> {
    if record.get("pid")?.as_u64()? != u64::from(pid)
        || record
            .get("procStart")
            .and_then(|v| v.as_u64().or_else(|| v.as_str()?.parse().ok()))?
            != start
        || !uuid(record.get("sessionId")?.as_str()?)
    {
        return None;
    }
    Some(NativeMetadata {
        title: record.get("name").and_then(Value::as_str).and_then(title),
        directory: directory(record.get("cwd")),
        activity: record
            .get("status")
            .and_then(Value::as_str)
            .filter(|s| {
                matches!(
                    *s,
                    "idle" | "working" | "running" | "busy" | "waiting" | "waiting_for_input"
                )
            })
            .map(str::to_owned),
        agents: Vec::new(),
    })
}

#[cfg(target_os = "linux")]
fn claude_tasks(path: &Path) -> Vec<NativeAgentTask> {
    let deadline = Instant::now() + Duration::from_millis(30);
    let Ok(entries) = fs::read_dir(path) else {
        return Vec::new();
    };
    let mut tasks = Vec::new();
    for (i, entry) in entries.enumerate() {
        if i >= 128 || Instant::now() >= deadline {
            break;
        }
        let Ok(entry) = entry else {
            continue;
        };
        // Task files are numbered; never follow metadata pointers or transcript paths.
        let name = entry.file_name();
        let Some(stem) = name.to_str().and_then(|s| s.strip_suffix(".json")) else {
            continue;
        };
        if stem.is_empty() || !stem.bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        if let Some(task) = read_record(&entry.path()).and_then(|v| claude_task(&v, stem)) {
            tasks.push(task);
        }
    }
    tasks.sort_by(|a, b| a.id.cmp(&b.id));
    tasks
}

fn claude_task(record: &Value, expected_id: &str) -> Option<NativeAgentTask> {
    if record.get("id")?.as_str()? != expected_id {
        return None;
    }
    let status = record.get("status")?.as_str()?;
    if !matches!(status, "pending" | "in_progress" | "completed") {
        return None;
    }
    let task = record
        .get("subject")
        .and_then(Value::as_str)
        .and_then(title)?;
    // An unassigned task remains a task, rather than inventing an agent identity.
    let name = record
        .get("owner")
        .and_then(Value::as_str)
        .and_then(title)
        .unwrap_or_else(|| format!("Task {expected_id}"));
    Some(NativeAgentTask {
        id: expected_id.to_owned(),
        name,
        task: Some(task),
        status: status.to_owned(),
    })
}

#[cfg(test)]
fn claude_record_title(record: &Value, pid: u32, start: u64) -> Option<String> {
    claude_record_metadata(record, pid, start)?.title
}

fn uuid(s: &str) -> bool {
    s.len() == 36
        && s.bytes().enumerate().all(|(i, b)| {
            if matches!(i, 8 | 13 | 18 | 23) {
                b == b'-'
            } else {
                b.is_ascii_hexdigit()
            }
        })
}

/// Collect exact owned writer IDs. Multiple writers require a separate proven
/// ancestry root; independent daemon roots remain deliberately ambiguous.
fn codex_identities(
    paths: impl IntoIterator<Item = PathBuf>,
) -> Option<(PathBuf, BTreeSet<String>)> {
    let mut ids = BTreeSet::new();
    for path in paths {
        let parent = path.parent()?;
        let file = path.file_name()?.to_str()?;
        if parent.file_name().and_then(|s| s.to_str()) == Some("thread-writer-locks") {
            if let Some(id) = file.strip_suffix(".lock").filter(|s| uuid(s)) {
                ids.insert((parent.parent()?.to_path_buf(), id.to_ascii_lowercase()));
            }
        } else if file.starts_with("rollout-") && file.ends_with(".jsonl") {
            let suffix = file.strip_suffix(".jsonl")?;
            let Some(id) = suffix
                .get(suffix.len().saturating_sub(36)..)
                .filter(|s| uuid(s))
            else {
                continue;
            };
            // Supported rollout layout: HOME/sessions/YYYY/MM/DD/rollout-...jsonl.
            let mut root = parent;
            for _ in 0..3 {
                root = root.parent()?;
            }
            if root.file_name().and_then(|s| s.to_str()) == Some("sessions") {
                ids.insert((root.parent()?.to_path_buf(), id.to_ascii_lowercase()));
            }
        }
    }
    if ids.is_empty() || ids.len() > 64 {
        return None;
    }
    let home = ids.iter().next()?.0.clone();
    if ids.iter().any(|(candidate, _)| *candidate != home) {
        return None;
    }
    Some((home, ids.into_iter().map(|(_, id)| id).collect()))
}

#[cfg(test)]
fn codex_identity(paths: impl IntoIterator<Item = PathBuf>) -> Option<(PathBuf, String)> {
    let (home, ids) = codex_identities(paths)?;
    if ids.len() == 1 {
        Some((home, ids.into_iter().next()?))
    } else {
        None
    }
}

#[cfg(target_os = "linux")]
fn codex_metadata(pid: u32) -> Option<NativeMetadata> {
    let deadline = Instant::now() + Duration::from_millis(100);
    let mut paths = Vec::new();
    for (count, entry) in fs::read_dir(format!("/proc/{pid}/fd")).ok()?.enumerate() {
        if count >= 256 {
            return None;
        }
        if Instant::now() >= deadline {
            return None;
        }
        // An incomplete scan cannot prove there is only one writer identity.
        let entry = entry.ok()?;
        let path = fs::read_link(entry.path()).ok()?;
        let mut bytes = Vec::new();
        File::open(format!(
            "/proc/{pid}/fdinfo/{}",
            entry.file_name().to_str()?
        ))
        .ok()?
        .take(4096)
        .read_to_end(&mut bytes)
        .ok()?;
        let info = std::str::from_utf8(&bytes).ok()?;
        let flags = info
            .lines()
            .find_map(|line| line.strip_prefix("flags:\t"))?;
        let flags = u32::from_str_radix(flags.trim(), 8).ok()?;
        // Historical read handles are not an active writer association.
        if flags & libc::O_ACCMODE as u32 != libc::O_RDONLY as u32 {
            paths.push(path);
        }
    }
    let (home, ids) = codex_identities(paths)?;
    let id = if ids.len() == 1 {
        ids.iter().next()?.clone()
    } else {
        codex_root(&home, &ids)?
    };
    // Read the rename index first, with its own budget, so a slow optional
    // database query cannot suppress a fresh /rename update.
    let native_name = private_file(&home.join("session_index.jsonl"))
        .and_then(|file| index_file_title(file, &id, Instant::now() + Duration::from_millis(100)));
    let mut metadata = codex_database(&home, &id).unwrap_or_default();
    if let Some(name) = native_name {
        metadata.title = Some(name);
    }

    Some(metadata)
}

fn index_file_title(mut file: File, id: &str, deadline: Instant) -> Option<String> {
    let size = file.metadata().ok()?.len();
    let offset = size.saturating_sub(INDEX_TAIL);
    file.seek(SeekFrom::Start(offset)).ok()?;
    let mut reader = BufReader::new(file.take(INDEX_TAIL));
    if offset > 0 {
        let mut skip = Vec::new();
        reader
            .by_ref()
            .take(MAX_RECORD as u64 + 1)
            .read_until(b'\n', &mut skip)
            .ok()?;
        if skip.len() > MAX_RECORD {
            return None;
        }
    }
    index_title(reader, id, deadline)
}

/// Optional compatibility path for Codex 0.163 state_5 schema. Only select
/// metadata columns: `title`, `preview`, and `first_user_message` are excluded.
#[cfg(target_os = "linux")]
fn codex_database(home: &Path, id: &str) -> Option<NativeMetadata> {
    if !uuid(id) {
        return None;
    }
    let query = format!(
        "WITH RECURSIVE family(id,depth) AS (\
         SELECT child_thread_id,1 FROM thread_spawn_edges WHERE parent_thread_id='{id}' \
         UNION SELECT e.child_thread_id,f.depth+1 FROM thread_spawn_edges e \
         JOIN family f ON e.parent_thread_id=f.id WHERE f.depth<8 LIMIT 128) \
         SELECT json_object('id',id,'name',name,'cwd',cwd,'parent',1) FROM threads WHERE id='{id}' \
         UNION ALL SELECT json_object('id',t.id,'name',t.name,'nickname',t.agent_nickname,\
         'role',t.agent_role,'status',e.status,'parent',0) FROM threads t \
         JOIN family f ON f.id=t.id JOIN thread_spawn_edges e ON e.child_thread_id=t.id \
         GROUP BY t.id LIMIT 129;"
    );
    let output = sqlite_metadata(home, &query)?;
    codex_database_records(&output, id)
}

/// Find exactly one owned writer that is a proven ancestor of every other
/// owned writer. Independent TUIs sharing a daemon have unrelated roots and
/// remain deliberately unassociated. Traversal truncation fails closed.
#[cfg(target_os = "linux")]
fn codex_root(home: &Path, owned: &BTreeSet<String>) -> Option<String> {
    if owned.len() < 2 || owned.len() > 64 || owned.iter().any(|id| !uuid(id)) {
        return None;
    }
    let seeds = owned
        .iter()
        .map(|id| format!("('{id}','{id}',0)"))
        .collect::<Vec<_>>()
        .join(",");
    let query = format!(
        "WITH RECURSIVE ancestry(root,id,depth) AS (VALUES {seeds} \
        UNION SELECT a.root,e.child_thread_id,a.depth+1 FROM ancestry a \
        JOIN thread_spawn_edges e ON e.parent_thread_id=a.id WHERE a.depth<8 LIMIT 8193) \
        SELECT json_object('root',root,'id',id) FROM ancestry LIMIT 8193;"
    );
    let output = sqlite_metadata(home, &query)?;
    codex_root_records(&output, owned)
}

fn codex_root_records(output: &[u8], owned: &BTreeSet<String>) -> Option<String> {
    let mut descendants = std::collections::BTreeMap::<String, BTreeSet<String>>::new();
    for (count, line) in output
        .split(|b| *b == b'\n')
        .filter(|l| !l.is_empty())
        .enumerate()
    {
        if count >= 8192 || line.len() > 1024 {
            return None;
        }
        let record: Value = serde_json::from_slice(line).ok()?;
        let root = record.get("root")?.as_str()?;
        let id = record.get("id")?.as_str()?;
        if !owned.contains(root) || !uuid(id) {
            return None;
        }
        descendants
            .entry(root.to_owned())
            .or_default()
            .insert(id.to_owned());
    }
    let roots: Vec<_> = owned
        .iter()
        .filter(|id| descendants.get(*id).is_some_and(|set| owned.is_subset(set)))
        .collect();
    if roots.len() == 1 {
        Some(roots[0].clone())
    } else {
        None
    }
}

#[cfg(target_os = "linux")]
fn sqlite_metadata(home: &Path, query: &str) -> Option<Vec<u8>> {
    use std::os::fd::AsRawFd;
    let path = home.join("state_5.sqlite");
    let _file = private_file(&path)?;
    let mut child = sqlite_command(&path, query).spawn().ok()?;
    let mut stdout = child.stdout.take()?;
    let fd = stdout.as_raw_fd();
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        let _ = child.kill();
        let _ = child.wait();
        return None;
    }
    let deadline = Instant::now() + Duration::from_millis(150);
    let mut output = Vec::new();
    let success = loop {
        let mut buf = [0; 4096];
        match stdout.read(&mut buf) {
            Ok(0) => match child.try_wait() {
                Ok(Some(status)) => break status.success(),
                Ok(None) => {}
                Err(_) => break false,
            },
            Ok(n) => {
                output.extend_from_slice(&buf[..n]);
                if output.len() > 256 * 1024 {
                    break false;
                }
            }
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(_) => break false,
        }
        if Instant::now() >= deadline {
            break false;
        }
        std::thread::sleep(Duration::from_millis(2));
    };
    if !success {
        let _ = child.kill();
    }
    let _ = child.wait();
    if !success {
        return None;
    }
    Some(output)
}

#[cfg(target_os = "linux")]
fn sqlite_command(path: &Path, query: &str) -> std::process::Command {
    use std::process::{Command, Stdio};
    let mut command = Command::new("sqlite3");
    command
        .args(["-init", "/dev/null", "-readonly", "-batch", "-noheader"])
        .arg(path)
        .arg(query)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    command
}

fn codex_database_records(output: &[u8], id: &str) -> Option<NativeMetadata> {
    let mut metadata = NativeMetadata::default();
    let mut found = false;
    let mut seen = BTreeSet::new();
    for line in output
        .split(|b| *b == b'\n')
        .filter(|line| !line.is_empty())
        .take(129)
    {
        if line.len() > MAX_RECORD {
            return None;
        }
        let record: Value = serde_json::from_slice(line).ok()?;
        let record_id = record.get("id")?.as_str()?;
        if !uuid(record_id) {
            return None;
        }
        if record.get("parent")?.as_u64()? == 1 {
            if record_id != id || found {
                return None;
            }
            found = true;
            metadata.title = record.get("name").and_then(Value::as_str).and_then(title);
            metadata.directory = directory(record.get("cwd"));
        } else if record_id != id && seen.insert(record_id.to_owned()) {
            let name = record
                .get("nickname")
                .and_then(Value::as_str)
                .and_then(title)
                .or_else(|| record.get("name").and_then(Value::as_str).and_then(title))
                .unwrap_or_else(|| format!("Agent {}", &record_id[..8]));
            let task = record
                .get("name")
                .and_then(Value::as_str)
                .and_then(title)
                .or_else(|| record.get("role").and_then(Value::as_str).and_then(title));
            let status = record
                .get("status")
                .and_then(Value::as_str)
                .filter(|s| matches!(*s, "open" | "closed"))
                .unwrap_or("unknown");
            metadata.agents.push(NativeAgentTask {
                id: record_id.to_owned(),
                name,
                task,
                status: status.to_owned(),
            });
        }
    }
    found.then_some(metadata)
}

fn index_title(mut reader: impl BufRead, id: &str, deadline: Instant) -> Option<String> {
    let mut result = None;
    let mut line = Vec::new();
    loop {
        if Instant::now() >= deadline {
            return None;
        }
        line.clear();
        // Bounded line read; never deserialize a rollout or arbitrary transcript.
        let n = reader
            .by_ref()
            .take(MAX_RECORD as u64 + 1)
            .read_until(b'\n', &mut line)
            .ok()?;
        if n == 0 {
            return result;
        }
        if n > MAX_RECORD {
            return None;
        }
        let Ok(record) = serde_json::from_slice::<Value>(&line) else {
            continue;
        };
        if record
            .get("id")
            .and_then(Value::as_str)
            .is_some_and(|v| v.eq_ignore_ascii_case(id))
        {
            result = record
                .get("thread_name")
                .and_then(Value::as_str)
                .and_then(title);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const ID: &str = "01a10f4e-85f4-7c00-a244-67a78928c221";
    #[test]
    fn claude_requires_same_pid_start_and_session_identity() {
        let mut record =
            serde_json::json!({"pid":7,"procStart":"42","sessionId":ID,"name":"Named task"});
        assert_eq!(
            claude_record_title(&record, 7, 42).as_deref(),
            Some("Named task")
        );
        assert!(claude_record_title(&record, 8, 42).is_none());
        assert!(claude_record_title(&record, 7, 43).is_none());
        record["sessionId"] = serde_json::json!("../../wrong");
        assert!(claude_record_title(&record, 7, 42).is_none());
    }
    #[test]
    fn terminal_controls_bidi_and_excess_are_removed() {
        assert_eq!(
            title("\x1b[31mNamed\x1b[0m\x1b]52;c;secret\x07\n task\u{202e}").as_deref(),
            Some("Named task")
        );
        assert!(title("\x1b]52;c;secret\x07").is_none());
        assert_eq!(title(&"界".repeat(150)).unwrap().chars().count(), 97);
    }
    #[test]
    fn exact_writer_identity_rejects_daemon_ambiguity() {
        let first = PathBuf::from(format!(
            "/home/example/.codex/thread-writer-locks/{ID}.lock"
        ));
        assert_eq!(codex_identity([first.clone()]).unwrap().1, ID);
        assert!(codex_identity([first, PathBuf::from("/home/example/.codex/thread-writer-locks/01a10f4e-85f4-7c00-a244-67a78928c222.lock")]).is_none());
        assert!(codex_identity([PathBuf::from("/home/example/.codex/logs_2.sqlite")]).is_none());
    }
    #[test]
    fn index_uses_exact_id_and_last_name_without_prompt_fallback() {
        let data = format!("{{\"id\":\"other\",\"thread_name\":\"wrong\"}}\n{{\"id\":\"{ID}\",\"thread_name\":\"old\"}}\n{{\"id\":\"{ID}\",\"thread_name\":\"new\"}}\n");
        assert_eq!(
            index_title(data.as_bytes(), ID, Instant::now() + Duration::from_secs(1)).as_deref(),
            Some("new")
        );
        let data = format!("{{\"id\":\"{ID}\",\"title\":\"prompt body\"}}\n");
        assert!(
            index_title(data.as_bytes(), ID, Instant::now() + Duration::from_secs(1)).is_none()
        );
    }
    #[test]
    fn index_has_time_and_record_limits() {
        assert!(index_title(b"{}\n".as_slice(), ID, Instant::now()).is_none());
        let huge = vec![b'x'; MAX_RECORD + 1];
        assert!(
            index_title(huge.as_slice(), ID, Instant::now() + Duration::from_secs(1)).is_none()
        );
    }
    #[test]
    fn claude_current_directory_and_activity_do_not_require_a_name() {
        let mut record = serde_json::json!({"pid":7,"procStart":"42","sessionId":ID,
            "cwd":"/project/current", "status":"working"});
        let metadata = claude_record_metadata(&record, 7, 42).unwrap();
        assert!(metadata.title.is_none());
        assert_eq!(metadata.directory.as_deref(), Some("/project/current"));
        assert_eq!(metadata.activity.as_deref(), Some("working"));
        record["cwd"] = serde_json::json!("relative/path");
        record["status"] = serde_json::json!("invented");
        let metadata = claude_record_metadata(&record, 7, 42).unwrap();
        assert!(metadata.directory.is_none());
        assert!(metadata.activity.is_none());
        assert!(claude_record_metadata(&record, 7, 43).is_none());
    }

    #[test]
    fn claude_tasks_use_subject_and_owner_without_description() {
        let record = serde_json::json!({"id":"7","owner":"researcher", "subject":"Investigate naming",
            "description":"PRIVATE DETAIL MUST NOT BECOME DISPLAY", "status":"in_progress"});
        let task = claude_task(&record, "7").unwrap();
        assert_eq!(task.name, "researcher");
        assert_eq!(task.task.as_deref(), Some("Investigate naming"));
        assert_eq!(task.status, "in_progress");
        assert!(claude_task(&record, "8").is_none());
        assert!(claude_task(
            &serde_json::json!({"id":"7", "description":"not a subject", "status":"pending"}),
            "7"
        )
        .is_none());
    }

    #[test]
    fn codex_native_name_cwd_and_descendants_ignore_prompt_fields() {
        let data = format!(
            "{}\n{}\n",
            serde_json::json!({"id":ID,"parent":1,
            "name":"Native renamed parent", "cwd":"/project/switched", "title":"PRIVATE PROMPT",
            "preview":"PRIVATE PROMPT"}),
            serde_json::json!({
            "id":"01a10f4e-85f4-7c00-a244-67a78928c222", "parent":0,
            "nickname":"Ada", "name":"Review protocol", "role":"reviewer", "status":"open"})
        );
        let metadata = codex_database_records(data.as_bytes(), ID).unwrap();
        assert_eq!(metadata.title.as_deref(), Some("Native renamed parent"));
        assert_eq!(metadata.directory.as_deref(), Some("/project/switched"));
        assert_eq!(metadata.agents.len(), 1);
        assert_eq!(metadata.agents[0].name, "Ada");
        assert_eq!(metadata.agents[0].task.as_deref(), Some("Review protocol"));
        assert_eq!(metadata.agents[0].status, "open");
        assert!(metadata.activity.is_none());
        let unnamed = format!(
            "{}\n",
            serde_json::json!({"id":ID,"parent":1,"title":"PRIVATE PROMPT"})
        );
        assert!(codex_database_records(unnamed.as_bytes(), ID)
            .unwrap()
            .title
            .is_none());
        assert!(
            codex_database_records(data.as_bytes(), "01a10f4e-85f4-7c00-a244-67a78928c223")
                .is_none()
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn bounded_task_reader_skips_symlinks_and_wrong_file_ids() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(
            temp.path().join("1.json"),
            serde_json::json!({"id":"1","subject":"Safe task","status":"pending"}).to_string(),
        )
        .unwrap();
        fs::write(
            temp.path().join("2.json"),
            serde_json::json!({"id":"3","subject":"Wrong task","status":"pending"}).to_string(),
        )
        .unwrap();
        std::os::unix::fs::symlink(temp.path().join("1.json"), temp.path().join("4.json")).unwrap();
        fs::write(temp.path().join("5.json"), vec![b'x'; MAX_RECORD + 1]).unwrap();
        let tasks = claude_tasks(temp.path());
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].name, "Task 1");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn codex_sqlite_exact_parent_and_descendants_end_to_end() {
        use std::process::Command;
        if Command::new("sqlite3").arg("--version").output().is_err() {
            return;
        }
        let temp = tempfile::tempdir().unwrap();
        let child = "01a10f4e-85f4-7c00-a244-67a78928c222";
        let grandchild = "01a10f4e-85f4-7c00-a244-67a78928c223";
        let other = "01a10f4e-85f4-7c00-a244-67a78928c224";
        let sql = format!("CREATE TABLE threads(id TEXT PRIMARY KEY,name TEXT,cwd TEXT,agent_nickname TEXT,agent_role TEXT); \
            CREATE TABLE thread_spawn_edges(parent_thread_id TEXT,child_thread_id TEXT,status TEXT); \
            INSERT INTO threads VALUES('{ID}','Current title','/current',NULL,NULL), \
            ('{child}',NULL,'/current','Ada','reviewer'), \
            ('{grandchild}','Nested task','/current','Grace','worker'), \
            ('{other}','Unrelated','/same-directory','Wrong agent','worker'); \
            INSERT INTO thread_spawn_edges VALUES('{ID}','{child}','open'), \
            ('{child}','{grandchild}','closed');");
        assert!(Command::new("sqlite3")
            .arg(temp.path().join("state_5.sqlite"))
            .arg(sql)
            .status()
            .unwrap()
            .success());
        let metadata = codex_database(temp.path(), ID).unwrap();
        assert_eq!(metadata.title.as_deref(), Some("Current title"));
        assert_eq!(metadata.directory.as_deref(), Some("/current"));
        assert_eq!(metadata.agents.len(), 2);
        assert!(metadata.agents.iter().all(|agent| agent.id != other));
        assert!(metadata
            .agents
            .iter()
            .any(|agent| agent.name == "Ada" && agent.task.as_deref() == Some("reviewer")));
        assert!(metadata
            .agents
            .iter()
            .any(|agent| agent.name == "Grace" && agent.status == "closed"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn sqlite_metadata_ignores_user_startup_commands() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(
            temp.path().join(".sqliterc"),
            ".print UNEXPECTED_STARTUP_OUTPUT\n",
        )
        .unwrap();
        let output = match sqlite_command(Path::new(":memory:"), "SELECT 7;")
            .env("HOME", temp.path())
            .output()
        {
            Ok(output) => output,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
            Err(error) => panic!("{error}"),
        };
        assert!(output.status.success());
        assert_eq!(String::from_utf8(output.stdout).unwrap(), "7\n");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn codex_multiple_writers_resolve_only_proven_unique_root() {
        use std::process::Command;
        let temp = tempfile::tempdir().unwrap();
        let child = "01a10f4e-85f4-7c00-a244-67a78928c222";
        let grandchild = "01a10f4e-85f4-7c00-a244-67a78928c223";
        let other = "01a10f4e-85f4-7c00-a244-67a78928c224";
        let sql = format!("CREATE TABLE thread_spawn_edges(parent_thread_id TEXT,child_thread_id TEXT,status TEXT); \
            INSERT INTO thread_spawn_edges VALUES('{ID}','{child}','open'),('{child}','{grandchild}','open');");
        let result = Command::new("sqlite3")
            .args(["-init", "/dev/null"])
            .arg(temp.path().join("state_5.sqlite"))
            .arg(sql)
            .status();
        match result {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
            result => assert!(result.unwrap().success()),
        }
        let owned = [ID, child, grandchild]
            .into_iter()
            .map(str::to_owned)
            .collect();
        assert_eq!(codex_root(temp.path(), &owned).as_deref(), Some(ID));
        let independent = [ID, other].into_iter().map(str::to_owned).collect();
        assert!(codex_root(temp.path(), &independent).is_none());
        let cycles =
            format!("INSERT INTO thread_spawn_edges VALUES('{grandchild}','{ID}','open');");
        assert!(Command::new("sqlite3")
            .args(["-init", "/dev/null"])
            .arg(temp.path().join("state_5.sqlite"))
            .arg(cycles)
            .status()
            .unwrap()
            .success());
        assert!(codex_root(temp.path(), &owned).is_none());
    }

    #[test]
    fn codex_root_traversal_fails_closed_on_bad_or_truncated_output() {
        let owned = [ID.to_owned()].into_iter().collect();
        let row = format!("{}\n", serde_json::json!({"root":ID,"id":ID}));
        assert!(codex_root_records(row.repeat(8193).as_bytes(), &owned).is_none());
        assert!(codex_root_records(b"{bad json}\n", &owned).is_none());
    }

    #[test]
    fn stat_handles_parentheses_in_comm() {
        let raw = format!("7 (worker ) name) S {} 1234 0", vec!["0"; 18].join(" "));
        assert_eq!(start_ticks(&raw), Some(1234));
    }
}
