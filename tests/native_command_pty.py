#!/usr/bin/env python3
"""Native command PTY acceptance; no providers, networking, or user state.
Usage: native_command_pty.py BINARY [--unit-helper]
The unit helper is cargo's test executable; otherwise BINARY is cx.
"""
import base64, errno, fcntl, json, os, pathlib, pty, select, struct, subprocess, sys, tempfile, termios, time
binary = str(pathlib.Path(sys.argv[1]).resolve())
unit = '--unit-helper' in sys.argv[2:]
checks = []
with tempfile.TemporaryDirectory(prefix='cx-command-pty-') as temporary:
    root = pathlib.Path(temporary)
    folder = root / "folder ' quoted ; ☃"
    folder.mkdir()
    def scenario(command, interact=None, expected='Command ended:'):
        payload = {'directory':str(folder), 'command':command}
        master, slave = pty.openpty()
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 24, 80, 0, 0))
        before = termios.tcgetattr(slave)
        environment = dict(os.environ, HOME=str(root), XDG_STATE_HOME=str(root/'state'), SHELL='/bin/bash', TERM='xterm-256color')
        args = [binary, 'native-command', base64.b64encode(json.dumps(payload).encode()).decode()]
        if unit:
            environment['CX_COMMAND_PTY_FIXTURE'] = json.dumps(payload)
            args = [binary, 'sessions::tests::native_command_fixture', '--exact', '--nocapture']
        def tty():
            os.setsid()
            fcntl.ioctl(slave, termios.TIOCSCTTY, 0)
        process = subprocess.Popen(args, stdin=slave, stdout=slave, stderr=slave, env=environment, preexec_fn=tty)
        output = bytearray()
        def wait_for(text, timeout=8):
            deadline = time.monotonic() + timeout
            while text.encode() not in output:
                if time.monotonic() >= deadline:
                    raise AssertionError('timeout '+repr(text)+' output='+repr(bytes(output)[-3000:]))
                if select.select([master], [], [], .1)[0]:
                    try:
                        block = os.read(master, 65536)
                    except OSError as error:
                        if error.errno != errno.EIO: raise
                        raise AssertionError('PTY closed output='+repr(bytes(output)[-3000:]))
                    if not block: raise AssertionError('PTY EOF')
                    output.extend(block)
        try:
            if interact: interact(master, process, wait_for)
            wait_for(expected)
            assert process.poll() is None, 'helper exited before Enter'
            # The helper owns the foreground again, and child stty edits are gone.
            assert os.tcgetpgrp(master) == process.pid, 'foreground ownership not restored'
            assert termios.tcgetattr(slave) == before, 'terminal modes not restored'
            os.write(master, b'\r')
            process.wait(timeout=5)
            assert process.returncode == 0, bytes(output)[-3000:]
            return bytes(output)
        finally:
            if process.poll() is None:
                process.kill(); process.wait(timeout=5)
            os.close(master); os.close(slave)
    scenario("pwd; printf '%s' 'proof ☃' > 'result file'")
    assert (folder/'result file').read_text() == 'proof ☃'
    checks.append('quoted Unicode directory and exact write')
    scenario('exit 7', expected='exit status: 7')
    checks.append('nonzero status and explicit Enter return')
    def interrupt(master, process, wait):
        wait('INTERRUPT_READY')
        time.sleep(.2)  # Let the shell finish foreground job handoff.
        os.write(master, b'\x03')
    scenario('echo INTERRUPT_READY; sleep 30', interrupt)
    checks.append('Ctrl+C interrupts child without exiting helper')
    def interactive(master, process, wait):
        wait('INPUT_READY')
        os.write(master, b'hello\r')
    scenario('echo INPUT_READY; read answer; printf "%s" "$answer" > input-result', interactive)
    assert (folder/'input-result').read_text() == 'hello'
    checks.append('native stdin')
    scenario('stty -echo -icanon; exit 0')
    checks.append('termios and foreground group restoration')
    print(json.dumps({'result':'PASS', 'checks':checks, 'backend':'native Bash PTY', 'binary':binary, 'unit_helper':unit}))
