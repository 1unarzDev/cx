//! Network/neighbor view: observation refresh, neighbor probes, tree rows and the network render.
use super::*;

// Network browser rendering: evidence remains scoped to the selected enrolled device.
pub(super) fn normalized_mac(value: &str) -> Option<String> {
    let bytes: Vec<_> = value.split(':').collect();
    if bytes.len() != 6
        || bytes
            .iter()
            .any(|b| b.len() != 2 || !b.bytes().all(|c| c.is_ascii_hexdigit()))
    {
        return None;
    }
    let normalized = value.to_ascii_lowercase();
    if normalized == "00:00:00:00:00:00" || normalized == "ff:ff:ff:ff:ff:ff" {
        return None;
    }
    Some(normalized)
}
pub(super) fn neighbor_connectable(candidate: &Value) -> bool {
    !matches!(candidate["address"].as_str().and_then(|s| s.parse::<std::net::IpAddr>().ok()), Some(std::net::IpAddr::V6(ip)) if ip.is_unicast_link_local())
}
/// SSH port evidence of one candidate at `now`: `port open`, `port closed` or `unknown`.
pub(super) fn neighbor_ssh(candidate: &Value, now: u64) -> &'static str {
    let observed = candidate["observed_at"].as_u64().unwrap_or(0);
    if observed == 0 || now.saturating_sub(observed) > 90 {
        return "unknown";
    }
    match candidate["ssh"]["state"].as_str() {
        Some("open" | "tcp_reachable") => "port open",
        Some("closed") => "port closed",
        _ => "unknown",
    }
}
pub(super) fn neighbor_detail(candidate: &Value) -> String {
    format!(
        "Host {} · link {}\nSSH {} · authentication unknown\nInternet unknown · source {}",
        safe_label(candidate["hostname"].as_str().unwrap_or("unknown")),
        safe_label(candidate["link_state"].as_str().unwrap_or("unknown")),
        neighbor_ssh(candidate, transport::now()),
        safe_label(candidate["source"].as_str().unwrap_or("unknown"))
    )
}

impl App {
    pub(super) fn network_device(&self) -> Option<usize> {
        if self.device > 0 {
            Some(self.device - 1)
        } else {
            self.network_rows()
                .get(self.network_selected)
                .and_then(|c| c["_device"].as_u64())
                .map(|d| d as usize)
                .or_else(|| self.devices.iter().position(|d| d.target.is_none()))
        }
    }
    pub(super) fn network_candidates(&self) -> Vec<Value> {
        let mut rows = Vec::new();
        for d in 0..self.devices.len() {
            if self.device > 0 && self.device != d + 1 {
                continue;
            }
            if let Some(candidates) = self
                .network
                .get(&d)
                .and_then(|v| v["candidates"].as_array())
            {
                for candidate in candidates
                    .iter()
                    .filter(|c| c["address"].as_str().is_some())
                {
                    let mut candidate = candidate.clone();
                    candidate["_device"] = serde_json::json!(d);
                    candidate["_snapshot"] = self.network[&d]["candidates_observed_at"].clone();
                    rows.push(candidate);
                }
            }
        }
        rows.sort_by_key(|c| {
            (
                c["_device"].as_u64().unwrap_or(0),
                c["interface"].as_str().unwrap_or("").to_string(),
                c["source"].as_str().unwrap_or("").to_string(),
                c["address"]
                    .as_str()
                    .and_then(|s| s.parse::<std::net::IpAddr>().ok()),
            )
        });
        rows
    }
    pub(super) fn network_key(parts: Value) -> String {
        parts.to_string()
    }
    pub(super) fn peer_key(&self, d: usize) -> String {
        Self::network_key(serde_json::json!(["peer", self.devices[d].id]))
    }
    pub(super) fn known_neighbor(&self, candidate: &Value) -> Option<usize> {
        let mac = normalized_mac(candidate["lladdr"].as_str()?)?;
        let ip: std::net::IpAddr = candidate["address"].as_str()?.parse().ok()?;
        if candidate["_snapshot"]
            .as_u64()
            .is_none_or(|time| transport::now().saturating_sub(time) > 90)
        {
            return None;
        }
        let matches: Vec<_> = self
            .devices
            .iter()
            .enumerate()
            .filter_map(|(d, _)| {
                let value = self.network.get(&d)?;
                let checked = value["_authenticated_at"].as_u64()?;
                if transport::now().saturating_sub(checked) > 90 {
                    return None;
                }
                let interfaces = value["interfaces"]
                    .as_array()
                    .or_else(|| value["interfaces"]["data"].as_array())?;
                interfaces
                    .iter()
                    .any(|interface| {
                        normalized_mac(interface["address"].as_str().unwrap_or(""))
                            == Some(mac.clone())
                            && interface["addr_info"].as_array().is_some_and(|addresses| {
                                addresses.iter().any(|address| {
                                    address["local"]
                                        .as_str()
                                        .and_then(|ip| ip.parse::<std::net::IpAddr>().ok())
                                        == Some(ip)
                                })
                            })
                    })
                    .then_some(d)
            })
            .collect();
        (matches.len() == 1).then(|| matches[0])
    }
    // Every passive observation remains a separate route beneath its enrolled observer.
    pub(super) fn network_rows(&self) -> Vec<Value> {
        let candidates = self.network_candidates();
        let mut rows = Vec::new();
        for (d, device) in self.devices.iter().enumerate() {
            if self.device > 0 && self.device != d + 1 {
                continue;
            }
            let key = self.peer_key(d);
            let owned: Vec<_> = candidates
                .iter()
                .filter(|c| c["_device"] == d)
                .cloned()
                .collect();
            let in_scope = self.device == 0 || self.device == d + 1;
            let label = if owned.is_empty() {
                device.name.clone()
            } else {
                format!("{} · {} LAN", device.name, owned.len())
            };
            rows.push(serde_json::json!({"_peer":d,"_device":d,"_key":key,"_depth":0,"_label":label,"_branch":self.device == 0 && !owned.is_empty(),"_count":owned.len()}));
            if in_scope && (self.device > 0 || self.network_expanded.contains(&key)) {
                for candidate in owned {
                    rows.push(self.network_route(
                        candidate,
                        &key,
                        if self.device == 0 { 1 } else { 0 },
                    ));
                }
            }
        }
        rows
    }
    pub(super) fn network_route(&self, mut candidate: Value, parent: &str, depth: usize) -> Value {
        let d = candidate["_device"].as_u64().unwrap_or(0) as usize;
        candidate["_key"] = serde_json::json!(Self::network_key(serde_json::json!([
            "route",
            self.devices[d].id,
            candidate["interface"],
            candidate["address"]
        ])));
        candidate["_parent"] = serde_json::json!(parent);
        candidate["_depth"] = serde_json::json!(depth);
        if let Some(peer) = self.known_neighbor(&candidate) {
            candidate["_known_peer"] = serde_json::json!(peer);
        }
        candidate
    }
    pub(super) fn network_selection(&self) -> Option<(Value, Value, usize)> {
        self.network_rows().get(self.network_selected).map(|row| {
            (
                row["_key"].clone(),
                row["_parent"].clone(),
                row["_device"].as_u64().unwrap_or(0) as usize,
            )
        })
    }
    pub(super) fn restore_network_selection(&mut self, selection: Option<(Value, Value, usize)>) {
        let rows = self.network_rows();
        self.network_selected = selection
            .and_then(|(key, parent, owner)| {
                rows.iter()
                    .position(|r| r["_key"] == key)
                    .or_else(|| rows.iter().position(|r| r["_key"] == parent))
                    .or_else(|| rows.iter().position(|r| r["_key"] == self.peer_key(owner)))
            })
            .unwrap_or_else(|| self.network_selected.min(rows.len().saturating_sub(1)));
    }
    pub(super) fn network_tree_motion(&mut self, expand: bool) {
        self.network_detail_scroll = 0;
        let rows = self.network_rows();
        let Some(row) = rows.get(self.network_selected) else {
            return;
        };
        let key = row["_key"].as_str().unwrap_or("").to_owned();
        if expand {
            if row["_branch"] == true {
                if !self.network_expanded.insert(key.clone()) {
                    if let Some(child) =
                        self.network_rows().iter().position(|r| r["_parent"] == key)
                    {
                        self.network_selected = child;
                    }
                }
            }
        } else if !self.network_expanded.remove(&key) {
            if let Some(parent) = rows.iter().position(|r| r["_key"] == row["_parent"]) {
                self.network_selected = parent;
            }
        }
    }
    pub(super) fn network_action_device(&self) -> Option<usize> {
        self.network_rows()
            .get(self.network_selected)
            .and_then(|r| r["_peer"].as_u64().or_else(|| r["_device"].as_u64()))
            .map(|d| d as usize)
            .or_else(|| self.network_device())
    }
    pub(super) fn peer_status(&self, d: usize) -> String {
        if self.peer_inflight.contains(&d) {
            return "checking helper".into();
        }
        if let Some((checked, success)) = self.peer_evidence.get(&d) {
            let age = transport::now().saturating_sub(*checked);
            return if *success && age <= 90 {
                format!("helper reached · {age}s ago")
            } else if *success {
                format!("cached helper · {age}s ago")
            } else {
                format!("helper unavailable · {age}s ago")
            };
        }
        let checked = self.providers.get(&d).map(|(_, time)| *time).or_else(|| {
            self.work
                .get(d)
                .filter(|w| w.error.is_none() && w.fetched > 0)
                .map(|w| w.fetched)
        });
        checked
            .map(|time| {
                format!(
                    "cached helper · {}s ago",
                    transport::now().saturating_sub(time)
                )
            })
            .unwrap_or_else(|| "helper not checked".into())
    }
    pub(super) fn pump_peer_checks(&mut self) {
        while self.peer_inflight.len()
            + self.neighbor_probes.len()
            + self
                .network_inflight
                .union(&self.network_candidates_inflight)
                .count()
            < 4
        {
            let Some(d) = self.peer_checks.pop_front() else {
                break;
            };
            if !self.background_checks_enabled(d) {
                continue;
            }
            if self.provider_loading.contains(&d) {
                continue;
            }
            if !self.send(d, Operation::Info) {
                self.peer_checks.push_front(d);
                break;
            }
            self.provider_loading.insert(d);
            self.peer_inflight.insert(d);
        }
    }
    pub(super) fn open_neighbor(&mut self) {
        if let Some(row) = self.network_rows().get(self.network_selected).cloned() {
            self.dialog_selected = 0;
            if let Some(d) = row["_peer"].as_u64() {
                self.dialog = Some(Dialog::Peer(d as usize));
            } else if let Some(d) = row["_device"].as_u64() {
                self.connect_neighbor(d as usize, row);
            }
        } else {
            self.set_notice_as(
                NoticeKind::Warning,
                "No devices or observed LAN neighbors".into(),
            );
        }
    }
    pub(super) fn pump_network_refresh(&mut self) {
        // Four enrolled helpers at once; each host has one observation and one passive snapshot.
        while self.peer_inflight.len()
            + self.neighbor_probes.len()
            + self
                .network_inflight
                .union(&self.network_candidates_inflight)
                .count()
            < 4
        {
            let Some(d) = self.network_refresh_queue.pop_front() else {
                break;
            };
            let observation =
                self.network_inflight.contains(&d) || self.send(d, Operation::Network);
            if observation {
                self.network_inflight.insert(d);
            }
            let candidates = self.network_candidates_inflight.contains(&d)
                || self.send(d, Operation::NetworkCandidates);
            if candidates {
                self.network_candidates_inflight.insert(d);
            }
            if !observation || !candidates {
                self.network_refresh_queue.push_front(d);
                break;
            }
        }
        self.pump_peer_checks();
        self.pump_neighbor_checks();
        self.network_loading = !self.network_inflight.is_empty()
            || !self.network_candidates_inflight.is_empty()
            || !self.network_refresh_queue.is_empty();
    }
    pub(super) fn connect_neighbor(&mut self, d: usize, candidate: Value) {
        if !neighbor_connectable(&candidate) {
            self.set_notice_as(
                NoticeKind::Warning,
                "Link-local SSH enrollment needs an interface scope · unavailable here".into(),
            );
        } else if let Some(address) = candidate["address"].as_str() {
            self.network_add_target = Some((d, address.into()));
            self.input = Some(Input::Add);
            self.text.clear();
            self.set_notice(
                "Enter SSH account · enrollment checks authentication and helper access".into(),
            );
        }
    }
    pub(super) fn queue_neighbor_checks(&mut self) {
        if self.view != View::Network {
            self.neighbor_queue.clear();
            return;
        }
        let current = self.network_candidates();
        self.neighbor_queue.retain(|(d, c)| {
            current.iter().any(|row| {
                row["_device"] == *d
                    && row["address"] == c["address"]
                    && row["interface"] == c["interface"]
                    && row["_snapshot"] == c["_snapshot"]
            })
        });
        self.neighbor_evidence.retain(|_, evidence| {
            transport::now().saturating_sub(evidence["observed_at"].as_u64().unwrap_or(0)) <= 90
        });
        for candidate in self.network_candidates() {
            let Some(d) = candidate["_device"].as_u64().map(|d| d as usize) else {
                continue;
            };
            let Some(address) = candidate["address"].as_str() else {
                continue;
            };
            let Some(interface) = candidate["interface"].as_str() else {
                continue;
            };
            let observed = candidate["_snapshot"].as_u64().unwrap_or(0);
            if observed == 0
                || transport::now().saturating_sub(observed) > 90
                || address.parse::<std::net::IpAddr>().is_err()
                || interface.is_empty()
                || interface.len() >= libc::IFNAMSIZ
                || !interface
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_.-:".contains(&b))
            {
                continue;
            }
            let key = (d, address.to_owned(), Some(interface.to_owned()));
            if self.neighbor_probes.contains(&key)
                || self.neighbor_evidence.get(&key).is_some_and(|v| {
                    transport::now().saturating_sub(v["observed_at"].as_u64().unwrap_or(0)) <= 90
                })
                || self.neighbor_queue.iter().any(|(owner, c)| {
                    *owner == d
                        && c["address"] == candidate["address"]
                        && c["interface"] == candidate["interface"]
                })
                || self.neighbor_budget == 0
            {
                continue;
            }
            self.neighbor_budget -= 1;
            self.neighbor_queue.push_back((d, candidate));
        }
    }
    pub(super) fn pump_neighbor_checks(&mut self) {
        if self.view != View::Network {
            self.neighbor_queue.clear();
            return;
        }
        while self.neighbor_probes.len()
            + self.peer_inflight.len()
            + self
                .network_inflight
                .union(&self.network_candidates_inflight)
                .count()
            < 4
        {
            let Some((d, candidate)) = self.neighbor_queue.pop_front() else {
                break;
            };
            if self.device > 0 && self.device != d + 1 {
                continue;
            }
            let observed = candidate["_snapshot"].as_u64().unwrap_or(0);
            if observed == 0 || transport::now().saturating_sub(observed) > 90 {
                continue;
            }
            if !self.network_candidates().iter().any(|c| {
                c["_device"] == d
                    && c["address"] == candidate["address"]
                    && c["interface"] == candidate["interface"]
                    && c["_snapshot"] == candidate["_snapshot"]
            }) {
                continue;
            }
            if !self.probe_neighbor(d, &candidate) {
                self.neighbor_queue.push_front((d, candidate));
                break;
            }
        }
    }
    pub(super) fn probe_neighbor(&mut self, d: usize, candidate: &Value) -> bool {
        let Some(address) = candidate["address"].as_str() else {
            return true;
        };
        let interface = candidate["interface"].as_str().map(str::to_string);
        let key = (d, address.to_string(), interface.clone());
        if self.neighbor_probes.contains(&key) {
            return true;
        }
        if self.send(
            d,
            Operation::ProbeCandidate {
                address: address.into(),
                interface,
            },
        ) {
            self.neighbor_probes.insert(key);
            true
        } else {
            false
        }
    }
}
