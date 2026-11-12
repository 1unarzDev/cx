use crate::model::*;
use anyhow::{anyhow, bail, Context, Result};
use std::{
    io::{BufRead, BufReader, Read, Write},
    process::{Command, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};
pub const MAX_MESSAGE: usize = 1024 * 1024;
pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
pub fn valid_target(s: &str) -> bool {
    !s.is_empty()
        && s.len() < 256
        && !s.starts_with('-')
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"@._-:[]".contains(&b))
}
pub fn frame<T: serde::Serialize>(w: &mut impl Write, v: &T) -> Result<()> {
    let b = serde_json::to_vec(v)?;
    if b.len() > MAX_MESSAGE {
        bail!("message exceeds limit")
    }
    write!(w, "CX1 {}\n", b.len())?;
    w.write_all(&b)?;
    w.flush()?;
    Ok(())
}
pub fn read_frame<T: serde::de::DeserializeOwned>(r: &mut impl BufRead) -> Result<T> {
    let mut scanned = 0;
    loop {
        let mut line = Vec::new();
        let n = r.take(4097).read_until(b'\n', &mut line)?;
        scanned += n;
        if n == 0 {
            bail!("helper closed before response")
        }
        if scanned > 16384 || line.len() > 4096 {
            bail!("excessive shell startup noise")
        }
        if let Some(raw) = line.strip_prefix(b"CX1 ") {
            let raw = std::str::from_utf8(raw)?.trim();
            let size = raw.parse::<usize>().context("invalid frame length")?;
            if size > MAX_MESSAGE {
                bail!("message exceeds limit")
            }
            let mut b = vec![0; size];
            r.read_exact(&mut b)?;
            return Ok(serde_json::from_slice(&b)?);
        }
    }
}
/// How long a terminal waits for a route's dedicated master before it
/// connects directly instead (the master keeps starting in the background).
const INTERACTIVE_MASTER_WAIT: std::time::Duration = std::time::Duration::from_millis(1500);

/// An SSH command for `target`. Before a terminal-owning (interactive)
/// command, outside askpass enrollment, the route's dedicated master is
/// started or joined for at most 1.5 s, so the terminal multiplexes over it;
/// the terminal itself never becomes the master.
pub fn ssh(target: &str, interactive: bool) -> Result<Command> {
    if interactive && !crate::auth::active() {
        warm_master(target, INTERACTIVE_MASTER_WAIT);
    }
    crate::store::ssh(target, interactive)
}

// Shared SSH masters. Each route has at most one cx-owned OpenSSH master: a
// dedicated detached `-N` process (`store::SshMode::Master`). Helper channels
// and terminals only join it, so only the first contact pays the handshake
// through every ProxyJump hop and no pane or channel lifetime can end it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MasterState {
    Ready,
    /// Typed evidence from the master attempt; a direct retry would fail alike.
    Failed(ConnectionFailure),
    /// Sharing is off, the socket path is too long, the master is not ready
    /// yet, or the outcome is unclear: callers connect directly as before.
    Unavailable,
}
/// A master attempt that has not authenticated by now is reaped.
const MASTER_DEADLINE: std::time::Duration = std::time::Duration::from_secs(25);
/// A helper channel waits this long for a starting master, then connects
/// directly while the master keeps starting in the background, so short
/// budgets (3 s screen reads, 15 s default) still cover their own handshake.
const CHANNEL_MASTER_WAIT: std::time::Duration = std::time::Duration::from_millis(1500);
/// After a failed attempt, connections go direct (and report their own typed
/// failure) instead of starting another master for this long.
const MASTER_COOLDOWN: std::time::Duration = std::time::Duration::from_secs(20);
const PREWARM_LIMIT: usize = 4;
/// A trivial channel through a healthy master answers well within this.
const PROBE_LIMIT: std::time::Duration = std::time::Duration::from_secs(3);
/// A master confirmed this recently is not re-checked (a terminal right
/// after its session lookup); a master gone since then only means the
/// joining ssh connects directly.
const CONFIRMED_FOR: std::time::Duration = std::time::Duration::from_secs(1);
/// A helper channel that joined the master just answered: the master was
/// carrying it, so a terminal opened soon after (an attach) skips `-O check`.
const ANSWERED_FOR: std::time::Duration = std::time::Duration::from_secs(10);

/// One target's in-flight master attempt; waiters see its outcome.
#[derive(Default)]
struct Flight {
    state: std::sync::Mutex<Option<MasterState>>,
    done: std::sync::Condvar,
}
#[derive(Default)]
struct Masters {
    flights: std::collections::HashMap<String, std::sync::Arc<Flight>>,
    failed: std::collections::HashMap<String, std::time::Instant>,
    /// Targets whose owner record this process has written (or confirmed).
    recorded: std::collections::HashSet<String>,
    recovering: std::collections::HashSet<String>,
    /// Until when each target's master counts as answering.
    confirmed: std::collections::HashMap<String, std::time::Instant>,
    prewarming: usize,
}
static MASTERS: std::sync::LazyLock<std::sync::Mutex<Masters>> =
    std::sync::LazyLock::new(Default::default);

fn masters() -> Option<std::sync::MutexGuard<'static, Masters>> {
    MASTERS.lock().ok()
}

fn wait_bounded(child: &mut std::process::Child, limit: std::time::Duration) -> Option<bool> {
    let deadline = std::time::Instant::now() + limit;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status.success()),
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(5))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

/// Local bounded `ssh -O <op>` against this route's master (check, stop).
fn master_control(target: &str, op: &'static str) -> bool {
    let Ok(mut command) = crate::store::ssh_mode(target, crate::store::SshMode::Control(op)) else {
        return false;
    };
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let Ok(mut child) = command.spawn() else {
        return false;
    };
    wait_bounded(&mut child, std::time::Duration::from_secs(2)) == Some(true)
}

/// True only when this route's master answers `-O check`.
fn master_alive(target: &str) -> bool {
    master_control(target, "check")
}

/// The expanded socket path of this route's master, from `ssh -G` (local only).
fn master_socket(target: &str) -> Option<std::path::PathBuf> {
    let mut command = crate::store::ssh_mode(target, crate::store::SshMode::Resolve).ok()?;
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = command.spawn().ok()?;
    let stdout = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut out = Vec::new();
        let _ = stdout.take(256 * 1024).read_to_end(&mut out);
        out
    });
    let finished = wait_bounded(&mut child, std::time::Duration::from_secs(2)) == Some(true);
    let out = reader.join().ok()?;
    if !finished {
        return None;
    }
    String::from_utf8_lossy(&out).lines().find_map(|line| {
        let path = std::path::PathBuf::from(line.strip_prefix("controlpath ")?);
        path.is_absolute().then_some(path)
    })
}

/// Write the live master's owner record (once per process and target).
fn record_owner(target: &str) {
    let Some(socket) = master_socket(target) else {
        return;
    };
    if crate::store::record_master_owner(&socket, target).is_ok() {
        if let Some(mut m) = masters() {
            if m.recorded.len() >= 256 {
                m.recorded.clear();
            }
            m.recorded.insert(target.to_owned());
        }
    }
}

/// Record a master this process found already running (another viewer's, or
/// askpass enrollment's) off the request path.
fn record_owner_later(target: &str) {
    if masters().is_none_or(|m| m.recorded.contains(target)) {
        return;
    }
    let owned = target.to_owned();
    let _ = std::thread::Builder::new()
        .name("cx-ssh-owner".into())
        .spawn(move || record_owner(&owned));
}

/// An unlinked private file: the master and any ProxyJump child may keep the
/// descriptor for their whole life, so a pipe would never reach EOF.
fn private_scratch() -> Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    let path = crate::store::control_dir()?.join(format!(
        ".errors-{}-{}",
        std::process::id(),
        NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&path)?;
    std::fs::remove_file(&path)?;
    Ok(file)
}

/// OpenSSH's notice that it continues without a master socket.
fn mux_refused(stderr: &[u8]) -> bool {
    let text = String::from_utf8_lossy(stderr);
    text.contains("disabling multiplexing") || text.contains("ControlSocket")
}

/// Run a dedicated `-N` master in its own session so no cx process-group
/// reaping (or terminal hangup) can take down the master or its jump hops.
/// Blocks until the master detaches, fails, or MASTER_DEADLINE passes; only
/// the background attempt thread calls it.
fn establish_master(target: &str) -> MasterState {
    use std::os::unix::{fs::FileExt, process::CommandExt};
    let Ok(mut command) = crate::store::ssh_mode(target, crate::store::SshMode::Master) else {
        return MasterState::Unavailable;
    };
    let Ok(errors) = private_scratch() else {
        return MasterState::Unavailable;
    };
    let Ok(child_errors) = errors.try_clone() else {
        return MasterState::Unavailable;
    };
    // A long-lived master must not pin the caller's working directory.
    command
        .current_dir("/")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(child_errors);
    // SAFETY: setsid is async-signal-safe and touches no parent state.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let Ok(mut child) = command.spawn() else {
        return MasterState::Unavailable;
    };
    drop(command);
    let pid = child.id() as i32;
    let started = std::time::Instant::now();
    let mut next_check = started + std::time::Duration::from_secs(1);
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return MasterState::Ready,
            Ok(Some(_)) => {
                let mut sample = vec![0; 4096];
                let length = errors.read_at(&mut sample, 0).unwrap_or(0);
                sample.truncate(length);
                return match classify_connection_failure(&sample) {
                    failure @ (ConnectionFailure::Unreachable
                    | ConnectionFailure::Authentication
                    | ConnectionFailure::HostKey) => MasterState::Failed(failure),
                    _ if master_alive(target) => MasterState::Ready,
                    _ => MasterState::Unavailable,
                };
            }
            Ok(None) => {}
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return MasterState::Unavailable;
            }
        }
        let now = std::time::Instant::now();
        if now >= next_check {
            let mut sample = vec![0; 4096];
            let length = errors.read_at(&mut sample, 0).unwrap_or(0);
            if mux_refused(&sample[..length]) {
                // OpenSSH could not bind the socket and kept a plain
                // connection; it holds no master, so reap it and go direct.
                unsafe { libc::kill(-pid, libc::SIGKILL) };
                let _ = child.kill();
                let _ = child.wait();
                return if master_alive(target) {
                    MasterState::Ready
                } else {
                    MasterState::Unavailable
                };
            }
            if master_alive(target) {
                // Another process won the race; ours is an idle mux client
                // (it holds no jump hop), so stop only that process.
                unsafe { libc::kill(pid, libc::SIGTERM) };
                if wait_bounded(&mut child, std::time::Duration::from_secs(1)).is_none() {
                    let _ = child.wait();
                }
                return MasterState::Ready;
            }
            next_check = now + std::time::Duration::from_millis(250);
        }
        if now.duration_since(started) > MASTER_DEADLINE {
            // No master detached: reap the whole attempt, jump hops included.
            unsafe { libc::kill(-pid, libc::SIGKILL) };
            let _ = child.kill();
            let _ = child.wait();
            return MasterState::Failed(ConnectionFailure::Timeout);
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

/// Join `target`'s in-flight master attempt, or start one on a background
/// thread. None while a recent attempt's failure is cooling down.
fn start_master(target: &str) -> Option<std::sync::Arc<Flight>> {
    let mut m = masters()?;
    if let Some(flight) = m.flights.get(target) {
        return Some(flight.clone());
    }
    let now = std::time::Instant::now();
    m.failed
        .retain(|_, at| now.saturating_duration_since(*at) < MASTER_COOLDOWN);
    if m.failed.contains_key(target) {
        return None;
    }
    let flight = std::sync::Arc::new(Flight::default());
    m.flights.insert(target.to_owned(), flight.clone());
    drop(m);
    let owned = target.to_owned();
    let running = flight.clone();
    let spawned = std::thread::Builder::new()
        .name("cx-ssh-master".into())
        .spawn(move || {
            let state = establish_master(&owned);
            if state == MasterState::Ready {
                record_owner(&owned);
            }
            if let Some(mut m) = masters() {
                m.flights.remove(&owned);
                if matches!(state, MasterState::Failed(_)) {
                    if m.failed.len() >= 128 {
                        m.failed.clear();
                    }
                    m.failed.insert(owned.clone(), std::time::Instant::now());
                } else {
                    m.failed.remove(&owned);
                }
            }
            if let Ok(mut slot) = running.state.lock() {
                *slot = Some(state);
            }
            running.done.notify_all();
        });
    if spawned.is_err() {
        if let Some(mut m) = masters() {
            m.flights.remove(target);
        }
        return None;
    }
    Some(flight)
}

/// The attempt's outcome, or None when it is still running after `limit`.
fn await_flight(flight: &Flight, limit: std::time::Duration) -> Option<MasterState> {
    let state = flight.state.lock().ok()?;
    let (state, _) = flight
        .done
        .wait_timeout_while(state, limit, |state| state.is_none())
        .ok()?;
    *state
}

/// Make sure `target` has a live master, waiting at most `wait` for one that
/// is starting. Never blocks longer than `wait` plus two local `ssh -O`/`-G`
/// queries; an attempt that needs longer keeps running in the background.
fn ensure_master(target: &str, wait: std::time::Duration) -> MasterState {
    if !crate::store::multiplex_enabled() {
        return MasterState::Unavailable;
    }
    if masters().is_some_and(|m| {
        m.confirmed
            .get(target)
            .is_some_and(|until| std::time::Instant::now() < *until)
    }) {
        return MasterState::Ready;
    }
    let state = if master_alive(target) {
        record_owner_later(target);
        MasterState::Ready
    } else if let Some(flight) = start_master(target) {
        await_flight(&flight, wait).unwrap_or(MasterState::Unavailable)
    } else {
        MasterState::Unavailable
    };
    if let Some(mut m) = masters() {
        if state == MasterState::Ready {
            if m.confirmed.len() >= 256 {
                m.confirmed.clear();
            }
            m.confirmed
                .insert(target.to_owned(), std::time::Instant::now() + CONFIRMED_FOR);
        } else {
            m.confirmed.remove(target);
        }
    }
    state
}

/// Used before a terminal's ssh is built: start or join the route's
/// dedicated master for at most `wait`. The terminal then joins it, or
/// connects directly (and may prompt) when it is not ready; it never becomes
/// the master itself. Returns whether a master is ready.
pub(crate) fn warm_master(target: &str, wait: std::time::Duration) -> bool {
    valid_target(target) && ensure_master(target, wait) == MasterState::Ready
}

/// Establish `device`'s shared SSH master in the background, e.g. on hover or
/// focus, so the next helper request or attach skips the handshake. Never
/// blocks or prompts (BatchMode), is idempotent while a master is live or an
/// attempt is in flight, runs at most four prewarms at once, backs off for
/// twenty seconds after a failed attempt, and each attempt ends within
/// twenty-five seconds. Returns whether a background prewarm was started.
pub fn prewarm(device: &Device) -> bool {
    let Some(target) = device.target.clone() else {
        return false;
    };
    if !valid_target(&target) || !crate::store::multiplex_enabled() {
        return false;
    }
    {
        let Some(mut m) = masters() else {
            return false;
        };
        let now = std::time::Instant::now();
        let cooling = m
            .failed
            .get(&target)
            .is_some_and(|at| now.saturating_duration_since(*at) < MASTER_COOLDOWN);
        if m.flights.contains_key(&target) || cooling || m.prewarming >= PREWARM_LIMIT {
            return false;
        }
        m.prewarming += 1;
    }
    let finished = || {
        if let Some(mut m) = masters() {
            m.prewarming = m.prewarming.saturating_sub(1);
        }
    };
    let started = std::thread::Builder::new()
        .name("cx-ssh-prewarm".into())
        .spawn(move || {
            ensure_master(&target, std::time::Duration::ZERO);
            finished();
        })
        .is_ok();
    if !started {
        finished();
    }
    started
}

/// `cx prewarm`: confirm `device`'s shared master really opens channels
/// (replacing one that does not), else establish one, and wait for the
/// outcome, bounded by the master attempt deadline. Returns whether an
/// unresponsive master was replaced.
pub fn prewarm_blocking(device: &Device) -> Result<bool> {
    let target = device
        .target
        .as_deref()
        .context("the local device needs no SSH connection")?;
    if !valid_target(target) {
        bail!("invalid SSH target")
    }
    // A refused edge or route hop gets no connection, warm or not.
    crate::edges::guard(device)?;
    anyhow::ensure!(
        crate::store::multiplex_enabled(),
        "SSH connection sharing is turned off (CX_SSH_MULTIPLEX)"
    );
    let replaced = stop_stuck_master(target);
    // The same background start the UI uses on hover; the wait below joins it.
    prewarm(device);
    match ensure_master(target, MASTER_DEADLINE + std::time::Duration::from_secs(2)) {
        MasterState::Ready => Ok(replaced),
        MasterState::Failed(failure) => Err(failure.into()),
        MasterState::Unavailable => {
            bail!("SSH connection sharing unavailable for this device; connections stay direct")
        }
    }
}

/// A trivial channel through the master; false when it cannot open in time.
fn master_probe(target: &str) -> bool {
    use std::os::unix::process::CommandExt;
    let Ok(mut command) = crate::store::ssh_mode(target, crate::store::SshMode::Background) else {
        return false;
    };
    command
        .arg("exit 0")
        .process_group(0)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let Ok(mut child) = command.spawn() else {
        return false;
    };
    let pid = child.id() as i32;
    let ok = wait_bounded(&mut child, PROBE_LIMIT) == Some(true);
    // Reap the probe's group (a direct fallback may hold a jump hop).
    unsafe { libc::kill(-pid, libc::SIGKILL) };
    ok
}

/// Stop a master that answers `-O check` but cannot open a trivial channel
/// in time; returns whether one was stopped.
fn stop_stuck_master(target: &str) -> bool {
    let stopped = master_alive(target) && !master_probe(target) && master_control(target, "stop");
    if stopped {
        if let Some(mut m) = masters() {
            m.confirmed.remove(target);
        }
    }
    stopped
}

/// After a request timeout: a master whose network silently died still
/// answers `-O check` until ServerAlive ends it. Probe it with a trivial
/// channel (bounded) and, when that cannot open, stop it from accepting new
/// clients (`-O stop`) so the next request starts a fresh master at once.
/// Sessions already on the stopped master are not cut; a dead link ends them
/// by itself. Runs on a background thread, once per target at a time.
fn recover_master(target: &str) {
    if !crate::store::multiplex_enabled() {
        return;
    }
    if !masters()
        .is_some_and(|mut m| m.recovering.len() < 64 && m.recovering.insert(target.to_owned()))
    {
        return;
    }
    let owned = target.to_owned();
    let spawned = std::thread::Builder::new()
        .name("cx-ssh-recover".into())
        .spawn(move || {
            stop_stuck_master(&owned);
            if let Some(mut m) = masters() {
                m.recovering.remove(&owned);
            }
        });
    if spawned.is_err() {
        if let Some(mut m) = masters() {
            m.recovering.remove(target);
        }
    }
}

/// Receives helper events: the SSH target (None for this host) and the
/// event's `{kind, ...}` result. Set once by the viewer.
pub type EventSink = Box<dyn Fn(Option<String>, serde_json::Value) + Send + Sync>;
static EVENTS: std::sync::OnceLock<EventSink> = std::sync::OnceLock::new();
pub fn set_event_sink(sink: EventSink) {
    let _ = EVENTS.set(sink);
}
/// Deliver one event; dropped when no viewer listens.
pub(crate) fn deliver_event(target: Option<String>, event: serde_json::Value) {
    if let Some(sink) = EVENTS.get() {
        sink(target, event);
    }
}
/// Pushed frames carry `evt:<n>` ids; replies carry the request's id.
fn is_event(response: &Response) -> bool {
    response.version == 1 && response.id.starts_with("evt:")
}
struct Connection {
    child: std::process::Child,
    input: std::process::ChildStdin,
    replies: std::sync::mpsc::Receiver<Result<Response>>,
    failure: std::sync::mpsc::Receiver<ConnectionFailure>,
    route: Vec<String>,
    /// Opened while the route's master was ready, so it rides the master.
    shared: bool,
    /// Set once a watch was requested: the reader reports `closed` when the
    /// channel ends, so the viewer polls again.
    watching: std::sync::Arc<std::sync::atomic::AtomicBool>,
}
impl Drop for Connection {
    fn drop(&mut self) {
        unsafe {
            libc::kill(-(self.child.id() as i32), libc::SIGKILL);
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
impl Connection {
    fn open(target: &str) -> Result<Self> {
        // Join (or start) the route's shared master, waiting only briefly so
        // the operation's own timeout still covers a direct handshake. A
        // typed failure is the same evidence a direct BatchMode attempt gives.
        let master = ensure_master(target, CHANNEL_MASTER_WAIT);
        if let MasterState::Failed(failure) = master {
            return Err(failure.into());
        }
        let route = crate::store::route(target)?;
        let mut c = ssh(target, false)?;
        anyhow::ensure!(
            crate::store::route(target)? == route,
            "SSH route changed during connection setup"
        );
        c.arg("exec ~/.local/bin/cx helper");
        let mut connection = Self::from_command(c, Some(target.to_owned()))?;
        connection.route = route;
        connection.shared = master == MasterState::Ready;
        Ok(connection)
    }
    fn from_command(mut c: Command, target: Option<String>) -> Result<Self> {
        use std::os::unix::process::CommandExt;
        c.process_group(0)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = c.spawn()?;
        let input = child.stdin.take().context("helper input")?;
        let out = child.stdout.take().context("helper output")?;
        let mut stderr = child.stderr.take().context("helper error output")?;
        let (failure_tx, failure) = std::sync::mpsc::sync_channel(1);
        std::thread::spawn(move || {
            // Retain only a bounded sample for classification. Never expose startup
            // text, paths, prompts or credential material in UI errors or logs.
            let mut sample = Vec::new();
            let mut chunk = [0; 512];
            while let Ok(n) = stderr.read(&mut chunk) {
                if n == 0 {
                    break;
                }
                let keep = n.min(4096usize.saturating_sub(sample.len()));
                sample.extend_from_slice(&chunk[..keep]);
            }
            let _ = failure_tx.send(classify_connection_failure(&sample));
        });
        let (tx, replies) = std::sync::mpsc::sync_channel(4);
        let watching = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let watched = watching.clone();
        std::thread::spawn(move || {
            let mut reader = BufReader::new(out);
            loop {
                let result = read_frame::<Response>(&mut reader);
                if let Some(event) = result.as_ref().ok().filter(|r| is_event(r)) {
                    // Events never answer a request; only a viewer channel gets them.
                    if let (Some(target), Some(value)) = (&target, event.result.clone()) {
                        deliver_event(Some(target.clone()), value);
                    }
                    continue;
                }
                let failed = result.is_err();
                if tx.send(result).is_err() || failed {
                    break;
                }
            }
            if watched.load(std::sync::atomic::Ordering::Relaxed) {
                if let Some(target) = target {
                    deliver_event(Some(target), serde_json::json!({"kind": "closed"}));
                }
            }
        });
        Ok(Self {
            child,
            input,
            replies,
            failure,
            route: Vec::new(),
            shared: false,
            watching,
        })
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConnectionFailure {
    Unreachable,
    Authentication,
    HostKey,
    MissingHelper,
    Closed,
    Timeout,
}
fn classify_connection_failure(stderr: &[u8]) -> ConnectionFailure {
    let text = String::from_utf8_lossy(stderr);
    if text.contains("Host key verification failed")
        || text.contains("REMOTE HOST IDENTIFICATION HAS CHANGED")
    {
        ConnectionFailure::HostKey
    } else if text.contains("Permission denied")
        || text.contains("sign_and_send_pubkey: signing failed")
    {
        ConnectionFailure::Authentication
    } else if [
        "No route to host",
        "Connection refused",
        "Connection timed out",
        "Could not resolve hostname",
        "Network is unreachable",
    ]
    .iter()
    .any(|message| text.contains(message))
    {
        ConnectionFailure::Unreachable
    } else if text.contains("cx")
        && (text.contains("not found") || text.contains("No such file or directory"))
    {
        ConnectionFailure::MissingHelper
    } else {
        ConnectionFailure::Closed
    }
}
impl std::fmt::Display for ConnectionFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Unreachable => "Device unreachable over SSH · check its connection, then Refresh",
            Self::Authentication => "SSH access unavailable · unlock or authenticate, then Refresh",
            Self::HostKey => "SSH host identity could not be verified · review the host key before reconnecting",
            Self::MissingHelper => "cx helper unavailable on device · run cx add to repair enrollment",
            Self::Closed => "Session metadata connection closed · Refresh to reconnect; terminal state is unconfirmed",
            Self::Timeout => "Device check timed out · Refresh to retry; terminal state is unconfirmed",
        })
    }
}
impl std::error::Error for ConnectionFailure {}

/// Typed connection evidence, distinct from an operation rejected by a reachable helper.
pub(crate) fn is_connection_failure(error: &anyhow::Error) -> bool {
    error.downcast_ref::<ConnectionFailure>().is_some()
}
impl Connection {
    fn failure_reason(&self) -> ConnectionFailure {
        self.failure
            .recv_timeout(std::time::Duration::from_millis(100))
            .unwrap_or(ConnectionFailure::Closed)
    }
}
fn reconnectable_observation(op: &Operation) -> bool {
    matches!(
        op,
        Operation::Info | Operation::Sessions | Operation::ExternalSessions
    )
}
fn retry_observation<T>(op: &Operation, mut attempt: impl FnMut() -> Result<T>) -> Result<T> {
    let result = attempt();
    if reconnectable_observation(op)
        && result.as_ref().err().is_some_and(|e| {
            e.downcast_ref::<ConnectionFailure>() == Some(&ConnectionFailure::Closed)
        })
    {
        // The failed metadata channel was discarded. Never retry a mutation,
        // authentication/trust failure, offline endpoint or timed-out request.
        attempt()
    } else {
        result
    }
}
static CONNECTIONS: std::sync::LazyLock<
    std::sync::Mutex<
        std::collections::HashMap<String, std::sync::Arc<std::sync::Mutex<Option<Connection>>>>,
    >,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));
static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
/// One framed metadata channel per endpoint, separate from native PTY traffic.
pub fn request(d: &Device, op: Operation) -> Result<serde_json::Value> {
    retry_observation(&op, || request_once(d, op.clone()))
}
fn request_once(d: &Device, op: Operation) -> Result<serde_json::Value> {
    let Some(target) = &d.target else {
        return crate::dispatch(op);
    };
    if !valid_target(target) {
        bail!("invalid SSH target")
    }
    // The UI, transfers and helper-relayed requests all pass here: a refused
    // edge opens no SSH. Edge enforcement uses verified_request instead.
    crate::edges::guard(d)?;
    let entry = {
        let mut all = CONNECTIONS
            .lock()
            .map_err(|_| anyhow!("transport lock unavailable"))?;
        let channel = match channel_class(&op) {
            Channel::Interactive => format!("session-preview:{target}"),
            Channel::Capabilities => format!("capabilities:{target}"),
            Channel::Bulk => format!("bulk:{target}"),
            Channel::Metadata => target.clone(),
        };
        all.entry(channel)
            .or_insert_with(|| std::sync::Arc::new(std::sync::Mutex::new(None)))
            .clone()
    };
    let mut slot = entry
        .lock()
        .map_err(|_| anyhow!("connection lock unavailable"))?;
    let route = crate::store::route(target)?;
    if slot.as_ref().is_some_and(|c| c.route != route) {
        // Future requests must not reuse a channel opened through an old gateway.
        // This does not terminate separately attached interactive sessions.
        *slot = None;
    }
    if slot
        .as_mut()
        .is_some_and(|c| !matches!(c.child.try_wait(), Ok(None)))
    {
        *slot = None;
    }
    if slot.is_none() {
        *slot = Some(Connection::open(target)?);
    }
    let response_timeout = if channel_class(&op) == Channel::Interactive {
        3
    } else if matches!(
        &op,
        Operation::DevcontainerUp { .. } | Operation::DevcontainerRebuild { .. }
    ) {
        660
    } else if matches!(
        &op,
        Operation::ContainerFiles { .. }
            | Operation::ContainerLifecycle { .. }
            | Operation::ContainerCreate { .. }
            | Operation::ContainerAgentCreate { .. }
            | Operation::ContainerProviders { .. }
            | Operation::Containers
            | Operation::Sessions
            | Operation::ContainerFileAction { .. }
    ) {
        90
    } else {
        15
    };
    let is_info = matches!(&op, Operation::Info);
    let result = exchange(&mut slot, op, response_timeout);
    if result.is_ok() && slot.as_ref().is_some_and(|c| c.shared) {
        if let Some(mut m) = masters() {
            let until = std::time::Instant::now() + ANSWERED_FOR;
            if m.confirmed.get(target).is_none_or(|current| *current < until) {
                m.confirmed.insert(target.clone(), until);
            }
        }
    }
    drop(slot);
    if let Err(error) = &result {
        if error.downcast_ref::<ConnectionFailure>() == Some(&ConnectionFailure::Timeout) {
            recover_master(target);
        }
    }
    let result = result?;
    if is_info {
        schedule_host_update(target, &result);
    }
    Ok(result)
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Channel {
    /// Screens, keystrokes and the watch: never queued behind anything else.
    Interactive,
    Capabilities,
    /// Files, containers and transfers: long bursts that must not delay sessions.
    Bulk,
    /// Session inventory and everything else.
    Metadata,
}
/// Which of a device's helper channels (one helper process each, all over
/// the same SSH master) carries `op`.
pub(crate) fn channel_class(op: &Operation) -> Channel {
    match op {
        Operation::SessionPreview { .. }
        | Operation::SessionPaneInput { .. }
        | Operation::Watch { .. } => Channel::Interactive,
        Operation::Info => Channel::Capabilities,
        Operation::Containers
        | Operation::ContainerInspect { .. }
        | Operation::ContainerFiles { .. }
        | Operation::ContainerFileAction { .. }
        | Operation::ContainerProviders { .. }
        | Operation::List { .. }
        | Operation::ListPage { .. }
        | Operation::Preview { .. }
        | Operation::PreviewPage { .. }
        | Operation::ReadChunk { .. }
        | Operation::FileInfo { .. }
        | Operation::ReceivePrepare { .. }
        | Operation::ReceiveChunk { .. }
        | Operation::ReceiveFinalize { .. }
        | Operation::ReceiveSymlink { .. }
        | Operation::Jobs
        | Operation::TransferJobs => Channel::Bulk,
        _ => Channel::Metadata,
    }
}
/// Reuse a private framed helper channel. Callers validate their execution scope before every request.
pub(crate) fn owned_helper_request(
    key: &str,
    command: impl FnOnce() -> Result<Command>,
    op: Operation,
) -> Result<serde_json::Value> {
    static OWNED: std::sync::LazyLock<
        std::sync::Mutex<std::collections::HashMap<String, Option<Connection>>>,
    > = std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));
    let mut channels = OWNED
        .lock()
        .map_err(|_| anyhow!("owned helper lock unavailable"))?;
    if !channels.contains_key(key) && channels.len() >= 32 {
        if let Some(old) = channels.keys().next().cloned() {
            channels.remove(&old);
        }
    }
    let slot = channels.entry(key.into()).or_default();
    if slot
        .as_mut()
        .is_some_and(|c| !matches!(c.child.try_wait(), Ok(None)))
    {
        *slot = None;
    }
    if slot.is_none() {
        *slot = Some(Connection::from_command(command()?, None)?);
    }
    exchange(slot, op, 20)
}
fn exchange(
    slot: &mut Option<Connection>,
    op: Operation,
    response_timeout: u64,
) -> Result<serde_json::Value> {
    let id = format!(
        "{}-{}",
        std::process::id(),
        NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    );
    let req = Request {
        version: 1,
        id: id.clone(),
        op,
    };
    let conn = slot.as_mut().context("helper channel unavailable")?;
    if let Operation::Watch { sessions, pane } = &req.op {
        conn.watching.store(
            *sessions || pane.is_some(),
            std::sync::atomic::Ordering::Relaxed,
        );
    }
    if let Err(error) = frame(&mut conn.input, &req) {
        let reason = conn.failure_reason();
        *slot = None;
        return Err(error).context(reason);
    }
    let response = match conn
        .replies
        .recv_timeout(std::time::Duration::from_secs(response_timeout))
    {
        Ok(Ok(r)) => r,
        Ok(Err(e)) => {
            let reason = conn.failure_reason();
            *slot = None;
            return Err(e).context(reason);
        }
        Err(_) => {
            *slot = None;
            return Err(ConnectionFailure::Timeout.into());
        }
    };
    if response.version != 1 || response.id != id {
        *slot = None;
        bail!("incompatible or mismatched helper response")
    }
    if let Some(e) = response.error {
        bail!("{e}")
    }
    response
        .result
        .ok_or_else(|| anyhow!("empty helper response"))
}
/// A verified helper replacement; consumers refresh observations, never replay mutations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostUpdate {
    pub target: String,
    pub version: String,
}
const UPDATE_OUTPUT_LIMIT: usize = 64 * 1024;
const UPDATE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(240);
const UPDATE_MODERN: &str = "exec ~/.local/bin/cx update --automatic --json";
const UPDATE_LEGACY: &str = "exec ~/.local/bin/cx update";
struct UpdateAttempt {
    running: bool,
    finished: Option<std::time::Instant>,
    successful: bool,
}
#[derive(Default)]
struct HostUpdates {
    attempts: std::collections::HashMap<String, UpdateAttempt>,
    events: std::collections::VecDeque<HostUpdate>,
}
impl HostUpdates {
    fn reserve(&mut self, target: &str, now: std::time::Instant) -> bool {
        self.attempts.retain(|_, attempt| {
            attempt.running
                || attempt.finished.is_some_and(|time| {
                    now.saturating_duration_since(time)
                        < std::time::Duration::from_secs(if attempt.successful {
                            3600
                        } else {
                            300
                        })
                })
        });
        if self.attempts.contains_key(target)
            || self.attempts.len() >= 128
            || self.attempts.values().filter(|a| a.running).count() >= 2
        {
            return false;
        }
        self.attempts.insert(
            target.to_owned(),
            UpdateAttempt {
                running: true,
                finished: None,
                successful: false,
            },
        );
        true
    }
    fn finish(
        &mut self,
        target: &str,
        now: std::time::Instant,
        successful: bool,
        version: Option<String>,
    ) {
        if let Some(attempt) = self.attempts.get_mut(target) {
            attempt.running = false;
            attempt.finished = Some(now);
            attempt.successful = successful;
        }
        if let Some(version) = version {
            if self.events.len() == 64 {
                self.events.pop_front();
            }
            self.events.push_back(HostUpdate {
                target: target.to_owned(),
                version,
            });
        }
    }
}
static HOST_UPDATES: std::sync::LazyLock<std::sync::Mutex<HostUpdates>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(HostUpdates::default()));
pub fn drain_host_updates() -> Vec<HostUpdate> {
    HOST_UPDATES
        .lock()
        .map(|mut state| state.events.drain(..).collect())
        .unwrap_or_default()
}
fn stable_version(version: &str) -> Option<[u64; 3]> {
    if version.len() > 62 {
        return None;
    }
    let mut parts = version.split('.');
    let mut result = [0; 3];
    for number in &mut result {
        let part = parts.next()?;
        if part.is_empty()
            || !part.bytes().all(|b| b.is_ascii_digit())
            || (part.len() > 1 && part.starts_with('0'))
        {
            return None;
        }
        *number = part.parse().ok()?;
    }
    if parts.next().is_some() {
        return None;
    }
    Some(result)
}
fn modern_update(info: &serde_json::Value) -> bool {
    let version = info["version"].as_str().and_then(stable_version);
    version
        .zip(stable_version(env!("CARGO_PKG_VERSION")))
        .is_some_and(|(host, viewer)| host >= viewer)
        && info["capabilities"].as_array().is_some_and(|caps| {
            caps.iter()
                .any(|cap| cap.as_str() == Some("stable-update-v1"))
        })
}
fn upgraded_version(before: &serde_json::Value, after: &serde_json::Value) -> Option<String> {
    let old = before["version"].as_str().and_then(stable_version)?;
    let raw = after["version"].as_str()?;
    if stable_version(raw)? <= old {
        return None;
    }
    if let Some(machine) = before["machine_id"].as_str().filter(|m| !m.is_empty()) {
        if after["machine_id"].as_str() != Some(machine) {
            return None;
        }
    }
    Some(raw.to_owned())
}
fn schedule_host_update(target: &str, info: &serde_json::Value) {
    if info["version"].as_str().and_then(stable_version).is_none() {
        return;
    }
    let Ok(mut state) = HOST_UPDATES.try_lock() else {
        return;
    };
    if !state.reserve(target, std::time::Instant::now()) {
        return;
    }
    drop(state);
    let target = target.to_owned();
    let before = info.clone();
    // Only authenticated successful Info observations reach here. No update is on the request path.
    let spawn_target = target.clone();
    if std::thread::Builder::new()
        .name("cx-host-update".into())
        .spawn(move || {
            let outcome = update_host(&target, &before);
            let (successful, version) = outcome.unwrap_or((false, None));
            if let Ok(mut state) = HOST_UPDATES.lock() {
                state.finish(&target, std::time::Instant::now(), successful, version);
            }
        })
        .is_err()
    {
        if let Ok(mut state) = HOST_UPDATES.lock() {
            state.finish(&spawn_target, std::time::Instant::now(), false, None);
        }
    }
}
/// Detached from the cached channel so check/reconciliation cannot consume its replies.
/// Read-only verification without automatic updates or shared cached channels.
pub(crate) fn probe_info(target: &str) -> Result<serde_json::Value> {
    let mut connection = Some(Connection::open(target)?);
    exchange(&mut connection, Operation::Info, 15)
}

/// Keep identity/capability verification and its mutation on one SSH connection.
pub(crate) fn verified_request(
    target: &str,
    op: Operation,
    verify: impl FnOnce(&serde_json::Value) -> Result<()>,
) -> Result<serde_json::Value> {
    let mut connection = Some(Connection::open(target)?);
    let info = exchange(&mut connection, Operation::Info, 15)?;
    verify(&info)?;
    exchange(&mut connection, op, 30)
}

fn fresh_info(target: &str) -> Result<serde_json::Value> {
    let mut connection = Connection::open(target)?;
    let id = format!(
        "update-{}-{}",
        std::process::id(),
        NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    );
    frame(
        &mut connection.input,
        &Request {
            version: 1,
            id: id.clone(),
            op: Operation::Info,
        },
    )?;
    let response = connection
        .replies
        .recv_timeout(std::time::Duration::from_secs(15))
        .context("update verification timed out")??;
    if response.version != 1 || response.id != id || response.error.is_some() {
        bail!("update verification failed");
    }
    response.result.context("empty update verification")
}
/// Explicit CLI update shares the automatic authenticated, architecture-aware path.
pub fn update_device(device: &Device) -> Result<serde_json::Value> {
    let target = device
        .target
        .as_deref()
        .context("Use cx update for the local device")?;
    let before = fresh_info(target)?;
    let (successful, upgraded) = update_host(target, &before)?;
    Ok(serde_json::json!({
        "state": if upgraded.is_some() { "updated" } else if successful { "current" } else { "unavailable" },
        "version": upgraded.as_deref().or_else(|| before["version"].as_str()).unwrap_or("unknown")
    }))
}
fn update_host(target: &str, before: &serde_json::Value) -> Result<(bool, Option<String>)> {
    let modern = modern_update(before);
    let mut command = ssh(target, false)?;
    command.arg(if modern { UPDATE_MODERN } else { UPDATE_LEGACY });
    let direct = update_host_with(target, before, command, || fresh_info(target));
    match direct {
        Ok((true, version)) => Ok((true, version)),
        _ => mediated_host_update(target, before),
    }
}
fn mediated_identity(before: &serde_json::Value, after: &serde_json::Value) -> bool {
    ["machine_id", "account"].iter().all(|field| {
        before[*field]
            .as_str()
            .is_some_and(|v| !v.is_empty() && after[*field].as_str() == Some(v))
    })
}
fn mediated_arch(bytes: &[u8]) -> Result<&'static str> {
    match std::str::from_utf8(bytes)?.trim() {
        "Linux aarch64" => Ok("aarch64"),
        "Linux x86_64" => Ok("x86_64"),
        _ => bail!("unsupported remote execution platform"),
    }
}
fn mediated_version(before: &serde_json::Value, version: &str) -> Result<bool> {
    let old = before["version"]
        .as_str()
        .and_then(stable_version)
        .context("remote version unavailable")?;
    let new = stable_version(version).context("release version unavailable")?;
    Ok(new > old)
}
/// A mesh viewer can supply a verified artifact even when the robot has no public egress.
/// This never enrolls a new host or changes trust, profiles, packages or network state.
fn mediated_host_update(
    target: &str,
    before: &serde_json::Value,
) -> Result<(bool, Option<String>)> {
    let observed = fresh_info(target)?;
    anyhow::ensure!(
        mediated_identity(before, &observed),
        "remote update identity changed"
    );
    let mut platform = ssh(target, false)?;
    platform.arg("uname -sm");
    let platform = run_bounded_update(platform, std::time::Duration::from_secs(20), 4096)?;
    anyhow::ensure!(platform.success, "remote platform check failed");
    let arch = mediated_arch(&platform.stdout)?;
    let current = observed["version"]
        .as_str()
        .context("remote version unavailable")?;
    let Some(artifact) = crate::update::obtain_remote_update_binary(arch, current)? else {
        let version = upgraded_version(before, &observed);
        if version.is_some() {
            invalidate_metadata(target)?;
        }
        return Ok((true, version));
    };
    anyhow::ensure!(
        mediated_version(&observed, &artifact.version)?,
        "remote update would not advance"
    );
    let input = std::fs::File::open(artifact.path())?;
    let script = include_str!("../scripts/enroll-helper.sh").replace('\'', "'\"'\"'");
    let mut transfer = ssh(target, false)?;
    transfer.arg(format!(
        "CX_ENROLL_MINIMAL=1 CX_ENROLL_UPDATE_VERSION={} sh -c '{script}'",
        artifact.version
    ));
    let installed = run_bounded_update_input(
        transfer,
        UPDATE_TIMEOUT,
        UPDATE_OUTPUT_LIMIT,
        Stdio::from(input),
    )?;
    anyhow::ensure!(installed.success, "verified remote update transfer failed");
    let after = fresh_info(target)?;
    anyhow::ensure!(
        mediated_identity(&observed, &after)
            && after["version"].as_str() == Some(&artifact.version),
        "remote update verification failed"
    );
    let version = upgraded_version(before, &after).context("remote update did not advance")?;
    invalidate_metadata(target)?;
    Ok((true, Some(version)))
}
/// Replace the target's helper channels (metadata, bulk, interactive) after
/// its helper changed; each waits for its in-flight request first.
fn invalidate_metadata(target: &str) -> Result<()> {
    let entries: Vec<_> = {
        let all = CONNECTIONS
            .lock()
            .map_err(|_| anyhow!("transport lock unavailable"))?;
        [
            target.to_owned(),
            format!("bulk:{target}"),
            format!("session-preview:{target}"),
        ]
        .iter()
        .filter_map(|key| all.get(key).cloned())
        .collect()
    };
    for entry in entries {
        *entry
            .lock()
            .map_err(|_| anyhow!("connection lock unavailable"))? = None;
    }
    Ok(())
}
fn update_host_with(
    target: &str,
    before: &serde_json::Value,
    command: Command,
    fresh: impl FnOnce() -> Result<serde_json::Value>,
) -> Result<(bool, Option<String>)> {
    let modern = modern_update(before);
    let output = run_bounded_update(command, UPDATE_TIMEOUT, UPDATE_OUTPUT_LIMIT)?;
    if !output.success {
        return Ok((false, None));
    }
    let claim = update_claim(modern, &output)?;
    if claim == UpdateClaim::Unavailable {
        return Ok((false, None));
    }
    // Disk CLI 'current' can still mean our cached helper is an older live executable.
    let after = fresh()?;
    let Some(version) = upgraded_version(before, &after) else {
        // A stable equal-version Info can confirm current, but never emits an upgrade event.
        let current = claim == UpdateClaim::Current
            && before["version"] == after["version"]
            && after["version"].as_str().and_then(stable_version).is_some()
            && before["machine_id"] == after["machine_id"];
        return Ok((current, None));
    };
    // Wait for pending requests (including mutations), then replace the helper channels.
    invalidate_metadata(target)?;
    Ok((true, Some(version)))
}
#[derive(Debug, PartialEq, Eq)]
enum UpdateClaim {
    Current,
    Verify,
    Unavailable,
}
fn update_claim(modern: bool, output: &UpdateOutput) -> Result<UpdateClaim> {
    if !output.success {
        return Ok(UpdateClaim::Unavailable);
    }
    if !modern {
        return Ok(UpdateClaim::Verify);
    }
    let json: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    Ok(
        match json["state"].as_str().or_else(|| json["status"].as_str()) {
            Some("current") => UpdateClaim::Current,
            Some("updated") => UpdateClaim::Verify,
            _ => UpdateClaim::Unavailable,
        },
    )
}
#[derive(Debug)]
struct UpdateOutput {
    success: bool,
    stdout: Vec<u8>,
}
/// Output is never printed, and stderr is discarded rather than retained as evidence.
fn run_bounded_update(
    command: Command,
    timeout: std::time::Duration,
    limit: usize,
) -> Result<UpdateOutput> {
    run_bounded_update_input(command, timeout, limit, Stdio::null())
}
fn run_bounded_update_input(
    mut command: Command,
    timeout: std::time::Duration,
    limit: usize,
    input: Stdio,
) -> Result<UpdateOutput> {
    use std::os::unix::process::CommandExt;
    use std::sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    };
    command
        .stdin(input)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    let mut child = command.spawn().context("remote update unavailable")?;
    let count = Arc::new(AtomicUsize::new(0));
    let stdout = child.stdout.take().context("update output unavailable")?;
    let stderr = child
        .stderr
        .take()
        .context("update error output unavailable")?;
    // Nonblocking readers bound cleanup if a detached descendant retains a pipe.
    use std::os::fd::AsRawFd;
    for fd in [stdout.as_raw_fd(), stderr.as_raw_fd()] {
        if unsafe { libc::fcntl(fd, libc::F_SETFL, libc::O_NONBLOCK) } < 0 {
            unsafe {
                libc::kill(-(child.id() as i32), libc::SIGKILL);
            }
            let _ = child.kill();
            let _ = child.wait();
            bail!("cannot bound update output reader");
        }
    }
    let stopping = Arc::new(AtomicBool::new(false));
    let reader = |mut stream: Box<dyn Read + Send>,
                  keep: bool,
                  count: Arc<AtomicUsize>,
                  stopping: Arc<AtomicBool>| {
        std::thread::spawn(move || {
            let mut saved = Vec::new();
            let mut buffer = [0; 4096];
            loop {
                let n = match stream.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(n) => n,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        if stopping.load(Ordering::Relaxed) {
                            break;
                        }
                        std::thread::sleep(std::time::Duration::from_millis(5));
                        continue;
                    }
                    Err(_) => break,
                };
                let old = count.fetch_add(n, Ordering::Relaxed);
                if keep && old < limit {
                    saved.extend_from_slice(&buffer[..n.min(limit - old)]);
                }
                if old.saturating_add(n) > limit {
                    break;
                }
            }
            saved
        })
    };
    let out = reader(Box::new(stdout), true, count.clone(), stopping.clone());
    let err = reader(Box::new(stderr), false, count.clone(), stopping.clone());
    let started = std::time::Instant::now();
    let outcome = loop {
        if count.load(Ordering::Relaxed) > limit {
            break Err(anyhow!("update output exceeded limit"));
        }
        if started.elapsed() >= timeout {
            break Err(anyhow!("remote update timed out"));
        }
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status.success()),
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(10)),
            Err(_) => break Err(anyhow!("remote update wait failed")),
        }
    };
    // An exited shell can leave descendants holding the pipes. Kill our isolated group on every path.
    unsafe {
        libc::kill(-(child.id() as i32), libc::SIGKILL);
    }
    let _ = child.kill();
    let _ = child.wait();
    stopping.store(true, Ordering::Relaxed);
    let stdout = out
        .join()
        .map_err(|_| anyhow!("update output reader failed"))?;
    let _ = err.join();
    let success = outcome?;
    if count.load(Ordering::Relaxed) > limit {
        bail!("update output exceeded limit");
    }
    Ok(UpdateOutput { success, stdout })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn helper_exit_with_inherited_stderr_is_bounded_and_owned_group_is_reaped() {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "sleep 30 >&2 & exit 0"]);
        let mut connection = Connection::from_command(command, None).unwrap();
        let start = std::time::Instant::now();
        assert!(connection
            .replies
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap()
            .is_err());
        assert_eq!(connection.failure_reason(), ConnectionFailure::Closed);
        assert!(start.elapsed() < std::time::Duration::from_secs(2));
        let (_, empty) = std::sync::mpsc::channel();
        let failure = std::mem::replace(&mut connection.failure, empty);
        drop(connection);
        assert_eq!(
            failure
                .recv_timeout(std::time::Duration::from_secs(2))
                .unwrap(),
            ConnectionFailure::Closed
        );
    }
    #[test]
    fn disconnected_observations_retry_once_but_mutations_never_replay() {
        for op in [Operation::Info, Operation::Sessions] {
            let mut attempts = 0;
            let value = retry_observation(&op, || {
                attempts += 1;
                if attempts == 1 {
                    Err(anyhow!(ConnectionFailure::Closed))
                } else {
                    Ok(42)
                }
            })
            .unwrap();
            assert_eq!((value, attempts), (42, 2));
            let mut attempts = 0;
            let result: Result<()> = retry_observation(&op, || {
                attempts += 1;
                Err(anyhow!(ConnectionFailure::Closed))
            });
            assert!(result.is_err());
            assert_eq!(attempts, 2);
        }
        for op in [
            Operation::Create(CreateSession {
                key: "fixture".into(),
                directory: "/tmp".into(),
                provider: "shell".into(),
                name: "fixture".into(),
                resume: false,
                resume_pick: false,
            }),
            Operation::StopSession {
                id: "fixture".into(),
                pid: 1,
                started: "fixture".into(),
                boot_id: "fixture".into(),
            },
            Operation::Cancel {
                key: "fixture".into(),
            },
        ] {
            let mut attempts = 0;
            let result: Result<()> = retry_observation(&op, || {
                attempts += 1;
                Err(anyhow!(ConnectionFailure::Closed))
            });
            assert!(result.is_err());
            assert_eq!(attempts, 1);
        }
    }
    #[test]
    fn ssh_failures_are_actionable_and_never_expose_stderr() {
        for (sample, expected) in [
            (
                "No route to host\nSYNTHETIC_SECRET",
                ConnectionFailure::Unreachable,
            ),
            (
                "Permission denied (publickey).\nSYNTHETIC_SECRET",
                ConnectionFailure::Authentication,
            ),
            (
                "Host key verification failed.\nSYNTHETIC_SECRET",
                ConnectionFailure::HostKey,
            ),
            (
                "cx: command not found\nSYNTHETIC_SECRET",
                ConnectionFailure::MissingHelper,
            ),
            (
                "unexpected startup SYNTHETIC_SECRET",
                ConnectionFailure::Closed,
            ),
        ] {
            let failure = classify_connection_failure(sample.as_bytes());
            assert_eq!(failure, expected);
            assert!(!failure.to_string().contains("SYNTHETIC_SECRET"));
            if failure != ConnectionFailure::Closed {
                let mut attempts = 0;
                let result: Result<()> = retry_observation(&Operation::Sessions, || {
                    attempts += 1;
                    Err(anyhow!(failure))
                });
                assert!(result.is_err());
                assert_eq!(attempts, 1);
            }
        }
    }
    #[test]
    fn actual_helper_stderr_is_bounded_and_classified() {
        let mut command = Command::new("/bin/sh");
        command.args([
            "-c",
            "printf 'No route to host\n' >&2; head -c 100000 /dev/zero >&2; exit 255",
        ]);
        let connection = Connection::from_command(command, None).unwrap();
        assert!(connection
            .replies
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap()
            .is_err());
        assert_eq!(connection.failure_reason(), ConnectionFailure::Unreachable);
    }
    #[test]
    fn master_attempts_recognise_refused_sockets_and_skip_local_devices() {
        assert!(mux_refused(
            b"ControlSocket /s/ssh/x already exists, disabling multiplexing\n"
        ));
        assert!(!mux_refused(b"Permission denied (publickey).\n"));
        // The local device has no SSH route, so prewarm never starts work.
        assert!(!prewarm(&crate::store::local_device()));
        let invalid = Device {
            target: Some("-oProxyCommand=evil".into()),
            ..crate::store::local_device()
        };
        assert!(!prewarm(&invalid));
    }
    #[test]
    fn bounded_frames() {
        let v = serde_json::json!({"x":"quoted\n☃"});
        let mut b = Vec::new();
        frame(&mut b, &v).unwrap();
        let got: serde_json::Value = read_frame(&mut BufReader::new(&b[..])).unwrap();
        assert_eq!(got, v);
        assert!(
            read_frame::<serde_json::Value>(&mut BufReader::new(&b"CX1 1048577\n"[..])).is_err()
        );
        assert!(read_frame::<serde_json::Value>(&mut BufReader::new(&b"CX1 -1\n"[..])).is_err());
    }
    #[test]
    fn startup_noise() {
        let b = b"welcome\nCX1 2\n{}";
        let v: serde_json::Value = read_frame(&mut BufReader::new(&b[..])).unwrap();
        assert!(v.is_object());
    }
    #[test]
    fn targets() {
        for s in ["-oProxyCommand=evil", "x;id", "x\ny", "$(id)", "a b"] {
            assert!(!valid_target(s))
        }
        assert!(valid_target("peace@host.example"));
    }
    #[test]
    fn update_coalescing_limits_and_cooldowns() {
        let now = std::time::Instant::now();
        let mut state = HostUpdates::default();
        assert!(state.reserve("a", now));
        assert!(!state.reserve("a", now));
        assert!(state.reserve("b", now));
        assert!(!state.reserve("c", now));
        state.finish("a", now, true, None);
        assert!(state.reserve("c", now));
        state.finish("b", now, false, None);
        assert!(!state.reserve("b", now + std::time::Duration::from_secs(299)));
        assert!(state.reserve("b", now + std::time::Duration::from_secs(300)));
        state.finish("b", now + std::time::Duration::from_secs(300), false, None);
        assert!(!state.reserve("a", now + std::time::Duration::from_secs(3599)));
        assert!(state.reserve("a", now + std::time::Duration::from_secs(3600)));
    }
    #[test]
    fn update_event_queue_bounded() {
        let mut state = HostUpdates::default();
        for n in 0..100 {
            state.finish(
                "a",
                std::time::Instant::now(),
                true,
                Some(format!("0.1.{n}")),
            );
        }
        assert_eq!(state.events.len(), 64);
        assert_eq!(state.events.front().unwrap().version, "0.1.36");
    }
    #[test]
    fn stable_versions_and_host_identity() {
        for invalid in [
            "0.1",
            "v0.1.9",
            "0.1.9-dev",
            "00.1.9",
            "0.1.+9",
            "1.2.3.4",
            "18446744073709551616.0.0",
        ] {
            assert!(stable_version(invalid).is_none(), "{invalid}");
        }
        assert_eq!(stable_version("12.30.10"), Some([12, 30, 10]));
        let before = serde_json::json!({"version":"0.1.9","machine_id":"fixture"});
        for version in ["0.1.9", "0.1.8", "0.2.0-beta", "01.2.0"] {
            assert!(upgraded_version(
                &before,
                &serde_json::json!({"version":version,"machine_id":"fixture"})
            )
            .is_none());
        }
        assert!(upgraded_version(
            &before,
            &serde_json::json!({"version":"0.2.0","machine_id":"other"})
        )
        .is_none());
        assert!(upgraded_version(&before, &serde_json::json!({"version":"0.2.0"})).is_none());
        assert_eq!(
            upgraded_version(
                &before,
                &serde_json::json!({"version":"0.2.0","machine_id":"fixture"})
            ),
            Some("0.2.0".into())
        );
    }
    #[test]
    fn mediated_artifact_transfer_uses_bounded_file_stdin() {
        let fixture = tempfile::tempdir().unwrap();
        let path = fixture.path().join("verified-artifact");
        std::fs::write(&path, vec![42; 2 * 1024 * 1024]).unwrap();
        let output = run_bounded_update_input(
            shell("wc -c"),
            std::time::Duration::from_secs(2),
            128,
            Stdio::from(std::fs::File::open(&path).unwrap()),
        )
        .unwrap();
        assert!(output.success);
        assert_eq!(
            std::str::from_utf8(&output.stdout).unwrap().trim(),
            "2097152"
        );
        assert!(run_bounded_update_input(
            shell("sleep 30"),
            std::time::Duration::from_millis(80),
            128,
            Stdio::from(std::fs::File::open(&path).unwrap()),
        )
        .is_err());
    }
    #[test]
    fn mediated_update_requires_same_identity_architecture_and_newer_version() {
        let before =
            serde_json::json!({"version":"0.1.32","machine_id":"robot","account":"roboboat"});
        let mut after = before.clone();
        after["version"] = "0.1.34".into();
        assert!(mediated_identity(&before, &before));
        assert_eq!(mediated_arch(b"Linux aarch64\n").unwrap(), "aarch64");
        assert_eq!(mediated_arch(b"Linux x86_64\n").unwrap(), "x86_64");
        for invalid in [
            b"Darwin arm64".as_slice(),
            b"Linux armv7l",
            b"banner\nLinux aarch64",
        ] {
            assert!(mediated_arch(invalid).is_err());
        }
        assert!(mediated_version(&before, "0.1.34").unwrap());
        assert!(!mediated_version(&before, "0.1.32").unwrap());
        assert!(!mediated_version(&before, "0.1.31").unwrap());
        assert!(mediated_version(&before, "0.1.34-dev").is_err());
        for field in ["machine_id", "account"] {
            let mut wrong = before.clone();
            wrong[field] = "other".into();
            assert!(!mediated_identity(&before, &wrong));
            wrong[field] = serde_json::Value::Null;
            assert!(!mediated_identity(&wrong, &wrong));
        }
        assert!(mediated_identity(&before, &after));
    }
    #[test]
    fn update_capability_requires_viewer_version() {
        let current = env!("CARGO_PKG_VERSION");
        assert!(modern_update(
            &serde_json::json!({"version":current,"capabilities":["stable-update-v1"]})
        ));
        assert!(!modern_update(
            &serde_json::json!({"version":"0.0.0","capabilities":["stable-update-v1"]})
        ));
        assert!(!modern_update(
            &serde_json::json!({"version":current,"capabilities":[]})
        ));
        assert!(!modern_update(
            &serde_json::json!({"version":"unknown","capabilities":["stable-update-v1"]})
        ));
    }
    #[test]
    fn update_claims_never_substitute_for_version_evidence() {
        let output = |text: &str| UpdateOutput {
            success: true,
            stdout: text.as_bytes().to_vec(),
        };
        assert_eq!(
            update_claim(false, &output("Updated cx to 0.2.0")).unwrap(),
            UpdateClaim::Verify
        );
        assert_eq!(
            update_claim(false, &output("offline")).unwrap(),
            UpdateClaim::Verify
        );
        assert_eq!(
            update_claim(true, &output(r#"{"state":"updated"}"#)).unwrap(),
            UpdateClaim::Verify
        );
        assert_eq!(
            update_claim(true, &output(r#"{"state":"current"}"#)).unwrap(),
            UpdateClaim::Current
        );
        for state in ["offline", "unavailable", "busy", "unknown"] {
            assert_eq!(
                update_claim(true, &output(&format!(r#"{{"state":"{state}"}}"#))).unwrap(),
                UpdateClaim::Unavailable
            );
        }
        assert!(update_claim(true, &output("shell banner\n{}")).is_err());
        assert_eq!(
            update_claim(
                false,
                &UpdateOutput {
                    success: false,
                    stdout: Vec::new()
                }
            )
            .unwrap(),
            UpdateClaim::Unavailable
        );
    }
    fn shell(script: &str) -> Command {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", script]);
        command
    }
    #[test]
    fn bounded_process_preserves_stdout_discards_stderr_and_exit_status() {
        let result = run_bounded_update(
            shell("printf '{\"state\":\"current\"}'; printf 'synthetic-private' >&2"),
            std::time::Duration::from_secs(2),
            65536,
        )
        .unwrap();
        assert!(result.success);
        assert_eq!(result.stdout, b"{\"state\":\"current\"}");
        let result =
            run_bounded_update(shell("exit 7"), std::time::Duration::from_secs(2), 65536).unwrap();
        assert!(!result.success);
    }
    #[test]
    fn bounded_process_caps_combined_output_and_kills_flood() {
        let started = std::time::Instant::now();
        let result=run_bounded_update(shell("while :; do printf '12345678901234567890123456789012'; printf '12345678901234567890123456789012' >&2; done"),std::time::Duration::from_secs(2),1024);
        assert!(result.unwrap_err().to_string().contains("exceeded"));
        assert!(started.elapsed() < std::time::Duration::from_secs(2));
    }
    #[test]
    fn bounded_process_timeout_cleans_descendant_and_reader_threads() {
        let fixture = tempfile::tempdir().unwrap();
        let path = fixture.path().join("pid");
        let mut command = shell("sleep 30 & printf '%s' \"$!\" > \"$1\"; wait");
        command.arg("cx-update-test").arg(&path);
        let started = std::time::Instant::now();
        let result = run_bounded_update(command, std::time::Duration::from_millis(150), 65536);
        assert!(result.unwrap_err().to_string().contains("timed out"));
        assert!(started.elapsed() < std::time::Duration::from_secs(2));
        let pid = std::fs::read_to_string(path).unwrap();
        // SIGKILL delivery and reaping are asynchronous; observe the actual bounded outcome.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
        loop {
            match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
                Ok(stat)
                    if stat
                        .rsplit(')')
                        .next()
                        .unwrap()
                        .trim_start()
                        .starts_with('Z') =>
                {
                    break
                }
                _ => {
                    assert!(
                        std::time::Instant::now() < deadline,
                        "owned descendant survived SIGKILL"
                    );
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
            }
        }
    }
    #[test]
    fn ssh_update_arguments_are_fixed_and_noninteractive() {
        let command = ssh("fixture.example", false).unwrap();
        let args = command
            .get_args()
            .map(|v| v.to_str().unwrap())
            .collect::<Vec<_>>();
        assert!(args.contains(&"BatchMode=yes"));
        assert!(args.contains(&"ForwardAgent=no"));
        assert_eq!(args.last(), Some(&"fixture.example"));
        assert_eq!(
            UPDATE_MODERN,
            "exec ~/.local/bin/cx update --automatic --json"
        );
        assert_eq!(UPDATE_LEGACY, "exec ~/.local/bin/cx update");
    }
    #[test]
    fn event_frames_route_to_the_sink_and_never_answer_a_request() {
        static SEEN: std::sync::Mutex<Vec<(Option<String>, serde_json::Value)>> =
            std::sync::Mutex::new(Vec::new());
        set_event_sink(Box::new(|target, event| {
            SEEN.lock().unwrap().push((target, event))
        }));
        let event =
            r#"{"version":1,"id":"evt:1","result":{"kind":"sessions","sessions":[]},"error":null}"#;
        let reply = r#"{"version":1,"id":"req-1","result":{"ok":true},"error":null}"#;
        let script = format!(
            "printf 'CX1 {}\\n%s' '{event}'; printf 'CX1 {}\\n%s' '{reply}'; sleep 1",
            event.len(),
            reply.len()
        );
        let connection =
            Connection::from_command(shell(&script), Some("evt-fixture".into())).unwrap();
        connection
            .watching
            .store(true, std::sync::atomic::Ordering::Relaxed);
        let first = connection
            .replies
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap()
            .unwrap();
        assert_eq!(first.id, "req-1", "the event is not a reply");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let mine = |seen: &[(Option<String>, serde_json::Value)]| {
            seen.iter()
                .filter(|(t, _)| t.as_deref() == Some("evt-fixture"))
                .map(|(_, e)| e["kind"].as_str().unwrap_or("").to_owned())
                .collect::<Vec<_>>()
        };
        while mine(&SEEN.lock().unwrap()).len() < 2 && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        // The helper exiting ends the watch: the viewer is told to poll again.
        assert_eq!(mine(&SEEN.lock().unwrap()), ["sessions", "closed"]);
    }
    #[test]
    fn interactive_work_never_shares_a_channel_with_bulk_work() {
        let session: Session = serde_json::from_value(serde_json::json!({
            "id":"x","name":"x","directory":"/","provider":"shell","host":"h",
            "account":"a","pid":1,"started":"0","boot_id":"b","external":false,"socket":null
        }))
        .unwrap();
        let watch = Operation::Watch {
            sessions: true,
            pane: Some(session),
        };
        assert_eq!(channel_class(&watch), Channel::Interactive);
        assert_eq!(channel_class(&Operation::Containers), Channel::Bulk);
        assert_eq!(
            channel_class(&Operation::List { path: "/".into() }),
            Channel::Bulk
        );
        assert_eq!(channel_class(&Operation::Sessions), Channel::Metadata);
        assert_eq!(channel_class(&Operation::Info), Channel::Capabilities);
        // Additive: older viewers' frames still decode, and an empty watch is valid.
        let decoded: Operation = serde_json::from_str(r#"{"op":"watch","args":{}}"#).unwrap();
        assert!(matches!(
            decoded,
            Operation::Watch {
                sessions: false,
                pane: None
            }
        ));
    }
    #[test]
    fn current_disk_cli_refreshes_stale_helper_and_waits_for_pending_request() {
        let target = "cx-update-fixture-current";
        let before = serde_json::json!({"version":env!("CARGO_PKG_VERSION"),"machine_id":"fixture","capabilities":["stable-update-v1"]});
        let newer = serde_json::json!({"version":"99.0.0","machine_id":"fixture"});
        let old = Connection::from_command(shell("sleep 30"), None).unwrap();
        let pid = old.child.id() as i32;
        let entry = std::sync::Arc::new(std::sync::Mutex::new(Some(old)));
        CONNECTIONS
            .lock()
            .unwrap()
            .insert(target.into(), entry.clone());
        let slot = entry.lock().unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            update_host_with(
                target,
                &before,
                shell("printf '{\"state\":\"current\"}'"),
                || {
                    tx.send(()).unwrap();
                    Ok(newer)
                },
            )
        });
        rx.recv_timeout(std::time::Duration::from_secs(2)).unwrap();
        assert_eq!(
            unsafe { libc::kill(pid, 0) },
            0,
            "pending channel should stay alive"
        );
        assert!(!worker.is_finished());
        drop(slot);
        assert_eq!(
            worker.join().unwrap().unwrap(),
            (true, Some("99.0.0".into()))
        );
        assert!(entry.lock().unwrap().is_none());
        assert_eq!(
            unsafe { libc::kill(pid, 0) },
            -1,
            "old channel must be reaped after request finishes"
        );
        CONNECTIONS.lock().unwrap().remove(target);
    }
    #[test]
    fn legacy_zero_exit_and_updated_claim_require_a_new_helper_version() {
        let before = serde_json::json!({"version":"0.0.1","machine_id":"fixture"});
        let result = update_host_with(
            "fixture-legacy",
            &before,
            shell("printf 'Updated cx'"),
            || Ok(before.clone()),
        )
        .unwrap();
        assert_eq!(result, (false, None));
        let result = update_host_with(
            "fixture-legacy-upgraded",
            &before,
            shell("printf 'cx current'"),
            || Ok(serde_json::json!({"version":"0.1.9","machine_id":"fixture"})),
        )
        .unwrap();
        assert_eq!(result, (true, Some("0.1.9".into())));
        let modern = serde_json::json!({"version":env!("CARGO_PKG_VERSION"),"machine_id":"fixture","capabilities":["stable-update-v1"]});
        let result = update_host_with(
            "fixture-updated",
            &modern,
            shell("printf '{\"state\":\"updated\"}'"),
            || Ok(modern.clone()),
        )
        .unwrap();
        assert_eq!(result, (false, None));
        let result = update_host_with(
            "fixture-current",
            &modern,
            shell("printf '{\"state\":\"current\"}'"),
            || Ok(modern.clone()),
        )
        .unwrap();
        assert_eq!(result, (true, None));
    }
    #[test]
    fn offline_update_does_not_open_verification_connection() {
        let before = serde_json::json!({"version":env!("CARGO_PKG_VERSION"),"capabilities":["stable-update-v1"]});
        let result = update_host_with(
            "fixture-offline",
            &before,
            shell("printf '{\"state\":\"offline\"}'"),
            || panic!("offline update must not reconnect"),
        )
        .unwrap();
        assert_eq!(result, (false, None));
    }
}
