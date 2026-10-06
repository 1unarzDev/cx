#!/usr/bin/env python3
"""Native command PTY acceptance; no providers, networking, or user state.
Usage: native_command_pty.py BINARY [--unit-helper]
The unit helper is cargo's test executable; otherwise BINARY is cx.
"""
import base64, errno, fcntl, json, os, pathlib, pty, select, signal, struct, subprocess, sys, tempfile, termios, time
binary = str(pathlib.Path(sys.argv[1]).resolve())
unit = '--unit-helper' in sys.argv[2:]
checks = []
shell = os.environ.get('CX_TEST_SHELL', '/bin/bash')
with tempfile.TemporaryDirectory(prefix='cx-command-pty-') as temporary:
    root = pathlib.Path(temporary)
    folder = root / "folder ' quoted ; ☃"
    folder.mkdir()
    (root/'.bash_profile').write_text("cx_fixture_wrapper() { printf wrapper-ok; }\n")
    # Isolate synthetic Zsh startup from runner-owned global compinit prompts.
    (root/'.zshenv').write_text("unsetopt GLOBAL_RCS\n")
    (root/'.zshrc').write_text("cx_fixture_wrapper() { printf wrapper-ok; }\n")
    config = root/'config/fish'; config.mkdir(parents=True)
    (config/'config.fish').write_text("function cx_fixture_wrapper; printf wrapper-ok; end\n")
    def scenario(command, interact=None, expected='Command ended:'):
        payload = {'directory':str(folder), 'command':command}
        master, slave = pty.openpty()
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 24, 80, 0, 0))
        before = termios.tcgetattr(slave)
        environment = dict(os.environ, HOME=str(root), XDG_STATE_HOME=str(root/'state'), XDG_CONFIG_HOME=str(root/'config'), SHELL=shell, TERM='xterm-256color')
        environment.pop("ZDOTDIR", None)
        args = [binary, 'native-command', base64.b64encode(json.dumps(payload).encode()).decode()]
        if unit:
            environment['CX_COMMAND_PTY_FIXTURE'] = json.dumps(payload)
            args = [binary, 'sessions::tests::native_command_fixture', '--exact', '--nocapture']
        def tty():
            os.setsid()
            fcntl.ioctl(slave, termios.TIOCSCTTY, 0)
        process = subprocess.Popen(args, stdin=slave, stdout=slave, stderr=slave, env=environment, preexec_fn=tty)
        output = bytearray()
        owned_groups = set()
        def wait_for(text, timeout=8):
            deadline = time.monotonic() + timeout
            while text.encode() not in output:
                group = os.tcgetpgrp(master)
                if group > 0 and group != process.pid: owned_groups.add(group)
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
            # Failure cleanup only touches process groups observed on this
            # fixture's private tty and still belonging to its private session.
            for group in owned_groups:
                try:
                    fields = pathlib.Path('/proc/%s/stat' % group).read_text().rsplit(')',1)[1].split()
                    if int(fields[3]) == process.pid:
                        os.killpg(group, signal.SIGKILL)
                except (OSError, ValueError): pass
            os.close(master); os.close(slave)
    scenario("pwd; printf '%s' 'proof ☃' > 'result file'")
    assert (folder/'result file').read_text() == 'proof ☃'
    checks.append('quoted Unicode directory and exact write')
    scenario('cx_fixture_wrapper > wrapper-result')
    assert (folder/'wrapper-result').read_text() == 'wrapper-ok'
    checks.append('interactive host wrapper preserved')
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
    scenario('echo INPUT_READY; head -n1 > input-result', interactive)
    assert (folder/'input-result').read_text().strip() == 'hello'
    checks.append('native stdin')
    def stop_and_cancel(master, process, wait):
        wait('STOP_READY')
        time.sleep(.2)
        os.write(master, b'\x1a')
        wait('Command suspended')
        assert os.tcgetpgrp(master) == process.pid
        os.write(master, b'c')
    scenario('echo STOP_READY; exec sleep 30', stop_and_cancel)
    scenario('echo STOP_READY; sleep 30; printf unsafe > cancelled-suffix', stop_and_cancel)
    assert not (folder/'cancelled-suffix').exists()
    checks.append('ordinary and exec Ctrl+Z explicit cancel, no suffix replay')
    def stop_resume_stop_cancel(master, process, wait):
        wait('STOP_READY')
        time.sleep(.2)
        os.write(master, b'\x1a')
        wait('Command suspended')
        os.write(master, b'r')
        deadline = time.monotonic() + 5
        while os.tcgetpgrp(master) == process.pid:
            assert time.monotonic() < deadline, 'resume did not return foreground'
            time.sleep(.02)
        os.write(master, b'\x1a')
        deadline = time.monotonic() + 5
        while os.tcgetpgrp(master) != process.pid:
            assert time.monotonic() < deadline, 'second stop did not restore helper'
            time.sleep(.02)
        os.write(master, b'c')
    scenario('echo STOP_READY; exec sleep 30', stop_resume_stop_cancel)
    checks.append('Ctrl+Z explicit resume, repeated stop and cancel')
    def stop_resume_complete(master, process, wait):
        wait('STOP_READY')
        time.sleep(.2)
        os.write(master, b'\x1a')
        wait('Command suspended')
        os.write(master, b'r')
    scenario('echo STOP_READY; sleep .6; printf finished > resumed-result', stop_resume_complete)
    assert (folder/'resumed-result').read_text() == 'finished'
    checks.append('ordinary stop resume completes same command')
    scenario('stty -echo -icanon; exit 0')
    checks.append('termios and foreground group restoration')
    print(json.dumps({'result':'PASS', 'checks':checks, 'backend':'native shell PTY', 'shell':shell, 'binary':binary, 'unit_helper':unit}))
