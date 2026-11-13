#!/usr/bin/env python3
"""Secrets exposed to sessions reach new local sessions' environment and nothing else.

usage: secrets_sessions_live.py BINARY          a local `cx new`
       secrets_sessions_live.py BINARY --ssh    also a launch on a device over a loopback sshd

A private HOME/XDG_STATE_HOME/TMUX_TMPDIR, the file backend and no session bus. Two canary secrets are stored, one
exposed with its default variable (anthropic.api_key, ANTHROPIC_API_KEY) and one with an edited variable, by
writing the index as the keys screen does. `~/.local/bin/tmux` is a wrapper that logs every tmux argv cx runs.

Local: a session started before exposure, one with the secrets exposed and one after exposure is turned off. Only
the exposed one has the canaries, in its shell's environment; they are absent from every tmux argv, the pane's
start command, the cmdline of every process under the fixture's tmux server, the server's global and session
environments, its options, cx's own output, and every file under the private root except the secrets store. A cx
run with the exposed pane's environment (as one typed into it) keeps them there too: its sessions get them only
while exposure is on, never from tmux's server environment.
--ssh: the device is this machine with the same state (so its helper could read the store): a session created on
it through ssh gets neither canary, and neither appears in any ssh argv or in ssh's environment.
Only processes under the fixture's own tmux server are inspected. Prints one JSON line.
"""
import contextlib, json, os, pathlib, pwd, shutil, socket, subprocess, sys, tempfile, time

BINARY = str(pathlib.Path(sys.argv[1]).resolve())
SSH = '--ssh' in sys.argv[2:]
CANARY = 'cx-canary-5be1c0de-expose-DO-NOT-LOG'
CANARY2 = 'cx-canary-77aa0f19-custom-var'
MARKS = (CANARY.encode(), CANARY2.encode(), CANARY[:14].encode())

tmp = tempfile.TemporaryDirectory(prefix='cx-sx-', dir='/tmp')  # short: sun_path
root = pathlib.Path(tmp.name)
root.chmod(0o700)
home = root / 'h'
state = home / 'state'
store = state / 'cx'
for sub in ('.local/bin', 'state/cx', 'tmux', 'tools'):
    (home / sub).mkdir(parents=True, mode=0o700, exist_ok=True)
real_tmux = shutil.which('tmux')
log = root / 'tmux-argv.jsonl'
wrapper = home / '.local/bin/tmux'
wrapper.write_text(f'''#!{sys.executable}
import json,os,sys
with open({str(log)!r},'a') as f:f.write(json.dumps(sys.argv[1:])+chr(10))
os.execv({real_tmux!r},[{real_tmux!r},*sys.argv[1:]])
''')
wrapper.chmod(0o700)
# XDG_RUNTIME_DIR stays: the managed server starts as its own user service, as in every session fixture.
keep = {k: v for k, v in os.environ.items() if k in ('PATH', 'LANG', 'LC_ALL', 'XDG_RUNTIME_DIR')}
env = dict(keep, HOME=str(home), XDG_STATE_HOME=str(state),
           TMUX_TMPDIR=str(home / 'tmux'), SHELL='/bin/bash', TERM='xterm-256color', HISTFILE='/dev/null',
           CX_SECRETS_BACKEND='file', USER=pwd.getpwuid(os.getuid()).pw_name)
socket_path = store / 'managed.sock'
outputs = []


def cx(*args, stdin=None, run_env=None):
    r = subprocess.run([BINARY, *args], env=run_env or env, input=stdin, capture_output=True, timeout=60,
                       start_new_session=True)
    outputs.append(r.stdout + r.stderr)
    assert r.returncode == 0, (args, r.stderr[-600:])
    return r.stdout


def tmux(*args):
    r = subprocess.run([real_tmux, '-S', str(socket_path), *args], env=env, capture_output=True, timeout=10)
    return r.stdout


def expose(on):
    """What the keys screen's `e` and `v` write: the flag and the edited variable in the index."""
    path = store / 'secrets.json'
    index = json.loads(path.read_text())
    index['secrets']['anthropic.api_key']['expose'] = on
    index['secrets']['robot.token']['expose'] = on
    index['secrets']['robot.token']['env'] = 'FIXTURE_TOKEN'
    path.write_text(json.dumps(index))


def new(key, device=None, run_env=None):
    args = ['new', '--directory', str(home), '--key', key, '--name', key]
    if device:
        args += ['--device', device]
    return json.loads(cx(*args, run_env=run_env))


def inside(pane):
    """A cx typed into `pane`: its whole environment, the canaries included. The pane itself gets HOME and state
    from the tmux user service (the real ones), so those, PATH, SHELL and the backend are the fixture's: a cx run in a
    fixture pane must never reach the real state or the real managed server."""
    nested = dict(pane, **{k: env[k] for k in ('HOME', 'XDG_STATE_HOME', 'TMUX_TMPDIR', 'PATH', 'SHELL',
                                                'CX_SECRETS_BACKEND', 'XDG_RUNTIME_DIR') if k in env})
    assert nested['HOME'] == str(home) and nested['XDG_STATE_HOME'] == str(state)
    return nested


def pane_env(session):
    pid = int(tmux('display-message', '-p', '-t', session['id'], '#{pane_pid}').decode())
    end = time.monotonic() + 10
    while True:
        # The pane runs `sh -c 'exec env … [cx secrets get … --exec --] bash -l'`: one pid, the shell last.
        if pathlib.Path(f'/proc/{pid}/comm').read_text().strip() == 'bash':
            break
        assert time.monotonic() < end, 'the pane never became the login shell'
        time.sleep(0.05)
    raw = pathlib.Path(f'/proc/{pid}/environ').read_bytes().split(b'\0')
    return dict(v.decode(errors='replace').split('=', 1) for v in raw if b'=' in v)


def subtree(pid):
    found, queue = [], [pid]
    while queue:
        p = queue.pop()
        found.append(p)
        for task in pathlib.Path(f'/proc/{p}/task').glob('*'):
            try:
                queue += [int(c) for c in (task / 'children').read_text().split()]
            except OSError:
                pass
    return found


def leaks(data):
    return any(m in data for m in MARKS)


def check_nowhere_but_the_session():
    server = int(tmux('display-message', '-p', '#{pid}').decode())
    for pid in subtree(server):
        try:
            assert not leaks(pathlib.Path(f'/proc/{pid}/cmdline').read_bytes()), f'argv of {pid}'
        except FileNotFoundError:
            pass
    assert not leaks(log.read_bytes()), 'a tmux argv carried a value'
    assert not leaks(tmux('show-environment', '-g')), 'global environment'
    for s in tmux('list-sessions', '-F', '#{session_id}').decode().split():
        assert not leaks(tmux('show-environment', '-t', s)), 'session environment'
        assert not leaks(tmux('show-options', '-t', s)), 'session options'
        assert not leaks(tmux('list-panes', '-s', '-t', s, '-F', '#{pane_start_command}')), 'start command'
    for scope in ('-g', '-s', '-gw'):
        assert not leaks(tmux('show-options', scope)), f'options {scope}'
    assert not any(leaks(o) for o in outputs), 'cx output'
    allowed = store / 'secrets.values.json'
    files = 0
    for path in root.rglob('*'):
        # (tools/ holds the fixture's own ssh wrapper, which carries the canaries to look for them.)
        if path.is_file() and not path.is_symlink() and path != allowed and home / 'tools' not in path.parents:
            files += 1
            assert not leaks(path.read_bytes()), f'{path} holds a value'
    assert CANARY.encode() in allowed.read_bytes()
    return files


@contextlib.contextmanager
def loopback():
    """Device `fixture`: a private sshd on 127.0.0.1 whose `cx helper` runs with this same HOME and state (one
    machine), through an `ssh` wrapper that logs each argv and whether its own environment holds a value."""
    account = env['USER']
    sshd = shutil.which('sshd') or '/usr/sbin/sshd'
    real_ssh = shutil.which('ssh')
    with socket.socket() as s:
        s.bind(('127.0.0.1', 0))
        port = s.getsockname()[1]
    for name in ('host', 'key'):
        subprocess.run(['ssh-keygen', '-q', '-t', 'ed25519', '-N', '', '-f', str(root / name)], check=True,
                       timeout=10)
    (root / 'authorized').write_text((root / 'key.pub').read_text())
    (root / 'sshd.conf').write_text(f"""Port {port}
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
    host_key = (root / 'host.pub').read_text().split()
    (root / 'known').write_text(f'[127.0.0.1]:{port} {host_key[0]} {host_key[1]}\n')
    (root / 'client.conf').write_text(f"""Host fixture
 HostName 127.0.0.1
 Port {port}
 User {account}
 IdentityFile {root}/key
 IdentitiesOnly yes
 IdentityAgent none
 UserKnownHostsFile {root}/known
 GlobalKnownHostsFile /dev/null
 StrictHostKeyChecking yes
""")
    device_env = ' '.join(f'{k}={env[k]}' for k in ('HOME', 'XDG_STATE_HOME', 'TMUX_TMPDIR', 'SHELL',
                                                     'CX_SECRETS_BACKEND', 'PATH') + tuple(
                                                         k for k in ('XDG_RUNTIME_DIR',) if k in env))
    calls = root / 'ssh-calls.jsonl'
    tools = home / 'tools'
    (tools / 'ssh').write_text(f"""#!{sys.executable}
import json,os,sys
args=sys.argv[1:]
if '-F' in args:
 pos=args.index('-F');del args[pos:pos+2]
held=any(m in v for v in os.environb.values() for m in {MARKS!r})
with open({str(calls)!r},'a') as f:f.write(json.dumps(dict(argv=args,env_holds_value=held))+chr(10))
if args and args[-1]=='exec ~/.local/bin/cx helper':args[-1]={f'exec env {device_env} {BINARY} helper'!r}
elif args and args[-1].startswith('~/.local/bin/cx native-attach'):args[-1]='true'
os.execv({real_ssh!r},[{real_ssh!r},'-F',{str(root / 'client.conf')!r},*args])
""")
    (tools / 'ssh').chmod(0o700)
    (store / 'devices.json').write_text(json.dumps([dict(
        id='fixture:fixture', name='fixture', target='fixture', account=account, host='fixture',
        status='unknown', observed_at=0)]))
    (store / 'devices.json').chmod(0o600)
    daemon = subprocess.Popen([sshd, '-D', '-e', '-f', str(root / 'sshd.conf')],
                              stdout=(root / 'sshd.log').open('wb'), stderr=subprocess.STDOUT,
                              start_new_session=True)
    path = env['PATH']
    env['PATH'] = f'{tools}{os.pathsep}{path}'
    try:
        for _ in range(100):
            assert daemon.poll() is None, (root / 'sshd.log').read_text()[-400:]
            try:
                with socket.create_connection(('127.0.0.1', port), timeout=0.1):
                    break
            except OSError:
                time.sleep(0.05)
        yield lambda: [json.loads(l) for l in calls.read_text().splitlines()]
    finally:
        for master in (store / 'ssh').glob('*'):
            if master.is_socket():
                subprocess.run([real_ssh, '-F', '/dev/null', '-S', str(master), '-O', 'exit', 'cx'],
                               capture_output=True, timeout=5)
        env['PATH'] = path
        daemon.terminate()
        daemon.wait(timeout=5)


try:
    cx('secrets', 'set', 'anthropic.api_key', stdin=CANARY.encode())
    cx('secrets', 'set', 'robot.token', stdin=CANARY2.encode())
    before = new('sx-before')
    expose(True)
    exposed = new('sx-exposed')
    expose(False)
    after = new('sx-after')
    got = pane_env(exposed)
    assert got.get('ANTHROPIC_API_KEY') == CANARY and got.get('FIXTURE_TOKEN') == CANARY2, sorted(got)
    assert got.get('CX_MANAGED_SESSION') == exposed['id'], 'the launcher environment is intact'
    for s in (before, after):
        e = pane_env(s)
        assert not any(leaks(v.encode()) for v in e.values()), f'{s["name"]} got a secret'
        assert 'ANTHROPIC_API_KEY' not in e and 'FIXTURE_TOKEN' not in e
    # cx run inside the exposed session (an agent's or the user's `cx new`, the viewer) has the values in its own
    # environment: they stay out of tmux's argv and server environment, and its sessions get them only through
    # the switch, never by inheritance.
    nested_off = new('sx-nested-off', run_env=inside(got))
    e = pane_env(nested_off)
    assert not any(leaks(v.encode()) for v in e.values()), 'a session started from the exposed one inherited it'
    expose(True)
    nested_on = new('sx-nested-on', run_env=inside(got))
    assert pane_env(nested_on).get('ANTHROPIC_API_KEY') == CANARY, 'exposed from inside a session too'
    expose(False)
    files = check_nowhere_but_the_session()
    checks = dict(exposed_in_new_session=True, absent_when_off=True, existing_session_unaffected=True,
                  cx_inside_exposed_session=True,
                  tmux_argv_logged=len(log.read_text().splitlines()), files_scanned=files)
    if SSH:
        expose(True)
        with loopback() as ssh_calls:
            remote = new('sx-remote', device='fixture')
            e = pane_env(remote)
            assert not any(leaks(v.encode()) for v in e.values()), 'the device session got a secret'
            assert 'ANTHROPIC_API_KEY' not in e and 'FIXTURE_TOKEN' not in e
            calls = ssh_calls()
            assert any(c['argv'][-1] == 'exec ~/.local/bin/cx helper' for c in calls), calls
            assert not any(leaks(json.dumps(c['argv']).encode()) or c['env_holds_value'] for c in calls), calls
        local = new('sx-local-again')
        assert pane_env(local).get('ANTHROPIC_API_KEY') == CANARY, 'exposure stayed on for local launches'
        check_nowhere_but_the_session()
        checks.update(remote_launch_without_secrets=True, ssh_calls=len(calls))
    print(json.dumps(dict(result='PASS', **checks)))
finally:
    tmux('kill-server')
    tmp.cleanup()
