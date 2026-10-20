#!/usr/bin/env python3
"""Adopt synthetic outside-CX terminals into a private managed tmux; no real agents.

The target is a compiled churn stub that writes to its terminal every 10 ms,
runs worker threads and a piped child tool. Each check observes the processes
directly: PIDs, start times, parents, signals, output gaps and leftovers.

Ptrace needs either an unprivileged user namespace (re-executed automatically)
or CX_ADOPT_CAPABLE=1 with a relay carrying cap_sys_ptrace (CI).
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
scope = Path('/proc/sys/kernel/yama/ptrace_scope')
if (os.geteuid() != 0 and os.environ.get('CX_ADOPT_CAPABLE') != '1'
        and scope.exists() and scope.read_text().strip() != '0'):
    os.execvp('unshare', ['unshare', '-Ur', sys.executable, *sys.argv])

EMULATOR = r'''
import fcntl, os, pty, struct, sys, termios, time
master, slave = pty.openpty()
fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 30, 90, 0, 0))
pid = os.fork()
if pid == 0:
    os.setsid(); fcntl.ioctl(slave, termios.TIOCSCTTY, 0)
    for fd in (0, 1, 2): os.dup2(slave, fd)
    os.execvp(sys.argv[2], sys.argv[2:])
os.close(slave)
log = open(sys.argv[1], 'a', buffering=1)
log.write(f'leader {pid}\n')
total = 0
while not os.path.exists(sys.argv[1] + '.close'):
    try:
        import select
        if select.select([master], [], [], 0.05)[0]:
            data = os.read(master, 65536)
            if not data:
                break
            total += len(data)
            log.write(f'bytes {total}\n')
    except OSError as e:
        log.write(f'master-error {e.errno}\n')
        break
os.close(master)
log.write('closed\n')
while True:
    time.sleep(1)
'''

checks = []


def stat(pid):
    fields = Path(f'/proc/{pid}/stat').read_text().rsplit(')', 1)[1].split()
    return dict(state=fields[0], ppid=int(fields[1]), pgrp=int(fields[2]), sid=int(fields[3]), start=fields[19])


def alive(pid):
    try:
        return stat(pid)['state'] != 'Z'
    except OSError:
        return False


def children(pid):
    found = []
    for task in os.listdir(f'/proc/{pid}/task'):
        try:
            found += [int(c) for c in Path(f'/proc/{pid}/task/{task}/children').read_text().split()]
        except OSError:
            pass
    return found


def descendants(pid):
    seen, todo = [], [pid]
    while todo:
        for child in children(todo.pop()):
            seen.append(child)
            todo.append(child)
    return seen


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


with tempfile.TemporaryDirectory(prefix='cx-adopt-') as temporary:
    home = Path(temporary)
    bins = home / 'bin'
    bins.mkdir()
    root = home / 'state/cx'
    root.mkdir(parents=True, mode=0o700)
    socket = root / 'managed.sock'
    subprocess.run(['cc', '-O1', '-pthread', '-o', str(bins / 'claude'), str(here / 'adopt_churn.c')], check=True)
    relay = os.environ.get('CX_ADOPT_REPTYR')
    if not relay:
        relay = str(home / 'cx-reptyr')
        subprocess.run(['sh', str(here.parent / 'scripts/build-portable-reptyr.sh'), relay],
                       env=dict(os.environ, CX_REPTYR_DYNAMIC='1'), check=True, stdout=subprocess.DEVNULL)
    env = dict(os.environ, HOME=str(home), XDG_STATE_HOME=str(home / 'state'), TMUX_TMPDIR=str(home),
               SHELL='/bin/bash', TERM='xterm-256color', PATH=f'{bins}:{os.environ["PATH"]}',
               CX_ADOPT_REPTYR=relay, PS1='$ ')
    for key in ('TMUX', 'TMUX_PANE', 'DBUS_SESSION_BUS_ADDRESS'):
        env.pop(key, None)
    boot = Path('/proc/sys/kernel/random/boot_id').read_text().strip()
    emulators = []

    def tmux(*args, check=True):
        return subprocess.run(['tmux', '-f', '/dev/null', '-S', str(socket), *args], capture_output=True,
                              text=True, check=check, timeout=5, env=env)

    def helper(op, args=None):
        body = dict(op=op) if args is None else dict(op=op, args=args)
        request = json.dumps(dict(version=1, id='fixture', op=body)).encode()
        wire = b'CX1 ' + str(len(request)).encode() + b'\n' + request
        result = subprocess.run([binary, 'helper'], input=wire, env=env, capture_output=True, timeout=30)
        assert result.returncode == 0, result.stderr.decode()
        header, data = result.stdout.split(b'\n', 1)
        assert header == b'CX1 ' + str(len(data)).encode(), result.stdout
        return json.loads(data)

    def emulate(name, *argv):
        log = home / f'{name}.emulator'
        process = subprocess.Popen([sys.executable, '-c', EMULATOR, str(log), *argv], env=dict(
            env, CHURN_LOG=str(home / f'{name}.churn')), stdin=subprocess.DEVNULL)
        emulators.append(process)
        leader = int(wait_for(lambda: log.exists() and log.read_text().split('\n')[0].split()[-1:], 5,
                              'emulator')[0])
        return process, leader, log

    def target_of(leader):
        claude = os.path.realpath(bins / 'claude')
        return wait_for(lambda: next((p for p in [leader] + descendants(leader) if exe(p) == claude), None),
                        5, 'stub')

    def row(sessions, tty):
        return next((s for s in sessions if (s.get('origin') or {}).get('tty') == tty), None)

    def beats(name):
        lines = (home / f'{name}.churn').read_text().splitlines()
        return lines, [l for l in lines if l.startswith('beat')]

    try:
        # Keep the private managed server alive without a user service.
        tmux('start-server', ';', 'set', '-g', 'exit-empty', 'off')
        info = helper('info')['result']
        assert 'adopt-session-v1' in info['capabilities'] and 'adopt-reptyr-v1' in info['capabilities'], info
        checks.append('helper advertises adoption with a verified relay')

        # 1. Agent as session leader of a bare terminal, mid-output.
        emulator, leader, log = emulate('leader', str(bins / 'claude'))
        target = target_of(leader)
        tool = wait_for(lambda: children(target), 5, 'tool child')
        before = stat(target)
        time.sleep(0.6)
        sessions = helper('sessions')['result']
        tty = os.readlink(f'/proc/{target}/fd/0')
        found = row(sessions, tty)
        assert found and found['external'] and found['id'].startswith('ext-'), sessions
        assert found['provider'] == 'claude' and found['origin']['kind'] == 'terminal', found
        assert found['origin']['holder']['pid'] == emulator.pid, found
        checks.append('bare terminal agent is listed with its terminal holder')

        stale = json.loads(json.dumps(found['origin']))
        stale['leader']['start_ticks'] = str(int(stale['leader']['start_ticks']) + 1)
        refused = helper('adopt_session', dict(id=found['id'], origin=stale, boot_id=boot))
        assert refused['error'] and 'changed' in refused['error'], refused
        assert os.readlink(f'/proc/{target}/fd/0') == tty and alive(target)
        assert not any(n.endswith('.json') for n in os.listdir(root)), os.listdir(root)
        checks.append('stale identity is refused before any change')

        started = time.time()
        adopted = helper('adopt_session', dict(id=found['id'], origin=found['origin'], boot_id=boot))
        assert adopted['error'] is None, adopted
        adopted = adopted['result']
        elapsed = time.time() - started
        assert adopted['id'].startswith('cx-') and not adopted['external'], adopted
        after = stat(target)
        assert (after['start'], after['ppid'], after['pgrp'], after['sid']) == (
            before['start'], before['ppid'], before['pgrp'], before['sid']), (before, after)
        assert all(alive(t) for t in tool) and children(target) == tool
        relay_pid = adopted['pid']
        status = Path(f'/proc/{relay_pid}/status').read_text()
        assert 'CapEff:\t0000000000000000' in status, status
        checks.append(f'agent adopted in {elapsed:.2f}s with PID, parent, group, tool and threads intact')

        name = adopted['id']
        tmux('send-keys', '-t', name, 'ping', 'Enter')
        tmux('resize-window', '-t', name, '-x', '101', '-y', '33')
        wait_for(lambda: 'input 5 ping' in (home / 'leader.churn').read_text(), 5, 'input')
        wait_for(lambda: 'WINCH 101x33' in (home / 'leader.churn').read_text(), 5, 'resize')
        wait_for(lambda: 'tick' in tmux('capture-pane', '-p', '-t', name).stdout, 5, 'output')
        checks.append('pane output, input and resize reach the adopted agent')

        (Path(str(log) + '.close')).touch()
        wait_for(lambda: 'closed' in log.read_text(), 5, 'window close')
        time.sleep(0.5)
        assert alive(target)
        sessions = helper('sessions')['result']
        managed = [s for s in sessions if s['id'] == name]
        assert managed and managed[0]['provider'] == 'claude' and managed[0]['origin'], sessions
        assert not [s for s in sessions if s['id'].startswith('ext-') and s['origin']['tty'] == tty], sessions
        checks.append('closing the original window leaves the agent running; listed once, as managed')

        lines, beat = beats('leader')
        gap = max(int(float(l.split('worst_gap_ms=')[1].split()[0])) for l in beat)
        assert gap < 500, gap
        assert not [l for l in lines if l.startswith(('signal HUP', 'write-error', 'read-error', 'stdin-eof'))], lines
        checks.append(f'no hangup or terminal I/O error; largest output gap {gap} ms')

        stopped = helper('stop_agent_session', dict(id=name, pid=managed[0]['pid'], started=managed[0]['started'],
                                                     boot_id=boot, provider='claude'))
        assert stopped['error'] is None, stopped
        wait_for(lambda: not alive(target) and not any(alive(t) for t in tool), 5, 'agent exit')
        wait_for(lambda: tmux('has-session', '-t', '=' + name, check=False).returncode != 0, 5, 'relay exit')
        assert not alive(relay_pid)
        assert 'signal TERM' in (home / 'leader.churn').read_text()
        checks.append('stop terminates the adopted agent, its tool child and the relay')

        # 2. Agent as a foreground job under an interactive shell.
        emulator, leader, log = emulate('job', 'bash', '--norc', '--noprofile', '-i')
        time.sleep(0.4)
        tty = os.readlink(f'/proc/{leader}/fd/0')
        found = wait_for(lambda: row(helper('sessions')['result'], tty), 5, 'shell row')
        assert found['provider'] == 'shell' and found['activity'] == 'idle', found
        # Adopt the idle shell, then start the agent inside the moved terminal.
        adopted = helper('adopt_session', dict(id=found['id'], origin=found['origin'], boot_id=boot))
        assert adopted['error'] is None, adopted
        name = adopted['result']['id']
        tmux('send-keys', '-t', name, f'{bins / "claude"}', 'Enter')
        target = target_of(leader)
        wait_for(lambda: any(s['id'] == name and s['provider'] == 'claude'
                             for s in helper('sessions')['result']), 5, 'agent detection after move')
        checks.append('an adopted shell keeps job control: an agent started in the pane is detected')
        tmux('send-keys', '-t', name, 'C-c', '')
        wait_for(lambda: not alive(target), 5, 'interrupt')
        tmux('send-keys', '-t', name, 'exit', 'Enter')
        wait_for(lambda: not alive(leader), 5, 'shell exit')
        wait_for(lambda: tmux('has-session', '-t', '=' + name, check=False).returncode != 0, 5, 'relay exit')
        checks.append('exiting the adopted shell ends its relay and managed session')

        # 3. Reset ends adopted terminals instead of stranding them.
        emulator, leader, log = emulate('reset', str(bins / 'claude'))
        target = target_of(leader)
        tty = os.readlink(f'/proc/{target}/fd/0')
        found = wait_for(lambda: row(helper('sessions')['result'], tty), 5, 'reset row')
        adopted = helper('adopt_session', dict(id=found['id'], origin=found['origin'], boot_id=boot))
        assert adopted['error'] is None, adopted
        # Restarting the server afterwards may be refused without a user
        # service; termination of adopted terminals happens before that.
        subprocess.run([binary, 'reset'], env=env, capture_output=True, text=True, timeout=20)
        wait_for(lambda: not alive(target), 5, 'reset termination')
        checks.append('reset terminates adopted agents rather than leaving them on a dead terminal')

        leftovers = [p for p in map(int, filter(str.isdigit, os.listdir('/proc')))
                     if exe(p) in (os.path.realpath(bins / 'claude'), os.path.realpath(relay))]
        assert not leftovers, leftovers
        assert not [n for n in os.listdir(root) if n.endswith(('.adopt', '.tmp'))], os.listdir(root)
        checks.append('no leftover agents, relays or temporary adoption files')
        print(json.dumps(dict(result='PASS', checks=checks)))
    finally:
        for process in emulators:
            process.kill()
        subprocess.run(['tmux', '-f', '/dev/null', '-S', str(socket), 'kill-server'], env=env,
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        claude = os.path.realpath(bins / 'claude')
        for pid in map(int, filter(str.isdigit, os.listdir('/proc'))):
            if exe(pid) == claude:
                os.kill(pid, signal.SIGKILL)
