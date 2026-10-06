#!/usr/bin/env python3
import os,pty,subprocess,select,time,termios,fcntl,struct,json
master,slave=pty.openpty();fcntl.ioctl(slave,termios.TIOCSWINSZ,struct.pack('HHHH',40,120,0,0))
env=dict(os.environ,TERM='xterm-256color');env.pop('TMUX',None);env.pop('TMUX_PANE',None)
def control():os.setsid();fcntl.ioctl(slave,termios.TIOCSCTTY,0)
p=subprocess.Popen(['ssh','-tt','-o','BatchMode=yes','verybeautifulserver','~/.local/bin/cx'],stdin=slave,stdout=slave,stderr=slave,preexec_fn=control,env=env)
def read_for(seconds):
 output=bytearray();end=time.monotonic()+seconds
 while time.monotonic()<end:
  if select.select([master],[],[],.05)[0]:
   try:output.extend(os.read(master,65536))
   except OSError:break
 return bytes(output)
a=read_for(2);os.write(master,b'j\r');b=read_for(3);os.write(master,b'\x1d ');c=read_for(1);os.write(master,b'\x03')
p.wait(timeout=5)
print(json.dumps({'viewer':'verybeautifulserver','initial_work_visible':b'Work' in a,'codex_native_visible':b'Codex' in b or b'codex' in b,'execution_strip_visible':b'peace@tranquility' in b,'workspace_restored':b'Work' in c,'exit':p.returncode,'agent_free_server':True}))
assert b'Work' in a and b'peace@tranquility' in b and b'Work' in c and p.returncode==0
