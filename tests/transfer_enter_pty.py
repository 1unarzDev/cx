#!/usr/bin/env python3
"""Genuine destination Enter performs an owned local transfer into current folder."""
import atexit,fcntl,json,os,pathlib,pty,select,struct,subprocess,sys,tempfile,termios,time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
with tempfile.TemporaryDirectory(prefix='cx-transfer-enter-') as tmp:
 root=pathlib.Path(tmp);source=root/'source';source.mkdir();destination=root/'destination';destination.mkdir();(destination/'nested').mkdir()
 (source/'proof.txt').write_text('Enter confirms current folder')
 state=root/'state/cx';state.mkdir(parents=True);name='viewer-restart-123-789.json'
 def browser(path):return dict(device=0,path=str(path),display_path=str(path),parent=str(root),entries=[],selected=0,search='',preview_scroll=0,restore_selection=None)
 clip=dict(id='enter-proof',device=0,entries=[dict(name='proof.txt',path=str(source/'proof.txt'),kind='file',size=28,identity=None,hidden=False,rename_name=None)],cut=False)
 snapshot=dict(schema=1,expires_at=int(time.time())+300,device_ids=['local'],device=1,focus='Workspace',view='Files',selected=0,side_selected=0,search='',browser=browser(destination),other_browser=browser(source),destination_active=True,conflict=2,launch_provider=None,clipboard=clip,submitted={})
 path=state/name;path.write_text(json.dumps(snapshot));path.chmod(0o600)
 env=dict(os.environ,HOME=tmp,XDG_STATE_HOME=str(root/'state'),SHELL='/bin/bash',TERM='xterm-256color',HTTPS_PROXY='http://127.0.0.1:9');env.pop('TMUX',None);env.pop('TMUX_PANE',None)
 container_id=None
 if '--container' in sys.argv[2:]:
  container_id=subprocess.check_output(['docker','run','-d','--network','none','--name',f'cx-transfer-ui-proof-{os.getpid()}','--label','devcontainer.metadata=[{"remoteUser":"root"}]','ubuntu:24.04','sleep','infinity'],timeout=60).decode().strip()
  def cleanup_container():subprocess.run(['docker','rm','-f',container_id],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL,timeout=60)
  atexit.register(cleanup_container)
  subprocess.check_call(['docker','exec',container_id,'mkdir','-p','/tmp/ui-output/nested'],timeout=10)
  c=json.loads(subprocess.check_output([binary,'containers'],env=env,timeout=30))['containers']
  c=next(c for c in c if c['id']==container_id)
  scope={key:c[key] for key in ['engine','id','name','user','started_at','folder']}
  snapshot['browser']=dict(browser('/tmp/ui-output'),container=scope,parent='/tmp')
  path.write_text(json.dumps(snapshot))
 def exists(relative):
  if container_id:return subprocess.run(['docker','exec',container_id,'test','-e','/tmp/ui-output/'+relative],capture_output=True,timeout=10).returncode==0
  return (destination/relative).exists()
 def contents():
  if container_id:return subprocess.check_output(['docker','exec',container_id,'cat','/tmp/ui-output/proof.txt'],timeout=10).decode()
  return (destination/'proof.txt').read_text()

 m,t=pty.openpty();before=termios.tcgetattr(t);fcntl.ioctl(t,termios.TIOCSWINSZ,struct.pack('HHHH',24,100,0,0))
 def tty():os.setsid();fcntl.ioctl(t,termios.TIOCSCTTY,0)
 p=subprocess.Popen([binary,'restart',name],env=env,stdin=t,stdout=t,stderr=t,preexec_fn=tty)
 screen=pyte.Screen(100,24);stream=pyte.Stream(screen)
 def read(duration=.2):
  end=time.monotonic()+duration
  while time.monotonic()<end:
   if select.select([m],[],[],.02)[0]:stream.feed(os.read(m,65536).decode('utf-8','replace'))
 def wait(predicate):
  end=time.monotonic()+15
  while not predicate() and time.monotonic()<end:read(.1)
  assert predicate(),'\n'.join(screen.display)
 try:
  wait(lambda:'Enter transfer here' in '\n'.join(screen.display) and 'nested' in '\n'.join(screen.display));assert not exists('proof.txt')
  os.write(m,b'\r');wait(lambda:exists('proof.txt'))
  assert contents()=='Enter confirms current folder'
  assert not exists('nested/proof.txt'), 'hovered folder retargeted transfer'
  wait(lambda:'Copy complete' in '\n'.join(screen.display))
  jobs=json.loads(subprocess.check_output([binary,'jobs'],env=env))['jobs'];assert len(jobs)==1 and jobs[0]['status']=='complete'
  os.write(m,b'\x03');read(.2);p.wait(timeout=5);assert p.returncode==0 and termios.tcgetattr(t)==before
 finally:
  if p.poll() is None:p.kill();p.wait()
  os.close(m);os.close(t)
  if container_id:cleanup_container();atexit.unregister(cleanup_container)
print(json.dumps(dict(result='PASS',checks='destination Enter, current folder rather than hovered directory, actual copied content/durable completion and termios',scope='owned local files/private CX state'+('/network-none container' if container_id else ''))))
