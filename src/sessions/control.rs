//! One tmux control-mode client (`tmux -C`) per managed server, owned by this
//! process: pushed session-list changes, `%output` of the watched pane as a
//! wake signal only (tmux owns the grid; cx renders its captures), and
//! captures and keystrokes sent as commands, with no process spawn. tmux
//! before 3.2 (no `ignore-size`, so a control client could resize windows), or
//! any control failure, leaves the caller on the polling path.
use super::*;
use std::collections::VecDeque;
use std::io::{BufRead, BufReader};
use std::sync::{mpsc, Arc, Condvar, Mutex};
use std::time::Instant;

/// Delivers one event; false once nobody is listening.
pub type Emit = Arc<dyn Fn(serde_json::Value) -> bool + Send + Sync>;
/// The hidden session the client sits in while no pane is watched. Never
/// listed; it lives as long as the server.
pub(super) const CONTROL_SESSION: &str = "cxctl";
const REPLY_LIMIT: Duration = Duration::from_secs(1);
const BODY_LIMIT: usize = 1024 * 1024;
/// At most one capture per frame (~60/s), however busy the pane.
const CAPTURE_INTERVAL: Duration = Duration::from_millis(16);
/// List changes come in bursts (a command starting renames its window).
const LIST_DEBOUNCE: Duration = Duration::from_millis(100);
const SAFETY_POLL: Duration = Duration::from_secs(30);

pub(super) struct Client {
    pid: u32,
    /// Server pid and start ticks: the instance this connection is bound to.
    server: std::sync::OnceLock<(u32, String)>,
    io: Mutex<Io>,
    state: Mutex<State>,
    changed: Condvar,
}
struct Io {
    stdin: std::process::ChildStdin,
    pending: VecDeque<mpsc::SyncSender<Result<String>>>,
}
#[derive(Default)]
struct State {
    alive: bool,
    /// The watched pane (`%N`) and the session's tmux id, once verified.
    pane: Option<(String, Session)>,
    pane_dirty: bool,
    list_dirty: Option<Instant>,
    /// Bumped by every watch; an older watch thread ends.
    generation: u64,
}

/// One line of control-mode output, as far as cx needs it.
#[derive(Debug, PartialEq)]
pub(super) enum Notice<'a> {
    /// `%begin`/`%end`/`%error` with their `time number` key; `ours` is a
    /// command this client sent (flags 1).
    Begin {
        key: &'a str,
        ours: bool,
    },
    End {
        key: &'a str,
        ok: bool,
    },
    Output {
        pane: &'a str,
    },
    ListChanged,
    PaneChanged,
    Exit,
    Other,
}
pub(super) fn parse(line: &str) -> Notice<'_> {
    let (word, rest) = line.split_once(' ').unwrap_or((line, ""));
    match word {
        "%begin" | "%end" | "%error" => {
            let mut fields = rest.splitn(3, ' ');
            let (Some(time), Some(number)) = (fields.next(), fields.next()) else {
                return Notice::Other;
            };
            let key = &rest[..time.len() + 1 + number.len()];
            let ours = fields.next().is_some_and(|f| f.trim() == "1");
            match word {
                "%begin" => Notice::Begin { key, ours },
                _ => Notice::End {
                    key,
                    ok: word == "%end",
                },
            }
        }
        "%output" => Notice::Output {
            pane: rest.split(' ').next().unwrap_or(""),
        },
        "%sessions-changed"
        | "%session-renamed"
        | "%window-add"
        | "%window-close"
        | "%window-renamed"
        | "%unlinked-window-add"
        | "%unlinked-window-close"
        | "%unlinked-window-renamed" => Notice::ListChanged,
        "%layout-change" | "%pane-mode-changed" | "%window-pane-changed" | "%session-changed" => {
            Notice::PaneChanged
        }
        "%exit" => Notice::Exit,
        _ => Notice::Other,
    }
}
/// A tmux command line for control mode. Every argument cx sends is
/// generated (ids, formats, hex); refuse anything that could leave quotes.
pub(super) fn command_line(args: &[&str]) -> Result<String> {
    let mut line = String::new();
    for arg in args {
        anyhow::ensure!(
            !arg.contains(['\'', '\n', '\r', '\0']),
            "unsupported control argument"
        );
        if !line.is_empty() {
            line.push(' ');
        }
        line.push('\'');
        line.push_str(arg);
        line.push('\'');
    }
    line.push('\n');
    Ok(line)
}

static CLIENT: Mutex<Option<Arc<Client>>> = Mutex::new(None);

/// Debug-only switch for the forced polling fallback in tests and benches.
fn disabled() -> bool {
    cfg!(debug_assertions) && std::env::var_os("CX_TMUX_CONTROL").is_some_and(|v| v == "0")
}
fn supported() -> bool {
    static SUPPORTED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    !disabled() && *SUPPORTED.get_or_init(|| tmux_at_least(3, 2))
}

impl Client {
    pub(super) fn alive(&self) -> bool {
        self.state.lock().is_ok_and(|s| s.alive)
    }
    /// Run one tmux command on this server; its output, newline-terminated lines.
    pub(super) fn run(&self, args: &[&str]) -> Result<String> {
        let line = command_line(args)?;
        let (tx, rx) = mpsc::sync_channel(1);
        {
            let mut io = self
                .io
                .lock()
                .map_err(|_| anyhow::anyhow!("control lock"))?;
            anyhow::ensure!(self.alive(), "tmux control client ended");
            io.pending.push_back(tx);
            if io.stdin.write_all(line.as_bytes()).is_err() {
                io.pending.pop_back();
                drop(io);
                self.end();
                bail!("tmux control client ended");
            }
        }
        match rx.recv_timeout(REPLY_LIMIT) {
            Ok(result) => result,
            Err(_) => {
                // A lost reply would pair every later reply with the wrong command.
                self.end();
                bail!("tmux control reply timed out")
            }
        }
    }
    fn end(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.alive = false;
        }
        self.changed.notify_all();
        unsafe { libc::kill(self.pid as i32, libc::SIGKILL) };
    }
    /// Reader loop on the owning thread: replies in order, notices as wakes.
    fn read(&self, out: impl std::io::Read) {
        let mut reader = BufReader::new(out);
        let mut block: Option<(String, bool, String)> = None;
        let mut line = Vec::new();
        loop {
            line.clear();
            match reader.read_until(b'\n', &mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
            let text = String::from_utf8_lossy(line.strip_suffix(b"\n").unwrap_or(&line));
            if let Some((key, _, body)) = &mut block {
                // Only the exact closing key ends a block; pane text cannot
                // guess it from inside a capture without also knowing the time.
                if let Notice::End { key: end, ok } = parse(&text) {
                    if end == key.as_str() {
                        let (_, ours, body) = block.take().unwrap();
                        if ours {
                            let reply = self
                                .io
                                .lock()
                                .ok()
                                .and_then(|mut io| io.pending.pop_front());
                            if let Some(reply) = reply {
                                let _ = reply.send(if ok {
                                    Ok(body)
                                } else {
                                    Err(anyhow::anyhow!("tmux: {}", body.trim()))
                                });
                            }
                        }
                        continue;
                    }
                }
                if body.len() + text.len() < BODY_LIMIT {
                    body.push_str(&text);
                    body.push('\n');
                }
                continue;
            }
            let mut state = match self.state.lock() {
                Ok(state) => state,
                Err(_) => break,
            };
            match parse(&text) {
                Notice::Begin { key, ours } => {
                    block = Some((key.to_owned(), ours, String::new()));
                }
                Notice::Output { pane } => {
                    if state.pane.as_ref().is_some_and(|(p, _)| p == pane) && !state.pane_dirty {
                        state.pane_dirty = true;
                        self.changed.notify_all();
                    }
                }
                Notice::PaneChanged => {
                    state.pane_dirty |= state.pane.is_some();
                    self.changed.notify_all();
                }
                Notice::ListChanged => {
                    state.list_dirty.get_or_insert_with(Instant::now);
                    self.changed.notify_all();
                }
                Notice::Exit => break,
                Notice::End { .. } | Notice::Other => {}
            }
        }
        if let Ok(mut state) = self.state.lock() {
            state.alive = false;
        }
        if let Ok(mut io) = self.io.lock() {
            for reply in io.pending.drain(..) {
                let _ = reply.send(Err(anyhow::anyhow!("tmux control client ended")));
            }
        }
        self.changed.notify_all();
    }
}

/// The live client for the managed server, started on first use. Never
/// starts a server: it attaches to an existing one or fails.
pub(super) fn client() -> Result<Arc<Client>> {
    anyhow::ensure!(supported(), "tmux control mode unavailable");
    let mut slot = CLIENT.lock().map_err(|_| anyhow::anyhow!("control lock"))?;
    if let Some(client) = slot.as_ref().filter(|c| c.alive()) {
        return Ok(client.clone());
    }
    *slot = None;
    let client = start()?;
    *slot = Some(client.clone());
    Ok(client)
}
/// The live client, if one is already running; never starts one.
pub(super) fn running() -> Option<Arc<Client>> {
    CLIENT.lock().ok()?.as_ref().filter(|c| c.alive()).cloned()
}

fn start() -> Result<Arc<Client>> {
    let mut command = tmux(true)?;
    command.args(["-C", "attach-session", "-f", "ignore-size"]);
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let (tx, rx) = mpsc::sync_channel::<Result<Arc<Client>>>(1);
    // The owning thread spawns the client and reads it until it ends:
    // PR_SET_PDEATHSIG fires when this thread exits, so the client can
    // never outlive its reader, and its own process group keeps terminal
    // signals away from it.
    thread::Builder::new()
        .name("cx-tmux-control".into())
        .spawn(move || {
            use std::os::unix::process::CommandExt;
            unsafe {
                command.pre_exec(|| {
                    libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL);
                    Ok(())
                });
            }
            command.process_group(0);
            let mut child = match command.spawn() {
                Ok(child) => child,
                Err(error) => {
                    let _ = tx.send(Err(error.into()));
                    return;
                }
            };
            let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
                let _ = child.kill();
                let _ = child.wait();
                return;
            };
            let client = Arc::new(Client {
                pid: child.id(),
                server: std::sync::OnceLock::new(),
                io: Mutex::new(Io {
                    stdin,
                    pending: VecDeque::new(),
                }),
                state: Mutex::new(State {
                    alive: true,
                    ..State::default()
                }),
                changed: Condvar::new(),
            });
            let reader = client.clone();
            let _ = tx.send(Ok(client));
            reader.read(stdout);
            let _ = child.kill();
            let _ = child.wait();
        })?;
    let client = rx
        .recv_timeout(REPLY_LIMIT)
        .map_err(|_| anyhow::anyhow!("tmux control client did not start"))??;
    let ready = (|| -> Result<(u32, String)> {
        let pid: u32 = client
            .run(&["display-message", "-p", "#{pid}"])?
            .trim()
            .parse()?;
        let start = process_start_ticks(pid).context("tmux server identity unavailable")?;
        if client.run(&["has-session", "-t", "=cxctl"]).is_err() {
            client.run(&[
                "new-session",
                "-d",
                "-s",
                CONTROL_SESSION,
                "-x",
                "10",
                "-y",
                "2",
                "cat",
            ])?;
        }
        client.run(&["switch-client", "-t", "=cxctl"])?;
        Ok((pid, start))
    })();
    match ready {
        Ok(server) => {
            let _ = client.server.set(server);
            Ok(client)
        }
        Err(error) => {
            client.end();
            Err(error)
        }
    }
}

/// Start, replace or (with neither field) end this process's watch. Events
/// are emitted only from the watch thread, after this returns.
pub fn watch(sessions: bool, pane: Option<Session>, emit: Emit) -> Result<serde_json::Value> {
    if let Some(client) = running() {
        if let Ok(mut state) = client.state.lock() {
            state.generation += 1;
            state.pane = None;
        }
        client.changed.notify_all();
    }
    if !sessions && pane.is_none() {
        return Ok(serde_json::json!({"watching": false}));
    }
    anyhow::ensure!(
        pane.as_ref().is_none_or(|s| !s.external),
        "live watch covers managed sessions only"
    );
    let client = client()?;
    let mut first = None;
    let watched = match pane {
        Some(session) => {
            let screen = verified_pane(&client, &session)?;
            let pane_id = screen["pane_id"].as_str().unwrap_or_default().to_owned();
            first = Some(screen);
            let terminal = session
                .terminal
                .as_ref()
                .context("terminal identity unavailable")?;
            // Joining the session is what makes tmux report its output.
            client.run(&["switch-client", "-t", &terminal.session_id])?;
            Some((pane_id, session))
        }
        None => {
            let _ = client.run(&["switch-client", "-t", "=cxctl"]);
            None
        }
    };
    let pane_id = watched.as_ref().map(|(p, _)| p.clone());
    let generation = {
        let mut state = client
            .state
            .lock()
            .map_err(|_| anyhow::anyhow!("control lock"))?;
        state.generation += 1;
        state.pane_dirty = false;
        state.pane = watched;
        state.list_dirty = None;
        state.generation
    };
    let owned = client.clone();
    let shown = first.clone();
    thread::Builder::new()
        .name("cx-watch".into())
        .spawn(move || watch_loop(owned, generation, sessions, shown, emit))?;
    // The reply carries the first screen; events follow only on change.
    Ok(serde_json::json!({"watching": true, "pane_id": pane_id, "pane": first}))
}

/// The session's current screen, after the same identity check as a
/// preview, on the server this client is bound to.
fn verified_pane(client: &Client, session: &Session) -> Result<serde_json::Value> {
    let terminal = session
        .terminal
        .as_ref()
        .context("terminal identity unavailable")?;
    let server = client
        .server
        .get()
        .context("tmux server identity unavailable")?;
    anyhow::ensure!(
        server.0 == terminal.server_pid && server.1 == terminal.server_start,
        "Session changed or ended; refresh before attaching"
    );
    preview::preview_with(&|args| client.run(args), session, false, None)
}

fn watch_loop(
    client: Arc<Client>,
    generation: u64,
    sessions: bool,
    first: Option<serde_json::Value>,
    emit: Emit,
) {
    let mut last_capture = first.as_ref().map(|_| Instant::now());
    let mut last_pane = first;
    let mut last_list: Option<serde_json::Value> = None;
    let mut last_poll = Instant::now();
    let mut prepared = false;
    loop {
        let (capture, list) = {
            let Ok(mut state) = client.state.lock() else {
                return;
            };
            loop {
                if state.generation != generation {
                    return;
                }
                if !state.alive {
                    drop(state);
                    emit(serde_json::json!({"kind": "closed"}));
                    return;
                }
                let now = Instant::now();
                let since = last_capture.map(|t| now.saturating_duration_since(t));
                let capture_due = state.pane_dirty && since.is_none_or(|s| s >= CAPTURE_INTERVAL);
                let safety = now.saturating_duration_since(last_poll) >= SAFETY_POLL;
                let list_due = sessions
                    && (safety
                        || state
                            .list_dirty
                            .is_some_and(|t| now.saturating_duration_since(t) >= LIST_DEBOUNCE));
                if capture_due || list_due {
                    state.pane_dirty &= !capture_due;
                    if list_due {
                        state.list_dirty = None;
                        // A list change may be the watched session ending.
                        state.pane_dirty |= state.pane.is_some();
                    }
                    break (
                        capture_due.then(|| state.pane.clone()).flatten(),
                        list_due.then_some(safety),
                    );
                }
                let mut wait = SAFETY_POLL.saturating_sub(now.saturating_duration_since(last_poll));
                if state.pane_dirty {
                    wait = wait.min(CAPTURE_INTERVAL.saturating_sub(since.unwrap_or_default()));
                }
                if let Some(t) = state.list_dirty.filter(|_| sessions) {
                    wait = wait.min(LIST_DEBOUNCE.saturating_sub(now.saturating_duration_since(t)));
                }
                state = match client
                    .changed
                    .wait_timeout(state, wait.max(Duration::from_millis(1)))
                {
                    Ok((state, _)) => state,
                    Err(_) => return,
                };
            }
        };
        if let Some((pane_id, session)) = capture {
            last_capture = Some(Instant::now());
            let session_id = session.terminal.as_ref().map(|t| t.session_id.clone());
            match preview::preview_with(&|args| client.run(args), &session, false, None) {
                Ok(mut value) => {
                    let current = value["pane_id"].as_str().unwrap_or_default().to_owned();
                    if current != pane_id {
                        if let Ok(mut state) = client.state.lock() {
                            if state.generation == generation {
                                state.pane = Some((current, session.clone()));
                            }
                        }
                    }
                    if last_pane.as_ref() != Some(&value) {
                        last_pane = Some(value.clone());
                        value["kind"] = "pane".into();
                        value["session_id"] = session_id.into();
                        if !emit(value) {
                            return;
                        }
                    }
                    if !prepared {
                        // Attach later skips this: tmux setup is done while watching.
                        prepared = true;
                        prepare_attach(&session);
                    }
                }
                Err(_) => {
                    if let Ok(mut state) = client.state.lock() {
                        if state.generation == generation {
                            state.pane = None;
                        }
                    }
                    let ended = serde_json::json!({
                        "kind": "pane", "session_id": session_id,
                        "error": "Terminal unavailable · refresh to check"
                    });
                    if !emit(ended) {
                        return;
                    }
                }
            }
        }
        if let Some(safety) = list {
            last_poll = Instant::now();
            if let Ok(value) = super::list().and_then(|l| Ok(serde_json::to_value(l)?)) {
                // The safety poll always reports, so the list stays recently confirmed.
                if safety || last_list.as_ref() != Some(&value) {
                    last_list = Some(value.clone());
                    if !emit(serde_json::json!({"kind": "sessions", "sessions": value})) {
                        return;
                    }
                }
            }
        }
    }
}

/// Keystrokes for the watched pane go through the control client: no spawn,
/// and the connection itself pins the server instance verified at subscribe
/// time (a restarted server ends the client). None: use the checked path.
pub(super) fn input(session: &Session, pane_id: &str, hex: &str) -> Option<Result<()>> {
    let client = running()?;
    {
        let state = client.state.lock().ok()?;
        let (watched, watched_session) = state.pane.as_ref()?;
        let server = client.server.get()?;
        let terminal = session.terminal.as_ref()?;
        if watched != pane_id
            || watched_session.id != session.id
            || watched_session.boot_id != session.boot_id
            || watched_session.terminal != session.terminal
            || server.0 != terminal.server_pid
            || server.1 != terminal.server_start
        {
            return None;
        }
    }
    let pairs: Vec<&str> = (0..hex.len()).step_by(2).map(|i| &hex[i..i + 2]).collect();
    let mut args = vec!["send-keys", "-t", pane_id, "-H"];
    args.extend(pairs);
    Some(client.run(&args).map(|_| ()))
}

/// Attach tickets: 0600 files saying this session's tmux setup (status line,
/// styles) was refreshed while watching, so a native attach within
/// TICKET_LIFETIME skips it. Identity is still verified on every attach.
const TICKET_LIFETIME: Duration = Duration::from_secs(120);
fn ticket_path(session: &Session, ascii: bool) -> Result<PathBuf> {
    let terminal = session
        .terminal
        .as_ref()
        .context("terminal identity unavailable")?;
    let digest = Sha256::digest(
        format!(
            "{}\n{}\n{}\n{}\n{}\n{}",
            session.id,
            session.boot_id,
            terminal.server_pid,
            terminal.server_start,
            terminal.session_id,
            ascii
        )
        .as_bytes(),
    );
    let name: String = digest[..16].iter().map(|b| format!("{b:02x}")).collect();
    Ok(state()?.join(format!("attach-ticket-{name}")))
}
fn prepare_attach(session: &Session) {
    let ascii = viewer_ascii();
    let (Ok(path), Some(terminal)) = (ticket_path(session, ascii), session.terminal.as_ref())
    else {
        return;
    };
    let configured =
        configured(terminal.server_pid, &terminal.server_start) || configure_managed().is_ok();
    if configured && refresh_managed_status_as(&session.id, ascii).is_ok() {
        let _ = fs::remove_file(&path);
        let _ = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&path);
    }
}
/// True when a fresh, private ticket exists for this exact session, viewer
/// glyph mode and server instance.
pub(super) fn ticket_valid(session: &Session, ascii: bool) -> bool {
    use std::os::unix::fs::MetadataExt;
    let Ok(path) = ticket_path(session, ascii) else {
        return false;
    };
    fs::symlink_metadata(&path).is_ok_and(|m| {
        m.is_file()
            && m.uid() == unsafe { libc::geteuid() }
            && m.mode() & 0o777 == 0o600
            && m.nlink() == 1
            && m.modified()
                .ok()
                .and_then(|t| t.elapsed().ok())
                .is_some_and(|age| age < TICKET_LIFETIME)
    })
}

/// `configure_managed` once per server instance: a 0600 marker names the
/// server pid, its start ticks and this binary's version.
fn marker() -> Result<PathBuf> {
    Ok(state()?.join("tmux-configured"))
}
fn marker_text(pid: u32, start: &str) -> String {
    format!("{pid} {start} {}\n", env!("CARGO_PKG_VERSION"))
}
pub(super) fn configured(pid: u32, start: &str) -> bool {
    marker()
        .and_then(|p| Ok(fs::read_to_string(p)?))
        .is_ok_and(|text| text == marker_text(pid, start))
}
/// Record that the running server `pid` was configured by this binary.
pub(super) fn record_configured(pid: u32) -> Result<()> {
    let start = process_start_ticks(pid).context("tmux server identity unavailable")?;
    let path = marker()?;
    let _ = fs::remove_file(&path);
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&path)?
        .write_all(marker_text(pid, &start).as_bytes())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn control_lines_parse_into_blocks_wakes_and_exit() {
        assert_eq!(
            parse("%begin 1794358991 317 1"),
            Notice::Begin {
                key: "1794358991 317",
                ours: true
            }
        );
        assert_eq!(
            parse("%begin 1794358991 317 0"),
            Notice::Begin {
                key: "1794358991 317",
                ours: false
            }
        );
        assert_eq!(
            parse("%error 1794358991 318 1"),
            Notice::End {
                key: "1794358991 318",
                ok: false
            }
        );
        assert_eq!(
            parse("%output %12 echo\\015\\012"),
            Notice::Output { pane: "%12" }
        );
        assert_eq!(parse("%sessions-changed"), Notice::ListChanged);
        assert_eq!(
            parse("%unlinked-window-renamed @3 vim"),
            Notice::ListChanged
        );
        assert_eq!(parse("%layout-change @1 abcd"), Notice::PaneChanged);
        assert_eq!(parse("%exit"), Notice::Exit);
        assert_eq!(parse("%begin"), Notice::Other);
        assert_eq!(parse("plain text"), Notice::Other);
    }
    #[test]
    fn control_commands_are_single_quoted_and_refuse_quotes() {
        assert_eq!(
            command_line(&["send-keys", "-t", "%3", "-H", "61"]).unwrap(),
            "'send-keys' '-t' '%3' '-H' '61'\n"
        );
        assert_eq!(
            command_line(&["display-message", "-p", "#{pid} #{session_id}"]).unwrap(),
            "'display-message' '-p' '#{pid} #{session_id}'\n"
        );
        assert!(command_line(&["a'b"]).is_err());
        assert!(command_line(&["a\nkill-server"]).is_err());
    }
    fn fixture() -> (Arc<Client>, std::process::Child) {
        let mut child = Command::new("sleep")
            .arg("30")
            .stdin(Stdio::piped())
            .spawn()
            .unwrap();
        let client = Arc::new(Client {
            pid: child.id(),
            server: std::sync::OnceLock::new(),
            io: Mutex::new(Io {
                stdin: child.stdin.take().unwrap(),
                pending: VecDeque::new(),
            }),
            state: Mutex::new(State {
                alive: true,
                ..State::default()
            }),
            changed: Condvar::new(),
        });
        (client, child)
    }
    #[test]
    fn replies_pair_in_order_and_pane_text_cannot_end_a_block_early() {
        let (client, mut child) = fixture();
        let (first_tx, first) = mpsc::sync_channel(1);
        let (second_tx, second) = mpsc::sync_channel(1);
        {
            let mut io = client.io.lock().unwrap();
            io.pending.push_back(first_tx);
            io.pending.push_back(second_tx);
        }
        client.state.lock().unwrap().pane = Some(("%7".into(), serde_json::from_value(
            serde_json::json!({"id":"x","name":"x","directory":"/","provider":"shell","host":"h",
                "account":"a","pid":1,"started":"0","boot_id":"b","external":false,"socket":null})).unwrap()));
        let transcript = "%begin 100 1 0\n%end 100 1 0\n%session-changed $1 cxctl\n\
            %begin 100 2 1\nline one\n%end 100 9 1\n%end 100 2 1\n\
            %output %7 x\n%output %8 y\n%sessions-changed\n\
            %begin 100 3 1\ncan't find pane\n%error 100 3 1\n%exit\n";
        client.read(std::io::Cursor::new(transcript.as_bytes().to_vec()));
        // A capture line that looks like another block's end is pane content.
        assert_eq!(first.recv().unwrap().unwrap(), "line one\n%end 100 9 1\n");
        assert!(second.recv().unwrap().is_err());
        let state = client.state.lock().unwrap();
        assert!(
            state.pane_dirty,
            "the watched pane's output wakes a capture"
        );
        assert!(state.list_dirty.is_some());
        assert!(!state.alive, "%exit ends the client");
        let _ = child.kill();
        let _ = child.wait();
    }
    #[test]
    fn a_dead_client_fails_pending_and_new_commands_at_once() {
        let (client, mut child) = fixture();
        let (tx, rx) = mpsc::sync_channel(1);
        client.io.lock().unwrap().pending.push_back(tx);
        client.read(std::io::Cursor::new(Vec::new()));
        assert!(rx.recv().unwrap().is_err());
        assert!(client.run(&["display-message", "-p", "x"]).is_err());
        let _ = child.kill();
        let _ = child.wait();
    }
}
