#!/usr/bin/env python3
"""Exercise actual browser keys and detached jobs on disposable files in a real PTY."""
import fcntl, json, os, pathlib, pty, select, struct, subprocess, sys, tempfile, termios, time
import pyte

binary = str(pathlib.Path(sys.argv[1]).resolve())
evidence = pathlib.Path('local-evidence/browser-ecosystem')
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
            wait_for(lambda: 'alpha.txt' in text())
            assert '.hidden' not in text(), text()
            send(b'.'); wait_for(lambda: '.hidden' in text()); send(b'.')
            filter_name('alpha'); send(b' ')
            filter_name('beta'); send(b' ')
            assert '2 selected' in text(), text(); capture('selected')
            send(b'c'); assert 'COPY 2' in text(), text()
            assert 'destination device' not in text().lower()
            send(b'h'); filter_name('destination'); send(b'l')
            wait_for(lambda: str(destination)[-20:] in text())
            send(b'p')
            wait_for(lambda: (destination/'alpha.txt').exists() and (destination/'beta.txt').exists())
            assert (destination/'alpha.txt').read_bytes()==b'alpha fixture'
            assert (destination/'beta.txt').read_bytes()==b'beta fixture'
            assert 'Transfers · c cancel' not in text(), text()
            capture('copied')
            # Rename uses the hovered file and preserves other selection state.
            filter_name('alpha'); wait_for(lambda: 'alpha.txt' in text()); send(b'r'); capture('rename-panel')
            assert 'Rename · Enter' not in text() and 'Enter confirm' not in text(),text()
            send(b'\x15renamed.txt\r')
            wait_for(lambda: (destination/'renamed.txt').exists())
            filter_name('renamed'); send(b'd'); assert 'permanently?' in text()
            send(b'\r'); assert (destination/'renamed.txt').exists()  # Cancel default
            send(b'dj\r'); wait_for(lambda: not (destination/'renamed.txt').exists())
            filter_name('beta'); send(b'x')
            send(b'h'); filter_name('source'); send(b'l'); send(b'p')
            wait_for(lambda: not (destination/'beta.txt').exists() and (source/'beta.txt.copy-1').exists())
            assert (source/'beta.txt.copy-1').read_bytes()==b'beta fixture'
            send(b'f\x15\r'); send(b'vjj'); assert 'VISUAL' in text(); capture('visual')
            send(b'\x1b'); assert 'NORMAL' in text()
            send(b'd'); capture('delete-scope')
            assert 'No undo.' in text() and 'folders include all contents' in text(), text()
            assert 'Cancel' in text() and 'Copy / cut' not in text(), text()
            send(b'\r'); assert (source/'directory').is_dir()
            send(b'T'); capture('transfer-details')
            assert 'Route:' in text(), text()
            assert 'Copy / cut' not in text(), text()
            send(b'\x1b')
            send(b't'); capture('transfer-picker')
            assert 'destination device' in text(),text()
            assert 'Choose device' in text() and 'p Paste' in text() and 'here' in text(),text()
            send(b'\x1b')
            send(b'?'); capture('help-colored-keys')
            for row, line in enumerate(screen.display):
                if 'Arrows / h j k l' in line:
                    col=line.index('Arrows')
                    assert screen.buffer[row][col].bold
                    if width>48: assert screen.buffer[row][col].fg != 'default'
                    break
            else: raise AssertionError('navigation key row missing from help: '+text())
            send(b'\x1b')
            send(b'\x03'); proc.wait(timeout=4)
            assert termios.tcgetattr(slave)==before
            results.append(dict(size=f'{width}x{height}', result='PASS', scenarios='hidden, multiselect, copy/paste, rename, guarded delete, cut/rename conflict, range, termios'))
        finally:
            if proc.poll() is None: proc.terminate(); proc.wait(timeout=4)
            os.close(master); os.close(slave)
(evidence/'results.json').write_text(json.dumps(results, indent=2))
print(json.dumps(results))
