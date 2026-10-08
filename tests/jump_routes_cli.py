#!/usr/bin/env python3
"""Actual OpenSSH gateway authentication must remain noninteractive.
Uses a disposable loopback sshd and synthetic keys; no trust files are changed.
"""
import json, os, pathlib, socket, subprocess, sys, tempfile, time, shutil
binary = str(pathlib.Path(sys.argv[1]).resolve())
with tempfile.TemporaryDirectory(prefix='cx-jump-') as tmp:
    root = pathlib.Path(tmp); root.chmod(0o700)
    state = root/'state/cx'; state.mkdir(parents=True); state.chmod(0o700)
    subprocess.run(['ssh-keygen','-q','-t','ed25519','-N','','-f',str(root/'host')],check=True)
    sock=socket.socket();sock.bind(('127.0.0.1',0));port=sock.getsockname()[1];sock.close()
    config=root/'sshd.conf'
    config.write_text(f'Port {port}\nListenAddress 127.0.0.1\nHostKey {root}/host\nPidFile {root}/pid\nUsePAM no\nPasswordAuthentication yes\nPubkeyAuthentication no\n')
    log=open(root/'sshd.log','wb')
    sshd=shutil.which('sshd') or '/usr/sbin/sshd'
    daemon=subprocess.Popen([sshd,'-D','-e','-f',str(config)],stdout=log,stderr=log)
    try:
        for _ in range(50):
            if daemon.poll() is not None: raise RuntimeError((root/'sshd.log').read_text())
            try:
                with socket.create_connection(('127.0.0.1',port),timeout=.1): break
            except OSError:time.sleep(.02)
        target='fixture@127.0.0.1';gateway=f'fixture@127.0.0.1:{port}'
        (state/'devices.json').write_text(json.dumps([dict(id='fixture',name='fixture',target=target,account='fixture',host='fixture',status='unknown',observed_at=0)]))
        route=state/'routes.json';route.write_text(json.dumps({target:[gateway]}));route.chmod(0o600)
        askpass=root/'askpass';askpass.write_text('#!/bin/sh\nprintf called >> "$CX_ASKPASS_EVIDENCE"\nprintf no\\n\n');askpass.chmod(0o700)
        env=dict(os.environ,XDG_STATE_HOME=str(root/'state'),SSH_ASKPASS=str(askpass),SSH_ASKPASS_REQUIRE='force',DISPLAY='synthetic:0',CX_ASKPASS_EVIDENCE=str(root/'asked'))
        start=time.monotonic();r=subprocess.run([binary,'sessions','--device','fixture'],env=env,capture_output=True,stdin=subprocess.DEVNULL,timeout=12,start_new_session=True)
        assert r.returncode!=0,'unknown gateway trust must fail closed'
        assert not (root/'asked').exists(),'background jump triggered credential/trust prompt'
        cfg=state/'ssh-background-v2.conf';assert 'BatchMode yes' in cfg.read_text()
        # An enrolled endpoint may not silently move to a different gateway.
        gateway_device=dict(id='gateway',name='gateway',target='different-gateway',account='fixture',host='gateway',status='unknown',observed_at=0)
        ds=json.loads((state/'devices.json').read_text());ds.append(gateway_device);(state/'devices.json').write_text(json.dumps(ds))
        previous=route.read_bytes()
        conflict=subprocess.run([binary,'add',target,'--via','gateway'],env=env,capture_output=True,timeout=5)
        assert conflict.returncode!=0 and b'already enrolled' in conflict.stderr,conflict.stderr
        assert route.read_bytes()==previous,'conflict changed original route'
        # Simulate the user's config-defined alias using a private wrapper config.
        # No personal SSH config or known_hosts is edited. The wrapper requires
        # cx's inherited -F policy and retains it before adding fixture aliases.
        known=root/'known_hosts'
        public=(root/'host.pub').read_text().split()
        known.write_text(f'[127.0.0.1]:{port} {public[0]} {public[1]}\n')
        tools=root/'tools';tools.mkdir()
        wrapper=tools/'ssh'
        wrapper.write_text(r"""#!/usr/bin/env python3
import os,sys,pathlib,subprocess
root=pathlib.Path(os.environ['CX_JUMP_FIXTURE']);args=sys.argv[1:]
assert '-F' in args, 'cx omitted inherited noninteractive policy'
pos=args.index('-F')+1;policy=pathlib.Path(args[pos]).read_text()
assert 'BatchMode yes' in policy
config=root/'private-client.conf'
config.write_text(policy+'\nHost cx-fixture-final\n HostName 127.0.0.1\n ProxyJump cx-fixture-gateway\nHost cx-fixture-gateway\n HostName 127.0.0.1\n User fixture\n Port '+os.environ['CX_JUMP_PORT']+'\n UserKnownHostsFile '+str(root/'known_hosts')+'\n PreferredAuthentications password\n PubkeyAuthentication no\n')
args[pos]=str(config)
r=subprocess.run(['/usr/bin/ssh',*args],stdin=sys.stdin.buffer,stdout=sys.stdout.buffer,stderr=subprocess.PIPE)
(root/'client-error').write_bytes(r.stderr);sys.stderr.buffer.write(r.stderr);sys.exit(r.returncode)
""")
        wrapper.chmod(0o700)
        route.write_text('{}')
        ds[0]['target']='cx-fixture-final';(state/'devices.json').write_text(json.dumps(ds))
        env.update(PATH=str(tools)+os.pathsep+os.environ['PATH'],CX_JUMP_FIXTURE=tmp,CX_JUMP_PORT=str(port))
        r=subprocess.run([binary,'sessions','--device','fixture'],env=env,capture_output=True,stdin=subprocess.DEVNULL,timeout=12,start_new_session=True)
        assert r.returncode!=0 and not (root/'asked').exists(),'config-defined jump prompted'
        diagnostic=(root/'client-error').read_bytes()
        assert b'Permission denied (password' in diagnostic,diagnostic
        print(json.dumps(dict(result='PASS',backend=subprocess.check_output(['ssh','-V'],stderr=subprocess.STDOUT,text=True).strip(),unknown_gateway_no_prompt=True,overlapping_route_preserved=True,config_jump_password_no_prompt=True,seconds=round(time.monotonic()-start,3))))
    finally:
        daemon.terminate();daemon.wait(timeout=3);log.close()
