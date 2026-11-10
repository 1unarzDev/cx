#!/usr/bin/env python3
"""Capture the Sessions flow (list, zoom, inline typing, native attach with the
detach chip, return) with the capture_matrix fixture, for screenshot review.

Usage: python tests/session_flow_capture.py target/debug/cx OUT_DIR [--sizes 40x20,...] [--env K=V]
       [--trend] [--shared-proc]
--trend adds a flow-trend shot (>=120 cols) after a 64 s wait, so the 60 s
sparkline has real history. By default the run re-execs itself in a private
user+mount+pid namespace with its own /proc and a masked user bus, so session
discovery sees only the fixture; --shared-proc skips that.
Reuses tests/capture_matrix.py: owned temporary HOME/XDG state, private managed tmux
servers killed at exit, a fake ssh, and masking of host-identifying text.
"""
import importlib.util, os, pathlib, pty, select, fcntl, struct, subprocess, sys, tempfile, termios, time

here = pathlib.Path(__file__).resolve().parent
# Discovery lists every outside-tmux terminal this uid owns (from /proc). Run in
# a private user+mount+pid namespace with its own /proc, so the captures show
# only the fixture's sessions, never the host's own terminals. The user bus is
# masked inside that namespace too, so managed tmux runs as a plain owned server
# (no systemd user units) and dies with the namespace; a private UTS
# namespace fixes the hostname; a private cgroup
# namespace keeps cx's service-cgroup guard from misreading the caller's unit
# (the fixture servers are meant to die with the capture). Same uid; the
# host's mounts, network and services are untouched.


def isolate():
    import ctypes
    uid, gid = os.getuid(), os.getgid()
    os.unshare(os.CLONE_NEWUSER | os.CLONE_NEWNS | os.CLONE_NEWPID | os.CLONE_NEWCGROUP | os.CLONE_NEWUTS)
    for name, text in (('setgroups', 'deny'), ('uid_map', f'{uid} {uid} 1'), ('gid_map', f'{gid} {gid} 1')):
        with open(f'/proc/self/{name}', 'w') as f:
            f.write(text)
    child = os.fork()
    if child:
        _, status = os.waitpid(child, 0)
        os._exit(os.waitstatus_to_exitcode(status))
    # A fixed hostname: shell prompts cut at narrow widths can no longer show
    # a fragment of the real one that the regex masking would miss.
    import socket
    socket.sethostname('fixturehost')
    libc = ctypes.CDLL(None, use_errno=True)
    MS_REC, MS_PRIVATE = 16384, 1 << 18

    def mount(src, target, fstype, flags, data):
        if libc.mount(src, target, fstype, ctypes.c_ulong(flags), data) != 0:
            raise OSError(ctypes.get_errno(), f'mount {target!r}')
    mount(None, b'/', None, MS_REC | MS_PRIVATE, None)
    mount(b'proc', b'/proc', b'proc', 0, None)
    run_user = f'/run/user/{uid}'.encode()
    if os.path.isdir(run_user):
        mount(b'tmpfs', run_user, b'tmpfs', 0, b'mode=700')
    env = dict(os.environ, CX_FLOW_ISOLATED='1', TMUX_TMPDIR=tempfile.mkdtemp(prefix='cx-flow-tmux-'))
    env.pop('TMUX', None)
    env.pop('TMUX_PANE', None)
    os.execve(sys.executable, [sys.executable, *sys.argv], env)


if os.environ.get('CX_FLOW_ISOLATED') != '1' and '--shared-proc' not in sys.argv:
    try:
        isolate()
    except OSError as e:
        print(f'warning: namespace isolation unavailable ({e}); captures may list host terminals',
              file=sys.stderr)
sys.argv = [a for a in sys.argv if a != '--shared-proc']
trend = '--trend' in sys.argv
sys.argv = [a for a in sys.argv if a != '--trend']
# A fixture shell can still write its history while capture_matrix removes its
# temporary HOME (ENOTEMPTY). Tolerate that there and remove the leftovers here,
# after the fixture servers are gone.
_owned_temporaries = []
_TemporaryDirectory = tempfile.TemporaryDirectory


def _tolerant_temporary_directory(*a, **k):
    k.setdefault('ignore_cleanup_errors', True)
    directory = _TemporaryDirectory(*a, **k)
    _owned_temporaries.append(directory.name)
    return directory


tempfile.TemporaryDirectory = _tolerant_temporary_directory


def _remove_leftovers():
    import shutil
    time.sleep(.5)
    for name in _owned_temporaries:
        shutil.rmtree(name, ignore_errors=True)


spec = importlib.util.spec_from_file_location('capture_matrix', here / 'capture_matrix.py')
cm = importlib.util.module_from_spec(spec)
argv = sys.argv
sys.argv = [argv[0]] + argv[1:]
spec.loader.exec_module(cm)


def capture_size(cols, rows, env):
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', rows, cols, 0, 0))

    def control():
        os.setsid()
        fcntl.ioctl(slave, termios.TIOCSCTTY, 0)

    proc = subprocess.Popen([cm.binary], stdin=slave, stdout=slave, stderr=slave, env=env,
                            cwd=env['HOME'], preexec_fn=control)
    screen = cm.Screen(cols, rows)
    stream = cm.pyte.Stream(screen)
    # Multibyte characters can straddle reads; decode incrementally.
    decoder = __import__('codecs').getincrementaldecoder('utf-8')('replace')

    def read(seconds=.35):
        end = time.monotonic() + seconds
        while time.monotonic() < end:
            if select.select([master], [], [], .03)[0]:
                try:
                    stream.feed(decoder.decode(os.read(master, 65536)))
                except OSError:
                    break

    def send(b, wait=.45):
        os.write(master, b)
        read(wait)

    results = []

    def shot(view):
        name = f'{view}-{cols}x{rows}'
        cm.redact(screen, cols, rows)
        (cm.out / f'{name}.txt').write_text('\n'.join(screen.display) + '\n')
        cm.render_png(screen, cols, rows, cm.out / f'{name}.png')
        results.append(dict(view=view, cols=cols, rows=rows, png=f'{name}.png'))

    try:
        read(2.5)
        send(b'\x1b', .8)   # Files -> Sessions
        send(b'\x1b', .4)   # focus devices
        send(b'gg', .4)     # All devices
        send(b'\r', 2.5)
        shot('flow-list')
        send(b'j', 1.5)
        shot('flow-next')
        # Discovery may list the host's own outside-tmux terminals; keep the flow
        # on the fixture's managed shell by searching for it.
        send(b'/build', .6)
        send(b'\x1b', 2.0)  # Esc keeps the filter (Enter would attach)
        shot('flow-search')
        send(b'z', 1.2)
        shot('flow-zoom')
        send(b'z', .6)
        send(b'i', 1.2)
        send(b'echo typed-inline', 1.2)
        shot('flow-typing')
        send(b'\x15', .4)    # Ctrl+U clears the typed line in the shell
        send(b'\x1c', .8)    # Ctrl+\ leaves inline input
        send(b'\r', 2.0)     # Enter attaches natively
        shot('flow-attached')
        send(b'\x1c', 2.0)   # Ctrl+\ detaches back to the list row
        shot('flow-returned')
        if trend and cols >= 120:
            # The sparkline is 60 s of successive listings (one per 3 s). Give
            # robot-logs bursty output, clear the filter, and wait a full window.
            # (The 120-column list has no sparkline column; it shows the state.)
            sock = pathlib.Path(env['XDG_STATE_HOME']) / 'cx/managed.sock'
            panes = subprocess.run(['tmux', '-S', str(sock), 'list-panes', '-a', '-F',
                                    '#{session_id} #{pane_current_path}'],
                                   capture_output=True, text=True, timeout=5).stdout
            target = next((line.partition(' ')[0] for line in panes.splitlines()
                           if line.endswith('/robot-logs')), None)
            send(b'\x1b', .6)   # Esc clears the search filter
            # Bursts of output from the capture itself (no foreground loop, so
            # the shell is honestly idle between bursts): busy, quiet, busy.
            bursts = {0, 1, 2, 3, 4, 5, 6, 7, 8, 24, 25, 26, 27, 44, 45, 52}
            start = time.monotonic()
            for second in range(64):
                if target and second in bursts:
                    subprocess.run(['tmux', '-S', str(sock), 'send-keys', '-t', target,
                                    f'echo lidar tick {second}', 'Enter'],
                                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
                read(max(0, start + second + 1 - time.monotonic()))
            shot('flow-trend')
    finally:
        try:
            os.write(master, b'\x03')
            proc.wait(timeout=5)
        except Exception:
            proc.kill()
            proc.wait()
        os.close(master)
        os.close(slave)
    return results


cm.capture_size = capture_size
try:
    cm.main()
finally:
    _remove_leftovers()
