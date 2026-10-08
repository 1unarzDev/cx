#!/usr/bin/env python3
"""Owned network-none container; synthetic native agents only, no inference."""
import fcntl,json,os,pathlib,pty,select,struct,subprocess,sys,tempfile,termios,time
binary=str(pathlib.Path(sys.argv[1]).resolve())
with tempfile.TemporaryDirectory(prefix='cx-container-agents-') as tmp:
 root=pathlib.Path(tmp);(root/'.local/bin').mkdir(parents=True);(root/'.local/bin/cx').symlink_to(binary)
 env=dict(os.environ,HOME=tmp,XDG_STATE_HOME=tmp+'/state',SHELL='/bin/bash',TERM='xterm-256color');env.pop('TMUX',None);env.pop('TMUX_PANE',None)
 def docker(*args):return subprocess.check_output(['docker',*args],timeout=60)
 def cli(*args):return json.loads(subprocess.check_output([binary,*args],env=env,timeout=100))
 def request(op,args):
  data=json.dumps(dict(version=1,id='agents-proof',op=dict(op=op,args=args))).encode()
  return json.loads(subprocess.check_output([binary,'helper'],input=b'CX1 '+str(len(data)).encode()+b'\n'+data,env=env,timeout=100).split(b'\n',1)[1])
 source=root/'agent.c';source.write_text(r'''
#include <stdio.h>
#include <unistd.h>
#include <string.h>
#include <termios.h>
int main(int argc,char **argv){
 if(argc>1 && !strcmp(argv[1],"--version")){puts("fixture");return 0;}
 FILE *f=fopen("/tmp/args","w");fprintf(f,"%s",argv[0]);for(int i=1;i<argc;i++)fprintf(f," %s",argv[i]);fclose(f);
 struct termios t;tcgetattr(0,&t);cfmakeraw(&t);tcsetattr(0,TCSANOW,&t);
 printf("\033[?1049h\033[?2004hSYNTHETIC_CONTAINER_AGENT\r\n");fflush(stdout);
 FILE *keys=fopen("/tmp/keys","w");char buf[1024];ssize_t n;while((n=read(0,buf,sizeof(buf)))>0){fwrite(buf,1,n,keys);fflush(keys);}return 0;
}''')
 subprocess.check_call(['cc','-static',str(source),'-o',str(root/'agent')])
 id=docker('run','-d','--network','none','--name',f'cx-agent-proof-{os.getpid()}','--label','devcontainer.metadata=[{"remoteUser":"root"}]','ubuntu:24.04','sleep','infinity').decode().strip()
 sock=tmp+'/state/cx/managed.sock';checks=[]
 def tmux(*args):return subprocess.check_output(['tmux','-S',sock,*args],env=env,timeout=5)
 try:
  for provider in ['codex','claude']:docker('cp',str(root/'agent'),id+':/usr/local/bin/'+provider)
  for provider,flag in [('codex','--dangerously-bypass-approvals-and-sandbox'),('claude','--dangerously-skip-permissions')]:
   for yolo in [False,True]:
    docker('exec',id,'rm','-f','/tmp/args','/tmp/keys')
    session=cli('container-new',id,'--provider',provider,*(['--yolo'] if yolo else []))
    tmux('set-environment','-g','HOME',tmp);tmux('set-environment','-g','XDG_STATE_HOME',tmp+'/state')
    deadline=time.monotonic()+12
    while time.monotonic()<deadline:
     ready=subprocess.run(['docker','exec',id,'test','-f','/tmp/keys'],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL).returncode==0
     if ready:break
     time.sleep(.1)
    assert ready, 'agent failed to launch'
    argv=docker('exec',id,'cat','/tmp/args').decode();assert argv==provider+(' '+flag if yolo else ''),argv
    m,t=pty.openpty();fcntl.ioctl(t,termios.TIOCSWINSZ,struct.pack('HHHH',24,100,0,0));before=termios.tcgetattr(t)
    def tty():os.setsid();fcntl.ioctl(t,termios.TIOCSCTTY,0)
    p=subprocess.Popen([binary,'native-attach','attach-session','-t','='+session['id']],env=env,stdin=t,stdout=t,stderr=t,preexec_fn=tty)
    def drain(seconds):
     end=time.monotonic()+seconds
     while time.monotonic()<end:
      if select.select([m],[],[],.02)[0]:os.read(m,65536)
    try:
     drain(.8);assert p.poll() is None
     # Genuine bracketed paste, Tab and stale fullscreen Codex mouse policy.
     expected=b'\x1b[200~paste proof\x1b[201~\t';os.write(m,expected);drain(.5)
     if provider=='codex':
      tmux('clear-history','-t',session['id']);event=b'\x1b[<64;40;10M';os.write(m,event);expected+=event;drain(1)
      assert tmux('display-message','-p','-t',session['id'],'#{pane_in_mode}').strip()==b'0'
     actual=docker('exec',id,'cat','/tmp/keys');assert actual==expected,f'{provider} input mismatch: {actual.hex()} vs {expected.hex()}'
     os.write(m,b'\x1d');deadline=time.monotonic()+5
     while p.poll() is None and time.monotonic()<deadline:drain(.1)
     assert p.poll()==0 and termios.tcgetattr(t)==before
     live=next(s for s in cli('sessions') if s['id']==session['id']);assert live['container'] and live['provider']==provider
     fields={k:live[k] for k in ['id','pid','started','boot_id']}
     assert request('stop_agent_session',dict(fields,provider=provider))['result']=={'status':'stopped'}
     assert not any(s['id']==session['id'] for s in cli('sessions'))
    finally:
     if p.poll() is None:p.kill();p.wait()
     os.close(m);os.close(t)
  checks.append('container Codex/Claude Default/YOLO argv, bracketed paste, Tab, Codex namespace wheel fallback, one detach, termios and scoped agent stop')
 finally:
  subprocess.run(['tmux','-S',sock,'kill-server'],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
  subprocess.run(['docker','rm','-f',id],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
print(json.dumps(dict(result='PASS',checks=checks,scope='owned network-none fixture; synthetic ELF agents; no inference')))
