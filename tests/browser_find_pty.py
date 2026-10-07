#!/usr/bin/env python3
"""Real file-find PTYs: preserve rows, highlight names and navigate matches."""
import fcntl, json, os, pathlib, pty, select, struct, subprocess, sys, tempfile, termios, time
import pyte

binary = str(pathlib.Path(sys.argv[1]).resolve())
evidence = pathlib.Path('local-evidence/browser-find')
evidence.mkdir(parents=True, exist_ok=True)
results = []
for width, height in [(120, 40), (80, 24), (48, 24)]:
    with tempfile.TemporaryDirectory(prefix='cx-browser-') as temporary:
        root = pathlib.Path(temporary)
        source = root / 'source'; source.mkdir()
        destination = root / 'destination'; destination.mkdir()
        (source / 'alpha.txt').write_bytes(b'alpha fixture')
        (source / 'beta.txt').write_bytes(b'beta fixture')
        (source / '.hidden').write_bytes(b'hidden fixture')
        (source / 'directory').mkdir()
        (source / 'evil\x1b[31m.txt').write_bytes(b'hostile name')
        state = root / 'state/cx'; state.mkdir(parents=True)
        name = 'viewer-restart-123-789.json'
        snapshot = dict(schema=1, expires_at=int(time.time())+300, device_ids=['local'], device=1,
            focus='Workspace', view='Files', selected=0, selected_session=None, side_selected=0,
            search='', browser=dict(device=0, path=str(source), display_path=str(source), parent=str(root),
                entries=[], selected=0, search='', preview_scroll=0, restore_selection=None),
            other_browser=None, destination_active=False, conflict=2, launch_provider=None, clipboard=None, submitted={})
        snapshot_path = state / name; snapshot_path.write_text(json.dumps(snapshot)); snapshot_path.chmod(0o600)
        env = dict(os.environ, HOME=str(root), XDG_STATE_HOME=str(root/'state'), SHELL='/bin/sh', TERM='xterm-256color', HTTPS_PROXY='http://127.0.0.1:9')
        env.pop('TMUX', None); env.pop('TMUX_PANE', None)
        env.pop('NO_COLOR', None); env.pop('CX_ASCII', None)
        if width == 48:
            env['NO_COLOR'] = '1'; env['CX_ASCII'] = '1'
        master, slave = pty.openpty(); before = termios.tcgetattr(slave)
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', height, width, 0, 0))
        def control():
            os.setsid(); fcntl.ioctl(slave, termios.TIOCSCTTY, 0)
        proc = subprocess.Popen([binary, 'restart', name], stdin=slave, stdout=slave, stderr=slave, env=env, preexec_fn=control)
        screen = pyte.Screen(width, height); stream = pyte.Stream(screen)
        def read(duration=.2):
            end = time.monotonic()+duration
            while time.monotonic()<end:
                if select.select([master], [], [], .02)[0]:
                    try: stream.feed(os.read(master, 65536).decode(errors='replace'))
                    except OSError: break
        def send(keys):
            os.write(master, keys); read()
        def text(): return '\n'.join(screen.display)
        def wait_for(predicate, seconds=8):
            deadline=time.monotonic()+seconds
            while not predicate() and time.monotonic()<deadline: read(.1)
            assert predicate(), text()
        def capture(label):
            (evidence/f'{width}-{label}.txt').write_text(text())
            cells = [[dict(data=screen.buffer[y][x].data, fg=screen.buffer[y][x].fg, bg=screen.buffer[y][x].bg,
                bold=screen.buffer[y][x].bold, reverse=screen.buffer[y][x].reverse) for x in range(width)] for y in range(height)]
            (evidence/f'{width}-{label}.json').write_text(json.dumps(cells))
        def filter_name(value): send(b'f\x15'+value.encode()+b'\r')
        try:
            wait_for(lambda: 'alpha.txt' in text() and 'beta.txt' in text())
            send(b'/alpha')
            wait_for(lambda: 'n/N matches' in text())
            assert 'beta.txt' in text() and 'directory' in text(), text()
            row=next(y for y,line in enumerate(screen.display) if 'alpha.txt' in line)
            col=screen.display[row].index('alpha.txt')
            assert screen.buffer[row][col].underscore, text()
            assert screen.buffer[row][col].bg=='default'
            send(b'\r'); assert 'alpha fixture' not in text()
            send(b'nN'); assert 'alpha fixture' not in text()
            send(b'/\x15a\r')
            wait_for(lambda: 'n/N matches' in text())
            capture('find')
            def selected_name():
                for line in screen.display:
                    if ('▸ ' in line or '> ' in line) and any(name in line for name in ['alpha.txt','beta.txt','directory']):
                        return next(name for name in ['alpha.txt','beta.txt','directory'] if name in line)
                return None
            first=selected_name(); assert first, text()
            send(b'n'); assert selected_name() and selected_name()!=first, text(); capture('next')
            send(b'N'); assert selected_name()==first, text(); capture('previous')
            send(b'/\x15missing\r')
            wait_for(lambda: '0/0' in text())
            assert 'alpha.txt' in text() and 'beta.txt' in text(), text()
            before_no_match=text(); send(b'n'); assert text()==before_no_match, text()
            send(b'\x1b'); wait_for(lambda: 'n/N matches' not in text())
            filter_name('beta'); assert 'alpha.txt' not in text(), text()
            send(b'\x03'); proc.wait(timeout=5)
            assert proc.returncode == 0 and termios.tcgetattr(slave)==before
            results.append(dict(size=f'{width}x{height}', result='PASS', scenarios='live non-filtering highlights, Enter finishes, next/previous, no-match, filter separate, termios'))
        finally:
            if proc.poll() is None: proc.terminate(); proc.wait(timeout=4)
            os.close(master); os.close(slave)
(evidence/'results.json').write_text(json.dumps(results, indent=2))
print(json.dumps(results))
