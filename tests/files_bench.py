#!/usr/bin/env python3
"""cx Files load-time benchmark: directory open, file preview, preview scrolling, helper protocol.

usage: files_bench.py BINARY [--remote] [--runs N] [--out FILE] [--label NAME]
                      [--rtt MS] [--only startup,dirs,deep,previews,scroll,helper] [--sizes 100,1000,...]
                      [--files KEY,...] [--sides local,remote] [--oplog FILE]

Measures what the user feels in a real PTY (pyte screen model, 120x40): every UI time runs from
the write of the key to the arrival of the PTY output chunk that first made a screen predicate
true. Nothing waits on sleeps. A separate helper microbenchmark drives `cx helper` with the
framed protocol (`CX1 <len>\\n<json>`) locally on stdin and, with --remote, over SSH. That splits
protocol and transport cost from UI cost.

--remote starts a disposable loopback sshd that authenticates only a temporary key for the
current account. A PATH-wrapped `ssh` gives cx a private client config and swaps the fixed
helper command for `exec env HOME=<private> ... BINARY helper`. Any other remote command is
refused, so the installed ~/.local/bin/cx and the real ~/.ssh are never used. --rtt MS adds a
userspace relay (ProxyCommand) that delays each direction by MS/2 to model a real link.

Isolation: each viewer gets a private HOME/XDG_STATE_HOME/TMUX_TMPDIR with TMUX unset. Fixtures
live on disk (/var/tmp, not tmpfs) and are removed afterwards. Every spawned process carries a
private tag in its environment, and cleanup kills only processes with that tag. Cold means
"cx-cold": a fresh viewer process. Directory metadata stays OS-warm because the page and dentry
caches cannot be dropped without root. File previews drop the fixture file's pages with
posix_fadvise(DONTNEED) before each cold open (best effort; recorded per run).

Output: JSON (--out, default local-evidence/perf/files-bench-<label>.json) holding every sample
and a flat `metrics` map {name: {unit, better, samples, median, p95}}, plus a markdown table
on stdout and next to the JSON (.md). Compare two results with tests/files_bench_compare.py.
"""
import argparse, hashlib, json, os, pathlib, platform, pwd, re, shlex, shutil, signal, socket
import statistics, subprocess, sys, tempfile, time, uuid

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import perf_harness as ph  # noqa: E402  (PTY plumbing, Env cleanup, fixture text)

HERE = pathlib.Path(__file__).resolve().parent
REPO = HERE.parent
HELPER_COMMAND = 'exec ~/.local/bin/cx helper'
SIZES = (100, 1000, 20000, 100000)
DEEP_LEVELS = 24
UI_CAP = 100000           # LISTING_CAP in src/ui/apply/files.rs (= files::MAX_LIST_OFFSET)
OLD_UI_CAP = 10000        # v0.1.95 (the pre-hub layout) stopped paging at 10,000 entries
FULL_TIMEOUT = 90.0
END_MARKER = ph.END_MARKER
WHEEL_DOWN = ph.WHEEL_DOWN
ROW = re.compile(r'\b([bs])-(\d{6})\.txt')
# Both layouts: v0.1.95 ('╭ ▸ N items' title, '▸ name' row) and the revamp (bar '▌', a
# type glyph before the name, a trailing '/' on folders, 'N items' or 'loaded/total items'
# on the bottom border).
PREVIEW_TITLE = re.compile(r'┏ (?:\S.*? · [\d.]+ (?:B|KiB|MiB|GiB) · |PDF · \d+/\d+ )')
COUNT = re.compile(r'(?:╭ ▸ |[━─] )(?:(\d+)/)?(\d+) items')
SELECTED = re.compile(r'(?:│\s+(?:/\s+)?▸ |[┃│]▌ \S )(\S(?:.*?\S)?)/?(?:\s{2,}|\s*[│┃])')
# The hub design (2026-11): a crumb `cx › …` on row 0, a FILES label with `N items` on its
# right, rows `▌▸ name/` (bar, fold or type glyph, name) from column 1, and a PREVIEW label
# row over the preview surface (inline from 100 columns, or the whole body when open).
NEW_COUNT = re.compile(r'FILES\s+(?:(\d+)/)?(\d+) items')
NEW_SELECTED = re.compile(r'^ ▌(?:[▸▾→] |  )(\S(?:.*?\S)?)/?(?=\s{2,}|$)')
# The yazi Files view (2026-11-12): a status line `NOR  4K  name … perm  Top   3/9`
# (cap glyphs may sit at both ends), the current column between x=cols/8 and x=cols*5/8.
YAZI_STATUS = re.compile(r'^\W?\s*(?:NOR|SEL|UNS)\s+\S+\s+(\S(?:.*?\S)?)(?:\s{2,}|$)')
YAZI_POS = re.compile(r'(\d+)/(\d+)\s*\W?\s*$')


def pct(values, q):
    return ph.percentile([v for v in values if v is not None], q)


# ---------------------------------------------------------------------------
# results
class Results:
    def __init__(self):
        self.metrics = {}
        self.notes = []
        self.details = {}

    def add(self, name, value, unit='ms', better='lower'):
        m = self.metrics.setdefault(name, dict(unit=unit, better=better, samples=[], failed=0))
        if value is None:
            m['failed'] += 1
        elif isinstance(value, (list, tuple)):
            m['samples'].extend(round(v, 3) for v in value if v is not None)
        else:
            m['samples'].append(round(value, 3) if isinstance(value, float) else value)

    def finish(self):
        for m in self.metrics.values():
            s = m['samples']
            m['n'] = len(s)
            m['median'] = round(statistics.median(s), 2) if s else None
            m['p95'] = pct(s, .95) if s else None
            m['min'] = min(s) if s else None
            m['max'] = max(s) if s else None


def shown_as(shown, name):
    """A list row shows name, or (narrow revamp list) name with its middle cut to '…'."""
    if shown is None or shown == name:
        return shown == name
    for text in (shown, re.sub(r' \S+$', '', shown)):  # a narrow row puts the type right after
        head, cut, tail = text.partition('…')
        if text == name or (cut and name.startswith(head) and name.endswith(tail)):
            return True
    return False


def ms(t0, t1):
    return None if t1 is None else round((t1 - t0) * 1000, 2)


# ---------------------------------------------------------------------------
# fixtures
def source_text(target_bytes):
    """Rust-like source with a unique number on every line and END_MARKER on the last line."""
    sample = ph.source_fixture(600)
    lines = int(target_bytes / (len(sample) / 600)) + 1
    return ph.source_fixture(max(lines, 10))


def markdown_text(target_bytes):
    out, size, i = [], 0, 0
    while size < target_bytes:
        if i % 25 == 0:
            chunk = [f'## Section {i}', '']
        elif i % 7 == 0:
            chunk = [f'Paragraph {i} with *emphasis*, `code` and a [link](#section-{i}).']
        else:
            chunk = [f'- list item {i} with some ordinary words to wrap']
        out += chunk
        size += sum(len(c) + 1 for c in chunk)
        i += 1
    out.append(END_MARKER)
    return '\n'.join(out) + '\n'


def drop_file_cache(path):
    """Best effort without root: write back and drop this file's pages. Returns True when
    fincore (read-only) confirms that no pages stay resident, None when it cannot tell."""
    try:
        fd = os.open(path, os.O_RDONLY)
        try:
            os.fsync(fd)
            os.posix_fadvise(fd, 0, 0, os.POSIX_FADV_DONTNEED)
        finally:
            os.close(fd)
    except OSError:
        return False
    if shutil.which('fincore'):
        out = subprocess.run(['fincore', '--bytes', '--noheadings', '--output', 'RES', str(path)],
                             capture_output=True, text=True)
        if out.returncode == 0 and out.stdout.strip().isdigit():
            return int(out.stdout.strip()) == 0
    return None


class Fixtures:
    """Directory trees and preview files on a disk-backed temporary directory."""

    def __init__(self, sizes):
        base = '/var/tmp' if os.access('/var/tmp', os.W_OK) else None
        self.tmp = tempfile.TemporaryDirectory(prefix='cx-files-bench-', dir=base)
        self.root = pathlib.Path(self.tmp.name)
        self.sizes = sizes
        t0 = time.monotonic()
        self.dirs = {}
        for n in sizes:
            parent = self.root / f'dirs/n{n}'
            for name, letter in (('big', 'b'), ('sibling', 's')):
                d = parent / name
                d.mkdir(parents=True)
                for i in range(n):
                    os.close(os.open(d / f'{letter}-{i:06}.txt', os.O_CREAT | os.O_WRONLY, 0o644))
            self.dirs[n] = parent
        # Deep tree: l00/l01/.../l23, ten files per level; the next level sorts first.
        self.deep = self.root / 'deep'
        level = self.deep
        for k in range(DEEP_LEVELS):
            level = level / f'l{k:02}'
            level.mkdir(parents=True)
            for i in range(10):
                (level / f'f-{i:02}.txt').write_text(f'level {k} file {i}\n')
        self.files_dir = self.root / 'files'
        self.files_dir.mkdir()
        self.files = {}
        self._make_files()
        self.build_s = round(time.monotonic() - t0, 2)

    def _file(self, key, name, data, kind, probe=None, numbered=False, directory=None):
        folder = directory or self.files_dir
        folder.mkdir(parents=True, exist_ok=True)
        path = folder / name
        if isinstance(data, str):
            data = data.encode()
        path.write_bytes(data)
        self.files[key] = dict(path=path, kind=kind, probe=probe, numbered=numbered, bytes=len(data))

    def _make_files(self):
        self._file('src_2k', 'src-2k.rs', source_text(2 * 1024), 'text', 'Doc comment for item 0', True)
        self._file('src_200k', 'src-200k.rs', source_text(200 * 1024), 'text', 'Doc comment for item 0', True)
        # Same bytes as src_200k but a .txt name: kind "text" skips syntax highlighting.
        self._file('txt_200k', 'text-200k.txt', source_text(200 * 1024), 'text', 'Doc comment for item 0', True)
        self._file('src_5m', 'src-5m.rs', source_text(5 * 1024 * 1024), 'text', 'Doc comment for item 0', True)
        self._file('src_50m', 'src-50m.rs', source_text(50 * 1024 * 1024), 'text', 'Doc comment for item 0', True)
        self._file('md_2m', 'big-2m.md', markdown_text(2 * 1024 * 1024), 'text', 'Section 0', True)
        from PIL import Image
        w, h = 1920, 1080
        gradient = Image.linear_gradient('L').resize((w, h))
        image = Image.merge('RGB', (gradient, gradient.transpose(Image.Transpose.ROTATE_90).resize((w, h)),
                                    Image.radial_gradient('L').resize((w, h))))
        for key, name, fmt in (('png', 'photo.png', 'PNG'), ('jpeg', 'photo.jpg', 'JPEG')):
            path = self.files_dir / name
            image.save(path, fmt)
            self.files[key] = dict(path=path, kind='raster', probe=None, numbered=False, bytes=path.stat().st_size)
        pdf = self.files_dir / 'doc.pdf'
        page = image.resize((850, 1100))
        page.save(pdf, 'PDF', save_all=True, append_images=[page, page])
        self.files['pdf'] = dict(path=pdf, kind='raster', probe=None, numbered=False, bytes=pdf.stat().st_size)
        rng = hashlib.sha256(b'cx-files-bench').digest()
        blob = bytearray()
        while len(blob) < 1024 * 1024:
            rng = hashlib.sha256(rng).digest()
            blob += rng
        blob[100:110] = b'\0' * 10
        self._file('binary', 'blob.bin', bytes(blob), 'binary', 'Binary file ·')
        self._file('unicode', 'файл — ✓ 文件.rs', 'юникод probe ✓\n' + source_text(20 * 1024), 'text',
                   'юникод probe', False, directory=self.files_dir / 'ünïcødé ✓ 目录')
        # Previews open from the files directory; the unicode file opens from its own folder.

    def cleanup(self):
        self.tmp.cleanup()


# ---------------------------------------------------------------------------
# loopback SSH
RELAY = r'''import os,socket,sys,threading,time,queue
host,port,delay=sys.argv[1],int(sys.argv[2]),float(sys.argv[3])/2000
s=socket.create_connection((host,port));s.setsockopt(socket.IPPROTO_TCP,socket.TCP_NODELAY,1)
def pump(read,write):
    q=queue.Queue()
    def writer():
        while True:
            due,data=q.get()
            if data is None:break
            wait=due-time.monotonic()
            if wait>0:time.sleep(wait)
            write(data)
    t=threading.Thread(target=writer,daemon=True);t.start()
    while True:
        data=read()
        if not data:break
        q.put((time.monotonic()+delay,data))
    q.put((0,None));t.join()
def up():
    pump(lambda:os.read(0,65536),s.sendall);s.shutdown(socket.SHUT_WR)
def down():
    def out(d):
        v=memoryview(d)
        while v:v=v[os.write(1,v):]
    pump(lambda:s.recv(65536),out);os._exit(0)
threading.Thread(target=up,daemon=True).start();down()
'''


OPLOG = r'''import json,os,subprocess,sys,threading,time
log=open(sys.argv[1],'a',buffering=1);child=subprocess.Popen(sys.argv[2:],stdin=subprocess.PIPE,stdout=subprocess.PIPE)
pending={}
def frames(src):
    while True:
        line=src.readline()
        if not line:return
        if not line.startswith(b'CX1 '):continue
        body=src.read(int(line[4:]));yield line,body
def up():
    for line,body in frames(sys.stdin.buffer):
        r=json.loads(body);pending[r['id']]=(time.monotonic(),r['op']['op'],len(body))
        child.stdin.write(line+body);child.stdin.flush()
    child.stdin.close()
threading.Thread(target=up,daemon=True).start()
for line,body in frames(child.stdout):
    r=json.loads(body);t0,op,n=pending.pop(r.get('id'),(time.monotonic(),'?',0))
    log.write(json.dumps(dict(at=round(time.time(),4),op=op,req=n,resp=len(body),helper_ms=round((time.monotonic()-t0)*1000,2),error=bool(r.get('error'))))+'\n')
    sys.stdout.buffer.write(line+body);sys.stdout.buffer.flush()
'''


class LoopbackSSH:
    """A private sshd on 127.0.0.1 that runs the benchmarked binary as the remote helper."""

    def __init__(self, binary, tag, rtt_ms=0.0, oplog=None):
        self.binary = binary
        self.tag = tag
        self.rtt_ms = rtt_ms
        self.tmp = tempfile.TemporaryDirectory(prefix='cx-fb-ssh-')
        root = self.root = pathlib.Path(self.tmp.name)
        root.chmod(0o700)
        self.account = pwd.getpwuid(os.getuid()).pw_name
        for name in ('host', 'client'):
            subprocess.run(['ssh-keygen', '-q', '-t', 'ed25519', '-N', '', '-f', str(root / name)],
                           check=True, timeout=10)
        (root / 'authorized').write_text((root / 'client.pub').read_text())
        (root / 'authorized').chmod(0o600)
        with socket.socket() as sock:
            sock.bind(('127.0.0.1', 0))
            self.port = sock.getsockname()[1]
        self.remote_home = root / 'remote-home'
        (self.remote_home / 'state/cx').mkdir(parents=True)
        self.remote_home.chmod(0o700)
        sshd = shutil.which('sshd') or '/usr/sbin/sshd'
        (root / 'sshd.conf').write_text(f'''Port {self.port}
ListenAddress 127.0.0.1
HostKey {root}/host
PidFile {root}/sshd.pid
AuthorizedKeysFile {root}/authorized
StrictModes no
UsePAM no
PasswordAuthentication no
KbdInteractiveAuthentication no
PubkeyAuthentication yes
AllowUsers {self.account}
AllowAgentForwarding no
AllowTcpForwarding no
X11Forwarding no
PermitTTY yes
PermitUserRC no
''')
        host_key = ' '.join((root / 'host.pub').read_text().split()[:2])
        (root / 'known').write_text(f'fixture {host_key}\n')
        relay = ''
        if rtt_ms > 0:
            (root / 'relay.py').write_text(RELAY)
            relay = f' ProxyCommand {sys.executable} -I {root}/relay.py 127.0.0.1 {self.port} {rtt_ms}\n'
        (root / 'client.conf').write_text(f'''Host fixture
 HostName 127.0.0.1
 Port {self.port}
 HostKeyAlias fixture
 User {self.account}
 IdentityFile {root}/client
 IdentitiesOnly yes
 IdentityAgent none
 UserKnownHostsFile {root}/known
 GlobalKnownHostsFile /dev/null
 StrictHostKeyChecking yes
 BatchMode yes
 ForwardAgent no
 ControlMaster no
 ControlPath none
{relay}''')
        remote_env = dict(HOME=str(self.remote_home), XDG_STATE_HOME=str(self.remote_home / 'state'),
                          TMUX_TMPDIR=str(self.remote_home), CX_FILES_BENCH=tag, SHELL='/bin/sh')
        helper = f'{shlex.quote(binary)} helper'
        if oplog:
            # Diagnostic only: a framing proxy that records op names, sizes and helper-side time.
            (root / 'oplog.py').write_text(OPLOG)
            helper = f'{shlex.quote(sys.executable)} -I {root}/oplog.py {shlex.quote(str(oplog))} {helper}'
        self.remote_command = 'exec env -u TMUX -u TMUX_PANE ' + ' '.join(
            f'{k}={shlex.quote(v)}' for k, v in remote_env.items()) + f' {helper}'
        tools = self.tools = root / 'tools'
        tools.mkdir()
        wrapper = tools / 'ssh'
        wrapper.write_text(f'''#!{sys.executable} -I
import os,sys
args=sys.argv[1:]
while '-F' in args:
    i=args.index('-F');del args[i:i+2]
if not args or args[-1]!={HELPER_COMMAND!r} or len(args)<2 or args[-2]!='fixture':
    words=(args[-1].split() if args else [])
    kind=words[2] if words[:2]==['exec','~/.local/bin/cx'] and len(words)>2 and words[2].isalnum() else 'other'
    with open({str(root / 'refused.log')!r},'a') as log:log.write(kind+'\\n')
    sys.exit(255)
args[-1]={self.remote_command!r}
os.execv({shutil.which('ssh')!r},[{shutil.which('ssh')!r},'-F',{str(root / 'client.conf')!r},*args])
''')
        wrapper.chmod(0o700)
        self.log = (root / 'sshd.log').open('wb')
        env = {k: v for k, v in os.environ.items() if k in ('PATH', 'LANG', 'LC_ALL')}
        env['CX_FILES_BENCH_SSHD'] = tag
        self.sshd = subprocess.Popen([sshd, '-D', '-e', '-f', str(root / 'sshd.conf')], stdout=self.log,
                                     stderr=self.log, env=env, start_new_session=True)
        for _ in range(200):
            if self.sshd.poll() is not None:
                raise RuntimeError('fixture sshd exited at startup')
            try:
                with socket.create_connection(('127.0.0.1', self.port), timeout=.1):
                    break
            except OSError:
                time.sleep(.02)

    def helper_argv(self):
        return [shutil.which('ssh'), '-F', str(self.root / 'client.conf'), 'fixture', self.remote_command]

    def refused(self):
        """Refused remote commands by cx subcommand name only (never arguments)."""
        log = self.root / 'refused.log'
        out = {}
        for kind in (log.read_text().splitlines() if log.exists() else []):
            out[kind] = out.get(kind, 0) + 1
        return out

    def cleanup(self):
        subprocess.run(['tmux', '-S', str(self.remote_home / 'state/cx/managed.sock'), 'kill-server'],
                       capture_output=True)
        if self.sshd.poll() is None:
            os.killpg(self.sshd.pid, signal.SIGTERM)
            try:
                self.sshd.wait(timeout=3)
            except subprocess.TimeoutExpired:
                os.killpg(self.sshd.pid, signal.SIGKILL)
                self.sshd.wait()
        killed = kill_tagged(self.tag, sshd=True)
        self.log.close()
        self.tmp.cleanup()
        return killed


def tagged_pids(tag, sshd=False):
    markers = [f'CX_FILES_BENCH={tag}\0'.encode()]
    if sshd:
        markers.append(f'CX_FILES_BENCH_SSHD={tag}\0'.encode())
    found = set()
    for entry in pathlib.Path('/proc').iterdir():
        if entry.name.isdigit():
            try:
                environ = (entry / 'environ').read_bytes()
            except OSError:
                continue
            if any(m in environ for m in markers):
                found.add(int(entry.name))
    found.discard(os.getpid())
    return found


def kill_tagged(tag, sshd=False):
    pids = tagged_pids(tag, sshd)
    for pid in pids:
        try:
            os.kill(pid, signal.SIGKILL)
        except OSError:
            pass
    return len(pids)


# ---------------------------------------------------------------------------
# PTY viewer
class BenchEnv(ph.Env):
    def __init__(self, binary, tag, ssh=None):
        super().__init__(binary)
        self.tag = tag
        self.env['CX_FILES_BENCH'] = tag
        self.env.pop('SSH_AUTH_SOCK', None)
        self.marker2 = f'CX_FILES_BENCH={tag}\0'.encode()
        local = dict(id='local', name=socket.gethostname(), target=None, account=os.environ.get('USER', ''),
                     host=socket.gethostname(), status='local', observed_at=0)
        self.device_ids = ['local']
        if ssh:
            self.env['PATH'] = str(ssh.tools) + os.pathsep + self.env['PATH']
            remote = dict(id='bench-fixture:' + ssh.account, name='fixture', target='fixture', account=ssh.account,
                          host='fixture', status='online', observed_at=int(time.time()))
            (self.state / 'devices.json').write_text(json.dumps([local, remote]))
            (self.state / 'devices.json').chmod(0o600)
            self.device_ids.append(remote['id'])

    def owned_pids(self):
        found = super().owned_pids()
        # The private root also prefixes remote-helper paths; the tag catches every descendant.
        found |= tagged_pids(self.tag)
        found.discard(os.getpid())
        self.seen |= found
        return found


class Viewer(ph.Viewer):
    """ph.Viewer with an arbitrary device: the restart handoff opens Files on that device."""

    def __init__(self, env, device_index, directory, select):
        browser = dict(device=device_index, path=str(directory), display_path=str(directory),
                       parent=str(directory.parent), entries=[], selected=0, search='', preview_scroll=0,
                       restore_selection=str(select) if select else None)
        self.open(env, dict(device_ids=env.device_ids, device=device_index + 1, focus='Workspace',
                            view='Files', browser=browser))

    def yazi(self, display=None):
        """The yazi Files list: its status line names the mode."""
        last = (display or self.screen.display)[-1]
        return bool(re.match(r'^\W?\s*(?:NOR|SEL|UNS) ', last))

    def columns(self, display=None):
        """(parent, current, preview) column texts of the yazi view, without the rails."""
        display = display or self.screen.display
        w = len(display[0])
        a, b = w // 8, w * 5 // 8
        return ([line[:a] for line in display[1:-1]], [line[a + 1:b - 1] for line in display[1:-1]],
                [line[b:] for line in display[1:-1]])

    def hub_layout(self, display=None):
        """The hub design: a crumb on the first row instead of the v0.1.95 tab header.
        (pyte builds `display` anew on every access, ~1 ms at 120x40: readers take it
        once, or a predicate costs tens of ms and shows up in every latency.)"""
        top = (display or self.screen.display)[0]
        return ' cx › ' in top or ' cx / ' in top

    # Screen readers. v0.1.95's workspace starts at column 21 (the device column is
    # excluded); the hub design's list starts at column 1 and is 48 wide at 120 columns.
    def work(self):
        display = self.screen.display
        if self.hub_layout(display):
            return display
        return [line[21:] for line in display]

    def selected(self):
        display = self.screen.display
        if self.yazi(display):
            m = YAZI_STATUS.match(display[-1])
            return m.group(1) if m else None
        hub = self.hub_layout(display)
        for line in display[1:-1]:
            if hub:
                m = NEW_SELECTED.search(line[:50])
                if m:
                    return m.group(1)
                continue
            m = SELECTED.search(line[21:])
            if m and not line[21:].lstrip().startswith(('╭', '┏')):
                return m.group(1)
        return None

    def count(self):
        if self.yazi():
            m = YAZI_POS.search(self.screen.display[-1])
            return int(m.group(2)) if m else None
        m = (NEW_COUNT if self.hub_layout() else COUNT).search(self.text())
        return int(m.group(1) or m.group(2)) if m else None

    def loading(self):
        return '· loading' in self.screen.display[3] or 'Loading…' in self.text()

    def rows(self, letter=None):
        # The list only: the revamp's inline peek beside it can show a folder's children.
        if self.yazi():
            return [m for line in self.columns()[1] for m in ROW.finditer(line)
                    if letter is None or m.group(1) == letter]
        hub = self.hub_layout()

        def listed(line):
            if hub:
                return line[:50]
            borders = [i for i, c in enumerate(line) if c in '│┃']
            return line[:borders[1]] if len(borders) > 1 else line
        return [m for line in self.work() for m in ROW.finditer(listed(line))
                if letter is None or m.group(1) == letter]

    def has_row(self, name):
        if self.yazi():
            return any(re.search(r'(?:^|\s)' + re.escape(name) + r'(?:\s|$)', line) for line in self.columns()[1])
        return any(re.search(r'(?:▸ |\s)' + re.escape(name) + r'/?(?:\s|$)', line) for line in self.work()[1:-1])

    def write_at(self, data):
        t0 = time.monotonic()
        os.write(self.master, data)
        return t0

    def until(self, predicate, timeout=ph.TIMEOUT, what='condition'):
        """Monotonic arrival time of the PTY chunk that first made predicate true."""
        start = len(self.chunks)
        if predicate():
            return self.chunks[-1][0] if len(self.chunks) > start else time.monotonic()
        end = time.monotonic() + timeout
        while True:
            if self.proc.poll() is not None:
                raise AssertionError(f'viewer exited while waiting for {what}')
            remaining = end - time.monotonic()
            if remaining <= 0:
                if os.environ.get('CX_BENCH_DEBUG'):
                    print(self.text(), file=sys.stderr)
                raise AssertionError(f'timed out waiting for {what}')
            if self._read(min(remaining, .05)) and predicate():
                if os.environ.get('CX_BENCH_DEBUG'):
                    print(f'  {what}: {(self.chunks[-1][0] - (end - timeout)) * 1000:.1f} ms', file=sys.stderr)
                return self.chunks[-1][0]

    def preview_box(self):
        display = self.screen.display
        if self.hub_layout():
            # The open preview's PREVIEW label row (at the left edge; the inline peek
            # beside the list is not the opened preview); the surface runs to the footer.
            for y, line in enumerate(display):
                if line[:12].lstrip(' ▍>').startswith('PREVIEW'):
                    return y, len(display) - 1, line.index('PREVIEW') - 2
            return None
        for y, line in enumerate(display):
            # v0.1.95: '╭ ▸ … Escape back'; revamp: '┏ name · size · kind' (the list's
            # breadcrumb title has no size).
            if '╭ ▸' in line[20:] and 'Escape back' in line:
                column = line.index('╭ ▸')
            elif PREVIEW_TITLE.search(line[20:]):
                column = line.index('┏', 20)
            else:
                continue
            bottom = next((b for b in range(y + 1, len(display))
                           if len(display[b]) > column and display[b][column] in '╰┗'), len(display))
            return y, bottom, column
        return None

    def preview_content(self):
        box = self.preview_box()
        if not box:
            return None
        top, bottom, column = box
        return tuple(line[column + 1:] for line in self.screen.display[top + 1:bottom])


REMOTE_SETTLE = 2.5


def open_viewer(env, device_index, directory, select, ready, settle=None):
    """Start a viewer and wait until it is ready. Remote viewers then wait REMOTE_SETTLE s: at
    startup the single per-host metadata channel runs Info and Containers (about 1 s on this host),
    and file requests queue behind them. The startup scenario measures that queueing separately."""
    viewer = Viewer(env, device_index, directory, select)
    viewer.until(lambda: ready(viewer), timeout=30, what='viewer ready')
    if settle is None:
        settle = REMOTE_SETTLE if device_index else 0
    viewer.quiesce(.3, 3)
    if settle:
        viewer.pump(settle)
    return viewer


def scenario_startup(binary, fx, side, device_index, ssh, tag, res, run):
    """Act as soon as the first listing is painted, as a user who opens cx and goes straight to
    a file: requests compete with the startup Info/Sessions/Containers burst."""
    spec = fx.files['src_2k']
    path = spec['path']
    n = 1000 if 1000 in fx.sizes else min(fx.sizes)
    for what in ('preview', 'dir'):
        env = BenchEnv(binary, tag, ssh)
        try:
            t_start = time.monotonic()
            if what == 'preview':
                viewer = Viewer(env, device_index, path.parent, path)
                t_ready = viewer.until(lambda: viewer.selected() == path.name, timeout=30, what='startup ready')
            else:
                viewer = Viewer(env, device_index, fx.dirs[n], fx.dirs[n] / 'big')
                t_ready = viewer.until(lambda: viewer.selected() == 'big', timeout=30, what='startup ready')
            try:
                res.add(f'{side}.startup.viewer_ready_ms', ms(t_start, t_ready))
                t0 = viewer.write_at(b'\r')
                if what == 'preview':
                    t = viewer.until(lambda: first_content(viewer, spec), timeout=60, what='startup preview')
                    res.add(f'{side}.startup.preview_src_2k_first_paint_ms', ms(t0, t))
                else:
                    t = viewer.until(lambda: viewer.rows('b'), timeout=60, what='startup dir')
                    res.add(f'{side}.startup.dir_{n}_first_rows_ms', ms(t0, t))
            finally:
                res.details.setdefault('viewer_close', []).append(viewer.close())
        except AssertionError as error:
            res.notes.append(f'run {run} {side} startup {what}: {error}')
        finally:
            res.details.setdefault('cleanup', []).append(env.cleanup())


# ---------------------------------------------------------------------------
# yazi navigation: what the 2026-11-12 targets name
def preview_has(viewer, probe):
    return viewer.yazi() and any(probe in line for line in viewer.columns()[2])


def scenario_nav(binary, fx, side, device_index, ssh, tag, res, run):
    """Folder moves served by the prefetch, hover previews, and `'` device switches."""
    n = min(fx.sizes)
    env = BenchEnv(binary, tag, ssh)
    base = f'{side}.nav'
    try:
        parent = fx.dirs[n]
        viewer = open_viewer(env, device_index, parent, parent / 'big',
                             lambda v: v.selected() == 'big' and v.has_row('sibling'))
        try:
            # l into the hovered folder (its listing was prefetched while hovering).
            viewer.quiesce(.3, 3)
            t0 = viewer.write_at(b'l')
            t = viewer.until(lambda: viewer.rows('b'), timeout=FULL_TIMEOUT, what='enter prefetched')
            res.add(f'{base}.enter_prefetched_ms', ms(t0, t))
            viewer.quiesce(.3, 3)
            # h back: the parent column's listing is in memory.
            t0 = viewer.write_at(b'h')
            t = viewer.until(lambda: viewer.has_row('sibling') and not viewer.rows('b'), timeout=FULL_TIMEOUT,
                             what='leave')
            res.add(f'{base}.leave_ms', ms(t0, t))
            viewer.quiesce(.3, 3)
            # j onto the sibling then l at once (key-repeat speed: the prefetch races).
            viewer.write_at(b'j')
            t0 = viewer.write_at(b'l')
            t = viewer.until(lambda: viewer.rows('s'), timeout=FULL_TIMEOUT, what='enter right after hover')
            res.add(f'{base}.enter_after_hover_ms', ms(t0, t))
        finally:
            res.details.setdefault('viewer_close', []).append(viewer.close())
        # Hover previews of text files: cold, then moving back (kept in memory).
        two, other = fx.files['src_2k'], fx.files['src_200k']
        viewer = open_viewer(env, device_index, other['path'].parent, other['path'],
                             lambda v: v.selected() == other['path'].name)
        try:
            viewer.until(lambda: preview_has(viewer, other['probe']), timeout=30, what='first hover preview')
            viewer.quiesce(.3, 3)
            down = b'j' if sorted([two['path'].name.lower(), other['path'].name.lower()])[0] == other['path'].name.lower() else b'k'
            back = b'k' if down == b'j' else b'j'
            t0 = viewer.write_at(down)
            viewer.until(lambda: viewer.selected() == two['path'].name, what='hover src_2k')
            t = viewer.until(lambda: preview_has(viewer, two['probe']), timeout=30, what='hover preview cold')
            res.add(f'{base}.hover_preview_cold_ms', ms(t0, t))
            viewer.quiesce(.3, 3)
            t0 = viewer.write_at(back)
            viewer.until(lambda: viewer.selected() == other['path'].name, what='hover back')
            t = viewer.until(lambda: preview_has(viewer, other['probe']), timeout=30, what='hover preview warm')
            res.add(f'{base}.hover_preview_warm_ms', ms(t0, t))
        finally:
            res.details.setdefault('viewer_close', []).append(viewer.close())
        if device_index:
            # `'` + letter: from the local device to this remote one (cold), back (seen),
            # and to the remote again (seen).
            viewer = open_viewer(env, 0, parent, parent / 'big', lambda v: v.selected() == 'big', settle=REMOTE_SETTLE)
            try:
                viewer.quiesce(.3, 3)
                viewer.write_at(b"'")
                viewer.until(lambda: '(here)' in viewer.text(), what='device panel')
                m = re.search(r'(?:^|\s)(\w)\s+\W?\s*fixture', viewer.text())
                if not m:
                    raise AssertionError('no letter for the fixture device in the panel')
                letter = m.group(1).encode()

                def on_remote():
                    # The remote's home (its last folder): rows listed, nothing loading.
                    d = viewer.screen.display
                    return (viewer.yazi(d) and 'fixture' in d[0] and 'Loading' not in '\n'.join(d)
                            and YAZI_POS.search(d[-1]) is not None and not d[-1].rstrip().endswith('0/0'))
                t0 = viewer.write_at(letter)
                t = viewer.until(on_remote, timeout=30, what='switch to the remote (cold)')
                res.add(f'{base}.switch_cold_ms', ms(t0, t))
                viewer.quiesce(.3, 3)
                t0 = viewer.write_at(b"''")
                t = viewer.until(lambda: viewer.yazi() and 'fixture' not in viewer.screen.display[0]
                                 and viewer.selected() == 'big', timeout=30, what="'' back (seen)")
                res.add(f'{base}.switch_seen_ms', ms(t0, t))
                viewer.quiesce(.3, 3)
                t0 = viewer.write_at(b"''")
                t = viewer.until(on_remote, timeout=30, what="'' to the remote (seen)")
                res.add(f'{base}.switch_seen_remote_ms', ms(t0, t))
            finally:
                res.details.setdefault('viewer_close', []).append(viewer.close())
    except AssertionError as error:
        res.notes.append(f'run {run} {side} nav: {error}')
    finally:
        res.details.setdefault('cleanup', []).append(env.cleanup())


# ---------------------------------------------------------------------------
# UI scenarios
def dir_open(viewer, letter, expected, res, prefix):
    """Enter on the selected directory: first rows, j usable, final count."""
    viewer.quiesce(.15, 2)
    t0 = viewer.write_at(b'\r')
    t_first = viewer.until(lambda: viewer.rows(letter), timeout=FULL_TIMEOUT, what=f'{prefix} first rows')
    res.add(f'{prefix}.first_rows_ms', ms(t0, t_first))
    first_selected = viewer.selected()
    t_j = viewer.write_at(b'j')
    t_usable = viewer.until(lambda: viewer.selected() not in (None, first_selected), timeout=FULL_TIMEOUT,
                            what=f'{prefix} j moves')
    res.add(f'{prefix}.usable_ms', ms(t0, t_usable))
    res.add(f'{prefix}.j_after_first_rows_ms', ms(t_j, t_usable))
    want = min(expected, UI_CAP if viewer.hub_layout() else OLD_UI_CAP)
    t_full = viewer.until(lambda: not viewer.loading() and (viewer.count() or 0) >= want,
                          timeout=FULL_TIMEOUT, what=f'{prefix} full count')
    res.add(f'{prefix}.full_count_ms', ms(t0, t_full))
    res.add(f'{prefix}.final_count', viewer.count(), unit='entries', better='higher')
    return viewer.count()


def scenario_dirs(binary, fx, side, device_index, ssh, tag, res, run):
    for n in fx.sizes:
        env = BenchEnv(binary, tag, ssh)
        try:
            parent = fx.dirs[n]
            viewer = open_viewer(env, device_index, parent, parent / 'big',
                                 lambda v: v.has_row('sibling') and v.selected() == 'big')
            try:
                base = f'{side}.dir.{n}'
                shown = dir_open(viewer, 'b', n, res, f'{base}.cold')
                res.details.setdefault(f'{base}.cap', dict(entries=n, shown=shown, all_listed=shown == n,
                                                           cap_notice='remaining entries require' in viewer.text()))
                # j inside the loaded list (selection motion cost grows with list size).
                j = []
                for _ in range(10):
                    before = viewer.selected()
                    settled, stats = viewer.measure(b'j', lambda: viewer.selected() != before)
                    j.append(stats.get('visible_ms'))
                res.add(f'{base}.j_in_list_ms', j)
                # Back to the parent (cached browser), then into the sibling (cold).
                t0 = viewer.write_at(b'h')
                t = viewer.until(lambda: viewer.has_row('sibling') and not viewer.rows(), timeout=FULL_TIMEOUT,
                                 what='parent rows')
                res.add(f'{base}.parent_ms', ms(t0, t))
                viewer.until(lambda: not viewer.loading(), timeout=FULL_TIMEOUT, what='parent refreshed')
                viewer.quiesce(.15, 2)
                if viewer.selected() != 'sibling':
                    viewer.write_at(b'j')
                    viewer.until(lambda: viewer.selected() == 'sibling', what='sibling selected')
                dir_open(viewer, 's', n, res, f'{base}.sibling')
                # Back again, select big, Enter: warm revisit served from the browser cache.
                viewer.write_at(b'h')
                viewer.until(lambda: viewer.has_row('big') and not viewer.rows(), timeout=FULL_TIMEOUT, what='parent')
                viewer.until(lambda: not viewer.loading(), timeout=FULL_TIMEOUT, what='parent refreshed')
                viewer.quiesce(.15, 2)
                if viewer.selected() != 'big':
                    viewer.write_at(b'k')
                    viewer.until(lambda: viewer.selected() == 'big', what='big selected')
                dir_open(viewer, 'b', n, res, f'{base}.warm')
            finally:
                res.details.setdefault('viewer_close', []).append(viewer.close())
        except AssertionError as error:
            res.notes.append(f'run {run} {side} dir {n}: {error}')
        finally:
            res.details.setdefault('cleanup', []).append(env.cleanup())


def scenario_deep(binary, fx, side, device_index, ssh, tag, res, run):
    env = BenchEnv(binary, tag, ssh)
    try:
        viewer = open_viewer(env, device_index, fx.deep, fx.deep / 'l00', lambda v: v.selected() == 'l00')
        try:
            down, up = [], []
            for k in range(1, DEEP_LEVELS):
                viewer.quiesce(.05, 1)
                t0 = viewer.write_at(b'\r')
                name = f'l{k:02}'
                t = viewer.until(lambda: viewer.has_row(name) and viewer.has_row('f-09.txt'), timeout=30,
                                 what=f'deep level {k}')
                down.append(ms(t0, t))
                viewer.until(lambda: not viewer.loading() and viewer.selected() == name, timeout=30,
                             what=f'deep level {k} selection')
            for k in range(DEEP_LEVELS - 2, -1, -1):
                viewer.quiesce(.05, 1)
                t0 = viewer.write_at(b'h')
                name = f'l{k + 1:02}'
                t = viewer.until(lambda: viewer.selected() == name, timeout=30, what=f'deep up {k}')
                up.append(ms(t0, t))
            res.add(f'{side}.deep.enter_hop_ms', down)
            res.add(f'{side}.deep.parent_hop_ms', up)
            res.add(f'{side}.deep.enter_total_ms', sum(down))
        finally:
            res.details.setdefault('viewer_close', []).append(viewer.close())
    except AssertionError as error:
        res.notes.append(f'run {run} {side} deep: {error}')
    finally:
        res.details.setdefault('cleanup', []).append(env.cleanup())


def numbered_lines(text):
    line_of = {}
    for index, line in enumerate(text.splitlines()):
        found = re.search(r'\d+', line)
        if found and found.group() not in line_of:
            line_of[found.group()] = index + 1
    return line_of


def first_content(viewer, spec):
    content = viewer.preview_content()
    if not content:
        return False
    joined = '\n'.join(content)
    if spec['kind'] == 'raster':
        return joined.count('▀') + joined.count('▄') >= 200
    return spec['probe'] in joined


def scenario_previews(binary, fx, side, device_index, ssh, tag, res, run, line_maps):
    for key in getattr(fx, 'preview_keys', None) or list(fx.files):
        spec = fx.files[key]
        env = BenchEnv(binary, tag, ssh)
        path = spec['path']
        try:
            viewer = open_viewer(env, device_index, path.parent, path, lambda v: shown_as(v.selected(), path.name))
            try:
                base = f'{side}.preview.{key}'
                dropped = drop_file_cache(path)
                res.details.setdefault('page_cache_dropped', []).append(dropped)
                t0 = viewer.write_at(b'\r')
                t = viewer.until(lambda: first_content(viewer, spec), timeout=60, what=f'{key} first paint')
                res.add(f'{base}.first_paint_ms', ms(t0, t))
                viewer.quiesce(.1, 5)
                if spec['numbered']:
                    before = viewer.preview_content()
                    end_settled, stats = viewer.measure(
                        b'G', lambda: viewer.preview_content() != before or END_MARKER in viewer.text(), timeout=20)
                    res.add(f'{base}.end_ms', stats.get('visible_ms'))
                    visible = END_MARKER in viewer.text()
                    res.add(f'{base}.end_reachable', 1 if visible else 0, unit='bool', better='higher')
                    lines = line_maps[key]
                    rows = [lines.get(m.group()) for row in viewer.preview_content() or ()
                            for m in [re.search(r'\d+', row)] if m]
                    rows = [r for r in rows if r]
                    total = len(spec['path'].read_text().splitlines()) if spec['bytes'] < 8 << 20 else None
                    res.details.setdefault(f'{base}.end', []).append(dict(
                        last_line_visible=visible, last_numbered_line=max(rows) if rows else None,
                        total_lines=total, bytes=spec['bytes']))
            finally:
                res.details.setdefault('viewer_close', []).append(viewer.close())
        except AssertionError as error:
            res.add(f'{side}.preview.{key}.first_paint_ms', None)
            res.notes.append(f'run {run} {side} preview {key}: {error}')
        finally:
            res.details.setdefault('cleanup', []).append(env.cleanup())


def scenario_scroll(binary, fx, side, device_index, ssh, tag, res, run, presses=30):
    for key in ('src_5m', 'md_2m'):
        spec = fx.files[key]
        path = spec['path']
        env = BenchEnv(binary, tag, ssh)
        try:
            viewer = open_viewer(env, device_index, path.parent, path, lambda v: shown_as(v.selected(), path.name))
            try:
                viewer.write_at(b'\r')
                viewer.until(lambda: first_content(viewer, spec), timeout=60, what='scroll preview')
                viewer.quiesce(.2, 5)
                top, bottom, column = viewer.preview_box()
                wheel = f'\x1b[<{WHEEL_DOWN};{column + 40};{(top + bottom) // 2 + 1}M'.encode()
                for label, data in (('j', b'j'), ('pagedown', b'\x1b[6~'), ('wheel', wheel)):
                    visible, settled = [], []
                    for _ in range(presses):
                        before = viewer.preview_content()
                        s, stats = viewer.measure(data, lambda: viewer.preview_content() != before)
                        visible.append(stats.get('visible_ms'))
                        settled.append(s)
                    res.add(f'{side}.scroll.{key}.{label}.key_to_paint_ms', visible)
                    res.add(f'{side}.scroll.{key}.{label}.settled_ms', settled)
            finally:
                res.details.setdefault('viewer_close', []).append(viewer.close())
        except (AssertionError, TypeError) as error:
            res.notes.append(f'run {run} {side} scroll {key}: {error}')
        finally:
            res.details.setdefault('cleanup', []).append(env.cleanup())


# ---------------------------------------------------------------------------
# helper microbenchmark
class Helper:
    def __init__(self, argv, env):
        t0 = time.monotonic()
        self.proc = subprocess.Popen(argv, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                     stderr=subprocess.DEVNULL, env=env, start_new_session=True)
        self.spawned = t0
        self.next = 0

    def request(self, op, args=None):
        self.next += 1
        body = dict(version=1, id=f'bench-{self.next}', op=dict(op=op, **({'args': args} if args else {})))
        payload = json.dumps(body).encode()
        t0 = time.monotonic()
        self.proc.stdin.write(b'CX1 %d\n' % len(payload) + payload)
        self.proc.stdin.flush()
        noise = 0
        while True:
            line = self.proc.stdout.readline()
            if not line:
                raise RuntimeError('helper closed')
            if line.startswith(b'CX1 '):
                break
            noise += len(line)
            if noise > 16384:
                raise RuntimeError('excessive helper startup noise')
        size = int(line[4:])
        data = self.proc.stdout.read(size)
        t1 = time.monotonic()
        response = json.loads(data)
        if response.get('error'):
            raise RuntimeError('helper error: ' + response['error'][:200])
        return (t1 - t0) * 1000, size + len(line), response['result']

    def close(self):
        try:
            self.proc.stdin.close()
            self.proc.wait(timeout=5)
        except (OSError, subprocess.TimeoutExpired):
            pass
        if self.proc.poll() is None:
            os.killpg(self.proc.pid, signal.SIGKILL)
            self.proc.wait()


def fs_floor(directory, first):
    """Python lower bound for the helper's work: readdir with d_type, sort, lstat of one page."""
    t0 = time.monotonic()
    with os.scandir(directory) as it:
        entries = sorted(((not e.is_dir(follow_symlinks=False), e.name) for e in it))
    t1 = time.monotonic()
    for _, name in entries[:first]:
        os.lstat(os.path.join(directory, name))
    t2 = time.monotonic()
    return (t1 - t0) * 1000, (t2 - t1) * 1000


def helper_bench(fx, side, argv, env, res, run):
    helper = Helper(argv, env)
    try:
        empty = fx.root / 'empty'
        empty.mkdir(exist_ok=True)
        t, _, _ = helper.request('list', dict(path=str(empty)))
        res.add(f'{side}.helper.connect_first_response_ms', (time.monotonic() - helper.spawned) * 1000)
        rtts = [helper.request('list', dict(path=str(empty)))[0] for _ in range(20)]
        res.add(f'{side}.helper.min_request_ms', rtts)
        info = [helper.request('info')[0] for _ in range(3)]
        res.add(f'{side}.helper.info_ms', info)
        for n in fx.sizes:
            d = str(fx.dirs[n] / 'big')
            base = f'{side}.helper.list.{n}'
            first_ms, first_bytes, first = helper.request('list', dict(path=d))
            entries = len(first['entries'])
            res.add(f'{base}.first_page_ms', first_ms)
            res.add(f'{base}.first_page_entries', entries, unit='entries', better='info')
            res.add(f'{base}.first_page_bytes', first_bytes, unit='bytes', better='lower')
            res.add(f'{base}.bytes_per_entry', first_bytes / max(entries, 1), unit='bytes', better='lower')
            # UI pattern: continuation pages until UI_CAP entries, 4000 per page once the helper
            # reports its `order` (older helpers: 1000). The metric keeps its baseline name.
            limit = 4000 if first.get('order') else 1000
            offset, loaded, pages, t_ui = first.get('next_offset'), entries, [], first_ms
            while offset is not None and loaded < UI_CAP:
                p_ms, _, page = helper.request('list_page', dict(path=d, offset=offset, limit=limit))
                pages.append(p_ms)
                t_ui += p_ms
                loaded += len(page['entries'])
                offset = page.get('next_offset')
            if pages:
                res.add(f'{base}.page1000_ms', pages)
            res.add(f'{base}.ui_pattern_total_ms', t_ui)
            res.add(f'{base}.ui_pattern_requests', 1 + len(pages), unit='requests', better='lower')
            # Full listing with the largest page the helper accepts.
            t_full, offset, total_bytes, count, requests = first_ms, first.get('next_offset'), first_bytes, entries, 1
            while offset is not None:
                p_ms, p_bytes, page = helper.request('list_page', dict(path=d, offset=offset, limit=4000))
                t_full += p_ms
                total_bytes += p_bytes
                count += len(page['entries'])
                requests += 1
                offset = page.get('next_offset')
            res.add(f'{base}.full_listing_ms', t_full)
            res.add(f'{base}.full_listing_requests', requests, unit='requests', better='lower')
            res.add(f'{base}.full_listing_entries', count, unit='entries', better='higher')
            res.add(f'{base}.full_listing_bytes', total_bytes, unit='bytes', better='lower')
            if side == 'local':
                scan, stat = fs_floor(d, entries)
                res.add(f'fs.list.{n}.scandir_sort_ms', scan)
                res.add(f'fs.list.{n}.lstat_first_page_ms', stat)
        for key, spec in fx.files.items():
            drop_file_cache(spec['path'])
            p_ms, p_bytes, preview = helper.request('preview', dict(path=str(spec['path'])))
            res.add(f'{side}.helper.preview.{key}.ms', p_ms)
            res.add(f'{side}.helper.preview.{key}.bytes', p_bytes, unit='bytes', better='info')
            warm = [helper.request('preview', dict(path=str(spec['path'])))[0] for _ in range(3)]
            res.add(f'{side}.helper.preview.{key}.warm_ms', warm)
            res.details.setdefault(f'{side}.helper.preview.{key}', dict(
                kind=preview.get('kind'), truncated=preview.get('truncated'),
                text_chars=len(preview.get('text') or ''), file_bytes=spec['bytes']))
    except RuntimeError as error:
        res.notes.append(f'run {run} {side} helper: {error}')
    finally:
        helper.close()


# ---------------------------------------------------------------------------
TABLE_UNITS = {'ms': 'ms', 'entries': '', 'bytes': 'B', 'bool': '', 'requests': ''}


def fmt(v):
    if v is None:
        return '-'
    if isinstance(v, float):
        return f'{v:.1f}' if abs(v) < 1000 else f'{v:.0f}'
    return str(v)


def markdown(report):
    m = report['metrics']
    remote_label = 'off'
    if report['remote']:
        remote_label = 'loopback sshd' + (f" + {report['rtt_ms']} ms relay" if report['rtt_ms'] else '')
    out = [f"# cx Files benchmark · {report['label']}", '',
           f"Binary `{report['binary_version']}` sha256 `{report['binary_sha256'][:12]}`, "
           f"commit `{report.get('git_commit') or '?'}`, runs {report['runs']}, "
           f"load {report['host']['loadavg_before'][0]} -> {report['host']['loadavg_after'][0]} on {report['host']['cpus']} CPUs, remote "
           f"{remote_label}. "
           'Values: median / p95 in ms unless noted. Cold = fresh viewer process (OS-warm directory metadata).'
           + (f" Remote viewers settle {report['remote_settle_s']} s after startup before each measured action"
              ' (see the startup table for the unsettled case).' if report['remote'] else ''), '']

    def present(prefix):
        return any(k.startswith(prefix) for k in m)

    def cell(name, digits=None):
        x = m.get(name)
        if not x or x['median'] is None:
            return 'fail' if x and x['failed'] else '-'
        return f"{fmt(x['median'])} / {fmt(x['p95'])}"
    sides = [s for s in ['local'] + (['remote'] if report['remote'] else []) if present(f'{s}.')]
    if present('local.startup') or present('remote.startup'):
        out += ['## Straight after startup (Enter as soon as the first listing paints)', '',
                '| Side | Viewer ready | 2 KB preview first paint | Directory first rows |', '|---|---|---|---|']
        for side in sides:
            if not present(f'{side}.startup.'):
                continue
            d = next((k[:-3] for k in m if k.startswith(f'{side}.startup.dir_') and k.endswith('_ms')), '')
            out.append(f"| {side} | {cell(side + '.startup.viewer_ready_ms')} | "
                       f"{cell(side + '.startup.preview_src_2k_first_paint_ms')} | {cell(d + '_ms') if d else '-'} |")
        out.append('')
    out += ['## Directory open (Enter -> ...)', '',
            '| Side | Entries | Case | First rows | Usable (j moves) | Full count | Final count |',
            '|---|---|---|---|---|---|---|']
    for side in sides:
        for n in report['sizes']:
            for case in ('cold', 'warm', 'sibling'):
                b = f'{side}.dir.{n}.{case}'
                if f'{b}.first_rows_ms' in m:
                    fc = m.get(f'{b}.final_count', {}).get('median')
                    out.append(f'| {side} | {n} | {case} | {cell(b + ".first_rows_ms")} | {cell(b + ".usable_ms")} | '
                               f'{cell(b + ".full_count_ms")} | {fmt(fc)} |')
    out += ['', '| Side | Entries | Parent (h) | j in loaded list |', '|---|---|---|---|']
    for side in sides:
        for n in report['sizes']:
            b = f'{side}.dir.{n}'
            if f'{b}.parent_ms' in m:
                out.append(f'| {side} | {n} | {cell(b + ".parent_ms")} | {cell(b + ".j_in_list_ms")} |')
    out += ['', f'Deep tree ({DEEP_LEVELS} levels, 10 files each): per hop', '',
            '| Side | Enter hop | Parent hop | Enter total |', '|---|---|---|---|']
    for side in sides:
        if present(f'{side}.deep.'):
            out.append(f'| {side} | {cell(side + ".deep.enter_hop_ms")} | {cell(side + ".deep.parent_hop_ms")} | '
                   f'{cell(side + ".deep.enter_total_ms")} |')
    out += ['', '## File preview (Enter -> first content painted; G -> end)', '',
            '| Side | File | Bytes | First paint | G | End reachable | Helper request (cold) | Response bytes |',
            '|---|---|---|---|---|---|---|---|']
    for side in sides:
        for key, spec in report['fixtures']['files'].items():
            b = f'{side}.preview.{key}'
            if not present(b) and not present(f'{side}.helper.preview.{key}.'):
                continue
            reach = m.get(b + '.end_reachable')
            reach = '-' if not reach or reach['median'] is None else ('yes' if reach['median'] >= 1 else 'no')
            out.append(f"| {side} | {key} | {spec['bytes']} | {cell(b + '.first_paint_ms')} | {cell(b + '.end_ms')} | "
                       f"{reach} | {cell(f'{side}.helper.preview.{key}.ms')} | "
                       f"{fmt(m.get(f'{side}.helper.preview.{key}.bytes', {}).get('median'))} |")
    out += ['', '## Preview scrolling (key -> first changed paint, p50 / p95 over all presses)', '',
            '| Side | File | j | PageDown | Wheel | j settled |', '|---|---|---|---|---|---|']
    for side in sides:
        for key in ('src_5m', 'md_2m'):
            b = f'{side}.scroll.{key}'
            if not present(b):
                continue
            out.append(f'| {side} | {key} | {cell(b + ".j.key_to_paint_ms")} | {cell(b + ".pagedown.key_to_paint_ms")} | '
                       f'{cell(b + ".wheel.key_to_paint_ms")} | {cell(b + ".j.settled_ms")} |')
    out += ['', '## Helper protocol microbenchmark (framed `cx helper`, no UI)', '',
            '| Side | Connect -> first response | Minimal request | Info |', '|---|---|---|---|']
    for side in sides:
        b = f'{side}.helper'
        if not present(b + '.'):
            continue
        out.append(f'| {side} | {cell(b + ".connect_first_response_ms")} | {cell(b + ".min_request_ms")} | '
                   f'{cell(b + ".info_ms")} |')
    out += ['', '| Side | Entries | First page | Entries/page | Bytes/entry | 1000-page | UI pattern total (requests) | '
            'Full listing (requests) | fs floor scandir+sort / lstat page |', '|---|---|---|---|---|---|---|---|---|']
    for side in sides:
        for n in report['sizes']:
            b = f'{side}.helper.list.{n}'
            if f'{b}.first_page_ms' not in m:
                continue
            g = lambda k: m.get(f'{b}.{k}', {}).get('median')  # noqa: E731
            floor = (f"{fmt(m.get(f'fs.list.{n}.scandir_sort_ms', {}).get('median'))} / "
                     f"{fmt(m.get(f'fs.list.{n}.lstat_first_page_ms', {}).get('median'))}") if side == 'local' else ''
            out.append(f"| {side} | {n} | {cell(b + '.first_page_ms')} | {fmt(g('first_page_entries'))} | "
                       f"{fmt(g('bytes_per_entry'))} | {cell(b + '.page1000_ms')} | {cell(b + '.ui_pattern_total_ms')} "
                       f"({fmt(g('ui_pattern_requests'))}) | {cell(b + '.full_listing_ms')} ({fmt(g('full_listing_requests'))}) "
                       f"| {floor} |")
    if report['notes']:
        out += ['', '## Notes', ''] + [f'- {n}' for n in report['notes']]
    return '\n'.join(out) + '\n'


def main():
    parser = argparse.ArgumentParser(description=__doc__.split('\n')[0])
    parser.add_argument('binary')
    parser.add_argument('--remote', action='store_true', help='also measure over a loopback sshd')
    parser.add_argument('--runs', type=int, default=3)
    parser.add_argument('--out')
    parser.add_argument('--label', default='run')
    parser.add_argument('--rtt', type=float, default=0.0, help='added round-trip ms for --remote (relay)')
    parser.add_argument('--oplog', help='diagnostic: log remote helper ops (names, sizes, helper ms) to FILE')
    parser.add_argument('--only', default='', help='comma list of startup,dirs,deep,previews,scroll,helper')
    parser.add_argument('--files', default='', help='comma list of preview fixture keys (default all)')
    parser.add_argument('--sides', default='local,remote', help='comma list: local,remote')
    parser.add_argument('--sizes', default=','.join(map(str, SIZES)))
    args = parser.parse_args()
    binary = str(pathlib.Path(args.binary).resolve())
    for tool in ('tmux',) + (('ssh', 'ssh-keygen') if args.remote else ()):
        if not shutil.which(tool):
            sys.exit(f'{tool} is required')
    if args.remote and not (shutil.which('sshd') or os.path.exists('/usr/sbin/sshd')):
        sys.exit('sshd is required for --remote')
    for key in ('TMUX', 'TMUX_PANE'):
        os.environ.pop(key, None)
    only = {s for s in args.only.split(',') if s} or {'startup', 'dirs', 'deep', 'previews', 'scroll', 'helper',
                                                      'nav'}
    sizes = tuple(int(s) for s in args.sizes.split(',') if s)
    tag = uuid.uuid4().hex
    res = Results()

    def stop(signum, frame):
        raise SystemExit(128 + signum)
    signal.signal(signal.SIGTERM, stop)
    signal.signal(signal.SIGHUP, stop)
    version = subprocess.run([binary, '--version'], capture_output=True, text=True).stdout.strip()
    sha = hashlib.sha256(pathlib.Path(binary).read_bytes()).hexdigest()
    commit = subprocess.run(['git', '-C', str(REPO), 'rev-parse', '--short', 'HEAD'], capture_output=True,
                            text=True).stdout.strip() or None
    started = time.monotonic()
    load_before = os.getloadavg()
    fx = Fixtures(sizes)
    ssh = None
    killed = 0
    try:
        line_maps = {k: numbered_lines(s['path'].read_text()[:256 * 1024])
                     for k, s in fx.files.items() if s['numbered']}
        if args.remote:
            ssh = LoopbackSSH(binary, tag, args.rtt, args.oplog and str(pathlib.Path(args.oplog).resolve()))
        wanted = set(args.sides.split(','))
        sides = [(n, i) for n, i in [('local', 0)] + ([('remote', 1)] if ssh else []) if n in wanted]
        if args.files:
            keep = set(args.files.split(','))
            fx.files = {k: v for k, v in fx.files.items() if k in keep or k in ('src_5m', 'md_2m', 'src_200k')}
            fx.preview_keys = [k for k in fx.files if k in keep]
        for run in range(args.runs):
            for side, index in sides:
                remote = ssh if side == 'remote' else None
                if 'helper' in only:
                    home = pathlib.Path(tempfile.mkdtemp(prefix='cx-fb-helper-'))
                    try:
                        env = dict(os.environ, HOME=str(home), XDG_STATE_HOME=str(home / 'state'),
                                   TMUX_TMPDIR=str(home), CX_FILES_BENCH=tag)
                        env.pop('SSH_AUTH_SOCK', None)
                        argv = remote.helper_argv() if remote else [binary, 'helper']
                        helper_bench(fx, side, argv, env, res, run)
                    finally:
                        shutil.rmtree(home, ignore_errors=True)
                if 'startup' in only:
                    scenario_startup(binary, fx, side, index, remote, tag, res, run)
                if 'dirs' in only:
                    scenario_dirs(binary, fx, side, index, remote, tag, res, run)
                if 'deep' in only:
                    scenario_deep(binary, fx, side, index, remote, tag, res, run)
                if 'previews' in only:
                    scenario_previews(binary, fx, side, index, remote, tag, res, run, line_maps)
                if 'scroll' in only:
                    scenario_scroll(binary, fx, side, index, remote, tag, res, run)
                if 'nav' in only:
                    scenario_nav(binary, fx, side, index, remote, tag, res, run)
                print(f'run {run + 1}/{args.runs} {side} done at {time.monotonic() - started:.0f}s', file=sys.stderr)
    finally:
        refused = ssh.refused() if ssh else {}
        if ssh:
            killed += ssh.cleanup()
        killed += kill_tagged(tag)
        survivors = len(tagged_pids(tag, sshd=True))
        fx_summary = dict(build_s=fx.build_s, sizes=list(sizes), deep_levels=DEEP_LEVELS,
                          files={k: dict(name=s['path'].name, bytes=s['bytes'], kind=s['kind'])
                                 for k, s in fx.files.items()})
        fx.cleanup()
    res.finish()
    cleanups = res.details.get('cleanup', [])
    dropped = res.details.get('page_cache_dropped', [])
    report = dict(
        schema=1, harness='tests/files_bench.py', label=args.label, binary=binary, binary_version=version,
        binary_sha256=sha, git_commit=commit, runs=args.runs, remote=bool(ssh), rtt_ms=args.rtt,
        sizes=list(sizes), only=sorted(only), remote_settle_s=REMOTE_SETTLE if ssh else None, terminal=dict(cols=ph.COLS, rows=ph.ROWS),
        host=dict(kernel=platform.release(), cpus=os.cpu_count(), python=platform.python_version(),
                  loadavg_before=[round(x, 1) for x in load_before],
                  loadavg_after=[round(x, 1) for x in os.getloadavg()]),
        cache=dict(directories='cx-cold, OS-warm (no root to drop dentry/page caches)',
                   files='posix_fadvise(DONTNEED) before each cold preview; fincore-confirmed '
                         f'{sum(1 for d in dropped if d)}/{len(dropped)}'),
        cleanup=dict(viewers=len(cleanups), remaining=sum(c['remaining'] for c in cleanups),
                     orphans_ours_stray=sum(c['orphans_ours_stray'] for c in cleanups),
                     stale_handoffs=sum(c['stale_handoffs'] for c in cleanups),
                     tagged_killed_at_end=killed, tagged_survivors=survivors, ssh_commands_refused=refused),
        fixtures=fx_summary, wall_s=round(time.monotonic() - started, 1),
        notes=res.notes, metrics=res.metrics,
        details={k: v for k, v in res.details.items() if k not in ('cleanup',)})
    ok = not res.notes and survivors == 0 and report['cleanup']['remaining'] == 0 \
        and report['cleanup']['orphans_ours_stray'] == 0
    report['result'] = 'PASS' if ok else 'FAIL'
    out = pathlib.Path(args.out or REPO / f'local-evidence/perf/files-bench-{args.label}.json')
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(report, indent=1) + '\n')
    table = markdown(report)
    out.with_suffix('.md').write_text(table)
    print(table)
    print(f'JSON: {out}', file=sys.stderr)
    sys.exit(0 if ok else 1)


def self_test():
    """The screen readers the latency predicates use read pyte's display (rebuilt on
    every access) a bounded number of times: one per line made `rows` cost ~40 ms per
    check, which landed in every first-rows number and doubled it when a frame came in
    two reads (the v0.2.1 review's 100k sibling outlier)."""
    class Screen:
        reads = 0

        @property
        def display(self):
            Screen.reads += 1
            lines = [' cx › local › /n100000/sibling'.ljust(120)]
            lines += [f' ▌  s-{i:06d}.txt'.ljust(120) for i in range(1, 39)]
            return lines + [' '.ljust(120)]
    viewer = Viewer.__new__(Viewer)
    viewer.screen = Screen()
    for reader, check in ((lambda: viewer.rows('s'), lambda r: len(r) == 38),
                          (viewer.selected, lambda r: r == 's-000001.txt'),
                          (viewer.work, lambda r: len(r) == 40)):
        Screen.reads = 0
        result = reader()
        assert check(result), result
        assert Screen.reads <= 2, f'{reader}: {Screen.reads} display reads'
    print('files_bench self-test: PASS')


if __name__ == '__main__':
    if sys.argv[1:] == ['--self-test']:
        self_test()
    else:
        main()
