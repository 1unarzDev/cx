mod model;
mod store;
mod transport;
mod sessions;
mod files;
mod network;
mod ui;
use anyhow::{bail,Context,Result};
use clap::{Parser,Subcommand};
use model::*;
use std::{io::{BufReader,Write},process::{Command,Stdio}};
#[derive(Parser)] #[command(version,about="A terminal workspace across your devices")]
struct Cli { #[command(subcommand)] command:Option<Cmd> }
#[derive(Subcommand)] enum Cmd {
 Add{target:String}, Devices, Sessions{#[arg(long)] device:Option<String>},
 New{#[arg(long)] device:Option<String>,#[arg(long,default_value="shell")] provider:String,#[arg(long)] directory:String,#[arg(long)] key:Option<String>,#[arg(long,default_value="work")] name:String},
 Attach{id:String,#[arg(long)] device:Option<String>,#[arg(long)] observe:bool},
 Files{path:String,#[arg(long)] device:Option<String>}, Network{#[arg(long)] device:Option<String>},
 #[command(hide=true)] Helper,
}
fn info()->Result<serde_json::Value> { let d=store::local_device(); let machine=std::fs::read_to_string("/etc/machine-id").unwrap_or_default(); let mut caps=Vec::new(); for tool in ["tmux","codex","claude"] { if Command::new(tool).arg("--version").stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s|s.success()){caps.push(tool)} } Ok(serde_json::json!({"host":d.host,"account":d.account,"machine_id":machine.trim(),"capabilities":caps,"protocol":1,"version":env!("CARGO_PKG_VERSION")})) }
pub fn dispatch(op:Operation)->Result<serde_json::Value> { match op {Operation::Info=>info(),Operation::Sessions=>Ok(serde_json::to_value(sessions::list()?)?),Operation::Create(ref c)=>Ok(serde_json::to_value(sessions::create(c)?)?),Operation::Network=>network::observe(),ref o=>files::handle(o)} }
fn helper()->Result<()> { let req:Request=transport::read_frame(&mut BufReader::new(std::io::stdin().lock()))?; let result=if req.version!=1 {Err(anyhow::anyhow!("unsupported helper protocol"))}else{dispatch(req.op)}; let response=match result {Ok(v)=>Response{version:1,id:req.id,result:Some(v),error:None},Err(e)=>Response{version:1,id:req.id,result:None,error:Some(format!("{e:#}"))}};transport::frame(&mut std::io::stdout().lock(),&response) }
fn device(name:Option<String>)->Result<Device> { let ds=store::devices()?; if let Some(n)=name {ds.into_iter().find(|d|d.name==n || d.id==n || d.target.as_deref()==Some(&n)).context("device not enrolled; use cx add") }else{Ok(store::local_device())} }
fn add(target:&str)->Result<()> { if !transport::valid_target(target){bail!("invalid SSH target")}; let p=store::ensure()?.join("maintenance.lock"); let lock=std::fs::File::create(p)?; use std::os::fd::AsRawFd; if unsafe{libc::flock(lock.as_raw_fd(),libc::LOCK_EX|libc::LOCK_NB)}!=0{bail!("another cx maintenance operation is active")};
 let mut c=transport::ssh(target,true)?; c.arg("uname -sm; id -un"); let out=c.output()?; if !out.status.success(){bail!("SSH access failed; verify or unlock this host with ssh {target}")}; let facts=String::from_utf8(out.stdout)?; let lines:Vec<_>=facts.lines().collect(); if !lines.iter().any(|s|*s=="Linux x86_64"){bail!("enrollment currently supports Linux x86_64; host architecture differs")};
 let binary=std::env::current_exe()?; let bytes=std::fs::read(binary)?;
 let mut c=transport::ssh(target,true)?; c.arg("umask 077; mkdir -p ~/.local/bin ~/.local/state/cx; (flock -n 9 || exit 75; cat > ~/.local/bin/cx.installing && chmod 700 ~/.local/bin/cx.installing && mv ~/.local/bin/cx.installing ~/.local/bin/cx) 9> ~/.local/state/cx/maintenance.lock").stdin(Stdio::piped()); let mut child=c.spawn()?; child.stdin.take().context("installer stdin")?.write_all(&bytes)?; if !child.wait()?.success(){bail!("helper installation failed")};
 let mut d=Device{id:target.into(),name:target.into(),target:Some(target.into()),account:String::new(),host:String::new(),status:"unknown".into(),observed_at:0}; let value=transport::request(&d,Operation::Info)?; d.account=value["account"].as_str().unwrap_or("unknown").into(); d.host=value["host"].as_str().unwrap_or(target).into(); d.id=format!("{}:{}",value["machine_id"].as_str().unwrap_or(&d.host),d.account); d.status="reachable".into(); d.observed_at=transport::now(); let mut ds=store::devices()?; if let Some(old)=ds.iter_mut().find(|o|o.id==d.id || (o.host==d.host && o.account==d.account)){*old=d.clone()}else{ds.push(d.clone())}; store::save_devices(&ds)?; println!("Added {} · {}@{} · {}",d.name,d.account,d.host,value["capabilities"]); Ok(()) }
fn run()->Result<()> { match Cli::parse().command {None=>ui::run(),Some(Cmd::Helper)=>helper(),Some(Cmd::Add{target})=>add(&target),Some(Cmd::Devices)=>{println!("{}",serde_json::to_string_pretty(&store::devices()?)?);Ok(())},Some(Cmd::Sessions{device:d})=>{println!("{}",transport::request(&device(d)?,Operation::Sessions)?);Ok(())},Some(Cmd::New{device:d,provider,directory,key,name})=>{let key=key.unwrap_or_else(||format!("{}-{}",std::process::id(),transport::now()));println!("{}",transport::request(&device(d)?,Operation::Create(CreateSession{key,directory,provider,name}))?);Ok(())},Some(Cmd::Attach{id,device:d,observe})=>{let d=device(d)?;let list:Vec<Session>=serde_json::from_value(transport::request(&d,Operation::Sessions)?)?;let s=list.iter().find(|s|s.id==id || s.name==id).context("session not found")?;sessions::attach(&d,s,observe)},Some(Cmd::Files{path,device:d})=>{println!("{}",transport::request(&device(d)?,Operation::List{path})?);Ok(())},Some(Cmd::Network{device:d})=>{println!("{}",transport::request(&device(d)?,Operation::Network)?);Ok(())}} }
fn main(){if let Err(e)=run(){eprintln!("cx: {e:#}");std::process::exit(1)}}
