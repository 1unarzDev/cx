#!/usr/bin/env python3
"""Antigravity (agy) and opencode scroll in-app: the wheel reaches them as SGR mouse bytes.

Both run full screen with mouse reporting (agy through `--settings
'{"altScreenMode":"always"}'`, opencode through its inline `{"mouse":true}`), so the
managed server's wheel binding forwards the wheel natively (`send-keys -M`). For each,
a stand-in (a copy of python named `agy` / `opencode`, so /proc/PID/exe says so) that
enters the alternate screen and requests ?1002h/?1006h, on a real managed tmux PTY:
  1. Wheel up/down: the exact SGR bytes reach it; no copy-mode; never an arrow key.
  2. A read-only observer's wheel reaches nothing.
  3. Mouse off (a stale or refused policy), alternate screen, no history: the wheel
     sends nothing and opens no empty copy-mode (their table row has no SGR fallback,
     unlike Codex), and `cx native-wheel` from the controller sends nothing either.
  4. With history and mouse off, the wheel opens tmux scrollback over real history
     only (the stock rule for any app), still never arrows.
No provider, transcripts, user config, or inference.
Usage: tests/isolate.sh python3 tests/agent_wheel_pty.py BINARY
"""
import fcntl, json, os, pathlib, pty, select, shlex, shutil, struct, subprocess, sys, tempfile, termios, time

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
from isolation import isolate  # noqa: E402

isolate()
binary = str(pathlib.Path(sys.argv[1]).resolve())
PYTHON = os.path.realpath(sys.executable)
ARROWS = (b'\x1b[A', b'\x1b[B', b'\x1bOA', b'\x1bOB')
# The stand-in: raw, full screen, mouse on; `m` turns mouse reporting off, `M` on,
# `h` prints history lines on the main screen first. Every byte read is logged.
STANDIN = r'''import os, pathlib, sys, tty
log = pathlib.Path(sys.argv[1]); tty.setraw(0)
def out(s): sys.stdout.write(s); sys.stdout.flush()
if len(sys.argv) > 2:
    for n in range(120): out('HISTORY_LINE_%03d\r\n' % n)
out('\x1b[?1049h\x1b[?1002h\x1b[?1006hSTANDIN_TRANSCRIPT\r\n')
log.write_bytes(b'')
while True:
    data = os.read(0, 4096)
    with log.open('ab') as f: f.write(data)
    if data == b'm': out('\x1b[?1002l\x1b[?1006l')
    if data == b'M': out('\x1b[?1002h\x1b[?1006h')
'''

with tempfile.TemporaryDirectory(prefix='cx-agent-wheel-') as temporary:
    root = pathlib.Path(temporary)
    (root / '.local/bin').mkdir(parents=True)
    (root / '.local/bin/cx').symlink_to(binary)
    env = dict(os.environ, HOME=str(root), XDG_STATE_HOME=str(root / 'state'),
               TMUX_TMPDIR=str(root), SHELL='/bin/bash', TERM='xterm-256color')
    env.pop('TMUX', None); env.pop('TMUX_PANE', None)
    socket = str(root / 'state/cx/managed.sock')
    script = root / 'standin.py'
    script.write_text(STANDIN)

    def tmux(*args):
        return subprocess.check_output(['tmux', '-S', socket, *args], env=env).decode().strip()

    checks = []
    clients = []
    master = slave = None
    try:
        for name in ('agy', 'opencode'):
            exe = root / 'bin' / name
            exe.parent.mkdir(exist_ok=True)
            shutil.copyfile(PYTHON, exe); exe.chmod(0o700)
            for history in (False, True):
                key = f'{name}-{"history" if history else "fresh"}'
                session = json.loads(subprocess.check_output(
                    [binary, 'new', '--provider', 'shell', '--directory', str(root), '--key', key], env=env))
                tmux('set-environment', '-g', 'HOME', str(root))
                tmux('set-environment', '-g', 'XDG_STATE_HOME', str(root / 'state'))
                master, slave = pty.openpty()
                fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 24, 80, 0, 0))

                def tty(slave=slave):
                    os.setsid(); fcntl.ioctl(slave, termios.TIOCSCTTY, 0)
                client = subprocess.Popen([binary, 'native-attach', 'attach-session', '-t', session['id']],
                                          stdin=slave, stdout=slave, stderr=slave, env=env, preexec_fn=tty)
                clients.append(client)

                def drain(master=master):
                    while select.select([master], [], [], 0)[0]:
                        os.read(master, 65536)

                def wait(predicate, message, client=client):
                    deadline = time.monotonic() + 6
                    while not predicate():
                        drain()
                        assert client.poll() is None, 'client unexpectedly exited'
                        assert time.monotonic() < deadline, f'{name}: {message}'
                        time.sleep(.02)
                pane = session['id'] + ':0.0'

                def state(fmt, pane=pane):
                    return tmux('display-message', '-p', '-t', pane, fmt)
                log = root / f'{key}.keys'
                wait(lambda: 'native-wheel' in tmux('list-keys', '-T', 'root'), 'wheel binding unavailable')
                command = 'exec ' + ' '.join(shlex.quote(a) for a in
                                             [str(exe), str(script), str(log)] + (['history'] if history else []))
                tmux('send-keys', '-t', pane, '-l', command); tmux('send-keys', '-t', pane, 'Enter')
                wait(log.exists, 'stand-in not ready')
                wait(lambda: state('#{alternate_on} #{mouse_any_flag}') == '1 1', 'full screen with mouse')
                if not history:
                    tmux('clear-history', '-t', pane)
                up, down = b'\x1b[<64;10;10M', b'\x1b[<65;10;10M'

                def no_arrows(log=log):
                    data = log.read_bytes()
                    assert not any(a in data for a in ARROWS), f'{name}: arrow keys synthesized: {data!r}'
                if not history:
                    # 1. The wheel reaches the app as SGR, in order; no copy-mode.
                    os.write(master, up)
                    wait(lambda: log.read_bytes() == up, 'wheel up not delivered as SGR')
                    os.write(master, down)
                    wait(lambda: log.read_bytes() == up + down, 'wheel down not delivered as SGR')
                    for _ in range(8):
                        os.write(master, up)
                    wait(lambda: log.read_bytes() == up + down + up * 8, 'wheel burst lost or reordered')
                    assert state('#{pane_in_mode}') == '0', f'{name}: wheel entered copy-mode'
                    no_arrows()
                    checks.append(f'{name}: wheel up/down/burst reach it as exact SGR bytes, no copy-mode, no arrows')
                    # 2. A read-only observer sends nothing.
                    om, oslv = pty.openpty()
                    fcntl.ioctl(oslv, termios.TIOCSWINSZ, struct.pack('HHHH', 24, 80, 0, 0))

                    def otty(oslv=oslv):
                        os.setsid(); fcntl.ioctl(oslv, termios.TIOCSCTTY, 0)
                    observer = subprocess.Popen([binary, 'native-attach', 'attach-session', '-r', '-t', session['id']],
                                                stdin=oslv, stdout=oslv, stderr=oslv, env=env, preexec_fn=otty)
                    clients.append(observer)
                    wait(lambda: any(l.endswith(' 1') for l in tmux('list-clients', '-F', '#{client_pid} #{client_readonly}').splitlines()),
                         'read-only observer')
                    readonly = next(l.split()[0] for l in tmux('list-clients', '-F', '#{client_pid} #{client_readonly}').splitlines() if l.endswith(' 1'))
                    before = log.read_bytes()
                    os.write(om, up); time.sleep(.3)
                    while select.select([om], [], [], 0)[0]:
                        os.read(om, 65536)
                    subprocess.run([binary, 'native-wheel', state('#{pane_id}'), 'up', '9', '9', readonly], env=env, check=True)
                    time.sleep(.2); drain()
                    assert log.read_bytes() == before, f'{name}: a read-only observer injected input'
                    observer.terminate(); observer.wait(timeout=5); os.close(om); os.close(oslv)
                    checks.append(f'{name}: a read-only observer\'s wheel reaches nothing')
                    # 3. Mouse off, full screen, no history: nothing, and no empty copy-mode.
                    os.write(master, b'm')
                    wait(lambda: state('#{mouse_any_flag}') == '0', 'mouse off')
                    tmux('clear-history', '-t', pane)
                    before = log.read_bytes()
                    os.write(master, up); time.sleep(.3); drain()
                    controller = next(l.split()[0] for l in tmux('list-clients', '-F', '#{client_pid} #{client_readonly}').splitlines() if l.endswith(' 0'))
                    subprocess.run([binary, 'native-wheel', state('#{pane_id}'), 'up', '9', '9', controller], env=env, check=True)
                    time.sleep(.2); drain()
                    assert log.read_bytes() == before, f'{name}: stale-mouse wheel sent bytes: {log.read_bytes()[len(before):]!r}'
                    assert state('#{pane_in_mode}') == '0', f'{name}: empty 0/0 copy-mode opened'
                    no_arrows()
                    checks.append(f'{name}: mouse off + full screen + no history: nothing sent, no copy-mode (no SGR fallback)')
                else:
                    # 4. Real history, mouse off: tmux scrollback over it; never arrows.
                    wait(lambda: int(state('#{history_size}')) > 50, 'history')
                    os.write(master, b'm')
                    wait(lambda: state('#{mouse_any_flag}') == '0', 'mouse off')
                    before = log.read_bytes()
                    os.write(master, up)
                    wait(lambda: state('#{pane_in_mode}') == '1', 'scrollback over real history')
                    assert log.read_bytes() == before, f'{name}: wheel reached the app with mouse off'
                    no_arrows()
                    tmux('send-keys', '-t', pane, '-X', 'cancel')
                    checks.append(f'{name}: mouse off with real history: tmux scrollback, no input sent')
                pid = state('#{pane_pid}')
                os.write(master, b'\x1c'); client.wait(timeout=5)
                assert client.returncode == 0 and state('#{pane_pid}') == pid and state('#{pane_dead}') == '0', \
                    f'{name}: Ctrl+\\ did not detach cleanly'
                os.close(master); os.close(slave); master = slave = None
        checks.append('Ctrl+\\ detaches from both and leaves them running')
        print(json.dumps(dict(result='PASS', backend=tmux('-V'), checks=checks)))
    finally:
        for c in clients:
            if c.poll() is None:
                c.terminate(); c.wait(timeout=5)
        for fd in (master, slave):
            if fd is not None:
                os.close(fd)
        subprocess.run(['tmux', '-S', socket, 'kill-server'], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
