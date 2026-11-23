#!/usr/bin/env python3
"""`cx claude|codex|agy|opencode|shell`, `cx kill` and `cx claude lend` dispatch from a
plain terminal, in tests/isolate.sh with a private HOME, state and tmux dir and fake
claude, agy and opencode on PATH (isolate.sh covers any real agy/opencode there).

  python tests/cli_launch_pty.py [path/to/cx]
"""
import json, os, pathlib, subprocess, sys, tempfile, time

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import pty_lib  # noqa: E402

pty_lib.isolate()
ROOT = pathlib.Path(__file__).resolve().parent.parent
CX = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else ROOT / 'target/debug/cx').resolve()
tmp = pathlib.Path(tempfile.mkdtemp(prefix='cx-cli-'))
home = tmp / 'home'
fake = tmp / 'bin'
proj = home / 'proj' / 'foo'
for d in (home, fake, proj, tmp / 'tmux'):
    d.mkdir(parents=True, exist_ok=True)
(fake / 'claude').write_text(
    '#!/bin/sh\n[ "$1" = --version ] && { echo "1.0.0 (Claude Code)"; exit 0; }\n'
    'echo "FAKE-CLAUDE cwd=$PWD args=$*"\nexec sleep 600\n')
(fake / 'claude').chmod(0o755)
for name in ('agy', 'opencode'):
    (fake / name).write_text(
        f'#!/bin/sh\n[ "$1" = --version ] && {{ echo "9.9.9"; exit 0; }}\n'
        f'echo "FAKE-{name.upper()} cwd=$PWD args=$* cfg=${{OPENCODE_CLI_CONFIG_CONTENT:-none}}"\nexec sleep 600\n')
    (fake / name).chmod(0o755)
env = dict(os.environ, HOME=str(home), XDG_STATE_HOME=str(home / '.local/state'),
           XDG_CONFIG_HOME=str(home / '.config'), XDG_CACHE_HOME=str(home / '.cache'),
           TMUX_TMPDIR=str(tmp / 'tmux'), SHELL='/bin/sh', TERM='xterm-256color',
           PATH=f"{fake}:{os.environ.get('PATH', '/usr/bin:/bin')}")
env.pop('TMUX', None)
failures = []


def check(name, ok, detail=''):
    print(('ok   ' if ok else 'FAIL ') + name + (f'  ({detail})' if detail and not ok else ''))
    if not ok:
        failures.append(name)


def cx(*args, cwd=proj):
    return subprocess.run([str(CX), *args], env=env, cwd=cwd, capture_output=True, text=True, timeout=30)


def attached(args, expect, cwd=proj):
    """Run `cx ARGS` on a PTY until EXPECT shows, then detach with Ctrl+\\; the screen text."""
    p = pty_lib.Pty([str(CX), *args], env, cwd, 100, 30)
    start, text = time.monotonic(), ''
    while time.monotonic() - start < 15:
        p.read(0.2)
        text = p.text()
        if expect in text:
            break
    shown = time.monotonic() - start
    p.write(b'\x1c')
    try:
        p.proc.wait(timeout=10)
    except subprocess.TimeoutExpired:
        pass
    code = p.proc.poll()
    p.close()
    return text, code, shown


def sessions():
    return json.loads(cx('sessions').stdout or '[]')


text, code, shown = attached(['claude'], 'FAKE-CLAUDE')
check('cx claude starts claude in this folder and attaches', f'FAKE-CLAUDE cwd={proj}' in text, text[-400:])
check('Ctrl+\\ detaches cx claude with status 0', code == 0, code)
print(f'     cx claude -> first agent frame {shown * 1000:.0f} ms')
listed = sessions()
check('the session is managed and listed in that folder',
      any(s['provider'] == 'claude' and s['directory'] == str(proj) for s in listed), listed)

text, code, _ = attached(['claude', '--yolo', '..'], 'FAKE-CLAUDE')
check('cx claude --yolo DIR (relative) starts there in YOLO',
      f'cwd={proj.parent} args=--dangerously-skip-permissions' in text, text[-400:])
text, code, _ = attached(['claude'], 'FAKE-CLAUDE', cwd=home)
check('the last mode is remembered (YOLO again without a flag)',
      'args=--dangerously-skip-permissions' in text, text[-400:])
text, code, _ = attached(['claude', '--normal', '~/proj'], 'FAKE-CLAUDE', cwd=tmp)
check('-n switches back to normal mode; ~/ expands', f'cwd={home}/proj args=' in text
      and 'dangerously' not in text.split('FAKE-CLAUDE')[-1], text[-400:])

text, code, _ = attached(['shell'], '$')
check('cx shell attaches a shell here', code == 0, code)
check('cx shell rejects --yolo', cx('shell', '--yolo').returncode != 0)
check('cx claude rejects --yolo with --normal', cx('claude', '--yolo', '--normal').returncode != 0)
bad = cx('claude', 'missing-dir')
check('a missing folder is an error before anything starts',
      bad.returncode != 0 and 'not a folder' in bad.stderr, bad.stderr)

AGY_NEW = 'args=--settings {"altScreenMode":"always"}'
text, code, _ = attached(['agy'], 'FAKE-AGY')
check('cx agy starts agy full screen in this folder and attaches',
      f'FAKE-AGY cwd={proj} {AGY_NEW} cfg=none' in text, text[-400:])
check('Ctrl+\\ detaches cx agy with status 0', code == 0, code)
text, code, _ = attached(['antigravity', '--yolo', '-r'], 'FAKE-AGY')
# (Its line wraps at 100 columns: compare without whitespace.)
check('cx antigravity --yolo -r resumes agy in YOLO',
      ''.join(f'{AGY_NEW} -c --dangerously-skip-permissions'.split()) in ''.join(text.split()),
      text[-400:])
text, code, _ = attached(['opencode'], 'FAKE-OPENCODE')
check('cx opencode starts its own server, mouse on through its inline config',
      f'FAKE-OPENCODE cwd={proj} args=--standalone cfg={{"mouse":true}}' in text, text[-400:])
text, code, _ = attached(['opencode', '--yolo', '-r'], 'FAKE-OPENCODE')
check('cx opencode --yolo -r', 'args=--standalone -c --auto' in text, text[-400:])
for agent, hint in (('agy', '/resume inside agy'), ('opencode', '/sessions inside opencode')):
    before = len(sessions())
    picked = cx(agent, '-p')
    check(f'cx {agent} -p is refused with the in-app picker hint',
          picked.returncode != 0 and hint in picked.stderr and len(sessions()) == before, picked.stderr)

lend = cx('claude', 'lend', '--help')
check('cx claude lend still reaches the lending commands', 'usage' in (lend.stdout + lend.stderr).lower(),
      lend.stdout + lend.stderr)

before = sessions()
victim = next(s for s in before if s['provider'] == 'claude')
refused = subprocess.run([str(CX), 'kill', victim['id']], env=env, cwd=proj, capture_output=True,
                         text=True, stdin=subprocess.DEVNULL, timeout=30)
check('cx kill without a terminal needs --yes', refused.returncode != 0 and '--yes' in refused.stderr,
      refused.stderr)
killed = cx('kill', victim['id'], '--yes')
check('cx kill --yes stops the session', killed.returncode == 0, killed.stderr)
after = sessions()
check('the killed session is gone, the others stay',
      all(s['id'] != victim['id'] for s in after) and len(after) == len(before) - 1, after)
check('cx kill of an unknown session fails clearly', 'no session' in cx('kill', 'nope', '-y').stderr)

print('PASS' if not failures else f'FAILED: {failures}')
sys.exit(1 if failures else 0)
