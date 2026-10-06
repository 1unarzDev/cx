import re,os,sys,json,pathlib,tempfile,subprocess,pty,fcntl,termios,struct,select,time,base64
binary=str(pathlib.Path(sys.argv[1]).resolve())
with tempfile.TemporaryDirectory(prefix='cx-clipboard-') as tmp:
 env=dict(os.environ,HOME=tmp,XDG_STATE_HOME=tmp+'/state',SHELL='/bin/bash',TERM='xterm-256color')
 for k in ('TMUX','TMUX_PANE'):env.pop(k,None)
 s=json.loads(subprocess.check_output([binary,'new','--directory',tmp,'--key','clipboard-fixture'],env=env))
 sock=tmp+'/state/cx/managed.sock'
 def tmux(*args):return subprocess.check_output(['tmux','-S',sock,*args],env=env)
 m,t=pty.openpty();fcntl.ioctl(t,termios.TIOCSWINSZ,struct.pack('HHHH',24,80,0,0))
 def ctl():os.setsid();fcntl.ioctl(t,termios.TIOCSCTTY,0)
 p=subprocess.Popen([binary,'native-attach','attach-session','-t',s['id']],env=env,stdin=t,stdout=t,stderr=t,preexec_fn=ctl)
 out=bytearray()
 def read_for(seconds):
  until=time.monotonic()+seconds
  while time.monotonic()<until:
   if select.select([m],[],[],.02)[0]:out.extend(os.read(m,65536))
 try:
  read_for(.5);out.clear()
  # Output originates inside the managed pane, just like an application's yank.
  code="import os;os.write(1,b'\\x1b]52;c;Y3gtY2xpcGJvYXJkLWZpeHR1cmU=\\x07')"
  import shlex
  tmux('send-keys','-t',s['id'],'python3 -c '+shlex.quote(code),'Enter');read_for(.5)
  def copied(text):
   return re.search(rb'\x1b\]52;[^;]*;'+re.escape(base64.b64encode(text))+rb'(?:\x07|\x1b\\)',out) is not None
  assert copied(b'cx-clipboard-fixture'), 'remote application copy did not reach terminal'
  out.clear();tmux('set-buffer','-w','cx scrollback fixture');read_for(.4)
  assert copied(b'cx scrollback fixture'), 'tmux copy did not reach terminal'
  assert tmux('show-options','-sv','set-clipboard').strip()==b'on'
  # Repeated owned configuration leaves override length/content unchanged.
  overrides=tmux('show-options','-sv','terminal-overrides')
  for _ in range(3):tmux('source-file',tmp+'/state/cx/tmux.conf')
  assert tmux('show-options','-sv','terminal-overrides')==overrides
  # Keyboard selection executes real copy-mode bindings in both tables.
  pane=s['id']+':0.0'
  tmux('send-keys','-t',pane,'clear; printf "cx-keyboard-fixture\\n"','Enter');read_for(.3)
  for mode in ('emacs','vi'):
   tmux('set-option','-g','mode-keys',mode)
   for key in (b'y',b'\r'):
    tmux('copy-mode','-t',pane)
    tmux('send-keys','-X','-t',pane,'history-top')
    out.clear();os.write(m,b' ');read_for(.05)
    tmux('send-keys','-X','-t',pane,'end-of-line')
    os.write(m,key);read_for(.2)
    assert b'\x1b]52;' in out, (mode,key,'binding did not copy')
    assert tmux('display-message','-p','-t',pane,'#{pane_in_mode}').strip()==b'0'
  out.clear()
  for event in (b'\x1b[<0;1;1M',b'\x1b[<32;3;1M',b'\x1b[<32;12;1M',b'\x1b[<0;12;1m'):
   os.write(m,event);read_for(.15)
  assert b'\x1b]52;' in out, 'mouse drag selection did not copy'
  assert tmux('display-message','-p','-t',pane,'#{pane_in_mode}').strip()==b'0'
  os.write(m,b'\x1d');p.wait(timeout=5)
  print(json.dumps(dict(result='PASS',application_osc52=True,tmux_copy=True,keyboard_copy_both_modes=True,mouse_drag_copy=True,idempotent_config=True,return_key=True)))
 finally:
  if p.poll() is None:p.terminate();p.wait(timeout=5)
  tmux('kill-server');os.close(m);os.close(t)
