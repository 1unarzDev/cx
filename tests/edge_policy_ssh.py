#!/usr/bin/env python3
"""Per-edge access policy enforced against two disposable loopback sshd devices.

Viewer A and device B are the current OS account with separate temporary HOME
and state directories and their own private sshd. Only temporary files change;
an SSH wrapper substitutes B's port and B's fixture helper command. Proves:
one-way A->B keeps A's cx key on B and removes B's cx key on A; password
removes A's cx key on B and BatchMode fails without prompting; none refuses
before any SSH; users' own authorized_keys lines stay byte-for-byte.
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
machine = pathlib.Path("/etc/machine-id").read_text().strip()
assert machine, "machine-id required for helper identity"


def free_port():
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


with tempfile.TemporaryDirectory(prefix="cx-edge-policy-") as temporary:
    root = pathlib.Path(temporary)
    root.chmod(0o700)
    sides = {}
    for side in ("a", "b"):
        home = root / f"home-{side}"
        (home / ".ssh").mkdir(parents=True, mode=0o700)
        state = root / f"state-{side}"
        (state / "cx").mkdir(parents=True, mode=0o700)
        subprocess.run(["ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-f", str(root / f"host-{side}")],
                       check=True, timeout=10)
        subprocess.run(["ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-C", "user-own", "-f",
                        str(root / f"user-{side}")], check=True, timeout=10)
        sides[side] = dict(home=home, state=state, port=free_port(),
                           host_key=" ".join((root / f"host-{side}.pub").read_text().split()[:2]))
    # Users' own lines, including one with options and a comment, must survive untouched.
    for side, info in sides.items():
        user_key = (root / f"user-{side}.pub").read_text().strip()
        info["user_lines"] = f"# my notes\nfrom=\"127.0.0.1\" {user_key}\n"
        (info["home"] / ".ssh/authorized_keys").write_text(info["user_lines"])
        (info["home"] / ".ssh/authorized_keys").chmod(0o600)

    tools = root / "tools"
    tools.mkdir()
    wrapper = tools / "ssh"
    wrapper.write_text('''#!/usr/bin/env python3
import json,os,pathlib,sys
args=sys.argv[1:];root=pathlib.Path(os.environ['CX_EDGE_ROOT'])
with (root/'argv.jsonl').open('a') as log:log.write(json.dumps(args)+'\\n')
assert 'BatchMode=yes' in args and 'ForwardAgent=no' in args and 'IdentitiesOnly=yes' in args
assert args[-1]=='exec ~/.local/bin/cx helper'
args[args.index('Port=22')]='Port='+os.environ['CX_EDGE_PORT']
args[-1]=os.environ['CX_EDGE_COMMAND']
os.execv(os.environ['CX_EDGE_SSH'],[os.environ['CX_EDGE_SSH'],*args])
''')
    wrapper.chmod(0o700)
    askpass = root / "askpass"
    askpass.write_text("#!/bin/sh\nprintf called >> \"$CX_EDGE_ROOT/asked\"\nexit 1\n")
    askpass.chmod(0o700)
    base = {k: v for k, v in os.environ.items() if k not in ("SSH_AUTH_SOCK", "SSH_AGENT_PID")}

    def env(side):
        info = sides[side]
        return dict(base, HOME=str(info["home"]), XDG_STATE_HOME=str(info["state"]), USER=account,
                    PATH=str(tools) + os.pathsep + os.environ["PATH"], CX_EDGE_ROOT=str(root),
                    CX_EDGE_PORT=str(sides["b"]["port"]), CX_EDGE_SSH=ssh,
                    CX_EDGE_COMMAND=shlex.join(["env", f"HOME={sides['b']['home']}",
                                                f"XDG_STATE_HOME={sides['b']['state']}",
                                                f"USER={account}", binary, "helper"]),
                    SSH_ASKPASS=str(askpass), SSH_ASKPASS_REQUIRE="force", DISPLAY="fixture:0")

    fake_machine = root / "viewer-machine-id"
    fake_machine.write_text("0" * 32 + "\n")

    def operation(side, op, args=None, distinct_machine=False):
        request = dict(version=1, id="edge-fixture", op=dict(op=op))
        if args is not None:
            request["op"]["args"] = args
        payload = json.dumps(request).encode()
        command = [binary, "helper"]
        if distinct_machine:
            # Both fixture devices share this host's machine-id; give the viewer's
            # one setup call a private mount-namespace view so it is not "self".
            command = ["unshare", "-rm", "sh", "-c",
                       f"mount --bind {shlex.quote(str(fake_machine))} /etc/machine-id && exec \"$@\"",
                       "sh", *command]
        result = subprocess.run(command, env=env(side),
                                input=b"CX1 " + str(len(payload)).encode() + b"\n" + payload,
                                capture_output=True, timeout=20)
        assert result.returncode == 0, result.stderr
        response = json.loads(result.stdout.split(b"\n", 1)[1])
        assert not response["error"], response
        return response["result"]

    def cx(*args):
        return subprocess.run([binary, *args], env=env("a"), stdin=subprocess.DEVNULL,
                              capture_output=True, timeout=30, text=True)

    viewer = dict(id=f"viewer-a:{account}", name="viewer-a", target=f"{account}@127.0.0.1",
                  account=account, host="viewer-a", status="unknown", observed_at=0)
    device_b = dict(id=f"{machine}:{account}", name="peer-b", target=f"{account}@127.0.0.1",
                    account=account, host="peer-b", status="unknown", observed_at=0)
    identity = {side: operation(side, "peer_identity")["client_key"] for side in sides}
    # Mutual pairing as core sync would establish it, with fixture host keys.
    operation("b", "peer_configure", dict(peer=viewer, host_key=sides["a"]["host_key"],
                                          client_key=identity["a"]))
    operation("a", "peer_configure", dict(peer=device_b, host_key=sides["b"]["host_key"],
                                          client_key=identity["b"]), distinct_machine=True)
    state_a = sides["a"]["state"] / "cx"
    (state_a / "devices.json").write_text(json.dumps([device_b]))
    policies = state_a / "device-policies.json"
    policies.write_text(json.dumps({"devices": {device_b["id"]: {"access": "core"}}}))
    policies.chmod(0o600)
    policy_bytes = policies.read_bytes()

    daemons = []
    log = (root / "sshd.log").open("wb")

    def start(side, password):
        info = sides[side]
        configuration = root / f"sshd-{side}.conf"
        configuration.write_text(f'''Port {info["port"]}
ListenAddress 127.0.0.1
HostKey {root / f"host-{side}"}
PidFile {root}/sshd-{side}.pid
AuthorizedKeysFile {info["home"]}/.ssh/authorized_keys
StrictModes no
UsePAM no
PasswordAuthentication {"yes" if password else "no"}
KbdInteractiveAuthentication {"yes" if password else "no"}
PubkeyAuthentication yes
AllowUsers {account}
''')
        process = subprocess.Popen([sshd, "-D", "-e", "-f", str(configuration)], stdout=log, stderr=log)
        daemons.append(process)
        for _ in range(100):
            assert process.poll() is None, (root / "sshd.log").read_text()
            try:
                with socket.create_connection(("127.0.0.1", info["port"]), timeout=0.1):
                    return
            except OSError:
                time.sleep(0.02)
        raise AssertionError("fixture sshd did not start")

    def can_login(source, destination):
        """Native BatchMode login with only `source`'s cx key into `destination`."""
        key = sides[source]["state"] / "cx/peer-ssh/id_ed25519"
        result = subprocess.run([ssh, "-F", "/dev/null", "-i", str(key), "-o", "IdentitiesOnly=yes",
                                 "-o", "BatchMode=yes", "-o", "StrictHostKeyChecking=no",
                                 "-o", "UserKnownHostsFile=/dev/null", "-o", "LogLevel=ERROR",
                                 "-p", str(sides[destination]["port"]), f"{account}@127.0.0.1", "true"],
                                env=env(source), stdin=subprocess.DEVNULL, capture_output=True, timeout=15)
        return result.returncode == 0

    def authorized(side):
        return (sides[side]["home"] / ".ssh/authorized_keys").read_text()

    def user_lines_intact(side):
        text = authorized(side)
        return text.startswith(sides[side]["user_lines"])

    def calls():
        path = root / "argv.jsonl"
        return path.read_text().count("\n") if path.exists() else 0

    try:
        start("a", password=True)
        start("b", password=True)
        assert can_login("a", "b") and can_login("b", "a"), "baseline pairing failed"

        listed = cx("edge", "list", "--json")
        assert listed.returncode == 0, listed.stderr
        [edge] = json.loads(listed.stdout)
        assert edge["label"] == "key ⇄" and edge["forward"]["source"] == "migrated core", edge

        preview = cx("edge", "set", "local", "peer-b", "one-way", "--dry-run", "--json")
        assert preview.returncode == 0, preview.stderr
        plan = json.loads(preview.stdout)
        assert plan["steps"] == [dict(action="remove_key", **{"from": device_b["id"], "to": "local"})], plan
        assert any("peer-b will no longer reach" in c for c in plan["consequences"]), plan
        assert not (state_a / "edges.json").exists(), "dry run saved state"
        unconfirmed = cx("edge", "set", "local", "peer-b", "one-way")
        assert unconfirmed.returncode != 0 and "confirmation required" in unconfirmed.stderr
        assert not (state_a / "edges.json").exists()

        # One-way A -> B: A keeps its cx key on B, B's cx key on A is removed.
        result = cx("edge", "set", "local", "peer-b", "one-way", "--yes")
        assert result.returncode == 0, result.stderr
        assert json.loads(result.stdout)["enforced"] is True
        assert can_login("a", "b"), "one-way removed the forward key"
        assert not can_login("b", "a"), "reverse cx key still authenticates"
        assert "cx-managed-peer" not in authorized("a") and "cx-managed-peer" in authorized("b")
        assert user_lines_intact("a") and user_lines_intact("b")
        assert (state_a / "edges.json").stat().st_mode & 0o777 == 0o600
        # Core pairing in the refused direction is refused before any SSH.
        before = calls()
        pair = cx("device", "pair", "peer-b", "--return-target", f"{account}@127.0.0.1")
        assert pair.returncode != 0 and "edge policy refuses pairing" in pair.stderr, pair.stderr
        assert calls() == before

        # Password A -> B: remote helper removes A's cx key on B over the existing channel.
        before = calls()
        result = cx("edge", "set", "local", "peer-b", "password", "--yes")
        assert result.returncode == 0, result.stderr + result.stdout
        applied = json.loads(result.stdout)
        assert applied["enforced"] is True and calls() > before, applied
        assert "cx-managed-peer" not in authorized("b"), "password left a cx key installed"
        assert not can_login("a", "b") and not can_login("b", "a")
        assert user_lines_intact("a") and user_lines_intact("b")
        sessions = cx("sessions", "--device", "peer-b")
        assert sessions.returncode != 0, "BatchMode authenticated without a key"
        assert not (root / "asked").exists(), "background check prompted for a password"
        last = json.loads((root / "argv.jsonl").read_text().splitlines()[-1])
        assert "BatchMode=yes" in last
        [edge] = json.loads(cx("edge", "list", "--json").stdout)
        assert edge["forward"]["intended"] == "password", edge
        assert edge["forward"]["enforced"].startswith("cx key absent"), edge
        assert "authenticated" in edge["forward"]["observed"], edge
        assert "password required" in edge["forward"]["observed"], edge
        assert edge["forward"]["background_authenticates"] is False
        assert edge["reverse"]["intended"] == "refused", edge

        # None: saved, enforced where reachable, and every route over the edge refused.
        result = cx("edge", "set", "local", "peer-b", "none", "--yes")
        applied = json.loads(result.stdout)
        assert applied["saved"] is True
        assert result.returncode != 0, "unreachable enforcement claimed success"
        assert any(not s["ok"] and s["state"].startswith("failed:") for s in applied["steps"]), applied
        before = calls()
        refused = cx("sessions", "--device", "peer-b")
        assert refused.returncode != 0 and "edge policy refuses" in refused.stderr, refused.stderr
        refused = cx("add", f"{account}@10.0.0.5", "--via", "peer-b")
        assert refused.returncode != 0 and "edge policy refuses" in refused.stderr, refused.stderr
        assert calls() == before, "refused edge still opened SSH"
        [edge] = json.loads(cx("edge", "list", "--json").stdout)
        assert edge["label"] == "⊘" and edge["forward"]["enforced"].startswith("failed:"), edge
        assert not (root / "asked").exists()
        assert user_lines_intact("a") and user_lines_intact("b")
        assert policies.read_bytes() == policy_bytes, "device-policies.json was rewritten"
    finally:
        for process in daemons:
            if process.poll() is None:
                process.terminate()
                process.wait(timeout=3)
        log.close()

print(json.dumps(dict(result="PASS", one_way_forward_key=True, one_way_reverse_refused=True,
                      password_no_cx_key=True, batchmode_no_prompt=True, none_refused_before_ssh=True,
                      user_lines_untouched=True, device_policies_untouched=True)))
