//! Terminal capability detection (design spec section 9).
//!
//! Detection is a pure function of an environment lookup and a lazy probe, so
//! the whole matrix is table-tested. The live result is computed once per
//! process ([`caps`]) and never re-reads the environment afterwards. Probes are
//! bounded (infocmp 100 ms, tmux 200 ms) and run only when the environment
//! alone cannot decide; they never print or log environment values.
use std::{
    io::Read,
    process::{Command, Stdio},
    sync::OnceLock,
    time::{Duration, Instant},
};

/// Colors the terminal can show. Ordered: a higher depth can show every lower one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum ColorDepth {
    /// `NO_COLOR` or `TERM=dumb`: attributes only.
    None,
    Ansi16,
    Ansi256,
    TrueColor,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Background {
    Dark,
    Light,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Caps {
    pub color: ColorDepth,
    /// False selects the ASCII glyph set (borders included).
    pub unicode: bool,
    pub background: Background,
    pub tmux: bool,
    pub termux: bool,
    /// Termux or Blink: prefer the XS layout, larger chips and the tap bar.
    pub phone: bool,
    /// OSC 8 hyperlinks on the allowlist (never inside tmux).
    pub hyperlinks: bool,
    /// Heuristic only; icons still need `CX_ICONS=1` ([`Caps::icons`]).
    pub nerd_font_likely: bool,
    pub icons: bool,
    /// Key chips may be pills with rounded powerline caps (U+E0B6, U+E0B4): only where
    /// the terminal draws those glyphs itself, or `CX_NERD_FONT=1` (`=0` turns it off).
    pub pills: bool,
}

impl Default for Caps {
    /// The deterministic profile used when nothing is known: truecolor, dark, Unicode.
    fn default() -> Self {
        Self {
            color: ColorDepth::TrueColor,
            unicode: true,
            background: Background::Dark,
            tmux: false,
            termux: false,
            phone: false,
            hyperlinks: false,
            nerd_font_likely: false,
            icons: false,
            pills: false,
        }
    }
}

/// Terminfo facts for `$TERM`, when a probe could read them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Terminfo {
    pub colors: Option<u32>,
    /// `Tc` or `RGB` extended capability.
    pub rgb: bool,
}

/// Lazy side-channel facts. Implementations must be bounded and read-only.
pub(crate) trait Probe {
    fn terminfo(&self, _term: &str) -> Option<Terminfo> {
        None
    }
    /// `Some(true)` when the enclosing tmux client reports the `RGB` feature.
    fn tmux_rgb(&self) -> Option<bool> {
        None
    }
    /// Terminal background from an OSC 11 query (spec 4.1), when answered in time.
    fn osc11_background(&self) -> Option<Background> {
        None
    }
}

/// A probe that knows nothing; detection then uses the environment alone.
#[cfg(test)]
pub(crate) struct NoProbe;
#[cfg(test)]
impl Probe for NoProbe {}

impl Caps {
    /// Detect from an environment lookup (`None` = unset) and a probe.
    pub(crate) fn detect(env: &dyn Fn(&str) -> Option<String>, probe: &dyn Probe) -> Self {
        let get = |k: &str| env(k);
        let set = |k: &str| env(k).is_some();
        let nonempty = |k: &str| env(k).filter(|v| !v.is_empty());
        let term = get("TERM").unwrap_or_default().to_ascii_lowercase();
        let program = get("TERM_PROGRAM").unwrap_or_default();
        let lc_terminal = get("LC_TERMINAL").unwrap_or_default();
        let colorterm = get("COLORTERM").unwrap_or_default().to_ascii_lowercase();
        let tmux = nonempty("TMUX").is_some();
        let termux =
            set("TERMUX_VERSION") || get("PREFIX").is_some_and(|p| p.contains("com.termux"));
        let blink = program.eq_ignore_ascii_case("Blink") || lc_terminal == "Blink";
        let kitty = set("KITTY_WINDOW_ID") || term == "xterm-kitty";
        let ghostty = program.eq_ignore_ascii_case("ghostty")
            || set("GHOSTTY_RESOURCES_DIR")
            || term == "xterm-ghostty";
        let foot = term.starts_with("foot");
        let alacritty =
            set("ALACRITTY_SOCKET") || set("ALACRITTY_WINDOW_ID") || term == "alacritty";
        let wezterm = program == "WezTerm";
        let iterm = program == "iTerm.app";
        let vscode = program == "vscode";
        let dumb = term == "dumb";
        let console = term == "linux";

        let truecolor_env = colorterm == "truecolor" || colorterm == "24bit";
        let color = if set("NO_COLOR") || dumb {
            ColorDepth::None
        } else if tmux {
            // Inside tmux the outer terminal's signals (KITTY_WINDOW_ID, ...) are
            // inherited but prove nothing about what tmux forwards.
            if truecolor_env || probe.tmux_rgb() == Some(true) {
                ColorDepth::TrueColor
            } else if term.contains("256color")
                || probe.terminfo(&term).and_then(|t| t.colors).unwrap_or(0) >= 256
            {
                ColorDepth::Ansi256
            } else {
                // tmux itself always draws 256 colors; screen/tmux TERMs without
                // the suffix are a conservative configuration, honour it.
                ColorDepth::Ansi16
            }
        } else if truecolor_env
            || term.ends_with("-direct")
            || kitty
            || ghostty
            || foot
            || alacritty
            || wezterm
            || iterm
            || vscode
        {
            ColorDepth::TrueColor
        } else if term.contains("256color") {
            ColorDepth::Ansi256
        } else if console || term.is_empty() {
            ColorDepth::Ansi16
        } else {
            match probe.terminfo(&term) {
                Some(Terminfo { rgb: true, .. }) => ColorDepth::TrueColor,
                Some(Terminfo {
                    colors: Some(n), ..
                }) if n >= 256 => ColorDepth::Ansi256,
                _ => ColorDepth::Ansi16,
            }
        };

        let unicode = crate::sessions::unicode_from(env);

        let background = match get("CX_THEME").as_deref().map(str::to_ascii_lowercase) {
            Some(v) if v == "light" => Background::Light,
            Some(v) if v == "dark" => Background::Dark,
            // CX_THEME, then OSC 11, then COLORFGBG, then dark (spec 4.1). The
            // query is skipped where it cannot be answered: TERM=dumb, and Blink,
            // which leaves it unanswered.
            _ => (!dumb && !blink)
                .then(|| probe.osc11_background())
                .flatten()
                .or_else(|| get("COLORFGBG").and_then(|v| colorfgbg_background(&v)))
                .unwrap_or(Background::Dark),
        };

        let vte_links = get("VTE_VERSION")
            .and_then(|v| v.trim().parse::<u32>().ok())
            .is_some_and(|v| v >= 5000);
        let hyperlinks =
            !tmux && (kitty || foot || ghostty || wezterm || iterm || vte_links || alacritty);
        Self {
            color,
            unicode,
            background,
            tmux,
            termux,
            phone: termux || blink,
            hyperlinks,
            nerd_font_likely: kitty || ghostty || wezterm,
            icons: get("CX_ICONS").as_deref() == Some("1"),
            // Terminals that draw U+E0B4 and U+E0B6 themselves, whatever the font:
            // - kitty: kitty/decorations.c maps both to `filled_D` (a half disc);
            // - ghostty: src/font/sprite/draw/powerline.zig, `drawE0B4` and `drawE0B6`;
            // - wezterm: wezterm-gui/src/customglyph.rs has `0xe0b4` and `0xe0b6` polys,
            //   on by default (`custom_block_glyphs`, docs: "Powerline triangle, curve
            //   and diagonal glyphs").
            // foot draws only U+2500-259F, U+2800-28FF, U+1CD00-1CDE5 and U+1FB00-1FB9B
            // itself (foot.ini(5), `box-drawings-uses-font-glyphs`), so the caps there
            // depend on the font: `CX_NERD_FONT=1`. Inside tmux the outer terminal's
            // variables are inherited and still name the terminal that draws.
            pills: match get("CX_NERD_FONT").as_deref() {
                Some("1") => true,
                Some("0") => false,
                _ => kitty || ghostty || wezterm,
            },
        }
    }

    /// One bounded line for debug logs: no environment values, only verdicts.
    #[allow(dead_code)]
    pub(crate) fn summary(&self) -> String {
        format!(
            "caps color={:?} unicode={} bg={:?} tmux={} phone={} links={} nerd={}",
            self.color,
            self.unicode,
            self.background,
            self.tmux,
            self.phone,
            self.hyperlinks,
            self.nerd_font_likely
        )
    }
}

/// `COLORFGBG` is `fg;bg` or `fg;default;bg`. ANSI 0-6 and 8 are dark backgrounds.
fn colorfgbg_background(value: &str) -> Option<Background> {
    let bg = value.rsplit(';').next()?.trim().parse::<u8>().ok()?;
    Some(if bg == 7 || (9..=15).contains(&bg) {
        Background::Light
    } else {
        Background::Dark
    })
}

/// Parse an OSC 11 reply (`ESC ] 11 ; rgb:RRRR/GGGG/BBBB` + BEL or ST; 1-4 hex
/// digits per channel, `rgba:` accepted) anywhere in `bytes`.
pub(crate) fn osc11_reply_background(bytes: &[u8]) -> Option<Background> {
    let text = String::from_utf8_lossy(bytes);
    let start = text.find("\x1b]11;")? + 5;
    let body = &text[start..];
    let end = body.find(['\x07', '\x1b']).unwrap_or(body.len());
    let spec = body[..end].trim();
    let channels = spec
        .strip_prefix("rgb:")
        .or_else(|| spec.strip_prefix("rgba:"))?;
    let mut parts = channels.split('/');
    let mut channel = || -> Option<f64> {
        let hex = parts.next()?;
        if hex.is_empty() || hex.len() > 4 {
            return None;
        }
        let value = u32::from_str_radix(hex, 16).ok()?;
        Some(f64::from(value) / f64::from((1u32 << (4 * hex.len() as u32)) - 1))
    };
    let (r, g, b) = (channel()?, channel()?, channel()?);
    // sRGB relative luminance; mid-grey (#777 ~ 0.18) and below count as dark.
    let lin = |c: f64| {
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    let luminance = 0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b);
    Some(if luminance > 0.3 {
        Background::Light
    } else {
        Background::Dark
    })
}

/// Until when a timed-out probe's replies may still arrive (as keystrokes).
static STRAY_UNTIL: std::sync::Mutex<Option<Instant>> = std::sync::Mutex::new(None);
const STRAY_WINDOW: Duration = Duration::from_secs(3);

/// Query the terminal background with OSC 11 followed by a DA1 sentinel (every
/// terminal answers DA1, so an unanswered OSC 11 ends early instead of after the
/// limit). Reads `/dev/tty` with echo and canonical mode off, restores the
/// previous termios, and never blocks longer than `limit`. Skipped unless stdin
/// and stdout are terminals. If the sentinel did not arrive in time, a late reply
/// would reach the event loop as `Alt+]`/`Alt+_` and printable keys, so for a few
/// seconds [`ReplyFilter`] drops the events that spell a complete expected reply.
fn query_osc11(limit: Duration) -> Option<Background> {
    use std::os::fd::AsRawFd;
    if unsafe { libc::isatty(0) != 1 || libc::isatty(1) != 1 } {
        return None;
    }
    let mut tty = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags_noctty()
        .open("/dev/tty")
        .ok()?;
    let fd = tty.as_raw_fd();
    let mut saved: libc::termios = unsafe { std::mem::zeroed() };
    if unsafe { libc::tcgetattr(fd, &mut saved) } != 0 {
        return None;
    }
    let mut raw = saved;
    raw.c_lflag &= !(libc::ICANON | libc::ECHO);
    raw.c_cc[libc::VMIN] = 0;
    raw.c_cc[libc::VTIME] = 0;
    if unsafe { libc::tcsetattr(fd, libc::TCSANOW, &raw) } != 0 {
        return None;
    }
    let started = Instant::now();
    let deadline = started + limit;
    let mut reply = Vec::new();
    let mut answered = false;
    // Then yazi's graphics probe (kitty query, cell pixels, DA1; inside tmux also
    // through passthrough to the terminal outside it): see `terminal_preview::probe_query`.
    let (graphics, da1_wanted, tmux) = crate::terminal_preview::probe_query();
    let mut query = b"\x1b]11;?\x07".to_vec();
    query.extend_from_slice(&graphics);
    if std::io::Write::write_all(&mut tty, &query).is_ok() {
        let mut chunk = [0u8; 256];
        while reply.len() < 1024 {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            let mut pfd = libc::pollfd {
                fd,
                events: libc::POLLIN,
                revents: 0,
            };
            let ready = unsafe { libc::poll(&mut pfd, 1, left.as_millis().max(1) as i32) };
            if ready < 0 && std::io::Error::last_os_error().raw_os_error() == Some(libc::EINTR) {
                continue;
            }
            if ready <= 0 {
                break;
            }
            match tty.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(n) => reply.extend_from_slice(&chunk[..n]),
            }
            if crate::terminal_preview::da1_replies(&reply).len() >= da1_wanted {
                answered = true;
                let ms = started.elapsed().as_millis().min(u128::from(u32::MAX - 1)) as u32;
                TERMINAL_RTT_MS.store(ms, std::sync::atomic::Ordering::Relaxed);
                break;
            }
        }
    }
    // A terminal that does not know APC prints the query's body: erase that line
    // (the line under the shell prompt, blank when cx starts).
    let _ = std::io::Write::write_all(&mut tty, b"\r\x1b[2K");
    unsafe {
        libc::tcsetattr(fd, libc::TCSANOW, &saved);
    }
    crate::terminal_preview::note_probe(&reply, tmux);
    if !answered {
        // (No reply within the limit: at least that slow.)
        let ms = limit.as_millis().min(u128::from(u32::MAX - 2)) as u32 + 1;
        TERMINAL_RTT_MS.store(ms, std::sync::atomic::Ordering::Relaxed);
        if let Ok(mut guard) = STRAY_UNTIL.lock() {
            *guard = Some(Instant::now() + STRAY_WINDOW);
        }
    }
    osc11_reply_background(&reply)
}

/// The terminal's round trip as the startup query measured it (u32::MAX: unknown).
static TERMINAL_RTT_MS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(u32::MAX);

/// The terminal's measured round trip in milliseconds, when the startup query was
/// answered (a terminal reached over a slow link answers late).
pub(crate) fn terminal_rtt_ms() -> Option<u32> {
    let v = TERMINAL_RTT_MS.load(std::sync::atomic::Ordering::Relaxed);
    (v != u32::MAX).then_some(v)
}

trait NoCtty {
    fn custom_flags_noctty(&mut self) -> &mut Self;
}
impl NoCtty for std::fs::OpenOptions {
    fn custom_flags_noctty(&mut self) -> &mut Self {
        use std::os::unix::fs::OpenOptionsExt;
        self.custom_flags(libc::O_NOCTTY)
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Fit {
    Prefix,
    Reply,
    No,
}

/// Whether `b` is (the start of) one of the replies the startup probe asks for:
/// OSC 11, kitty graphics, XTVERSION, `CSI 16 t` and DA1 (the last two only reach
/// the event loop as keys when the terminal's write was split after its ESC).
fn reply_fit(b: &[u8]) -> Fit {
    const ST: u8 = b'\\';
    fn text(c: u8) -> bool {
        (0x20..0x7f).contains(&c)
    }
    fn num(c: u8) -> bool {
        c.is_ascii_digit() || c == b';'
    }
    // (introducer, body bytes allowed, final bytes: ESC starts an ST)
    type Form = (&'static [u8], fn(u8) -> bool, &'static [u8]);
    let forms: [Form; 5] = [
        (b"\x1b]11;", text, b"\x07\x1b"),
        (b"\x1b_G", text, b"\x1b"),
        (b"\x1bP>|", text, b"\x1b"),
        (b"\x1b[6;", num, b"t"),
        (b"\x1b[?", num, b"c"),
    ];
    let mut best = Fit::No;
    for (intro, body, ends) in forms {
        let fit = if b.len() <= intro.len() {
            if intro.starts_with(b) {
                Fit::Prefix
            } else {
                Fit::No
            }
        } else if !b.starts_with(intro) || b.len() > 256 {
            Fit::No
        } else {
            let rest = &b[intro.len()..];
            match rest.iter().position(|&c| !body(c)) {
                None => Fit::Prefix,
                Some(i) if !ends.contains(&rest[i]) => Fit::No,
                Some(i) if rest[i] == 0x1b => match rest.len() - i {
                    1 => Fit::Prefix,
                    2 if rest[i + 1] == ST => Fit::Reply,
                    _ => Fit::No,
                },
                Some(i) if i + 1 == rest.len() => Fit::Reply,
                Some(_) => Fit::No,
            }
        };
        match fit {
            Fit::Reply => return Fit::Reply,
            Fit::Prefix => best = Fit::Prefix,
            Fit::No => {}
        }
    }
    best
}

/// The bytes a key event stands for, when it could be part of a terminal reply
/// (crossterm reads `ESC x` as Alt+x, BEL as Ctrl+G).
fn event_bytes(event: &crossterm::event::Event) -> Option<Vec<u8>> {
    use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers};
    let Event::Key(k) = event else {
        return None;
    };
    let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
    let alt = k.modifiers.contains(KeyModifiers::ALT);
    if k.kind != KeyEventKind::Press {
        return None;
    }
    match k.code {
        KeyCode::Esc if !ctrl && !alt => Some(vec![0x1b]),
        KeyCode::Char('g') if ctrl && !alt => Some(vec![0x07]),
        KeyCode::Char(c) if !ctrl => {
            let mut out = if alt { vec![0x1b] } else { Vec::new() };
            out.extend_from_slice(c.encode_utf8(&mut [0; 4]).as_bytes());
            Some(out)
        }
        _ => None,
    }
}

/// Late replies to a timed-out startup probe arrive as keystrokes (`ESC _ G i=31;OK
/// ESC \` reads as Alt+_, G, i, =, 3, 1, ;, O, K, Alt+\). For a few seconds after
/// such a timeout, events that could start a reply are held; they are dropped only
/// once they spell a COMPLETE expected reply, and are delivered unchanged as soon as
/// they cannot (or after [`REPLY_HOLD`] without the rest). Inert otherwise.
#[derive(Default)]
pub(crate) struct ReplyFilter {
    held: Vec<crossterm::event::Event>,
    bytes: Vec<u8>,
    last: Option<Instant>,
}
/// How long a possible reply's start waits for the rest of it.
const REPLY_HOLD: Duration = Duration::from_millis(150);

impl ReplyFilter {
    /// The events to deliver now, given one more from the terminal.
    pub(crate) fn push(
        &mut self,
        event: crossterm::event::Event,
        now: Instant,
    ) -> Vec<crossterm::event::Event> {
        let armed = STRAY_UNTIL
            .lock()
            .ok()
            .and_then(|g| *g)
            .is_some_and(|until| now < until);
        if self.held.is_empty() && !armed {
            return vec![event];
        }
        let Some(more) = event_bytes(&event) else {
            let mut out = self.flush();
            out.push(event);
            return out;
        };
        let mut bytes = self.bytes.clone();
        bytes.extend_from_slice(&more);
        match reply_fit(&bytes) {
            Fit::Reply => {
                self.held.clear();
                self.bytes.clear();
                Vec::new()
            }
            Fit::Prefix => {
                self.held.push(event);
                self.bytes = bytes;
                self.last = Some(now);
                Vec::new()
            }
            Fit::No if self.held.is_empty() => vec![event],
            Fit::No => {
                // Not a reply after all: deliver what was held; this event may start one.
                let mut out = self.flush();
                out.extend(self.push(event, now));
                out
            }
        }
    }
    /// When held events are due for delivery.
    pub(crate) fn deadline(&self) -> Option<Instant> {
        self.last
            .filter(|_| !self.held.is_empty())
            .map(|t| t + REPLY_HOLD)
    }
    /// Held events whose reply never completed.
    pub(crate) fn expire(&mut self, now: Instant) -> Vec<crossterm::event::Event> {
        if self.deadline().is_some_and(|d| d <= now) {
            self.flush()
        } else {
            Vec::new()
        }
    }
    /// Everything held, in order.
    pub(crate) fn flush(&mut self) -> Vec<crossterm::event::Event> {
        self.bytes.clear();
        self.last = None;
        std::mem::take(&mut self.held)
    }
}

/// Bounded, read-only probes of the real system.
struct SystemProbe;

impl Probe for SystemProbe {
    fn terminfo(&self, term: &str) -> Option<Terminfo> {
        if term.is_empty()
            || !term
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-._+".contains(&b))
        {
            return None;
        }
        let out = bounded_output(
            Command::new("infocmp").args(["-1", "-x", term]),
            Duration::from_millis(100),
        )?;
        let mut info = Terminfo::default();
        for cap in out.lines().map(|l| l.trim().trim_end_matches(',')) {
            if let Some(n) = cap.strip_prefix("colors#") {
                info.colors = if let Some(hex) = n.strip_prefix("0x") {
                    u32::from_str_radix(hex, 16).ok()
                } else {
                    n.parse().ok()
                };
            } else if cap == "Tc" || cap == "RGB" || cap.starts_with("RGB#") {
                info.rgb = true;
            }
        }
        Some(info)
    }
    fn tmux_rgb(&self) -> Option<bool> {
        // Read-only query of the user's own tmux client (tmux >= 3.2 knows the
        // format; older versions print nothing and we fall back to 256 colors).
        let out = bounded_output(
            Command::new("tmux").args(["display-message", "-p", "#{client_termfeatures}"]),
            Duration::from_millis(200),
        )?;
        let features = out.trim();
        (!features.is_empty()).then(|| features.split(',').any(|f| f == "RGB"))
    }
    fn osc11_background(&self) -> Option<Background> {
        query_osc11(Duration::from_millis(100))
    }
}

/// Run a command with null stdin/stderr in its own process group and return its
/// stdout if it exits successfully, all within `limit` (plus a few ms to reap).
/// Output is capped at 64 KiB.
///
/// The whole call is time-bounded, not just the wait for exit: stdout is read on
/// a helper thread, and once the leader exits (or the limit expires) the whole
/// group is SIGKILLed, so a wrapper that leaves a background child holding the
/// pipe can neither stall startup nor outlive the probe (the regression class of
/// `sessions::bounded_provider_check`). A descendant that escaped the group with
/// `setsid` only keeps the detached reader thread alive; the caller never waits.
fn bounded_output(command: &mut Command, limit: Duration) -> Option<String> {
    use std::os::unix::process::CommandExt;
    let started = Instant::now();
    let deadline = started + limit;
    command.process_group(0);
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let stdout = child.stdout.take()?;
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut out = Vec::new();
        let ok = stdout.take(64 * 1024).read_to_end(&mut out).is_ok();
        let _ = tx.send(ok.then_some(out));
    });
    // Observe exit without reaping: the zombie pins the group ID until the group
    // has been killed, so the signal can never reach a recycled PID.
    let exited = loop {
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
            // Group ownership cannot be established: never signal a group then.
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        if Instant::now() >= deadline {
            break false;
        }
        std::thread::sleep(Duration::from_millis(2));
    };
    // Kill leftovers (or the stalled leader) before reaping, then collect what
    // the leader wrote; the pipe closes as soon as the group is gone.
    unsafe {
        libc::kill(-(child.id() as i32), libc::SIGKILL);
    }
    let success = child.wait().is_ok_and(|s| s.success());
    if !(exited && success) {
        return None;
    }
    let grace = deadline
        .saturating_duration_since(Instant::now())
        .max(Duration::from_millis(5));
    let out = rx.recv_timeout(grace).ok()??;
    String::from_utf8(out).ok()
}

static CAPS: OnceLock<Caps> = OnceLock::new();

/// The process-wide capabilities, detected once on first use.
#[cfg(not(test))]
pub(crate) fn caps() -> Caps {
    *CAPS.get_or_init(|| Caps::detect(&|k| std::env::var(k).ok(), &SystemProbe))
}

#[cfg(test)]
thread_local! {
    static OVERRIDE: std::cell::Cell<Option<Caps>> = const { std::cell::Cell::new(None) };
}

/// Tests: a fixed truecolor/dark/Unicode profile that still honours `NO_COLOR`
/// and `CX_ASCII`, so results never depend on the developer's terminal.
#[cfg(test)]
pub(crate) fn caps() -> Caps {
    OVERRIDE.with(|o| o.get()).unwrap_or_else(|| {
        *CAPS.get_or_init(|| {
            let _ = &SystemProbe; // keep the live probe compiled in tests
            Caps {
                color: if std::env::var_os("NO_COLOR").is_some() {
                    ColorDepth::None
                } else {
                    ColorDepth::TrueColor
                },
                unicode: std::env::var_os("CX_ASCII").is_none(),
                ..Caps::default()
            }
        })
    })
}

/// Run `f` with `caps` in effect on this thread (tests only).
#[cfg(test)]
pub(crate) fn with_caps<R>(caps: Caps, f: impl FnOnce() -> R) -> R {
    struct Restore(Option<Caps>);
    impl Drop for Restore {
        fn drop(&mut self) {
            OVERRIDE.with(|o| o.set(self.0));
        }
    }
    let _restore = Restore(OVERRIDE.with(|o| o.replace(Some(caps))));
    f()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    struct Fake {
        terminfo: Option<Terminfo>,
        tmux_rgb: Option<bool>,
    }
    impl Probe for Fake {
        fn terminfo(&self, _term: &str) -> Option<Terminfo> {
            self.terminfo
        }
        fn tmux_rgb(&self) -> Option<bool> {
            self.tmux_rgb
        }
    }
    fn detect_with(pairs: &[(&str, &str)], probe: &dyn Probe) -> Caps {
        let env: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        Caps::detect(&|k| env.get(k).cloned(), probe)
    }
    fn detect(pairs: &[(&str, &str)]) -> Caps {
        detect_with(pairs, &NoProbe)
    }
    const UTF8: (&str, &str) = ("LANG", "en_US.UTF-8");

    #[test]
    fn color_depth_matrix() {
        use ColorDepth::*;
        let cases: &[(&str, &[(&str, &str)], ColorDepth)] = &[
            ("foot", &[("TERM", "foot"), UTF8], TrueColor),
            ("foot-extra", &[("TERM", "foot-extra"), UTF8], TrueColor),
            (
                "kitty",
                &[("TERM", "xterm-kitty"), ("KITTY_WINDOW_ID", "1"), UTF8],
                TrueColor,
            ),
            (
                "ghostty",
                &[("TERM", "xterm-ghostty"), ("TERM_PROGRAM", "ghostty"), UTF8],
                TrueColor,
            ),
            (
                "alacritty",
                &[("TERM", "alacritty"), ("ALACRITTY_SOCKET", "/x"), UTF8],
                TrueColor,
            ),
            (
                "alacritty-xterm",
                &[
                    ("TERM", "xterm-256color"),
                    ("ALACRITTY_WINDOW_ID", "9"),
                    UTF8,
                ],
                TrueColor,
            ),
            (
                "wezterm",
                &[
                    ("TERM", "xterm-256color"),
                    ("TERM_PROGRAM", "WezTerm"),
                    UTF8,
                ],
                TrueColor,
            ),
            (
                "iterm",
                &[
                    ("TERM", "xterm-256color"),
                    ("TERM_PROGRAM", "iTerm.app"),
                    UTF8,
                ],
                TrueColor,
            ),
            (
                "vscode",
                &[("TERM", "xterm-256color"), ("TERM_PROGRAM", "vscode"), UTF8],
                TrueColor,
            ),
            (
                "colorterm",
                &[("TERM", "xterm-256color"), ("COLORTERM", "truecolor"), UTF8],
                TrueColor,
            ),
            (
                "colorterm-24bit",
                &[("TERM", "xterm"), ("COLORTERM", "24bit"), UTF8],
                TrueColor,
            ),
            ("direct", &[("TERM", "xterm-direct"), UTF8], TrueColor),
            ("xterm-256", &[("TERM", "xterm-256color"), UTF8], Ansi256),
            ("xterm", &[("TERM", "xterm"), UTF8], Ansi16),
            ("unset-term", &[UTF8], Ansi16),
            ("console", &[("TERM", "linux")], Ansi16),
            ("dumb", &[("TERM", "dumb")], None),
            (
                "no-color",
                &[
                    ("TERM", "xterm-kitty"),
                    ("COLORTERM", "truecolor"),
                    ("NO_COLOR", "1"),
                ],
                None,
            ),
            (
                "no-color-empty",
                &[("TERM", "xterm-256color"), ("NO_COLOR", "")],
                None,
            ),
            // tmux inside kitty: inherited kitty markers are not evidence.
            (
                "tmux-in-kitty",
                &[
                    ("TERM", "tmux-256color"),
                    ("TMUX", "/tmp/s,1,0"),
                    ("KITTY_WINDOW_ID", "1"),
                    UTF8,
                ],
                Ansi256,
            ),
            (
                "tmux-colorterm",
                &[
                    ("TERM", "tmux-256color"),
                    ("TMUX", "/tmp/s,1,0"),
                    ("COLORTERM", "truecolor"),
                    UTF8,
                ],
                TrueColor,
            ),
            (
                "tmux-screen",
                &[("TERM", "screen"), ("TMUX", "/tmp/s,1,0"), UTF8],
                Ansi16,
            ),
            (
                "termux",
                &[
                    ("TERM", "xterm-256color"),
                    ("TERMUX_VERSION", "0.118"),
                    ("COLORTERM", "truecolor"),
                    UTF8,
                ],
                TrueColor,
            ),
            (
                "blink",
                &[("TERM", "xterm-256color"), ("TERM_PROGRAM", "Blink"), UTF8],
                Ansi256,
            ),
        ];
        for (name, env, want) in cases {
            assert_eq!(detect(env).color, *want, "{name}");
        }
    }

    #[test]
    fn probes_decide_only_when_the_environment_cannot() {
        let rgb = Fake {
            terminfo: Some(Terminfo {
                colors: Some(256),
                rgb: true,
            }),
            tmux_rgb: Some(true),
        };
        let tmux = [("TERM", "tmux-256color"), ("TMUX", "/tmp/s,1,0"), UTF8];
        assert_eq!(detect_with(&tmux, &rgb).color, ColorDepth::TrueColor);
        let no_rgb = Fake {
            terminfo: None,
            tmux_rgb: Some(false),
        };
        assert_eq!(detect_with(&tmux, &no_rgb).color, ColorDepth::Ansi256);
        // Unknown TERM name: terminfo decides.
        let odd = [("TERM", "rxvt-unicode"), UTF8];
        assert_eq!(detect_with(&odd, &rgb).color, ColorDepth::TrueColor);
        let colors256 = Fake {
            terminfo: Some(Terminfo {
                colors: Some(256),
                rgb: false,
            }),
            tmux_rgb: None,
        };
        assert_eq!(detect_with(&odd, &colors256).color, ColorDepth::Ansi256);
        assert_eq!(detect_with(&odd, &NoProbe).color, ColorDepth::Ansi16);
        // A panicking probe proves the env-decided path never calls it.
        struct Panics;
        impl Probe for Panics {
            fn terminfo(&self, _: &str) -> Option<Terminfo> {
                panic!("terminfo probed")
            }
            fn tmux_rgb(&self) -> Option<bool> {
                panic!("tmux probed")
            }
        }
        for env in [
            &[("TERM", "xterm-kitty"), UTF8][..],
            &[("TERM", "xterm-256color")][..],
            &[("TERM", "dumb")][..],
            &[
                ("TERM", "tmux-256color"),
                ("TMUX", "x"),
                ("COLORTERM", "truecolor"),
            ][..],
        ] {
            detect_with(env, &Panics);
        }
    }

    #[test]
    fn glyph_set_matrix() {
        let cases: &[(&str, &[(&str, &str)], bool)] = &[
            ("utf8", &[("TERM", "foot"), UTF8], true),
            ("utf8-lower", &[("TERM", "foot"), ("LANG", "C.utf8")], true),
            (
                "lang-c",
                &[("TERM", "xterm-256color"), ("LANG", "C")],
                false,
            ),
            (
                "posix",
                &[("TERM", "xterm-256color"), ("LC_ALL", "POSIX"), UTF8],
                false,
            ),
            (
                "lc_all-wins",
                &[("TERM", "xterm"), ("LC_ALL", "en_US.UTF-8"), ("LANG", "C")],
                true,
            ),
            (
                "lc_ctype",
                &[("TERM", "xterm"), ("LC_CTYPE", "en_US.ISO-8859-1"), UTF8],
                false,
            ),
            (
                "empty-lc_all-skipped",
                &[("TERM", "xterm"), ("LC_ALL", ""), UTF8],
                true,
            ),
            ("no-locale", &[("TERM", "xterm-256color")], true),
            (
                "cx-ascii",
                &[("TERM", "xterm-kitty"), ("CX_ASCII", "1"), UTF8],
                false,
            ),
            ("dumb", &[("TERM", "dumb"), UTF8], false),
            ("console", &[("TERM", "linux"), UTF8], false),
        ];
        for (name, env, want) in cases {
            assert_eq!(detect(env).unicode, *want, "{name}");
        }
    }

    #[test]
    fn phone_background_links_and_icons() {
        let termux = detect(&[
            ("TERM", "xterm-256color"),
            ("PREFIX", "/data/data/com.termux/files/usr"),
            UTF8,
        ]);
        assert!(termux.termux && termux.phone);
        let blink = detect(&[("TERM", "xterm-256color"), ("TERM_PROGRAM", "Blink"), UTF8]);
        assert!(blink.phone && !blink.termux);
        let desk = detect(&[("TERM", "foot"), UTF8]);
        assert!(!desk.phone);

        assert_eq!(
            detect(&[("CX_THEME", "light")]).background,
            Background::Light
        );
        assert_eq!(
            detect(&[("CX_THEME", "DARK"), ("COLORFGBG", "0;15")]).background,
            Background::Dark
        );
        assert_eq!(
            detect(&[("COLORFGBG", "0;15")]).background,
            Background::Light
        );
        assert_eq!(
            detect(&[("COLORFGBG", "0;default;7")]).background,
            Background::Light
        );
        assert_eq!(
            detect(&[("COLORFGBG", "15;0")]).background,
            Background::Dark
        );
        assert_eq!(
            detect(&[("COLORFGBG", "junk")]).background,
            Background::Dark
        );
        assert_eq!(detect(&[]).background, Background::Dark);

        assert!(detect(&[("TERM", "foot")]).hyperlinks);
        assert!(detect(&[("TERM", "xterm-256color"), ("VTE_VERSION", "7600")]).hyperlinks);
        assert!(!detect(&[("TERM", "xterm-256color"), ("VTE_VERSION", "4000")]).hyperlinks);
        assert!(
            !detect(&[
                ("TERM", "tmux-256color"),
                ("TMUX", "x"),
                ("KITTY_WINDOW_ID", "1")
            ])
            .hyperlinks
        );
        assert!(!detect(&[("TERM", "xterm-256color")]).hyperlinks);

        let kitty = detect(&[("TERM", "xterm-kitty")]);
        assert!(kitty.nerd_font_likely && !kitty.icons && kitty.pills);
        // Rounded key caps: drawn by kitty, ghostty and wezterm; foot needs the font.
        assert!(detect(&[("TERM", "xterm-ghostty")]).pills);
        assert!(detect(&[("TERM_PROGRAM", "WezTerm")]).pills);
        assert!(!detect(&[("TERM", "foot")]).pills);
        assert!(detect(&[("TERM", "foot"), ("CX_NERD_FONT", "1")]).pills);
        assert!(!detect(&[("TERM", "xterm-kitty"), ("CX_NERD_FONT", "0")]).pills);
        assert!(detect(&[("CX_ICONS", "1")]).icons);
        assert!(!detect(&[("CX_ICONS", "yes")]).icons);
        // The summary is one bounded line without environment values.
        let s = detect(&[("TERM", "xterm-kitty"), ("HOME", "/home/secret")]).summary();
        assert!(!s.contains('\n') && !s.contains("secret") && s.len() < 160);
    }

    #[test]
    fn override_is_scoped_and_restored() {
        let before = caps();
        let ascii = Caps {
            unicode: false,
            color: ColorDepth::None,
            ..Caps::default()
        };
        with_caps(ascii, || {
            assert_eq!(caps(), ascii);
            with_caps(Caps::default(), || assert_eq!(caps(), Caps::default()));
            assert_eq!(caps(), ascii);
        });
        assert_eq!(caps(), before);
    }

    struct Osc(Option<Background>);
    impl Probe for Osc {
        fn osc11_background(&self) -> Option<Background> {
            self.0
        }
    }

    #[test]
    fn background_order_is_cx_theme_osc11_colorfgbg_dark() {
        let light = Osc(Some(Background::Light));
        let dark = Osc(Some(Background::Dark));
        assert_eq!(
            detect_with(&[("CX_THEME", "dark")], &light).background,
            Background::Dark
        );
        assert_eq!(detect_with(&[], &light).background, Background::Light);
        // OSC 11 outranks COLORFGBG (often stale after a theme switch).
        assert_eq!(
            detect_with(&[("COLORFGBG", "0;15")], &dark).background,
            Background::Dark
        );
        assert_eq!(
            detect_with(&[("COLORFGBG", "0;15")], &Osc(None)).background,
            Background::Light
        );
        // Never queried where it cannot be answered.
        struct Panics;
        impl Probe for Panics {
            fn osc11_background(&self) -> Option<Background> {
                panic!("osc 11 queried")
            }
        }
        detect_with(&[("TERM", "dumb")], &Panics);
        detect_with(
            &[("TERM_PROGRAM", "Blink"), ("TERM", "xterm-256color")],
            &Panics,
        );
        detect_with(&[("CX_THEME", "light")], &Panics);
    }

    #[test]
    fn osc11_replies_parse() {
        use Background::*;
        let cases: &[(&[u8], Option<Background>)] = &[
            (b"\x1b]11;rgb:0000/0000/0000\x07", Some(Dark)),
            (b"\x1b]11;rgb:1e1e/1e1e/2e2e\x1b\\\x1b[?62;22c", Some(Dark)),
            (b"\x1b]11;rgb:ffff/ffff/ffff\x1b\\", Some(Light)),
            (b"\x1b]11;rgb:fd/f6/e3\x07", Some(Light)),
            (b"\x1b]11;rgba:f0f0/f0f0/f0f0/ffff\x07", Some(Light)),
            (b"\x1b]11;rgb:28/2c/34\x07", Some(Dark)),
            (b"\x1b]11;rgb:7777/7777/7777\x07", Some(Dark)),
            (b"\x1b[?62;22c", None),
            (b"\x1b]11;rgb:zz/00/00\x07", None),
            (b"\x1b]11;rgb:12345/0/0\x07", None),
            (b"", None),
        ];
        for (reply, want) in cases {
            assert_eq!(osc11_reply_background(reply), *want, "{reply:?}");
        }
        let da1 = |r: &[u8]| crate::terminal_preview::da1_replies(r).len();
        assert_eq!(da1(b"\x1b]11;rgb:0/0/0\x07\x1b[?62;22c"), 1);
        assert_eq!(da1(b"\x1b[?1;2c\x1b[?62;4c"), 2);
        assert_eq!(da1(b"\x1b[?62;2"), 0);
        assert_eq!(da1(b"\x1b]11;rgb:0/0/0\x07"), 0);
    }

    #[test]
    fn late_probe_replies_are_dropped_only_when_complete() {
        use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
        let key = |c: char, m: KeyModifiers| Event::Key(KeyEvent::new(KeyCode::Char(c), m));
        let alt = |c: char| key(c, KeyModifiers::ALT);
        let esc = Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        // The events crossterm makes of a byte string (ESC x: Alt+x, a lone ESC: Esc).
        let events = |b: &str| {
            let mut out = Vec::new();
            let mut it = b.chars().peekable();
            while let Some(c) = it.next() {
                out.push(match c {
                    '\x1b' => match it.next() {
                        Some(n) => alt(n),
                        None => esc.clone(),
                    },
                    '\x07' => key('g', KeyModifiers::CONTROL),
                    c => key(c, KeyModifiers::NONE),
                });
            }
            out
        };
        let feed = |f: &mut ReplyFilter, evs: Vec<Event>, now: Instant| -> Vec<Event> {
            evs.into_iter().flat_map(|e| f.push(e, now)).collect()
        };
        let now = Instant::now();
        let mut f = ReplyFilter::default();
        // Unarmed: everything passes, replies included.
        *STRAY_UNTIL.lock().unwrap() = None;
        assert_eq!(feed(&mut f, events("\x1b_Gi=31;OK\x1b\\"), now).len(), 10);
        *STRAY_UNTIL.lock().unwrap() = Some(now + STRAY_WINDOW);
        // Complete replies vanish: kitty graphics, XTVERSION, OSC 11 (BEL and ST),
        // and a 16 t / DA1 split after its ESC.
        for reply in [
            "\x1b_Gi=31;OK\x1b\\",
            "\x1bP>|kitty(0.39.1)\x1b\\",
            "\x1b]11;rgb:1e1e/1e1e/2e2e\x07",
            "\x1b]11;rgb:1e1e/1e1e/2e2e\x1b\\",
        ] {
            assert!(feed(&mut f, events(reply), now).is_empty(), "{reply:?}");
        }
        let mut split = vec![esc.clone()];
        split.extend(events("[6;20;10t"));
        split.push(esc.clone());
        split.extend(events("[?62;22c"));
        assert!(feed(&mut f, split, now).is_empty());
        // Real keys pass at once, even right after a reply.
        for k in ["q", "H", "j", ";", "`"] {
            assert_eq!(feed(&mut f, events(k), now), events(k), "{k}");
        }
        // A reply's start then something else: all of it is delivered, in order.
        let mixed = events("\x1b[q");
        assert_eq!(feed(&mut f, mixed.clone(), now), mixed);
        // An unfinished reply is delivered once it stops arriving.
        let part = events("\x1b_Gi=31");
        assert!(feed(&mut f, part.clone(), now).is_empty());
        assert_eq!(f.expire(now + REPLY_HOLD), part);
        // Esc is held only while it may start a reply, then delivered.
        assert!(f.push(esc.clone(), now).is_empty());
        assert_eq!(f.deadline(), Some(now + REPLY_HOLD));
        assert!(f.expire(now).is_empty());
        assert_eq!(f.expire(now + REPLY_HOLD), vec![esc.clone()]);
        assert!(f.push(esc.clone(), now).is_empty());
        let q = key('q', KeyModifiers::NONE);
        assert_eq!(f.push(q.clone(), now), vec![esc.clone(), q]);
        // A non-key event releases what is held, before itself.
        assert!(f.push(alt('_'), now).is_empty());
        let resize = Event::Resize(80, 24);
        assert_eq!(f.push(resize.clone(), now), vec![alt('_'), resize]);
        // Expired window: inert.
        *STRAY_UNTIL.lock().unwrap() = Some(now);
        assert_eq!(f.push(esc.clone(), now), vec![esc]);
        *STRAY_UNTIL.lock().unwrap() = None;
    }

    fn assert_gone(pid: &str) {
        let deadline = Instant::now() + Duration::from_millis(250);
        loop {
            let stat = std::fs::read_to_string(format!("/proc/{}/stat", pid.trim()));
            let gone = stat.as_ref().map_or(true, |s| {
                s.rsplit_once(')')
                    .is_some_and(|(_, rest)| rest.trim_start().starts_with('Z'))
            });
            if gone {
                return;
            }
            assert!(Instant::now() < deadline, "probe descendant {pid} survived");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn bounded_output_is_time_bounded_and_leaves_no_descendants() {
        let limit = Duration::from_millis(100);
        // A wrapper that answers and exits but leaves a child holding stdout.
        let dir = tempfile::tempdir().unwrap();
        let pid = dir.path().join("child.pid");
        let started = Instant::now();
        let out = bounded_output(
            Command::new("/bin/sh")
                .args(["-c", "sleep 3 & echo $! > \"$1\"; echo colors#256", "probe"])
                .arg(&pid),
            limit,
        );
        let elapsed = started.elapsed();
        assert_eq!(out.as_deref(), Some("colors#256\n"));
        assert!(elapsed < Duration::from_millis(400), "{elapsed:?}");
        assert_gone(&std::fs::read_to_string(&pid).unwrap());

        // A wrapper that never exits: None at the limit, the whole group killed.
        let pid = dir.path().join("stalled.pid");
        let started = Instant::now();
        let out = bounded_output(
            Command::new("/bin/sh")
                .args([
                    "-c",
                    "sleep 5 & echo $! > \"$1\"; echo partial; wait",
                    "probe",
                ])
                .arg(&pid),
            limit,
        );
        let elapsed = started.elapsed();
        assert_eq!(out, None);
        assert!(elapsed < Duration::from_millis(400), "{elapsed:?}");
        assert_gone(&std::fs::read_to_string(&pid).unwrap());

        // Failure exits yield nothing, and their leftovers are killed too.
        let pid = dir.path().join("failed.pid");
        let out = bounded_output(
            Command::new("/bin/sh")
                .args(["-c", "sleep 5 & echo $! > \"$1\"; echo x; exit 1", "probe"])
                .arg(&pid),
            limit,
        );
        assert_eq!(out, None);
        assert_gone(&std::fs::read_to_string(&pid).unwrap());
        // Missing binaries fail fast.
        assert_eq!(
            bounded_output(&mut Command::new("/nonexistent/cx-probe"), limit),
            None
        );
    }
}
