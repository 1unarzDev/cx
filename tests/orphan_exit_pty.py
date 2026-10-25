#!/usr/bin/env python3
"""A cx UI whose pty master is closed (terminal gone, no SIGHUP delivered) must exit, not spin.
Runs cx in its own session as a non-leader process group; usage: script BINARY.
"""
import fcntl, os, pathlib, pty, signal, struct, subprocess, sys, tempfile, termios, time
binary = str(pathlib.Path(sys.argv[1]).resolve())
with tempfile.TemporaryDirectory(prefix='cx-orphan-') as temporary:
    root = pathlib.Path(temporary)
    env = dict(os.environ, HOME=str(root), XDG_STATE_HOME=str(root/'state'), TERM='xterm-256color')
    env.pop('TMUX', None); env.pop('TMUX_PANE', None)
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 24, 80, 0, 0))
    # Leader shell owns the session; cx runs in a background-style own process group,
    # so the kernel's hangup SIGHUP never reaches it.
    leader = subprocess.Popen(['sh', '-c', 'setsid -w true; exec sleep 60'], stdin=slave, stdout=slave, stderr=slave, env=env, start_new_session=True)
    client = subprocess.Popen([binary], stdin=slave, stdout=slave, stderr=slave, env=env, process_group=0)
    try:
        time.sleep(2)
        assert client.poll() is None, 'cx exited before the terminal closed'
        os.close(master)
        leader.kill(); leader.wait()
        deadline = time.monotonic()+3
        while client.poll() is None and time.monotonic() < deadline:
            time.sleep(.05)
        assert client.poll() is not None, 'cx still running after its terminal went away'
        print('orphan_exit_pty: ok')
    finally:
        for p in (client, leader):
            if p.poll() is None: p.kill()
