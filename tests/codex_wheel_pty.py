#!/usr/bin/env python3
"""Verified foreground synthetic Codex: stale mouse wheel forwarding and empty copy mode.
No provider, transcripts, user config, or inference. Usage: script BINARY.
"""
import fcntl, json, os, pathlib, pty, select, shlex, struct, subprocess, sys, shutil, tempfile, termios, time
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
    executable=root/'codex'
    shutil.copyfile('/bin/cat',str(executable)); executable.chmod(0o700)
    observer=None
    checks=[]
    try:
        wait(lambda: 'native-wheel' in tmux('list-keys', '-T', 'root'), 'fallback binding unavailable')
        command = 'stty raw; printf \'\\033[?1049hSYNTHETIC_TRANSCRIPT\\r\\n\'; exec '+shlex.quote(str(executable))+' > '+shlex.quote(str(log))
        tmux('send-keys','-t',pane,'-l',command); tmux('send-keys','-t',pane,'Enter')
        wait(log.exists,'synthetic Codex not ready')
        wait(lambda: state('#{alternate_on}')=='1', 'owned screen missing')
        tmux('clear-history','-t',pane)
        pid=state('#{pane_pid}')
        wheel_up=b'\x1b[<64;10;10M'; wheel_down=b'\x1b[<65;10;10M'
        expected=b'\x1b[<64;10;10M'
        os.write(master,wheel_up)
        wait(lambda: log.read_bytes()==expected,'stale Codex wheel not delivered intact')
        assert state('#{pane_in_mode}')=='0','Codex entered empty 0/0 copy mode'
        os.write(master,wheel_down)
        expected+=b'\x1b[<65;10;10M'
        wait(lambda: log.read_bytes()==expected,'reverse native wheel not delivered')
        # Recover a previously opened empty buffer without replacing the process.
        tmux('copy-mode','-t',pane)
        wait(lambda: state('#{pane_in_mode}')=='1','copy mode fixture failed')
        os.write(master,wheel_up); expected+=b'\x1b[<64;10;10M'
        wait(lambda: log.read_bytes()==expected,'existing 0/0 mode not recovered')
        assert state('#{pane_in_mode}')=='0'
        # Observation must not bypass the originating read-only client policy.
        om, oslv=pty.openpty()
        fcntl.ioctl(oslv,termios.TIOCSWINSZ,struct.pack('HHHH',24,80,0,0))
        def observe_tty():
            os.setsid();fcntl.ioctl(oslv,termios.TIOCSCTTY,0)
        observer=subprocess.Popen([binary,'native-attach','attach-session','-r','-t',session['id']],stdin=oslv,stdout=oslv,stderr=oslv,env=env,preexec_fn=observe_tty)
        wait(lambda: ' 1' in tmux('list-clients','-F','#{client_pid} #{client_readonly}'), 'read-only client unavailable')
        readonly=next(line.split()[0] for line in tmux('list-clients','-F','#{client_pid} #{client_readonly}').splitlines() if line.endswith(' 1'))
        tmux('copy-mode','-t',pane)
        os.write(om,wheel_up)
        subprocess.run([binary,'native-wheel',state('#{pane_id}'),'up','9','9',readonly],env=env,check=True)
        time.sleep(.2);drain()
        assert log.read_bytes()==expected, 'observer injected application input'
        assert state('#{pane_in_mode}')=='1', 'observer canceled controller copy mode'
        observer.terminate();observer.wait(timeout=5);os.close(om);os.close(oslv)
        tmux('send-keys','-t',pane,'-X','cancel')
        # No event outside the pane, no foreign session targeting or arbitrary key input.
        subprocess.run([binary,'native-wheel','%0','up','65535','65535','0'],env=env,check=True)
        time.sleep(.1);assert log.read_bytes()==expected
        r=subprocess.run([binary,'native-wheel','bad;target','up','9','9','0'],env=env,capture_output=True)
        assert r.returncode!=0
        # Ordered burst must remain native, without leaking into prompt history.
        for _ in range(10): os.write(master,wheel_up)
        expected+=b'\x1b[<64;10;10M'*10
        wait(lambda: log.read_bytes()==expected,'wheel burst lost or reordered')
        assert state('#{pane_pid}')==pid
        # A foreground editor inherited the Codex parent's process group.
        editor=root/'editor'; shutil.copyfile('/bin/dd',str(editor));editor.chmod(0o700)
        parent_dir=root/'parent';parent_dir.mkdir()
        parent=parent_dir/'codex';shutil.copyfile('/bin/bash',str(parent));parent.chmod(0o700)
        session=json.loads(subprocess.check_output([binary,'new','--provider','shell','--directory',str(root),'--key','editor-fixture'],env=env))
        pane=session['id']+':0.0'
        log=root/'editor-keys'
        child='exec 0</dev/null; stty raw </dev/tty; printf \'\\033[?1049hEDITOR\\r\\n\'; '+shlex.quote(str(editor))+' if=/dev/tty of='+shlex.quote(str(log))+' bs=1; :'
        command='exec '+shlex.quote(str(parent))+' -c '+shlex.quote(child)
        tmux('send-keys','-t',pane,'-l',command);tmux('send-keys','-t',pane,'Enter')
        wait(log.exists,'editor fixture not ready')
        tmux('clear-history','-t',pane)
        tmux('switch-client','-t',session['id'])
        time.sleep(.2);drain()
        os.write(master,wheel_up)
        time.sleep(.2);drain()
        assert log.read_bytes()==b'', 'Codex ancestor caused wheel injection into editor'
        assert state('#{pane_in_mode}')=='0'
        os.write(master,b'\x1d');client.wait(timeout=5)
        assert client.returncode==0 and state('#{pane_dead}')=='0'
        print(json.dumps(dict(result='PASS',backend=tmux('-V'),native_wheel_exact_bytes=True,empty_mode_recovery=True,invalid_event_suppressed=True,same_process=True,embedded_editor_input_preserved=True,observer_no_input=True)))
    finally:
        if observer is not None and observer.poll() is None: observer.terminate();observer.wait(timeout=5)
        if client.poll() is None: client.terminate(); client.wait(timeout=5)
        os.close(master); os.close(slave)
        subprocess.run(['tmux','-S',socket,'kill-server'],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
