//! Event loop and terminal ownership: Screen, input batching, task workers, auth prompts,
//! self-restart state and update checks.
use super::*;

pub(super) fn load_cache(app: &mut App) {
    let path = store::state_dir().join("ui-sessions.json");
    if std::fs::metadata(&path).map_or(true, |m| m.len() > 1024 * 1024) {
        return;
    }
    if let Ok(data) = std::fs::read(path) {
        if let Ok(cache) = serde_json::from_slice::<HashMap<String, Vec<Session>>>(&data) {
            for (d, w) in app.devices.iter().zip(&mut app.work) {
                if let Some(s) = cache.get(&d.id) {
                    w.sessions = s.clone();
                }
            }
        }
    }
}
// Secrets have their own short-lived editor; they never enter App/cache/snapshots.
pub(super) struct SecretInput(String);
impl Drop for SecretInput {
    fn drop(&mut self) {
        unsafe {
            for b in self.0.as_bytes_mut() {
                std::ptr::write_volatile(b, 0);
            }
        }
    }
}
/// The modal reads the terminal itself, so the input thread waits meanwhile.
pub(super) fn authentication_modal(
    screen: &mut Screen,
    prompt: crate::auth::Prompt,
) -> Result<crate::auth::Answer> {
    pause_input();
    let answer = prompt_modal(screen, prompt);
    resume_input();
    answer
}
fn prompt_modal(screen: &mut Screen, prompt: crate::auth::Prompt) -> Result<crate::auth::Answer> {
    use crate::auth::{Answer, PromptKind};
    let trust = prompt.kind == PromptKind::HostKey;
    let title = match prompt.kind {
        PromptKind::HostKey => "Verify host key",
        PromptKind::Password if prompt.text.to_ascii_lowercase().contains("sudo") => {
            "Administrator password"
        }
        PromptKind::Password => "SSH password",
        PromptKind::KeyPassphrase => "Unlock SSH key",
        PromptKind::Verification => "SSH verification",
    };
    let mut secret = SecretInput(String::with_capacity(8192));
    let started = Instant::now();
    let mut scroll = 0u16;
    let mut reviewed = false;
    let mut show_secret = false;
    let mut toggle_hit = None;
    loop {
        if started.elapsed() > Duration::from_secs(240) {
            return Ok(Answer::Cancel);
        }
        screen.terminal.draw(|f| {
            let area = popup(
                f.area(),
                if trust { 76 } else { 64 },
                if trust { 13 } else { 7 },
            );
            f.render_widget(Clear, area);
            let panel = block(title.into(), true).padding(Padding::horizontal(1));
            let inner = panel.inner(area);
            f.render_widget(panel, area);
            if trust {
                let content = Rect {
                    height: inner.height.saturating_sub(2),
                    ..inner
                };
                let mut lines = Vec::new();
                for original in prompt.text.lines() {
                    let mut line = String::new();
                    let mut width = 0usize;
                    for c in safe_label(original).chars() {
                        let cells = if c.is_ascii() { 1 } else { 2 };
                        if width + cells > content.width as usize && !line.is_empty() {
                            lines.push(Line::from(std::mem::take(&mut line)));
                            width = 0;
                        }
                        line.push(c);
                        width += cells;
                    }
                    lines.push(Line::from(line));
                }
                let last = lines
                    .len()
                    .saturating_sub(content.height as usize)
                    .min(u16::MAX as usize) as u16;
                scroll = scroll.min(last);
                reviewed |= content.height > 0 && scroll >= last;
                f.render_widget(Paragraph::new(lines).scroll((scroll, 0)), content);
                let footer = Rect {
                    y: inner.y + inner.height.saturating_sub(1),
                    height: 1,
                    ..inner
                };
                f.render_widget(
                    Paragraph::new(if reviewed {
                        "y Trust fingerprint   n Cancel"
                    } else {
                        "↓ Review fingerprint   n Cancel"
                    })
                    .style(warn()),
                    footer,
                );
            } else {
                let prompt_area = Rect { height: 1, ..inner };
                f.render_widget(
                    Paragraph::new(safe_label(
                        prompt.text.lines().next().unwrap_or("SSH authentication"),
                    )),
                    prompt_area,
                );
                let field = Rect {
                    y: inner.y + 2,
                    height: 1,
                    ..inner
                };
                let shown = if show_secret {
                    safe_label(&secret.0)
                } else {
                    (if ascii() { "*" } else { "•" }).repeat(
                        secret
                            .0
                            .chars()
                            .count()
                            .min(inner.width.saturating_sub(2) as usize),
                    )
                };
                let width = Span::raw(shown.as_str()).width() as u16;
                f.render_widget(
                    Paragraph::new(Line::from(vec![
                        Span::raw("> "),
                        Span::styled(shown, accent()),
                    ])),
                    field,
                );
                f.set_cursor_position((
                    field.x + (width + 2).min(field.width.saturating_sub(1)),
                    field.y,
                ));
                let footer = Rect {
                    y: inner.y + inner.height.saturating_sub(1),
                    height: 1,
                    ..inner
                };
                let controls = Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([Constraint::Min(1), Constraint::Length(9)])
                    .split(footer);
                f.render_widget(
                    Paragraph::new("Enter OK · Esc Back").style(muted()),
                    controls[0],
                );
                f.render_widget(
                    Paragraph::new(if show_secret { "F2 Hide" } else { "F2 Show" }).style(accent()),
                    controls[1],
                );
                toggle_hit = Some(controls[1]);
            }
        })?;
        if !event::poll(Duration::from_millis(100))? {
            continue;
        }
        match event::read()? {
            Event::Key(key) if key.kind != KeyEventKind::Release => {
                if key.code == KeyCode::Esc
                    || (key.code == KeyCode::Char('c')
                        && key.modifiers.contains(KeyModifiers::CONTROL))
                {
                    return Ok(Answer::Cancel);
                }
                if trust {
                    match key.code {
                        KeyCode::Char('y') if reviewed => return Ok(Answer::Submit("yes".into())),
                        KeyCode::Char('n') => return Ok(Answer::Cancel),
                        KeyCode::Down | KeyCode::PageDown => scroll = scroll.saturating_add(1),
                        KeyCode::Up | KeyCode::PageUp => scroll = scroll.saturating_sub(1),
                        _ => {}
                    }
                } else {
                    match key.code {
                        KeyCode::Enter => return Ok(Answer::Submit(std::mem::take(&mut secret.0))),
                        KeyCode::F(2) => show_secret = !show_secret,
                        KeyCode::Backspace => {
                            if let Some((index, _)) = secret.0.char_indices().last() {
                                unsafe {
                                    for b in &mut secret.0.as_bytes_mut()[index..] {
                                        std::ptr::write_volatile(b, 0);
                                    }
                                }
                                secret.0.truncate(index);
                            }
                        }
                        KeyCode::Char(c)
                            if !c.is_control()
                                && !key
                                    .modifiers
                                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                                && secret.0.len() + c.len_utf8() < 8192 =>
                        {
                            secret.0.push(c)
                        }
                        _ => {}
                    }
                }
            }
            Event::Mouse(mouse)
                if !trust
                    && mouse.kind == MouseEventKind::Down(crossterm::event::MouseButton::Left) =>
            {
                if toggle_hit
                    .is_some_and(|rect: Rect| rect.contains((mouse.column, mouse.row).into()))
                {
                    show_secret = !show_secret;
                }
            }
            _ => {}
        }
    }
}
// A physical detent can arrive as identical wheel reports a few microseconds
// apart (verified by a physical input trace). Normalize only that short burst;
// direction, coordinates and modifiers distinguish separate input contexts.
pub(super) const WHEEL_BURST: Duration = Duration::from_millis(2);
#[derive(Default)]
pub(super) struct WheelReports {
    last: Option<(MouseEvent, Instant)>,
}
pub(super) fn wheel_report(input: &Event) -> Option<MouseEvent> {
    match input {
        Event::Mouse(mouse)
            if matches!(
                mouse.kind,
                MouseEventKind::ScrollUp | MouseEventKind::ScrollDown
            ) =>
        {
            Some(*mouse)
        }
        _ => None,
    }
}
impl WheelReports {
    pub(super) fn accepts(&mut self, input: &Event, observed: Instant) -> bool {
        let Some(mouse) = wheel_report(input) else {
            self.last = None;
            return true;
        };
        if self.last.is_some_and(|(previous, at)| {
            previous == mouse && observed.saturating_duration_since(at) < WHEEL_BURST
        }) {
            return false;
        }
        self.last = Some((mouse, observed));
        true
    }
}
// A terminal whose pty hung up (or whose session leader died and left us
// reparented to init) must not keep the UI spinning: nothing will ever deliver
// SIGHUP to a process in its own group.
pub(super) fn terminal_gone() -> bool {
    // SAFETY: tcgetattr only reads into a zeroed termios; getppid has no preconditions.
    unsafe {
        let mut t: libc::termios = std::mem::zeroed();
        libc::tcgetattr(0, &mut t) != 0 || (libc::getppid() == 1 && libc::isatty(0) == 0)
    }
}
// Backstop for a main thread stuck inside a terminal read/poll: exit when the
// terminal is gone, or when a requested stop (SIGHUP/SIGTERM/SIGINT) is not honoured.
pub(super) fn spawn_exit_watchdog(stopping: std::sync::Arc<std::sync::atomic::AtomicBool>) {
    thread::spawn(move || {
        let mut stop_seen = 0u32;
        loop {
            thread::sleep(Duration::from_millis(500));
            if stopping.load(std::sync::atomic::Ordering::Relaxed) {
                stop_seen += 1;
            }
            if terminal_gone() || stop_seen >= 6 {
                // _exit skips destructors: drop the askpass directory first.
                crate::auth::sweep_askpass_dirs(true);
                // SAFETY: _exit is async-signal-safe and skips destructors on purpose.
                unsafe { libc::_exit(1) }
            }
        }
    });
}
/// Reports that cannot change cx state relevant to a pending attachment.
pub(super) fn passive_input(input: &Event) -> bool {
    match input {
        Event::Mouse(m) => matches!(
            m.kind,
            MouseEventKind::Moved | MouseEventKind::Up(_) | MouseEventKind::Drag(_)
        ),
        Event::FocusGained | Event::FocusLost | Event::Resize(..) => true,
        _ => false,
    }
}
/// Everything that can wake the event loop arrives on one channel: terminal
/// input (from the input thread), task replies and helper events.
pub(super) enum Wake {
    Input(Event, Instant),
    Reply(Box<Reply>),
    /// A pushed helper event: the SSH target (None for this host) and `{kind, ...}`.
    Event(Option<String>, Value),
}
impl From<Reply> for Wake {
    fn from(reply: Reply) -> Self {
        Wake::Reply(Box::new(reply))
    }
}
#[derive(Default)]
struct InputGate {
    pauses: u32,
    parked: bool,
}
/// The only reader of terminal input while cx owns the terminal. It blocks
/// in poll(2) on the terminal, SIGWINCH and a wake pipe, and parks whenever
/// the terminal is handed to something else (an attach, a prompt), so it
/// never reads input meant for that program.
pub(super) struct InputThread {
    gate: std::sync::Arc<(std::sync::Mutex<InputGate>, std::sync::Condvar)>,
    wake: std::os::fd::OwnedFd,
}
static INPUT: std::sync::OnceLock<InputThread> = std::sync::OnceLock::new();
fn pipe() -> io::Result<(std::os::fd::OwnedFd, std::os::fd::OwnedFd)> {
    use std::os::fd::FromRawFd;
    let mut fds = [0; 2];
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC | libc::O_NONBLOCK) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe {
        (
            std::os::fd::OwnedFd::from_raw_fd(fds[0]),
            std::os::fd::OwnedFd::from_raw_fd(fds[1]),
        )
    })
}
fn drain(fd: &std::os::fd::OwnedFd) {
    use std::os::fd::AsRawFd;
    let mut buf = [0u8; 64];
    while unsafe { libc::read(fd.as_raw_fd(), buf.as_mut_ptr().cast(), buf.len()) } > 0 {}
}
pub(super) fn start_input(tx: mpsc::Sender<Wake>) -> Result<()> {
    use std::os::fd::AsRawFd;
    let (wake_read, wake) = pipe()?;
    let (winch_read, winch) = pipe()?;
    signal_hook::low_level::pipe::register(signal_hook::consts::SIGWINCH, winch)?;
    // crossterm reads stdin when it is the terminal, else /dev/tty.
    let tty = (unsafe { libc::isatty(0) } == 0)
        .then(|| std::fs::File::open("/dev/tty"))
        .transpose()?;
    let tty_fd = tty.as_ref().map_or(0, |t| t.as_raw_fd());
    let gate = std::sync::Arc::new((
        std::sync::Mutex::new(InputGate::default()),
        std::sync::Condvar::new(),
    ));
    let shared = gate.clone();
    thread::Builder::new()
        .name("cx-input".into())
        .spawn(move || {
            let _tty = tty;
            let (lock, changed) = &*shared;
            read_input(lock, changed, &tx, tty_fd, &wake_read, &winch_read);
            // Gone for good: a pause must never wait for it.
            lock.lock().unwrap().parked = true;
            changed.notify_all();
        })?;
    let _ = INPUT.set(InputThread { gate, wake });
    Ok(())
}
fn read_input(
    lock: &std::sync::Mutex<InputGate>,
    changed: &std::sync::Condvar,
    tx: &mpsc::Sender<Wake>,
    tty_fd: i32,
    wake_read: &std::os::fd::OwnedFd,
    winch_read: &std::os::fd::OwnedFd,
) {
    use std::os::fd::AsRawFd;
    loop {
        {
            let mut state = lock.lock().unwrap();
            while state.pauses > 0 {
                state.parked = true;
                changed.notify_all();
                state = changed.wait(state).unwrap();
            }
            state.parked = false;
        }
        // crossterm may hold parsed events poll(2) cannot see.
        loop {
            match event::poll(Duration::ZERO) {
                Ok(true) => {}
                Ok(false) => break,
                Err(_) => return,
            }
            let Ok(input) = event::read() else {
                return;
            };
            if tx.send(Wake::Input(input, Instant::now())).is_err() {
                return;
            }
        }
        let mut fds =
            [tty_fd, wake_read.as_raw_fd(), winch_read.as_raw_fd()].map(|fd| libc::pollfd {
                fd,
                events: libc::POLLIN,
                revents: 0,
            });
        unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, -1) };
        if fds[0].revents & (libc::POLLHUP | libc::POLLERR | libc::POLLNVAL) != 0
            && fds[0].revents & libc::POLLIN == 0
        {
            return; // The terminal is gone; the loop notices and exits.
        }
        drain(wake_read);
        drain(winch_read);
    }
}
/// Stop reading terminal input until the matching `resume_input`; returns
/// once the input thread is parked. A no-op without the thread (tests).
pub(super) fn pause_input() {
    use std::os::fd::AsRawFd;
    let Some(input) = INPUT.get() else {
        return;
    };
    let (lock, changed) = &*input.gate;
    let mut state = lock.lock().unwrap();
    state.pauses += 1;
    unsafe { libc::write(input.wake.as_raw_fd(), [1u8].as_ptr().cast(), 1) };
    while !state.parked {
        state = changed.wait(state).unwrap();
    }
}
pub(super) fn resume_input() {
    let Some(input) = INPUT.get() else {
        return;
    };
    let (lock, changed) = &*input.gate;
    let mut state = lock.lock().unwrap();
    state.pauses = state.pauses.saturating_sub(1);
    changed.notify_all();
}
/// DEC private mode 2026: begin / end synchronized update.
const SYNC_BEGIN: &[u8] = b"\x1b[?2026h";
const SYNC_END: &[u8] = b"\x1b[?2026l";
/// At most one frame per this interval (~60 fps); bursts are coalesced.
pub(super) const FRAME_INTERVAL: Duration = Duration::from_millis(16);
/// Files paints a reply that lands just after a key's frame without waiting out the
/// 60 fps budget (a local listing or preview arrives in a few milliseconds).
fn frame_interval(app: &App) -> Duration {
    if app.view == View::Files {
        FRAME_INTERVAL / 4
    } else {
        FRAME_INTERVAL
    }
}
/// How long the event loop may block before its next pass. Input, replies
/// and helper events wake it at once; this covers only timers.
pub(super) fn loop_wait(app: &App, now: Instant, frame_wait: Option<Duration>) -> Duration {
    let mut wait = notifications::poll_interval(app.notice_started, app.notice_deadline, now);
    if matches!(app.view, View::Work | View::Network) {
        wait = wait.min(Duration::from_millis(250));
    }
    if app.command_panel.as_ref().is_some_and(|p| p.alive) {
        wait = wait.min(Duration::from_millis(80));
    }
    if let Some(due) = app.files_due(now) {
        wait = wait.min(due);
    }
    if let Some(preview) = app.session_preview_wait() {
        wait = wait.min(preview);
    }
    if let Some(frame_wait) = frame_wait {
        wait = wait.min(frame_wait);
    }
    for deadline in [app.press_deadline()].into_iter().flatten() {
        wait = wait.min(deadline.saturating_duration_since(now));
    }
    wait
}

/// What ratatui's crossterm backend writes after every frame's cell updates
/// (colour/attribute resets), derived from the backend itself so it tracks
/// the crate's features rather than a hard-coded byte string.
fn frame_reset() -> &'static [u8] {
    static RESET: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();
    RESET.get_or_init(|| {
        use ratatui::backend::Backend;
        struct Sink(std::rc::Rc<std::cell::RefCell<Vec<u8>>>);
        impl io::Write for Sink {
            fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
                self.0.borrow_mut().extend_from_slice(buf);
                Ok(buf.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let bytes = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let mut backend = CrosstermBackend::new(Sink(bytes.clone()));
        let _ = backend.draw(std::iter::empty());
        let reset = bytes.borrow().clone();
        reset
    })
}
fn find_last(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    (needle.len() <= haystack.len())
        .then(|| {
            (0..=haystack.len() - needle.len())
                .rev()
                .find(|&i| haystack[i..].starts_with(needle))
        })
        .flatten()
}
/// Decide what one composed frame must send. A frame with no cell updates
/// whose cursor state matches the previous frame sends nothing at all (the
/// idle redraws used to emit resets and cursor-hide on every tick).
pub(super) fn frame_payload(bytes: &[u8], last_tail: &mut Vec<u8>) -> Option<Vec<u8>> {
    let reset = frame_reset();
    let (body, tail) = match find_last(bytes, reset) {
        Some(at) => (&bytes[..at], &bytes[at + reset.len()..]),
        None => (&bytes[..0], bytes),
    };
    if body.is_empty() {
        if tail == last_tail.as_slice() {
            return None;
        }
        *last_tail = tail.to_vec();
        return Some(tail.to_vec());
    }
    *last_tail = tail.to_vec();
    Some(bytes.to_vec())
}
#[derive(Default)]
struct FrameState {
    frame: Option<Vec<u8>>,
    last_tail: Vec<u8>,
}
/// The terminal's writer. Outside a frame it passes through; inside a frame it
/// buffers, then sends the frame once, wrapped in synchronized output.
pub(super) struct FrameWriter {
    out: io::Stdout,
    state: std::rc::Rc<std::cell::RefCell<FrameState>>,
}
impl io::Write for FrameWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut state = self.state.borrow_mut();
        if let Some(frame) = &mut state.frame {
            frame.extend_from_slice(buf);
            Ok(buf.len())
        } else {
            // Anything written between frames may move the cursor.
            state.last_tail.clear();
            self.out.write(buf)
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        if self.state.borrow().frame.is_some() {
            Ok(())
        } else {
            self.out.flush()
        }
    }
}
/// Screen-side handle on the writer's frame buffer.
struct FrameControl {
    state: std::rc::Rc<std::cell::RefCell<FrameState>>,
    sync: bool,
}
impl FrameControl {
    fn begin(&self) {
        self.state.borrow_mut().frame = Some(Vec::new());
    }
    /// Send the composed frame; false when it was a no-op and nothing was sent.
    fn commit(&self) -> io::Result<bool> {
        use io::Write;
        let payload = {
            let mut state = self.state.borrow_mut();
            let bytes = state.frame.take().unwrap_or_default();
            frame_payload(&bytes, &mut state.last_tail)
        };
        let Some(payload) = payload else {
            return Ok(false);
        };
        let mut out = Vec::with_capacity(payload.len() + 16);
        if self.sync {
            out.extend_from_slice(SYNC_BEGIN);
        }
        out.extend_from_slice(&payload);
        if self.sync {
            out.extend_from_slice(SYNC_END);
        }
        let mut stdout = io::stdout();
        stdout.write_all(&out)?;
        stdout.flush()?;
        Ok(true)
    }
}
/// OSC 52: put `text` on the system clipboard (tmux passes it on with `set-clipboard on`).
fn osc52(text: &str) -> Vec<u8> {
    use base64::Engine;
    format!(
        "\x1b]52;c;{}\x07",
        base64::engine::general_purpose::STANDARD.encode(text)
    )
    .into_bytes()
}
fn frame_writer() -> (FrameWriter, FrameControl) {
    let state = std::rc::Rc::new(std::cell::RefCell::new(FrameState::default()));
    // Terminals ignore unknown private modes; the Linux console and dumb
    // terminals are skipped so nothing stray ever reaches them.
    let sync = !std::env::var("TERM").is_ok_and(|t| t == "dumb" || t == "linux");
    (
        FrameWriter {
            out: io::stdout(),
            state: state.clone(),
        },
        FrameControl { state, sync },
    )
}
pub(super) struct Screen {
    terminal: Terminal<CrosstermBackend<FrameWriter>>,
    frames: FrameControl,
    active: bool,
    mouse: bool,
    paste: bool,
}
impl Screen {
    fn new() -> Result<Self> {
        terminal::enable_raw_mode()?;
        if let Err(e) = execute!(io::stdout(), EnterAlternateScreen) {
            let _ = terminal::disable_raw_mode();
            return Err(e.into());
        }
        let (writer, frames) = frame_writer();
        let terminal = match Terminal::new(CrosstermBackend::new(writer)) {
            Ok(t) => t,
            Err(e) => {
                let _ = terminal::disable_raw_mode();
                let _ = execute!(io::stdout(), LeaveAlternateScreen);
                return Err(e.into());
            }
        };
        Ok(Self {
            terminal,
            frames,
            active: true,
            mouse: false,
            paste: false,
        })
    }
    fn bracketed_paste(&mut self, enabled: bool) -> Result<()> {
        if enabled != self.paste {
            if enabled {
                execute!(self.terminal.backend_mut(), event::EnableBracketedPaste)?;
            } else {
                execute!(self.terminal.backend_mut(), event::DisableBracketedPaste)?;
            }
            self.paste = enabled;
        }
        Ok(())
    }
    fn mouse_capture(&mut self, enabled: bool) -> Result<()> {
        if enabled != self.mouse {
            if enabled {
                execute!(self.terminal.backend_mut(), EnableMouseCapture)?;
            } else {
                execute!(self.terminal.backend_mut(), DisableMouseCapture)?;
            }
            self.mouse = enabled;
        }
        Ok(())
    }
    /// Leave the TUI for a subprocess. `clear` wipes the primary screen that
    /// LeaveAlternateScreen restores: without it, a slow remote handoff briefly
    /// shows the previous attach's "Connection closed"/"[detached]" teardown
    /// text before tmux repaints, which looks like the session failed to load.
    /// Final teardown (Drop) passes `false` so the user's own terminal is kept.
    fn suspend(&mut self, clear: bool) -> Result<()> {
        self.release(true, clear)
    }
    /// Hand the terminal to a native attach while the frame just drawn (the
    /// session's live screen) stays up: tmux paints over it in place.
    fn hand_off(&mut self) -> Result<()> {
        self.release(false, false)
    }
    fn release(&mut self, leave_screen: bool, clear: bool) -> Result<()> {
        if self.active {
            pause_input();
            let paste = self.bracketed_paste(false);
            let mouse = self.mouse_capture(false);
            self.active = false;
            let raw = terminal::disable_raw_mode();
            let leave = if leave_screen {
                execute!(self.terminal.backend_mut(), LeaveAlternateScreen)
            } else {
                Ok(())
            };
            let wipe = if clear {
                execute!(
                    self.terminal.backend_mut(),
                    terminal::Clear(terminal::ClearType::All),
                    crossterm::cursor::MoveTo(0, 0)
                )
            } else {
                Ok(())
            };
            let cursor = self.terminal.show_cursor();
            paste?;
            mouse?;
            raw?;
            leave?;
            wipe?;
            cursor?;
        }
        Ok(())
    }
    /// Draw one frame: composed off-screen, sent once (synchronized), or not
    /// at all when nothing on screen changed.
    /// Returns whether anything was written.
    fn draw(&mut self, render: impl FnOnce(&mut Frame<'_>)) -> Result<bool> {
        self.frames.begin();
        let drawn = self.terminal.draw(render);
        let committed = self.frames.commit();
        drawn?;
        Ok(committed?)
    }
    /// Back from a handoff. The retained buffer still describes the last
    /// frame, so only a clear of the terminal's screen is needed, not of the
    /// buffer: the next draw repaints every cell that differs from blank.
    fn resume(&mut self) -> Result<()> {
        terminal::enable_raw_mode()?;
        let was_active = std::mem::replace(&mut self.active, true);
        execute!(
            self.terminal.backend_mut(),
            EnterAlternateScreen,
            terminal::Clear(terminal::ClearType::All)
        )?;
        // Diff the next frame against blank: that repaints exactly what is drawn.
        self.terminal.swap_buffers();
        self.terminal.current_buffer_mut().reset();
        if !was_active {
            resume_input();
        }
        Ok(())
    }
}
impl Drop for Screen {
    fn drop(&mut self) {
        let _ = self.suspend(false);
    }
}
pub(super) const MAX_RESTART_STATE: usize = 4 * 1024 * 1024;
#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct RestartState {
    schema: u32,
    expires_at: u64,
    device_ids: Vec<String>,
    device: usize,
    focus: Focus,
    view: View,
    selected: usize,
    #[serde(default)]
    selected_session: Option<(String, String)>,
    search: String,
    browser: Option<Browser>,
    other_browser: Option<Browser>,
    destination_active: bool,
    conflict: usize,
    launch_provider: Option<String>,
    clipboard: Option<Clipboard>,
    submitted: HashMap<String, crate::model::TransferSpec>,
}
pub(super) fn restart_browser(browser: &Option<Browser>) -> Option<Browser> {
    browser.as_ref().map(|browser| {
        let mut snapshot = browser.clone();
        snapshot.restore_selection = browser_entries(browser)
            .get(browser.selected)
            .map(|entry| entry.path.clone());
        snapshot
    })
}
pub(super) fn save_restart(app: &App) -> Result<String> {
    save_restart_at(app, &store::ensure()?)
}
pub(super) fn save_restart_at(app: &App, directory: &std::path::Path) -> Result<String> {
    use std::{io::Write, os::unix::fs::OpenOptionsExt};
    let state = RestartState {
        schema: 1,
        expires_at: transport::now() + 300,
        device_ids: app.devices.iter().map(|d| d.id.clone()).collect(),
        device: app.device,
        focus: app.focus,
        view: app.view,
        selected: app.selected,
        selected_session: app
            .selected_session()
            .map(|(device, session)| (app.devices[device].id.clone(), session.id)),
        search: app.search.clone(),
        browser: restart_browser(&app.browser),
        other_browser: restart_browser(&app.other_browser),
        destination_active: app.destination_active,
        conflict: app.conflict,
        launch_provider: app.launch_provider.clone(),
        clipboard: app.clipboard.clone(),
        submitted: app.submitted.clone(),
    };
    let bytes = serde_json::to_vec(&state)?;
    anyhow::ensure!(
        bytes.len() <= MAX_RESTART_STATE,
        "Workspace too large for automatic restart; return to Sessions first"
    );
    let name = format!("viewer-restart-{}.json", unique_key());
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(directory.join(&name))?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    Ok(name)
}
pub(super) fn restore_restart(app: &mut App, name: &str) -> Result<()> {
    restore_restart_at(app, name, &store::ensure()?)
}
// A restart handoff is single-use: whether it restores or not (expired, unsafe,
// unreadable), consume it so a failed handoff never lingers in the state directory.
pub(super) fn restore_restart_at(
    app: &mut App,
    name: &str,
    directory: &std::path::Path,
) -> Result<()> {
    let result = restore_restart_inner(app, name, directory);
    if restart_name_valid(name) {
        let _ = std::fs::remove_file(directory.join(name));
    }
    result
}
pub(super) fn restart_name_valid(name: &str) -> bool {
    name.starts_with("viewer-restart-")
        && name.ends_with(".json")
        && name.len() < 100
        && name
            .bytes()
            .all(|b| b.is_ascii_digit() || b"viewer-sta.json".contains(&b))
}
/// Remove restart handoffs older than their 300 s validity window that no viewer consumed.
pub(crate) fn sweep_stale_restarts(directory: &std::path::Path, max_age: Duration) -> usize {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return 0;
    };
    let mut removed = 0;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if !restart_name_valid(name) {
            continue;
        }
        let stale = entry
            .metadata()
            .and_then(|m| m.modified())
            .is_ok_and(|t| t.elapsed().is_ok_and(|age| age > max_age));
        if stale && std::fs::remove_file(entry.path()).is_ok() {
            removed += 1;
        }
    }
    removed
}
pub(super) fn restore_restart_inner(
    app: &mut App,
    name: &str,
    directory: &std::path::Path,
) -> Result<()> {
    use std::{
        io::Read,
        os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    };
    anyhow::ensure!(restart_name_valid(name), "Invalid restart state name");
    let path = directory.join(name);
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&path)?;
    let metadata = file.metadata()?;
    anyhow::ensure!(
        metadata.is_file()
            && metadata.uid() == unsafe { libc::geteuid() }
            && metadata.permissions().mode() & 0o077 == 0
            && metadata.len() <= MAX_RESTART_STATE as u64,
        "Unsafe restart state"
    );
    let mut bytes = Vec::new();
    file.by_ref()
        .take(MAX_RESTART_STATE as u64 + 1)
        .read_to_end(&mut bytes)?;
    anyhow::ensure!(
        bytes.len() <= MAX_RESTART_STATE,
        "Restart state exceeds limit"
    );
    let mut state: RestartState = serde_json::from_slice(&bytes)?;
    anyhow::ensure!(
        state.schema == 1
            && state.expires_at >= transport::now()
            && state.expires_at <= transport::now() + 300,
        "Restart state expired or unsupported"
    );
    let remap = |old: usize| {
        state
            .device_ids
            .get(old)
            .and_then(|id| app.devices.iter().position(|d| &d.id == id))
    };
    app.device = if state.device == 0 {
        0
    } else {
        remap(state.device - 1).map(|d| d + 1).unwrap_or(0)
    };
    for browser in [&mut state.browser, &mut state.other_browser] {
        if let Some(b) = browser {
            if let Some(device) = remap(b.device) {
                b.device = device;
            } else {
                *browser = None;
            }
        }
    }
    app.clipboard = state.clipboard.and_then(|mut c| {
        c.device = remap(c.device)?;
        Some(c)
    });
    app.browser = state.browser;
    app.other_browser = state.other_browser;
    app.view = if state.view == View::Files && app.browser.is_none() {
        View::Work
    } else {
        state.view
    };
    app.focus = state.focus;
    app.search = state.search;
    app.selected = state
        .selected_session
        .and_then(|(device, id)| {
            app.session_rows()
                .iter()
                .position(|(d, s)| app.devices[*d].id == device && workspaces::runtime_key(s) == id)
        })
        .unwrap_or(
            state
                .selected
                .min(app.session_rows().len().saturating_sub(1)),
        );
    app.destination_active = state.destination_active;
    app.conflict = state.conflict.min(2);
    app.launch_provider = state.launch_provider;
    app.submitted = state.submitted;
    Ok(())
}
pub(super) fn can_restart(app: &App) -> bool {
    app.input.is_none()
        && app.dialog.is_none()
        && !app.help
        && !app.creating
        && app.pending_attach.is_none()
        && app.pending_command.is_none()
        && app.pending_jump.is_none()
        && app.pending_command_panel.is_none()
        && app.command_panel.is_none()
        && app.pending_add.is_none()
        && app.network_add_target.is_none()
        && app.pending_add_via.is_none()
        && app.pending_requests.get() == 0
        && !app.file_busy
        && app.file_queue.is_empty()
        && app.browser.as_ref().is_none_or(|b| b.preview.is_none())
}
pub(super) fn restart_ready(app: &App, idle: Duration, pending_input: bool) -> bool {
    !pending_input && idle >= Duration::from_secs(3) && can_restart(app)
}
pub(super) fn version_notice(message: &str) -> String {
    format!(
        "cx v{} {} {}",
        env!("CARGO_PKG_VERSION"),
        if ascii() { "|" } else { "·" },
        safe_text(message)
    )
}
pub(super) fn update_check_notice(result: &Result<crate::update::CheckOutcome>) -> String {
    use crate::update::CheckOutcome;
    version_notice(&match result {
        Ok(CheckOutcome::Offline) => "update unreachable; retry".into(),
        Ok(CheckOutcome::Unavailable(message)) => safe_text(message),
        Ok(CheckOutcome::Skipped) => "update check already running".into(),
        Ok(CheckOutcome::Current) => "up to date".into(),
        Ok(CheckOutcome::Ready(plan)) => {
            format!("v{} ready; waiting for idle", safe_label(&plan.version))
        }
        Err(_) => "update unavailable; version kept".into(),
    })
}
pub(super) fn update_notice_kind(result: &Result<crate::update::CheckOutcome>) -> NoticeKind {
    use crate::update::CheckOutcome;
    match result {
        Ok(CheckOutcome::Current | CheckOutcome::Ready(_)) => NoticeKind::Success,
        Ok(CheckOutcome::Offline | CheckOutcome::Unavailable(_) | CheckOutcome::Skipped) => {
            NoticeKind::Warning
        }
        Err(_) => NoticeKind::Error,
    }
}
pub(super) enum UpdatePhase {
    Idle,
    Checking,
    Ready(crate::update::UpdatePlan),
    Installing,
    Installed(std::path::PathBuf),
}
pub(super) enum UpdateEvent {
    Checked(bool, Result<crate::update::CheckOutcome>),
    Installed(Result<std::path::PathBuf>),
}
pub(super) fn start_update_check(tx: &mpsc::Sender<UpdateEvent>, force: bool) {
    let tx = tx.clone();
    thread::spawn(move || {
        let result = crate::update::check(force);
        let _ = tx.send(UpdateEvent::Checked(force, result));
    });
}
// Eight workers permit independent hosts and lanes to progress while preserving FIFO per
// lane. A device has up to seven lanes (metadata, capabilities, bulk, interactive, panel,
// files, hover), and opening a lane's helper channel blocks its worker for a handshake:
// with fewer workers a Files read waited on another lane's channel setup.
// Both the incoming channel and scheduler backlog are bounded. Sleeping workers use a
// condition variable, so an idle fleet causes no polling or redraw loop.
pub(super) fn start_task_workers<W: From<Reply> + Send + 'static>(
    rx: mpsc::Receiver<Task>,
    replies: mpsc::Sender<W>,
) {
    use std::sync::{Arc, Condvar, Mutex};
    #[derive(Default)]
    struct Queue {
        tasks: VecDeque<Task>,
        busy: BTreeSet<String>,
        closed: bool,
    }
    fn endpoint(task: &Task) -> String {
        // Interactive traffic (screens, keystrokes, the watch) and a command
        // panel's start run on their own lane, as the former have their own
        // helper channel: never serialized behind slow refreshes.
        let panel = matches!(task.op, Operation::CommandPanel { .. })
            || transport::channel_class(&task.op) == transport::Channel::Interactive;
        // Bulk work (files, containers, transfers) has its own lane, as it
        // has its own helper channel: a container burst never delays sessions.
        let class = transport::channel_class(&task.op);
        let bulk = class == transport::Channel::Bulk;
        // File browsing reads have their own lane and helper channel too: a listing
        // never waits for a container burst, a transfer or the startup inventory.
        let files = class == transport::Channel::Files;
        if matches!(task.op, Operation::Info) {
            return format!(
                "capabilities:{}",
                task.execution.target.as_deref().unwrap_or("<local>")
            );
        }
        match &task.execution.target {
            Some(target) if panel => format!("panel:{target}"),
            Some(target) if bulk => format!("bulk:{target}"),
            Some(target) if files && task.hover => format!("files-hover:{target}"),
            Some(target) if files => format!("files:{target}"),
            Some(target) => target.clone(),
            None if panel => "<local-command>".into(),
            None if matches!(
                task.op,
                Operation::List { .. }
                    | Operation::ListPage { .. }
                    | Operation::Preview { .. }
                    | Operation::PreviewPage { .. }
            ) =>
            {
                "<local-files-read>".into()
            }
            None if bulk => "<local-bulk>".into(),
            None => "<local>".into(),
        }
    }
    let shared = Arc::new((Mutex::new(Queue::default()), Condvar::new()));
    let input = shared.clone();
    let discarded_replies = replies.clone();
    thread::spawn(move || {
        let (lock, changed) = &*input;
        while let Ok(task) = rx.recv() {
            let mut queue = lock.lock().unwrap();
            for old in workspaces::superseded_reads(&mut queue.tasks, &task) {
                let _ = discarded_replies.send(W::from(Reply {
                    device_id: Some(old.execution.id.clone()),
                    execution_target: Some(old.execution.target.clone()),
                    preview: None,
                    device: old.device,
                    op: old.op,
                    generation: old.generation,
                    result: Ok(Value::Null),
                }));
            }
            while queue.tasks.len() >= 32 {
                queue = changed.wait(queue).unwrap();
            }
            queue.tasks.push_back(task);
            changed.notify_all();
        }
        lock.lock().unwrap().closed = true;
        changed.notify_all();
    });
    for _ in 0..8 {
        let shared = shared.clone();
        let replies = replies.clone();
        thread::spawn(move || {
            let (lock, changed) = &*shared;
            loop {
                let (task, host) = {
                    let mut queue = lock.lock().unwrap();
                    loop {
                        if let Some(index) = queue
                            .tasks
                            .iter()
                            .position(|task| !queue.busy.contains(&endpoint(task)))
                        {
                            let task = queue.tasks.remove(index).unwrap();
                            let host = endpoint(&task);
                            queue.busy.insert(host.clone());
                            changed.notify_all();
                            break (task, host);
                        }
                        if queue.closed && queue.tasks.is_empty() {
                            return;
                        }
                        queue = changed.wait(queue).unwrap();
                    }
                };
                let result = if task.hover {
                    transport::request_hover(&task.execution, task.op.clone())
                } else {
                    transport::request(&task.execution, task.op.clone())
                };
                let preview = if matches!(
                    match &task.op {
                        Operation::ContainerFiles { operation, .. } => operation.as_ref(),
                        other => other,
                    },
                    Operation::Preview { .. } | Operation::PreviewPage { .. }
                ) {
                    // Plain text paints first; source highlighting follows below.
                    result.as_ref().ok().map(RichPreview::plain)
                } else if matches!(task.op, Operation::ReadChunk { .. }) {
                    // Streamed text previews: decode and highlight off the UI thread.
                    result
                        .as_ref()
                        .ok()
                        .and_then(|value| stream_chunk(&task.op, value))
                } else {
                    None
                };
                let source = preview
                    .as_ref()
                    .filter(|p| p.kind == "code")
                    .and(result.as_ref().ok())
                    .map(|value| {
                        let text = safe_text(value["text"].as_str().unwrap_or(""));
                        (text, value["path"].as_str().unwrap_or("").to_owned())
                    });
                let reply = |preview, op, result| Reply {
                    device_id: Some(task.execution.id.clone()),
                    execution_target: Some(task.execution.target.clone()),
                    preview,
                    device: task.device,
                    op,
                    generation: task.generation,
                    result,
                };
                let delivered = replies
                    .send(W::from(reply(preview, task.op.clone(), result)))
                    .is_ok();
                lock.lock().unwrap().busy.remove(&host);
                changed.notify_all();
                if !delivered {
                    return;
                }
                if let Some((text, path)) = source {
                    let highlighted = highlighted_preview(text, &path);
                    if replies
                        .send(W::from(reply(Some(highlighted), task.op, Ok(Value::Null))))
                        .is_err()
                    {
                        return;
                    }
                }
            }
        });
    }
}
/// Geometry mirrors the existing workspace layout; no preview owns another location.
pub(super) fn native_preview_area(app: &App, area: Rect) -> Option<Rect> {
    if area.width < 36
        || area.height < 10
        || app.view != View::Files
        || app.help
        || app.dialog.is_some()
        || app.input.is_some()
        || notifications::visible(&app.notice)
    {
        return None;
    }
    let browser = app.browser.as_ref()?;
    browser.preview.as_ref()?;
    let rich = browser.preview_rich.as_ref()?;
    if !matches!(rich.kind.as_str(), "image" | "pdf") || rich.raster.is_none() {
        return None;
    }
    // Where the last frame drew the preview surface.
    let _ = area;
    app.preview_rect
        .get()
        .filter(|r| r.width > 0 && r.height > 0)
}
pub(super) fn prepare_native_preview(
    app: &App,
    area: Rect,
    native: &mut crate::terminal_preview::NativePreview,
) {
    if let Some(area) = native_preview_area(app, area) {
        let browser = app.browser.as_ref().unwrap();
        let (width, height, rgba) = browser
            .preview_rich
            .as_ref()
            .unwrap()
            .raster
            .as_ref()
            .unwrap();
        // Revision changes on every accepted response, even if title/path/dimensions match.
        let key = format!(
            "{}:{}:{}",
            app.generation, browser.device, browser.preview_revision
        );
        native.prepare(&key, *width as u32, *height as u32, rgba, area);
    } else {
        native.hide();
    }
}
pub(super) fn cleanup_native_preview(
    screen: &mut Screen,
    native: &mut crate::terminal_preview::NativePreview,
) -> Result<()> {
    native.hide();
    if native.cleanup(&mut io::stdout())? {
        screen.terminal.clear()?;
    }
    Ok(())
}
/// Apply one task reply; returns (redraw, navigation cache changed).
fn apply_reply(app: &mut App, reply: Reply) -> (bool, bool) {
    if matches!(reply.op, Operation::Watch { .. }) {
        return (app.watch_replied(reply), false);
    }
    // An identical live capture changes nothing on screen: skip the frame.
    // (A thumbnail capture always lands in a tile.)
    let preview = matches!(reply.op, Operation::SessionPreview { .. })
        && reply.generation != super::session_preview::THUMB_GENERATION;
    let revision = app.session_preview_revision();
    let cached = matches!(
        reply.op,
        Operation::Sessions
            | Operation::ExternalSessions
            | Operation::Containers
            | Operation::List { .. }
            | Operation::ListPage { .. }
            | Operation::ContainerFiles { .. }
    ) && reply.result.is_ok();
    app.apply(reply);
    (
        !preview || app.session_preview_revision() != revision,
        cached,
    )
}
/// With the input thread parked: apply queued replies and events, and move
/// the first typed (non-passive) input, queued or still unread, into
/// `carried`. Returns whether there was one.
fn queued_input(
    wake_rx: &mpsc::Receiver<Wake>,
    app: &mut App,
    carried: &mut Vec<(Event, Instant)>,
) -> Result<bool> {
    while let Ok(wake) = wake_rx.try_recv() {
        match wake {
            Wake::Input(input, at) => {
                if !passive_input(&input) {
                    carried.push((input, at));
                    break;
                }
            }
            Wake::Reply(reply) => {
                apply_reply(app, *reply);
            }
            Wake::Event(target, event) => {
                app.watch_event(target, event);
            }
        }
    }
    while carried.is_empty() && event::poll(Duration::ZERO)? {
        let input = event::read()?;
        if !passive_input(&input) {
            carried.push((input, Instant::now()));
        }
    }
    Ok(!carried.is_empty())
}
pub fn run() -> Result<()> {
    run_restored(None)
}
pub fn run_restored(restore: Option<&str>) -> Result<()> {
    let stopping = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    for sig in [
        signal_hook::consts::SIGTERM,
        signal_hook::consts::SIGHUP,
        signal_hook::consts::SIGINT,
    ] {
        signal_hook::flag::register(sig, stopping.clone())?;
    }
    // Detect terminal capabilities (incl. the bounded OSC 11 background query)
    // before any thread or raw mode touches the terminal.
    let _ = super::caps::caps();
    let devices = store::devices()?;
    anyhow::ensure!(!devices.is_empty(), "No local device available");
    // Off the start path: attach guards a killed attach left behind.
    thread::spawn(crate::sessions::sweep_attach_leftovers);
    let (tx, rx) = mpsc::sync_channel::<Task>(32);
    // One wake source: input, replies and helper events share this channel.
    let (wake_tx, wake_rx) = mpsc::channel::<Wake>();
    start_task_workers(rx, wake_tx.clone());
    let events = std::sync::Mutex::new(wake_tx.clone());
    transport::set_event_sink(Box::new(move |target, event| {
        if let Ok(events) = events.lock() {
            let _ = events.send(Wake::Event(target, event));
        }
    }));
    let (core_tx, core_rx) = mpsc::channel();
    let core_stopping = stopping.clone();
    thread::spawn(move || {
        while !core_stopping.load(std::sync::atomic::Ordering::Relaxed) {
            let result = crate::peers::sync();
            if core_tx.send(result).is_err() {
                break;
            }
            for _ in 0..60 {
                if core_stopping.load(std::sync::atomic::Ordering::Relaxed) {
                    return;
                }
                thread::sleep(Duration::from_secs(1));
            }
        }
    });
    let mut last_inventory_check = Instant::now();
    let mut app = App::new(devices, tx);
    load_cache(&mut app);
    app.pins = store::session_pins().unwrap_or_default();
    if let Ok(Some(cache)) =
        store::read_state::<workspaces::NavigationCache>("ui-navigation.json", 2 * 1024 * 1024)
    {
        cache.restore(&mut app);
    }
    let navigation_writer = workspaces::cache_writer();
    let _ = sweep_stale_restarts(&store::state_dir(), Duration::from_secs(600));
    if let Some(name) = restore {
        if let Err(error) = restore_restart(&mut app, name) {
            app.set_notice(safe_text(&format!(
                "Update restored workspace defaults: {error:#}"
            )));
        }
    }
    if restore.is_some() && app.view == View::Files {
        app.refresh_browser();
        if let Some(b) = app.browser.as_ref() {
            let (d, path) = (b.device, b.path.clone());
            app.yazi_warm_hover(d, path);
        }
        app.yazi_warm_devices();
    }
    if restore.is_none() {
        // The start view is the hub; Files of this device open in the launch folder.
        app.open_startup_directory(std::env::current_dir()?);
        app.view = View::Network;
        app.device = 0;
    }
    app.reload_edges();
    app.refresh_work();
    app.refresh_containers();
    for d in 0..app.devices.len() {
        app.check_providers(d);
        app.send(d, Operation::TransferJobs);
    }
    let mut screen = Screen::new().context("open terminal workspace")?;
    start_input(wake_tx)?;
    let mut prewarmed: Option<usize> = None;
    spawn_exit_watchdog(stopping.clone());
    // Drop ordering cleans owned bitmaps before Screen restores the original terminal.
    let mut native_preview = crate::terminal_preview::NativePreview::detect();
    let mut dirty = true;
    let mut last_draw = Instant::now() - FRAME_INTERVAL;
    let mut carried: Vec<(Event, Instant)> = Vec::new();
    let (update_tx, update_rx) = mpsc::channel::<UpdateEvent>();
    start_update_check(&update_tx, false);
    let mut update_phase = UpdatePhase::Checking;
    let mut update_report_requested = false;
    let mut last_update_check = Instant::now();
    let mut last_interaction = Instant::now();
    let mut wheel_reports = WheelReports::default();
    let mut last_refresh = Instant::now();
    let mut last_session_refresh = Instant::now();
    let mut last_containers = Instant::now();
    let mut last_jobs = Instant::now();
    let mut last_transfer_frame = Instant::now();
    let mut notice_was_animating = false;
    // A persistent askpass broker lets `sudo` while moving a terminal prompt
    // for a password without blocking the event loop or the loader animation.
    let askpass = crate::auth::Broker::start().ok();
    if let Some(broker) = &askpass {
        std::env::set_var("CX_ASKPASS_SOCKET", broker.socket());
        std::env::set_var("CX_ASKPASS_NONCE", broker.nonce());
    }
    while !app.quit && !stopping.load(std::sync::atomic::Ordering::Relaxed) && !terminal_gone() {
        // Service one pending password prompt per iteration, if any.
        if let Some((prompt, pending)) = askpass.as_ref().and_then(|b| b.accept()) {
            let answer =
                authentication_modal(&mut screen, prompt).unwrap_or(crate::auth::Answer::Cancel);
            pending.answer(answer);
            last_interaction = Instant::now();
            dirty = true;
        }
        if core_rx.try_iter().next().is_some()
            || last_inventory_check.elapsed() >= Duration::from_secs(5)
        {
            last_inventory_check = Instant::now();
            if let Ok(devices) = store::devices() {
                if app.reconcile_devices(devices) {
                    app.refresh_work();
                    for d in 0..app.devices.len() {
                        app.check_providers(d);
                    }
                    dirty = true;
                }
            }
        }
        for update in transport::drain_host_updates() {
            app.host_updated(update);
            dirty = true;
        }
        if app
            .pending_adoption
            .as_ref()
            .is_some_and(|(_, _, at)| at.elapsed() >= Duration::from_secs(12))
        {
            app.pending_adoption = None;
            app.set_notice_as(
                NoticeKind::Warning,
                "Terminal availability timed out · no move was started".into(),
            );
            dirty = true;
        }
        app.start_next_file_action();
        if app.force_update {
            app.force_update = false;
            if matches!(update_phase, UpdatePhase::Idle) {
                start_update_check(&update_tx, true);
                update_phase = UpdatePhase::Checking;
                last_update_check = Instant::now();
                app.set_notice(version_notice("checking verified releases..."));
                dirty = true;
            } else if matches!(update_phase, UpdatePhase::Checking) {
                update_report_requested = true;
                app.set_notice(version_notice("checking verified releases..."));
                dirty = true;
            }
        }
        while let Ok(event) = update_rx.try_recv() {
            match event {
                UpdateEvent::Checked(force, result) => {
                    let force = force || std::mem::take(&mut update_report_requested);
                    update_phase = match result {
                        Ok(crate::update::CheckOutcome::Ready(plan)) => {
                            app.set_notice_as(
                                NoticeKind::Success,
                                version_notice(&format!(
                                    "v{} ready; waiting for idle",
                                    safe_label(&plan.version)
                                )),
                            );
                            dirty = true;
                            UpdatePhase::Ready(plan)
                        }
                        other => {
                            if force {
                                app.set_notice_as(
                                    update_notice_kind(&other),
                                    update_check_notice(&other),
                                );
                                dirty = true;
                            }
                            UpdatePhase::Idle
                        }
                    };
                }
                UpdateEvent::Installed(result) => {
                    update_phase = match result {
                        Ok(path) => UpdatePhase::Installed(path),
                        Err(_) => {
                            app.set_notice_as(
                                NoticeKind::Error,
                                version_notice("update could not install; version kept"),
                            );
                            dirty = true;
                            UpdatePhase::Idle
                        }
                    };
                }
            }
        }
        if matches!(
            update_phase,
            UpdatePhase::Ready(_) | UpdatePhase::Installed(_)
        ) && restart_ready(&app, last_interaction.elapsed(), !carried.is_empty())
        {
            if matches!(update_phase, UpdatePhase::Ready(_)) {
                if let UpdatePhase::Ready(plan) =
                    std::mem::replace(&mut update_phase, UpdatePhase::Installing)
                {
                    let tx = update_tx.clone();
                    thread::spawn(move || {
                        let _ = tx.send(UpdateEvent::Installed(crate::update::install(&plan)));
                    });
                    app.set_notice(version_notice("installing verified update..."));
                    dirty = true;
                }
            } else if let UpdatePhase::Installed(path) = &update_phase {
                match save_restart(&app) {
                    Ok(name) => {
                        // A key can arrive while serializing the snapshot; consume it before takeover.
                        pause_input();
                        let queued = queued_input(&wake_rx, &mut app, &mut carried)?;
                        if queued {
                            resume_input();
                            let _ = std::fs::remove_file(store::state_dir().join(name));
                            last_interaction = Instant::now();
                            continue;
                        }
                        use std::os::unix::process::CommandExt;
                        cleanup_native_preview(&mut screen, &mut native_preview)?;
                        screen.suspend(true)?;
                        let error = std::process::Command::new(path)
                            .arg("restart")
                            .arg(&name)
                            .exec();
                        let _ = std::fs::remove_file(store::state_dir().join(name));
                        resume_input();
                        screen.resume()?;
                        app.set_notice_as(
                            NoticeKind::Warning,
                            safe_text(&format!("Update installed; reopen cx to use it: {error}")),
                        );
                        dirty = true;
                        update_phase = UpdatePhase::Idle;
                    }
                    Err(_) => {
                        app.set_notice_as(NoticeKind::Warning, "Update installed · workspace too large to restore; reopen cx when convenient".into());
                        dirty = true;
                        update_phase = UpdatePhase::Idle;
                    }
                }
            }
        }
        if let Some(target) = app.pending_add.take() {
            cleanup_native_preview(&mut screen, &mut native_preview)?;
            let via = app.pending_add_via.take();
            let owned_target = target.clone();
            app.set_notice(format!("Connecting to {}", safe_label(&target)));
            screen
                .terminal
                .draw(|frame| render_with_native(frame, &app, None))?;
            let result = crate::auth::with_broker(
                move || crate::add_via(&owned_target, via.as_ref()),
                |prompt| authentication_modal(&mut screen, prompt),
            );
            match result {
                Ok(()) => {
                    let updated = store::devices()?;
                    let old: HashMap<_, _> = app
                        .devices
                        .iter()
                        .zip(app.work.drain(..))
                        .map(|(d, w)| (d.id.clone(), w))
                        .collect();
                    app.work = updated
                        .iter()
                        .map(|d| {
                            old.get(&d.id)
                                .map(|w| Cached {
                                    sessions: w.sessions.clone(),
                                    loading: false,
                                    error: w.error.clone(),
                                    fetched: w.fetched,
                                })
                                .unwrap_or(Cached {
                                    sessions: vec![],
                                    loading: false,
                                    error: None,
                                    fetched: 0,
                                })
                        })
                        .collect();
                    app.devices = updated;
                    app.policies = app
                        .devices
                        .iter()
                        .map(|device| store::policy(device).unwrap_or_default())
                        .collect();
                    app.set_notice_as(
                        NoticeKind::Success,
                        format!("Added {}", safe_label(&target)),
                    );
                    app.refresh_work();
                }
                Err(e) => app.set_notice_as(
                    NoticeKind::Error,
                    safe_text(&format!("Enrollment failed: {e:#}")),
                ),
            }
            dirty = true;
        }
        if let Some(d) = app.pending_ptrace.take() {
            cleanup_native_preview(&mut screen, &mut native_preview)?;
            let device = app.devices[d].clone();
            let name = if device.target.is_none() {
                "local".to_string()
            } else {
                device.name.clone()
            };
            screen.suspend(true)?;
            eprintln!(
                "\nSetting up terminal moving on {}. This installs a small helper and asks for your \n\
                 administrator password once; it is only used to let CX move terminals you own.\n",
                identity(&device)
            );
            let result = crate::device_ptrace(&name, crate::PtraceAction::Enable, None);
            screen.resume()?;
            last_interaction = Instant::now();
            match result {
                Ok(_) => {
                    app.providers.remove(&d);
                    app.check_providers(d);
                    app.set_notice(
                        "Terminal moving is set up · select the terminal again to move it".into(),
                    );
                }
                Err(e) => app.set_notice_as(
                    NoticeKind::Error,
                    safe_text(&format!("Setup did not complete: {e:#}")),
                ),
            }
            dirty = true;
        }
        if let Some(device) = app.pending_terminal.take() {
            cleanup_native_preview(&mut screen, &mut native_preview)?;
            screen.suspend(true)?;
            let result = sessions::login_terminal(&device);
            screen.resume()?;
            last_interaction = Instant::now();
            app.set_notice_as(
                if result.is_err() {
                    NoticeKind::Error
                } else {
                    NoticeKind::Info
                },
                match result {
                    Ok(()) => format!("Returned from {}", identity(&device)),
                    Err(e) => safe_text(&format!("Terminal failed: {e:#}")),
                },
            );
            dirty = true;
        }
        if let Some((d, command)) = app.pending_command_panel.take() {
            // Open a floating panel for the command instead of leaving CX. A
            // panel is already open? Close it first.
            if let Some(panel) = app.command_panel.take() {
                app.send(panel.device, Operation::PaneClose { id: panel.id });
            }
            let (cols, rows) = command_panel_size(screen.terminal.size()?);
            app.command_panel_label = Some(safe_text(&command.command));
            app.send(
                d,
                Operation::CommandPanel {
                    request: command,
                    cols,
                    rows,
                },
            );
            app.set_notice("Running…".into());
            dirty = true;
        }
        // Poll the open command panel for fresh output while it runs.
        if let Some(panel) = app.command_panel.as_ref() {
            let due = panel.last_poll.elapsed() >= Duration::from_millis(100);
            if !panel.inflight && due && (panel.alive || panel.lines.is_empty()) {
                let (id, device, cols, rows) =
                    (panel.id.clone(), panel.device, panel.cols, panel.rows);
                if app.send(device, Operation::CapturePane { id, cols, rows }) {
                    if let Some(panel) = app.command_panel.as_mut() {
                        panel.inflight = true;
                    }
                }
            }
        }
        if let Some((d, session)) = app.pending_panel_expand.take() {
            cleanup_native_preview(&mut screen, &mut native_preview)?;
            app.stop_watches();
            screen.suspend(true)?;
            let result = sessions::attach(&app.devices[d], &session, false);
            screen.resume()?;
            last_interaction = Instant::now();
            if let Err(e) = result {
                app.set_notice_as(
                    NoticeKind::Error,
                    safe_text(&format!("Expand failed: {e:#}")),
                );
            }
            dirty = true;
        }
        if let Some((zoxide, dir)) = app.pending_jump.take() {
            // `z`/`Z`: fzf or zoxide owns the terminal; its answer is where Files goes.
            cleanup_native_preview(&mut screen, &mut native_preview)?;
            app.stop_watches();
            screen.suspend(true)?;
            let mut command = if zoxide {
                let mut c = std::process::Command::new("zoxide");
                c.args(["query", "-i"]);
                c
            } else {
                std::process::Command::new("fzf")
            };
            let output = command
                .current_dir(&dir)
                .stdin(std::process::Stdio::inherit())
                .stderr(std::process::Stdio::inherit())
                .stdout(std::process::Stdio::piped())
                .output();
            screen.resume()?;
            last_interaction = Instant::now();
            if let Some(out) = output.ok().filter(|o| o.status.success()) {
                let picked = String::from_utf8_lossy(&out.stdout).trim().to_owned();
                if !picked.is_empty() {
                    app.yazi_jump_to(&dir, &picked);
                }
            }
            dirty = true;
        }
        if let Some((device, command)) = app.pending_command.take() {
            cleanup_native_preview(&mut screen, &mut native_preview)?;
            app.stop_watches();
            screen.suspend(true)?;
            let result = sessions::run_command(&device, &command);
            screen.resume()?;
            last_interaction = Instant::now();
            app.set_notice_as(
                if result.is_err() {
                    NoticeKind::Error
                } else {
                    NoticeKind::Info
                },
                match result {
                    Ok(()) => format!("Returned from {}", identity(&device)),
                    Err(e) => safe_text(&format!("Command failed: {e:#}")),
                },
            );
            if app.view == View::Files {
                app.refresh();
            }
            dirty = true;
        }
        if app.pending_attach.is_some() && carried.is_empty() {
            // Typed input stays cx-owned: a queued key, paste, click or wheel is
            // handled first and may cancel the attachment. Passive reports
            // (pointer motion, button release, focus, resize) never postpone it.
            // Parked first, the input thread cannot read ahead into the attach.
            pause_input();
            if queued_input(&wake_rx, &mut app, &mut carried)? || app.pending_attach.is_none() {
                resume_input();
            }
        }
        if app.pending_attach.is_some() && carried.is_empty() {
            let (d, session, observe) = app.pending_attach.take().unwrap();
            cleanup_native_preview(&mut screen, &mut native_preview)?;
            app.stop_watches();
            // The tmux client paints over the frame it shows: the live screen.
            if app.show_attach_frame(&session) {
                let size = screen.terminal.size()?;
                screen.draw(|frame| {
                    session_preview::render_attach_frame(
                        frame,
                        &app,
                        Rect::new(0, 0, size.width, size.height),
                    )
                })?;
            }
            screen.hand_off()?;
            // The drain's pause ends here; hand_off holds its own until resume.
            resume_input();
            let result = sessions::attach(&app.devices[d], &session, observe);
            screen.resume()?;
            last_interaction = Instant::now();
            app.returned_from(d, &session);
            let returned = result.is_ok();
            app.set_notice_as(
                if result.is_err() {
                    NoticeKind::Error
                } else {
                    NoticeKind::Info
                },
                match result {
                    Ok(()) => format!(
                        "Returned from {} · session remains on execution host",
                        identity(&app.devices[d])
                    ),
                    Err(e) => safe_text(&format!("Attachment failed: {e:#}")),
                },
            );
            if returned {
                // Back at once: the notice appears without its fade-in.
                app.notice_started = app
                    .notice_started
                    .and_then(|t| t.checked_sub(notifications::TRANSITION));
            }
            app.refresh_work();
            dirty = true;
        }
        dirty |= app.expire_notice(Instant::now());
        dirty |= app.poll_edge_job();
        dirty |= app.poll_invite();
        dirty |= app.poll_secret_job();
        let was_encoding = native_preview.pending();
        let size = screen.terminal.size()?;
        prepare_native_preview(
            &app,
            Rect::new(0, 0, size.width, size.height),
            &mut native_preview,
        );
        if was_encoding && !native_preview.pending() {
            // The fitted native image may occupy fewer cells than the fallback.
            // Reset the terminal cache so old half-block rows cannot survive.
            screen.terminal.clear()?;
            dirty = true;
        }
        if native_preview.cleanup(&mut io::stdout())? {
            screen.terminal.clear()?;
            dirty = true;
        }
        // Animate only while work is pending; idle keeps its existing bounded redraw.
        if (app.creating
            || !app.active_transfer_keys().is_empty()
            || (matches!(app.view, View::Work | View::Network)
                && app
                    .session_rows()
                    .iter()
                    .any(|(d, s)| session_activity_current(&app.work[*d]) && session_busy(s))))
            && last_transfer_frame.elapsed() >= Duration::from_millis(160)
        {
            app.transfer_frame = app.transfer_frame.wrapping_add(1);
            last_transfer_frame = Instant::now();
            dirty = true;
        }
        let notice_is_animating =
            notifications::animating(app.notice_started, app.notice_deadline, Instant::now());
        dirty |= notice_is_animating || notice_was_animating;
        notice_was_animating = notice_is_animating;
        // Capture wheel reports throughout cx so the emulator does not replace
        // them with accelerated arrow-key bursts. suspend() releases this before
        // handing the terminal to SSH/tmux/native commands.
        screen.bracketed_paste(app.pane_focused() || app.secret_entry_open())?;
        screen.mouse_capture(true)?;
        dirty |= app.refresh_session_preview();
        app.sync_watches();
        app.fetch_thumbnails();
        // Warm the master of the device the selection points at (hover/focus).
        let focus = app
            .selected_session()
            .map(|(d, _)| d)
            .or(app.device.checked_sub(1));
        if focus != prewarmed {
            if let Some(d) = focus.filter(|d| *d < app.devices.len()) {
                transport::prewarm(&app.devices[d]);
            }
            prewarmed = focus;
        }
        let now = Instant::now();
        dirty |= app.tick_press(now);
        dirty |= app.files_tick(now);
        // Coalesce bursts (key repeat, reply floods) to at most ~60 fps.
        let mut frame_wait = None;
        if dirty {
            let since = last_draw.elapsed();
            let grace = app.live_frame_grace();
            let interval = frame_interval(&app);
            if let Some(grace) = grace {
                frame_wait = Some(grace.max(interval.saturating_sub(since)));
            } else if since >= interval {
                // A pass that wrote nothing (a key with no visible effect)
                // does not delay the next frame, such as its echo.
                if screen
                    .draw(|frame| render_with_native(frame, &app, Some(&mut native_preview)))?
                {
                    last_draw = Instant::now();
                }
                dirty = false;
            } else {
                frame_wait = Some(interval - since);
            }
        }
        let batch = if !carried.is_empty() {
            std::mem::take(&mut carried)
        } else {
            let mut batch = Vec::new();
            let first = match wake_rx.recv_timeout(loop_wait(&app, now, frame_wait)) {
                Ok(wake) => Some(wake),
                Err(mpsc::RecvTimeoutError::Timeout) => None,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            };
            let mut cache_changed = false;
            for wake in first
                .into_iter()
                .chain(std::iter::from_fn(|| wake_rx.try_recv().ok()).take(255))
            {
                match wake {
                    Wake::Input(input, at) => batch.push((input, at)),
                    Wake::Reply(reply) => {
                        let (redraw, cached) = apply_reply(&mut app, *reply);
                        dirty |= redraw;
                        cache_changed |= cached;
                    }
                    Wake::Event(target, event) => {
                        let (redraw, cached) = app.watch_event(target, event);
                        dirty |= redraw;
                        cache_changed |= cached;
                    }
                }
            }
            if cache_changed {
                navigation_writer.save(workspaces::NavigationCache::snapshot(&app));
            }
            batch
        };
        {
            for (input, observed) in batch {
                if super::caps::swallow_stray_reply(&input) {
                    continue;
                }
                if !wheel_reports.accepts(&input, observed) {
                    continue;
                }
                match input {
                    Event::Key(k) => {
                        last_interaction = Instant::now();
                        app.key(k);
                        dirty = true;
                    }
                    Event::Paste(text) if app.pane_focused() => {
                        app.session_pane_paste(&text);
                        dirty = true;
                    }
                    Event::Paste(mut text) if app.secret_entry_open() => {
                        app.secret_paste(&text);
                        zeroize::Zeroize::zeroize(&mut text);
                        dirty = true;
                    }
                    Event::Resize(_, _) => {
                        native_preview.hide();
                        dirty = true;
                    }
                    Event::Mouse(mouse) => {
                        let size = screen.terminal.size()?;
                        let handled = app.mouse(mouse, Rect::new(0, 0, size.width, size.height));
                        if handled {
                            last_interaction = Instant::now();
                            dirty = true;
                        }
                    }
                    _ => {}
                }
            }
        }
        if let Some(text) = app.copy_out.take() {
            use io::Write;
            let mut out = io::stdout();
            out.write_all(&osc52(&text))?;
            out.flush()?;
        }
        if last_jobs.elapsed() >= Duration::from_secs(2) {
            let active = app
                .jobs
                .values()
                .filter_map(|v| v["jobs"].as_array())
                .flatten()
                .any(|j| {
                    matches!(
                        j["status"].as_str(),
                        Some("queued" | "running" | "submitting")
                    )
                });
            if active {
                if let Some(local) = app.devices.iter().position(|d| d.target.is_none()) {
                    app.send(local, Operation::TransferJobs);
                }
            }
            last_jobs = Instant::now();
        }
        if matches!(update_phase, UpdatePhase::Idle)
            && last_update_check.elapsed() >= Duration::from_secs(3600)
        {
            start_update_check(&update_tx, false);
            update_phase = UpdatePhase::Checking;
            last_update_check = Instant::now();
        }
        // Containers refresh on demand, plus once a minute in the background.
        if last_containers.elapsed() >= Duration::from_secs(60) {
            app.refresh_containers();
            last_containers = Instant::now();
        }
        if matches!(app.view, View::Work | View::Network)
            && last_session_refresh.elapsed() >= Duration::from_secs(3)
        {
            for d in 0..app.devices.len() {
                // A device pushing its list (watch) needs no poll.
                if app.background_checks_enabled(d)
                    && !app.sessions_pushed(d)
                    && (app.device == 0 || app.device == d + 1)
                    && !app.work[d].loading
                {
                    app.work[d].loading = app.send(d, Operation::Sessions);
                }
            }
            last_session_refresh = Instant::now();
        }
        // Session and container caches are reconciled even while browsing files.
        // Bounded fleet refresh; idle does not continuously redraw.
        if last_refresh.elapsed() >= Duration::from_secs(30) {
            if let Some(browser) = &app.browser {
                app.check_providers(browser.device);
            }
            if let Some(Dialog::Provider(device, _) | Dialog::SessionChooser(device, _)) =
                app.dialog
            {
                app.check_providers(device);
            }
            app.refresh_work();
            if app.view == View::Network {
                app.reload_edges();
            }
            if app.browser.is_some() {
                if let Some(local) = app.devices.iter().position(|d| d.target.is_none()) {
                    app.send(local, Operation::TransferJobs);
                }
            }
            last_refresh = Instant::now();
        }
    }
    cleanup_native_preview(&mut screen, &mut native_preview)?;
    navigation_writer.finish(workspaces::NavigationCache::snapshot(&app));
    Ok(())
}
