#!/usr/bin/env python3
"""Private tmux plus fake Docker: saved container records cannot resurrect stale runtimes."""
import json,os,pathlib,subprocess,sys,tempfile
binary=str(pathlib.Path(sys.argv[1]).resolve())
with tempfile.TemporaryDirectory(prefix='cx-scope-reconcile-') as t:
 root=pathlib.Path(t);fixture=root/'docker.json';calls=root/'docker-calls';id='a'*64;other='b'*64
 docker=root/'docker'
 docker.write_text('#!'+sys.executable+'\n'+'''
import json,os,sys
f=json.load(open(os.environ['CX_SCOPE_FIXTURE']));a=sys.argv[1:]
with open(os.environ['CX_SCOPE_CALLS'],'a') as out:out.write(a[0]+'\\n')
if a[0]=='context':print('unix:///fixture/docker.sock')
elif a[0]=='info':print(f['engine'])
elif a[0]=='ps':print('\\n'.join(f['ids']))
elif a[0]=='inspect':
 v=f['containers'].get(a[-1])
 if v is None:sys.exit(1)
 print(json.dumps(v))
else:sys.exit(99)
''');docker.chmod(0o700)
 env=dict(os.environ,HOME=t,XDG_STATE_HOME=t+'/state',SHELL='/bin/bash',TERM='xterm-256color',PATH=t+os.pathsep+os.environ['PATH'],CX_SCOPE_FIXTURE=str(fixture),CX_SCOPE_CALLS=str(calls))
 env.pop('TMUX',None);env.pop('TMUX_PANE',None);env.pop('DOCKER_CONTEXT',None);env.pop('DOCKER_HOST',None)
 socket=t+'/state/cx/managed.sock'
 container=dict(id=id,name='/workspace',state='running',started_at='first',user='dev',folder='/workspace',workspace='/fixture/project')
 def write(ids,containers,engine='fixture-engine'):fixture.write_text(json.dumps(dict(ids=ids,containers=containers,engine=engine)))
 def helper(operation):
  body=json.dumps(dict(version=1,id='fixture',op=dict(op=operation))).encode()
  result=subprocess.check_output([binary,'helper'],input=b'CX1 '+str(len(body)).encode()+b'\n'+body,env=env,timeout=20)
  reply=json.loads(result.split(b'\n',1)[1]);assert not reply.get('error'),reply.get('error');return reply['result']
 try:
  write([id],{id:container})
  observed=helper('containers')['containers'][0]
  s=json.loads(subprocess.check_output([binary,'new','--provider','shell','--directory',t,'--key','scoped'],env=env));sid=s['id']
  path=root/'state/cx'/f'{sid}.json';record=json.loads(path.read_text())
  record['container']={key:observed[key] for key in ['engine','id','name','user','started_at','folder']};path.write_text(json.dumps(record))
  def visible():return any(s['id']==sid for s in helper('sessions'))
  assert visible(),'running exact scope missing'
  write([id],{})
  assert visible(),'failed inspection mistaken for authoritative absence'
  write([], {})
  assert not visible(),'complete absence resurrected cached container terminal'
  for field,value in [('state','exited'),('started_at','second'),('user','other')]:
   changed=dict(container);changed[field]=value;write([id,other],{id:changed})
   assert not visible(),f'explicit {field} evidence failed to prune during partial discovery'
  write([id],{id:container},engine='replaced-engine')
  assert not visible(),'engine replacement resurrected old scoped terminal'
  write([id],{id:container})
  assert visible(),'unchanged live scope missing after unrelated failed observations'
  assert 'exec' not in calls.read_text().splitlines(),'passive reconciliation executed inside container'
  print(json.dumps({'result':'PASS','checks':['saved scoped sessions revalidated every listing','partial uncertainty preserved','complete absence removed','stopped/restarted/user/engine evidence removed','no container execution or helper deployment']}))
 finally:subprocess.run(['tmux','-S',socket,'kill-server'],env=env,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
