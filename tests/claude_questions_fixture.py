#!/usr/bin/env python3
"""Real Claude UI against a synthetic loopback API; run only in an owned test window.

No real account/configuration is used. Captures contain only fixture questions.
This does not claim to reproduce compositor transparency; root may attach a foot
window to the printed private socket while --hold-seconds keeps it available.
"""
import argparse
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import shlex
import signal
import shutil
import subprocess
import tempfile
import threading
import time


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--cx", type=Path, help="use an actual private CX managed session")
    parser.add_argument("--claude", default=shutil.which("claude"))
    parser.add_argument("--theme", choices=["dark", "light", "dark-ansi", "light-ansi",
                        "dark-daltonized", "light-daltonized"], default="dark")
    parser.add_argument("--artifacts", type=Path, required=True)
    parser.add_argument("--tmux-config", type=Path, default=Path("/dev/null"))
    parser.add_argument("--hold-seconds", type=int, default=0)
    args = parser.parse_args()
    if not args.claude or not 0 <= args.hold_seconds <= 120:
        parser.error("installed Claude required; hold must be 0..120 seconds")
    args.artifacts.mkdir(parents=True, exist_ok=True)
    calls = []

    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *_):
            pass

        def do_POST(self):
            size = int(self.headers.get("Content-Length", "0"))
            if size > 4 * 1024 * 1024:
                self.send_error(413)
                return
            body = json.loads(self.rfile.read(size))
            if "count_tokens" in self.path:
                response = json.dumps({"input_tokens": 100}).encode()
                self.send_response(200)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(response)))
                self.end_headers()
                self.wfile.write(response)
                return
            if not self.path.startswith("/v1/messages"):
                self.send_error(404)
                return
            # Inspect only tool names/counts, never print request bodies or headers.
            calls.append(any(t.get("name") == "AskUserQuestion" for t in body.get("tools", [])))
            if not calls[-1]:
                content = [{"type": "text", "text": "Synthetic fixture auxiliary response."}]
                reason = "end_turn"
            else:
                content = [{"type": "tool_use", "id": "toolu_cx_question_fixture",
                            "name": "AskUserQuestion", "input": {"questions": [
                    {"question": "CX fixture: choose a project color.", "header": "Color",
                     "options": [{"label": "Ocean", "description": "Synthetic first option."},
                                 {"label": "Forest", "description": "Synthetic second option."}],
                     "multiSelect": False},
                    {"question": "CX fixture: choose a second project mode.", "header": "Mode",
                     "options": [{"label": "Inspect", "description": "Synthetic inspection option."},
                                 {"label": "Compare", "description": "Synthetic comparison option."}],
                     "multiSelect": False}]}}]
                reason = "tool_use"
            model = body.get("model", "claude-sonnet-4-6")
            if body.get("stream"):
                events = [("message_start", {"type": "message_start", "message": {
                    "id": "msg_cx_fixture", "type": "message", "role": "assistant",
                    "model": model, "content": [], "stop_reason": None,
                    "stop_sequence": None, "usage": {"input_tokens": 100, "output_tokens": 1}}})]
                for index, block in enumerate(content):
                    empty = dict(block)
                    empty["input" if block["type"] == "tool_use" else "text"] = {} if block["type"] == "tool_use" else ""
                    events.append(("content_block_start", {"type": "content_block_start", "index": index, "content_block": empty}))
                    delta = ({"type": "input_json_delta", "partial_json": json.dumps(block["input"])}
                             if block["type"] == "tool_use" else {"type": "text_delta", "text": block["text"]})
                    events.extend([("content_block_delta", {"type": "content_block_delta", "index": index, "delta": delta}),
                                   ("content_block_stop", {"type": "content_block_stop", "index": index})])
                events.extend([("message_delta", {"type": "message_delta", "delta": {"stop_reason": reason, "stop_sequence": None}, "usage": {"output_tokens": 50}}),
                               ("message_stop", {"type": "message_stop"})])
                response = "".join(f"event: {name}\ndata: {json.dumps(data)}\n\n" for name, data in events).encode()
                kind = "text/event-stream"
            else:
                response = json.dumps({"id": "msg_cx_fixture", "type": "message", "role": "assistant",
                    "model": model, "content": content, "stop_reason": reason, "stop_sequence": None,
                    "usage": {"input_tokens": 100, "output_tokens": 50}}).encode()
                kind = "application/json"
            self.send_response(200)
            self.send_header("Content-Type", kind)
            self.send_header("Content-Length", str(len(response)))
            self.end_headers()
            self.wfile.write(response)

    with tempfile.TemporaryDirectory(prefix="cx-claude-questions-") as tmp:
        root = Path(tmp)
        config = root / "config"
        config.mkdir()
        dummy = "sk-ant-cx-fixture-dummy"
        (config / ".claude.json").write_text(json.dumps({"theme": args.theme,
            "hasCompletedOnboarding": True, "numStartups": 1,
            "customApiKeyResponses": {"approved": [dummy[-20:]], "rejected": []},
            "projects": {tmp: {"hasTrustDialogAccepted": True}}}))
        server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        threading.Thread(target=server.serve_forever, daemon=True).start()
        env = {"PATH": os.environ["PATH"], "HOME": tmp, "XDG_CONFIG_HOME": tmp + "/xdg",
            "XDG_STATE_HOME": tmp + "/state", "XDG_CACHE_HOME": tmp + "/cache",
            "CLAUDE_CONFIG_DIR": str(config), "ANTHROPIC_API_KEY": dummy,
            "ANTHROPIC_BASE_URL": f"http://127.0.0.1:{server.server_port}",
            "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC": "1", "DISABLE_AUTOUPDATER": "1",
            "DISABLE_TELEMETRY": "1", "DISABLE_ERROR_REPORTING": "1",
            "TERM": "tmux-256color", "COLORTERM": "truecolor", "LANG": "C.UTF-8"}
        session = "fixture"
        socket = root / "fixture.sock"
        if args.cx:
            args.cx = args.cx.resolve()
            socket = root / "state/cx/managed.sock"
            socket.parent.mkdir(parents=True, mode=0o700)
            subprocess.run(["tmux", "-S", str(socket), "-f", str(args.tmux_config.resolve()),
                "new-session", "-d", "-s", "bootstrap", "sleep 60"], env=env, check=True)
            created = json.loads(subprocess.check_output([str(args.cx), "new", "--directory", tmp,
                "--key", "claude-question-fixture"], env=env, text=True, timeout=10))
            session = created["id"]
            subprocess.run(["tmux", "-S", str(socket), "kill-session", "-t", "bootstrap"], env=env, check=True)
        def tmux(*command, check=True):
            return subprocess.run(["tmux", "-S", str(socket), *command], env=env,
                capture_output=True, text=True, timeout=5, check=check).stdout
        launch = [str(Path(args.claude).resolve()), "--setting-sources", "", "--strict-mcp-config",
                  "--tools", "AskUserQuestion", "--model", "claude-sonnet-4-6",
                  "Use AskUserQuestion to ask the two synthetic fixture questions."]
        fish = shutil.which("fish")
        command = "exec " + shlex.join(launch)
        if fish:
            command = shlex.join([fish, "--no-config", "-c", command])
        pane_pid = None
        pane_start = None
        try:
            if args.cx:
                tmux("send-keys", "-t", session, "-l", "exec " + command)
                tmux("send-keys", "-t", session, "Enter")
            else:
                tmux("-f", str(args.tmux_config.resolve()), "new-session", "-d", "-s", "fixture", "-x", "110", "-y", "38", "-c", tmp, command)
            deadline = time.monotonic() + 35
            pane = ""
            while time.monotonic() < deadline:
                pane = tmux("capture-pane", "-e", "-p", "-t", session)
                if "CX fixture: choose a project color." in pane:
                    break
                time.sleep(0.1)
            pane_pid = int(tmux("display-message", "-p", "-t", session, "#{pane_pid}").strip())
            pane_start = Path(f"/proc/{pane_pid}/stat").read_text().rsplit(")", 1)[1].split()[19]
            (args.artifacts / "question-first.ansi").write_text(pane)
            if "CX fixture: choose a project color." not in pane:
                raise RuntimeError(f"Question UI not reached; private synthetic capture saved; API calls={len(calls)}")
            tmux("send-keys", "-t", session, "Enter")
            time.sleep(0.4)
            (args.artifacts / "question-next.ansi").write_text(tmux("capture-pane", "-e", "-p", "-t", session))
            print(json.dumps({"result": "question UI reached", "theme": args.theme,
                              "api_calls": len(calls), "private_socket": str(socket), "session": session,
                              "artifacts": str(args.artifacts)}), flush=True)
            if args.hold_seconds:
                time.sleep(args.hold_seconds)
        finally:
            tmux("kill-server", check=False)
            # Claude may finish asynchronous config writes after tmux exits.
            # Revalidate the exact fixture-owned PID before stopping it.
            for sig in [signal.SIGTERM, signal.SIGKILL]:
                try:
                    stat = Path(f"/proc/{pane_pid}/stat").read_text().rsplit(")", 1)[1].split()
                    if stat[19] != pane_start or stat[0] == "Z":
                        break
                    os.kill(pane_pid, sig)
                    deadline = time.monotonic()+1
                    while time.monotonic()<deadline and Path(f"/proc/{pane_pid}").exists():
                        time.sleep(.02)
                except (FileNotFoundError, ProcessLookupError):
                    break
            server.shutdown()
            server.server_close()


if __name__ == "__main__":
    main()
