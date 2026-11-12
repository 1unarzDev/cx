#!/usr/bin/env python3
"""No GitHub workflow repeats a mapping key (Actions rejects the whole file, so a tag
push would publish nothing). Stdlib only: block-style YAML as the workflows write it."""
import pathlib, re, sys

KEY = re.compile(r'^(\s*)(- )?([A-Za-z0-9_.-]+|"[^"]*"|\'[^\']*\'):(\s|$)')


def duplicates(text):
    found = []
    stack = []  # (indent of the mapping's keys, set of keys)
    block = None  # indent a block scalar (`run: |`) must exceed
    for n, line in enumerate(text.splitlines(), 1):
        if not line.strip() or line.lstrip().startswith('#'):
            continue
        indent = len(line) - len(line.lstrip())
        if block is not None:
            if indent > block:
                continue
            block = None
        m = KEY.match(line)
        if not m:
            continue
        if m.group(2):  # `- key:` opens a new mapping inside a list item
            indent += 2
            while stack and stack[-1][0] >= indent:
                stack.pop()
            stack.append((indent, set()))
        while stack and stack[-1][0] > indent:
            stack.pop()
        if not stack or stack[-1][0] < indent:
            stack.append((indent, set()))
        key = m.group(3)
        if key in stack[-1][1]:
            found.append(f'{n}: {key}')
        stack[-1][1].add(key)
        if re.search(r':\s*[|>][-+]?\s*(#.*)?$', line):
            block = indent - (2 if m.group(2) else 0)
    return found


def main():
    assert duplicates('a:\n  b: 1\n  b: 2\n') == ['3: b'], 'self-test: a repeat'
    assert duplicates('a:\n  - b: 1\n  - b: 2\nc:\n  run: |\n    b: 1\n    b: 2\n') == []
    root = pathlib.Path(__file__).resolve().parent.parent / '.github' / 'workflows'
    bad = {p.name: d for p in sorted(root.glob('*.yml')) if (d := duplicates(p.read_text()))}
    if bad:
        sys.exit(f'duplicate workflow keys: {bad}')
    print(f'ok: {len(list(root.glob("*.yml")))} workflows, no repeated keys')


main()
