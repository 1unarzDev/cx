#!/usr/bin/env python3
"""Real CLI/framed helper boundary: reconnect reads, never replay lost mutations."""
import json, os, pathlib, subprocess, sys, tempfile
binary=str(pathlib.Path(sys.argv[1]).resolve())
checks=[]
with tempfile.TemporaryDirectory(prefix='cx-reconnect-') as tmp:
 root=pathlib.Path(tmp); state=root/'state/cx';state.mkdir(parents=True); tools=root/'bin';tools.mkdir()
 (state/'devices.json').write_text(json.dumps([dict(id='fixture',name='fixture',target='fixture',account='test',host='fixture',status='unknown',observed_at=0)]))
 script=tools/'ssh';script.write_text('''#!/usr/bin/env python3
import sys,os,pathlib,json
root=pathlib.Path(os.environ['CX_RECONNECT_FIXTURE']);mode=os.environ['CX_RECONNECT_MODE']
count=root/'count';n=int(count.read_text())+1 if count.exists() else 1;count.write_text(str(n))
line=sys.stdin.buffer.readline();length=int(line.split()[1]);req=json.loads(sys.stdin.buffer.read(length))
if mode=='offline':print('No route to host SYNTHETIC_SECRET',file=sys.stderr);sys.exit(255)
if mode=='auth':print('Permission denied (publickey) SYNTHETIC_SECRET',file=sys.stderr);sys.exit(255)
if mode=='trust':print('Host key verification failed SYNTHETIC_SECRET',file=sys.stderr);sys.exit(255)
if mode=='mutate':(root/'accepted').write_text(req['op']['op']);sys.exit(0)
if mode=='closed' or n==1:sys.exit(0)
result=[];payload=json.dumps(dict(version=1,id=req['id'],result=result,error=None)).encode()
sys.stdout.buffer.write(b'CX1 '+str(len(payload)).encode()+b'\\n'+payload);sys.stdout.buffer.flush()
''');script.chmod(0o700)
 env=dict(os.environ,XDG_STATE_HOME=str(root/'state'),PATH=str(tools)+os.pathsep+os.environ['PATH'],CX_RECONNECT_FIXTURE=tmp)
 for mode,calls,success,command in [
  ('reconnect',2,True,['sessions','--device','fixture']),
  ('closed',2,False,['sessions','--device','fixture']),
  ('offline',1,False,['sessions','--device','fixture']),
  ('auth',1,False,['sessions','--device','fixture']),
  ('trust',1,False,['sessions','--device','fixture']),
  ('mutate',1,False,['new','--device','fixture','--directory','/tmp','--key','accepted-before-drop']),
 ]:
  (root/'count').unlink(missing_ok=True);env['CX_RECONNECT_MODE']=mode
  r=subprocess.run([binary,*command],env=env,capture_output=True,timeout=10)
  assert (r.returncode==0)==success,(mode,r.stderr)
  assert int((root/'count').read_text())==calls,mode
  assert b'SYNTHETIC_SECRET' not in r.stdout+r.stderr
  if mode=='mutate':assert (root/'accepted').read_text()=='create'
  checks.append(dict(mode=mode,result='PASS',attempts=calls))
print(json.dumps(dict(result='PASS',checks=checks)))
