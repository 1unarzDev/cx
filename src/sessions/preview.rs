//! Read-only, ephemeral screen snapshots. Never resize, attach, read history or write state.
use super::*;
use std::io::Read;
use std::os::fd::AsRawFd;
use std::process::Stdio;
use std::time::Instant;

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
                    anyhow::ensure!(bytes.len() + n <= 65536, "preview exceeds screen limit");
                    bytes.extend_from_slice(&buf[..n]);
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(5))
                }
                Err(e) => return Err(e.into()),
            }
        }
        loop {
            if let Some(status) = child.try_wait()? {
                anyhow::ensure!(status.success(), "preview unavailable");
                return Ok(String::from_utf8_lossy(&bytes).into_owned());
            }
            anyhow::ensure!(Instant::now() < end, "preview timed out");
            thread::sleep(Duration::from_millis(5));
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

pub fn preview(session: &Session) -> Result<serde_json::Value> {
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
        "#{pane_height}",
    ]);
    let height: u32 = bounded(size)?
        .trim()
        .parse()
        .context("invalid pane height")?;
    anyhow::ensure!(height > 0 && height <= 10000, "invalid pane height");
    let mut command = tmux(managed)?;
    command
        .args(["capture-pane", "-p", "-t", &before.pane_id, "-S"])
        .arg(height.saturating_sub(60).to_string())
        .arg("-E")
        .arg((height - 1).to_string());
    let screen = bounded(command)?;
    // Discard everything if the target changes during capture. Numeric pane IDs are
    // server-unique; checking the server generation prevents ID reuse after restart.
    let after = observe(managed, &terminal.session_id)?;
    verify_attach_generation(&request, after.server_pid, &after.id, &after.created)?;
    anyhow::ensure!(before.pane_id == after.pane_id, "preview pane changed");
    Ok(serde_json::json!({"lines": clean_screen(&screen)}))
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
        cmd.args(["-c", "head -c 70000 /dev/zero"]);
        assert!(bounded(cmd).is_err());
    }
}
