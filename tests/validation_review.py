#!/usr/bin/env python3
"""Independent bounded review. No providers, live sessions, trust or network changes.

Usage: python3 tests/validation_review.py /absolute/release/cx
Only assertion summaries and measurements are printed, never terminal/provider content.
"""
import fcntl
import hashlib
import json
import os
import pathlib
import pty
import select
import signal
import statistics
import struct
import subprocess
import sys
import tempfile
import termios
import time

BINARY = str(pathlib.Path(sys.argv[1]).resolve())


def summary(values):
    ordered = sorted(values)
    return {"n": len(values), "p50": statistics.median(values),
            "p95": ordered[min(len(values)-1, int(len(values)*.95))]}


class Terminal:
    def __init__(self, env, size=(24, 80)):
        self.master, self.slave = pty.openpty()
        self.before = termios.tcgetattr(self.slave)
        fcntl.ioctl(self.slave, termios.TIOCSWINSZ, struct.pack("HHHH", *size, 0, 0))
        def control():
            os.setsid()
            fcntl.ioctl(self.slave, termios.TIOCSCTTY, 0)
        self.started = time.monotonic()
        self.proc = subprocess.Popen([BINARY], stdin=self.slave, stdout=self.slave,
                                     stderr=self.slave, env=env, cwd=env["HOME"], preexec_fn=control)

    def read(self, duration=.15, first=False, marker=None):
        output = bytearray()
        deadline = time.monotonic() + duration
        while time.monotonic() < deadline:
            if select.select([self.master], [], [], min(.01, deadline-time.monotonic()))[0]:
                try:
                    output.extend(os.read(self.master, 65536))
                except OSError:
                    break
                if first and output and (marker is None or marker in output):
                    break
        return bytes(output)

    def close(self, sig=None):
        if sig:
            os.kill(self.proc.pid, sig)
        else:
            os.write(self.master, b"\x03")
        try:
            self.proc.wait(timeout=3)
        except subprocess.TimeoutExpired:
            self.proc.kill()
            self.proc.wait()
        restored = termios.tcgetattr(self.slave) == self.before
        os.close(self.master)
        os.close(self.slave)
        return {"exit": self.proc.returncode, "termios_restored": restored}


with tempfile.TemporaryDirectory(prefix="cx-independent-review-") as temporary:
    root = pathlib.Path(temporary)
    bindir = root / "bin"
    bindir.mkdir()
    tmux = bindir / "tmux"
    tmux.write_text("#!/bin/sh\nsleep .1\nprintf 'no server running\\n' >&2\nexit 1\n")
    tmux.chmod(0o700)
    state = root / "state" / "cx"
    state.mkdir(parents=True)
    sessions = [{"id": "cx-" + hashlib.sha256(str(i).encode()).hexdigest(),
                 "name": "fixture-" + str(i), "directory": "/fixture/project",
                 "provider": "shell", "host": "fixture-execution", "account": "fixture-user",
                 "pid": 0, "started": "unknown", "boot_id": "fixture", "external": False,
                 "socket": None} for i in range(100)]
    env = dict(os.environ, HOME=temporary, XDG_STATE_HOME=str(root / "state"),
               USER="fixture-user", TERM="xterm-256color", PATH=str(bindir)+":/usr/bin:/bin")
    env.pop("TMUX", None)
    env.pop("TMUX_PANE", None)
    report = {"binary": BINARY, "sha256": hashlib.sha256(pathlib.Path(BINARY).read_bytes()).hexdigest(),
              "population": "100 cached synthetic local sessions at startup; synthetic refresh clears them; remote probes absent",
              "hardware": subprocess.run(["uname", "-m"], capture_output=True, text=True).stdout.strip()}
    first = []
    for _ in range(20):
        (state / "ui-sessions.json").write_text(json.dumps({"local": sessions}))
        terminal = Terminal(env)
        output = terminal.read(2, first=True, marker=b"Sessions")
        first.append((time.monotonic()-terminal.started)*1000)
        assert b"Sessions" in output, "first screen absent"
        assert terminal.close()["termios_restored"], "Ctrl+C left raw terminal"
    report["cached_first_screen_ms"] = summary(first)
    sizes = []
    for rows, cols in [(24, 80), (40, 120), (12, 40), (8, 30)]:
        terminal = Terminal(env, (rows, cols))
        output = terminal.read(.25)
        sizes.append({"rows": rows, "cols": cols, "rendered": bool(output),
                      "narrow_fallback": b"enlarge" in output,
                      "normal_restore": terminal.close()["termios_restored"]})
    report["sizes"] = sizes
    terminal = Terminal(env)
    terminal.read(.3)
    interaction = []
    for _ in range(40):
        started = time.monotonic()
        os.write(terminal.master, b"?")
        output = terminal.read(.5, first=True)
        interaction.append((time.monotonic()-started)*1000)
        os.write(terminal.master, b"\x1b")
        terminal.read(.04)
    report["help_interaction_ms"] = summary(interaction)
    terminal.read(.2)
    before_stat = pathlib.Path(f"/proc/{terminal.proc.pid}/stat").read_text().rsplit(")", 1)[1].split()
    idle = terminal.read(2)
    after_stat = pathlib.Path(f"/proc/{terminal.proc.pid}/stat").read_text().rsplit(")", 1)[1].split()
    status = pathlib.Path(f"/proc/{terminal.proc.pid}/status").read_text().splitlines()
    report["idle"] = {"seconds": 2, "output_bytes": len(idle),
                      "cpu_ticks": sum(int(after_stat[i])-int(before_stat[i]) for i in (11, 12)),
                      "rss_kib": int(next(line for line in status if line.startswith("VmRSS:")).split()[1])}
    report["normal_exit"] = terminal.close()
    helper_process = subprocess.Popen([BINARY, "helper"], stdin=subprocess.PIPE, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, env=env)
    time.sleep(.1)
    helper_status = pathlib.Path(f"/proc/{helper_process.pid}/status").read_text().splitlines()
    report["helper_waiting_rss_kib"] = int(next(line for line in helper_status if line.startswith("VmRSS:")).split()[1])
    helper_process.stdin.close()
    helper_process.wait(timeout=3)
    terminal = Terminal(env)
    terminal.read(.2)
    report["sigterm_exit"] = terminal.close(signal.SIGTERM)
    terminal = Terminal(env, (40, 120))
    terminal.read(.2)
    usability = {}
    os.write(terminal.master, b"\x10add\r")
    output = terminal.read(.15)
    usability["add_modal_discoverable"] = b"Add by SSH address" in output
    os.write(terminal.master, b"\x1b")
    terminal.read(.1)
    os.write(terminal.master, b"\x10Files\r")
    output = terminal.read(.25)
    usability["browser_host_visible"] = b"fixture-user@" in output
    os.write(terminal.master, b"\x10Copy")
    output = terminal.read(.15)
    usability["copy_action_discoverable"] = b"Copy selected" in output
    os.write(terminal.master, b"\x1b")
    terminal.read(.1)
    os.write(terminal.master, b"\x10Network\r")
    output = terminal.read(.3)
    usability["network_view_discoverable"] = b"Network" in output
    os.write(terminal.master, b"\x10shar")
    output = terminal.read(.15)
    usability["sharing_action_available"] = b"Sharing" in output or b"share" in output.lower()
    os.write(terminal.master, b"\x03")
    terminal.read(.2)
    usability["ctrl_c_ignored_in_palette"] = terminal.proc.poll() is None
    os.write(terminal.master, b"\x1b")
    terminal.read(.1)
    usability["exit"] = terminal.close()
    report["first_run_tasks"] = usability
    cases = [("noise", b"welcome\n", {"version": 1, "id": "fixture", "op": {"op": "info"}}, True),
             ("version", b"", {"version": 2, "id": "fixture", "op": {"op": "info"}}, True)]
    framing = []
    for name, prefix, request, expected in cases:
        payload = json.dumps(request).encode()
        proc = subprocess.run([BINARY, "helper"], input=prefix+b"CX1 "+str(len(payload)).encode()+b"\n"+payload,
                              env=env, capture_output=True, timeout=3)
        frame, content = proc.stdout.split(b"\n", 1)
        response = json.loads(content)
        framing.append({"case": name, "exit": proc.returncode, "framed": frame.startswith(b"CX1 "),
                        "id_matches": response["id"] == "fixture", "has_error": response["error"] is not None})
    for name, frame in [("oversize", b"CX1 1048577\n"), ("negative", b"CX1 -1\n"),
                        ("noise_limit", b"a"*16385+b"\n"), ("truncated", b"CX1 10\n{}")]:
        proc = subprocess.run([BINARY, "helper"], input=frame, env=env, capture_output=True, timeout=3)
        framing.append({"case": name, "rejected": proc.returncode != 0})
    report["framing"] = framing
    def helper(op):
        payload = json.dumps({"version": 1, "id": "fixture", "op": op}).encode()
        proc = subprocess.run([BINARY, "helper"], input=b"CX1 "+str(len(payload)).encode()+b"\n"+payload,
                              env=env, cwd=temporary, capture_output=True, timeout=3)
        return json.loads(proc.stdout.split(b"\n", 1)[1])
    hostile = root / "hostile"
    hostile.mkdir()
    (hostile / "escape\x1b[31m\n.txt").write_text("line\x1b]52;c;SYNTHETIC\x07\n")
    listing = helper({"op": "list", "args": {"path": str(hostile)}})["result"]
    preview = helper({"op": "preview", "args": {"path": str(next(hostile.iterdir()))}})["result"]
    (hostile / "link").symlink_to(next(hostile.iterdir()))
    symlink = helper({"op": "preview", "args": {"path": str(hostile / "link")}})
    large = root / "large"
    large.mkdir()
    for i in range(1001):
        (large / str(i)).touch()
    listing_large = helper({"op": "list", "args": {"path": str(large)}})["result"]
    report["filesystem_security"] = {
        "filename_esc_escaped": "\x1b" not in listing["entries"][0]["name"],
        "preview_esc_escaped": "\x1b" not in preview["text"],
        "preview_symlink_rejected": symlink["error"] is not None,
        "large_directory_bounded": len(listing_large["entries"]),
        "large_directory_truncated": listing_large["truncated"]}
    print(json.dumps(report, indent=2))
