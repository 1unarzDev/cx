#!/usr/bin/env python3
"""Render ratatui cell-grid dumps (JSON from the ignored loading_peek_capture_dump test)
to PNG with the capture_matrix palette. Unlike pyte, it honors DIM (blended toward the
background) so dimmed states can be reviewed.
Usage: python tests/render_buffer_png.py dump.json [...]   (writes dump.png beside it)
"""
import json, pathlib, re, subprocess, sys
from PIL import Image, ImageDraw, ImageFont

BG = '#171a1e'
DEFAULT = '#cbd0d7'
NAMED = {'Black': '#2a2e35', 'Red': '#dc817d', 'Green': '#91b49b', 'Yellow': '#c7b887',
         'Blue': '#8ca7cb', 'Magenta': '#b29cc8', 'Cyan': '#8cbabd', 'Gray': '#d6dce4',
         'DarkGray': '#5c6370', 'LightRed': '#e6908c', 'LightGreen': '#a3c7ad',
         'LightYellow': '#d8ca99', 'LightBlue': '#9db8dc', 'LightMagenta': '#c3add9',
         'LightCyan': '#9dcbce', 'White': '#eef1f5'}
ORDER = ['Black', 'Red', 'Green', 'Yellow', 'Blue', 'Magenta', 'Cyan', 'Gray', 'DarkGray',
         'LightRed', 'LightGreen', 'LightYellow', 'LightBlue', 'LightMagenta', 'LightCyan', 'White']


def xterm(n):
    if n < 16:
        return NAMED[ORDER[n]]
    if n < 232:
        n -= 16
        steps = [0, 95, 135, 175, 215, 255]
        return '#%02x%02x%02x' % (steps[n // 36], steps[n // 6 % 6], steps[n % 6])
    v = 8 + (n - 232) * 10
    return '#%02x%02x%02x' % (v, v, v)


def color(name, fallback):
    if name == 'Reset':
        return fallback
    if name in NAMED:
        return NAMED[name]
    m = re.fullmatch(r'Indexed\((\d+)\)', name)
    if m:
        return xterm(int(m.group(1)))
    m = re.fullmatch(r'Rgb\((\d+), (\d+), (\d+)\)', name)
    if m:
        return '#%02x%02x%02x' % tuple(int(g) for g in m.groups())
    return fallback


def blend(a, b, t):
    pa = [int(a[i:i + 2], 16) for i in (1, 3, 5)]
    pb = [int(b[i:i + 2], 16) for i in (1, 3, 5)]
    return '#%02x%02x%02x' % tuple(round(x + (y - x) * t) for x, y in zip(pa, pb))


FONTS = {}


def font(bold):
    if bold not in FONTS:
        try:
            path = subprocess.check_output(['fc-match', '-f', '%{file}', 'monospace:' + ('bold' if bold else 'regular')], text=True)
            FONTS[bold] = ImageFont.truetype(path, 16)
        except Exception:
            FONTS[bold] = ImageFont.load_default()
    return FONTS[bold]


def render(path):
    doc = json.loads(pathlib.Path(path).read_text())
    cols, rows, cells = doc['cols'], doc['rows'], doc['cells']
    cw, ch, pad = 10, 21, 12
    im = Image.new('RGB', (cols * cw + 2 * pad, rows * ch + 2 * pad), BG)
    d = ImageDraw.Draw(im)
    for i, c in enumerate(cells):
        x, y = i % cols, i // cols
        fg, bg = color(c['fg'], DEFAULT), color(c['bg'], BG)
        m = c['m']
        if 'REVERSED' in m:
            fg, bg = bg, fg
        if 'DIM' in m:
            fg = blend(fg, bg, 0.45)
        x0, y0 = pad + x * cw, pad + y * ch
        if bg != BG:
            d.rectangle((x0, y0, x0 + cw - 1, y0 + ch - 1), fill=bg)
        if c['s'].strip():
            d.text((x0, y0), c['s'], font=font('BOLD' in m), fill=fg)
        if 'UNDERLINED' in m:
            d.line((x0, y0 + ch - 2, x0 + cw - 1, y0 + ch - 2), fill=fg)
    out = pathlib.Path(path).with_suffix('.png')
    im.save(out)
    return out


if __name__ == '__main__':
    for p in sys.argv[1:]:
        print(render(p))
