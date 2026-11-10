#!/usr/bin/env python3
"""Repeatable cx TUI performance harness over a real PTY (pyte screen model).

usage: perf_harness.py BINARY [--out FILE] [--repeat N] [--iterations N] [--only NAME,...]
Run with: uv run --with pyte python tests/perf_harness.py target/release/cx --out perf.json

Every run uses a private temp HOME/XDG_STATE_HOME/TMUX_TMPDIR, only local fixture files and
a fixture shell session; no SSH, providers, user config or networking. The viewer is opened
through the private `restart` handoff so each scenario starts in a known view.

Scenarios (all times in milliseconds):
  preview_scroll   5,000-line Markdown and source previews: j / PageDown / wheel keypress ->
                   screen settled (expected preview change visible and PTY quiet), plus a
                   30 Hz key-repeat burst.
  directory_list   Enter on a 20,000-entry local directory: first rows visible, final count.
  sessions_attach  `cx new` shell session; Enter -> focused pane, pane echo, detach -> list,
                   expand (native attach) -> prompt visible, detach -> cx UI back.
  mouse_motion     pure pointer motion (no buttons) over the Sessions and Files lists: bytes
                   and write bursts per motion event, against an idle baseline window.
  idle             5 s idle in Files and Sessions: CPU seconds and bytes/s written.

"Settled" means the expected change is on the pyte screen and no PTY output arrived for
QUIET seconds; latency is measured to the last output chunk before that quiet window.
"bursts" approximates frames: output chunks separated by more than BURST_GAP seconds.
Cleanup kills only what this harness spawned and verifies no process carrying the private
HOME survives; `cx orphans --json` (read-only) must report no stray among our pids.
"""
import argparse, fcntl, json, os, pathlib, platform, pty, re, select, shutil, statistics
import struct, subprocess, sys, tempfile, termios, time
import pyte

# The detach key is expected to change to Ctrl+\ later; keep it in one place.
DETACH_KEY = b'\x1d'          # Ctrl+]
DETACH_LABEL = 'Ctrl+]'
EXPAND_KEY = b'\x0f'          # Ctrl+O from a focused session pane
QUIT_KEY = b'\x03'
COLS, ROWS = 120, 40
QUIET = 0.06
BURST_GAP = 0.003
TIMEOUT = 10.0
WHEEL_DOWN = 65
END_MARKER = 'perf fixture final line'
MOTION = 35                   # SGR 1006 motion with no button held

HZ = os.sysconf('SC_CLK_TCK')


class FixtureScreen(pyte.Screen):
    def report_device_status(self, *args, **kwargs):
        pass


def percentile(values, q):
    if not values:
        return None
    ordered = sorted(values)
    k = (len(ordered) - 1) * q
    lo, hi = int(k), min(int(k) + 1, len(ordered) - 1)
    return round(ordered[lo] + (ordered[hi] - ordered[lo]) * (k - lo), 2)


def summary(samples):
    ms = [s for s in samples if s is not None]
    return dict(n=len(ms), failed=len(samples) - len(ms), p50=percentile(ms, .5),
                p95=percentile(ms, .95), max=round(max(ms), 2) if ms else None)


class Timings:
    """Settled and visible latencies plus output volume for one repeated action."""

    def __init__(self):
        self.settled, self.visible, self.bytes, self.bursts = [], [], [], []

    def add(self, measured):
        ms, stats = measured
        self.settled.append(ms)
        self.visible.append(stats.get('visible_ms'))
        self.bytes.append(stats['bytes'])
        self.bursts.append(stats['bursts'])
        return ms

    def report(self):
        return dict(settled_ms=summary(self.settled), visible_ms=summary(self.visible),
                    bytes_median=statistics.median(self.bytes) if self.bytes else None,
                    bursts_median=statistics.median(self.bursts) if self.bursts else None)


def cpu_ticks(pid):
    try:
        fields = pathlib.Path(f'/proc/{pid}/stat').read_text().rsplit(')', 1)[1].split()
        return int(fields[11]) + int(fields[12])
    except (OSError, IndexError, ValueError):
        return 0


class Env:
    """A private cx home. Tracks every pid observed carrying its HOME for cleanup checks."""

    def __init__(self, binary):
        self.binary = binary
        self.tmp = tempfile.TemporaryDirectory(prefix='cx-perf-')
        self.root = pathlib.Path(self.tmp.name)
        (self.root / '.local/bin').mkdir(parents=True)
        (self.root / '.local/bin/cx').symlink_to(binary)
        self.state = self.root / 'state/cx'
        self.state.mkdir(parents=True)
        self.env = dict(os.environ, HOME=str(self.root), XDG_STATE_HOME=str(self.root / 'state'),
                        TMUX_TMPDIR=str(self.root), SHELL='/bin/bash', TERM='xterm-256color',
                        HTTPS_PROXY='http://127.0.0.1:9', HISTFILE='/dev/null')
        for key in ('TMUX', 'TMUX_PANE', 'NO_COLOR', 'CX_ASCII'):
            self.env.pop(key, None)
        self.marker = f'HOME={self.root}\0'.encode()
        self.path_marker = str(self.root).encode()
        self.seen = set()

    def owned_pids(self):
        """Pids whose environment carries this private HOME or whose command line names the
        private root (tmux -S socket). Never printed, only matched."""
        found = set()
        for entry in pathlib.Path('/proc').iterdir():
            if not entry.name.isdigit():
                continue
            try:
                if (self.marker in (entry / 'environ').read_bytes()
                        or self.path_marker in (entry / 'cmdline').read_bytes()):
                    found.add(int(entry.name))
            except OSError:
                pass
        found.discard(os.getpid())
        self.seen |= found
        return found

    def cx(self, *args):
        self.owned_pids()
        return subprocess.run([self.binary, *args], env=self.env, capture_output=True, timeout=30)

    def cleanup(self):
        subprocess.run(['tmux', '-S', str(self.state / 'managed.sock'), 'kill-server'],
                       env=self.env, capture_output=True)
        deadline = time.monotonic() + 3
        while self.owned_pids() and time.monotonic() < deadline:
            time.sleep(.1)
        survivors = self.owned_pids()
        for pid in survivors:
            try:
                os.kill(pid, 9)
            except OSError:
                pass
        time.sleep(.2)
        orphans = self.cx('orphans', '--json')
        report = json.loads(orphans.stdout or b'{}')
        ours = [r for r in report.get('processes', []) if r.get('pid') in self.seen]
        result = dict(pids_observed=len(self.seen), survivors_killed=len(survivors),
                      remaining=len(self.owned_pids()),
                      orphans_ours=len(ours), orphans_ours_stray=sum(bool(r.get('stray')) for r in ours),
                      stale_handoffs=len(report.get('stale_handoffs', [])))
        self.tmp.cleanup()
        return result


class Viewer:
    """One cx viewer on a private PTY, opened into a given view via the restart handoff."""

    def __init__(self, env, view, browser=None, focus='Workspace'):
        self.env = env
        name = f'viewer-restart-{os.getpid()}-{int(time.time() * 1000) % 10**9}.json'
        snapshot = dict(schema=1, expires_at=int(time.time()) + 300, device_ids=['local'], device=1,
                        focus=focus, view=view, selected=0, side_selected=0, search='',
                        browser=browser, other_browser=None, destination_active=False, conflict=2,
                        launch_provider=None, clipboard=None, submitted={})
        path = env.state / name
        path.write_text(json.dumps(snapshot))
        path.chmod(0o600)
        self.master, self.slave = pty.openpty()
        self.termios_before = termios.tcgetattr(self.slave)
        fcntl.ioctl(self.slave, termios.TIOCSWINSZ, struct.pack('HHHH', ROWS, COLS, 0, 0))

        def tty():
            os.setsid()
            fcntl.ioctl(self.slave, termios.TIOCSCTTY, 0)
        self.proc = subprocess.Popen([env.binary, 'restart', name], env=env.env, cwd=str(env.root),
                                     stdin=self.slave, stdout=self.slave, stderr=self.slave,
                                     preexec_fn=tty)
        self.screen = FixtureScreen(COLS, ROWS)
        self.stream = pyte.Stream(self.screen)
        self.chunks = []  # (monotonic time, byte count)
        self.total = 0
        self.mouse_modes = set()  # DEC private mouse modes the viewer last enabled

    # ---- output plumbing -------------------------------------------------
    def _read(self, timeout):
        if select.select([self.master], [], [], max(timeout, 0))[0]:
            try:
                data = os.read(self.master, 1 << 16)
            except OSError:
                return False
            now = time.monotonic()
            self.chunks.append((now, len(data)))
            self.total += len(data)
            for mode, action in re.findall(rb'\x1b\[\?(100[0-6]|1015)([hl])', data):
                (self.mouse_modes.add if action == b'h' else self.mouse_modes.discard)(mode.decode())
            self.stream.feed(data.decode('utf-8', 'replace'))
            return True
        return False

    def pump(self, seconds):
        end = time.monotonic() + seconds
        while time.monotonic() < end:
            self._read(end - time.monotonic())

    def text(self):
        return '\n'.join(self.screen.display)

    def wait(self, predicate, timeout=TIMEOUT, what='condition'):
        end = time.monotonic() + timeout
        while not predicate():
            if time.monotonic() > end:
                if os.environ.get('CX_PERF_DEBUG'):
                    print(self.text(), file=sys.stderr)
                raise AssertionError(f'timed out waiting for {what}')
            if self.proc.poll() is not None:
                raise AssertionError(f'viewer exited while waiting for {what}')
            self._read(.05)

    def quiesce(self, quiet=0.25, timeout=2.0):
        end = time.monotonic() + timeout
        last = time.monotonic()
        while time.monotonic() - last < quiet and time.monotonic() < end:
            if self._read(quiet - (time.monotonic() - last)):
                last = time.monotonic()

    def measure(self, data, predicate, quiet=QUIET, timeout=5.0):
        """Write data; return (settled ms or None on timeout, stats). stats['visible_ms'] is
        the arrival of the output chunk that first made the predicate true."""
        start_index = len(self.chunks)
        t0 = time.monotonic()
        os.write(self.master, data)
        visible = t0 if predicate() else None
        while True:
            now = time.monotonic()
            last = self.chunks[-1][0] if len(self.chunks) > start_index else t0
            if visible is not None and now - last >= quiet:
                stats = self._stats(start_index, t0)
                stats['visible_ms'] = round((visible - t0) * 1000, 2)
                return round((last - t0) * 1000, 2), stats
            if now - t0 > timeout:
                stats = self._stats(start_index, t0)
                stats['visible_ms'] = None
                return None, stats
            wait = quiet - (now - last) if visible is not None else min(.05, timeout - (now - t0))
            if self._read(max(wait, .001)) and visible is None and predicate():
                visible = self.chunks[-1][0]

    def _stats(self, start_index, t0):
        chunks = self.chunks[start_index:]
        bursts, previous = 0, None
        for at, _ in chunks:
            if previous is None or at - previous > BURST_GAP:
                bursts += 1
            previous = at
        return dict(bytes=sum(n for _, n in chunks), bursts=bursts,
                    first_byte_ms=round((chunks[0][0] - t0) * 1000, 2) if chunks else None)

    def window(self, seconds):
        start_index = len(self.chunks)
        t0 = time.monotonic()
        self.pump(seconds)
        return self._stats(start_index, t0)

    # ---- process accounting -----------------------------------------------
    def descendants(self):
        children = {}
        for entry in pathlib.Path('/proc').iterdir():
            if entry.name.isdigit():
                try:
                    ppid = int((entry / 'stat').read_text().rsplit(')', 1)[1].split()[1])
                    children.setdefault(ppid, []).append(int(entry.name))
                except (OSError, IndexError, ValueError):
                    pass
        out, stack = [], [self.proc.pid]
        while stack:
            pid = stack.pop()
            out.append(pid)
            stack.extend(children.get(pid, []))
        return out

    def close(self):
        result = dict(clean_exit=False, termios_restored=False)
        self.env.owned_pids()
        try:
            if self.proc.poll() is None:
                os.write(self.master, QUIT_KEY)
                try:
                    self.proc.wait(timeout=5)
                    result['clean_exit'] = self.proc.returncode == 0
                    result['termios_restored'] = termios.tcgetattr(self.slave) == self.termios_before
                except subprocess.TimeoutExpired:
                    pass
        finally:
            if self.proc.poll() is None:
                self.proc.kill()
                self.proc.wait()
            os.close(self.master)
            os.close(self.slave)
        return result


def browser_snapshot(directory, select_path):
    return dict(device=0, path=str(directory), display_path=str(directory), parent=str(directory.parent),
                entries=[], selected=0, search='', preview_scroll=0, restore_selection=str(select_path))


# ---------------------------------------------------------------------------
def markdown_fixture(lines):
    out, i = [], 0
    while len(out) < lines:
        if i % 25 == 0:
            out += [f'## Section {i}', '']
        elif i % 7 == 0:
            out.append(f'Paragraph {i} with *emphasis*, `code` and a [link](#section-{i}).')
        else:
            out.append(f'- list item {i} with some ordinary words to wrap')
        i += 1
    return '\n'.join(out[:lines - 1] + [END_MARKER]) + '\n'


def source_fixture(lines):
    out = []
    for i in range(lines):
        if i % 6 == 0:
            out.append(f'/// Doc comment for item {i}.')
        elif i % 6 == 1:
            out.append(f'pub fn function_{i}(value: u32, name: &str) -> Result<u32, String> {{')
        elif i % 6 == 2:
            out.append(f'    let text = format!("{{name}}-{i}"); // trailing comment')
        elif i % 6 == 3:
            out.append(f'    if value > {i} {{ return Err(text); }}')
        elif i % 6 == 4:
            out.append(f'    Ok(value.wrapping_mul({i}))')
        else:
            out.append('}')
    return '\n'.join(out[:lines - 1] + ['// ' + END_MARKER]) + '\n'


def preview_region(viewer):
    """Rows inside the preview box (between its titled top border and its bottom border)."""
    display = viewer.screen.display
    # Focused panels draw a heavy border with a title chip (spec 5); older builds used '╭ ▸'.
    marks = ('┏ ', '╭ ▸')
    top = next((y for y, line in enumerate(display) if any(m in line for m in marks) and 'bytes' in line), None)
    if top is None:
        return None
    column = next(display[top].index(m) for m in marks if m in display[top])
    bottom = next((y for y in range(top + 1, len(display))
                   if len(display[y]) > column and display[y][column] in '╰┗'), len(display))
    return top, bottom, column


def scenario_preview(binary, iterations):
    results = {}
    for kind, name, text, probe in (
            ('markdown', 'big.md', markdown_fixture(5000), 'Section 0'),
            ('source', 'big.rs', source_fixture(5000), 'Doc comment for item 0')):
        env = Env(binary)
        try:
            folder = env.root / 'files'
            folder.mkdir()
            target = folder / name
            target.write_text(text)
            assert len(text.splitlines()) == 5000
            viewer = Viewer(env, 'Files', browser_snapshot(folder, target))
            try:
                viewer.wait(lambda: name in viewer.text(), what='file row')
                viewer.quiesce()
                open_ms, open_stats = viewer.measure(b'\r', lambda: probe in viewer.text(), timeout=TIMEOUT)
                viewer.quiesce()
                region = preview_region(viewer)
                assert region, 'preview box not found'
                top, bottom, column = region

                def content():
                    return tuple(line[column:] for line in viewer.screen.display[top + 1:bottom])
                mid_row = (top + bottom) // 2 + 1
                wheel = f'\x1b[<{WHEEL_DOWN};{column + 40};{mid_row}M'.encode()
                per_key = {}
                for label, data in (('j', b'j'), ('pagedown', b'\x1b[6~'), ('wheel', wheel)):
                    timings = Timings()
                    for _ in range(iterations):
                        before = content()
                        timings.add(viewer.measure(data, lambda: content() != before))
                    per_key[label] = timings.report()
                # Key repeat: 30 j presses at ~30 Hz, then time until the screen settles.
                before = content()
                t_start = time.monotonic()
                start_index = len(viewer.chunks)
                for _ in range(30):
                    os.write(viewer.master, b'j')
                    viewer.pump(1 / 30)
                last_key = time.monotonic()
                viewer.quiesce(QUIET)
                last_chunk = viewer.chunks[-1][0] if len(viewer.chunks) > start_index else last_key
                stats = viewer._stats(start_index, t_start)
                per_key['repeat_30hz'] = dict(keys=30, settle_after_last_key_ms=round(max(0, last_chunk - last_key) * 1000, 2),
                                              bytes=stats['bytes'], bursts=stats['bursts'], changed=content() != before)
                # End: the whole file must be reachable in the preview (not truncated).
                # Today the preview is a 32 KiB prefix, so record how far End actually reaches.
                before = content()
                end_ms, _ = viewer.measure(b'\x1b[F', lambda: content() != before or END_MARKER in viewer.text())
                line_of = {}
                for index, line in enumerate(text.splitlines()):
                    found = re.search(r'\d+', line)
                    if found:
                        line_of[found.group()] = index + 1
                visible = [line_of.get(m.group()) for row in content() for m in [re.search(r'\d+', row)] if m]
                visible = [v for v in visible if v]
                per_key['end'] = dict(latency_ms=end_ms, last_line_visible=END_MARKER in viewer.text(),
                                      last_line_reachable=max(visible) if visible else None)
                results[kind] = dict(lines=5000, bytes=target.stat().st_size,
                                     open_ms=open_ms, open_bytes=open_stats['bytes'], keys=per_key)
            finally:
                results.setdefault(kind, {})['viewer'] = viewer.close()
        finally:
            results.setdefault(kind, {})['cleanup'] = env.cleanup()
    return results


def scenario_directory(binary, entries=20000):
    env = Env(binary)
    result = dict(entries=entries)
    try:
        folder = env.root / 'files'
        many = folder / 'many'
        many.mkdir(parents=True)
        for i in range(entries):
            (many / f'entry-{i:05}.txt').touch()
        viewer = Viewer(env, 'Files', browser_snapshot(folder, many))
        try:
            viewer.wait(lambda: '▸ many' in viewer.text() or ' many' in viewer.text(), what='directory row')
            viewer.quiesce()
            row = re.compile(r'entry-\d{5}\.txt')
            count = re.compile(r'(?:▸ )?(\d+) items')

            def shown():
                match = count.search(viewer.text())
                return int(match.group(1)) if match else 0
            t0 = time.monotonic()
            os.write(viewer.master, b'\r')
            viewer.wait(lambda: row.search(viewer.text()) is not None, what='first rows')
            first = time.monotonic()
            first_count = shown()
            # Full: header count stable, loading indicator gone, PTY quiet.
            last_change, previous = time.monotonic(), shown()
            deadline = time.monotonic() + 30
            while time.monotonic() < deadline:
                viewer._read(.02)
                current = shown()
                if current != previous:
                    previous, last_change = current, time.monotonic()
                if 'loading' not in viewer.text() and time.monotonic() - last_change > 1.0:
                    break
            result.update(first_rows_ms=round((first - t0) * 1000, 2), first_count=first_count,
                          final_count_ms=round((last_change - t0) * 1000, 2), final_count=previous,
                          all_entries_listed=previous == entries,
                          cap_notice='remaining entries require' in viewer.text(),
                          bytes=viewer.total)
            # Selection motion inside the large list.
            timings = Timings()
            for _ in range(20):
                before = viewer.text()
                timings.add(viewer.measure(b'j', lambda: viewer.text() != before))
            result['j'] = timings.report()
        finally:
            result['viewer'] = viewer.close()
    finally:
        result['cleanup'] = env.cleanup()
    return result


def scenario_sessions(binary, cycles):
    env = Env(binary)
    result = dict(detach_key=DETACH_LABEL)
    try:
        folder = env.root / 'work'
        folder.mkdir()
        t0 = time.monotonic()
        created = env.cx('new', '--directory', str(folder), '--name', 'perf-shell', '--key', 'perf-shell')
        result['cx_new_ms'] = round((time.monotonic() - t0) * 1000, 2)
        assert created.returncode == 0, 'cx new failed'
        viewer = Viewer(env, 'Work')
        try:
            viewer.wait(lambda: 'perf-shell' in viewer.text() and 'Enter Open' in viewer.text(), what='session row')
            viewer.wait(lambda: '$' in viewer.text(), what='session preview prompt')
            viewer.quiesce(.1)

            def focused():
                return 'back to sessions' in viewer.text()

            def listing():
                return 'Enter Open' in viewer.text() and not focused()

            def native():
                return 'Return:' in viewer.screen.display[-1] and '╭ Devices' not in viewer.text() and '$' in viewer.text()

            def ui_back():
                return '╭ Devices' in viewer.text()
            enter, echo, back, expand, ret = Timings(), Timings(), Timings(), Timings(), Timings()
            for _ in range(cycles):
                enter.add(viewer.measure(b'\r', lambda: focused() and '$' in viewer.text(), timeout=TIMEOUT))
                viewer.quiesce(.1)
                for _ in range(3):
                    before = viewer.text()
                    echo.add(viewer.measure(b'x', lambda: viewer.text() != before))
                    before = viewer.text()
                    viewer.measure(b'\x7f', lambda: viewer.text() != before)
                expand.add(viewer.measure(EXPAND_KEY, native, timeout=TIMEOUT))
                viewer.quiesce(.1)
                ret.add(viewer.measure(DETACH_KEY, ui_back, timeout=TIMEOUT))
                viewer.quiesce(.1)
                if not focused():
                    # Returning from expand landed on the list; refocus the pane first.
                    viewer.measure(b'\r', focused, timeout=TIMEOUT)
                    viewer.quiesce(.1)
                back.add(viewer.measure(DETACH_KEY, listing, timeout=TIMEOUT))
                viewer.quiesce(.1)
                env.owned_pids()
            result.update(enter_to_focused_pane=enter.report(), pane_echo=echo.report(),
                          detach_pane_to_list=back.report(), expand_to_native_prompt=expand.report(),
                          detach_native_to_ui=ret.report(), cycles=cycles)
            sessions = json.loads(env.cx('sessions').stdout or b'[]')
            result['sessions_after'] = len(sessions)
        finally:
            result['viewer'] = viewer.close()
    finally:
        result['cleanup'] = env.cleanup()
    return result


def motion_probe(viewer, points, spacing=.04):
    """Bytes/bursts written in response to pure pointer motion over the given (col, row) cells."""
    start_index = len(viewer.chunks)
    t0 = time.monotonic()
    per_event = []
    for col, row in points:
        mark = len(viewer.chunks)
        os.write(viewer.master, f'\x1b[<{MOTION};{col};{row}M'.encode())
        viewer.pump(spacing)
        per_event.append(sum(n for _, n in viewer.chunks[mark:]))
    stats = viewer._stats(start_index, t0)
    events = len(points)
    return dict(events=events, bytes=stats['bytes'], bursts=stats['bursts'],
                bytes_per_event=round(stats['bytes'] / events, 1),
                bursts_per_event=round(stats['bursts'] / events, 3),
                events_with_output=sum(1 for b in per_event if b))


def motion_phases(viewer, list_rows, selected_row, cols, events=40, spacing=.04):
    """same_row: motion along the selected row (nothing should change); outside: motion over
    the blank row under the tab bar; across_rows: motion that crosses list rows (hover may
    legitimately change selection). The idle baseline is a same-length window with no input."""
    baseline = viewer.window(events * spacing)
    phases = dict(
        same_row=[(cols[(i * 7) % len(cols)], selected_row) for i in range(events)],
        outside=[(cols[(i * 7) % len(cols)], 2) for i in range(events)],
        across_rows=[(cols[0], list_rows[i % len(list_rows)]) for i in range(events)])
    out = dict(mouse_modes=sorted(viewer.mouse_modes), any_motion_reporting='1003' in viewer.mouse_modes,
               spacing_ms=spacing * 1000,
               idle_baseline=dict(seconds=round(events * spacing, 2), bytes=baseline['bytes'], bursts=baseline['bursts']))
    for name, points in phases.items():
        out[name] = motion_probe(viewer, points, spacing)
        if name == 'across_rows':
            viewer.measure(f'\x1b[<{MOTION};{cols[0]};{selected_row}M'.encode(), lambda: True)
    return out


def list_rows(viewer, pattern):
    rows = [y + 1 for y, line in enumerate(viewer.screen.display) if re.search(pattern, line[20:66])]
    selected = next((y + 1 for y, line in enumerate(viewer.screen.display)
                     if re.search(pattern, line[20:66]) and ('›' in line[20:66] or '▌' in line[20:66] or '▸' in line[20:24])), rows[0] if rows else 6)
    return rows, selected


def scenario_mouse(binary):
    result = {}
    env = Env(binary)
    try:
        folder = env.root / 'work'
        folder.mkdir()
        for i in range(8):
            env.cx('new', '--directory', str(folder), '--name', f'perf-{i}', '--key', f'perf-{i}')
        viewer = Viewer(env, 'Work')
        try:
            viewer.wait(lambda: 'perf-7' in viewer.text(), what='session rows')
            viewer.quiesce(.5)
            rows, selected = list_rows(viewer, r'perf-\d')
            result['sessions'] = motion_phases(viewer, rows, selected, list(range(24, 64)))
        finally:
            result.setdefault('sessions', {})['viewer'] = viewer.close()
    finally:
        result.setdefault('sessions', {})['cleanup'] = env.cleanup()
    env = Env(binary)
    try:
        folder = env.root / 'files'
        folder.mkdir()
        for i in range(20):
            (folder / f'file-{i:02}.txt').write_text(f'{i}\n')
        viewer = Viewer(env, 'Files', browser_snapshot(folder, folder / 'file-00.txt'))
        try:
            viewer.wait(lambda: 'file-19' in viewer.text(), what='file rows')
            viewer.quiesce(.5)
            rows, selected = list_rows(viewer, r'file-\d\d')
            result['files'] = motion_phases(viewer, rows, selected, list(range(24, 110)))
        finally:
            result.setdefault('files', {})['viewer'] = viewer.close()
    finally:
        result.setdefault('files', {})['cleanup'] = env.cleanup()
    return result


def idle_probe(viewer, env, seconds):
    pids = set(viewer.descendants()) | env.owned_pids()
    viewer_pids = viewer.descendants()
    before_all = {pid: cpu_ticks(pid) for pid in pids}
    before_viewer = sum(cpu_ticks(pid) for pid in viewer_pids)
    stats = viewer.window(seconds)
    after_viewer = sum(cpu_ticks(pid) for pid in viewer_pids)
    after_all = sum(cpu_ticks(pid) for pid in pids)
    return dict(seconds=seconds, viewer_cpu_s=round((after_viewer - before_viewer) / HZ, 3),
                tree_cpu_s=round((after_all - sum(before_all.values())) / HZ, 3), processes=len(pids),
                bytes=stats['bytes'], bytes_per_s=round(stats['bytes'] / seconds, 1), bursts=stats['bursts'])


def scenario_idle(binary, seconds=5.0):
    result = {}
    env = Env(binary)
    try:
        folder = env.root / 'files'
        folder.mkdir()
        for i in range(20):
            (folder / f'file-{i:02}.txt').write_text(f'{i}\n')
        viewer = Viewer(env, 'Files', browser_snapshot(folder, folder / 'file-00.txt'))
        try:
            viewer.wait(lambda: 'file-19' in viewer.text(), what='file rows')
            viewer.quiesce(1.0)
            result['files'] = idle_probe(viewer, env, seconds)
        finally:
            result.setdefault('files', {})['viewer'] = viewer.close()
    finally:
        result.setdefault('files', {})['cleanup'] = env.cleanup()
    env = Env(binary)
    try:
        folder = env.root / 'work'
        folder.mkdir()
        env.cx('new', '--directory', str(folder), '--name', 'perf-idle', '--key', 'perf-idle')
        viewer = Viewer(env, 'Work')
        try:
            viewer.wait(lambda: 'perf-idle' in viewer.text() and '$' in viewer.text(), what='session row')
            viewer.quiesce(1.0)
            result['sessions'] = idle_probe(viewer, env, seconds)
        finally:
            result.setdefault('sessions', {})['viewer'] = viewer.close()
    finally:
        result.setdefault('sessions', {})['cleanup'] = env.cleanup()
    return result


SCENARIOS = ('preview_scroll', 'directory_list', 'sessions_attach', 'mouse_motion', 'idle')


def run_once(binary, iterations, only):
    out = {}
    for name in SCENARIOS:
        if only and name not in only:
            continue
        started = time.monotonic()
        try:
            if name == 'preview_scroll':
                out[name] = scenario_preview(binary, iterations)
            elif name == 'directory_list':
                out[name] = scenario_directory(binary)
            elif name == 'sessions_attach':
                out[name] = scenario_sessions(binary, max(3, iterations // 6))
            elif name == 'mouse_motion':
                out[name] = scenario_mouse(binary)
            elif name == 'idle':
                out[name] = scenario_idle(binary)
        except AssertionError as error:
            out[name] = dict(error=str(error))
        out[name]['wall_s'] = round(time.monotonic() - started, 2)
    return out


def cleanup_failures(value, path=''):
    """Any cleanup block with survivors or our strays; returned for the overall verdict."""
    bad = []
    if isinstance(value, dict):
        if 'pids_observed' in value and (value['remaining'] or value['orphans_ours_stray'] or value['stale_handoffs']):
            bad.append(path)
        for key, item in value.items():
            bad += cleanup_failures(item, f'{path}.{key}' if path else key)
    return bad


def errors(value, path=''):
    bad = []
    if isinstance(value, dict):
        if 'error' in value:
            bad.append(f'{path}: {value["error"]}')
        for key, item in value.items():
            if key != 'error':
                bad += errors(item, f'{path}.{key}' if path else key)
    return bad


def flatten(value, path=''):
    if isinstance(value, dict):
        for key, item in value.items():
            yield from flatten(item, f'{path}.{key}' if path else key)
    elif isinstance(value, (int, float)) and not isinstance(value, bool):
        yield path, value


def aggregate(runs):
    """Across repeats: median/min/max of every numeric leaf (wall times excluded)."""
    values = {}
    for run in runs:
        for path, number in flatten(run):
            if 'cleanup' in path or path.endswith('wall_s'):
                continue
            values.setdefault(path, []).append(number)
    return {path: dict(median=round(statistics.median(v), 2), min=min(v), max=max(v))
            for path, v in values.items() if len(v) > 1}


def main():
    parser = argparse.ArgumentParser(description=__doc__.split('\n')[0])
    parser.add_argument('binary')
    parser.add_argument('--out')
    parser.add_argument('--repeat', type=int, default=1)
    parser.add_argument('--iterations', type=int, default=30)
    parser.add_argument('--only', default='')
    args = parser.parse_args()
    binary = str(pathlib.Path(args.binary).resolve())
    if not shutil.which('tmux'):
        sys.exit('tmux is required')
    only = {s for s in args.only.split(',') if s}
    version = subprocess.run([binary, '--version'], capture_output=True, text=True).stdout.strip()
    runs = []
    for _ in range(args.repeat):
        runs.append(run_once(binary, args.iterations, only))
    failures = [f'run{i}.{p}' for i, run in enumerate(runs) for p in cleanup_failures(run)]
    errs = [f'run{i}.{p}' for i, run in enumerate(runs) for p in errors(run)]
    report = dict(schema=1, harness='tests/perf_harness.py', binary_version=version,
                  build=pathlib.Path(binary).parent.name, host=dict(kernel=platform.release(), cpus=os.cpu_count(),
                  python=platform.python_version(), tmux=subprocess.run(['tmux', '-V'], capture_output=True, text=True).stdout.strip()),
                  terminal=dict(cols=COLS, rows=ROWS), quiet_ms=QUIET * 1000, burst_gap_ms=BURST_GAP * 1000,
                  iterations=args.iterations, repeat=args.repeat, detach_key=DETACH_LABEL,
                  result='PASS' if not failures and not errs else 'FAIL',
                  cleanup_failures=failures, errors=errs,
                  aggregate=aggregate(runs) if len(runs) > 1 else None, runs=runs)
    text = json.dumps(report, indent=1)
    if args.out:
        pathlib.Path(args.out).parent.mkdir(parents=True, exist_ok=True)
        pathlib.Path(args.out).write_text(text + '\n')
    print(text)
    sys.exit(0 if report['result'] == 'PASS' else 1)


if __name__ == '__main__':
    main()
