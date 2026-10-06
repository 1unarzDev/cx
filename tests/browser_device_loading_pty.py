#!/usr/bin/env python3
"""A slow remote helper must not prevent local Files loading; no real SSH or host mutation."""
import fcntl,json,os,pathlib,pty,select,struct,subprocess,sys,tempfile,termios,time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
with tempfile.TemporaryDirectory(prefix='cx-device-loading-') as tmp:
    root=pathlib.Path(tmp); (root/'visible-fixture.txt').write_text('fixture')
    state=root/'state/cx';state.mkdir(parents=True)
    fake=root/'bin';fake.mkdir();ssh=fake/'ssh';ssh.write_text('#!/bin/sh\nsleep 4\nexit 255\n');ssh.chmod(0o700)
    devices=[dict(id='local',name='fixture-local',target=None,account='tester',host='fixture-local',status='unknown',observed_at=0),dict(id='slow',name='fixture-offline',target='fixture-offline',account='tester',host='fixture-offline',status='unknown',observed_at=0)]
    (state/'devices.json').write_text(json.dumps(devices))
    name='viewer-restart-123-456.json'
    snapshot=dict(schema=1,expires_at=int(time.time())+300,device_ids=['local','slow'],device=1,focus='Workspace',view='Files',selected=0,selected_session=None,side_selected=0,search='',browser=dict(device=0,path=str(root),display_path=str(root),parent='/tmp',entries=[],selected=0,search='',preview_scroll=0,restore_selection=None),other_browser=None,destination_active=False,conflict=2,launch_provider=None,clipboard=None,submitted={})
    saved=state/name;saved.write_text(json.dumps(snapshot));saved.chmod(0o600)
    env=dict(os.environ,HOME=tmp,XDG_STATE_HOME=str(root/'state'),PATH=str(fake)+':/usr/bin:/bin',SHELL='/bin/sh',TERM='xterm-256color',HTTPS_PROXY='http://127.0.0.1:9')
    env.pop('TMUX',None);env.pop('TMUX_PANE',None)
    master,slave=pty.openpty();before=termios.tcgetattr(slave);fcntl.ioctl(slave,termios.TIOCSWINSZ,struct.pack('HHHH',24,80,0,0))
    def control():os.setsid();fcntl.ioctl(slave,termios.TIOCSCTTY,0)
    proc=subprocess.Popen([binary,'restart',name],stdin=slave,stdout=slave,stderr=slave,env=env,preexec_fn=control)
    screen=pyte.Screen(80,24);stream=pyte.Stream(screen)
    def read(duration=.1):
        end=time.monotonic()+duration
        while time.monotonic()<end:
            if select.select([master],[],[],.02)[0]:
                try:stream.feed(os.read(master,65536).decode(errors='replace'))
                except OSError:break
    try:
        start=time.monotonic()
        while 'visible-fixture.txt' not in '\n'.join(screen.display) and time.monotonic()-start<2.5:read()
        elapsed=time.monotonic()-start
        assert 'visible-fixture.txt' in '\n'.join(screen.display), 'Local Files blocked behind slow remote: '+ '\n'.join(screen.display)
        os.write(master,b'/visible-fixture');read(.15);os.write(master,b'\x1b');read(.1)
        os.write(master,b'c');read(.1)
        assert 'COPY' in '\n'.join(screen.display)
        os.write(master,b'\t');read(.1)
        os.write(master,b'j');read(.1)
        assert 'tester@fixture-offline' in '\n'.join(screen.display),'Browser did not retarget'
        assert 'Focus: Devices' in '\n'.join(screen.display)
        os.write(master,b'k');read(.2)
        assert 'visible-fixture.txt' in '\n'.join(screen.display),'Local cached folder did not return'
        assert 'COPY' in '\n'.join(screen.display),'Device switch lost clipboard'
        os.write(master,b'\x03');proc.wait(timeout=3)
        assert termios.tcgetattr(slave)==before
        evidence=pathlib.Path('local-evidence/browser-device-loading');evidence.mkdir(parents=True,exist_ok=True)
        (evidence/'results.json').write_text(json.dumps(dict(result='PASS',local_listing_seconds=elapsed,remote_fixture_delay_seconds=4),indent=2))
        print(json.dumps(dict(result='PASS',local_listing_seconds=elapsed)))
    finally:
        if proc.poll() is None:proc.terminate();proc.wait(timeout=3)
        os.close(master);os.close(slave)
