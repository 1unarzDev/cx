#!/usr/bin/env python3
"""A terminal that no longer exists is never offered, and a failed attach never moves
the viewer (decision 2026-11-23).

usage: tests/isolate.sh python tests/dead_terminals_pty.py BINARY

A private HOME inside private namespaces (pty_lib.isolate) with two managed shells and
one outside-tmux interactive bash on a PTY of its own; the viewer runs on a 160x45 PTY
and opens the device's screen (`t`).
  1. The outside bash is a tile (`outside cx`). Its leader is killed: the tile goes
     within one device poll (3 s; it took the 30 s fleet refresh before).
  2. The focused managed shell is killed behind the viewer's back and Enter pressed at
     once (before any listing could drop it): no tmux client, still the device's
     screen (never the hub), the tile is gone, the other shell stays, and the notice
     says `ended · removed` (or the listing won the race and dropped it first).
"""
import json, os, pathlib, re, signal, subprocess, sys, time

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import pty_lib  # noqa: E402

pty_lib.isolate()
import perf_harness as ph  # noqa: E402

COLS, ROWS = 160, 45
NAMES = ['dead-a', 'dead-b']


def main():
    binary = str(pathlib.Path(sys.argv[1]).resolve())
    env = ph.Env(binary)
    viewer = outside = None
    try:
        env.env['CX_REDUCED_MOTION'] = '1'
        local = dict(id='local', name='tranquility', target=None, account='fx', host='tranquility',
                     status='unknown', observed_at=0)
        (env.state / 'devices.json').write_text(json.dumps([local]))
        sock = str(env.state / 'managed.sock')
        ids = {}
        for name in NAMES:
            folder = env.root / name
            folder.mkdir()
            made = env.cx('new', '--directory', str(folder), '--name', name, '--key', name)
            assert made.returncode == 0, f'cx new {name} failed: {made.stderr[-400:]!r}'
            ids[name] = json.loads(made.stdout)['id']
        outside = pty_lib.Pty(['/bin/bash', '--norc', '--noprofile', '-i'], env.env, env.root, 80, 24)
        viewer = pty_lib.Pty([binary], env.env, env.root, COLS, ROWS)
        text = lambda: '\n'.join(viewer.screen.display)  # noqa: E731

        def settle(until, what, limit=8.0):
            end = time.monotonic() + limit
            while not until():
                assert time.monotonic() < end, f'{what}:\n{text()}'
                assert viewer.proc.poll() is None, f'viewer exited {viewer.proc.returncode}:\n{text()}'
                viewer.read(.03)
                while outside.read(0):
                    pass
            viewer.read(.05)

        def clients():
            out = subprocess.run(['tmux', '-S', sock, 'list-clients', '-F', '#{client_control_mode}'],
                                 env=env.env, capture_output=True, text=True).stdout
            return out.split().count('0')

        def focused():
            for line in viewer.screen.display:
                at = line.find('▌')
                if at >= 0:
                    m = re.search(r'dead-[ab]', line[at:].split('│')[0])
                    if m:
                        return m.group(0)
            return None

        def device_screen():
            return not ph.on_hub(viewer.screen.display[0]) and 'cx ›' in viewer.screen.display[0]

        settle(lambda: 'cx ›' in text(), 'viewer never drew', 15)
        viewer.write(b't')
        settle(lambda: device_screen() and all(n in text() for n in NAMES), "the device's screen", 15)
        # 1. The outside bash: a tile, then gone within one device poll of its end.
        settle(lambda: 'outside cx' in text(), 'the outside bash is not a tile', 15)
        os.kill(outside.proc.pid, signal.SIGKILL)
        killed = time.monotonic()
        settle(lambda: 'outside cx' not in text(), 'the closed outside terminal stayed offered', 4.5)
        outside_gone = time.monotonic() - killed
        # 2. The focused managed shell ends behind the viewer's back; Enter at once.
        settle(lambda: focused(), 'no focused shell tile')
        gone = focused()
        other = next(n for n in NAMES if n != gone)
        killed = subprocess.run(['tmux', '-S', sock, 'kill-session', '-t', f'={ids[gone]}'],
                                env=env.env, capture_output=True)
        viewer.write(b'\r')
        assert killed.returncode == 0, killed.stderr
        end, notice = time.monotonic() + 2.5, False
        while time.monotonic() < end:
            assert not clients(), f'Enter attached something:\n{text()}'
            viewer.read(.05)
            notice |= f'{gone} ended · removed' in text()
        settle(lambda: 'cx ›' in text() and gone not in text(), 'the ended tile stayed offered')
        assert device_screen(), f'the failed attach left the device screen:\n{text()}'
        assert other in text(), f'the other shell went too:\n{text()}'
        assert clients() == 0
        print(f'PASS dead_terminals_pty: outside tile gone {outside_gone:.1f}s after its end; '
              f'{gone} ended → Enter stayed on the device screen, tile gone'
              f'{" (ended · removed)" if notice else " (the listing dropped it first)"}')
    finally:
        if outside and outside.proc.poll() is None:
            outside.proc.kill()
        if viewer:
            viewer.close()
        result = env.cleanup()
        if result['remaining'] or result['orphans_ours_stray']:
            raise AssertionError(f'cleanup: {result}')


if __name__ == '__main__':
    main()
