#!/usr/bin/env python3
"""Offline transport fixtures with real RSA4096 signatures and pinned-key substitution."""
import concurrent.futures, fcntl, io, json, os, pathlib, pty, re, signal, subprocess, tarfile, tempfile, termios, time
source = pathlib.Path('install.sh').resolve()
if os.getuid() == 0:
    raise SystemExit('Run installer fixtures as an ordinary user (installer refuses root)')
base = pathlib.Path.home() / '.cache'
base.mkdir(mode=0o700, exist_ok=True)
with tempfile.TemporaryDirectory(prefix='cx-install-test-', dir=base) as directory:
    root = pathlib.Path(directory); home = root/'home'; bins=home/'.local/bin'; bins.mkdir(parents=True)
    fake=root/'tools';fake.mkdir(); state=home/'.local/state/cx';state.mkdir(parents=True)
    key=root/'private.pem';pub=root/'public.pem'
    subprocess.run(['openssl','genpkey','-algorithm','RSA','-pkeyopt','rsa_keygen_bits:4096','-out',str(key)],check=True,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
    subprocess.run(['openssl','pkey','-in',str(key),'-pubout','-out',str(pub)],check=True,stdout=subprocess.DEVNULL)
    script=root/'install.sh';script.write_text(re.sub(r'-----BEGIN PUBLIC KEY-----.*?-----END PUBLIC KEY-----',pub.read_text().strip(),source.read_text(),flags=re.S))
    target=bins/'cx'; old=b'#!/bin/sh\nprintf "cx 0.0.1\\n"\n';body=b'#!/bin/sh\nsleep 0.2\nprintf "cx 9.9.9\\n"\n'
    archive=root/'artifact.tar.gz';signature=root/'artifact.sig'
    def artifact(kind='regular', content=body, name='cx', duplicate=False):
        with tarfile.open(archive,'w:gz') as tar:
            entry=tarfile.TarInfo(name);entry.mode=0o700
            if kind=='regular':entry.size=len(content);tar.addfile(entry,io.BytesIO(content))
            else:
                entry.type={'symlink':tarfile.SYMTYPE,'hardlink':tarfile.LNKTYPE,'device':tarfile.CHRTYPE}[kind];entry.linkname='/etc/passwd';tar.addfile(entry)
            if duplicate:tar.addfile(entry,io.BytesIO(content))
        subprocess.run(['openssl','dgst','-sha256','-sign',str(key),'-out',str(signature),str(archive)],check=True)
    artifact()
    (fake/'curl').write_text('''#!/usr/bin/python3
import os,sys,pathlib,time
if os.environ.get('CX_FIXTURE_OFFLINE'):sys.exit(7)
if os.environ.get('CX_FIXTURE_SLOW'):time.sleep(20)
args=sys.argv[1:];dest=pathlib.Path(args[args.index('-o')+1]);url=next(a for a in args if a.startswith('https://'))
pathlib.Path(os.environ['CX_FIXTURE_URLS']).open('a').write(url+'\\n')
if 'releases/latest' in url:dest.write_text('{"tag_name":"v9.9.9","assets":[]}' if os.environ.get('CX_FIXTURE_COMPACT') else '{\\n "tag_name": "v9.9.9"\\n}\\n')
else:dest.write_bytes(pathlib.Path(os.environ['CX_FIXTURE_SIGNATURE' if url.endswith('.sig') else 'CX_FIXTURE_ARCHIVE']).read_bytes())
''');(fake/'curl').chmod(0o700)
    env=dict(os.environ,HOME=str(home),XDG_CONFIG_HOME=str(home/'.config'),XDG_STATE_HOME=str(home/'.local/state'),PATH=str(fake)+':/usr/bin:/bin',CX_FIXTURE_ARCHIVE=str(archive),CX_FIXTURE_SIGNATURE=str(signature),CX_FIXTURE_URLS=str(root/'urls'),CX_NO_LAUNCH='1',CX_INSTALL_DEPS='never')
    for name in ('CX_REPO','CX_VERSION','CX_PREFIX'):env.pop(name,None)
    def run(extra=None):return subprocess.run(['sh',str(script)],env=dict(env,**(extra or {})),stdin=subprocess.DEVNULL,capture_output=True,text=True,timeout=15)
    def reset():
        if target.exists() or target.is_symlink():target.unlink()
        target.write_bytes(old);target.chmod(0o700)
    def refused(label,extra=None):
        result=run(extra);assert result.returncode!=0,(label,result.stdout,result.stderr)
        assert target.read_bytes()==old,(label,'installed binary changed');outcomes.append(label)
    outcomes=[];reset()
    refused('offline preserves binary',{'CX_FIXTURE_OFFLINE':'1'})
    refused('alternate repository refused',{'CX_REPO':'someone/fork'})
    for tag in ['v1.2','v1.2.3/../x','v01.2.3','v1.2.3\nv4.5.6','v1.2.3-rc1']:
        refused('malformed tag '+repr(tag),{'CX_VERSION':tag})
    with (state/'maintenance.lock').open('a') as lock:
        fcntl.flock(lock,fcntl.LOCK_EX|fcntl.LOCK_NB);inode=os.stat(state/'maintenance.lock').st_ino
        refused('stable locked inode preserves binary');assert os.stat(state/'maintenance.lock').st_ino==inode
    for kind in ('symlink','hardlink','device'):
        artifact(kind);refused('signed '+kind+' archive refused')
    artifact(name='../cx');refused('signed path traversal refused')
    artifact(duplicate=True);refused('duplicate members refused')
    artifact(content=b'#!/bin/sh\necho "cx 8.8.8"\n');refused('signed binary version mismatch refused')
    artifact(content=b'');refused('empty binary refused')
    artifact(content=b'0'*67108865);refused('expanded binary size limit enforced')
    artifact();signature.write_bytes(bytes(512));refused('invalid real RSA signature refused')
    artifact();target.unlink();target.symlink_to(root/'external');result=run();assert result.returncode and target.is_symlink();outcomes.append('symlink target refused');reset()
    linked=root/'linked';os.link(target,linked);refused('hardlinked target refused');linked.unlink()
    redirected=root/'redirected';redirected.mkdir();redirect=home/'redirect';redirect.symlink_to(redirected,target_is_directory=True)
    refused('symlink parent refused',{'CX_PREFIX':str(redirect)})
    result=run({'CX_VERSION':'v9.9.9'});assert result.returncode==0,result.stderr;assert target.read_bytes()==body
    result=run();assert result.returncode==0,result.stderr;assert target.read_bytes()==body
    assert (home/'.config/fish/conf.d/cx-path.fish').is_file()
    shellenv=home/'.config/cx/env.sh'
    subprocess.run(['sh','-n',str(shellenv)],check=True)
    result=subprocess.run(['sh','-c','. "$1"; . "$1"; printf "%s" "$PATH"','sh',str(shellenv)],env=dict(env,PATH='/usr/bin:/bin'),capture_output=True,text=True,check=True)
    assert result.stdout.split(':').count(str(bins))==1
    outcomes.append('verified install, safe rerun, additive PATH idempotence')
    result=run({'CX_FIXTURE_COMPACT':'1'});assert result.returncode==0,result.stderr;assert target.read_bytes()==body
    outcomes.append('compact GitHub latest release JSON')
    reset()
    with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:results=list(pool.map(lambda _:run(),range(2)))
    assert sorted(r.returncode for r in results)==[0,1],[(r.returncode,r.stderr) for r in results]
    assert target.read_bytes()==body and not list(bins.glob('.cx-install.*'))
    outcomes.append('concurrent atomic installation serialized')
    reset();proc=subprocess.Popen(['sh',str(script)],env=dict(env,CX_FIXTURE_SLOW='1'),stdin=subprocess.DEVNULL,stdout=subprocess.PIPE,stderr=subprocess.PIPE,start_new_session=True)
    time.sleep(.3);os.killpg(proc.pid,signal.SIGTERM);proc.communicate(timeout=5)
    assert target.read_bytes()==old and not list(state.glob('install.*'));outcomes.append('interrupted download cleanup preserves binary')
    for architecture,asset in [('x86_64','x86_64'),('aarch64','aarch64')]:
        (fake/'uname').write_text('#!/bin/sh\ncase "$1" in -s) echo Linux;; -m) echo '+architecture+';; esac\n');(fake/'uname').chmod(0o700)
        result=run();assert result.returncode==0,result.stderr
        assert 'linux-'+asset+'.tar.gz' in (root/'urls').read_text();outcomes.append('asset selection '+architecture)
    # Explicit piped-bootstrap consent through controlling tty, mocked sudo only.
    for manager, package in [('apt-get','openssh-client'),('dnf','openssh-clients'),('yum','openssh-clients'),('pacman','openssh')]:
        tools=root/('deps-'+manager);tools.mkdir()
        for command in ('id','uname','curl','openssl','ssh','flock','tar','stat','timeout','install','mktemp','head','sed','grep'):
            import shutil
            found=shutil.which(command);assert found,command;(tools/command).symlink_to(found)
        (tools/manager).write_text('#!/bin/sh\nexit 0\n');(tools/manager).chmod(0o700)
        logfile=root/('sudo-'+manager)
        (tools/'sudo').write_text('#!/bin/sh\nprintf "%s\\n" "$*" > '+str(logfile)+'\ncase "$*" in "apt-get update") exit 0;; esac\nexit 1\n');(tools/'sudo').chmod(0o700)
        master,slave=pty.openpty()
        def tty_setup():
            os.setsid();fcntl.ioctl(slave,termios.TIOCSCTTY,0)
        proc=subprocess.Popen(['/bin/sh',str(script)],env=dict(env,PATH=str(tools),CX_INSTALL_DEPS='ask'),stdin=subprocess.DEVNULL,stdout=slave,stderr=slave,preexec_fn=tty_setup)
        os.close(slave);output=b''
        while b'[y/N]' not in output:
            output+=os.read(master,4096)
        os.write(master,b'y\n');proc.wait(timeout=5);os.close(master)
        packages = logfile.read_text().split()
        required = {
            'apt-get': ['gzip', 'grep', 'sed', 'poppler-utils', 'iproute2', 'iputils-ping', 'ncurses-bin', 'ncurses-term'],
            'dnf': ['gzip', 'grep', 'sed', 'poppler-utils', 'iproute', 'iputils', 'ncurses', 'ncurses-term'],
            'yum': ['gzip', 'grep', 'sed', 'poppler-utils', 'iproute', 'iputils', 'ncurses', 'ncurses-term'],
            'pacman': ['gzip', 'grep', 'sed', 'poppler', 'iproute2', 'iputils', 'ncurses'],
        }[manager]
        assert proc.returncode != 0 and manager in packages and package in packages
        assert all(p in packages for p in required), (manager, packages)
        if manager == 'apt-get':
            assert not set(packages) & {'iproute', 'iputils', 'ncurses'}, packages
        outcomes.append('explicit piped tty dependency consent '+manager)
    # Enrollment refuses missing host tools before replacing any owned helper.
    import shutil
    enroll_tools = root/'enroll-tools'; enroll_tools.mkdir()
    for command in ('id', 'curl', 'openssl', 'ssh', 'tmux', 'ip', 'ping', 'infocmp', 'flock', 'tar', 'gzip', 'stat', 'timeout', 'install', 'mktemp', 'head', 'sed', 'grep'):
        found = shutil.which(command); assert found, command
        (enroll_tools/command).symlink_to(found)
    reset()
    enrollment = subprocess.run(['/bin/sh', str(source.parent/'scripts/enroll-helper.sh')], input=body,
        env=dict(env, PATH=str(enroll_tools)), capture_output=True, timeout=5)
    assert enrollment.returncode != 0 and b'pdftoppm' in enrollment.stderr
    assert target.read_bytes() == old
    outcomes.append('enrollment prerequisites checked before helper replacement')
    (fake/'id').write_text('#!/bin/sh\necho 0\n');(fake/'id').chmod(0o700);result=run();assert result.returncode!=0 and 'ordinary user' in result.stderr;outcomes.append('root refusal')
    print(json.dumps({'result':'PASS','scenarios':outcomes,'signature':'REAL synthetic RSA4096 SHA256; temporary script substitutes pinned public key','public_release':'not tested by this fixture'}))
