#!/usr/bin/env python3
"""The hub's tiles, the palette and the file search answer the arrows (v0.2.2).

usage: grid_keys_pty.py BINARY

A private HOME with four managed shells; the viewer runs on a 190x50 PTY.
  * Hub tiles: Tab focuses them; the arrows and `h j k l` move the focus in 2D
    (the focused tile carries the `▌` bar); Up from the top row returns to the map;
    `␣` peeks the focused tile and Enter attaches it (a tmux client appears).
  * Palette (`/`): Down, Ctrl+N and Ctrl+P move the drawn selection; Left moves the
    text cursor, so typing lands where it is.
  * File search (`f`, then `/`): Down moves to the next match, Left/Right move the
    cursor in the query.
Before v0.2.2 the palette drew the hub's selection, never its own (the arrows changed
nothing visible), and the tiles only moved in a line.
"""
import pathlib, re, subprocess, sys, time

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import perf_harness as ph  # noqa: E402
import pty_lib  # noqa: E402

COLS, ROWS = 190, 50
NAMES = ['grid-a', 'grid-b', 'grid-c', 'grid-d']
UP, DOWN, RIGHT, LEFT = b'\x1b[A', b'\x1b[B', b'\x1b[C', b'\x1b[D'


def main():
    binary = str(pathlib.Path(sys.argv[1]).resolve())
    env = ph.Env(binary)
    viewer = None
    try:
        sock = str(env.state / 'managed.sock')
        for i, name in enumerate(NAMES):
            folder = env.root / f'work-{i}'
            folder.mkdir()
            made = env.cx('new', '--directory', str(folder), '--name', name, '--key', name)
            assert made.returncode == 0, f'cx new {name} failed'
        viewer = pty_lib.Pty([binary], env.env, env.root, COLS, ROWS)
        text = lambda: '\n'.join(viewer.screen.display)  # noqa: E731

        def settle(until, what, limit=5.0):
            end = time.monotonic() + limit
            while not until():
                assert time.monotonic() < end, f'{what}:\n{text()}'
                assert viewer.proc.poll() is None, f'viewer exited {viewer.proc.returncode}:\n{text()}'
                viewer.read(.03)
            viewer.read(.05)

        def press(keys, until, what):
            viewer.write(keys)
            settle(until, what)

        def focused():
            """(name, row, col) of the tile under the `▌` focus bar, else None."""
            for y, line in enumerate(viewer.screen.display):
                at = line.find('▌')
                if at < 0:
                    continue
                m = re.search(r'grid-[a-d]', line[at:].split('│')[0])
                if m:
                    return m.group(0), y, at
            return None

        settle(lambda: all(n in text() for n in NAMES), 'hub tiles never appeared', 15)
        # The tiles rank by attention (state, then the latest activity), so they settle
        # once every new shell has drawn its prompt and gone idle: then the grid stays.
        def order():
            return [m.group(0) for m in re.finditer(r'grid-[a-d]', text())]
        settle(lambda: text().count('idle') >= len(NAMES), 'the shells never went idle', 10)
        end, last, since = time.monotonic() + 10, None, time.monotonic()
        while time.monotonic() - since < 1.5:
            assert time.monotonic() < end, f'the tiles never settled: {order()}'
            if order() != last:
                last, since = order(), time.monotonic()
            viewer.read(.1)
        # Tiles: Tab focuses them.
        press(b'\t', lambda: '▌SESSIONS' in text(), 'Tab did not focus the tiles')
        start = focused()
        assert start, f'no focused tile:\n{text()}'
        # Four tiles are a 2x2 grid; Tab focuses the first (top left). Each key below
        # moves the focus one tile the way it points.
        assert start[2] < 4, f'the first tile is not top left: {start}'
        moves = 0
        for key, ok in [(DOWN, lambda a, b: b[1] > a[1] and b[2] == a[2]),
                        (UP, lambda a, b: b[1] < a[1] and b[2] == a[2]),
                        (RIGHT, lambda a, b: b[1] == a[1] and b[2] > a[2]),
                        (LEFT, lambda a, b: b[1] == a[1] and b[2] < a[2]),
                        (b'j', lambda a, b: b[1] > a[1] and b[2] == a[2]),
                        (b'k', lambda a, b: b[1] < a[1] and b[2] == a[2]),
                        (b'l', lambda a, b: b[1] == a[1] and b[2] > a[2]),
                        (b'h', lambda a, b: b[1] == a[1] and b[2] < a[2])]:
            before = focused()
            viewer.write(key)
            end = time.monotonic() + 2
            while focused() == before and time.monotonic() < end:
                viewer.read(.03)
            viewer.read(.05)
            after = focused()
            assert after and ok(before, after), f'{key!r}: {before} -> {after}\n{text()}'
            moves += 1
        # Up from the top row: the map (the tiles lose their focused label).
        for _ in range(3):
            viewer.write(UP)
            viewer.read(.1)
        settle(lambda: '▌SESSIONS' not in text(), 'Up from the top row stayed in the tiles')
        press(DOWN, lambda: '▌SESSIONS' in text(), 'Down from the map did not enter the tiles')
        # ␣ peeks the focused tile, Esc returns, Enter attaches it.
        name = focused()[0]
        press(b' ', lambda: name in viewer.screen.display[0] and 'OTHERS' in text(),
              'space did not peek the focused tile')
        press(b'\x1b', lambda: '▌SESSIONS' in text(), 'Esc did not leave the peek')
        settle(lambda: focused() and focused()[0] == name, 'the focus moved')
        def attached():
            """Terminal clients of the managed server (cx's control-mode client aside)."""
            out = subprocess.run(['tmux', '-S', sock, 'list-clients', '-F', '#{client_control_mode}'],
                                 env=env.env, capture_output=True, text=True).stdout
            return out.split().count('0')
        viewer.write(b'\r')
        end = time.monotonic() + 5
        while not attached():
            assert time.monotonic() < end, f'Enter attached nothing:\n{text()}'
            viewer.read(.05)
        # (Ctrl+\ only once the client drew its status line: before tmux puts the
        # terminal in raw mode the byte would be a SIGQUIT.)
        settle(lambda: 'detach' in viewer.screen.display[-1], 'the attached client drew nothing')
        viewer.write(b'\x1c')  # Ctrl+\ detaches
        end = time.monotonic() + 5
        while attached():
            assert time.monotonic() < end, 'Ctrl+\\ did not detach'
            viewer.read(.05)
        viewer.screen.reset()
        settle(lambda: all(n in text() for n in NAMES), 'detach did not return to the hub', 8)

        # Palette: the arrows and Ctrl+N/P move its own selection.
        def marked():
            for line in viewer.screen.display:
                if '│▌' in line:
                    return line.split('│▌')[1].split('│')[0].strip()
            return None
        press(b'/', lambda: '/ ▏' in text() and marked(), 'the palette did not open')
        rows = [marked()]
        for key in (DOWN, b'\x0e', b'\x10', UP):  # Down, Ctrl+N, Ctrl+P, Up
            before = marked()
            press(key, lambda: marked() != before, f'{key!r} did not move the palette selection')
            rows.append(marked())
        assert rows[3] == rows[1] and rows[4] == rows[0], rows
        # Left moves the cursor: typing lands before the last letter.
        viewer.write(b'grd')
        viewer.read(.2)
        press(LEFT, lambda: 'gr▏d' in text(), 'Left did not move the palette cursor')
        press(b'i', lambda: 'gri▏d' in text(), 'typing did not land at the cursor')
        press(RIGHT, lambda: 'grid▏' in text(), 'Right did not move the palette cursor')
        press(b'\x1b', lambda: '/ ' not in text().split('\n')[4], 'Esc did not close the palette')

        # File search: Down moves between matches, Left/Right edit the query.
        letter = re.search(r'\s(\w)\s+then', text())
        assert letter, f'no device letter on the vitals strip:\n{text()}'
        press(letter.group(1).encode(), lambda: 'SESSIONS' in text() and 'LINKS' in text() or 'new' in text(),
              'the device did not open')
        press(b'f', lambda: 'work-0' in text(), 'files did not open')
        press(b'/', lambda: 'Search' in text(), 'file search did not open')
        viewer.write(b'work')
        settle(lambda: re.search(r'Search · 1/4', text()), 'four matches for "work"')
        press(DOWN, lambda: re.search(r'Search · 2/4', text()), 'Down did not move to the next match')
        press(b'\x0e', lambda: re.search(r'Search · 3/4', text()), 'Ctrl+N did not move to the next match')
        press(UP, lambda: re.search(r'Search · 2/4', text()), 'Up did not move to the previous match')
        viewer.write(LEFT * 4 + b'x')
        settle(lambda: '/  xwork' in viewer.screen.display[-1], 'Left did not move the search cursor')
        print(f'PASS grid_keys_pty: {moves} grid moves, palette, file search')
    finally:
        if viewer:
            viewer.close()
        result = env.cleanup()
        if result['remaining'] or result['orphans_ours_stray']:
            raise AssertionError(f'cleanup: {result}')


if __name__ == '__main__':
    main()
