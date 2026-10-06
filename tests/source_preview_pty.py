#!/usr/bin/env python3
"""Real source preview colors, inertness and keyboard restoration in disposable PTYs."""
import fcntl, json, os, pathlib, pty, select, struct, subprocess, sys, tempfile, termios, time
import pyte
binary = str(pathlib.Path(sys.argv[1]).resolve())
checks = []
for mono in (False, True):
    with tempfile.TemporaryDirectory(prefix='cx-source-preview-') as temporary:
        root = pathlib.Path(temporary); source = root/'files'; source.mkdir()
        script = source/'sample.py'
        script.write_text('from pathlib import Path\nPath("EXECUTED").write_text("not allowed")\nprint("hello")\n# inert preview\n')
        state = root/'state/cx'; state.mkdir(parents=True)
        name = 'viewer-restart-123-789.json'
        snapshot = dict(schema=1, expires_at=int(time.time())+300, device_ids=['local'], device=1,
            focus='Workspace', view='Files', selected=0, side_selected=0, search='',
            browser=dict(device=0, path=str(source), display_path=str(source), parent=str(root), entries=[], selected=0,
                search='', preview_scroll=0, restore_selection=str(script)), other_browser=None,
            destination_active=False, conflict=2, launch_provider=None, clipboard=None, submitted={})
        path = state/name; path.write_text(json.dumps(snapshot)); path.chmod(0o600)
        env = dict(os.environ, HOME=str(root), XDG_STATE_HOME=str(root/'state'), SHELL='/bin/sh', TERM='xterm-256color', HTTPS_PROXY='http://127.0.0.1:9')
        for key in ('TMUX', 'TMUX_PANE', 'NO_COLOR', 'CX_ASCII'): env.pop(key, None)
        if mono: env.update(NO_COLOR='1', CX_ASCII='1')
        master, slave = pty.openpty(); before = termios.tcgetattr(slave)
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 24, 80, 0, 0))
        def control():
            os.setsid(); fcntl.ioctl(slave, termios.TIOCSCTTY, 0)
        proc = subprocess.Popen([binary, 'restart', name], stdin=slave, stdout=slave, stderr=slave, env=env, preexec_fn=control, cwd=str(source))
        screen = pyte.Screen(80, 24); stream = pyte.Stream(screen)
        def read():
            if select.select([master], [], [], .05)[0]:
                stream.feed(os.read(master, 65536).decode(errors='replace'))
        def wait(predicate):
            deadline = time.monotonic()+12
            while not predicate() and time.monotonic()<deadline: read()
            assert predicate(), '\n'.join(screen.display)
        try:
            wait(lambda: any('sample.py' in line for line in screen.display))
            os.write(master, b'\r')
            wait(lambda: any('print("hello")' in line for line in screen.display))
            row = next(i for i, line in enumerate(screen.display) if 'print("hello")' in line)
            column = screen.display[row].index('hello')
            color = screen.buffer[row][column].fg
            assert color in (('default',) if mono else ('green', '00cd00')), (mono, color)
            assert not (root/'EXECUTED').exists() and not (source/'EXECUTED').exists()
            os.write(master, b'j')
            deadline = time.monotonic()+.2
            while time.monotonic()<deadline: read()
            assert any('print("hello")' in line for line in screen.display)
            os.write(master, b'\x1b')
            wait(lambda: any('sample.py' in line for line in screen.display) and not any('print("hello")' in line for line in screen.display))
            os.write(master, b'\x03'); proc.wait(timeout=5)
            assert proc.returncode == 0 and termios.tcgetattr(slave) == before
            checks.append(dict(mode='monochrome' if mono else 'ANSI palette', result='PASS', string_color=color, source_inert=True, escape_selection_and_termios=True))
        finally:
            if proc.poll() is None: proc.terminate(); proc.wait(timeout=5)
            os.close(master); os.close(slave)
print(json.dumps(dict(result='PASS', checks=checks)))
