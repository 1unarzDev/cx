#!/usr/bin/env python3
"""Real managed tmux PTY: wheel scrollback, native mouse forwarding, key ownership.
No provider, transcripts, user config, or inference. Usage: script BINARY.
"""
import fcntl, json, os, pathlib, pty, select, shlex, struct, subprocess, sys, tempfile, termios, time
binary = str(pathlib.Path(sys.argv[1]).resolve())
with tempfile.TemporaryDirectory(prefix='cx-wheel-') as temporary:
    root = pathlib.Path(temporary)
    (root/'.local/bin').mkdir(parents=True)
    (root/'.local/bin/cx').symlink_to(binary)
    env = dict(os.environ, HOME=str(root), XDG_STATE_HOME=str(root/'state'), SHELL='/bin/bash', TERM='xterm-256color')
    env.pop('TMUX', None); env.pop('TMUX_PANE', None)
    socket = str(root/'state/cx/managed.sock')
    session = json.loads(subprocess.check_output([binary, 'new', '--provider', 'shell', '--directory', str(root), '--key', 'scroll-fixture'], env=env))
    def tmux(*args):
        return subprocess.check_output(['tmux', '-S', socket, *args], env=env).decode().strip()
    tmux('set-environment', '-g', 'HOME', str(root))
    tmux('set-environment', '-g', 'XDG_STATE_HOME', str(root/'state'))
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 24, 80, 0, 0))
    def tty():
        os.setsid(); fcntl.ioctl(slave, termios.TIOCSCTTY, 0)
    client = subprocess.Popen([binary, 'native-attach', 'attach-session', '-t', session['id']], stdin=slave, stdout=slave, stderr=slave, env=env, preexec_fn=tty)
    screen = bytearray()
    def drain():
        while select.select([master], [], [], 0)[0]:
            screen.extend(os.read(master, 65536))
    def wait(predicate, message):
        deadline = time.monotonic()+6
        while not predicate():
            drain()
            assert client.poll() is None, 'client unexpectedly exited'
            assert time.monotonic() < deadline, message
            time.sleep(.02)
    pane = session['id'] + ':0.0'
    def state(format): return tmux('display-message', '-p', '-t', pane, format)
    log = root/'keys'
    fixture = root/'fixture.py'
    fixture.write_text('''import os,pathlib,sys,tty
log=pathlib.Path(sys.argv[1]); tty.setraw(0)
for n in range(150): print('TRANSCRIPT_LINE_%03d'%n, end='\\r\\n')
sys.stdout.flush(); log.write_bytes(b'')
while True:
 data=os.read(0,4096)
 with log.open('ab') as stream: stream.write(data)
 if data==b'a':
  sys.stdout.write('\\x1b[?1049h');sys.stdout.flush()
 if data==b'm':
  sys.stdout.write('\\x1b[?1000h\\x1b[?1006h');sys.stdout.flush()
''')
    checks=[]
    try:
        wait(lambda: 'C-]' in tmux('list-keys', '-T', 'root'), 'managed bindings unavailable')
        assert tmux('show-options', '-gv', 'mouse') == 'on', 'wheel reporting disabled: terminal may send prompt-history arrows'
        command = 'exec '+shlex.quote(sys.executable)+' '+shlex.quote(str(fixture))+' '+shlex.quote(str(log))
        tmux('send-keys', '-t', pane, '-l', command); tmux('send-keys', '-t', pane, 'Enter')
        wait(log.exists, 'fixture not ready')
        wait(lambda: state('#{history_size}').isdigit() and int(state('#{history_size}'))>50, 'no scrollback')
        drain()
        wait(lambda: any(code in screen for code in (b'\x1b[?1000h', b'\x1b[?1002h', b'\x1b[?1003h')), 'client did not request mouse reporting')
        wheel_up=b'\x1b[<64;10;10M'; wheel_down=b'\x1b[<65;10;10M'
        os.write(master,wheel_up)
        wait(lambda: state('#{pane_in_mode}') == '1', 'wheel failed to enter scrollback')
        assert log.read_bytes()==b'', 'wheel reached application prompt'
        wait(lambda: int(state('#{scroll_position}'))>0, 'scrollback did not move up')
        assert 'TRANSCRIPT_LINE_' in tmux('capture-pane', '-p', '-t', pane), 'transcript unavailable'
        pid = state('#{pane_pid}')
        os.write(master,b'\x1d'); client.wait(timeout=5)
        assert client.returncode==0 and state('#{pane_pid}')==pid
        client = subprocess.Popen([binary, 'native-attach', 'attach-session', '-t', session['id']], stdin=slave, stdout=slave, stderr=slave, env=env, preexec_fn=tty)
        wait(lambda: state('#{pane_in_mode}')=='1', 'scrollback lost on reattach')
        time.sleep(.1); drain()
        os.write(master,b'\x1b')
        wait(lambda: state('#{pane_in_mode}')=='0', 'Escape did not return to live application')
        time.sleep(.15); drain()
        checks.append('wheel enters retained transcript scrollback without prompt-history input; Escape returns live')
        for keys in ('emacs', 'vi'):
            tmux('set-window-option', '-t', session['id'], 'mode-keys', keys)
            expected=log.read_bytes()+b'a'
            os.write(master, b'a')
            # alternate_on is already true on pass two. Acknowledge the input
            # before taking the baseline, or a delayed 'a' looks like wheel input.
            wait(lambda: log.read_bytes()==expected, 'alternate-screen input not acknowledged')
            wait(lambda: state('#{alternate_on}')=='1', 'alternate screen not enabled')
            tmux('clear-history', '-t', pane)
            before=log.read_bytes()
            os.write(master, wheel_up)
            time.sleep(.15); drain()
            assert state('#{pane_in_mode}')=='0', 'wheel entered an empty 0/0 copy buffer instead of scrolling chat'
            assert log.read_bytes()==before, 'alternate-screen wheel reached application'

            assert state('#{pane_in_mode}')=='0', keys+' empty history must remain live'
        checks.append('alternate-screen/no-mouse with empty history stays live and receives no fabricated input')
        os.write(master,b'\x1b[A')
        wait(lambda: b'\x1b[A' in log.read_bytes(), 'native Up key intercepted')
        checks.append('native Up key remains application-owned; Ctrl+] works inside scrollback with same PID on reattach')
        os.write(master,b'm')
        wait(lambda: state('#{mouse_any_flag}')=='1', 'application mouse not enabled')
        before=log.read_bytes()
        os.write(master,wheel_up)
        wait(lambda: len(log.read_bytes())>len(before), 'mouse-aware application did not receive wheel')
        assert b'\x1b[<64;' in log.read_bytes()[len(before):], 'wrong native wheel forwarding'
        assert state('#{pane_in_mode}')=='0', 'native mouse application forced into scrollback'
        checks.append('mouse-aware applications receive native wheel events')
        os.write(master,b'\x1d'); client.wait(timeout=5)
        assert client.returncode==0 and state('#{pane_dead}')=='0'
        checks.append('Ctrl+] returns without killing original process')
        # Updates must reload configuration even when the owned server persists
        # with zero sessions. Otherwise the next shell inherits old bindings.
        old_bindings=tmux('list-keys','-T','root')
        tmux('kill-session','-t',session['id'])
        tmux('bind-key','-n','WheelUpPane','display-message','obsolete-binding')
        subprocess.check_output([binary,'new','--provider','shell','--directory',str(root),'--key','empty-server-refresh'],env=env)
        assert tmux('list-keys','-T','root')==old_bindings, 'empty persistent server retained obsolete wheel bindings'
        checks.append('empty persistent server reloads current owned bindings')
        print(json.dumps(dict(result='PASS',backend=tmux('-V'),checks=checks)))
    finally:
        if client.poll() is None: client.terminate(); client.wait(timeout=5)
        os.close(master); os.close(slave)
        subprocess.run(['tmux','-S',socket,'kill-server'],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
