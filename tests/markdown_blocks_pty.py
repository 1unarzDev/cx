#!/usr/bin/env python3
"""Real responsive Markdown tables and highlighted fenced-code previews."""
import fcntl, json, os, pathlib, pty, select, struct, subprocess, sys, tempfile, termios, time
import pyte
binary = str(pathlib.Path(sys.argv[1]).resolve())
checks = []
for mono in (False, True):
    with tempfile.TemporaryDirectory(prefix='cx-source-preview-') as temporary:
        root = pathlib.Path(temporary); source = root/'files'; source.mkdir()
        script = source/'sample.md'
        script.write_text('# Preview\n| Device | Count |\n| :--- | ---: |\n| workstation-with-a-long-label | 42 |\n\n```python3\nprint("hello")\n```\n')
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
        def settle():
            # Key sequences must not race an in-flight frame or Escape decoding.
            deadline=time.monotonic()+.25
            while time.monotonic()<deadline: read()
        def wait(predicate):
            deadline = time.monotonic()+12
            while not predicate() and time.monotonic()<deadline: read()
            assert predicate(), '\n'.join(screen.display)
        try:
            wait(lambda: any('sample.md' in line for line in screen.display))
            os.write(master, b'\r')
            wait(lambda: any('python3' in line and '```' not in line for line in screen.display))
            wait(lambda: any('print("hello")' in line for line in screen.display))
            assert not any('| :--- | ---: |' in line for line in screen.display)
            row = next(i for i,line in enumerate(screen.display) if 'print("hello")' in line)
            column = screen.display[row].index('hello')
            cell=screen.buffer[row][column]
            assert cell.bg=='default'
            assert cell.fg in (('default',) if mono else ('green','00cd00')), cell
            os.write(master,b'/hello')
            wait(lambda: any('Search' in line and '1/1' in line for line in screen.display))
            os.write(master,b'\r'); settle()
            wait(lambda: any('j/k' in line and 'Scroll' in line for line in screen.display))
            # Resize into the narrow labeled-row layout without losing the search.
            screen.resize(lines=24,columns=48)
            fcntl.ioctl(slave,termios.TIOCSWINSZ,struct.pack('HHHH',24,48,0,0))
            import signal
            os.kill(proc.pid,signal.SIGWINCH)
            wait(lambda: any('Device:' in line for line in screen.display) and any('Count: 42' in line for line in screen.display))
            settle()
            os.write(master,b'n'); settle()
            os.write(master,b'G'); settle()
            wait(lambda: any('hello' in line for line in screen.display))
            os.write(master,b'\x1b'); settle()
            wait(lambda: not any('Search' in line and '1/1' in line for line in screen.display))
            os.write(master,b'\x1b')
            wait(lambda: any('sample.md' in line for line in screen.display) and not any('print("hello")' in line for line in screen.display))
            os.write(master, b'\x03'); proc.wait(timeout=5)
            assert proc.returncode == 0 and termios.tcgetattr(slave) == before
            checks.append(dict(mode='monochrome' if mono else 'ANSI palette', result='PASS', responsive_table=True, native_code_highlight=True, resize_search=True, escape_selection_and_termios=True))
        finally:
            if proc.poll() is None: proc.terminate(); proc.wait(timeout=5)
            os.close(master); os.close(slave)
print(json.dumps(dict(result='PASS', checks=checks)))
