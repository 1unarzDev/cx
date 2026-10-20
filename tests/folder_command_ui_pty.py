#!/usr/bin/env python3
"""Integrated colon command and current-folder session selection in a real PTY."""
import fcntl, json, os, pathlib, pty, select, struct, subprocess, sys, tempfile, termios, time
binary = str(pathlib.Path(sys.argv[1]).resolve())
results = []
for shell in sys.argv[2:] or ['/bin/bash']:
    with tempfile.TemporaryDirectory(prefix='cx-folder-ui-') as temporary:
        root = pathlib.Path(temporary); folder = root / "current ' folder ☃"; folder.mkdir()
        (folder/'before.txt').write_text('fixture')
        config = root/'config/fish'; config.mkdir(parents=True)
        (config/'config.fish').write_text("if not set -q CX_VIEWER_THEME; printf '%b' '\\e]4;1;rgb:12/34/56\\a'; end\n")
        state = root/'state/cx'; state.mkdir(parents=True)
        name = 'viewer-restart-123-789.json'
        snapshot = dict(schema=1, expires_at=int(time.time())+300, device_ids=['local'], device=1,
            focus='Workspace', view='Files', selected=0, selected_session=None, side_selected=0,
            search='', browser=dict(device=0, path=str(folder), display_path=str(folder), parent=str(root),
                entries=[], selected=0, search='', preview_scroll=0, restore_selection=None),
            other_browser=None, destination_active=False, conflict=2, launch_provider=None, clipboard=None, submitted={})
        snapshot_path = state/name; snapshot_path.write_text(json.dumps(snapshot)); snapshot_path.chmod(0o600)
        env = dict(os.environ, HOME=str(root), XDG_CONFIG_HOME=str(root/'config'), XDG_STATE_HOME=str(root/'state'), SHELL=shell,
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
            send(b'\x10'); wait('Actions')
            time.sleep(.2); read()
            assert b'Run command here' not in output, 'command remains in Actions'
            send(b'\x1b'); time.sleep(.2); read()
            send(b'n'); wait('New session'); wait('Shell')
            send(b'\x1b'); time.sleep(.2); read()
            # A command runs in a floating panel; CX is never suspended. The
            # markers use printf format strings so the panel OUTPUT differs from
            # the command text echoed in the input box (which also appears).
            send(b':'); wait('Run command')
            os.write(master,b"printf 'MARK%s hjkl:n x\\n' DONE; read reply; printf 'GOT%s\\n' \"$reply\" > proof.txt; exit 4\r")
            wait('MARKDONE')
            assert os.tcgetpgrp(master)==process.pid, 'CX was suspended for the command'
            # Input typed while the panel is focused reaches the command; the
            # finish notice (rendered as a block) reports the exit status.
            send(b'world\r'); wait('Command exited with status 4')
            assert (folder/'proof.txt').read_text() == 'GOTworld\n'
            for code in (b'\x1b]4;', b'\x1b]104', b'\x1b]110', b'\x1b]111'):
                assert code not in output, 'command panel changed viewer palette: '+repr(code)
            # Esc closes the panel; the command input then opens over the browser,
            # proving CX is responsive again (the small panel no longer overlaps
            # the file list, so we don't rely on it being redrawn).
            send(b"\x1b"); time.sleep(.4); read(); assert os.tcgetpgrp(master)==process.pid
            # Ctrl+C is delivered to the running command, not to CX.
            send(b':'); wait('Run command')
            os.write(master,b"printf 'READY%s\\n' NOW; sleep 30\r")
            wait('READYNOW'); time.sleep(.3); send(b'\x03')
            # Ctrl+C ends the command; the finish notice confirms CX survived.
            wait('Command'); assert process.poll() is None, 'Ctrl+C killed viewer'
            send(b'\x1b'); time.sleep(.3); read(); assert process.poll() is None
            send(b'\x03'); process.wait(timeout=5)
            assert process.returncode==0
            assert termios.tcgetattr(slave)==before
            results.append(dict(shell=shell,result='PASS',checks=['Actions omits command','colon preserves viewer palette','n current-folder provider dialog','command runs in a panel without suspending CX','panel input reaches the command and it finishes','palette preserved','Esc closes panel','Ctrl+C interrupts the command not CX','foreground/modes restored']))
        finally:
            if process.poll() is None: process.kill(); process.wait(timeout=5)
            os.close(master); os.close(slave)
print(json.dumps(results))
