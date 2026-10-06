#!/usr/bin/env python3
"""Real disposable managed-shell attach/return; isolated state and tmux server."""
import fcntl, json, os, pathlib, pty, select, struct, subprocess, tempfile, termios, time
binary = str(pathlib.Path('target/release/cx').resolve())
with tempfile.TemporaryDirectory(prefix='cx-shell-return-') as tmp:
    env = dict(os.environ, XDG_STATE_HOME=tmp, TERM='xterm-256color', LANG='C', LC_ALL='C')
    env.pop('TMUX', None)
    env.pop('TMUX_PANE', None)
    socket = str(pathlib.Path(tmp) / 'cx/managed.sock')
    try:
        subprocess.run([binary, 'new', '--directory', tmp, '--provider', 'shell',
                        '--name', 'cx-return-fixture', '--key', 'shell-return-fixture'],
                       env=env, check=True, stdout=subprocess.DEVNULL)
        master, slave = pty.openpty()
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 24, 80, 0, 0))
        before = termios.tcgetattr(slave)
        def controlling_tty():
            os.setsid()
            fcntl.ioctl(slave, termios.TIOCSCTTY, 0)
        p = subprocess.Popen([binary], stdin=slave, stdout=slave, stderr=slave,
                             env=env, preexec_fn=controlling_tty)
        def read(seconds):
            result = bytearray()
            end = time.monotonic() + seconds
            while time.monotonic() < end:
                if select.select([master], [], [], .05)[0]:
                    try:
                        result.extend(os.read(master, 65536))
                    except OSError:
                        break
            return bytes(result)
        try:
            initial = read(1)
            os.write(master, b'/cx-return-fixture\r')
            attached = read(1)
            # Synthetic output only: isolate UTF-8 transport from font rendering.
            sample = "CX_GLYPHS: ● ◆ ─ 日本語"
            command = "printf '%s\\n' '" + sample + "'"
            subprocess.run(['tmux', '-S', socket, 'send-keys', '-l', command], check=True)
            subprocess.run(['tmux', '-S', socket, 'send-keys', 'Enter'], check=True)
            glyph_output = read(.5)
            command = "printf '\\033[3mCX_ITALIC\\033[0m\\n'"
            subprocess.run(['tmux', '-S', socket, 'send-keys', '-l', command], check=True)
            subprocess.run(['tmux', '-S', socket, 'send-keys', 'Enter'], check=True)
            italic_output = read(.5)
            pane = subprocess.check_output(['tmux', '-S', socket, 'capture-pane', '-p', '-e'])
            italic_line = next(line for line in pane.splitlines()
                               if line.endswith(b'CX_ITALIC\x1b[0m') or
                                  line.endswith(b'CX_ITALIC\x1b[23m'))
            os.write(master, b'\x1d ')
            returned = read(1)
            os.write(master, b'\x03')
            p.wait(timeout=3)
            alive = subprocess.run(['tmux', '-S', socket, 'has-session'],
                                   capture_output=True).returncode == 0
            result = {'workspace_before': b'Work' in initial,
                      'return_hint_complete': b'Ctrl+] Space' in attached,
                      'utf8_with_ascii_ssh_locale': sample.encode() in glyph_output,
                      'italic_not_reverse': b'\x1b[3m' in italic_line and b'\x1b[7m' not in italic_line,
                      'workspace_returned': b'Work' in returned,
                      'selection_restored': b'cx-return-fixture' in returned,
                      'shell_survived': alive,
                      'terminal_restored': termios.tcgetattr(slave) == before,
                      'exit': p.returncode}
            print(json.dumps(result))
            assert all(v for k, v in result.items() if k != 'exit') and p.returncode == 0
        finally:
            if p.poll() is None:
                p.terminate()
                p.wait(timeout=3)
            os.close(master)
            os.close(slave)
    finally:
        subprocess.run(['tmux', '-S', socket, 'kill-server'], capture_output=True)
