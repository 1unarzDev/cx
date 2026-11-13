#!/usr/bin/env python3
"""The local agent check offers Claude/Codex when cx runs in a terminal (v0.2.2).

usage: provider_probe_tty.py BINARY

cx probes `claude --version` / `codex --version` through the login shell, as
`SHELL -l -i -c ...` in a process group of its own. Started from a terminal, an
interactive bash or sh in a background group of that terminal took it over for job
control and was stopped (SIGTTOU), so the probe timed out after 3 s and bash/sh users
were not offered the agents. Here `cx helper` runs on a private PTY (its controlling
terminal) in a private HOME with stand-in `claude` and `codex` on PATH, once per
shell (bash, sh, and fish when installed); its Info capabilities must list both, well
inside the 3 s bound. Prints one JSON line.
"""
import fcntl, json, os, pathlib, pty, select, shutil, struct, subprocess, sys, tempfile, termios, time

BINARY = str(pathlib.Path(sys.argv[1]).resolve())


def probe(shell, home):
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 24, 80, 0, 0))

    def tty():
        os.setsid()
        fcntl.ioctl(slave, termios.TIOCSCTTY, 0)
    env = dict(os.environ, HOME=str(home), XDG_STATE_HOME=str(home / 'state'),
               TMUX_TMPDIR=str(home / 'tmux'), SHELL=shell, TERM='xterm-256color',
               PATH=f"{home / 'bin'}{os.pathsep}{os.environ['PATH']}", HISTFILE='/dev/null',
               HTTPS_PROXY='http://127.0.0.1:9')
    for key in ('TMUX', 'TMUX_PANE', 'BASH_ENV', 'ENV'):
        env.pop(key, None)
    payload = json.dumps(dict(version=1, id='probe', op=dict(op='info'))).encode()
    started = time.monotonic()
    proc = subprocess.Popen([BINARY, 'helper'], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                            stderr=slave, env=env, cwd=str(home), preexec_fn=tty)
    os.close(slave)
    try:
        out, _ = proc.communicate(b'CX1 %d\n' % len(payload) + payload, timeout=30)
    finally:
        while select.select([master], [], [], 0)[0]:
            try:
                os.read(master, 65536)
            except OSError:
                break
        os.close(master)
    elapsed = time.monotonic() - started
    header, body = out.split(b'\n', 1)
    assert header.startswith(b'CX1 '), out[:200]
    response = json.loads(body[:int(header.split()[1])])
    assert not response['error'], response['error']
    return response['result'], elapsed


def main():
    shells = [s for s in ('/bin/bash', '/bin/sh', shutil.which('fish')) if s and os.path.exists(s)]
    found = {}
    with tempfile.TemporaryDirectory(prefix='cx-probe-') as tmp:
        home = pathlib.Path(tmp)
        for sub in ('bin', 'state/cx', 'tmux'):
            (home / sub).mkdir(parents=True, mode=0o700)
        for name in ('claude', 'codex'):
            stand_in = home / 'bin' / name
            stand_in.write_text(f'#!/bin/sh\necho "{name} 0.0.0 (stand-in)"\n')
            stand_in.chmod(0o700)
        for shell in shells:
            info, elapsed = probe(shell, home)
            text = json.dumps(info)
            offered = [p for p in ('claude', 'codex') if f'"{p}"' in text]
            assert offered == ['claude', 'codex'], (shell, offered, round(elapsed, 2))
            assert elapsed < 2.5, (shell, round(elapsed, 2))
            found[os.path.basename(shell)] = dict(offered=offered, seconds=round(elapsed, 2))
    print(json.dumps(dict(result='PASS', shells=found)))


if __name__ == '__main__':
    main()
