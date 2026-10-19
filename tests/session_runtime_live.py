#!/usr/bin/env python3
"""Disposable tmux regression: dead cached shells and evidence-based activity."""
import json, os, pathlib, shlex, subprocess, sys, tempfile, time
binary = str(pathlib.Path(sys.argv[1]).resolve())
with tempfile.TemporaryDirectory(prefix='cx-runtime-') as tmp:
    root = pathlib.Path(tmp)
    env = dict(os.environ, HOME=tmp, XDG_STATE_HOME=tmp+'/state', SHELL='/bin/bash', TERM='xterm-256color')
    env.pop('TMUX', None); env.pop('TMUX_PANE', None)
    socket = root/'state/cx/managed.sock'
    def run(*args):
        return subprocess.check_output(args, env=env, text=True, stderr=subprocess.PIPE)
    def tmux(*args): return run('tmux', '-S', str(socket), *args)
    def rows(): return json.loads(run(binary, 'sessions'))
    try:
        session = json.loads(run(binary, 'new', '--provider', 'shell', '--directory', tmp, '--key', 'runtime-fixture'))
        sid = session['id']
        tmux('set-option', '-t', sid, 'remain-on-exit', 'on')
        tmux('send-keys', '-t', sid, '-l', 'exit')
        tmux('send-keys', '-t', sid, 'Enter')
        deadline = time.monotonic()+5
        while tmux('display-message','-p','-t',sid,'#{pane_dead}').strip() != '1':
            assert time.monotonic()<deadline, 'fixture shell failed to exit'
            time.sleep(.05)
        assert all(s['id'] != sid for s in rows()), 'dead retained tmux pane advertised as attachable shell'
        print(json.dumps({'result':'PASS','checks':['retained dead tmux pane excluded from attachable sessions']}))
    finally:
        if socket.exists(): subprocess.run(['tmux','-S',str(socket),'kill-server'], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
