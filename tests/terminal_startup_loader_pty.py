#!/usr/bin/env python3
"""Slow owned shell: CX animates until visible content, then hands off once."""
import fcntl, json, os, pathlib, pty, select, struct, subprocess, sys, tempfile, termios, time
import pyte
binary = str(pathlib.Path(sys.argv[1]).resolve())
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
    def read(seconds=.1):
        end=time.monotonic()+seconds
        while time.monotonic()<end:
            if select.select([master],[],[],.02)[0]: stream.feed(os.read(master,65536).decode(errors='replace'))
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
        os.write(master,b'\x1d'); wait(lambda:'Files' in text())
        os.write(master,b'\x03');proc.wait(timeout=5)
        assert termios.tcgetattr(slave)==before
        print(json.dumps({'result':'PASS','checks':['loader animates during delayed shell startup','no native client before visible screen','single handoff accepts input and restores viewer/termios']}))
    finally:
        if proc.poll() is None: proc.kill();proc.wait()
        subprocess.run(['tmux','-S',socket,'kill-server'],env=env,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
        os.close(master);os.close(slave)
