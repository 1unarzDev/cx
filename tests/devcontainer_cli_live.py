#!/usr/bin/env python3
"""Explicit CLI workspace startup and exec with owned network-none Docker fixture.
Requires a pinned Dev Containers CLI on PATH; never modifies global npm packages.
"""
import json,os,pathlib,subprocess,sys,tempfile,time
binary=str(pathlib.Path(sys.argv[1]).resolve())
with tempfile.TemporaryDirectory(prefix='cx-cli-workspace-') as tmp:
 root=pathlib.Path(tmp);(root/'.local/bin').mkdir(parents=True);(root/'.local/bin/cx').symlink_to(binary)
 work=root/'project';(work/'.devcontainer').mkdir(parents=True)
 name=f'cx-cli-proof-{os.getpid()}'
 (work/'.devcontainer/devcontainer.json').write_text(json.dumps(dict(name='CX workspace proof',image='ubuntu:24.04',remoteUser='root',runArgs=['--network=none','--name='+name],remoteEnv={'CX_CONFIG_PROOF':'configuration honored'},postCreateCommand='printf hook-proof > /tmp/cx-hook-proof')))
 env=dict(os.environ,HOME=tmp,XDG_STATE_HOME=tmp+'/state',SHELL='/bin/bash',TERM='xterm-256color');env.pop('TMUX',None);env.pop('TMUX_PANE',None)
 def cli(*args):return json.loads(subprocess.check_output([binary,*args],env=env,timeout=660))
 id=None
 try:
  refused=subprocess.run([binary,'devcontainer-up',str(work)],env=env,capture_output=True);assert refused.returncode!=0
  c=cli('devcontainer-up',str(work),'--yes');id=c['id'];assert c['devcontainer'] and c['network']=='none' and c['config']
  assert subprocess.check_output(['docker','exec',id,'cat','/tmp/cx-hook-proof'])==b'hook-proof'
  s=cli('container-new',id);sock=tmp+'/state/cx/managed.sock'
  subprocess.check_call(['tmux','-S',sock,'send-keys','-t',s['id'],'-l','printf "%s" "$CX_CONFIG_PROOF" > /tmp/cx-env-proof'],env=env)
  subprocess.check_call(['tmux','-S',sock,'send-keys','-t',s['id'],'Enter'],env=env)
  deadline=time.monotonic()+12;proof=b''
  while time.monotonic()<deadline:
   p=subprocess.run(['docker','exec',id,'cat','/tmp/cx-env-proof'],capture_output=True)
   proof=p.stdout
   if proof:break
   time.sleep(.1)
  assert proof==b'configuration honored',proof
  assert 'entries' in cli('container-files',id,'--path','/tmp')
 finally:
  subprocess.run(['tmux','-S',tmp+'/state/cx/managed.sock','kill-server'],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
  # Name is unique and owned even if up times out before returning the ID.
  subprocess.run(['docker','rm','-f',id or name],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
print(json.dumps(dict(result='PASS',checks='explicit confirmation, real Node CLI up/hooks, devcontainer evidence, CLI exec remoteEnv, native shell and scoped files',scope='owned network-none workspace')))
