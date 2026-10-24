#!/usr/bin/env python3
"""Verify Claude preserves terminal transparency in private tmux."""
import fcntl
import json
import os
from pathlib import Path
import pty
import select
import shlex
import shutil
import subprocess
import sys
import tempfile
import termios
import time

binary = str(Path(sys.argv[1]).resolve())
with tempfile.TemporaryDirectory(prefix='cx-claude-contrast-') as tmp:
    root = Path(tmp)
    env = dict(os.environ, HOME=tmp, XDG_STATE_HOME=tmp+'/state',
               SHELL='/bin/bash', TERM='xterm-256color')
    for key in ['TMUX', 'TMUX_PANE', 'CLAUDE_CONFIG_DIR']:
        env.pop(key, None)
    socket = tmp+'/state/cx/managed.sock'
    def tmux(*args):
        return subprocess.check_output(['tmux', '-S', socket, *args], env=env, text=True)
    def attach(sid):
        master, slave = pty.openpty()
        def tty():
            os.setsid()
            fcntl.ioctl(slave, termios.TIOCSCTTY, 0)
        client = subprocess.Popen([binary, 'native-attach', 'attach-session', '-t', sid],
            env=env, stdin=slave, stdout=slave, stderr=slave, preexec_fn=tty)
        try:
            deadline = time.monotonic()+5
            while not tmux('list-clients', '-F', '#{client_pid}').strip():
                assert client.poll() is None, 'native attach failed'
                assert time.monotonic()<deadline, 'native attach timed out'
                time.sleep(.02)
            return tmux('show-options', '-w', '-A', '-t', sid, '-v', 'window-style').strip()
        finally:
            if client.poll() is None:
                tmux('detach-client', '-t', os.ttyname(slave))
            client.wait(timeout=5)
            os.close(master)
            os.close(slave)
    try:
        session = json.loads(subprocess.check_output([binary, 'new', '--directory', tmp,
            '--key', 'contrast'], env=env))
        sid = session['id']
        baseline = attach(sid)
        shutil.copy2(shutil.which('bash'), root/'claude')
        tmux('send-keys', '-t', sid, '-l', shlex.join([str(root/'claude'), '-c', 'sleep 60; :']))
        tmux('send-keys', '-t', sid, 'Enter')
        deadline = time.monotonic()+5
        while next(s for s in json.loads(subprocess.check_output([binary, 'sessions'], env=env)) if s['id']==sid)['provider']!='claude':
            assert time.monotonic()<deadline
            time.sleep(.02)
        for theme in ['dark', 'light']:
            (root/'.claude.json').write_text(json.dumps({'theme': theme}))
            # Reproduce a window left opaque by older CX versions.
            for option in ['window-style', 'window-active-style']:
                tmux('set-option', '-w', '-t', sid, option, 'bg=#181818')
            style = attach(sid)
            assert style == baseline, 'Claude changed terminal background: '+style
            active = tmux('show-options', '-w', '-A', '-t', sid, '-v', 'window-active-style').strip()
            assert '#181818' not in active and '#f5f5f5' not in active, active
            assert 'fg=terminal' in style or 'fg=default' in style, 'existing text palette changed'
        tmux('send-keys', '-t', sid, 'C-c')
        time.sleep(.1)
        assert attach(sid)==baseline, 'shell did not restore original terminal style'
        print(json.dumps({'result': 'PASS', 'checks': ['Claude dark/light preserve terminal background', 'legacy opaque styles cleared',
            'existing foreground palette preserved', 'shell original background restored']}))
    finally:
        subprocess.run(['tmux', '-S', socket, 'kill-server'], env=env,
            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
