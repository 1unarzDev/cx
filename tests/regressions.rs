//! Black-box regression guards for previously fixed bugs (see tests/REGRESSIONS.md).
//!
//! Every test drives the real `cx helper` binary over its framed stdin
//! protocol with a private HOME/XDG_STATE_HOME and, where tmux is involved, a
//! recording fake `~/.local/bin/tmux`. Nothing here talks to a real tmux
//! server, the network, or the user's state, and nothing depends on UI text,
//! so the guards survive a UI redesign unchanged.
use serde_json::{json, Value};
use std::{
    fs,
    io::{Read, Write},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

const CX: &str = env!("CARGO_BIN_EXE_cx");

struct Home {
    dir: tempfile::TempDir,
}

impl Home {
    fn new() -> Self {
        let dir = tempfile::Builder::new()
            .prefix("cx-regression-")
            .tempdir()
            .unwrap();
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
        fs::create_dir_all(dir.path().join("state")).unwrap();
        Self { dir }
    }
    fn path(&self) -> &Path {
        self.dir.path()
    }
    fn state(&self) -> PathBuf {
        self.path().join("state")
    }
    fn log(&self) -> PathBuf {
        self.path().join("tmux.log")
    }
    /// A recording tmux that answers `list-sessions` according to `mode` and
    /// succeeds silently otherwise. It lives where cx looks for a user tmux
    /// (`~/.local/bin/tmux`), which is deliberately absent from PATH.
    fn fake_tmux(&self, mode: &str) {
        let bin = self.path().join(".local/bin");
        fs::create_dir_all(&bin).unwrap();
        let script = format!(
            r#"#!/bin/sh
printf 'ARGS %s|TMUX=%s|PANE=%s\n' "$*" "${{TMUX-<unset>}}" "${{TMUX_PANE-<unset>}}" >> '{log}'
case "$*" in
*list-sessions*)
  case '{mode}' in
  absent) echo "no server running on /tmp/tmux-fixture/default" >&2; exit 1;;
  refused) echo "error connecting to /tmp/tmux-fixture/default (Connection refused)" >&2; exit 1;;
  missing) echo "error connecting to /tmp/tmux-fixture/default (No such file or directory)" >&2; exit 1;;
  broken) echo "tmux: error while loading shared libraries: libevent_core-2.1.so.7: cannot open shared object file: No such file or directory" >&2; exit 127;;
  wrapper) echo "/usr/local/bin/tmux-wrapper: line 3: exec: tmux.real: not found (No such file or directory)" >&2; exit 126;;
  esac;;
esac
exit 0
"#,
            log = self.log().display(),
        );
        let tmux = bin.join("tmux");
        fs::write(&tmux, script).unwrap();
        fs::set_permissions(&tmux, fs::Permissions::from_mode(0o700)).unwrap();
    }
    fn calls(&self) -> Vec<String> {
        fs::read_to_string(self.log())
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }
    fn command(&self) -> Command {
        let mut c = Command::new(CX);
        c.env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", self.path())
            .env("XDG_STATE_HOME", self.state())
            .env("LANG", "C.UTF-8");
        c
    }
}

/// One framed helper request; the helper is killed if it does not answer
/// within `limit`, so a regression to a blocking open fails instead of hanging.
fn helper(home: &Home, op: Value, extra_env: &[(&str, &str)], limit: Duration) -> Value {
    let mut c = home.command();
    for (k, v) in extra_env {
        c.env(k, v);
    }
    let mut child = c
        .arg("helper")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let body = serde_json::to_vec(&json!({"version": 1, "id": "regression", "op": op})).unwrap();
    {
        let mut stdin = child.stdin.take().unwrap();
        write!(stdin, "CX1 {}\n", body.len()).unwrap();
        stdin.write_all(&body).unwrap();
    }
    let started = Instant::now();
    loop {
        if child.try_wait().unwrap().is_some() {
            break;
        }
        if started.elapsed() > limit {
            let _ = child.kill();
            let _ = child.wait();
            panic!("helper did not answer within {limit:?}: request blocked");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let mut out = Vec::new();
    child.stdout.take().unwrap().read_to_end(&mut out).unwrap();
    let newline = out
        .iter()
        .position(|b| *b == b'\n')
        .expect("framed reply header");
    let header = std::str::from_utf8(&out[..newline]).unwrap();
    let length: usize = header.strip_prefix("CX1 ").unwrap().parse().unwrap();
    assert!(length <= 1024 * 1024, "reply exceeds frame limit");
    let reply: Value = serde_json::from_slice(&out[newline + 1..newline + 1 + length]).unwrap();
    assert_eq!(reply["version"], 1);
    assert_eq!(reply["id"], "regression");
    reply
}

fn sessions(home: &Home, extra_env: &[(&str, &str)]) -> Value {
    helper(
        home,
        json!({"op": "sessions"}),
        extra_env,
        Duration::from_secs(20),
    )
}

/// 06f0d91 / 2c0f977 (v0.1.28): started inside a managed pane, cx inherited
/// TMUX/TMUX_PANE, so "external" discovery and attachment were redirected to
/// the managed server (shared numeric `$0` ids, duplicate rows, broken remote
/// attach). Also guards 0e1cfc3 (v0.1.14): a user tmux in ~/.local/bin is used
/// even when the noninteractive PATH omits it.
#[test]
fn inherited_tmux_environment_never_redirects_default_server_discovery() {
    let home = Home::new();
    home.fake_tmux("absent");
    let reply = sessions(
        &home,
        &[
            ("TMUX", "/tmp/tmux-fixture/managed.sock,4242,0"),
            ("TMUX_PANE", "%7"),
        ],
    );
    assert_eq!(reply["error"], Value::Null, "{reply}");
    assert_eq!(reply["result"], json!([]));
    let calls = home.calls();
    assert!(
        !calls.is_empty(),
        "cx did not use the user tmux in ~/.local/bin when PATH omits it"
    );
    let socket = format!("-S {}", home.state().join("cx/managed.sock").display());
    let listings: Vec<&String> = calls
        .iter()
        .filter(|c| c.contains("list-sessions"))
        .collect();
    let managed: Vec<&&String> = listings.iter().filter(|c| c.contains(&socket)).collect();
    let default: Vec<&&String> = listings.iter().filter(|c| !c.contains(" -S ")).collect();
    assert_eq!(
        managed.len(),
        1,
        "managed server not listed exactly once: {listings:?}"
    );
    assert_eq!(
        default.len(),
        1,
        "default server not listed exactly once: {listings:?}"
    );
    for call in &default {
        assert!(
            call.contains("|TMUX=<unset>|PANE=<unset>"),
            "default-server discovery inherited the managed client's TMUX: {call}"
        );
    }
    for call in &calls {
        assert!(
            call.starts_with("ARGS -u "),
            "tmux must run with -u: {call}"
        );
    }
}

/// 343dc07 (v0.1.8): any stderr containing "No such file" (for example a
/// missing shared library or a broken wrapper) was read as "no server
/// running", so a failed tmux runtime reported sessions as already stopped.
#[test]
fn absent_tmux_server_is_empty_but_broken_tmux_runtime_is_an_error() {
    for mode in ["absent", "refused", "missing"] {
        let home = Home::new();
        home.fake_tmux(mode);
        let reply = sessions(&home, &[]);
        assert_eq!(reply["error"], Value::Null, "{mode}: {reply}");
        assert_eq!(reply["result"], json!([]), "{mode}");
    }
    for mode in ["broken", "wrapper"] {
        let home = Home::new();
        home.fake_tmux(mode);
        let reply = sessions(&home, &[]);
        assert_eq!(
            reply["result"],
            Value::Null,
            "{mode}: a failed tmux runtime was reported as an empty session list"
        );
        let error = reply["error"].as_str().unwrap_or_default();
        assert!(
            error.contains("cannot list tmux sessions"),
            "{mode}: {error}"
        );
    }
}

/// 42315b2 (v0.1.x files): previews opened special files before checking
/// their type. Opening a FIFO blocks forever and opening a device can have
/// hardware side effects; previews must refuse both without opening them.
#[test]
fn previews_refuse_fifos_and_devices_without_opening_them() {
    let home = Home::new();
    let fifo = home.path().join("pipe");
    let c = std::ffi::CString::new(fifo.to_str().unwrap()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
    // inotify reports any open of the FIFO inode, proving whether cx opened it.
    let watch = unsafe { libc::inotify_init1(libc::IN_NONBLOCK | libc::IN_CLOEXEC) };
    assert!(watch >= 0);
    assert!(unsafe { libc::inotify_add_watch(watch, c.as_ptr(), libc::IN_OPEN) } >= 0);
    let fifo = fifo.to_str().unwrap().to_owned();
    for (label, op) in [
        (
            "fifo preview",
            json!({"op": "preview", "args": {"path": fifo}}),
        ),
        (
            "fifo page",
            json!({"op": "preview_page", "args": {"path": fifo, "page": 1}}),
        ),
        (
            "device preview",
            json!({"op": "preview", "args": {"path": "/dev/null"}}),
        ),
    ] {
        let reply = helper(&home, op, &[], Duration::from_secs(10));
        assert_eq!(reply["result"], Value::Null, "{label}: {reply}");
        assert!(reply["error"].is_string(), "{label}: {reply}");
    }
    let mut events = [0u8; 4096];
    let read = unsafe { libc::read(watch, events.as_mut_ptr().cast(), events.len()) };
    unsafe { libc::close(watch) };
    assert!(
        read < 0,
        "preview opened the FIFO before checking its file type ({read} event bytes)"
    );
    // A regular file beside them still previews, so the refusal is specific.
    fs::write(home.path().join("note.txt"), "plain text\n").unwrap();
    let path = home.path().join("note.txt").to_str().unwrap().to_owned();
    let reply = helper(
        &home,
        json!({"op": "preview", "args": {"path": path}}),
        &[],
        Duration::from_secs(10),
    );
    assert_eq!(reply["error"], Value::Null, "{reply}");
}

/// 88a40ba (v0.1.79): folding the /proc terminal scan into `sessions` made
/// `cx sessions` (and every fixture/script) list the user's real terminals.
/// `sessions` must stay managed-only; outside terminals arrive only through
/// `external_sessions`. The fixture proves its own pty shell is discoverable
/// there, so the managed-only assertion is meaningful.
#[test]
fn sessions_stay_managed_only_while_outside_terminals_use_their_own_operation() {
    use std::os::unix::{io::FromRawFd, process::CommandExt};
    let home = Home::new();
    home.fake_tmux("absent");
    let (mut master, mut slave) = (0, 0);
    assert_eq!(
        unsafe {
            libc::openpty(
                &mut master,
                &mut slave,
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null(),
            )
        },
        0
    );
    // Keep the pty ends out of the helper children: an inherited master would
    // make cx itself the terminal's holder, which discovery rightly excludes.
    for fd in [master, slave] {
        assert_eq!(
            unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) },
            0
        );
    }
    let stdio = |fd: i32| unsafe { Stdio::from_raw_fd(libc::dup(fd)) };
    let mut shell = Command::new("/bin/bash");
    shell
        .args(["--norc", "--noprofile", "-i"])
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", home.path())
        .env("TERM", "xterm-256color")
        .env("PS1", "fixture$ ")
        .current_dir(home.path())
        .stdin(stdio(slave))
        .stdout(stdio(slave))
        .stderr(stdio(slave));
    unsafe {
        shell.pre_exec(|| {
            if libc::setsid() < 0 || libc::ioctl(0, libc::TIOCSCTTY as _, 0) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut shell = shell.spawn().unwrap();
    unsafe { libc::close(slave) };
    let pid = shell.id() as u64;
    let has_shell = |reply: &Value| {
        reply["result"]
            .as_array()
            .is_some_and(|rows| rows.iter().any(|s| s["pid"].as_u64() == Some(pid)))
    };
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut external = Value::Null;
    while Instant::now() < deadline {
        external = helper(
            &home,
            json!({"op": "external_sessions"}),
            &[],
            Duration::from_secs(20),
        );
        if has_shell(&external) {
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    let managed = sessions(&home, &[]);
    let _ = shell.kill();
    let _ = shell.wait();
    unsafe { libc::close(master) };
    assert!(
        has_shell(&external),
        "fixture shell was never discovered as an outside terminal: error={}",
        external["error"]
    );
    assert_eq!(managed["error"], Value::Null, "{}", managed["error"]);
    assert_eq!(
        managed["result"],
        json!([]),
        "`sessions` listed outside-CX terminals"
    );
}
