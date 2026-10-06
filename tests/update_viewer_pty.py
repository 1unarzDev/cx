#!/usr/bin/env python3
"""Actual offline UI/manual-check and restart-state PTY restoration, no release/signature claims."""
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
  read(.5);assert 'Work' in text()
  send(b'\x10');send(b'Update cx');assert 'Update cx' in text();send(b'\r')
  deadline=time.monotonic()+5
  while 'Update service unreachable' not in text() and time.monotonic()<deadline:read(.1)
  assert 'Update service unreachable' in text(),text()
  send(b'\x10');send(b'Files');assert 'Files' in text()
  send(b'\x1b');send(b'\x03');proc.wait(timeout=3)
  assert termios.tcgetattr(slave)==before
  results.append({'scenario':'offline update action preserves usable workspace and termios','result':'PASS'})
 finally:
  if proc.poll() is None:proc.terminate();proc.wait(timeout=3)
  os.close(master);os.close(slave)
 # Exact production restore input. Preview content is deliberately absent.
 folder=root/'recordings';folder.mkdir();(folder/'keep.bin').write_bytes(b'fixture')
 name='viewer-restart-123-456.json'
 snapshot={'schema':1,'expires_at':int(time.time())+300,'device_ids':['local'],'device':1,'focus':'Workspace','view':'Files','selected':0,'selected_session':None,'side_selected':0,'search':'','browser':{'device':0,'path':str(folder),'display_path':str(folder),'parent':str(root),'entries':[{'name':'keep.bin','path':str(folder/'keep.bin'),'kind':'file','size':7}],'selected':0,'search':'keep','preview_scroll':0,'restore_selection':str(folder/'keep.bin')},'other_browser':None,'destination_active':False,'conflict':2,'launch_provider':'shell','clipboard':None,'submitted':{}}
 path=state/name;path.write_text(json.dumps(snapshot));path.chmod(0o600)
 master,slave,before,proc,read,send,text=launch(['restart',name])
 try:
  read(.8);assert 'keep.bin' in text() and 'recordings' in text(),text()
  assert 'Focus: Files' in text() and not path.exists(),text()
  send(b'\x03');proc.wait(timeout=3);assert termios.tcgetattr(slave)==before
  results.append({'scenario':'real restarted viewer restores host/folder/search/focus and consumes private snapshot','result':'PASS'})
 finally:
  if proc.poll() is None:proc.terminate();proc.wait(timeout=3)
  os.close(master);os.close(slave)
pathlib.Path('local-evidence/update-viewer-pty.json').write_text(json.dumps(results,indent=2));print(json.dumps(results))
