use crate::model::{CreateSession, Device, Session};
use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};
use std::{fs, io::Write, os::unix::{fs::{OpenOptionsExt, PermissionsExt}, io::AsRawFd}, path::PathBuf, process::{Command, Stdio}, thread, time::Duration};

fn state() -> Result<PathBuf> {
    let root = PathBuf::from(std::env::var_os("HOME").context("HOME unavailable")?).join(".local/state/cx");
    fs::create_dir_all(&root)?;
    // Refuse foreign ownership and symlinks before changing permissions.
    use std::os::unix::fs::MetadataExt;
    let meta = fs::symlink_metadata(&root)?;
    if !meta.is_dir() || meta.uid() != unsafe { libc::geteuid() } { bail!("unsafe cx state directory"); }
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700))?;
    Ok(root)
}
fn socket() -> Result<PathBuf> { Ok(state()?.join("managed.sock")) }
fn tmux(managed: bool) -> Result<Command> {
    let mut c = Command::new("tmux");
    if managed { c.arg("-S").arg(socket()?); }
    Ok(c)
}
fn output(mut c: Command) -> Result<String> {
    let o = c.output().context("tmux unavailable; install tmux on the execution device")?;
    if !o.status.success() { bail!("tmux operation failed: {}", String::from_utf8_lossy(&o.stderr).trim()); }
    let text=String::from_utf8_lossy(&o.stdout); Ok(text.strip_suffix('\n').unwrap_or(&text).to_string())
}
fn field(managed: bool, id: &str, fmt: &str) -> Result<String> {
    let mut c = tmux(managed)?; c.args(["display-message", "-p", "-t", id, fmt]); output(c)
}
fn identity() -> (String, String, String) {
    let host = fs::read_to_string("/proc/sys/kernel/hostname").unwrap_or_else(|_| "unknown".into()).trim().to_string();
    let user = unsafe { let p=libc::getpwuid(libc::geteuid()); if p.is_null() { "unknown".into() } else { std::ffi::CStr::from_ptr((*p).pw_name).to_string_lossy().into_owned() } };
    let boot = fs::read_to_string("/proc/sys/kernel/random/boot_id").unwrap_or_default().trim().to_string();
    (host,user,boot)
}
fn ids(managed: bool) -> Result<Vec<String>> {
    let mut c = tmux(managed)?; c.args(["list-sessions", "-F", "#{session_id}"]);
    let out = c.output()?;
    if !out.status.success() {
        let e=String::from_utf8_lossy(&out.stderr);
        if e.contains("no server running") || e.contains("No such file") || e.contains("Connection refused") { return Ok(vec![]); }
        bail!("cannot list tmux sessions: {}",e.trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).lines().filter(|s| s.starts_with('$') && s[1..].bytes().all(|b|b.is_ascii_digit())).map(str::to_owned).collect())
}
fn inspect(managed: bool, id: &str) -> Result<Session> {
    let (host,account,boot_id)=identity();
    let name=field(managed,id,"#{session_name}")?;
    let record=state()?.join(format!("{name}.json"));
    let prior:Option<Session> = if managed && name.len()==67 && name.starts_with("cx-") && name[3..].bytes().all(|b|b.is_ascii_hexdigit()) { fs::read(&record).ok().and_then(|v|serde_json::from_slice(&v).ok()) } else { None };
    let pid=field(managed,id,"#{pane_pid}")?.parse().unwrap_or(0);
    let process_start=fs::read_to_string(format!("/proc/{pid}/stat")).ok().and_then(|v| v.rsplit_once(')').and_then(|(_,rest)|rest.split_whitespace().nth(19).map(str::to_owned))).unwrap_or_else(||"unknown".into());
    let started=format!("{}:{process_start}",field(managed,id,"#{session_created}")?);
    Ok(Session { id:if managed {name.clone()} else {id.into()}, name:prior.as_ref().map(|s|s.name.clone()).unwrap_or(name), directory:field(managed,id,"#{pane_current_path}")?, provider:prior.map(|s|s.provider).unwrap_or_else(||"shell".into()), host,account,pid,started,boot_id,external:!managed,socket:if managed{Some(socket()?.to_string_lossy().into_owned())}else{None} })
}
pub fn list() -> Result<Vec<Session>> {
    let mut result=vec![];
    for managed in [true,false] { for id in ids(managed)? { if let Ok(s)=inspect(managed,&id) { result.push(s); } } }
    result.sort_by(|a,b|a.name.cmp(&b.name)); Ok(result)
}
fn ensure_server() -> Result<()> {
    if !ids(true)?.is_empty() { return Ok(()); }
    let root=state()?;
    let conf=root.join("tmux.conf");
    fs::write(&conf,include_str!("../assets/tmux.conf"))?;
    fs::set_permissions(&conf,fs::Permissions::from_mode(0o600))?;
    let sock=socket()?;
    // A foreground tmux server in its own user service cannot inherit the metadata service cgroup.
    let bus=PathBuf::from(format!("/run/user/{}/bus",unsafe{libc::geteuid()}));
    if bus.exists() {
        let unit=format!("--unit=cx-tmux-{:x}",Sha256::digest(sock.as_os_str().as_encoded_bytes()));
        let status=Command::new("systemd-run").args(["--user","--quiet","--collect",&unit,"--property=Restart=no"])
            .arg("tmux").arg("-D").arg("-S").arg(&sock).arg("-f").arg(&conf).stdout(Stdio::null()).stderr(Stdio::null()).status()?;
        // An existing owned service is acceptable if its socket responds.
        for _ in 0..40 { let mut c=tmux(true)?; c.args(["show-options","-g","exit-empty"]); if c.output()?.status.success(){return Ok(());} thread::sleep(Duration::from_millis(25)); }
        bail!("persistent tmux user service did not start (systemd-run {status}); check user-service access");
    }
    let cgroup=fs::read_to_string("/proc/self/cgroup").unwrap_or_default();
    if cgroup.contains(".service") { bail!("user services unavailable; refusing to put sessions in the helper service cgroup"); }
    let mut c=tmux(true)?; c.arg("-f").arg(conf).arg("start-server"); output(c)?; Ok(())
}
pub fn create(request: &CreateSession) -> Result<Session> {
    if request.key.is_empty() || request.key.len()>1024 { bail!("creation key must be 1–1024 bytes"); }
    if !["shell","claude","codex"].contains(&request.provider.as_str()) { bail!("unsupported launch profile"); }
    let directory=fs::canonicalize(&request.directory).context("execution directory unavailable")?;
    if !directory.is_dir(){bail!("execution location is not a directory");}
    let root=state()?;
    let lock=fs::OpenOptions::new().create(true).truncate(false).read(true).write(true).mode(0o600).open(root.join("sessions.lock"))?;
    if unsafe{libc::flock(lock.as_raw_fd(),libc::LOCK_EX)}!=0 {bail!("session lock unavailable");}
    let name=format!("cx-{:x}",Sha256::digest(request.key.as_bytes()));
    let mut exists=tmux(true)?;exists.args(["has-session","-t",&format!("={name}")]);
    if exists.output()?.status.success(){return inspect(true,&name);}
    if root.join(format!("{name}.json")).exists() { bail!("original session has ended; use a new creation key to start replacement work"); }
    ensure_server()?;
    let shell=std::env::var("SHELL").unwrap_or_else(|_|"/bin/sh".into());
    let basename=std::path::Path::new(&shell).file_name().and_then(|v|v.to_str()).unwrap_or("");
    if !["fish","bash","zsh","sh","dash"].contains(&basename) {bail!("unsupported login shell: choose Fish, Bash, Zsh or sh");}
    let (host,account,boot_id)=identity();
    let pending=Session {id:name.clone(),name:request.name.clone(),directory:directory.to_string_lossy().into_owned(),provider:request.provider.clone(),host,account,pid:0,started:String::new(),boot_id,external:false,socket:Some(socket()?.to_string_lossy().into_owned())};
    // Persist intent before starting: an interrupted helper response must never lose the launcher identity.
    let mut intent=fs::OpenOptions::new().create_new(true).write(true).mode(0o600).open(root.join(format!("{name}.json")))?;
    intent.write_all(&serde_json::to_vec(&pending)?)?; intent.sync_all()?;
    let mut c=tmux(true)?;
    c.args(["new-session","-d","-s",&name,"-n","work","-c"]).arg(directory).arg(&shell);
    if request.provider=="shell" {c.arg("-l");} else { c.args(["-l","-i","-c",&request.provider]); }
    output(c)?;
    let mut session=inspect(true,&name)?;session.name=request.name.clone();session.provider=request.provider.clone();
    let temp=root.join(format!("{name}.tmp"));
    let mut f=fs::OpenOptions::new().create(true).truncate(true).write(true).mode(0o600).open(&temp)?;
    f.write_all(&serde_json::to_vec(&session)?)?;f.sync_all()?;fs::rename(temp,root.join(format!("{name}.json")))?;
    let safe=|v:&str|v.chars().filter(|c|!c.is_control() && *c!='#').take(50).collect::<String>();
    let status=format!("{}@{} | {}",safe(&session.account),safe(&session.host),safe(&session.name));
    let mut c=tmux(true)?;c.args(["set-option","-t",&name,"status-right",&status]);output(c)?;
    Ok(session)
}
fn quote(s:&str)->String{format!("'{}'",s.replace('\'',"'\\''"))}
pub fn attach(device:&Device,session:&Session,observe:bool)->Result<()> {
    if std::env::var_os("TMUX").is_some(){bail!("cx attachment from inside tmux requires leaving the outer client first; protected outer sessions are kept intact");}
    if session.external {
        if !session.id.starts_with('$') || !session.id[1..].bytes().all(|b|b.is_ascii_digit()){bail!("invalid external session identity");}
    } else if !session.id.starts_with("cx-") || session.id.len()!=67 || !session.id[3..].bytes().all(|b|b.is_ascii_hexdigit()){bail!("invalid managed session identity");}
    let mut args=vec!["attach-session".to_string(),"-t".into(),if session.external{session.id.clone()}else{format!("={}",session.id)}];
    if observe {args.extend(["-r".into(),"-f".into(),"ignore-size".into()]);}
    let mut c=if let Some(target)=&device.target {
        if target.starts_with('-') || target.bytes().any(|b|b.is_ascii_whitespace() || b<32){bail!("invalid SSH target");}
        let mut c=Command::new("ssh"); c.args(["-t","-o","ForwardAgent=no","--",target]);
        let prefix=if session.external{"tmux".to_string()}else{"tmux -S ~/.local/state/cx/managed.sock".to_string()};
        c.arg(format!("{prefix} {}",args.iter().map(|a|quote(a)).collect::<Vec<_>>().join(" ")));c
    }else{let mut c=tmux(!session.external)?;c.args(args);c};
    let result=c.status().context("native terminal attachment failed")?;
    if !result.success(){bail!("native terminal attachment ended with {result}");} Ok(())
}
