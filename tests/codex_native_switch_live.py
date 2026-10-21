#!/usr/bin/env python3
"""Actual Codex idle project switching; private config and a rejecting loopback API.

Requires installed Codex (tested 0.163.0), tmux and sqlite3. No real credentials,
provider configuration, conversations or model responses are used.
"""
import argparse
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import queue
import signal
import shutil
import subprocess
import tempfile
import threading
import time


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('binary', type=Path)
    parser.add_argument('--artifacts', type=Path)
    args = parser.parse_args()
    binary = str(args.binary.resolve())
    codex = shutil.which('codex')
    if not codex:
        parser.error('installed Codex CLI required')
    requests = []

    class RejectInference(BaseHTTPRequestHandler):
        def log_message(self, *_):
            pass

        def do_POST(self):
            size = int(self.headers.get('Content-Length', '0'))
            if size > 10 * 1024 * 1024:
                self.send_error(413)
                return
            self.rfile.read(size)  # Discard the synthetic request, never log it.
            requests.append(1)
            body = b'{"error":{"message":"Synthetic fixture rejects inference","type":"invalid_request_error","code":"fixture"}}'
            self.send_response(400)
            self.send_header('Content-Length', str(len(body)))
            self.end_headers()
            self.wfile.write(body)

    server = ThreadingHTTPServer(('127.0.0.1', 0), RejectInference)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    try:
        with tempfile.TemporaryDirectory(prefix='cx-native-switch-') as tmp:
            root = Path(tmp)
            home = root/'home'
            native = home/'.codex'
            native.mkdir(parents=True)
            first, second = root/'first', root/'second'
            first.mkdir()
            second.mkdir()
            env = {key: os.environ[key] for key in ['PATH', 'LANG'] if key in os.environ}
            env.update(HOME=str(home), CODEX_HOME=str(native),
                XDG_STATE_HOME=str(root/'state'), XDG_CONFIG_HOME=str(root/'config'),
                TERM='xterm-256color', COLORTERM='truecolor', SHELL='/bin/bash',
                CX_SYNTHETIC_KEY='dummy')
            (native/'config.toml').write_text(
                f'model="gpt-5.4"\nmodel_provider="fixture"\napproval_policy="never"\n'
                f'sandbox_mode="read-only"\n[model_providers.fixture]\nname="fixture"\n'
                f'base_url="http://127.0.0.1:{server.server_port}/v1"\n'
                'env_key="CX_SYNTHETIC_KEY"\nwire_api="responses"\n'
                'request_max_retries=0\nstream_max_retries=0\n'
                + ''.join(f'[projects.{json.dumps(str(folder))}]\ntrust_level="trusted"\n'
                          for folder in [first, second]))
            socket = root/'state/cx/managed.sock'
            socket.parent.mkdir(parents=True, mode=0o700)
            # Prestart only this owned socket, avoiding user-service environment.
            subprocess.run(['tmux', '-S', str(socket), '-f', '/dev/null', 'new-session',
                '-d', '-s', 'bootstrap', 'sleep 60'], env=env, check=True)
            app = None
            sid = None
            log = open(root/'appserver.log', 'w')

            def tmux(*command):
                return subprocess.check_output(['tmux', '-S', str(socket), *command],
                    env=env, text=True, stderr=subprocess.PIPE, timeout=5)

            def capture(label):
                pane = tmux('capture-pane', '-p', '-t', sid)
                if args.artifacts:
                    args.artifacts.mkdir(parents=True, exist_ok=True)
                    (args.artifacts/(label+'.txt')).write_text(pane)
                return pane

            def send(text):
                tmux('send-keys', '-t', sid, '-l', text)
                time.sleep(.4)  # Respect native paste burst detection before Enter.
                tmux('send-keys', '-t', sid, 'Enter')

            def current():
                rows = json.loads(subprocess.check_output([binary, 'sessions'], env=env,
                                                         text=True, timeout=20))
                return next(row for row in rows if row['id']==sid)

            def expect(check, message):
                deadline = time.monotonic()+10
                while time.monotonic()<deadline:
                    row = current()
                    if check(row):
                        return row
                    time.sleep(.1)
                capture('failure')
                raise AssertionError(message)

            try:
                app = subprocess.Popen([codex, 'app-server'], env=env, stdin=subprocess.PIPE,
                    stdout=subprocess.PIPE, stderr=log, text=True, bufsize=1, start_new_session=True)
                replies = queue.Queue()

                def read_replies():
                    for line in app.stdout:
                        replies.put(json.loads(line))

                threading.Thread(target=read_replies, daemon=True).start()
                rpc_id = 0

                def rpc(method, params):
                    nonlocal rpc_id
                    rpc_id += 1
                    app.stdin.write(json.dumps(dict(id=rpc_id, method=method, params=params))+'\n')
                    app.stdin.flush()
                    deadline = time.monotonic()+15
                    while time.monotonic()<deadline:
                        try:
                            reply = replies.get(timeout=.1)
                        except queue.Empty:
                            continue
                        if reply.get('id')==rpc_id:
                            assert 'error' not in reply, 'native fixture RPC failed: '+method
                            return reply['result']
                    raise AssertionError('native fixture RPC timeout: '+method)

                rpc('initialize', {'clientInfo': {'name': 'cx_fixture', 'version': '1'},
                                   'capabilities': {'experimentalApi': True}})
                app.stdin.write('{"method":"initialized"}\n')
                app.stdin.flush()
                for folder, name in [(second, 'Synthetic second project'),
                                     (first, 'Synthetic first project')]:
                    thread = rpc('thread/start', {'cwd': str(folder), 'modelProvider': 'fixture',
                        'model': 'gpt-5.4', 'approvalPolicy': 'never', 'sandbox': 'read-only'})['thread']['id']
                    rpc('thread/name/set', {'threadId': thread, 'name': name})
                    rpc('turn/start', {'threadId': thread, 'input': [
                        {'type': 'text', 'text': 'Synthetic local fixture; no real inference.'}]})
                    time.sleep(.5)
                app.terminate()
                app.wait(timeout=5)
                seeded_requests = len(requests)
                assert seeded_requests == 2, 'both synthetic fixture turns must hit loopback'
                request = json.dumps(dict(version=1, id='native-switch', op=dict(op='create', args=dict(
                    provider='codex', resume=True, directory=str(first), name='', key='native-switch')))).encode()
                response = subprocess.check_output([binary, 'helper'], env=env,
                    input=b'CX1 '+str(len(request)).encode()+b'\n'+request, timeout=15)
                response = json.loads(response.split(b'\n', 1)[1])
                assert not response.get('error'), response.get('error')
                session = response['result']
                sid = session['id']
                tmux('kill-session', '-t', 'bootstrap')
                tmux('resize-window', '-t', sid, '-x', '110', '-y', '35')
                initial = expect(lambda row: row['name']=='Synthetic first project'
                    and row['directory']==str(first), 'actual initial Codex metadata missing')
                capture('initial')
                native_title = tmux('display-message', '-p', '-t', sid, '#{pane_title}').strip()
                # When the thread cannot be pinned the row still shows a real
                # directory (the launch cwd), never "Directory unavailable", and
                # the name mirrors whatever title Codex itself is displaying.
                tmux('select-pane', '-t', sid, '-T', 'CX awaiting native title')
                expect(lambda row: row['directory']==str(first) and row['name']=='codex',
                       'pending selection should fall back to the launch directory')
                tmux('select-pane', '-t', sid, '-T', 'Synthetic unmatched title')
                expect(lambda row: row['directory']==str(first)
                       and row['name']=='Synthetic unmatched title',
                       'an unpinned thread should mirror the live terminal title and keep a directory')
                tmux('select-pane', '-t', sid, '-T', native_title)
                expect(lambda row: row['directory']==str(first)
                       and row['name']=='Synthetic first project',
                       'restored native title did not recover directory and name')
                send('/resume')
                deadline = time.monotonic()+5
                while 'Resume a previous session' not in capture('picker'):
                    assert time.monotonic()<deadline, 'native resume picker missing'
                    time.sleep(.1)
                tmux('send-keys', '-t', sid, 'Right')
                time.sleep(.3)
                send('Synthetic second project')
                time.sleep(.7)
                choice = capture('directory-choice')
                assert 'Use session directory' in choice, 'native directory choice missing'
                tmux('send-keys', '-t', sid, 'Enter')
                # See --artifacts on failure for version-sensitive native UI evidence.
                switched = expect(lambda row: row['name']=='Synthetic second project'
                    and row['directory']==str(second), 'idle native project switch kept old metadata')
                assert all(initial[key]==switched[key] for key in
                    ['id', 'pid', 'started', 'boot_id', 'socket']), 'switch replaced terminal identity'
                assert len(requests)==seeded_requests, 'idle switch unexpectedly submitted a turn'
                print(json.dumps({'result': 'PASS', 'checks': [
                    'actual production Codex launch uses isolated process/title tracking',
                    'idle native resume picker updates exact name and selected directory',
                    'an unpinned thread shows the launch directory and the live title, never "unavailable"',
                    'terminal identity preserved', 'no inference on idle switch']}))
            finally:
                if app is not None and app.poll() is None:
                    app.terminate()
                    app.wait(timeout=5)
                subprocess.run(['tmux', '-S', str(socket), 'kill-server'], env=env,
                    stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
                if app is not None:
                    try:
                        os.killpg(app.pid, signal.SIGTERM)
                    except ProcessLookupError:
                        pass
                log.close()
    finally:
        server.shutdown()
        server.server_close()


if __name__ == '__main__':
    main()
