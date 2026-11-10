#!/usr/bin/env python3
"""Light-background variant of tests/capture_matrix.py (theme evidence only).

Runs the unmodified capture matrix, but renders PNGs on a light terminal palette
(#fafafa background, dark default foreground, a Solarized/GNOME-like light ANSI
set) so light-theme tokens are judged against the background they target.
Usage: python capture_light.py target/debug/cx OUT_DIR [capture_matrix args...]
The caller passes --env CX_THEME=light (or exercises OSC 11 separately).
"""
import pathlib, sys

repo = pathlib.Path(__file__).resolve().parents[3]
source = (repo / 'tests/capture_matrix.py').read_text()
light = '''
PALETTE = {'default': '#24292f', 'black': '#24292f', 'red': '#b3261e', 'green': '#2b7a3a',
           'brown': '#8a6100', 'yellow': '#8a6100', 'blue': '#1f5fbf', 'magenta': '#8a3fa0',
           'cyan': '#0b7285', 'white': '#d0d7de', 'brightblack': '#6e7781', 'brightred': '#cf3a32',
           'brightgreen': '#2f8f45', 'brightyellow': '#9a6e00', 'brightblue': '#2f6fd0',
           'brightmagenta': '#9a50b0', 'brightcyan': '#127f93', 'brightwhite': '#ffffff'}
BG = '#fafafa'
'''
marker = "\n\ndef color(name, fallback):"
assert marker in source
source = source.replace(marker, '\n' + light + marker, 1)
sys.argv = [str(repo / 'tests/capture_matrix.py')] + sys.argv[1:]
code = compile(source, str(repo / 'tests/capture_matrix.py'), 'exec')
exec(code, {'__name__': '__main__', '__file__': str(repo / 'tests/capture_matrix.py')})
