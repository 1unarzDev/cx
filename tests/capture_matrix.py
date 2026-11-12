#!/usr/bin/env python3
"""Capture the real cx UI across a size x view matrix for design/critic passes.

  python tests/capture_matrix.py [--bin target/debug/cx] [--out DIR]
      [--sizes 40x20,80x24,120x40,200x60] [--views startup,files,...] [--network]
      [--theme dark|light] [--ascii] [--nocolor] [--env K=V ...]
  python tests/capture_matrix.py --self-test   # masking/arguments/cleanup checks, no cx

Needs pyte and pillow (for example the worktrees .venv-test interpreter,
or `uv run --with pyte --with pillow python ...`). Writes <view>-<cols>x<rows>.txt
and .png per capture plus index.json into --out (default
local-evidence/ui-passes/latest). Every view is captured from a fresh cx process.
--theme picks the emulated terminal: the PNG palette (tests/lab_png.py, through
tests/pty_lib.py) plus COLORFGBG/CX_THEME; index.json records it as `terminal_palette`.

Isolation (everything lives under one owned temp root, removed at exit):
  * cx starts from an allowlisted environment, never the parent's: a fixed system
    PATH behind a fixture bin dir, temp HOME/XDG_STATE_HOME, LANG/LC_ALL, TERM,
    SHELL, USER/LOGNAME, and explicit test variables only (XDG_RUNTIME_DIR so the
    managed tmux server can start as its normal `cx-tmux-*` user unit, a private
    TMUX_TMPDIR so the default/external tmux server is a private empty one, a
    dead HTTP(S) proxy, theme/ASCII/NO_COLOR switches, and --env extras).
  * fixture shells: SHELL is a fixture `bash` wrapper that drops the inherited
    environment and user dotfiles (`bash --noprofile --rcfile <fixture>`), so a
    pane prompt is `fixture:<dir>$` and two sessions print distinct colored output.
  * a fake `ssh` serves one "online" remote (lab-arm) by exec'ing this binary's
    helper under a second temp HOME and refuses one "offline" remote; fake `ping`,
    `timeout` (name lookups), `nmcli`, `docker` and `podman` keep the Network and
    container probes local and deterministic. `ip` is read-only and always the
    host's (cx resolves /usr/sbin/ip itself), so Network captures mask every
    interface name, address, MAC, *.local name, user and hostname (re-read before
    each capture). `unmasked` counts snapshot identifiers still visible whole or
    cut short (`name…`, or at the screen edge).
  * --network (off by default) adds the network/network-expanded views. They run
    cx's normal automatic neighbour check (a bounded TCP connect to port 22 of
    current LAN neighbour-cache entries, no payload, no auth); there is no
    fixture hook for it yet.
  * cleanup is registered before anything is created: every managed tmux socket,
    every socket in the private TMUX_TMPDIR, the cx-tmux-<sha256(socket)> units of
    our sockets (only if their ExecStart names our temp root), and every process
    carrying this run's marker variable are stopped in a finally; the temp root is
    removed and re-checked. The result line reports leftovers.
Never runs ~/.local/bin/cx; never touches the user's state, tmux servers, SSH or network config.
"""
import argparse, getpass, hashlib, ipaddress, json, os, pathlib, re, shlex, shutil, signal
import socket, subprocess, sys, tempfile, time

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))

try:
    import pyte  # noqa: F401  (pty_lib's screen model)
    import PIL  # noqa: F401  (lab_png's PNGs)
    import pty_lib
except ImportError as e:
    sys.exit(f'capture_matrix needs pyte and pillow: {e}')

REPO = pathlib.Path(__file__).resolve().parent.parent
DEFAULT_VIEWS = ['startup', 'device', 'files', 'files-projects', 'files-inside', 'files-preview', 'sessions', 'sessions-all',
                 'sessions-next', 'palette', 'help', 'edge', 'keys']
NETWORK_VIEWS = ['network', 'network-expanded']
ALL_VIEWS = DEFAULT_VIEWS + NETWORK_VIEWS
SYSTEM_PATH = '/usr/local/sbin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin'


def parse_args(argv):
    p = argparse.ArgumentParser(description=__doc__.split('\n')[0])
    p.add_argument('--bin', default=str(REPO / 'target/debug/cx'), help='cx binary (default target/debug/cx)')
    p.add_argument('--out', default=str(REPO / 'local-evidence/ui-passes/latest'), help='output directory')
    p.add_argument('--sizes', default='40x20,80x24,120x40,200x60')
    p.add_argument('--views', default=','.join(DEFAULT_VIEWS), help=f'comma list from {ALL_VIEWS}')
    p.add_argument('--theme', choices=['dark', 'light'], default='dark')
    p.add_argument('--ascii', action='store_true', help='set CX_ASCII=1')
    p.add_argument('--nocolor', action='store_true', help='set NO_COLOR=1')
    p.add_argument('--env', action='append', default=[], metavar='K=V', help='extra explicit test variable')
    p.add_argument('--network', action='store_true',
                   help="also capture network views (runs cx's real LAN neighbour SSH-port checks)")
    a = p.parse_args(argv)
    a.bin = str(pathlib.Path(a.bin).resolve())
    if not os.access(a.bin, os.X_OK):
        p.error(f'not an executable: {a.bin}')
    sizes = [s for s in a.sizes.split(',') if s]
    for s in sizes:
        if not re.fullmatch(r'[1-9]\d*x[1-9]\d*', s):
            p.error(f'--sizes expects COLSxROWS, got {s!r}')
    a.sizes = [tuple(int(n) for n in s.split('x')) for s in sizes]
    a.views = [v for v in a.views.split(',') if v]
    a.views += [v for v in NETWORK_VIEWS if a.network and v not in a.views]
    unknown = [v for v in a.views if v not in ALL_VIEWS]
    if unknown:
        p.error(f'unknown views {unknown}; choose from {ALL_VIEWS}')
    extra = {}
    for kv in a.env:
        k, sep, v = kv.partition('=')
        if not sep or not re.fullmatch(r'[A-Z_][A-Z0-9_]*', k):
            p.error(f'--env expects K=V, got {kv!r}')
        extra[k] = v
    a.extra_env = extra
    return a


# --------------------------------------------------------------------------- masking
def host_identifiers():
    """(interface names, addresses, user/host names) from a bounded read-only `ip -j` snapshot."""
    ifnames, addrs = set(), set()
    for args in (['-j', 'address', 'show'], ['-j', 'neigh', 'show'], ['-j', 'route', 'show']):
        try:  # bounded read-only observation of the same data cx itself reads
            out = subprocess.run(['ip', *args], capture_output=True, text=True, timeout=5,
                                 env={'PATH': SYSTEM_PATH, 'LC_ALL': 'C'}).stdout
            data = json.loads(out or '[]')
        except Exception:
            data = []
        for item in data if isinstance(data, list) else []:
            for key in ('ifname', 'dev', 'link'):
                if isinstance(item.get(key), str):
                    ifnames.add(item[key])
            for key in ('dst', 'gateway', 'prefsrc', 'lladdr', 'address', 'broadcast'):
                if isinstance(item.get(key), str):
                    addrs.add(item[key].split('/')[0])
            for info in item.get('addr_info', []) or []:
                for key in ('local', 'broadcast', 'address'):
                    if isinstance(info.get(key), str):
                        addrs.add(info[key])
    ids = {getpass.getuser(), socket.gethostname().split('.')[0], socket.gethostname()}
    try:
        ids.add(pathlib.Path('/proc/sys/kernel/hostname').read_text().strip())
    except OSError:
        pass
    return ifnames, addrs, ids


class Masker:
    """Same-length masks, so masking never shifts or pads the layout."""

    def __init__(self, root, idents):
        ifnames, addrs, ids = idents
        self.literal = []  # (text, replacement)
        # The temp root is the only random part of fixture paths; mask its suffix in place.
        name = root.name
        self.literal.append((name, re.sub(r'(?<=cxcap-)\w+$', lambda m: 'x' * len(m.group()), name)))
        # Longest first so a name that contains another is masked whole; ties by name keep ifN stable.
        longest = lambda n: (-len(n), n)
        for i, n in enumerate(sorted((n for n in ifnames if len(n) >= 2 and n != 'lo'), key=longest)):
            self.literal.append((n, f'if{i}'.ljust(len(n), 'x')[:len(n)]))
        for a in sorted((a for a in addrs if len(a) >= 3 and a != 'default'), key=longest):
            self.literal.append((a, ''.join(c if c in '.:/' else 'x' for c in a)))
        for w in sorted((w for w in ids if len(w) > 2), key=longest):
            self.literal.append((w, 'x' * len(w)))
        self.regex = [re.compile(p, re.I) for p in [
            r'\b(?:\d{1,3}\.){3}\d{1,3}\b',
            r'\b(?:[0-9a-f]{2}:){5}[0-9a-f]{2}\b',
            r'\b[\w-]+\.(?:local|lan|home|internal)\b']]
        # IPv6 candidates (any compressed form); masked only when they parse, so times stay.
        self.ipv6 = re.compile(r'(?<![\w:.])[0-9a-f]*:[0-9a-f:]*:[0-9a-f.:]*[0-9a-f]', re.I)

    @staticmethod
    def bounded(text):
        return re.compile(r'(?<![A-Za-z0-9])' + re.escape(text) + r'(?![A-Za-z0-9])')

    @staticmethod
    def mask(text):
        return ''.join(c if c in '.:-/' else 'x' for c in text)

    def spans(self, row):
        hits = []
        for text, rep in self.literal:
            for m in self.bounded(text).finditer(row):
                hits.append((m.start(), rep))
        for pattern in self.regex:
            for m in pattern.finditer(row):
                hits.append((m.start(), self.mask(m.group())))
        for m in self.ipv6.finditer(row):
            try:
                ipaddress.IPv6Address(m.group())
            except ValueError:
                continue
            hits.append((m.start(), self.mask(m.group())))
        return hits

    def apply(self, screen, cols, rows):
        for y in range(rows):
            line = screen.buffer[y]
            row = ''.join((line[x].data or ' ')[:1] for x in range(cols))
            for start, rep in self.spans(row):
                for i, ch in enumerate(rep):
                    if start + i < cols:
                        line[start + i] = line[start + i]._replace(data=ch)

    def leaks(self, text):
        """How many host identifiers are still visible, whole or cut short (`name…`, `name...`
        or at the screen edge, at least 4 chars); a count, never the values themselves."""
        rows = text.split('\n')
        count = 0
        for t, rep in self.literal[1:]:  # host identifiers; [0] is the temp root, not one
            if t == rep:
                continue
            cut = '|'.join(re.escape(t[:k]) for k in range(len(t) - 1, 3, -1))
            pattern = self.bounded(t).pattern + (f'|(?<![A-Za-z0-9])(?:{cut})(?:…|\\.\\.\\.|$)' if cut else '')
            if any(re.search(pattern, r) for r in rows):
                count += 1
        return count


# --------------------------------------------------------------------------- ownership
class Owned:
    """Everything this run creates; cleanup() is idempotent and safe to call early."""

    def __init__(self):
        self.root = None
        self.marker = None
        self.sockets = []
        self.procs = []

    def unit_for(self, sock):
        return f'cx-tmux-{hashlib.sha256(str(sock).encode()).hexdigest()}.service'

    def all_sockets(self):
        found = list(self.sockets)
        if self.root and (self.root / 'tmux').is_dir():
            found += [p for p in (self.root / 'tmux').rglob('*') if p.is_socket()]
        return found

    def marked_pids(self):
        if not self.marker:
            return []
        needle = f'CX_CAPTURE_MATRIX_RUN={self.marker}'.encode()
        pids, me, uid = [], os.getpid(), os.getuid()
        for d in pathlib.Path('/proc').iterdir():
            if not d.name.isdigit() or int(d.name) == me:
                continue
            try:
                if d.stat().st_uid != uid:
                    continue
                if needle in (d / 'environ').read_bytes().split(b'\0'):
                    pids.append(int(d.name))
            except OSError:
                continue
        return pids

    def owned_units(self):
        units = []
        for sock in self.sockets:
            unit = self.unit_for(sock)
            try:
                show = subprocess.run(['systemctl', '--user', 'show', '-p', 'LoadState', '-p', 'ExecStart', '--', unit],
                                      capture_output=True, text=True, timeout=10).stdout
            except Exception:
                continue
            if 'LoadState=loaded' in show and str(self.root) in show:
                units.append(unit)
        return units

    def cleanup(self):
        for p in self.procs:
            if p.poll() is None:
                p.kill()
                try:
                    p.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    pass
        # One failed or slow step must never skip the rest (or the rmtree).
        for sock in self.all_sockets():
            try:
                subprocess.run(['tmux', '-S', str(sock), 'kill-server'], stdout=subprocess.DEVNULL,
                               stderr=subprocess.DEVNULL, env={'PATH': SYSTEM_PATH}, timeout=10)
            except (OSError, subprocess.SubprocessError):
                pass
        for unit in self.owned_units():
            try:
                subprocess.run(['systemctl', '--user', 'stop', '--', unit], stdout=subprocess.DEVNULL,
                               stderr=subprocess.DEVNULL, timeout=20)
            except (OSError, subprocess.SubprocessError):
                pass
        for sig in (signal.SIGTERM, signal.SIGKILL):
            pids = self.marked_pids()
            for pid in pids:
                try:
                    os.kill(pid, sig)
                except OSError:
                    pass
            deadline = time.monotonic() + 3
            while pids and time.monotonic() < deadline and self.marked_pids():
                time.sleep(0.1)
        if self.root:
            shutil.rmtree(self.root, ignore_errors=True)

    def leftovers(self):
        time.sleep(1.0)  # a late child could recreate state after removal
        left = dict(pids=len(self.marked_pids()), units=self.owned_units() if self.root else [],
                    root_exists=bool(self.root and self.root.exists()))
        if left['pids'] or left['root_exists']:
            self.cleanup()
            left = dict(pids=len(self.marked_pids()), units=left['units'],
                        root_exists=bool(self.root and self.root.exists()), retried=True)
        return left


# --------------------------------------------------------------------------- fixtures
FIXTURE_RC = r"""PS1='{label}:\W\$ '
PROMPT_COMMAND=
HISTFILE=/dev/null
case "$PWD" in
  */cx-demo) printf '\033[1;32m   Compiling\033[0m cx-demo v0.1.0\n\033[1;32m    Finished\033[0m dev profile in 2.41s\n\033[33mwarning\033[0m: unused variable `x`\n' ;;
  */robot-logs) for i in 1 2 3 4 5 6; do printf '\033[36mrun-05\033[0m step %s \033[32mok\033[0m  lidar=\033[35m1%sHz\033[0m\n' $i $i; done ;;
  */catkin_ws) printf '\033[34mstarted\033[0m roslaunch server http://lab-arm:41011/\nprocess[master]: \033[32mstarted\033[0m\n' ;;
esac
"""


def fixture_shell(path, home, label, marker, rc):
    rc.write_text(FIXTURE_RC.replace('{label}', label))
    # The tmux server runs as a user unit with the user manager's environment, so the
    # pane must not trust inherited HOME/PATH: rebuild a minimal one, keep cx's pane vars.
    keep = 'TERM COLORTERM CX_VIEWER_THEME CX_TMUX_BIN CX_MANAGED_SOCKET CX_MANAGED_SESSION CX_THEME NO_COLOR CX_ASCII'
    path.write_text(f"""#!/bin/sh
cmd= has_cmd=
while [ $# -gt 0 ]; do
  if [ "$1" = -c ]; then shift; cmd=$1; has_cmd=1; fi
  shift
done
set -- env -i HOME={shlex.quote(str(home))} PATH={shlex.quote(str(path.parent) + ':' + SYSTEM_PATH)} \\
  LANG=C.UTF-8 LC_ALL=C.UTF-8 USER=fixture LOGNAME=fixture SHELL={shlex.quote(str(path))} \\
  CX_CAPTURE_MATRIX_RUN={marker}
for v in {keep}; do
  eval "x=\\${{$v-__cx_unset__}}"
  [ "$x" = __cx_unset__ ] || set -- "$@" "$v=$x"
done
if [ -n "$has_cmd" ]; then exec "$@" /bin/bash --noprofile --norc -c "$cmd"; fi
exec "$@" /bin/bash --noprofile --rcfile {shlex.quote(str(rc))} -i
""")
    path.chmod(0o700)


def write_fixture_files(home):
    proj = home / 'projects/cx-demo'
    (proj / 'src').mkdir(parents=True)
    (proj / 'docs').mkdir()
    (proj / '.git').mkdir()
    (proj / 'README.md').write_text(
        '# cx demo\n\nA fixture project for **UI captures**.\n\n## Layout\n\n- `src/main.rs`: entry point\n'
        '- `docs/notes.txt`: notes\n\n```rust\nfn main() {}\n```\n\n> Deterministic fixture content.\n')
    (proj / 'src/main.rs').write_text(
        'use std::env;\n\n/// Entry point for the fixture.\nfn main() {\n    let name = env::args().nth(1).unwrap_or("cx".into());\n'
        '    for i in 0..3 {\n        println!("hello {name} #{i}");\n    }\n}\n')
    (proj / 'Cargo.toml').write_text('[package]\nname = "cx-demo"\nversion = "0.1.0"\nedition = "2021"\n')
    (proj / 'docs/notes.txt').write_text(''.join(f'note {i}: fixture line\n' for i in range(40)))
    (proj / 'data.bin').write_bytes(bytes(range(256)) * 64)
    (home / 'projects/robot-logs').mkdir()
    for i in range(6):
        (home / f'projects/robot-logs/run-{i:02}.log').write_text(f'run {i}\n' * (i + 1) * 30)
    (home / 'Downloads').mkdir()
    (home / 'Downloads/firmware-v2.tar.gz').write_bytes(b'\0' * 4096)
    # Fixed mtimes keep size/age columns stable between runs (relative ages still move).
    stamp = time.time() - 3 * 3600
    for p in sorted(home.rglob('*'), reverse=True):
        os.utime(p, (stamp, stamp), follow_symlinks=False)


def stub(path, body):
    path.write_text('#!/bin/sh\n' + body)
    path.chmod(0o700)


def build_fixture(owned, a):
    root = owned.root
    home, remote = root / 'home', root / 'remote'
    bindir = root / 'bin'
    for d in (home, remote, bindir, root / 'tmux', root / 'shells/local', root / 'shells/remote'):
        d.mkdir(parents=True, mode=0o700)
    os.chmod(root / 'tmux', 0o700)
    for h in (home, remote):
        (h / '.local/bin').mkdir(parents=True)
        (h / '.local/bin/cx').symlink_to(a.bin)
        (root / f'state-{h.name}/cx').mkdir(parents=True)
    # Register the managed sockets before any tmux server can exist.
    owned.sockets += [root / 'state-home/cx/managed.sock', root / 'state-remote/cx/managed.sock']
    write_fixture_files(home)
    (remote / 'catkin_ws/src').mkdir(parents=True)
    fixture_shell(root / 'shells/local/bash', home, 'fixture', owned.marker, root / 'shells/local/rc')
    fixture_shell(root / 'shells/remote/bash', remote, 'robot', owned.marker, root / 'shells/remote/rc')

    q = shlex.quote
    renv_prefix = (f"HOME={q(str(remote))} XDG_STATE_HOME={q(str(root / 'state-remote'))} "
                   f"SHELL={q(str(root / 'shells/remote/bash'))} USER=robot LOGNAME=robot")
    stub(bindir / 'ssh', f"""for arg do last="$arg"; done
case "$*" in
  *lab-arm.invalid*)
    [ "$last" = 'exec ~/.local/bin/cx helper' ] || exit 1
    exec env {renv_prefix} {q(a.bin)} helper ;;
esac
echo "ssh: connect to host: Connection refused" >&2
exit 255
""")
    stub(bindir / 'ping', 'echo "64 bytes from fixture: icmp_seq=1 ttl=57 time=12.3 ms"\n')
    stub(bindir / 'timeout', 'exit 1\n')  # cx uses it only for bounded reverse name lookups here
    stub(bindir / 'nmcli', 'echo "nmcli tool, version 1.0.0-fixture"\n')
    for engine in ('docker', 'podman'):
        stub(bindir / engine, 'exit 1\n')

    uid = os.getuid()
    env = {
        'PATH': f'{bindir}:{SYSTEM_PATH}',
        'HOME': str(home), 'XDG_STATE_HOME': str(root / 'state-home'), 'XDG_CACHE_HOME': str(root / 'cache'),
        'XDG_CONFIG_HOME': str(root / 'config'),
        'LANG': 'C.UTF-8', 'LC_ALL': 'C.UTF-8',
        'TERM': 'xterm-256color',
        'SHELL': str(root / 'shells/local/bash'), 'USER': 'fixture', 'LOGNAME': 'fixture',
        # explicit test variables
        'TMUX_TMPDIR': str(root / 'tmux'),
        'XDG_RUNTIME_DIR': os.environ.get('XDG_RUNTIME_DIR', f'/run/user/{uid}'),
        'HTTPS_PROXY': 'http://127.0.0.1:9', 'HTTP_PROXY': 'http://127.0.0.1:9', 'ALL_PROXY': 'http://127.0.0.1:9',
        'NO_PROXY': '', 'CX_CAPTURE_MATRIX_RUN': owned.marker,
        'CX_THEME': a.theme, 'COLORFGBG': '0;15' if a.theme == 'light' else '15;0',
    }
    if a.ascii:
        env['CX_ASCII'] = '1'
    if a.nocolor:
        env['NO_COLOR'] = '1'
    env.update(a.extra_env)
    renv = dict(env, HOME=str(remote), XDG_STATE_HOME=str(root / 'state-remote'),
                SHELL=str(root / 'shells/remote/bash'), USER='robot', LOGNAME='robot')
    (root / 'state-home/cx/devices.json').write_text(json.dumps([
        {'id': 'local', 'name': 'laptop', 'target': None, 'account': 'fixture', 'host': 'laptop', 'status': 'local', 'observed_at': 0},
        {'id': 'lab-arm', 'name': 'lab-arm', 'target': 'lab-arm.invalid', 'account': 'robot', 'host': 'lab-arm', 'status': 'unknown', 'observed_at': 0},
        {'id': 'field-rover', 'name': 'field-rover', 'target': 'field-rover.invalid', 'account': 'robot', 'host': 'field-rover', 'status': 'unknown', 'observed_at': 0}]))

    def cli(e, *args):
        p = subprocess.Popen([a.bin, *args], env=e, cwd=e['HOME'], stdout=subprocess.PIPE,
                             stderr=subprocess.DEVNULL, stdin=subprocess.DEVNULL, start_new_session=True)
        owned.procs.append(p)
        out, _ = p.communicate(timeout=30)
        if p.returncode != 0:
            raise RuntimeError(f'cx {args[0]} failed with {p.returncode}')
        return json.loads(out)

    cli(env, 'new', '--directory', str(home / 'projects/cx-demo'), '--key', 'matrix-build', '--name', 'build watcher')
    cli(env, 'new', '--directory', str(home / 'projects/robot-logs'), '--key', 'matrix-logs', '--name', 'lidar logs')
    cli(renv, 'new', '--directory', str(remote / 'catkin_ws'), '--key', 'matrix-remote', '--name', 'ros core')
    time.sleep(1.0)
    return env


# --------------------------------------------------------------------------- driving
# Each view: list of (bytes to send, seconds to read afterwards). Every view starts
# from a fresh cx process, so recipes never depend on another view's leftovers.
# Keep recipes short and data-only: UI keymap changes should only need edits here.
ESC, CTRL_P = b'\x1b', b'\x10'
RECIPES = {
    # The hub is the start view; Enter opens the selected node (this device), `f` its
    # files (in the launch folder), Tab the hub's sessions zone. Letters follow the
    # fixture names: laptop is `p`, lab-arm `b` (see src/ui/hotkeys.rs).
    'startup': [],  # read at 2.5s, before the device probes settle (P10's error toast)
    'device': [(b'\r', 1.5)],
    'files': [(b'\r', .8), (b'f', 1.5)],
    'files-projects': [(b'\r', .8), (b'f', 1.5), (b'j', .3), (b'\r', 1.2)],
    'files-inside': [(b'\r', .8), (b'f', 1.5), (b'j', .3), (b'\r', 1.2), (b'\r', 1.2)],
    'files-preview': [(b'\r', .8), (b'f', 1.5), (b'j', .3), (b'\r', 1.2), (b'\r', 1.2), (b'G', .3), (b'k', .3),
                      (b'\r', 1.5)],
    'sessions': [(b'\t', 1.0)],
    'sessions-all': [(b'b', 1.5)],
    'sessions-next': [(b'\t', .8), (b'j', 1.0), (b' ', 1.5)],
    'edge': [(b'e', .3), (b'b', 1.5)],
    'keys': [(b'K', 1.0)],
    # Refresh observes the network (and checks LAN neighbours); they show in the palette.
    'network': [(b'/', .5), (b'Refresh', .5), (b'\r', 3.0)],
    'network-expanded': [(b'/', .5), (b'Refresh', .5), (b'\r', 3.0), (b'/', .5)],
    'help': [(b'?', .8)],
    'palette': [(b'/', .8)],
}


def capture(a, env, owned, out, view, cols, rows):
    term = pty_lib.Pty([a.bin], env, env['HOME'], cols, rows)
    owned.procs.append(term.proc)
    try:
        def read(seconds):
            end = time.monotonic() + seconds
            while time.monotonic() < end:
                term.read(.03)

        read(2.5 if view == 'startup' else 5.0)  # device probes (online lab-arm, refused field-rover) settle
        for keys, wait in RECIPES[view]:
            term.write(keys)
            read(wait)
        read(0.6)
        masker = Masker(owned.root, host_identifiers())  # fresh snapshot: names that appeared mid-run
        masker.apply(term.screen, cols, rows)
        name = f'{view}-{cols}x{rows}'
        text = term.text() + '\n'
        (out / f'{name}.txt').write_text(text)
        pty_lib.png(term.screen, out / f'{name}.png', a.theme)
        return dict(view=view, cols=cols, rows=rows, txt=f'{name}.txt', png=f'{name}.png',
                    terminal_palette=a.theme, ascii=a.ascii, nocolor=a.nocolor, unmasked=masker.leaks(text))
    finally:
        term.close()


def self_test():
    """Masking, argument and cleanup checks with synthetic identifiers; needs no cx, tmux or network."""
    root = pathlib.Path('/tmp/cxcap-selftest')
    idents = ({'lo', 'wg0', 'br0', 'tun0', 'eth0', 'CloudflareWARP'}, {'10.9.8.7', 'fe80::1'}, {'fixturehost'})
    m = Masker(root, idents)
    ifmask = {t: r for t, r in m.literal if t in idents[0]}
    assert ifmask == {'CloudflareWARP': 'if0xxxxxxxxxxx', 'eth0': 'if1x', 'tun0': 'if2x', 'br0': 'if3', 'wg0': 'if4'}, ifmask
    masked = ['fe80::1', '2001::1', '2001:db8::1', 'fe80::a00:27ff:fe4e:66a1', '::1', 'aa:bb:cc:dd:ee:ff',
              '10.0.0.1', 'host.local', 'eth0', 'fixturehost']
    kept = ['12:30:45', 'ver 1:2:3', 'use std::env;', 'env::args()', '0:0', 'cx 0.1.95', 'a:b:c']
    for text in masked + kept:
        screen = pty_lib.Screen(40, 1)
        pyte.Stream(screen).feed(text)
        m.apply(screen, 40, 1)
        out = screen.display[0][:len(text)]
        if text in masked:
            assert out != text and len(screen.display[0].rstrip()) == len(text), (text, out)  # same length
        else:
            assert out == text, (text, out)
    assert m.leaks('dev if0xxxxxxxxxxx xx.x.x.x\n') == 0
    assert m.leaks('dev Cloudflare…\n') == 1  # truncated name
    assert m.leaks('dev Cloudflare...\n') == 1  # ASCII truncation
    assert m.leaks('dev ' + ' ' * 30 + 'Cloudf\nlareWARP\n') == 1  # wrapped at the screen edge
    assert m.leaks('cxcap-…  /tmp/cxcap-self\n') == 0  # the temp root is not a host identifier
    exe = sys.executable
    assert parse_args(['--bin', exe]).views == DEFAULT_VIEWS and not set(NETWORK_VIEWS) & set(DEFAULT_VIEWS)
    assert parse_args(['--bin', exe, '--network']).views == DEFAULT_VIEWS + NETWORK_VIEWS
    assert parse_args(['--bin', exe, '--sizes', '80x24,40x20']).sizes == [(80, 24), (40, 20)]
    for bad in (['--sizes', '80x'], ['--sizes', '80'], ['--views', 'nope'], ['legacy-bin']):
        try:
            with open(os.devnull, 'w') as null:
                stderr, sys.stderr = sys.stderr, null
                try:
                    parse_args(['--bin', exe, *bad])
                finally:
                    sys.stderr = stderr
        except SystemExit:
            continue
        raise AssertionError(f'accepted {bad}')
    # A hung kill-server must not skip the remaining cleanup steps or the rmtree.
    owned, run = Owned(), subprocess.run
    owned.root = pathlib.Path(tempfile.mkdtemp(prefix='cxcap-selftest-', dir='/tmp'))
    owned.sockets = [owned.root / 'missing.sock']
    calls = []

    def fake_run(cmd, **kw):
        calls.append(cmd[0])
        if cmd[0] == 'tmux':
            raise subprocess.TimeoutExpired(cmd, kw.get('timeout'))
        return subprocess.CompletedProcess(cmd, 0, f'LoadState=loaded\nExecStart={owned.root}\n', '')
    subprocess.run = fake_run
    try:
        owned.cleanup()
    finally:
        subprocess.run = run
    assert calls == ['tmux', 'systemctl', 'systemctl'] and not owned.root.exists(), (calls, owned.root.exists())
    print('capture_matrix self-test: PASS')
    return 0


def main(argv):
    if argv == ['--self-test']:
        return self_test()
    a = parse_args(argv)
    out = pathlib.Path(a.out).resolve()
    out.mkdir(parents=True, exist_ok=True)
    owned = Owned()
    # SIGTERM/SIGHUP take the same finally path as Ctrl+C.
    def interrupt(*_):
        raise KeyboardInterrupt
    for sig in (signal.SIGTERM, signal.SIGHUP):
        signal.signal(sig, interrupt)
    index, left, failure, version = [], None, None, None
    try:
        owned.root = pathlib.Path(tempfile.mkdtemp(prefix='cxcap-', dir='/tmp'))
        owned.marker = owned.root.name
        env = build_fixture(owned, a)
        version = subprocess.run([a.bin, '--version'], env=env, capture_output=True, text=True,
                                 timeout=10).stdout.strip()[:80]
        for cols, rows in a.sizes:
            for view in a.views:
                index.append(capture(a, env, owned, out, view, cols, rows))
    except BaseException as e:  # still clean up and report
        failure = f'{type(e).__name__}: {e}'
    finally:
        for sig in (signal.SIGINT, signal.SIGTERM, signal.SIGHUP):
            signal.signal(sig, signal.SIG_IGN)  # a second signal must not cut cleanup short
        owned.cleanup()
        left = owned.leftovers()
    meta = dict(harness='tests/capture_matrix.py', bin=os.path.relpath(a.bin, REPO) if a.bin.startswith(str(REPO)) else
                pathlib.Path(a.bin).name, cx_version=version, terminal_palette=a.theme, ascii=a.ascii,
                nocolor=a.nocolor, env=sorted(a.extra_env), sizes=[f'{c}x{r}' for c, r in a.sizes], views=a.views,
                captures=index)
    (out / 'index.json').write_text(json.dumps(meta, indent=1) + '\n')
    leaks = sum(c['unmasked'] for c in index)
    ok = failure is None and not leaks and not left['pids'] and not left['units'] and not left['root_exists']
    print(json.dumps(dict(result='PASS' if ok else 'FAIL', captures=len(index), out=str(out),
                          failure=failure, unmasked=leaks, leftovers=left)))
    return 0 if ok else 1


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
