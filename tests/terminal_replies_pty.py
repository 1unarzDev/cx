#!/usr/bin/env python3
"""Files stays escapable on a terminal that answers cx's startup probe (OSC 11, CSI 16 t,
kitty graphics query, XTVERSION, DA1) like kitty, ghostty and foot do: at once, late
(100-800 ms, after the probe gave up), split across reads, and from inside tmux (a private
tmux server with passthrough). After the replies, q / Esc / H / arrows must each act.

usage: terminal_replies_pty.py BINARY

Private HOME/XDG state/TMUX_TMPDIR per viewer (perf_harness.Env); the tmux case starts
its own server on a socket under that HOME. Prints one JSON line; exits non-zero on failure.
"""
import json, os, pathlib, re, select, subprocess, sys, time

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import perf_harness as ph  # noqa: E402
import pty_lib  # noqa: E402

BINARY = str(pathlib.Path(sys.argv[1]).resolve())
results = {}

# What each terminal answers, per query it understands (None: ignores the query).
TERMINALS = {
    'kitty': {b'\x1b]11;?': b'\x1b]11;rgb:1e1e/1e1e/2e2e\x1b\\', b'\x1b[16t': b'\x1b[6;20;10t',
              b'\x1b_Gi=31': b'\x1b_Gi=31;OK\x1b\\', b'\x1b[>q': b'\x1bP>|kitty(0.39.1)\x1b\\',
              b'\x1b[c': b'\x1b[?62;c'},
    'ghostty': {b'\x1b]11;?': b'\x1b]11;rgb:2828/2c2c/3434\x07', b'\x1b[16t': b'\x1b[6;17;8t',
                b'\x1b_Gi=31': b'\x1b_Gi=31;OK\x1b\\', b'\x1b[>q': b'\x1bP>|ghostty 1.1.3\x1b\\',
                b'\x1b[c': b'\x1b[?62;22c'},
    'foot': {b'\x1b]11;?': b'\x1b]11;rgb:1010/1010/1010\x1b\\', b'\x1b[16t': b'\x1b[6;18;9t',
             b'\x1b[>q': b'\x1bP>|foot(1.20.2)\x1b\\', b'\x1b[c': b'\x1b[?62;4;22c'},
}


class Terminal(pty_lib.Pty):
    """A PTY that answers the probe like a real terminal: after `delay` seconds, in pieces
    of `split` bytes written `gap` seconds apart (0: one write)."""

    def __init__(self, *args, answers, delay, split, tmux):
        self.answers, self.delay, self.split, self.tmux = answers, delay, split, tmux
        self.due = []  # (time, bytes)
        self.answered = False
        super().__init__(*args)

    def _queue(self, data):
        if self.answered or b'\x1b[c' not in data:
            return
        self.answered = True
        # Inside tmux the query is wrapped for passthrough; tmux answers its own share
        # at once (OSC 11, 16 t, DA1) and the outer terminal answers the wrapped ones late.
        reply, outer = b'', b''
        plain = data.replace(b'\x1b\x1b', b'\x1b')
        for query, answer in self.answers.items():
            if query in plain:
                if self.tmux and query in (b'\x1b_Gi=31', b'\x1b[>q'):
                    outer += answer
                else:
                    reply += answer
        if self.tmux:
            # tmux's own DA1, then the outer terminal's (tmux forwards unknown replies).
            reply = reply.replace(self.answers[b'\x1b[c'], b'\x1b[?1;2;4c')
            outer += self.answers[b'\x1b[c']
            now = time.monotonic()
            self.due.append((now + .005, reply))
            self._spread(now + self.delay, outer)
        else:
            self._spread(time.monotonic() + self.delay, reply)

    def _spread(self, at, data):
        step = self.split or len(data)
        for i in range(0, len(data), step):
            self.due.append((at + (i // step) * .03, data[i:i + step]))

    def flush(self):
        now = time.monotonic()
        for item in [d for d in self.due if d[0] <= now]:
            os.write(self.master, item[1])
            self.due.remove(item)

    def read(self, timeout):
        end = time.monotonic() + max(timeout, 0)
        while True:
            self.flush()
            wait = end - time.monotonic()
            if self.due:
                wait = min(wait, min(d[0] for d in self.due) - time.monotonic())
            if select.select([self.master], [], [], max(wait, 0))[0]:
                break
            if time.monotonic() >= end:
                self.flush()
                return None
        try:
            data = os.read(self.master, 1 << 16)
        except OSError:
            return None
        self._queue(data)
        self.stream.feed(self.decoder.decode(data))
        return data


def viewer(env, project, extra, **term):
    browser = ph.browser_snapshot(project, project / 'b.txt')
    original = pty_lib.Pty
    pty_lib.Pty = lambda *a: Terminal(*a, **term)
    try:
        v = ph.Viewer.__new__(ph.Viewer)
        env.env.update(extra)
        for k in [k for k, val in extra.items() if val is None]:
            env.env.pop(k, None)
        v.open(env, dict(device_ids=['local'], device=1, focus='Workspace', view='Files', browser=browser))
    finally:
        pty_lib.Pty = original
    return v


def status(v):
    last = v.screen.display[-1]
    return last if re.match(r'^\W?\s*(NOR|SEL|UNS) ', last) else None


def in_files(v):
    return status(v) is not None


def run_case(name, answers, delay, split, tmux):
    env = ph.Env(BINARY)
    server = None
    try:
        project = env.root / 'project'
        (project / 'sub').mkdir(parents=True)
        for n in ('a.txt', 'b.txt', 'c.txt'):
            (project / n).write_text(n)
        extra = dict(CX_THEME=None, TERM='xterm-kitty' if name.startswith('kitty') else 'xterm-256color')
        if tmux:
            sock = env.root / 'outer.sock'
            subprocess.run(['tmux', '-S', str(sock), '-f', '/dev/null', 'new-session', '-d', '-s', 'outer',
                            'sleep', '600'], env=env.env, check=True, capture_output=True)
            subprocess.run(['tmux', '-S', str(sock), 'set-option', '-g', 'allow-passthrough', 'on'],
                           env=env.env, check=True, capture_output=True)
            server = sock
            out = subprocess.run(['tmux', '-S', str(sock), 'display-message', '-p', '#{pid}'],
                                 env=env.env, capture_output=True, text=True).stdout.strip()
            extra['TMUX'] = f'{sock},{out},0'
            extra['TMUX_PANE'] = '%0'
        v = viewer(env, project, extra, answers=answers, delay=delay, split=split, tmux=tmux)
        try:
            v.wait(lambda: in_files(v), what=f'{name}: files view')
            # Let every late reply arrive and be read.
            v.pump(delay + .6)
            steps = []

            def press(key, what, ok, again=None):
                os.write(v.master, key)
                try:
                    v.wait(ok, timeout=3, what=f'{name}: {what}')
                except AssertionError as e:
                    raise AssertionError(f'{e}\n' + v.text())
                steps.append(what)
                if again:
                    os.write(v.master, again)
                    v.wait(lambda: in_files(v), timeout=3, what=f'{name}: back into files after {what}')

            row = lambda: next((l for l in v.screen.display if '│' in l or 'txt' in l), '')
            before = v.text()
            press(b'\x1b[B', 'down arrow moves', lambda: v.text() != before)
            press(b'q', 'q leaves files', lambda: not in_files(v), b'f')
            press(b'\x1b', 'Esc leaves files', lambda: not in_files(v), b'f')
            if name == 'kitty_prompt':
                # The header's back chip and the status hint tap like their keys.
                def click(line, label):
                    col = v.screen.display[line].index(label) + 1
                    return f'\x1b[<0;{col};{line % v.screen.lines + 1}M\x1b[<0;{col};{line % v.screen.lines + 1}m'.encode()
                hint = v.screen.display[-1]
                if 'q back' not in hint or 'H hub' not in hint or '‹ back' not in v.screen.display[0]:
                    raise AssertionError(f'no way-out hints: {v.screen.display[0]!r} {hint!r}')
                press(click(0, '‹ back'), 'tap ‹ back leaves files', lambda: not in_files(v), b'f')
                press(click(-1, 'H hub'), 'tap H hub goes to the hub', lambda: ph.on_hub(v.text()))
                results[name] = steps
                return
            press(b'H', 'H goes to the hub', lambda: ph.on_hub(v.text()))
            results[name] = steps
        finally:
            v.close()
    finally:
        if server:
            subprocess.run(['tmux', '-S', str(server), 'kill-server'], env=env.env, capture_output=True)
        env.cleanup()


def main():
    cases = []
    for term, answers in TERMINALS.items():
        cases.append((f'{term}_prompt', answers, .01, 0, False))
        cases.append((f'{term}_late', answers, .35, 0, False))
        cases.append((f'{term}_late_split', answers, .2, 3, False))
    cases.append(('kitty_very_late', TERMINALS['kitty'], .8, 0, False))
    cases.append(('kitty_tmux', TERMINALS['kitty'], .3, 0, True))
    cases.append(('ghostty_tmux_split', TERMINALS['ghostty'], .15, 4, True))
    failures = []
    for case in cases:
        try:
            run_case(*case)
        except AssertionError as e:
            failures.append(f'{case[0]}: {e}')
    print(json.dumps(dict(passed=sorted(results), failures=[f.splitlines()[0] for f in failures])))
    for f in failures:
        print(f, file=sys.stderr)
    sys.exit(1 if failures else 0)


if __name__ == '__main__':
    main()
