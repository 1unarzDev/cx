#!/usr/bin/env python3
"""Integrated colon command and current-folder session selection in a real PTY."""
import fcntl, json, os, pathlib, pty, select, struct, subprocess, sys, tempfile, termios, time
binary = str(pathlib.Path(sys.argv[1]).resolve())
results = []
for shell in sys.argv[2:] or ['/bin/bash']:
    with tempfile.TemporaryDirectory(prefix='cx-folder-ui-') as temporary:
        root = pathlib.Path(temporary); folder = root / "current ' folder ☃"; folder.mkdir()
        (folder/'before.txt').write_text('fixture')
        state = root/'state/cx'; state.mkdir(parents=True)
        name = 'viewer-restart-123-789.json'
        snapshot = dict(schema=1, expires_at=int(time.time())+300, device_ids=['local'], device=1,
            focus='Workspace', view='Files', selected=0, selected_session=None, side_selected=0,
            search='', browser=dict(device=0, path=str(folder), display_path=str(folder), parent=str(root),
                entries=[], selected=0, search='', preview_scroll=0, restore_selection=None),
            other_browser=None, destination_active=False, conflict=2, launch_provider=None, clipboard=None, submitted={})
        snapshot_path = state/name; snapshot_path.write_text(json.dumps(snapshot)); snapshot_path.chmod(0o600)
        env = dict(os.environ, HOME=str(root), XDG_STATE_HOME=str(root/'state'), SHELL=shell,
            TERM='xterm-256color', HTTPS_PROXY='http://127.0.0.1:9')
        env.pop('TMUX',None); env.pop('TMUX_PANE',None)
        master, slave = pty.openpty(); before = termios.tcgetattr(slave)
        fcntl.ioctl(slave,termios.TIOCSWINSZ,struct.pack('HHHH',24,80,0,0))
        def tty():
            os.setsid(); fcntl.ioctl(slave,termios.TIOCSCTTY,0)
        process = subprocess.Popen([binary,'restart',name],stdin=slave,stdout=slave,stderr=slave,env=env,preexec_fn=tty)
        output = bytearray()
        def read():
            if select.select([master],[],[],.1)[0]:
                try: output.extend(os.read(master,65536))
                except OSError: pass
        def wait(text,seconds=10):
            deadline=time.monotonic()+seconds
            while text.encode() not in output:
                read()
                assert process.poll() is None, repr(bytes(output)[-16000:])
                assert time.monotonic()<deadline, 'missing '+text+' '+repr(bytes(output)[-16000:])
        def send(text):
            output.clear(); os.write(master,text)
        try:
            wait('before.txt')
            send(b'n'); wait('New session'); wait('Shell')
            send(b'\x1b'); time.sleep(.2); read()
            send(b':'); wait('Run command')
            # Includes browser shortcut characters, safely treated as command text.
            os.write(master,b"printf '%s' 'hjkl:n x' > proof.txt\r")
            wait('Command ended:'); assert process.poll() is None
            assert (folder/'proof.txt').read_text() == 'hjkl:n x'
            send(b'\r'); wait('Returned from'); wait('proof.txt')
            assert os.tcgetpgrp(master)==process.pid
            send(b':'); wait('Run command')
            os.write(master,b'echo INTERRUPT_READY; sleep 30\r')
            wait('INTERRUPT_READY'); time.sleep(.3); os.write(master,b'\x03')
            wait('Command ended:'); assert process.poll() is None, 'Ctrl+C killed viewer'
            send(b'\r'); wait('Returned from'); assert process.poll() is None
            send(b'\x03'); process.wait(timeout=5)
            assert process.returncode==0
            assert termios.tcgetattr(slave)==before
            results.append(dict(shell=shell,result='PASS',checks=['n current-folder provider dialog','colon input owns browser shortcuts','exact current folder execution','Enter restores browser and refreshes new file','Ctrl+C interrupts command and preserves viewer','foreground/modes restored']))
        finally:
            if process.poll() is None: process.kill(); process.wait(timeout=5)
            os.close(master); os.close(slave)
print(json.dumps(results))
