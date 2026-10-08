#!/usr/bin/env python3
"""Discover the same managed/default terminals inside or outside managed tmux."""
import fcntl, json, os, pathlib, pty, select, struct, subprocess, sys, tempfile, termios, time
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
        # SSH native helper semantics: inherited managed environment must still
        # attach the advertised external terminal on the default server.
        external = next(r for r in rows if r['external'])
        master, slave = pty.openpty()
        modes = termios.tcgetattr(slave)
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 24, 80, 0, 0))
        def controlling_terminal():
            os.setsid(); fcntl.ioctl(slave, termios.TIOCSCTTY, 0)
        client = subprocess.Popen([binary,'native-attach','--external','attach-session','-t',external['id']],
            env=inside,stdin=slave,stdout=slave,stderr=slave,preexec_fn=controlling_terminal)
        try:
            deadline = time.monotonic()+5
            while time.monotonic()<deadline:
                if select.select([master],[],[],.05)[0]: os.read(master,65536)
                names = tmux('-L','default','list-clients','-F','#{session_name}').splitlines()
                if names==['external-fixture']: break
                if client.poll() is not None: raise AssertionError('external attach exited '+str(client.returncode))
            else: raise AssertionError('native helper did not enter default external fixture')
            assert not tmux('-S',socket,'list-clients','-F','#{session_name}')
            tmux('-L','default','detach-client','-t',os.ttyname(slave))
            client.wait(timeout=5)
            assert client.returncode==0 and termios.tcgetattr(slave)==modes
        finally:
            if client.poll() is None:client.terminate();client.wait(timeout=5)
            os.close(master);os.close(slave)
        print(json.dumps({'result':'PASS','checks':['inside/outside same discovery','managed sessions not duplicated as external','real default external retained','all identities and external session unchanged','native external attach selects default server and restores terminal']}))
    finally:
        subprocess.run(['tmux','-S',socket,'kill-server'],env=env,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
        subprocess.run(['tmux','-L','default','kill-server'],env=env,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
