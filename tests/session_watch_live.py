#!/usr/bin/env python3
"""Robustness of the instant-sessions path (helper watch + tmux control client).

usage: session_watch_live.py BINARY [--old OLD_BINARY] [--cycles N]

Private HOMEs, private tmux servers and (for the protocol mix) a loopback sshd; the
fixtures come from perf_harness/files_bench. Checks:
  control-kill   kill the viewer's tmux control client mid-session: typing still echoes
                 (polling fallback), and a new control client takes over.
  cycles         N attach/detach cycles (default 100) leave exactly one control client,
                 no leftover attach processes and no cx orphan.
  old-helper     (--old) a new viewer against an old remote helper: preview and echo work
                 by polling and the old helper is never sent a Watch.
  old-viewer     (--old) an old viewer against a new remote helper: preview and echo work.
"""
import argparse, json, os, pathlib, signal, subprocess, sys, time

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import perf_harness as ph  # noqa: E402
import files_bench as fb  # noqa: E402


def control_clients(home):
    """Pids of tmux control clients on this fixture's managed socket."""
    socket = str(home / 'state/cx/managed.sock')
    found = []
    for entry in pathlib.Path('/proc').iterdir():
        if entry.name.isdigit():
            try:
                args = (entry / 'cmdline').read_bytes().split(b'\0')
            except OSError:
                continue
            if b'-C' in args and socket.encode() in args:
                found.append(int(entry.name))
    return found


class Fixture:
    """A viewer on one shell session, local or on a loopback remote."""

    def __init__(self, viewer_binary, helper_binary=None, remote=False):
        self.tag = f'watch-{os.getpid()}-{time.time_ns() % 10**9}'
        self.ssh, run_env = (ph.remote_fixture(helper_binary or viewer_binary, self.tag, False, 0.0)
                             if remote else (None, None))
        self.env = None
        try:
            self.env = fb.BenchEnv(viewer_binary, self.tag, self.ssh)
            self.home = self.ssh.remote_home if self.ssh else self.env.root
            run_env = run_env or self.env.env
            folder = self.home / 'work'
            folder.mkdir(exist_ok=True)
            made = subprocess.run([helper_binary or viewer_binary, 'new', '--directory', str(folder),
                                   '--name', 'watch-shell', '--key', 'watch-shell'],
                                  env=run_env, capture_output=True, timeout=30)
            assert made.returncode == 0, 'cx new failed'
            self.viewer = ph.Viewer.__new__(ph.Viewer)
            index = len(self.env.device_ids) - 1
            self.viewer.open(self.env, dict(device_ids=self.env.device_ids, device=index + 1,
                                            focus='Workspace', view='Work', browser=None))
            self.viewer.wait(lambda: 'watch-shell' in self.text() and '$' in self.text(), timeout=30,
                             what='session preview')
        except BaseException:
            # A fixture that never finished starting still owns its sshd and HOME.
            if self.env:
                self.env.cleanup()
            if self.ssh:
                self.ssh.cleanup()
            raise

    def text(self):
        return self.viewer.text()

    def echo(self, key=b'x'):
        """Type into the focused pane; ms until the screen changed (None on timeout)."""
        before = self.text()
        ms, _ = self.viewer.measure(key, lambda: self.text() != before, timeout=5)
        return ms

    def hub(self):
        return ph.hub_layout(self.viewer)

    def focused(self):
        return '⏏ detach' in self.text() or 'typing into the session' in self.text()

    def focus(self):
        if not self.hub():
            self.viewer.measure(ph.INLINE_KEY, self.focused, timeout=10)
            self.viewer.quiesce(.2)
            return
        # The hub design: peek the session (Space), then a tap on its prompt types into it.
        self.viewer.measure(b' ', lambda: 'OTHERS' in self.text() or ' next' in self.viewer.screen.display[-1],
                            timeout=10)
        self.viewer.quiesce(.2)
        display = self.viewer.screen.display
        y = min(y for y, line in enumerate(display) if '$' in line and y > 1)
        x = display[y].index('$')
        self.viewer.measure(f'\x1b[<0;{x + 1};{y + 1}M\x1b[<0;{x + 1};{y + 1}m'.encode(), self.focused, timeout=10)
        self.viewer.quiesce(.2)

    def close(self):
        """Stop everything; fail when one of our processes survived or turned stray."""
        self.viewer.close()
        cleanup = self.env.cleanup()
        refused = {}
        if self.ssh:
            self.ssh.cleanup()
            refused = self.ssh.refused()
        check(not cleanup['remaining'] and not cleanup['orphans_ours_stray'],
              f'cleanup: {cleanup}')
        check(not refused, f'refused remote commands: {refused}')


def check(condition, message):
    if not condition:
        raise AssertionError(message)


def control_kill(binary):
    fixture = Fixture(binary)
    try:
        fixture.viewer.wait(lambda: control_clients(fixture.home), timeout=10, what='a control client')
        first = control_clients(fixture.home)
        fixture.focus()
        check(fixture.echo() is not None, 'echo before the kill')
        for pid in first:
            os.kill(pid, signal.SIGKILL)
        # Typing right away still echoes: the viewer polls while the watch is down.
        echoes = [fixture.echo(b'y'), fixture.echo(b'z')]
        check(all(e is not None for e in echoes), f'echo after the kill: {echoes}')
        deadline = time.monotonic() + 15
        while time.monotonic() < deadline and not (set(control_clients(fixture.home)) - set(first)):
            fixture.viewer.pump(.2)
        recovered = set(control_clients(fixture.home)) - set(first)
        check(recovered, 'no new control client after the kill')
        check(fixture.echo(b'w') is not None, 'echo after recovery')
        return dict(killed=len(first), recovered=len(recovered), echo_after_kill_ms=echoes)
    finally:
        fixture.close()


def cycles(binary, count):
    fixture = Fixture(binary)
    try:
        def native():
            return ('detach' in fixture.viewer.screen.display[-1] and '╭ Devices' not in fixture.text()
                    and 'NETWORK' not in fixture.text() and 'SESSIONS' not in fixture.text())

        def ui_back():
            # Detach lands on the hub (its NETWORK zone), the old layout on its Devices box.
            return '╭ Devices' in fixture.text() or 'NETWORK' in fixture.text()
        failures = 0
        for _ in range(count):
            attached, _ = fixture.viewer.measure(ph.ATTACH_KEY, native, timeout=10)
            back, _ = fixture.viewer.measure(ph.DETACH_KEY, ui_back, timeout=10)
            failures += attached is None or back is None
            fixture.viewer.quiesce(.05)
            fixture.env.owned_pids()
        fixture.viewer.quiesce(.5)
        clients = control_clients(fixture.home)
        attaching = [p for p in fixture.viewer.descendants()[1:]
                     if p not in clients and b'attach-session' in pathlib.Path(f'/proc/{p}/cmdline').read_bytes()]
        orphans = json.loads(fixture.env.cx('orphans', '--json').stdout or b'{}')
        strays = [r for r in orphans.get('processes', []) if r.get('stray')]
        check(failures == 0, f'{failures} of {count} cycles failed')
        check(len(clients) <= 1, f'{len(clients)} control clients')
        check(not attaching, 'attach clients left behind')
        check(not strays, f'{len(strays)} stray processes')
        return dict(cycles=count, control_clients=len(clients))
    finally:
        fixture.close()


def mixed(viewer_binary, helper_binary, expect_watch):
    fixture = Fixture(viewer_binary, helper_binary, remote=True)
    try:
        fixture.focus()
        echoes = [fixture.echo(b'x'), fixture.echo(b'\x7f')]
        check(all(e is not None for e in echoes), f'echo: {echoes}')
        clients = control_clients(fixture.home)
        check(bool(clients) == expect_watch, f'control clients: {len(clients)}')
        return dict(echo_ms=echoes, control_clients=len(clients))
    finally:
        fixture.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__.split('\n')[0])
    parser.add_argument('binary')
    parser.add_argument('--old')
    parser.add_argument('--cycles', type=int, default=100)
    args = parser.parse_args()
    binary = str(pathlib.Path(args.binary).resolve())
    old = str(pathlib.Path(args.old).resolve()) if args.old else None
    checks = [('control-kill', lambda: control_kill(binary)),
              ('cycles', lambda: cycles(binary, args.cycles))]
    if old:
        checks += [('old-helper', lambda: mixed(binary, old, expect_watch=False)),
                   ('old-viewer', lambda: mixed(old, binary, expect_watch=False))]
    report, failed = {}, []
    for name, run in checks:
        try:
            report[name] = run()
            print(f'PASS {name} {json.dumps(report[name])}')
        except AssertionError as error:
            failed.append(name)
            print(f'FAIL {name}: {error}')
    sys.exit(1 if failed else 0)


if __name__ == '__main__':
    main()
