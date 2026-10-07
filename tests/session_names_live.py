#!/usr/bin/env python3
"""Disposable real tmux sessions: shell labels and exact native metadata, no inference."""
import json, os, pathlib, shutil, subprocess, sys, tempfile, time
binary=str(pathlib.Path(sys.argv[1]).resolve())
results=[]
with tempfile.TemporaryDirectory(prefix='cx-native-names-') as tmp:
 root=pathlib.Path(tmp);bins=root/'bin';bins.mkdir();codex_home=root/'.codex';codex_home.mkdir()
 uuid='12345678-1234-1234-1234-123456789abc'
 locks=codex_home/'thread-writer-locks';locks.mkdir()
 lock=locks/(uuid+'.lock');lock.touch()
 second=locks/'12345678-1234-1234-1234-12345fffffff.lock';second.touch()
 index=codex_home/'session_index.jsonl';index.write_text(json.dumps(dict(id=uuid,thread_name='Camera timestamp fix'))+'\n')
 source=root/'agent.c';source.write_text(r'''#include <stdio.h>
#include <string.h>
#include <unistd.h>
#include <stdlib.h>
#include <fcntl.h>
#include <signal.h>
void another(int sig){open(CX_FIXTURE_SECOND_LOCK,O_RDWR);}
int main(int argc,char **argv) {
 if(argc>1 && !strcmp(argv[1],"--version")){puts("codex-cli 0.160.1");return 0;}
 open(CX_FIXTURE_LOCK,O_RDWR);
 signal(SIGUSR1,another);
 for(;;)pause();
}''')
 subprocess.check_call(['cc','-DCX_FIXTURE_LOCK='+json.dumps(str(lock)),'-DCX_FIXTURE_SECOND_LOCK='+json.dumps(str(second)),str(source),'-o',str(bins/'codex')])
 env=dict(os.environ,HOME=tmp,XDG_STATE_HOME=str(root/'state'),SHELL='/bin/bash',TERM='xterm-256color',PATH=str(bins)+':'+os.environ['PATH'],CX_FIXTURE_LOCK=str(lock),CX_FIXTURE_SECOND_LOCK=str(second))
 env.pop('TMUX',None);env.pop('TMUX_PANE',None)
 # Bash login reads system profile, so use a tiny isolated allowed-name launcher
 # which delegates to actual Bash with rc disabled and fixture PATH intact.
 launch=root/'bash';launch.write_text('#!/bin/sh\nexec /bin/bash --noprofile --norc "$@"\n');launch.chmod(0o700);env['SHELL']=str(launch)
 sock=str(root/'state/cx/managed.sock')
 def cli(*args):return json.loads(subprocess.check_output([binary,*args],env=env,cwd=tmp))
 def sessions():return cli('sessions')
 try:
  created=cli('new','--provider','codex','--directory','.')
  deadline=time.monotonic()+5
  while time.monotonic()<deadline:
   live=next(s for s in sessions() if s['id']==created['id'])
   if live['name']=='Camera timestamp fix':break
   time.sleep(.1)
  assert live['name']=='Camera timestamp fix',live['name']
  assert live['directory']==tmp
  for key in ['id','pid','started','boot_id','socket']:assert live[key]==created[key],key
  index.write_text(json.dumps(dict(id=uuid,thread_name='Renamed native task'))+'\n')
  assert next(s for s in sessions() if s['id']==created['id'])['name']=='Renamed native task'
  # Multiple active writer identities must never resolve by recency.
  with index.open('a') as f:f.write(json.dumps(dict(id='12345678-1234-1234-1234-12345fffffff',thread_name='Other task'))+'\n')
  os.kill(live['process']['pid'], __import__('signal').SIGUSR1)
  time.sleep(.1)
  assert next(s for s in sessions() if s['id']==created['id'])['name']=='codex · '+root.name
  custom=cli('new','--provider','shell','--directory',tmp,'--name','work')
  assert next(s for s in sessions() if s['id']==custom['id'])['name']=='work'
  results.append(dict(result='PASS',scenario='Codex exact writer association, rename refresh, multiple-writer fallback, same terminal',provider='synthetic ELF executable; no inference'))
 finally:subprocess.run(['tmux','-S',sock,'kill-server'],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
print(json.dumps(results))
