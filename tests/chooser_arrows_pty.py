#!/usr/bin/env python3
"""Arrows never dismiss the new-session popups (v0.2.2).

usage: chooser_arrows_pty.py BINARY

A private HOME; the viewer runs on a 120x40 PTY.
  * Files, `!`: the session chooser. Left and Right keep it open and, with more than
    one choice offered (Shell, plus Devcontainer, Claude or Codex where the host has
    them), move its selection along the row; Esc closes it. Before v0.2.2 Left closed it.
  * The palette's New session: the device chooser, Enter: the agent step. Left there
    returns to the device chooser; Esc closes it.
No session is created.
"""
import json, pathlib, re, sys, time

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import perf_harness as ph  # noqa: E402
import pty_lib  # noqa: E402

COLS, ROWS = 120, 40
LEFT, RIGHT = b'\x1b[D', b'\x1b[C'
CHOICES = ['[c] Claude', '[x] Codex', '[n] Shell', '[d] Devcontainer']


def main():
    binary = str(pathlib.Path(sys.argv[1]).resolve())
    env = ph.Env(binary)
    viewer = None
    try:
        viewer = pty_lib.Pty([binary], env.env, env.root, COLS, ROWS)
        text = lambda: '\n'.join(viewer.screen.display)  # noqa: E731

        def settle(until, what, limit=8.0):
            end = time.monotonic() + limit
            while not until():
                assert time.monotonic() < end, f'{what}:\n{text()}'
                assert viewer.proc.poll() is None, f'viewer exited {viewer.proc.returncode}:\n{text()}'
                viewer.read(.03)
            viewer.read(.05)

        def press(keys, until, what):
            viewer.write(keys)
            settle(until, what)

        def selected():
            """The chooser choice in the box marked `›` (the selection), else None."""
            rows = viewer.screen.display
            for y, line in enumerate(rows[:-1]):
                x = line.find('┏ ›')
                if x >= 0:
                    cell = rows[y + 1][x:].split('┃')[1]
                    return next((c for c in CHOICES if c in cell), None)
            return None

        def sessions():
            out = env.cx('sessions')
            assert out.returncode == 0, 'cx sessions failed'
            return json.loads(out.stdout or b'[]') == []

        # A device with no sessions opens on the welcome page: Esc is plain SSH.
        settle(lambda: 'cx › ' in viewer.screen.display[0], 'the viewer drew nothing', 15)
        if 'welcome' in viewer.screen.display[0]:
            press(b'\x1b', lambda: 'welcome' not in viewer.screen.display[0], 'Esc kept the welcome page')
        letter = None
        def strip():
            nonlocal letter
            m = re.search(r'\s(\w)\s+then', text())
            letter = m and m.group(1)
            return letter
        settle(strip, 'no device letter on the vitals strip', 15)
        press(letter.encode(), lambda: 'new' in text(), 'the device did not open')
        press(b'f', lambda: 'NOR' in text(), 'files did not open')  # yazi's mode pill
        press(b'!', lambda: '[n] Shell' in text() and selected(), '! did not open the chooser')
        # (The availability check may add choices: it can take two 3 s probe timeouts.)
        end = time.monotonic() + 8
        while time.monotonic() < end and sum(c in text() for c in CHOICES) < 2:
            viewer.read(.05)
        offered = [c for c in CHOICES if c in text()]
        moves = 0
        # Left first: on the first choice it stays there (before v0.2.2 it closed).
        for key in (LEFT, RIGHT, LEFT):
            before = selected()
            viewer.write(key)
            end = time.monotonic() + 1
            while selected() == before and time.monotonic() < end:
                viewer.read(.03)
            viewer.read(.1)
            assert '[n] Shell' in text() and selected(), f'{key!r} closed the chooser:\n{text()}'
            if len(offered) > 1 and moves:
                assert selected() != before, f'{key!r} did not move from {before}:\n{text()}'
            moves += 1
        press(b'\x1b', lambda: '[n] Shell' not in text(), 'Esc did not close the chooser')

        # The agent step, from the device chooser: Left goes back a step.
        press(b'H', lambda: re.search(r'cx › (network|sessions)', viewer.screen.display[0]),
              'Shift+H did not reach the hub')
        press(b'\x10', lambda: '▏' in text(), 'Ctrl+P did not open the palette')
        viewer.write(b'New session')
        viewer.read(.3)
        press(b'\r', lambda: 'execution device' in text(), 'New session did not ask for a device')
        press(b'\r', lambda: 'execution device' not in text() and 'Shell' in text(),
              'Enter did not reach the agent step')
        press(LEFT, lambda: 'execution device' in text(), 'Left did not go back to the device chooser')
        press(b'\x1b', lambda: 'execution device' not in text(), 'Esc did not close the device chooser')
        assert sessions(), 'a popup key created a session'
        print(f'PASS chooser_arrows_pty: {moves} chooser arrows over {len(offered)} choices kept it open, agent step Left went back')
    finally:
        if viewer:
            viewer.close()
        result = env.cleanup()
        if result['remaining'] or result['orphans_ours_stray']:
            raise AssertionError(f'cleanup: {result}')


if __name__ == '__main__':
    main()
