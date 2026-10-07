#!/usr/bin/env python3
"""Real Fish/tmux command labels: short builtins, no arguments, same terminal."""
import os,pathlib,tempfile,subprocess,json,time,sys
binary=str(pathlib.Path(sys.argv[1]).resolve())
with tempfile.TemporaryDirectory(prefix='cx-fish-label-') as tmp:
 root=pathlib.Path(tmp);env=dict(os.environ,HOME=str(root),XDG_STATE_HOME=str(root/'state'),SHELL='/usr/bin/fish',TERM='xterm-256color');env.pop('TMUX',None);env.pop('TMUX_PANE',None)
 sock=str(root/'state/cx/managed.sock')
 try:
  s=json.loads(subprocess.check_output([binary,'new','--directory',tmp],env=env))
  time.sleep(.5)
  subprocess.check_call(['tmux','-S',sock,'send-keys','-t',s['id'],'echo synthetic-secret-label-test','Enter'],stdout=subprocess.DEVNULL)
  deadline=time.monotonic()+4
  while time.monotonic()<deadline:
   records=json.loads(subprocess.check_output([binary,'sessions'],env=env));found=next(x for x in records if x['id']==s['id'])
   if found['name']=='last: echo':break
   time.sleep(.1)
  assert found['name']=='last: echo',found['name'];assert 'synthetic-secret' not in json.dumps(records)
  assert all(found[k]==s[k] for k in ['id','pid','started','boot_id','socket'])
  print(json.dumps(dict(result='PASS',shell='Fish',last_builtin='echo',same_terminal_identity=True,arguments_excluded=True)))
 finally:subprocess.run(['tmux','-S',sock,'kill-server'],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
