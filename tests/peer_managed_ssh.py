#!/usr/bin/env python3
"""Managed CX keys authenticate against disposable native OpenSSH, fail closed.

Uses existing OS account and a private loopback sshd. Changes only temporary
files; the SSH wrapper substitutes its port and remote fixture helper command.
"""
import json
import os
import pathlib
import pwd
import shlex
import shutil
import socket
import subprocess
import sys
import tempfile
import time


binary = str(pathlib.Path(sys.argv[1]).resolve())
account = pwd.getpwuid(os.getuid()).pw_name
ssh = shutil.which("ssh")
sshd = shutil.which("sshd") or "/usr/sbin/sshd"

# Short root: the private socket path must fit sun_path (see store::control_socket).
with tempfile.TemporaryDirectory(prefix="cx-mp-", dir="/tmp") as temporary:
    root = pathlib.Path(temporary)
    root.chmod(0o700)
    home = root / "home"
    home.mkdir(mode=0o700)
    state = root / "state/cx"
    state.mkdir(parents=True, mode=0o700)
    tools = root / "tools"
    tools.mkdir()
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        port = sock.getsockname()[1]
    for name in ("host", "changed-host"):
        subprocess.run(["ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-f", str(root / name)],
                       check=True, timeout=10)
    host_key = " ".join((root / "host.pub").read_text().split()[:2])

    # Real sshd executes a fixture protocol helper after native authentication.
    helper = root / "remote-helper.py"
    helper.write_text('''import json,sys
while True:
 header=sys.stdin.buffer.readline()
 if not header:break
 prefix,size=header.split();assert prefix==b'CX1'
 request=json.loads(sys.stdin.buffer.read(int(size)))
 op=request['op']['op'];assert op in ('info','sessions')
 result=[] if op=='sessions' else dict(machine_id='synthetic',account=''' + repr(account) + ''',host='fixture',capabilities=['peer-enroll-v1'])
 response=json.dumps(dict(version=1,id=request['id'],result=result,error=None)).encode()
 sys.stdout.buffer.write(b'CX1 '+str(len(response)).encode()+b'\\n'+response);sys.stdout.buffer.flush()
''')
    wrapper = tools / "ssh"
    wrapper.write_text('''#!/usr/bin/env python3
import json,os,pathlib,sys
args=sys.argv[1:];root=pathlib.Path(os.environ['CX_NATIVE_PEER'])
assert 'BatchMode=yes' in args
assert 'StrictHostKeyChecking=yes' in args
assert 'IdentitiesOnly=yes' in args
assert 'ForwardAgent=no' in args
assert 'Port=22' in args
assert '-i' in args and str(root) in args[args.index('-i')+1]
assert any(a.startswith('UserKnownHostsFile=') and str(root) in a for a in args)
# Shared-master setup (-N), control (-O) and resolution (-G) carry no remote command.
mux='-N' in args or '-O' in args or '-G' in args
assert mux or args[-1]=='exec ~/.local/bin/cx helper'
args[args.index('Port=22')]='Port='+os.environ['CX_NATIVE_PORT']
if not mux:args[-1]=os.environ['CX_NATIVE_COMMAND']
with (root/'argv.jsonl').open('a') as log:log.write(json.dumps(args)+'\\n')
os.execv(os.environ['CX_NATIVE_SSH'],[os.environ['CX_NATIVE_SSH'],*args])
''')
    wrapper.chmod(0o700)
    askpass = root / "askpass"
    askpass.write_text("#!/bin/sh\nprintf called >> \"$CX_NATIVE_PEER/asked\"\nexit 1\n")
    askpass.chmod(0o700)
    runtime = root / "run"
    runtime.mkdir(mode=0o700)
    env = dict(os.environ, HOME=str(home), XDG_STATE_HOME=str(root / "state"), USER=account,
               XDG_RUNTIME_DIR=str(runtime),
               PATH=str(tools) + os.pathsep + os.environ["PATH"],
               CX_NATIVE_PEER=str(root), CX_NATIVE_PORT=str(port), CX_NATIVE_SSH=ssh,
               CX_NATIVE_COMMAND=shlex.join([sys.executable, str(helper)]),
               SSH_ASKPASS=str(askpass), SSH_ASKPASS_REQUIRE="force", DISPLAY="fixture:0")

    def operation(op, args=None):
        request = dict(version=1, id="native-fixture", op=dict(op=op))
        if args is not None:
            request["op"]["args"] = args
        payload = json.dumps(request).encode()
        result = subprocess.run([binary, "helper"], env=env,
                                input=b"CX1 " + str(len(payload)).encode() + b"\n" + payload,
                                capture_output=True, timeout=15)
        assert result.returncode == 0, result.stderr
        header, payload = result.stdout.split(b"\n", 1)
        assert header == b"CX1 " + str(len(payload)).encode()
        response = json.loads(payload)
        assert not response["error"], response
        return response["result"]

    identity = operation("peer_identity")
    peer = dict(id=f"synthetic:{account}", name="synthetic", target=f"{account}@127.0.0.1",
                account=account, host="fixture", status="unknown", observed_at=0)
    operation("peer_configure", dict(peer=peer, host_key=host_key, client_key=identity["client_key"]))
    (state / "devices.json").write_text(json.dumps([peer]))
    private = state / "peer-ssh/id_ed25519"
    assert private.stat().st_mode & 0o777 == 0o600

    configuration = root / "sshd.conf"
    log = (root / "sshd.log").open("wb")
    daemon = None

    def start(key):
        configuration.write_text(f'''Port {port}
ListenAddress 127.0.0.1
HostKey {root / key}
PidFile {root}/sshd.pid
AuthorizedKeysFile {home}/.ssh/authorized_keys
StrictModes no
UsePAM no
PasswordAuthentication no
KbdInteractiveAuthentication no
PubkeyAuthentication yes
AllowUsers {account}
''')
        process = subprocess.Popen([sshd, "-D", "-e", "-f", str(configuration)], stdout=log, stderr=log)
        for _ in range(100):
            assert process.poll() is None, (root / "sshd.log").read_text()
            try:
                with socket.create_connection(("127.0.0.1", port), timeout=0.1):
                    return process
            except OSError:
                time.sleep(0.02)
        process.terminate()
        process.wait(timeout=3)
        raise AssertionError("fixture sshd did not start")

    def sessions():
        return subprocess.run([binary, "sessions", "--device", "synthetic"], env=env,
                              stdin=subprocess.DEVNULL, capture_output=True, timeout=15)

    def masters():
        return [p for p in (state / "ssh").glob("*") if p.is_socket()]

    def close_masters():
        # Only this fixture's private sockets; never a user ControlPath.
        for candidate in masters():
            subprocess.run([ssh, "-F", "/dev/null", "-S", str(candidate), "-O", "exit", "cx"],
                           stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=3)

    def argv_log():
        path = root / "argv.jsonl"
        return [json.loads(line) for line in path.read_text().splitlines()] if path.exists() else []

    try:
        daemon = start("host")
        result = sessions()
        assert result.returncode == 0, result.stderr
        assert json.loads(result.stdout) == [], result.stdout
        assert not (root / "asked").exists()
        # The managed endpoint now has one private shared master; the next
        # request joins it without a new handshake (no second -N).
        assert len(masters()) == 1, list((state / "ssh").iterdir())
        assert masters()[0].stat().st_mode & 0o077 == 0
        before = len([a for a in argv_log() if "-N" in a])
        result = sessions()
        assert result.returncode == 0, result.stderr
        assert len([a for a in argv_log() if "-N" in a]) == before == 1
        assert all("ControlMaster=no" in a for a in argv_log() if a[-1] != "-N" and "-N" not in a and "-O" not in a and "-G" not in a)
        # The master records its exact managed target (0600 sidecar).
        [master] = masters()
        record = master.with_name(master.name + ".owner")
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline and not record.exists():
            time.sleep(0.05)
        assert record.stat().st_mode & 0o777 == 0o600
        assert json.loads(record.read_text())["target"] == peer["target"]
        # A managed mesh endpoint that is not an inventory device keeps its
        # idle master: orphans never marks it stray.
        (state / "devices.json").write_text("[]")
        result = subprocess.run([binary, "orphans", "--json"], env=env, stdin=subprocess.DEVNULL,
                                capture_output=True, timeout=30)
        assert result.returncode == 0, result.stderr
        rows = json.loads(result.stdout)["ssh_masters"]
        assert rows and all(r["owner_recorded"] and r["enrolled"] and not r["stray"] for r in rows), rows
        (state / "devices.json").write_text(json.dumps([peer]))

        known = state / "peer-ssh/known_hosts"
        pinned = known.read_bytes()
        known.write_text("")
        result = sessions()
        assert result.returncode != 0, "untrusted server authenticated"
        assert known.read_text() == "", "unknown host was automatically trusted"
        assert not (root / "asked").exists()
        # A missing trust file is reported by ssh as a host-identity failure,
        # not an I/O error while building the command.
        known.unlink()
        result = sessions()
        assert result.returncode != 0, "host without trust file authenticated"
        assert b"host identity could not be verified" in result.stderr, result.stderr[-500:]
        assert not known.exists() and not (root / "asked").exists()
        known.write_bytes(pinned)
        known.chmod(0o600)

        # A live master keeps its authenticated connection to the pinned host;
        # retire it so the changed key below meets a fresh handshake.
        close_masters()
        daemon.terminate()
        daemon.wait(timeout=3)
        daemon = start("changed-host")
        result = sessions()
        assert result.returncode != 0, "changed host authenticated"
        assert known.read_bytes() == pinned, "changed host replaced pin"
        assert not (root / "asked").exists()
    finally:
        close_masters()
        if daemon is not None and daemon.poll() is None:
            daemon.terminate()
            daemon.wait(timeout=3)
        log.close()

print(json.dumps(dict(result="PASS", native_openssh=True, managed_client_key_authenticated=True,
                      strict_managed_host_pin=True, unknown_host_rejected=True,
                      changed_host_rejected=True, no_askpass_prompt=True, private_key_mode_600=True,
                      shared_master_reused=True, repin_selects_new_master=True,
                      owner_record_exact_target=True, mesh_master_not_stray=True,
                      missing_trust_file_is_host_key_failure=True)))
