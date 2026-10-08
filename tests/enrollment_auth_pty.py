#!/usr/bin/env python3
"""Integrated masked enrollment prompts with synthetic SSH; no live trust changes."""
import os,sys,pathlib,tempfile,subprocess,pty,fcntl,termios,struct,time,select,json
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
results=[]
for width,kind in [(80,'password'),(48,'host'),(80,'cancel')]:
 with tempfile.TemporaryDirectory(prefix='cx-auth-pty-') as tmp:
  root=pathlib.Path(tmp);tools=root/'bin';tools.mkdir()
  ssh=tools/'ssh'
  ssh.write_text('''#!/usr/bin/env python3
import os,subprocess,sys
kind=os.environ['CX_FIXTURE_KIND']
open(os.environ['CX_FIXTURE_RESULT']+'.pid','w').write(str(os.getpid()))
prompt="fixture@host's password: " if kind!='host' else "The authenticity of host 'fixture.example (192.0.2.1)' can't be established.\\nED25519 key fingerprint is SHA256:ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqr.\\nAre you sure you want to continue connecting (yes/no/[fingerprint])? "
r=subprocess.run([os.environ['SSH_ASKPASS'],prompt],capture_output=True)
expected=b'yes\\n' if kind=='host' else b'fixture-secret-937\\n'
passed=(r.returncode!=0 and not r.stdout) if kind=='cancel' else r.stdout==expected
open(os.environ['CX_FIXTURE_RESULT'],'w').write('PASS' if passed else 'FAIL')
sys.exit(1)
''');ssh.chmod(0o700)
  master,slave=pty.openpty();before=termios.tcgetattr(slave)
  fcntl.ioctl(slave,termios.TIOCSWINSZ,struct.pack('HHHH',24,width,0,0))
  env=dict(os.environ,HOME=str(root),XDG_STATE_HOME=str(root/'state'),SHELL='/bin/sh',TERM='xterm-256color',PATH=str(tools)+':'+os.environ['PATH'],HTTPS_PROXY='http://127.0.0.1:9',CX_FIXTURE_KIND=kind,CX_FIXTURE_RESULT=str(root/'result'))
  env.pop('TMUX',None);env.pop('TMUX_PANE',None)
  def tty():os.setsid();fcntl.ioctl(slave,termios.TIOCSCTTY,0)
  p=subprocess.Popen([binary],stdin=slave,stdout=slave,stderr=slave,env=env,preexec_fn=tty)
  screen=pyte.Screen(width,24);stream=pyte.Stream(screen);output=bytearray()
  def read(t=.25):
   end=time.monotonic()+t
   while time.monotonic()<end:
    if select.select([master],[],[],.02)[0]:
     try:b=os.read(master,65536)
     except OSError:break
     output.extend(b);stream.feed(b.decode(errors='replace'))
  def text():return '\n'.join(screen.display)
  def wait(label):
   deadline=time.monotonic()+8
   while label not in text() and time.monotonic()<deadline:read()
   assert label in text(),text()
  try:
   read(1);os.write(master,b'a');wait('Add by SSH address')
   assert 'Enter Confirm' not in text() and 'Esc Cancel' not in text() and 'Enter confirm' not in text() and 'Escape cancel' not in text()
   os.write(master,b'fixture@example.test\r');wait('Verify host key' if kind=='host' else 'SSH password')
   if kind=='host':
    # Fingerprint wrapping at 48 columns; scroll to inspect remaining text.
    os.write(master,b'\x1b[B'*12);read();assert 'Trust fingerprint' in text();os.write(master,b'y')
   elif kind=='cancel':os.write(master,b'\x1b')
   else:
    os.write(master,b'fixture-secret-937');read();assert 'fixture-secret-937' not in text();assert '********' in text();os.write(master,b'\r')
   if kind=='cancel':
    # Cancellation may kill the owned SSH group before the fixture can acknowledge it.
    wait('authentication cancelled')
    fixture_pid=(root/'result.pid').read_text()
    deadline=time.monotonic()+2
    while pathlib.Path(f'/proc/{fixture_pid}').exists() and time.monotonic()<deadline:read(.05)
    assert not pathlib.Path(f'/proc/{fixture_pid}').exists(), 'cancelled SSH process survived'
    if (root/'result').exists():assert (root/'result').read_text()=='PASS'
   else:
    deadline=time.monotonic()+8
    while not (root/'result').exists() and time.monotonic()<deadline:read()
    assert (root/'result').read_text()=='PASS'
   assert b'fixture-secret-937' not in output,'secret leaked into terminal output'
   read();os.write(master,b'\x03');p.wait(timeout=5)
   assert termios.tcgetattr(slave)==before
   assert not list(pathlib.Path('/tmp').glob(f'cx-askpass-{p.pid}-*'))
   results.append(dict(width=width,scenario=kind,result='PASS'))
  finally:
   if p.poll() is None:p.kill();p.wait()
   os.close(master);os.close(slave)
print(json.dumps(results))
