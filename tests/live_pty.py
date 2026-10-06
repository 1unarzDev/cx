#!/usr/bin/env python3
"""Bounded native PTY checks; disposable test sessions only, no provider prompts."""
import os,pty,subprocess,select,time,termios,fcntl,struct,json,sys
TARGET=sys.argv[1] if len(sys.argv)>1 else 'verybeautifulserver'
SESSION=sys.argv[2] if len(sys.argv)>2 else 'cx-priority-codex'
master,slave=pty.openpty()
fcntl.ioctl(slave,termios.TIOCSWINSZ,struct.pack('HHHH',24,80,0,0))
env=dict(os.environ,TERM='xterm-256color');env.pop('TMUX',None);env.pop('TMUX_PANE',None)
cmd=['/home/lunarz/cx/target/release/cx','attach',SESSION,'--device','tranquility'] if TARGET=='local' else ['ssh','-tt','-o','BatchMode=yes',TARGET,f'~/.local/bin/cx attach {SESSION} --device tranquility']
def control_tty():
 os.setsid();fcntl.ioctl(slave,termios.TIOCSCTTY,0)
proc=subprocess.Popen(cmd,stdin=slave,stdout=slave,stderr=slave,preexec_fn=control_tty,env=env)
output=bytearray();deadline=time.monotonic()+6
while time.monotonic()<deadline and proc.poll() is None:
 if select.select([master],[],[],.1)[0]:
  try:output.extend(os.read(master,65536))
  except OSError:break
# Managed prefix + Space must return without sending Ctrl+B/Ctrl+C into provider.
os.write(master,b'\x1d ')
try:proc.wait(timeout=5)
except subprocess.TimeoutExpired:proc.terminate();proc.wait(timeout=3)
print(json.dumps({'host':TARGET,'session':SESSION,'pty_exit':proc.returncode,'bytes':len(output),'host_identity_visible':b'peace@tranquility' in output,'codex_visible':b'Codex' in output or b'codex' in output,'detach_marker':b'detached' in output}))
# Sanitized evidence deliberately records assertions only, never provider screens.
os.close(master);os.close(slave)
if proc.returncode!=0:
 print(bytes(output[:250]).decode(errors='replace').encode('unicode_escape').decode());sys.exit(1)
