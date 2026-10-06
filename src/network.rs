//! Read-only network observation. No discovery scan or interface mutation is implicit.
use anyhow::{bail, Result};
use serde_json::{json, Value};
use std::{
    io::Read,
    process::{Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
const OUTPUT_LIMIT: usize = 256 * 1024;
/// Execute an allowlisted observation with a strict wall clock and output bound.
fn bounded_command(program: &str, args: &[&str]) -> Result<String> {
    let mut child = Command::new(program)
        .args(args)
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let stdout = child.stdout.take().unwrap();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = stdout
            .take((OUTPUT_LIMIT + 1) as u64)
            .read_to_end(&mut bytes);
        let _ = tx.send((result, bytes));
    });
    let deadline = Instant::now() + Duration::from_secs(2);
    let outcome = loop {
        if let Some(status) = child.try_wait()? {
            break Ok(status);
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            break Err(anyhow::anyhow!("observation timed out"));
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let status = outcome?;
    let (read, bytes) = rx.recv_timeout(Duration::from_millis(200))?;
    read?;
    if bytes.len() > OUTPUT_LIMIT {
        bail!("observation exceeded output limit");
    }
    if !status.success() {
        bail!("observation unavailable");
    }
    Ok(String::from_utf8(bytes)?)
}
fn observation(program: &str, args: &[&str]) -> Value {
    match bounded_command(program, args) {
        Ok(value) => match serde_json::from_str::<Value>(&value) {
            Ok(v) => json!({"state":"observed","data":v}),
            Err(_) => json!({"state":"unknown","reason":"invalid structured output"}),
        },
        Err(e) => json!({"state":"unknown","reason":crate::files::display(&e.to_string())}),
    }
}
pub fn observe() -> Result<Value> {
    #[cfg(not(target_os = "linux"))]
    return Ok(
        json!({"backend":"unsupported","sharing":"unsupported","reason":"network observation currently supports Linux","observed_at":timestamp()}),
    );
    #[cfg(target_os = "linux")]
    {
        // Each command observes one bounded snapshot; no internet probes or SSH calls.
        let mut interfaces = observation("ip", &["-j", "address", "show"]);
        if let Some(items) = interfaces.get_mut("data").and_then(Value::as_array_mut) {
            items.retain(|v| {
                v["ifname"] != "lo"
                    && !v["ifname"].as_str().is_some_and(|n| {
                        n.starts_with("veth") || n.starts_with("docker") || n.starts_with("br-")
                    })
            });
            for item in items.iter_mut() {
                if let Some(name) = item.get("ifname").and_then(Value::as_str) {
                    let display = crate::files::display(name);
                    item["display_name"] = json!(display);
                }
            }
        }
        let routes = observation("ip", &["-j", "route", "show"]);
        let neighbors = observation("ip", &["-j", "neigh", "show"]);
        let network_manager = match bounded_command("nmcli", &["--version"]) {
            Ok(version) => {
                json!({"state":"installed","version":crate::files::display(version.trim()),"sharing":"not implemented","reason":"No mutation occurs through observation"})
            }
            Err(_) => {
                json!({"state":"unavailable","sharing":"unsupported","reason":"NetworkManager capability unverified"})
            }
        };
        Ok(
            json!({"backend":"linux-iproute2","observed_at":timestamp(),"interfaces":interfaces,"routes":routes,"neighbors":neighbors,"network_manager":network_manager,"internet":"unknown","overlay":"unknown","sharing":{"state":"unsupported","reason":"Transactional sharing backend not yet implemented"}}),
        )
    }
}
fn timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn command_failure_is_unknown() {
        assert_eq!(
            observation("cx-nonexistent-observer", &[])["state"],
            "unknown"
        );
    }
    #[test]
    fn malformed_output_is_unknown() {
        assert_eq!(observation("printf", &["not-json"])["state"], "unknown");
    }
    #[test]
    fn observation_timeout_is_bounded() {
        let started = Instant::now();
        assert!(bounded_command("sleep", &["5"]).is_err());
        assert!(started.elapsed() < Duration::from_secs(3));
    }
    #[test]
    fn observation_output_is_bounded() {
        assert!(bounded_command("head", &["-c", "1048576", "/dev/zero"]).is_err());
    }
    #[test]
    fn snapshot_never_claims_internet_or_sharing() {
        let v = observe().unwrap();
        assert_eq!(v["internet"], "unknown");
        assert_eq!(v["sharing"]["state"], "unsupported");
    }
}
