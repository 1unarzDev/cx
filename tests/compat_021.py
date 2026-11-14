#!/usr/bin/env python3
"""Compatibility of two cx builds over a disposable loopback sshd, both directions
(written for v0.2.1 <-> v0.2.2; used for v0.2.1, v0.2.2 and fed0fc3 <-> v0.3.0).

usage: compat_021.py OLD_CX NEW_CX     (for example ~/.cache/cx-targets/v022/cx
                                         and target/release/cx)

Run it inside tests/isolate.sh where available (private pid/user namespaces).

Not in CI (it needs two builds and sshd). A viewer HOME and a device HOME, both this OS
account, each private (HOME, XDG_STATE_HOME, TMUX_TMPDIR); the device's helper is
`<device HOME>/.local/bin/cx`, reached through a private sshd on 127.0.0.1 with cx's
managed peer keys. An ssh wrapper sets the fixture port and runs the remote
`~/.local/bin/cx ...` under the device's private environment; nothing else runs remotely.

Two scenarios, each on a fresh device tmux server:
  A  old viewer, new helper: the device's server and session `a1` are started by the
     old helper; the helper is swapped to the new binary (write, rename: the fleet
     update) and `a2` is created by it.
  B  new viewer, old helper: the server and `b1` are started by the new helper; it is
     rolled back to the old binary and `b2` is created by it.
For each: the viewer lists the device's sessions (CLI), previews them (the viewer's own
UI on a PTY: the device's letter, then the arrows across its tiles, which must draw a
marker each pane printed), attaches every session (`cx attach --device`, on a PTY) and detaches
with Ctrl+\\. The device server's pid, every session id and every pane pid are
recorded before and after and must be unchanged. The server's global, server and window
options and its key tables are recorded once only the first version has configured it
and compared at the end: the second version's create and attach re-source its managed
config into the running server, which must change nothing.
Prints one JSON line with the evidence.
"""
import json, os, pathlib, pwd, shlex, shutil, socket, subprocess, sys, tempfile, time

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import pty_lib  # noqa: E402

OLD = str(pathlib.Path(sys.argv[1]).resolve())
NEW = str(pathlib.Path(sys.argv[2]).resolve())
account = pwd.getpwuid(os.getuid()).pw_name
ssh = shutil.which('ssh')
sshd = shutil.which('sshd') or '/usr/sbin/sshd'
machine = pathlib.Path('/etc/machine-id').read_text().strip()


def version(binary):
    return subprocess.run([binary, '--version'], capture_output=True, text=True).stdout.split()[-1]


def main():
    versions = dict(old=version(OLD), new=version(NEW))
    assert versions['old'] != versions['new'], versions
    # Short root: ssh control sockets must fit sun_path.
    tmp = tempfile.TemporaryDirectory(prefix='cx-c21-', dir='/tmp')
    root = pathlib.Path(tmp.name)
    root.chmod(0o700)
    viewer, device = root / 'v', root / 'd'
    for home in (viewer, device):
        (home / '.ssh').mkdir(parents=True, mode=0o700)
        (home / 'state/cx').mkdir(parents=True, mode=0o700)
        (home / 'tmux').mkdir(mode=0o700)
        (home / '.local/bin').mkdir(parents=True)
    with socket.socket() as s:
        s.bind(('127.0.0.1', 0))
        port = s.getsockname()[1]
    subprocess.run(['ssh-keygen', '-q', '-t', 'ed25519', '-N', '', '-f', str(root / 'host')],
                   check=True, timeout=10)
    host_key = ' '.join((root / 'host.pub').read_text().split()[:2])
    base = {k: v for k, v in os.environ.items()
            if k not in ('SSH_AUTH_SOCK', 'SSH_AGENT_PID', 'TMUX', 'TMUX_PANE', 'NO_COLOR', 'CX_ASCII')}

    def env_of(home, **extra):
        return dict(base, HOME=str(home), XDG_STATE_HOME=str(home / 'state'),
                    TMUX_TMPDIR=str(home / 'tmux'), USER=account, SHELL='/bin/bash',
                    TERM='xterm-256color', HISTFILE='/dev/null', CX_THEME='dark',
                    HTTPS_PROXY='http://127.0.0.1:9', **extra)
    dev_env = env_of(device)
    remote_cx = ' '.join(['env'] + [f'{k}={shlex.quote(dev_env[k])}' for k in
                                    ('HOME', 'XDG_STATE_HOME', 'TMUX_TMPDIR', 'USER', 'SHELL')]
                         + [shlex.quote(str(device / '.local/bin/cx'))])
    tools = root / 'tools'
    tools.mkdir()
    (tools / 'ssh').write_text(f'''#!{sys.executable} -I
import json,os,sys
args=sys.argv[1:]
with open({str(root / 'argv.jsonl')!r},'a') as log:log.write(json.dumps(args)+'\\n')
assert 'ForwardAgent=no' in args and 'IdentitiesOnly=yes' in args, args
args[args.index('Port=22')]='Port={port}'
if '-F' not in args:args=['-F','/dev/null']+args
if not ('-N' in args or '-O' in args or '-G' in args):
    # Only cx's own fixed helper invocations run on the device.
    assert args[-1].startswith(('exec ~/.local/bin/cx ','~/.local/bin/cx ')), args[-1][:80]
    args[-1]=args[-1].replace('~/.local/bin/cx',{remote_cx!r},1)
os.execv({ssh!r},[{ssh!r},*args])
''')
    (tools / 'ssh').chmod(0o700)
    view_env = env_of(viewer, PATH=f"{tools}{os.pathsep}{base['PATH']}")
    sock = str(device / 'state/cx/managed.sock')
    evidence = dict(versions=versions, scenarios={})
    daemon = viewer_pty = None

    def helper_op(home, binary, op, args=None, distinct_machine=False):
        request = dict(version=1, id='compat', op=dict(op=op))
        if args is not None:
            request['op']['args'] = args
        payload = json.dumps(request).encode()
        command = [binary, 'helper']
        if distinct_machine:
            # Both sides share this host's machine-id; the viewer's one pairing call
            # sees a private one so the device is not "self" (as edge_policy_ssh).
            fake = root / 'machine-id'
            fake.write_text('0' * 32 + '\n')
            command = ['unshare', '-rm', 'sh', '-c',
                       f'mount --bind {shlex.quote(str(fake))} /etc/machine-id && exec "$@"', 'sh',
                       *command]
        out = subprocess.run(command, env=env_of(home), capture_output=True, timeout=30,
                             input=b'CX1 ' + str(len(payload)).encode() + b'\n' + payload)
        assert out.returncode == 0, out.stderr[-400:]
        response = json.loads(out.stdout.split(b'\n', 1)[1])
        assert not response['error'], response
        return response['result']

    def tmux(*args):
        out = subprocess.run(['tmux', '-S', sock, *args], env=dev_env, capture_output=True,
                             text=True, timeout=10)
        return out.stdout if out.returncode == 0 else None

    def snapshot():
        """Server pid, and each session's panes as {session_name: [pane_pid, ...]}."""
        panes = {}
        for line in (tmux('list-panes', '-a', '-F', '#{session_name} #{pane_pid}') or '').splitlines():
            name, pid = line.split()
            if name != 'cxctl':  # cx's hidden control-mode session
                panes.setdefault(name, []).append(int(pid))
        return dict(server=int((tmux('display-message', '-p', '#{pid}') or '0').strip() or 0),
                    panes=panes)

    def config():
        return dict(options=tmux('show-options', '-g'), server=tmux('show-options', '-s'),
                    window=tmux('show-options', '-gw'), keys=tmux('list-keys'))

    def digest(cfg):
        """Line counts and a hash per table, and allow-passthrough, for the evidence."""
        import hashlib
        out = {k: [len((v or '').splitlines()), hashlib.sha256((v or '').encode()).hexdigest()[:12]]
               for k, v in cfg.items()}
        out['allow-passthrough'] = [line for v in cfg.values() for line in (v or '').splitlines()
                                    if line.startswith('allow-passthrough')]
        return out

    def configured_by():
        """The version that last sourced the managed config into the running server."""
        return (device / 'state/cx/tmux-configured').read_text().split()[-1]

    def install(binary):
        """The fleet update: write cx.new, fsync, rename over cx (old processes keep theirs)."""
        target = device / '.local/bin/cx'
        staged = target.with_name('cx.new')
        shutil.copyfile(binary, staged)
        staged.chmod(0o755)
        with open(staged, 'rb') as f:
            os.fsync(f.fileno())
        os.rename(staged, target)

    def create(name):
        out = subprocess.run([str(device / '.local/bin/cx'), 'new', '--directory', str(device),
                              '--name', name, '--key', name], env=dev_env, capture_output=True,
                             text=True, timeout=30)
        assert out.returncode == 0, out.stderr[-400:]
        listed = json.loads(subprocess.run([str(device / '.local/bin/cx'), 'sessions'], env=dev_env,
                                           capture_output=True, timeout=30).stdout)
        [s] = [s for s in listed if s['name'] == name]
        # The pane prints a marker the viewer's preview must show.
        mark = f'compat-mark-{name}'
        for _ in range(50):
            if tmux('send-keys', '-t', s['id'], '-l', f"printf '%s\\n' {mark.upper()} | tr A-Z a-z") is not None:
                break
            time.sleep(.1)
        tmux('send-keys', '-t', s['id'], 'Enter')
        end = time.monotonic() + 10
        while mark not in (tmux('capture-pane', '-p', '-t', s['id']) or ''):
            assert time.monotonic() < end, f'{name} never printed its marker'
            time.sleep(.1)
        return s['id'], mark

    def owned():
        found = set()
        marks = [f'HOME={h}\0'.encode() for h in (viewer, device)]
        for entry in pathlib.Path('/proc').iterdir():
            if entry.name.isdigit():
                try:
                    data = (entry / 'environ').read_bytes() + (entry / 'cmdline').read_bytes()
                except OSError:
                    continue
                if any(m in data for m in marks) or str(root).encode() in (entry / 'cmdline').read_bytes():
                    found.add(int(entry.name))
        found.discard(os.getpid())
        return found

    def cli(binary, *args):
        return subprocess.run([binary, *args], env=view_env, stdin=subprocess.DEVNULL,
                              capture_output=True, text=True, timeout=60)

    def preview(binary, marks):
        """The viewer's UI: the device's letter opens it and the arrows select each
        tile; every pane's marker must be drawn."""
        nonlocal viewer_pty
        viewer_pty = pty_lib.Pty([binary], view_env, viewer, 160, 50)
        p = viewer_pty
        text = lambda: '\n'.join(p.screen.display)  # noqa: E731

        def settle(until, what, limit=20.0):
            end = time.monotonic() + limit
            while not until():
                assert time.monotonic() < end, f'{what}:\n{text()}'
                assert p.proc.poll() is None, f'viewer exited:\n{text()}'
                p.read(.05)
        settle(lambda: 'remote' in text(), 'the viewer never listed the device')
        if 'welcome' in p.screen.display[0]:
            p.write(b'\x1b')
            settle(lambda: 'welcome' not in p.screen.display[0], 'Esc kept the welcome page')
        p.write(b'r')
        settle(lambda: sum(n in text() for n in marks) >= 1 or 'SESSIONS' in text(),
               'the device did not open')
        # (v0.2.1 captures only the selected tile, so the selection walks the tiles: in
        # v0.2.1 Right from the last tile leaves the row, so Left comes first.)
        seen = set()
        for key in [b'', b'\x1b[D', b'h', b'\t', b'k', b'j', b'\x1b[C']:
            p.write(key)
            settle(lambda: any(m in text() for m in marks), 'no preview drawn')
            end = time.monotonic() + 3
            while time.monotonic() < end and not (set(marks) - seen) & {m for m in marks if m in text()}:
                p.read(.05)
            seen |= {m for m in marks if m in text()}
            if seen >= set(marks):
                break
        assert seen >= set(marks), seen
        p.write(b'\x03')
        end = time.monotonic() + 5
        while p.proc.poll() is None and time.monotonic() < end:
            p.read(.05)
        p.close()
        viewer_pty = None
        return sorted(seen)

    def attach(binary, sid):
        """`cx attach` on a PTY until the device server has a terminal client, then Ctrl+\\."""
        p = pty_lib.Pty([binary, 'attach', sid, '--device', 'remote'], view_env, viewer, 120, 40)
        try:
            def clients():
                out = tmux('list-clients', '-F', '#{client_control_mode} #{session_id}') or ''
                return [line for line in out.splitlines() if line.startswith('0 ')]
            end = time.monotonic() + 20
            while not clients():
                assert p.proc.poll() is None, f'attach exited {p.proc.returncode}: {p.text()[-600:]}'
                assert time.monotonic() < end, f'attach of {sid} never reached the server'
                p.read(.05)
            # (Ctrl+\ once the client drew its status line; before tmux's raw mode it is SIGQUIT.)
            end = time.monotonic() + 10
            while 'detach' not in p.screen.display[-1]:
                assert time.monotonic() < end, f'no status line: {p.text()[-600:]}'
                p.read(.05)
            p.write(b'\x1c')
            end = time.monotonic() + 10
            while p.proc.poll() is None or clients():
                assert time.monotonic() < end, 'Ctrl+\\ did not detach'
                p.read(.05)
            return p.proc.returncode
        finally:
            p.close()

    try:
        # Pair the viewer with the device (cx's managed peer keys, the fixture host key).
        keys = {name: helper_op(home, NEW, 'peer_identity')['client_key']
                for name, home in (('viewer', viewer), ('device', device))}
        viewer_peer = dict(id=f'viewer:{account}', name='viewer', target=f'{account}@127.0.0.1',
                           account=account, host='viewer', status='unknown', observed_at=0)
        remote = dict(id=f'{machine}:{account}', name='remote', target=f'{account}@127.0.0.1',
                      account=account, host='remote', status='unknown', observed_at=0)
        helper_op(device, NEW, 'peer_configure',
                  dict(peer=viewer_peer, host_key=host_key, client_key=keys['viewer']))
        helper_op(viewer, NEW, 'peer_configure',
                  dict(peer=remote, host_key=host_key, client_key=keys['device']), distinct_machine=True)
        (viewer / 'state/cx/devices.json').write_text(json.dumps([remote]))
        conf = root / 'sshd.conf'
        conf.write_text(f'''Port {port}
ListenAddress 127.0.0.1
HostKey {root / 'host'}
PidFile {root}/sshd.pid
AuthorizedKeysFile {device}/.ssh/authorized_keys
StrictModes no
UsePAM no
PasswordAuthentication no
KbdInteractiveAuthentication no
PubkeyAuthentication yes
AllowUsers {account}
''')
        daemon = subprocess.Popen([sshd, '-D', '-e', '-f', str(conf)],
                                  stdout=(root / 'sshd.log').open('wb'), stderr=subprocess.STDOUT)
        for _ in range(100):
            try:
                with socket.create_connection(('127.0.0.1', port), timeout=.1):
                    break
            except OSError:
                assert daemon.poll() is None, (root / 'sshd.log').read_text()[-400:]
                time.sleep(.05)

        for name, viewer_bin, first, second in (('A old viewer, new helper', OLD, OLD, NEW),
                                                ('B new viewer, old helper', NEW, NEW, OLD)):
            tag = name[0].lower()
            tmux('kill-server')
            install(first)
            s1, m1 = create(f'{tag}1')
            # The server as only `first` configured it: the other version's create and
            # attach re-source the managed config into it, which must change nothing.
            cfg, started, by = config(), snapshot(), configured_by()
            install(second)
            s2, m2 = create(f'{tag}2')
            before = snapshot()
            assert before['server'] and set(before['panes']) >= {s1, s2}, before
            listed = cli(viewer_bin, 'sessions', '--device', 'remote')
            assert listed.returncode == 0, listed.stderr[-400:]
            ids = sorted(s['id'] for s in json.loads(listed.stdout))
            assert {s1, s2} <= set(ids), ids
            shown = preview(viewer_bin, [m1, m2])
            detached = {}
            for sid in (s1, s2):
                detached[sid] = attach(viewer_bin, sid)
                assert detached[sid] == 0, (sid, detached[sid])
            same_config = config() == cfg
            after = snapshot()
            assert after == before and after['server'] == started['server'], (started, before, after)
            assert after['panes'][s1] == started['panes'][s1], (started, after)
            # (The marker proves the second version did re-source its config.)
            assert (by, configured_by()) == (version(first), version(second)), (by, configured_by())
            assert same_config, 'the other version changed the running server\'s options or keys'
            evidence['scenarios'][name] = dict(
                viewer=version(viewer_bin), helper=version(second),
                server_started_by=version(first),
                created={s1[:12] + '…': f'{tag}1 by {version(first)}',
                         s2[:12] + '…': f'{tag}2 by {version(second)}'},
                listed=len(ids), previews=shown, attach_exit=list(detached.values()),
                before=dict(server=before['server'],
                            panes={k[:12] + '…': v for k, v in before['panes'].items()}),
                after=dict(server=after['server'],
                           panes={k[:12] + '…': v for k, v in after['panes'].items()}),
                configured_by=[by, configured_by()],
                config_before=digest(cfg), config_after=digest(config()),
                unchanged=after == before, config_unchanged=same_config)
        evidence['result'] = 'PASS'
        print(json.dumps(evidence))
    finally:
        if viewer_pty:
            viewer_pty.close()
        tmux('kill-server')
        subprocess.run(['tmux', '-S', str(viewer / 'state/cx/managed.sock'), 'kill-server'],
                       env=view_env, capture_output=True)
        for master in (viewer / 'state/cx/ssh').glob('*') if (viewer / 'state/cx/ssh').exists() else []:
            if master.is_socket():
                subprocess.run([ssh, '-F', '/dev/null', '-S', str(master), '-O', 'exit', 'cx'],
                               capture_output=True, timeout=5)
        if daemon:
            daemon.terminate()
            daemon.wait(timeout=5)
        end = time.monotonic() + 3
        while owned() and time.monotonic() < end:
            time.sleep(.1)
        left = owned()
        for pid in left:
            try:
                os.kill(pid, 9)
            except OSError:
                pass
        tmp.cleanup()
        if left:
            print(json.dumps(dict(cleanup_killed=len(left))), file=sys.stderr)


if __name__ == '__main__':
    main()
