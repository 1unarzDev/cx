#!/usr/bin/env python3
"""Address fallback, failure reasons and remote outside terminals, against a loopback sshd.

usage: address_fallback_pty.py BINARY

Runs itself inside tests/isolate.sh (private user, pid and mount namespaces): the viewer
and the device's helper only ever see this fixture's processes. A private sshd on
127.0.0.1 serves a device HOME whose `cx helper` (this binary) holds one managed session
and discovers one fake agent terminal (a copy of bash named claude, on a pty this script
holds, outside any tmux). The viewer has its own HOME; an `ssh` wrapper on its PATH
replaces the remote `cx helper` command with the device HOME's and lets `exit 0` probes
and `ssh -G` through. Nothing touches the caller's HOME, SSH trust, tmux or networking.

Checks:
- fallback: the device's target name does not resolve (`*.invalid`); its recorded LAN
  address (127.0.0.1) is tried, presents the key pinned under the original name
  (HostKeyAlias), and is remembered: the hub lists the device's session, the device
  strip says `via 127.0.0.1`, and the outside agent terminal is a tile marked
  `outside cx`, not counted as a managed session;
- strict keys: the same, with a different key pinned under the name: the fallback is
  refused, never remembered, and the device says its name does not resolve;
- reasons: an unresolvable name and a refused port each show their reason on the
  device within the connect timeout.
"""
import json, os, pathlib, pwd, shutil, socket, subprocess, sys, tempfile, time

HERE = pathlib.Path(__file__).resolve().parent
if not pathlib.Path('/dev/shm/cx-isolated').exists():
    os.execv(str(HERE / 'isolate.sh'), [str(HERE / 'isolate.sh'), sys.executable, *sys.argv])
sys.path.insert(0, str(HERE))
import pty_lib  # noqa: E402

BINARY = str(pathlib.Path(sys.argv[1]).resolve())
ACCOUNT = pwd.getpwuid(os.getuid()).pw_name
SSH = shutil.which('ssh')
SSHD = shutil.which('sshd') or '/usr/sbin/sshd'
NAME = 'cx-fallback.invalid'


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


def free_port():
    with socket.socket() as s:
        s.bind(('127.0.0.1', 0))
        return s.getsockname()[1]


tmp = tempfile.TemporaryDirectory(prefix='cx-fb-', dir='/tmp')
root = pathlib.Path(tmp.name)
root.chmod(0o700)
device = root / 'd'
for sub in ('.local/bin', 'state/cx', 'tmux', 'run', '.ssh', 'bin', 'work'):
    (device / sub).mkdir(parents=True, mode=0o700)
shutil.copyfile(BINARY, device / '.local/bin/cx')
(device / '.local/bin/cx').chmod(0o755)
denv = private_env(device)
for name in ('host', 'other', 'client'):
    subprocess.run(['ssh-keygen', '-q', '-t', 'ed25519', '-N', '', '-f', str(root / name)],
                   check=True, timeout=10)
shutil.copyfile(root / 'client.pub', device / '.ssh/authorized_keys')
port, closed = free_port(), free_port()
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
log = root / 'argv.jsonl'
remote = ' '.join(f'{k}={denv[k]}' for k in ('HOME', 'XDG_STATE_HOME', 'TMUX_TMPDIR', 'SHELL'))
tools = root / 'tools'
tools.mkdir(mode=0o700)
(tools / 'ssh').write_text(f'''#!{sys.executable}
import json,os,sys
args=sys.argv[1:]
# Inside the user namespace /etc/ssh/ssh_config has no mapped owner, which ssh refuses to
# Include: cx's background config is swapped for the viewer's own (its policy is in -o).
if '-F' in args:
 i=args.index('-F');del args[i:i+2]
args=['-F',os.path.expanduser('~/.ssh/config'),*args]
with open({str(log)!r},'a') as f:f.write(json.dumps(args)+chr(10))
if '-G' not in args and '-O' not in args and '-N' not in args and len(args)>2:
 if args[-1]=='exec ~/.local/bin/cx helper':args[-1]={f'exec env {remote} {device}/.local/bin/cx helper'!r}
 elif args[-1]!='exit 0':args[-1]='true'
os.execv({SSH!r},[{SSH!r},*args])
''')
(tools / 'ssh').chmod(0o700)

daemon = subprocess.Popen([SSHD, '-D', '-e', '-f', str(root / 'sshd.conf')],
                          stdout=(root / 'sshd.log').open('wb'), stderr=subprocess.STDOUT)
agent = None
viewers = []
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
                          '--name', 'fb-session', '--key', 'fb-session'],
                         env=denv, capture_output=True, text=True, timeout=30)
    assert out.returncode == 0, out.stderr[-400:]
    # An agent terminal outside any tmux: its own session on a pty held here.
    claude = device / 'bin/claude'
    shutil.copy2(shutil.which('bash'), claude)
    agent = pty_lib.Pty([str(claude), '--norc', '--noprofile', '-c', 'sleep 100000; :'],
                        denv, device / 'work', 80, 24)

    def viewer(case, target, known_key, addresses):
        home = root / case
        for sub in ('state/cx', 'tmux', 'run', '.ssh'):
            (home / sub).mkdir(parents=True, mode=0o700)
        env = private_env(home)
        env['PATH'] = f'{tools}{os.pathsep}{env["PATH"]}'
        host_key = ' '.join((root / f'{known_key}.pub').read_text().split()[:2])
        private(home / '.ssh/known_hosts', f'[{NAME}]:{port} {host_key}\n[127.0.0.1]:{closed} {host_key}\n')
        private(home / '.ssh/config', f'''Host {NAME} 127.0.0.1
  Port {port if case != 'refused' else closed}
  IdentityFile {root / 'client'}
  IdentitiesOnly yes
  IdentityAgent none
  UserKnownHostsFile {home / '.ssh/known_hosts'}
  GlobalKnownHostsFile /dev/null
''')
        private(home / 'state/cx/devices.json', json.dumps([
            dict(id='local', name='tranquil', target=None, account=ACCOUNT, host='tranquil',
                 status='local', observed_at=0),
            dict(id='zephyr-id', name='zephyr', target=target, account=ACCOUNT, host='zephyr',
                 status='unknown', observed_at=0)]))
        if addresses:
            private(home / 'state/cx/addresses.json', json.dumps({target: addresses}))
        v = pty_lib.Pty([BINARY], env, home, 160, 45)
        viewers.append(v)
        return home, v

    def settle(v, until, what, limit):
        end = time.monotonic() + limit
        while not until():
            assert time.monotonic() < end, f'{what}:\n{v.text()}'
            assert v.proc.poll() is None, f'viewer exited {v.proc.returncode}:\n{v.text()}'
            v.read(.05)
        v.read(.1)

    # 1. Fallback: the name fails, the recorded LAN address answers with the pinned key.
    target = f'{ACCOUNT}@{NAME}'
    home, v = viewer('fallback', target, 'host', {'lan': '127.0.0.1'})
    started = time.monotonic()
    settle(v, lambda: 'fb-session' in v.text(), 'the hub never listed the session via the fallback', 25)
    listed_after = round(time.monotonic() - started, 1)
    book = json.loads((home / 'state/cx/addresses.json').read_text())
    assert book[target]['via'] == '127.0.0.1', book
    assert book[target]['alias'] == f'[{NAME}]:{port}', book
    probes = [json.loads(l) for l in log.read_text().splitlines()]
    assert any(f'HostKeyAlias=[{NAME}]:{port}' in a and 'StrictHostKeyChecking=yes' in a
               and 'HostName=127.0.0.1' in a for a in probes), 'no strict alias probe'
    v.write(b'z')
    settle(v, lambda: 'via 127.0.0.1' in v.text(), 'the device never said via 127.0.0.1', 10)
    settle(v, lambda: 'outside cx' in v.text(), 'the outside agent terminal is not a tile', 20)
    assert '1 session listed' in v.text(), v.text()
    results['fallback'] = dict(listed_s=listed_after, via='127.0.0.1', alias=book[target]['alias'],
                               outside_tile=True, managed_count=1)

    # 2. A different key under the original name: refused, never remembered.
    home, v = viewer('wrongkey', target, 'other', {'lan': '127.0.0.1'})
    v.write(b'z')
    started = time.monotonic()
    settle(v, lambda: 'Name does not resolve' in v.text(), 'no reason for the unresolvable name', 8)
    results['unresolved_reason_s'] = round(time.monotonic() - started, 1)
    end = time.monotonic() + 6
    while time.monotonic() < end:
        v.read(.1)
    book = json.loads((home / 'state/cx/addresses.json').read_text())
    assert 'via' not in book.get(target, {}), book
    assert 'fb-session' not in v.text(), v.text()
    results['wrong_key'] = 'refused, not remembered'

    # 3. A refused port says so within the connect timeout.
    home, v = viewer('refused', f'{ACCOUNT}@127.0.0.1', 'host', None)
    v.write(b'z')
    started = time.monotonic()
    settle(v, lambda: 'SSH connection refused' in v.text(), 'no reason for the refused port', 8)
    results['refused_reason_s'] = round(time.monotonic() - started, 1)
finally:
    for v in viewers:
        v.close()
    if agent:
        agent.close()
    subprocess.run(['tmux', '-S', str(device / 'state/cx/managed.sock'), 'kill-server'],
                   env=denv, capture_output=True, timeout=10)
    daemon.terminate()
    daemon.wait(timeout=5)
    tmp.cleanup()
print(json.dumps(dict(result='PASS', **results)))
