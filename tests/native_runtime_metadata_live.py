#!/usr/bin/env python3
"""Synthetic native metadata, real processes/tmux: switching projects without restart."""
import json,os,pathlib,shlex,shutil,sqlite3,subprocess,sys,tempfile,time
binary=str(pathlib.Path(sys.argv[1]).resolve())
ID='11111111-2222-4333-8444-555555555555'
CHILD='aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee'
with tempfile.TemporaryDirectory(prefix='cx-native-metadata-') as t:
 root=pathlib.Path(t);first=root/'first';second=root/'second';first.mkdir();second.mkdir()
 env=dict(os.environ,HOME=t,XDG_STATE_HOME=t+'/state',SHELL='/bin/bash',TERM='xterm-256color')
 env.pop('TMUX',None);env.pop('TMUX_PANE',None)
 socket=t+'/state/cx/managed.sock'
 def tm(*args):return subprocess.check_output(['tmux','-S',socket,*args],env=env,text=True,stderr=subprocess.PIPE)
 def rows():return json.loads(subprocess.check_output([binary,'sessions'],env=env))
 def current():return next(s for s in rows() if s['id']==sid)
 def send(command):tm('send-keys','-t',sid,'-l',command);tm('send-keys','-t',sid,'Enter')
 def expect(check,message):
  end=time.monotonic()+5
  while time.monotonic()<end:
   s=current()
   if check(s):return s
   time.sleep(.04)
  raise AssertionError(message)
 try:
  s=json.loads(subprocess.check_output([binary,'new','--provider','shell','--directory',str(first),'--key','metadata'],env=env));sid=s['id']
  for provider in ['claude','codex']:
   shutil.copy2(shutil.which('bash'),root/provider)
   if provider=='claude':
    send(shlex.join([str(root/provider),'-c','sleep 30; :']))
    row=expect(lambda s:s['provider']=='claude','Claude process was not detected')
    pid=row['process']['pid'];start=row['process']['start_ticks']
    folder=root/'.claude/sessions';folder.mkdir(parents=True)
    record=folder/f'{pid}.json'
    def write(path,status,name):record.write_text(json.dumps({'pid':pid,'procStart':start,'sessionId':ID,'cwd':str(path),'name':name,'status':status}))
    write(first,'working','First project')
    expect(lambda s:s['directory']==str(first) and s.get('activity')=='running' and s['name']=='First project','Claude native metadata unavailable')
    write(second,'waiting_for_input','Second project')
    row=expect(lambda s:s['directory']==str(second) and s.get('activity')=='waiting' and s['name']=='Second project','Claude control-panel project switch kept stale workspace/title')
    record.unlink()
    expect(lambda s:s.get('activity') is None and s['name']!='Second project','removed native metadata remained cached')
   else:
    home=root/'.codex';rollout=home/'sessions/2026/10/19'/f'rollout-fixture-{ID}.jsonl';rollout.parent.mkdir(parents=True);rollout.touch()
    db=sqlite3.connect(home/'state_5.sqlite')
    db.executescript('CREATE TABLE threads(id TEXT,name TEXT,cwd TEXT,agent_nickname TEXT,agent_role TEXT);CREATE TABLE thread_spawn_edges(parent_thread_id TEXT,child_thread_id TEXT,status TEXT);')
    db.execute('INSERT INTO threads VALUES(?,?,?,?,?)',(ID,'First native project',str(first),None,None));db.execute('INSERT INTO threads VALUES(?,?,?,?,?)',(CHILD,'Review worker',str(first),'Reviewer','review'))
    db.execute('INSERT INTO thread_spawn_edges VALUES(?,?,?)',(ID,CHILD,'running'));db.commit()
    send(shlex.join([str(root/provider),'-c',f'exec 3>{shlex.quote(str(rollout))}; sleep 30; :']))
    expect(lambda s:s['provider']=='codex' and s['directory']==str(first) and len(s.get('agent_tasks',[]))==1,'Codex exact writer metadata unavailable')
    db.execute('UPDATE threads SET cwd=?,name=? WHERE id=?',(str(second),'Second native project',ID));db.commit()
    row=expect(lambda s:s['directory']==str(second) and s['name']=='Second native project','Codex project switch kept stale launch directory/title')
    assert row.get('activity') is None,'provider presence incorrectly marked as working'
    assert row['agent_tasks'][0]['id']==CHILD
    db.close()
   for key in ['pid','started','boot_id','socket']:assert row[key]==s[key],key+' changed during native switch'
   tm('send-keys','-t',sid,'C-c');expect(lambda s:s['provider']=='shell','provider did not return to shell')
  print(json.dumps({'result':'PASS','checks':['Claude project/name/activity switch without process restart','unknown native status clears prior metadata','Codex exact writer project/name switch without process restart','structured Codex child task metadata','provider presence does not invent busy state']}))
 finally:
  subprocess.run(['tmux','-S',socket,'kill-server'],env=env,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
