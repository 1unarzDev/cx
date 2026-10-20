#!/usr/bin/env python3
"""Command panel helper layer: styled capture, input, exit status, hidden pane."""
import json, os, subprocess, sys, tempfile, time

binary = sys.argv[1]
with tempfile.TemporaryDirectory(prefix='cx-panel-') as tmp:
    env = dict(os.environ, HOME=tmp, XDG_STATE_HOME=f'{tmp}/state', TMUX_TMPDIR=tmp, SHELL='/bin/bash')
    for k in ('TMUX', 'TMUX_PANE'):
        env.pop(k, None)
    socket = f'{tmp}/state/cx/managed.sock'

    def helper(op):
        req = json.dumps(dict(version=1, id='p', op=op)).encode()
        wire = b'CX1 ' + str(len(req)).encode() + b'\n' + req
        r = subprocess.run([binary, 'helper'], input=wire, env=env, capture_output=True, timeout=30)
        assert r.returncode == 0, r.stderr.decode()
        return json.loads(r.stdout.split(b'\n', 1)[1])

    try:
        # 1. A colored command that then reads input and exits with a code.
        start = helper(dict(op='command_panel', args=dict(
            request=dict(directory=tmp, command='printf "\\033[32mhello\\033[0m\\n"; read x; echo "got:$x"; exit 3'),
            cols=60, rows=10)))
        pane = start['result']['id']
        assert pane.startswith('cxcmd-'), start
        time.sleep(0.4)
        cap = helper(dict(op='capture_pane', args=dict(id=pane, cols=60, rows=10)))['result']
        assert cap['alive'] and any('hello' in l for l in cap['lines']), cap
        assert any('\x1b[32m' in l or '[32m' in l for l in cap['lines']), cap['lines']
        # not in the ordinary session list
        sessions = helper(dict(op='sessions'))['result']
        assert not any(s['id'] == pane for s in sessions), sessions
        # 2. Send "world\n" as hex and read echoed result + exit status.
        hexkeys = ''.join(f'{b:02x}' for b in b'world\n')
        assert helper(dict(op='pane_input', args=dict(id=pane, hex=hexkeys)))['result']['delivered']
        time.sleep(0.5)
        cap = helper(dict(op='capture_pane', args=dict(id=pane, cols=60, rows=10)))['result']
        assert any('got:world' in l for l in cap['lines']), cap
        assert cap['alive'] is False and cap['status'] == 3, cap
        # input to a dead pane is a no-op
        assert helper(dict(op='pane_input', args=dict(id=pane, hex='61')))['result']['delivered'] is False
        # 3. Close it.
        assert helper(dict(op='pane_close', args=dict(id=pane)))['result']['closed']
        time.sleep(0.2)
        assert subprocess.run(['tmux', '-f', '/dev/null', '-S', socket, 'has-session', '-t', '=' + pane],
                              env=env, capture_output=True).returncode != 0
        # 4. Bad inputs are refused.
        assert helper(dict(op='capture_pane', args=dict(id='cxcmd-xx', cols=10, rows=4)))['error']
        assert helper(dict(op='pane_input', args=dict(id=pane, hex='zz')))['error']
        print(json.dumps(dict(result='PASS')))
    finally:
        subprocess.run(['tmux', '-f', '/dev/null', '-S', socket, 'kill-server'], env=env,
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
