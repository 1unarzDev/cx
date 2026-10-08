//! Containers belong to enrolled hosts. Discovery never executes inside a container.
//! Exact IDs and lifecycle checks keep ordinary service containers opt-in.
use crate::model::{ContainerScope, Operation, Request, Response};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    fs,
    io::{Read, Seek, SeekFrom, Write},
    os::unix::{
        fs::OpenOptionsExt,
        io::{AsRawFd, FromRawFd},
        process::CommandExt,
    },
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Container {
    #[serde(default)]
    pub engine: String,
    pub id: String,
    pub name: String,
    pub image: String,
    pub state: String,
    pub started_at: String,
    pub devcontainer: bool,
    pub evidence: String,
    pub user: String,
    pub folder: String,
    pub workspace: Option<String>,
    pub config: Option<String>,
    pub network: String,
    pub networks: Vec<String>,
    pub ports: Value,
    pub allowed: bool,
    #[serde(skip)]
    pub remote_env: std::collections::BTreeMap<String, String>,
}
impl Container {
    pub fn scope(&self) -> ContainerScope {
        ContainerScope {
            engine: self.engine.clone(),
            id: self.id.clone(),
            name: self.name.clone(),
            user: self.user.clone(),
            started_at: self.started_at.clone(),
            folder: self.folder.clone(),
        }
    }
}
pub fn valid_id(id: &str) -> bool {
    id.len() == 64 && id.bytes().all(|b| b.is_ascii_hexdigit())
}

fn engine_identity() -> Result<String> {
    use sha2::{Digest, Sha256};
    // Execution scopes belong to the enrolled host, never an unrelated remote daemon.
    let endpoint = if let Some(context) = std::env::var("DOCKER_CONTEXT")
        .ok()
        .filter(|s| !s.is_empty())
    {
        text(bounded(
            docker(&[
                "context",
                "inspect",
                &context,
                "--format",
                "{{.Endpoints.docker.Host}}",
            ]),
            None,
            5,
            65536,
        )?)?
    } else if let Some(host) = std::env::var("DOCKER_HOST").ok().filter(|s| !s.is_empty()) {
        host
    } else {
        text(bounded(
            docker(&[
                "context",
                "inspect",
                "--format",
                "{{.Endpoints.docker.Host}}",
            ]),
            None,
            5,
            65536,
        )?)?
    };
    anyhow::ensure!(endpoint.starts_with("unix://"), "CX containers require a local Docker socket on this device. Enroll the Docker host through SSH instead of selecting a remote Docker context");
    let id = text(bounded(
        docker(&["info", "--format", "{{.ID}}"]),
        None,
        5,
        65536,
    )?)?;
    anyhow::ensure!(!id.is_empty(), "Docker engine identity unavailable");
    Ok(format!(
        "{:x}",
        Sha256::digest(format!("{endpoint}\n{id}").as_bytes())
    ))
}
/// Explicit startup may execute workspace hooks and Compose services. Never called by discovery.
pub fn up(workspace: &str) -> Result<Value> {
    let workspace =
        fs::canonicalize(workspace).context("Workspace does not exist on this device")?;
    anyhow::ensure!(workspace.is_dir(), "Choose a workspace folder");
    let config = config_at(&workspace)
        .context("No .devcontainer/devcontainer.json or .devcontainer.json in this folder")?;
    read_config(&config)?;
    let cli = cli().context("Install the Node Dev Containers CLI on this device to start a workspace; existing containers still support Docker attachment")?;
    let engine = engine_identity()?;
    let mut command = cli_command(&cli);
    command
        .args(["up", "--workspace-folder"])
        .arg(&workspace)
        .arg("--config")
        .arg(&config);
    let bytes = bounded(command, None, 600, 1024 * 1024)?;
    let result: Value = bytes
        .split(|b| *b == b'\n')
        .rev()
        .filter_map(|line| serde_json::from_slice::<Value>(line).ok())
        .find(|v| v.get("containerId").is_some())
        .context("CLI did not return a container ID; refresh to check the outcome")?;
    anyhow::ensure!(
        result["outcome"] == "success",
        "Workspace startup did not succeed; refresh to check its state"
    );
    anyhow::ensure!(
        engine_identity()? == engine,
        "Docker engine changed during workspace startup"
    );
    let id = result["containerId"]
        .as_str()
        .context("CLI returned no container identity")?;
    Ok(serde_json::to_value(inspect(id)?)?)
}

fn memfile() -> Result<fs::File> {
    let fd = unsafe { libc::memfd_create(c"cx-container-io".as_ptr(), libc::MFD_CLOEXEC) };
    anyhow::ensure!(fd >= 0, "bounded container I/O storage unavailable");
    Ok(unsafe { fs::File::from_raw_fd(fd) })
}
fn bounded(
    mut command: Command,
    input: Option<fs::File>,
    timeout: u64,
    cap: u64,
) -> Result<Vec<u8>> {
    let mut out = memfile()?;
    let err = memfile()?;
    command
        .process_group(0)
        .stdin(input.map(Stdio::from).unwrap_or_else(Stdio::null))
        .stdout(out.try_clone()?)
        .stderr(err.try_clone()?);
    let mut child = command
        .spawn()
        .context("Container engine/tool unavailable on this device")?;
    let deadline = Instant::now() + Duration::from_secs(timeout);
    let result = loop {
        if out.metadata()?.len().saturating_add(err.metadata()?.len()) > cap {
            break Err(anyhow::anyhow!("container command output exceeded limit"));
        }
        if Instant::now() > deadline {
            break Err(anyhow::anyhow!(
                "container command timed out; refresh before retrying"
            ));
        }
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status.success()),
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Err(e) => break Err(e.into()),
        }
    };
    unsafe {
        libc::kill(-(child.id() as i32), libc::SIGKILL);
    }
    let _ = child.kill();
    let _ = child.wait();
    anyhow::ensure!(result?,"Container command failed. Check Docker access, container state and tooling on this device; command output is kept private");
    out.seek(SeekFrom::Start(0))?;
    let mut bytes = Vec::new();
    out.take(cap + 1).read_to_end(&mut bytes)?;
    anyhow::ensure!(
        bytes.len() as u64 <= cap,
        "container command output exceeded limit"
    );
    Ok(bytes)
}
fn input(bytes: &[u8]) -> Result<fs::File> {
    let mut f = memfile()?;
    f.write_all(bytes)?;
    f.seek(SeekFrom::Start(0))?;
    Ok(f)
}
fn docker(args: &[&str]) -> Command {
    let mut c = Command::new("docker");
    c.args(args);
    c
}
fn text(bytes: Vec<u8>) -> Result<String> {
    Ok(String::from_utf8(bytes)?.trim().into())
}
fn value_string(v: &Value, key: &str) -> String {
    v[key].as_str().unwrap_or_default().into()
}
fn config_at(workspace: &Path) -> Option<PathBuf> {
    [
        workspace.join(".devcontainer/devcontainer.json"),
        workspace.join(".devcontainer.json"),
    ]
    .into_iter()
    .find(|p| p.is_file())
}
// JSONC comment/trailing-comma removal preserves literals and is bounded by the caller.
fn jsonc(raw: &str) -> Result<Value> {
    let bytes = raw.as_bytes();
    let mut clean = Vec::with_capacity(bytes.len());
    let mut i = 0;
    let mut string = false;
    let mut escape = false;
    while i < bytes.len() {
        let b = bytes[i];
        if string {
            clean.push(b);
            if escape {
                escape = false;
            } else if b == b'\\' {
                escape = true;
            } else if b == b'"' {
                string = false;
            }
            i += 1;
            continue;
        }
        if b == b'"' {
            string = true;
            clean.push(b);
            i += 1;
            continue;
        }
        if b == b'/' && bytes.get(i + 1) == Some(&b'/') {
            i += 2;
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if b == b'/' && bytes.get(i + 1) == Some(&b'*') {
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            anyhow::ensure!(i + 1 < bytes.len(), "unterminated configuration comment");
            i += 2;
            clean.push(b' ');
            continue;
        }
        clean.push(b);
        i += 1;
    }
    let mut final_bytes = Vec::with_capacity(clean.len());
    string = false;
    escape = false;
    for (i, &b) in clean.iter().enumerate() {
        if string {
            final_bytes.push(b);
            if escape {
                escape = false;
            } else if b == b'\\' {
                escape = true;
            } else if b == b'"' {
                string = false;
            }
        } else {
            if b == b'"' {
                string = true;
            }
            if b == b','
                && clean[i + 1..]
                    .iter()
                    .find(|b| !b.is_ascii_whitespace())
                    .is_some_and(|b| matches!(b, b'}' | b']'))
            {
                continue;
            }
            final_bytes.push(b);
        }
    }
    Ok(serde_json::from_slice(&final_bytes)?)
}
fn read_config(path: &Path) -> Result<Value> {
    let mut data = String::new();
    fs::File::open(path)?
        .take(262145)
        .read_to_string(&mut data)?;
    anyhow::ensure!(
        data.len() <= 262144,
        "devcontainer configuration exceeds limit"
    );
    jsonc(&data)
}
fn allow_path() -> Result<PathBuf> {
    Ok(crate::store::ensure()?.join("container-access.json"))
}
fn allowed(id: &str) -> Result<bool> {
    let p = allow_path()?;
    match fs::read(p) {
        Ok(b) => {
            anyhow::ensure!(b.len() < 65536, "container access policy exceeds limit");
            let ids: Vec<String> = serde_json::from_slice(&b)?;
            Ok(ids.iter().any(|s| s == id))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e.into()),
    }
}
pub fn set_access(id: &str, engine: &str, enable: bool) -> Result<Value> {
    let c = inspect(id)?;
    anyhow::ensure!(
        c.engine == engine,
        "Docker engine changed; refresh before changing access"
    );
    let p = allow_path()?;
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .mode(0o600)
        .open(p.with_extension("lock"))?;
    anyhow::ensure!(
        unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX) } == 0,
        "container policy lock unavailable"
    );
    let mut ids: Vec<String> = match fs::read(&p) {
        Ok(b) => {
            anyhow::ensure!(b.len() < 65536, "container access policy exceeds limit");
            serde_json::from_slice(&b)?
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => vec![],
        Err(e) => return Err(e.into()),
    };
    let key = format!("{}:{}", c.engine, id);
    ids.retain(|s| s != &key);
    if enable {
        ids.push(key);
    }
    let temp = p.with_extension(format!("new-{}", std::process::id()));
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temp)?;
    f.write_all(&serde_json::to_vec(&ids)?)?;
    f.sync_all()?;
    fs::rename(temp, p)?;
    Ok(json!({"enabled":enable}))
}
const INSPECT: &str = r#"{"id":{{json .Id}},"name":{{json .Name}},"image":{{json .Config.Image}},"state":{{json .State.Status}},"started_at":{{json .State.StartedAt}},"user":{{json .Config.User}},"folder":{{json .Config.WorkingDir}},"network":{{json .HostConfig.NetworkMode}},"networks":{{json .NetworkSettings.Networks}},"ports":{{json .NetworkSettings.Ports}},"mounts":{{json .Mounts}},"workspace":{{json (index .Config.Labels "devcontainer.local_folder")}},"legacy_workspace":{{json (index .Config.Labels "vsch.local.folder")}},"config":{{json (index .Config.Labels "devcontainer.config_file")}},"metadata":{{json (index .Config.Labels "devcontainer.metadata")}}}"#;
pub fn inspect(id: &str) -> Result<Container> {
    inspect_on(id, &engine_identity()?)
}
fn inspect_on(id: &str, engine: &str) -> Result<Container> {
    anyhow::ensure!(valid_id(id), "choose a full, verified container ID");
    let raw = bounded(
        docker(&["inspect", "--type", "container", "--format", INSPECT, id]),
        None,
        8,
        1024 * 1024,
    )?;
    let v: Value = serde_json::from_slice(&raw)?;
    anyhow::ensure!(v["id"].as_str() == Some(id), "container identity changed");
    let mut c = Container {
        engine: engine.into(),
        id: id.into(),
        name: value_string(&v, "name").trim_start_matches('/').into(),
        image: value_string(&v, "image"),
        state: value_string(&v, "state"),
        started_at: value_string(&v, "started_at"),
        devcontainer: false,
        evidence: "Docker container · inspection only".into(),
        user: value_string(&v, "user"),
        folder: value_string(&v, "folder"),
        workspace: None,
        config: None,
        network: value_string(&v, "network"),
        networks: v["networks"]
            .as_object()
            .map(|n| n.keys().cloned().collect())
            .unwrap_or_default(),
        ports: v["ports"].clone(),
        allowed: allowed(&format!("{engine}:{id}"))?,
        remote_env: Default::default(),
    };
    if c.user.is_empty() {
        c.user = "root".into();
    }
    if c.folder.is_empty() {
        c.folder = "/".into();
    }
    let metadata = v["metadata"]
        .as_str()
        .filter(|s| s.len() <= 262144)
        .and_then(|s| serde_json::from_str::<Vec<Value>>(s).ok());
    let label_workspace = v["workspace"]
        .as_str()
        .filter(|s| !s.is_empty())
        .or(v["legacy_workspace"].as_str().filter(|s| !s.is_empty()));
    if metadata.is_some() || label_workspace.is_some() {
        c.devcontainer = true;
        c.evidence = "Dev Containers labels".into();
    }
    let mut configs = vec![];
    if let Some(s) = label_workspace {
        let p = PathBuf::from(s);
        if p.is_absolute() {
            if let Some(config) = config_at(&p) {
                configs.push((p, config, None));
            }
        }
    }
    if let Some(s) = v["config"].as_str() {
        let p = PathBuf::from(s);
        if p.is_absolute() && p.is_file() {
            if let Some(w) = label_workspace {
                configs.push((PathBuf::from(w), p, None));
            }
        }
    }
    if let Some(mounts) = v["mounts"].as_array() {
        for mount in mounts.iter().take(128) {
            if mount["Type"] != "bind" {
                continue;
            }
            if let (Some(source), Some(destination)) =
                (mount["Source"].as_str(), mount["Destination"].as_str())
            {
                let w = PathBuf::from(source);
                if let Some(config) = config_at(&w) {
                    configs.push((w, config, Some(destination.to_owned())));
                }
            }
        }
    }
    let mut effective = json!({});
    if let Some(metadata) = metadata {
        for entry in metadata {
            merge(&mut effective, &entry);
        }
    }
    for (workspace, config, destination) in configs {
        if let Ok(configuration) = read_config(&config) {
            c.devcontainer = true;
            if c.evidence.starts_with("Docker") {
                c.evidence = "Mounted devcontainer.json verified".into();
            }
            c.workspace = Some(workspace.to_string_lossy().into_owned());
            c.config = Some(config.to_string_lossy().into_owned());
            if let Some(destination) = destination {
                c.folder = destination;
            }
            merge(&mut effective, &configuration);
            break;
        }
    }
    if let Some(user) = effective["remoteUser"]
        .as_str()
        .or(effective["containerUser"].as_str())
    {
        if !user.contains("${") {
            c.user = user.into();
        }
    }
    if let Some(folder) = effective["workspaceFolder"].as_str() {
        if !folder.contains("${") && folder.starts_with('/') {
            c.folder = folder.into();
        }
    }
    if let Some(env) = effective["remoteEnv"].as_object() {
        for (k, v) in env {
            if valid_env(k) {
                if let Some(value) = v.as_str() {
                    if !value.contains("${") && value.len() < 8192 {
                        c.remote_env.insert(k.clone(), value.into());
                    }
                }
            }
        }
    }
    Ok(c)
}
fn merge(dst: &mut Value, src: &Value) {
    if let (Some(a), Some(b)) = (dst.as_object_mut(), src.as_object()) {
        for (k, v) in b {
            a.insert(k.clone(), v.clone());
        }
    }
}
fn valid_env(name: &str) -> bool {
    !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}
pub fn list() -> Result<Value> {
    let ids = text(bounded(
        docker(&["ps", "--all", "--quiet", "--no-trunc"]),
        None,
        5,
        65536,
    )?)?;
    let engine = engine_identity()?;
    let deadline = Instant::now() + Duration::from_secs(40);
    let mut truncated = ids.lines().count() > 256;
    let mut items = vec![];
    for id in ids.lines().take(256) {
        if Instant::now() >= deadline {
            truncated = true;
            break;
        }
        if let Ok(c) = inspect_on(id, &engine) {
            items.push(c);
        }
    }
    anyhow::ensure!(
        engine_identity()? == engine,
        "Docker engine changed during discovery; refresh again"
    );
    items.sort_by(|a, b| (!a.devcontainer, &a.name).cmp(&(!b.devcontainer, &b.name)));
    Ok(
        json!({"containers":items,"truncated":truncated,"engine":"docker","network_policy":"Existing networks and ports are preserved; no automatic forwarding"}),
    )
}
fn check(scope: &ContainerScope) -> Result<Container> {
    let c = inspect(&scope.id)?;
    anyhow::ensure!(
        !scope.engine.is_empty() && c.engine == scope.engine,
        "Docker engine changed; refresh this container before accessing it"
    );
    anyhow::ensure!(
        c.devcontainer || c.allowed,
        "Ordinary container access is disabled; enable access explicitly for this container first"
    );
    anyhow::ensure!(
        c.state == "running",
        "Container is {}. Start the selected container before attaching",
        c.state
    );
    anyhow::ensure!(
        c.started_at == scope.started_at,
        "Container restarted; refresh its entry before opening files or starting a session"
    );
    anyhow::ensure!(
        c.user == scope.user,
        "Container execution user changed; refresh its entry"
    );
    Ok(c)
}
pub fn lifecycle(id: &str, engine: &str, started_at: &str, action: &str) -> Result<Value> {
    let c = inspect(id)?;
    anyhow::ensure!(
        c.engine == engine,
        "Docker engine changed; refresh before managing this container"
    );
    anyhow::ensure!(
        c.devcontainer || c.allowed,
        "Enable access before managing an ordinary Docker container"
    );
    anyhow::ensure!(
        c.started_at == started_at,
        "Container lifecycle changed; refresh before retrying"
    );
    match action {
        "start" if c.state == "running" => {}
        "start" => {
            bounded(docker(&["start", id]), None, 60, 65536)?;
        }
        "stop" if c.state != "running" => {}
        "stop" => {
            bounded(docker(&["stop", "--time", "10", id]), None, 25, 65536)?;
        }
        _ => bail!("unsupported container action"),
    }
    Ok(serde_json::to_value(inspect(id)?)?)
}
fn exec(scope: &ContainerScope, tty: bool) -> Result<Command> {
    let mut c = docker(&["exec", "-i"]);
    if tty {
        c.arg("-t");
    }
    c.args(["--user", &scope.user, "--workdir", &scope.folder, &scope.id]);
    Ok(c)
}
fn helper_location(scope: &ContainerScope) -> Result<String> {
    // Use the selected container user's uid, not the host account uid.
    let mut c = exec(scope, false)?;
    c.args(["/bin/sh", "-c", "id -u"]);
    let uid = text(bounded(c, None, 5, 65536)?)?;
    anyhow::ensure!(
        !uid.is_empty() && uid.bytes().all(|b| b.is_ascii_digit()),
        "container user identity unavailable"
    );
    Ok(format!(
        "/tmp/cx-tools-{uid}/cx-{}",
        env!("CARGO_PKG_VERSION")
    ))
}
fn ensure_helper(scope: &ContainerScope) -> Result<String> {
    check(scope)?;
    let mut c = exec(scope, false)?;
    c.args(["/bin/sh", "-c", "uname -m"]);
    let arch = text(bounded(c, None, 5, 65536)?)?;
    anyhow::ensure!(matches!((std::env::consts::ARCH,arch.as_str()),("x86_64","x86_64")|("aarch64","aarch64")),"Container architecture differs from this helper; use a supported native Linux x86_64/ARM64 devcontainer");
    let path = helper_location(scope)?;
    let mut c = exec(scope, false)?;
    c.args([&path, "--version"]);
    if bounded(c, None, 5, 65536).ok().is_some_and(|b| {
        String::from_utf8_lossy(&b).trim() == format!("cx {}", env!("CARGO_PKG_VERSION"))
    }) {
        return Ok(path);
    }
    let parent = Path::new(&path)
        .parent()
        .context("tool parent")?
        .to_str()
        .context("tool path")?;
    let mut c = exec(scope, false)?;
    // Positional args only. Refuse symlinks/foreign directories and stage with noclobber.
    c.args(["/bin/sh","-c",r#"set -eu; umask 077; dir=$1; dest=$2; test ! -L "$dir"; if test ! -d "$dir"; then mkdir -m 700 "$dir"; fi; test "$(stat -c %u "$dir")" = "$(id -u)"; stage="$dest.new-$$"; trap 'rm -f "$stage"' EXIT HUP INT TERM; set -C; cat > "$stage"; chmod 700 "$stage"; "$stage" --version >/dev/null; mv -f "$stage" "$dest""#,"cx-tool",parent,&path]);
    bounded(c,Some(fs::File::open(std::env::current_exe()?)?),45,65536).context("Container tooling could not be installed. An executable, writable /tmp and /bin/sh are required")?;
    check(scope)?;
    let mut c = exec(scope, false)?;
    c.args([&path, "--version"]);
    anyhow::ensure!(
        text(bounded(c, None, 5, 65536)?)? == format!("cx {}", env!("CARGO_PKG_VERSION")),
        "container helper version mismatch"
    );
    Ok(path)
}
pub fn files(scope: &ContainerScope, operation: &Operation) -> Result<Value> {
    anyhow::ensure!(matches!(operation,Operation::List{..}|Operation::ListPage{..}|Operation::Preview{..}|Operation::PreviewPage{..}|Operation::FileInfo{..}),"Container file browser is read-only; host mutations and transfers are never applied to container paths");
    let helper = ensure_helper(scope)?;
    check(scope)?;
    let mut bytes = vec![];
    let request = Request {
        version: 1,
        id: "container-file".into(),
        op: operation.clone(),
    };
    let json = serde_json::to_vec(&request)?;
    anyhow::ensure!(json.len() <= 1024 * 1024, "request exceeds limit");
    write!(&mut bytes, "CX1 {}\n", json.len())?;
    bytes.extend_from_slice(&json);
    let mut c = exec(scope, false)?;
    c.args([&helper, "container-helper"]);
    let output = bounded(c, Some(input(&bytes)?), 20, 1024 * 1024 + 16384)?;
    let response: Response =
        crate::transport::read_frame(&mut std::io::BufReader::new(output.as_slice()))?;
    anyhow::ensure!(
        response.version == 1 && response.id == request.id,
        "container helper response identity mismatch"
    );
    check(scope)?;
    if let Some(e) = response.error {
        bail!("{e}");
    }
    response.result.context("empty container helper response")
}
fn cli() -> Option<PathBuf> {
    for p in std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()) {
        let p = p.join("devcontainer");
        if p.is_file() {
            return Some(p);
        }
    }
    let root = PathBuf::from(std::env::var_os("HOME")?).join(".nvm/versions/node");
    let mut versions: Vec<_> = fs::read_dir(root)
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .collect();
    versions.sort();
    versions.reverse();
    versions
        .into_iter()
        .map(|p| p.join("bin/devcontainer"))
        .find(|p| p.is_file())
}
fn cli_command(path: &Path) -> Command {
    let mut c = Command::new(path);
    if let Some(parent) = path.parent() {
        let mut paths = vec![parent.to_path_buf()];
        paths.extend(std::env::split_paths(
            &std::env::var_os("PATH").unwrap_or_default(),
        ));
        if let Ok(path) = std::env::join_paths(paths) {
            c.env("PATH", path);
        }
    }
    c
}
const SHELL_SCRIPT: &str = r#"set -e
uid=$(id -u)
while IFS=: read -r name password entry_uid gid gecos home shell; do
 if test "$entry_uid" = "$uid"; then export HOME="$home"; break; fi
done < /etc/passwd
cd -- "$1"
shift
if command -v infocmp >/dev/null 2>&1 && ! infocmp "$TERM" >/dev/null 2>&1; then export TERM=xterm-256color; fi
# docker exec does not execute the ROS image's entrypoint. Load only its selected distro.
if ! command -v ros2 >/dev/null 2>&1; then
 case "${ROS_DISTRO:-}" in ''|*[!a-zA-Z0-9_-]*) ;; *) if test -f "/opt/ros/$ROS_DISTRO/setup.sh"; then . "/opt/ros/$ROS_DISTRO/setup.sh"; fi ;; esac
fi
if test "$1" != shell; then
 if command -v bash >/dev/null 2>&1; then exec bash -ilc 'exec "$@"' cx-agent "$@"; fi
 if command -v zsh >/dev/null 2>&1; then exec zsh -ilc 'exec "$@"' cx-agent "$@"; fi
 exec "$@"
fi
if command -v bash >/dev/null 2>&1; then exec bash -il; fi
if command -v zsh >/dev/null 2>&1; then exec zsh -il; fi
exec /bin/sh -i
"#;
pub fn prepare(scope: &ContainerScope, provider: &str) -> Result<()> {
    let container = check(scope)?;
    ensure_helper(scope)?;
    anyhow::ensure!(
        ["shell", "codex", "claude"].contains(&provider),
        "unsupported container session profile"
    );
    let mut c = terminal_command(&container, scope, false);
    c.args([
        "/bin/sh",
        "-c",
        "if command -v bash >/dev/null 2>&1; then exec bash -ilc \"$1\"; elif command -v zsh >/dev/null 2>&1; then exec zsh -ilc \"$1\"; else exec /bin/sh -lc \"$1\"; fi",
        "cx-provider-check",
        if provider == "shell" {
            "command -v bash || command -v zsh || command -v sh"
        } else if provider == "codex" {
            "command -v codex"
        } else {
            "command -v claude"
        },
    ]);
    bounded(c, None, 30, 262144).with_context(|| {
        format!("{provider} is unavailable in this container; no packages were installed")
    })?;
    Ok(())
}
pub fn run(scope: &ContainerScope, provider: &str, yolo: bool) -> Result<()> {
    use base64::Engine;
    let container = check(scope)?;
    prepare(scope, provider)?;
    let helper = ensure_helper(scope)?;
    let id = std::env::var("CX_MANAGED_SESSION")
        .context("managed container session identity unavailable")?;
    anyhow::ensure!(valid_session(&id), "invalid container session identity");
    let payload = base64::engine::general_purpose::STANDARD.encode(serde_json::to_vec(&(
        provider,
        yolo,
        &scope.folder,
        &id,
    ))?);
    let mut command = terminal_command(&container, scope, true);
    command.args([&helper, "container-shell", &payload]);
    Err(command.exec()).context("Container terminal could not start")
}
fn terminal_command(container: &Container, scope: &ContainerScope, tty: bool) -> Command {
    if let Some(cli) = cli().filter(|_| container.devcontainer && container.config.is_some()) {
        let mut command = cli_command(&cli);
        command.args([
            "exec",
            "--container-id",
            &scope.id,
            "--config",
            container.config.as_deref().unwrap(),
        ]);
        if let Some(workspace) = &container.workspace {
            command.args(["--workspace-folder", workspace]);
        }
        for name in ["TERM", "COLORTERM", "CX_VIEWER_THEME"] {
            if let Ok(value) = std::env::var(name) {
                command.args(["--remote-env", &format!("{name}={value}")]);
            }
        }
        command
    } else {
        let mut command = docker(&[
            "exec",
            "-i",
            "--user",
            &scope.user,
            "--workdir",
            &scope.folder,
        ]);
        if tty {
            command.arg("-t");
        }
        for (name, value) in &container.remote_env {
            command.args(["--env", &format!("{name}={value}")]);
        }
        for name in ["TERM", "COLORTERM", "CX_VIEWER_THEME"] {
            if let Ok(value) = std::env::var(name) {
                command.args(["--env", &format!("{name}={value}")]);
            }
        }
        command.arg(&scope.id);
        command
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn jsonc_preserves_strings_and_accepts_comments_and_trailing_commas() {
        assert_eq!(
            jsonc(
                r#"{"name":"https://host/*literal*/", // hello
    "remoteUser":"robot",}"#
            )
            .unwrap()["remoteUser"],
            "robot"
        );
        assert!(jsonc("{/*broken").is_err());
    }
    #[test]
    fn only_full_ids_are_targets() {
        assert!(valid_id(&"a".repeat(64)));
        for s in ["roboboat_dev", "-x", "abc", "$(touch PWN)"] {
            assert!(!valid_id(s));
        }
    }
    #[test]
    fn container_files_reject_mutations_before_any_engine_call() {
        let s = ContainerScope {
            engine: "fixture".into(),
            id: "a".repeat(64),
            name: "service".into(),
            user: "root".into(),
            folder: "/".into(),
            started_at: "now".into(),
        };
        assert!(files(
            &s,
            &Operation::Remove {
                path: "/etc/config".into(),
                expected_identity: None
            }
        )
        .unwrap_err()
        .to_string()
        .contains("read-only"));
    }
    #[test]
    fn bounded_commands_timeout_and_limit_output() {
        let mut c = Command::new("sh");
        c.args(["-c", "sleep 3"]);
        assert!(bounded(c, None, 1, 128).is_err());
        let mut c = Command::new("sh");
        c.args(["-c", "yes"]);
        assert!(bounded(c, None, 2, 128).is_err());
    }
}

fn valid_session(id: &str) -> bool {
    id.len() == 67 && id.starts_with("cx-") && valid_id(&id[3..])
}
pub fn container_home() -> Result<()> {
    unsafe {
        let entry = libc::getpwuid(libc::geteuid());
        anyhow::ensure!(
            !entry.is_null() && !(*entry).pw_dir.is_null(),
            "container account home unavailable"
        );
        use std::os::unix::ffi::OsStrExt;
        let home = std::ffi::CStr::from_ptr((*entry).pw_dir).to_bytes();
        std::env::set_var("HOME", std::ffi::OsStr::from_bytes(home));
    }
    let root = tool_root()?;
    std::env::set_var("XDG_CACHE_HOME", root.join("cache"));
    std::env::set_var("XDG_STATE_HOME", root.join("state"));
    Ok(())
}
fn tool_root() -> Result<PathBuf> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let root = PathBuf::from(format!("/tmp/cx-tools-{}", unsafe { libc::geteuid() }));
    match fs::create_dir(&root) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e.into()),
    }
    let meta = fs::symlink_metadata(&root)?;
    anyhow::ensure!(
        meta.is_dir() && meta.uid() == unsafe { libc::geteuid() },
        "unsafe container tool directory"
    );
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700))?;
    Ok(root)
}
fn process_record(id: &str) -> Result<PathBuf> {
    anyhow::ensure!(valid_session(id), "invalid container session identity");
    Ok(tool_root()?.join(format!("{id}.process")))
}
fn start_ticks(pid: u32) -> Option<String> {
    fs::read_to_string(format!("/proc/{pid}/stat"))
        .ok()?
        .rsplit_once(')')?
        .1
        .split_whitespace()
        .nth(19)
        .map(str::to_owned)
}
fn local_identity(id: &str) -> Result<crate::model::ProcessIdentity> {
    let path = process_record(id)?;
    let meta = fs::symlink_metadata(&path)?;
    use std::os::unix::fs::MetadataExt;
    anyhow::ensure!(
        meta.is_file() && meta.uid() == unsafe { libc::geteuid() } && meta.len() < 4096,
        "unsafe container process record"
    );
    let identity: crate::model::ProcessIdentity = serde_json::from_slice(&fs::read(path)?)?;
    anyhow::ensure!(identity.pid > 1, "invalid container process identity");
    anyhow::ensure!(
        start_ticks(identity.pid).as_deref() == Some(&identity.start_ticks),
        "container terminal ended or process identity changed"
    );
    Ok(identity)
}
pub fn local_shell(provider: &str, yolo: bool, folder: &str, id: &str) -> Result<()> {
    let original_xdg =
        ["XDG_CACHE_HOME", "XDG_STATE_HOME"].map(|name| (name, std::env::var_os(name)));
    container_home()?;
    anyhow::ensure!(
        ["shell", "codex", "claude"].contains(&provider),
        "unsupported container profile"
    );
    anyhow::ensure!(!yolo || provider != "shell", "YOLO applies only to agents");
    let identity = crate::model::ProcessIdentity {
        pid: std::process::id(),
        start_ticks: start_ticks(std::process::id())
            .context("container process identity unavailable")?,
        native_id: None,
    };
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(process_record(id)?)?;
    file.write_all(&serde_json::to_vec(&identity)?)?;
    file.sync_all()?;
    let folder = crate::files::decode_path(folder)?;
    let mut c = Command::new("/bin/sh");
    c.args(["-c", SHELL_SCRIPT, "cx-container"])
        .arg(folder)
        .arg(provider);
    if yolo {
        c.arg(if provider == "codex" {
            "--dangerously-bypass-approvals-and-sandbox"
        } else {
            "--dangerously-skip-permissions"
        });
    }
    for (name, value) in original_xdg {
        match value {
            Some(v) => {
                c.env(name, v);
            }
            None => {
                c.env_remove(name);
            }
        }
    }
    Err(c.exec()).context("container shell could not execute")
}
pub fn local_wheel_owner(id: &str) -> Result<Value> {
    container_home()?;
    let identity = local_identity(id)?;
    Ok(serde_json::to_value(crate::sessions::wheel_codex_owner(
        identity.pid,
    ))?)
}
pub fn local_stop(id: &str) -> Result<Value> {
    container_home()?;
    let path = process_record(id)?;
    if !path.exists() {
        return Ok(json!({"status":"already_stopped"}));
    }
    let identity = local_identity(id)?;
    let mut queue = std::collections::VecDeque::from([identity.pid]);
    let mut processes = vec![];
    let mut seen = std::collections::BTreeSet::new();
    let mut tasks = 0;
    while let Some(pid) = queue.pop_front() {
        if !seen.insert(pid) {
            continue;
        }
        anyhow::ensure!(
            seen.len() <= 256,
            "container terminal process tree exceeds stop limits; stop work from the terminal"
        );
        let Some(start) = start_ticks(pid) else {
            continue;
        };
        processes.push((pid, start));
        if let Ok(entries) = fs::read_dir(format!("/proc/{pid}/task")) {
            for task in entries {
                tasks += 1;
                anyhow::ensure!(
                    tasks <= 1024,
                    "container terminal thread tree exceeds stop limits"
                );
                let task = task?;
                let mut bytes = String::new();
                match fs::File::open(task.path().join("children")) {
                    Ok(f) => {
                        f.take(16385).read_to_string(&mut bytes)?;
                        anyhow::ensure!(
                            bytes.len() <= 16384,
                            "container process descendants exceed limit"
                        );
                        for child in bytes.split_whitespace() {
                            queue.push_back(child.parse()?);
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(e.into()),
                }
            }
        }
    }
    for &(pid, ref start) in processes.iter().rev() {
        if pid > 1 && start_ticks(pid).as_deref() == Some(start) {
            unsafe {
                libc::kill(pid as i32, libc::SIGTERM);
            }
        }
    }
    std::thread::sleep(Duration::from_millis(250));
    for &(pid, ref start) in processes.iter().rev() {
        if pid > 1 && start_ticks(pid).as_deref() == Some(start) {
            unsafe {
                libc::kill(pid as i32, libc::SIGKILL);
            }
        }
    }
    fs::remove_file(path)?;
    Ok(json!({"status":"stopped"}))
}
fn private_helper(scope: &ContainerScope, operation: &str, id: &str) -> Result<Vec<u8>> {
    anyhow::ensure!(valid_session(id), "invalid container session identity");
    check(scope)?;
    let helper = ensure_helper(scope)?;
    let mut command = exec(scope, false)?;
    command.args([&helper, operation, id]);
    bounded(command, None, 5, 65536)
}
pub fn stop_terminal(scope: &ContainerScope, id: &str) -> Result<()> {
    private_helper(scope, "container-process-stop", id)?;
    Ok(())
}
pub fn wheel_owner(scope: &ContainerScope, id: &str) -> Option<crate::model::ProcessIdentity> {
    serde_json::from_slice(&private_helper(scope, "container-wheel-owner", id).ok()?).ok()?
}
