#!/usr/bin/env python3
"""Shared PTY driver and PNG export for the cx PTY tools (capture_matrix, perf_harness,
files_bench). One pyte screen model; PNGs go through lab_png.py, the single renderer.

  python tests/pty_lib.py png DUMP.json [...]   # lab-format cell dumps -> DUMP.png

Needs pyte (and Pillow for PNGs).
"""
import fcntl, json, os, pathlib, pty, select, struct, subprocess, sys, termios

import pyte

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))


class Screen(pyte.Screen):
    """A pyte screen that never answers device-status queries or records titles."""

    def report_device_status(self, *args, **kwargs):
        pass

    def set_title(self, *args, **kwargs):
        pass


class Pty:
    """One process on a private PTY of cols x rows (its controlling terminal), with a screen."""

    def __init__(self, argv, env, cwd, cols, rows):
        self.master, self.slave = pty.openpty()
        self.termios_before = termios.tcgetattr(self.slave)
        fcntl.ioctl(self.slave, termios.TIOCSWINSZ, struct.pack('HHHH', rows, cols, 0, 0))

        def tty():
            os.setsid()
            fcntl.ioctl(self.slave, termios.TIOCSCTTY, 0)
        self.proc = subprocess.Popen(argv, env=env, cwd=str(cwd), stdin=self.slave, stdout=self.slave,
                                     stderr=self.slave, preexec_fn=tty)
        self.screen = Screen(cols, rows)
        self.stream = pyte.Stream(self.screen)

    def read(self, timeout):
        """Feed one output chunk to the screen and return it; None when nothing arrived."""
        if not select.select([self.master], [], [], max(timeout, 0))[0]:
            return None
        try:
            data = os.read(self.master, 1 << 16)
        except OSError:
            return None
        self.stream.feed(data.decode('utf-8', 'replace'))
        return data

    def write(self, data):
        os.write(self.master, data)

    def text(self):
        return '\n'.join(self.screen.display)

    def close(self, quit_key=b'\x03', timeout=5):
        """Send quit_key, wait, kill if needed. Reports a clean exit and restored termios."""
        result = dict(clean_exit=False, termios_restored=False)
        try:
            if self.proc.poll() is None:
                os.write(self.master, quit_key)
                try:
                    self.proc.wait(timeout=timeout)
                    result['clean_exit'] = self.proc.returncode == 0
                    result['termios_restored'] = termios.tcgetattr(self.slave) == self.termios_before
                except subprocess.TimeoutExpired:
                    pass
        except OSError:
            pass
        finally:
            if self.proc.poll() is None:
                self.proc.kill()
                self.proc.wait()
            os.close(self.master)
            os.close(self.slave)
        return result


# pyte color names -> palette indices (lab_png's dump format: None, 0..255 or "#rrggbb").
NAMED = {name: i for i, name in enumerate(['black', 'red', 'green', 'brown', 'blue', 'magenta', 'cyan', 'white'])}
NAMED['yellow'] = 3


def color(name):
    if name == 'default':
        return None
    bright = name.startswith('bright')
    base = name[6:] if bright else name
    if base in NAMED:
        return NAMED[base] + (8 if bright else 0)
    if len(name) == 6:
        return '#' + name
    return None


def dump(screen, theme='dark'):
    """A pyte screen as a lab_png cell dump. pyte drops SGR 2 (dim), so no cell is dim."""
    cells = []
    for y in range(screen.lines):
        line, row = screen.buffer[y], []
        for x in range(screen.columns):
            c = line[x]
            mods = (1 if c.bold else 0) | (4 if c.italics else 0) | (8 if c.underscore else 0) \
                | (64 if c.reverse else 0) | (256 if c.strikethrough else 0)
            cell = [c.data, color(c.fg), color(c.bg), mods]
            if c.data and x + 1 < screen.columns and line[x + 1].data == '':
                cell.append(2)
            row.append(cell)
        cells.append(row)
    return dict(cols=screen.columns, rows=screen.lines, theme=theme, cells=cells)


_FONTS = None


def png(source, path, theme='dark'):
    """Render a pyte screen or a lab-format dump dict to a PNG through lab_png."""
    global _FONTS
    import lab_png
    if _FONTS is None:
        _FONTS = lab_png.Fonts(lab_png.FONT_PX)
    data = source if isinstance(source, dict) else dump(source, theme)
    image, _ = lab_png.render(data, _FONTS)
    image.save(path)


if __name__ == '__main__':
    if len(sys.argv) < 3 or sys.argv[1] != 'png':
        sys.exit(__doc__)
    for name in sys.argv[2:]:
        src = pathlib.Path(name)
        png(json.loads(src.read_text()), src.with_suffix('.png'))
