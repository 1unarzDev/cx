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
  sessions=[]
  if view=='Work':
   for i in range(12):sessions.append(json.loads(subprocess.check_output([binary,'new','--directory',str(source),'--name',f'wheel-{i:02}','--key',f'wheel-{i}'],env=env)))
  name='viewer-restart-123-789.json'
  snapshot=dict(schema=1,expires_at=int(time.time())+300,device_ids=['local'],device=1,focus='Workspace',view=view,selected=0,side_selected=0,search='',browser=dict(device=0,path=str(source),display_path=str(source),parent=str(root),entries=[],selected=0,search='',preview_scroll=0,restore_selection=None) if view=='Files' else None,other_browser=None,destination_active=False,conflict=2,launch_provider=None,clipboard=None,submitted={})
  path=state/name;path.write_text(json.dumps(snapshot));path.chmod(0o600)
  master,slave=pty.openpty();before=termios.tcgetattr(slave);fcntl.ioctl(slave,termios.TIOCSWINSZ,struct.pack('HHHH',40,120,0,0))
  def tty():os.setsid();fcntl.ioctl(slave,termios.TIOCSCTTY,0)
  proc=subprocess.Popen([binary,'restart',name],env=env,stdin=slave,stdout=slave,stderr=slave,preexec_fn=tty)
  class FixtureScreen(pyte.Screen):
   def report_device_status(self,*args,**kwargs):pass
  screen=FixtureScreen(120,40);stream=pyte.Stream(screen);raw=bytearray()
  pattern=r'file\d{2}\.txt' if view=='Files' else r'\bwheel-\d{2}\b'
  def read(duration=.2):
   end=time.monotonic()+duration
   while time.monotonic()<end:
    if select.select([master],[],[],.02)[0]:
     data=os.read(master,65536);raw.extend(data);stream.feed(data.decode(errors='replace'))
  def current():
   # The selection bar marks the selected row; older layouts used a chevron.
   for row,line in enumerate(screen.display):
    # Files (yazi's columns): the current column only; the parent's cwd row has a bar too.
    line=line[15:75] if view=='Files' else line
    if ('▸' in line or '›' in line or '▌' in line) and re.search(pattern,line):
     # (Tiles sit side by side: the name after the bar is the selected one.)
     at=line.index('▌') if '▌' in line else 0
     found=re.search(pattern,line[at:]) or re.search(pattern,line)
     return found.group(),row
   return None
  def wait(predicate):
   end=time.monotonic()+10
   while not predicate() and time.monotonic()<end:read(.1)
   assert predicate(),'\n'.join(screen.display)
  def send(data):os.write(master,data);read(.25)
  try:
   wait(lambda:current() is not None);read(.3)
   # Files: the list's rows (its 48 columns at 120); a device's sessions: its tiles
   # in slot order, which is creation order.
   if view=='Files':
    ordered=[re.search(pattern,line[15:75]).group() for line in screen.display if re.search(pattern,line[15:75])]
   else:
    ordered=[f'wheel-{i:02}' for i in range(12)]
   ordered=list(dict.fromkeys(ordered));assert len(ordered)==12,repr(ordered)+'\n'+'\n'.join(screen.display)
   if view=='Work':send(b'\x1b[H')  # Home: the first slot (a restored selection may be any)
   first,row=current();assert first==ordered[0],(first,ordered,'\n'.join(screen.display))
   send(f'\x1b[<65;40;{row+1}M'.encode())
   assert current()[0]==ordered[1],{'view':view,'expected':ordered[1],'actual':current(),'mouse_reporting':b'\x1b[?1000h' in raw}
   assert b'\x1b[?1000h' in raw,'list must request genuine wheel reports'
   for index in range(2,7):
    send(f'\x1b[<65;40;{current()[1]+1}M'.encode());assert current()[0]==ordered[index],(view,index,current())
   send(f'\x1b[<64;40;{current()[1]+1}M'.encode());assert current()[0]==ordered[5]
   # The keys step too: Down/Up in a list; in a device's grid Right/Left (Down is the
   # tile below there).
   send(b'\x1b[B' if view=='Files' else b'\x1b[C');assert current()[0]==ordered[6],(view,current())
   send(b'\x1b[A' if view=='Files' else b'\x1b[D');assert current()[0]==ordered[5],(view,current())
   if view=='Files':
    # Search stays inline and highlights file00..file09. The wheel must cross
    # the last match into adjacent nonmatches, rather than wrapping to file00.
    send(b'/file0')
    for index in range(6,12):
     send(f'\x1b[<65;40;{current()[1]+1}M'.encode())
     assert current()[0]==ordered[index],('search wheel skipped adjacent file',index,current())
    # The physical capture contains identical reports in microsecond pairs.
    # A pair is one detent; two distinct ticks 43 ms apart remain two steps.
    up=f'\x1b[<64;40;{current()[1]+1}M'.encode()
    send(up+up);assert current()[0]==ordered[10]
    os.write(master,up);read(.043)
    os.write(master,up);read(.25);assert current()[0]==ordered[8]
    send(b'\x1b');read(.1)
    for index in range(7,4,-1):
     send(f'\x1b[<64;40;{current()[1]+1}M'.encode());assert current()[0]==ordered[index]
   if view=='Work':
    down=f'\x1b[<65;40;{current()[1]+1}M'.encode()
    send(down+down);assert current()[0]==ordered[6]
    up=f'\x1b[<64;40;{current()[1]+1}M'.encode()
    send(up+up);assert current()[0]==ordered[5]
    os.write(master,down);read(.043)
    os.write(master,down);read(.25);assert current()[0]==ordered[7]
    send(up);assert current()[0]==ordered[6]
    send(up);assert current()[0]==ordered[5]
   # Header/footer wheel events cannot retarget the current object.
   send(b'\x1b[<65;40;1M');assert current()[0]==ordered[5]
   if view=='Work':
    socket=str(state/'managed.sock')
    # Enter attaches natively at once (no focus/expand chain); Ctrl+\ returns.
    display=lambda:'\n'.join(screen.display)
    wait(lambda:current() is not None and current()[0]==ordered[5]);read(.5)
    target=next(r['id'] for r in sessions if r['name']==ordered[5])
    # Native clients only: the viewer's control-mode client (its live watch) is not one.
    def clients():return [l[2:] for l in subprocess.check_output(['tmux','-S',socket,'list-clients','-F','#{client_control_mode} #{session_name}'],env=env,text=True).splitlines() if l.startswith('0 ')]
    before_native=len(raw);send(b'\r')
    wait(lambda:clients()==[target] and b'Ctrl+\\' in raw[before_native:] and b'detach' in raw[before_native:])
    assert b'\x1b[?1000l' in raw[before_native:],'cx releases mouse before native terminal'
    back_start=len(raw);send(b'\x1c')
    wait(lambda:clients()==[] and current() is not None and b'\x1b[?1000h' in raw[back_start:])
    assert current()[0]==ordered[5]
    # Space peeks; a tap on the peeked screen (the top one: the filmstrip below shows
    # other sessions, whose tiles attach) types into it; Ctrl+] is application
    # input there, Ctrl+\ leaves.
    send(b' ');wait(lambda:'OTHERS' in display() or ' next' in screen.display[-1])
    y=min(y for y,line in enumerate(screen.display) if '$' in line and y>1);x=screen.display[y].index('$')
    send(f'\x1b[<0;{x+1};{y+1}M\x1b[<0;{x+1};{y+1}m'.encode());wait(lambda:'typing into the session' in display())
    send(b'\x1d');read(.3)
    assert 'typing into the session' in display() and clients()==[],'Ctrl+] must reach the pane, not detach'
    send(b'\x1c');wait(lambda:'typing into the session' not in display())
    restored=json.loads(subprocess.check_output([binary,'sessions'],env=env));by_id={r['id']:r for r in restored}
    assert all(all(by_id[r['id']][key]==r[key] for key in ['pid','started','boot_id','socket']) for r in sessions)
   send(b'\x03');proc.wait(timeout=5);assert proc.returncode==0 and termios.tcgetattr(slave)==before
   read(.1);assert b'\x1b[?1000l' in raw,'mouse reporting disabled on exit'
   checks.append(dict(view=view,result='PASS',one_item_per_wheel=True,rapid_sequence=True,duplicate_bursts_normalized=True,ticks_43ms_apart_preserved=True,keyboard_unchanged=True,header_ignored=True,termios_restored=True,native_mouse_handoff=view=='Work'))
  finally:
   if view=='Work':subprocess.run(['tmux','-S',str(state/'managed.sock'),'kill-server'],env=env,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
   if proc.poll() is None:proc.terminate();proc.wait(timeout=5)
   os.close(master);os.close(slave)
print(json.dumps(dict(result='PASS',checks=checks)))
