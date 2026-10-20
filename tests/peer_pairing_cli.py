#!/usr/bin/env python3
"""Exercise reciprocal enrollment with two real helpers and disposable SSH transport.

The wrapper substitutes SSH transport, never helper policy/stable identity behavior.
Both endpoints share the test machine-id and use distinct account names, HOME and
state directories; the wire display-name is changed to emulate distinct hosts.
No real network, SSH configuration, trust or credentials change.
"""
import json
import os
import pathlib
import subprocess
import sys
import tempfile


binary = str(pathlib.Path(sys.argv[1]).resolve())
machine = pathlib.Path("/etc/machine-id").read_text().strip()
host = pathlib.Path("/proc/sys/kernel/hostname").read_text().strip()


def device(account, name, target, identity=None):
    return dict(id=identity or f"{machine}:{account}", name=name, target=target,
                account=account, host=host, status="unknown", observed_at=0)


WRAPPER = r'''#!/usr/bin/env python3
import json,os,pathlib,subprocess,sys
args=sys.argv[1:]
assert 'BatchMode=yes' in args, 'pairing must never prompt for authentication'
assert 'ForwardAgent=no' in args, 'pairing must not forward credentials'
assert args[-1]=='exec ~/.local/bin/cx helper', 'fixed helper command required'
target=args[-2]
root=pathlib.Path(os.environ['CX_PAIR_FIXTURE'])
if target=='conception@creation.example': endpoint='creation'; account='conception'
elif target=='tester@innovation.example': endpoint='innovation'; account='tester'
else: raise AssertionError('unexpected fixture SSH target')
mode=os.environ['CX_PAIR_MODE']
if endpoint=='innovation' and mode=='reverse-trust':
    sys.stderr.write('Host key verification failed.\n');sys.exit(255)
env=dict(os.environ,HOME=str(root/endpoint/'home'),XDG_STATE_HOME=str(root/endpoint/'state'),USER=account)
child=subprocess.Popen([os.environ['CX_PAIR_BINARY'],'helper'],env=env,stdin=subprocess.PIPE,stdout=subprocess.PIPE)
def read(stream):
    header=stream.readline()
    if not header:return None
    prefix,size=header.split();assert prefix==b'CX1'
    count=int(size);assert 0<=count<=1048576
    payload=stream.read(count);assert len(payload)==count
    return json.loads(payload)
def write(stream,value):
    payload=json.dumps(value).encode();stream.write(b'CX1 '+str(len(payload)).encode()+b'\n'+payload);stream.flush()
try:
    while True:
        request=read(sys.stdin.buffer)
        if request is None:break
        with (root/'requests.jsonl').open('a') as log:
            log.write(json.dumps(dict(endpoint=endpoint,op=request['op']['op']))+'\n')
        # Distinct machines would have distinct kernel hostnames. Keep actual
        # account/machine identity while avoiding a same-host display-name collision.
        if endpoint=='creation' and request['op']['op']=='peer_enroll':
            request['op']['args']['peer']['name']='fixture-source'
            request['op']['args']['peer']['host']='fixture-source'
        write(child.stdin,request);response=read(child.stdout)
        if endpoint=='creation' and mode=='old-helper' and request['op']['op']=='info':
            response['result']['capabilities']=[c for c in response['result']['capabilities'] if c!='peer-enroll-v1']
        if endpoint=='innovation' and mode=='reverse-identity' and request['op']['op']=='info':
            response['result']['machine_id']='wrong-return-machine'
        write(sys.stdout.buffer,response)
finally:
    child.stdin.close();child.terminate();child.wait(timeout=3)
'''


with tempfile.TemporaryDirectory(prefix="cx-peer-pair-") as temporary:
    root = pathlib.Path(temporary)
    tools = root / "tools"
    tools.mkdir()
    wrapper = tools / "ssh"
    wrapper.write_text(WRAPPER)
    wrapper.chmod(0o700)
    for endpoint in ("innovation", "creation"):
        (root / endpoint / "home/.ssh").mkdir(parents=True)
        (root / endpoint / "home/.ssh/config").write_text("# fixture configuration\n")
        (root / endpoint / "home/.ssh/known_hosts").write_text("# fixture trust\n")
        (root / endpoint / "state/cx").mkdir(parents=True)
        (root / endpoint / "state/cx").chmod(0o700)

    def env(endpoint="innovation", mode="success"):
        return dict(os.environ, HOME=str(root / endpoint / "home"),
                    XDG_STATE_HOME=str(root / endpoint / "state"),
                    USER="tester" if endpoint == "innovation" else "conception",
                    PATH=str(tools) + os.pathsep + os.environ["PATH"],
                    CX_PAIR_FIXTURE=str(root), CX_PAIR_BINARY=binary, CX_PAIR_MODE=mode)

    def state(endpoint, filename):
        return root / endpoint / "state/cx" / filename

    def reset(identity=None):
        for endpoint in ("innovation", "creation"):
            for filename in ("devices.json", "device-policies.json", "routes.json"):
                state(endpoint, filename).unlink(missing_ok=True)
        state("innovation", "devices.json").write_text(json.dumps([
            device("tester", "innovation", None, "local"),
            device("conception", "creation", "conception@creation.example", identity),
        ]))
        (root / "requests.jsonl").write_text("")

    def pair(mode="success", target="tester@innovation.example"):
        return subprocess.run([binary, "device", "pair", "creation", "--return-target", target],
                              env=env(mode=mode), capture_output=True, text=True, timeout=15)

    def policy(endpoint, identity):
        path = state(endpoint, "device-policies.json")
        return json.loads(path.read_text()).get("devices", {}).get(identity, {}).get("access", "directed") if path.exists() else "directed"

    def unchanged_security_files():
        for endpoint in ("innovation", "creation"):
            home = root / endpoint / "home"
            assert (home / ".ssh/config").read_text() == "# fixture configuration\n"
            assert (home / ".ssh/known_hosts").read_text() == "# fixture trust\n"
            assert not state(endpoint, "routes.json").exists(), "pairing changed a route"
            assert sorted(str(p.relative_to(home)) for p in home.rglob("*") if p.is_file()) == [".ssh/config", ".ssh/known_hosts"]

    reset()
    result = pair()
    assert result.returncode == 0, result.stderr
    peers = json.loads(state("creation", "devices.json").read_text())
    reciprocal = [p for p in peers if p["target"] is not None]
    assert len(reciprocal) == 1, peers
    assert reciprocal[0]["id"] == f"{machine}:tester", reciprocal
    assert reciprocal[0]["target"] == "tester@innovation.example", reciprocal
    assert reciprocal[0]["account"] == "tester", reciprocal
    assert policy("innovation", f"{machine}:conception") == "core"
    assert policy("creation", f"{machine}:tester") == "core"
    # Repeating a verified pairing does not duplicate the return endpoint.
    result = pair()
    assert result.returncode == 0, result.stderr
    assert len([p for p in json.loads(state("creation", "devices.json").read_text()) if p["target"]]) == 1
    requests = [json.loads(line) for line in (root / "requests.jsonl").read_text().splitlines()]
    assert any(r == dict(endpoint="innovation", op="info") for r in requests), requests
    unchanged_security_files()

    for mode, identity in (("old-helper", None), ("success", "wrong-machine:conception"),
                           ("reverse-trust", None), ("reverse-identity", None)):
        reset(identity)
        previous = state("innovation", "devices.json").read_bytes()
        result = pair(mode)
        assert result.returncode != 0, (mode, result.stdout)
        assert state("innovation", "devices.json").read_bytes() == previous, mode
        assert not state("creation", "devices.json").exists(), mode
        assert policy("innovation", identity or f"{machine}:conception") == "directed", mode
        requests = [json.loads(line) for line in (root / "requests.jsonl").read_text().splitlines()]
        if mode == "old-helper" or identity:
            assert not any(r["op"] == "peer_enroll" for r in requests), requests
        unchanged_security_files()

    # A receiver may not enroll itself or overwrite an existing endpoint identity.
    def receiver(peer):
        request = dict(version=1, id="receiver-test", op=dict(op="peer_enroll", args=dict(peer=peer)))
        payload = json.dumps(request).encode()
        result = subprocess.run([binary, "helper"], env=env("creation"),
                                input=b"CX1 " + str(len(payload)).encode() + b"\n" + payload,
                                capture_output=True, timeout=15)
        assert result.returncode == 0, result.stderr
        header, payload = result.stdout.split(b"\n", 1)
        assert header == b"CX1 " + str(len(payload)).encode()
        return json.loads(payload)

    reset()
    response = receiver(device("conception", "self", "conception@creation.example"))
    assert response["error"], response
    assert not state("creation", "devices.json").exists()
    collision = [device("intruder", "existing", "tester@innovation.example", "other-machine:intruder")]
    state("creation", "devices.json").write_text(json.dumps(collision))
    previous = state("creation", "devices.json").read_bytes()
    response = receiver(device("tester", "fixture-source", "tester@innovation.example"))
    assert response["error"], response
    assert state("creation", "devices.json").read_bytes() == previous
    unchanged_security_files()

    # Reciprocal pairing cannot silently promote an already directed receiver peer.
    reset()
    directed = device("tester", "fixture-source", "tester@innovation.example")
    state("creation", "devices.json").write_text(json.dumps([directed]))
    previous = state("creation", "devices.json").read_bytes()
    result = pair()
    assert result.returncode != 0, result.stdout
    assert state("creation", "devices.json").read_bytes() == previous
    assert policy("creation", directed["id"]) == "directed"
    assert policy("innovation", f"{machine}:conception") == "directed"
    unchanged_security_files()

    # A local alias or wrong account is not a portable return address.
    for target in ("innovation", "other@innovation.example"):
        reset()
        result = pair(target=target)
        assert result.returncode != 0, result.stdout
        assert not state("creation", "devices.json").exists()
        assert not (root / "requests.jsonl").read_text(), "invalid endpoint reached SSH"
        unchanged_security_files()

print(json.dumps(dict(result="PASS", real_helpers=True, reciprocal_enrollment=True,
                      reverse_identity_probed=True, old_helper_rejected=True,
                      identity_mismatch_rejected=True, reverse_trust_fail_closed=True,
                      reverse_identity_rejected=True, directed_peer_preserved=True,
                      self_and_endpoint_collision_rejected=True, ssh_and_routes_unchanged=True)))
