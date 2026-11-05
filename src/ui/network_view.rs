//! Network/neighbor view: candidate labels, the network tree render and summaries.
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
pub(super) fn neighbor_ssh(candidate: &Value) -> &'static str {
    let observed = candidate["observed_at"].as_u64().unwrap_or(0);
    if observed == 0 || transport::now().saturating_sub(observed) > 90 {
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
        neighbor_ssh(candidate),
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
            match neighbor_ssh(row) {
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
    .row_highlight_style(selected_style());
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
