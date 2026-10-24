#!/usr/bin/env python3
"""Slow owned shell: CX animates until visible content, then hands off once."""
import fcntl, json, os, pathlib, pty, select, struct, subprocess, sys, tempfile, termios, time
import pyte
binary = str(pathlib.Path(sys.argv[1]).resolve())
def process_identity(pid):
    try:
        # The command name may contain spaces and closing parentheses.
        fields = pathlib.Path(f'/proc/{pid}/stat').read_text().rsplit(')', 1)[1].split()
    except FileNotFoundError:
        return None
    return fields[19], fields[0]

def stop_owned_tmux(socket, env):
    # kill-server acknowledges the request before shells finish their exit hooks.
    # Pin identities before stopping, then wait without signalling recycled PIDs.
    owned = set()
    for command in [
        ['display-message', '-p', '#{pid}'],
        ['list-panes', '-a', '-F', '#{pane_pid}'],
    ]:
        result = subprocess.run(['tmux', '-S', socket, *command], env=env,
                                capture_output=True, text=True, timeout=5)
        if result.returncode == 0:
            owned.update(int(pid) for pid in result.stdout.split())
    identities = {pid: process_identity(pid) for pid in owned}
    subprocess.run(['tmux', '-S', socket, 'kill-server'], env=env,
                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=5)
    deadline = time.monotonic() + 5
    while True:
        alive = []
        for pid, original in identities.items():
            current = process_identity(pid)
            if original is not None and current is not None and current[0] == original[0] and current[1] != 'Z':
                alive.append(pid)
        if not alive:
            return
        assert time.monotonic() < deadline, f'Owned fixture tmux processes did not exit: {alive}'
        time.sleep(.02)

with tempfile.TemporaryDirectory(prefix='cx-startup-loader-') as tmp:
    root = pathlib.Path(tmp)
    state = root/'state/cx'; state.mkdir(parents=True)
    launch = root/'bash'
    launch.write_text('#!/bin/sh\nsleep 2\nprintf "CX_READY_SCREEN\\n"\nexec /bin/bash --noprofile --norc "$@"\n')
    launch.chmod(0o700)
    (state/'launcher.json').write_text(json.dumps(str(launch)))
    env = dict(os.environ, HOME=tmp, XDG_STATE_HOME=str(root/'state'), SHELL=str(launch), TERM='xterm-256color', HTTPS_PROXY='http://127.0.0.1:9')
    for key in ['TMUX', 'TMUX_PANE']: env.pop(key, None)
    # Avoid systemd inheriting the real user's environment into this fixture.
    socket = str(state/'managed.sock')
    subprocess.run(['tmux','-S',socket,'-f','/dev/null','new-session','-d','-s','bootstrap','sleep 60'],env=env,check=True)
    master, slave = pty.openpty(); before = termios.tcgetattr(slave)
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH',24,100,0,0))
    def tty(): os.setsid(); fcntl.ioctl(slave,termios.TIOCSCTTY,0)
    proc = subprocess.Popen([binary],env=env,cwd=tmp,stdin=slave,stdout=slave,stderr=slave,preexec_fn=tty)
    class Screen(pyte.Screen):
        def report_device_status(self,*args,**kwargs): pass
    screen=Screen(100,24); stream=pyte.Stream(screen)
    raw=bytearray()
    def read(seconds=.1):
        end=time.monotonic()+seconds
        while time.monotonic()<end:
            if select.select([master],[],[],.02)[0]:
                chunk=os.read(master,65536); raw.extend(chunk); stream.feed(chunk.decode(errors='replace'))
    def text(): return '\n'.join(screen.display)
    def wait(predicate):
        end=time.monotonic()+12
        while not predicate() and time.monotonic()<end: read()
        assert predicate(), text()
    try:
        wait(lambda:'Files' in text()); read(.4)
        os.write(master,b'n'); wait(lambda:'[s] Shell' in text())
        os.write(master,b's'); read(.6)
        assert 'Starting terminal' in text(), 'blank/native handoff before startup finished:\n'+text()
        first=text(); read(.25)
        assert 'Starting terminal' in text() and text()!=first, 'startup loader did not animate'
        clients=subprocess.check_output(['tmux','-S',socket,'list-clients','-F','#{client_pid}'],env=env,text=True)
        assert not clients.strip(), 'native attach started before the first screen'
        wait(lambda:'CX_READY_SCREEN' in text())
        os.write(master,b'printf done > loader-proof\r')
        wait(lambda:(root/'loader-proof').exists())
        # Leaving the TUI for the handoff must wipe the primary screen it restores,
        # so a slow remote attach cannot flash the prior "[detached]" teardown text
        # before tmux repaints. crossterm emits LeaveAlternateScreen then Clear(All).
        assert b'\x1b[?1049l\x1b[2J' in raw, 'handoff did not clear the primary screen'
        os.write(master,b'\x1d'); wait(lambda:'Files' in text())
        os.write(master,b'\x03');proc.wait(timeout=5)
        assert termios.tcgetattr(slave)==before
    finally:
        if proc.poll() is None: proc.kill();proc.wait()
        try:
            stop_owned_tmux(socket, env)
        finally:
            os.close(master);os.close(slave)
print(json.dumps({'result':'PASS','checks':['loader animates during delayed shell startup','no native client before visible screen','handoff clears the primary screen (no stale detach-text flash)','single handoff accepts input and restores viewer/termios','owned tmux server and panes exit before temporary state removal']}))
