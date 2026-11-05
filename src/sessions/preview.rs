//! Live tmux screens and explicitly focused input. Bounded read-only scrollback; no emulation or resize.
use super::*;
use std::io::Read;
use std::os::fd::AsRawFd;
use std::process::Stdio;
use std::time::Instant;

const LIVE_VIEW_FORMAT: &str = "#{pane_width} #{pane_height} #{cursor_x} #{cursor_y} #{?cursor_flag,1,0} #{?bracket_paste_flag,1,0} #{?keypad_cursor_flag,1,0} #{history_size} #{?alternate_on,1,0} #{?mouse_any_flag,1,0} #{?mouse_sgr_flag,1,0}";
fn bounded(mut command: Command) -> Result<String> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let mut pipe = child.stdout.take().context("preview pipe unavailable")?;
    let flags = unsafe { libc::fcntl(pipe.as_raw_fd(), libc::F_GETFL) };
    if flags < 0
        || unsafe { libc::fcntl(pipe.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0
    {
        let _ = child.kill();
        let _ = child.wait();
        bail!("preview pipe unavailable");
    }
    let result = (|| -> Result<String> {
        let end = Instant::now() + Duration::from_millis(750);
        let mut bytes = Vec::new();
        loop {
            anyhow::ensure!(Instant::now() < end, "preview timed out");
            let mut buf = [0u8; 4096];
            match pipe.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    anyhow::ensure!(
                        bytes.len() + n <= 256 * 1024,
                        "preview exceeds screen limit"
                    );
                    bytes.extend_from_slice(&buf[..n]);
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    // Block until output or the deadline instead of sleeping in
                    // fixed steps: keystroke echo pays this on every capture.
                    let mut fd = libc::pollfd {
                        fd: pipe.as_raw_fd(),
                        events: libc::POLLIN,
                        revents: 0,
                    };
                    let wait = end.saturating_duration_since(Instant::now()).as_millis();
                    unsafe { libc::poll(&mut fd, 1, wait.clamp(1, 750) as libc::c_int) };
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e.into()),
            }
        }
        // The pipe closed, so the child is exiting; reap it without a 5 ms step.
        let mut pause = Duration::from_micros(200);
        loop {
            if let Some(status) = child.try_wait()? {
                anyhow::ensure!(status.success(), "preview unavailable");
                return Ok(String::from_utf8_lossy(&bytes).into_owned());
            }
            anyhow::ensure!(Instant::now() < end, "preview timed out");
            thread::sleep(pause);
            pause = (pause * 2).min(Duration::from_millis(5));
        }
    })();
    if result.is_err() {
        let _ = child.kill();
        let _ = child.wait();
    }
    result
}
fn observe(managed: bool, id: &str) -> Result<SessionSnapshot> {
    let mut command = tmux(managed)?;
    command.args(["display-message", "-p", "-t", id, SNAPSHOT_FORMAT]);
    let rows = parse_snapshot(bounded(command)?.trim_end())?;
    anyhow::ensure!(rows.len() == 1 && !rows[0].dead, "preview terminal ended");
    Ok(rows.into_iter().next().unwrap())
}

pub fn preview(
    session: &Session,
    scrollback: bool,
    history_start: Option<u32>,
) -> Result<serde_json::Value> {
    // Reuse the exact checked-attachment identity validation, without its stateful guard.
    let request = CheckedAttach {
        session: session.clone(),
        observe: true,
        guard: None,
    };
    let request = parse_attach_payload(&attach_payload(&request)?)?;
    let terminal = request.session.terminal.as_ref().unwrap();
    let managed = !session.external;
    let before = observe(managed, &terminal.session_id)?;
    verify_attach_generation(&request, before.server_pid, &before.id, &before.created)?;
    let mut size = tmux(managed)?;
    size.args([
        "display-message",
        "-p",
        "-t",
        &before.pane_id,
        // Flags are written as conditionals so every field prints a digit: a
        // variable this tmux lacks (bracket_paste_flag before 3.5) reads false
        // instead of expanding to nothing and shifting the field count.
        LIVE_VIEW_FORMAT,
    ]);
    let meta = bounded(size)?;
    let dims: Vec<u32> = meta
        .split_whitespace()
        .map(str::parse)
        .collect::<std::result::Result<_, _>>()?;
    anyhow::ensure!(
        dims.len() == 11 && dims[0] > 0 && dims[0] <= 1000 && dims[1] > 0 && dims[1] <= 300,
        "terminal dimensions exceed live view limits"
    );
    // Coordinates are retained-history row numbers, not offsets from the cursor.
    // Fetch bounded pages rather than retaining an entire conversation in the UI.
    let history_total = if dims[8] == 0 { dims[7] } else { 0 };
    let total = history_total.saturating_add(dims[1]);
    let line_start = if scrollback {
        history_start
            .unwrap_or(history_total.saturating_sub(500))
            .min(total.saturating_sub(1))
    } else {
        history_total
    };
    let line_end = if history_start.is_some() && scrollback {
        line_start.saturating_add(500).min(total)
    } else {
        total
    };
    let start = (i64::from(line_start) - i64::from(history_total)).to_string();
    let end = (i64::from(line_end) - i64::from(history_total) - 1).to_string();
    let capture = |padding: bool| -> Result<String> {
        let mut command = tmux(managed)?;
        command.args(["capture-pane", "-e", "-p"]);
        if padding {
            command.arg("-N");
        }
        command.args(["-t", &before.pane_id, "-S", &start, "-E", &end]);
        bounded(command)
    };
    // -N preserves painted trailing cells. Older tmux is padded by the renderer.
    let screen = capture(true).or_else(|_| capture(false))?;
    let styled = styled_screen(&screen);
    let history = history_total
        .saturating_sub(line_start)
        .min(styled.len() as u32);
    let mut colours = tmux(managed)?;
    colours.args([
        "show-options",
        "-p",
        "-A",
        "-qv",
        "-t",
        &before.pane_id,
        "pane-colours[8]",
    ]);
    let palette8 = bounded(colours)
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|s| {
            s.len() == 7 && s.starts_with('#') && s[1..].bytes().all(|b| b.is_ascii_hexdigit())
        });
    let after = observe(managed, &terminal.session_id)?;
    verify_attach_generation(&request, after.server_pid, &after.id, &after.created)?;
    anyhow::ensure!(before.pane_id == after.pane_id, "preview pane changed");
    Ok(serde_json::json!({
        "lines": clean_screen(&screen), "styled_lines": styled,
        "pane_id": before.pane_id, "cols": dims[0], "rows": dims[1],
        "cursor_x": dims[2], "cursor_y": dims[3], "cursor_visible": dims[4] != 0,
        "bracketed_paste": dims[5] != 0, "application_cursor": dims[6] != 0,
        "palette8": palette8, "live": true, "history_rows": history,
        "history_total": history_total, "line_start": line_start, "alternate": dims[8] != 0, "mouse_tracking": dims[9] != 0, "mouse_sgr": dims[10] != 0
    }))
}

/// Input is guarded on the same tmux command queue as send-keys, preventing
/// server restarts from redirecting input between validation and delivery.
pub fn session_pane_input(
    session: &Session,
    pane_id: &str,
    hex: &str,
) -> Result<serde_json::Value> {
    anyhow::ensure!(
        pane_id
            .strip_prefix('%')
            .is_some_and(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())),
        "invalid pane identity"
    );
    anyhow::ensure!(
        !hex.is_empty()
            && hex.len() <= 8192
            && hex.len() % 2 == 0
            && hex.bytes().all(|b| b.is_ascii_hexdigit()),
        "invalid input"
    );
    let request = CheckedAttach {
        session: session.clone(),
        observe: false,
        guard: None,
    };
    let mut request = parse_attach_payload(&attach_payload(&request)?)?;
    let terminal = request.session.terminal.clone().unwrap();
    let current = observe(!session.external, &terminal.session_id)?;
    verify_attach_generation(&request, current.server_pid, &current.id, &current.created)?;
    anyhow::ensure!(
        current.pane_id == pane_id,
        "terminal pane changed; refocus before typing"
    );
    let mut guard = AttachGuard::new()?;
    request.guard = Some(guard.token.clone());
    let predicate = attach_predicate(&std::env::current_exe()?, &attach_payload(&request)?)?;
    let bytes = hex
        .as_bytes()
        .chunks_exact(2)
        .map(|b| std::str::from_utf8(b).unwrap())
        .collect::<Vec<_>>()
        .join(" ");
    let mut command = tmux(!session.external)?;
    command.args([
        "if-shell",
        "-t",
        &terminal.session_id,
        &predicate,
        &format!("send-keys -t {pane_id} -H {bytes}"),
        "",
    ]);
    bounded(command)?;
    anyhow::ensure!(guard.passed()?, "terminal changed; input refused");
    Ok(serde_json::json!({"delivered": true}))
}

/// Preserve screen rows and only SGR styling; discard OSC/DCS/other controls.
fn styled_screen(text: &str) -> Vec<String> {
    let mut out = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            match chars.next() {
                Some('[') => {
                    let mut sequence = String::new();
                    for c in chars.by_ref() {
                        if ('@'..='~').contains(&c) {
                            if c == 'm'
                                && sequence.len() <= 128
                                && sequence
                                    .bytes()
                                    .all(|b| b.is_ascii_digit() || b == b';' || b == b':')
                            {
                                out.push_str("\x1b[");
                                out.push_str(&sequence);
                                out.push('m');
                            }
                            break;
                        }
                        if sequence.len() <= 128 {
                            sequence.push(c);
                        }
                    }
                }
                Some(']') | Some('P') | Some('_') | Some('^') => {
                    while let Some(c) = chars.next() {
                        if c == '\x07' {
                            break;
                        }
                        if c == '\x1b' && chars.peek() == Some(&'\\') {
                            chars.next();
                            break;
                        }
                    }
                }
                _ => {}
            }
        } else if c == '\n'
            || c == '\t'
            || (!c.is_control() && !matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'))
        {
            out.push(c);
        }
        if out.len() >= 256 * 1024 {
            break;
        }
    }
    // Drop only capture-pane's final separator, retaining blank screen rows.
    out.strip_suffix('\n')
        .unwrap_or(&out)
        .split('\n')
        .take(800)
        .map(str::to_owned)
        .collect()
}

fn clean_screen(text: &str) -> Vec<String> {
    let mut out = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if out.len() + c.len_utf8() > 24 * 1024 {
            break;
        }
        if c == '\u{1b}' {
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
                        if c == '\u{7}' {
                            break;
                        }
                        if c == '\u{1b}' && chars.peek() == Some(&'\\') {
                            chars.next();
                            break;
                        }
                    }
                }
                _ => {}
            }
        } else if c == '\n'
            || c == '\t'
            || (!c.is_control() && !matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'))
        {
            out.push(c);
        }
    }
    out.trim_end_matches('\n')
        .lines()
        .take(60)
        .map(|line| line.chars().take(512).collect())
        .collect()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn live_view_flags_always_print_a_digit_on_older_tmux() {
        // tmux expands an unknown variable to nothing but treats it as false in a
        // conditional; bare flags dropped a field on tmux 3.4 (no bracket_paste_flag).
        let fields: Vec<&str> = LIVE_VIEW_FORMAT.split(' ').collect();
        assert_eq!(fields.len(), 11);
        for field in fields {
            let name = field.trim_start_matches("#{").trim_end_matches('}');
            if name.contains("flag") || name == "?alternate_on,1,0" {
                assert!(
                    field.starts_with("#{?") && field.ends_with(",1,0}"),
                    "{field} must be a conditional"
                );
            }
        }
    }
    #[test]
    fn live_screen_preserves_layout_and_sgr_only() {
        assert_eq!(
            styled_screen("\x1b[31mred\x1b[0m\n\nlast\n"),
            vec!["\x1b[31mred\x1b[0m", "", "last"]
        );
        assert_eq!(
            styled_screen("x\x1b]52;c;private\x07\x1b[2J\x1bPdiscard\x1b\\y\n"),
            vec!["xy"]
        );
        assert_eq!(styled_screen("\n\n\n"), vec!["", "", ""]);
        assert_eq!(styled_screen(&"line\n".repeat(900)).len(), 800);
    }
    #[test]
    fn snapshot_text_has_no_terminal_controls_and_is_bounded() {
        assert_eq!(
            clean_screen("ok\x1b[31m red\x1b[0m\x1b]52;c;hidden\x07\u{202e}\nnext"),
            vec!["ok red", "next"]
        );
        let lines = clean_screen(&"雪".repeat(40000));
        assert!(lines.iter().map(String::len).sum::<usize>() <= 24576);
        assert!(lines[0].chars().count() <= 512);
        assert_eq!(clean_screen(&"line\n".repeat(200)).len(), 60);
    }
    #[test]
    fn capture_process_timeout_and_output_limit() {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "exec sleep 10"]);
        let start = Instant::now();
        assert!(bounded(cmd).is_err());
        assert!(start.elapsed() < Duration::from_secs(2));
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "head -c 270000 /dev/zero"]);
        assert!(bounded(cmd).is_err());
    }
}
