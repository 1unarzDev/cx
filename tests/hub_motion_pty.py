#!/usr/bin/env python3
"""The hub map's arrows and animation, in the real viewer (v0.2.2 nebula).

usage: hub_motion_pty.py BINARY

A private HOME holding this device and five offline peers (a fake `ssh` that fails at
once; nothing is contacted); the viewer runs on a 120x40 PTY.
  * Idle: once settled, the hub writes nothing at all (0 bytes over 1.5 s).
  * An arrow moves the selection at once (the vitals strip names the new device in the
    first frame after the key), the transition draws a few frames (<= 60 fps) and stops:
    0 bytes once 400 ms have passed.
  * Left then Right (and Right then Left) return to the same device; every peer is
    reached by the arrows from this device.
  * CX_REDUCED_MOTION=1: a key draws exactly what it changes, in one frame burst.
"""
import json, os, pathlib, sys, time

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import perf_harness as ph  # noqa: E402
import pty_lib  # noqa: E402

COLS, ROWS = 120, 40
PEERS = ['alder', 'birch', 'cedar', 'dogwood', 'elm']
KEYS = {'left': b'\x1b[D', 'right': b'\x1b[C', 'up': b'\x1b[A', 'down': b'\x1b[B'}
BACK = {'left': 'right', 'right': 'left', 'up': 'down', 'down': 'up'}


def start(env_extra=None):
    env = ph.Env(sys.argv[1])
    fake = env.root / 'fakebin'
    fake.mkdir()
    ssh = fake / 'ssh'
    ssh.write_text('#!/bin/sh\nexit 255\n')
    ssh.chmod(0o700)
    devices = [dict(id='local', name='home-box', target=None, account='tester',
                    host='home-box', status='unknown', observed_at=0)]
    devices += [dict(id=p, name=p, target=f'{p}-fixture', account='tester', host=p,
                     status='unknown', observed_at=0) for p in PEERS]
    (env.state / 'devices.json').write_text(json.dumps(devices))
    env.env['PATH'] = f"{fake}:{env.env.get('PATH', '/usr/bin:/bin')}"
    env.env.pop('CX_REDUCED_MOTION', None)
    env.env.update(env_extra or {})
    viewer = pty_lib.Pty([str(pathlib.Path(sys.argv[1]).resolve())], env.env, env.root, COLS, ROWS)
    return env, viewer


def window(viewer, seconds):
    """Bytes and bursts (chunks more than 3 ms apart) written in `seconds`."""
    end = time.monotonic() + seconds
    total, bursts, last = 0, 0, 0.0
    while time.monotonic() < end:
        data = viewer.read(max(0.0, end - time.monotonic()))
        if data:
            now = time.monotonic()
            if now - last > 0.003:
                bursts += 1
            last = now
            total += len(data)
    return total, bursts


def settle(viewer, quiet=0.5, limit=15.0):
    end = time.monotonic() + limit
    while time.monotonic() < end:
        if window(viewer, quiet)[0] == 0:
            return
    raise AssertionError('never settled:\n' + viewer.text())


def selected(viewer):
    """The device the vitals strip (two rows and a blank above SESSIONS) names."""
    lines = viewer.screen.display
    at = next((i for i, l in enumerate(lines) if l.strip().startswith('SESSIONS')), None)
    if at is None or at < 3:
        return None
    words = lines[at - 3].split()
    return words[0] if words and words[0] in ['home-box'] + PEERS else None


def press(viewer, key):
    viewer.write(KEYS[key])
    window(viewer, 0.05)
    return selected(viewer)


def main():
    env, viewer = start()
    try:
        end = time.monotonic() + 10
        while not all(p in viewer.text() for p in PEERS):
            assert time.monotonic() < end, 'peers never drawn:\n' + viewer.text()
            viewer.read(0.1)
        settle(viewer)
        idle, _ = window(viewer, 1.5)
        assert idle == 0, f'idle hub wrote {idle} bytes'
        assert selected(viewer) == 'home-box', viewer.text()
        # Every peer is reached by the arrows, breadth first from this device (Shift+H
        # returns to it), and every move comes back.
        def goto(keys):
            viewer.write(b'H')
            window(viewer, 0.1)
            for k in keys:
                press(viewer, k)
        seen, todo, path = {'home-box'}, ['home-box'], {'home-box': []}
        while todo:
            at = todo.pop(0)
            for key in KEYS:
                goto(path[at])
                assert selected(viewer) == at, f'path to {at} landed on {selected(viewer)}'
                now = press(viewer, key)
                if now in (None, at):
                    continue
                got = press(viewer, BACK[key])
                if got != at:
                    # One way: back leads to a node that leads here again.
                    again = press(viewer, key)
                    assert again == now, f'{at} -{key}-> {now} -back-> {got} -{key}-> {again}'
                if now not in seen:
                    seen.add(now)
                    path[now] = path[at] + [key]
                    todo.append(now)
        goto([])
        assert seen == {'home-box', *PEERS}, f'arrows never reached {set(PEERS) - seen}'
        # One move: immediate, a short animation, then silence.
        settle(viewer)
        before = selected(viewer)
        key = next(k for k in ('right', 'left', 'up', 'down')
                   if press(viewer, k) not in (None, before))
        settle(viewer)
        press(viewer, BACK[key])
        settle(viewer)
        viewer.write(KEYS[key])
        t0 = time.monotonic()
        while selected(viewer) in (None, before):
            assert time.monotonic() - t0 < 0.3, 'the key waited for the animation'
            viewer.read(0.01)
        early = time.monotonic() - t0
        moving, bursts = window(viewer, 0.35)
        after, _ = window(viewer, 1.0)
        assert after == 0, f'still drawing after the transition: {after} bytes'
        assert bursts <= 0.35 * 62 + 2, f'{bursts} frames in 350 ms'
        print(f'hub_motion: key->frame {early * 1000:.0f} ms, transition {bursts} bursts '
              f'{moving} bytes, idle 0 bytes')
    finally:
        viewer.close()
        env.cleanup()
    # Reduced motion: the same key, one burst.
    env, viewer = start({'CX_REDUCED_MOTION': '1'})
    try:
        end = time.monotonic() + 10
        while not all(p in viewer.text() for p in PEERS):
            assert time.monotonic() < end, viewer.text()
            viewer.read(0.1)
        settle(viewer)
        before = selected(viewer)
        viewer.write(KEYS[key])
        _, bursts = window(viewer, 0.4)
        assert selected(viewer) != before, viewer.text()
        assert bursts <= 2, f'reduced motion drew {bursts} bursts'
        assert window(viewer, 1.0)[0] == 0
        print(f'hub_motion: reduced motion {bursts} burst(s)')
    finally:
        viewer.close()
        env.cleanup()
    print('hub_motion_pty: ok')


if __name__ == '__main__':
    main()
