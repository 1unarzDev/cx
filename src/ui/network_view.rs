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
pub(super) fn render_network(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let d = app.network_device();
    let title = d
        .map(|d| format!("Network · {}", identity(&app.devices[d])))
        .unwrap_or_else(|| "Network · select a device".into());
    let title = if app.device == 0 {
        "Network · All devices".to_string()
    } else {
        title
    };
    let outer = block(title, app.focus == Focus::Workspace);
    let inner = outer.inner(area);
    frame.render_widget(outer, area);
    let candidates = app.network_rows();
    let detail = candidates
        .get(app.network_selected)
        .map(|row| app.network_detail(row))
        .unwrap_or_default();
    // Use the installed Ratatui wrapper's count so word wrapping and Unicode match scrolling.
    let detail_lines = Paragraph::new(detail.as_str())
        .wrap(Wrap { trim: false })
        .line_count(inner.width);
    let detail_height = (detail_lines as u16)
        .max(7)
        .min(inner.height.saturating_sub(7));
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(3),
            Constraint::Length(detail_height),
        ])
        .split(inner);
    let value = d.and_then(|d| app.network.get(&d));
    let summary = value.map(|v| {
        let observed = v["internet"]["observed_at"].as_u64().unwrap_or(0);
        let age = transport::now().saturating_sub(observed);
        let stale = v["internet"]["stale"].as_bool().unwrap_or(true) || age > 90;
        let internet = if v["internet"]["state"] == "reachable" && !stale { "reachable (ICMP)" } else { "unknown" };
        let freshness = if d.is_some_and(|d| app.network_inflight.contains(&d)) { "refreshing".into() }
            else if observed == 0 { "observation age unknown".into() }
            else { format!("{age}s ago{}", if stale { " · stale" } else { "" }) };
        let interfaces = v["interfaces"].as_array().or_else(|| v["interfaces"]["data"].as_array())
            .map(|rows| rows.iter().take(8).map(|i| format!("{} {}", safe_label(i["ifname"].as_str().or_else(|| i["name"].as_str()).unwrap_or("?")), safe_label(i["operstate"].as_str().or_else(|| i["state"].as_str()).unwrap_or("unknown")))).collect::<Vec<_>>().join(" · ")).unwrap_or_else(|| "unknown".into());
        format!("{} · Internet {internet}\nInterfaces: {interfaces}\n{freshness} · automatic SSH-port checks", d.map(|d| identity(&app.devices[d])).unwrap_or_else(|| "unknown".into()))
    }).unwrap_or_else(|| if app.network_loading { "Reading network evidence…".into() } else { "No network observation · select device and refresh".into() });
    frame.render_widget(
        Paragraph::new(summary).wrap(Wrap { trim: false }),
        sections[0],
    );
    if candidates.is_empty() {
        frame.render_widget(
            Paragraph::new("No enrolled devices or observed LAN neighbors"),
            sections[1],
        );
        return;
    }
    let compact = sections[1].width < 45;
    let rows = candidates.iter().map(|row| {
        let depth = row["_depth"].as_u64().unwrap_or(0) as usize;
        let branch = row["_branch"] == true;
        let expanded = app
            .network_expanded
            .contains(row["_key"].as_str().unwrap_or(""));
        let marker = if branch {
            if expanded {
                "v"
            } else {
                ">"
            }
        } else {
            " "
        };
        let state = if let Some(d) = row["_peer"].as_u64() {
            let status = app.peer_status(d as usize);
            if status.contains("checking") {
                "~"
            } else if status.contains("reached") {
                "+"
            } else if status.contains("unavailable") {
                "!"
            } else {
                "?"
            }
        } else {
            match neighbor_ssh(row, transport::now()) {
                "port open" => "+",
                "port closed" => "!",
                _ => "?",
            }
        };
        let label = if row["_peer"].is_u64() {
            safe_label(row["_label"].as_str().unwrap_or("unknown"))
        } else if let Some(d) = row["_known_peer"].as_u64() {
            format!(
                "{} · {}",
                safe_label(&app.devices[d as usize].name),
                safe_label(row["address"].as_str().unwrap_or("unknown"))
            )
        } else if let Some(name) = row["hostname"].as_str() {
            format!(
                "{} · {}",
                safe_label(name),
                safe_label(row["address"].as_str().unwrap_or("unknown"))
            )
        } else {
            safe_label(row["address"].as_str().unwrap_or("unknown"))
        };
        let label = format!("{}{marker} {state} {label}", "  ".repeat(depth));
        if compact {
            return Row::new(vec![Cell::from(label)]);
        }
        let route = if let Some(d) = row["_peer"].as_u64() {
            identity(&app.devices[d as usize])
        } else {
            safe_label(row["interface"].as_str().unwrap_or(""))
        };
        Row::new(vec![Cell::from(label), Cell::from(route)])
    });
    let table = Table::new(
        rows,
        if compact {
            vec![Constraint::Min(1)]
        } else {
            vec![Constraint::Percentage(70), Constraint::Percentage(30)]
        },
    )
    .header(
        Row::new(if compact {
            vec!["Device / LAN neighbor"]
        } else {
            vec!["Device / LAN neighbor", "Execution / link"]
        })
        .style(muted()),
    )
    .row_highlight_style(selection(app.focus == Focus::Workspace));
    let mut state = TableState::default().with_selected(Some(app.network_selected));
    frame.render_stateful_widget(table, sections[1], &mut state);
    if !detail.is_empty() {
        frame.render_widget(
            Paragraph::new(detail).wrap(Wrap { trim: false }).scroll((
                app.network_detail_scroll
                    .min((detail_lines as u16).saturating_sub(detail_height)),
                0,
            )),
            sections[2],
        );
    }
}
pub(super) fn network_summary(value: &Value) -> String {
    let internet = &value["internet"];
    let age = transport::now().saturating_sub(internet["observed_at"].as_u64().unwrap_or(0));
    let stale = internet["stale"].as_bool().unwrap_or(true) || age > 90;
    let reachable = internet["state"] == "reachable" && !stale;
    let mut lines = vec![
        format!(
            "Internet {} · {}{}",
            if reachable {
                if ascii() {
                    "+"
                } else {
                    "●"
                }
            } else {
                "?"
            },
            if reachable {
                "reachable (ICMP)"
            } else {
                "not confirmed"
            },
            if internet["observed_at"].as_u64().unwrap_or(0) > 0 {
                format!(" · {age}s ago{}", if stale { " · stale" } else { "" })
            } else {
                String::new()
            }
        ),
        "DNS / HTTPS and overlay access: unknown".to_string(),
    ];
    if let Some(backend) = value.get("backend").and_then(Value::as_str) {
        lines.push(format!("Backend: {}", safe_label(backend)));
    }
    let interfaces = value.get("interfaces");
    if interfaces
        .and_then(|v| v.get("state"))
        .and_then(Value::as_str)
        == Some("unknown")
    {
        lines.push("\nInterfaces: unknown · observation unavailable".into());
    }
    if let Some(interfaces) = interfaces.and_then(|v| {
        v.as_array()
            .or_else(|| v.get("data").and_then(Value::as_array))
    }) {
        lines.push("\nInterfaces".into());
        for interface in interfaces.iter().take(24) {
            let name = interface
                .get("display_name")
                .or_else(|| interface.get("name"))
                .or_else(|| interface.get("ifname"))
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            let state = interface
                .get("state")
                .or_else(|| interface.get("operstate"))
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            lines.push(format!("  {} · {}", safe_label(name), safe_label(state)));
            if let Some(addresses) = interface["addr_info"].as_array() {
                for address in addresses.iter().take(8) {
                    if let Some(ip) = address["local"].as_str() {
                        lines.push(format!(
                            "    {}/{}",
                            safe_label(ip),
                            address["prefixlen"]
                                .as_u64()
                                .map(|v| v.to_string())
                                .unwrap_or_else(|| "?".into())
                        ));
                    }
                }
            }
        }
    }
    if let Some(routes) = value["routes"]
        .as_array()
        .or_else(|| value["routes"]["data"].as_array())
    {
        lines.push("\nRoutes (observed table)".into());
        for route in routes.iter().take(16) {
            lines.push(format!(
                "  {} via {} · {}{}",
                safe_label(route["dst"].as_str().unwrap_or("default")),
                safe_label(route["gateway"].as_str().unwrap_or("direct")),
                safe_label(route["dev"].as_str().unwrap_or("unknown")),
                route["metric"]
                    .as_u64()
                    .map(|m| format!(" · metric {m}"))
                    .unwrap_or_default()
            ));
        }
        if routes.len() > 16 {
            lines.push(format!("  +{} more routes", routes.len() - 16));
        }
        lines.push("Policy routing / forwarded egress: unverified".into());
    }
    let sharing = value.get("sharing");
    let state = sharing
        .and_then(|v| v.get("state"))
        .and_then(Value::as_str)
        .unwrap_or("unsupported");
    let reason = sharing
        .and_then(|v| v.get("reason"))
        .and_then(Value::as_str)
        .unwrap_or("No supported mutation backend is available");
    lines.push(format!(
        "\nSharing: {}\n{}",
        safe_label(state),
        safe_label(reason)
    ));
    lines.join("\n")
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
    pub(super) fn network_detail(&self, row: &Value) -> String {
        let d = row["_device"].as_u64().unwrap_or(0) as usize;
        let observer = &self.devices[d];
        let arrow = if ascii() { " -> " } else { " → " };
        let hops = match self.network_jump_routes.get(&d) {
            Some(Some(hops)) if hops.is_empty() => "none saved".into(),
            Some(Some(hops)) => hops
                .iter()
                .map(|h| safe_label(h))
                .collect::<Vec<_>>()
                .join(arrow),
            _ => "not read".into(),
        };
        if row["_peer"].is_u64() {
            let overview = if self.device > 0 {
                self.network
                    .get(&d)
                    .map(|v| format!("\n\n{}", network_summary(v)))
                    .unwrap_or_else(|| "\nNetwork observation unavailable · Refresh".into())
            } else {
                String::new()
            };
            return format!("{} · {}\nRoute found: viewer{arrow}{}\n{} · helper evidence from viewer\nSSH jumps (saved): {hops}\nVia another observer: unknown{overview}", safe_label(&observer.name), identity(observer), safe_label(&observer.name), self.peer_status(d));
        }
        let name = row["_known_peer"]
            .as_u64()
            .and_then(|d| self.devices.get(d as usize))
            .map(|d| {
                format!(
                    "{} · matching fresh authenticated MAC + IP\n",
                    safe_label(&d.name)
                )
            })
            .unwrap_or_default();
        let observed = row["_snapshot"].as_u64().unwrap_or(0);
        let age = if observed == 0 {
            "age unknown".into()
        } else {
            format!(
                "{}s ago{}",
                transport::now().saturating_sub(observed),
                if transport::now().saturating_sub(observed) > 90 {
                    " · stale"
                } else {
                    ""
                }
            )
        };
        let hostname = row["hostname"]
            .as_str()
            .map(|h| {
                format!(
                    "Host hint: {} · name lookup, SSH identity unchecked\n",
                    safe_label(h)
                )
            })
            .unwrap_or_default();
        format!("{name}{hostname}Enter: connect via observer and add to All devices\nDiscovery: {} · {} · {age}\nRoute found: viewer{arrow}{}{arrow}{} / {}\nSSH jumps to observer (saved): {hops}\nSSH {} · authentication unknown\nInternet unknown · source {} · link {}", safe_label(&observer.name), safe_label(row["source"].as_str().unwrap_or("unknown")), safe_label(&observer.name), safe_label(row["address"].as_str().unwrap_or("unknown")), safe_label(row["interface"].as_str().unwrap_or("unknown")), neighbor_ssh(row, transport::now()), safe_label(row["source"].as_str().unwrap_or("unknown")), safe_label(row["link_state"].as_str().unwrap_or("unknown")))
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
