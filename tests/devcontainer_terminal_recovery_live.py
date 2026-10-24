#!/usr/bin/env python3
"""Rediscover a pre-existing Docker exec in private tmux; owned network-none container."""
import json
import os
from pathlib import Path
import shlex
import subprocess
import sys
import tempfile
import time

binary = str(Path(sys.argv[1]).resolve())
with tempfile.TemporaryDirectory(prefix='cx-terminal-recovery-') as tmp:
    env = dict(os.environ, HOME=tmp, XDG_STATE_HOME=tmp+'/state',
               SHELL='/bin/bash', TERM='xterm-256color')
    env.pop('TMUX', None)
    env.pop('TMUX_PANE', None)
    socket = tmp+'/state/cx/managed.sock'
    name = 'cx-recovery-fixture-'+str(os.getpid())
    container = subprocess.check_output(['docker', 'run', '-d', '--network', 'none',
        '--name', name, '--label', 'devcontainer.metadata=[{"remoteUser":"root"}]',
        'ubuntu:24.04', 'sleep', 'infinity'], text=True).strip()
    def tmux(*args):
        return subprocess.check_output(['tmux', '-S', socket, *args], env=env, text=True)
    def rows():
        return json.loads(subprocess.check_output([binary, 'sessions'], env=env, timeout=100))
    try:
        session = json.loads(subprocess.check_output([binary, 'new', '--directory', tmp,
            '--key', 'recovery'], env=env))
        sid = session['id']
        assert session.get('container') is None
        command = shlex.join(['docker', 'exec', '-it', name, '/bin/bash', '--noprofile', '--norc'])
        tmux('send-keys', '-t', sid, '-l', command)
        tmux('send-keys', '-t', sid, 'Enter')
        deadline = time.monotonic()+15
        while True:
            recovered = next(s for s in rows() if s['id']==sid)
            if recovered.get('container'):
                break
            assert time.monotonic()<deadline, 'existing Docker terminal was not recovered'
            time.sleep(.1)
        assert recovered['container']['id']==container
        assert recovered['container']['name']==name
        for key in ['id', 'pid', 'started', 'boot_id', 'socket']:
            assert recovered[key]==session[key], 'discovery replaced terminal identity: '+key
        # Each independent CLI invocation has no cached recovery state.
        assert next(s for s in rows() if s['id']==sid)['container']['id']==container
        subprocess.run(['docker', 'stop', '-t', '1', container], check=True,
                       stdout=subprocess.DEVNULL, timeout=10)
        deadline = time.monotonic()+10
        while True:
            current = next((s for s in rows() if s['id']==sid), None)
            if current is None or current.get('container') is None:
                break
            assert time.monotonic()<deadline, 'stopped container remained recoverable'
            time.sleep(.1)
        print(json.dumps({'result': 'PASS', 'checks': [
            'recover pre-existing Docker exec without scoped session record',
            'fresh process repeats discovery with original tmux identity',
            'stopped instance no longer advertised as recoverable']}))
    finally:
        subprocess.run(['tmux', '-S', socket, 'kill-server'], env=env,
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        subprocess.run(['docker', 'rm', '-f', container],
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
