#!/usr/bin/env python3
"""Launch-directory and offline cache regression in a real PTY with owned state."""
import fcntl, json, os, pathlib, pty, select, struct, subprocess, sys, tempfile, termios, time
import pyte

binary = str(pathlib.Path(sys.argv[1]).resolve())
with tempfile.TemporaryDirectory(prefix='cx-startup-directory-') as temporary:
    root = pathlib.Path(temporary)
    project = root / "project ' space"
    project.mkdir()
    (project / 'launch-directory-proof.txt').write_text('fixture')
    state = root / 'state/cx'
    state.mkdir(parents=True)
    bins = root / 'bin'
    bins.mkdir()
    ssh = bins / 'ssh'
    ssh.write_text('#!/bin/sh\nexit 255\n')
    ssh.chmod(0o700)
    devices = [dict(id='local', name='fixture-local', target=None, account='tester', host='fixture-local', status='unknown', observed_at=0),
               dict(id='offline', name='fixture-offline', target='fixture-offline', account='tester', host='fixture-offline', status='unknown', observed_at=0)]
    (state / 'devices.json').write_text(json.dumps(devices))
    cached = dict(id='stale-shell', name='stale-shell', directory='/old/project', provider='shell', host='fixture-offline',
                  account='tester', pid=123, started='old', boot_id='old-boot', external=False, socket=None)
    (state / 'ui-sessions.json').write_text(json.dumps({'offline': [cached]}))
    env = dict(os.environ, HOME=temporary, XDG_STATE_HOME=str(root/'state'), XDG_CONFIG_HOME=str(root/'config'),
               PATH=str(bins)+':/usr/bin:/bin', SHELL='/bin/sh', TERM='xterm-256color', HTTPS_PROXY='http://127.0.0.1:9')
    env.pop('TMUX', None)
    env.pop('TMUX_PANE', None)
    master, slave = pty.openpty()
    before = termios.tcgetattr(slave)
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 24, 100, 0, 0))
    def controlling_terminal():
        os.setsid()
        fcntl.ioctl(slave, termios.TIOCSCTTY, 0)
    process = subprocess.Popen([binary], cwd=project, env=env, stdin=slave, stdout=slave, stderr=slave,
                               preexec_fn=controlling_terminal)
    screen = pyte.Screen(100, 24)
    stream = pyte.Stream(screen)
    def read(duration=.1):
        end = time.monotonic()+duration
        while time.monotonic()<end:
            if select.select([master], [], [], .02)[0]:
                try:
                    stream.feed(os.read(master, 65536).decode(errors='replace'))
                except OSError:
                    break
    try:
        deadline = time.monotonic()+10
        while time.monotonic()<deadline:
            read()
            saved = json.loads((state/'ui-sessions.json').read_text())
            if 'launch-directory-proof.txt' in '\n'.join(screen.display) and not saved.get('offline'):
                break
        assert 'launch-directory-proof.txt' in '\n'.join(screen.display), 'launch directory did not open'
        assert not saved.get('offline'), 'offline shell remained in persistent cache'
        os.write(master, b'\x03')
        process.wait(timeout=5)
        assert process.returncode == 0
        assert termios.tcgetattr(slave) == before, 'terminal modes not restored'
        print(json.dumps(dict(result='PASS', checks=['launch cwd opened in Files', 'offline cached device checked while browsing local Files',
                                                     'unattachable cached shell removed from disk', 'terminal modes restored'])))
    finally:
        if process.poll() is None:
            process.terminate()
            process.wait(timeout=5)
        os.close(master)
        os.close(slave)
