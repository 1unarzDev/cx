#!/usr/bin/env python3
"""Disposable ordinary-account enrollment, preservation and refusal checks."""
import base64
import json
import os
import pathlib
import shutil
import subprocess
import tempfile

repo = pathlib.Path(__file__).resolve().parents[1]
if os.getuid() == 0:
    raise SystemExit('Run as an ordinary account')
base = pathlib.Path.home() / '.cache'
base.mkdir(exist_ok=True)
with tempfile.TemporaryDirectory(prefix='cx-robot-enroll-', dir=base) as tmp:
    root = pathlib.Path(tmp)
    home = root / 'home'
    home.mkdir(mode=0o700)
    ssh = home / '.ssh'
    ssh.mkdir(mode=0o700)
    target = ssh / 'authorized_keys'
    preserved = '# existing unrelated entry\ncommand="true" ssh-ed25519 unchanged user\n'
    target.write_text(preserved)
    target.chmod(0o600)
    key = 'ssh-ed25519 ' + base64.b64encode(b'\x00\x00\x00\x0bssh-ed25519\x00\x00\x00\x20' + bytes(range(32))).decode()
    env = dict(os.environ, HOME=str(home))
    def enroll(payload):
        return subprocess.run(['python3', str(repo / 'scripts/enroll-public-keys.py')],
            input=json.dumps(payload), env=env, text=True, capture_output=True, timeout=5)
    payload = dict(version=1, keys={'innovation': key})
    first = enroll(payload)
    assert first.returncode == 0, first.stderr
    installed = target.read_bytes()
    assert installed.startswith(preserved.encode())
    assert b'no-agent-forwarding,no-X11-forwarding' in installed
    assert enroll(payload).returncode == 0 and target.read_bytes() == installed
    for invalid in ({'bad;id': key}, {'innovation': key + ' injected'}, {'innovation': 'ssh-ed25519 broken'}):
        assert enroll(dict(version=1, keys=invalid)).returncode != 0
        assert target.read_bytes() == installed
    target.unlink()
    outside = root / 'preserve'
    outside.write_text('untouched')
    target.symlink_to(outside)
    assert enroll(payload).returncode != 0 and outside.read_text() == 'untouched'
    target.unlink()
    target.write_bytes(installed)
    target.chmod(0o666)
    assert enroll(payload).returncode != 0 and target.read_bytes() == installed
    # Immutable/minimal host: no tmux, PDF, package manager or curl on PATH.
    tools = root / 'tools'
    tools.mkdir()
    for name in ('id', 'mkdir', 'ssh', 'ip', 'flock', 'stat', 'timeout', 'mktemp', 'cat', 'chmod', 'mv', 'rm'):
        (tools / name).symlink_to(shutil.which(name))
    body = b'#!/bin/sh\nprintf "cx fixture\\n"\n'
    run = subprocess.run(['/bin/sh', str(repo / 'scripts/enroll-helper.sh')], input=body,
        env=dict(env, PATH=str(tools), CX_ENROLL_MINIMAL='1'), capture_output=True, timeout=5)
    assert run.returncode == 0, run.stderr
    binary = home / '.local/bin/cx'
    assert binary.read_bytes() == body
    run = subprocess.run(['/bin/sh', str(repo / 'scripts/enroll-helper.sh')], input=b'replace',
        env=dict(env, PATH=str(tools)), capture_output=True, timeout=5)
    assert run.returncode != 0 and binary.read_bytes() == body
print(json.dumps(dict(result='PASS', public_key_preservation=True, idempotence=True,
    unsafe_trust_refused=True, minimal_without_optional_tools=True, full_mode_still_checks_tools=True)))
