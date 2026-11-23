#!/usr/bin/env python3
"""Launch keys: `cx` started in a folder, then `c` (2 keys) starts Claude there; the
hub's bare c / x / a / o / n and C / X / A / O work on this device in the folder cx was
started in,
each agent in the mode it was last started in; a device letter then c / x / C / X does
the same on that device's screen; Enter on a device with nothing running opens the
full chooser; Files `!` then c / x / n starts in the folder Files shows.

usage: tests/isolate.sh python tests/launch_keys_pty.py BINARY

A private HOME inside private namespaces (pty_lib.isolate), with stand-in `claude`,
`codex`, `agy` and `opencode` on PATH that answer `--version` and otherwise record their
argv, working folder (and opencode's inline config), then wait. (isolate.sh covers any
real agy/opencode on the caller's PATH, so only these can run.) Viewers run on a 120x40 PTY started in HOME/src/robot-arm; each
launch attaches, and Ctrl+\\ detaches back to the hub.
  1. `t`, Enter (nothing runs yet): the chooser; Left / Right keep it open; `x`, Enter
     (New), Down, Enter: codex in YOLO, in the folder cx was started in.
  2. A new viewer: `c` is its first key: claude (default) there, attached; the hub's
     SESSIONS rule says `here ~/src/robot-arm`.
  3. Bare `x` / `X` / `C`: codex YOLO (remembered), codex resume --last YOLO, claude
     --continue, all there. `t x` / `t C` on the device's screen: the same folder.
  4. Bare `a` / `A` / `o` / `O`: agy full screen (its per-run settings), agy -c,
     opencode --standalone with its inline `{"mouse":true}`, opencode -c; `t A`, `t o`
     on the device's screen.
  5. `t f` (Files opens where cx was started), `h`, then `!` `c` and `!` `a`: claude and
     agy in ~/src.
  6. Viewer B, open since before the YOLO choice, closes after this one; a new viewer
     still remembers codex's YOLO (`x`): the mode is merged state, not B's snapshot.
"""
import json, pathlib, subprocess, sys, time

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import pty_lib  # noqa: E402

pty_lib.isolate()
import perf_harness as ph  # noqa: E402

COLS, ROWS = 120, 40
LEFT, RIGHT, DOWN = b'\x1b[D', b'\x1b[C', b'\x1b[B'
STANDIN = '''#!/bin/sh
if [ "$1" = --version ]; then echo "9.9.9 (stand-in)"; exit 0; fi
[ "$1" = --help ] && exit 0
line="{name} $*${{OPENCODE_CLI_CONFIG_CONTENT:+ [cfg=$OPENCODE_CLI_CONFIG_CONTENT]}}"
echo "${{line% }} @ $(pwd -P)" >> {log}
echo "STANDIN {name} $*"
exec sleep 600
'''
YOLO = {'codex': '--dangerously-bypass-approvals-and-sandbox', 'claude': '--dangerously-skip-permissions'}


def main():
    binary = str(pathlib.Path(sys.argv[1]).resolve())
    env = ph.Env(binary)
    viewer = other = None
    try:
        bin_dir = env.root / 'bin'
        bin_dir.mkdir()
        log = env.root / 'argv.log'
        for name in ('claude', 'codex', 'agy', 'opencode'):
            (bin_dir / name).write_text(STANDIN.format(name=name, log=log))
            (bin_dir / name).chmod(0o700)
        env.env['PATH'] = f"{bin_dir}:{env.env['PATH']}"
        env.env['CX_REDUCED_MOTION'] = '1'
        here = env.root / 'src/robot-arm'
        here.mkdir(parents=True)
        here, home = here.resolve(), env.root.resolve()
        local = dict(id='local', name='tranquility', target=None, account='fx', host='tranquility',
                     status='unknown', observed_at=0)
        (env.state / 'devices.json').write_text(json.dumps([local]))
        sock = str(env.state / 'managed.sock')

        def start():
            v = pty_lib.Pty([binary], env.env, here, COLS, ROWS)
            wait(v, lambda: 'cx ›' in text(v), 'viewer never drew')
            return v

        def text(v):
            return '\n'.join(v.screen.display)

        def wait(v, until, what, limit=12.0):
            end = time.monotonic() + limit
            while not until():
                assert time.monotonic() < end, f'{what}:\n{text(v)}'
                assert v.proc.poll() is None, f'viewer exited {v.proc.returncode}:\n{text(v)}'
                v.read(.03)
                while other and other is not v and other.read(0):
                    pass  # The idle viewer's output is drained so it never blocks.
            v.read(.05)

        def clients():
            # (Not the helper's parked control-mode client.)
            out = subprocess.run(['tmux', '-S', sock, 'list-clients', '-F', '#{client_control_mode}'],
                                 env=env.env, capture_output=True, text=True)
            return [c for c in out.stdout.split() if c != '1']

        def launches():
            return [x.rstrip() for x in log.read_text().splitlines()] if log.exists() else []

        def launch(v, keys, expect, folder=here):
            """Press `keys`, wait for the stand-in's argv and the attached client, detach."""
            expect = f'{expect} @ {folder}'
            before = len(launches())
            for k in keys:
                v.write(k)
                v.read(.15)
            wait(v, lambda: len(launches()) > before, f'{keys!r}: no launch')
            got = launches()[before]
            assert got == expect, f'{keys!r}: launched {got!r}, expected {expect!r}'
            wait(v, lambda: clients(), f'{keys!r}: never attached')
            wait(v, lambda: 'STANDIN' in text(v), f'{keys!r}: attached screen')
            v.write(b'\x1c')
            wait(v, lambda: not clients() and 'cx ›' in text(v), f'{keys!r}: detach')
            return f'{b"".join(keys).decode()!r} -> {got.replace(str(home), "~")}'

        other = start()
        viewer = start()
        proof = []
        # 1. Nothing runs yet: Enter on the device's screen opens the full chooser;
        # arrows never close it.
        viewer.write(b't')
        wait(viewer, lambda: 'start' in text(viewer) and 'network' not in viewer.screen.display[0],
             "the device's screen")
        viewer.write(b'\r')
        wait(viewer, lambda: '[x] Codex' in text(viewer), 'chooser with codex')
        for k in (LEFT, RIGHT, LEFT):
            viewer.write(k)
            viewer.read(.2)
            assert '[x] Codex' in text(viewer), f'an arrow closed the chooser:\n{text(viewer)}'
        viewer.write(b'x')
        # (The stand-in has a `resume`, so the picker may offer past conversations first.)
        wait(viewer, lambda: 'New codex session' in text(viewer) or 'YOLO' in text(viewer),
             'new or past codex')
        if 'YOLO' not in text(viewer):
            viewer.write(b'\r')
            wait(viewer, lambda: 'YOLO' in text(viewer), 'permissions')
        proof.append(launch(viewer, [DOWN, b'\r'], f"codex {YOLO['codex']}"))
        closed = viewer.close()
        assert closed['clean_exit'], closed
        # 2. `cx` in the folder, then `c`: two keys.
        viewer = start()
        proof.append(launch(viewer, [b'c'], 'claude'))
        wait(viewer, lambda: 'here ~/src/robot-arm' in text(viewer), 'the hub names where c x n start')
        # 3. The hub's bare keys, and a device letter's, in the last mode, in that folder.
        proof.append(launch(viewer, [b'x'], f"codex {YOLO['codex']}"))
        proof.append(launch(viewer, [b'X'], f"codex resume --last {YOLO['codex']}"))
        proof.append(launch(viewer, [b'C'], 'claude --continue'))
        proof.append(launch(viewer, [b't', b'x'], f"codex {YOLO['codex']}"))
        proof.append(launch(viewer, [b't', b'C'], 'claude --continue'))
        # 4. Antigravity and opencode, the same keys' grammar.
        agy = 'agy --settings {"altScreenMode":"always"}'
        oc = 'opencode --standalone'
        cfg = ' [cfg={"mouse":true}]'
        proof.append(launch(viewer, [b'a'], agy))
        proof.append(launch(viewer, [b'A'], f'{agy} -c'))
        proof.append(launch(viewer, [b'o'], oc + cfg))
        proof.append(launch(viewer, [b'O'], f'{oc} -c{cfg}'))
        proof.append(launch(viewer, [b't', b'A'], f'{agy} -c'))
        proof.append(launch(viewer, [b't', b'o'], oc + cfg))
        # 5. Files' folder: `!` then c, and `!` then a.
        viewer.write(b't')
        viewer.read(.3)
        viewer.write(b'f')
        wait(viewer, lambda: 'NOR' in text(viewer), 'files')  # yazi's mode pill
        # (Files opens in the folder cx was started in; one up is another folder.)
        viewer.write(b'h')
        wait(viewer, lambda: 'robot-arm' in text(viewer) and '~/src' in viewer.screen.display[0]
             + viewer.screen.display[1], 'files one folder up')
        viewer.write(b'!')
        wait(viewer, lambda: 'New claude here' in text(viewer), 'the ! panel')
        proof.append(launch(viewer, [b'c'], 'claude', folder=here.parent))
        # (A detach returns to the hub; Files reopens in the folder it last showed.)
        viewer.write(b't')
        viewer.read(.3)
        viewer.write(b'f')
        wait(viewer, lambda: 'NOR' in text(viewer), 'files again')
        head = viewer.screen.display[0] + viewer.screen.display[1]
        shown = here if '~/src/robot-arm' in head else here.parent
        viewer.write(b'!')
        wait(viewer, lambda: 'New agy here' in text(viewer), 'the ! panel lists agy')
        proof.append(launch(viewer, [b'a'], agy, folder=shown))
        # 6. The mode survives another viewer closing last, and a new viewer.
        for v in (viewer, other):
            closed = v.close()
            assert closed['clean_exit'], closed
        other = None
        viewer = start()
        proof.append(launch(viewer, [b'x'], f"codex {YOLO['codex']}"))
        print('PASS launch_keys_pty: ' + '; '.join(proof))
    finally:
        for v in (viewer, other):
            if v:
                v.close()
        result = env.cleanup()
        if result['remaining'] or result['orphans_ours_stray']:
            raise AssertionError(f'cleanup: {result}')


if __name__ == '__main__':
    main()
