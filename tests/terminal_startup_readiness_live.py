#!/usr/bin/env python3
"""Creation replies as soon as the terminal exists; an agent launch that fails keeps its pane.

The viewer attaches while the terminal starts, so creation never waits for a first
screen. A terminal whose launch fails (here: the launcher exits 7, or a provider the
recorded probe still lists is gone) stays on screen with its error and status for the
startup grace, unlisted, and tmux removes it afterwards (current tmux: 3.3 or later)."""
import json, os, pathlib, shlex, shutil, subprocess, sys, tempfile, time
binary = str(pathlib.Path(sys.argv[1]).resolve())
GRACE = 30
LAUNCHERS = {
    'silent': 'exec sleep 30\n',
    'exit': 'sleep 0.3\nexit 7\n',
    # An agent launch (`-l -i -c COMMAND`) where the provider is no longer installed.
    'gone': 'sleep 0.3\nshift 3\nPATH=/var/empty exec /bin/sh -c "$1"\n',
    # Ended before the reply could go: creation says so (or, if the reply won, the pane shows it).
    'instant': 'exit 7\n',
    # A shell that ends with an error is not a failed launch (the user's last command failed).
    'shell-exit': 'sleep 0.3\nexit 7\n',
}
checks = []
for case, body in LAUNCHERS.items():
    with tempfile.TemporaryDirectory(prefix='cx-readiness-') as tmp:
        root=pathlib.Path(tmp); state=root/'state/cx';state.mkdir(parents=True)
        launch=root/'bash';launch.write_text('#!/bin/sh\n'+body);launch.chmod(0o700)
        (state/'launcher.json').write_text(json.dumps(str(launch)))
        provider = 'shell'
        if case in ('exit', 'gone', 'instant'):
            provider = 'claude'
            # Info found claude moments ago; the launch trusts that and does not probe.
            (state/'providers.json').write_text(json.dumps(dict(shell=str(launch), at=int(time.time()), claude=True,
                                                                 codex=False, codex_no_daemon=False)))
            (state/'providers.json').chmod(0o600)
        env=dict(os.environ,HOME=tmp,XDG_STATE_HOME=str(root/'state'),SHELL=str(launch),TERM='xterm-256color')
        for key in ['TMUX','TMUX_PANE']:env.pop(key,None)
        # tmux 2.7 does not accept the attachment-style query. Creation must
        # not acquire that unrelated option dependency.
        local_bin=root/'.local/bin';local_bin.mkdir(parents=True)
        tmux=local_bin/'tmux'
        tmux.write_text('#!/bin/sh\ncase "$*" in *window-style*|*window-active-style*) echo "unknown option: window-style" >&2; exit 1;; esac\nexec '+shlex.quote(shutil.which('tmux'))+' "$@"\n')
        tmux.chmod(0o700)
        socket=str(state/'managed.sock')
        subprocess.run(['tmux','-S',socket,'-f','/dev/null','new-session','-d','-s','bootstrap','sleep 300'],env=dict(env,SHELL='/bin/sh'),check=True)
        def tmux_out(*args):
            return subprocess.run(['tmux','-S',socket,*args],env=env,capture_output=True,text=True)
        try:
            start=time.monotonic()
            result=subprocess.run([binary,'new','--provider',provider,'--directory',tmp],env=env,capture_output=True,text=True,timeout=12)
            elapsed=time.monotonic()-start
            if case=='instant' and result.returncode!=0:
                assert 'exited during startup (status 7)' in result.stderr,result.stderr[-500:]
                ids=tmux_out('list-sessions','-F','#{session_name}').stdout.split()
                assert ids==['bootstrap'],'the ended terminal was left behind'
                checks.append('a terminal that ends before the reply is reported with its status')
                continue
            assert result.returncode==0,(case,result.stderr[-500:])
            assert elapsed<1.5,(case,elapsed)
            session=json.loads(result.stdout)
            assert session['provider']==provider
            if case=='shell-exit':
                time.sleep(1)
                assert tmux_out('has-session','-t','='+session['id']).returncode!=0,'an ended shell stayed'
                checks.append('a shell that ends with an error closes as before')
                continue
            if case=='silent':
                listed=json.loads(subprocess.run([binary,'sessions'],env=env,capture_output=True,timeout=12).stdout or b'[]')
                assert session['id'] in [s['id'] for s in listed],'the new terminal is not listed'
                checks.append('creation replies at once for a silent terminal')
                continue
            deadline=time.monotonic()+5
            while tmux_out('display-message','-p','-t',session['id']+':','#{pane_dead}').stdout.strip()!='1':
                assert time.monotonic()<deadline,(case,'pane did not end')
                time.sleep(.05)
            time.sleep(.2)
            screen=tmux_out('capture-pane','-p','-t',session['id']+':').stdout
            status='127' if case=='gone' else '7'
            # (tmux may drop a dying pane's last output, the shell's own message: cx says it.)
            assert f'cx: the launch failed (exit status {status}' in screen,(case,screen[-400:])
            if case=='gone':
                assert 'exit status 127: command not found' in screen,screen[-400:]
            listed=json.loads(subprocess.run([binary,'sessions'],env=env,capture_output=True,timeout=12).stdout or b'[]')
            assert session['id'] not in [s['id'] for s in listed],(case,'a failed launch is listed')
            if case=='instant':
                checks.append('a terminal that ends right after the reply keeps its pane')
            elif case=='exit':
                time.sleep(max(0,GRACE+1.5-(time.monotonic()-start)))
                assert tmux_out('has-session','-t','='+session['id']).returncode!=0,'failed pane outlived the grace'
                assert tmux_out('has-session','-t','=bootstrap').returncode==0,'another session was touched'
                checks.append('a failed launch keeps its pane (error, status) for the grace, then goes')
            else:
                checks.append('a provider gone since the recorded probe fails visibly in its pane')
        finally:
            subprocess.run(['tmux','-S',socket,'kill-server'],env=env,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
checks.append('shell creation needs no attachment-style options')
print(json.dumps({'result':'PASS','checks':checks}))
