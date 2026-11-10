#!/usr/bin/env python3
"""Real managed tmux PTY: Ctrl+] reaches the application during a native attach
even when the managed server is still running an older config that bound it to
detach (the `unbind-key -n C-]` line in the regenerated config releases it on the
next attach), and the status line honours CX_ASCII.
No provider, transcripts, user config, or inference. Usage: script BINARY.
"""
import fcntl, json, os, pathlib, pty, re, select, shlex, struct, subprocess, sys, tempfile, termios, time
binary = str(pathlib.Path(sys.argv[1]).resolve())
with tempfile.TemporaryDirectory(prefix='cx-ctrlbr-') as temporary:
    root = pathlib.Path(temporary)
    (root/'.local/bin').mkdir(parents=True)
    (root/'.local/bin/cx').symlink_to(binary)
    env = dict(os.environ, HOME=str(root), XDG_STATE_HOME=str(root/'state'), SHELL='/bin/bash', TERM='xterm-256color')
    for name in ('TMUX', 'TMUX_PANE', 'CX_ASCII', 'NO_COLOR'):
        env.pop(name, None)
    socket = str(root/'state/cx/managed.sock')
    session = json.loads(subprocess.check_output([binary, 'new', '--provider', 'shell', '--directory', str(root), '--key', 'ctrl-bracket-fixture'], env=env))

    def tmux(*args):
        return subprocess.check_output(['tmux', '-S', socket, *args], env=env).decode().strip()
    version = tmux('-V')
    major, minor = (int(x) for x in re.match(r'tmux (\d+)\.(\d+)', version).groups())
    tmux('set-environment', '-g', 'HOME', str(root))
    tmux('set-environment', '-g', 'XDG_STATE_HOME', str(root/'state'))
    # The server as an older cx left it: Ctrl+] detaches.
    tmux('bind-key', '-n', 'C-]', 'detach-client')
    assert 'C-]' in tmux('list-keys', '-T', 'root'), 'fixture could not install the old binding'
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 24, 80, 0, 0))

    def tty():
        os.setsid(); fcntl.ioctl(slave, termios.TIOCSCTTY, 0)
    screen = bytearray()

    def drain():
        while select.select([master], [], [], 0)[0]:
            screen.extend(os.read(master, 65536))

    def attach(extra):
        screen.clear()
        return subprocess.Popen([binary, 'native-attach', 'attach-session', '-t', session['id']],
                                stdin=slave, stdout=slave, stderr=slave, env=dict(env, **extra), preexec_fn=tty)
    client = None

    def wait(predicate, message):
        deadline = time.monotonic()+6
        while not predicate():
            drain()
            assert client is None or client.poll() is None, 'client unexpectedly exited'
            assert time.monotonic() < deadline, message
            time.sleep(.02)
    pane = session['id'] + ':0.0'
    log = root/'keys'
    fixture = root/'fixture.py'
    fixture.write_text('''import os,pathlib,sys,tty
log=pathlib.Path(sys.argv[1]); tty.setraw(0)
print('CTRL_BRACKET_FIXTURE_READY', end='\\r\\n'); sys.stdout.flush(); log.write_bytes(b'')
while True:
 data=os.read(0,4096)
 with log.open('ab') as stream: stream.write(data)
''')
    checks = []
    try:
        command = 'exec '+shlex.quote(sys.executable)+' '+shlex.quote(str(fixture))+' '+shlex.quote(str(log))
        tmux('send-keys', '-t', pane, '-l', command); tmux('send-keys', '-t', pane, 'Enter')
        wait(log.exists, 'fixture not ready')
        client = attach({'CX_ASCII': '1', 'NO_COLOR': '1'})
        wait(lambda: tmux('display-message', '-p', '-t', session['id'], '#{session_attached}') == '1', 'attach failed')
        keys = tmux('list-keys', '-T', 'root')
        assert 'C-]' not in keys, 'regenerated config did not release Ctrl+] on the running server'
        checks.append('attach re-sources the config on a running server and unbinds the old Ctrl+] detach')
        time.sleep(.4); drain()
        os.write(master, b'\x1d')
        wait(lambda: b'\x1d' in log.read_bytes(), 'Ctrl+] did not reach the application')
        time.sleep(.3); drain()
        assert client.poll() is None, 'Ctrl+] detached the client'
        checks.append('Ctrl+] (0x1d) reaches the application and does not detach')
        # ASCII viewer: the status line carries no Unicode glyphs.
        assert tmux('show-options', '-v', '-t', session['id'], '@cx_ascii') == '1', 'ASCII viewer not recorded'
        text = screen.decode('utf-8', 'replace')
        assert 'detach' in text, 'status line missing the detach hint'
        assert not re.search(r'[^\x00-\x7f]', text), 'Unicode leaked into an ASCII status line'
        checks.append('CX_ASCII attach: status line is pure ASCII ([detach] chip, | separators)')
        os.write(master, b'\x1c'); client.wait(timeout=5)
        assert client.returncode == 0 and tmux('display-message', '-p', '-t', pane, '#{pane_dead}') == '0'
        checks.append('Ctrl+\\ detaches without killing the application')
        client = attach({})
        wait(lambda: tmux('display-message', '-p', '-t', session['id'], '#{session_attached}') == '1', 'reattach failed')
        assert tmux('show-options', '-v', '-t', session['id'], '@cx_ascii') == '0', 'Unicode viewer kept ASCII glyphs'
        if (major, minor) >= (3, 0):
            wait(lambda: '⏏ detach' in screen.decode('utf-8', 'replace'), 'Unicode detach chip missing')
            checks.append('Unicode viewer gets the tappable ⏏ detach chip')
        os.write(master, b'\x1c'); client.wait(timeout=5)
        assert client.returncode == 0
        print(json.dumps(dict(result='PASS', backend=version, checks=checks)))
    finally:
        if client is not None and client.poll() is None:
            client.terminate(); client.wait(timeout=5)
        os.close(master); os.close(slave)
        subprocess.run(['tmux', '-S', socket, 'kill-server'], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
