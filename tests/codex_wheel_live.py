#!/usr/bin/env python3
"""Actual configured Codex: native transcript scrolling with stale tmux mouse policy.
Requires configured Fish/Codex on the execution account. Uses native /status only;
no inference prompts, configuration changes, or retained terminal content. Usage: script BINARY.
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
    session = json.loads(subprocess.check_output([binary, 'new', '--provider', 'shell', '--directory', str(pathlib.Path.cwd()), '--key', 'scroll-fixture'], env=env))
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
    home=pathlib.Path.home()
    # Shell wrapper/config remain on the execution account; never send inference.
    def visible(): return tmux('capture-pane','-p','-t',pane)
    def original():
        parent=int(state('#{pane_pid}'))
        queue=[parent];result=[]
        for pid in queue:
            try:
                if pathlib.Path('/proc/%d/exe'%pid).resolve().name=='codex':
                    stat=pathlib.Path('/proc/%d/stat'%pid).read_text().rsplit(')',1)[1].split()[19]
                    result.append((pid,stat))
                queue.extend(map(int,pathlib.Path('/proc/%d/task/%d/children'%(pid,pid)).read_text().split()))
            except (OSError,ValueError):pass
        return result
    try:
        wait(lambda: 'native-wheel' in tmux('list-keys','-T','root'),'binding missing')
        wait(lambda: bool(tmux('list-clients')) and not(termios.tcgetattr(slave)[3] & termios.ICANON),'native client not ready')
        time.sleep(.2);drain()
        tmux('set-option','-g','mouse','off')
        command='env HOME='+shlex.quote(str(home))+' XDG_STATE_HOME='+shlex.quote(str(home/'.local/state'))+' /usr/bin/fish -l -c codex'
        tmux('send-keys','-t',pane,'-l',command);tmux('send-keys','-t',pane,'Enter')
        wait(lambda: state('#{alternate_on}')=='1','native Codex not fullscreen')
        time.sleep(2);drain()
        assert state('#{mouse_any_flag}')=='0','stale policy not reproduced'
        before=original();assert before
        tmux('set-option','-g','mouse','on')
        for _ in range(4):
            os.write(master,b'/status\r');time.sleep(.5);drain()
        os.write(master,b'CX_SCROLL_DRAFT');time.sleep(.2);drain()
        assert 'CX_SCROLL_DRAFT' in visible()
        os.write(master,b'\x1b[<64;10;10M')
        wait(lambda: 'Back to bottom' in visible(),'wheel did not scroll native transcript')
        assert state('#{pane_in_mode}')=='0','empty tmux mode opened'
        assert 'CX_SCROLL_DRAFT' in visible(),'draft changed'
        os.write(master,b'\x1b');time.sleep(.2);drain()
        wait(lambda: 'Back to bottom' not in visible(),'Escape did not return native transcript live')
        assert 'CX_SCROLL_DRAFT' in visible()
        assert original()==before,'Codex process replaced'
        os.write(master,b'\x1d');client.wait(timeout=5)
        print(json.dumps(dict(result='PASS',codex='0.160.1',backend=tmux('-V'),stale_mouse_policy=True,native_transcript_scrolled=True,draft_preserved=True,same_frontend_identity=True,inference_prompt_sent=False)))
    finally:
        if client.poll() is None: client.terminate(); client.wait(timeout=5)
        os.close(master); os.close(slave)
        subprocess.run(['tmux','-S',socket,'kill-server'],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
