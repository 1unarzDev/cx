#!/usr/bin/env python3
"""Live client-only host acceptance: remote_folder_command_pty.py BINARY SSH_ALIAS.
Creates/removes one remote temporary folder; uses existing SSH trust/authentication.
Run only on an authorized agent-free host. No enrollment or service changes.
"""
import fcntl,json,os,pathlib,pwd,pty,select,socket,struct,subprocess,sys,tempfile,termios,time
binary=str(pathlib.Path(sys.argv[1]).resolve())
remote=sys.argv[2]
assert remote and not remote.startswith('-') and not any(c.isspace() for c in remote)
def ssh(command):
 return subprocess.check_output(['ssh','-o','BatchMode=yes','-o','ConnectTimeout=5','--',remote,command],timeout=20).decode().strip()
account=ssh('id -un'); host=ssh('hostname')
assert not ssh('command -v claude || :; command -v codex || :'), 'host must remain agent-free'
folder=ssh('mktemp -d /tmp/cx-command-live.XXXXXX')
assert folder.startswith('/tmp/cx-command-live.') and folder.replace('/','').replace('.','').replace('-','').isalnum()
ssh("printf fixture > '"+folder+"/marker.txt'")
results=[]
try:
 with tempfile.TemporaryDirectory(prefix='cx-live-viewer-') as root:
  state=pathlib.Path(root)/'cx';state.mkdir(mode=0o700)
  devices=[dict(id='local',name=socket.gethostname(),target=None,account=pwd.getpwuid(os.getuid()).pw_name,host=socket.gethostname(),status='local',observed_at=0),dict(id='live-server',name=remote,target=remote,account=account,host=host,status='unknown',observed_at=0)]
  (state/'devices.json').write_text(json.dumps(devices));(state/'devices.json').chmod(0o600)
  name='viewer-restart-123-789.json'
  snapshot=dict(schema=1,expires_at=int(time.time())+300,device_ids=['local','live-server'],device=2,focus='Workspace',view='Files',selected=0,selected_session=None,side_selected=0,search='',browser=dict(device=1,path=folder,display_path=folder,parent='/tmp',entries=[],selected=0,search='',preview_scroll=0,restore_selection=None),other_browser=None,destination_active=False,conflict=2,launch_provider=None,clipboard=None,submitted={})
  (state/name).write_text(json.dumps(snapshot));(state/name).chmod(0o600)
  env=dict(os.environ,XDG_STATE_HOME=root,TERM='xterm-256color',HTTPS_PROXY='http://127.0.0.1:9')
  env.pop('TMUX',None);env.pop('TMUX_PANE',None)
  master,slave=pty.openpty();before=termios.tcgetattr(slave)
  fcntl.ioctl(slave,termios.TIOCSWINSZ,struct.pack('HHHH',24,100,0,0))
  def tty():os.setsid();fcntl.ioctl(slave,termios.TIOCSCTTY,0)
  proc=subprocess.Popen([binary,'restart',name],stdin=slave,stdout=slave,stderr=slave,env=env,preexec_fn=tty)
  output=bytearray()
  def wait(text):
   end=time.monotonic()+20
   while text.encode() not in output:
    if select.select([master],[],[],.1)[0]:output.extend(os.read(master,65536))
    assert proc.poll() is None,repr(bytes(output)[-2000:])
    assert time.monotonic()<end,repr(bytes(output)[-3000:])
  def send(text):output.clear();os.write(master,text)
  try:
   wait('marker.txt');time.sleep(2)
   send(b'n');wait('New session');wait('Shell');assert b'Claude' not in output and b'Codex' not in output
   send(b'\x1b');time.sleep(.2)
   send(b':');wait('Run command')
   os.write(master,b"printf remote-proof > result.txt\r")
   wait(account+'@'+host);wait('Command ended:');assert ssh("cat '"+folder+"/result.txt'")=='remote-proof'
   send(b'\r');wait('Returned from');wait('result.txt')
   assert os.tcgetpgrp(master)==proc.pid
   send(b':');wait('Run command');os.write(master,b'echo REMOTE_READY; sleep 30\r');wait('REMOTE_READY');time.sleep(.3);os.write(master,b'\x03');wait('Command ended:')
   send(b'\r');wait('Returned from');assert proc.poll() is None
   send(b'\x03');proc.wait(timeout=5);assert proc.returncode==0 and termios.tcgetattr(slave)==before
   results.append(dict(result='PASS',viewer=socket.gethostname(),execution=account+'@'+host,checks=['real remote browser','n offers only Shell on agent-free server','colon pins server folder','native SSH exact write','browser refresh','Ctrl+C preserves viewer','termios/foreground restored']))
  finally:
   if proc.poll() is None:proc.kill();proc.wait(timeout=5)
   os.close(master);os.close(slave)
finally:
 ssh("rm -rf -- '"+folder+"'")
assert not ssh('command -v claude || :; command -v codex || :'), 'agent inventory changed'
print(json.dumps(results))
