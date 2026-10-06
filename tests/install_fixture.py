#!/usr/bin/env python3
"""Offline installer transaction fixtures; mocked provenance is NOT signature validation."""
import fcntl,io,json,os,pathlib,subprocess,tarfile,tempfile,concurrent.futures
script=str(pathlib.Path('install.sh').resolve())
with tempfile.TemporaryDirectory(prefix='cx-install-test-') as directory:
 root=pathlib.Path(directory);home=root/'home';bins=home/'.local/bin';bins.mkdir(parents=True)
 fake=root/'tools';fake.mkdir();state=home/'.local/state/cx';state.mkdir(parents=True)
 target=bins/'cx';old=b'#!/bin/sh\nprintf "cx 0.0.1\\n"\n';target.write_bytes(old);target.chmod(0o700)
 archive=root/'artifact.tar.gz'
 body=b'#!/bin/sh\nsleep 0.2\nprintf "cx 9.9.9\\n"\n'
 with tarfile.open(archive,'w:gz') as tar:
  entry=tarfile.TarInfo('cx');entry.size=len(body);entry.mode=0o700;tar.addfile(entry,io.BytesIO(body))
 (fake/'curl').write_text('''#!/usr/bin/python3
import os,sys,pathlib
if os.environ.get('CX_FIXTURE_OFFLINE'):sys.exit(7)
args=sys.argv[1:];dest=pathlib.Path(args[args.index('-o')+1])
url=next(arg for arg in args if arg.startswith('https://'))
if 'releases/latest' in url:dest.write_text('{\\n  "tag_name": "v9.9.9",\\n}\\n')
elif url.endswith('.intoto.jsonl'):dest.write_bytes(b'synthetic bundle')
else:dest.write_bytes(pathlib.Path(os.environ['CX_FIXTURE_ARCHIVE']).read_bytes())
''');(fake/'curl').chmod(0o700)
 (fake/'gh').write_text('#!/bin/sh\nexit 0\n');(fake/'gh').chmod(0o700)
 env=dict(os.environ,HOME=str(home),XDG_STATE_HOME=str(home/'.local/state'),PATH=str(fake)+':/usr/bin:/bin',CX_FIXTURE_ARCHIVE=str(archive))
 env.pop('CX_REPO',None);env.pop('CX_VERSION',None);env.pop('CX_PREFIX',None)
 def run(extra=None):return subprocess.run(['sh',script],env=dict(env,**(extra or {})),stdin=subprocess.DEVNULL,capture_output=True,text=True,timeout=10)
 outcomes=[]
 result=run({'CX_FIXTURE_OFFLINE':'1'});assert result.returncode!=0 and target.read_bytes()==old;outcomes.append('offline preserves installed binary')
 result=run({'CX_REPO':'someone/fork'});assert result.returncode!=0 and target.read_bytes()==old;outcomes.append('alternate trust root refused')
 with (state/'maintenance.lock').open('a') as lock:
  fcntl.flock(lock,fcntl.LOCK_EX|fcntl.LOCK_NB)
  inode=os.stat(state/'maintenance.lock').st_ino
  result=run();assert result.returncode!=0 and target.read_bytes()==old
  assert os.stat(state/'maintenance.lock').st_ino==inode
 outcomes.append('shared maintenance lock preserves binary and lock inode')
 with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:
  results=list(pool.map(lambda _:run(),range(2)))
 assert sorted(result.returncode for result in results)==[0,1],[(r.returncode,r.stderr) for r in results]
 assert target.read_bytes()==body and not list(bins.glob('.cx-install.*'))
 outcomes.append('concurrent installers use unique staging and serialized atomic replace')
 target.unlink();target.symlink_to(root/'external')
 # This case should be refused before changing the target resource.
 result=run();assert result.returncode!=0 and target.is_symlink()
 outcomes.append('symlink destination refused')
 print(json.dumps({'result':'PASS','scenarios':outcomes,'provenance':'MOCKED; real published verification remains untested'}))
