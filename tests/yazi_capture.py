#!/usr/bin/env python3
"""Capture the cx Files view in the same states as the yazi reference, side by side.

usage: yazi_capture.py BINARY ROUND [--ref DIR] [--only NAME,...] [--no-theme]

The reference (~/.cache/cx-yazi-ref, see its NOTES.md) holds the user's real yazi in
fifteen states at 120x40 and 80x24. This runs cx on a private HOME with a copy of the same
fixture tree at the same relative place (~/.cache/cx-yazi-ref/tree), reads a copy of the
reference's yazi config through XDG_CONFIG_HOME (never ~/.config), drives the same keys,
and writes <state>-<cols>x<rows>.{txt,json,png} plus <state>-<cols>x<rows>-sxs.png (yazi
left, cx right) into ~/.cache/cx-files-yazi/<ROUND>/. Needs pyte and pillow.
"""
import argparse, json, os, pathlib, shutil, sys, time

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import perf_harness as ph  # noqa: E402
import pty_lib  # noqa: E402

SP = b' '
STATES = [
    ('01-initial', '.', []),
    ('02-hover-code', 'src/main.rs', []),
    ('03-hover-markdown', 'README.md', []),
    ('04-hover-image', 'assets/photo.png', []),
    ('05-which-c', '.', [b'c']),
    ('06-select-3', '.', [SP, SP, SP]),
    ('07-visual-v', '.', [b'v', b'j', b'j']),
    ('08-help', '.', [b'~']),
    ('09-which-g', '.', [b'g']),
    ('10-yank-hover', '.', [SP, SP, b'y', b'j']),
    ('11-cut-hover', '.', [SP, b'x', b'j']),
    ('12-filter-prompt', '.', [b'f']),
    ('13-filter-typed', '.', [b'f', b'm', b'd']),
    ('14-which-comma', '.', [b',']),
    ('15-hidden', '.', [b'.']),
]


def settle(v, quiet=0.35, cap=4.0):
    end, last = time.time() + cap, time.time()
    while time.time() < end:
        if v.pty.read(0.05) is not None:
            last = time.time()
        elif time.time() - last > quiet:
            return


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('binary')
    ap.add_argument('round')
    ap.add_argument('--ref', default=str(pathlib.Path.home() / '.cache/cx-yazi-ref'))
    ap.add_argument('--only', default='')
    ap.add_argument('--no-theme', action='store_true', help='no yazi config: built-in look')
    ap.add_argument('--sizes', default='120x40,80x24')
    a = ap.parse_args()
    ref = pathlib.Path(a.ref)
    out = pathlib.Path.home() / '.cache/cx-files-yazi' / a.round
    out.mkdir(parents=True, exist_ok=True)
    only = {s for s in a.only.split(',') if s}
    from PIL import Image
    for size in a.sizes.split(','):
        cols, rows = map(int, size.split('x'))
        ph.COLS, ph.ROWS = cols, rows
        for name, start, keys in STATES:
            if only and name not in only:
                continue
            env = ph.Env(str(pathlib.Path(a.binary).resolve()))
            try:
                tree = env.root / '.cache/cx-yazi-ref/tree'
                shutil.copytree(ref / 'tree', tree, symlinks=True)
                if not a.no_theme:
                    shutil.copytree(ref / 'config/yazi', env.root / 'xdg/yazi', symlinks=True)
                    env.env['XDG_CONFIG_HOME'] = str(env.root / 'xdg')
                env.env['COLORTERM'] = 'truecolor'
                project = tree / 'project'
                target = (project / start).resolve()
                directory = target if target.is_dir() else target.parent
                select = None if target.is_dir() else target
                if select is None:
                    first = sorted(p for p in directory.iterdir() if p.is_dir() and not p.name.startswith('.'))
                    select = first[0] if first else None
                browser = ph.browser_snapshot(directory, select or directory)
                browser['display_path'] = '~/' + str(directory.relative_to(env.root))
                v = ph.Viewer(env, 'Files', browser)
                try:
                    v.wait(lambda: 'NOR' in v.text() or 'SEL' in v.text(), timeout=15, what='files')
                    settle(v, 0.6, 5)
                    for k in keys:
                        os.write(v.master, k)
                        settle(v, 0.3, 3)
                    settle(v, 0.5, 3)
                    stem = out / f'{name}-{cols}x{rows}'
                    data = pty_lib.dump(v.screen)
                    data['cursor'] = [v.screen.cursor.x, v.screen.cursor.y, not v.screen.cursor.hidden]
                    stem.with_suffix('.json').write_text(json.dumps(data))
                    stem.with_suffix('.txt').write_text(v.text() + '\n')
                    pty_lib.png(data, stem.with_suffix('.png'))
                    refpng = ref / 'caps' / f'{name}-{cols}x{rows}.png'
                    if refpng.exists():
                        l, r = Image.open(refpng), Image.open(stem.with_suffix('.png'))
                        sxs = Image.new('RGB', (l.width + r.width + 8, max(l.height, r.height)), (60, 60, 60))
                        sxs.paste(l, (0, 0))
                        sxs.paste(r, (l.width + 8, 0))
                        sxs.save(out / f'{name}-{cols}x{rows}-sxs.png')
                    print(stem.name, flush=True)
                finally:
                    v.close()
            finally:
                env.cleanup()


if __name__ == '__main__':
    main()
