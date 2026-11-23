#!/usr/bin/env python3
"""Move terminals the way a real install does: a setcap relay run by an unprivileged user.

A relay that gains cap_sys_ptrace from a file capability (or that sudo ran and that
then dropped to the user) is non-dumpable, so the user can no longer read its
/proc/<pid>/fd. The move must still be seen (from the holder and the leader), the
record finalized, and the frozen source window killed before its owner can close it:
a foot-like emulator here ignores SIGHUP and, once its window is empty, "is closed by
the user" two seconds later, signalling the moved terminal's process group HUP, TERM
and then KILL, as foot does. Also covers healing a record a failed check left stale
and stopping an outside terminal's agent (StopExternal).

The replacement window (section 6): after a verified move the same emulator is
reopened running `cx attach <id>`. The "emulator" reopened here is never a real
terminal: once each fake foot has started, its executable is replaced by a recorder
script (as a package update would) that writes its argv, environment, cwd, session
and stdio to a file and exits. The fixture then runs the recorded command on a PTY
of its own to prove that it attaches. Display variables are fixture values; no real
display, user bus or systemd-run is reachable (a fake systemd-run is first on PATH).

Runs in a user namespace: ns uid 0 is the caller (sets up the capability), ns uid 1
(a subuid) plays the user. No real terminal, agent or tmux server is touched.
"""
import fcntl
import json
import os
import pty
import select
import struct
import termios
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import tempfile
import time

source_binary = str(Path(sys.argv[1]).resolve())
here = Path(__file__).resolve().parent


def _subid(path, name):
    try:
        for line in Path(path).read_text().splitlines():
            who, base, count = line.split(':')
            if who in (name, str(os.getuid())) and int(count) >= 1:
                return int(base)
    except OSError:
        pass
    return None


if os.environ.get('CX_ADOPT_NS') != '1':
    import pwd
    name = pwd.getpwuid(os.getuid()).pw_name
    sub_u, sub_g = _subid('/etc/subuid', name), _subid('/etc/subgid', name)
    if sub_u is None or sub_g is None or not shutil.which('newuidmap') or not shutil.which('setcap'):
        print(json.dumps(dict(result='SKIP', reason='no subuid range, newuidmap or setcap')))
        sys.exit(0)
    maps = ['--map-users', f'0:{os.getuid()}:1', '--map-users', f'1:{sub_u}:1',
            '--map-groups', f'0:{os.getgid()}:1', '--map-groups', f'1:{sub_g}:1']
    if subprocess.run(['unshare', '-U', *maps, 'true'], capture_output=True).returncode != 0:
        print(json.dumps(dict(result='SKIP', reason='user namespace mapping unavailable')))
        sys.exit(0)
    os.environ['CX_ADOPT_NS'] = '1'
    # Own pid namespace and /proc, and tests/isolate.sh's marker in a private /dev/shm
    # (a tmpfs of this namespace, so a file capability set there is honoured).
    os.execvp('unshare', ['unshare', '-U', *maps, '--pid', '--fork', '--mount', '--mount-proc',
                          'sh', '-c', 'mount -t tmpfs tmpfs /dev/shm && : >/dev/shm/cx-isolated && exec "$@"',
                          'sh', sys.executable, *sys.argv])

EMULATOR = (here / 'adopt_terminal_live.py').read_text().split("EMULATOR = r'''", 1)[1].split("'''", 1)[0]
# foot: SIGHUP is ignored; an emptied window stays up until the user closes it, and
# closing it signals the child's process group HUP, then TERM, then KILL.
FOOT = EMULATOR.replace("import fcntl,", "import signal; signal.signal(signal.SIGHUP, signal.SIG_IGN)\nimport fcntl,", 1).replace(
    "os.close(master)\nlog.write('closed\\n')",
    "log.write('frozen\\n')\ntime.sleep(2)\n"
    "for s in (signal.SIGHUP, signal.SIGTERM, signal.SIGKILL):\n"
    "    log.write(f'close {s}\\n')\n"
    "    try: os.killpg(os.getpgid(pid), s)\n"
    "    except OSError: pass\n"
    "    time.sleep(1)\n"
    "os.close(master)\nlog.write('closed\\n')", 1)
assert FOOT.count('killpg') == 1 and 'SIG_IGN' in FOOT
USER = ['setpriv', '--reuid=1', '--regid=1', '--clear-groups', '--']


def stat(pid):
    f = Path(f'/proc/{pid}/stat').read_text().rsplit(')', 1)[1].split()
    return dict(state=f[0], ppid=int(f[1]), pgrp=int(f[2]), sid=int(f[3]), start=f[19])


def alive(pid):
    try:
        return stat(pid)['state'] != 'Z'
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


# No real terminal emulator, editor or systemd-run can be started from here: in this
# mount namespace each one on PATH is covered by /dev/null (not executable), so a
# replacement window only ever finds the fixture's recorders.
MASKED = ['foot', 'kitty', 'alacritty', 'ghostty', 'xterm', 'uxterm', 'urxvt', 'rxvt', 'st', 'konsole',
          'tilix', 'wezterm', 'wezterm-gui', 'xfce4-terminal', 'terminator', 'emacs', 'systemd-run']
for folder in dict.fromkeys(os.environ['PATH'].split(':')):
    for program in MASKED:
        target = os.path.realpath(os.path.join(folder, program))
        if os.path.isfile(target) and os.access(target, os.X_OK):
            subprocess.run(['mount', '--bind', '/dev/null', target], check=True)
assert not [p for p in MASKED if shutil.which(p)], [shutil.which(p) for p in MASKED]

checks = []
home = Path(tempfile.mkdtemp(prefix='cx-setcap-', dir='/dev/shm'))
bins = home / 'bin'
bins.mkdir()
root = home / 'state/cx'
root.mkdir(parents=True, mode=0o700)
socket = root / 'managed.sock'
subprocess.run(['cc', '-O1', '-pthread', '-o', str(bins / 'claude'), str(here / 'adopt_churn.c')], check=True)
stub = os.path.realpath(bins / 'claude')
# A stand-in for an updated emulator binary and for systemd-run (see section 6).
RECORDER = """#!/bin/sh
stdio="$(readlink /proc/$$/fd/0) $(readlink /proc/$$/fd/1) $(readlink /proc/$$/fd/2)"
{ echo "pid $$"; echo "group $(cut -d' ' -f5,6 /proc/$$/stat)"; echo "cwd $(pwd -P)"
  echo "stdio $stdio"; for a in "$0" "$@"; do echo "arg $a"; done; env | sed 's/^/env /'
} >"$0.record.tmp" && mv "$0.record.tmp" "$0.record"
"""
SYSTEMD_RUN = """#!/bin/sh
for a in "$@"; do echo "$a"; done >"$0.record"
while [ $# -gt 0 ] && [ "$1" != -- ]; do shift; done
shift
exec "$@"
"""
(bins / 'systemd-run').write_text(SYSTEMD_RUN)
os.chmod(bins / 'systemd-run', 0o755)
source = os.environ.get('CX_ADOPT_REPTYR')
relay = str(home / 'cx-reptyr')
if source:
    shutil.copyfile(source, relay)
else:
    subprocess.run(['sh', str(here.parent / 'scripts/build-portable-reptyr.sh'), relay],
                   env=dict(os.environ, CX_REPTYR_DYNAMIC='1'), check=True, stdout=subprocess.DEVNULL)
os.chmod(relay, 0o755)
# The user (ns uid 1) cannot reach the caller's home: run a copy, as an install would
# (the attach predicate execs cx again as the user).
binary = str(home / 'cx')
shutil.copyfile(source_binary, binary)
os.chmod(binary, 0o755)
subprocess.run(['setcap', 'cap_sys_ptrace+ep', relay], check=True)
for path in [home, *home.rglob('*')]:
    if path != Path(relay):
        os.chown(path, 1, 1)
for run in ('run', 'run-holder'):
    (home / run).mkdir(mode=0o700)
    os.chown(home / run, 1, 1)
env = dict(os.environ, HOME=str(home), XDG_STATE_HOME=str(home / 'state'), TMUX_TMPDIR=str(home),
           SHELL='/bin/bash', TERM='xterm-256color', PATH=f'{bins}:{os.environ["PATH"]}',
           CX_ADOPT_REPTYR=relay, XDG_RUNTIME_DIR=str(home / 'run'))
for key in ('TMUX', 'TMUX_PANE', 'DBUS_SESSION_BUS_ADDRESS', 'WAYLAND_DISPLAY', 'DISPLAY', 'XAUTHORITY'):
    env.pop(key, None)
# What a terminal emulator on a desktop has (fixture values only), plus things that
# must never reach a replacement window.
HOLDER_ENV = dict(WAYLAND_DISPLAY='wayland-cxtest', DISPLAY=':97', XAUTHORITY=str(home / 'xauth'),
                  XDG_RUNTIME_DIR=str(home / 'run-holder'),
                  DBUS_SESSION_BUS_ADDRESS=f'unix:path={home}/run-holder/bus',
                  CX_FIXTURE_SECRET='sk-never-copied', TERM='foot', SSH_AUTH_SOCK='/nonexistent')
WINDOW_KEYS = {'PATH', 'HOME', 'USER', 'LOGNAME', 'LANG', 'LANGUAGE', 'XDG_STATE_HOME', 'XDG_CONFIG_HOME',
               'WAYLAND_DISPLAY', 'DISPLAY', 'XAUTHORITY', 'XDG_RUNTIME_DIR', 'DBUS_SESSION_BUS_ADDRESS'}
boot = Path('/proc/sys/kernel/random/boot_id').read_text().strip()
emulators = []


def tmux(*args, check=True):
    return subprocess.run([*USER, 'tmux', '-f', '/dev/null', '-S', str(socket), *args], capture_output=True,
                          text=True, check=check, timeout=5, env=env)


def helper(op, args=None):
    body = dict(op=op) if args is None else dict(op=op, args=args)
    request = json.dumps(dict(version=1, id='setcap', op=body)).encode()
    wire = b'CX1 ' + str(len(request)).encode() + b'\n' + request
    result = subprocess.run([*USER, binary, 'helper'], input=wire, env=env, capture_output=True, timeout=30)
    assert result.returncode == 0, result.stderr.decode()
    return json.loads(result.stdout.split(b'\n', 1)[1])


def emulate(name, program=FOOT, as_name=None, update='same', cwd=None):
    """Start a fake emulator. The foot-like one runs as `foot` (cx kills only known
    emulators), others as python unless `as_name` names them. Once it runs, its
    executable is deleted (`update`): 'same' puts a recorder at that path, 'path'
    leaves the recorder only on PATH (bins/), 'missing' leaves none anywhere."""
    log = home / f'{name}.emulator'
    as_name = as_name or ('foot' if program == FOOT else None)
    interpreter = sys.executable
    if as_name:
        folder = home / 'emu' / name
        folder.mkdir(parents=True)
        interpreter = str(folder / as_name)
        shutil.copyfile(os.path.realpath(sys.executable), interpreter)
        os.chmod(interpreter, 0o755)
        for path in (folder.parent, folder, Path(interpreter)):
            os.chown(path, 1, 1)
    process = subprocess.Popen([*USER, interpreter, '-c', program, str(log), str(bins / 'claude')],
                               env=dict(env, CHURN_LOG=str(home / f"{name}.churn"), PYTHONHOME=sys.base_prefix,
                                        **HOLDER_ENV),
                               stdin=subprocess.DEVNULL,
                               cwd=str(cwd or home))
    emulators.append(process)
    leader = int(wait_for(lambda: log.exists() and log.read_text().split('\n')[0].split()[-1:], 5, 'emulator')[0])
    wait_for(lambda: exe(leader) == stub, 5, 'stub')
    if as_name:
        os.unlink(interpreter)
        assert exe(process.pid) == f'{interpreter} (deleted)', exe(process.pid)
        if update == 'same':
            Path(interpreter).write_text(RECORDER)
            os.chmod(interpreter, 0o755)
            os.chown(interpreter, 1, 1)
    tty = os.readlink(f'/proc/{leader}/fd/0')
    found = wait_for(lambda: next((s for s in helper('external_sessions')['result']
                                   if (s.get('origin') or {}).get('tty') == tty), None), 5, 'external row')
    return process, leader, found


def cli(args, answer=None, attach=True):
    """Run `cx ARGS` as the user on its own PTY (a terminal of its own, as a person
    would). `answer` is typed at the `move which?` prompt. Once the moved session's
    output is on screen, Ctrl+\\ detaches. Returns (exit status, transcript)."""
    pid, fd = pty.fork()
    if pid == 0:
        os.chdir(home)
        os.execvpe(USER[0], [*USER, binary, *args], env)
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack('HHHH', 30, 100, 0, 0))
    out, answered, detached = b'', answer is None, not attach
    deadline = time.time() + 30
    while time.time() < deadline:
        if select.select([fd], [], [], 0.05)[0]:
            try:
                data = os.read(fd, 65536)
            except OSError:
                break
            if not data:
                break
            out += data
        if not answered and b'move which?' in out:
            os.write(fd, answer.encode())
            answered = True
        if not detached and b'tick' in out.split(b'into CX', 1)[-1] and b'into CX' in out:
            time.sleep(0.3)
            os.write(fd, b'\x1c')
            detached = True
        done, status = os.waitpid(pid, os.WNOHANG)
        if done:
            break
    else:
        os.kill(pid, signal.SIGKILL)
        raise AssertionError(f'cx {args} did not finish: {out[-2000:]!r}')
    if not done:
        _, status = os.waitpid(pid, 0)
    os.close(fd)
    return os.waitstatus_to_exitcode(status), out.decode(errors='replace')


def record(name):
    return json.loads((root / f'{name}.json').read_text())


def recorded(path, seconds=5):
    """What a recorder (a replacement window's program) was started with."""
    text = wait_for(lambda: path.exists() and path.read_text(), seconds, f'{path.name}')
    lines = text.splitlines()
    pick = lambda tag: [line[len(tag) + 1:] for line in lines if line.startswith(tag + ' ')]
    env = dict(e.split('=', 1) for e in pick('env'))
    for key in ('PWD', 'OLDPWD', 'SHLVL', '_'):
        env.pop(key, None)
    return dict(pid=pick('pid')[0], group=pick('group')[0].split(), cwd=pick('cwd')[0],
                stdio=pick('stdio')[0].split(), args=pick('arg'), env=env)


def attach_like_window(args, run_env, cwd):
    """Run a replacement window's command on a PTY of its own, as the emulator would
    (it sets TERM itself): it must show the moved session; Ctrl+\\ detaches."""
    pid, fd = pty.fork()
    if pid == 0:
        os.chdir(cwd)
        os.execvpe(USER[0], [*USER, *args], dict(run_env, TERM='xterm-256color'))
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack('HHHH', 30, 100, 0, 0))
    out, detached, deadline = b'', False, time.time() + 20
    while time.time() < deadline:
        if select.select([fd], [], [], 0.05)[0]:
            try:
                data = os.read(fd, 65536)
            except OSError:
                break
            if not data:
                break
            out += data
        if not detached and b'tick' in out:
            time.sleep(0.3)
            os.write(fd, b'\x1c')
            detached = True
        done, status = os.waitpid(pid, os.WNOHANG)
        if done:
            break
    else:
        os.kill(pid, signal.SIGKILL)
        raise AssertionError(f'{args} did not finish: {out[-2000:]!r}')
    if not done:
        _, status = os.waitpid(pid, 0)
    os.close(fd)
    return os.waitstatus_to_exitcode(status), out.decode(errors='replace')


def stop(name, leader):
    listed = [s for s in helper('sessions')['result'] if s['id'] == name]
    assert listed, name
    stopped = helper('stop_agent_session', dict(id=name, pid=listed[0]['pid'], started=listed[0]['started'],
                                                 boot_id=boot, provider='claude'))
    assert stopped['error'] is None, stopped
    wait_for(lambda: not alive(leader), 5, 'agent exit')


try:
    tmux('start-server', ';', 'set', '-g', 'exit-empty', 'off')
    info = helper('info')['result']
    assert 'adopt-reptyr-v1' in info['capabilities'], info

    # 1. The move is detected although the relay's descriptors are unreadable.
    emulator, leader, found = emulate('move')
    before = stat(leader)
    started = time.time()
    moved = helper('adopt_session', dict(id=found['id'], origin=found['origin'], boot_id=boot))
    elapsed = time.time() - started
    assert moved['error'] is None, moved
    session = moved['result']
    name = session['id']
    assert elapsed < 8, elapsed
    assert session['window'] == 'replaced', session
    pane = int(tmux('display', '-p', '-t', name, '#{pane_pid}').stdout)
    relay_pid = next(p for p in [pane] + children(pane) if exe(p) == os.path.realpath(relay))
    peek = subprocess.run([*USER, 'ls', f'/proc/{relay_pid}/fd'], capture_output=True)
    assert peek.returncode != 0, 'the relay should be non-dumpable to its user'
    assert 'CapEff:\t0000000000000000' in Path(f'/proc/{relay_pid}/status').read_text()
    checks.append(f'move seen in {elapsed:.2f}s although the user cannot read the relay\'s fds')

    saved = record(name)
    assert saved['pid'] == pane == session['pid'] and saved['started'] == session['started'], (saved, session)
    assert not saved['started'].startswith('ext:'), saved
    assert not [n for n in os.listdir(root) if n.endswith(('.adopt', '.tmp'))], os.listdir(root)
    checks.append('record finalized with the pane identity; no .adopt or .tmp left')

    # The emptied foot-like window ignores SIGHUP: it must be SIGKILLed, before its
    # "user" closes it and it signals the moved terminal's process group.
    assert wait_for(lambda: emulator.poll() is not None, 3, 'holder killed') and emulator.returncode == -9, \
        emulator.returncode
    time.sleep(10)
    assert alive(leader) and stat(leader)['start'] == before['start'], 'the moved agent died'
    churn = (home / 'move.churn').read_text()
    assert 'signal HUP' not in churn and 'signal TERM' not in churn, churn
    assert 'close' not in (home / 'move.emulator').read_text()
    listed = [s for s in helper('sessions')['result'] if s['id'] == name]
    assert listed and listed[0]['pid'] == saved['pid'] and listed[0]['started'] == saved['started'], listed
    checks.append('the frozen single-window holder was SIGKILLed; the moved agent ran on for 10 s')

    stopped = helper('stop_agent_session', dict(id=name, pid=listed[0]['pid'], started=listed[0]['started'],
                                                 boot_id=boot, provider='claude'))
    assert stopped['error'] is None, stopped
    wait_for(lambda: not alive(leader), 5, 'agent exit')
    checks.append('the moved agent stops with its listed identity')

    # 2. A record left stale by the old check (pending identity, empty .adopt) heals
    #    on listing, and the session then stops with its listed identity.
    emulator, leader, found = emulate('heal')
    moved = helper('adopt_session', dict(id=found['id'], origin=found['origin'], boot_id=boot))
    assert moved['error'] is None, moved
    name = moved['result']['id']
    stale = dict(record(name), pid=leader, started='ext:' + found['origin']['leader']['start_ticks'])
    (root / f'{name}.json').write_text(json.dumps(stale))
    (root / f'{name}.adopt').write_text('')
    listed = [s for s in helper('sessions')['result'] if s['id'] == name]
    assert listed, 'adopted session not listed'
    healed = record(name)
    assert healed['pid'] == listed[0]['pid'] and healed['started'] == listed[0]['started'], healed
    assert not (root / f'{name}.adopt').exists()
    assert healed['origin'] == stale['origin'], healed
    stopped = helper('stop_agent_session', dict(id=name, pid=listed[0]['pid'], started=listed[0]['started'],
                                                 boot_id=boot, provider='claude'))
    assert stopped['error'] is None, stopped
    wait_for(lambda: not alive(leader), 5, 'healed agent exit')
    checks.append('a stale adopted record heals on listing and then stops')

    # 3. Stopping an agent in a terminal outside tmux: identity first, then HUP to its
    #    process group, KILL after 3 s (the stub ignores HUP); the window stays.
    assert 'stop-external-v1' in info['capabilities'], info
    emulator, leader, found = emulate('outside', EMULATOR)
    tool = wait_for(lambda: children(leader), 5, 'tool child')
    wrong = json.loads(json.dumps(found['origin']))
    wrong['leader']['start_ticks'] = str(int(wrong['leader']['start_ticks']) + 1)
    refused = helper('stop_external', dict(id=found['id'], origin=wrong, boot_id=boot))
    assert refused['error'] and alive(leader), refused
    refused = helper('stop_external', dict(id=found['id'], origin=found['origin'], boot_id='other-boot'))
    assert refused['error'] and alive(leader), refused
    checks.append('stop_external refuses a changed identity or boot without signalling')
    started = time.time()
    stopped = helper('stop_external', dict(id=found['id'], origin=found['origin'], boot_id=boot))
    assert stopped['error'] is None, stopped
    wait_for(lambda: not alive(leader) and not any(alive(t) for t in tool), 6, 'outside agent exit')
    took = time.time() - started
    assert 'signal HUP' in (home / 'outside.churn').read_text()
    assert 2.5 < took < 6, took
    assert emulator.poll() is None, 'the terminal window is left to its app'
    checks.append(f'outside agent got SIGHUP, then SIGKILL ({took:.1f}s); its window was left alone')

    # 4. The CLI: `cx adopt PID`, `cx adopt /dev/pts/N` and `cx adopt` with a choice
    #    from its list move the terminal and attach it; Ctrl+\\ detaches with status 0.
    for how in ('pid', 'tty', 'list'):
        emulator, leader, found = emulate(f'cli-{how}')
        args, answer = {'pid': (['adopt', str(leader)], None),
                        'tty': (['adopt', found['origin']['tty']], None),
                        'list': (['adopt'], '1\n')}[how]
        code, shown = cli(args, answer)
        assert code == 0 and 'into CX' in shown, (how, code, shown[-3000:])
        assert wait_for(lambda: emulator.poll() is not None, 3, 'holder killed') and emulator.returncode == -9, \
            (how, emulator.returncode)
        moved = [s for s in helper('sessions')['result'] if (s.get('origin') or {}).get('tty') == found['origin']['tty']]
        assert moved and not moved[0]['external'], (how, moved)
        assert not (root / f"{moved[0]['id']}.adopt").exists()
        time.sleep(3)
        assert alive(leader) and 'close' not in (home / f'cli-{how}.emulator').read_text(), how
        stopped = helper('stop_agent_session', dict(id=moved[0]['id'], pid=moved[0]['pid'],
                                                     started=moved[0]['started'], boot_id=boot, provider='claude'))
        assert stopped['error'] is None, stopped
        wait_for(lambda: not alive(leader), 5, 'cli agent exit')
        checks.append(f'`cx adopt` by {how} moved the terminal, attached it, detached with 0; '
                      'holder SIGKILLed, agent alive')

    # 5. `cx kill ID -y` on an agent outside tmux: StopExternal (HUP, KILL after 3 s).
    emulator, leader, found = emulate('cli-kill', EMULATOR)
    started = time.time()
    killed = subprocess.run([*USER, binary, 'kill', found['id'], '-y'], env=env, cwd=home,
                            capture_output=True, text=True, timeout=30)
    assert killed.returncode == 0, killed.stderr
    wait_for(lambda: not alive(leader), 6, 'cx kill agent exit')
    assert 'signal HUP' in (home / 'cli-kill.churn').read_text()
    assert 2.5 < time.time() - started < 8 and emulator.poll() is None
    checks.append('`cx kill ID -y` stopped the outside agent (HUP, then KILL); window untouched')

    # 6. The replacement window. A verified move reopens the same emulator (its
    #    updated binary at the same path) running `cx attach <id>` in the leader's
    #    cwd, with only whitelisted variables, in a session of its own, stdio on
    #    /dev/null; then the frozen window is SIGKILLed.
    project = home / 'project'
    project.mkdir()
    os.chown(project, 1, 1)
    emulator, leader, found = emulate('window', cwd=project)
    moved = helper('adopt_session', dict(id=found['id'], origin=found['origin'], boot_id=boot))
    assert moved['error'] is None and moved['result']['window'] == 'replaced', moved
    name = moved['result']['id']
    window = recorded(home / 'emu/window/foot.record')
    assert window['args'] == [str(home / 'emu/window/foot'), binary, 'attach', name], window['args']
    assert window['cwd'] == str(project), window['cwd']
    assert set(window['env']) <= WINDOW_KEYS, sorted(window['env'])
    for key in ('WAYLAND_DISPLAY', 'DISPLAY', 'XAUTHORITY', 'XDG_RUNTIME_DIR', 'DBUS_SESSION_BUS_ADDRESS'):
        assert window['env'][key] == HOLDER_ENV[key], (key, window['env'])
    assert window['env']['HOME'] == str(home) and window['env']['XDG_STATE_HOME'] == env['XDG_STATE_HOME']
    assert 'TERM' not in window['env'] and 'sk-never-copied' not in json.dumps(window), window
    assert window['group'] == [window['pid'], window['pid']], window['group']
    assert window['stdio'] == ['/dev/null'] * 3, window['stdio']
    assert not (bins / 'systemd-run.record').exists(), 'no user bus: systemd-run must not be used'
    assert wait_for(lambda: emulator.poll() is not None, 3, 'holder killed') and emulator.returncode == -9, \
        emulator.returncode
    checks.append('a verified move reopened the updated foot with `cx attach <id>`, the leader\'s cwd, '
                  'only display/login variables, its own session; then SIGKILLed the frozen one')
    code, shown = attach_like_window(window['args'][1:], window['env'], window['cwd'])
    assert code == 0 and 'tick' in shown, (code, shown[-2000:])
    assert alive(leader) and 'signal HUP' not in (home / 'window.churn').read_text()
    checks.append('the replacement window\'s exact command and environment attach the moved session; '
                  'Ctrl+\\ detaches with 0')
    stop(name, leader)

    #    An emulator whose binary moved is found on PATH; with a reachable user bus
    #    the window gets its own scope through systemd-run (a fake here).
    (bins / 'foot').write_text(RECORDER)
    os.chmod(bins / 'foot', 0o755)
    os.chown(bins / 'foot', 1, 1)
    import socket as sockets
    bus = sockets.socket(sockets.AF_UNIX)
    bus.bind(f'{home}/run-holder/bus')
    os.chmod(f'{home}/run-holder/bus', 0o777)
    bus.listen(4)
    emulator, leader, found = emulate('scope', update='path')
    moved = helper('adopt_session', dict(id=found['id'], origin=found['origin'], boot_id=boot))
    assert moved['error'] is None and moved['result']['window'] == 'replaced', moved
    name = moved['result']['id']
    window = recorded(bins / 'foot.record')
    assert window['args'] == [str(bins / 'foot'), binary, 'attach', name], window['args']
    scope = (bins / 'systemd-run.record').read_text().split('\n')
    assert scope[:6] == ['--user', '--scope', '--collect', '--quiet', '--', str(bins / 'foot')], scope
    assert wait_for(lambda: emulator.poll() is not None, 3, 'holder killed') and emulator.returncode == -9
    bus.close()
    os.unlink(f'{home}/run-holder/bus')
    os.unlink(bins / 'foot')
    os.unlink(bins / 'foot.record')
    os.unlink(bins / 'systemd-run.record')
    stop(name, leader)
    checks.append('an emulator binary gone from its path is found on PATH; a reachable user bus '
                  'places the window with `systemd-run --user --scope --collect --quiet --`')

    #    The emulator cannot be started again (its binary is gone everywhere): the
    #    frozen window is still SIGKILLed and the move reports "closed".
    emulator, leader, found = emulate('missing', update='missing')
    moved = helper('adopt_session', dict(id=found['id'], origin=found['origin'], boot_id=boot))
    assert moved['error'] is None and moved['result']['window'] == 'closed', moved
    assert wait_for(lambda: emulator.poll() is not None, 3, 'holder killed') and emulator.returncode == -9
    time.sleep(3)
    assert alive(leader) and 'close' not in (home / 'missing.emulator').read_text()
    assert not list((home / 'emu/missing').glob('*.record')) and not list(bins.glob('*.record'))
    stop(moved['result']['id'], leader)
    checks.append('a replacement that cannot start falls back to closing: holder SIGKILLed, "closed"')

    #    An unknown holder (an editor's terminal, here a python named `emacs`, with an
    #    `emacs` recorder on PATH) is neither reopened nor SIGKILLed: SIGHUP only.
    (bins / 'emacs').write_text(RECORDER)
    os.chmod(bins / 'emacs', 0o755)
    os.chown(bins / 'emacs', 1, 1)
    emulator, leader, found = emulate('editor', EMULATOR, as_name='emacs')
    moved = helper('adopt_session', dict(id=found['id'], origin=found['origin'], boot_id=boot))
    assert moved['error'] is None and moved['result']['window'] == 'closed', moved
    assert wait_for(lambda: emulator.poll() is not None, 3, 'holder hung up') and \
        emulator.returncode == -signal.SIGHUP, emulator.returncode
    time.sleep(1)
    assert not list((home / 'emu/editor').glob('*.record')) and not list(bins.glob('*.record'))
    assert alive(leader)
    stop(moved['result']['id'], leader)
    checks.append('an unknown holder (`emacs`) got SIGHUP only: never reopened, never SIGKILLed')

    leftovers = [p for p in map(int, filter(str.isdigit, os.listdir('/proc'))) if exe(p) == stub]
    assert not leftovers, leftovers
    print(json.dumps(dict(result='PASS', checks=checks)))
finally:
    for process in emulators:
        process.kill()
    tmux('kill-server', check=False)
    for pid in map(int, filter(str.isdigit, os.listdir('/proc'))):
        if exe(pid) == stub:
            try:
                os.kill(pid, signal.SIGKILL)
            except OSError:
                pass
    shutil.rmtree(home, ignore_errors=True)
