#!/usr/bin/env python3
"""A fixture's adoption discovery never lists a real terminal of this machine.

usage: probe_isolation.py BINARY

Outside, this starts a terminal of its own: an interactive bash leading a private PTY,
exactly what discovery lists. It stands in for the user's real terminals. Then it runs
its inner half under tests/isolate.sh, the wrapper every PTY harness and fixture runs
in. The inner half starts a second terminal and asks the fixture's helper for the
outside-CX terminals it can see (`external_sessions`, the same discovery the viewer
uses). The inner terminal must be listed, so discovery works, and it must be the only
one: the outside terminal is not there. Discovery never runs outside the namespace.
"""
import json, os, pathlib, subprocess, sys, tempfile, time

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import pty_lib  # noqa: E402

HERE = pathlib.Path(__file__).resolve().parent


def terminal(home):
    """An interactive bash leading its own PTY (this process holds the master), and its tty."""
    shell = pty_lib.Pty(['bash', '--norc', '--noprofile', '-i'], dict(os.environ, HOME=str(home), PS1='$ '),
                        home, 80, 24)
    return shell, os.readlink(f'/proc/{shell.proc.pid}/fd/0')


def listed(binary, home):
    """The ttys of the terminals the fixture helper's discovery lists."""
    env = dict(os.environ, HOME=str(home), XDG_STATE_HOME=str(home / 'state'), TMUX_TMPDIR=str(home))
    for key in ('TMUX', 'TMUX_PANE'):
        env.pop(key, None)
    request = json.dumps(dict(version=1, id='probe', op=dict(op='external_sessions'))).encode()
    out = subprocess.run([binary, 'helper'], input=b'CX1 %d\n' % len(request) + request, env=env,
                         capture_output=True, timeout=30)
    assert out.returncode == 0, out.stderr[-400:]
    reply = json.loads(out.stdout.split(b'\n', 1)[1])
    assert reply['error'] is None, reply['error']
    return {(row.get('origin') or {}).get('tty') for row in reply['result']}


def inside(binary, outside_tty):
    with tempfile.TemporaryDirectory(prefix='cx-probe-in-') as tmp:
        home = pathlib.Path(tmp)
        shell, tty = terminal(home)
        try:
            deadline = time.monotonic() + 10
            seen = listed(binary, home)
            while tty not in seen and time.monotonic() < deadline:
                time.sleep(0.1)
                seen = listed(binary, home)
            assert tty in seen, f'the fixture terminal was not discovered: {sorted(map(str, seen))}'
            assert outside_tty not in seen, 'a terminal outside the namespace was listed'
            assert seen == {tty}, f'more than the fixture terminal was listed: {len(seen)}'
            # /proc itself holds only this namespace's processes.
            pids = [int(p) for p in os.listdir('/proc') if p.isdigit()]
            assert len(pids) < 50 and 1 in pids, len(pids)
        finally:
            shell.close(quit_key=b'exit\n')
    print(json.dumps(dict(result='PASS', listed=len(seen))))


def main():
    binary = str(pathlib.Path(sys.argv[1]).resolve())
    if sys.argv[2:3] == ['--inside']:
        return inside(binary, sys.argv[3])
    with tempfile.TemporaryDirectory(prefix='cx-probe-out-') as tmp:
        shell, tty = terminal(pathlib.Path(tmp))
        try:
            # A fresh namespace even when this script already runs in one (the regression runner).
            env = {k: v for k, v in os.environ.items() if k != 'CX_TEST_NS'}
            done = subprocess.run([str(HERE / 'isolate.sh'), sys.executable, __file__, binary, '--inside', tty],
                                  env=env, timeout=120)
            assert shell.proc.poll() is None, 'the outside terminal ended early'
        finally:
            shell.close(quit_key=b'exit\n')
    return done.returncode


if __name__ == '__main__':
    sys.exit(main())
