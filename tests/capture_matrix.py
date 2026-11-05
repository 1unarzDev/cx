#!/usr/bin/env python3
"""Capture the real cx UI across a size x view matrix for design review passes.

Usage: uv run --with pyte --with pillow python tests/capture_matrix.py target/debug/cx [out_dir]
       [--sizes 40x20,80x24] [--env K=V] [--network]

Everything runs under an owned temporary HOME/XDG_STATE_HOME:
  * two CX-managed local shell sessions (private managed tmux socket, killed at exit),
  * a fake `ssh` on PATH that serves one "online" remote by exec'ing this binary's
    helper under a second private HOME, and refuses one "offline" remote; no network,
  * a small files tree in the local HOME.
--network adds the real Network view, which runs cx's normal bounded read-only host
observation (ip, ping, SSH-port TCP checks of LAN neighbours); IPs, MACs, *.local names,
the real user and hostname are masked in every capture. Off by default.
Writes <view>-<cols>x<rows>.txt and .png per capture plus index.json. Never runs
~/.local/bin/cx, never touches the user's real state, tmux server, SSH config or network.
"""
import fcntl, getpass, json, os, pathlib, pty, re, select, shlex, socket, struct, subprocess, sys, tempfile, termios, time
import pyte
from PIL import Image, ImageDraw, ImageFont

args = [a for a in sys.argv[1:]]
if not args:
    sys.exit(__doc__)
binary = str(pathlib.Path(args.pop(0)).resolve())
sizes = [(40, 20), (80, 24), (120, 40), (200, 60)]
extra_env = {}
out = None
network = False
while args:
    a = args.pop(0)
    if a == '--sizes':
        sizes = [tuple(int(n) for n in s.split('x')) for s in args.pop(0).split(',')]
    elif a == '--network':
        network = True  # opt-in: the real Network view runs cx's bounded host observation
    elif a == '--env':
        k, v = args.pop(0).split('=', 1)
        extra_env[k] = v
    else:
        out = pathlib.Path(a)
repo = pathlib.Path(__file__).resolve().parent.parent
out = (out or repo / 'local-evidence/ui-passes/pass0').resolve()
out.mkdir(parents=True, exist_ok=True)

PALETTE = {'default': '#cbd0d7', 'black': '#2a2e35', 'red': '#dc817d', 'green': '#91b49b',
           'brown': '#c7b887', 'yellow': '#c7b887', 'blue': '#8ca7cb', 'magenta': '#b29cc8',
           'cyan': '#8cbabd', 'white': '#d6dce4', 'brightblack': '#5c6370', 'brightred': '#e6908c',
           'brightgreen': '#a3c7ad', 'brightyellow': '#d8ca99', 'brightblue': '#9db8dc',
           'brightmagenta': '#c3add9', 'brightcyan': '#9dcbce', 'brightwhite': '#eef1f5'}
BG = '#171a1e'


def color(name, fallback):
    if name == 'default':
        return fallback
    if name in PALETTE:
        return PALETTE[name]
    if len(name) == 6:
        return '#' + name
    return fallback


def font_path(style):
    try:
        return subprocess.check_output(['fc-match', '-f', '%{file}', f'monospace:{style}'], text=True)
    except Exception:
        return None


FONTS = {}


def font(bold):
    if bold not in FONTS:
        path = font_path('bold' if bold else 'regular')
        FONTS[bold] = ImageFont.truetype(path, 16) if path else ImageFont.load_default()
    return FONTS[bold]


# Box-drawing glyphs rarely fill a cell; draw them as cell-spanning strokes (left, right, up, down, weight).
BOX = {}
for chars, segs in [('─━', 'lr'), ('│┃', 'ud'), ('╭┌┏', 'rd'), ('╮┐┓', 'ld'), ('╰└┗', 'ru'), ('╯┘┛', 'lu'),
                    ('├┣', 'udr'), ('┤┫', 'udl'), ('┬┳', 'lrd'), ('┴┻', 'lru'), ('┼╋', 'lrud')]:
    for i, ch in enumerate(chars):
        BOX[ch] = (segs, 3 if ch in '━┃┏┓┗┛┣┫┳┻╋' else 1)


def box(d, spec, x0, y0, cw, ch, fg):
    segs, w = spec
    cx, cy = x0 + cw // 2, y0 + ch // 2
    if 'l' in segs:
        d.line((x0, cy, cx, cy), fill=fg, width=w)
    if 'r' in segs:
        d.line((cx, cy, x0 + cw, cy), fill=fg, width=w)
    if 'u' in segs:
        d.line((cx, y0, cx, cy), fill=fg, width=w)
    if 'd' in segs:
        d.line((cx, cy, cx, y0 + ch), fill=fg, width=w)


def render_png(screen, cols, rows, path):
    cw, ch, pad = 10, 21, 12
    im = Image.new('RGB', (cols * cw + 2 * pad, rows * ch + 2 * pad), BG)
    d = ImageDraw.Draw(im)
    for y in range(rows):
        line = screen.buffer[y]
        for x in range(cols):
            c = line[x]
            fg = color(c.fg, PALETTE['default'])
            bg = color(c.bg, BG)
            if c.reverse:
                fg, bg = bg, fg
            if bg != BG:
                d.rectangle((pad + x * cw, pad + y * ch, pad + (x + 1) * cw - 1, pad + (y + 1) * ch - 1), fill=bg)
            if c.data in BOX:
                box(d, BOX[c.data], pad + x * cw, pad + y * ch, cw, ch, fg)
            elif c.data.strip():
                d.text((pad + x * cw, pad + y * ch), c.data, font=font(c.bold), fill=fg)
            if c.underscore:
                d.line((pad + x * cw, pad + (y + 1) * ch - 2, pad + (x + 1) * cw - 1, pad + (y + 1) * ch - 2), fill=fg)
    if screen.cursor and not screen.cursor.hidden:
        x, y = screen.cursor.x, screen.cursor.y
        if x < cols and y < rows:
            d.rectangle((pad + x * cw, pad + y * ch, pad + (x + 1) * cw - 1, pad + (y + 1) * ch - 1), outline='#e0e0e0')
    im.save(path)


# Captures are committed: mask host-identifying data the real observation may show.
REDACT = [re.compile(p, re.I) for p in [
    r'\b(?:\d{1,3}\.){3}\d{1,3}\b',
    r'(?<![\w:])(?:[0-9a-f]{1,4}:){1,7}:?(?::?[0-9a-f]{1,4}){1,7}\b',
    r'\b(?:[0-9a-f]{2}:){5}[0-9a-f]{2}\b',
    r'\b[\w-]+\.(?:local|lan|home)\b',
    r'cx-capture-matrix-\w+']] + [re.compile(re.escape(w)) for w in {getpass.getuser(), socket.gethostname().split('.')[0]} if len(w) > 2]


def redact(screen, cols, rows):
    for y in range(rows):
        line = screen.buffer[y]
        row = ''.join((line[x].data or ' ')[:1] for x in range(cols))
        for pattern in REDACT:
            for m in pattern.finditer(row):
                mask = 'tmp-xxxxxx' if m.group().startswith('cx-capture') else None
                for x in range(m.start(), m.end()):
                    ch = row[x]
                    if mask:
                        new = mask[x - m.start()] if x - m.start() < len(mask) else ' '
                    else:
                        new = ch if ch in '.:-/' else 'x'
                    line[x] = line[x]._replace(data=new)


class Screen(pyte.Screen):
    def report_device_status(self, *a, **k):
        pass


def write_fixture_files(home):
    proj = home / 'projects/cx-demo'
    (proj / 'src').mkdir(parents=True)
    (proj / 'docs').mkdir()
    (proj / '.git').mkdir()
    (proj / 'README.md').write_text('# cx demo\n\nA fixture project for UI captures.\n\n- item one\n- item two\n')
    (proj / 'src/main.rs').write_text('fn main() {\n    println!("hello from the fixture");\n}\n')
    (proj / 'Cargo.toml').write_text('[package]\nname = "cx-demo"\nversion = "0.1.0"\n')
    (proj / 'docs/notes.txt').write_text('notes\n' * 40)
    (proj / 'data.bin').write_bytes(bytes(range(256)) * 64)
    (home / 'projects/robot-logs').mkdir()
    for i in range(6):
        (home / f'projects/robot-logs/run-{i:02}.log').write_text(f'run {i}\n' * (i + 1) * 30)
    (home / 'Downloads').mkdir()
    (home / 'Downloads/firmware-v2.tar.gz').write_bytes(b'\0' * 4096)
    (home / '.bashrc').write_text("PS1='fixture:\\W$ '\n")
    (home / '.bash_profile').write_text(". ~/.bashrc\n")


def main():
    with tempfile.TemporaryDirectory(prefix='cx-capture-matrix-') as tmp:
        root = pathlib.Path(tmp)
        home = root / 'home'
        remote = root / 'remote'
        managed = []
        # State lives outside each HOME so the Files captures show only fixture files.
        for h in (home, remote):
            (h / '.local/bin').mkdir(parents=True)
            (h / '.local/bin/cx').symlink_to(binary)
            (root / f'state-{h.name}/cx').mkdir(parents=True)
            managed.append(root / f'state-{h.name}/cx/managed.sock')
        write_fixture_files(home)
        (remote / 'catkin_ws/src').mkdir(parents=True)
        (remote / '.bashrc').write_text("PS1='robot:\\W$ '\n")
        (remote / '.bash_profile').write_text(". ~/.bashrc\n")
        bindir = root / 'bin'
        bindir.mkdir()
        ssh = bindir / 'ssh'
        ssh.write_text(f"""#!/bin/sh
for arg do last="$arg"; done
case "$*" in
  *lab-arm.invalid*)
    [ "$last" = 'exec ~/.local/bin/cx helper' ] || exit 1
    HOME={shlex.quote(str(remote))} XDG_STATE_HOME={shlex.quote(str(root / 'state-remote'))} exec {shlex.quote(binary)} helper ;;
esac
echo "ssh: connect to host: Connection refused" >&2
exit 255
""")
        ssh.chmod(0o700)
        base = dict(os.environ)
        for k in ['TMUX', 'TMUX_PANE', 'NO_COLOR', 'CX_ASCII', 'SSH_AUTH_SOCK', 'COLORTERM', 'TERM_PROGRAM', 'KITTY_WINDOW_ID']:
            base.pop(k, None)
        env = dict(base, HOME=str(home), XDG_STATE_HOME=str(root / 'state-home'), SHELL='/bin/sh',
                   TERM='xterm-256color', PATH=str(bindir) + os.pathsep + base['PATH'],
                   HTTPS_PROXY='http://127.0.0.1:9', LANG='C.UTF-8', LC_ALL='C.UTF-8')
        env.update(extra_env)
        renv = dict(env, HOME=str(remote), XDG_STATE_HOME=str(root / 'state-remote'))
        (root / 'state-home/cx/devices.json').write_text(json.dumps([
            {'id': 'local', 'name': 'laptop', 'target': None, 'account': 'fixture', 'host': 'laptop', 'status': 'local', 'observed_at': 0},
            {'id': 'lab-arm', 'name': 'lab-arm', 'target': 'lab-arm.invalid', 'account': 'robot', 'host': 'lab-arm', 'status': 'unknown', 'observed_at': 0},
            {'id': 'field-rover', 'name': 'field-rover', 'target': 'field-rover.invalid', 'account': 'robot', 'host': 'field-rover', 'status': 'unknown', 'observed_at': 0}]))

        def cli(e, *a):
            return json.loads(subprocess.check_output([binary, *a], env=e, stderr=subprocess.DEVNULL, timeout=30))

        proj = home / 'projects/cx-demo'
        made = [(managed[0], 'matrix-build', cli(env, 'new', '--directory', str(proj), '--key', 'matrix-build', '--name', 'build watcher')),
                (managed[0], 'matrix-logs', cli(env, 'new', '--directory', str(home / 'projects/robot-logs'), '--key', 'matrix-logs')),
                (managed[1], 'matrix-remote', cli(renv, 'new', '--directory', str(remote / 'catkin_ws'), '--key', 'matrix-remote', '--name', 'ros core'))]
        time.sleep(1.0)
        # Owned private tmux servers only: give each shell recognizable output.
        demo = {'matrix-build': "clear; printf 'Compiling cx-demo v0.1.0\\n    Finished dev in 2.41s\\n[Running cargo check]\\n'",
                'matrix-logs': "clear; for i in 1 2 3 4 5 6; do printf 'run-05 step %s ok  lidar=%sHz\\n' $i 1$i; done",
                'matrix-remote': "clear; printf 'started roslaunch server http://lab-arm:41011/\\nprocess[master]: started\\n'"}
        for sock, key, session in made:
            subprocess.run(['tmux', '-S', str(sock), 'send-keys', '-t', session['id'], demo[key], 'Enter'],
                           stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        time.sleep(0.8)
        index = []
        try:
            for cols, rows in sizes:
                index += capture_size(cols, rows, env)
        finally:
            for sock in managed:
                subprocess.run(['tmux', '-S', str(sock), 'kill-server'], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        (out / 'index.json').write_text(json.dumps(index, indent=1))
        print(json.dumps(dict(result='PASS', captures=len(index), out=str(out))))


def capture_size(cols, rows, env):
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', rows, cols, 0, 0))

    def control():
        os.setsid()
        fcntl.ioctl(slave, termios.TIOCSCTTY, 0)

    proc = subprocess.Popen([binary], stdin=slave, stdout=slave, stderr=slave, env=env, cwd=env['HOME'], preexec_fn=control)
    screen = Screen(cols, rows)
    stream = pyte.Stream(screen)

    def read(seconds=.35):
        end = time.monotonic() + seconds
        while time.monotonic() < end:
            if select.select([master], [], [], .03)[0]:
                try:
                    stream.feed(os.read(master, 65536).decode('utf-8', 'replace'))
                except OSError:
                    break

    def send(b, wait=.45):
        os.write(master, b)
        read(wait)

    def text():
        return '\n'.join(screen.display)

    results = []

    def shot(view):
        name = f'{view}-{cols}x{rows}'
        redact(screen, cols, rows)
        (out / f'{name}.txt').write_text(text() + '\n')
        render_png(screen, cols, rows, out / f'{name}.png')
        results.append(dict(view=view, cols=cols, rows=rows, txt=f'{name}.txt', png=f'{name}.png'))

    def palette(query):
        send(b'\x10')
        send(query.encode())
        send(b'\r', 1.2)

    def reset():
        for _ in range(3):
            send(b'\x1b', .25)

    try:
        read(2.5)
        shot('startup')  # cx opens Files at the launch directory; remote errors toast
        read(6.0)
        shot('files')
        send(b'j', .3)
        send(b'\r', 1.2)
        shot('files-projects')
        send(b'\r', 1.2)
        shot('files-inside')
        send(b'G', .3)
        send(b'k', .3)  # README.md: a rich preview
        send(b'\r', 1.5)
        shot('files-preview')
        send(b'\x1b', .4)
        send(b'\x1b', .8)
        shot('sessions')
        send(b'\x1b', .4)  # focus devices
        send(b'gg', .4)  # All devices
        send(b'\r', 1.5)
        shot('sessions-all')
        send(b'j', 1.5)
        shot('sessions-next')
        send(b'\x10', .5)
        shot('palette')
        send(b'\x1b', .4)
        send(b'?', .5)
        shot('help')
        send(b'\x1b', .4)
        if network:
            palette('Network')
            send(b'\r', 2.0)
            shot('network')
            send(b'j', 1.0)
            send(b'l', 1.0)
            shot('network-expanded')
    finally:
        os.write(master, b'\x03')
        try:
            proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            proc.kill()
            proc.wait()
        os.close(master)
        os.close(slave)
    return results


main()
