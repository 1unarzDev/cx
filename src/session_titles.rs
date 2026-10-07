//! Native display names only: never read transcripts or infer identity from cwd/recency.
//! Claude 2.1.29x exposes a PID/start-tick record; Codex uses exact open writer IDs.
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

pub fn native_title(provider: &str, pid: u32) -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        let start = process_start(pid)?;
        let result = match provider {
            "claude" => claude_title(pid, start),
            "codex" => codex_title(pid),
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
fn claude_title(pid: u32, start: u64) -> Option<String> {
    let home = PathBuf::from(std::env::var_os("HOME")?);
    let file = private_file(&home.join(".claude/sessions").join(format!("{pid}.json")))?;
    let mut bytes = Vec::new();
    file.take(MAX_RECORD as u64 + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() > MAX_RECORD {
        return None;
    }
    let record: Value = serde_json::from_slice(&bytes).ok()?;
    if let Some(domain) = record.get("pidDomain") {
        let machine = fs::read_to_string("/etc/machine-id").ok()?;
        let namespace = fs::read_link(format!("/proc/{pid}/ns/pid")).ok()?;
        let expected = format!("linux:{}:{}", machine.trim(), namespace.to_str()?);
        if domain.as_str()? != expected {
            return None;
        }
    }
    claude_record_title(&record, pid, start)
}

fn claude_record_title(record: &Value, pid: u32, start: u64) -> Option<String> {
    if record.get("pid")?.as_u64()? != u64::from(pid)
        || record
            .get("procStart")
            .and_then(|v| v.as_u64().or_else(|| v.as_str()?.parse().ok()))?
            != start
        || !uuid(record.get("sessionId")?.as_str()?)
    {
        return None;
    }
    title(record.get("name")?.as_str()?)
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

/// Only a single exact open writer-lock or rollout ID identifies a process.
/// Daemons can own multiple threads, so ambiguous sets are intentionally rejected.
fn codex_identity(paths: impl IntoIterator<Item = PathBuf>) -> Option<(PathBuf, String)> {
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
    if ids.len() == 1 {
        ids.into_iter().next()
    } else {
        None
    }
}

#[cfg(target_os = "linux")]
fn codex_title(pid: u32) -> Option<String> {
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
    let (home, id) = codex_identity(paths)?;
    let mut file = private_file(&home.join("session_index.jsonl"))?;
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
    index_title(reader, &id, deadline)
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
    fn stat_handles_parentheses_in_comm() {
        let raw = format!("7 (worker ) name) S {} 1234 0", vec!["0"; 18].join(" "));
        assert_eq!(start_ticks(&raw), Some(1234));
    }
}
