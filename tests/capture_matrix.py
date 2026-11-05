#!/usr/bin/env python3
"""Capture the real cx UI across a size x view matrix for design/critic passes.

  python tests/capture_matrix.py [--bin target/debug/cx] [--out DIR]
      [--sizes 40x20,80x24,120x40,200x60]
      [--views sessions,files,files-preview,network,help,palette]
      [--theme dark|light] [--ascii] [--nocolor] [--env K=V ...]

Needs pyte and pillow (for example the worktrees .venv-test interpreter,
or `uv run --with pyte --with pillow python ...`). Writes <view>-<cols>x<rows>.txt
and .png per capture plus index.json into --out (default
local-evidence/ui-passes/latest). Every view is captured from a fresh cx process.

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
    interface name, address, MAC, *.local name, user and hostname. Note: the
    network/network-expanded views run cx's normal automatic neighbour check (a
    bounded TCP connect to port 22 of current LAN neighbour-cache entries, no
    payload, no auth); there is no fixture hook for it yet. Omit those views with
    --views to avoid it.
  * cleanup is registered before anything is created: every managed tmux socket,
    every socket in the private TMUX_TMPDIR, the cx-tmux-<sha256(socket)> units of
    our sockets (only if their ExecStart names our temp root), and every process
    carrying this run's marker variable are stopped in a finally; the temp root is
    removed and re-checked. The result line reports leftovers.
Never runs ~/.local/bin/cx; never touches the user's state, tmux servers, SSH or network config.
"""
import argparse, fcntl, getpass, hashlib, json, os, pathlib, pty, re, select, shlex, shutil, signal
import socket, struct, subprocess, sys, tempfile, termios, time

REPO = pathlib.Path(__file__).resolve().parent.parent
DEFAULT_VIEWS = ['sessions', 'files', 'files-preview', 'network', 'help', 'palette']
ALL_VIEWS = DEFAULT_VIEWS + ['network-expanded']
SYSTEM_PATH = '/usr/local/sbin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin'


def parse_args(argv):
    p = argparse.ArgumentParser(description=__doc__.split('\n')[0])
    p.add_argument('legacy', nargs='*', help=argparse.SUPPRESS)  # old form: BIN [OUT]
    p.add_argument('--bin', help='cx binary (default target/debug/cx)')
    p.add_argument('--out', help='output directory')
    p.add_argument('--sizes', default='40x20,80x24,120x40,200x60')
    p.add_argument('--views', default=','.join(DEFAULT_VIEWS), help=f'comma list from {ALL_VIEWS}')
    p.add_argument('--theme', choices=['dark', 'light'], default='dark')
    p.add_argument('--ascii', action='store_true', help='set CX_ASCII=1')
    p.add_argument('--nocolor', action='store_true', help='set NO_COLOR=1')
    p.add_argument('--merge-index', action='store_true',
                   help='keep other captures already listed in DIR/index.json (partial reruns)')
    p.add_argument('--env', action='append', default=[], metavar='K=V', help='extra explicit test variable')
    p.add_argument('--network', action='store_true', help=argparse.SUPPRESS)  # old flag; network is a view now
    a = p.parse_args(argv)
    legacy = list(a.legacy)
    a.bin = a.bin or (legacy.pop(0) if legacy else str(REPO / 'target/debug/cx'))
    a.out = a.out or (legacy.pop(0) if legacy else str(REPO / 'local-evidence/ui-passes/latest'))
    if legacy:
        p.error(f'unexpected arguments: {legacy}')
    a.bin = str(pathlib.Path(a.bin).resolve())
    if not os.access(a.bin, os.X_OK):
        p.error(f'not an executable: {a.bin}')
    a.sizes = [tuple(int(n) for n in s.split('x')) for s in a.sizes.split(',') if s]
    a.views = [v for v in a.views.split(',') if v]
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


# --------------------------------------------------------------------------- rendering
DARK = dict(bg='#171a1e', palette={
    'default': '#cbd0d7', 'black': '#2a2e35', 'red': '#dc817d', 'green': '#91b49b', 'brown': '#c7b887',
    'yellow': '#c7b887', 'blue': '#8ca7cb', 'magenta': '#b29cc8', 'cyan': '#8cbabd', 'white': '#d6dce4',
    'brightblack': '#5c6370', 'brightred': '#e6908c', 'brightgreen': '#a3c7ad', 'brightyellow': '#d8ca99',
    'brightblue': '#9db8dc', 'brightmagenta': '#c3add9', 'brightcyan': '#9dcbce', 'brightwhite': '#eef1f5'})
LIGHT = dict(bg='#fafafa', palette={
    'default': '#2b2f36', 'black': '#2b2f36', 'red': '#b4232c', 'green': '#2f7d3b', 'brown': '#8a6a00',
    'yellow': '#8a6a00', 'blue': '#1c64b8', 'magenta': '#8a3ea8', 'cyan': '#0f7480', 'white': '#d0d4da',
    'brightblack': '#6b7280', 'brightred': '#c9343c', 'brightgreen': '#3a8f47', 'brightyellow': '#9c7a00',
    'brightblue': '#2f74c8', 'brightmagenta': '#9b4fb9', 'brightcyan': '#1a8592', 'brightwhite': '#ffffff'})

BOX = {}
for chars, segs in [('─━═', 'lr'), ('│┃║', 'ud'), ('╭┌┏╔', 'rd'), ('╮┐┓╗', 'ld'), ('╰└┗╚', 'ru'), ('╯┘┛╝', 'lu'),
                    ('├┣╠', 'udr'), ('┤┫╣', 'udl'), ('┬┳╦', 'lrd'), ('┴┻╩', 'lru'), ('┼╋╬', 'lrud')]:
    for ch in chars:
        BOX[ch] = (segs, 3 if ch in '━┃┏┓┗┛┣┫┳┻╋═║╔╗╚╝╠╣╦╩╬' else 1)


class Renderer:
    def __init__(self, theme):
        from PIL import ImageFont
        self.scheme = LIGHT if theme == 'light' else DARK
        self.fonts = {}
        self.ImageFont = ImageFont

    def font(self, bold):
        if bold not in self.fonts:
            try:
                path = subprocess.check_output(['fc-match', '-f', '%{file}', f'monospace:{"bold" if bold else "regular"}'],
                                               text=True, timeout=10)
                self.fonts[bold] = self.ImageFont.truetype(path, 16)
            except Exception:
                self.fonts[bold] = self.ImageFont.load_default()
        return self.fonts[bold]

    def color(self, name, fallback):
        pal = self.scheme['palette']
        if name == 'default':
            return fallback
        if name in pal:
            return pal[name]
        if len(name) == 6 and re.fullmatch(r'[0-9a-fA-F]{6}', name):
            return '#' + name
        return fallback

    def png(self, screen, cols, rows, path):
        from PIL import Image, ImageDraw
        bg0, fg0 = self.scheme['bg'], self.scheme['palette']['default']
        cw, ch, pad = 10, 21, 12
        im = Image.new('RGB', (cols * cw + 2 * pad, rows * ch + 2 * pad), bg0)
        d = ImageDraw.Draw(im)
        for y in range(rows):
            line = screen.buffer[y]
            for x in range(cols):
                c = line[x]
                fg, bg = self.color(c.fg, fg0), self.color(c.bg, bg0)
                if c.reverse:
                    fg, bg = bg, fg
                x0, y0 = pad + x * cw, pad + y * ch
                if bg != bg0:
                    d.rectangle((x0, y0, x0 + cw - 1, y0 + ch - 1), fill=bg)
                if c.data in BOX:
                    segs, w = BOX[c.data]
                    mx, my = x0 + cw // 2, y0 + ch // 2
                    for s, coords in (('l', (x0, my, mx, my)), ('r', (mx, my, x0 + cw, my)),
                                      ('u', (mx, y0, mx, my)), ('d', (mx, my, mx, y0 + ch))):
                        if s in segs:
                            d.line(coords, fill=fg, width=w)
                elif c.data.strip():
                    d.text((x0, y0), c.data, font=self.font(c.bold), fill=fg)
                if c.underscore:
                    d.line((x0, y0 + ch - 2, x0 + cw - 1, y0 + ch - 2), fill=fg)
        if screen.cursor and not screen.cursor.hidden:
            x, y = screen.cursor.x, screen.cursor.y
            if x < cols and y < rows:
                d.rectangle((pad + x * cw, pad + y * ch, pad + (x + 1) * cw - 1, pad + (y + 1) * ch - 1),
                            outline=fg0)
        im.save(path)


# --------------------------------------------------------------------------- masking
class Masker:
    """Same-length masks, so masking never shifts or pads the layout."""

    def __init__(self, root):
        self.literal = []  # (text, replacement)
        # The temp root is the only random part of fixture paths; mask its suffix in place.
        name = root.name
        self.literal.append((name, re.sub(r'(?<=cxcap-)\w+$', lambda m: 'x' * len(m.group()), name)))
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
        # Longest first so a name that contains another is masked whole.
        for i, n in enumerate(sorted((n for n in ifnames if len(n) >= 2 and n != 'lo'), key=len, reverse=True)):
            self.literal.append((n, f'if{i}'.ljust(len(n), 'x')[:len(n)]))
        for a in sorted((a for a in addrs if len(a) >= 3 and a not in ('default',)), key=len, reverse=True):
            self.literal.append((a, ''.join(c if c in '.:/' else 'x' for c in a)))
        ids = {getpass.getuser(), socket.gethostname().split('.')[0], socket.gethostname()}
        try:
            ids.add(pathlib.Path('/proc/sys/kernel/hostname').read_text().strip())
        except OSError:
            pass
        for w in sorted((w for w in ids if len(w) > 2), key=len, reverse=True):
            self.literal.append((w, 'x' * len(w)))
        self.regex = [re.compile(p, re.I) for p in [
            r'\b(?:\d{1,3}\.){3}\d{1,3}\b',
            r'(?<![\w:])(?:[0-9a-f]{1,4}:){2,7}:?(?::?[0-9a-f]{1,4}){1,7}\b',
            r'(?<![\w:])(?:[0-9a-f]{1,4}:){1,7}:(?![\w:])',
            r'\b(?:[0-9a-f]{2}:){5}[0-9a-f]{2}\b',
            r'\b[\w-]+\.(?:local|lan|home|internal)\b']]

    @staticmethod
    def bounded(text):
        return re.compile(r'(?<![A-Za-z0-9])' + re.escape(text) + r'(?![A-Za-z0-9])')

    def spans(self, row):
        hits = []
        for text, rep in self.literal:
            for m in self.bounded(text).finditer(row):
                hits.append((m.start(), rep))
        for pattern in self.regex:
            for m in pattern.finditer(row):
                if m.group() in ('0:0', ):
                    continue
                hits.append((m.start(), ''.join(c if c in '.:-/' else 'x' for c in m.group())))
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
        """How many host identifiers survived masking (never the values themselves)."""
        return sum(1 for t, rep in self.literal if t != rep and self.bounded(t).search(text))


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
        for sock in self.all_sockets():
            subprocess.run(['tmux', '-S', str(sock), 'kill-server'], stdout=subprocess.DEVNULL,
                           stderr=subprocess.DEVNULL, env={'PATH': SYSTEM_PATH}, timeout=10)
        for unit in self.owned_units():
            subprocess.run(['systemctl', '--user', 'stop', '--', unit], stdout=subprocess.DEVNULL,
                           stderr=subprocess.DEVNULL, timeout=20)
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
def make_screen(cols, rows):
    import pyte

    class S(pyte.Screen):
        def report_device_status(self, *args, **kwargs):
            pass

        def set_title(self, *args, **kwargs):
            pass

    screen = S(cols, rows)
    return screen, pyte.Stream(screen)


# Each view: list of (bytes to send, seconds to read afterwards). Every view starts
# from a fresh cx process, so recipes never depend on another view's leftovers.
# Keep recipes short and data-only: UI keymap changes should only need edits here.
ESC, CTRL_P = b'\x1b', b'\x10'
RECIPES = {
    'files': [],
    'files-preview': [(b'j', .3), (b'\r', 1.2), (b'\r', 1.2), (b'G', .3), (b'k', .3), (b'\r', 1.5)],
    'sessions': [(ESC, .8), (ESC, .4), (b'gg', .4), (b'\r', 1.5)],
    'network': [(CTRL_P, .5), (b'Network', .5), (b'\r', 1.0), (b'\r', 2.5)],
    'network-expanded': [(CTRL_P, .5), (b'Network', .5), (b'\r', 1.0), (b'\r', 2.5), (b'j', 1.0), (b'l', 1.0)],
    'help': [(b'?', .8)],
    'palette': [(CTRL_P, .8)],
}


def capture(a, env, owned, masker, renderer, out, view, cols, rows):
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', rows, cols, 0, 0))

    def control():
        os.setsid()
        fcntl.ioctl(slave, termios.TIOCSCTTY, 0)

    proc = None
    try:
        proc = subprocess.Popen([a.bin], stdin=slave, stdout=slave, stderr=slave, env=env, cwd=env['HOME'],
                                preexec_fn=control)
        owned.procs.append(proc)
        screen, stream = make_screen(cols, rows)

        def read(seconds):
            end = time.monotonic() + seconds
            while time.monotonic() < end:
                if select.select([master], [], [], .03)[0]:
                    try:
                        stream.feed(os.read(master, 65536).decode('utf-8', 'replace'))
                    except OSError:
                        break

        read(5.0)  # startup: device probes (online lab-arm, refused field-rover) settle
        for keys, wait in RECIPES[view]:
            os.write(master, keys)
            read(wait)
        read(0.6)
        masker.apply(screen, cols, rows)
        name = f'{view}-{cols}x{rows}'
        text = '\n'.join(screen.display) + '\n'
        (out / f'{name}.txt').write_text(text)
        renderer.png(screen, cols, rows, out / f'{name}.png')
        return dict(view=view, cols=cols, rows=rows, txt=f'{name}.txt', png=f'{name}.png', theme=a.theme,
                    ascii=a.ascii, nocolor=a.nocolor, unmasked=masker.leaks(text))
    finally:
        if proc is not None:
            try:
                os.write(master, b'\x03')
                proc.wait(timeout=5)
            except (OSError, subprocess.TimeoutExpired):
                proc.kill()
                proc.wait()
        os.close(master)
        os.close(slave)


def main(argv):
    a = parse_args(argv)
    try:
        import pyte  # noqa: F401
        import PIL  # noqa: F401
    except ImportError as e:
        sys.exit(f'capture_matrix needs pyte and pillow: {e}')
    out = pathlib.Path(a.out).resolve()
    out.mkdir(parents=True, exist_ok=True)
    renderer = Renderer(a.theme)
    owned = Owned()
    # SIGTERM/SIGHUP take the same finally path as Ctrl+C.
    for sig in (signal.SIGTERM, signal.SIGHUP):
        signal.signal(sig, lambda *_: (_ for _ in ()).throw(KeyboardInterrupt()))
    index, left, failure = [], None, None
    try:
        owned.root = pathlib.Path(tempfile.mkdtemp(prefix='cxcap-', dir='/tmp'))
        owned.marker = owned.root.name
        masker = Masker(owned.root)
        env = build_fixture(owned, a)
        for cols, rows in a.sizes:
            for view in a.views:
                index.append(capture(a, env, owned, masker, renderer, out, view, cols, rows))
    except BaseException as e:  # still clean up and report
        failure = f'{type(e).__name__}: {e}'
    finally:
        owned.cleanup()
        left = owned.leftovers()
    captures = index
    if a.merge_index:
        try:
            old = json.loads((out / 'index.json').read_text())
            old = old.get('captures', []) if isinstance(old, dict) else old
            mine = {c['txt'] for c in index}
            captures = [c for c in old if isinstance(c, dict) and c.get('txt') not in mine] + index
        except (OSError, ValueError):
            pass
    meta = dict(harness='tests/capture_matrix.py', bin=os.path.relpath(a.bin, REPO) if a.bin.startswith(str(REPO)) else
                pathlib.Path(a.bin).name, theme=a.theme, ascii=a.ascii, nocolor=a.nocolor, env=sorted(a.extra_env),
                sizes=[f'{c}x{r}' for c, r in a.sizes], views=a.views, captures=captures)
    (out / 'index.json').write_text(json.dumps(meta, indent=1) + '\n')
    leaks = sum(c['unmasked'] for c in index)
    ok = failure is None and not leaks and not left['pids'] and not left['units'] and not left['root_exists']
    print(json.dumps(dict(result='PASS' if ok else 'FAIL', captures=len(index), out=str(out),
                          failure=failure, unmasked=leaks, leftovers=left)))
    return 0 if ok else 1


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
