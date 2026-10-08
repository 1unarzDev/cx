#!/usr/bin/env python3
"""Real tmux attachment with a vendor terminal absent from the execution OS."""
import fcntl,json,os,pathlib,pty,select,subprocess,sys,tempfile,termios,time,struct
binary=str(pathlib.Path(sys.argv[1]).resolve())
with tempfile.TemporaryDirectory(prefix='cx-attach-term-') as tmp:
 root=pathlib.Path(tmp)
 env=dict(os.environ,HOME=tmp,XDG_STATE_HOME=tmp+'/state',SHELL='/bin/bash',TERM='cx-vendor-terminal-not-installed')
 env.pop('TMUX',None);env.pop('TMUX_PANE',None)
 def cli(*args):return json.loads(subprocess.check_output([binary,*args],env=env,timeout=10))
 session=cli('new','--directory',tmp,'--key','owned-terminal-proof','--name','fixture')
 master,slave=pty.openpty();fcntl.ioctl(slave,termios.TIOCSWINSZ,struct.pack('HHHH',24,80,0,0));before=termios.tcgetattr(slave)
 def tty():os.setsid();fcntl.ioctl(slave,termios.TIOCSCTTY,0)
 command=[binary,'native-attach','attach-session','-t','='+session['id']]
 p=subprocess.Popen(command,stdin=slave,stdout=slave,stderr=slave,env=env,preexec_fn=tty)
 output=b''
 try:
  deadline=time.monotonic()+3
  while time.monotonic()<deadline and p.poll() is None:
   if select.select([master],[],[],.1)[0]:output+=os.read(master,8192)
  assert p.poll() is None,'unsupported vendor terminal closed attachment'
  os.write(master,b"printf '\\nCX_TERMINAL_COMMAND_PROOF\\n'\n")
  deadline=time.monotonic()+3
  while time.monotonic()<deadline:
   if select.select([master],[],[],.1)[0]:output+=os.read(master,8192)
   # Count command echo plus actual output to ensure the shell executed it.
   if output.count(b'CX_TERMINAL_COMMAND_PROOF')>=2:break
  assert output.count(b'CX_TERMINAL_COMMAND_PROOF')>=2,'shell did not execute command'
  # A terminal redraw can duplicate echoed text. Confirm actual command output
  # through the owned server before testing a single physical detach key.
  pane=subprocess.check_output(['tmux','-S',str(root/'state/cx/managed.sock'),
      'capture-pane','-p','-t',session['id']+':0.0'],env=env,timeout=3)
  assert b'CX_TERMINAL_COMMAND_PROOF' in pane.splitlines(),'command output absent from pane'
  os.write(master,b'\x1d')
  deadline=time.monotonic()+5
  while p.poll() is None and time.monotonic()<deadline:
   if select.select([master],[],[],.05)[0]:os.read(master,8192)
  assert p.poll()==0,'single detach key did not return: '+str(p.poll())
  assert p.returncode==0 and before==termios.tcgetattr(slave)
  assert session['id'] in {s['id'] for s in cli('sessions')}
  print(json.dumps(dict(result='PASS',unknown_terminal_fallback=True,interactive_command=True,detach_preserves_shell=True,termios_restored=True)))
 finally:
  if p.poll() is None:p.kill();p.wait()
  subprocess.run(['tmux','-S',str(root/'state/cx/managed.sock'),'kill-server'],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL,timeout=3)
  os.close(master);os.close(slave)
