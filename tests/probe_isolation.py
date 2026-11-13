#!/usr/bin/env python3
"""A fixture's adoption discovery never lists a real terminal of this machine.

usage: probe_isolation.py BINARY   (a debug build, as tests/run_regressions.sh passes)

Outside, this starts a terminal of its own: an interactive bash leading a private PTY,
exactly what discovery lists. It stands in for the user's real terminals. Two guards:
- the debug build's helper, asked outside any private namespace (as a fixture run
  without tests/isolate.sh, or one that needs the host's ids, asks), lists nothing;
- under tests/isolate.sh (always new namespaces, whatever the environment says), the
  helper lists a second terminal started there, so discovery works, and only that one.
pty_lib.Pty refuses to start outside a private pid namespace, so the outside terminal
is started here directly.
"""
import fcntl, json, os, pathlib, pty, subprocess, sys, tempfile, termios, time

HERE = pathlib.Path(__file__).resolve().parent


def terminal(home):
    """An interactive bash leading its own PTY (this process holds the master), and its tty."""
    master, slave = pty.openpty()

    def lead():
        os.setsid()
        fcntl.ioctl(slave, termios.TIOCSCTTY, 0)
    shell = subprocess.Popen(['bash', '--norc', '--noprofile', '-i'], stdin=slave, stdout=slave, stderr=slave,
                             env=dict(os.environ, HOME=str(home), PS1='$ '), cwd=home, preexec_fn=lead)
    os.close(slave)
    return (shell, master), os.readlink(f'/proc/{shell.pid}/fd/0')


def close(terminal):
    shell, master = terminal
    os.write(master, b'exit\n')
    try:
        shell.wait(timeout=5)
    except subprocess.TimeoutExpired:
        shell.kill()
        shell.wait()
    os.close(master)


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


def private():
    """In a private pid namespace with its own /proc (pty_lib.isolated, without pyte)."""
    return (os.readlink('/proc/self/ns/pid') != 'pid:[4026531836]'
            and os.readlink('/proc/self') == str(os.getpid()))


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
            close(shell)
    print(json.dumps(dict(result='PASS', listed=len(seen))))


def main():
    binary = str(pathlib.Path(sys.argv[1]).resolve())
    if sys.argv[2:3] == ['--inside']:
        return inside(binary, sys.argv[3])
    with tempfile.TemporaryDirectory(prefix='cx-probe-out-') as tmp:
        shell, tty = terminal(pathlib.Path(tmp))
        try:
            # The regression runner starts this on the host (hostids): there a debug build
            # lists nothing, the outside terminal included. isolate.sh nests anyway.
            if not private():
                assert listed(binary, pathlib.Path(tmp)) == set(), 'a terminal was listed outside a namespace'
            done = subprocess.run([str(HERE / 'isolate.sh'), sys.executable, __file__, binary, '--inside', tty],
                                  timeout=120)
            assert shell[0].poll() is None, 'the outside terminal ended early'
        finally:
            close(shell)
    return done.returncode


if __name__ == '__main__':
    sys.exit(main())
