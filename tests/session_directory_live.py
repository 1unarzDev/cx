#!/usr/bin/env python3
"""No-inference Codex -C regression on an isolated, disposable cx tmux server.

Usage: python3 tests/session_directory_live.py /absolute/path/to/cx
Requires an installed Codex CLI and tmux; never reads provider history/config.
"""
import json
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import sys
import tempfile
import time

binary = str(Path(sys.argv[1]).resolve())
codex = shutil.which("codex")
if not codex:
    raise SystemExit("BLOCKED: Codex CLI unavailable")
with tempfile.TemporaryDirectory(prefix="cx-session-directory-") as temp:
    root = Path(temp)
    launch = root / "launcher"
    workspace = root / "workspace ' ☃"
    home = root / "home"
    for directory in (launch, workspace, home):
        directory.mkdir()
    env = {**os.environ, "HOME": str(home), "XDG_STATE_HOME": str(root / "state"),
           "XDG_CONFIG_HOME": str(root / "config"), "CODEX_HOME": str(root / "codex")}
    sock = root / "state/cx/managed.sock"
    def run(*args):
        return subprocess.check_output(args, env=env, text=True, stderr=subprocess.PIPE)
    try:
        created = json.loads(run(binary, "new", "--provider", "shell", "--directory", str(launch),
                                 "--name", "directory-fixture", "--key", "directory-fixture"))
        session_id = created["id"]
        command = shlex.join([codex, "--no-daemon", "-C", str(workspace)])
        run("tmux", "-S", str(sock), "send-keys", "-t", session_id, "-l", command)
        run("tmux", "-S", str(sock), "send-keys", "-t", session_id, "Enter")
        deadline = time.monotonic() + 8
        while time.monotonic() < deadline:
            sessions = json.loads(run(binary, "sessions"))
            row = next(s for s in sessions if s["id"] == session_id)
            if row["provider"] == "codex":
                break
            time.sleep(.1)
        assert row["provider"] == "codex", "foreground Codex was not detected"
        pane_path = run("tmux", "-S", str(sock), "display-message", "-p", "-t", session_id,
                        "#{pane_current_path}").strip()
        assert pane_path == str(launch), ("fixture must expose launcher/workspace divergence", pane_path)
        assert row["directory"] == str(workspace), ("wrong project directory", row["directory"])
        print(json.dumps({"status": "PASS", "codex_version": run(codex, "--version").strip(),
                          "tmux_version": run("tmux", "-V").strip(), "pane_path": pane_path,
                          "reported_workspace": row["directory"], "provider": row["provider"],
                          "process_cwd": os.readlink(f'/proc/{row["process"]["pid"]}/cwd')}, ensure_ascii=False))
    finally:
        # Only the private socket created by this fixture is ever stopped.
        if sock.exists():
            subprocess.run(["tmux", "-S", str(sock), "kill-server"], env=env,
                           stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
