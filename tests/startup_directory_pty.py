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
    contacted = root / 'offline-contacted'
    ssh.write_text('#!/bin/sh\n: > ' + "'" + str(contacted).replace("'", "'\\''") + "'" + '\nexit 255\n')
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
        # The start view is the hub; this device's Files (Enter, then `f`) are the
        # launch folder.
        deadline = time.monotonic()+10
        while time.monotonic()<deadline and 'cx › network' not in '\n'.join(screen.display):
            read()
        assert 'cx › network' in '\n'.join(screen.display), 'the hub is not the start view'
        os.write(master, b'\r')
        read(.4)
        os.write(master, b'f')
        deadline = time.monotonic()+10
        while time.monotonic()<deadline:
            read()
            saved = json.loads((state/'ui-sessions.json').read_text())
            if 'launch-directory-proof.txt' in '\n'.join(screen.display) and contacted.exists():
                break
        assert 'launch-directory-proof.txt' in '\n'.join(screen.display), 'launch directory did not open'
        assert contacted.exists(), 'offline cached device was not checked'
        read(1)
        saved = json.loads((state/'ui-sessions.json').read_text())
        # A failed refresh retains cached rows as navigation context; it never
        # rewrites them as freshly observed or attachable.
        assert saved.get('offline') == [cached], ('failed refresh changed the cached offline row', saved.get('offline'))
        os.write(master, b'\x03')
        process.wait(timeout=5)
        assert process.returncode == 0
        assert termios.tcgetattr(slave) == before, 'terminal modes not restored'
        print(json.dumps(dict(result='PASS', checks=['hub is the start view', 'launch cwd opened in this device\'s Files', 'offline cached device checked while browsing local Files',
                                                     'failed refresh retained the cached row unchanged', 'terminal modes restored'])))
    finally:
        if process.poll() is None:
            process.terminate()
            process.wait(timeout=5)
        os.close(master)
        os.close(slave)
