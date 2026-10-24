#!/usr/bin/env python3
"""Bounded public-CLI session regressions. Live tmux mode is root-run only.

No personal sockets, provider history, terminal capture, Docker daemon, SSH
network, or services are used. See session_adversarial.md for the matrix.
"""
import argparse
import json
import math
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import sys
import tempfile
import time
import unittest


class Failure(Exception):
    pass


def require(condition, message):
    if not condition:
        raise Failure(message)


def decode_frame(data):
    header, body = data.split(b"\n", 1)
    require(header.startswith(b"CX1 "), "bad response marker")
    require(len(body) <= 1048576, "response exceeds frame bound")
    require(int(header[4:]) == len(body), "response length mismatch")
    value = json.loads(body)
    require(value["version"] == 1 and value["id"] == "adversarial", "response identity mismatch")
    return value


def percentile(values, fraction):
    return sorted(values)[max(0, math.ceil(len(values) * fraction) - 1)]


def runtime(row):
    # Intentionally exclude label/provider/path: these may change in one terminal.
    return tuple(row.get(key) for key in ("pid", "started", "boot_id", "host", "account"))


class Fixture:
    def __init__(self, binary, live=False):
        self.temp = tempfile.TemporaryDirectory(prefix="cx-adv-")
        self.root = Path(self.temp.name)
        self.binary = str(Path(binary).resolve())
        self.live = live
        self.bin = self.root / "bin"
        self.bin.mkdir()
        self.state = self.root / "state/cx"
        self.state.mkdir(parents=True, mode=0o700)
        self.socket = self.state / "managed.sock"
        self.default = self.root / f"tmux-{os.getuid()}/default"
        self.default.parent.mkdir(mode=0o700)
        # Allowlist: do not inherit provider homes, credentials, SSH agents,
        # Docker context, user startup files, or an outer tmux socket.
        self.env = {
            "HOME": str(self.root), "XDG_STATE_HOME": str(self.root / "state"),
            "XDG_CONFIG_HOME": str(self.root / "config"),
            "XDG_CACHE_HOME": str(self.root / "cache"),
            "TMUX_TMPDIR": str(self.root), "TERM": "xterm-256color",
            "PATH": str(self.bin) + ":/usr/bin:/bin", "LANG": "C.UTF-8",
            "USER": "cx-fixture", "LOGNAME": "cx-fixture",
        }
        self.write_tool("systemd-run", "#!/bin/sh\nexit 97\n")
        self.write_tool("ssh", "#!/bin/sh\necho 'Could not resolve hostname cx-fixture.invalid' >&2\nexit 255\n")
        self.write_tool("docker", "#!/bin/sh\nexit 97\n")
        bash = shutil.which("bash")
        require(bool(bash), "bash required")
        self.write_tool("bash", "#!/bin/sh\nexec " + shlex.quote(bash) + ' --noprofile --norc "$@"\n')
        self.env["SHELL"] = str(self.bin / "bash")
        self.tmux_log = self.root / "tmux-count"
        if live:
            self.real_tmux = shutil.which("tmux")
            require(bool(self.real_tmux), "tmux required")
            # Preserve every argument and log only invocation count, never argv.
            self.write_tool("tmux", "#!/bin/sh\nprintf x >> " + shlex.quote(str(self.tmux_log))
                            + "\nexec " + shlex.quote(self.real_tmux) + ' "$@"\n')
        else:
            self.write_tool("tmux", "#!/bin/sh\nexit 97\n")

    def write_tool(self, name, source):
        path = self.bin / name
        path.write_text(source)
        path.chmod(0o700)

    def run(self, args, env=None, data=None, check=True, timeout=15, label="fixture-command"):
        try:
            result = subprocess.run(args, cwd=self.root, env=env or self.env,
                                    input=data, stdout=subprocess.PIPE,
                                    stderr=subprocess.DEVNULL, timeout=timeout)
        except subprocess.TimeoutExpired:
            raise Failure(label + " exceeded bounded deadline") from None
        require(len(result.stdout) <= 1048576, "fixture stdout exceeds bound")
        if check:
            require(result.returncode == 0, label + " exited " + str(result.returncode))
        return result

    def cli(self, *args, env=None):
        return json.loads(self.run([self.binary, *args], env=env, label="cx:" + args[0]).stdout)

    def request(self, op, args):
        data = json.dumps(dict(version=1, id="adversarial", op=dict(op=op, args=args))).encode()
        return decode_frame(self.run([self.binary, "helper"],
                            data=b"CX1 " + str(len(data)).encode() + b"\n" + data,
                            label="helper:" + op).stdout)

    def tmux(self, *args, default=False, check=True):
        require(self.live, "live tmux requires explicit mode")
        sock = self.default if default else self.socket
        # Every direct tmux operation addresses an exact fixture-owned socket.
        return self.run([self.real_tmux, "-f", "/dev/null", "-S", str(sock), *args],
                        check=check, label="tmux:" + args[0])

    def start_managed(self):
        self.tmux("new-session", "-d", "-s", "fixture-keeper", "/bin/sh")
        self.tmux("set-option", "-g", "exit-empty", "off")
        self.tmux("kill-session", "-t", "=fixture-keeper")

    def create(self, key, provider="shell", name=None):
        args = ["new", "--directory", str(self.root), "--key", key, "--provider", provider]
        if name is not None:
            args += ["--name", name]
        return self.cli(*args)

    def rows(self, env=None):
        rows = self.cli("sessions", env=env)
        require(isinstance(rows, list), "sessions response is not a list")
        return rows

    def send(self, session, command, default=False):
        self.tmux("send-keys", "-t", session, "-l", command, default=default)
        self.tmux("send-keys", "-t", session, "Enter", default=default)

    def until(self, predicate, label, seconds=6):
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
            if predicate():
                return
            time.sleep(.05)
        raise Failure(label)

    def close(self):
        errors = []
        if self.live:
            # Remove only our alias, then kill each independently owned server.
            if self.default.is_symlink():
                self.default.unlink()
            for sock in (self.default, self.socket):
                if sock.exists():
                    try:
                        self.run([self.real_tmux, "-S", str(sock), "kill-server"], check=False)
                    except Failure as error:
                        errors.append(str(error))
        self.temp.cleanup()
        require(not errors, "owned-server cleanup failed")


def aliases(f):
    f.start_managed()
    first = f.create("same-runtime", name="same project")
    second = f.create("different-runtime", name="same project")
    require(runtime(first) != runtime(second), "fixture runtimes unexpectedly identical")
    f.default.symlink_to(f.socket)
    rows = f.rows()
    require(len(rows) == 2, "same server via managed/default alias produced duplicate rows")
    require({runtime(r) for r in rows} == {runtime(first), runtime(second)},
            "dedup lost a legitimate same-name/same-folder session")
    for inherited in (str(f.socket), str(f.default)):
        env = dict(f.env, TMUX=inherited + ",123,0", TMUX_PANE="%0")
        require({runtime(r) for r in f.rows(env)} == {runtime(r) for r in rows},
                "inherited tmux environment changes runtime discovery")
        require(len(f.rows(env)) == 2, "nested environment duplicates runtime")


def foreground(f):
    f.start_managed()
    original = f.create("foreground", name="literal '\t\n\u202e label")
    other = f.create("peer", name="literal '\t\n\u202e label")
    marker = f.root / "ready"
    f.send(original["id"], "printf ready > " + shlex.quote(str(marker)))
    f.until(marker.exists, "shell prompt never became ready")
    for provider in ("codex", "claude"):
        shutil.copy2(shutil.which("bash"), f.bin / provider)
        f.send(original["id"], shlex.quote(str(f.bin / provider)) + " -c 'while :; do sleep 1; done'")

        def observed(expected):
            rows = f.rows()
            require(len(rows) == 2, "foreground transition duplicated or removed a terminal")
            current = next(r for r in rows if r["id"] == original["id"])
            require(runtime(current) == runtime(original), "foreground transition replaced terminal identity")
            require(current["name"] == original["name"], "custom control-containing name was split or replaced")
            require(any(runtime(r) == runtime(other) for r in rows), "same-folder peer disappeared")
            return current["provider"] == expected

        f.until(lambda: observed(provider), "foreground agent was not detected")
        f.tmux("send-keys", "-t", original["id"], "C-c")
        f.until(lambda: observed("shell"), "agent exit did not restore shell metadata")


def dead_agent(f):
    f.start_managed()
    shutil.copy2(shutil.which("bash"), f.bin / "codex")
    f.tmux("set-window-option", "-g", "remain-on-exit", "on")
    agent = f.create("standalone-agent", provider="codex")
    marker = f.root / "agent-ready"
    f.send(agent["id"], "printf ready > " + shlex.quote(str(marker)))
    f.until(marker.exists, "synthetic standalone agent never became ready")
    f.send(agent["id"], "exit")
    f.until(lambda: f.tmux("display-message", "-p", "-t", agent["id"], "#{pane_dead}").stdout.strip() == b"1",
            "fixture pane did not become retained-dead")
    require(not any(r["id"] == agent["id"] for r in f.rows()),
            "retained dead standalone agent advertised as attachable live work")


def dead_active_pane(f):
    # Inverse control: another live pane means the session remains useful.
    f.tmux("new-session", "-d", "-s", "multi", "/bin/bash --noprofile --norc", default=True)
    f.tmux("set-window-option", "-g", "remain-on-exit", "on", default=True)
    f.tmux("split-window", "-d", "-t", "multi", "/bin/bash --noprofile --norc", default=True)
    f.send("multi:0.0", "exit", default=True)
    f.until(lambda: f.tmux("display-message", "-p", "-t", "multi:0.0", "#{pane_dead}", default=True).stdout.strip() == b"1",
            "fixture active pane did not exit")
    require(any(r["name"] == "multi" for r in f.rows()), "dead active pane hid a session with another live pane")


def distinct_servers(f):
    f.start_managed()
    managed = f.create("managed-one", name="identical")
    f.tmux("new-session", "-d", "-s", "identical", "-c", str(f.root), "/bin/bash --noprofile --norc", default=True)
    rows = f.rows()
    require(len(rows) == 2 and len({runtime(r) for r in rows}) == 2,
            "distinct servers with same name/folder were merged")
    require(any(runtime(r) == runtime(managed) for r in rows), "managed row missing")


def stale_stop(f):
    f.start_managed()
    session = f.create("stale-stop")
    fields = {k: session[k] for k in ("id", "pid", "started", "boot_id")}
    for field, value in (("pid", session["pid"] + 1), ("started", session["started"] + "0"),
                         ("boot_id", "fixture-old-boot")):
        reply = f.request("stop_session", dict(fields, **{field: value}))
        require(bool(reply.get("error")), "stale runtime stop unexpectedly accepted")
        require(any(runtime(r) == runtime(session) for r in f.rows()), "stale stop removed live runtime")


def container_scope(f):
    state = f.root / "docker-fixture.json"
    log = f.root / "docker-calls"
    original = dict(id="a" * 64, name="/fixture", image="fixture", state="running", started_at="start-one",
                    user="root", folder="/", network="none", networks={}, ports={}, mounts=[], metadata="[]")
    state.write_text(json.dumps(original))
    f.write_tool("docker", "#!" + sys.executable + "\nimport json,sys\nfrom pathlib import Path\n"
        + "args=sys.argv[1:]\n"
        + "with Path(" + repr(str(log)) + ").open('a') as f: f.write(args[0]+'\\n')\n"
        + "if args[0]=='context': print('unix:///fixture/docker.sock')\n"
        + "elif args[0]=='info': print('fixture-engine')\n"
        + "elif args[0]=='inspect': print(Path(" + repr(str(state)) + ").read_text())\n"
        + "else: sys.exit(97)\n")
    current = f.request("container_inspect", {"id": original["id"]})["result"]
    require(bool(current), "fixture container inspection failed")
    scope = {k: current[k] for k in ("engine", "id", "name", "user", "folder", "started_at")}
    cases = [("engine", dict(scope, engine="old-engine"), original),
             ("restart", scope, dict(original, started_at="start-two")),
             ("replacement", scope, dict(original, id="b" * 64)),
             ("user", scope, dict(original, user="other")),
             ("stopped", scope, dict(original, state="exited"))]
    for label, stale, observed in cases:
        state.write_text(json.dumps(observed))
        log.write_text("")
        reply = f.request("container_files", {"scope": stale, "operation": {"op": "list", "args": {"path": "/"}}})
        require(bool(reply.get("error")), "stale container " + label + " accepted")
        require("exec" not in log.read_text().splitlines(), "stale container " + label + " reached execution")


def offline(f):
    attempts = f.root / "ssh-attempts"
    f.write_tool("ssh", "#!/bin/sh\nprintf x >> " + shlex.quote(str(attempts))
                 + "\necho 'Could not resolve hostname cx-fixture.invalid' >&2\nexit 255\n")
    device = dict(id="fixture-remote", name="fixture", target="cx-fixture.invalid",
                  account="fixture", host="fixture", status="cached", observed_at=1)
    (f.state / "devices.json").write_text(json.dumps([device]))
    result = f.run([f.binary, "sessions", "--device", "fixture-remote"], check=False)
    require(attempts.exists() and 1 <= attempts.stat().st_size <= 2,
            "offline fixture did not exercise bounded SSH transport")
    require(result.returncode != 0, "offline sessions request reported successful empty/live result")
    require(not result.stdout.strip(), "offline request returned misleading result data")


def benchmark(f, samples):
    f.start_managed()
    results = []
    count = 0
    for total in (1, 10, 30):
        while count < total:
            f.create("bench-" + str(count), name="same project")
            count += 1
        durations, calls = [], []
        for _ in range(samples):
            f.tmux_log.write_bytes(b"")
            start = time.perf_counter()
            rows = f.rows()
            durations.append((time.perf_counter() - start) * 1000)
            calls.append(f.tmux_log.stat().st_size)
            require(len(rows) == total, "benchmark discovery lost or duplicated sessions")
        results.append(dict(sessions=total, samples=samples, first_ms=round(durations[0], 3),
                            p50_ms=round(percentile(durations, .5), 3),
                            p95_ms=round(percentile(durations, .95), 3),
                            tmux_calls_min=min(calls), tmux_calls_max=max(calls)))
    return results


class HarnessTests(unittest.TestCase):
    def test_framing(self):
        body = json.dumps(dict(version=1, id="adversarial", result=[])).encode()
        self.assertEqual(decode_frame(b"CX1 " + str(len(body)).encode() + b"\n" + body)["result"], [])

    def test_bad_length_and_id(self):
        with self.assertRaises(Failure):
            decode_frame(b"CX1 0\n{}")
        body = b'{"version":1,"id":"wrong"}'
        with self.assertRaises(Failure):
            decode_frame(b"CX1 " + str(len(body)).encode() + b"\n" + body)

    def test_percentile(self):
        self.assertEqual(percentile([5, 1, 2, 3, 4], .95), 5)
        self.assertEqual(percentile([5, 1, 2, 3, 4], .5), 3)

    def test_runtime_ignores_label_not_process(self):
        row = dict(pid=1, started="one", boot_id="boot", host="host", account="user", name="old")
        self.assertEqual(runtime(row), runtime(dict(row, name="new", provider="codex")))
        self.assertNotEqual(runtime(row), runtime(dict(row, pid=2)))

    def test_private_fixture_environment(self):
        fixture = Fixture("/does-not-exist")
        try:
            self.assertEqual(fixture.env["HOME"], str(fixture.root))
            self.assertNotIn("SSH_AUTH_SOCK", fixture.env)
            self.assertNotIn("TMUX", fixture.env)
            self.assertTrue(fixture.default.is_relative_to(fixture.root))
            self.assertTrue(fixture.socket.is_relative_to(fixture.root))
        finally:
            fixture.close()

    def test_fixture_mode_refuses_live_tmux(self):
        fixture = Fixture("/does-not-exist")
        try:
            with self.assertRaises(Failure):
                fixture.tmux("list-sessions")
        finally:
            fixture.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", nargs="?")
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--self-test", action="store_true")
    mode.add_argument("--fixtures", action="store_true", help="public CLI with fake Docker/SSH; no live host operations")
    mode.add_argument("--run-owned-tmux", action="store_true", help="ROOT ONLY: creates disposable exact-socket tmux servers")
    parser.add_argument("--case", choices=["aliases", "foreground", "dead-agent", "dead-active-pane", "distinct-servers", "stale-stop", "benchmark"])
    parser.add_argument("--samples", type=int, default=5)
    args = parser.parse_args()
    if args.self_test:
        suite = unittest.defaultTestLoader.loadTestsFromTestCase(HarnessTests)
        return 0 if unittest.TextTestRunner().run(suite).wasSuccessful() else 1
    parser.error("binary is required") if not args.binary else None
    require(2 <= args.samples <= 30, "samples must be 2..30")
    cases = {"container-scope": container_scope, "offline": offline} if args.fixtures else {
        "aliases": aliases, "foreground": foreground, "dead-agent": dead_agent,
        "dead-active-pane": dead_active_pane, "distinct-servers": distinct_servers, "stale-stop": stale_stop}
    if args.case:
        require(not args.fixtures, "--case is only valid for owned tmux")
        cases = {args.case: (lambda f: benchmark(f, args.samples)) if args.case == "benchmark" else cases[args.case]}
    report = []
    for label, test in cases.items():
        fixture = None
        record = dict(case=label, status="PASS")
        start = time.monotonic()
        try:
            fixture = Fixture(args.binary, live=args.run_owned_tmux)
            evidence = test(fixture)
            if evidence is not None:
                record["measurements"] = evidence
        except Exception as error:
            record.update(status="FAIL", reason=str(error) if isinstance(error, Failure) else type(error).__name__)
        finally:
            if fixture:
                try:
                    fixture.close()
                except Exception:
                    record.update(status="FAIL", cleanup="owned fixture cleanup failed")
        record["elapsed_ms"] = round((time.monotonic() - start) * 1000, 3)
        report.append(record)
        print(json.dumps(record), flush=True)
    return int(any(item["status"] != "PASS" for item in report))


if __name__ == "__main__":
    sys.exit(main())
