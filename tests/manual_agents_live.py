#!/usr/bin/env python3
"""Actual disposable tmux foreground/background detection; synthetic agent executables."""
import fcntl, json, os, pathlib, pty, select, shutil, struct, subprocess, sys, tempfile, termios, time
binary = str(pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else 'target/release/cx').resolve())
with tempfile.TemporaryDirectory(prefix='cx-manual-agent-') as directory:
    root = pathlib.Path(directory)
    env = dict(os.environ, HOME=str(root), XDG_STATE_HOME=str(root/'state'), SHELL='/bin/bash')
    env.pop('TMUX', None); env.pop('TMUX_PANE', None)
    socket = str(root/'state/cx/managed.sock')
    for provider in ['claude', 'codex']:
        shutil.copy2(shutil.which('bash'), root/provider)
    original = json.loads(subprocess.check_output([binary, 'new', '--provider', 'shell', '--directory', str(root), '--key', 'manual-detection'], env=env, universal_newlines=True))
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 24, 80, 0, 0))
    env['TERM'] = 'xterm-256color'
    def tty():
        os.setsid(); fcntl.ioctl(slave, termios.TIOCSCTTY, 0)
    client = subprocess.Popen(['tmux', '-u', '-S', socket, 'attach-session', '-t', original['id']], env=env, stdin=slave, stdout=slave, stderr=slave, preexec_fn=tty)
    terminal_output = bytearray()
    def drain():
        while select.select([master], [], [], 0)[0]:
            terminal_output.extend(os.read(master, 65536))
    time.sleep(.2); drain()
    def run(*args): return subprocess.check_output(['tmux', '-S', socket, *args], universal_newlines=True)
    def send(text): os.write(master, text.encode() + b'\r')
    def current(): return next(s for s in json.loads(subprocess.check_output([binary, 'sessions'], env=env, universal_newlines=True)) if s['id'] == original['id'])
    def expect(provider):
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline:
            drain()
            value = current()
            if value['provider'] == provider:
                for key in ['id', 'pid', 'started', 'boot_id', 'socket']: assert value[key] == original[key], key
                return value
            time.sleep(.1)
        raise AssertionError('provider did not become ' + provider + ': ' + run('capture-pane', '-p') + ' metadata=' + json.dumps(current()) + ' client=' + str(client.poll()) + ' output=' + repr(bytes(terminal_output)[-1500:]))
    try:
        for provider in ['codex', 'claude']:
            send('./' + provider + " -c 'sleep 30; :'")
            value = expect(provider)
            assert value['process']['pid'] != original['pid']
            os.write(master, b'\x03'); expect('shell')
        send("./codex -c 'sleep 30; :' &")
        time.sleep(.2); expect('shell')
        # An unrelated program with a spoofed argv[0] is not a provider.
        send("exec -a codex sleep 30")
        time.sleep(.2); expect('shell')
        print(json.dumps({'result':'PASS','checks':['manual Codex/Claude foreground discovery','return to shell','background excluded','spoofed argv0 excluded','same terminal id/pid/start/boot/socket'], 'agents':'synthetic Bash binaries named codex/claude; no inference'}))
    finally:
        if client.poll() is None:
            client.terminate(); client.wait(timeout=5)
        os.close(master); os.close(slave)
        subprocess.run(['tmux', '-S', socket, 'kill-server'], stdout=subprocess.PIPE, stderr=subprocess.PIPE)
