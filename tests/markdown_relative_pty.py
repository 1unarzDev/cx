#!/usr/bin/env python3
"""Relative Markdown links, exact mouse targets, anchors and preview return."""
import fcntl, json, os, pathlib, pty, select, struct, subprocess, sys, tempfile, termios, time
import pyte
binary = str(pathlib.Path(sys.argv[1]).resolve())
checks = []
for mono in (False, True):
    with tempfile.TemporaryDirectory(prefix='cx-source-preview-') as temporary:
        root = pathlib.Path(temporary); source = root/'files'; source.mkdir()
        script = source/'sample.md'
        script.write_text('# Parent\n[same](one.md) [same](two%20file.md#section)\n\nParent sentinel\n\n| Link | Purpose |\n| --- | --- |\n| [table](one.md) | test |\n')
        (source/'one.md').write_text('# One\nFirst destination\n')
        (source/'two file.md').write_text('# Intro\nSecond destination\n' + '\n'.join('filler %d' % i for i in range(35)) + '\n# Section\nAnchor destination\n')
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
            settle(); os.write(master, b'\r')
            wait(lambda: any('Parent sentinel' in line for line in screen.display)); settle()
            row = next(i for i,line in enumerate(screen.display) if 'same' in line)
            column = screen.display[row].index('same', screen.display[row].index('same') + 4)
            cell = screen.buffer[row][column]
            assert cell.underscore and cell.bg == 'default', cell
            os.write(master, f'\x1b[<0;{column+1};{row+1}M'.encode()); settle()
            wait(lambda: any('two file.md' in line for line in screen.display))
            os.write(master,b'\x1b'); settle()
            wait(lambda: any('Parent sentinel' in line for line in screen.display)); settle()
            # Shift modifier forwarded by this PTY opens the exact second target.
            os.write(master, f'\x1b[<4;{column+1};{row+1}M'.encode()); settle()
            wait(lambda: any('Anchor destination' in line for line in screen.display)); settle()
            assert not any('First destination' in line for line in screen.display)
            os.write(master,b'\x1b'); settle()
            wait(lambda: any('Parent sentinel' in line for line in screen.display)); settle()
            row = next(i for i,line in enumerate(screen.display) if 'same' in line)
            column = screen.display[row].index('same')
            os.write(master, f'\x1b[<16;{column+1};{row+1}M'.encode()); settle()
            wait(lambda: any('First destination' in line for line in screen.display)); settle()
            os.write(master,b'\x1b'); settle()
            wait(lambda: any('Parent sentinel' in line for line in screen.display)); settle()
            # Links inside the formatted table preserve exact mouse destinations.
            row = next(i for i,line in enumerate(screen.display) if 'table' in line)
            column = screen.display[row].index('table')
            os.write(master, f'\x1b[<16;{column+1};{row+1}M'.encode()); settle()
            wait(lambda: any('First destination' in line for line in screen.display)); settle()
            os.write(master,b'\x1b'); settle()
            wait(lambda: any('Parent sentinel' in line for line in screen.display)); settle()
            # Leaving linked preview via folder navigation discards link history.
            row = next(i for i,line in enumerate(screen.display) if 'same' in line)
            column = screen.display[row].index('same')
            os.write(master, f'\x1b[<16;{column+1};{row+1}M'.encode()); settle()
            wait(lambda: any('First destination' in line for line in screen.display)); settle()
            os.write(master,b'h'); settle()
            wait(lambda: any('files' in line for line in screen.display) and not any('First destination' in line for line in screen.display)); settle()
            os.write(master,b'l'); settle()
            wait(lambda: any('sample.md' in line for line in screen.display)); settle()
            os.write(master,b'\r'); settle()
            wait(lambda: any('Parent sentinel' in line for line in screen.display)); settle()
            os.write(master,b'\x1b'); settle()
            wait(lambda: any('sample.md' in line for line in screen.display) and not any('Parent sentinel' in line for line in screen.display))
            os.write(master, b'\x03'); proc.wait(timeout=5)
            assert proc.returncode == 0 and termios.tcgetattr(slave) == before
            checks.append(dict(mode='monochrome' if mono else 'ANSI palette', result='PASS', exact_duplicate_label=True, shift_and_ctrl_click=True, heading_anchor=True, table_link=True, escape_parent_and_termios=True, stale_history_cleared=True))
        finally:
            if proc.poll() is None: proc.terminate(); proc.wait(timeout=5)
            os.close(master); os.close(slave)
print(json.dumps(dict(result='PASS', checks=checks)))
