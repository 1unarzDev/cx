#!/usr/bin/env python3
"""A viewer adopts an outside-tmux agent terminal that lives on a remote device, over a loopback sshd.

usage: adopt_remote_pty.py BINARY        (a debug build: the device's relay is CX_ADOPT_REPTYR)

Runs itself in the adoption fixtures' sandbox (private user namespace with root, a second user
and `nobody` mapped through newuidmap, plus private pid and mount namespaces): nothing here sees
a real process or terminal. tests/isolate.sh is not enough here: it denies setgroups, which the
relay needs to drop to the terminal's owner. As that namespace's root the device's relay may
ptrace, as cap_sys_ptrace lets it on a real device. The "remote" device is a HOME with its own
managed tmux server and a fake agent terminal (a churn stub named claude on a pty held by a small
emulator, outside any tmux); a private sshd on 127.0.0.1 serves it, and the viewer's `ssh`
wrapper turns the remote `cx helper` and `cx native-attach` commands into the device HOME's. The
viewer has its own HOME, and no askpass reaches the device, as for any remote device.
CX_ADOPT_REPTYR names the fixture relay (default: build it with scripts/build-portable-reptyr.sh).

Checks: the device view shows the terminal as `outside cx · ⏎ adopt`; Enter moves it into the
device's managed tmux (same agent PID, start time, session and tool child; the relay holds it;
the emptied emulator closes) and the viewer attaches to it on its own (the agent's output is on
screen) with no password prompt, in under 5 s; after a detach the device's tile is the managed
session with its live screen. Prints one JSON line with the time from Enter to the attached
screen.
"""
import json, os, pathlib, pwd, shutil, signal, socket, subprocess, sys, tempfile, time

HERE = pathlib.Path(__file__).resolve().parent


def _subid(path, name):
    for line in pathlib.Path(path).read_text().splitlines():
        who, base, count = line.split(':')
        if who in (name, str(os.getuid())) and int(count) >= 2:
            return int(base)
    return None


if os.environ.get('CX_ADOPT_NS') != '1':
    # tests/isolate.sh denies setgroups, which the relay needs to drop to its owner; this
    # sandbox is the adoption fixtures' (newuidmap: root, a second user and `nobody`, the
    # sshd's privilege separation user) with its own pid and mount namespaces as well.
    name = pwd.getpwuid(os.getuid()).pw_name
    sub_u, sub_g = _subid('/etc/subuid', name), _subid('/etc/subgid', name)
    if sub_u is None or sub_g is None or not shutil.which('newuidmap'):
        print(json.dumps(dict(result='SKIP', reason='no subuid range for the ptrace sandbox')))
        sys.exit(0)
    maps = ['--map-users', f'0:{os.getuid()}:1', '--map-users', f'1:{sub_u}:1',
            '--map-users', f'65534:{sub_u + 1}:1', '--map-groups', f'0:{os.getgid()}:1',
            '--map-groups', f'1:{sub_g}:1', '--map-groups', f'65534:{sub_g + 1}:1']
    os.environ.update(CX_ADOPT_NS='1', CX_ADOPT_ACCOUNT=name)
    os.execvp('unshare', ['unshare', '-U', *maps, '--pid', '--fork', '--kill-child', '--mount',
                          '--mount-proc', sys.executable, *sys.argv])
# Inside: root of the sandbox. Debug builds discover terminals only under the marker; the
# tty group (unmapped) is left out so sshd's `ssh -t` ptys need no chown to it; sshd's
# privilege separation wants a root-owned /var/empty; the caller's user bus stays hidden.
for script in ('mount -t tmpfs -o mode=1777 tmpfs /dev/shm && : >/dev/shm/cx-isolated',
               'grep -v "^tty:" /etc/group >/dev/shm/group && mount --bind /dev/shm/group /etc/group',
               # (The host's root account is locked; this sandbox's root logs in by key.)
               'printf "root:*:19000::::::\\n" >/dev/shm/shadow && mount --bind /dev/shm/shadow /etc/shadow',
               'mount -t tmpfs -o mode=755 tmpfs /usr/share/empty.sshd 2>/dev/null; mount -t tmpfs -o mode=755 tmpfs /var/empty',
               'for d in /run/user/*; do mount -t tmpfs -o mode=700 tmpfs "$d"; done; true'):
    subprocess.run(['sh', '-c', script], check=True)
sys.path.insert(0, str(HERE))
import pty_lib  # noqa: E402

BINARY = str(pathlib.Path(sys.argv[1]).resolve())
ACCOUNT = 'root'
SSH = shutil.which('ssh')
SSHD = shutil.which('sshd') or '/usr/sbin/sshd'
EMULATOR = (HERE / 'adopt_terminal_live.py').read_text().split("EMULATOR = r'''", 1)[1].split("'''", 1)[0]


def private_env(home):
    env = {k: v for k, v in os.environ.items() if k in ('PATH', 'LANG', 'LC_ALL')}
    env.update(HOME=str(home), USER=ACCOUNT, XDG_STATE_HOME=str(home / 'state'),
               TMUX_TMPDIR=str(home / 'tmux'), XDG_RUNTIME_DIR=str(home / 'run'),
               SHELL='/bin/bash', TERM='xterm-256color', HTTPS_PROXY='http://127.0.0.1:9',
               HISTFILE='/dev/null', CX_THEME='dark')
    return env


def private(path, text):
    path.write_text(text)
    path.chmod(0o600)


def stat(pid):
    f = pathlib.Path(f'/proc/{pid}/stat').read_text().rsplit(')', 1)[1].split()
    return dict(state=f[0], ppid=int(f[1]), sid=int(f[3]), start=f[19])


def alive(pid):
    try:
        return stat(pid)['state'] != 'Z'
    except OSError:
        return False


def children(pid):
    out = []
    for task in os.listdir(f'/proc/{pid}/task'):
        try:
            out += [int(c) for c in pathlib.Path(f'/proc/{pid}/task/{task}/children').read_text().split()]
        except OSError:
            pass
    return out


def wait_for(predicate, seconds, what):
    end = time.monotonic() + seconds
    while time.monotonic() < end:
        value = predicate()
        if value:
            return value
        time.sleep(.03)
    raise AssertionError(f'timed out waiting for {what}')


tmp = tempfile.TemporaryDirectory(prefix='cx-ar-', dir='/tmp')  # short: sun_path
root = pathlib.Path(tmp.name)
root.chmod(0o700)
device = root / 'd'
for sub in ('.local/bin', 'state/cx', 'tmux', 'run', '.ssh', 'bin', 'work'):
    (device / sub).mkdir(parents=True, mode=0o700)
root.chmod(0o755)
for p in (device, device / 'bin', device / 'work'):
    p.chmod(0o755)
shutil.copyfile(BINARY, device / '.local/bin/cx')
(device / '.local/bin/cx').chmod(0o755)
subprocess.run(['cc', '-O1', '-pthread', '-o', str(device / 'bin/claude'), str(HERE / 'adopt_churn.c')], check=True)
relay = os.environ.get('CX_ADOPT_REPTYR')
if not relay:
    relay = str(root / 'cx-reptyr')
    subprocess.run(['sh', str(HERE.parent / 'scripts/build-portable-reptyr.sh'), relay],
                   env=dict(os.environ, CX_REPTYR_DYNAMIC='1'), check=True, stdout=subprocess.DEVNULL)
else:
    shutil.copyfile(relay, root / 'cx-reptyr')
    relay = str(root / 'cx-reptyr')
os.chmod(relay, 0o755)
denv = private_env(device)
denv['CX_ADOPT_REPTYR'] = relay

# The device's environment for each ssh command, read per call (the first viewer sees the
# device without the fixture relay).
device_env = root / 'device.env'


def device_env_with(*keys):
    device_env.write_text(' '.join(f'{k}={denv[k]}' for k in ('HOME', 'XDG_STATE_HOME', 'TMUX_TMPDIR',
                                                              'XDG_RUNTIME_DIR', 'SHELL', 'PATH', *keys)))


enter = f'exec env $(cat {device_env}) {device}/.local/bin/cx'

for name in ('host', 'client'):
    subprocess.run(['ssh-keygen', '-q', '-t', 'ed25519', '-N', '', '-f', str(root / name)], check=True, timeout=10)
shutil.copyfile(root / 'client.pub', device / '.ssh/authorized_keys')
with socket.socket() as s:
    s.bind(('127.0.0.1', 0))
    port = s.getsockname()[1]
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
PermitRootLogin prohibit-password
''')
log = root / 'argv.jsonl'
tools = root / 'tools'
tools.mkdir(mode=0o700)
(tools / 'ssh').write_text(f'''#!{sys.executable}
import json,os,sys
args=sys.argv[1:]
if '-F' in args:
 i=args.index('-F');del args[i:i+2]
args=['-F',os.path.expanduser('~/.ssh/config'),*args]
with open({str(log)!r},'a') as f:f.write(json.dumps(args)+chr(10))
if '-G' not in args and '-O' not in args and '-N' not in args and len(args)>2:
 cmd=args[-1]
 if cmd=='exec ~/.local/bin/cx helper':args[-1]={enter + ' helper'!r}
 elif cmd.startswith('~/.local/bin/cx '):args[-1]={enter!r}+cmd[len('~/.local/bin/cx'):]
 elif cmd!='exit 0':args[-1]='true'
os.execv({SSH!r},[{SSH!r},*args])
''')
(tools / 'ssh').chmod(0o700)

daemon = subprocess.Popen([SSHD, '-D', '-e', '-f', str(root / 'sshd.conf')],
                          stdout=(root / 'sshd.log').open('wb'), stderr=subprocess.STDOUT)
emulator = viewer = None
socket_path = device / 'state/cx/managed.sock'
results = {}


def tmux(*args):
    return subprocess.run(['tmux', '-f', '/dev/null', '-S', str(socket_path), *args], env=denv,
                          capture_output=True, text=True, timeout=10)


try:
    for _ in range(100):
        try:
            with socket.create_connection(('127.0.0.1', port), timeout=.1):
                break
        except OSError:
            assert daemon.poll() is None, (root / 'sshd.log').read_text()[-400:]
            time.sleep(.05)
    # The device's managed server, alive with no session yet (no user service here).
    assert tmux('start-server', ';', 'set', '-g', 'exit-empty', 'off').returncode == 0
    # The agent terminal on the device: the emulator holds its master, the stub leads it.
    emu_log = device / 'emulator'
    emulator = subprocess.Popen([sys.executable, '-c', EMULATOR, str(emu_log), str(device / 'bin/claude')],
                                env=dict(denv, CHURN_LOG=str(device / 'churn')), cwd=str(device / 'work'),
                                stdin=subprocess.DEVNULL)
    leader = int(wait_for(lambda: emu_log.exists() and emu_log.read_text().split('\n')[0].split()[-1:], 5,
                          'the agent terminal')[0])
    claude = os.path.realpath(device / 'bin/claude')
    wait_for(lambda: os.readlink(f'/proc/{leader}/exe') == claude, 5, 'the agent')
    tool = wait_for(lambda: children(leader), 5, 'its tool child')
    before = stat(leader)
    emulator_pid = emulator.pid  # (nsenter joins a user namespace in place: no fork)

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
        dict(id='local', name='tranquil', target=None, account=ACCOUNT, host='tranquil', status='local',
             observed_at=0),
        dict(id='zephyr-id', name='zephyr', target=f'{ACCOUNT}@127.0.0.1', account=ACCOUNT, host='zephyr',
             status='unknown', observed_at=0)]))
    def settle(until, what, limit):
        end = time.monotonic() + limit
        while not until():
            assert time.monotonic() < end, f'{what}:\n{viewer.text()}'
            assert viewer.proc.poll() is None, f'viewer exited {viewer.proc.returncode}:\n{viewer.text()}'
            viewer.read(.05)
        viewer.read(.1)

    # 1. The device has only a sudo relay (~/.local/bin/cx-reptyr), as innovation had: no
    # password prompt reaches its helper over SSH, so the tile offers the one-time
    # cap_sys_ptrace install instead of a move that could only fail there (v0.3.3 offered
    # `⏎ adopt` and failed with "needs a no-prompt relay").
    shutil.copyfile(relay, device / '.local/bin/cx-reptyr')
    (device / '.local/bin/cx-reptyr').chmod(0o755)
    device_env_with()
    # The viewer runs the device's copy: its helper never lists the viewer's own terminal.
    viewer = pty_lib.Pty([str(device / '.local/bin/cx')], env, home, 160, 45)
    settle(lambda: 'zephyr' in viewer.text(), 'the hub never showed the device', 20)
    viewer.write(b'z')
    settle(lambda: 'outside cx · ⏎ installs cx-reptyr' in viewer.text(),
           'a sudo-only device does not offer the one-time install', 25)
    viewer.close()
    os.remove(device / '.local/bin/cx-reptyr')
    results['sudo_only_device'] = 'offers the one-time cap_sys_ptrace install'

    # 2. With a relay that needs no prompt (here the debug fixture relay, as a
    # cap_sys_ptrace install): Enter moves the terminal and attaches.
    device_env_with('CX_ADOPT_REPTYR')
    viewer = pty_lib.Pty([str(device / '.local/bin/cx')], env, home, 160, 45)
    settle(lambda: 'zephyr' in viewer.text(), 'the hub never showed the device', 20)
    viewer.write(b'z')
    try:
        settle(lambda: 'outside cx · ⏎ adopt' in viewer.text(), 'the agent terminal is not an adopt tile', 25)
    except AssertionError:
        print((root / 'sshd.log').read_text()[-1500:], file=sys.stderr)
        raise
    viewer.write(b'\r')
    started = time.monotonic()
    settle(lambda: 'tick' in viewer.text(), 'the viewer never attached to the moved agent', 30)
    results['enter_to_attached_s'] = round(time.monotonic() - started, 2)
    assert 'password' not in viewer.text().lower(), viewer.text()
    sessions = tmux('list-sessions', '-F', '#{session_name}').stdout.split()
    moved = [s for s in sessions if s.startswith('cx-')]
    assert len(moved) == 1, sessions
    after = stat(leader)
    assert (after['start'], after['sid']) == (before['start'], before['sid']), (before, after)
    assert all(alive(t) for t in tool) and children(leader) == tool
    pane = int(tmux('display', '-p', '-t', moved[0], '#{pane_pid}').stdout)
    assert os.readlink(f'/proc/{pane}/exe') == os.path.realpath(relay), 'the relay holds the terminal'
    wait_for(lambda: not alive(emulator_pid), 5, 'the emptied emulator closes')
    results['agent'] = 'same pid, start time, session and tool child'
    # Detach (Ctrl+\\): the agent is now one of the device's managed sessions, its screen
    # live in the tile. (The viewer shares this sandbox, so its own device also lists the
    # agent, now held by a relay it does not know, as an outside terminal: ignored.)
    viewer.write(b'\x1c')
    settle(lambda: any(' z  s  work claude' in l and 'outside cx' not in l
                       for l in viewer.text().splitlines()) and 'tick' in viewer.text(),
           'the moved agent is not a managed tile after detaching', 20)
    calls = [json.loads(l) for l in log.read_text().splitlines()]
    assert any('native-attach' in c[-1] for c in calls), 'attached over ssh'
    results['ssh_calls'] = len(calls)
    assert results['enter_to_attached_s'] < 5, results
finally:
    if viewer and viewer.proc.poll() is None:
        viewer.close()
    if emulator:
        emulator.kill()
    tmux('kill-server')
    for pid in map(int, filter(str.isdigit, os.listdir('/proc'))):
        try:
            if os.readlink(f'/proc/{pid}/exe') == os.path.realpath(device / 'bin/claude'):
                os.kill(pid, signal.SIGKILL)
        except OSError:
            pass
    daemon.terminate()
    daemon.wait(timeout=5)
    tmp.cleanup()
print(json.dumps(dict(result='PASS', **results)))
