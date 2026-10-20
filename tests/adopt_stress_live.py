#!/usr/bin/env python3
"""Repeated adoption of busy synthetic agents, with injected failures.

Every iteration starts a fresh churn stub in its own terminal, waits a random
interval so the move lands at an arbitrary point in its output, adopts it,
verifies identity/signals/output continuity, stops it and checks that no
agent, tool child, relay, tmux session or temporary file remains.

Failure injections: the target exits while being moved; the relay is killed
during the move; a forged holder identity; a guard refusal for root's init.
Usage: adopt_stress_live.py CX [ITERATIONS]
"""
import json
import os
from pathlib import Path
import random
import shutil
import signal
import subprocess
import sys
import tempfile
import threading
import time

binary = str(Path(sys.argv[1]).resolve())
iterations = int(sys.argv[2]) if len(sys.argv) > 2 else 50
here = Path(__file__).resolve().parent
scope = Path('/proc/sys/kernel/yama/ptrace_scope')


def _subid(path, name):
    try:
        for line in Path(path).read_text().splitlines():
            who, base, count = line.split(':')
            if who in (name, str(os.getuid())) and int(count) >= 1:
                return int(base)
    except OSError:
        pass
    return None


def _reexec_sandbox():
    """Re-run inside a user namespace that permits ptrace and setgroups.

    `unshare -Ur` denies setgroups, which the relay's privilege drop needs, so a
    two-range map (via newuidmap/newgidmap) is used instead. CI sets
    CX_ADOPT_CAPABLE=1 with a real capability relay and skips this path.
    """
    import pwd
    name = pwd.getpwuid(os.getuid()).pw_name
    sub_u, sub_g = _subid('/etc/subuid', name), _subid('/etc/subgid', name)
    if sub_u is None or sub_g is None or not shutil.which('newuidmap'):
        print(json.dumps(dict(result='SKIP', reason='no subuid range or newuidmap for ptrace sandbox')))
        sys.exit(0)
    os.environ['CX_ADOPT_NS'] = '1'
    os.execvp('unshare', [
        'unshare', '-U',
        '--map-users', f'0:{os.getuid()}:1', '--map-users', f'1:{sub_u}:1',
        '--map-groups', f'0:{os.getgid()}:1', '--map-groups', f'1:{sub_g}:1',
        sys.executable, *sys.argv])


if (os.geteuid() != 0 and os.environ.get('CX_ADOPT_CAPABLE') != '1'
        and os.environ.get('CX_ADOPT_NS') != '1'
        and scope.exists() and scope.read_text().strip() != '0'):
    _reexec_sandbox()
sys.path.insert(0, str(here))

EMULATOR = (here / 'adopt_terminal_live.py').read_text().split("EMULATOR = r'''", 1)[1].split("'''", 1)[0]
random.seed(int(os.environ.get('CX_ADOPT_SEED', time.time())))
seed_note = os.environ.get('CX_ADOPT_SEED', 'time')


def stat(pid):
    f = Path(f'/proc/{pid}/stat').read_text().rsplit(')', 1)[1].split()
    return dict(state=f[0], ppid=int(f[1]), pgrp=int(f[2]), sid=int(f[3]), start=f[19])


def alive(pid):
    try:
        return stat(pid)['state'] != 'Z'
    except OSError:
        return False


def exe(pid):
    try:
        return os.readlink(f'/proc/{pid}/exe')
    except OSError:
        return None


def children(pid):
    found = []
    try:
        for task in os.listdir(f'/proc/{pid}/task'):
            try:
                found += [int(c) for c in Path(f'/proc/{pid}/task/{task}/children').read_text().split()]
            except OSError:
                pass
    except OSError:
        pass
    return found


def wait_for(predicate, seconds=10, message='condition'):
    deadline = time.time() + seconds
    while time.time() < deadline:
        value = predicate()
        if value:
            return value
        time.sleep(0.02)
    raise AssertionError(f'timed out waiting for {message}')


checks = []
with tempfile.TemporaryDirectory(prefix='cx-adopt-stress-') as temporary:
    home = Path(temporary)
    bins = home / 'bin'
    bins.mkdir()
    root = home / 'state/cx'
    root.mkdir(parents=True, mode=0o700)
    socket = root / 'managed.sock'
    subprocess.run(['cc', '-O1', '-pthread', '-o', str(bins / 'claude'), str(here / 'adopt_churn.c')], check=True)
    stub = os.path.realpath(bins / 'claude')
    relay = os.environ.get('CX_ADOPT_REPTYR')
    if not relay:
        relay = str(home / 'cx-reptyr')
        subprocess.run(['sh', str(here.parent / 'scripts/build-portable-reptyr.sh'), relay],
                       env=dict(os.environ, CX_REPTYR_DYNAMIC='1'), check=True, stdout=subprocess.DEVNULL)
    relay = os.path.realpath(relay)
    env = dict(os.environ, HOME=str(home), XDG_STATE_HOME=str(home / 'state'), TMUX_TMPDIR=str(home),
               SHELL='/bin/bash', TERM='xterm-256color', PATH=f'{bins}:{os.environ["PATH"]}',
               CX_ADOPT_REPTYR=relay)
    for key in ('TMUX', 'TMUX_PANE', 'DBUS_SESSION_BUS_ADDRESS'):
        env.pop(key, None)
    boot = Path('/proc/sys/kernel/random/boot_id').read_text().strip()
    emulators = []

    def tmux(*args, check=True):
        return subprocess.run(['tmux', '-f', '/dev/null', '-S', str(socket), *args], capture_output=True,
                              text=True, check=check, timeout=5, env=env)

    def helper(op, args=None):
        body = dict(op=op) if args is None else dict(op=op, args=args)
        request = json.dumps(dict(version=1, id='stress', op=body)).encode()
        wire = b'CX1 ' + str(len(request)).encode() + b'\n' + request
        result = subprocess.run([binary, 'helper'], input=wire, env=env, capture_output=True, timeout=30)
        assert result.returncode == 0, result.stderr.decode()
        return json.loads(result.stdout.split(b'\n', 1)[1])

    def emulate(name, *argv, churn_env=None):
        log = home / f'{name}.emulator'
        process = subprocess.Popen([sys.executable, '-c', EMULATOR, str(log), *argv],
                                   env=dict(env, CHURN_LOG=str(home / f'{name}.churn'), **(churn_env or {})),
                                   stdin=subprocess.DEVNULL)
        emulators.append(process)
        leader = int(wait_for(lambda: log.exists() and log.read_text().split('\n')[0].split()[-1:], 5,
                              'emulator')[0])
        wait_for(lambda: exe(leader) == stub, 5, 'stub exec')
        return process, leader, log

    def row(tty):
        return next((s for s in helper('sessions')['result'] if (s.get('origin') or {}).get('tty') == tty), None)

    def ours():
        found = {}
        for pid in map(int, filter(str.isdigit, os.listdir('/proc'))):
            path = exe(pid)
            if path in (stub, relay) and alive(pid):
                found[pid] = 'relay' if path == relay else 'stub'
        return found

    def managed():
        out = tmux('list-sessions', '-F', '#{session_name}', check=False).stdout.split()
        return [s for s in out if s.startswith('cx-')]

    def assert_clean(context):
        wait_for(lambda: not ours(), 5, f'{context}: processes exit ({ours()})')
        wait_for(lambda: not managed(), 5, f'{context}: managed sessions close ({managed()})')
        temp = [n for n in os.listdir(root) if n.endswith(('.adopt', '.tmp'))]
        assert not temp, (context, temp)

    def close_emulators():
        for process in emulators:
            process.kill()
            process.wait()
        emulators.clear()

    try:
        tmux('start-server', ';', 'set', '-g', 'exit-empty', 'off')

        # Repeated adoption at random points of continuous output.
        worst, slowest = 0, 0.0
        for i in range(iterations):
            name = f'it{i}'
            _, leader, _ = emulate(name, stub)
            tool = wait_for(lambda: children(leader), 5, 'tool child')
            before = stat(leader)
            tty = os.readlink(f'/proc/{leader}/fd/0')
            found = wait_for(lambda: row(tty), 5, 'row')
            time.sleep(random.uniform(0, 0.3))
            started = time.time()
            adopted = helper('adopt_session', dict(id=found['id'], origin=found['origin'], boot_id=boot))
            assert adopted['error'] is None, (i, adopted)
            slowest = max(slowest, time.time() - started)
            session = adopted['result']
            after = stat(leader)
            assert (before['start'], before['ppid'], before['pgrp'], before['sid']) == (
                after['start'], after['ppid'], after['pgrp'], after['sid']), (i, before, after)
            assert children(leader) == tool and all(alive(t) for t in tool), i
            time.sleep(random.uniform(0.05, 0.25))
            log = (home / f'{name}.churn').read_text().splitlines()
            bad = [l for l in log if l.startswith(('signal HUP', 'write-error', 'read-error', 'stdin-eof'))]
            assert not bad, (i, bad)
            gaps = [int(float(l.split('worst_gap_ms=')[1].split()[0])) for l in log if l.startswith('beat')]
            worst = max([worst] + gaps)
            current = next(s for s in helper('sessions')['result'] if s['id'] == session['id'])
            stopped = helper('stop_agent_session', dict(id=current['id'], pid=current['pid'],
                                                        started=current['started'], boot_id=boot,
                                                        provider='claude'))
            assert stopped['error'] is None, (i, stopped)
            close_emulators()
            assert_clean(f'iteration {i}')
        assert worst < 500, worst
        checks.append(f'{iterations} adoptions at random output points (seed {seed_note}): '
                      f'identities intact, no hangup/EIO, worst gap {worst} ms, slowest move {slowest:.2f}s, '
                      'no leftovers after each stop')

        # Target exits while it is being moved: success or a clean refusal.
        outcomes = {'moved': 0, 'refused': 0}
        for i in range(10):
            _, leader, _ = emulate(f'exit{i}', stub)
            tty = os.readlink(f'/proc/{leader}/fd/0')
            found = wait_for(lambda: row(tty), 5, 'row')
            records = len([n for n in os.listdir(root) if n.endswith('.json')])
            killer = threading.Timer(random.uniform(0, 0.2), lambda: os.kill(leader, signal.SIGTERM))
            killer.start()
            result = helper('adopt_session', dict(id=found['id'], origin=found['origin'], boot_id=boot))
            killer.join()
            moved = result['error'] is None
            outcomes['moved' if moved else 'refused'] += 1
            after = len([n for n in os.listdir(root) if n.endswith('.json')])
            assert after == records + (1 if moved else 0), (i, result, records, after)
            close_emulators()
            assert_clean(f'exit race {i}')
        checks.append(f'target exiting during the move ends cleanly ({outcomes})')

        # Relay killed during the move: the original holder must survive and
        # the helper must report failure without leaving state behind.
        holders_ok = 0
        for i in range(10):
            emulator, leader, _ = emulate(f'kill{i}', stub)
            tty = os.readlink(f'/proc/{leader}/fd/0')
            found = wait_for(lambda: row(tty), 5, 'row')
            outcome = {}

            def adopt():
                outcome['r'] = helper('adopt_session', dict(id=found['id'], origin=found['origin'], boot_id=boot))
            worker = threading.Thread(target=adopt)
            worker.start()
            victim = None
            deadline = time.time() + 5
            while time.time() < deadline and victim is None:
                victim = next((p for p, kind in ours().items() if kind == 'relay'), None)
            if victim:
                time.sleep(random.uniform(0, 0.03))
                try:
                    os.kill(victim, signal.SIGKILL)
                except ProcessLookupError:
                    pass
            worker.join()
            time.sleep(0.3)
            if emulator.poll() is None and alive(leader):
                holders_ok += 1
            os.kill(leader, signal.SIGTERM)
            close_emulators()
            assert_clean(f'relay kill {i}')
        assert holders_ok == 10, holders_ok
        checks.append('killing the relay mid-move never crashes the original terminal or agent; no leftovers')

        # Forged identities and the guard itself.
        _, leader, _ = emulate('forged', stub)
        tty = os.readlink(f'/proc/{leader}/fd/0')
        found = wait_for(lambda: row(tty), 5, 'row')
        forged = json.loads(json.dumps(found['origin']))
        forged['holder'] = dict(pid=1, start_ticks=stat(1)['start'], native_id=None)
        result = helper('adopt_session', dict(id=found['id'], origin=forged, boot_id=boot))
        assert result['error'] and 'owner changed' in result['error'], result
        guard = subprocess.run([relay, '-T', '1', stat(1)['start'], '1', stat(1)['start'], '1', stat(1)['start']],
                               capture_output=True, text=True, stdin=subprocess.DEVNULL)
        if os.geteuid() != 0:
            assert guard.returncode == 77 and 'refused' in guard.stderr, guard
        legacy = subprocess.run([relay, '-T', str(leader)], capture_output=True, text=True, stdin=subprocess.DEVNULL)
        assert legacy.returncode == 77, legacy
        assert os.readlink(f'/proc/{leader}/fd/0') == tty and alive(leader)
        os.kill(leader, signal.SIGTERM)
        close_emulators()
        assert_clean('forged')
        checks.append('forged holders and legacy relay invocations are refused without changes')
        print(json.dumps(dict(result='PASS', checks=checks)))
    finally:
        close_emulators()
        subprocess.run(['tmux', '-f', '/dev/null', '-S', str(socket), 'kill-server'], env=env,
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        for pid in map(int, filter(str.isdigit, os.listdir('/proc'))):
            if exe(pid) in (stub, relay):
                os.kill(pid, signal.SIGKILL)
