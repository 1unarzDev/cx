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
 # No agent here: stand-ins that fail shadow any claude/codex installed on this machine.
 (root/'bin').mkdir()
 for agent in ('claude','codex'):(root/'bin'/agent).write_text('#!/bin/sh\nexit 1\n');(root/'bin'/agent).chmod(0o700)
 env=dict(os.environ,HOME=tmp,XDG_STATE_HOME=str(root/'state'),SHELL='/bin/bash',TERM='xterm-256color',PATH=str(root/'bin')+os.pathsep+os.environ['PATH']);env.pop('TMUX',None);env.pop('TMUX_PANE',None)
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
  # The device's screen; its files (home), where `n` opens the chooser.
  read(1);send(b'f');read(1);send(b'!');wait(lambda:'[n] Shell' in text())
  # (Devcontainer is offered once this device's capabilities arrive: containers-v1.)
  assert '[c] Claude' not in text() and '[x] Codex' not in text(), text()
  assert not sessions(),'n created a session before a profile choice'
  def selected(label):
   for y,line in enumerate(screen.display):
    x=line.find(label)
    if x>=0:return screen.buffer[y][x].bold
   return False
  send(b'gg');wait(lambda:selected('[n] Shell'))
  send(b'l');send(b'\x1b[D');send(b'\x1b[C');send(b'h')
  send(b'\x1b');wait(lambda:'[n] Shell' not in text());assert not sessions()
  if ' NOR ' not in text():send(b'f');read(.5)  # an Esc from Files is the device's screen
  send(b'!');wait(lambda:'[n] Shell' in text());send(b'n')
  wait(lambda:len(sessions())==1);created=sessions()[0];assert created['provider']=='shell' and pathlib.Path(created['directory'])==root
  wait(lambda:'Ctrl+\\' in text() or '[n] Shell' not in text());read(1);send(b'printf "chooser-proof" > chooser-proof\n')
  wait(lambda:(root/'chooser-proof').exists());assert (root/'chooser-proof').read_text()=='chooser-proof'
  send(b'\x1c');read(.5);assert proc.poll() is None and len(sessions())==1
  # Detach lands on the hub's sessions zone; the network zone's node (this
  # device) opens its screen, where `w` watches the selected session.
  send(b'\t\rw')
  def readonly():
   p=subprocess.run(['tmux','-S',str(state/'managed.sock'),'list-clients','-F','#{client_readonly}'],capture_output=True)
   return b'1' in p.stdout.splitlines()
  wait(readonly)
  send(b'printf changed > chooser-proof\n');read(.5)
  assert (root/'chooser-proof').read_text()=='chooser-proof','watch mode injected terminal input'
  send(b'\x1c');read(.5);assert proc.poll() is None
  send(b'\x03');proc.wait(timeout=5);assert proc.returncode==0 and termios.tcgetattr(slave)==before
 finally:
  if proc.poll() is None:proc.kill();proc.wait()
  subprocess.run(['tmux','-S',str(state/'managed.sock'),'kill-server'],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
  os.close(master);os.close(slave)
print(json.dumps(dict(result='PASS',checks='Files n filters unavailable profiles, shell quick launch at home, native command, one detach, w read-only watch/input refusal and termios',scope='private CX state/tmux and owned shell')))
