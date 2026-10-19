#!/usr/bin/env python3
"""First-screen wait is bounded for silent terminals and reports startup exits."""
import json, os, pathlib, subprocess, sys, tempfile, time
binary = str(pathlib.Path(sys.argv[1]).resolve())
for silent in [True, False]:
    with tempfile.TemporaryDirectory(prefix='cx-readiness-') as tmp:
        root=pathlib.Path(tmp); state=root/'state/cx';state.mkdir(parents=True)
        launch=root/'bash';launch.write_text('#!/bin/sh\n'+('exec sleep 30\n' if silent else 'exit 7\n'));launch.chmod(0o700)
        (state/'launcher.json').write_text(json.dumps(str(launch)))
        env=dict(os.environ,HOME=tmp,XDG_STATE_HOME=str(root/'state'),SHELL=str(launch),TERM='xterm-256color')
        for key in ['TMUX','TMUX_PANE']:env.pop(key,None)
        socket=str(state/'managed.sock')
        subprocess.run(['tmux','-S',socket,'-f','/dev/null','new-session','-d','-s','bootstrap','sleep 30'],env=dict(env,SHELL='/bin/sh'),check=True)
        try:
            start=time.monotonic()
            result=subprocess.run([binary,'new','--directory',tmp],env=env,capture_output=True,text=True,timeout=12)
            elapsed=time.monotonic()-start
            if silent:
                assert result.returncode==0,result.stderr
                assert 4.8 <= elapsed < 10,elapsed
                assert json.loads(result.stdout)['provider']=='shell'
            else:
                assert result.returncode!=0,'startup exit was reported as success'
                assert 'startup' in result.stderr,result.stderr
                assert elapsed<4,elapsed
        finally:
            subprocess.run(['tmux','-S',socket,'kill-server'],env=env,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
print(json.dumps({'result':'PASS','checks':['silent terminal remains attachable after bounded wait','startup exit reports an error without attachment']}))
