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
ends it; helper channels and terminals only join (ControlMaster=no), even on
terminal-first contact, where cx starts the dedicated master instead; each
master has a 0600 owner record naming its exact target; no agent
forwarding; `cx device remove` exits it; `cx prewarm` opens one and replaces a master whose
link stopped answering; CX_SSH_MULTIPLEX=0 restores direct connections.
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
import json,os,sys,time
args=sys.argv[1:]
if '-F' in args:
 pos=args.index('-F');del args[pos:pos+2]
with open(os.environ['CX_MX_LOG'],'a') as log:log.write(json.dumps(args)+chr(10))
if '-N' in args and os.environ.get('CX_MX_MASTER_DELAY'):time.sleep(float(os.environ['CX_MX_MASTER_DELAY']))
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

    def cx(*args, multiplex=True, timeout=30, extra=None):
        env = dict(base_env, **(extra or {}))
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

    def session_leader(pid):
        stat = pathlib.Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()
        return int(stat[3]) == pid

    def wait_gone(paths, limit=5):
        deadline = time.monotonic() + limit
        while time.monotonic() < deadline and any(p.exists() for p in paths):
            time.sleep(0.05)

    def descendants(root_pid):
        children = {}
        for proc in pathlib.Path("/proc").iterdir():
            if proc.name.isdigit():
                try:
                    ppid = int((proc / "stat").read_text().rsplit(")", 1)[1].split()[1])
                except (OSError, IndexError, ValueError):
                    continue
                children.setdefault(ppid, []).append(int(proc.name))
        found, queue = [], [root_pid]
        while queue:
            for child in children.get(queue.pop(), []):
                found.append(child)
                queue.append(child)
        return found

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
    frozen = []
    started_masters = set()
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
        started_masters.update(pids)
        assert len(pids) == 2
        for pid in pids:
            assert session_leader(pid), "master must lead its own session"
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
            assert "ControlMaster=no" in a and "ControlPersist=600" not in a and "BatchMode=yes" not in a
            assert "ControlMaster=auto" not in a, "a terminal must never become the master"
        # Every master records its exact target in a private sidecar.
        owners = {}
        for candidate in sockets():
            record = candidate.with_name(candidate.name + ".owner")
            assert record.stat().st_mode & 0o777 == 0o600, record
            owners[candidate.name] = json.loads(record.read_text())["target"]
        assert sorted(owners.values()) == ["fixture", "fixture-routed"], owners
        for a in reused:
            assert "ForwardAgent=no" in a and "-A" not in a
            assert any(o.startswith("ControlPath=") and str(state / "ssh") in o for o in a)

        # Removing a device exits its master now, not after ControlPersist.
        before = set(master_pids())
        cx("device", "remove", "fixture")
        time.sleep(0.2)
        remaining = set(master_pids())
        assert len(remaining) == 1 and remaining < before
        gone = (before - remaining).pop()
        assert not alive(gone), "removed device's master still running"
        write_private(state / "devices.json", json.dumps([device("fixture", "fixture")]))

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

        # Terminal-first contact: cx starts the dedicated master; the terminal
        # itself only joins (or goes direct) and never becomes the master.
        close_all()
        wait_gone(sockets())
        log_path.write_text("")
        _, timings["fixture_cold_attach"] = cx("attach", "$1", "--device", "fixture")
        cold = argv()
        assert len([a for a in cold if "-N" in a]) == 1, cold
        cold_terminals = [a for a in cold if "-t" in a]
        assert cold_terminals and all("ControlMaster=no" in a and "ControlMaster=auto" not in a
                                      for a in cold_terminals)
        time.sleep(0.2)
        [terminal_master] = master_pids()
        started_masters.add(terminal_master)
        assert session_leader(terminal_master)
        _, timings["fixture_warm_attach"] = cx("attach", "$1", "--device", "fixture")
        assert master_pids() == [terminal_master], "the attach's master must outlive it"

        # A slow master handshake never holds a request: the helper channel
        # waits about 1.5 s, connects directly, and the master keeps
        # starting in the background (it outlives the cx process).
        close_all()
        wait_gone(sockets())
        log_path.write_text("")
        result, timings["fixture_slow_master_request"] = cx(
            "sessions", "--device", "fixture", extra={"CX_MX_MASTER_DELAY": "4"})
        assert json.loads(result.stdout)[0]["id"] == "$1"
        assert timings["fixture_slow_master_request"] < 2.5, timings
        slow = argv()
        assert len([a for a in slow if "-N" in a]) == 1, slow
        deadline = time.monotonic() + 15
        while time.monotonic() < deadline and not master_pids():
            time.sleep(0.1)
        [late] = master_pids()
        started_masters.add(late)
        assert session_leader(late)

        # cx prewarm opens the route's master without any request.
        write_private(state / "devices.json", json.dumps(devices))
        close_all()
        wait_gone(sockets())
        log_path.write_text("")
        result, timings["routed_prewarm"] = cx("prewarm", "routed")
        assert result.stdout.decode().strip() == "ready", result.stdout
        assert len([a for a in argv() if "-N" in a]) == 1, argv()
        [prewarmed] = master_pids()
        started_masters.add(prewarmed)
        log_path.write_text("")
        result, timings["routed_after_prewarm"] = cx("sessions", "--device", "routed")
        assert json.loads(result.stdout)[0]["id"] == "$1"
        assert not [a for a in argv() if "-N" in a], "prewarmed master was not reused"
        result, _ = cx("prewarm", "routed")
        assert result.stdout.decode().strip() == "ready" and master_pids() == [prewarmed]

        # Silent link loss: the master still answers -O check, but its server
        # side is frozen. Prewarm (like a timed-out request) probes it with a
        # bounded channel, stops it from taking clients, and starts a fresh one.
        close_all()
        wait_gone(sockets())
        cx("prewarm", "fixture")
        [stale] = master_pids()
        started_masters.add(stale)
        frozen = descendants(daemon.pid)
        assert frozen
        for pid in frozen:
            os.kill(pid, signal.SIGSTOP)
        result, timings["fixture_replace_stuck"] = cx("prewarm", "fixture")
        assert "replaced" in result.stdout.decode(), result.stdout
        [fresh] = master_pids()
        started_masters.add(fresh)
        assert fresh != stale
        _, timings["fixture_after_replace"] = cx("sessions", "--device", "fixture")
        assert master_pids() == [fresh]
    finally:
        for pid in frozen:
            try:
                os.kill(pid, signal.SIGCONT)
            except ProcessLookupError:
                pass
        close_all()
        for pid in started_masters:
            try:
                os.kill(pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
        if daemon.poll() is None:
            os.killpg(daemon.pid, signal.SIGTERM)
        daemon.wait(timeout=3)
        daemon_log.close()

ms = {k: round(v * 1000) for k, v in timings.items()}
print(json.dumps(dict(result="PASS", detached_master_per_route=True, helper_group_reap_keeps_master=True,
                      jump_route_master_survives=True, helpers_join_only=True,
                      terminals_never_master=True, terminal_first_contact_dedicated_master=True,
                      owner_records_exact_target=True, prewarm_cli=True, slow_master_bounded_wait=True, stuck_master_replaced=True, no_agent_forwarding=True,
                      orphans_recognise_masters=True, device_remove_exits_master=True, concurrent_cold_start_one_master=True,
                      opt_out_direct=True, latency_ms=ms)))
