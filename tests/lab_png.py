#!/usr/bin/env python3
"""Render render-lab dumps (src/ui/lab/dump.rs) to PNGs plus contact sheets.

Usage: lab_png.py FRAMES_DIR [SHEET_DIR]     (needs Pillow)

FRAMES_DIR holds <stem>.json dumps and manifest.json from the `lab_capture` test.
Writes <stem>.png next to each dump (and <stem>.targets.png when the concept exposes
tap targets), then into SHEET_DIR (default: FRAMES_DIR/..):
  contact-<variant>[-<fixture>].png  rows = views, columns = sizes
  contact.png                         rows = views, columns = variants, at one size

Cells are drawn the way kitty/foot/ghostty do: box drawing, block elements and
sextants are drawn as geometry so they join seamlessly; everything else (text,
braille, symbols, CJK, emoji) comes from the font, with fontconfig fallback per glyph.
Env: CX_LAB_FONT (family, default JetBrains Mono), CX_LAB_FONT_PX (default 16),
CX_LAB_PALETTE=solarized (Solarized Dark 16-colour palette for the dark theme).
"""
import json
import math
import multiprocessing
import os
import pathlib
import subprocess
import sys

from PIL import Image, ImageDraw, ImageFont

FONT_PX = int(os.environ.get('CX_LAB_FONT_PX', '16'))
FAMILIES = [f for f in [os.environ.get('CX_LAB_FONT'), 'JetBrainsMono Nerd Font Mono',
                         'JetBrains Mono', 'DejaVu Sans Mono', 'monospace'] if f]

# Default fg/bg and a calm 16-color palette per theme (what a well-configured terminal shows).
THEMES = {
    'dark': {
        'fg': (0xd5, 0xd9, 0xe0), 'bg': (0x12, 0x15, 0x19),
        'ansi': ['#1d2127', '#e06c6c', '#8fc98f', '#e0c27a', '#7aa7e0', '#c792e8', '#6cc4d0', '#b8bec8',
                 '#5c6470', '#ff8a85', '#a8e0a0', '#f0d898', '#9cc0f5', '#dcaaf5', '#8ae0e8', '#eef1f5'],
    },
    'light': {
        'fg': (0x24, 0x29, 0x2f), 'bg': (0xfa, 0xfa, 0xf8),
        'ansi': ['#24292f', '#c4332f', '#2a7d3a', '#9a6700', '#1f5fbf', '#8a3fb8', '#0b7a86', '#6e7781',
                 '#8c959f', '#d9433f', '#2f9a48', '#b07a00', '#2b73d9', '#a050d0', '#0f94a2', '#a8b0b8'],
    },
}
# CX_LAB_PALETTE=solarized: Solarized Dark, whose brightblack (base03) is the background,
# to prove that nothing readable is drawn in brightblack in the 16-colour tier.
if os.environ.get('CX_LAB_PALETTE') == 'solarized':
    THEMES['dark'] = {
        'fg': (0x83, 0x94, 0x96), 'bg': (0x00, 0x2b, 0x36),
        'ansi': ['#073642', '#dc322f', '#859900', '#b58900', '#268bd2', '#d33682', '#2aa198', '#eee8d5',
                 '#002b36', '#cb4b16', '#586e75', '#657b83', '#839496', '#6c71c4', '#93a1a1', '#fdf6e3'],
    }
BOLD, DIM, ITALIC, UNDER, REV, HIDDEN, STRIKE = 1, 2, 4, 8, 64, 128, 256


def hexrgb(h):
    return tuple(int(h[i:i + 2], 16) for i in (1, 3, 5))


def xterm256(i):
    if i < 232:
        i -= 16
        lv = lambda v: 0 if v == 0 else 55 + v * 40
        return (lv(i // 36), lv((i // 6) % 6), lv(i % 6))
    v = 8 + (i - 232) * 10
    return (v, v, v)


def mix(a, b, t):
    return tuple(round(x + (y - x) * t) for x, y in zip(a, b))


# ------------------------------------------------------------------ fonts

def fc(query):
    try:
        return subprocess.check_output(['fc-match', '-f', '%{file}', query], text=True,
                                       stderr=subprocess.DEVNULL).strip() or None
    except (OSError, subprocess.CalledProcessError):
        return None


class Fonts:
    def __init__(self, px):
        self.px = px
        self.family = next((f for f in FAMILIES if fc(f)), 'monospace')
        self.cache, self.fallback, self.notdef = {}, {}, {}
        self.base = self.load(fc(self.family))
        asc, desc = self.base.getmetrics()
        self.cw = math.ceil(self.base.getlength('M'))
        self.ch = asc + desc
        self.base_y = asc

    def load(self, path):
        if path not in self.cache:
            try:
                if path and 'Emoji' in path:  # bitmap color font: only its strike size loads
                    f = ImageFont.truetype(path, 109)
                else:
                    f = ImageFont.truetype(path, self.px)
            except OSError:
                f = ImageFont.load_default(self.px)
            self.cache[path] = f
        return self.cache[path]

    def style_query(self, bold, italic, extra=''):
        q = self.family
        if bold:
            q += ':bold'
        if italic:
            q += ':italic'
        return q + extra

    def ink(self, font, ch):
        im = Image.new('L', (self.px * 3, self.px * 2))
        ImageDraw.Draw(im).text((0, 0), ch, font=font, fill=255)
        return im.tobytes()

    def has(self, font, ch):
        """Whether `font` has a real glyph for `ch` (not .notdef)."""
        if ch.isspace():
            return True
        key = id(font)
        if key not in self.notdef:
            self.notdef[key] = self.ink(font, '\U0010fffd')
        try:
            return self.ink(font, ch) != self.notdef[key]
        except Exception:
            return False

    def font_for(self, ch, bold, italic):
        key = (ch, bold, italic)
        if key not in self.fallback:
            styled = self.load(fc(self.style_query(bold, italic)))
            f = styled if self.has(styled, ch) else None
            if f is None:
                cp = ord(ch[0])
                for q in (self.style_query(bold, italic, f':charset={cp:x}'), f':charset={cp:x}'):
                    path = fc(q)
                    cand = self.load(path)
                    if (path and 'Emoji' in path) or self.has(cand, ch):
                        f = cand
                        break
            self.fallback[key] = f or styled
        return self.fallback[key]


# ------------------------------------------------------------------ cell geometry

BOX = " ╴╸╷┐┑╻┒┓╶─╾┌┬┭┎┰┱╺╼━┍┮┯┏┲┳╵┘┙│┤┥╽┧┪└┴┵├┼┽┟╁╅┕┶┷┝┾┿┢╆╈╹┚┛╿┦┩┃┨┫┖┸┹┞╀╃┠╂╉┗┺┻┡╄╇┣╊╋"
ARMS = {}  # char -> (up, right, down, left); 1 light, 2 heavy, 3 double
for i, c in enumerate(BOX):  # BOX is indexed by base-3 digits (up, right, down, left)
    if c != ' ':
        ARMS[c] = (i // 27, (i // 9) % 3, (i // 3) % 3, i % 3)
DOUBLE = {'═': (0, 3, 0, 3), '║': (3, 0, 3, 0), '╒': (0, 3, 1, 0), '╓': (0, 1, 3, 0), '╔': (0, 3, 3, 0),
          '╕': (0, 0, 1, 3), '╖': (0, 0, 3, 1), '╗': (0, 0, 3, 3), '╘': (1, 3, 0, 0), '╙': (3, 1, 0, 0),
          '╚': (3, 3, 0, 0), '╛': (1, 0, 0, 3), '╜': (3, 0, 0, 1), '╝': (3, 0, 0, 3), '╞': (1, 3, 1, 0),
          '╟': (3, 1, 3, 0), '╠': (3, 3, 3, 0), '╡': (1, 0, 1, 3), '╢': (3, 0, 3, 1), '╣': (3, 0, 3, 3),
          '╤': (0, 3, 1, 3), '╥': (0, 1, 3, 1), '╦': (0, 3, 3, 3), '╧': (1, 3, 0, 3), '╨': (3, 1, 0, 1),
          '╩': (3, 3, 0, 3), '╪': (1, 3, 1, 3), '╫': (3, 1, 3, 1), '╬': (3, 3, 3, 3)}
ARMS.update(DOUBLE)
ROUND = {'╭': (0, 1, 1, 0), '╮': (0, 0, 1, 1), '╰': (1, 1, 0, 0), '╯': (1, 0, 0, 1)}
DASH = {'┄': ('h', 1, 3), '┅': ('h', 2, 3), '┆': ('v', 1, 3), '┇': ('v', 2, 3), '┈': ('h', 1, 4),
        '┉': ('h', 2, 4), '┊': ('v', 1, 4), '┋': ('v', 2, 4), '╌': ('h', 1, 2), '╍': ('h', 2, 2),
        '╎': ('v', 1, 2), '╏': ('v', 2, 2)}
QUAD = {'▖': 0b0010, '▗': 0b0001, '▘': 0b1000, '▙': 0b1011, '▚': 0b1001, '▛': 0b1110, '▜': 0b1101,
        '▝': 0b0100, '▞': 0b0110, '▟': 0b0111}  # bits: TL TR BL BR


def sextant_bits(cp):
    """U+1FB00.. sextants: 6 bits (TL TR ML MR BL BR = bit0..5), skipping 0, 21, 42, 63."""
    k = cp - 0x1FB00
    n = 0
    for v in range(1, 63):
        if v in (21, 42):
            continue
        if n == k:
            return v
        n += 1
    return None


class Painter:
    def __init__(self, fonts):
        self.f = fonts
        self.cw, self.ch = fonts.cw, fonts.ch
        self.lw = max(1, round(fonts.px / 14))
        self.tiles = {}

    def arms(self, d, x0, y0, arms, fg):
        cw, ch, lw = self.cw, self.ch, self.lw
        cx, cy = x0 + cw // 2, y0 + ch // 2
        for k, w in enumerate(arms):
            if not w:
                continue
            ends = [((cx, y0), (cx, cy)), ((cx, cy), (x0 + cw, cy)),
                    ((cx, cy), (cx, y0 + ch)), ((x0, cy), (cx, cy))][k]
            vertical = k in (0, 2)
            if w == 3:  # double: two parallel strokes
                off = lw + 1
                for s in (-off, off):
                    (ax, ay), (bx, by) = ends
                    if vertical:
                        d.rectangle((ax + s - lw // 2, min(ay, by) - (off if k == 2 else 0),
                                     ax + s - lw // 2 + lw - 1, max(ay, by) + (off if k == 0 else 0)), fill=fg)
                    else:
                        d.rectangle((min(ax, bx) - (off if k == 1 else 0), ay + s - lw // 2,
                                     max(ax, bx) + (off if k == 3 else 0), ay + s - lw // 2 + lw - 1), fill=fg)
                continue
            t = lw * (2 if w == 2 else 1)
            (ax, ay), (bx, by) = ends
            if vertical:
                d.rectangle((ax - t // 2, min(ay, by) - t // 2, ax - t // 2 + t - 1, max(ay, by) + (t - 1) // 2), fill=fg)
            else:
                d.rectangle((min(ax, bx) - t // 2, ay - t // 2, max(ax, bx) + (t - 1) // 2, ay - t // 2 + t - 1), fill=fg)

    def rounded(self, d, x0, y0, arms, fg):
        cw, ch, lw = self.cw, self.ch, self.lw
        cx, cy = x0 + cw // 2, y0 + ch // 2
        r = max(2, cw // 2)
        up, right, down, left = arms
        sx = 1 if right else -1
        sy = 1 if down else -1
        # vertical stub from the cell edge to the arc, horizontal stub from the arc to the edge
        ey = y0 + ch if down else y0
        ex = x0 + cw if right else x0
        d.rectangle((cx - lw // 2, min(ey, cy + sy * r), cx - lw // 2 + lw - 1, max(ey, cy + sy * r)), fill=fg)
        d.rectangle((min(ex, cx + sx * r), cy - lw // 2, max(ex, cx + sx * r), cy - lw // 2 + lw - 1), fill=fg)
        box = (min(cx, cx + 2 * sx * r), min(cy, cy + 2 * sy * r), max(cx, cx + 2 * sx * r), max(cy, cy + 2 * sy * r))
        start = {(1, 1): 180, (-1, 1): 270, (1, -1): 90, (-1, -1): 0}[(sx, sy)]
        d.arc(box, start, start + 90, fill=fg, width=lw)

    def dashed(self, d, x0, y0, spec, fg):
        axis, w, n = spec
        t = self.lw * w
        cx, cy = x0 + self.cw // 2, y0 + self.ch // 2
        if axis == 'h':
            seg = self.cw / n
            for i in range(n):
                a = x0 + i * seg
                d.rectangle((round(a), cy - t // 2, round(a + seg * 0.6) - 1, cy - t // 2 + t - 1), fill=fg)
        else:
            seg = self.ch / n
            for i in range(n):
                a = y0 + i * seg
                d.rectangle((cx - t // 2, round(a), cx - t // 2 + t - 1, round(a + seg * 0.6) - 1), fill=fg)

    def block(self, d, x0, y0, ch, fg, bg):
        """Block elements U+2580..259F; returns False if not one."""
        cw, h = self.cw, self.ch
        o = ord(ch)
        rect = lambda a, b, c, e: d.rectangle((x0 + a, y0 + b, x0 + c - 1, y0 + e - 1), fill=fg)
        if ch == '▀':
            rect(0, 0, cw, h // 2)
        elif 0x2581 <= o <= 0x2588:  # lower eighths
            n = o - 0x2580
            rect(0, h - round(h * n / 8), cw, h)
        elif 0x2589 <= o <= 0x258F:  # left eighths (7/8 .. 1/8)
            n = 8 - (o - 0x2588)
            rect(0, 0, max(1, round(cw * n / 8)), h)
        elif ch == '▐':
            rect(cw // 2, 0, cw, h)
        elif ch in '░▒▓':
            t = {'░': 0.25, '▒': 0.5, '▓': 0.75}[ch]
            d.rectangle((x0, y0, x0 + cw - 1, y0 + h - 1), fill=mix(bg, fg, t))
        elif ch == '▔':
            rect(0, 0, cw, max(1, round(h / 8)))
        elif ch == '▕':
            rect(cw - max(1, round(cw / 8)), 0, cw, h)
        elif ch in QUAD:
            b = QUAD[ch]
            hx, hy = cw // 2, h // 2
            if b & 8:
                rect(0, 0, hx, hy)
            if b & 4:
                rect(hx, 0, cw, hy)
            if b & 2:
                rect(0, hy, hx, h)
            if b & 1:
                rect(hx, hy, cw, h)
        elif 0x1FB00 <= o <= 0x1FB3B:
            b = sextant_bits(o)
            if b is None:
                return False
            xs, ys = [0, cw // 2, cw], [0, round(h / 3), round(2 * h / 3), h]
            for i in range(6):
                if b >> i & 1:
                    c, r = i % 2, i // 2
                    rect(xs[c], ys[r], xs[c + 1], ys[r + 1])
        else:
            return False
        return True

    def glyph(self, im, x0, y0, sym, width, fg, bold, italic):
        """Paste a cached glyph tile (alpha mask, or RGBA for color emoji)."""
        key = (sym, width, bold, italic)
        tile = self.tiles.get(key)
        if tile is None:
            tile = self.tiles[key] = self.make_tile(sym, width, bold, italic)
        img, dx, dy = tile
        if img is None:
            return
        if img.mode == 'RGBA':
            im.paste(img, (x0 + dx, y0 + dy), img)
        else:
            im.paste(fg, (x0 + dx, y0 + dy, x0 + dx + img.width, y0 + dy + img.height), img)

    def make_tile(self, sym, width, bold, italic):
        f = self.f.font_for(sym, bold, italic)
        box_w, ch = self.cw * width, self.ch
        if f.size == 109:  # bitmap color emoji: render at its strike size, fit to the cells
            tmp = Image.new('RGBA', (160, 140), (0, 0, 0, 0))
            ImageDraw.Draw(tmp).text((0, 0), sym, font=f, embedded_color=True)
            bb = tmp.getbbox()
            if not bb:
                return (None, 0, 0)
            tmp = tmp.crop(bb)
            sc = min(box_w / tmp.width, (ch - 2) / tmp.height)
            tmp = tmp.resize((max(1, round(tmp.width * sc)), max(1, round(tmp.height * sc))), Image.LANCZOS)
            return (tmp, (box_w - tmp.width) // 2, (ch - tmp.height) // 2)
        adv = f.getlength(sym)
        if adv > box_w + 0.5 and getattr(f, 'path', None):  # too wide (fallback fonts): shrink
            f = ImageFont.truetype(f.path, max(6, int(f.size * box_w / adv)))
            adv = f.getlength(sym)
        # Overhang margin of one cell on each side so italics and wide strokes are not clipped.
        m = self.cw
        mask = Image.new('L', (box_w + 2 * m, ch + 2 * m))
        x = m + (box_w - adv) / 2 if width > 1 or f is not self.f.base else m
        ImageDraw.Draw(mask).text((x, m + self.f.base_y), sym, font=f, fill=255, anchor='ls')
        bb = mask.getbbox()
        if not bb:
            return (None, 0, 0)
        return (mask.crop(bb), bb[0] - m, bb[1] - m)


def render(dump, fonts, p=None):
    theme = THEMES.get(dump.get('theme', 'dark'), THEMES['dark'])
    dfg, dbg = theme['fg'], theme['bg']
    ansi = [hexrgb(c) for c in theme['ansi']]

    def color(c, default):
        if c is None:
            return default
        if isinstance(c, int):
            return ansi[c] if c < 16 else xterm256(c)
        return hexrgb(c)

    p = p or Painter(fonts)
    cw, ch = p.cw, p.ch
    pad = cw
    cols, rows = dump['cols'], dump['rows']
    im = Image.new('RGB', (cols * cw + 2 * pad, rows * ch + 2 * pad), dbg)
    d = ImageDraw.Draw(im)
    text_cells = []
    for y, row in enumerate(dump['cells']):
        for x, cell in enumerate(row):
            sym, fg, bg, mods = cell[:4]
            width = cell[4] if len(cell) > 4 else 1
            if sym == '':
                continue  # continuation of a wide glyph
            f, b = color(fg, dfg), color(bg, dbg)
            if mods & REV:
                f, b = b, f
            if mods & DIM:
                f = mix(f, b, 0.5)
            x0, y0 = pad + x * cw, pad + y * ch
            if b != dbg:
                d.rectangle((x0, y0, x0 + cw * width - 1, y0 + ch - 1), fill=b)
            if mods & HIDDEN or not sym.strip():
                pass
            elif sym in ROUND:
                p.rounded(d, x0, y0, ROUND[sym], f)
            elif sym in DASH:
                p.dashed(d, x0, y0, DASH[sym], f)
            elif sym in ARMS:
                p.arms(d, x0, y0, ARMS[sym], f)
            elif sym in '╱╲╳':
                if sym in '╱╳':
                    d.line((x0, y0 + ch, x0 + cw, y0), fill=f, width=p.lw)
                if sym in '╲╳':
                    d.line((x0, y0, x0 + cw, y0 + ch), fill=f, width=p.lw)
            elif len(sym) == 1 and p.block(d, x0, y0, sym, f, b):
                pass
            else:
                text_cells.append((x0, y0, sym, width, f, bool(mods & BOLD), bool(mods & ITALIC)))
            if mods & UNDER:
                uy = y0 + fonts.base_y + 2
                d.rectangle((x0, uy, x0 + cw * width - 1, uy + p.lw - 1), fill=f)
            if mods & STRIKE:
                sy = y0 + ch // 2
                d.rectangle((x0, sy, x0 + cw * width - 1, sy + p.lw - 1), fill=f)
    # Text after all backgrounds so glyph overhang (italics, wide fallbacks) is not clipped.
    for x0, y0, sym, width, f, bold, italic in text_cells:
        p.glyph(im, x0, y0, sym, width, f, bold, italic)
    return im, (pad, cw, ch)


def targets_overlay(im, dump, geo):
    pad, cw, ch = geo
    ov = im.convert('RGBA')
    layer = Image.new('RGBA', ov.size, (0, 0, 0, 0))
    d = ImageDraw.Draw(layer)
    for t in dump['targets']:
        x0, y0 = pad + t['x'] * cw, pad + t['y'] * ch
        x1, y1 = x0 + t['w'] * cw - 1, y0 + t['h'] * ch - 1
        small = t['h'] < 1 or t['w'] < 3
        d.rectangle((x0, y0, x1, y1), fill=(255, 80, 80, 40) if small else (80, 200, 255, 36),
                    outline=(255, 80, 80, 200) if small else (80, 200, 255, 200))
    return Image.alpha_composite(ov, layer).convert('RGB')


# ------------------------------------------------------------------ contact sheets

def label_font(px, bold=False):
    path = fc('DejaVu Sans:bold' if bold else 'DejaVu Sans') or fc('sans')
    try:
        return ImageFont.truetype(path, px)
    except OSError:
        return ImageFont.load_default(px)


def sheet(title, subtitle, rows, cols, cell, path, box=(440, 280)):
    """rows/cols: labels; cell(r, c) -> (Image|None, caption)."""
    bw, bh = box
    gap, lab_h, row_lab_w, head_h = 18, 22, 150, 64
    W = row_lab_w + len(cols) * (bw + gap) + gap
    H = head_h + lab_h + len(rows) * (bh + lab_h + gap) + gap
    out = Image.new('RGB', (W, H), (0x0c, 0x0e, 0x11))
    d = ImageDraw.Draw(out)
    f, fb, fh = label_font(13), label_font(14, True), label_font(20, True)
    d.text((gap, 14), title, font=fh, fill=(0xee, 0xf0, 0xf4))
    d.text((gap, 40), subtitle, font=f, fill=(0x8b, 0x93, 0xa1))
    for c, name in enumerate(cols):
        d.text((row_lab_w + gap + c * (bw + gap), head_h), name, font=fb, fill=(0xc8, 0xcd, 0xd5))
    for r, rname in enumerate(rows):
        y = head_h + lab_h + r * (bh + lab_h + gap)
        d.text((gap, y + lab_h + bh // 2 - 8), rname, font=fb, fill=(0xc8, 0xcd, 0xd5))
        for c in range(len(cols)):
            x = row_lab_w + gap + c * (bw + gap)
            img, cap = cell(r, c)
            if img is None:
                d.rectangle((x, y + lab_h, x + bw - 1, y + lab_h + bh - 1), outline=(0x2a, 0x2e, 0x35))
                continue
            th = img.copy()
            th.thumbnail((bw, bh), Image.LANCZOS, reducing_gap=2.0)
            out.paste(th, (x, y + lab_h))
            d.rectangle((x - 1, y + lab_h - 1, x + th.width, y + lab_h + th.height), outline=(0x2a, 0x2e, 0x35))
            d.text((x, y + 3), cap, font=f, fill=(0xe3, 0xc2, 0x75) if '!' in cap else (0x8b, 0x93, 0xa1))
    out.save(path)
    return path


_P = None


def _init():
    global _P
    _P = Painter(Fonts(FONT_PX))


def _render_one(job):
    frames, stem = job
    dump = json.loads((frames / f'{stem}.json').read_text())
    im, geo = render(dump, _P.f, _P)
    im.save(frames / f'{stem}.png')
    if dump.get('targets'):
        targets_overlay(im, dump, geo).save(frames / f'{stem}.targets.png')
    return f'font {_P.f.family} {FONT_PX}px, cell {_P.cw}x{_P.ch}'


def main():
    if len(sys.argv) not in (2, 3):
        sys.exit(__doc__)
    frames = pathlib.Path(sys.argv[1])
    out = pathlib.Path(sys.argv[2]) if len(sys.argv) == 3 else frames.parent
    man = json.loads((frames / 'manifest.json').read_text())
    stems = [e['stem'] for e in man['entries']]
    with multiprocessing.Pool(min(8, os.cpu_count() or 1), initializer=_init) as pool:
        for line in pool.imap_unordered(_render_one, [(frames, st) for st in stems[:1]]):
            print(line, file=sys.stderr)
        list(pool.imap_unordered(_render_one, [(frames, st) for st in stems[1:]], chunksize=4))
    imgs = {(e['view'], e['cols'], e['rows'], e['variant'], e['fixture']):
            (Image.open(frames / f"{e['stem']}.png"), e) for e in man['entries']}

    views = list(dict.fromkeys(e['view'] for e in man['entries']))
    sizes = list(dict.fromkeys((e['cols'], e['rows']) for e in man['entries']))
    variants, fixtures = man['variants'], man['fixtures']
    concept, summary = man['concept'], man.get('summary', '')
    round_name = out.name

    def caption(e, base):
        lint = e['lint']
        bad = lint['non_ascii'] + lint['color_leaks']
        return base + (f"  ! ascii {lint['non_ascii']} color {lint['color_leaks']}" if bad else '')

    written = []
    for fx in fixtures:
        sfx = '' if len(fixtures) == 1 and fx == 'typical' else f'-{fx}'
        for v in variants:
            def cell(r, c, v=v, fx=fx):
                hit = imgs.get((views[r], *sizes[c], v, fx))
                return (hit[0], caption(hit[1], f'{sizes[c][0]}x{sizes[c][1]}')) if hit else (None, '')
            written.append(sheet(f'{concept} · {v} · {fx}', f'{summary}   [{round_name}]', views,
                                 [f'{w}x{h}' for w, h in sizes], cell, out / f'contact-{v}{sfx}.png'))
        pick = (80, 24) if (80, 24) in sizes else sizes[0]

        def cell2(r, c, fx=fx):
            hit = imgs.get((views[r], *pick, variants[c], fx))
            return (hit[0], caption(hit[1], variants[c])) if hit else (None, '')
        written.append(sheet(f'{concept} · {pick[0]}x{pick[1]} · {fx}', f'{summary}   [{round_name}]', views,
                             variants, cell2, out / f'contact{sfx}.png', box=(560, 340)))
    for w in written:
        print(w)


if __name__ == '__main__':
    main()
