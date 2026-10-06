use crate::model::*;
use anyhow::{anyhow, bail, Context, Result};
use std::{
    io::{BufRead, BufReader, Read, Write},
    process::{Command, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};
pub const MAX_MESSAGE: usize = 1024 * 1024;
pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
pub fn valid_target(s: &str) -> bool {
    !s.is_empty()
        && s.len() < 256
        && !s.starts_with('-')
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"@._-:[]".contains(&b))
}
pub fn frame<T: serde::Serialize>(w: &mut impl Write, v: &T) -> Result<()> {
    let b = serde_json::to_vec(v)?;
    if b.len() > MAX_MESSAGE {
        bail!("message exceeds limit")
    }
    write!(w, "CX1 {}\n", b.len())?;
    w.write_all(&b)?;
    w.flush()?;
    Ok(())
}
pub fn read_frame<T: serde::de::DeserializeOwned>(r: &mut impl BufRead) -> Result<T> {
    let mut scanned = 0;
    loop {
        let mut line = Vec::new();
        let n = r.take(4097).read_until(b'\n', &mut line)?;
        scanned += n;
        if n == 0 {
            bail!("helper closed before response")
        }
        if scanned > 16384 || line.len() > 4096 {
            bail!("excessive shell startup noise")
        }
        if let Some(raw) = line.strip_prefix(b"CX1 ") {
            let raw = std::str::from_utf8(raw)?.trim();
            let size = raw.parse::<usize>().context("invalid frame length")?;
            if size > MAX_MESSAGE {
                bail!("message exceeds limit")
            }
            let mut b = vec![0; size];
            r.read_exact(&mut b)?;
            return Ok(serde_json::from_slice(&b)?);
        }
    }
}
pub fn ssh(target: &str, interactive: bool) -> Result<Command> {
    if !valid_target(target) {
        bail!("invalid SSH target")
    };
    let mut c = Command::new("ssh");
    c.args([
        "-o",
        "ConnectTimeout=6",
        "-o",
        "ServerAliveInterval=5",
        "-o",
        "ServerAliveCountMax=2",
        "-o",
        "ForwardAgent=no",
    ]);
    if !interactive {
        c.args(["-o", "BatchMode=yes"]);
    }
    c.arg(target);
    Ok(c)
}
struct Connection {
    child: std::process::Child,
    input: std::process::ChildStdin,
    replies: std::sync::mpsc::Receiver<Result<Response>>,
}
impl Drop for Connection {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
impl Connection {
    fn open(target: &str) -> Result<Self> {
        let mut c = ssh(target, false)?;
        c.arg("exec ~/.local/bin/cx helper")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut child = c.spawn()?;
        let input = child.stdin.take().context("helper input")?;
        let out = child.stdout.take().context("helper output")?;
        let (tx, replies) = std::sync::mpsc::sync_channel(4);
        std::thread::spawn(move || {
            let mut reader = BufReader::new(out);
            loop {
                let result = read_frame::<Response>(&mut reader);
                let failed = result.is_err();
                if tx.send(result).is_err() || failed {
                    break;
                }
            }
        });
        Ok(Self {
            child,
            input,
            replies,
        })
    }
}
static CONNECTIONS: std::sync::LazyLock<
    std::sync::Mutex<
        std::collections::HashMap<String, std::sync::Arc<std::sync::Mutex<Option<Connection>>>>,
    >,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));
static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
/// One framed metadata channel per endpoint, separate from native PTY traffic.
pub fn request(d: &Device, op: Operation) -> Result<serde_json::Value> {
    let Some(target) = &d.target else {
        return crate::dispatch(op);
    };
    if !valid_target(target) {
        bail!("invalid SSH target")
    }
    let entry = {
        let mut all = CONNECTIONS
            .lock()
            .map_err(|_| anyhow!("transport lock unavailable"))?;
        all.entry(target.clone())
            .or_insert_with(|| std::sync::Arc::new(std::sync::Mutex::new(None)))
            .clone()
    };
    let mut slot = entry
        .lock()
        .map_err(|_| anyhow!("connection lock unavailable"))?;
    if slot
        .as_mut()
        .is_some_and(|c| !matches!(c.child.try_wait(), Ok(None)))
    {
        *slot = None;
    }
    if slot.is_none() {
        *slot = Some(Connection::open(target)?);
    }
    let id = format!(
        "{}-{}",
        std::process::id(),
        NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    );
    let req = Request {
        version: 1,
        id: id.clone(),
        op,
    };
    let conn = slot.as_mut().unwrap();
    if let Err(error) = frame(&mut conn.input, &req) {
        *slot = None;
        return Err(error).context("helper disconnected; reconcile mutations before retry");
    }
    let response = match conn
        .replies
        .recv_timeout(std::time::Duration::from_secs(15))
    {
        Ok(Ok(r)) => r,
        Ok(Err(e)) => {
            *slot = None;
            return Err(e).context("helper disconnected; work may still be running");
        }
        Err(_) => {
            *slot = None;
            bail!("host did not respond within 15 seconds; work may still be running")
        }
    };
    if response.version != 1 || response.id != id {
        *slot = None;
        bail!("incompatible or mismatched helper response")
    }
    if let Some(e) = response.error {
        bail!("{e}")
    }
    response
        .result
        .ok_or_else(|| anyhow!("empty helper response"))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_frames() {
        let v = serde_json::json!({"x":"quoted\n☃"});
        let mut b = Vec::new();
        frame(&mut b, &v).unwrap();
        let got: serde_json::Value = read_frame(&mut BufReader::new(&b[..])).unwrap();
        assert_eq!(got, v);
        assert!(
            read_frame::<serde_json::Value>(&mut BufReader::new(&b"CX1 1048577\n"[..])).is_err()
        );
        assert!(read_frame::<serde_json::Value>(&mut BufReader::new(&b"CX1 -1\n"[..])).is_err());
    }
    #[test]
    fn startup_noise() {
        let b = b"welcome\nCX1 2\n{}";
        let v: serde_json::Value = read_frame(&mut BufReader::new(&b[..])).unwrap();
        assert!(v.is_object());
    }
    #[test]
    fn targets() {
        for s in ["-oProxyCommand=evil", "x;id", "x\ny", "$(id)", "a b"] {
            assert!(!valid_target(s))
        }
        assert!(valid_target("peace@host.example"));
    }
}
