#!/usr/bin/env python3
"""Move an outside terminal using the sudo + askpass path, with no real sudo.

A fake `sudo` on PATH fetches the password through CX's own askpass client
(the same bridge SSH uses), checks it, sets SUDO_UID/GID and execs the relay.
This fixture is the askpass server: it answers the password prompt the way the
CX password panel would. It proves the move works when CX has no persistent
capability and must ask for a password per move.

Runs under the same setgroups-allowing user namespace as adopt_terminal_live.
"""
import json
import os
from pathlib import Path
import shutil
import signal
import socket
import struct
import subprocess
import sys
import tempfile
import threading
import time

binary = str(Path(sys.argv[1]).resolve())
here = Path(__file__).resolve().parent
helper_src = (here / 'adopt_terminal_live.py').read_text()

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
    import pwd
    name = pwd.getpwuid(os.getuid()).pw_name
    sub_u, sub_g = _subid('/etc/subuid', name), _subid('/etc/subgid', name)
    if sub_u is None or sub_g is None or not shutil.which('newuidmap'):
        print(json.dumps(dict(result='SKIP', reason='no subuid range for ptrace sandbox')))
        sys.exit(0)
    os.environ['CX_ADOPT_NS'] = '1'
    os.execvp('unshare', [
        'unshare', '-U',
        '--map-users', f'0:{os.getuid()}:1', '--map-users', f'1:{sub_u}:1',
        '--map-groups', f'0:{os.getgid()}:1', '--map-groups', f'1:{sub_g}:1',
        sys.executable, *sys.argv])


if (os.geteuid() != 0 and os.environ.get('CX_ADOPT_NS') != '1'
        and scope.exists() and scope.read_text().strip() != '0'):
    _reexec_sandbox()

PASSWORD = 'correct horse battery staple'
EMULATOR = helper_src.split("EMULATOR = r'''", 1)[1].split("'''", 1)[0]


def stat(pid):
    f = Path(f'/proc/{pid}/stat').read_text().rsplit(')', 1)[1].split()
    return dict(ppid=int(f[1]), sid=int(f[3]), start=f[19])


def exe(pid):
    try:
        return os.readlink(f'/proc/{pid}/exe')
    except OSError:
        return None


def alive(pid):
    try:
        return Path(f'/proc/{pid}/stat').read_text().rsplit(')', 1)[1].split()[0] != 'Z'
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


def descendants(pid):
    seen, todo = [], [pid]
    while todo:
        for c in children(todo.pop()):
            seen.append(c); todo.append(c)
    return seen


def wait_for(predicate, seconds=10, message='condition'):
    deadline = time.time() + seconds
    while time.time() < deadline:
        value = predicate()
        if value:
            return value
        time.sleep(0.03)
    raise AssertionError(f'timed out waiting for {message}')


checks = []
prompts = []
with tempfile.TemporaryDirectory(prefix='cx-adopt-sudo-') as temporary:
    home = Path(temporary)
    os.chmod(home, 0o777)
    bins = home / 'bin'
    bins.mkdir()
    root = home / 'state/cx'
    root.mkdir(parents=True, mode=0o700)
    managed = root / 'managed.sock'
    subprocess.run(['cc', '-O1', '-pthread', '-o', str(bins / 'claude'), str(here / 'adopt_churn.c')], check=True)
    stub = os.path.realpath(bins / 'claude')
    relay = os.environ.get('CX_ADOPT_REPTYR')
    if not relay:
        relay = str(home / 'cx-reptyr')
        subprocess.run(['sh', str(here.parent / 'scripts/build-portable-reptyr.sh'), relay],
                       env=dict(os.environ, CX_REPTYR_DYNAMIC='1'), check=True, stdout=subprocess.DEVNULL)

    # Fake sudo: authenticate through SUDO_ASKPASS, then run as "root" with
    # SUDO_UID set, exactly as real sudo would hand off to cx-reptyr.
    fake_sudo = bins / 'sudo'
    fake_sudo.write_text(
        '#!/bin/sh\n'
        'while [ "$1" = "-A" ] || [ "$1" = "-p" ]; do\n'
        '  [ "$1" = "-p" ] && shift\n'
        '  shift\n'
        'done\n'
        'pw=$("$SUDO_ASKPASS" "sudo password to move this terminal into CX: ")\n'
        f'[ "$pw" = "{PASSWORD}" ] || {{ echo "sudo: wrong password" >&2; exit 1; }}\n'
        'exec env SUDO_UID="$(id -u)" SUDO_GID="$(id -g)" "$@"\n')
    fake_sudo.chmod(0o755)

    # Askpass server: stand in for the CX password panel.
    askpass_sock = root / 'askpass'
    nonce = 'fixturenonce'
    server = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    server.bind(str(askpass_sock))
    os.chmod(askpass_sock, 0o600)
    server.listen(4)
    server.settimeout(15)

    def frame_read(conn):
        length = struct.unpack('>I', conn.recv(4))[0]
        data = b''
        while len(data) < length:
            data += conn.recv(length - len(data))
        return data

    def frame_write(conn, payload):
        conn.sendall(struct.pack('>I', len(payload)) + payload)

    def serve():
        while True:
            try:
                conn, _ = server.accept()
            except OSError:
                return
            try:
                got_nonce = frame_read(conn).decode()
                prompt = frame_read(conn).decode()
                prompts.append((got_nonce == nonce, prompt))
                frame_write(conn, b'\x01' + PASSWORD.encode())
            except OSError:
                pass
            finally:
                conn.close()

    threading.Thread(target=serve, daemon=True).start()

    env = dict(os.environ, HOME=str(home), XDG_STATE_HOME=str(home / 'state'), TMUX_TMPDIR=str(home),
               SHELL='/bin/bash', TERM='xterm-256color', PATH=f'{bins}:{os.environ["PATH"]}',
               CX_ASKPASS_SOCKET=str(askpass_sock), CX_ASKPASS_NONCE=nonce)
    for key in ('TMUX', 'TMUX_PANE', 'DBUS_SESSION_BUS_ADDRESS', 'CX_ADOPT_REPTYR'):
        env.pop(key, None)
    # cx-reptyr lives where the sudo path expects it: ~/.local/bin/cx-reptyr.
    (home / '.local/bin').mkdir(parents=True)
    shutil.copyfile(relay, home / '.local/bin/cx-reptyr')
    (home / '.local/bin/cx-reptyr').chmod(0o755)
    boot = Path('/proc/sys/kernel/random/boot_id').read_text().strip()
    emulator = None

    def tmux(*args, check=True):
        return subprocess.run(['tmux', '-f', '/dev/null', '-S', str(managed), *args], capture_output=True,
                              text=True, check=check, timeout=5, env=env)

    def helper(op, args=None):
        body = dict(op=op) if args is None else dict(op=op, args=args)
        request = json.dumps(dict(version=1, id='sudo', op=body)).encode()
        wire = b'CX1 ' + str(len(request)).encode() + b'\n' + request
        result = subprocess.run([binary, 'helper'], input=wire, env=env, capture_output=True, timeout=30)
        assert result.returncode == 0, result.stderr.decode()
        return json.loads(result.stdout.split(b'\n', 1)[1])

    try:
        tmux('start-server', ';', 'set', '-g', 'exit-empty', 'off')
        info = helper('info')['result']
        assert 'adopt-reptyr-v1' in info['capabilities'], info
        assert info['adoption_needs_password'] is True, info
        checks.append('without a capability install, the helper reports adoption needs a password')

        log = home / 'emu'
        emulator = subprocess.Popen([sys.executable, '-c', EMULATOR, str(log), stub],
                                    env=dict(env, CHURN_LOG=str(home / 'churn')), stdin=subprocess.DEVNULL)
        leader = int(wait_for(lambda: log.exists() and log.read_text().split('\n')[0].split()[-1:], 5, 'emu')[0])
        wait_for(lambda: exe(leader) == stub, 5, 'stub')
        before = stat(leader)
        tool = wait_for(lambda: children(leader), 5, 'tool')
        tty = os.readlink(f'/proc/{leader}/fd/0')
        found = wait_for(lambda: next((s for s in helper('external_sessions')['result']
                                       if (s.get('origin') or {}).get('tty') == tty), None), 5, 'row')
        time.sleep(0.5)
        moved = helper('adopt_session', dict(id=found['id'], origin=found['origin'], boot_id=boot))
        assert moved['error'] is None, moved
        session = moved['result']
        assert any(ok and 'sudo password' in text for ok, text in prompts), prompts
        checks.append('the move asked for a password through the askpass bridge with the right nonce')

        after = stat(leader)
        # ppid changes when the emptied source window closes; the process
        # itself (start time, session) and its tool child are unchanged.
        assert (before['start'], before['sid']) == (after['start'], after['sid'])
        assert all(alive(t) for t in tool)
        pane = int(tmux('display', '-p', '-t', session['id'], '#{pane_pid}').stdout)
        want = os.path.realpath(home / '.local/bin/cx-reptyr')
        relay_pid = next(p for p in [pane] + descendants(pane) if exe(p) == want)
        capeff = [l.split()[1] for l in Path(f'/proc/{relay_pid}/status').read_text().splitlines()
                  if l.startswith('CapEff')][0]
        assert capeff == '0000000000000000', capeff
        assert int([l.split()[1] for l in Path(f'/proc/{relay_pid}/status').read_text().splitlines()
                    if l.startswith('Uid:')][0]) == os.getuid()
        checks.append('the relay dropped to the caller with no capabilities; agent identity intact')

        tmux('send-keys', '-t', session['id'], 'ping', 'Enter')
        wait_for(lambda: 'input 5 ping' in (home / 'churn').read_text(), 5, 'input')
        wait_for(lambda: 'tick' in tmux('capture-pane', '-p', '-t', session['id']).stdout, 5, 'output')
        checks.append('input and output reach the moved agent in its new pane')

        listed = helper('sessions')['result']
        assert any(s['id'] == session['id'] and not s['external'] for s in listed), listed
        external = helper('external_sessions')['result']
        assert not [s for s in external if (s.get('origin') or {}).get('tty') == tty], external
        checks.append('after the move the terminal is listed once, as managed, with no stale ext row')

        # The single-window holder (our emulator) is now empty and gets closed.
        wait_for(lambda: not alive(emulator.pid), 5, 'source window closes')
        checks.append('the emptied single-window terminal is closed after the move')

        # A rejected password leaves the original terminal untouched.
        emulator2 = subprocess.Popen([sys.executable, '-c', EMULATOR, str(home / 'emu2'), stub],
                                     env=dict(env, CHURN_LOG=str(home / 'churn2')), stdin=subprocess.DEVNULL)
        leader2 = int(wait_for(lambda: (home / 'emu2').exists() and (home / 'emu2').read_text().split('\n')[0].split()[-1:], 5, 'emu2')[0])
        wait_for(lambda: exe(leader2) == stub, 5, 'stub2')
        tty2 = os.readlink(f'/proc/{leader2}/fd/0')
        found2 = wait_for(lambda: next((s for s in helper('external_sessions')['result']
                                        if (s.get('origin') or {}).get('tty') == tty2), None), 5, 'row2')
        global PASSWORD_BAD
        # Point the fake sudo at a wrong expected password by flipping the stored one.
        fake_sudo.write_text(fake_sudo.read_text().replace(PASSWORD, PASSWORD + '-wrong'))
        rejected = helper('adopt_session', dict(id=found2['id'], origin=found2['origin'], boot_id=boot))
        assert rejected['error'] and alive(leader2) and os.readlink(f'/proc/{leader2}/fd/0') == tty2, rejected
        assert not any(n.endswith('.json') and json.loads((root / n).read_text()).get('origin', {}).get('tty') == tty2
                       for n in os.listdir(root)), os.listdir(root)
        emulator2.kill()
        checks.append('a wrong password fails the move and leaves the original terminal intact')
        print(json.dumps(dict(result='PASS', checks=checks)))
    finally:
        if emulator:
            emulator.kill()
        server.close()
        subprocess.run(['tmux', '-f', '/dev/null', '-S', str(managed), 'kill-server'], env=env,
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        for pid in map(int, filter(str.isdigit, os.listdir('/proc'))):
            if exe(pid) == stub:
                try:
                    os.kill(pid, signal.SIGKILL)
                except OSError:
                    pass
