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
    # Viewer-mediated upgrades preserve optional-tool independence and reject races/downgrades.
    for name in ('sort', 'tail'):
        (tools / name).symlink_to(shutil.which(name))
    def fixture(version):
        return ('#!/bin/sh\nprintf "cx ' + version + '\\n"\n').encode()
    tmux = binary.parent / 'tmux'
    tmux.write_bytes(b'user-installed tmux preserved')
    tmux.chmod(0o700)
    binary.write_bytes(fixture('0.1.32'))
    binary.chmod(0o700)
    def upgrade(version, content):
        return subprocess.run(['/bin/sh', str(repo / 'scripts/enroll-helper.sh')], input=content,
            env=dict(env, PATH=str(tools), CX_ENROLL_MINIMAL='1', CX_ENROLL_UPDATE_VERSION=version),
            capture_output=True, timeout=5)
    for version in ('0.1.31', '0.1.32'):
        assert upgrade(version, fixture(version)).returncode == 76
        assert binary.read_bytes() == fixture('0.1.32')
    assert upgrade('0.1.34', fixture('0.1.33')).returncode != 0
    assert binary.read_bytes() == fixture('0.1.32')
    assert upgrade('0.1.34', fixture('0.1.34')).returncode == 0
    assert binary.read_bytes() == fixture('0.1.34')
    assert upgrade('0.1.33;id', fixture('0.1.35')).returncode != 0
    assert binary.read_bytes() == fixture('0.1.34')
    import fcntl
    with (home / '.local/state/cx/maintenance.lock').open('a') as held:
        fcntl.flock(held.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
        assert upgrade('0.1.35', fixture('0.1.35')).returncode == 75
        assert binary.read_bytes() == fixture('0.1.34')
    assert tmux.read_bytes() == b'user-installed tmux preserved'

    # Portable tool enrollment installs absent tools, rejects bad probes, and preserves existing ones.
    def install_tool(content):
        return subprocess.run(['/bin/sh', str(repo / 'scripts/enroll-tmux.sh')], input=content,
            env=dict(env, PATH=str(tools)), capture_output=True, timeout=5)
    # A broken existing binary is never overwritten.
    assert install_tool(b'replace').returncode != 0
    assert tmux.read_bytes() == b'user-installed tmux preserved'
    tmux.unlink()
    good_tool = b'#!/bin/sh\nprintf "tmux 3.7c\\n"\n'
    assert install_tool(b'#!/bin/sh\nexit 1\n').returncode != 0 and not tmux.exists()
    assert install_tool(good_tool).returncode == 0 and tmux.read_bytes() == good_tool
    assert install_tool(b'replace').returncode == 0 and tmux.read_bytes() == good_tool
    tmux.unlink()
    tmux.symlink_to(outside)
    assert install_tool(good_tool).returncode != 0 and outside.read_text() == 'untouched'
    assert not list(binary.parent.glob('.tmux-install.*'))
    print('PASS: minimal helper, monotonic updates, absent portable tmux, existing tool and trust preservation')

print(json.dumps(dict(result='PASS', public_key_preservation=True, idempotence=True,
    unsafe_trust_refused=True, minimal_without_optional_tools=True, full_mode_still_checks_tools=True, mediated_update_monotonic=True, install_lock_preserved=True)))
