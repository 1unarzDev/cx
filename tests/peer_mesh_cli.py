#!/usr/bin/env python3
"""Disposable three-helper core mesh bootstrap regression.

Real helpers create managed keys and persist configuration under separate HOME
and state roots. The SSH executable is a deterministic, framed transport fixture;
no network, installed helper, personal trust or configuration is touched.
"""
import json
import os
import pathlib
import pty
import select
import signal
import stat
import subprocess
import sys
import tempfile
import time


BINARY = str(pathlib.Path(sys.argv[1]).resolve())
MACHINE = pathlib.Path("/etc/machine-id").read_text().strip()
HOST_PUBLIC = pathlib.Path("/etc/ssh/ssh_host_ed25519_key.pub").read_text().split()
UNRELATED_KEY = (" ".join(HOST_PUBLIC[:2]) + " fixture-unrelated").encode()
ADDRESSES = json.loads(subprocess.check_output(["ip", "-j", "address", "show", "scope", "global"], text=True))
LOCAL_ADDRESS = next(a["local"] for interface in ADDRESSES for a in interface.get("addr_info", [])
                     if a.get("family") == "inet")
PEERS = {
    "innovation": ("tester", LOCAL_ADDRESS),
    "creation": ("conception", "creation.example"),
    "tranquility": ("peace", "tranquility.example"),
    "robot": ("robot", "robot.example"),
}


SSH_WRAPPER = r'''#!/usr/bin/env python3
import base64,json,os,pathlib,subprocess,sys,time
root=pathlib.Path(os.environ['CX_MESH_FIXTURE'])
peers=json.loads(os.environ['CX_MESH_PEERS'])
args=sys.argv[1:]
if '-G' in args:
    target=args[-1]
    for name,(account,host) in peers.items():
        if target in (name,account+'@'+host):
            print('hostname '+host+'\nuser '+account+'\nport 22');sys.exit(0)
    raise AssertionError('unknown fixture resolver target')
assert args[-1]=='exec ~/.local/bin/cx helper','fixed helper command required'
assert 'ForwardAgent=no' in args,'credentials must never be forwarded'
assert 'BatchMode=yes' in args,'core sync must never prompt for SSH authentication'
target=args[-2]
endpoint=None
for name,(account,host) in peers.items():
    if target in (name,account+'@'+host):endpoint=name;break
assert endpoint is not None,'unknown fixture SSH endpoint'
with (root/'connections.jsonl').open('a') as log:
    log.write(json.dumps(dict(source=os.environ['CX_MESH_ORIGIN'],target=endpoint,args=args,pid=os.getpid()))+'\n')
if target=='innovation' and os.environ['CX_MESH_ORIGIN']!='innovation' and os.environ['CX_MESH_MODE']=='stale-alias':
    sys.stderr.write('Could not resolve hostname innovation\n');sys.exit(255)
if endpoint=='tranquility' and os.environ['CX_MESH_MODE']=='offline':
    sys.stderr.write('No route to host\n');sys.exit(255)
account=peers[endpoint][0]
env=dict(os.environ,HOME=str(root/endpoint/'home'),XDG_STATE_HOME=str(root/endpoint/'state'),
         USER=account,CX_MESH_ORIGIN=endpoint,
         SSH_CONNECTION=peers['innovation'][1]+' 40000 '+peers['innovation'][1]+' 22')
child=subprocess.Popen([os.environ['CX_MESH_BINARY'],'helper'],env=env,stdin=subprocess.PIPE,stdout=subprocess.PIPE)
def read(stream):
    header=stream.readline()
    if not header:return None
    prefix,size=header.split();assert prefix==b'CX1'
    size=int(size);assert 0<=size<=1048576
    payload=stream.read(size);assert len(payload)==size
    assert b'PRIVATE KEY' not in payload,'private key crossed SSH helper protocol'
    return json.loads(payload)
def write(stream,value):
    payload=json.dumps(value).encode();stream.write(b'CX1 '+str(len(payload)).encode()+b'\n'+payload);stream.flush()
try:
    while True:
        request=read(sys.stdin.buffer)
        if request is None:break
        with (root/'requests.jsonl').open('a') as log:
            log.write(json.dumps(dict(source=os.environ['CX_MESH_ORIGIN'],target=endpoint,op=request['op']['op'],pid=os.getpid()))+'\n')
        if request['op']['op'] not in ('info','peer_identity','peer_configure','peer_enroll'):
            # UI observers are outside bootstrap scope. Return empty metadata
            # without asking real helpers to inspect unrelated host sessions.
            write(sys.stdout.buffer,dict(version=1,id=request['id'],result=[],error=None))
            continue
        if endpoint=='robot':
            raise AssertionError('Directed device received bootstrap mutation')
        if (os.environ['CX_MESH_MODE']=='reverse-barrier' and endpoint=='innovation'
                and os.environ['CX_MESH_ORIGIN']=='creation' and request['op']['op']=='info'):
            (root/'barrier-reached').touch()
            deadline=time.monotonic()+15
            while not (root/'barrier-release').exists():
                assert time.monotonic()<deadline,'reverse-probe barrier timed out'
                time.sleep(.01)
        write(child.stdin,request);response=read(child.stdout)
        if request['op']['op']=='peer_enroll' and response.get('error'):
            with (root/'errors.jsonl').open('a') as log:
                log.write(json.dumps(dict(target=endpoint,error=response['error']))+'\n')
        if endpoint=='creation' and os.environ['CX_MESH_MODE']=='changed-host-key' and request['op']['op']=='peer_identity':
            fields=response['result']['host_key'].split()
            raw=bytearray(base64.b64decode(fields[1]));raw[-1]^=1
            fields[1]=base64.b64encode(raw).decode()
            response['result']['host_key']=' '.join(fields)
        write(sys.stdout.buffer,response)
finally:
    child.stdin.close();child.terminate();child.wait(timeout=3)
'''


def device(name, local=False):
    account, host = PEERS[name]
    return dict(id="local" if local else f"{MACHINE}:{account}",
                name=f"fixture-local-{name}" if local else name,
                target=None if local else name, account=account, host=host,
                status="unknown", observed_at=0)


class Fixture:
    def __init__(self, root):
        self.root = root
        tools = root / "tools"
        tools.mkdir()
        wrapper = tools / "ssh"
        wrapper.write_text(SSH_WRAPPER)
        wrapper.chmod(0o700)
        for endpoint in PEERS:
            (root / endpoint / "home/.ssh").mkdir(parents=True)
            (root / endpoint / "home/.ssh/config").write_text("# untouched fixture config\n")
            (root / endpoint / "home/.ssh/known_hosts").write_text("# untouched personal trust\n")
            authorized = root / endpoint / "home/.ssh/authorized_keys"
            authorized.write_bytes(UNRELATED_KEY)  # Deliberately no final newline.
            authorized.chmod(0o600)
            self.state(endpoint).mkdir(parents=True)
            self.state(endpoint).chmod(0o700)
            self.write(endpoint, "devices.json", [device(endpoint, local=True)])
        self.write("innovation", "devices.json", [device("innovation", local=True),
                                                   device("creation"), device("tranquility"), device("robot")])
        self.write("innovation", "device-policies.json", {"devices": {
            f"{MACHINE}:{PEERS[name][0]}": {"access": "core", "availability": "usually_up"}
            for name in ("creation", "tranquility")
        }})

    def state(self, endpoint):
        return self.root / endpoint / "state/cx"

    def write(self, endpoint, filename, value):
        (self.state(endpoint) / filename).write_text(json.dumps(value))
        (self.state(endpoint) / filename).chmod(0o600)

    def read(self, endpoint, filename):
        return json.loads((self.state(endpoint) / filename).read_text())

    def env(self, mode="success"):
        return dict(os.environ, HOME=str(self.root / "innovation/home"),
                    XDG_STATE_HOME=str(self.root / "innovation/state"), USER="tester",
                    PATH=str(self.root / "tools") + os.pathsep + os.environ["PATH"],
                    CX_MESH_FIXTURE=str(self.root), CX_MESH_PEERS=json.dumps(PEERS),
                    CX_MESH_BINARY=BINARY, CX_MESH_ORIGIN="innovation", CX_MESH_MODE=mode)

    def sync(self, mode="success"):
        return subprocess.run([BINARY, "device", "sync"], env=self.env(mode),
                              capture_output=True, text=True, timeout=45)

    def startup(self):
        """Normal interactive startup must bootstrap without a pairing command."""
        master, slave = pty.openpty()
        environment = self.env()
        environment.update(TERM="xterm-256color", HTTP_PROXY="http://127.0.0.1:9",
                           HTTPS_PROXY="http://127.0.0.1:9", ALL_PROXY="http://127.0.0.1:9",
                           http_proxy="http://127.0.0.1:9", https_proxy="http://127.0.0.1:9",
                           all_proxy="http://127.0.0.1:9", NO_PROXY="", no_proxy="")
        process = subprocess.Popen([BINARY], env=environment, stdin=slave, stdout=slave,
                                   stderr=slave, start_new_session=True, cwd=self.root)
        os.close(slave)
        deadline = time.monotonic() + 20
        complete = False
        try:
            while time.monotonic() < deadline:
                assert process.poll() is None, "normal CX startup exited before mesh enrollment"
                # Drain terminal output boundedly; never retain session content.
                if select.select([master], [], [], 0.05)[0]:
                    try:
                        os.read(master, 65536)
                    except OSError:
                        pass
                try:
                    self.assert_mesh()
                    complete = True
                    break
                except (AssertionError, FileNotFoundError, json.JSONDecodeError):
                    continue
            if not complete:
                requests = self.root / "requests.jsonl"
                evidence = requests.read_text()[-6000:] if requests.exists() else "no SSH requests"
                errors = self.root / "errors.jsonl"
                if errors.exists():evidence += errors.read_text()[-4000:]
                raise AssertionError("normal CX startup did not enroll six core edges within 20 seconds; " + evidence)
        finally:
            # Stop only the owned UI process and subprocess group.
            if process.poll() is None:
                process.send_signal(signal.SIGTERM)
                try:
                    process.wait(timeout=3)
                except subprocess.TimeoutExpired:
                    os.killpg(process.pid, signal.SIGKILL)
                    process.wait(timeout=3)
            os.close(master)

    def revoke_during_reverse(self, action):
        """A local user action wins while an authenticated reverse probe waits."""
        peer = device("innovation")
        peer["target"] = f"tester@{LOCAL_ADDRESS}"
        self.write("creation", "devices.json", [device("creation", local=True), peer])
        self.write("creation", "device-policies.json", {"devices": {
            peer["id"]: {"access": "core", "availability": "usually_up"}
        }})
        environment = self.env("reverse-barrier")
        environment.update(HOME=str(self.root / "creation/home"),
                           XDG_STATE_HOME=str(self.root / "creation/state"),
                           USER="conception", CX_MESH_ORIGIN="creation")
        request = dict(version=1, id="barrier", op=dict(op="peer_enroll", args=dict(peer=peer, replace_stale=True)))
        payload = json.dumps(request).encode()
        process = subprocess.Popen([BINARY, "helper"], env=environment, stdin=subprocess.PIPE,
                                   stdout=subprocess.PIPE, stderr=subprocess.PIPE, start_new_session=True)
        try:
            process.stdin.write(b"CX1 " + str(len(payload)).encode() + b"\n" + payload)
            process.stdin.flush()
            deadline = time.monotonic() + 10
            while not (self.root / "barrier-reached").exists():
                assert process.poll() is None, "helper exited before reverse barrier"
                assert time.monotonic() < deadline, "helper never attempted reverse probe"
                time.sleep(.01)
            command = [BINARY, "device", action, "innovation"]
            mutation = subprocess.run(command, env=environment, capture_output=True, timeout=5)
            assert mutation.returncode == 0, mutation.stderr
            expected_devices = (self.state("creation") / "devices.json").read_bytes()
            expected_policy_path = self.state("creation") / "device-policies.json"
            expected_policy = expected_policy_path.read_bytes()
            (self.root / "barrier-release").touch()
            process.stdin.close()
            process.stdin = None
            output, errors = process.communicate(timeout=10)
            assert process.returncode == 0, errors
            header, payload = output.split(b"\n", 1)
            assert header == b"CX1 " + str(len(payload)).encode()
            assert json.loads(payload)["error"], "concurrent local revocation was ignored"
            assert (self.state("creation") / "devices.json").read_bytes() == expected_devices
            assert expected_policy_path.read_bytes() == expected_policy
            if action == "remove":
                assert all(p["id"] != peer["id"] for p in self.read("creation", "devices.json"))
                # A fresh enrollment also respects the removal tombstone.
                retry = subprocess.run([BINARY, "helper"], env=environment,
                                       input=b"CX1 " + str(len(json.dumps(request).encode())).encode() + b"\n" + json.dumps(request).encode(),
                                       capture_output=True, timeout=5)
                assert retry.returncode == 0, retry.stderr
                assert json.loads(retry.stdout.split(b"\n", 1)[1])["error"]
            else:
                policy = self.read("creation", "device-policies.json")
                assert policy["devices"][peer["id"]]["access"] == "directed"
        finally:
            (self.root / "barrier-release").touch()
            if process.poll() is None:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait(timeout=3)

    def security_invariants(self):
        for endpoint in PEERS:
            home = self.root / endpoint / "home"
            assert (home / ".ssh/config").read_text() == "# untouched fixture config\n"
            assert (home / ".ssh/known_hosts").read_text() == "# untouched personal trust\n"
            authorized = (home / ".ssh/authorized_keys").read_bytes()
            assert authorized.startswith(UNRELATED_KEY), "existing authorized key changed"
            assert authorized.count(UNRELATED_KEY) == 1, "existing authorized key duplicated"
            if endpoint == "robot":
                assert authorized == UNRELATED_KEY, "Directed device authorization changed"
        requests = (self.root / "requests.jsonl").read_text() if (self.root / "requests.jsonl").exists() else ""
        assert "PRIVATE KEY" not in requests
        connections = (self.root / "connections.jsonl").read_text() if (self.root / "connections.jsonl").exists() else ""
        assert all(json.loads(line)["target"] != "robot" or json.loads(line)["op"] not in
                   ("peer_identity", "peer_configure", "peer_enroll") for line in requests.splitlines())

    def assert_mesh(self):
        for endpoint in ("innovation", "creation", "tranquility"):
            peers = self.read(endpoint, "devices.json")
            expected = {f"{MACHINE}:{PEERS[name][0]}" for name in ("innovation", "creation", "tranquility")
                        if name != endpoint}
            observed = {peer["id"] for peer in peers if peer["target"] and peer["account"] != "robot"}
            assert expected == observed, (endpoint, expected, observed)
            if endpoint != "innovation":
                assert all(peer["account"] != "robot" for peer in peers), endpoint

    def private_keys(self):
        found = {}
        for endpoint in ("innovation", "creation", "tranquility"):
            keys = []
            for path in (self.root / endpoint).rglob("*"):
                if path.is_file() and path.stat().st_size <= 16384:
                    value = path.read_bytes()
                    if value.startswith(b"-----BEGIN OPENSSH PRIVATE KEY-----"):
                        assert stat.S_IMODE(path.stat().st_mode) == 0o600, path
                        keys.append((str(path.relative_to(self.root)), value))
            assert keys, (endpoint, "managed private client key missing")
            found[endpoint] = keys
        return found

    def managed_trust(self):
        result = {}
        for endpoint in ("innovation", "creation", "tranquility"):
            for path in (self.state(endpoint) / "peer-ssh/known_hosts",
                         self.state(endpoint) / "peer-ssh/peers.json",
                         self.root / endpoint / "home/.ssh/authorized_keys"):
                result[str(path.relative_to(self.root))] = path.read_bytes()
        return result


for action in ("remove", "demote"):
    with tempfile.TemporaryDirectory(prefix="cx-race-") as temporary:
        fixture = Fixture(pathlib.Path(temporary))
        fixture.revoke_during_reverse(action)
        fixture.security_invariants()

with tempfile.TemporaryDirectory(prefix="cx-ui-") as temporary:
    fixture = Fixture(pathlib.Path(temporary))
    fixture.startup()
    fixture.assert_mesh()
    fixture.security_invariants()

with tempfile.TemporaryDirectory(prefix="cx-rt-") as temporary:
    fixture = Fixture(pathlib.Path(temporary))
    fixture.write("innovation", "routes.json", {"creation": ["fixture-gateway"]})
    previous = (fixture.state("innovation") / "routes.json").read_bytes()
    fixture.sync()
    requests = [json.loads(line) for line in (fixture.root / "requests.jsonl").read_text().splitlines()]
    assert not any(r["target"] == "creation" for r in requests), requests
    assert (fixture.state("innovation") / "routes.json").read_bytes() == previous
    assert len(fixture.read("creation", "devices.json")) == 1
    assert not any(p["id"] == f"{MACHINE}:conception" for p in fixture.read("tranquility", "devices.json"))
    fixture.security_invariants()

with tempfile.TemporaryDirectory(prefix="cx-old-") as temporary:
    fixture = Fixture(pathlib.Path(temporary))
    # Alias meaningful on the viewer was previously persisted on the receiver.
    # It is stale there; automatic setup must adopt the proven portable address.
    fixture.write("creation", "devices.json", [device("creation", local=True), device("innovation")])
    fixture.write("creation", "device-policies.json", {"devices": {
        f"{MACHINE}:tester": {"access": "core", "availability": "usually_up"}
    }})
    repaired = fixture.sync("stale-alias")
    assert repaired.returncode == 0, repaired.stderr
    fixture.assert_mesh()
    repaired_peer = next(peer for peer in fixture.read("creation", "devices.json")
                         if peer["id"] == f"{MACHINE}:tester")
    assert repaired_peer["target"] == f"tester@{LOCAL_ADDRESS}", repaired_peer
    fixture.security_invariants()

with tempfile.TemporaryDirectory(prefix="cx-peer-mesh-") as temporary:
    fixture = Fixture(pathlib.Path(temporary))
    # Start with no managed key, reciprocal enrollment or private host trust.
    offline = fixture.sync("offline")
    assert offline.returncode in (0, 1), offline.stderr
    tranquility = fixture.read("tranquility", "devices.json")
    assert len(tranquility) == 1 and tranquility[0]["target"] is None, tranquility
    creation = fixture.read("creation", "devices.json")
    assert any(peer["id"] == f"{MACHINE}:tester" for peer in creation), offline.stderr
    fixture.security_invariants()

    # An ordinary retry recovers an offline core peer and completes every edge.
    recovered = fixture.sync()
    assert recovered.returncode == 0, recovered.stderr
    fixture.assert_mesh()
    fixture.security_invariants()
    original_keys = fixture.private_keys()
    original_authorizations = {
        endpoint: (fixture.root / endpoint / "home/.ssh/authorized_keys").read_bytes()
        for endpoint in ("innovation", "creation", "tranquility")
    }

    repeated = fixture.sync()
    assert repeated.returncode == 0, repeated.stderr
    fixture.assert_mesh()
    fixture.security_invariants()
    assert fixture.private_keys() == original_keys, "retry rotated a managed client key"
    for endpoint, contents in original_authorizations.items():
        assert (fixture.root / endpoint / "home/.ssh/authorized_keys").read_bytes() == contents, endpoint

    # Stable account/machine identity does not authorize a silent host-key change.
    # Failure remains pending; existing trusted pins and authorizations survive.
    trusted = fixture.managed_trust()
    changed = fixture.sync("changed-host-key")
    assert changed.returncode in (0, 1), changed.stderr
    assert "pending" in (changed.stdout + changed.stderr).lower(), changed.stdout + changed.stderr
    assert fixture.managed_trust() == trusted, "changed host key replaced managed trust"
    assert fixture.private_keys() == original_keys
    fixture.security_invariants()

print(json.dumps(dict(result="PASS", real_helpers=True, three_core_devices=True,
                      normal_ui_startup_bootstraps=True, stored_gateway_route_not_exported=True,
                      stale_core_alias_adopts_verified_portable_endpoint=True,
                      removal_and_demotion_win_during_reverse_probe=True,
                      offline_then_recovered=True, all_six_edges_registered=True,
                      directed_device_excluded=True, private_key_mode_600=True,
                      no_private_key_transmitted=True, repeated_sync_idempotent=True,
                      changed_host_key_rejected=True,
                      existing_authorized_key_without_newline_preserved=True,
                      personal_ssh_config_and_trust_unchanged=True)))
