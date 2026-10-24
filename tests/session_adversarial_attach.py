#!/usr/bin/env python3
"""ROOT ONLY: checked native attachment on disposable servers and owned PTYs.

No transcript is retained. Importing this module performs no host operation.
"""
import argparse
import base64
import copy
import fcntl
import hashlib
import json
import os
import pty
import select
import struct
import subprocess
import sys
import termios
import time
import unittest

sys.dont_write_bytecode = True
from session_adversarial import Fixture, Failure, require


def payload(session, observe=False):
    # Confirmed with the runtime owner; exact observed Session is not reconstructed.
    return base64.b64encode(json.dumps({"session": session, "observe": observe}).encode()).decode()


def clients(f, default):
    result = f.tmux("list-clients", "-F", "#{client_tty}|#{session_id}|#{client_readonly}",
                    default=default, check=False)
    return [line.split("|") for line in result.stdout.decode().splitlines() if line]


def server_snapshot(f, default):
    # Only private server metadata/options, never pane output or environment.
    chunks = []
    for args in (("show-options", "-g"), ("show-window-options", "-g"),
                 ("list-sessions", "-F", "#{session_id}|#{session_created}|#{session_name}")):
        chunks.append(f.tmux(*args, default=default).stdout)
    return hashlib.sha256(b"\0".join(chunks)).hexdigest()


class Client:
    def __init__(self, f, session, observe=False, env=None):
        self.f = f
        self.master, self.slave = pty.openpty()
        fcntl.ioctl(self.slave, termios.TIOCSWINSZ, struct.pack("HHHH", 24, 80, 0, 0))
        self.before = termios.tcgetattr(self.slave)
        self.tty = os.ttyname(self.slave)

        def controlling_terminal():
            os.setsid()
            fcntl.ioctl(self.slave, termios.TIOCSCTTY, 0)

        self.process = subprocess.Popen([f.binary, "native-attach-checked", payload(session, observe)],
            env=env or f.env, cwd=f.root, stdin=self.slave, stdout=self.slave,
            stderr=self.slave, preexec_fn=controlling_terminal)

    def drain(self):
        # Discard bounded chunks; never include terminal output in assertions.
        for _ in range(16):
            if not select.select([self.master], [], [], 0)[0]:
                break
            try:
                os.read(self.master, 65536)
            except OSError:
                break

    def wait_attached(self, default, numeric, observe):
        deadline = time.monotonic() + 6
        while time.monotonic() < deadline:
            self.drain()
            rows = clients(self.f, default)
            if [self.tty, numeric, str(int(observe))] in rows:
                require(not any(r[0] == self.tty for r in clients(self.f, not default)),
                        "client attached to both owned servers")
                return
            require(self.process.poll() is None, "checked client exited before attachment")
            time.sleep(.03)
        raise Failure("checked client did not attach to expected numeric session")

    def detach(self, default):
        self.f.tmux("detach-client", "-t", self.tty, default=default)
        self.process.wait(timeout=6)
        self.drain()
        require(self.process.returncode == 0, "normal checked detach returned failure")
        require(termios.tcgetattr(self.slave) == self.before, "termios not restored after detach")

    def reject(self):
        deadline = time.monotonic() + 6
        while self.process.poll() is None and time.monotonic() < deadline:
            self.drain()
            require(not any(r[0] == self.tty for default in (False, True) for r in clients(self.f, default)),
                    "invalid identity attached a client")
            time.sleep(.03)
        require(self.process.poll() is not None, "invalid identity did not fail promptly")
        require(self.process.returncode != 0, "invalid checked identity reported success")
        require(not any(r[0] == self.tty for default in (False, True) for r in clients(self.f, default)),
                "invalid identity left an attached client")
        require(termios.tcgetattr(self.slave) == self.before, "termios changed after rejection")

    def close(self):
        if self.process.poll() is None:
            self.process.terminate()
            try:
                self.process.wait(timeout=3)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait(timeout=3)
        os.close(self.master)
        os.close(self.slave)


def exercise(f):
    f.start_managed()
    managed = f.create("attach-managed", name="managed fixture")
    f.tmux("new-session", "-d", "-s", "external-original", "/bin/bash --noprofile --norc", default=True)
    # Distinct personal-style options on our disposable external server.
    f.tmux("set-option", "-g", "status", "off", default=True)
    f.tmux("set-option", "-g", "prefix", "C-a", default=True)
    external = next(r for r in f.rows() if r["external"])
    for row in (managed, external):
        require(bool(row.get("terminal")), "binary lacks terminal generation evidence")
    protected = server_snapshot(f, True)
    managed_protected = server_snapshot(f, False)
    checks = []
    identity = external["terminal"]
    predicate = [f.binary, "native-attach-check", payload(external), str(identity["server_pid"]),
                 identity["session_id"], identity["created"]]
    require(f.run(predicate, check=False).returncode == 0, "exact attachment predicate rejected current runtime")
    for index, wrong in ((3, "1"), (4, "$999999"), (5, "wrong-created")):
        invalid = list(predicate)
        invalid[index] = wrong
        require(f.run(invalid, check=False).returncode != 0, "attachment predicate accepted wrong queue identity")
    checks.append("same-queue predicate accepts exact identity and rejects changed server/session/creation")
    for default, row in ((False, managed), (True, external)):
        for watch in (False, True):
            client = Client(f, row, watch)
            try:
                client.wait_attached(default, row["terminal"]["session_id"], watch)
                if watch:
                    marker = f.root / ("watch-" + str(default))
                    os.write(client.master, ("printf forbidden > " + str(marker) + "\r").encode())
                    until = time.monotonic() + .2
                    while time.monotonic() < until:
                        client.drain()
                        time.sleep(.01)
                    require(not marker.exists(), "watch client injected shell input")
                client.detach(default)
            finally:
                client.close()
            require(server_snapshot(f, True) == protected, "checked attach changed external sessions/options")
            require(server_snapshot(f, False) == managed_protected, "checked attach changed managed session inventory/global options")
            require(len(f.rows()) == 2, "checked attach created or removed a session")
    checks.append("managed/external attach and watch target exact session, block watch input, restore termios")

    for field, value in (("server_start", "wrong-start"), ("server_pid", 1),
                         ("created", "wrong-created"), ("session_id", "$999999")):
        stale = copy.deepcopy(external)
        stale["terminal"][field] = value
        client = Client(f, stale)
        try:
            client.reject()
        finally:
            client.close()
        require(server_snapshot(f, True) == protected, "rejection changed external server")
    checks.append("wrong server generation, numeric target and creation evidence reject without attachment")

    # Labels are mutable: numeric identity must keep the original after rename.
    f.tmux("rename-session", "-t", external["terminal"]["session_id"], "renamed-original", default=True)
    f.tmux("new-session", "-d", "-s", "external-original", "/bin/bash --noprofile --norc", default=True)
    client = Client(f, external)
    try:
        client.wait_attached(True, external["terminal"]["session_id"], False)
        client.detach(True)
    finally:
        client.close()
    checks.append("reused friendly name does not replace original numeric terminal")

    f.tmux("kill-session", "-t", external["terminal"]["session_id"], default=True)
    before_dead = server_snapshot(f, True)
    client = Client(f, external)
    try:
        client.reject()
    finally:
        client.close()
    require(server_snapshot(f, True) == before_dead, "dead-session rejection changed surviving peer")
    checks.append("dead selected session refuses attachment and preserves same-name peer")
    return checks


class PayloadTests(unittest.TestCase):
    def test_payload_shape(self):
        session = {"name": "quote '\n unicode 日本語", "terminal": {"session_id": "$0"}}
        self.assertEqual(json.loads(base64.b64decode(payload(session, True))), {"session": session, "observe": True})


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", nargs="?")
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--self-test", action="store_true")
    mode.add_argument("--run-owned-tmux", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        result = unittest.TextTestRunner().run(unittest.defaultTestLoader.loadTestsFromTestCase(PayloadTests))
        return int(not result.wasSuccessful())
    if not args.binary:
        parser.error("binary required")
    fixture = None
    report = {"case": "checked-native-attach", "status": "PASS"}
    try:
        fixture = Fixture(args.binary, live=True)
        report["checks"] = exercise(fixture)
    except Exception as error:
        report.update(status="FAIL", reason=str(error) if isinstance(error, Failure) else type(error).__name__)
    finally:
        if fixture:
            try:
                fixture.close()
            except Exception:
                report.update(status="FAIL", cleanup="owned server cleanup failed")
    print(json.dumps(report))
    return int(report["status"] != "PASS")


if __name__ == "__main__":
    sys.exit(main())
