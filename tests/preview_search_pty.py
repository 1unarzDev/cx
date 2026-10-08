#!/usr/bin/env python3
"""Live fuzzy content search, bounded end motion and keyboard restoration in disposable PTYs."""
import fcntl, json, os, pathlib, pty, select, struct, subprocess, sys, tempfile, termios, time
import pyte
binary = str(pathlib.Path(sys.argv[1]).resolve())
checks = []
for mono in (False, True):
    with tempfile.TemporaryDirectory(prefix='cx-source-preview-') as temporary:
        root = pathlib.Path(temporary); source = root/'files'; source.mkdir()
        script = source/'sample.txt'
        script.write_text('TOP MARKER\nneedle first\n' + ''.join(f'ordinary line {i}\n' for i in range(80)) + 'n-e-e-d-l-e fuzzy\n' + ''.join(f'later line {i}\n' for i in range(80)) + 'needle last\nBOTTOM MARKER\n')
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
            wait(lambda: any('sample.txt' in line for line in screen.display))
            os.write(master, b'\r')
            wait(lambda: any('TOP MARKER' in line for line in screen.display))
            # Hovered preview owns the wheel even after keyboard focus leaves it.
            os.write(master, b'\t')
            wait(lambda: any('Focus: Devices' in line for line in screen.display))
            os.write(master, b'\x1b[<65;40;10M')
            wait(lambda: not any('TOP MARKER' in line for line in screen.display))
            wait(lambda: any('Focus: Files' in line for line in screen.display))
            os.write(master, b'\x1b[<64;40;10M')
            wait(lambda: any('TOP MARKER' in line for line in screen.display))
            os.write(master, b'\t\x1b[<0;40;10M')
            wait(lambda: any('Focus: Files' in line or 'Focus: Preview' in line for line in screen.display))
            os.write(master, b'G')
            wait(lambda: any('BOTTOM MARKER' in line for line in screen.display))
            os.write(master, b'j')
            deadline = time.monotonic()+.2
            while time.monotonic()<deadline: read()
            assert any('BOTTOM MARKER' in line for line in screen.display)
            os.write(master, b'gg')
            wait(lambda: any('TOP MARKER' in line for line in screen.display))
            os.write(master, b'/needle')
            wait(lambda: any('1/3' in line for line in screen.display))
            row = next(i for i, line in enumerate(screen.display) if 'needle first' in line)
            col = screen.display[row].index('needle first')
            cell = screen.buffer[row][col]
            assert cell.underscore and cell.bold and cell.bg == 'default', cell
            if not mono: assert cell.fg in ('cyan', '00cdcd'), cell
            os.write(master, b'\rn')
            wait(lambda: any('2/3' in line for line in screen.display) and any('n-e-e-d-l-e fuzzy' in line for line in screen.display))
            os.write(master, b'n')
            wait(lambda: any('3/3' in line for line in screen.display) and any('needle last' in line for line in screen.display))
            os.write(master, b'N')
            wait(lambda: any('2/3' in line for line in screen.display) and any('n-e-e-d-l-e fuzzy' in line for line in screen.display))
            os.write(master, b'\x1b')
            wait(lambda: not any('2/3' in line for line in screen.display))
            assert any('n-e-e-d-l-e fuzzy' in line for line in screen.display)
            os.write(master, b'\x1b')
            wait(lambda: any('sample.txt' in line for line in screen.display) and not any('n-e-e-d-l-e fuzzy' in line for line in screen.display))
            os.write(master, b'\x03'); proc.wait(timeout=5)
            assert proc.returncode == 0 and termios.tcgetattr(slave) == before
            checks.append(dict(mode='monochrome' if mono else 'ANSI palette', result='PASS', G_bottom=True, fuzzy_live_highlight=True, next_previous=True, escape_selection_and_termios=True))
        finally:
            if proc.poll() is None: proc.terminate(); proc.wait(timeout=5)
            os.close(master); os.close(slave)
print(json.dumps(dict(result='PASS', checks=checks)))
