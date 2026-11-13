#!/usr/bin/env python3
"""The yazi Files view in a real PTY: selection, visual mode, the `c` which-key panel
with `cc` copying the path through OSC 52, yank/paste, filter, tabs, image escape
sequences per graphics protocol and, with --ssh, the cross-device flow
(y, ' + letter, p) over a disposable loopback sshd.

usage: yazi_files_pty.py BINARY [--ssh]

Private HOME/XDG state per viewer (perf_harness.Env), a fixture tree under it, no user
config, tmux or SSH of the caller's. Prints one JSON line; exits non-zero on failure.
"""
import base64, json, os, pathlib, re, shutil, subprocess, sys, tempfile, time, uuid

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import perf_harness as ph  # noqa: E402
import pty_lib  # noqa: E402

BINARY = str(pathlib.Path(sys.argv[1]).resolve())
SSH = '--ssh' in sys.argv
results = {}


def tree(root):
    p = root / 'project'
    for d in ('assets', 'docs', 'src'):
        (p / d).mkdir(parents=True)
    (p / 'src' / 'main.rs').write_text('fn main() {}\n')
    (p / 'docs' / 'guide.md').write_text('# Guide\n')
    (p / 'README.md').write_text('# Readme\n')
    (p / 'notes.txt').write_text('notes\n')
    (p / 'zeta.txt').write_text('zeta\n')
    from PIL import Image
    Image.radial_gradient('L').resize((640, 480)).convert('RGB').save(p / 'assets' / 'photo.png')
    return p


class Raw:
    """A viewer whose raw output is kept (escape sequences are checked, not just cells)."""

    def __init__(self, env, directory, select, extra_env=None, cols=120, rows=40, device_index=0, ids=None):
        ph.COLS, ph.ROWS = cols, rows
        env.env.update(extra_env or {})
        browser = ph.browser_snapshot(directory, select)
        browser['device'] = device_index
        if ids:
            self.v = ph.Viewer.__new__(ph.Viewer)
            self.v.open(env, dict(device_ids=ids, device=device_index + 1, focus='Workspace', view='Files',
                                  browser=browser))
        else:
            self.v = ph.Viewer(env, 'Files', browser)
        self.raw = bytearray()
        read = self.v.pty.read

        def keep(timeout):
            data = read(timeout)
            if data:
                self.raw.extend(data)
            return data
        self.v.pty.read = keep
        self.wait(lambda: self.status() is not None, 'files view')
        self.settle()

    def text(self):
        return self.v.text()

    def lines(self):
        return self.v.screen.display

    def status(self):
        last = self.lines()[-1]
        return last if re.match(r'^\W?\s*(NOR|SEL|UNS) ', last) else None

    def wait(self, predicate, what, timeout=10):
        self.v.wait(predicate, timeout=timeout, what=what)

    def settle(self, quiet=.25):
        self.v.quiesce(quiet, 3)

    def key(self, data, quiet=.2):
        os.write(self.v.master, data)
        self.settle(quiet)

    def close(self):
        self.v.close()


def check(name, cond, detail=''):
    if not cond:
        raise AssertionError(f'{name}: {detail}')
    results[name] = True


def local_flows():
    env = ph.Env(BINARY)
    try:
        project = tree(env.root)
        r = Raw(env, project, project / 'assets')
        try:
            # Selection: three spaces mark three rows and the header counts them.
            r.key(b'   ')
            check('select_count', r.lines()[0].rstrip().endswith('3'), r.lines()[0])
            r.key(b'\x1b')
            check('escape_clears', not r.lines()[0].rstrip().endswith('3'), r.lines()[0])
            # Visual mode: v j j selects a live range; the mode pill says SEL.
            r.key(b'gg')
            r.key(b'vjj')
            check('visual_sel', ' SEL ' in r.status(), r.status())
            # yazi counts committed selections only: no count for the live range.
            check('visual_count', not r.lines()[0].rstrip().endswith('3'), r.lines()[0])
            r.key(b'\x1b')
            check('visual_commit', r.lines()[0].rstrip().endswith('3'), r.lines()[0])
            r.key(b'\x1b')
            check('visual_exit', ' NOR ' in r.status(), r.status())
            # The c which-key panel, then cc copies the hovered path through OSC 52.
            r.key(b'gg')
            r.key(b'c')
            text = r.text()
            check('which_c', 'Copy file path' in text and 'Copy directory path' in text, text[-600:])
            r.raw.clear()
            r.key(b'c')
            want = base64.b64encode(str(project / 'assets').encode())
            check('cc_osc52', re.search(rb'\x1b\]52;c;' + re.escape(want) + rb'\x07', bytes(r.raw)) is not None,
                  bytes(r.raw)[:200])
            # Filter: f md narrows the list and the header says so.
            r.key(b'f')
            check('filter_box', 'Filter:' in r.text(), r.text()[:400])
            r.key(b'md')
            r.key(b'\r')
            check('filter_header', '(filter: md)' in r.lines()[0], r.lines()[0])
            check('filter_rows', 'README.md' in r.text() and 'notes.txt' not in r.text())
            r.key(b'\x1b')
            check('filter_cleared', 'notes.txt' in r.text())
            # Tabs: t t opens a second tab (a tab bar appears), 1 and 2 switch, l moves one.
            r.key(b'tt')
            check('tab_bar', re.search(r' 1 \S+ +2 ', r.lines()[1]) is not None, r.lines()[1])
            r.key(b'gg')
            r.key(b'l')
            check('tab2_moved', r.lines()[0].rstrip().endswith('assets') or '/assets' in r.lines()[0], r.lines()[0])
            r.key(b'1')
            check('tab1_kept', '/assets' not in r.lines()[0], r.lines()[0])
            r.key(b'2')
            check('tab2_back', '/assets' in r.lines()[0], r.lines()[0])
            # Yank and paste: y on a file, into another folder, p copies it there.
            r.key(b'h')
            r.key(b'G')  # zeta.txt (last)
            r.key(b'y')
            check('yank_count', r.lines()[0].rstrip().endswith('1'), r.lines()[0])
            r.key(b'gg')  # assets
            r.key(b'l')
            r.key(b'p', .5)
            deadline = time.monotonic() + 15
            while not (project / 'assets' / 'zeta.txt').exists() and time.monotonic() < deadline:
                r.settle(.2)
            check('paste_copied', (project / 'assets' / 'zeta.txt').read_text() == 'zeta\n')
            r.wait(lambda: 'zeta.txt' in r.text(), 'pasted row listed', 10)
            results['paste_listed'] = True
        finally:
            r.close()
    finally:
        env.cleanup()


def image_flows():
    """Escape sequences of each protocol for a hovered image (preview column)."""
    cases = [
        ('kitty', dict(TERM='xterm-kitty', KITTY_WINDOW_ID='1'), rb'\x1b_G[^\x1b]*U=1'),
        ('iterm2', dict(TERM='xterm-256color', TERM_PROGRAM='WezTerm'), rb'\x1b\]1337;File='),
    ]
    tmux = shutil.which('tmux')
    pty_lib.CELL_PIXELS = (10, 20)
    for name, extra, pattern in cases + ([('kitty-tmux', None, rb'\x1bPtmux;\x1b\x1b_G')] if tmux else []):
        env = ph.Env(BINARY)
        server = None
        try:
            project = tree(env.root)
            if extra is None:
                # A private tmux server with passthrough, as cx's managed server sets it.
                sock = env.root / 'tmux.sock'
                subprocess.run(['tmux', '-S', str(sock), '-f', '/dev/null', 'new-session', '-d', '-s', 'x'],
                               env=env.env, check=True)
                subprocess.run(['tmux', '-S', str(sock), 'set', '-g', 'allow-passthrough', 'on'], env=env.env,
                               check=True)
                server = sock
                extra = dict(TERM='tmux-256color', KITTY_WINDOW_ID='1', TMUX=f'{sock},1,0')
            r = Raw(env, project / 'assets', project / 'assets' / 'photo.png', extra)
            try:
                deadline = time.monotonic() + 8
                while time.monotonic() < deadline and not re.search(pattern, bytes(r.raw)):
                    r.settle(.2)
                check(f'image_{name}', re.search(pattern, bytes(r.raw)) is not None, bytes(r.raw)[-300:])
            finally:
                r.close()
        finally:
            if server:
                subprocess.run(['tmux', '-S', str(server), 'kill-server'], env=env.env, capture_output=True)
            env.cleanup()


def ssh_flow():
    """y on device A, ' + B's letter, p: a transfer into B's folder (4 keys)."""
    import files_bench as fb
    tag = uuid.uuid4().hex
    ssh = fb.LoopbackSSH(BINARY, tag)
    env = fb.BenchEnv(BINARY, tag, ssh)
    # The transfer worker normally starts as a systemd user unit, which would not see
    # this fixture's ssh wrapper (PATH): a stand-in runs it detached with this
    # environment instead (tagged, so cleanup finds it).
    shim = env.root / 'shim'
    shim.mkdir()
    (shim / 'systemd-run').write_text('#!/bin/sh\nwhile [ "$1" != "--" ]; do shift; done; shift\n'
                                      'setsid "$@" </dev/null >/dev/null 2>&1 &\nexit 0\n')
    (shim / 'systemd-run').chmod(0o700)
    env.env['PATH'] = f"{shim}{os.pathsep}{env.env['PATH']}"
    try:
        project = tree(env.root)
        dest = env.root / 'dest'
        dest.mkdir()
        r = Raw(env, project, project / 'notes.txt', ids=env.device_ids)
        try:
            r.settle(1.0)
            r.key(b'y')
            r.key(b"'")
            r.wait(lambda: '(here)' in r.text(), 'device panel')
            m = re.search(r'(?:^|\s)(\w)\s+\W?\s*fixture', r.text())
            check('panel_letter', m is not None, r.text()[-500:])
            r.key(m.group(1).encode(), .5)
            r.wait(lambda: 'fixture' in r.lines()[0], 'remote files', 20)
            # Go to the destination folder on the remote: g <Space>, type, Enter.
            r.key(b'g ')
            r.key(b'\x15' + str(dest).encode() + b'\r', .5)
            r.wait(lambda: str(dest.name) in r.lines()[0], 'remote dest folder', 20)
            r.key(b'p', .5)
            deadline = time.monotonic() + 30
            while not (dest / 'notes.txt').exists() and time.monotonic() < deadline:
                r.settle(.3)
            check('cross_device_paste', (dest / 'notes.txt').exists() and
                  (dest / 'notes.txt').read_text() == 'notes\n', r.text())
            r.key(b"''", .5)
            r.wait(lambda: 'fixture' not in r.lines()[0], "'' back", 10)
            results['back_with_quote_quote'] = True
        finally:
            r.close()
    finally:
        env.cleanup()
        ssh.cleanup()
        fb.kill_tagged(tag)


def main():
    local_flows()
    image_flows()
    if SSH:
        ssh_flow()
    print(json.dumps(dict(result='PASS', **results)))


try:
    main()
except AssertionError as e:
    print(str(e), file=sys.stderr)
    print(json.dumps(dict(result='FAIL', error=str(e)[:800], **results)))
    sys.exit(1)
