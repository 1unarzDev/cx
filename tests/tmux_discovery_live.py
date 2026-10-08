#!/usr/bin/env python3
"""Discover the same managed/default terminals inside or outside managed tmux."""
import json, os, pathlib, subprocess, sys, tempfile
binary = str(pathlib.Path(sys.argv[1]).resolve())
with tempfile.TemporaryDirectory(prefix='cx-discovery-') as temporary:
    root = pathlib.Path(temporary)
    env = dict(os.environ, HOME=str(root), XDG_STATE_HOME=str(root/'state'),
               TMUX_TMPDIR=str(root), SHELL='/bin/sh', TERM='xterm-256color')
    env.pop('TMUX', None); env.pop('TMUX_PANE', None)
    socket = str(root/'state/cx/managed.sock')
    def tmux(*args):
        return subprocess.check_output(['tmux', *args], env=env, text=True).strip()
    try:
        original = json.loads(subprocess.check_output([binary, 'new', '--directory', str(root), '--key', 'discovery'], env=env))
        tmux('-L', 'default', 'new-session', '-d', '-s', 'external-fixture', '/bin/sh')
        default_socket = tmux('-L', 'default', 'display-message', '-p', '#{socket_path}')
        before = tmux('-L', 'default', 'list-sessions', '-F', '#{session_id}:#{session_name}:#{session_created}')
        baseline = json.loads(subprocess.check_output([binary, 'sessions'], env=env))
        inside = dict(env, TMUX=socket+',123,0', TMUX_PANE='%0')
        nested = json.loads(subprocess.check_output([binary, 'sessions'], env=inside))
        assert [(r['id'],r['external'],r['pid']) for r in nested] == [(r['id'],r['external'],r['pid']) for r in baseline], {'outside':[(r['id'],r['external'],r['pid']) for r in baseline],'inside':[(r['id'],r['external'],r['pid']) for r in nested]}
        assert len(nested)==2 and sum(r['external'] for r in nested)==1
        assert next(r for r in nested if r['external'])['name']=='external-fixture'
        found = next(r for r in nested if not r['external'])
        assert all(found[k]==original[k] for k in ['id','pid','started','boot_id','socket'])
        assert tmux('-L','default','list-sessions','-F','#{session_id}:#{session_name}:#{session_created}')==before
        # Even the inherited external socket yields the same execution identities.
        inherited = dict(env, TMUX=default_socket+',123,0', TMUX_PANE='%0')
        rows = json.loads(subprocess.check_output([binary,'sessions'],env=inherited))
        assert [(r['id'],r['external'],r['pid']) for r in rows]==[(r['id'],r['external'],r['pid']) for r in baseline]
        print(json.dumps({'result':'PASS','checks':['inside/outside same discovery','managed sessions not duplicated as external','real default external retained','all identities and external session unchanged']}))
    finally:
        subprocess.run(['tmux','-S',socket,'kill-server'],env=env,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
        subprocess.run(['tmux','-L','default','kill-server'],env=env,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
