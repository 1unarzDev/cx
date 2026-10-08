#!/usr/bin/env python3
"""Real viewer/transport outage recovery via an owned fake SSH endpoint; no network access."""
import fcntl,json,os,pathlib,pty,select,struct,subprocess,sys,tempfile,termios,time,signal,shlex
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve());results=[];frames=[];frame_started=time.monotonic()
with tempfile.TemporaryDirectory(prefix='cx-update-ui-') as directory:
 root=pathlib.Path(directory);state=root/'state/cx';state.mkdir(parents=True)
 env=dict(os.environ,HOME=str(root),XDG_STATE_HOME=str(root/'state'),SHELL='/bin/sh',TERM='xterm-256color',HTTPS_PROXY='http://127.0.0.1:9')
 for key in ['TMUX','TMUX_PANE','NO_COLOR','CX_ASCII']:env.pop(key,None)
 bindir=root/'bin';bindir.mkdir();mode=root/'mode';counter=root/'requests';helper_pid=root/'helper.pid'
 mode.write_text('offline');counter.write_text('')
 ssh=bindir/'ssh'
 ssh.write_text(f"""#!/bin/sh
echo request >> {shlex.quote(str(counter))}
for arg do last="$arg"; done
case "$last" in
  'exec ~/.local/bin/cx helper') ;;
  *) exit 1 ;;
esac
if [ "$(cat {shlex.quote(str(mode))})" = online ]; then
  echo "$$" >> {shlex.quote(str(helper_pid))}
  exec {shlex.quote(binary)} helper
fi
echo "Connection refused" >&2
exit 255
""")
 ssh.chmod(0o700);env['PATH']=str(bindir)+os.pathsep+env['PATH']
 (state/'devices.json').write_text(json.dumps([
  {'id':'local','name':'fixture-local','target':None,'account':'fixture','host':'fixture-local','status':'local','observed_at':0},
  {'id':'offline-fixture','name':'offline-fixture','target':'fixture-offline.invalid','account':'fixture','host':'offline-fixture','status':'unknown','observed_at':0}]))
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
     if len(frames)<160:
      frames.append({'ms':round((time.monotonic()-frame_started)*1000),'rows':[[{'text':screen.buffer[y][x].data,'fg':screen.buffer[y][x].fg} for x in range(100)] for y in range(24)]})
  def send(keys):os.write(master,keys);read()
  def text():return '\n'.join(screen.display)
  return master,slave,before,proc,read,send,text,screen
 master,slave,before,proc,read,send,text,screen=launch([])
 def toast():return any(' Error ' in line and ('╭' in line or '┌' in line) for line in screen.display)
 def refresh():send(b'\x10');send(b'Refresh');send(b'\r');read(.5)
 def count():return len(counter.read_text().splitlines())
 try:
  until=time.monotonic()+5
  while not toast() and time.monotonic()<until:read(.1)
  assert toast() and 'unreachable' in text(),text()
  read(5.5);assert not toast(),text()
  for _ in range(2):
   old=count();refresh();assert count()>old,'refresh did not retry transport'
   assert not toast(),text()
  # A real framed helper reply proves recovery, then a new outage can notify again.
  mode.write_text('online');refresh();assert helper_pid.exists(),'owned helper did not start'
  until=time.monotonic()+15
  while '2/2 devices' not in text() and time.monotonic()<until:read(.1)
  assert '2/2 devices' in text(),'successful helper reply was not applied: '+text()
  assert not toast(),text()
  mode.write_text('offline')
  for raw_pid in helper_pid.read_text().splitlines():
   pid=int(raw_pid)
   if not pathlib.Path('/proc/'+str(pid)).exists():continue
   status=pathlib.Path('/proc/'+str(pid)+'/stat').read_text().split(') ',1)[1].split()[0]
   if status=='Z':continue
   assert os.getpgid(pid)==pid and pathlib.Path('/proc/'+str(pid)+'/exe').resolve()==pathlib.Path(binary),(pid,os.getpgid(pid),str(pathlib.Path('/proc/'+str(pid)+'/exe').resolve()),status)
   os.killpg(pid,signal.SIGKILL)
  refresh()
  until=time.monotonic()+20
  while not toast() and time.monotonic()<until:read(.1)
  assert toast(),text()
  send(b'\x03');proc.wait(timeout=3);assert termios.tcgetattr(slave)==before
  results.append({'result':'PASS','checks':'one popup per outage, repeated refresh retries without popup, real helper recovery re-arms outage, termios','scope':'private CX state and owned fake SSH/helper; no network'})
 finally:
  if proc.poll() is None:proc.terminate();proc.wait(timeout=3)
  os.close(master);os.close(slave)
print(json.dumps(results))
