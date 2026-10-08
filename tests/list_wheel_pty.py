#!/usr/bin/env python3
"""Real list wheel events select one adjacent file/session, not terminal arrow bursts."""
import fcntl,json,os,pathlib,pty,re,select,struct,subprocess,sys,tempfile,termios,time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve());checks=[]
for view in ['Files','Work']:
 with tempfile.TemporaryDirectory(prefix='cx-wheel-') as temporary:
  root=pathlib.Path(temporary);source=root/'source';source.mkdir()
  for i in range(12):(source/f'file{i:02}.txt').write_text(f'content {i}')
  state=root/'state/cx';state.mkdir(parents=True)
  env=dict(os.environ,HOME=str(root),XDG_STATE_HOME=str(root/'state'),TMUX_TMPDIR=str(root),SHELL='/bin/sh',TERM='xterm-256color',HTTPS_PROXY='http://127.0.0.1:9')
  for key in ['TMUX','TMUX_PANE','NO_COLOR','CX_ASCII']:env.pop(key,None)
  if view=='Work':
   for i in range(12):subprocess.check_output([binary,'new','--directory',str(source),'--name',f'wheel-{i:02}','--key',f'wheel-{i}'],env=env)
  name='viewer-restart-123-789.json'
  snapshot=dict(schema=1,expires_at=int(time.time())+300,device_ids=['local'],device=1,focus='Workspace',view=view,selected=0,side_selected=0,search='',browser=dict(device=0,path=str(source),display_path=str(source),parent=str(root),entries=[],selected=0,search='',preview_scroll=0,restore_selection=None) if view=='Files' else None,other_browser=None,destination_active=False,conflict=2,launch_provider=None,clipboard=None,submitted={})
  path=state/name;path.write_text(json.dumps(snapshot));path.chmod(0o600)
  master,slave=pty.openpty();before=termios.tcgetattr(slave);fcntl.ioctl(slave,termios.TIOCSWINSZ,struct.pack('HHHH',40,120,0,0))
  def tty():os.setsid();fcntl.ioctl(slave,termios.TIOCSCTTY,0)
  proc=subprocess.Popen([binary,'restart',name],env=env,stdin=slave,stdout=slave,stderr=slave,preexec_fn=tty)
  screen=pyte.Screen(120,40);stream=pyte.Stream(screen);raw=bytearray()
  pattern=r'file\d{2}\.txt' if view=='Files' else r'\bwheel-\d{2}\b'
  def read(duration=.2):
   end=time.monotonic()+duration
   while time.monotonic()<end:
    if select.select([master],[],[],.02)[0]:
     data=os.read(master,65536);raw.extend(data);stream.feed(data.decode(errors='replace'))
  def current():
   for row,line in enumerate(screen.display):
    if ('▸' in line or '›' in line) and re.search(pattern,line):return re.search(pattern,line).group(),row
   return None
  def wait(predicate):
   end=time.monotonic()+10
   while not predicate() and time.monotonic()<end:read(.1)
   assert predicate(),'\n'.join(screen.display)
  def send(data):os.write(master,data);read(.25)
  try:
   wait(lambda:current() is not None);read(.3)
   ordered=[re.search(pattern,line[21:]).group() for line in screen.display if re.search(pattern,line[21:]) and '│' in line]
   # Table rows include each fixture exactly once; selected-item text lives in sidebar.
   ordered=list(dict.fromkeys(ordered));assert len(ordered)==12,repr(ordered)+'\n'+'\n'.join(screen.display)
   first,row=current();assert first==ordered[0],(first,ordered)
   send(f'\x1b[<65;40;{row+1}M'.encode())
   assert current()[0]==ordered[1],{'view':view,'expected':ordered[1],'actual':current(),'mouse_reporting':b'\x1b[?1000h' in raw}
   assert b'\x1b[?1000h' in raw,'list must request genuine wheel reports'
   for index in range(2,7):
    send(f'\x1b[<65;40;{current()[1]+1}M'.encode());assert current()[0]==ordered[index],(view,index,current())
   send(f'\x1b[<64;40;{current()[1]+1}M'.encode());assert current()[0]==ordered[5]
   send(b'\x1b[B');assert current()[0]==ordered[6]
   send(b'\x1b[A');assert current()[0]==ordered[5]
   # Header/footer wheel events cannot retarget the current object.
   send(b'\x1b[<65;40;1M');assert current()[0]==ordered[5]
   send(b'\x03');proc.wait(timeout=5);assert proc.returncode==0 and termios.tcgetattr(slave)==before
   read(.1);assert b'\x1b[?1000l' in raw,'mouse reporting disabled on exit'
   checks.append(dict(view=view,result='PASS',one_item_per_wheel=True,rapid_sequence=True,keyboard_unchanged=True,header_ignored=True,termios_restored=True))
  finally:
   if proc.poll() is None:proc.terminate();proc.wait(timeout=5)
   os.close(master);os.close(slave)
   if view=='Work':subprocess.run(['tmux','-S',str(state/'managed.sock'),'kill-server'],env=env,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
print(json.dumps(dict(result='PASS',checks=checks)))
