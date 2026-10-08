#!/usr/bin/env python3
"""Ordinary login terminal exits and restores termios without tmux."""
import fcntl,json,os,pathlib,pty,select,subprocess,sys,tempfile,termios,time
binary=str(pathlib.Path(sys.argv[1]).resolve())
with tempfile.TemporaryDirectory(prefix='cx-login-') as tmp:
    master,slave=pty.openpty();before=termios.tcgetattr(slave)
    def tty():
        os.setsid();fcntl.ioctl(slave,termios.TIOCSCTTY,0)
    env=dict(os.environ,HOME=tmp,XDG_STATE_HOME=tmp+'/state',XDG_CONFIG_HOME=tmp+'/config',SHELL='/bin/sh')
    p=subprocess.Popen([binary,'terminal'],stdin=slave,stdout=slave,stderr=slave,env=env,preexec_fn=tty)
    try:
        time.sleep(.3)
        os.write(master,b"printf 'LOGIN_PROOF\\n'; stty -echo; exit\n")
        output=b'';deadline=time.monotonic()+8
        while p.poll() is None and time.monotonic()<deadline:
            if select.select([master],[],[],.1)[0]:output+=os.read(master,4096)
        assert p.poll()==0,'login did not return cleanly'
        while select.select([master],[],[],.1)[0]:output+=os.read(master,4096)
        assert b'LOGIN_PROOF' in output
        assert before==termios.tcgetattr(slave),'native terminal modes not restored'
        print(json.dumps(dict(result='PASS',ordinary_interactive_login=True,exit_returns=True,termios_restored=True,tmux_required=False)))
    finally:
        if p.poll() is None:p.kill();p.wait()
        os.close(master);os.close(slave)
