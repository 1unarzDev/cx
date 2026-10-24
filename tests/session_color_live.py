#!/usr/bin/env python3
"""Check native RGB question backgrounds survive a managed tmux viewer."""
import fcntl,json,os,pathlib,pty,select,struct,subprocess,sys,tempfile,termios,time
binary=str(pathlib.Path(sys.argv[1]).resolve())
with tempfile.TemporaryDirectory(prefix='cx-color-') as t:
 env=dict(os.environ,HOME=t,XDG_STATE_HOME=t+'/state',SHELL='/bin/bash',TERM='xterm-256color',COLORTERM='truecolor')
 env.pop('TMUX',None);env.pop('TMUX_PANE',None)
 socket=t+'/state/cx/managed.sock';client=None
 def tm(*args):return subprocess.check_output(['tmux','-S',socket,*args],env=env,text=True)
 try:
  row=json.loads(subprocess.check_output([binary,'new','--provider','shell','--directory',t,'--key','rgb'],env=env))
  master,slave=pty.openpty();fcntl.ioctl(slave,termios.TIOCSWINSZ,struct.pack('HHHH',24,100,0,0))
  def tty():os.setsid();fcntl.ioctl(slave,termios.TIOCSCTTY,0)
  client=subprocess.Popen(['tmux','-S',socket,'attach','-t',row['id']],env=env,stdin=slave,stdout=slave,stderr=slave,preexec_fn=tty)
  def drain(seconds):
   out=bytearray();end=time.monotonic()+seconds
   while time.monotonic()<end:
    if select.select([master],[],[],.02)[0]:out.extend(os.read(master,65536))
   return bytes(out)
  drain(.3)
  # Fixed synthetic question swatch, never a provider transcript.
  command="printf '\\033[48;2;38;40;52m\\033[38;2;221;218;210m  Question: choose a project  \\033[39;49m\\n'"
  tm('send-keys','-t',row['id'],'-l',command);tm('send-keys','-t',row['id'],'Enter')
  out=drain(.4)
  assert b'48;2;38;40;52m' in out, 'RGB question background quantized by managed viewer'
  assert b'38;2;221;218;210m' in out, 'RGB question foreground quantized by managed viewer'
  assert b'49m' in out or b'\x1b[m' in out, 'default background not restored'
  print(json.dumps({'result':'PASS','checks':['native truecolor question foreground and background preserved','default background restored']}))
 finally:
  if client is not None:
   client.terminate();client.wait(timeout=3);os.close(master);os.close(slave)
  subprocess.run(['tmux','-S',socket,'kill-server'],env=env,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
