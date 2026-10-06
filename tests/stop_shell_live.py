#!/usr/bin/env python3
"""Exact shell-stop wire regression: private tmux socket, no services/real agents."""
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time

binary = str(Path(sys.argv[1]).resolve())
checks = []
with tempfile.TemporaryDirectory(prefix='cx-stop-shell-') as temporary:
    home = Path(temporary)
    root = home / 'state/cx'
    root.mkdir(parents=True, mode=0o700)
    socket = root / 'managed.sock'
    env = dict(os.environ, HOME=str(home), XDG_STATE_HOME=str(home/'state'), SHELL='/bin/bash')
    env.pop('TMUX', None)
    boot = Path('/proc/sys/kernel/random/boot_id').read_text().strip()
    def tmux(*args, check=True):
        return subprocess.run(['tmux', '-f', '/dev/null', '-S', str(socket), *args], capture_output=True,
                              text=True, check=check, timeout=5)
    def create(label):
        name = 'cx-' + hashlib.sha256(label.encode()).hexdigest()
        tmux('new-session', '-d', '-s', name, '-c', str(home), '/bin/bash --noprofile --norc')
        pid = int(tmux('display-message', '-p', '-t', name, '#{pane_pid}').stdout.strip())
        created = tmux('display-message', '-p', '-t', name, '#{session_created}').stdout.strip()
        start = Path(f'/proc/{pid}/stat').read_text().rsplit(')', 1)[1].split()[19]
        record = dict(id=name, name=label, directory=str(home), provider='shell', host='fixture',
                      account='fixture', pid=pid, started=created+':'+start, boot_id=boot,
                      external=False, socket=str(socket), launcher='/bin/bash', process=None)
        (root/(name+'.json')).write_text(json.dumps(record))
        return {k: record[k] for k in ('id', 'pid', 'started', 'boot_id')}
    def stop(args):
        request = json.dumps(dict(version=1, id='fixture-stop', op=dict(op='stop_session', args=args))).encode()
        wire = b'CX1 '+str(len(request)).encode()+b'\n'+request
        result = subprocess.run([binary, 'helper'], input=wire, env=env, capture_output=True, timeout=5)
        assert result.returncode == 0, result.stderr.decode()
        header, data = result.stdout.split(b'\n', 1)
        assert header == b'CX1 '+str(len(data)).encode(), result.stdout
        return json.loads(data)
    def alive(args):
        return tmux('has-session', '-t', '='+args['id'], check=False).returncode == 0
    try:
        victim = create('victim')
        guard = create('protected-equivalent')
        for field, value in [('id', '$0'), ('id', victim['id'][:12]), ('pid', victim['pid']+1),
                             ('started', victim['started']+'0'), ('boot_id', 'different-boot')]:
            bad = dict(victim, **{field:value})
            assert stop(bad)['error'] and alive(victim) and alive(guard)
        checks.append('external/prefix/pid/start/boot mismatches rejected without mutation')
        missing = dict(victim, id='cx-'+'f'*64)
        assert stop(missing)['error']
        checks.append('missing ownership record rejected')
        tmux('split-window', '-d', '-t', victim['id'], '/bin/bash --noprofile --norc')
        assert stop(victim)['error'] and alive(victim)
        tmux('kill-pane', '-t', victim['id']+':0.1')
        checks.append('additional panes protected')
        codex = home/'codex'
        shutil.copyfile('/bin/sleep', codex)
        codex.chmod(0o700)
        tmux('send-keys', '-t', victim['id'], str(codex)+' 20 &', 'Enter')
        deadline = time.monotonic()+3
        child = None
        while time.monotonic()<deadline:
            children = Path(f'/proc/{victim["pid"]}/task/{victim["pid"]}/children').read_text().split()
            for candidate in children:
                try:
                    if Path(f'/proc/{candidate}/exe').resolve() == codex:
                        child = int(candidate)
                        break
                except OSError:
                    pass
            if child:
                break
            time.sleep(.03)
        assert child, 'synthetic provider did not start'
        assert stop(victim)['error'] and alive(victim)
        checks.append('background provider takeover rejected')
        # Only our synthetic child is stopped, never a real agent.
        os.kill(child, 15)
        deadline = time.monotonic()+3
        while Path(f'/proc/{child}').exists() and time.monotonic()<deadline:
            time.sleep(.03)
        tmux('send-keys', '-t', victim['id'], str(codex)+' 20', 'Enter')
        deadline = time.monotonic()+3
        child = None
        while time.monotonic()<deadline:
            children = Path(f'/proc/{victim["pid"]}/task/{victim["pid"]}/children').read_text().split()
            for candidate in children:
                try:
                    if Path(f'/proc/{candidate}/exe').resolve() == codex:
                        child = int(candidate)
                        break
                except OSError:
                    pass
            if child:
                break
            time.sleep(.03)
        assert child, 'foreground synthetic provider did not start'
        assert stop(victim)['error'] and alive(victim)
        checks.append('foreground provider takeover rejected')
        os.kill(child, 15)
        deadline = time.monotonic()+3
        while Path(f'/proc/{child}').exists() and time.monotonic()<deadline:
            time.sleep(.03)
        result = stop(victim)
        assert result['result'] == {'status':'stopped'}, result
        assert not alive(victim) and alive(guard)
        assert stop(victim)['result'] == {'status':'already_stopped'}
        assert stop(dict(victim, boot_id='old-boot'))['error']
        checks.append('exact shell stopped, repeat reconciled, reboot retry rejected, peer preserved')
        # Tombstone must not authorize stopping a reused name/new process.
        tmux('new-session', '-d', '-s', victim['id'], '/bin/bash --noprofile --norc')
        assert stop(victim)['error'] and alive(victim)
        checks.append('reused session name with changed runtime rejected')
    finally:
        tmux('kill-server', check=False)
print(json.dumps(dict(result='PASS', checks=checks, backend='private tmux socket + helper protocol', binary=binary)))
