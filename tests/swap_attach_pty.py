#!/usr/bin/env python3
"""A running cx keeps working after its binary is atomically replaced (v0.2.2).

usage: swap_attach_pty.py BINARY          the local viewer (needs pyte)
       swap_attach_pty.py BINARY --ssh    a remote helper over a loopback sshd

An update writes `cx.new` and renames it over `~/.local/bin/cx`; processes already
running keep the old (now deleted) inode, and /proc/self/exe reads
"<path> (deleted)". Before the fix cx handed that path to tmux (the attach check
`if-shell` predicate), so:
  local   a viewer started before the swap could not attach a session: the
          predicate named a file that does not exist and the attach was refused.
  --ssh   a device's helper channel started before the swap refused typed input to
          a session pane (the same predicate, on the device).
Local: a private HOME whose `.local/bin/cx` is a copy of BINARY; a session is created,
the viewer starts, the file is replaced, and the same viewer attaches the session
(Tab, Enter: a tmux client appears) and detaches with Ctrl+\\ back to the hub.
--ssh: a private sshd on 127.0.0.1 and a private device HOME; `cx helper` is started
over ssh, the device's binary is replaced, and the same helper process delivers
SessionPaneInput to a session pane (the typed command runs). Nothing outside the
private directories is touched. Prints one JSON line.
"""
import json, os, pathlib, pwd, shutil, socket, subprocess, sys, tempfile, time

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))

BINARY = str(pathlib.Path(sys.argv[1]).resolve())


def private_env(home):
    env = dict(os.environ, HOME=str(home), XDG_STATE_HOME=str(home / 'state'),
               TMUX_TMPDIR=str(home / 'tmux'), SHELL='/bin/bash', TERM='xterm-256color',
               HTTPS_PROXY='http://127.0.0.1:9', HISTFILE='/dev/null', CX_THEME='dark')
    for key in ('TMUX', 'TMUX_PANE', 'NO_COLOR', 'CX_ASCII', 'SSH_AUTH_SOCK', 'SSH_AGENT_PID'):
        env.pop(key, None)
    return env


def make_home(home):
    for sub in ('.local/bin', 'state/cx', 'tmux', '.ssh'):
        (home / sub).mkdir(parents=True, mode=0o700, exist_ok=True)
    install(home)


def install(home):
    """The fleet update: write cx.new, fsync, rename over cx (running processes keep theirs)."""
    target = home / '.local/bin/cx'
    staged = target.with_name('cx.new')
    shutil.copyfile(BINARY, staged)
    staged.chmod(0o755)
    with open(staged, 'rb') as f:
        os.fsync(f.fileno())
    os.rename(staged, target)


def tmux(env, home, *args):
    out = subprocess.run(['tmux', '-S', str(home / 'state/cx/managed.sock'), *args], env=env,
                         capture_output=True, text=True, timeout=10)
    return out.stdout if out.returncode == 0 else None


def create(env, home, name):
    out = subprocess.run([str(home / '.local/bin/cx'), 'new', '--directory', str(home),
                          '--name', name, '--key', name], env=env, capture_output=True,
                         text=True, timeout=30)
    assert out.returncode == 0, out.stderr[-400:]


def owned(roots):
    found = set()
    marks = [f'HOME={r}'.encode() for r in roots]
    for entry in pathlib.Path('/proc').iterdir():
        if entry.name.isdigit():
            try:
                data = (entry / 'environ').read_bytes() + (entry / 'cmdline').read_bytes()
            except OSError:
                continue
            if any(m in data for m in marks):
                found.add(int(entry.name))
    found.discard(os.getpid())
    return found


def reap(roots):
    end = time.monotonic() + 3
    while owned(roots) and time.monotonic() < end:
        time.sleep(.1)
    for pid in owned(roots):
        try:
            os.kill(pid, 9)
        except OSError:
            pass


def deleted_exe(pid):
    return os.readlink(f'/proc/{pid}/exe').endswith(' (deleted)')


def local():
    import pty_lib
    tmp = tempfile.TemporaryDirectory(prefix='cx-swap-')
    home = pathlib.Path(tmp.name)
    make_home(home)
    env = private_env(home)
    installed = str(home / '.local/bin/cx')
    viewer = None
    try:
        create(env, home, 'swap-a')
        viewer = pty_lib.Pty([installed], env, home, 160, 45)
        text = lambda: '\n'.join(viewer.screen.display)  # noqa: E731

        def settle(until, what, limit=10.0):
            end = time.monotonic() + limit
            while not until():
                assert time.monotonic() < end, f'{what}:\n{text()}'
                assert viewer.proc.poll() is None, f'viewer exited {viewer.proc.returncode}:\n{text()}'
                viewer.read(.03)
            viewer.read(.05)

        def clients():
            out = tmux(env, home, 'list-clients', '-F', '#{client_control_mode}') or ''
            return out.split().count('0')
        settle(lambda: 'swap-a' in text(), 'the hub never showed the session', 15)
        settle(lambda: 'idle' in text(), 'the shell never went idle')
        install(home)
        assert deleted_exe(viewer.proc.pid), 'the viewer does not run a replaced binary'
        viewer.write(b'\t')
        settle(lambda: '▌SESSIONS' in text(), 'Tab did not focus the tiles')
        viewer.write(b'\r')
        end = time.monotonic() + 8
        while not clients():
            assert time.monotonic() < end, f'the viewer could not attach after the swap:\n{text()}'
            assert viewer.proc.poll() is None, f'viewer exited:\n{text()}'
            viewer.read(.05)
        # (Ctrl+\ once tmux drew its status line: before raw mode it is SIGQUIT.)
        settle(lambda: 'detach' in viewer.screen.display[-1], 'the attached client drew nothing')
        viewer.write(b'\x1c')
        end = time.monotonic() + 5
        while clients():
            assert time.monotonic() < end, 'Ctrl+\\ did not detach'
            viewer.read(.05)
        viewer.screen.reset()
        settle(lambda: 'swap-a' in text(), 'detach did not return to the hub')
        return dict(viewer_exe_deleted=True, attached=True, detached=True, back_in_hub=True)
    finally:
        if viewer:
            viewer.close()
        tmux(env, home, 'kill-server')
        reap([home])
        tmp.cleanup()


class Helper:
    """One `cx helper` channel: CX1-framed requests on stdin, responses on stdout."""

    def __init__(self, argv):
        self.proc = subprocess.Popen(argv, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                     stderr=subprocess.DEVNULL)
        self.next = 0

    def request(self, op, args=None):
        self.next += 1
        body = dict(version=1, id=f'swap-{self.next}', op=dict(op=op))
        if args is not None:
            body['op']['args'] = args
        payload = json.dumps(body).encode()
        self.proc.stdin.write(b'CX1 %d\n' % len(payload) + payload)
        self.proc.stdin.flush()
        while True:
            header = self.proc.stdout.readline()
            assert header, 'the helper channel closed'
            if not header.startswith(b'CX1 '):
                continue  # bounded startup noise
            response = json.loads(self.proc.stdout.read(int(header.split()[1])))
            if response['id'] == body['id']:
                return response


def remote():
    account = pwd.getpwuid(os.getuid()).pw_name
    ssh = shutil.which('ssh')
    sshd = shutil.which('sshd') or '/usr/sbin/sshd'
    # Short root: sun_path.
    tmp = tempfile.TemporaryDirectory(prefix='cx-swr-', dir='/tmp')
    root = pathlib.Path(tmp.name)
    root.chmod(0o700)
    device = root / 'd'
    make_home(device)
    env = private_env(device)
    daemon = helper = None
    try:
        with socket.socket() as s:
            s.bind(('127.0.0.1', 0))
            port = s.getsockname()[1]
        for name in ('host', 'client'):
            subprocess.run(['ssh-keygen', '-q', '-t', 'ed25519', '-N', '', '-f', str(root / name)],
                           check=True, timeout=10)
        shutil.copyfile(root / 'client.pub', device / '.ssh/authorized_keys')
        host_key = ' '.join((root / 'host.pub').read_text().split()[:2])
        (root / 'known_hosts').write_text(f'[127.0.0.1]:{port} {host_key}\n')
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
        create(env, device, 'swap-r')
        remote_env = ' '.join(f'{k}={env[k]}' for k in ('HOME', 'XDG_STATE_HOME', 'TMUX_TMPDIR', 'SHELL'))
        helper = Helper([ssh, '-F', '/dev/null', '-p', str(port), '-i', str(root / 'client'),
                         '-o', 'IdentitiesOnly=yes', '-o', 'BatchMode=yes', '-o', 'ForwardAgent=no',
                         '-o', f'UserKnownHostsFile={root / "known_hosts"}',
                         '-o', 'StrictHostKeyChecking=yes', f'{account}@127.0.0.1',
                         f'exec env {remote_env} {device}/.local/bin/cx helper'])
        listed = helper.request('sessions')
        assert not listed['error'], listed['error']
        [session] = [s for s in listed['result'] if s['name'] == 'swap-r']
        terminal = session['terminal']['session_id']
        pane = tmux(env, device, 'list-panes', '-t', terminal, '-F', '#{pane_id}').split()[0]
        [pid] = [p for p in owned([device])
                 if b'helper' in pathlib.Path(f'/proc/{p}/cmdline').read_bytes()
                 and pathlib.Path(f'/proc/{p}/exe').resolve().name.startswith('cx')]
        install(device)
        assert deleted_exe(pid), 'the helper does not run a replaced binary'
        mark = 'swap-remote-mark'
        typed = f"printf '%s\\n' {mark.upper()} | tr A-Z a-z\r".encode()
        sent = helper.request('session_pane_input',
                              dict(session=session, pane_id=pane, hex=typed.hex()))
        assert not sent['error'], f'the swapped helper refused input: {sent["error"]}'
        assert sent['result'] == dict(delivered=True), sent
        end = time.monotonic() + 10
        while mark not in (tmux(env, device, 'capture-pane', '-p', '-t', pane) or ''):
            assert time.monotonic() < end, 'the typed command never ran'
            time.sleep(.1)
        assert helper.proc.poll() is None and deleted_exe(pid), 'the helper was restarted'
        return dict(helper_exe_deleted=True, input_delivered=True, command_ran=True)
    finally:
        if helper:
            helper.proc.stdin.close()
            try:
                helper.proc.wait(timeout=5)
            except subprocess.TimeoutExpired:
                helper.proc.kill()
        tmux(env, device, 'kill-server')
        if daemon:
            daemon.terminate()
            daemon.wait(timeout=5)
        reap([device])
        tmp.cleanup()


if __name__ == '__main__':
    checks = remote() if '--ssh' in sys.argv[2:] else local()
    print(json.dumps(dict(result='PASS', **checks)))
