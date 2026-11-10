#!/usr/bin/env python3
"""Run one fixture against an older tmux build, the way a host without a
systemd user bus (Rocky 8 over SSH, containers) runs cx.

Usage: python tests/legacy_tmux_run.py TMUX_BIN_DIR FIXTURE.py|COMMAND [ARGS...]
  e.g. ... target/tmux-2.7/bin tests/run_regressions.sh --no-rust --only list_wheel_pty

cx starts its managed server through `systemd-run --user ... tmux -D` when the
user bus exists, and `tmux -D` needs tmux >= 3.2. This wrapper re-executes the
fixture in a private user+mount+pid+cgroup namespace (same uid, own /proc) with
/run/user/<uid> masked by an empty tmpfs, so cx uses its plain owned-server
path and the fixture's tmux servers die with the namespace. TMUX_BIN_DIR is put
first on PATH (for example a locally built tmux 2.7 or 3.4 prefix's bin/).
Host mounts, services, networking and the caller's tmux servers are untouched.
"""
import ctypes, os, sys, tempfile

if len(sys.argv) < 3:
    sys.exit(__doc__)
bindir, fixture, *rest = sys.argv[1:]
uid, gid = os.getuid(), os.getgid()
os.unshare(os.CLONE_NEWUSER | os.CLONE_NEWNS | os.CLONE_NEWPID | os.CLONE_NEWCGROUP)
for name, text in (('setgroups', 'deny'), ('uid_map', f'{uid} {uid} 1'), ('gid_map', f'{gid} {gid} 1')):
    with open(f'/proc/self/{name}', 'w') as f:
        f.write(text)
child = os.fork()
if child:
    _, status = os.waitpid(child, 0)
    sys.exit(os.waitstatus_to_exitcode(status))
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
env = dict(os.environ, PATH=os.path.abspath(bindir) + os.pathsep + os.environ.get('PATH', ''))
env.setdefault('TMUX_TMPDIR', tempfile.mkdtemp(prefix='cx-legacy-tmux-'))
env.pop('TMUX', None)
env.pop('TMUX_PANE', None)
env.pop('DBUS_SESSION_BUS_ADDRESS', None)
if fixture.endswith('.py'):
    os.execve(sys.executable, [sys.executable, fixture, *rest], env)
os.execvpe(fixture, [fixture, *rest], env)
