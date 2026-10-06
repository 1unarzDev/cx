#!/usr/bin/env python3
"""Actual colored PDF pages, keyboard/wheel navigation and terminal restoration."""
import fcntl, json, os, pathlib, pty, select, struct, subprocess, sys, tempfile, termios, time
import pyte
binary = str(pathlib.Path(sys.argv[1]).resolve())
checks = []
for mono in (False,):
    with tempfile.TemporaryDirectory(prefix='cx-source-preview-') as temporary:
        root = pathlib.Path(temporary); source = root/'files'; source.mkdir()
        objects = ['<< /Type /Catalog /Pages 2 0 R >>', '<< /Type /Pages /Kids [3 0 R 5 0 R 7 0 R] /Count 3 >>']
        for index, color in enumerate(['1 0 0', '0 1 0', '0 0 1']):
            objects.append(f'<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 100] /Resources << >> /Contents {4+index*2} 0 R >>')
            content = f'{color} rg 0 0 200 100 re f\n'
            objects.append(f'<< /Length {len(content)} >>\nstream\n{content}endstream')
        data = bytearray(b'%PDF-1.4\n'); offsets = []
        for index, body in enumerate(objects, 1):
            offsets.append(len(data)); data.extend(f'{index} 0 obj\n{body}\nendobj\n'.encode())
        xref = len(data); data.extend(f'xref\n0 {len(objects)+1}\n0000000000 65535 f \n'.encode())
        for offset in offsets: data.extend(f'{offset:010} 00000 n \n'.encode())
        data.extend(f'trailer\n<< /Size {len(objects)+1} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n'.encode())
        script = source/'book.pdf'; script.write_bytes(data)
        state = root/'state/cx'; state.mkdir(parents=True)
        name = 'viewer-restart-123-789.json'
        snapshot = dict(schema=1, expires_at=int(time.time())+300, device_ids=['local'], device=1,
            focus='Workspace', view='Files', selected=0, side_selected=0, search='',
            browser=dict(device=0, path=str(source), display_path=str(source), parent=str(root), entries=[], selected=0,
                search='', preview_scroll=0, restore_selection=str(script)), other_browser=None,
            destination_active=False, conflict=2, launch_provider=None, clipboard=None, submitted={})
        path = state/name; path.write_text(json.dumps(snapshot)); path.chmod(0o600)
        env = dict(os.environ, HOME=str(root), XDG_STATE_HOME=str(root/'state'), SHELL='/bin/sh', TERM='xterm-256color', HTTPS_PROXY='http://127.0.0.1:9')
        for key in ('TMUX', 'TMUX_PANE', 'NO_COLOR', 'CX_ASCII'): env.pop(key, None)
        if mono: env.update(NO_COLOR='1', CX_ASCII='1')
        master, slave = pty.openpty(); before = termios.tcgetattr(slave)
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 24, 80, 0, 0))
        def control():
            os.setsid(); fcntl.ioctl(slave, termios.TIOCSCTTY, 0)
        proc = subprocess.Popen([binary, 'restart', name], stdin=slave, stdout=slave, stderr=slave, env=env, preexec_fn=control, cwd=str(source))
        screen = pyte.Screen(80, 24); stream = pyte.Stream(screen)
        def read():
            if select.select([master], [], [], .05)[0]:
                stream.feed(os.read(master, 65536).decode(errors='replace'))
        def wait(predicate):
            deadline = time.monotonic()+12
            while not predicate() and time.monotonic()<deadline: read()
            assert predicate(), '\n'.join(screen.display)
        try:
            wait(lambda: any('book.pdf' in line for line in screen.display))
            os.write(master, b'\r')
            wait(lambda: any('PDF · 1/3' in line for line in screen.display))
            def pixels(color):
                return sum(1 for row in screen.buffer.values() for cell in row.values()
                    if cell.data in ('▀','▄','█') and (cell.fg == color or cell.bg == color))
            wait(lambda: pixels('ff0000') > 30)
            os.write(master, b'j')
            wait(lambda: any('PDF · 2/3' in line for line in screen.display) and pixels('00ff00') > 30)
            os.write(master, b'\x1b[6~')
            wait(lambda: any('PDF · 3/3' in line for line in screen.display) and pixels('0000ff') > 30)
            os.write(master, b'\x1b[<64;40;10M')
            wait(lambda: any('PDF · 2/3' in line for line in screen.display) and pixels('00ff00') > 30)
            os.write(master, b'k')
            wait(lambda: any('PDF · 1/3' in line for line in screen.display) and pixels('ff0000') > 30)
            os.write(master, b'\x1b')
            wait(lambda: any('book.pdf' in line for line in screen.display) and not any('PDF · 1/3' in line for line in screen.display))
            os.write(master, b'\x03'); proc.wait(timeout=5)
            assert proc.returncode == 0 and termios.tcgetattr(slave) == before
            checks.append(dict(result='PASS', distinct_visible_pages=3, page_counter=True,
                keys='j/k PageDown SGR wheel', escape_selection_and_termios=True))
        finally:
            if proc.poll() is None: proc.terminate(); proc.wait(timeout=5)
            os.close(master); os.close(slave)
print(json.dumps(dict(result='PASS', checks=checks)))
