#!/usr/bin/env python3
"""ssh-overlay `cx join` end to end over a disposable loopback sshd (docs/mesh.md, "ssh-overlay").

Two cx identities on this machine, each with its own temporary HOME and state: the viewer invites,
a private sshd (running as this user, authorizing only the viewer's temporary authorized_keys, with
SetEnv pointing the forced command at the viewer's temporary HOME) serves the redeem, the joiner joins.
Also: replay refused, a changed host key refused before any authentication, revoke, and
`cx mesh backends --json` with a fake tailscale on PATH. Nothing outside the temporary tree changes;
the real ~/.ssh/authorized_keys and cx state are checked unchanged (stat only).

Usage: python3 tests/mesh_ssh_overlay.py target/debug/cx
"""
import json
import os
import pathlib
import pwd
import shutil
import socket
import subprocess
import sys
import tempfile
import time

binary = str(pathlib.Path(sys.argv[1]).resolve())
account = pwd.getpwuid(os.getuid()).pw_name
sshd = shutil.which("sshd") or "/usr/sbin/sshd"
real_home = pathlib.Path(pwd.getpwuid(os.getuid()).pw_dir)
watched = [real_home / ".ssh/authorized_keys", real_home / ".local/state/cx/devices.json", real_home / ".config/cx/mesh/config.toml"]


def stamp(p):
    try:
        s = p.stat()
        return (s.st_mtime_ns, s.st_size)
    except FileNotFoundError:
        return None


before = [stamp(p) for p in watched]
result = {}

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
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        port = sock.getsockname()[1]
    for name in ("host", "changed-host"):
        subprocess.run(["ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-f", str(root / name)], check=True, timeout=10)

    def cx(env, *args, input=None, check=True):
        r = subprocess.run([binary, *args], env=env, input=input, capture_output=True, text=True, timeout=60)
        if check:
            assert r.returncode == 0, (args, r.stdout, r.stderr)
        return r

    def invite(*extra):
        r = cx(venv, "mesh", "invite", "--backend", "ssh-overlay", "--address", f"127.0.0.1:{port}",
               "--host-key", str(root / "host.pub"), "--json", *extra)
        return json.loads(r.stdout)

    configuration = root / "sshd.conf"
    log = (root / "sshd.log").open("wb")
    daemon = None

    def start(key):
        setenv = " ".join(f"{k}={venv[k]}" for k in ("HOME", "XDG_STATE_HOME", "XDG_CONFIG_HOME", "XDG_CACHE_HOME", "CX_JOIN_MACHINE_ID"))
        configuration.write_text(f"""Port {port}
ListenAddress 127.0.0.1
HostKey {root / key}
PidFile {root}/sshd.pid
AuthorizedKeysFile {viewer}/.ssh/authorized_keys
StrictModes no
UsePAM no
PasswordAuthentication no
KbdInteractiveAuthentication no
PubkeyAuthentication yes
AllowUsers {account}
SetEnv {setenv}
""")
        process = subprocess.Popen([sshd, "-D", "-e", "-f", str(configuration)], stdout=log, stderr=log)
        for _ in range(100):
            assert process.poll() is None, (root / "sshd.log").read_text()
            try:
                with socket.create_connection(("127.0.0.1", port), timeout=0.1):
                    return process
            except OSError:
                time.sleep(0.02)
        raise AssertionError("fixture sshd did not start")

    def authorized(home):
        p = home / ".ssh/authorized_keys"
        return p.read_text() if p.exists() else ""

    def devices(home):
        p = home / "state/cx/devices.json"
        return json.loads(p.read_text()) if p.exists() else []

    try:
        inv = invite("--name", "rover")
        code = inv["code"]
        assert inv["one_liner_sh"] == f"cx join {code}" and inv["uses"] == 1, inv
        line = [l for l in authorized(viewer).splitlines() if "cx-join-invite:" in l]
        assert len(line) == 1 and line[0].startswith("restrict,command=") and "expiry-time=" in line[0], line
        assert f"{binary} join --redeem " in line[0]

        daemon = start("host")
        # Takeover: an invite holder claiming an enrolled identity (here a directed device with no
        # SSH pins) is refused, and the record keeps its target and access. Runs before any pairing
        # so that no 127.0.0.1 endpoint pin masks the identity check.
        victim = {"id": f"victim-machine:{account}", "name": "victim", "target": f"{account}@10.9.9.9",
                  "account": account, "host": "victim", "status": "seeded", "observed_at": 1}
        (viewer / "state/cx/devices.json").write_text(json.dumps([d for d in devices(viewer)] + [victim]))
        policy_file = viewer / "state/cx/device-policies.json"
        policies_before = policy_file.read_text() if policy_file.exists() else None
        intruder, ienv = side("intruder", "victim-machine")
        r = cx(ienv, "join", invite()["code"], "--name", "mallory", check=False)
        assert r.returncode != 0 and "already enrolled" in r.stderr, r.stderr
        assert [d for d in devices(viewer) if d["id"] == victim["id"]] == [victim], devices(viewer)
        assert (policy_file.read_text() if policy_file.exists() else None) == policies_before
        assert not any(l.endswith(" cx-managed-peer") for l in authorized(viewer).splitlines())
        pins = viewer / "state/cx/peer-ssh/peers.json"
        assert not pins.exists() or "victim-machine" not in pins.read_text()
        (viewer / "state/cx/devices.json").write_text(json.dumps([d for d in devices(viewer) if d["id"] != victim["id"]]))
        result["takeover_refused"] = True

        r = cx(jenv, "join", code, "--json")
        joined = json.loads(r.stdout)
        assert joined["joined"] and joined["target"] == f"{account}@127.0.0.1" and joined["as"] != "", joined
        result["joined"] = True

        # The viewer: the invite is burnt, the joiner enrolled as core with a restricted managed key.
        assert "cx-join-invite:" not in authorized(viewer), "invite authorization removed after use"
        managed = [l for l in authorized(viewer).splitlines() if l.endswith(" cx-managed-peer")]
        assert len(managed) == 1 and managed[0].startswith("no-agent-forwarding,no-port-forwarding,no-X11-forwarding,no-user-rc ssh-ed25519 "), managed
        vdev = [d for d in devices(viewer) if d.get("target")]
        assert [(d["name"], d["id"], d["target"]) for d in vdev] == [("rover", f"joiner-machine:{account}", f"{account}@127.0.0.1")], vdev
        policies = json.loads((viewer / "state/cx/device-policies.json").read_text())
        assert policies["devices"][f"joiner-machine:{account}"]["access"] == "core", policies
        joiner_pub = (joiner / "state/cx/peer-ssh/id_ed25519.pub").read_text().split()[:2]
        assert managed[0].split()[1:3] == joiner_pub, "the viewer authorized exactly the joiner's own key"

        # The joiner: the viewer enrolled and pinned with the code-verified host key.
        jdev = [d for d in devices(joiner) if d.get("target")]
        assert [(d["id"], d["target"]) for d in jdev] == [(f"viewer-machine:{account}", f"{account}@127.0.0.1")], jdev
        host_key = " ".join((root / "host.pub").read_text().split()[:2])
        assert (joiner / "state/cx/peer-ssh/known_hosts").read_text() == f"127.0.0.1 {host_key}\n"
        assert any(l.endswith(" cx-managed-peer") for l in authorized(joiner).splitlines()), "the joiner authorized the viewer"
        result["pinned_both_ways"] = True

        # The joiner's managed identity now authenticates to the viewer's sshd (strict host key).
        kh = root / "kh"
        kh.write_text(f"[127.0.0.1]:{port} {host_key}\n")
        probe = subprocess.run(["ssh", "-F", "none", "-i", str(joiner / "state/cx/peer-ssh/id_ed25519"), "-o", "IdentitiesOnly=yes",
                                "-o", "IdentityAgent=none", "-o", "BatchMode=yes", "-o", "StrictHostKeyChecking=yes",
                                "-o", f"UserKnownHostsFile={kh}", "-p", str(port), f"{account}@127.0.0.1", "echo managed-ok"],
                               capture_output=True, text=True, timeout=20, env=jenv)
        assert probe.stdout.strip() == "managed-ok", probe.stderr
        result["managed_key_authenticates"] = True

        # Replay: the same code no longer authenticates.
        r = cx(jenv, "join", code, check=False)
        assert r.returncode != 0 and "could not join" in r.stderr, r.stderr
        result["replay_refused"] = True

        # A changed host key is refused before authenticating; the invite stays pending.
        second = invite()
        daemon.terminate()
        daemon.wait(timeout=3)
        daemon = start("changed-host")
        r = cx(jenv, "join", second["code"], "--name", "rover2", check=False)
        assert r.returncode != 0 and "different SSH host key" in r.stderr, r.stderr
        assert "cx-join-invite:" in authorized(viewer), "never connected, so the invite is still pending"
        result["changed_host_key_refused"] = True

        # A mistyped code fails its checksum offline.
        typo = second["code"][:-1] + ("0" if second["code"][-1] != "0" else "1")
        r = cx(jenv, "join", typo, check=False)
        assert r.returncode != 0 and ("checksum" in r.stderr or "mistyped" in r.stderr), r.stderr

        # Teardown cancels pending invites (and their authorizations); revoke removes the member.
        plan = json.loads(cx(venv, "mesh", "teardown", "--backend", "ssh-overlay", "--yes", "--json").stdout)
        assert plan["applied"] and len(plan["steps"]) == 1, plan
        assert "cx-join-invite:" not in authorized(viewer)
        members = json.loads(cx(venv, "mesh", "members", "--backend", "ssh-overlay", "--json").stdout)
        assert [m["name"] for m in members if m["state"] != "local"] == ["rover"], members
        assert [m["role"] for m in members if m["name"] == "rover"] == ["core"]
        gone = json.loads(cx(venv, "mesh", "revoke", "rover", "--backend", "ssh-overlay", "--yes", "--json").stdout)
        assert gone["ssh"]["authorizations_removed"] == 1, gone
        assert not any(l.endswith(" cx-managed-peer") for l in authorized(viewer).splitlines())
        audit = json.loads(cx(venv, "mesh", "audit", "--backend", "ssh-overlay", "--json").stdout)
        assert [a["action"] for a in audit][:2] == ["revoke", "teardown"], audit
        assert "join" in [a["action"] for a in audit]
        result["revoke_and_teardown"] = True

        # --short end to end: cx -> embedded mesh.py -> the mesh Worker (local HTTPS fake) -> cx join <host>/<code>.
        node = shutil.which("node")
        assert node, "node is needed for the Worker fixture"
        daemon.terminate()
        daemon.wait(timeout=3)
        daemon = start("host")
        subprocess.run(["openssl", "req", "-x509", "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:prime256v1", "-nodes",
                        "-keyout", str(root / "tls.key"), "-out", str(root / "tls.crt"), "-days", "1", "-subj", "/CN=localhost",
                        "-addext", "subjectAltName=DNS:localhost"], check=True, capture_output=True, timeout=30)
        with socket.socket() as sock:
            sock.bind(("127.0.0.1", 0))
            wport = sock.getsockname()[1]
        owner = "owner-test-token"
        worker = subprocess.Popen([node, str(pathlib.Path(__file__).resolve().parent / "mesh_worker_server.mjs"),
                                   str(root / "tls.crt"), str(root / "tls.key"), str(wport), owner],
                                  stdout=subprocess.PIPE, stderr=log, text=True)
        try:
            assert worker.stdout.readline().strip() == "ready"
            mesh_dir = viewer / "config/cx/mesh"
            mesh_dir.mkdir(parents=True)
            (mesh_dir / "config.toml").write_text('name = "demo"\naccount = "acct"\n')
            (mesh_dir / "demo.state.json").write_text(json.dumps({"url": f"https://localhost:{wport}", "account": "acct"}))
            tls = {"SSL_CERT_FILE": str(root / "tls.crt")}
            senv = dict(venv, CLOUDFLARE_API_TOKEN=owner, **tls)
            short = json.loads(cx(senv, "mesh", "invite", "--backend", "ssh-overlay", "--address",
                                                                 f"127.0.0.1:{port}", "--host-key", str(root / "host.pub"),
                                                                 "--short", "--json").stdout)
            sc = short["short_code"]
            assert len(sc.replace("-", "")) == 12 and short["one_liner_sh"] == f"cx join localhost:{wport}/{sc}", short
            assert short["url"] == f"https://localhost:{wport}/p/{sc}", short
            assert short["code"] not in json.dumps(short["one_liner_sh"])
            shorty, shenv = side("shorty", "shorty-machine")
            shenv.update(tls)
            r = cx(shenv, "join", f"localhost:{wport}/{sc}", "--name", "shorty", "--json")
            j = json.loads(r.stdout)
            assert j["joined"] and j["target"] == f"{account}@127.0.0.1", j
            assert [d["name"] for d in devices(viewer) if d.get("target")] == ["shorty"], devices(viewer)
            again = cx(shenv, "join", f"localhost:{wport}/{sc}", check=False)
            assert again.returncode != 0 and "already used" in again.stderr, again.stderr
            result["short_code_end_to_end"] = True
        finally:
            worker.terminate()
            worker.wait(timeout=3)
            shutil.rmtree(viewer / "config/cx/mesh", ignore_errors=True)
    finally:
        if daemon is not None and daemon.poll() is None:
            daemon.terminate()
            daemon.wait(timeout=3)
        log.close()

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

after = [stamp(p) for p in watched]
assert before == after, "real ~/.ssh and cx state untouched"
print(json.dumps(dict(result="PASS", **result, real_state_untouched=True)))
