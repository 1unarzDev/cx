#!/usr/bin/env python3
"""Private fake-helper UI acceptance: real p pin, restart, saved folder cache.

Root executes --run-pty. No live SSH, tmux server, agent, or Docker is required.
Only synthetic workspace screen cells are inspected; none are printed/saved.
"""
import argparse
import fcntl
import io
import json
import os
from pathlib import Path
import pty
import select
import struct
import subprocess
import sys
import termios
import time
import unittest
from unittest.mock import patch
from types import SimpleNamespace

sys.dont_write_bytecode = True
from session_adversarial import Fixture, Failure, require


HELPER = r'''
import json,sys,time
from pathlib import Path
root=Path(__file__).resolve().parent.parent
state=root/'remote-state.json'
log=root/'remote-ops.jsonl'
def record(op,phase):
    with log.open('a') as f: f.write(json.dumps({'op':op,'phase':phase})+'\n')
if sys.argv[-1] != 'exec ~/.local/bin/cx helper':
    record('non-helper-command','rejected')
    sys.exit(97)
while True:
    header=sys.stdin.buffer.readline(64)
    if not header: break
    if not header.startswith(b'CX1 '): sys.exit(97)
    size=int(header[4:])
    if not 0<size<=1048576: sys.exit(97)
    request=json.loads(sys.stdin.buffer.read(size))
    operation=request['op']
    op=operation.get('op') if isinstance(operation,dict) else operation
    mode=json.loads(state.read_text())
    record(op,'received')
    result=None; error=None
    if op=='sessions': result=mode['sessions']
    elif op=='transfer_jobs': result={'jobs':[]}
    elif op in ('list','list_page'):
        if mode['hold_listing']:
            end=time.monotonic()+6
            while not (root/'release-listing').exists() and time.monotonic()<end: time.sleep(.02)
            error='fixture listing unavailable'
        else: result=mode['listing']
    elif op=='info': error='fixture capability observation unavailable'
    else: error='fixture rejects non-observational operation'
    reply=json.dumps({'version':1,'id':request['id'],'result':result,'error':error}).encode()
    record(op,'responded')
    sys.stdout.buffer.write(b'CX1 '+str(len(reply)).encode()+b'\n'+reply)
    sys.stdout.buffer.flush()
'''


def restart_state():
    return dict(schema=1, expires_at=int(time.time()) + 300,
                device_ids=['local', 'remote-fixture'], device=2,
                focus='Workspace', view='Work', selected=0, selected_session=None,
                side_selected=0, search='', browser=None, other_browser=None,
                destination_active=False, conflict=2, launch_provider=None,
                clipboard=None, submitted={})


def fixture_session():
    return dict(id='$0', name='Pinned synthetic work', directory='/fixture-project',
                provider='shell', host='fixture-host', account='fixture', pid=4242,
                started='100:200', boot_id='fixture-boot', external=True,
                socket=None, launcher=None, process=None, container=None,
                terminal=dict(server_pid=4240, server_start='190', session_id='$0', created='100'))


def fixture_listing():
    # files::list serializes Identity as JSON inside the wire string field.
    identity = json.dumps(dict(size=7, modified=123456789, device=1, inode=42))
    return dict(path='/fixture-project', display_path='/fixture-project', parent='/',
        entries=[dict(name='cached-proof.txt', path='/fixture-project/cached-proof.txt',
                      kind='file', size=7, identity=identity, hidden=False)])


def read_operations(f):
    path = f.root / 'remote-ops.jsonl'
    if not path.exists():
        return []
    # Concurrent writer may have an incomplete final line; ignore it until next poll.
    data = path.read_bytes()
    require(len(data) <= 65536, 'fixture operation log exceeded bound')
    return [json.loads(line) for line in data.split(b'\n')[:-1] if line]


class Viewer:
    def __init__(self, f):
        import pyte
        self.f = f
        name = 'viewer-restart-123-789.json'
        snapshot = f.state / name
        snapshot.write_text(json.dumps(restart_state()))
        snapshot.chmod(0o600)
        self.master, self.slave = pty.openpty()
        self.before = termios.tcgetattr(self.slave)
        fcntl.ioctl(self.slave, termios.TIOCSWINSZ, struct.pack('HHHH', 30, 120, 0, 0))
        self.screen = pyte.Screen(120, 30)
        self.stream = pyte.Stream(self.screen)

        def control():
            os.setsid()
            fcntl.ioctl(self.slave, termios.TIOCSCTTY, 0)

        self.process = subprocess.Popen([f.binary, 'restart', name], env=f.env, cwd=f.root,
            stdin=self.slave, stdout=self.slave, stderr=self.slave, preexec_fn=control)

    def pump(self, seconds=.08):
        end = time.monotonic() + seconds
        while time.monotonic() < end:
            if select.select([self.master], [], [], .02)[0]:
                try:
                    self.stream.feed(os.read(self.master, 65536).decode(errors='replace'))
                except OSError:
                    break

    def text(self):
        return '\n'.join(self.screen.display)

    def wait(self, predicate, label, seconds=8):
        end = time.monotonic() + seconds
        while time.monotonic() < end:
            self.pump()
            if predicate():
                return
            require(self.process.poll() is None, 'viewer exited while awaiting ' + label)
        raise Failure(label)

    def send(self, data):
        os.write(self.master, data)
        self.pump()

    def quit(self):
        self.send(b'\x03')
        end = time.monotonic() + 8
        while self.process.poll() is None and time.monotonic() < end:
            self.pump()
        require(self.process.poll() == 0, 'viewer did not exit cleanly')
        require(termios.tcgetattr(self.slave) == self.before, 'viewer did not restore termios')

    def close(self):
        if self.process.poll() is None:
            self.process.terminate()
            try:
                self.process.wait(timeout=3)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait(timeout=3)
        os.close(self.master)
        os.close(self.slave)


def exercise(f):
    f.env.update(HTTPS_PROXY='http://127.0.0.1:9', HTTP_PROXY='http://127.0.0.1:9',
                 ALL_PROXY='http://127.0.0.1:9')
    f.write_tool('ssh', '#!' + sys.executable + '\n' + HELPER)
    devices = [dict(id='local', name='local-fixture', target=None, account='fixture',
                    host='local-fixture', status='unknown', observed_at=0),
               dict(id='remote-fixture', name='remote-fixture', target='cx-fixture.invalid',
                    account='fixture', host='fixture-host', status='unknown', observed_at=0)]
    (f.state / 'devices.json').write_text(json.dumps(devices))
    session = fixture_session()
    listing = fixture_listing()
    state = f.root / 'remote-state.json'
    state.write_text(json.dumps(dict(sessions=[session], listing=listing, hold_listing=False)))
    viewer = Viewer(f)
    try:
        viewer.wait(lambda: 'Pinned synthetic work' in viewer.text(), 'live fixture session not displayed')
        viewer.send(b'p')
        pins = f.state / 'session-pins.json'
        viewer.wait(pins.exists, 'p key did not persist pin')
        saved = json.loads(pins.read_text())
        require(len(saved) == 1 and saved[0]['session']['id'] == '$0', 'pin has wrong runtime')
        require(saved[0]['device']['id'] == 'remote-fixture', 'pin has wrong device')
        require(pins.stat().st_mode & 0o777 == 0o600, 'pin file permissions are not private')
        viewer.send(b'\x10Files\r')
        viewer.wait(lambda: 'Files · choose device' in viewer.text(),
                    'Files palette did not open its mandatory device chooser')
        require(not any(r['op'] in ('list', 'list_page') for r in read_operations(f)),
                'Files palette loaded a device before explicit confirmation')
        # execute_palette selects actual_device(), which is the saved remote filter.
        viewer.send(b'\r')
        viewer.wait(lambda: 'cached-proof.txt' in viewer.text(), 'saved-folder source listing did not display')
        viewer.quit()
    finally:
        viewer.close()
    cache = f.state / 'ui-navigation.json'
    require(cache.exists(), 'normal quit did not persist navigation cache')
    cached = json.loads(cache.read_text())
    remote = next(d for d in cached['devices'] if d['device']['id'] == 'remote-fixture')
    require(any(b['path'] == '/fixture-project' and b['entries'] for b in remote['browsers']),
            'navigation cache lost observed remote folder')

    state.write_text(json.dumps(dict(sessions=[], listing=listing, hold_listing=True)))
    (f.root / 'remote-ops.jsonl').write_text('')
    viewer = Viewer(f)
    try:
        viewer.wait(lambda: 'Pinned synthetic work' in viewer.text() and 'not seen' in viewer.text(),
                    'absent runtime pin did not survive viewer restart')
        require(len(json.loads(pins.read_text())) == 1, 'viewer restart duplicated or removed pin')
        viewer.send(b'\r')
        viewer.wait(lambda: 'Open saved folder' in viewer.text(), 'absent pin did not offer saved folder')
        start = time.monotonic()
        viewer.send(b'\r')
        viewer.wait(lambda: 'cached-proof.txt' in viewer.text(), 'cached folder not visible before remote reply', seconds=2)
        elapsed = time.monotonic() - start
        viewer.wait(lambda: any(r == {'op': 'list', 'phase': 'received'} for r in read_operations(f)),
                    'saved folder did not request fresh listing')
        require(not any(r == {'op': 'list', 'phase': 'responded'} for r in read_operations(f)),
                'cache visibility was established only after a remote reply')
        # Restored navigation is not destructive authorization, before OR after error.
        for key in (b'd', b'r', b'x'):
            viewer.send(key)
            require('Delete 1 item?' not in viewer.text() and 'Focus: Rename' not in viewer.text()
                    and 'CUT 1' not in viewer.text(), 'cached entry permitted destructive action')
        (f.root / 'release-listing').touch()
        viewer.wait(lambda: any(r == {'op': 'list', 'phase': 'responded'} for r in read_operations(f)),
                    'failed listing response did not complete')
        viewer.pump(.3)
        require('cached-proof.txt' in viewer.text(), 'failed refresh removed cached navigation')
        for key in (b'd', b'r', b'x'):
            viewer.send(key)
            require('Delete 1 item?' not in viewer.text() and 'Focus: Rename' not in viewer.text()
                    and 'CUT 1' not in viewer.text(), 'failed refresh authorized cached destructive action')
        viewer.quit()
        allowed = {'sessions', 'info', 'list', 'list_page', 'transfer_jobs', 'containers'}
        require(all(r['op'] in allowed for r in read_operations(f)),
                'saved-folder/cache flow submitted unexpected operations: ' + str(sorted({r['op'] for r in read_operations(f)} - allowed)))
        require(len(json.loads(pins.read_text())) == 1, 'absent bookmark not retained on second quit')
        return dict(cached_folder_visible_ms=round(elapsed * 1000, 3),
                    checks=['p persisted exact remote runtime', 'pin survived restart and absence',
                            'saved folder displayed cache before remote reply',
                            'failed refresh retained rows without mutation authority',
                            'no Create or mutation operation', 'termios restored twice'])
    finally:
        (f.root / 'release-listing').touch(exist_ok=True)
        viewer.close()


class PureTests(unittest.TestCase):
    def test_helper_compiles_without_execution(self):
        compile(HELPER, '<fake-helper>', 'exec')

    def test_restart_uses_explicit_private_device(self):
        saved = restart_state()
        self.assertEqual(saved['device_ids'][saved['device'] - 1], 'remote-fixture')
        self.assertIsNone(saved['clipboard'])

    def test_session_metadata_is_synthetic(self):
        row = fixture_session()
        self.assertEqual(row['terminal']['session_id'], row['id'])
        self.assertEqual(row['directory'], '/fixture-project')

    def test_listing_identity_matches_serialized_wire_shape(self):
        identity = fixture_listing()['entries'][0]['identity']
        self.assertIsInstance(identity, str)
        self.assertEqual(json.loads(identity), dict(size=7, modified=123456789, device=1, inode=42))

    def test_helper_frame_round_trip_without_subprocess(self):
        fixture = Fixture('/does-not-exist', live=False)
        try:
            (fixture.root / 'remote-state.json').write_text(json.dumps(
                dict(sessions=[fixture_session()], hold_listing=False, listing={})))
            request = json.dumps(dict(version=1, id='wire-test', op=dict(op='sessions'))).encode()
            source = io.BytesIO(b'CX1 ' + str(len(request)).encode() + b'\n' + request)
            output = io.BytesIO()
            with patch.object(sys, 'stdin', SimpleNamespace(buffer=source)), \
                 patch.object(sys, 'stdout', SimpleNamespace(buffer=output)), \
                 patch.object(sys, 'argv', ['ssh', 'exec ~/.local/bin/cx helper']):
                exec(compile(HELPER, '<fake-helper>', 'exec'),
                     {'__file__': str(fixture.bin / 'ssh')})
            header, body = output.getvalue().split(b'\n', 1)
            self.assertEqual(header, b'CX1 ' + str(len(body)).encode())
            self.assertEqual(json.loads(body)['result'][0]['id'], '$0')
            self.assertEqual(read_operations(fixture), [dict(op='sessions', phase='received'),
                                                         dict(op='sessions', phase='responded')])
        finally:
            fixture.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('binary', nargs='?')
    modes = parser.add_mutually_exclusive_group(required=True)
    modes.add_argument('--self-test', action='store_true')
    modes.add_argument('--run-pty', action='store_true')
    args = parser.parse_args()
    if args.self_test:
        result = unittest.TextTestRunner().run(unittest.defaultTestLoader.loadTestsFromTestCase(PureTests))
        return int(not result.wasSuccessful())
    if not args.binary:
        parser.error('binary required')
    fixture = None
    report = dict(case='pin-cache-viewer-restart', status='PASS')
    try:
        fixture = Fixture(args.binary, live=False)
        report.update(exercise(fixture))
    except Exception as error:
        report.update(status='FAIL', reason=str(error) if isinstance(error, Failure) else type(error).__name__)
    finally:
        if fixture:
            fixture.close()
    print(json.dumps(report))
    return int(report['status'] != 'PASS')


if __name__ == '__main__':
    sys.exit(main())
