#!/usr/bin/env python3
"""Real scoped file I/O and transfers on owned network-none Docker containers.
Remote hops are private fake SSH pipes into real CX helpers; no fleet contact.
"""
import base64, hashlib, json, os, pathlib, shlex, subprocess, sys, tempfile, time
binary=str(pathlib.Path(sys.argv[1]).resolve())
owned=[];checks=[]
def docker(*args,check=True):
 return subprocess.run(['docker',*args],capture_output=True,check=check,timeout=60)
with tempfile.TemporaryDirectory(prefix='cx-container-transfers-') as tmp:
 root=pathlib.Path(tmp);state=root/'state/cx';state.mkdir(parents=True)
 env=dict(os.environ,HOME=tmp,XDG_STATE_HOME=str(root/'state'),SHELL='/bin/sh',HTTPS_PROXY='http://127.0.0.1:9')
 for key in ['TMUX','TMUX_PANE']:env.pop(key,None)
 def request(op,args=None):
  data=json.dumps(dict(version=1,id='transfer-proof',op=dict(op=op,args=args))).encode()
  p=subprocess.run([binary,'helper'],input=b'CX1 '+str(len(data)).encode()+b'\n'+data,capture_output=True,env=env,timeout=100)
  assert p.returncode==0,p.stderr[-500:]
  return json.loads(p.stdout.split(b'\n',1)[1])
 def result(op,args=None):
  r=request(op,args);assert not r['error'],r['error'];return r['result']
 def make(name,dev=True):
  args=['run','-d','--network','none','--name',f'cx-transfer-proof-{os.getpid()}-{name}']
  if dev:args+=['--label','devcontainer.metadata=[{"remoteUser":"root"}]']
  id=docker(*args,'ubuntu:24.04','sleep','infinity').stdout.decode().strip();owned.append(id);return id
 def scope(id):
  c=result('container_inspect',dict(id=id));return {k:c[k] for k in ['engine','id','name','user','started_at','folder']}
 def io(c,op,**args):return result('container_file_action',dict(scope=c,operation=dict(op=op,args=args)))
 def worker(spec,success=True):
  path=root/(spec['key']+'.json');path.write_text(json.dumps(spec));path.chmod(0o600)
  p=subprocess.run([binary,'transfer-worker',str(path)],env=env,capture_output=True,timeout=180)
  assert (p.returncode==0)==success,(spec['key'],p.stderr.decode()[-1200:])
  return p
 def spec(key,src,dst,sc=None,dc=None,cut=False,conflict='rename',remote=False):
  a=dict(device,id='source-fixture',target='source-fixture.invalid' if remote else None)
  b=dict(device,id='destination-fixture' if remote else 'source-fixture',target='destination-fixture.invalid' if remote else None)
  return dict(source=a,source_container=sc,source_path=str(src),destination=b,destination_container=dc,destination_path=str(dst),key=key+'-'+str(os.getpid()),cut=cut,conflict=conflict,source_identity=None)
 try:
  a=make('a');b=make('b');service=make('service',False);sa=scope(a);sb=scope(b);ss=scope(service)
  info=result('info');device=dict(id='local',name='fixture',account=info['account'],host=info['host'],target=None,status='local',observed_at=0)
  payload=bytes(range(256))*1537;host=root/'payload.bin';host.write_bytes(payload);host.chmod(0o640)
  docker('exec',a,'mkdir','-p','/tmp/in','/tmp/tree/nested','/tmp/tree/empty')
  docker('exec',b,'mkdir','-p','/tmp/in')
  worker(spec('host-to-container',host,'/tmp/in',dc=sa))
  assert docker('exec',a,'cat','/tmp/in/payload.bin').stdout==payload
  assert docker('exec',a,'stat','-c','%a','/tmp/in/payload.bin').stdout.strip()==b'640'
  out=root/'out';out.mkdir();worker(spec('container-to-host','/tmp/in/payload.bin',out,sc=sa))
  assert (out/'payload.bin').read_bytes()==payload
  worker(spec('container-to-container','/tmp/in/payload.bin','/tmp/in',sc=sa,dc=sb))
  assert docker('exec',b,'cat','/tmp/in/payload.bin').stdout==payload
  checks.append('host/container and container/container chunk streaming, exact same paths isolated, modes/checksums')
  # Partial receive is staged on destination, then resumed by the real worker.
  rs=spec('resume','/tmp/in/payload.bin','/tmp/resumed.bin',sc=sa,dc=sb)
  meta=io(sa,'file_info',path=rs['source_path']);dest=rs['destination_path']
  k=rs['key'].encode();receive='cx-'+hashlib.sha256(len(k).to_bytes(8,'big')+k+dest.encode()).hexdigest()
  io(sb,'receive_prepare',path=dest,key=receive,source_identity=meta['identity'],total=len(payload),conflict='rename',mode=0o640)
  io(sb,'receive_chunk',key=receive,offset=0,data=base64.b64encode(payload[:131072]).decode())
  assert docker('exec',b,'test','-e',dest,check=False).returncode!=0
  worker(rs);assert docker('exec',b,'cat',dest).stdout==payload
  worker(rs)
  checks.append('destination-only partial staging, resume prefix verification and idempotent receipt')
  # Fake SSH endpoints retain actual framing, helper deployment and Docker I/O.
  bindir=root/'bin';bindir.mkdir();ssh=bindir/'ssh'
  ssh.write_text('#!/bin/sh\nfor arg do last="$arg"; done\ncase "$last" in\n "exec ~/.local/bin/cx helper") exec '+shlex.quote(binary)+' helper ;;\n *) exit 1 ;;\nesac\n');ssh.chmod(0o700)
  env['PATH']=str(bindir)+os.pathsep+env['PATH']
  relay=root/'relay.bin';worker(spec('relay-two-devices','/tmp/in/payload.bin',relay,sc=sa,remote=True));assert relay.read_bytes()==payload
  checks.append('two simulated SSH devices via actual helper framing and container adapter; no network')
  cut=spec('cut-file','/tmp/in/payload.bin',root/'cut-file.bin',sc=sa,cut=True)
  worker(cut);assert (root/'cut-file.bin').read_bytes()==payload
  assert docker('exec',a,'test','-e','/tmp/in/payload.bin',check=False).returncode!=0
  # A skipped cut never removes its source.
  (root/'retained.bin').write_bytes(b'existing');docker('exec',a,'sh','-c','printf retained > /tmp/retained.bin')
  worker(spec('skip-cut','/tmp/retained.bin',root/'retained.bin',sc=sa,cut=True,conflict='skip'),False)
  assert docker('exec',a,'cat','/tmp/retained.bin').stdout==b'retained'
  # Copy whole trees, preserve symlinks and move only verified entries.
  docker('exec',a,'sh','-c','printf tree-content > /tmp/tree/nested/data; ln -s nested/data /tmp/tree/link')
  tree=root/'trees';tree.mkdir();worker(spec('cut-tree','/tmp/tree',tree,sc=sa,cut=True))
  assert (tree/'tree/nested/data').read_bytes()==b'tree-content'
  assert (tree/'tree/link').is_symlink() and os.readlink(tree/'tree/link')=='nested/data'
  assert (tree/'tree/empty').is_dir()
  assert docker('exec',a,'test','-e','/tmp/tree',check=False).returncode!=0
  checks.append('verified file/tree cuts, symlinks/empty directories, skip retains source')
  # An external addition during cleanup is retained; retry resumes our own removals.
  docker('exec',a,'sh','-c','mkdir /tmp/retry-tree; for n in $(seq 1 20); do printf original > /tmp/retry-tree/file-$n; done')
  retry_spec=spec('cut-tree-retry','/tmp/retry-tree',root/'retry-dest',sc=sa,cut=True)
  retry_spec['source_identity']=io(sa,'file_info',path='/tmp/retry-tree')['identity']
  spec_file=root/'cut-tree-retry.json';spec_file.write_text(json.dumps(retry_spec))
  saved=state/('transfers/'+retry_spec['key']+'.spec.json');saved.write_text(json.dumps(retry_spec));saved.chmod(0o600)
  process=subprocess.Popen([binary,'transfer-worker',str(spec_file)],env=env,stdout=subprocess.PIPE,stderr=subprocess.PIPE)
  record=state/('transfers/'+retry_spec['key']+'.job.json');end=time.monotonic()+120;added=False
  while time.monotonic()<end and process.poll() is None:
   try:job=json.loads(record.read_text())
   except (FileNotFoundError,json.JSONDecodeError):job={}
   if any(e['status']=='removed' for e in job.get('entries',[])):
    docker('exec',a,'sh','-c','printf unverified > /tmp/retry-tree/new-file');added=True;break
   time.sleep(.01)
  stdout,stderr=process.communicate(timeout=120)
  assert added and process.returncode!=0,(added,stderr[-800:])
  assert docker('exec',a,'cat','/tmp/retry-tree/new-file').stdout==b'unverified'
  assert (root/'retry-dest/file-1').read_bytes()==b'original'
  docker('exec',a,'rm','/tmp/retry-tree/new-file')
  result('transfer_retry',dict(key=retry_spec['key']))
  end=time.monotonic()+90
  while time.monotonic()<end:
   job=next(j for j in result('transfer_jobs')['jobs'] if j['key']==retry_spec['key'])
   if job['status'] in ['complete','failed']:break
   time.sleep(.2)
  assert job['status']=='complete',job
  assert docker('exec',a,'test','-e','/tmp/retry-tree',check=False).returncode!=0
  checks.append('unverified source addition retained during tree cleanup, durable public retry after own directory timestamp changes')

  # Identity, policy and nesting refusal cannot fall back to host paths.
  stale=dict(sa,started_at='stale');sentinel=root/'sentinel';sentinel.write_bytes(b'host untouched')
  worker(spec('stale-scope','/tmp/retained.bin',sentinel,sc=stale,conflict='overwrite'),False);assert sentinel.read_bytes()==b'host untouched'
  worker(spec('service-refused',host,'/tmp/payload.bin',dc=ss),False)
  nested=dict(op='container_file_action',args=dict(scope=sa,operation=dict(op='info')))
  assert request('container_file_action',dict(scope=sa,operation=nested))['error']
  bad=request('transfer',spec('legacy-refused',host,'/tmp/payload.bin',dc=sb));assert bad['error'] and 'scoped_transfer' in bad['error']
  # Permissions are the actual selected container user's permissions.
  io(sa,'mkdir',path='/tmp/actions')
  docker('exec',a,'sh','-c','printf action > /tmp/actions/a')
  m=io(sa,'file_info',path='/tmp/actions/a')
  io(sa,'rename',path='/tmp/actions/a',name='b',expected_identity=m['identity'])
  m=io(sa,'file_info',path='/tmp/actions/b');io(sa,'remove',path='/tmp/actions/b',expected_identity=m['identity'])
  assert request('container_file_action',dict(scope=sa,operation=dict(op='remove',args=dict(path='/tmp/retained.bin',expected_identity=None))))['error']
  missing_folder=dict(sa,folder='/tmp/no-longer-present-workspace')
  assert 'entries' in result('container_files',dict(scope=missing_folder,operation=dict(op='list',args=dict(path='/tmp'))))
  relative_folder=dict(sa,folder='/tmp/actions')
  io(relative_folder,'mkdir',path='relative-directory')
  assert docker('exec',a,'test','-d','/tmp/actions/relative-directory').returncode==0
  checks.append('stale lifecycle, ordinary-service isolation, nested/unguarded/legacy protocol rejection, scoped mkdir/rename/delete')
  # Starting through the public CLI creates durable detached progress.
  public_cli_key='public-cli-'+str(os.getpid())
  p=subprocess.run([binary,'copy',str(host),'/tmp/cli.bin','--destination-container',b,'--key',public_cli_key],env=env,capture_output=True,timeout=120)
  assert p.returncode==0,p.stderr.decode()[-1000:]
  end=time.monotonic()+90
  while time.monotonic()<end:
   jobs=result('transfer_jobs')['jobs'];job=next((j for j in jobs if j['key']==public_cli_key),None)
   if job and job['status'] in ['complete','failed']:break
   time.sleep(.2)
  assert job and job['status']=='complete',job
  assert docker('exec',b,'cat','/tmp/cli.bin').stdout==payload
  assert '/' in job['destination_host'] and job['operation']=='copy'
  checks.append('public CLI scoped flags, detached worker and named durable progress')
 finally:
  for id in reversed(owned):docker('rm','-f',id,check=False)
print(json.dumps(dict(result='PASS',checks=checks,scope='owned network-none containers/private CX state/fake SSH; no robot or live networking changes')))
