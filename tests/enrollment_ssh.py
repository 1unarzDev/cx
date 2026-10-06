#!/usr/bin/env python3
"""Disposable native OpenSSH enrollment broker/multiplex regression fixture.
Usage: python tests/enrollment_ssh.py [source-root]
Compiles the actual auth/store/model modules in an isolated temporary harness.
No personal SSH configuration, trust files, accounts, or installed helpers change.
"""
import json, os, pathlib, pwd, shutil, signal, socket, subprocess, sys, tempfile, time

source = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else pathlib.Path(__file__).resolve().parents[1]).resolve()
cargo = shutil.which('cargo') or str(pathlib.Path.home()/'.cargo/bin/cargo')
sshd = shutil.which('sshd') or '/usr/sbin/sshd'
ssh = shutil.which('ssh') or '/usr/bin/ssh'
with tempfile.TemporaryDirectory(prefix='cx-a-', dir='/tmp') as temporary:
    root = pathlib.Path(temporary); root.chmod(0o700)
    for name, password in [('host',''), ('key','synthetic-fixture-passphrase')]:
        subprocess.run(['ssh-keygen','-q','-t','ed25519','-N',password,'-f',str(root/name)],check=True,timeout=10)
    (root/'authorized').write_text((root/'key.pub').read_text()); (root/'authorized').chmod(0o600)
    def port():
        with socket.socket() as sock:
            sock.bind(('127.0.0.1',0)); return sock.getsockname()[1]
    first_port, changed_port = port(), port()
    account = pwd.getpwuid(os.getuid()).pw_name
    (root/'daemon.conf').write_text(f'''Port {first_port}
ListenAddress 127.0.0.1
HostKey {root}/host
PidFile {root}/pid
AuthorizedKeysFile {root}/authorized
StrictModes no
UsePAM no
PasswordAuthentication no
KbdInteractiveAuthentication no
PubkeyAuthentication yes
AllowUsers {account}
''')
    def client_config(value):
        (root/'client.conf').write_text(f'''Host fixture
 HostName 127.0.0.1
 Port {value}
 User {account}
 IdentityFile {root}/key
 IdentitiesOnly yes
 IdentityAgent none
 UserKnownHostsFile {root}/known
 GlobalKnownHostsFile /dev/null
 StrictHostKeyChecking ask
''')
    client_config(first_port)
    tools = root/'tools'; tools.mkdir()
    # Preserve cx's argv policy but replace its includes with disposable config.
    wrapper = tools/'ssh'
    wrapper.write_text(f'''#!{sys.executable}
import os,sys
args=sys.argv[1:]
if '-F' in args:
 pos=args.index('-F');del args[pos:pos+2]
os.execv({ssh!r},[{ssh!r},'-F',os.environ['CX_FIXTURE_CONFIG'],*args])
'''); wrapper.chmod(0o700)
    (root/'info-helper.py').write_text('''import json,sys
header=sys.stdin.buffer.readline().decode().split()
assert header[0]=='CX1'
request=json.loads(sys.stdin.buffer.read(int(header[1])))
assert request['op']['op']=='info'
response=json.dumps(dict(version=1,id=request['id'],result=dict(host='fixture-host'),error=None)).encode()
sys.stdout.buffer.write(('CX1 %d\\n'%len(response)).encode()+response)
''')
    harness = root/'harness'; (harness/'src').mkdir(parents=True)
    (harness/'Cargo.toml').write_text('''[package]
name="cx-enrollment-fixture"
version="0.0.0"
edition="2021"
[dependencies]
anyhow="1"
libc="0.2"
serde={version="1",features=["derive"]}
serde_json="1"
sha2="0.10"
''')
    modules = '\n'.join(f'#[path={json.dumps(str(source/"src"/(name+".rs")))}] mod {name};' for name in ['auth','model','store'])
    (harness/'src/main.rs').write_text(modules + r'''
use anyhow::{bail,Result};
use std::{process::{Command,Stdio},os::unix::process::CommandExt,fs};
fn main()->Result<()> {
 if auth::is_askpass_client(){std::process::exit(if auth::askpass_client().is_ok(){0}else{1});}
 let mut keys=0; let mut trust=0;
 auth::with_broker(|| {
   let mut c=store::ssh("fixture",true)?;
   c.arg("printf fixture-probe").stdin(Stdio::null()).process_group(0);
   let mut child=c.spawn()?;let pid=child.id();let status=child.wait()?;
   unsafe{libc::kill(-(pid as i32),libc::SIGKILL);}
   anyhow::ensure!(status.success(),"native encrypted-key login failed");Ok(())
 },|p|match p.kind{
   auth::PromptKind::KeyPassphrase=>{keys+=1;Ok(auth::Answer::Submit("synthetic-fixture-passphrase".into()))},
   auth::PromptKind::HostKey=>{trust+=1;anyhow::ensure!(p.text.contains("SHA256:"),"missing host fingerprint");Ok(auth::Answer::Submit("yes".into()))},
   _=>bail!("unexpected native authentication kind")
 })?;
 anyhow::ensure!(keys==1 && trust==1,"unexpected native prompt count");
 // A real version-1 framed Info request over a background BatchMode connection.
 let request=model::Request{version:1,id:"fixture".into(),op:model::Operation::Info};
 let payload=serde_json::to_vec(&request)?;
 let mut background=store::ssh("fixture",false)?;
 background.arg(std::env::var("CX_FIXTURE_HELPER_COMMAND")?).stdin(Stdio::piped()).stdout(Stdio::piped());
 let mut child=background.spawn()?;
 {use std::io::Write;let mut input=child.stdin.take().unwrap();
 write!(input,"CX1 {}\n",payload.len())?;input.write_all(&payload)?;}
 let output=child.wait_with_output()?;
 anyhow::ensure!(output.status.success(),"background master reuse failed");
 let output=String::from_utf8(output.stdout)?;
 let (header,payload)=output.split_once('\n').ok_or_else(||anyhow::anyhow!("missing frame"))?;
 anyhow::ensure!(header==format!("CX1 {}",payload.len()),"bad response length");
 let response:model::Response=serde_json::from_str(payload)?;
 anyhow::ensure!(response.id=="fixture" && response.result.unwrap()["host"]=="fixture-host","missing background Info observation");
 // No key prompt callback is available now: a changed alias must not reuse old master.
 let path=std::env::var("CX_FIXTURE_CONFIG")?;let old=std::env::var("CX_FIXTURE_PORT")?;
 let changed=std::env::var("CX_FIXTURE_CHANGED_PORT")?;
 let config=fs::read_to_string(&path)?;fs::write(path,config.replace(&format!("Port {old}"),&format!("Port {changed}")))?;
 let changed=store::ssh("fixture",false)?.arg("printf WRONG-HOST").stdin(Stdio::null()).output()?;
 anyhow::ensure!(!changed.status.success(),"changed alias reused old authenticated endpoint");
 println!("encrypted_key=true host_key=true background_info_reused=true alias_port_isolated=true");
 Ok(())
}
''')
    environment = dict(os.environ, XDG_STATE_HOME=str(root/'state'), PATH=str(tools)+os.pathsep+os.environ['PATH'], CX_FIXTURE_CONFIG=str(root/'client.conf'), CX_FIXTURE_HELPER_COMMAND=f'{sys.executable} {root}/info-helper.py', CX_FIXTURE_PORT=str(first_port), CX_FIXTURE_CHANGED_PORT=str(changed_port))
    log = open(root/'daemon.log','wb')
    daemon = subprocess.Popen([sshd,'-D','-e','-f',str(root/'daemon.conf')],stdout=log,stderr=log,start_new_session=True)
    try:
        for _ in range(100):
            if daemon.poll() is not None: raise RuntimeError((root/'daemon.log').read_text())
            try:
                with socket.create_connection(('127.0.0.1',first_port),timeout=.1): break
            except OSError: time.sleep(.02)
        subprocess.run([cargo,'build','--quiet','--offline','--manifest-path',str(harness/'Cargo.toml')],check=True,timeout=90,stderr=subprocess.PIPE)
        probe = subprocess.Popen([str(harness/'target/debug/cx-enrollment-fixture')],env=environment,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True,start_new_session=True)
        try:
            output, errors = probe.communicate(timeout=25)
        except subprocess.TimeoutExpired:
            os.killpg(probe.pid,signal.SIGKILL); probe.communicate(timeout=3); raise
        assert probe.returncode==0, errors + (root/'daemon.log').read_text()[-3000:]
        assert 'alias_port_isolated=true' in output, output
        print(json.dumps({'result':'PASS','encrypted_key_prompt':True,'explicit_host_trust':True,'owned_probe_group_reaped_master_survives':True,'background_batchmode_info_reuses_master':True,'changed_alias_port_does_not_reuse_master':True}))
    finally:
        # Close only private fixture masters. Their files are never user sockets.
        for candidate in (root/'state/cx/ssh').glob('*'):
            subprocess.run([ssh,'-F',str(root/'client.conf'),'-S',str(candidate),'-O','exit','fixture'],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL,timeout=3)
        if daemon.poll() is None: os.killpg(daemon.pid,signal.SIGTERM)
        daemon.wait(timeout=3); log.close()
