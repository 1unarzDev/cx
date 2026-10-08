#!/usr/bin/env python3
"""Actual add command: platform gate, helper provisioning, tool refusal and retry safety.
SSH is a bounded local fixture; no live host, trust or network configuration changes.
"""
import os, pathlib, shutil, subprocess, sys, tempfile, json
binary = str(pathlib.Path(sys.argv[1]).resolve())
base = pathlib.Path.home()/'.cache'
base.mkdir(exist_ok=True)
for scenario in ('unsupported', 'existing', 'broken', 'missing-offline'):
    with tempfile.TemporaryDirectory(prefix='cx-tooling-cli-', dir=base) as tmp:
        root = pathlib.Path(tmp); home = root/'home'; home.mkdir(mode=0o700); (root/'s').mkdir(mode=0o700)
        tools = root/'tools'; tools.mkdir()
        for name in ('id','mkdir','ip','flock','stat','timeout','mktemp','cat','chmod','mv','rm','sh'):
            (tools/name).symlink_to(shutil.which(name))
        ssh = tools/'ssh'
        ssh.write_text(f'''#!{sys.executable}
import os, subprocess, sys
command = sys.argv[-1]
if command == 'uname -sm; id -un':
    print('Darwin arm64' if os.environ['CX_ENROLL_FIXTURE']=='unsupported' else 'Linux '+os.uname().machine)
    print('fixture')
    sys.exit(0)
sys.exit(subprocess.call(['/bin/sh','-c',command]))
'''); ssh.chmod(0o700)
        if scenario in ('existing','broken'):
            tmux = tools/'tmux'
            tmux.write_text('#!/bin/sh\n'+('printf "tmux fixture\\n"\n' if scenario=='existing' else 'exit 1\n'))
            tmux.chmod(0o700); preserved = tmux.read_bytes()
        env = dict(os.environ, HOME=str(home), XDG_STATE_HOME=str(root/'s'), PATH=str(tools),
            CX_ENROLL_FIXTURE=scenario, HTTPS_PROXY='http://127.0.0.1:9')
        result = subprocess.run([binary,'add','fixture@example.test'], env=env, capture_output=True, timeout=20)
        helper = home/'.local/bin/cx'
        if scenario=='unsupported':
            assert result.returncode!=0 and b'Linux x86_64 or ARM64' in result.stderr, (result.returncode, result.stderr)
            assert not helper.exists(), 'unsupported host installed a helper'
        elif scenario=='existing':
            assert result.returncode==0, result.stderr.decode()
            assert helper.exists() and tmux.read_bytes()==preserved
        else:
            assert result.returncode!=0 and helper.exists()
            assert b'tmux' in result.stderr
            if scenario=='broken': assert tmux.read_bytes()==preserved
            assert not (root/'s/cx/devices.json').exists(), 'incomplete host saved as ready'
        print(json.dumps(dict(scenario=scenario,result='PASS')))
