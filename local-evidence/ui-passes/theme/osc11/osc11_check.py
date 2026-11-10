import os, pty, select, struct, fcntl, termios, subprocess, tempfile, time, sys, pathlib, re
binary = sys.argv[1]

def run(mode):
    with tempfile.TemporaryDirectory(prefix='cx-osc11-') as tmp:
        root = pathlib.Path(tmp)
        home = root / 'home'; home.mkdir()
        state = root / 'state'; state.mkdir()
        tmuxdir = root / 'tmux'; tmuxdir.mkdir()
        env = {k: v for k, v in os.environ.items() if k not in ('TMUX', 'TMUX_PANE', 'NO_COLOR', 'CX_ASCII', 'COLORTERM', 'CX_THEME', 'COLORFGBG', 'TERM_PROGRAM', 'KITTY_WINDOW_ID', 'SSH_AUTH_SOCK')}
        env.update(HOME=str(home), XDG_STATE_HOME=str(state), TMUX_TMPDIR=str(tmuxdir), TERM='xterm-256color',
                   LANG='C.UTF-8', LC_ALL='C.UTF-8', HTTPS_PROXY='http://127.0.0.1:9', SHELL='/bin/sh')
        master, slave = pty.openpty()
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 24, 80, 0, 0))
        def ctl():
            os.setsid(); fcntl.ioctl(slave, termios.TIOCSCTTY, 0)
        t0 = time.monotonic()
        p = subprocess.Popen([binary], stdin=slave, stdout=slave, stderr=slave, env=env, cwd=str(home), preexec_fn=ctl)
        out = b''; answered_at = None; query_at = None; alt_at = None
        try:
            end = time.monotonic() + 4
            while time.monotonic() < end:
                if select.select([master], [], [], .02)[0]:
                    try: chunk = os.read(master, 65536)
                    except OSError: break
                    out += chunk
                    if alt_at is None and b'\x1b[?1049h' in out:
                        alt_at = time.monotonic() - t0
                    if query_at is None and b'\x1b]11;?' in out:
                        query_at = time.monotonic() - t0
                        reply = {'light': b'\x1b]11;rgb:fafa/fafa/fafa\x07\x1b[?62;22c',
                                 'dark': b'\x1b]11;rgb:1e1e/1e1e/1e1e\x1b\\\x1b[?62;22c'}.get(mode)
                        if reply:
                            os.write(master, reply)
                        elif mode == 'late':
                            answered_at = time.monotonic() + 0.4
                if answered_at and time.monotonic() >= answered_at:
                    os.write(master, b'\x1b]11;rgb:fafa/fafa/fafa\x07\x1b[?62;22c'); answered_at = None
        finally:
            os.write(master, b'\x03'); time.sleep(.3)
            p.kill(); p.wait()
            sock = state / 'cx/managed.sock'
            if sock.exists():
                subprocess.run(['tmux', '-S', str(sock), 'kill-server'], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            os.close(master); os.close(slave)
        alt = out.find(b'\x1b[?1049h')
        light = out.count(b'38;5;30;')
        dark = out.count(b'38;5;80;')
        # stray keystrokes would show as typed text, e.g. a search/filter with "rgb"
        stray = b'rgb' in out[alt:] if alt >= 0 else None
        print(f'{mode}: altscreen_at={alt_at and round(alt_at,3)} query_at={query_at and round(query_at,3)} query_before_altscreen={0 <= out.find(b"]11;?") < alt} accent_light={light} accent_dark={dark} stray_rgb_echo={stray}')

for m in ['light', 'dark', 'none', 'late']:
    run(m)
