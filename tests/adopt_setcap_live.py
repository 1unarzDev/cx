#!/usr/bin/env python3
"""Move terminals the way a real install does: a setcap relay run by an unprivileged user.

A relay that gains cap_sys_ptrace from a file capability (or that sudo ran and that
then dropped to the user) is non-dumpable, so the user can no longer read its
/proc/<pid>/fd. The move must still be seen (from the holder and the leader), the
record finalized, and the frozen source window killed before its owner can close it:
a foot-like emulator here ignores SIGHUP and, once its window is empty, "is closed by
the user" two seconds later, signalling the moved terminal's process group HUP, TERM
and then KILL, as foot does. Also covers healing a record a failed check left stale
and stopping an outside terminal's agent (StopExternal).

Runs in a user namespace: ns uid 0 is the caller (sets up the capability), ns uid 1
(a subuid) plays the user. No real terminal, agent or tmux server is touched.
"""
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import tempfile
import time

binary = str(Path(sys.argv[1]).resolve())
here = Path(__file__).resolve().parent


def _subid(path, name):
    try:
        for line in Path(path).read_text().splitlines():
            who, base, count = line.split(':')
            if who in (name, str(os.getuid())) and int(count) >= 1:
                return int(base)
    except OSError:
        pass
    return None


if os.environ.get('CX_ADOPT_NS') != '1':
    import pwd
    name = pwd.getpwuid(os.getuid()).pw_name
    sub_u, sub_g = _subid('/etc/subuid', name), _subid('/etc/subgid', name)
    if sub_u is None or sub_g is None or not shutil.which('newuidmap') or not shutil.which('setcap'):
        print(json.dumps(dict(result='SKIP', reason='no subuid range, newuidmap or setcap')))
        sys.exit(0)
    maps = ['--map-users', f'0:{os.getuid()}:1', '--map-users', f'1:{sub_u}:1',
            '--map-groups', f'0:{os.getgid()}:1', '--map-groups', f'1:{sub_g}:1']
    if subprocess.run(['unshare', '-U', *maps, 'true'], capture_output=True).returncode != 0:
        print(json.dumps(dict(result='SKIP', reason='user namespace mapping unavailable')))
        sys.exit(0)
    os.environ['CX_ADOPT_NS'] = '1'
    # Own pid namespace and /proc, and tests/isolate.sh's marker in a private /dev/shm
    # (a tmpfs of this namespace, so a file capability set there is honoured).
    os.execvp('unshare', ['unshare', '-U', *maps, '--pid', '--fork', '--mount', '--mount-proc',
                          'sh', '-c', 'mount -t tmpfs tmpfs /dev/shm && : >/dev/shm/cx-isolated && exec "$@"',
                          'sh', sys.executable, *sys.argv])

EMULATOR = (here / 'adopt_terminal_live.py').read_text().split("EMULATOR = r'''", 1)[1].split("'''", 1)[0]
# foot: SIGHUP is ignored; an emptied window stays up until the user closes it, and
# closing it signals the child's process group HUP, then TERM, then KILL.
FOOT = EMULATOR.replace("import fcntl,", "import signal; signal.signal(signal.SIGHUP, signal.SIG_IGN)\nimport fcntl,", 1).replace(
    "os.close(master)\nlog.write('closed\\n')",
    "log.write('frozen\\n')\ntime.sleep(2)\n"
    "for s in (signal.SIGHUP, signal.SIGTERM, signal.SIGKILL):\n"
    "    log.write(f'close {s}\\n')\n"
    "    try: os.killpg(os.getpgid(pid), s)\n"
    "    except OSError: pass\n"
    "    time.sleep(1)\n"
    "os.close(master)\nlog.write('closed\\n')", 1)
assert FOOT.count('killpg') == 1 and 'SIG_IGN' in FOOT
USER = ['setpriv', '--reuid=1', '--regid=1', '--clear-groups', '--']


def stat(pid):
    f = Path(f'/proc/{pid}/stat').read_text().rsplit(')', 1)[1].split()
    return dict(state=f[0], ppid=int(f[1]), pgrp=int(f[2]), sid=int(f[3]), start=f[19])


def alive(pid):
    try:
        return stat(pid)['state'] != 'Z'
    except OSError:
        return False


def children(pid):
    out = []
    for task in os.listdir(f'/proc/{pid}/task'):
        try:
            out += [int(c) for c in Path(f'/proc/{pid}/task/{task}/children').read_text().split()]
        except OSError:
            pass
    return out


def exe(pid):
    try:
        return os.readlink(f'/proc/{pid}/exe')
    except OSError:
        return None


def wait_for(predicate, seconds=10, message='condition'):
    deadline = time.time() + seconds
    while time.time() < deadline:
        value = predicate()
        if value:
            return value
        time.sleep(0.05)
    raise AssertionError(f'timed out waiting for {message}')


checks = []
home = Path(tempfile.mkdtemp(prefix='cx-setcap-', dir='/dev/shm'))
bins = home / 'bin'
bins.mkdir()
root = home / 'state/cx'
root.mkdir(parents=True, mode=0o700)
socket = root / 'managed.sock'
subprocess.run(['cc', '-O1', '-pthread', '-o', str(bins / 'claude'), str(here / 'adopt_churn.c')], check=True)
stub = os.path.realpath(bins / 'claude')
source = os.environ.get('CX_ADOPT_REPTYR')
relay = str(home / 'cx-reptyr')
if source:
    shutil.copyfile(source, relay)
else:
    subprocess.run(['sh', str(here.parent / 'scripts/build-portable-reptyr.sh'), relay],
                   env=dict(os.environ, CX_REPTYR_DYNAMIC='1'), check=True, stdout=subprocess.DEVNULL)
os.chmod(relay, 0o755)
subprocess.run(['setcap', 'cap_sys_ptrace+ep', relay], check=True)
for path in [home, *home.rglob('*')]:
    if path != Path(relay):
        os.chown(path, 1, 1)
env = dict(os.environ, HOME=str(home), XDG_STATE_HOME=str(home / 'state'), TMUX_TMPDIR=str(home),
           SHELL='/bin/bash', TERM='xterm-256color', PATH=f'{bins}:{os.environ["PATH"]}',
           CX_ADOPT_REPTYR=relay)
for key in ('TMUX', 'TMUX_PANE', 'DBUS_SESSION_BUS_ADDRESS'):
    env.pop(key, None)
boot = Path('/proc/sys/kernel/random/boot_id').read_text().strip()
emulators = []


def tmux(*args, check=True):
    return subprocess.run([*USER, 'tmux', '-f', '/dev/null', '-S', str(socket), *args], capture_output=True,
                          text=True, check=check, timeout=5, env=env)


def helper(op, args=None):
    body = dict(op=op) if args is None else dict(op=op, args=args)
    request = json.dumps(dict(version=1, id='setcap', op=body)).encode()
    wire = b'CX1 ' + str(len(request)).encode() + b'\n' + request
    result = subprocess.run([*USER, binary, 'helper'], input=wire, env=env, capture_output=True, timeout=30)
    assert result.returncode == 0, result.stderr.decode()
    return json.loads(result.stdout.split(b'\n', 1)[1])


def emulate(name, program=FOOT):
    log = home / f'{name}.emulator'
    process = subprocess.Popen([*USER, sys.executable, '-c', program, str(log), str(bins / 'claude')],
                               env=dict(env, CHURN_LOG=str(home / f"{name}.churn")), stdin=subprocess.DEVNULL,
                               cwd=str(home))
    emulators.append(process)
    leader = int(wait_for(lambda: log.exists() and log.read_text().split('\n')[0].split()[-1:], 5, 'emulator')[0])
    wait_for(lambda: exe(leader) == stub, 5, 'stub')
    tty = os.readlink(f'/proc/{leader}/fd/0')
    found = wait_for(lambda: next((s for s in helper('external_sessions')['result']
                                   if (s.get('origin') or {}).get('tty') == tty), None), 5, 'external row')
    return process, leader, found


def record(name):
    return json.loads((root / f'{name}.json').read_text())


try:
    tmux('start-server', ';', 'set', '-g', 'exit-empty', 'off')
    info = helper('info')['result']
    assert 'adopt-reptyr-v1' in info['capabilities'], info

    # 1. The move is detected although the relay's descriptors are unreadable.
    emulator, leader, found = emulate('move')
    before = stat(leader)
    started = time.time()
    moved = helper('adopt_session', dict(id=found['id'], origin=found['origin'], boot_id=boot))
    elapsed = time.time() - started
    assert moved['error'] is None, moved
    session = moved['result']
    name = session['id']
    assert elapsed < 8, elapsed
    pane = int(tmux('display', '-p', '-t', name, '#{pane_pid}').stdout)
    relay_pid = next(p for p in [pane] + children(pane) if exe(p) == os.path.realpath(relay))
    peek = subprocess.run([*USER, 'ls', f'/proc/{relay_pid}/fd'], capture_output=True)
    assert peek.returncode != 0, 'the relay should be non-dumpable to its user'
    assert 'CapEff:\t0000000000000000' in Path(f'/proc/{relay_pid}/status').read_text()
    checks.append(f'move seen in {elapsed:.2f}s although the user cannot read the relay\'s fds')

    saved = record(name)
    assert saved['pid'] == pane == session['pid'] and saved['started'] == session['started'], (saved, session)
    assert not saved['started'].startswith('ext:'), saved
    assert not [n for n in os.listdir(root) if n.endswith(('.adopt', '.tmp'))], os.listdir(root)
    checks.append('record finalized with the pane identity; no .adopt or .tmp left')

    # The emptied foot-like window ignores SIGHUP: it must be SIGKILLed, before its
    # "user" closes it and it signals the moved terminal's process group.
    assert wait_for(lambda: emulator.poll() is not None, 3, 'holder killed') and emulator.returncode == -9, \
        emulator.returncode
    time.sleep(10)
    assert alive(leader) and stat(leader)['start'] == before['start'], 'the moved agent died'
    churn = (home / 'move.churn').read_text()
    assert 'signal HUP' not in churn and 'signal TERM' not in churn, churn
    assert 'close' not in (home / 'move.emulator').read_text()
    listed = [s for s in helper('sessions')['result'] if s['id'] == name]
    assert listed and listed[0]['pid'] == saved['pid'] and listed[0]['started'] == saved['started'], listed
    checks.append('the frozen single-window holder was SIGKILLed; the moved agent ran on for 10 s')

    stopped = helper('stop_agent_session', dict(id=name, pid=listed[0]['pid'], started=listed[0]['started'],
                                                 boot_id=boot, provider='claude'))
    assert stopped['error'] is None, stopped
    wait_for(lambda: not alive(leader), 5, 'agent exit')
    checks.append('the moved agent stops with its listed identity')

    # 2. A record left stale by the old check (pending identity, empty .adopt) heals
    #    on listing, and the session then stops with its listed identity.
    emulator, leader, found = emulate('heal')
    moved = helper('adopt_session', dict(id=found['id'], origin=found['origin'], boot_id=boot))
    assert moved['error'] is None, moved
    name = moved['result']['id']
    stale = dict(record(name), pid=leader, started='ext:' + found['origin']['leader']['start_ticks'])
    (root / f'{name}.json').write_text(json.dumps(stale))
    (root / f'{name}.adopt').write_text('')
    listed = [s for s in helper('sessions')['result'] if s['id'] == name]
    assert listed, 'adopted session not listed'
    healed = record(name)
    assert healed['pid'] == listed[0]['pid'] and healed['started'] == listed[0]['started'], healed
    assert not (root / f'{name}.adopt').exists()
    assert healed['origin'] == stale['origin'], healed
    stopped = helper('stop_agent_session', dict(id=name, pid=listed[0]['pid'], started=listed[0]['started'],
                                                 boot_id=boot, provider='claude'))
    assert stopped['error'] is None, stopped
    wait_for(lambda: not alive(leader), 5, 'healed agent exit')
    checks.append('a stale adopted record heals on listing and then stops')

    # 3. Stopping an agent in a terminal outside tmux: identity first, then HUP to its
    #    process group, KILL after 3 s (the stub ignores HUP); the window stays.
    assert 'stop-external-v1' in info['capabilities'], info
    emulator, leader, found = emulate('outside', EMULATOR)
    tool = wait_for(lambda: children(leader), 5, 'tool child')
    wrong = json.loads(json.dumps(found['origin']))
    wrong['leader']['start_ticks'] = str(int(wrong['leader']['start_ticks']) + 1)
    refused = helper('stop_external', dict(id=found['id'], origin=wrong, boot_id=boot))
    assert refused['error'] and alive(leader), refused
    refused = helper('stop_external', dict(id=found['id'], origin=found['origin'], boot_id='other-boot'))
    assert refused['error'] and alive(leader), refused
    checks.append('stop_external refuses a changed identity or boot without signalling')
    started = time.time()
    stopped = helper('stop_external', dict(id=found['id'], origin=found['origin'], boot_id=boot))
    assert stopped['error'] is None, stopped
    wait_for(lambda: not alive(leader) and not any(alive(t) for t in tool), 6, 'outside agent exit')
    took = time.time() - started
    assert 'signal HUP' in (home / 'outside.churn').read_text()
    assert 2.5 < took < 6, took
    assert emulator.poll() is None, 'the terminal window is left to its app'
    checks.append(f'outside agent got SIGHUP, then SIGKILL ({took:.1f}s); its window was left alone')

    leftovers = [p for p in map(int, filter(str.isdigit, os.listdir('/proc'))) if exe(p) == stub]
    assert not leftovers, leftovers
    print(json.dumps(dict(result='PASS', checks=checks)))
finally:
    for process in emulators:
        process.kill()
    tmux('kill-server', check=False)
    for pid in map(int, filter(str.isdigit, os.listdir('/proc'))):
        if exe(pid) == stub:
            try:
                os.kill(pid, signal.SIGKILL)
            except OSError:
                pass
    shutil.rmtree(home, ignore_errors=True)
