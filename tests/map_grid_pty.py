#!/usr/bin/env python3
"""The map's nodes and a device's tiles are walked by place, and a tap focuses a tile
(v0.2.2).

usage: map_grid_pty.py BINARY

A private HOME: this device (`home`) with six managed shells and four devices that never
answer (`.invalid` targets: north, east, south, west), on a 190x50 PTY.
  * Hub map (focused at startup on `home`): Right reaches a node right of home, Left
    returns to home, Left again a node left of it, Right home again; Down a node below
    it, Up home. The vitals strip names the selected node.
  * A device's grid (its letter): six tiles in a 3x2 grid. Right/Left/Down/Up and
    `l h j k` move the `▌` focus one tile the way they point; Up from the top row stays
    put; Down from the bottom row focuses LINKS, and Tab returns to the tiles.
  * LINKS: Enter opens the selected link's editor; Left/Right select another link
    (Esc, an arrow, Enter, Esc is no double Esc: the hub stays away).
  * A tap on a tile focuses it (and attaches nothing).
On v0.2.1 a device's arrows stepped along the order (Down from a top tile reached the
next tile in the row), Tab stepped tiles, and LINKS was no zone.
"""
import json, pathlib, re, subprocess, sys, time

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import perf_harness as ph  # noqa: E402
import pty_lib  # noqa: E402

COLS, ROWS = 190, 50
NAMES = ['north', 'east', 'south', 'west']
UP, DOWN, RIGHT, LEFT = b'\x1b[A', b'\x1b[B', b'\x1b[C', b'\x1b[D'


def main():
    binary = str(pathlib.Path(sys.argv[1]).resolve())
    env = ph.Env(binary)
    viewer = None
    try:
        (env.state / 'devices.json').write_text(json.dumps(
            [{'id': 'local', 'name': 'home', 'target': None, 'account': 'me', 'host': 'home',
              'status': 'local', 'observed_at': 0}]
            + [{'id': n, 'name': n, 'target': f'{n}.invalid', 'account': 'robot', 'host': n,
                'status': 'unknown', 'observed_at': 0} for n in NAMES]))
        sock = str(env.state / 'managed.sock')
        for i in range(6):
            folder = env.root / f'w{i}'
            folder.mkdir()
            made = env.cx('new', '--directory', str(folder), '--name', f'tile-{i}', '--key', f'tile-{i}')
            assert made.returncode == 0, f'cx new tile-{i} failed'
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

        # ---------------------------------------------------------------- the map
        def picked():
            """The node the vitals strip names (`  name  this device` / `  name  no route`)."""
            for line in viewer.screen.display:
                m = re.match(r'  (home|north|east|south|west)  ', line)
                if m:
                    return m.group(1)
            return None

        def where(name):
            """(row, column) of a node's name on the map."""
            for y, line in enumerate(viewer.screen.display[:20]):
                m = re.search(rf'(?<![\w@]){name}(?![\w@])', line)
                if m:
                    return y, m.start()
            raise AssertionError(f'{name} is not on the map:\n{text()}')

        settle(lambda: all(n in text() for n in NAMES) and picked() == 'home'
               and 'tile-5' in text(), 'the hub never drew its map and tiles', 15)
        home = where('home')
        steps = 0
        for key, side, back in [(RIGHT, lambda p: p[1] > home[1], LEFT),
                                (LEFT, lambda p: p[1] < home[1], RIGHT)]:
            press(key, lambda: picked() != 'home', f'{key!r} left home nowhere')
            node = picked()
            assert side(where(node)), f'{key!r} reached {node} at {where(node)}, home at {home}'
            press(back, lambda: picked() == 'home', f'{back!r} did not return from {node} to home')
            steps += 2
        # Down reaches a node below, Up from there one above it (straight above a lower
        # corner sits the upper one, not home), and Left/Right lead home again.
        press(DOWN, lambda: picked() != 'home', 'Down left home nowhere')
        low = picked()
        assert where(low)[0] > home[0], f'Down reached {low} at {where(low)}, home at {home}'
        press(UP, lambda: picked() != low, f'Up did not leave {low}')
        high = picked()
        assert where(high)[0] < where(low)[0], f'Up from {low} reached {high}'
        steps += 2
        toward = RIGHT if where(high)[1] < home[1] else LEFT
        press(toward, lambda: picked() == 'home', f'{toward!r} from {high} did not reach home')
        steps += 1

        # -------------------------------------------------------- a device's grid
        letter = re.search(r'\s(\w)\s+then', text())
        assert letter, f'no device letter on the vitals strip:\n{text()}'
        press(letter.group(1).encode(), lambda: '▍SESSIONS' in text() and 'LINKS' in text(),
              'the device did not open with its tiles focused')

        def focused():
            """(name, row, col) of the tile under the `▌` focus bar."""
            for y, line in enumerate(viewer.screen.display):
                at = line.find('▌')
                if at < 0:
                    continue
                m = re.search(r'tile-\d', line[at:].split('│')[0])
                if m:
                    return m.group(0), y, at
            return None

        def tile_at(name):
            for y, line in enumerate(viewer.screen.display):
                at = line.find(name)
                if at >= 0:
                    return y, at
            raise AssertionError(f'{name} not drawn:\n{text()}')

        settle(focused, 'no focused tile')
        start = focused()
        assert start[2] < 4, f'the first tile is not top left: {start}'
        moves = 0
        for key, ok in [(RIGHT, lambda a, b: b[1] == a[1] and b[2] > a[2]),
                        (DOWN, lambda a, b: b[1] > a[1] and b[2] == a[2]),
                        (LEFT, lambda a, b: b[1] == a[1] and b[2] < a[2]),
                        (UP, lambda a, b: b[1] < a[1] and b[2] == a[2]),
                        (b'l', lambda a, b: b[1] == a[1] and b[2] > a[2]),
                        (b'j', lambda a, b: b[1] > a[1] and b[2] == a[2]),
                        (b'h', lambda a, b: b[1] == a[1] and b[2] < a[2]),
                        (b'k', lambda a, b: b[1] < a[1] and b[2] == a[2])]:
            before = focused()
            press(key, lambda: focused() != before, f'{key!r} did not move from {before}')
            after = focused()
            assert ok(before, after), f'{key!r}: {before} -> {after}\n{text()}'
            moves += 1
        # Up from the top row stays put, in the tiles.
        before = focused()
        viewer.write(UP)
        viewer.read(.4)
        assert focused() == before and '▍SESSIONS' in text(), f'Up from the top row moved: {focused()}'
        # Down from the bottom row: LINKS. Tab: the tiles again, on the same tile.
        press(DOWN, lambda: focused() != before, 'Down did not reach the bottom row')
        press(DOWN, lambda: '▍LINKS' in text(), 'Down from the bottom row did not focus LINKS')
        press(b'\t', lambda: '▍SESSIONS' in text(), 'Tab did not return to the tiles')
        press(b'\t', lambda: '▍LINKS' in text(), 'Tab did not focus LINKS')

        # LINKS: Enter edits the selected link; another arrow, another link.
        def edited():
            m = re.search(r'\w→(\w)', text())
            return m and m.group(1)
        press(b'\r', edited, 'Enter on LINKS did not open a link')
        first = edited()
        press(b'\x1b', lambda: '▍LINKS' in text(), 'Esc did not return to LINKS')
        targets = {first}
        for key in (LEFT, RIGHT, RIGHT):
            viewer.write(key)
            viewer.read(.3)
            assert '▍LINKS' in text(), f'{key!r} left LINKS:\n{text()}'
            press(b'\r', edited, 'Enter on LINKS did not open a link')
            targets.add(edited())
            press(b'\x1b', lambda: '▍LINKS' in text(), 'Esc did not return to LINKS')
        assert len(targets) >= 2, f'the arrows never selected another link: {targets}'
        press(b'\t', lambda: '▍SESSIONS' in text(), 'Tab did not return to the tiles')

        # A tap focuses the tile it lands on, and attaches nothing.
        def attached():
            out = subprocess.run(['tmux', '-S', sock, 'list-clients', '-F', '#{client_control_mode}'],
                                 env=env.env, capture_output=True, text=True).stdout
            return out.split().count('0')
        current = focused()[0]
        other = next(f'tile-{i}' for i in (4, 5, 1, 2) if f'tile-{i}' != current)
        y, x = tile_at(other)
        y += 3  # inside the screen, below the title
        viewer.write(f'\x1b[<0;{x + 1};{y + 1}M\x1b[<0;{x + 1};{y + 1}m'.encode())
        settle(lambda: focused() and focused()[0] == other, f'a tap on {other} did not focus it')
        viewer.read(.5)
        assert not attached(), 'a first tap attached'
        print(f'PASS map_grid_pty: {steps} map moves, {moves} grid moves, LINKS {sorted(targets)}, tap')
    finally:
        if viewer:
            viewer.close()
        result = env.cleanup()
        if result['remaining'] or result['orphans_ours_stray']:
            raise AssertionError(f'cleanup: {result}')


if __name__ == '__main__':
    main()
