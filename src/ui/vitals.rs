//! What the hub shows live about each device beyond its sessions, all in memory:
//! - vitals: `Operation::Metrics` every 2 s for the devices on screen (helpers with
//!   `metrics-v1`; older ones show none), cpu from successive jiffies samples;
//! - link: the round trip of each metrics request on the warm channel (transport adds
//!   `rtt_ms`), a failed one counting as lost; no traffic of its own;
//! - topology: mesh membership from the mesh backends and the lan neighbour table,
//!   observed off the UI thread every few minutes (`observe_topology`), never blocking.
use super::views::{self, MeshBackend};
use super::*;
use crate::mesh::Member;

/// Samples kept per ring (the view draws the last 32).
const HIST: usize = 32;
/// How often the devices on screen are sampled.
pub(super) const EVERY: Duration = Duration::from_secs(2);
/// The next ask for a device still without a cpu reading (cpu needs two samples).
pub(super) const SOON: Duration = Duration::from_millis(250);
/// A sample older than this is no longer shown as live.
const FRESH: u64 = 30;

#[derive(Default)]
pub(super) struct Vitals {
    sample: Value,
    /// When `sample` arrived (unix seconds; 0: never).
    pub at: u64,
    prev: Option<(u64, u64)>,
    cpu: Option<f32>,
    cpu_hist: VecDeque<f32>,
    /// Round trips in ms, oldest first; None is a lost one.
    rtt: VecDeque<Option<f32>>,
    inflight: bool,
}

fn push<T>(q: &mut VecDeque<T>, v: T) {
    if q.len() == HIST {
        q.pop_front();
    }
    q.push_back(v);
}

/// One mesh and its members as its backend reports them.
#[derive(Clone)]
pub(super) struct Mesh {
    pub id: String,
    pub name: String,
    pub backend: MeshBackend,
    pub members: Vec<Member>,
}

impl App {
    /// The devices to sample: every device on the hub, the open one on its page; never
    /// one whose listing failed or whose helper lacks `metrics-v1`.
    fn sampled(&self) -> Vec<usize> {
        if !matches!(self.view, View::Work | View::Network) {
            return Vec::new();
        }
        (0..self.devices.len().min(self.work.len()))
            .filter(|&d| {
                (self.device == 0 || self.device == d + 1)
                    && self.work[d].error.is_none()
                    && (self.devices[d].target.is_none()
                        || self
                            .providers
                            .get(&d)
                            .is_some_and(|(caps, _)| caps.iter().any(|c| c == "metrics-v1")))
            })
            .collect()
    }

    /// Ask the sampled devices for a sample: one request each at a time, never into a
    /// crowded queue.
    pub(super) fn poll_vitals(&mut self) {
        for d in self.sampled() {
            let id = self.devices[d].id.clone();
            if self.pending_requests.get() > 16 || self.vitals.get(&id).is_some_and(|v| v.inflight)
            {
                continue;
            }
            if self.send(d, Operation::Metrics) {
                self.vitals.entry(id).or_default().inflight = true;
            }
        }
    }

    /// A sampled device was never asked (the queue was crowded) or has one sample and no
    /// cpu reading yet: the next ask is due soon, not a full period later, so the figures
    /// land within a moment of the hub opening. (A failed ask waits the full period.)
    pub(super) fn vitals_due(&self) -> bool {
        self.sampled().into_iter().any(|d| {
            self.vitals
                .get(&self.devices[d].id)
                .is_none_or(|v| v.at > 0 && v.cpu.is_none() && !v.inflight)
        })
    }

    /// The background observation landed: meshes, and this host's lan candidates as
    /// the network view keeps them (the hub's lan evidence).
    pub(super) fn topology(&mut self, meshes: Vec<Mesh>, lan: Option<Value>) {
        self.meshes = meshes;
        if let Some(lan) = lan {
            let local = self.local_index();
            let v = self
                .network
                .entry(local)
                .or_insert_with(|| serde_json::json!({}));
            v["candidates"] = lan["candidates"].clone();
            v["candidates_observed_at"] = lan["observed_at"].clone();
        }
    }

    pub(super) fn vitals_replied(&mut self, d: usize, result: Result<Value>) {
        let remote = self.devices[d].target.is_some();
        let v = self.vitals.entry(self.devices[d].id.clone()).or_default();
        v.inflight = false;
        let Ok(sample) = result else {
            if remote {
                push(&mut v.rtt, None);
            }
            return;
        };
        if remote {
            push(&mut v.rtt, sample["rtt_ms"].as_f64().map(|ms| ms as f32));
        }
        let cpu = sample["cpu"]
            .as_array()
            .and_then(|c| Some((c.first()?.as_u64()?, c.get(1)?.as_u64()?)));
        if let (Some((b0, t0)), Some((b1, t1))) = (v.prev, cpu) {
            if t1 >= t0 && b1 >= b0 {
                // No time passed (a frozen fixture): idle, not unknown.
                let c = (b1 - b0) as f32 / (t1 - t0).max(1) as f32;
                v.cpu = Some(c.clamp(0.0, 1.0));
                push(&mut v.cpu_hist, c.clamp(0.0, 1.0));
            }
        }
        v.prev = cpu;
        v.sample = sample;
        v.at = transport::now();
    }
}

/// `4d 2h`, `3h 12m`, `45m`.
fn uptime(secs: u64) -> String {
    let (d, h, m) = (secs / 86_400, secs / 3600 % 24, secs / 60 % 60);
    match (d, h) {
        (0, 0) => format!("{m}m"),
        (0, _) => format!("{h}h {m}m"),
        _ => format!("{d}d {h}h"),
    }
}

impl Vitals {
    /// The device's vitals for the view, while a recent sample with a cpu reading exists.
    pub(super) fn metrics(&self) -> Option<views::Metrics> {
        if transport::now().saturating_sub(self.at) > FRESH {
            return None;
        }
        let s = &self.sample;
        let total = s["mem_total_kb"].as_u64().filter(|t| *t > 0)? as f32;
        let free = s["mem_available_kb"].as_u64()? as f32;
        let load: Vec<f32> = s["load"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|v| v.as_f64().map(|v| v as f32))
            .collect();
        Some(views::Metrics {
            cpu: self.cpu?,
            mem: (1.0 - free / total).clamp(0.0, 1.0),
            mem_total_gb: total / 1_048_576.0,
            temp_c: s["temp_c"].as_f64().map(|t| t as f32),
            load: [0, 1, 2].map(|i| load.get(i).copied().unwrap_or(0.0)),
            cores: s["cores"].as_u64().unwrap_or(0).min(u64::from(u16::MAX)) as u16,
            cpu_hist: self.cpu_hist.iter().copied().collect(),
            battery: None,
            gpu: None,
            disk: s["disk"].as_f64().unwrap_or(0.0) as f32,
            uptime: s["uptime_secs"].as_u64().map(uptime).unwrap_or_default(),
        })
    }
    /// `ubuntu 22.04 aarch64`, from the last sample.
    pub(super) fn os(&self) -> String {
        let s = &self.sample;
        let os = [s["os"].as_str(), s["arch"].as_str()]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" ");
        safe_label(&os)
    }
    /// The last round trip, the ring (oldest first), loss over the ring and jitter (the
    /// mean change between successive answered round trips).
    pub(super) fn link(&self) -> (Option<f32>, Vec<Option<f32>>, f32, Option<f32>) {
        let hist: Vec<Option<f32>> = self.rtt.iter().copied().collect();
        let last = hist.iter().rev().flatten().next().copied();
        let lost = hist.iter().filter(|v| v.is_none()).count();
        let loss = if hist.is_empty() {
            0.0
        } else {
            lost as f32 / hist.len() as f32
        };
        let got: Vec<f32> = hist.iter().flatten().copied().collect();
        let jitter = (got.len() >= 3).then(|| {
            got.windows(2).map(|w| (w[1] - w[0]).abs()).sum::<f32>() / (got.len() - 1) as f32
        });
        (last, hist, loss, jitter)
    }
}

/// What the hub draws its clusters from, observed together off the UI thread: the
/// meshes, and this host's neighbour table (`network::candidates`: local `ip` reads and
/// cached name lookups, which never hold up a request lane).
pub(super) fn observe_topology() -> (Vec<Mesh>, Option<Value>) {
    (observe_meshes(), crate::network::candidates().ok())
}

/// Every mesh this host can see members of, read-only and bounded: ssh-overlay's
/// devices paired here (their endpoint is pinned), the running tailnet, and a deployed
/// Cloudflare WARP mesh's enrolled devices (one Worker read through `cx mesh devices`).
/// Slow (seconds at worst): call it off the UI thread.
fn observe_meshes() -> Vec<Mesh> {
    let mut out = Vec::new();
    let warp = crate::mesh::cloudflare::quick_status();
    if warp.provisioned {
        let members = std::env::current_exe().ok().and_then(|exe| {
            let mut c = std::process::Command::new(exe);
            c.args(["mesh", "devices", "--json"]);
            let (ok, out) =
                crate::mesh::run_bounded(c, b"", 1024 * 1024, Duration::from_secs(20)).ok()?;
            let v: Value = serde_json::from_slice(&out).ok().filter(|_| ok)?;
            Some(
                v["enrolled"]
                    .as_array()?
                    .iter()
                    .map(|e| Member {
                        name: e["name"].as_str().unwrap_or("").into(),
                        host: e["host"].as_str().unwrap_or("").into(),
                        overlay_ipv4: e["ipv4"].as_str().map(Into::into),
                        overlay_ipv6: e["ipv6"].as_str().map(Into::into),
                        state: e["state"].as_str().unwrap_or("").into(),
                        ..Member::default()
                    })
                    .collect::<Vec<_>>(),
            )
        });
        if let Some(members) = members.filter(|m| !m.is_empty()) {
            out.push(Mesh {
                id: crate::mesh::cloudflare::ID.into(),
                name: warp.name.clone().unwrap_or_else(|| "warp".into()),
                backend: MeshBackend::CloudflareWarp,
                members,
            });
        }
    }
    let paired: Vec<Member> = store::devices()
        .unwrap_or_default()
        .into_iter()
        .filter(|d| {
            d.target
                .as_deref()
                .is_some_and(|t| crate::peer_ssh::options(t).ok().flatten().is_some())
        })
        .map(|d| Member {
            name: d.name,
            host: d.host,
            ..Member::default()
        })
        .collect();
    if !paired.is_empty() {
        out.push(Mesh {
            id: crate::mesh::ssh_overlay::ID.into(),
            name: "ssh-overlay".into(),
            backend: MeshBackend::SshOverlay,
            members: paired,
        });
    }
    let tailnet = crate::mesh::tailscale::Tailscale::new();
    if let Ok(status) = tailnet.raw_status() {
        let members = crate::mesh::tailscale::members_from(&status);
        if status["BackendState"] == "Running" && !members.is_empty() {
            out.push(Mesh {
                id: crate::mesh::tailscale::ID.into(),
                name: status["CurrentTailnet"]["Name"]
                    .as_str()
                    .map_or("tailnet".into(), safe_label),
                backend: MeshBackend::Tailscale,
                members,
            });
        }
    }
    out
}

/// The host part of an SSH target (`user@host`, `user@[v6]`, or an alias).
pub(super) fn target_host(target: &str) -> &str {
    let host = target.rsplit_once('@').map_or(target, |(_, h)| h);
    host.trim_start_matches('[').trim_end_matches(']')
}

/// The device a mesh member is: the viewer for a tailnet's own node, else the device
/// whose name or host matches the member's name, host or DNS name (first label, any
/// case), or whose SSH address is the member's overlay address or DNS name.
pub(super) fn member_device(devices: &[Device], m: &Member) -> Option<usize> {
    if m.role.as_deref() == Some("self") {
        return devices.iter().position(|d| d.target.is_none());
    }
    let key = |s: &str| s.split('.').next().unwrap_or(s).to_ascii_lowercase();
    let names: Vec<String> = [Some(&m.name), Some(&m.host), m.dns_name.as_ref()]
        .into_iter()
        .flatten()
        .filter(|n| !n.is_empty())
        .map(|n| key(n))
        .collect();
    let addrs: Vec<&str> = [&m.overlay_ipv4, &m.overlay_ipv6, &m.dns_name]
        .into_iter()
        .flatten()
        .map(|a| a.trim_end_matches('.'))
        .collect();
    devices
        .iter()
        .position(|d| {
            d.target
                .as_deref()
                .is_some_and(|t| addrs.contains(&target_host(t)))
        })
        .or_else(|| {
            devices
                .iter()
                .position(|d| names.contains(&key(&d.name)) || names.contains(&key(&d.host)))
        })
}

/// The mesh whose overlay address or DNS name the device's SSH address is (its link
/// rides that mesh), by index into `meshes`.
pub(super) fn mesh_route(meshes: &[Mesh], d: &Device) -> Option<usize> {
    let host = target_host(d.target.as_deref()?);
    meshes.iter().position(|m| {
        m.members.iter().any(|x| {
            [&x.overlay_ipv4, &x.overlay_ipv6, &x.dns_name]
                .into_iter()
                .flatten()
                .any(|a| a.trim_end_matches('.') == host)
        })
    })
}

/// LAN evidence for a device, from this host's last network observation: its SSH address
/// is in this host's neighbour table or LAN candidates (not failed), or inside the subnet
/// of one of this host's own non-overlay interfaces.
pub(super) fn on_lan(observation: Option<&Value>, d: &Device) -> bool {
    let (Some(obs), Some(target)) = (observation, d.target.as_deref()) else {
        return false;
    };
    let Ok(ip) = target_host(target).parse::<std::net::IpAddr>() else {
        return false;
    };
    let rows = |v: &Value| -> Vec<Value> {
        v.as_array()
            .or_else(|| v["data"].as_array())
            .cloned()
            .unwrap_or_default()
    };
    let addr = ip.to_string();
    let neighbour = rows(&obs["neighbors"]).iter().any(|n| {
        n["dst"].as_str() == Some(&addr)
            && !n["state"]
                .as_array()
                .is_some_and(|s| s.iter().any(|x| x == "FAILED" || x == "INCOMPLETE"))
    }) || rows(&obs["candidates"])
        .iter()
        .any(|c| c["address"].as_str() == Some(&addr));
    if neighbour {
        return true;
    }
    let overlay: Vec<&str> = obs["overlays"]["interfaces"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|i| i["ifname"].as_str())
        .collect();
    rows(&obs["interfaces"])
        .iter()
        .filter(|i| {
            let name = i["ifname"].as_str().unwrap_or("");
            !overlay.contains(&name)
                && !["lo", "tailscale", "wg", "tun", "CloudflareWARP", "zt"]
                    .iter()
                    .any(|p| name.starts_with(p))
        })
        .flat_map(|i| i["addr_info"].as_array().cloned().unwrap_or_default())
        .any(|a| {
            let (Some(local), Some(len)) = (
                a["local"]
                    .as_str()
                    .and_then(|s| s.parse::<std::net::IpAddr>().ok()),
                a["prefixlen"].as_u64(),
            ) else {
                return false;
            };
            same_subnet(local, ip, len as u32)
        })
}

fn same_subnet(a: std::net::IpAddr, b: std::net::IpAddr, len: u32) -> bool {
    use std::net::IpAddr::{V4, V6};
    let (a, b, bits) = match (a, b) {
        (V4(a), V4(b)) => (u32::from(a) as u128, u32::from(b) as u128, 32),
        (V6(a), V6(b)) => (u128::from(a), u128::from(b), 128),
        _ => return false,
    };
    // A host route (/32, /128) or a link-local /64 says nothing about a shared segment.
    if len == 0 || len >= bits || (bits == 128 && a >> 118 == 0x3fa) {
        return false;
    }
    let shift = bits - len;
    a >> shift == b >> shift && a != b
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dev(name: &str, target: Option<&str>) -> Device {
        Device {
            id: name.into(),
            name: name.into(),
            target: target.map(Into::into),
            account: "t".into(),
            host: name.into(),
            status: String::new(),
            observed_at: 0,
        }
    }

    #[test]
    fn cpu_comes_from_successive_samples_and_rtt_from_round_trips() {
        let mut v = Vitals::default();
        let sample = |busy: u64, total: u64, rtt: f64| {
            serde_json::json!({"cpu":[busy,total],"mem_total_kb":4_194_304u64,
                "mem_available_kb":1_048_576u64,"cores":4,"load":[0.5,0.4,0.3],
                "uptime_secs":360_000u64,"os":"debian 12","arch":"aarch64","rtt_ms":rtt})
        };
        // (vitals_replied without an App: the same steps.)
        let feed = |v: &mut Vitals, s: Option<Value>| {
            let mut a = viewmodel::tests::hub_app();
            a.vitals.insert("inno".into(), std::mem::take(v));
            a.vitals_replied(1, s.ok_or_else(|| anyhow::anyhow!("lost")));
            *v = a.vitals.remove("inno").unwrap();
        };
        feed(&mut v, Some(sample(100, 1000, 10.0)));
        assert!(v.metrics().is_none(), "one sample has no cpu yet");
        feed(&mut v, Some(sample(400, 2000, 14.0)));
        let m = v.metrics().unwrap();
        assert!((m.cpu - 0.3).abs() < 1e-4);
        assert!((m.mem - 0.75).abs() < 1e-4 && (m.mem_total_gb - 4.0).abs() < 1e-4);
        assert_eq!((m.cores, m.uptime.as_str(), m.temp_c), (4, "4d 4h", None));
        assert_eq!(v.os(), "debian 12 aarch64");
        feed(&mut v, None);
        feed(&mut v, Some(sample(400, 2000, 12.0)));
        let (last, hist, loss, jitter) = v.link();
        assert_eq!(last, Some(12.0));
        assert_eq!(hist, vec![Some(10.0), Some(14.0), None, Some(12.0)]);
        assert_eq!(loss, 0.25);
        assert_eq!(jitter, Some(3.0));
        // No time passed: idle.
        assert_eq!(v.metrics().unwrap().cpu, 0.0);
        assert_eq!(uptime(59 * 60), "59m");
        assert_eq!(uptime(3 * 3600 + 120), "3h 2m");
    }

    /// Vitals are asked of the devices on screen whose helper says `metrics-v1` (an Info
    /// reply keeps that capability), once at a time, and never of an older helper.
    #[test]
    fn only_helpers_with_metrics_are_sampled_once_at_a_time() {
        crate::store::isolate_test_state();
        let (tx, rx) = mpsc::sync_channel(64);
        let mut a = App::new(
            vec![
                dev("local", None),
                dev("new", Some("me@new")),
                dev("old", Some("me@old")),
            ],
            tx,
        );
        a.view = View::Network;
        for (d, caps) in [(1, vec!["metrics-v1"]), (2, vec!["watch-v1"])] {
            a.apply(Reply {
                device_id: None,
                execution_target: None,
                preview: None,
                device: d,
                op: Operation::Info,
                generation: a.generation,
                result: Ok(serde_json::json!({ "capabilities": caps })),
            });
        }
        let metrics = |rx: &mpsc::Receiver<Task>| -> Vec<usize> {
            rx.try_iter()
                .filter(|t| matches!(t.op, Operation::Metrics))
                .map(|t| t.device)
                .collect()
        };
        a.poll_vitals();
        assert_eq!(metrics(&rx), vec![0, 1]);
        a.poll_vitals();
        assert_eq!(
            metrics(&rx),
            Vec::<usize>::new(),
            "one request in flight each"
        );
        assert!(!a.vitals_due(), "both in flight");
        a.vitals_replied(0, Ok(serde_json::json!({"cpu": [1, 10]})));
        a.vitals_replied(1, Err(anyhow::anyhow!("lost")));
        assert!(a.vitals_due(), "one sample, no cpu yet");
        a.vitals_replied(0, Ok(serde_json::json!({"cpu": [2, 20]})));
        assert!(!a.vitals_due(), "a failed ask waits the full period");
        assert_eq!(metrics(&rx), Vec::<usize>::new());
        a.view = View::Files;
        a.poll_vitals();
        assert_eq!(metrics(&rx), Vec::<usize>::new(), "nothing off the hub");
        a.view = View::Work;
        a.device = 2;
        a.poll_vitals();
        assert_eq!(metrics(&rx), vec![1], "only the open device");
    }

    #[test]
    fn members_map_to_devices_by_address_then_name() {
        let devices = [
            dev("tranquility", None),
            dev("innovation", Some("me@100.96.0.7")),
            dev("nas", Some("nas-alias")),
        ];
        let m = |name: &str, ip: Option<&str>, dns: Option<&str>| Member {
            name: name.into(),
            overlay_ipv4: ip.map(Into::into),
            dns_name: dns.map(Into::into),
            ..Member::default()
        };
        assert_eq!(
            member_device(&devices, &m("x", Some("100.96.0.7"), None)),
            Some(1)
        );
        assert_eq!(member_device(&devices, &m("NAS", None, None)), Some(2));
        assert_eq!(
            member_device(&devices, &m("n", None, Some("nas.tail1.ts.net."))),
            Some(2)
        );
        assert_eq!(member_device(&devices, &m("other", None, None)), None);
        let selfnode = Member {
            role: Some("self".into()),
            ..m("whatever", None, None)
        };
        assert_eq!(member_device(&devices, &selfnode), Some(0));
        let mesh = Mesh {
            id: "w".into(),
            name: "w".into(),
            backend: MeshBackend::CloudflareWarp,
            members: vec![m("innovation", Some("100.96.0.7"), None)],
        };
        assert_eq!(
            mesh_route(std::slice::from_ref(&mesh), &devices[1]),
            Some(0)
        );
        assert_eq!(mesh_route(&[mesh], &devices[2]), None);
    }

    #[test]
    fn lan_evidence_is_a_neighbour_or_a_shared_subnet() {
        let obs = serde_json::json!({
            "interfaces": {"state":"observed","data":[
                {"ifname":"eth0","addr_info":[{"local":"192.168.1.10","prefixlen":24}]},
                {"ifname":"tailscale0","addr_info":[{"local":"100.64.0.1","prefixlen":10}]}]},
            "neighbors": {"state":"observed","data":[
                {"dst":"10.0.0.9","state":["REACHABLE"]},
                {"dst":"10.0.0.8","state":["FAILED"]}]},
        });
        let lan = |t: &str| on_lan(Some(&obs), &dev("x", Some(t)));
        assert!(lan("me@192.168.1.20"));
        assert!(lan("me@10.0.0.9"));
        assert!(!lan("me@10.0.0.8"));
        assert!(!lan("me@100.64.0.5"), "an overlay interface is not a lan");
        assert!(!lan("alias"));
        assert!(!on_lan(None, &dev("x", Some("me@192.168.1.20"))));
    }
}
