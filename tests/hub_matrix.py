#!/usr/bin/env python3
"""Real-app hub screenshot matrix: the real cx viewer on a PTY, its devices served by real
`cx helper` processes, across device counts, meshes, states, sizes and colour tiers.

usage: hub_matrix.py BINARY [--configs a,b] [--sizes WxH,...] [--tiers dark,light,16,ascii]
                            [--jobs N] [--run NAME] [--lab DIR | --no-lab]

Isolation: the whole run is inside a private user, pid, uts, mount, cgroup and network namespace (`unshare
-rnmCpfu`, then the caller's uid again; /run/user/UID is an empty tmpfs there): a bridge `lan0` (192.168.77.1/24) with permanent neighbour entries
is the fixture lan, loopback carries a fake Cloudflare mesh Worker, and no packet leaves.
Every device has its own HOME (tmux server, sessions, vitals from a CX_METRICS_ROOT tree).
`ssh` on PATH runs `cx helper` under that HOME (CX_SSH_MULTIPLEX=0), refuses for a down
device and hangs for one never reached; `tailscale` prints a fixture tailnet; ssh-overlay
pairs are pinned peers in the viewer's state; the WARP mesh is a config whose Worker answers
on loopback. Sessions are real tmux panes: a shell at its prompt (idle), a shell in a loop
(running), and a copy of bash named claude whose native record says waiting or working
(needs you / working). Nothing touches the caller's devices, HOME or networking.

Configs nebula-N mirror the lab fixture `nebula-N` (same names, meshes, lan, relays, down and
never-reached devices), so their maps are compared with the lab's render of it; `long-5`
has no mesh, long names and eight sessions.

Each capture (config x tier x size) is kept as text, cell JSON and PNG under
~/.cache/cx-hub-matrix/<run>/, with contact.png (dark) and contact-<tier>.png sheets.
Checks per capture: the viewer is alive (no panic); device labels never overlap; every
device's name shows or its folded cluster counts it (letters only on the smallest sizes);
no cell has a background or reverse video (the sessions print none); the vitals strip shows
the cpu sparkline, mem and uptime, and temperature exactly when the device has a sensor;
remote devices show cpu and rtt; curves are drawn; each session state has its badge in its
own colour, the lab's; and the map's labels sit where the lab puts them (within 2 rows and
4 columns, for 80% of the labels).
"""
import argparse, concurrent.futures, http.server, json, os, pathlib, re, shutil, signal
import subprocess, sys, tempfile, threading, time, uuid

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import pty_lib  # noqa: E402

SIZES = [(40, 20), (60, 30), (80, 24), (95, 50), (120, 40), (190, 50)]
TIERS = ['dark', 'light', '16', 'ascii']
LAB_VARIANT = {'dark': 'dark-truecolor', 'light': 'light-truecolor', '16': 'dark-16', 'ascii': 'ascii'}
NAMES = ['innovation', 'creation', 'nas', 'laptop-guest', 'blastoise-odroid', 'squirtle-jetson',
         'render-farm-a', 'render-farm-b', 'pixel-9-termux', 'homeassistant', 'ci-runner-arm64',
         'ci-runner-x86', 'drone-ground', 'lab-bench-scope', 'charmander-rpi5', 'bulbasaur-orin',
         'studio-mac-mini', 'old-thinkpad', 'vps-fra1', 'ipad-blink', 'picker-robot-07',
         'picker-robot-08', 'warehouse-cam', 'garage-door', 'media-box']
LONG = ['an-extraordinarily-long-workstation-name', 'build-server-in-the-basement-rack-02',
        'the-living-room-media-center-box', 'experimental-arm64-board-on-the-desk']
MESH_NAMES = ['cloudflare-warp', 'ssh-overlay', 'tailnet']
SPARK = set('▁▂▃▄▅▆▇█')
BRAILLE = re.compile('[⠁-⣿]')
# What each session state says (src/ui/views/mission/mod.rs `badge`), in attention order.
STATES = ['needs you', 'working', 'running', 'idle']


def nebula(n, sessions):
    """The lab's `nebula(n)` (src/ui/views/state/fixtures.rs) as real devices."""
    meshes = 1 if n <= 3 else 2 if n <= 10 else 3
    devices = []
    for i, name in enumerate(NAMES[:max(n - 1, 0)]):
        devices.append(dict(name=name, state={4: 'down', 7: 'hang'}.get(i % 9, 'up'),
                            mesh=i % meshes, lan=i % 5 == 2,
                            relay=NAMES[i - 1] if i >= 4 and i % 4 == 3 else None,
                            seen=3 + 600 * i, temp=None if i % 2 else 40 + i,
                            cpu=(i * 0.173) % 1.0))
    return dict(name=f'nebula-{n}', lab=f'nebula-{n}', meshes=meshes if n > 1 else 0,
                devices=devices, sessions=sessions, temp=47.0)


def configs():
    four = [('local', s) for s in STATES]
    out = [nebula(1, []), nebula(3, four), nebula(5, four),
           nebula(7, four + [('innovation', 'needs you'), ('innovation', 'idle')]),
           nebula(10, four), nebula(15, four[:2]), nebula(25, four)]
    long5 = dict(name='long-5', lab=None, meshes=0, temp=None, sessions=four + four,
                 devices=[dict(name=nm, state='down' if i == 3 else 'up', mesh=None, lan=i == 1,
                               relay=None, seen=7200, temp=None, cpu=0.3 + 0.1 * i)
                          for i, nm in enumerate(LONG)])
    out.append(long5)
    return {c['name']: c for c in out}


# ---------------------------------------------------------------------------- namespace

def enter_namespace():
    """Re-run this script inside a private user + network namespace with the fixture lan."""
    if os.environ.get('CX_HUB_MATRIX_NS') == '1':
        return
    if subprocess.run(['unshare', '-UrnmCpfu', '--mount-proc', 'true'], capture_output=True).returncode != 0:
        sys.exit('hub_matrix: unprivileged user/network namespaces are unavailable')
    neigh = ' && '.join(f'ip neigh add 192.168.77.{10 + i} lladdr 02:00:00:00:00:{10 + i:02x} '
                     f'dev lan0 nud permanent' for i in range(40))
    # A private mount namespace hides the caller's user bus and runtime directory: the
    # fixture's tmux servers never become units of the caller's systemd.
    setup = (f'mount -t tmpfs -o mode=700 tmpfs /run/user/{os.getuid()} && '
             f'{sys.executable} -c "import socket; socket.sethostname(\\"fixture-host\\")" && ip link set lo up && ip link add lan0 type bridge && '
             'ip addr add 192.168.77.1/24 dev lan0 && ip link set lan0 up && ' + neigh)
    uid, gid = os.getuid(), os.getgid()
    inner = ['unshare', '-U', f'--map-user={uid}', f'--map-group={gid}',
             sys.executable, *sys.argv]
    env = dict(os.environ, CX_HUB_MATRIX_NS='1')
    script = setup + ' && exec "$@"'
    os.execvpe('unshare', ['unshare', '-rnmCpfu', '--mount-proc', 'sh', '-c', script, 'sh', *inner], env)


# ---------------------------------------------------------------------------- fixtures

FAKE_SSH = r'''#!/usr/bin/env python3
import json, os, sys, time
args = sys.argv[1:]
if len(args) < 2 or 'cx helper' not in args[-1]:
    sys.exit(255)
target = args[-2]
d = json.load(open(os.environ['CX_HM_TABLE'])).get(target)
if d is None or d['mode'] == 'down':
    sys.stderr.write('ssh: connect to host %s port 22: Connection refused\n' % target.split('@')[-1])
    sys.exit(255)
if d['mode'] == 'hang':
    time.sleep(3600)
    sys.exit(255)
env = dict(os.environ, HOME=d['home'], XDG_STATE_HOME=d['home'] + '/state', TMUX_TMPDIR=d['home'],
           CX_METRICS_ROOT=d['proc'], XDG_RUNTIME_DIR=d['home'] + '/run')
os.execve(d['cx'], [d['cx'], 'helper'], env)
'''
FAKE_TAILSCALE = '#!/bin/sh\n[ "$1" = status ] && exec cat "$CX_HM_TAILNET"\nexit 1\n'


class Worker(http.server.BaseHTTPRequestHandler):
    """The WARP mesh Worker's admin devices list, on loopback inside the namespace."""
    enrolled = []

    def do_GET(self):
        body = json.dumps(dict(dns_suffix='.mesh', enrolled=self.server.enrolled, connectors=[], dns=[]))
        self.send_response(200 if self.path == '/v1/admin/devices' else 404)
        self.send_header('content-type', 'application/json')
        self.end_headers()
        self.wfile.write(body.encode())

    def log_message(self, *a):
        pass


class Fleet:
    """One config's devices, sessions and viewer HOME under a private root."""

    def __init__(self, binary, cfg, root):
        self.binary, self.cfg, self.root = binary, cfg, pathlib.Path(root)
        self.run_id = uuid.uuid4().hex
        self.viewer = self.root / 'viewer'
        self.state = self.viewer / 'state/cx'
        self.state.mkdir(parents=True)
        (self.viewer / 'run').mkdir(mode=0o700)
        fake = self.root / 'fakebin'
        fake.mkdir()
        (fake / 'ssh').write_text(FAKE_SSH)
        (fake / 'tailscale').write_text(FAKE_TAILSCALE)
        for f in fake.iterdir():
            f.chmod(0o700)
        self.env = {k: v for k, v in os.environ.items()
                    if k not in ('TMUX', 'TMUX_PANE', 'NO_COLOR', 'CX_ASCII', 'COLORTERM', 'COLORFGBG')
                    and not k.lower().endswith('_proxy')}
        self.env.update(HOME=str(self.viewer), XDG_STATE_HOME=str(self.viewer / 'state'),
                        XDG_CONFIG_HOME=str(self.viewer / '.config'), XDG_RUNTIME_DIR=str(self.viewer / 'run'),
                        TMUX_TMPDIR=str(self.viewer), SHELL='/bin/bash', HISTFILE='/dev/null',
                        PATH=f"{fake}:/usr/bin:/bin", CX_SSH_MULTIPLEX='0', CX_HM_RUN=self.run_id,
                        CX_HM_TABLE=str(self.root / 'table.json'), CX_HM_TAILNET=str(self.root / 'tailnet.json'),
                        CX_METRICS_ROOT=str(self.root / 'proc/local'), CLOUDFLARE_API_TOKEN='fixture-token',
                        CX_REDUCED_MOTION='1', no_proxy='*')
        self.server = None
        self.stop = threading.Event()
        self.build()

    def target(self, i, d):
        return f'fx@192.168.77.{10 + i}' if d['lan'] else f"fx@{d['name']}"

    def build(self):
        cfg = self.cfg
        pty_lib.metrics_fixture(self.root / 'proc/local', busy=300, total=1000, temp=cfg['temp'],
                                os_release='ID=arch\n')
        devices = [dict(id='local', name='tranquility', target=None, account='fx', host='tranquility',
                        status='unknown', observed_at=0)]
        table, routes, seen, pins, warp, tail = {}, {}, [], {}, [], {}
        for i, d in enumerate(cfg['devices']):
            home = self.root / 'dev' / d['name']
            (home / 'state').mkdir(parents=True)
            (home / 'run').mkdir(mode=0o700)
            proc = pty_lib.metrics_fixture(self.root / 'proc' / d['name'], busy=int(d['cpu'] * 1000),
                                           total=1000, temp=d['temp'], mem_kb=(8 << 20, int(4.8 * (1 << 20))))
            target = self.target(i, d)
            table[target] = dict(home=str(home), proc=str(proc), cx=self.binary,
                                 mode={'up': 'up', 'down': 'down', 'hang': 'hang'}[d['state']])
            devices.append(dict(id=f'dev{i}', name=d['name'], target=target, account='fx', host=d['name'],
                                status='unknown', observed_at=0))
            if d['relay']:
                j = next(k for k, x in enumerate(cfg['devices']) if x['name'] == d['relay'])
                routes[target] = [self.target(j, cfg['devices'][j])]
            if d['state'] == 'down':
                seen.append((f'dev{i}', d, target))
            if d['mesh'] == 0:
                warp.append(dict(name=d['name'], host=d['name'], os='linux', ipv4=f'100.96.0.{10 + i}',
                                 ipv6=None, state='active'))
            elif d['mesh'] == 1:
                pins[target] = dict(device_id=f'dev{i}', host=target.split('@')[1],
                                    key='ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIFixtureFixtureFixtureFixtureFixture00',
                                    client_key='ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIFixtureFixtureFixtureFixtureFixture01')
            elif d['mesh'] == 2:
                tail[f'node{i}'] = dict(HostName=d['name'], DNSName=f"{d['name']}.tailnet.ts.net.",
                                        TailscaleIPs=[f'100.100.0.{10 + i}'], Online=True, OS='linux')
        self.table = table
        (self.root / 'table.json').write_text(json.dumps(table))
        (self.state / 'devices.json').write_text(json.dumps(devices))
        if routes:
            p = self.state / 'routes.json'
            p.write_text(json.dumps(routes))
            p.chmod(0o600)
        # Down devices were seen before this run (`seen 40m`): the navigation cache says when.
        now = int(time.time())
        cache = dict(version=1, devices=[dict(device=dict(id=i, name=d['name'], target=t, account='fx',
                                                          host=d['name'], status='unknown',
                                                          observed_at=now - d['seen']),
                                              sessions=[], browsers=[]) for i, d, t in seen])
        (self.state / 'ui-navigation.json').write_text(json.dumps(cache))
        (self.state / 'ui-navigation.json').chmod(0o600)
        if pins:
            peer = self.state / 'peer-ssh'
            peer.mkdir(mode=0o700)
            for name, text in (('peers.json', json.dumps(dict(endpoints=pins))), ('id_ed25519', 'fixture\n'),
                               ('known_hosts', '')):
                (peer / name).write_text(text)
                (peer / name).chmod(0o600)
        if tail:
            (self.root / 'tailnet.json').write_text(json.dumps(dict(
                BackendState='Running', CurrentTailnet=dict(Name='tailnet', MagicDNSSuffix='tailnet.ts.net'),
                Self=dict(HostName='tranquility', DNSName='tranquility.tailnet.ts.net.',
                          TailscaleIPs=['100.100.0.1'], Online=True, OS='linux'), Peer=tail)))
        if warp:
            self.server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Worker)
            self.server.enrolled = warp
            threading.Thread(target=self.server.serve_forever, daemon=True).start()
            mesh = self.viewer / '.config/cx/mesh'
            mesh.mkdir(parents=True)
            (mesh / 'config.toml').write_text('name = "cloudflare-warp"\naccount = "fixture"\n')
            (mesh / 'cloudflare-warp.state.json').write_text(
                json.dumps(dict(url=f'http://127.0.0.1:{self.server.server_address[1]}')))
        # Live cpu: the fixture jiffies advance in a wave, so sparklines have a shape.
        threading.Thread(target=self.tick, daemon=True).start()
        for where, state in self.cfg['sessions']:
            self.session(where, state)

    def tick(self):
        acc = {}
        step = 0
        while not self.stop.wait(0.5):
            step += 1
            for d in [dict(name='local', cpu=0.3)] + self.cfg['devices']:
                busy, total = acc.get(d['name'], (0, 0))
                load = min(1.0, d['cpu'] * (0.5 + (step % 7) / 6))
                busy, total = busy + int(100 * load), total + 100
                acc[d['name']] = (busy, total)
                stat = self.root / 'proc' / d['name'] / 'proc/stat'
                lines = stat.read_text().splitlines()
                lines[0] = f'cpu  {busy} 0 0 {total - busy} 0 0 0 0 0 0'
                tmp = stat.with_suffix('.tmp')
                tmp.write_text('\n'.join(lines) + '\n')
                tmp.replace(stat)

    def device_env(self, where):
        if where == 'local':
            return self.env
        d = next(x for x in self.table.values() if x['home'].endswith('/' + where))
        return dict(self.env, HOME=d['home'], XDG_STATE_HOME=d['home'] + '/state', TMUX_TMPDIR=d['home'],
                    CX_METRICS_ROOT=d['proc'], XDG_RUNTIME_DIR=d['home'] + '/run')

    def session(self, where, state):
        env = self.device_env(where)
        home = pathlib.Path(env['HOME'])
        n = len(list((home / 'work').glob('*'))) if (home / 'work').exists() else 0
        folder = home / 'work' / f's{n}'
        folder.mkdir(parents=True)
        title = {'needs you': 'fix flaky test', 'working': 'refactor transport', 'running': 'watch logs',
                 'idle': 'notes'}[state]
        out = subprocess.run([self.binary, 'new', '--provider', 'shell', '--directory', str(folder),
                              '--name', title, '--key', f'hm{n}'], env=env, capture_output=True, timeout=30)
        assert out.returncode == 0, out.stderr.decode()[-400:]
        sid = json.loads(out.stdout)['id']
        sock = str(home / 'state/cx/managed.sock')

        def send(text):
            subprocess.run(['tmux', '-S', sock, 'send-keys', '-t', sid, '-l', text], env=env, check=True)
            subprocess.run(['tmux', '-S', sock, 'send-keys', '-t', sid, 'Enter'], env=env, check=True)
        if state == 'idle':
            send("clear; printf 'hm idle marker\\n'")
        elif state == 'running':
            send("clear; while :; do echo 'hm running tick'; sleep 1; done")
        else:
            claude = home / 'bin/claude'
            if not claude.exists():
                claude.parent.mkdir(exist_ok=True)
                shutil.copy2(shutil.which('bash'), claude)
            words = 'hm asks: allow the fixture to run?' if state == 'needs you' else 'hm works: compiling'
            send(f"clear; {claude} -c \"printf '{words}\\\\n'; sleep 100000; :\"")
            row = self.wait_row(env, sid, lambda r: r.get('provider') == 'claude')
            rec = home / '.claude/sessions'
            rec.mkdir(parents=True, exist_ok=True)
            p = row['process']
            (rec / f"{p['pid']}.json").write_text(json.dumps(dict(
                pid=p['pid'], procStart=p['start_ticks'], sessionId=str(uuid.uuid4()), cwd=str(folder),
                name=title, status='waiting_for_input' if state == 'needs you' else 'working')))

    def wait_row(self, env, sid, check):
        end = time.monotonic() + 15
        while time.monotonic() < end:
            rows = json.loads(subprocess.run([self.binary, 'sessions'], env=env, capture_output=True,
                                             timeout=30).stdout or b'[]')
            row = next((r for r in rows if r['id'] == sid), None)
            if row and check(row):
                return row
            time.sleep(0.1)
        raise AssertionError('fixture session never matched')

    def tier_env(self, tier):
        env = dict(self.env)
        env.update(TERM='xterm-256color', COLORTERM='truecolor', CX_THEME='dark')
        if tier == 'light':
            env['CX_THEME'] = 'light'
        elif tier == '16':
            env.update(TERM='xterm')
            env.pop('COLORTERM')
        elif tier == 'ascii':
            env['CX_ASCII'] = '1'
        return env

    def cleanup(self):
        self.stop.set()
        if self.server:
            self.server.shutdown()
        for home in [self.viewer] + [pathlib.Path(d['home']) for d in self.table.values()]:
            subprocess.run(['tmux', '-S', str(home / 'state/cx/managed.sock'), 'kill-server'],
                           capture_output=True, env=self.env)
        marker = f'CX_HM_RUN={self.run_id}'.encode()
        for _ in range(30):
            pids = []
            for e in pathlib.Path('/proc').iterdir():
                try:
                    if e.name.isdigit() and int(e.name) != os.getpid() and marker in (e / 'environ').read_bytes():
                        pids.append(int(e.name))
                except OSError:
                    pass
            if not pids:
                return
            for pid in pids:
                try:
                    os.kill(pid, signal.SIGKILL)
                except OSError:
                    pass
            time.sleep(0.1)


# ---------------------------------------------------------------------------- capture

def settle(pty, quiet=0.3, limit=6.0):
    """Read until `quiet` seconds pass without output (or `limit`)."""
    end = time.monotonic() + limit
    last = time.monotonic()
    while time.monotonic() < end:
        if pty.read(0.05):
            last = time.monotonic()
        elif time.monotonic() - last >= quiet:
            return True
    return False


def ready(fleet, text):
    """Every live part has landed: this device's vitals, a remote's latency, the sessions."""
    if '% of' not in text:
        return False
    if any(d['state'] == 'up' for d in fleet.cfg['devices']) and 'ms' not in text:
        return False
    return 'needs you' in text or 'needs you' not in [s for _, s in fleet.cfg['sessions']]


def capture_config(binary, cfg, tiers, sizes, out, lab):
    results = []
    with tempfile.TemporaryDirectory(prefix='cx-hm-') as tmp:
        fleet = Fleet(binary, cfg, tmp)
        try:
            for tier in tiers:
                big = max(sizes)
                pty = pty_lib.Pty([binary], fleet.tier_env(tier), fleet.viewer, *big)
                try:
                    end = time.monotonic() + 40
                    while time.monotonic() < end and not ready(fleet, pty.text()):
                        pty.read(0.2)
                    # Two more vitals samples: sparklines with a few points.
                    deadline = time.monotonic() + 4.5
                    while time.monotonic() < deadline:
                        pty.read(0.1)
                    for cols, rows in sizes:
                        pty.resize(cols, rows)
                        settle(pty)
                        stem = f"{cfg['name']}__{tier}__{cols}x{rows}"
                        text = pty.text()
                        dump = pty_lib.dump(pty.screen, 'light' if tier == 'light' else 'dark')
                        alive = pty.proc.poll() is None
                        (out / f'{stem}.txt').write_text(text)
                        (out / f'{stem}.json').write_text(json.dumps(dump))
                        pty_lib.png(dump, out / f'{stem}.png')
                        fails = check(cfg, tier, (cols, rows), text, dump, alive, lab)
                        results.append(dict(config=cfg['name'], tier=tier, size=f'{cols}x{rows}',
                                            ok=not fails, fails=fails))
                finally:
                    pty.close()
        finally:
            fleet.cleanup()
    return results


# ---------------------------------------------------------------------------- checks

def strip_row(lines):
    """The vitals strip: the last row naming this device (the map's root names it first)."""
    rows = [y for y, l in enumerate(lines) if 'tranquility' in l]
    return rows[-1] if len(rows) >= 2 else None


def labels(lines, names, band):
    """Where each device's label starts in the map band: its whole name, else a prefix cut
    with an ellipsis. Returns {name: (row, col, end)}."""
    found = {}
    for name in sorted(names, key=len, reverse=True):
        for y in range(band):
            line = lines[y]
            at = line.find(name)
            if at < 0:
                for k in range(len(name) - 1, 3, -1):
                    m = re.search(re.escape(name[:k]) + '(…|~|\\.\\.)', line)
                    if m:
                        at = m.start()
                        end = m.end()
                        break
                else:
                    continue
            else:
                end = at + len(name)
            if any(y == r and at < e and s < end for r, s, e in found.values()):
                continue
            found[name] = (y, at, end)
            break
    return found


def lab_dump(lab, cfg, tier, size):
    if not lab or not cfg.get('lab'):
        return None
    p = pathlib.Path(lab) / f"hub__{size[0]}x{size[1]}__{LAB_VARIANT[tier]}__{cfg['lab']}.json"
    return json.loads(p.read_text()) if p.exists() else None


def cell_text(dump):
    return ['' .join(c[0] or ' ' for c in row) for row in dump['cells']]


def color_of(dump, word):
    for row in dump['cells']:
        line = ''.join(c[0] or ' ' for c in row)
        at = line.find(word)
        if at >= 0:
            return row[at][1]
    return None


def check(cfg, tier, size, text, dump, alive, lab):
    fails = []
    cols, rows = size
    lines = text.split('\n')
    if not alive or 'panicked' in text:
        return ['viewer died or panicked']
    names = ['tranquility'] + [d['name'] for d in cfg['devices']]
    strip = strip_row(lines)
    band = strip if strip is not None else max(rows - 3, 1)
    found = labels(lines, names, band)
    # Overlap: two labels on one row never share a cell (labels() only keeps disjoint ones,
    # so a name lost to an overlap shows up below as missing).
    folded = sum(int(n) for n in re.findall(r'\+(\d+)', '\n'.join(lines[:band])))
    missing = [n for n in names if n not in found]
    letters = len(re.findall(r'(?<![\w@])[a-z](?![\w])', '\n'.join(lines[:band])))
    if missing and len(missing) > folded and letters < len(missing):
        fails.append(f'devices neither shown nor counted: {missing[:4]}')
    bg = sum(1 for row in dump['cells'] for c in row if c[2] is not None or c[3] & 64)
    if bg:
        fails.append(f'{bg} cells with a background')
    uni = tier != 'ascii'
    # The strip's second row: cpu sparkline and %, mem, temperature, uptime.
    vitals = lines[strip + 1] if strip is not None and strip + 1 < rows else ''
    if 'cpu' in vitals:
        if not re.search(r'\d+%', vitals) or 'mem' not in vitals and cols >= 95:
            fails.append('vitals row lacks cpu % or mem')
        if uni and not SPARK & set(vitals):
            fails.append('no cpu sparkline in the strip')
        if cols >= 95 and not re.search(r'\d+% of \d+G', vitals):
            fails.append('no "N% of NG" memory figure')
        temp = re.search(r'\d+(°|C\b)', vitals)
        if cfg['temp'] is None and temp:
            fails.append('a temperature shown for a device without a sensor')
        if cfg['temp'] is not None and cols >= 120 and not temp:
            fails.append('no temperature though the device has a sensor')
        if re.search(r'(--|\?|n/a)\s*°', vitals):
            fails.append('a blank temperature placeholder')
        if cols >= 120 and not re.search(r'up \d+d \d+h', vitals):
            fails.append('no uptime')
    elif cols >= 95 and rows >= 40:
        fails.append('no vitals row in the strip')
    remote_up = [d for d in cfg['devices'] if d['state'] == 'up']
    mapped = '\n'.join(lines[:band])
    if remote_up and cols >= 120:
        if not re.search(r'\d+(\.\d+)?ms', mapped):
            fails.append('no link rtt on the map')
        if uni and not SPARK & set(mapped):
            fails.append('no sparkline on the map')
        if uni and len(BRAILLE.findall(mapped)) < 4:
            fails.append('no curves drawn')
    lab_d = lab_dump(lab, cfg, tier, size)
    if lab_d is not None and len(names) > 1:
        lab_lines = cell_text(lab_d)
        lstrip = strip_row(lab_lines)
        lab_found = labels(lab_lines, names, lstrip if lstrip is not None else max(rows - 3, 1))
        both = [n for n in names if n in found and n in lab_found]
        near = [n for n in both if abs(found[n][0] - lab_found[n][0]) <= 2
                and abs(found[n][1] - lab_found[n][1]) <= 4]
        if len(lab_found) >= 2 and len(near) < 0.8 * len(lab_found):
            far = [(n, found.get(n, '-')[:2], lab_found[n][:2]) for n in lab_found if n not in near]
            fails.append(f'labels away from the lab ({len(near)}/{len(lab_found)}): {far[:3]}')
        if uni and remote_up:
            lab_curves = len(BRAILLE.findall('\n'.join(lab_lines[:lstrip or rows])))
            curves = len(BRAILLE.findall(mapped))
            if lab_curves > 20 and curves < 0.5 * lab_curves:
                fails.append(f'curves: {curves} braille cells, the lab draws {lab_curves}')
    # Session badges: the right word, each state its own colour (the lab's where it has it).
    below = '\n'.join(lines[(strip or 0) + 1:])
    want = [s for s in STATES if s in {s for _, s in cfg['sessions']}]
    shown = [s for s in want if s in below]
    if want and rows >= 30 and cols >= 80 and 'needs you' in want and 'needs you' not in below:
        fails.append('the session that needs you has no badge')
    if cols >= 190 and rows >= 50 and len(want) >= 2 and len(shown) < 2:
        fails.append(f'session states shown: {shown} of {want}')
    hues = {}
    for s in shown:
        hues[s] = color_of(dict(cells=dump['cells'][(strip or 0) + 1:]), s)
    if tier != 'ascii' and len(set(hues.values())) < len(hues):
        fails.append(f'session states share a colour: {hues}')
    if lab_d is not None and tier != 'ascii':
        for s, hue in hues.items():
            lab_hue = color_of(lab_d, s)
            if lab_hue is not None and lab_hue != hue:
                fails.append(f'"{s}" is {hue}, the lab draws {lab_hue}')
    return fails


# ---------------------------------------------------------------------------- lab + sheets

def lab_capture(sizes, tiers, fixtures, out):
    """The lab's render of the nebula fixtures (one cargo test run), as cell dumps."""
    out.mkdir(parents=True, exist_ok=True)
    env = dict(os.environ, CX_LAB_OUT=str(out), CX_LAB_VIEWS='hub', CX_LAB_FOCUS='graph',
               CX_LAB_FIXTURES=','.join(fixtures), CX_LAB_VARIANTS=','.join(LAB_VARIANT[t] for t in tiers),
               CX_LAB_SIZES=','.join(f'{w}x{h}' for w, h in sizes))
    r = subprocess.run(['nice', '-n', '15', 'cargo', 'test', '--locked', '--quiet', '--bin', 'cx',
                        'ui::lab::capture::lab_capture', '--', '--ignored', '--exact'],
                       cwd=HERE.parent, env=env, capture_output=True, text=True)
    if r.returncode != 0:
        sys.exit('hub_matrix: lab capture failed:\n' + r.stderr[-2000:])
    return out


def sheets(out, results, sizes, tiers, names):
    import lab_png
    from PIL import Image
    by = {(r['config'], r['tier'], r['size']): r for r in results}
    paths = []
    for tier in tiers:
        def cell(r, c):
            key = (names[r], tier, f'{sizes[c][0]}x{sizes[c][1]}')
            p = out / f'{key[0]}__{tier}__{key[2]}.png'
            if not p.exists():
                return None, ''
            res = by.get(key)
            cap = key[2] + ('' if res and res['ok'] else '  ! ' + '; '.join(res['fails'])[:60] if res else '')
            return Image.open(p), cap
        name = 'contact.png' if tier == 'dark' else f'contact-{tier}.png'
        paths.append(lab_png.sheet(f'cx hub matrix · {tier}', f'{out.name} · real viewer, real helpers',
                                   names, [f'{w}x{h}' for w, h in sizes], cell, out / name, box=(420, 260)))
    return paths


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('binary')
    ap.add_argument('--configs', default=','.join(configs()))
    ap.add_argument('--sizes', default=','.join(f'{w}x{h}' for w, h in SIZES))
    ap.add_argument('--tiers', default=','.join(TIERS))
    ap.add_argument('--jobs', type=int, default=2)
    ap.add_argument('--run', default=time.strftime('%Y%m%d-%H%M%S'))
    ap.add_argument('--lab', help='lab dumps already captured (default: capture them now)')
    ap.add_argument('--no-lab', action='store_true')
    a = ap.parse_args()
    binary = str(pathlib.Path(a.binary).resolve())
    sizes = [tuple(int(v) for v in s.split('x')) for s in a.sizes.split(',')]
    tiers = a.tiers.split(',')
    all_cfg = configs()
    chosen = [all_cfg[n] for n in a.configs.split(',')]
    out = pathlib.Path.home() / '.cache/cx-hub-matrix' / a.run
    if os.environ.get('CX_HUB_MATRIX_NS') != '1':
        out.mkdir(parents=True, exist_ok=True)
        lab = None
        if not a.no_lab:
            lab = a.lab or str(lab_capture(sizes, tiers, sorted({c['lab'] for c in chosen if c['lab']}),
                                           out / 'lab'))
        sys.argv = [sys.argv[0], *[x for x in sys.argv[1:] if not x.startswith('--lab')],
                    '--run', a.run] + (['--lab', lab] if lab else ['--no-lab'])
        enter_namespace()
    lab = None if a.no_lab else a.lab
    results = []
    with concurrent.futures.ThreadPoolExecutor(max_workers=max(1, a.jobs)) as pool:
        for res in pool.map(lambda c: capture_config(binary, c, tiers, sizes, out, lab), chosen):
            results.extend(res)
    (out / 'results.json').write_text(json.dumps(results, indent=1))
    paths = sheets(out, results, sizes, tiers, [c['name'] for c in chosen])
    bad = [r for r in results if not r['ok']]
    for c in chosen:
        mine = [r for r in results if r['config'] == c['name']]
        print(f"{'PASS' if all(r['ok'] for r in mine) else 'FAIL'} {c['name']}: "
              f"{sum(r['ok'] for r in mine)}/{len(mine)}")
    for r in bad[:40]:
        print(f"  {r['config']} {r['tier']} {r['size']}: {'; '.join(r['fails'])}")
    print('contact sheets:', *[str(p) for p in paths], sep='\n  ')
    sys.exit(1 if bad else 0)


if __name__ == '__main__':
    main()
