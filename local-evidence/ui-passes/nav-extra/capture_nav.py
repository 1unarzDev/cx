#!/usr/bin/env python3
"""Nav-wave captures on top of tests/capture_matrix.py's isolated fixture.

usage: python capture_nav.py BINARY OUT_DIR [--sizes 40x20,...] [--env K=V]
Reuses capture_matrix's private HOME/XDG_STATE_HOME, fake ssh, owned tmux
servers (killed at exit) and masking; only the per-size flow differs. Never
opens the Network view (no host observation).
"""
import codecs, pathlib, sys, time
repo = pathlib.Path(__file__).resolve().parents[3]
source = (repo / 'tests/capture_matrix.py').read_text()
source = source[: source.rstrip().rfind('main()')]
ns = {'__file__': str(repo / 'tests/capture_matrix.py'), '__name__': 'capture_nav'}
exec(compile(source, 'capture_matrix.py', 'exec'), ns)
for name in ['fcntl', 'os', 'pty', 'select', 'struct', 'subprocess', 'termios', 'pyte']:
    globals()[name] = ns[name]
Screen, redact, render_png = ns['Screen'], ns['redact'], ns['render_png']


def capture_size(cols, rows, env):
    out = ns['out']
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', rows, cols, 0, 0))

    def control():
        os.setsid()
        fcntl.ioctl(slave, termios.TIOCSCTTY, 0)

    proc = subprocess.Popen([ns['binary']], stdin=slave, stdout=slave, stderr=slave, env=env,
                            cwd=env['HOME'], preexec_fn=control)
    screen = Screen(cols, rows)
    stream = pyte.Stream(screen)
    # Incremental decoding: large frames split UTF-8 sequences across reads.
    decoder = codecs.getincrementaldecoder('utf-8')('replace')

    def read(seconds=.35):
        end = time.monotonic() + seconds
        while time.monotonic() < end:
            if select.select([master], [], [], .03)[0]:
                try:
                    stream.feed(decoder.decode(os.read(master, 65536)))
                except OSError:
                    break

    def send(b, wait=.45):
        os.write(master, b)
        read(wait)

    def find(text):
        for y, line in enumerate(screen.display):
            x = line.find(text)
            if x >= 0:
                return x, y
        return None

    def click(x, y, button=0):
        send(f'\x1b[<{button};{x + 1};{y + 1}M'.encode(), .1)
        send(f'\x1b[<{button};{x + 1};{y + 1}m'.encode(), .6)

    results = []

    def shot(view):
        name = f'{view}-{cols}x{rows}'
        redact(screen, cols, rows)
        (out / f'{name}.txt').write_text('\n'.join(screen.display) + '\n')
        render_png(screen, cols, rows, out / f'{name}.png')
        results.append(dict(view=view, cols=cols, rows=rows, txt=f'{name}.txt', png=f'{name}.png'))

    try:
        read(6.0)
        send(b'gs', 1.5)
        send(b'\x1b', .4)   # focus devices
        send(b'gg', .4)     # All devices
        send(b'\r', 1.5)
        shot('nav-sessions')
        at = find('ros core')
        if at:
            click(at[0], at[1])          # select by tap
            shot('nav-tap-select')
            send(f'\x1b[<2;{at[0] + 1};{at[1] + 1}M'.encode(), .6)  # right click
            shot('nav-context-menu')
            send(b'\x1b', .4)
        send(b'\x10', .5)
        send(b'ros', .6)
        shot('nav-palette-query')
        send(b'\x1b', .4)
        send(b'gk', 1.0)
        shot('nav-keys')
        send(b'gf', 1.5)
        shot('nav-files')
        send(b'?', .6)
        shot('nav-help')
        send(b'\x1b', .4)
        tab = find('Sess')
        if tab:
            click(tab[0] + 1, tab[1])
            shot('nav-tab-click-sessions')
    finally:
        os.write(master, b'\x03')
        try:
            proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            proc.kill()
            proc.wait()
        os.close(master)
        os.close(slave)
    return results


ns['capture_size'] = capture_size
ns['main']()
