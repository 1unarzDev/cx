#!/usr/bin/env python3
"""A focus attach cancels a leftover scrollback (copy-mode) view so input lands
live; a read-only watch keeps it. Isolated managed tmux server and state."""
import fcntl, json, os, pathlib, pty, select, struct, subprocess, sys, tempfile, termios, time
binary = str(pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else 'target/release/cx').resolve())
with tempfile.TemporaryDirectory(prefix='cx-copymode-') as tmp:
    env = dict(os.environ, XDG_STATE_HOME=tmp, TERM='xterm-256color', LANG='C', LC_ALL='C')
    env.pop('TMUX', None)
    env.pop('TMUX_PANE', None)
    socket = str(pathlib.Path(tmp) / 'cx/managed.sock')

    def tmux(*args, **kw):
        return subprocess.run(['tmux', '-S', socket, *args], capture_output=True, text=True, **kw)

    def in_mode(sid):
        return tmux('display-message', '-p', '-t', sid, '#{pane_in_mode}').stdout.strip()

    def session_id(name):
        out = json.loads(subprocess.check_output([binary, 'sessions'], env=env, timeout=30))
        return next(s['id'] for s in out if s['name'] == name)

    def attach(name, observe):
        # Drive the real interactive attach over a PTY, then detach with Ctrl+].
        master, slave = pty.openpty()
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 24, 80, 0, 0))

        def controlling_tty():
            os.setsid()
            fcntl.ioctl(slave, termios.TIOCSCTTY, 0)
        cmd = [binary, 'attach', name] + (['--observe'] if observe else [])
        p = subprocess.Popen(cmd, stdin=slave, stdout=slave, stderr=slave, env=env,
                             preexec_fn=controlling_tty)
        end = time.monotonic() + 3
        while time.monotonic() < end:
            if select.select([master], [], [], .05)[0]:
                try:
                    os.read(master, 65536)
                except OSError:
                    break
            if tmux('list-clients', '-t', sid).stdout.strip():
                break
        time.sleep(.4)
        state = in_mode(sid)
        os.write(master, b'\x1d')  # Ctrl+] detach
        try:
            p.wait(timeout=3)
        finally:
            if p.poll() is None:
                p.terminate(); p.wait(timeout=3)
            os.close(master); os.close(slave)
        return state

    checks = {}
    try:
        subprocess.run([binary, 'new', '--directory', tmp, '--provider', 'shell',
                        '--name', 'cx-copymode-fixture', '--key', 'copymode-fixture'],
                       env=env, check=True, stdout=subprocess.DEVNULL)
        sid = session_id('cx-copymode-fixture')

        # Produce scrollback and enter copy-mode, as a wheel-up scroll would.
        tmux('send-keys', '-t', sid, 'seq 1 200', 'Enter'); time.sleep(.3)
        tmux('copy-mode', '-t', sid)
        checks['left_in_copy_mode'] = in_mode(sid) == '1'

        # A read-only watch must not disturb the scrollback view.
        checks['watch_keeps_copy_mode'] = attach('cx-copymode-fixture', observe=True) == '1'

        # Re-enter copy-mode (watch may have been detached without changing it).
        if in_mode(sid) != '1':
            tmux('copy-mode', '-t', sid)
        # A focus attach lands live: copy-mode is cancelled during the attach.
        checks['focus_exits_copy_mode'] = attach('cx-copymode-fixture', observe=False) == '0'

        print(json.dumps({'result': 'PASS' if all(checks.values()) else 'FAIL', 'checks': checks}))
        assert all(checks.values()), checks
    finally:
        subprocess.run(['tmux', '-S', socket, 'kill-server'], capture_output=True)
