#!/usr/bin/env python3
"""Compare two tests/files_bench.py result JSONs and print a regression/improvement table.

usage: files_bench_compare.py BASE.json NEW.json [--threshold PCT] [--min-ms MS] [--filter REGEX]
                              [--all] [--gate]

A metric regresses when its median (or p95, for metrics with at least 10 samples) moves the wrong
way by more than --threshold percent (default 10) and, for millisecond metrics, by more than
--min-ms (default 2 ms) absolute, so jitter on tiny values is ignored. Metrics that are not
timings (final counts, end reachability, bytes) compare by median only, with no absolute floor.
A metric that had samples in BASE and has none in NEW (every attempt failed) is a regression.
The table lists regressions, improvements and new/missing metrics; --all also lists unchanged.
--gate exits 1 when any regression is found (for integration gating); without it, exit 0.
"""
import argparse, json, re, sys


def load(path):
    with open(path) as f:
        data = json.load(f)
    if data.get('harness') != 'tests/files_bench.py' or 'metrics' not in data:
        sys.exit(f'{path}: not a files_bench result')
    return data


def change(base, new):
    if base in (None, 0) or new is None:
        return None
    return (new - base) / abs(base) * 100


def worse(better, base, new):
    if better == 'lower':
        return new > base
    if better == 'higher':
        return new < base
    return False


def classify(name, b, n, threshold, min_ms):
    """Returns (status, detail) for one metric."""
    if b is None:
        return 'new', ''
    if n is None:
        return 'missing', ''
    better = b.get('better', 'lower')
    if b.get('median') is not None and n.get('median') is None:
        return 'regression', 'all attempts failed'
    if b.get('median') is None or n.get('median') is None:
        return 'unchanged', 'no samples'
    if better == 'info':
        return 'unchanged', 'informational'
    stats = ['median'] + (['p95'] if min(b.get('n', 0), n.get('n', 0)) >= 10 else [])
    verdicts = []
    for stat in stats:
        bv, nv = b.get(stat), n.get(stat)
        if bv is None or nv is None:
            continue
        pct = change(bv, nv)
        delta = nv - bv
        significant = (pct is None and bv != nv) or (pct is not None and abs(pct) > threshold)
        if b.get('unit') == 'ms':
            significant = significant and abs(delta) > min_ms
        if significant:
            verdicts.append(('regression' if worse(better, bv, nv) else 'improvement', stat))
    if any(v == 'regression' for v, _ in verdicts):
        return 'regression', ','.join(s for v, s in verdicts if v == 'regression')
    if verdicts:
        return 'improvement', ','.join(s for _, s in verdicts)
    if n.get('failed', 0) > b.get('failed', 0):
        return 'regression', f"failures {b.get('failed', 0)} -> {n.get('failed', 0)}"
    return 'unchanged', ''


def fmt(v):
    if v is None:
        return '-'
    if isinstance(v, float):
        return f'{v:.1f}' if abs(v) < 10000 else f'{v:.0f}'
    return str(v)


def main():
    parser = argparse.ArgumentParser(description=__doc__.split('\n')[0])
    parser.add_argument('base')
    parser.add_argument('new')
    parser.add_argument('--threshold', type=float, default=10.0, help='percent change considered significant')
    parser.add_argument('--min-ms', type=float, default=2.0, help='absolute ms floor for timing metrics')
    parser.add_argument('--filter', default='', help='only metrics whose name matches this regex')
    parser.add_argument('--all', action='store_true', help='also list unchanged metrics')
    parser.add_argument('--gate', action='store_true', help='exit 1 on any regression')
    args = parser.parse_args()
    base, new = load(args.base), load(args.new)
    pattern = re.compile(args.filter) if args.filter else None
    names = sorted(set(base['metrics']) | set(new['metrics']))
    rows, counts = [], dict(regression=0, improvement=0, unchanged=0, new=0, missing=0)
    for name in names:
        if pattern and not pattern.search(name):
            continue
        b, n = base['metrics'].get(name), new['metrics'].get(name)
        status, detail = classify(name, b, n, args.threshold, args.min_ms)
        counts[status] += 1
        if status == 'unchanged' and not args.all:
            continue
        ref = b or n
        bm, nm = (b or {}).get('median'), (n or {}).get('median')
        pct = change(bm, nm)
        rows.append((status, name, ref.get('unit', ''), bm, nm, (b or {}).get('p95'), (n or {}).get('p95'),
                     pct, detail))
    order = dict(regression=0, missing=1, improvement=2, new=3, unchanged=4)
    rows.sort(key=lambda r: (order[r[0]], r[1]))
    print(f"# files_bench comparison: {base.get('label')} ({base.get('binary_version')}, "
          f"{base.get('git_commit')}) -> {new.get('label')} ({new.get('binary_version')}, {new.get('git_commit')})")
    print()
    print(f'Threshold {args.threshold:g}% and {args.min_ms:g} ms floor; '
          f"runs {base.get('runs')} -> {new.get('runs')}; remote {base.get('remote')} -> {new.get('remote')}.")
    if base.get('rtt_ms') != new.get('rtt_ms'):
        print(f"WARNING: different relay RTT ({base.get('rtt_ms')} vs {new.get('rtt_ms')} ms); remote rows are not comparable.")
    print()
    print('| Status | Metric | Unit | Base median | New median | Base p95 | New p95 | Change | Note |')
    print('|---|---|---|---|---|---|---|---|---|')
    for status, name, unit, bm, nm, bp, np_, pct, detail in rows:
        mark = {'regression': '**REGRESSION**', 'improvement': 'improved'}.get(status, status)
        print(f"| {mark} | `{name}` | {unit} | {fmt(bm)} | {fmt(nm)} | {fmt(bp)} | {fmt(np_)} | "
              f"{'-' if pct is None else f'{pct:+.0f}%'} | {detail} |")
    print()
    print(' · '.join(f'{k}: {v}' for k, v in counts.items()))
    for side, data in (('base', base), ('new', new)):
        if data.get('result') != 'PASS':
            print(f"WARNING: {side} result is {data.get('result')}: {'; '.join(data.get('notes', [])[:5])}")
    if args.gate and counts['regression']:
        sys.exit(1)


if __name__ == '__main__':
    main()
