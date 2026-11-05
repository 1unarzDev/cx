#!/usr/bin/env python3
"""Shared SSH masters against a disposable loopback sshd.

Usage: python3 tests/ssh_multiplex.py target/debug/cx

Uses the existing OS account, a private sshd on 127.0.0.1 and an isolated
HOME/XDG_STATE_HOME/XDG_RUNTIME_DIR. An ssh wrapper substitutes a fixture
config and a fixture helper for the remote `cx helper`; no personal SSH
configuration, trust, agent, installed helper or tmux session is used.
Every master the fixture creates is closed before it exits.

Checks: one dedicated `-N` master per route, detached in its own session so
reaping a helper channel's process group (including a ProxyJump hop) never
ends it; helper channels only join (ControlMaster=no, BatchMode); terminals
may join or become a master (ControlMaster=auto, ControlPersist=600); no
agent forwarding; `cx orphans --json` recognises the master (read-only);
`cx device remove` exits it; CX_SSH_MULTIPLEX=0 restores direct connections.
Prints first-request vs warm-request vs attach latency, with and without.
"""
import json, os, pathlib, pwd, shutil, signal, socket, subprocess, sys, tempfile, time

binary = str(pathlib.Path(sys.argv[1]).resolve())
ssh = shutil.which("ssh")
sshd = shutil.which("sshd") or "/usr/sbin/sshd"
account = pwd.getpwuid(os.getuid()).pw_name


def free_port():
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


# Short root so the private socket path fits sun_path.
with tempfile.TemporaryDirectory(prefix="cx-mx-", dir="/tmp") as temporary:
    root = pathlib.Path(temporary)
    root.chmod(0o700)
    home, runtime, tools = root / "home", root / "run", root / "tools"
    for path in (home, runtime, tools):
        path.mkdir(mode=0o700)
    state = root / "state/cx"
    state.mkdir(parents=True, mode=0o700)
    for name in ("host", "key"):
        subprocess.run(["ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-f", str(root / name)],
                       check=True, timeout=10)
    (root / "authorized").write_text((root / "key.pub").read_text())
    (root / "authorized").chmod(0o600)
    port = free_port()
    (root / "sshd.conf").write_text(f"""Port {port}
ListenAddress 127.0.0.1
HostKey {root}/host
PidFile {root}/sshd.pid
AuthorizedKeysFile {root}/authorized
StrictModes no
UsePAM no
PasswordAuthentication no
KbdInteractiveAuthentication no
PubkeyAuthentication yes
AllowUsers {account}
""")
    host_key = (root / "host.pub").read_text().split()
    (root / "known").write_text(f"[127.0.0.1]:{port} {host_key[0]} {host_key[1]}\n")
    block = f""" HostName 127.0.0.1
 Port {port}
 User {account}
 IdentityFile {root}/key
 IdentitiesOnly yes
 IdentityAgent none
 UserKnownHostsFile {root}/known
 GlobalKnownHostsFile /dev/null
 StrictHostKeyChecking yes
"""
    (root / "client.conf").write_text(f"Host fixture fixture-routed\n{block}")

    helper = root / "remote-helper.py"
    helper.write_text("""import json,sys
session=dict(id='$1',name='fixture',directory='/tmp',provider='shell',host='fixture',
 account='fixture',pid=1,started='0',boot_id='fixture',external=True,socket=None)
while True:
 header=sys.stdin.buffer.readline()
 if not header:break
 prefix,size=header.split();assert prefix==b'CX1'
 request=json.loads(sys.stdin.buffer.read(int(size)))
 op=request['op']['op'];assert op=='sessions',op
 response=json.dumps(dict(version=1,id=request['id'],result=[session],error=None)).encode()
 sys.stdout.buffer.write(b'CX1 '+str(len(response)).encode()+b'\\n'+response);sys.stdout.buffer.flush()
""")
    wrapper = tools / "ssh"
    wrapper.write_text(f"""#!{sys.executable}
import json,os,sys
args=sys.argv[1:]
if '-F' in args:
 pos=args.index('-F');del args[pos:pos+2]
with open(os.environ['CX_MX_LOG'],'a') as log:log.write(json.dumps(args)+chr(10))
if args and args[-1]=='exec ~/.local/bin/cx helper':args[-1]=os.environ['CX_MX_HELPER']
elif args and args[-1].startswith('~/.local/bin/cx native-attach'):args[-1]='true'
os.execv({ssh!r},[{ssh!r},'-F',os.environ['CX_MX_CONFIG'],*args])
""")
    wrapper.chmod(0o700)

    device = lambda name, target: dict(id=f"fixture:{name}", name=name, target=target,
                                       account=account, host="fixture", status="unknown",
                                       observed_at=0)
    devices = [device("fixture", "fixture"), device("routed", "fixture-routed")]

    def write_private(path, text):
        path.write_text(text)
        path.chmod(0o600)

    write_private(state / "devices.json", json.dumps(devices))
    # cx route: fixture-routed is reached through the fixture hop (-J).
    write_private(state / "routes.json", json.dumps({"fixture-routed": ["fixture"]}))
    log_path = root / "argv.jsonl"
    keep = {k: v for k, v in os.environ.items()
            if k in ("PATH", "LANG", "LC_ALL", "TERM") or k.startswith("CARGO")}
    base_env = dict(keep, HOME=str(home), USER=account, XDG_STATE_HOME=str(root / "state"),
                    XDG_RUNTIME_DIR=str(runtime), PATH=str(tools) + os.pathsep + keep["PATH"],
                    CX_MX_LOG=str(log_path), CX_MX_CONFIG=str(root / "client.conf"),
                    CX_MX_HELPER=f"{sys.executable} {helper}")

    def cx(*args, multiplex=True, timeout=30):
        env = dict(base_env)
        if not multiplex:
            env["CX_SSH_MULTIPLEX"] = "0"
        started = time.monotonic()
        result = subprocess.run([binary, *args], env=env, stdin=subprocess.DEVNULL,
                                capture_output=True, timeout=timeout, start_new_session=True)
        elapsed = time.monotonic() - started
        assert result.returncode == 0, (args, result.stderr[-2000:])
        return result, elapsed

    def argv():
        return [json.loads(l) for l in log_path.read_text().splitlines()] if log_path.exists() else []

    def sockets():
        return sorted(p for p in (state / "ssh").glob("*") if p.is_socket())

    def master_pids():
        pids = []
        for candidate in sockets():
            out = subprocess.run([ssh, "-F", "/dev/null", "-S", str(candidate), "-O", "check", "cx"],
                                 capture_output=True, text=True, timeout=3).stderr
            if "pid=" in out:
                pids.append(int(out.split("pid=")[1].split(")")[0]))
        return pids

    def close_all():
        for candidate in sockets():
            subprocess.run([ssh, "-F", "/dev/null", "-S", str(candidate), "-O", "exit", "cx"],
                           stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=3)

    def alive(pid):
        try:
            os.kill(pid, 0)
            return True
        except ProcessLookupError:
            return False

    daemon_log = open(root / "sshd.log", "wb")
    daemon = subprocess.Popen([sshd, "-D", "-e", "-f", str(root / "sshd.conf")],
                              stdout=daemon_log, stderr=daemon_log, start_new_session=True)
    timings = {}
    try:
        for _ in range(100):
            assert daemon.poll() is None, (root / "sshd.log").read_text()
            try:
                with socket.create_connection(("127.0.0.1", port), timeout=0.1):
                    break
            except OSError:
                time.sleep(0.02)

        # Before: every request and attach pays a full handshake.
        for name in ("fixture", "routed"):
            _, timings[f"{name}_off_first"] = cx("sessions", "--device", name, multiplex=False)
            _, timings[f"{name}_off_second"] = cx("sessions", "--device", name, multiplex=False)
            _, timings[f"{name}_off_attach"] = cx("attach", "$1", "--device", name, multiplex=False)
        assert not sockets(), "multiplexing opt-out must not create masters"
        assert all("ControlPath=none" in a and "ControlMaster=no" in a for a in argv())

        # After: first contact creates one detached master per route.
        log_path.write_text("")
        for name in ("fixture", "routed"):
            result, timings[f"{name}_on_first"] = cx("sessions", "--device", name)
            assert json.loads(result.stdout)[0]["id"] == "$1"
        masters = argv()
        assert len([a for a in masters if "-N" in a]) == 2, masters
        assert len(sockets()) == 2 and all(p.stat().st_mode & 0o077 == 0 for p in sockets())
        pids = master_pids()
        assert len(pids) == 2
        for pid in pids:
            stat = pathlib.Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()
            assert int(stat[3]) == pid, "master must lead its own session"
        # The cx processes (and their reaped helper groups, including the
        # routed jump hop) are gone; both masters survive and are reused.
        log_path.write_text("")
        for name in ("fixture", "routed"):
            _, timings[f"{name}_on_second"] = cx("sessions", "--device", name)
            _, timings[f"{name}_on_attach"] = cx("attach", "$1", "--device", name)
        assert sorted(master_pids()) == sorted(pids), "a master was replaced"
        reused = argv()
        assert not [a for a in reused if "-N" in a], "warm requests must not open a master"
        helpers = [a for a in reused if a[-1] == "exec ~/.local/bin/cx helper"]
        terminals = [a for a in reused if "-t" in a]
        assert helpers and terminals
        for a in helpers:
            assert "BatchMode=yes" in a and "ControlMaster=no" in a and "ControlPersist=600" not in a
        for a in terminals:
            assert "ControlMaster=auto" in a and "ControlPersist=600" in a and "BatchMode=yes" not in a
        for a in reused:
            assert "ForwardAgent=no" in a and "-A" not in a
            assert any(o.startswith("ControlPath=") and str(state / "ssh") in o for o in a)

        # Read-only orphan scan recognises both masters as enrolled and in use-able.
        result, _ = cx("orphans", "--json")
        found = {m["pid"]: m for m in json.loads(result.stdout)["ssh_masters"]}
        assert set(pids) <= set(found), found
        assert all(found[p]["enrolled"] and found[p]["socket_present"] and not found[p]["stray"]
                   for p in pids)

        # Removing a device exits its master now, not after ControlPersist.
        before = set(master_pids())
        cx("device", "remove", "fixture")
        time.sleep(0.2)
        remaining = set(master_pids())
        assert len(remaining) == 1 and remaining < before
        gone = (before - remaining).pop()
        assert not alive(gone), "removed device's master still running"

        # A master for a device that left the inventory without cleanup is stray.
        write_private(state / "devices.json", json.dumps([device("fixture", "fixture")]))
        result, _ = cx("orphans", "--json")
        found = {m["pid"]: m for m in json.loads(result.stdout)["ssh_masters"]}
        routed = remaining.pop()
        assert found[routed]["stray"] and not found[routed]["enrolled"], found

        # Concurrent cold starts from separate viewers converge on one master
        # and leave no stray master attempt behind.
        close_all()
        time.sleep(0.2)
        env = dict(base_env)
        racers = [subprocess.Popen([binary, "sessions", "--device", "fixture"], env=env,
                                   stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                                   stderr=subprocess.PIPE, start_new_session=True)
                  for _ in range(3)]
        for racer in racers:
            out, err = racer.communicate(timeout=60)
            assert racer.returncode == 0, err[-2000:]
        time.sleep(0.3)
        assert len(master_pids()) == 1, sockets()
        attempts = []
        for proc in pathlib.Path("/proc").iterdir():
            if not proc.name.isdigit():
                continue
            try:
                cmdline = (proc / "cmdline").read_bytes().split(b"\0")
            except OSError:
                continue
            if b"-N" in cmdline and any(str(state).encode() in a for a in cmdline):
                attempts.append(int(proc.name))
        assert attempts == master_pids(), (attempts, master_pids())
    finally:
        close_all()
        if daemon.poll() is None:
            os.killpg(daemon.pid, signal.SIGTERM)
        daemon.wait(timeout=3)
        daemon_log.close()

ms = {k: round(v * 1000) for k, v in timings.items()}
print(json.dumps(dict(result="PASS", detached_master_per_route=True, helper_group_reap_keeps_master=True,
                      jump_route_master_survives=True, helpers_join_only=True,
                      terminals_join_or_master=True, no_agent_forwarding=True,
                      orphans_recognise_masters=True, device_remove_exits_master=True, concurrent_cold_start_one_master=True,
                      opt_out_direct=True, latency_ms=ms)))
