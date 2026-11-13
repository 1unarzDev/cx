//! `Operation::Metrics` (capability `metrics-v1`): this host's vitals from a handful of
//! /proc and /sys reads, no process spawned. CPU is raw jiffies; the viewer turns two
//! samples into a percentage, so the helper keeps no state between requests.
//! `CX_METRICS_ROOT` (tests only) points every read at a fixture tree instead of `/`.
use serde_json::{json, Value};
use std::path::PathBuf;

fn root() -> PathBuf {
    std::env::var_os("CX_METRICS_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

fn read(path: &str) -> String {
    std::fs::read_to_string(root().join(path)).unwrap_or_default()
}

/// (busy, total) jiffies from the aggregate `cpu` line of /proc/stat.
fn cpu(stat: &str) -> Option<(u64, u64)> {
    let line = stat.lines().find(|l| l.starts_with("cpu "))?;
    let v: Vec<u64> = line
        .split_whitespace()
        .skip(1)
        .take(8)
        .filter_map(|x| x.parse().ok())
        .collect();
    (v.len() >= 4).then(|| {
        let total: u64 = v.iter().sum();
        // idle + iowait are not busy.
        (total - v[3] - v.get(4).copied().unwrap_or(0), total)
    })
}

/// (total, available) kB from /proc/meminfo.
fn memory(info: &str) -> Option<(u64, u64)> {
    let field = |name: &str| {
        info.lines()
            .find(|l| l.starts_with(name))?
            .split_whitespace()
            .nth(1)?
            .parse::<u64>()
            .ok()
    };
    Some((field("MemTotal:")?, field("MemAvailable:")?))
}

/// A CPU temperature in °C: a CPU/SoC thermal zone first, then any zone, then a CPU hwmon
/// sensor. None when the host has no sensor (containers, VMs).
fn temperature() -> Option<f32> {
    let celsius = |text: String| {
        let t = text.trim().parse::<f32>().ok()? / 1000.0;
        (t > 0.0 && t < 150.0).then_some(t)
    };
    let mut any = None;
    for i in 0..16 {
        let kind = read(&format!("sys/class/thermal/thermal_zone{i}/type"));
        if kind.is_empty() {
            break;
        }
        let Some(t) = celsius(read(&format!("sys/class/thermal/thermal_zone{i}/temp"))) else {
            continue;
        };
        let kind = kind.trim();
        if ["pkg", "cpu", "soc", "k10"]
            .iter()
            .any(|k| kind.contains(k))
        {
            return Some(t);
        }
        any = any.or(Some(t));
    }
    if any.is_some() {
        return any;
    }
    (0..16).find_map(|i| {
        let name = read(&format!("sys/class/hwmon/hwmon{i}/name"));
        ["coretemp", "k10temp", "zenpower", "cpu_thermal"]
            .contains(&name.trim())
            .then(|| celsius(read(&format!("sys/class/hwmon/hwmon{i}/temp1_input"))))
            .flatten()
    })
}

/// `ubuntu 22.04` from os-release (its ID and VERSION_ID), else the kernel's name.
fn os(release: &str) -> String {
    let field = |name: &str| {
        release
            .lines()
            .find_map(|l| l.strip_prefix(name)?.strip_prefix('='))
            .map(|v| v.trim().trim_matches('"').to_string())
            .filter(|v| !v.is_empty())
    };
    match (field("ID"), field("VERSION_ID")) {
        (Some(id), Some(v)) => format!("{id} {v}"),
        (Some(id), None) => id,
        _ => std::env::consts::OS.into(),
    }
}

fn disk() -> Option<f32> {
    let path = std::ffi::CString::new(root().as_os_str().as_encoded_bytes()).ok()?;
    let mut s: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(path.as_ptr(), &mut s) } != 0 || s.f_blocks == 0 {
        return None;
    }
    Some(1.0 - s.f_bavail as f32 / s.f_blocks as f32)
}

pub fn read_all() -> Value {
    let stat = read("proc/stat");
    let cores = stat
        .lines()
        .filter(|l| l.starts_with("cpu") && !l.starts_with("cpu "))
        .count();
    let load: Vec<f32> = read("proc/loadavg")
        .split_whitespace()
        .take(3)
        .filter_map(|v| v.parse().ok())
        .collect();
    let uptime = read("proc/uptime")
        .split_whitespace()
        .next()
        .and_then(|v| v.parse::<f64>().ok())
        .map(|v| v as u64);
    let release = match read("etc/os-release") {
        r if r.is_empty() => read("usr/lib/os-release"),
        r => r,
    };
    let mem = memory(&read("proc/meminfo"));
    json!({
        "cpu": cpu(&stat).map(|(b, t)| [b, t]),
        "cores": cores,
        "load": (load.len() == 3).then_some(load),
        "mem_total_kb": mem.map(|m| m.0),
        "mem_available_kb": mem.map(|m| m.1),
        "temp_c": temperature(),
        "uptime_secs": uptime,
        "disk": disk(),
        "os": crate::files::display(&os(&release)),
        "arch": std::env::consts::ARCH,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_proc_text() {
        assert_eq!(
            cpu("cpu  100 0 50 800 50 0 0 0 0 0\ncpu0 1 2 3 4\n"),
            Some((150, 1000))
        );
        assert_eq!(cpu("intr 1 2"), None);
        assert_eq!(
            memory("MemTotal:  8000 kB\nMemFree: 1 kB\nMemAvailable:  2000 kB\n"),
            Some((8000, 2000))
        );
        assert_eq!(memory("MemTotal: 1 kB\n"), None);
        assert_eq!(
            os("NAME=\"Ubuntu\"\nID=ubuntu\nVERSION_ID=\"22.04\"\n"),
            "ubuntu 22.04"
        );
        assert_eq!(os("ID=arch\n"), "arch");
    }

    /// Every read is bounded and cheap: the whole sample is far under a millisecond's
    /// worth of work on any host (measured loosely here; the budget is ≤1 ms).
    #[test]
    fn sampling_is_cheap() {
        let start = std::time::Instant::now();
        let v = read_all();
        assert!(v["cores"].as_u64().is_some_and(|c| c > 0), "{v}");
        assert!(start.elapsed() < std::time::Duration::from_millis(50));
    }
}
