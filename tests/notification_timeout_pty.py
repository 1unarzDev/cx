#!/usr/bin/env python3
"""Actual bottom-right notice and idle timeout, using private state and offline transport."""
import fcntl,json,os,pathlib,pty,select,struct,subprocess,sys,tempfile,termios,time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve());results=[]
with tempfile.TemporaryDirectory(prefix='cx-update-ui-') as directory:
 root=pathlib.Path(directory);state=root/'state/cx';state.mkdir(parents=True)
 env=dict(os.environ,HOME=str(root),XDG_STATE_HOME=str(root/'state'),SHELL='/bin/sh',TERM='xterm-256color',HTTPS_PROXY='http://127.0.0.1:9')
 for key in ['TMUX','TMUX_PANE','NO_COLOR','CX_ASCII']:env.pop(key,None)
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
  return master,slave,before,proc,read,send,text,screen
 master,slave,before,proc,read,send,text,screen=launch([])
 try:
  read(.5);assert 'Sessions' in text()
  send(b'\x10');send(b'Update cx');assert 'Update cx' in text();send(b'\r')
  deadline=time.monotonic()+5
  while 'update unreachable; retry' not in text() and time.monotonic()<deadline:read(.1)
  assert 'update unreachable; retry' in text(),text()
  rows=text().splitlines()
  y=next(i for i,row in enumerate(rows) if 'Warning' in row)
  x=rows[y].index('Warning')
  assert x>45 and 10<y<20,(x,y,text())
  assert screen.buffer[y][x].fg in ('brown','cdcd00'),screen.buffer[y][x]
  assert all(cell.bg=='default' for row in screen.buffer.values() for cell in row.values())
  assert rows[-1].strip()=='' and 'Ctrl C Quit' in text(),text()
  pathlib.Path('/tmp/cx-notice-pty-visible.json').write_text(json.dumps([
   [{'text':screen.buffer[y][x].data,'fg':screen.buffer[y][x].fg,'bold':screen.buffer[y][x].bold} for x in range(100)] for y in range(24)]))
  pathlib.Path('/tmp/cx-notice-pty-visible.txt').write_text(text())
  read(5.5)
  assert 'update unreachable; retry' not in text(),text()
  pathlib.Path('/tmp/cx-notice-pty-expired.txt').write_text(text())
  assert proc.poll() is None
  send(b'\x10');send(b'Files');assert 'Files' in text()
  send(b'\x1b');send(b'\x03');proc.wait(timeout=3)
  assert termios.tcgetattr(slave)==before
  results.append({'scenario':'floating warning panel/color/default background, idle expiry and preserved workspace/termios','result':'PASS'})
 finally:
  if proc.poll() is None:proc.terminate();proc.wait(timeout=3)
  os.close(master);os.close(slave)
print(json.dumps(results))
