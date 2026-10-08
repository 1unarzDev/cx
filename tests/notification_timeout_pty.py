#!/usr/bin/env python3
"""Actual bottom-right notice and idle timeout, using private state and offline transport."""
import fcntl,json,os,pathlib,pty,select,struct,subprocess,sys,tempfile,termios,time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve());results=[]
with tempfile.TemporaryDirectory(prefix='cx-update-ui-') as directory:
 root=pathlib.Path(directory);state=root/'state/cx';state.mkdir(parents=True)
 env=dict(os.environ,HOME=str(root),XDG_STATE_HOME=str(root/'state'),SHELL='/bin/sh',TERM='xterm-256color',HTTPS_PROXY='http://127.0.0.1:9')
 env.pop('TMUX',None);env.pop('TMUX_PANE',None)
 def launch(args):
  master,slave=pty.openpty();before=termios.tcgetattr(slave)
  fcntl.ioctl(slave,termios.TIOCSWINSZ,struct.pack('HHHH',24,100,0,0))
  def control():os.setsid();fcntl.ioctl(slave,termios.TIOCSCTTY,0)
  proc=subprocess.Popen([binary]+args,stdin=slave,stdout=slave,stderr=slave,env=env,preexec_fn=control)
  screen=pyte.Screen(100,24);stream=pyte.Stream(screen)
  def read(duration=.25):
   end=time.monotonic()+duration
   while time.monotonic()<end:
    if select.select([master],[],[],.02)[0]:
     try:stream.feed(os.read(master,65536).decode(errors='replace'))
     except OSError:break
  def send(keys):os.write(master,keys);read()
  def text():return '\n'.join(screen.display)
  return master,slave,before,proc,read,send,text
 master,slave,before,proc,read,send,text=launch([])
 try:
  read(.5);assert 'Sessions' in text()
  send(b'\x10');send(b'Update cx');assert 'Update cx' in text();send(b'\r')
  deadline=time.monotonic()+5
  while 'update unreachable; retry' not in text() and time.monotonic()<deadline:read(.1)
  assert 'update unreachable; retry' in text(),text()
  row=text().splitlines()[-1]
  assert row.endswith('  ') and row.rstrip().endswith('retry'),row
  assert row.startswith(' ' * 10),row
  pathlib.Path('/tmp/cx-notice-pty-visible.txt').write_text(text())
  read(5.5)
  assert 'update unreachable; retry' not in text(),text()
  pathlib.Path('/tmp/cx-notice-pty-expired.txt').write_text(text())
  assert proc.poll() is None
  send(b'\x10');send(b'Files');assert 'Files' in text()
  send(b'\x1b');send(b'\x03');proc.wait(timeout=3)
  assert termios.tcgetattr(slave)==before
  results.append({'scenario':'right-aligned notice disappears without input; workspace and termios preserved','result':'PASS'})
 finally:
  if proc.poll() is None:proc.terminate();proc.wait(timeout=3)
  os.close(master);os.close(slave)
print(json.dumps(results))
