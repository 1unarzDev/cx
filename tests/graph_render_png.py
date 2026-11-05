#!/usr/bin/env python3
"""Render graph-core style dumps (<name>.json from CX_GRAPH_EVIDENCE) to PNG.

Usage: python tests/graph_render_png.py <dir>   (needs Pillow)
Each <name>.json is {"cols","rows","cells":[[[symbol, fg, bg, bold, dim, reversed], ...], ...]}
with fg/bg as "rrggbb", a named ANSI color, or null (terminal default).
Box-drawing glyphs are drawn as cell-spanning strokes so lines join like a terminal.
"""
import json, pathlib, subprocess, sys
from PIL import Image, ImageDraw, ImageFont

FG = '#cbd0d7'
BG = '#16191d'
NAMED = {'black': '#2a2e35', 'red': '#dc817d', 'green': '#91b49b', 'yellow': '#c7b887', 'blue': '#8ca7cb',
         'magenta': '#b29cc8', 'cyan': '#8cbabd', 'white': '#d6dce4', 'gray': '#9aa0a8', 'darkgray': '#5c6370'}
CW, CH, PAD = 10, 21, 14


def font_path(style):
    try:
        return subprocess.check_output(['fc-match', '-f', '%{file}', f'monospace:{style}'], text=True)
    except Exception:
        return None


FONTS = {}


def font(bold, ch=' '):
    """Monospace font, or a fallback that has `ch` (fc-match by charset) for symbols."""
    key = (bold, ch if ord(ch) > 0x2000 else '')
    if key not in FONTS:
        style = 'bold' if bold else 'regular'
        if key[1]:
            style += f':charset={ord(ch):x}'
        p = font_path(style)
        FONTS[key] = ImageFont.truetype(p, 16) if p else ImageFont.load_default()
    return FONTS[key]


def color(c, fallback):
    if c is None:
        return fallback
    if c in NAMED:
        return NAMED[c]
    if len(c) == 6:
        return '#' + c
    return fallback


def mix(a, b, t):
    a = [int(a[i:i + 2], 16) for i in (1, 3, 5)]
    b = [int(b[i:i + 2], 16) for i in (1, 3, 5)]
    return '#' + ''.join(f'{round(x + (y - x) * t):02x}' for x, y in zip(a, b))


# arms: (up, right, down, left) weights, 1 light 2 heavy; dashed variants drawn with gaps.
LIGHT = {'─': (0, 1, 0, 1), '│': (1, 0, 1, 0), '┌': (0, 1, 1, 0), '┐': (0, 0, 1, 1), '└': (1, 1, 0, 0), '┘': (1, 0, 0, 1),
         '├': (1, 1, 1, 0), '┤': (1, 0, 1, 1), '┬': (0, 1, 1, 1), '┴': (1, 1, 0, 1), '┼': (1, 1, 1, 1)}
BOX = " ╴╸╷┐┑╻┒┓╶─╾┌┬┭┎┰┱╺╼━┍┮┯┏┲┳╵┘┙│┤┥╽┧┪└┴┵├┼┽┟╁╅┕┶┷┝┾┿┢╆╈╹┚┛╿┦┩┃┨┫┖┸┹┞╀╃┠╂╉┗┺┻┡╄╇┣╊╋"
ARMS = {}
for i, ch in enumerate(BOX):
    if ch != ' ':
        ARMS[ch] = (i // 27, (i // 9) % 3, (i // 3) % 3, i % 3)
ROUND = {'╭': (0, 1, 1, 0), '╮': (0, 0, 1, 1), '╰': (1, 1, 0, 0), '╯': (1, 0, 0, 1)}
DASH = {'┄': ('h', 1, 3), '┆': ('v', 1, 3), '┈': ('h', 1, 4), '┊': ('v', 1, 4), '╌': ('h', 1, 2), '╎': ('v', 1, 2)}


def stroke(d, x0, y0, arms, fg):
    cx, cy = x0 + CW // 2, y0 + CH // 2
    for k, w in enumerate(arms):
        if not w:
            continue
        lw = 3 if w == 2 else 1
        if k == 0:
            d.line((cx, y0, cx, cy), fill=fg, width=lw)
        elif k == 1:
            d.line((cx, cy, x0 + CW, cy), fill=fg, width=lw)
        elif k == 2:
            d.line((cx, cy, cx, y0 + CH), fill=fg, width=lw)
        else:
            d.line((x0, cy, cx, cy), fill=fg, width=lw)


def rounded(d, x0, y0, arms, fg):
    cx, cy = x0 + CW // 2, y0 + CH // 2
    r = 4
    up, right, down, left = arms
    if right and down:
        d.line((cx, y0 + CH, cx, cy + r), fill=fg); d.line((cx + r, cy, x0 + CW, cy), fill=fg)
        d.arc((cx, cy, cx + 2 * r, cy + 2 * r), 180, 270, fill=fg)
    elif left and down:
        d.line((cx, y0 + CH, cx, cy + r), fill=fg); d.line((x0, cy, cx - r, cy), fill=fg)
        d.arc((cx - 2 * r, cy, cx, cy + 2 * r), 270, 360, fill=fg)
    elif up and right:
        d.line((cx, y0, cx, cy - r), fill=fg); d.line((cx + r, cy, x0 + CW, cy), fill=fg)
        d.arc((cx, cy - 2 * r, cx + 2 * r, cy), 90, 180, fill=fg)
    else:
        d.line((cx, y0, cx, cy - r), fill=fg); d.line((x0, cy, cx - r, cy), fill=fg)
        d.arc((cx - 2 * r, cy - 2 * r, cx, cy), 0, 90, fill=fg)


def dashed(d, x0, y0, spec, fg):
    axis, w, n = spec
    cx, cy = x0 + CW // 2, y0 + CH // 2
    if axis == 'h':
        seg = CW / n
        for i in range(n):
            a = x0 + i * seg
            d.line((a + 1, cy, a + seg * 0.6, cy), fill=fg, width=w)
    else:
        seg = CH / n
        for i in range(n):
            a = y0 + i * seg
            d.line((cx, a + 1, cx, a + seg * 0.6), fill=fg, width=w)


def render(dump, path):
    cols, rows = dump['cols'], dump['rows']
    im = Image.new('RGB', (cols * CW + 2 * PAD, rows * CH + 2 * PAD), BG)
    d = ImageDraw.Draw(im)
    for y, row in enumerate(dump['cells']):
        for x, (sym, fg, bg, bold, dim, rev) in enumerate(row):
            f = color(fg, FG)
            b = color(bg, BG)
            if dim:
                f = mix(f, BG, 0.45)
            if rev:
                f, b = b, f
            x0, y0 = PAD + x * CW, PAD + y * CH
            if b != BG:
                d.rectangle((x0, y0, x0 + CW - 1, y0 + CH - 1), fill=b)
            if sym in ROUND:
                rounded(d, x0, y0, ROUND[sym], f)
            elif sym in DASH:
                dashed(d, x0, y0, DASH[sym], f)
            elif sym in ARMS:
                stroke(d, x0, y0, ARMS[sym], f)
            elif sym.strip():
                d.text((x0, y0), sym, font=font(bold, sym[0]), fill=f)
    im.save(path)


def main():
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    root = pathlib.Path(sys.argv[1])
    for j in sorted(root.glob('*.json')):
        render(json.loads(j.read_text()), j.with_suffix('.png'))
        print(j.with_suffix('.png').name)


if __name__ == '__main__':
    main()
