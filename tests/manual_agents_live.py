#!/usr/bin/env python3
"""Actual disposable tmux foreground/background detection; synthetic agent executables."""
import json, os, pathlib, shutil, subprocess, sys, tempfile, time
binary = str(pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else 'target/release/cx').resolve())
with tempfile.TemporaryDirectory(prefix='cx-manual-agent-') as directory:
    root = pathlib.Path(directory)
    env = dict(os.environ, HOME=str(root), XDG_STATE_HOME=str(root/'state'), SHELL='/bin/bash')
    env.pop('TMUX', None); env.pop('TMUX_PANE', None)
    socket = str(root/'state/cx/managed.sock')
    for provider in ['claude', 'codex']:
        shutil.copy2(shutil.which('sleep'), root/provider)
    original = json.loads(subprocess.check_output([binary, 'new', '--provider', 'shell', '--directory', str(root), '--key', 'manual-detection'], env=env, text=True))
    def run(*args): return subprocess.check_output(['tmux', '-S', socket, *args], text=True)
    def send(text): run('send-keys', '-l', text); run('send-keys', 'Enter')
    def current(): return next(s for s in json.loads(subprocess.check_output([binary, 'sessions'], env=env, text=True)) if s['id'] == original['id'])
    def expect(provider):
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline:
            value = current()
            if value['provider'] == provider:
                for key in ['id', 'pid', 'started', 'boot_id', 'socket']: assert value[key] == original[key], key
                return value
            time.sleep(.1)
        raise AssertionError('provider did not become ' + provider)
    try:
        for provider in ['codex', 'claude']:
            send('./' + provider + ' 30')
            value = expect(provider)
            assert value['process']['pid'] != original['pid']
            run('send-keys', 'C-c'); expect('shell')
        send('./codex 30 &')
        time.sleep(.2); expect('shell')
        # An unrelated program with a spoofed argv[0] is not a provider.
        send("exec -a codex sleep 30")
        time.sleep(.2); expect('shell')
        print(json.dumps({'result':'PASS','checks':['manual Codex/Claude foreground discovery','return to shell','background excluded','spoofed argv0 excluded','same terminal id/pid/start/boot/socket'], 'agents':'synthetic sleep binaries named codex/claude; no inference'}))
    finally:
        subprocess.run(['tmux', '-S', socket, 'kill-server'], capture_output=True)
