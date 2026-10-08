#!/usr/bin/env python3
"""Approved graph paths reach the real route store; rejected paths preserve it."""
import json, os, pathlib, subprocess, sys, tempfile
binary = str(pathlib.Path(sys.argv[1]).resolve())
with tempfile.TemporaryDirectory(prefix='cx-access-') as tmp:
    root=pathlib.Path(tmp);state=root/'cx';state.mkdir(mode=0o700)
    env=dict(os.environ,XDG_STATE_HOME=tmp)
    target='robot@destination'
    (state/'devices.json').write_text(json.dumps([dict(id='robot',name='robot',target=target,account='robot',host='destination',status='unknown',observed_at=0)]))
    graph=dict(revision=1,nodes=[dict(id=i,target=t,role=r) for i,t,r in [('viewer','viewer','mesh'),('gateway','gateway','mesh'),('robot',target,'downstream')]],edges=[dict(from_=a,to=b,allowed=True,authenticated_at=None) for a,b in [('viewer','gateway'),('gateway','robot')]],grants=[dict(from_=a,to=b,allowed=True,authenticated_at=None) for a,b in [('viewer','gateway'),('viewer','robot')]])
    for records in (graph['edges'],graph['grants']):
        for record in records: record['from']=record.pop('from_')
    policy=root/'policy.json'
    def run(install=True):
        policy.write_text(json.dumps(graph))
        return subprocess.run([binary,'access','viewer','robot','--graph',str(policy),*(['--install-route'] if install else [])],env=env,capture_output=True,text=True,timeout=5)
    preview=run(False);assert preview.returncode==0,preview.stderr
    assert not (state/'routes.json').exists()
    result=run();assert result.returncode==0,result.stderr
    route=state/'routes.json';assert json.loads(route.read_text())=={target:['gateway']}
    report=json.loads(result.stdout);assert report['forward']=='not_checked' and report['reverse']=='not_granted' and not report['bidirectional']
    saved=route.read_bytes()
    graph['grants'][0]['allowed']=False
    assert run().returncode!=0 and route.read_bytes()==saved
    graph['grants'][0]['allowed']=True
    graph['nodes'][2]['target']='robot@unenrolled'
    assert run().returncode!=0 and route.read_bytes()==saved
print(json.dumps(dict(result='PASS',review_has_no_mutation=True,approved_route_installed=True,denied_or_unenrolled_preserves_route=True,authorization_not_claimed_authentication=True)))
