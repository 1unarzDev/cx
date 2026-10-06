#!/usr/bin/env python3
"""Opt-in SSH file acceptance on an enrolled agent-free host; owns temporary fixtures only."""
import hashlib, json, os, pathlib, shlex, subprocess, sys, tempfile, time, uuid

binary = str(pathlib.Path(sys.argv[1]).resolve())
alias = sys.argv[2]
results = []
remote_dir = None
def rpc(op, remote=False, env=None):
    request = json.dumps(dict(version=1, id='browser-test', op=op)).encode()
    command = ['ssh', '-o', 'BatchMode=yes', '-o', 'ConnectTimeout=5', alias, 'exec ~/.local/bin/cx helper'] if remote else [binary, 'helper']
    done = subprocess.run(command, input=b'CX1 '+str(len(request)).encode()+b'\n'+request, capture_output=True, timeout=30, env=env)
    assert done.returncode == 0, done.stderr.decode(errors='replace')
    at = done.stdout.index(b'CX1 '); size, data = done.stdout[at+4:].split(b'\n', 1)
    response = json.loads(data[:int(size)])
    assert not response.get('error'), response
    return response['result']
def inventory():
    return subprocess.check_output(['ssh','-o','BatchMode=yes','-o','ConnectTimeout=5',alias,
        'command -v claude || true; command -v codex || true'], timeout=10).decode().strip()
before_agents = inventory()
assert not before_agents, 'This scenario requires an agent-free fixture host'
metadata = rpc({'op':'info'}, True)
device = dict(id=metadata['machine_id']+':'+metadata['account'], name=alias, target=alias,
    account=metadata['account'], host=metadata['host'], status='unknown', observed_at=0)
remote_dir = subprocess.check_output(['ssh','-o','BatchMode=yes','-o','ConnectTimeout=5',alias,
    'umask 077; mktemp -d ~/.local/state/cx/browser-live.XXXXXX'], timeout=10).decode().strip()
try:
    with tempfile.TemporaryDirectory(prefix='cx-remote-files-') as directory:
        root=pathlib.Path(directory); state=root/'state/cx'; state.mkdir(parents=True)
        local=dict(id='local',name='viewer',target=None,account=os.environ['USER'],host=os.uname().nodename,status='local',observed_at=0)
        (state/'devices.json').write_text(json.dumps([local,device]))
        env=dict(os.environ,XDG_STATE_HOME=str(root/'state'))
        source=root/"-recording' α.bin"; content=b'synthetic remote recording\x00'*4096; source.write_bytes(content)
        def copy(path,destination,src=None,dst=None,cut=False):
            key='browser-'+uuid.uuid4().hex
            args=[binary,'copy',str(path),str(destination),'--conflict','rename','--key',key]
            if src:args+=['--source-device',src]
            if dst:args+=['--destination-device',dst]
            if cut:args+=['--cut']
            done=subprocess.run(args,capture_output=True,text=True,env=env,timeout=30)
            assert done.returncode==0,done.stderr
            deadline=time.monotonic()+30; job=None
            while time.monotonic()<deadline:
                rows=json.loads(subprocess.check_output([binary,'jobs'],env=env,timeout=30))['jobs']
                job=next((j for j in rows if j['key']==key),None)
                if job and job['status'] in ['complete','failed','cancelled']:break
                time.sleep(.2)
            assert job and job['status']=='complete',job
            return job
        first=copy(source,remote_dir,dst=alias)
        assert first['copied']==1
        second=copy(source,remote_dir,dst=alias)
        remote_file=second['actual_destinations'][0]
        assert remote_file.endswith('.copy-1'),second
        remote_info=rpc({'op':'file_info','args':{'path':remote_file}},True)
        chunk=rpc({'op':'read_chunk','args':{'path':remote_file,'offset':0,'limit':len(content),'identity':remote_info['identity']}},True)
        import base64
        assert hashlib.sha256(base64.b64decode(chunk['data'])).digest()==hashlib.sha256(content).digest()
        results.append(dict(scenario='local → SSH copy, opaque punctuation/Unicode, integrity and rename conflicts',result='PASS',host=alias))
        receive=root/'receive'; receive.mkdir()
        moved=copy(remote_file,receive,src=alias,cut=True)
        assert rpc({'op':'file_info','args':{'path':remote_file}},True)['kind']=='missing'
        assert (receive/pathlib.Path(remote_file).name).read_bytes()==content
        results.append(dict(scenario='SSH → local verified cut retains exact bytes and removes exact source',result='PASS',host=alias,route=moved['route']))
        tree=remote_dir+'/tree'; output=remote_dir+'/output'
        rpc({'op':'mkdir','args':{'path':tree}},True); rpc({'op':'mkdir','args':{'path':output}},True)
        tree_move=copy(tree,output,src=alias,dst=alias,cut=True)
        assert rpc({'op':'file_info','args':{'path':tree}},True)['kind']=='missing'
        assert rpc({'op':'file_info','args':{'path':output+'/tree'}},True)['kind']=='directory'
        info=rpc({'op':'file_info','args':{'path':output+'/tree'}},True)
        rpc({'op':'rename','args':{'path':output+'/tree','name':'renamed tree','expected_identity':info['identity']}},True)
        info=rpc({'op':'file_info','args':{'path':output+'/renamed tree'}},True)
        rpc({'op':'remove','args':{'path':output+'/renamed tree','expected_identity':info['identity']}},True)
        results.append(dict(scenario='SSH same-host directory move, rename and identity-guarded deletion',result='PASS',host=alias))
finally:
    if remote_dir:
        info=rpc({'op':'file_info','args':{'path':remote_dir}},True)
        rpc({'op':'remove','args':{'path':remote_dir,'expected_identity':info['identity']}},True)
assert inventory()==before_agents
results.append(dict(scenario='agent inventory unchanged; fixture cleanup',result='PASS',host=alias))
evidence=pathlib.Path('local-evidence/browser-remote-live'); evidence.mkdir(parents=True,exist_ok=True)
(evidence/'results.json').write_text(json.dumps(results,indent=2));print(json.dumps(results))
