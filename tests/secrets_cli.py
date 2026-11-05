#!/usr/bin/env python3
"""`cx secrets` keeps values out of stdout/stderr, argv and files, offline in a temp HOME.

Uses the age-file backend only (CX_SECRETS_BACKEND=file, no session bus), so the user's keyring is never
touched. Checks: set from stdin, list/--json carry metadata only, get --fd to a pipe (a terminal is refused),
get --exec puts the value in the child's env and in no process's /proc/PID/cmdline, migrate imports the
plaintext `cx mesh` token and deletes it only with --delete-plaintext, `cx mesh` hands the stored token to
python3 as $CLOUDFLARE_API_TOKEN (an explicit one wins) for API subcommands only, never for join/leave (whose
fetched script runs without it, end to end) nor with --config (that mesh keeps its own credentials), a stored but
unreadable token stops `cx mesh` instead of letting mesh.py prompt and save it in plaintext, Ctrl-C and
SIGINT/SIGTERM/SIGHUP/SIGQUIT at the prompt restore the terminal, Ctrl-D and Backspace edit like a line, rm, and
0600/0700 modes.
"""
import hashlib, json, os, pathlib, stat, subprocess, sys, tempfile

binary = str(pathlib.Path(sys.argv[1]).resolve())
CANARY = 'cx-cli-canary-5d2b81c4'
CANARY2 = 'cx-cli-canary-mesh-0e77a1'
seen = []  # every stdout/stderr byte cx produced


def sha(s):
    return hashlib.sha256(s.encode()).hexdigest()


with tempfile.TemporaryDirectory(prefix='cx-secrets-') as tmp:
    root = pathlib.Path(tmp); root.chmod(0o700)
    env = {k: v for k, v in os.environ.items() if not k.startswith(('DBUS_', 'CLOUDFLARE_', 'XDG_', 'CX_'))}
    env.update(HOME=str(root), XDG_STATE_HOME=str(root / 'state'), XDG_CONFIG_HOME=str(root / 'config'),
               XDG_CACHE_HOME=str(root / 'cache'), XDG_RUNTIME_DIR=str(root / 'run'), CX_SECRETS_BACKEND='file')
    (root / 'run').mkdir(mode=0o700)
    state = root / 'state/cx'

    def cx(*args, input=None, ok=True, extra=None, **kw):
        r = subprocess.run([binary, *args], env=dict(env, **(extra or {})), input=input, capture_output=True,
                           timeout=30, **kw)
        seen.extend([r.stdout, r.stderr])
        if ok:
            assert r.returncode == 0, (args, r.returncode, r.stderr.decode(errors='replace')[-2000:])
        return r

    r = cx('secrets', 'list')
    assert b'no secrets' in r.stdout
    cx('secrets', 'set', 'robot.wifi', input=(CANARY + '\n').encode())
    r = cx('secrets', 'list', '--json')
    meta = json.loads(r.stdout)
    assert [m['name'] for m in meta] == ['robot.wifi'], meta
    assert meta[0]['backend'] == 'file' and meta[0]['length'] == len(CANARY), meta
    assert 'value' not in meta[0]
    r = cx('secrets', 'list')
    assert f'({len(CANARY)})'.encode() in r.stdout
    r = cx('secrets', 'set', 'robot.wifi', '--kind', 'generic', input=b'x', ok=False)
    assert r.returncode != 0, 'there is no --kind'
    r = cx('secrets', 'set', 'robot.x', input=b'x', ok=False, extra=dict(CX_SECRETS_BACKEND='keyring'))
    assert r.returncode != 0 and b'no OS keyring' in r.stderr, r.stderr
    r = cx('secrets', 'list', ok=False, extra=dict(CX_SECRETS_BACKEND='bogus'))
    assert r.returncode != 0 and b'auto, file or keyring' in r.stderr, r.stderr

    # On a terminal, set prompts with echo off: the typed value never comes back on the screen.
    import pty, select, time
    pid, master = pty.fork()
    if pid == 0:
        os.execve(binary, [binary, 'secrets', 'set', 'robot.typed'], env)
    screen = b''
    deadline = time.monotonic() + 10
    while b'not echoed' not in screen and time.monotonic() < deadline:
        if select.select([master], [], [], 0.2)[0]:
            screen += os.read(master, 4096)
    assert b'not echoed' in screen, screen
    os.write(master, (CANARY + '\n').encode())
    while time.monotonic() < deadline:
        if select.select([master], [], [], 0.2)[0]:
            try:
                chunk = os.read(master, 4096)
            except OSError:
                break
            if not chunk:
                break
            screen += chunk
    assert os.waitstatus_to_exitcode(os.waitpid(pid, 0)[1]) == 0, screen
    os.close(master)
    seen.append(screen)
    assert b'stored robot.typed' in screen, screen
    cx('secrets', 'rm', 'robot.typed')

    import signal

    def typed(name, keys):
        """Type `keys` at the no-echo prompt of `set name` and return the stored value."""
        pid, master = pty.fork()
        if pid == 0:
            os.execve(binary, [binary, 'secrets', 'set', name], env)
        screen, deadline = b'', time.monotonic() + 10
        while b'not echoed' not in screen and time.monotonic() < deadline:
            if select.select([master], [], [], 0.2)[0]:
                screen += os.read(master, 4096)
        assert b'not echoed' in screen, screen
        os.write(master, keys)
        status = None
        while status is None and time.monotonic() < deadline:
            done, st = os.waitpid(pid, os.WNOHANG)
            if done:
                status = st
            elif select.select([master], [], [], 0.2)[0]:
                try:
                    os.read(master, 4096)
                except OSError:
                    time.sleep(0.05)
        if status is None:
            os.kill(pid, signal.SIGKILL)
            os.waitpid(pid, 0)
            raise AssertionError(f'prompt for {name} did not end after {keys!r}')
        assert os.waitstatus_to_exitcode(status) == 0, status
        os.close(master)
        rd, wr = os.pipe()
        cx('secrets', 'get', name, '--fd', str(wr), pass_fds=(wr,))
        os.close(wr)
        with os.fdopen(rd, 'rb') as f:
            value = f.read()
        cx('secrets', 'rm', name)
        return value

    # Ctrl-D ends the value like Enter (it is never stored as a byte); Backspace erases a whole character.
    assert typed('robot.eof', b'tok123\x04') == b'tok123'
    assert typed('robot.erase', 'xé\x7fy\n'.encode()) == b'xy'

    # Ctrl-C at the prompt cancels it (nothing stored) and SIGTERM kills it, both with echo and line mode restored.
    import signal, termios

    def prompt(then):
        pid, master = pty.fork()
        if pid == 0:
            os.execve(binary, [binary, 'secrets', 'set', 'robot.cancel'], env)
        screen, deadline = b'', time.monotonic() + 10
        while b'not echoed' not in screen and time.monotonic() < deadline:
            if select.select([master], [], [], 0.2)[0]:
                screen += os.read(master, 4096)
        assert b'not echoed' in screen, screen
        lflag = termios.tcgetattr(master)[3]
        assert not lflag & termios.ECHO, 'echo is off while prompting'
        os.write(master, CANARY[:8].encode())
        then(pid, master)
        while time.monotonic() < deadline:
            if select.select([master], [], [], 0.2)[0]:
                try:
                    chunk = os.read(master, 4096)
                except OSError:
                    break
                if not chunk:
                    break
                screen += chunk
            elif os.waitpid(pid, os.WNOHANG)[0]:
                pid = 0
                break
        lflag = termios.tcgetattr(master)[3]
        status = os.waitpid(pid, 0)[1] if pid else None
        os.close(master)
        seen.append(screen)
        assert lflag & termios.ECHO and lflag & termios.ICANON and lflag & termios.ISIG, 'terminal mode restored'
        return screen, status

    screen, _ = prompt(lambda pid, master: (time.sleep(0.2), os.write(master, b'\x03')))
    assert b'cancelled' in screen, screen
    for sig in (signal.SIGTERM, signal.SIGINT, signal.SIGHUP, signal.SIGQUIT):
        screen, status = prompt(lambda pid, master: (time.sleep(0.2), os.kill(pid, sig)))
        assert status is None or os.WIFSIGNALED(status) and os.WTERMSIG(status) == sig, (sig, status)
    assert 'robot.cancel' not in [m['name'] for m in json.loads(cx('secrets', 'list', '--json').stdout)]

    # --fd: a pipe gets exactly the value; a terminal is refused.
    rd, wr = os.pipe()
    cx('secrets', 'get', 'robot.wifi', '--fd', str(wr), pass_fds=(wr,))
    os.close(wr)
    with os.fdopen(rd, 'rb') as f:
        assert f.read() == CANARY.encode()
    master, slave = os.openpty()
    r = cx('secrets', 'get', 'robot.wifi', '--fd', str(slave), pass_fds=(slave,), ok=False)
    os.close(slave)
    os.set_blocking(master, False)
    try:
        leaked = os.read(master, 4096)
    except (BlockingIOError, OSError):
        leaked = b''
    os.close(master)
    assert r.returncode != 0 and b'terminal' in r.stderr and CANARY.encode() not in leaked
    r = cx('secrets', 'get', 'robot.wifi', 'robot.wifi', '--fd', '1', ok=False)
    assert r.returncode != 0 and b'exactly one' in r.stderr

    # --exec: the child sees the value in its env; no process's argv carries it.
    child = (
        'import hashlib,os,sys\n'
        'c=open(sys.argv[1],"rb").read()\n'
        'hits=[]\n'
        'for p in os.listdir("/proc"):\n'
        '    if p.isdigit():\n'
        '        try: a=open(f"/proc/{p}/cmdline","rb").read()\n'
        '        except OSError: continue\n'
        '        if c in a: hits.append(p)\n'
        'v=os.environ.get("WIFI","")\n'
        'print(hashlib.sha256(v.encode()).hexdigest(), os.environ.get("ROBOT_WIFI") is None, len(hits))\n'
    )
    needle = root / 'needle'; needle.write_text(CANARY); needle.chmod(0o600)
    r = cx('secrets', 'get', 'WIFI=robot.wifi', '--exec', '--', sys.executable, '-c', child, str(needle))
    out = r.stdout.decode().split()
    assert out[0] == sha(CANARY) and out[1] == 'True', out
    assert out[2] == '0', f'canary in argv of {out[2]} processes'
    cx('secrets', 'get', 'robot.wifi', '--exec', '--', 'sh', '-c', 'test "$ROBOT_WIFI" = "$(cat "$1")"', 'x', str(needle))
    r = cx('secrets', 'get', 'missing', '--exec', '--', 'true', ok=False)
    assert r.returncode != 0

    # migrate: imports the plaintext mesh token, deletes it only on request.
    creds = root / 'config/cx/mesh/credentials'
    creds.parent.mkdir(parents=True); creds.write_text(CANARY2 + '\n'); creds.chmod(0o600)
    r = cx('secrets', 'migrate')
    assert creds.exists() and b'--delete-plaintext' in r.stdout
    assert json.loads(cx('secrets', 'list', '--json').stdout)[0]['name'] == 'cloudflare.api_token'
    cx('secrets', 'migrate', '--delete-plaintext')
    assert not creds.exists()
    r = cx('secrets', 'migrate')
    assert b'nothing to migrate' in r.stdout
    # A different plaintext token is refused: the stored one is kept and so is the file.
    creds.write_text('cx-cli-other-token\n'); creds.chmod(0o600)
    r = cx('secrets', 'migrate', '--delete-plaintext', ok=False)
    assert r.returncode != 0 and b'different token' in r.stderr and creds.exists(), r.stderr
    rd, wr = os.pipe()
    cx('secrets', 'get', 'cloudflare.api_token', '--fd', str(wr), pass_fds=(wr,))
    os.close(wr)
    with os.fdopen(rd, 'rb') as f:
        assert f.read() == CANARY2.encode()
    creds.unlink()

    # cx mesh: the stored token reaches python3 through the environment only.
    bin_dir = root / 'bin'; bin_dir.mkdir()
    fake = bin_dir / 'python3'
    fake.write_text('#!/bin/sh\nprintf %s "$CLOUDFLARE_API_TOKEN" | sha256sum | cut -c1-64 > "$CX_FAKE_OUT"\n'
                    'printf "%s\\n" "$@" > "$CX_FAKE_OUT.argv"\n')
    fake.chmod(0o700)
    out_file = root / 'mesh-out'
    path = f'{bin_dir}:{env["PATH"]}'
    cx('mesh', 'devices', extra=dict(PATH=path, CX_FAKE_OUT=str(out_file)))
    assert out_file.read_text().strip() == sha(CANARY2), 'stored token not injected'
    assert CANARY2 not in (root / 'mesh-out.argv').read_text()
    cx('mesh', 'devices', extra=dict(PATH=path, CX_FAKE_OUT=str(out_file), CLOUDFLARE_API_TOKEN='explicit'))
    assert out_file.read_text().strip() == sha('explicit'), 'explicit env must win'
    # Only API subcommands get it: join/leave run a fetched script and never see it; neither do argvs whose
    # subcommand cx cannot read with certainty.
    for args in (['join', 'https://mesh.example.org/abc'], ['leave'], ['--conf', 'devices', 'join', 'x'],
                 ['--config', 'devices', 'join', 'x'], ['-h'], []):
        out_file.write_text('unset')
        cx('mesh', *args, extra=dict(PATH=path, CX_FAKE_OUT=str(out_file)))
        assert out_file.read_text().strip() == sha(''), f'token passed on to cx mesh {args}'
    # Another --config keeps the credentials file next to it: the stored (default mesh) token is not passed.
    for args, want in ((['--json', 'audit'], CANARY2), (['--config=c.toml', 'invite'], ''),
                       (['--config', 'c.toml', 'devices'], '')):
        cx('mesh', *args, extra=dict(PATH=path, CX_FAKE_OUT=str(out_file)))
        assert out_file.read_text().strip() == sha(want), args
    for args, want in ((['list'], CANARY2), (['lend', 'evan'], CANARY2), (['revoke', 'evan'], CANARY2)):
        cx('claude', *args, extra=dict(PATH=path, CX_FAKE_OUT=str(out_file)))
        assert out_file.read_text().strip() == sha(want), args

    # A stored token this session cannot read (keyring-held, no bus) stops cx mesh before mesh.py runs, so the util
    # cannot prompt and write the plaintext credentials file again.
    index_path = state / 'secrets.json'
    original = index_path.read_bytes()
    index = json.loads(original)
    index['secrets']['cloudflare.api_token']['backend'] = 'keyring'
    index_path.write_text(json.dumps(index))
    out_file.write_text('unset')
    r = cx('mesh', 'devices', ok=False, extra=dict(PATH=path, CX_FAKE_OUT=str(out_file)))
    assert r.returncode != 0 and b'CLOUDFLARE_API_TOKEN' in r.stderr, r.stderr
    assert out_file.read_text() == 'unset', 'mesh.py ran without the stored token'
    assert not creds.exists()
    index_path.write_bytes(original)

    # End to end with the real mesh.py: the script a join URL serves runs without the token, stored or explicit.
    curl_dir = root / 'curl-bin'; curl_dir.mkdir()
    (curl_dir / 'curl').write_text('#!/bin/sh\nprintf \'%s\\n\' \'printf %s "${CLOUDFLARE_API_TOKEN-unset}" > "$CX_FAKE_OUT"\'\n')
    (curl_dir / 'curl').chmod(0o700)
    curl_path = f'{curl_dir}:{env["PATH"]}'
    for extra in ({}, {'CLOUDFLARE_API_TOKEN': CANARY2}):
        out_file.write_text('')
        cx('mesh', 'join', 'https://mesh.example.org/abc', extra=dict(PATH=curl_path, CX_FAKE_OUT=str(out_file), **extra))
        assert out_file.read_text() == 'unset', f'join script saw the token ({sorted(extra)})'

    # Nothing on disk or in any output carries a value in clear; modes stay private.
    for p in state.iterdir():
        data = p.read_bytes()
        assert CANARY.encode() not in data and CANARY2.encode() not in data, p
        if p.is_file():
            assert stat.S_IMODE(p.stat().st_mode) == 0o600, (p, oct(p.stat().st_mode))
    assert stat.S_IMODE(state.stat().st_mode) == 0o700
    for f in ('secrets.age', 'secrets.key', 'secrets.json'):
        assert (state / f).exists(), f

    cx('secrets', 'rm', 'robot.wifi')
    assert [m['name'] for m in json.loads(cx('secrets', 'list', '--json').stdout)] == ['cloudflare.api_token']
    r = cx('secrets', 'rm', 'robot.wifi', ok=False)
    assert r.returncode != 0

    for blob in seen:
        assert CANARY.encode() not in blob and CANARY2.encode() not in blob, blob[-500:]
        assert CANARY[:16].encode() not in blob
print('secrets CLI: values stayed out of output, argv and disk')
