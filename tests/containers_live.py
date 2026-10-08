#!/usr/bin/env python3
"""Owned Docker fixtures only. Inspect, scope, lifecycle and native PTY passthrough."""
import base64,fcntl,json,os,pathlib,pty,re,select,struct,subprocess,sys,tempfile,termios,time
binary=str(pathlib.Path(sys.argv[1]).resolve())
checks=[];owned=[]
def docker(*args,check=True):return subprocess.run(['docker',*args],capture_output=True,text=True,check=check,timeout=60)
with tempfile.TemporaryDirectory(prefix='cx-containers-') as tmp:
 root=pathlib.Path(tmp);(root/'.local/bin').mkdir(parents=True);(root/'.local/bin/cx').symlink_to(binary); env=dict(os.environ,HOME=tmp,XDG_STATE_HOME=tmp+'/state',SHELL='/bin/bash',TERM='xterm-256color',COLORTERM='truecolor')
 for key in ['TMUX','TMUX_PANE']:env.pop(key,None)
 def cli(*args):return json.loads(subprocess.check_output([binary,*args],env=env,timeout=100))
 def request(op,args):
  data=json.dumps(dict(version=1,id='container-proof',op=dict(op=op,args=args))).encode()
  reply=subprocess.check_output([binary,'helper'],input=b'CX1 '+str(len(data)).encode()+b'\n'+data,env=env,timeout=100)
  return json.loads(reply.split(b'\n',1)[1])
 def container(name,label=True,*extra):
  args=['run','-d','--network','none','--name','cx-proof-'+str(os.getpid())+'-'+name]
  if label:args+=['--label','devcontainer.metadata=[{"remoteUser":"root"}]']
  args+=list(extra)+['ubuntu:24.04','sleep','infinity'];id=docker(*args).stdout.strip();owned.append(id);return id
 def info(id):return request('container_inspect',dict(id=id))['result']
 def alive_session(id):return id in {s['id'] for s in cli('sessions')}
 try:
  a=container('dev'); b=container('service',False); ro=container('readonly',True,'--read-only')
  docker('exec',a,'sh','-c','printf "# Container scope\\n\\n[other](other.md)\\n" > /tmp/context.md; printf "container-only" > /tmp/other.md; touch /tmp/cx-completion-proof')
  (root/'context.md').write_text('HOST SENTINEL')
  entries=cli('containers')['containers'];by_id={c['id']:c for c in entries}
  assert by_id[a]['devcontainer'] and not by_id[b]['devcontainer'] and not by_id[b]['allowed']
  # Ordinary services cannot be executed or started until individually opted in.
  c=info(b);scope=dict(engine=c['engine'],id=c['id'],name=c['name'],user=c['user'],folder=c['folder'],started_at=c['started_at'])
  assert request('container_files',dict(scope=scope,operation=dict(op='list',args=dict(path='/'))))['error']
  assert request('container_lifecycle',dict(id=b,engine=c['engine'],started_at=c['started_at'],action='stop'))['error']
  listing=cli('container-files',a,'--path','/tmp')
  assert 'context.md' in {e['name'] for e in listing['entries']}
  preview=cli('container-files',a,'--path','/tmp/context.md','--preview');assert 'Container scope' in preview['text']
  docker('exec',a,'sh','-c',r"mkdir /tmp/$(printf '\377')")
  raw=next(e for e in cli('container-files',a,'--path','/tmp')['entries'] if e['kind']=='directory' and e['path'].startswith('cx-bytes:'));
  assert 'entries' in cli('container-files',a,'--path',raw['path'])
  c=info(a);scope=dict(engine=c['engine'],id=c['id'],name=c['name'],user=c['user'],folder=c['folder'],started_at=c['started_at'])
  assert request('container_files',dict(scope=scope,operation=dict(op='remove',args=dict(path='/tmp/context.md',expected_identity=None))))['error']
  nested=dict(op='container_files',args=dict(scope=scope,operation=dict(op='info')))
  assert request('container_files',dict(scope=scope,operation=nested))['error']
  # Read-only image refuses installation; host file is never substituted.
  c_ro=info(ro);ro_scope=dict(engine=c_ro['engine'],id=c_ro['id'],name=c_ro['name'],user=c_ro['user'],folder=c_ro['folder'],started_at=c_ro['started_at'])
  assert request('container_files',dict(scope=ro_scope,operation=dict(op='list',args=dict(path='/tmp'))))['error']
  stale=dict(scope,engine='different-engine')
  assert request('container_files',dict(scope=stale,operation=dict(op='list',args=dict(path='/tmp'))))['error']
  assert request('container_lifecycle',dict(id=a,engine='different-engine',started_at=c['started_at'],action='stop'))['error']
  cli('container-access',b);assert info(b)['allowed']
  assert 'entries' in cli('container-files',b,'--path','/tmp')
  cli('container-access',b,'--disable');assert not info(b)['allowed']
  cli('container-stop',a);cli('container-start',a)
  assert request('container_files',dict(scope=scope,operation=dict(op='list',args=dict(path='/tmp'))))['error'],'stale lifecycle accepted'
  assert info(a)['network']=='none'
  checks.append('devcontainer classification, inspection-only services, real scoped markdown/files, mutation/nesting/readonly refusal, exact resume and stale lifecycle rejection')
  session=cli('container-new',a,'--directory','/tmp');sock=str(root/'state/cx/managed.sock')
  def tmux(*args):return subprocess.check_output(['tmux','-S',sock,*args],env=env,timeout=5)
  tmux('set-environment','-g','HOME',tmp);tmux('set-environment','-g','XDG_STATE_HOME',tmp+'/state')
  m,t=pty.openpty();fcntl.ioctl(t,termios.TIOCSWINSZ,struct.pack('HHHH',24,100,0,0));before=termios.tcgetattr(t)
  def ctl():os.setsid();fcntl.ioctl(t,termios.TIOCSCTTY,0)
  p=subprocess.Popen([binary,'native-attach','attach-session','-t','='+session['id']],env=env,stdin=t,stdout=t,stderr=t,preexec_fn=ctl)
  out=bytearray()
  def read(seconds):
   until=time.monotonic()+seconds
   while time.monotonic()<until:
    if select.select([m],[],[],.02)[0]:out.extend(os.read(m,65536))
  def send(data):os.write(m,data);read(.3)
  def wait(predicate):
   deadline=time.monotonic()+8
   while not predicate() and time.monotonic()<deadline:read(.1)
   assert predicate(), 'container terminal proof failed: '+tmux('display-message','-p','-t',session['id'],'#{history_size} #{alternate_on} #{mouse_any_flag} #{pane_in_mode} #{pane_width} #{pane_height}').decode()
  def pane():return tmux('capture-pane','-p','-t',session['id']+':0.0')
  try:
   read(1);assert p.poll() is None
   send(b'printf "%s\\n" "$BASHPID" > /tmp/cx-terminal.pid\n')
   # Bracketed paste stays literal; typed Tab performs actual shell completion.
   send(b"printf '\\nCX_PASTE_PROOF:%s\\n' 'space and quotes'\n")
   wait(lambda:b'CX_PASTE_PROOF:space and quotes' in pane())
   send(b'printf "CX_TAB_PROOF:%s\\n" /tmp/cx-comple\t\n')
   wait(lambda:b'CX_TAB_PROOF:/tmp/cx-completion-proof' in pane())
   out.clear();send(b"printf '\\033]52;c;Y3gtY29udGFpbmVyLWNvcHk=\\007'\n")
   assert re.search(rb'\x1b\]52;[^;]*;Y3gtY29udGFpbmVyLWNvcHk=(?:\x07|\x1b\\)',out),'OSC52 not passed through'
   out.clear();send(b"printf '\\033[38;2;12;34;56mCX_RGB_PROOF\\033[39;49m\\n'\n")
   assert b'CX_RGB_PROOF' in out and (b'38;2;12;34;56' in out or b'38;2;12;34;56' in tmux('capture-pane','-e','-p','-t',session['id'])),'RGB color lost'
   assert b'48;2;' not in out,'transparent/default background unexpectedly changed'
   send(b"for i in $(seq 1 80); do printf 'CX_SCROLL_%s\\n' \"$i\"; done\n")
   wait(lambda:b'CX_SCROLL_80' in pane())
   send(b'\x1b[<64;40;10M');wait(lambda:tmux('display-message','-p','-t',session['id'],'#{pane_in_mode}').strip()==b'1')
   send(b'\x1b');wait(lambda:tmux('display-message','-p','-t',session['id'],'#{pane_in_mode}').strip()==b'0');send(b'\x1d')
   deadline=time.monotonic()+5
   while p.poll() is None and time.monotonic()<deadline:read(.1)
   assert p.poll()==0,'one detach did not return'
   assert p.returncode==0 and termios.tcgetattr(t)==before and alive_session(session['id'])
   live=next(s for s in cli('sessions') if s['id']==session['id'])
   fields={k:live[k] for k in ['id','pid','started','boot_id']}
   assert request('stop_session',fields)['result']=={'status':'stopped'}
   assert not alive_session(session['id'])
   assert docker('exec',a,'sh','-c','test ! -d /proc/$(cat /tmp/cx-terminal.pid)',check=False).returncode==0,'container shell survived CX stop'
   checks.append('native container paste, Tab completion, OSC52, RGB/default background, wheel scrollback, one detach/termios, stop terminates container shell')
  finally:
   if p.poll() is None:p.kill();p.wait()
   os.close(m);os.close(t)
 finally:
  subprocess.run(['tmux','-S',str(root/'state/cx/managed.sock'),'kill-server'],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
  for id in owned:docker('rm','-f',id,check=False)
print(json.dumps(dict(result='PASS',checks=checks,scope='only owned network-none Docker fixtures')))
