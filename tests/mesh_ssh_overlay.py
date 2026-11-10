#!/usr/bin/env python3
"""ssh-overlay pairing end to end on loopback (docs/mesh.md, "ssh-overlay").

Two cx identities on this machine, each with its own temporary HOME and state. The viewer runs
`cx mesh invite --backend ssh-overlay` (a one-shot listener on 127.0.0.1) and the joiner runs
`cx mesh pair CODE 127.0.0.1:PORT`. Then a private sshd (running as this user, authorizing only the
viewer's temporary authorized_keys) proves the key the viewer installed logs in. Also: an enrolled
identity refused, wrong codes refused and rate-limited, replay, a typo caught offline, expiry and
Ctrl-C (the port closed each time), revoke, and `cx mesh backends --json` with a fake tailscale on PATH.
Nothing outside the temporary tree changes: the real ~/.ssh/authorized_keys, known_hosts and cx devices are
compared by content hash, and must not contain anything of this test (its keys, machine ids, temporary paths).
A real cx viewer (or ssh) on this host may rewrite those files meanwhile, so a changed hash alone is only reported.

Usage: python3 tests/mesh_ssh_overlay.py target/debug/cx
"""
import hashlib
import json
import os
import pathlib
import pwd
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
import time

binary = str(pathlib.Path(sys.argv[1]).resolve())
account = pwd.getpwuid(os.getuid()).pw_name
sshd = shutil.which("sshd") or "/usr/sbin/sshd"
real_home = pathlib.Path(pwd.getpwuid(os.getuid()).pw_dir)
watched = [real_home / ".ssh/authorized_keys", real_home / ".ssh/known_hosts", real_home / ".local/state/cx/devices.json",
           real_home / ".config/cx/mesh/config.toml"]


def digest(p):
    try:
        return hashlib.sha256(p.read_bytes()).hexdigest()
    except FileNotFoundError:
        return None


before = [digest(p) for p in watched]
result = {}


def free_port():
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def is_open(port):
    try:
        with socket.create_connection(("127.0.0.1", port), timeout=0.5):
            return True
    except OSError:
        return False


with tempfile.TemporaryDirectory(prefix="cx-mesh-join-") as temporary:
    root = pathlib.Path(temporary)
    root.chmod(0o700)

    def side(name, machine):
        home = root / name
        (home / ".ssh").mkdir(parents=True, mode=0o700)
        env = dict(os.environ, HOME=str(home), XDG_STATE_HOME=str(home / "state"), XDG_CONFIG_HOME=str(home / "config"),
                   XDG_CACHE_HOME=str(home / "cache"), USER=account, CX_JOIN_MACHINE_ID=machine, CLOUDFLARE_API_TOKEN="")
        return home, env

    viewer, venv = side("viewer", "viewer-machine")
    joiner, jenv = side("joiner", "joiner-machine")
    subprocess.run(["ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-f", str(root / "host")], check=True, timeout=10)
    host_key = " ".join((root / "host.pub").read_text().split()[:2])

    def cx(env, *args, check=True):
        r = subprocess.run([binary, *args], env=env, capture_output=True, text=True, timeout=60)
        if check:
            assert r.returncode == 0, (args, r.stdout, r.stderr)
        return r

    class Invite:
        """A `cx mesh invite` running in the background of this test, as it runs in a terminal."""

        def __init__(self, *extra):
            self.port = free_port()
            self.process = subprocess.Popen(
                [binary, "mesh", "invite", "--backend", "ssh-overlay", "--address", "127.0.0.1", "--port", str(self.port),
                 "--host-key", str(root / "host.pub"), "--json", *extra],
                env=venv, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
            line = self.process.stdout.readline()
            assert line, self.process.stderr.read()
            self.invite = json.loads(line)
            self.code = self.invite["code"]
            self.host = f"127.0.0.1:{self.port}"

        def finish(self, timeout=30):
            out, err = self.process.communicate(timeout=timeout)
            assert not is_open(self.port), "the listener port is closed once the invite is"
            return self.process.returncode, out, err

    def authorized(home):
        p = home / ".ssh/authorized_keys"
        return p.read_text() if p.exists() else ""

    def devices(home):
        p = home / "state/cx/devices.json"
        return json.loads(p.read_text()) if p.exists() else []

    daemon = None
    invites = []
    try:
        # Takeover: an invite holder claiming an enrolled identity (here a directed device with no
        # SSH pins) is refused, and the record keeps its target and access. The right code uses the
        # invite up, so the listener closes with it.
        victim = {"id": f"victim-machine:{account}", "name": "victim", "target": f"{account}@10.9.9.9",
                  "account": account, "host": "victim", "status": "seeded", "observed_at": 1}
        (viewer / "state/cx").mkdir(parents=True, mode=0o700)
        (viewer / "state/cx/devices.json").write_text(json.dumps([victim]))
        policy_file = viewer / "state/cx/device-policies.json"
        policies_before = policy_file.read_text() if policy_file.exists() else None
        intruder, ienv = side("intruder", "victim-machine")
        first = Invite()
        invites.append(first)
        r = cx(ienv, "mesh", "pair", first.code, first.host, "--name", "mallory", check=False)
        assert r.returncode != 0 and "already enrolled" in r.stderr, r.stderr
        code, _, err = first.finish()
        assert code != 0 and "cancelled" in err, err
        assert [d for d in devices(viewer) if d["id"] == victim["id"]] == [victim], devices(viewer)
        assert (policy_file.read_text() if policy_file.exists() else None) == policies_before
        assert not any(l.endswith(" cx-managed-peer") for l in authorized(viewer).splitlines())
        pins = viewer / "state/cx/peer-ssh/peers.json"
        assert not pins.exists() or "victim-machine" not in pins.read_text()
        (viewer / "state/cx/devices.json").write_text(json.dumps([]))
        result["takeover_refused"] = True

        inv = Invite("--name", "rover")
        invites.append(inv)
        assert inv.invite["one_liner_sh"] == f"cx mesh pair {inv.code} {inv.host}" and inv.invite["uses"] == 1, inv.invite
        assert len(inv.code) == 13 and inv.invite["hosts"] == [inv.host], inv.invite
        # The code lives only in the listener's memory.
        bare = inv.code.replace("-", "")
        assert not [p for p in viewer.rglob("*") if p.is_file() and (bare in p.read_text(errors="ignore") or inv.code in p.read_text(errors="ignore"))]
        # A wrong code (with a valid check symbol: another fresh invite's) is refused and the invite stays open.
        other = Invite()
        invites.append(other)
        r = cx(jenv, "mesh", "pair", other.code, inv.host, check=False)
        assert r.returncode != 0 and "wrong pair code" in r.stderr, r.stderr
        assert inv.process.poll() is None and is_open(inv.port)
        other.process.send_signal(signal.SIGINT)
        code, _, _ = other.finish()
        assert code != 0
        result["wrong_code_refused_ctrl_c_closes"] = True

        r = cx(jenv, "mesh", "pair", inv.code, inv.host, "--json")
        joined = json.loads(r.stdout)
        assert joined["joined"] and joined["target"] == f"{account}@127.0.0.1" and joined["as"] != "", joined
        code, out, err = inv.finish()
        assert code == 0, err
        assert json.loads(out.splitlines()[-1])["name"] == "rover", out
        result["joined"] = True

        # The viewer: the joiner enrolled as core with a restricted managed key.
        managed = [l for l in authorized(viewer).splitlines() if l.endswith(" cx-managed-peer")]
        assert len(managed) == 1 and managed[0].startswith("no-agent-forwarding,no-port-forwarding,no-X11-forwarding,no-user-rc ssh-ed25519 "), managed
        vdev = [d for d in devices(viewer) if d.get("target")]
        assert [(d["name"], d["id"], d["target"]) for d in vdev] == [("rover", f"joiner-machine:{account}", f"{account}@127.0.0.1")], vdev
        policies = json.loads((viewer / "state/cx/device-policies.json").read_text())
        assert policies["devices"][f"joiner-machine:{account}"]["access"] == "core", policies
        joiner_pub = (joiner / "state/cx/peer-ssh/id_ed25519.pub").read_text().split()[:2]
        assert managed[0].split()[1:3] == joiner_pub, "the viewer authorized exactly the joiner's own key"

        # The joiner: the viewer enrolled and pinned with the code-confirmed host key.
        jdev = [d for d in devices(joiner) if d.get("target")]
        assert [(d["id"], d["target"]) for d in jdev] == [(f"viewer-machine:{account}", f"{account}@127.0.0.1")], jdev
        assert (joiner / "state/cx/peer-ssh/known_hosts").read_text() == f"127.0.0.1 {host_key}\n"
        assert any(l.endswith(" cx-managed-peer") for l in authorized(joiner).splitlines()), "the joiner authorized the viewer"
        result["pinned_both_ways"] = True

        # The joiner's managed identity logs in to a loopback sshd that serves the viewer's authorized_keys.
        sport = free_port()
        configuration = root / "sshd.conf"
        configuration.write_text(f"""Port {sport}
ListenAddress 127.0.0.1
HostKey {root / "host"}
PidFile {root}/sshd.pid
AuthorizedKeysFile {viewer}/.ssh/authorized_keys
StrictModes no
UsePAM no
PasswordAuthentication no
KbdInteractiveAuthentication no
PubkeyAuthentication yes
AllowUsers {account}
""")
        log = (root / "sshd.log").open("wb")
        daemon = subprocess.Popen([sshd, "-D", "-e", "-f", str(configuration)], stdout=log, stderr=log)
        for _ in range(100):
            assert daemon.poll() is None, (root / "sshd.log").read_text()
            if is_open(sport):
                break
            time.sleep(0.02)
        kh = root / "kh"
        kh.write_text(f"[127.0.0.1]:{sport} {host_key}\n")
        probe = subprocess.run(["ssh", "-F", "none", "-i", str(joiner / "state/cx/peer-ssh/id_ed25519"), "-o", "IdentitiesOnly=yes",
                                "-o", "IdentityAgent=none", "-o", "BatchMode=yes", "-o", "StrictHostKeyChecking=yes",
                                "-o", f"UserKnownHostsFile={kh}", "-p", str(sport), f"{account}@127.0.0.1", "echo managed-ok"],
                               capture_output=True, text=True, timeout=20, env=jenv)
        assert probe.stdout.strip() == "managed-ok", probe.stderr
        result["managed_key_authenticates"] = True

        # Replay: the invite is closed, so nothing answers.
        r = cx(jenv, "mesh", "pair", inv.code, inv.host, check=False)
        assert r.returncode != 0 and "no invite is open" in r.stderr, r.stderr
        result["replay_refused"] = True

        # A mistyped code fails its check symbol offline.
        typo = inv.code[:-1] + ("0" if inv.code[-1] != "0" else "1")
        r = cx(jenv, "mesh", "pair", typo, inv.host, check=False)
        assert r.returncode != 0 and "mistyped" in r.stderr, r.stderr

        # Rate limit: after 3 wrong codes within a minute even the right one is refused unchecked.
        limited = Invite()
        invites.append(limited)
        stranger, senv = side("stranger", "stranger-machine")
        for _ in range(3):
            wrong = Invite()
            r = cx(senv, "mesh", "pair", wrong.code, limited.host, check=False)
            assert "wrong pair code" in r.stderr, r.stderr
            wrong.process.send_signal(signal.SIGINT)
            wrong.finish()
        r = cx(senv, "mesh", "pair", limited.code, limited.host, check=False)
        assert r.returncode != 0 and "wait a minute" in r.stderr, r.stderr
        assert limited.process.poll() is None, "still open, unused"
        limited.process.send_signal(signal.SIGINT)
        limited.finish()
        assert not [d for d in devices(viewer) if d["id"].startswith("stranger-machine")]
        result["rate_limited"] = True

        # Expiry closes the listener.
        brief = Invite("--hours", "0.0005")
        invites.append(brief)
        code, _, err = brief.finish(timeout=15)
        assert code != 0 and "expired" in err, err
        result["expiry_closes"] = True

        # Teardown has nothing to cancel; revoke removes the member.
        plan = json.loads(cx(venv, "mesh", "teardown", "--backend", "ssh-overlay", "--yes", "--json").stdout)
        assert not plan["applied"], plan
        members = json.loads(cx(venv, "mesh", "members", "--backend", "ssh-overlay", "--json").stdout)
        assert [m["name"] for m in members if m["state"] != "local"] == ["rover"], members
        assert [m["role"] for m in members if m["name"] == "rover"] == ["core"]
        gone = json.loads(cx(venv, "mesh", "revoke", "rover", "--backend", "ssh-overlay", "--yes", "--json").stdout)
        assert gone["ssh"]["authorizations_removed"] == 1, gone
        assert not any(l.endswith(" cx-managed-peer") for l in authorized(viewer).splitlines())
        # `cx device remove` on the joiner does the same for the viewer: no pin, no managed line, user lines kept.
        (joiner / ".ssh/authorized_keys").write_text(authorized(joiner) + "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl user-own\n")
        vname = [d["name"] for d in devices(joiner) if d.get("target")][0]
        removed = json.loads(cx(jenv, "device", "remove", vname).stdout)
        assert removed["ssh"] == {"unpinned": 1, "authorizations_removed": 1}, removed
        assert not any(l.endswith(" cx-managed-peer") for l in authorized(joiner).splitlines())
        assert authorized(joiner).endswith(" user-own\n"), "the user's own line is kept"
        assert (joiner / "state/cx/peer-ssh/known_hosts").read_text() == ""
        assert not [d for d in devices(joiner) if d.get("target")]
        result["device_remove_revokes_ssh"] = True
        audit = [a["action"] for a in json.loads(cx(venv, "mesh", "audit", "--backend", "ssh-overlay", "--json", "--limit", "100").stdout)]
        assert audit[0] == "revoke" and "join" in audit and "invite-closed" in audit, audit
        result["revoke_and_audit"] = True
        # What would show up in the real files if any of this leaked there.
        markers = {str(root), host_key.split()[1], "viewer-machine", "joiner-machine", "victim-machine", "stranger-machine"}
        markers |= {p.read_text().split()[1] for p in root.glob("*/state/cx/peer-ssh/id_ed25519.pub")}
        assert len(markers) >= 8, markers
    finally:
        for i in invites:
            if i.process.poll() is None:
                i.process.kill()
                i.process.wait(timeout=3)
        if daemon is not None and daemon.poll() is None:
            daemon.terminate()
            daemon.wait(timeout=3)

    # Backends: tailscale adopted read-only from a fake binary on PATH; ranges and collisions reported.
    tools = root / "tools"
    tools.mkdir()
    (tools / "tailscale").write_text("""#!/bin/sh
[ "$1 $2" = "status --json" ] || exit 2
cat <<'EOF'
{"BackendState":"Running","MagicDNSSuffix":"tail1.ts.net","Self":{"HostName":"me","DNSName":"me.tail1.ts.net.","TailscaleIPs":["100.101.2.3"],"Online":true},
 "Peer":{"k":{"HostName":"pi","DNSName":"pi.tail1.ts.net.","OS":"linux","TailscaleIPs":["100.80.0.4"],"Online":true}}}
EOF
""")
    (tools / "tailscale").chmod(0o755)
    env = dict(venv, PATH=f"{tools}{os.pathsep}{os.environ['PATH']}")
    report = json.loads(cx(env, "mesh", "backends", "--json").stdout)
    by = {b["id"]: b for b in report["backends"]}
    assert report["default"] == "ssh-overlay" and list(by) == ["ssh-overlay", "cloudflare-warp", "tailscale"], report
    assert by["tailscale"]["status"]["available"] and by["tailscale"]["status"]["members"] == 2, by["tailscale"]
    assert by["tailscale"]["ranges"] == ["100.64.0.0/10", "fd7a:115c:a1e0::/48"]
    assert by["ssh-overlay"]["ranges"] == [] and "coexists-with-vpn" in by["ssh-overlay"]["capabilities"]
    assert by["cloudflare-warp"]["status"]["provisioned"] is False
    # No WARP config (and no WARP interface here): WARP claims nothing, so no false nested collision.
    assert by["cloudflare-warp"]["status"]["available"] is False, by["cloudflare-warp"]
    if not any(c["owner"] == "cloudflare-warp" for c in report["claims"]):
        assert not any("cloudflare-warp" in json.dumps(c) for c in report["collisions"]), report["collisions"]
    assert "tailscale" in [c["owner"] for c in report["claims"]], report["claims"]
    members = json.loads(cx(env, "mesh", "members", "--backend", "tailscale", "--json").stdout)
    assert [(m["name"], m["dns_name"]) for m in members] == [("me", "me.tail1.ts.net"), ("pi", "pi.tail1.ts.net")], members
    r = cx(env, "mesh", "revoke", "pi", "--backend", "tailscale", "--yes", check=False)
    assert r.returncode != 0 and "read-only" in r.stderr
    plain = cx(venv, "mesh", "backends", "--json")
    assert not {b["id"]: b for b in json.loads(plain.stdout)["backends"]}["tailscale"]["status"]["available"] or shutil.which("tailscale")
    result["backends_report"] = True

after = [digest(p) for p in watched]
for p in watched:
    text = p.read_text(errors="ignore") if p.exists() else ""
    assert not [m for m in markers if m in text], f"this test's data reached {p}"
changed = [str(p) for p, a, b in zip(watched, before, after) if a != b]
print(json.dumps(dict(result="PASS", **result, real_state_untouched=True, changed_meanwhile_by_others=changed)))
