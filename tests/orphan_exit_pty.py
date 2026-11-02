#!/usr/bin/env python3
"""cx processes must not outlive their terminal or parent, and must not spin while dying.

1. A viewer, and a viewer started through the update `restart` handoff, run in their own
   process group (so the kernel's hangup SIGHUP never reaches them). Closing the pty master
   must make each exit within 3 s, without busy-looping (CPU time is checked), and the
   handoff file must be consumed.
2. A helper whose parent died (reparented to init or a user-manager subreaper) is reported
   as stray by `cx orphans --json`. The test stops its own fixture; it never runs --clean.
usage: script BINARY
"""
import fcntl, json, os, pathlib, pty, struct, subprocess, sys, tempfile, termios, time
binary = str(pathlib.Path(sys.argv[1]).resolve())
HZ = os.sysconf('SC_CLK_TCK')

def cpu_seconds(pid, children=False):
    fields = pathlib.Path(f'/proc/{pid}/stat').read_text().rsplit(')', 1)[1].split()
    # wait4's rusage also counts reaped children (cutime/cstime), so compare like with like.
    return sum(int(f) for f in fields[11:15 if children else 13]) / HZ

def viewer_exits(env, args, label, state=None):
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 24, 80, 0, 0))
    leader = subprocess.Popen(['sh', '-c', 'setsid -w true; exec sleep 60'], stdin=slave, stdout=slave, stderr=slave, env=env, start_new_session=True)
    client = subprocess.Popen([binary] + args, stdin=slave, stdout=slave, stderr=slave, env=env, process_group=0)
    os.close(slave)
    try:
        # Drain output so the viewer never blocks on a full pty buffer.
        deadline = time.monotonic() + 2
        while time.monotonic() < deadline:
            try:
                os.set_blocking(master, False); os.read(master, 65536)
            except BlockingIOError:
                pass
            time.sleep(.05)
        assert client.poll() is None, f'{label}: exited before the terminal closed'
        idle_start = cpu_seconds(client.pid); time.sleep(1); idle = cpu_seconds(client.pid) - idle_start
        assert idle < .5, f'{label}: {idle:.2f}s CPU in 1s while idle'
        if state is not None:
            assert not state.exists(), f'{label}: restart handoff not consumed'
        before = cpu_seconds(client.pid, children=True)
        os.close(master); master = None
        leader.kill(); leader.wait()
        started = time.monotonic(); status = None
        while time.monotonic() - started < 3:
            pid, status, usage = os.wait4(client.pid, os.WNOHANG)
            if pid:
                break
            time.sleep(.05)
        else:
            raise AssertionError(f'{label}: still running 3s after its terminal went away')
        client.returncode = status
        dying = usage.ru_utime + usage.ru_stime - before
        elapsed = time.monotonic() - started
        assert dying < max(.6, elapsed * .5), f'{label}: spun {dying:.2f}s CPU in {elapsed:.2f}s while exiting'
        print(f'orphan_exit_pty: {label} exited in {elapsed:.2f}s using {dying:.2f}s CPU')
    finally:
        if master is not None:
            os.close(master)
        for p in (client, leader):
            if p.returncode is None and p.poll() is None:
                p.kill(); p.wait()

def orphaned_helper_reported(env):
    r, w = os.pipe()
    launcher = subprocess.run(['sh', '-c', '"$0" helper <&3 >/dev/null 2>&1 & echo $!', binary], env=env, pass_fds=(r,), stdout=subprocess.PIPE, text=True, check=True)
    os.close(r)
    pid = int(launcher.stdout.strip())
    try:
        time.sleep(.3)
        report = json.loads(subprocess.run([binary, 'orphans', '--json'], env=env, stdout=subprocess.PIPE, text=True, check=True, timeout=20).stdout)
        row = next((p for p in report['processes'] if p['pid'] == pid), None)
        assert row and row['role'] == 'helper' and row['orphaned'] and row['stray'], f'orphaned helper not reported stray: {row}'
        own = [p for p in report['processes'] if p['pid'] == os.getpid()]
        assert not own
        print('orphan_exit_pty: orphaned helper reported stray')
    finally:
        os.close(w)  # EOF: the helper exits by itself
        deadline = time.monotonic() + 3
        while time.monotonic() < deadline and pathlib.Path(f'/proc/{pid}').exists() and 'Z' not in pathlib.Path(f'/proc/{pid}/stat').read_text().rsplit(')', 1)[1].split()[0]:
            time.sleep(.05)
        if pathlib.Path(f'/proc/{pid}').exists():
            try:
                os.kill(pid, 9)
            except ProcessLookupError:
                pass

with tempfile.TemporaryDirectory(prefix='cx-orphan-') as temporary:
    root = pathlib.Path(temporary)
    env = dict(os.environ, HOME=str(root), XDG_STATE_HOME=str(root/'state'), TERM='xterm-256color', SHELL='/bin/sh', HTTPS_PROXY='http://127.0.0.1:9')
    env.pop('TMUX', None); env.pop('TMUX_PANE', None)
    viewer_exits(env, [], 'viewer')
    state = root/'state/cx'; state.mkdir(parents=True, exist_ok=True)
    name = 'viewer-restart-1-2.json'
    handoff = state/name
    handoff.write_text(json.dumps({'schema': 1, 'expires_at': 0, 'device_ids': ['local'], 'device': 0, 'focus': 'Workspace', 'view': 'Work', 'selected': 0, 'selected_session': None, 'side_selected': 0, 'search': '', 'browser': None, 'other_browser': None, 'destination_active': False, 'conflict': 0, 'launch_provider': None, 'clipboard': None, 'submitted': {}}))
    handoff.chmod(0o600)
    stale = state/'viewer-restart-3-4.json'; stale.write_text('{}'); os.utime(stale, (time.time() - 3600,) * 2)
    viewer_exits(env, ['restart', name], 'restarted viewer (expired handoff)', state=handoff)
    assert not stale.exists(), 'stale restart handoff not swept'
    orphaned_helper_reported(env)
    print('orphan_exit_pty: ok')
