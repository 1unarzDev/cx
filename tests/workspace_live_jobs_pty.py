#!/usr/bin/env python3
"""Real disposable PTY copy and shell-start integration, no agent prompts.
Run: uv run --with pyte python tests/workspace_live_jobs_pty.py /absolute/cx [evidence-dir]
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
for rows, cols in [(40, 120)]:
    with tempfile.TemporaryDirectory(prefix='cx-ui-flows-') as root:
        home = pathlib.Path(root)
        (home / 'source.bin').write_bytes(b'synthetic file preview\n')
        (home / 'destination').mkdir()
        (home / 'destination/source.bin').write_bytes(b'original destination\n')
        master, slave = pty.openpty()
        before = termios.tcgetattr(slave)
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', rows, cols, 0, 0))
        env = dict(os.environ, HOME=root, XDG_STATE_HOME=str(home/'state'), TERM='xterm-256color', SHELL='/bin/sh')
        env.pop('TMUX', None)
        env.pop('TMUX_PANE', None)
        def control():
            os.setsid()
            fcntl.ioctl(slave, termios.TIOCSCTTY, 0)
        proc = subprocess.Popen([binary], stdin=slave, stdout=slave, stderr=slave, env=env, cwd=root, preexec_fn=control)
        class Screen(pyte.Screen):
            def report_device_status(self, mode, **kwargs):
                # pyte does not model private DSR; this harness checks display/state.
                if not kwargs.get('private'):
                    return super().report_device_status(mode)
        screen = Screen(cols, rows)
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
            send(b'\r')
            assert 'Start shell' in text(), text()
            deadline = time.monotonic() + 10
            while 'source.bin' not in text() and time.monotonic() < deadline:
                read(.1)
            assert 'source.bin' in text(), 'listing did not arrive: '+text()
            # Enter remains preview, never Start here.
            send(b'/source.bin')
            send(b'\r')
            assert 'synthetic file preview' in text(), text()
            send(b'\x1b')
            send(b'y')
            assert 'destination device' not in text()
            send(b't')
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
            send(b'p')
            target = home/'destination/source.bin.copy-1'
            deadline = time.monotonic()+8
            while not target.exists() and time.monotonic()<deadline:
                read(.2)
            assert target.exists(), 'UI copy did not produce file'
            assert target.read_bytes() == (home/'source.bin').read_bytes()
            assert (home/'destination/source.bin').read_bytes() == b'original destination\n'
            read(2.5)
            assert 'complete' in text(), 'Completed copy missing from drawer: '+text()
            # Paste stays in Files; its nonmodal drawer needs no Escape.
            palette('New session')
            assert 'Enter starts here:' in text(), 'Files launch should use current folder: '+text()
            send(b'\r')
            read(1)
            assert 'Return to cx' in text(), 'shell did not auto-attach from Files actions'
            directory = subprocess.run(['tmux','-S',str(home/'state/cx/managed.sock'),'display-message','-p','#{pane_current_path}'],capture_output=True,text=True,check=True).stdout.strip()
            assert directory == str(home/'destination'), ('wrong launch directory', directory)
            send(b'\x1d')
            assert 'Files' in text() or 'Start shell' in text(), 'browser state did not return'

            send(b'\x03')
            proc.wait(timeout=3)
            assert termios.tcgetattr(slave) == before, 'terminal not restored'
            results.append({'size': f'{cols}x{rows}', 'result': 'PASS', 'scenario': 'actual detached copy/integrity/job drawer and actual folder shell start/single-key return'})
        finally:
            if proc.poll() is None:
                os.write(master, b'\x1d')
                read(.2)
                os.write(master, b'\x03')
                try:
                    proc.wait(timeout=2)
                except subprocess.TimeoutExpired:
                    subprocess.run(['tmux','-S',str(home/'state/cx/managed.sock'),'kill-server'],capture_output=True)
                    proc.kill()
                    proc.wait(timeout=2)
            os.close(master)
            os.close(slave)
            subprocess.run(['tmux','-S',str(home/'state/cx/managed.sock'),'kill-server'],capture_output=True)

(evidence / 'results.json').write_text(json.dumps(results, indent=2))
print(json.dumps(results))
