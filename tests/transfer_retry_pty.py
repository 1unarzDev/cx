#!/usr/bin/env python3
"""Actual failed detached job, new viewer, visible error and same-key retry via r."""
import fcntl, json, os, pathlib, pty, select, struct, subprocess, sys, tempfile, termios, time
import pyte
binary=str(pathlib.Path(sys.argv[1]).resolve())
with tempfile.TemporaryDirectory(prefix='cx-retry-') as temporary:
    root=pathlib.Path(temporary); source=root/'source.bin'; source.write_bytes(b'retry fixture')
    assert os.geteuid()!=0, 'permission failure fixture requires an ordinary user'
    destination=root/'blocked-destination'; destination.mkdir(); destination.chmod(0o555); key='retry-fixture'
    env=dict(os.environ, HOME=str(root), XDG_STATE_HOME=str(root/'state'), SHELL='/bin/sh', TERM='xterm-256color', HTTPS_PROXY='http://127.0.0.1:9')
    env.pop('TMUX',None); env.pop('TMUX_PANE',None)
    queued=subprocess.run([binary,'copy',str(source),str(destination),'--key',key],env=env,capture_output=True,text=True,timeout=15)
    assert queued.returncode==0, queued.stderr
    def jobs(): return json.loads(subprocess.check_output([binary,'jobs'],env=env,timeout=15))['jobs']
    deadline=time.monotonic()+10
    while jobs()[0]['status']!='failed' and time.monotonic()<deadline: time.sleep(.1)
    assert jobs()[0]['status']=='failed', jobs()
    destination.chmod(0o755)
    state=root/'state/cx'; name='viewer-restart-123-999.json'
    snapshot=dict(schema=1,expires_at=int(time.time())+300,device_ids=['local'],device=1,focus='Workspace',view='Files',selected=0,selected_session=None,side_selected=0,search='',browser=dict(device=0,path=str(root),display_path=str(root),parent='/tmp',entries=[],selected=0,search='',preview_scroll=0,restore_selection=None),other_browser=None,destination_active=False,conflict=2,launch_provider=None,clipboard=None,submitted={})
    saved=state/name;saved.write_text(json.dumps(snapshot));saved.chmod(0o600)
    master,slave=pty.openpty();before=termios.tcgetattr(slave)
    fcntl.ioctl(slave,termios.TIOCSWINSZ,struct.pack('HHHH',24,48,0,0))
    def control(): os.setsid();fcntl.ioctl(slave,termios.TIOCSCTTY,0)
    proc=subprocess.Popen([binary,'restart',name],stdin=slave,stdout=slave,stderr=slave,env=env,preexec_fn=control)
    screen=pyte.Screen(48,24);stream=pyte.Stream(screen)
    def read(duration=.2):
        end=time.monotonic()+duration
        while time.monotonic()<end:
            if select.select([master],[],[],.02)[0]:
                try: stream.feed(os.read(master,65536).decode(errors='replace'))
                except OSError: break
    def send(keys): os.write(master,keys);read()
    def text(): return '\n'.join(screen.display)
    try:
        read(1);send(b't')
        deadline=time.monotonic()+15
        while not ('Permission' in text() and 'denied' in text()) and time.monotonic()<deadline: read(.1)
        assert 'Permission' in text() and 'denied' in text(),text()
        assert 'Retry job' in text() and 'Copy / cut' not in text(),text()
        evidence=pathlib.Path('local-evidence/browser-retry');evidence.mkdir(parents=True,exist_ok=True)
        (evidence/'failed-details-48.txt').write_text(text())
        send(b'r')
        deadline=time.monotonic()+12
        while not (destination/'source.bin').exists() and time.monotonic()<deadline:read(.1)
        assert (destination/'source.bin').read_bytes()==b'retry fixture'
        deadline=time.monotonic()+6
        while 'Copy complete' not in text() and time.monotonic()<deadline:read(.1)
        assert jobs()[0]['key']==key and jobs()[0]['status']=='complete'
        assert 'Transfer queued' not in text(),text()
        (evidence/'retried-48.txt').write_text(text())
        send(b'\x1b');send(b'\x03');proc.wait(timeout=4)
        assert termios.tcgetattr(slave)==before
        result=dict(result='PASS',scenario='actual destination permission failure, visible reason at48x24, new-viewer r retry uses existing durable key, content integrity, completion notice and termios')
        (evidence/'results.json').write_text(json.dumps(result,indent=2));print(json.dumps(result))
    finally:
        if proc.poll() is None:proc.terminate();proc.wait(timeout=4)
        os.close(master);os.close(slave)
