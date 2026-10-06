#!/usr/bin/env python3
"""Read-only live provider-picker acceptance against enrolled helpers; no agent starts.
Run: uv run --with pyte python tests/provider_choices_pty.py target/release/cx
"""
import fcntl,json,os,pathlib,pty,select,struct,subprocess,sys,tempfile,termios,time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
state=pathlib.Path(os.environ.get('XDG_STATE_HOME',str(pathlib.Path.home()/'.local/state')))/'cx'
devices=json.loads((state/'devices.json').read_text())
evidence=pathlib.Path('local-evidence/provider-picker'); evidence.mkdir(parents=True,exist_ok=True)
results=[]
for name,expected in [('innovation',['Claude','Codex']),('tranquility',['Codex']),('verybeautifulserver',[])]:
 index=next(i for i,d in enumerate(devices) if d['name']==name or d['host']==name)
 with tempfile.TemporaryDirectory(prefix='cx-provider-picker-') as root:
  test_state=pathlib.Path(root)/'cx';test_state.mkdir()
  (test_state/'devices.json').write_text(json.dumps(devices))
  if (state/'launcher.json').exists(): (test_state/'launcher.json').write_bytes((state/'launcher.json').read_bytes())
  master,slave=pty.openpty();before=termios.tcgetattr(slave)
  fcntl.ioctl(slave,termios.TIOCSWINSZ,struct.pack('HHHH',24,100,0,0))
  def control():
   os.setsid();fcntl.ioctl(slave,termios.TIOCSCTTY,0)
  env=dict(os.environ,XDG_STATE_HOME=root,TERM='xterm-256color');env.pop('TMUX',None);env.pop('TMUX_PANE',None)
  proc=subprocess.Popen([binary],stdin=slave,stdout=slave,stderr=slave,env=env,preexec_fn=control)
  screen=pyte.Screen(100,24);stream=pyte.Stream(screen)
  def read(duration=.2):
   end=time.monotonic()+duration
   while time.monotonic()<end:
    if select.select([master],[],[],.02)[0]:
     try: stream.feed(os.read(master,65536).decode(errors='replace'))
     except OSError: break
  def send(keys): os.write(master,keys);read()
  def text(): return '\n'.join(screen.display)
  try:
   read(.5);send(b'\x10');send(b'New session');send(b'\r')
   assert 'execution device' in text(),text()
   selected=None
   for i,device in enumerate(devices):
    label=device['name']+' · '+device['account']+'@'+device['host']
    for row,line in enumerate(screen.display):
     col=line.find(label)
     if col>=0 and screen.buffer[row][col].reverse: selected=i
   assert selected is not None, 'device highlight missing: '+text()
   send((b'j'*(index-selected) if index>=selected else b'k'*(selected-index))+b'\r')
   deadline=time.monotonic()+15
   while 'next: browse folder' not in text() and time.monotonic()<deadline:read(.1)
   capture=text();assert 'next: browse folder' in capture,capture
   actual=[provider for provider in ['Claude','Codex'] if provider+' · existing host profile' in capture]
   assert actual==expected,(name,actual,capture)
   assert 'Shell · ordinary terminal' in capture,capture
   (evidence/(name+'.txt')).write_text(capture)
   send(b'\x03');proc.wait(timeout=3)
   assert termios.tcgetattr(slave)==before
   results.append({'host':name,'choices':['Shell']+actual,'result':'PASS','backend':'real helpers, allocated PTY','agents_started':0})
  finally:
   if proc.poll() is None:proc.terminate();proc.wait(timeout=3)
   os.close(master);os.close(slave)
(evidence/'results.json').write_text(json.dumps(results,indent=2));print(json.dumps(results))
