#!/usr/bin/env python3
"""Owned tmux fixture: window_activity tracks output, expires, ignores silent input."""
import json,os,pathlib,shutil,subprocess,sys,tempfile,time
binary=str(pathlib.Path(sys.argv[1]).resolve())
with tempfile.TemporaryDirectory(prefix='cx-output-') as t:
 root=pathlib.Path(t);env=dict(os.environ,HOME=t,XDG_STATE_HOME=t+'/state',SHELL='/bin/bash',TERM='xterm-256color')
 env.pop('TMUX',None);env.pop('TMUX_PANE',None)
 socket=t+'/state/cx/managed.sock'
 def tm(*args):return subprocess.check_output(['tmux','-S',socket,*args],env=env,text=True,stderr=subprocess.PIPE)
 def rows():return json.loads(subprocess.check_output([binary,'sessions'],env=env))
 def current():return next(s for s in rows() if s['id']==sid)
 def timestamp():return int(tm('display-message','-p','-t',sid,'#{window_activity}').strip())
 try:
  s=json.loads(subprocess.check_output([binary,'new','--provider','shell','--directory',t,'--key','output'],env=env));sid=s['id']
  shutil.copy2(shutil.which('bash'),root/'codex')
  tm('send-keys','-t',sid,'-l',str(root/'codex')+" -c 'stty -echo; printf CX_SYNTHETIC_OUTPUT; sleep 30; :'")
  tm('send-keys','-t',sid,'Enter')
  end=time.monotonic()+5
  while current()['provider']!='codex':
   assert time.monotonic()<end,'provider not observed';time.sleep(.04)
  old=timestamp()
  assert current().get('activity')=='output','recent output missing'
  end=time.monotonic()+5
  while current().get('activity') is not None:
   assert time.monotonic()<end,'output indicator failed to expire';time.sleep(.1)
  tm('send-keys','-t',sid,'-l','silent input')
  time.sleep(.15)
  assert timestamp()==old,'silent input unexpectedly changed window_activity'
  assert current().get('activity') is None,'silent input advertised as output'
  # Write fixed synthetic output directly to this fixture-owned PTY.
  with open(f'/proc/{s["pid"]}/fd/1','wb',buffering=0) as pane: pane.write(b'CX_NEW_SYNTHETIC_OUTPUT\r\n')
  end=time.monotonic()+3
  while timestamp()==old:
   assert time.monotonic()<end,'PTY output did not update window_activity';time.sleep(.04)
  assert current().get('activity')=='output','new output not advertised'
  print(json.dumps({'result':'PASS','checks':['window_activity is terminal-output evidence','silent input does not advance activity','output indicator expires','new output reactivates indicator']}))
 finally:subprocess.run(['tmux','-S',socket,'kill-server'],env=env,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
