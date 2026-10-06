#!/usr/bin/env python3
"""Real PTY fixture navigation, no agents, transfers or host mutation.
Run: uv run --with pyte python tests/workspace_flows_pty.py /absolute/cx [evidence-dir]
"""
import fcntl
import json
import os
import pathlib
import pty
import select
import struct
import subprocess
import sys
import tempfile
import termios
import time
import pyte

binary = str(pathlib.Path(sys.argv[1]).resolve())
evidence = pathlib.Path(sys.argv[2] if len(sys.argv) > 2 else '/tmp/cx-workspace-flows-evidence')
evidence.mkdir(parents=True, exist_ok=True)
results = []
for rows, cols in [(24, 80), (40, 120), (24, 48)]:
    with tempfile.TemporaryDirectory(prefix='cx-ui-flows-') as root:
        home = pathlib.Path(root)
        (home / 'source.bin').write_bytes(b'synthetic file preview\n')
        (home / 'destination').mkdir()
        master, slave = pty.openpty()
        before = termios.tcgetattr(slave)
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', rows, cols, 0, 0))
        env = dict(os.environ, HOME=root, XDG_STATE_HOME=str(home/'state'), TERM='xterm-256color')
        env.pop('TMUX', None)
        env.pop('TMUX_PANE', None)
        def control():
            os.setsid()
            fcntl.ioctl(slave, termios.TIOCSCTTY, 0)
        proc = subprocess.Popen([binary], stdin=slave, stdout=slave, stderr=slave, env=env, cwd=root, preexec_fn=control)
        screen = pyte.Screen(cols, rows)
        stream = pyte.Stream(screen)
        def read(duration=.35):
            end = time.monotonic() + duration
            while time.monotonic() < end:
                if select.select([master], [], [], .02)[0]:
                    try:
                        stream.feed(os.read(master, 65536).decode(errors='replace'))
                    except OSError:
                        break
        def send(keys):
            os.write(master, keys)
            read()
        def text():
            return '\n'.join(screen.display)
        def palette(query):
            send(b'\x10')
            send(query.encode())
            send(b'\r')
        try:
            read(.7)
            palette('New session')
            assert 'execution device' in text(), text()
            send(b'\r')
            assert 'ordinary terminal' in text(), text()
            send(b'jj\r')
            assert 'Start codex' in text(), text()
            # Enter remains preview, never Start here.
            send(b'/source.bin')
            send(b'\r')
            assert 'synthetic file preview' in text(), text()
            send(b'\x1b')
            palette('Copy selected')
            assert 'destination device' in text(), text()
            send(b'\r')
            assert 'Source' in text() and 'Destination' in text(), text()
            send(b'/destination')
            send(b'\r')
            assert '/destination' in text(), text()
            (evidence / f'panes-{cols}x{rows}.txt').write_text(text())
            # BackTab activates source and Tab returns to destination.
            send(b'\x1b[Z')
            send(b'\t')
            palette('Existing files')
            assert 'overwrite' in text(), text()
            # Inspect jobs without starting a copy worker.
            palette('Transfers')
            assert 'No transfers yet' in text(), text()
            send(b'\x03')
            proc.wait(timeout=3)
            assert termios.tcgetattr(slave) == before, 'terminal not restored'
            results.append({'size': f'{cols}x{rows}', 'result': 'PASS', 'scenario': 'All → New → Codex directory browser → preview → two panes → destination → conflict → jobs → restore'})
        finally:
            if proc.poll() is None:
                proc.terminate()
                proc.wait(timeout=3)
            os.close(master)
            os.close(slave)
(evidence / 'results.json').write_text(json.dumps(results, indent=2))
print(json.dumps(results))
