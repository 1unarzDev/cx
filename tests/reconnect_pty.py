#!/usr/bin/env python3
"""A connection blip does not throw the viewer out of a remote terminal (v0.3.5).

usage: reconnect_pty.py BINARY

Runs itself inside tests/isolate.sh (private user, pid and mount namespaces). A private
sshd on 127.0.0.1 serves a device HOME holding one managed session; the viewer has its
own HOME and an `ssh` wrapper on its PATH that points the remote `cx helper` and
`cx native-attach*` commands at the device HOME's binary. Nothing touches the caller's
HOME, SSH trust, tmux or networking.

Checks, in one viewer attached to the device's session:
- detach: Ctrl+\\ returns to the hub at once, with no reconnect banner;
- master lost: every sshd connection process is killed (the shared master dies with
  the terminal): the banner shows, the viewer reattaches the same session (pane pid
  unchanged) within a few seconds over a new master, and typed input reaches it;
- client lost: only the terminal's ssh client is killed: the same, over the same
  master (its pid unchanged);
- esc: with sshd stopped (connections hang), the banner shows and Esc returns to the
  hub with the session's tile still focused (Enter attaches it again later);
- ended: the session is killed while sshd is stopped; once sshd answers again the
  viewer says the session ended and returns to the hub.
"""
import json, os, pathlib, pwd, shutil, signal, socket, subprocess, sys, tempfile, time

HERE = pathlib.Path(__file__).resolve().parent
if not pathlib.Path('/dev/shm/cx-isolated').exists():
    os.execv(str(HERE / 'isolate.sh'), [str(HERE / 'isolate.sh'), sys.executable, *sys.argv])
sys.path.insert(0, str(HERE))
import pty_lib  # noqa: E402

BINARY = str(pathlib.Path(sys.argv[1]).resolve())
ACCOUNT = pwd.getpwuid(os.getuid()).pw_name
SSH = shutil.which('ssh')
SSHD = shutil.which('sshd') or '/usr/sbin/sshd'
BANNER = 'connection lost · reconnecting'.encode()


def private_env(home):
    env = {k: v for k, v in os.environ.items() if k in ('PATH', 'LANG', 'LC_ALL')}
    env.update(HOME=str(home), USER=ACCOUNT, XDG_STATE_HOME=str(home / 'state'),
               TMUX_TMPDIR=str(home / 'tmux'), XDG_RUNTIME_DIR=str(home / 'run'),
               SHELL='/bin/bash', TERM='xterm-256color', HTTPS_PROXY='http://127.0.0.1:9',
               HISTFILE='/dev/null', CX_THEME='dark', CX_REDUCED_MOTION='1')
    return env


def private(path, text):
    path.write_text(text)
    path.chmod(0o600)


def free_port():
    with socket.socket() as s:
        s.bind(('127.0.0.1', 0))
        return s.getsockname()[1]


def processes():
    """(pid, ppid, argv) of every process in this namespace."""
    found = []
    for entry in pathlib.Path('/proc').iterdir():
        if not entry.name.isdigit():
            continue
        try:
            stat = (entry / 'stat').read_text()
            argv = (entry / 'cmdline').read_bytes().split(b'\0')
        except OSError:
            continue
        ppid = int(stat.rsplit(')', 1)[1].split()[1])
        found.append((int(entry.name), ppid, [a.decode(errors='replace') for a in argv if a]))
    return found


def descendants(root_pid):
    procs = processes()
    out, frontier = [], {root_pid}
    while frontier:
        children = [p for p, pp, _ in procs if pp in frontier]
        out += children
        frontier = set(children)
    return out


def kill(pids):
    for pid in pids:
        try:
            os.kill(pid, signal.SIGKILL)
        except OSError:
            pass


tmp = tempfile.TemporaryDirectory(prefix='cx-rc-', dir='/tmp')
root = pathlib.Path(tmp.name)
root.chmod(0o700)
device = root / 'd'
for sub in ('.local/bin', 'state/cx', 'tmux', 'run', '.ssh', 'work'):
    (device / sub).mkdir(parents=True, mode=0o700)
shutil.copyfile(BINARY, device / '.local/bin/cx')
(device / '.local/bin/cx').chmod(0o755)
denv = private_env(device)
for name in ('host', 'client'):
    subprocess.run(['ssh-keygen', '-q', '-t', 'ed25519', '-N', '', '-f', str(root / name)],
                   check=True, timeout=10)
shutil.copyfile(root / 'client.pub', device / '.ssh/authorized_keys')
port = free_port()
(root / 'sshd.conf').write_text(f'''Port {port}
ListenAddress 127.0.0.1
HostKey {root / 'host'}
PidFile {root}/sshd.pid
AuthorizedKeysFile {device}/.ssh/authorized_keys
StrictModes no
UsePAM no
PasswordAuthentication no
KbdInteractiveAuthentication no
PubkeyAuthentication yes
AllowUsers {ACCOUNT}
''')
remote = ' '.join(f'{k}={denv[k]}' for k in ('HOME', 'XDG_STATE_HOME', 'TMUX_TMPDIR', 'SHELL'))
tools = root / 'tools'
tools.mkdir(mode=0o700)
(tools / 'ssh').write_text(f'''#!{sys.executable}
import os,sys
args=sys.argv[1:]
# /etc/ssh/ssh_config has no mapped owner inside the user namespace: use the viewer's.
if '-F' in args:
 i=args.index('-F');del args[i:i+2]
args=['-F',os.path.expanduser('~/.ssh/config'),*args]
if '-G' not in args and '-O' not in args and '-N' not in args and len(args)>2:
 cmd=args[-1]
 if cmd=='exec ~/.local/bin/cx helper':args[-1]={f'exec env {remote} {device}/.local/bin/cx helper'!r}
 elif cmd.startswith('~/.local/bin/cx native-attach'):args[-1]={f'env {remote} {device}/.local/bin/cx'!r}+cmd[len('~/.local/bin/cx'):]
 elif cmd!='exit 0':args[-1]='true'
os.execv({SSH!r},[{SSH!r},*args])
''')
(tools / 'ssh').chmod(0o700)


def tmux(*args):
    out = subprocess.run(['tmux', '-S', str(device / 'state/cx/managed.sock'), *args], env=denv,
                         capture_output=True, text=True, timeout=10)
    return out.stdout if out.returncode == 0 else ''


def clients():
    return tmux('list-clients', '-F', '#{client_control_mode}').split().count('0')


daemon = subprocess.Popen([SSHD, '-D', '-e', '-f', str(root / 'sshd.conf')],
                          stdout=(root / 'sshd.log').open('wb'), stderr=subprocess.STDOUT)
v = None
results = {}
try:
    for _ in range(100):
        try:
            with socket.create_connection(('127.0.0.1', port), timeout=.1):
                break
        except OSError:
            assert daemon.poll() is None, (root / 'sshd.log').read_text()[-400:]
            time.sleep(.05)
    out = subprocess.run([str(device / '.local/bin/cx'), 'new', '--directory', str(device / 'work'),
                          '--name', 'rc-session', '--key', 'rc-session'],
                         env=denv, capture_output=True, text=True, timeout=30)
    assert out.returncode == 0, out.stderr[-400:]

    home = root / 'v'
    for sub in ('state/cx', 'tmux', 'run', '.ssh'):
        (home / sub).mkdir(parents=True, mode=0o700)
    env = private_env(home)
    env['PATH'] = f'{tools}{os.pathsep}{env["PATH"]}'
    host_key = ' '.join((root / 'host.pub').read_text().split()[:2])
    private(home / '.ssh/known_hosts', f'[127.0.0.1]:{port} {host_key}\n')
    private(home / '.ssh/config', f'''Host 127.0.0.1
  Port {port}
  IdentityFile {root / 'client'}
  IdentitiesOnly yes
  IdentityAgent none
  UserKnownHostsFile {home / '.ssh/known_hosts'}
  GlobalKnownHostsFile /dev/null
''')
    private(home / 'state/cx/devices.json', json.dumps([
        dict(id='local', name='tranquil', target=None, account=ACCOUNT, host='tranquil',
             status='local', observed_at=0),
        dict(id='zephyr-id', name='zephyr', target=f'{ACCOUNT}@127.0.0.1', account=ACCOUNT,
             host='zephyr', status='unknown', observed_at=0)]))
    v = pty_lib.Pty([BINARY], env, home, 160, 45)
    raw = bytearray()

    def pump(t):
        data = v.read(t)
        if data:
            raw.extend(data)

    def settle(until, what, limit):
        end = time.monotonic() + limit
        while not until():
            assert time.monotonic() < end, f'{what}:\n{v.text()}'
            assert v.proc.poll() is None, f'viewer exited {v.proc.returncode}:\n{v.text()}'
            pump(.05)
        pump(.1)

    def attach():
        v.write(b'\r')
        settle(lambda: clients() == 1, 'Enter did not attach the session', 15)
        settle(lambda: 'detach' in v.screen.display[-1], 'the attached client drew nothing', 10)

    def session_pane():
        """(pane id, pane pid) of the session (not the hidden control session)."""
        rows = [r.split() for r in tmux('list-panes', '-a', '-F',
                                        '#{session_name} #{pane_id} #{pane_pid}').splitlines()]
        [(pane, pid)] = [(r[1], r[2]) for r in rows if r[0].startswith('cx-')]
        return pane, pid

    def pane_pid():
        return session_pane()[1]

    def masters():
        return sorted(p for p, _, a in processes() if a and a[0].endswith('ssh') and '-N' in a)

    def typed_input_works(mark):
        v.write(f'echo {mark}-$((40+2))\r'.encode())
        end = time.monotonic() + 10
        while f'{mark}-42' not in tmux('capture-pane', '-p', '-t', pid_target):
            assert time.monotonic() < end, 'typed input never reached the session'
            pump(.05)

    def drop_and_reattach(case, cut):
        del raw[:]
        before, started = pane_pid(), time.monotonic()
        cut()
        settle(lambda: BANNER in raw, f'{case}: no reconnect banner', 10)
        settle(lambda: clients() == 1 and 'detach' in v.screen.display[-1],
               f'{case}: never reattached', 15)
        took = round(time.monotonic() - started, 2)
        assert pane_pid() == before, f'{case}: a different pane'
        typed_input_works(case)
        return took

    settle(lambda: 'rc-session' in v.text(), 'the hub never listed the remote session', 25)
    v.write(b'\t')
    settle(lambda: '▌SESSIONS' in v.text(), 'Tab did not focus the tiles', 10)
    attach()
    pid_target = session_pane()[0]
    pid_before = pane_pid()

    # Deliberate detach: back to the hub at once, no banner.
    del raw[:]
    started = time.monotonic()
    v.write(b'\x1c')
    settle(lambda: clients() == 0, 'Ctrl+\\ did not detach', 5)
    v.screen.reset()
    settle(lambda: 'Returned from' in v.text(), 'detach did not return to the hub', 5)
    assert BANNER not in raw, 'a deliberate detach showed the reconnect banner'
    results['detach_s'] = round(time.monotonic() - started, 2)

    # The connection dies with its master: every sshd connection process is killed.
    attach()
    results['master_lost_reattach_s'] = drop_and_reattach(
        'master', lambda: kill(descendants(daemon.pid)))
    # Only the terminal's ssh client dies: the master is reused.
    warm = masters()
    assert warm, 'the master was not rebuilt'
    attach_client = [p for p, _, a in processes()
                     if a and a[0].endswith('ssh') and any('native-attach' in x for x in a)]
    assert attach_client, 'no attach ssh client'
    results['client_lost_reattach_s'] = drop_and_reattach('client', lambda: kill(attach_client))
    assert masters() == warm, 'the warm master was not reused'
    assert pane_pid() == pid_before

    # Esc while sshd hangs: back to the hub, the tile still focused.
    del raw[:]
    daemon.send_signal(signal.SIGSTOP)
    kill(descendants(daemon.pid))
    settle(lambda: BANNER in raw, 'esc: no reconnect banner', 10)
    pump(.5)
    started = time.monotonic()
    v.write(b'\x1b')
    v.screen.reset()
    settle(lambda: 'connection lost · the session keeps' in v.text(),
           'Esc did not return to the hub', 5)
    results['esc_s'] = round(time.monotonic() - started, 2)
    assert clients() == 0
    daemon.send_signal(signal.SIGCONT)
    settle(lambda: '▌SESSIONS' in v.text(), 'the tiles lost focus after Esc', 5)
    attach()
    assert pane_pid() == pid_before

    # The session ends while the connection is down: said so, back to the hub.
    del raw[:]
    daemon.send_signal(signal.SIGSTOP)
    kill(descendants(daemon.pid))
    settle(lambda: BANNER in raw, 'ended: no reconnect banner', 10)
    tmux('kill-server')
    daemon.send_signal(signal.SIGCONT)
    v.screen.reset()
    settle(lambda: 'Session ended on' in v.text() and 'while disconnected' in v.text(),
           'the ended session was not reported', 30)
    results['ended'] = 'reported'
    results['pane_pid_unchanged'] = True
finally:
    if v:
        v.close()
    tmux('kill-server')
    try:
        daemon.send_signal(signal.SIGCONT)
        daemon.terminate()
        daemon.wait(timeout=5)
    except Exception:
        daemon.kill()
    tmp.cleanup()
print(json.dumps(dict(result='PASS', **results)))
