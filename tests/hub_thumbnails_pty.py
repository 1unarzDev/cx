#!/usr/bin/env python3
"""Hub thumbnails show every session's screen, not only the selected one's.

usage: hub_thumbnails_pty.py BINARY

A private HOME with three managed shells, each printing a known line; the viewer starts
on the hub at 190x50 and every tile must show its own line within 1 s of the tiles
appearing. Before v0.2.2 only the selected session was ever captured, so the other tiles
stayed blank (title, `idle 0s` and folder over empty rows).
"""
import pathlib, subprocess, sys, time

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import perf_harness as ph  # noqa: E402
import pty_lib  # noqa: E402

COLS, ROWS = 190, 50
NAMES = ['thumb-a', 'thumb-b', 'thumb-c']


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
        listed = subprocess.run(['tmux', '-S', sock, 'list-sessions', '-F', '#{session_name}'],
                                env=env.env, capture_output=True, text=True, timeout=10)
        sessions = [s for s in listed.stdout.split() if s.startswith('cx-')]
        assert len(sessions) == len(NAMES), f'{len(sessions)} managed sessions'
        # Printed text differs from the typed command, so only output can match.
        marks = [f'MARK{i}OK' for i in range(len(sessions))]
        for i, s in enumerate(sessions):
            subprocess.run(['tmux', '-S', sock, 'send-keys', '-t', s, f"printf 'MARK%sOK\\n' {i}", 'Enter'],
                           env=env.env, check=True, timeout=10)
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline:
            shown = [subprocess.run(['tmux', '-S', sock, 'capture-pane', '-p', '-t', s], env=env.env,
                                    capture_output=True, text=True).stdout for s in sessions]
            if all(m in out for m, out in zip(marks, shown)):
                break
            time.sleep(.05)
        viewer = pty_lib.Pty([binary], env.env, env.root, COLS, ROWS)
        text = lambda: '\n'.join(viewer.screen.display)  # noqa: E731
        end = time.monotonic() + 15
        while not all(n in text() for n in NAMES):
            assert time.monotonic() < end, 'hub tiles never appeared'
            assert viewer.proc.poll() is None, 'viewer exited'
            viewer.read(.05)
        tiles = time.monotonic()
        while not all(m in text() for m in marks):
            if time.monotonic() - tiles > 1.0:
                missing = [m for m in marks if m not in text()]
                raise AssertionError(f'thumbnails without their screens after 1 s: {missing}')
            viewer.read(.02)
        print(f'PASS hub_thumbnails_pty: {len(marks)} tiles show their screens '
              f'{(time.monotonic() - tiles) * 1000:.0f} ms after the tiles appeared')
    finally:
        if viewer:
            viewer.close()
        result = env.cleanup()
        if result['remaining'] or result['orphans_ours_stray']:
            raise AssertionError(f'cleanup: {result}')


if __name__ == '__main__':
    main()
