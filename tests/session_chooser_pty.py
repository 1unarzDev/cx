#!/usr/bin/env python3
"""Real n chooser keys, safe cancel, shell attachment and home scope; owned state."""
import fcntl,json,os,pathlib,pty,select,struct,subprocess,sys,tempfile,termios,time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
with tempfile.TemporaryDirectory(prefix='cx-session-chooser-') as tmp:
 root=pathlib.Path(tmp);(root/'.local/bin').mkdir(parents=True);(root/'.local/bin/cx').symlink_to(binary)
 state=root/'state/cx';state.mkdir(parents=True);name='viewer-restart-123-789.json'
 snapshot=dict(schema=1,expires_at=int(time.time())+300,device_ids=['local'],device=1,focus='Devices',view='Work',selected=0,side_selected=0,search='',browser=None,other_browser=None,destination_active=False,conflict=2,launch_provider=None,clipboard=None,submitted={})
 path=state/name;path.write_text(json.dumps(snapshot));path.chmod(0o600)
 env=dict(os.environ,HOME=tmp,XDG_STATE_HOME=str(root/'state'),SHELL='/bin/bash',TERM='xterm-256color');env.pop('TMUX',None);env.pop('TMUX_PANE',None)
 master,slave=pty.openpty();before=termios.tcgetattr(slave);fcntl.ioctl(slave,termios.TIOCSWINSZ,struct.pack('HHHH',24,100,0,0))
 def tty():os.setsid();fcntl.ioctl(slave,termios.TIOCSCTTY,0)
 proc=subprocess.Popen([binary,'restart',name],env=env,stdin=slave,stdout=slave,stderr=slave,preexec_fn=tty)
 class Screen(pyte.Screen):
  def report_device_status(self,*args,**kwargs):pass
 screen=Screen(100,24);stream=pyte.Stream(screen)
 def read(seconds=.3):
  end=time.monotonic()+seconds
  while time.monotonic()<end:
   if select.select([master],[],[],.02)[0]:stream.feed(os.read(master,65536).decode('utf-8','replace'))
 def send(s):os.write(master,s);read()
 def wait(predicate):
  end=time.monotonic()+12
  while not predicate() and time.monotonic()<end:read(.1)
  assert predicate(),'\n'.join(screen.display)
 def text():return '\n'.join(screen.display)
 def sessions():return json.loads(subprocess.check_output([binary,'sessions'],env=env))
 try:
  read(1);send(b'n');wait(lambda:'[d] Devcontainer' in text())
  for label in ['[c] Claude','[x] Codex','[s] Shell']:assert label in text()
  assert not sessions(),'n created a session before a profile choice'
  send(b'l');send(b'\x1b[D');send(b'\x1b[C');send(b'h')
  send(b'\x1b');wait(lambda:'[d] Devcontainer' not in text());assert not sessions()
  send(b'n');wait(lambda:'[s] Shell' in text());send(b's')
  wait(lambda:len(sessions())==1);created=sessions()[0];assert created['provider']=='shell' and pathlib.Path(created['directory'])==root
  wait(lambda:'Ctrl+]' in text() or '[s] Shell' not in text());read(1);send(b'printf "chooser-proof" > chooser-proof\n')
  wait(lambda:(root/'chooser-proof').exists());assert (root/'chooser-proof').read_text()=='chooser-proof'
  send(b'\x1d');read(.5);assert proc.poll() is None and len(sessions())==1
  send(b'\x03');proc.wait(timeout=5);assert proc.returncode==0 and termios.tcgetattr(slave)==before
 finally:
  if proc.poll() is None:proc.kill();proc.wait()
  subprocess.run(['tmux','-S',str(state/'managed.sock'),'kill-server'],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
  os.close(master);os.close(slave)
print(json.dumps(dict(result='PASS',checks='n opens four choices without launching, h/l and arrows, cancel, s quick launch at home, native command, one detach and termios',scope='private CX state/tmux and owned shell')))
