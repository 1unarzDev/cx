use super::*;
use ratatui::backend::TestBackend;
fn app() -> App {
    let (tx, _) = mpsc::sync_channel(32);
    App::new(
        vec![Device {
            id: "local".into(),
            name: "workstation".into(),
            target: None,
            account: "tester".into(),
            host: "workstation".into(),
            status: "unknown".into(),
            observed_at: 0,
        }],
        tx,
    )
}
fn queued_app() -> (App, mpsc::Receiver<Task>) {
    let (tx, rx) = mpsc::sync_channel(64);
    let mut devices = app().devices;
    let mut remote = devices[0].clone();
    remote.id = "remote".into();
    remote.name = "laptop".into();
    remote.account = "peace".into();
    remote.host = "laptop".into();
    remote.target = Some("laptop".into());
    devices.push(remote);
    let mut a = App::new(devices, tx);
    for d in 0..a.devices.len() {
        a.providers
            .insert(d, (vec!["claude".into(), "codex".into()], transport::now()));
    }
    (a, rx)
}
fn file_app() -> (App, mpsc::Receiver<Task>) {
    let (mut a, rx) = queued_app();
    let mut b = Browser::new(0, "/files".into());
    b.entries = ["alpha.txt", "beta.txt", "gamma.txt", ".secret"]
        .iter()
        .map(|name| Entry {
            name: (*name).into(),
            path: format!("/files/{name}"),
            kind: "file".into(),
            size: 15,
            identity: Some(format!("identity-{name}")),
            hidden: name.starts_with('.'),
            rename_name: Some((*name).into()),
        })
        .collect();
    a.browser = Some(b);
    a.view = View::Files;
    (a, rx)
}
fn press(a: &mut App, c: char) {
    a.key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
}

#[test]
fn device_access_marker_and_toggle_guard_are_clear() {
    let (mut a, _) = queued_app();
    a.policies[1].access = store::AccessMode::Core;
    let mut terminal = ratatui::Terminal::new(TestBackend::new(80, 24)).unwrap();
    terminal.draw(|frame| render(frame, &a)).unwrap();
    let rendered = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(rendered.contains(if ascii() { "<-->" } else { "⟷" }));
    a.focus = Focus::Devices;
    press(&mut a, 'b');
    assert!(a.notice.contains("Select one remote device"));
}
fn expand_network_observers(a: &mut App) {
    for d in 0..a.devices.len() {
        a.network_expanded.insert(a.peer_key(d));
        a.network_expanded
            .insert(App::network_key(serde_json::json!([
                "lan",
                a.devices[d].id
            ])));
    }
}
fn select_network_route(a: &mut App, d: usize, address: &str) {
    expand_network_observers(a);
    a.network_selected = a
        .network_rows()
        .iter()
        .position(|r| r["_device"] == d && r["address"] == address)
        .expect("visible route");
}
fn select_network_peer(a: &mut App, d: usize) {
    a.network_selected = a
        .network_rows()
        .iter()
        .position(|r| r["_peer"] == d)
        .expect("visible peer");
}
fn network_fixture(a: &mut App) {
    a.view = View::Network;
    a.focus = Focus::Workspace;
    a.network.insert(0, serde_json::json!({"internet":{"state":"unknown"}, "candidates":[
            {"address":"192.0.2.2","interface":"eth0","source":"neighbor","link_state":"STALE","ssh":{"state":"unknown"}},
            {"address":"192.0.2.1","interface":"eth0","source":"neighbor","link_state":"REACHABLE","observed_at":transport::now(),"ssh":{"state":"open"}}
        ]}));
    select_network_route(a, 0, "192.0.2.1");
}
fn observer_network_fixture(a: &mut App) {
    a.view = View::Network;
    a.focus = Focus::Workspace;
    let mut candidates = vec![
        serde_json::json!({"address":"192.0.2.20","interface":"eth0","lladdr":"02:11:22:33:44:55","link_state":"REACHABLE","source":"neighbor"}),
        serde_json::json!({"address":"2001:db8::20","interface":"eth0","lladdr":"02:11:22:33:44:55","link_state":"STALE","source":"neighbor"}),
        serde_json::json!({"address":"192.0.2.21","interface":"eth1","lladdr":"02:11:22:33:44:55","link_state":"STALE","source":"neighbor"}),
        serde_json::json!({"address":"2001:db8::21","interface":"eth1","lladdr":"02:11:22:33:44:55","link_state":"STALE","source":"neighbor"}),
        serde_json::json!({"address":"192.0.2.30","interface":"eth0","link_state":"STALE","source":"neighbor"}),
        serde_json::json!({"address":"192.0.2.31","interface":"eth0","link_state":"STALE","source":"neighbor"}),
    ];
    candidates.extend((100..111).map(|i| serde_json::json!({"address":format!("192.0.2.{i}"),"interface":"eth0","link_state":"FAILED","source":"neighbor"})));
    a.network.insert(0,serde_json::json!({"candidates_observed_at":transport::now(),"candidates":candidates,"internet":{"state":"unknown"}}));
    a.network.insert(1,serde_json::json!({"candidates_observed_at":transport::now(),"candidates":[
            {"address":"192.0.2.20","interface":"eth0","lladdr":"02:11:22:33:44:55","link_state":"STALE","source":"neighbor"},
            {"address":"192.0.2.22","interface":"eth0","lladdr":"02:11:22:33:44:55","link_state":"STALE","source":"neighbor"}
        ],"internet":{"state":"unknown"}}));
}
#[test]
fn relative_markdown_click_uses_document_host_and_restores_parent() {
    let (mut a, rx) = file_app();
    let source = "# Top\n[same](one.md) [same](../two%20file.md#section)\n\nOriginal document";
    let b = a.browser.as_mut().unwrap();
    b.device = 1;
    b.selected = 2;
    b.preview_path = Some("/remote/docs/index.md".into());
    b.preview = Some(source.into());
    b.preview_rich = Some(RichPreview::from_value(
        &serde_json::json!({"kind":"markdown","text":source}),
    ));
    let mut t = Terminal::new(TestBackend::new(80, 24)).unwrap();
    t.draw(|f| render(f, &a)).unwrap();
    let target = "../two%20file.md#section";
    let point = *a
        .browser
        .as_ref()
        .unwrap()
        .preview_link_cells
        .borrow()
        .iter()
        .find(|(_, _, tag)| *tag == markdown_link_tag(target))
        .unwrap();
    assert_eq!(
        t.backend().buffer()[(point.0, point.1)].bg,
        Color::Reset,
        "private hit-test tags never reach terminal"
    );
    let mouse = MouseEvent {
        kind: MouseEventKind::Down(event::MouseButton::Left),
        column: point.0,
        row: point.1,
        modifiers: KeyModifiers::NONE,
    };
    assert!(a.preview_mouse(mouse, Rect::new(0, 0, 80, 24)));
    assert_eq!(a.dialog_selected, 1);
    a.dialog = None;
    assert!(a.preview_mouse(
        MouseEvent {
            modifiers: KeyModifiers::SHIFT,
            ..mouse
        },
        Rect::new(0, 0, 80, 24)
    ));
    let task = rx.try_recv().unwrap();
    assert_eq!(task.device, 1);
    assert!(matches!(&task.op,Operation::Preview{path} if path=="/remote/docs/../two file.md"));
    let linked = "# Section\nLinked destination\n\n# Section";
    a.apply(Reply {
        device: 1,
        generation: a.generation,
        op: task.op,
        preview: None,
        result: Ok(serde_json::json!({"kind":"markdown","text":linked})),
    });
    assert_eq!(a.browser.as_ref().unwrap().device, 1);
    assert_eq!(
        a.browser.as_ref().unwrap().preview_path.as_deref(),
        Some("/remote/docs/../two file.md")
    );
    assert_eq!(a.browser.as_ref().unwrap().preview_history.len(), 1);
    assert!(markdown_anchor_scroll(a.browser.as_ref().unwrap(), "section-1").is_some());
    a.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    let back = rx.try_recv().unwrap();
    assert_eq!(back.device, 1);
    assert!(matches!(&back.op,Operation::Preview{path} if path=="/remote/docs/index.md"));
    a.apply(Reply {
        device: 1,
        generation: a.generation,
        op: back.op,
        preview: None,
        result: Ok(serde_json::json!({"kind":"markdown","text":source})),
    });
    assert_eq!(a.browser.as_ref().unwrap().selected, 2);
    assert!(a
        .browser
        .as_ref()
        .unwrap()
        .preview
        .as_deref()
        .unwrap()
        .contains("Original document"));
    assert_eq!(a.browser.as_ref().unwrap().path, "/files");
    t.draw(|f| render(f, &a)).unwrap();
    let point = *a
        .browser
        .as_ref()
        .unwrap()
        .preview_link_cells
        .borrow()
        .iter()
        .find(|(_, _, tag)| *tag == markdown_link_tag("one.md"))
        .unwrap();
    assert!(a.preview_mouse(
        MouseEvent {
            column: point.0,
            row: point.1,
            modifiers: KeyModifiers::CONTROL,
            ..mouse
        },
        Rect::new(0, 0, 80, 24)
    ));
    assert!(
        matches!(rx.try_recv().unwrap().op,Operation::Preview{path} if path=="/remote/docs/one.md")
    );
}

#[test]
fn leaving_linked_preview_discards_history_even_in_cached_locations() {
    let (mut a, _rx) = file_app();
    let b = a.browser.as_mut().unwrap();
    b.preview_history
        .push(("/old.md".into(), 5, PreviewFind::default()));
    b.preview_restore = Some((5, PreviewFind::default()));
    b.preview_anchor = Some("old".into());
    a.open_browser(0, "/other".into());
    let old = a
        .browser_cache
        .get(&(0, "/files".into(), String::new()))
        .unwrap();
    assert!(old.preview_history.is_empty());
    assert!(old.preview_restore.is_none());
    assert!(old.preview_anchor.is_none());
    a.open_browser(0, "/files".into());
    let b = a.browser.as_mut().unwrap();
    b.preview_history
        .push(("/old.md".into(), 5, PreviewFind::default()));
    a.refresh_browser();
    assert!(a.browser.as_ref().unwrap().preview_history.is_empty());
}

#[test]
fn markdown_links_math_and_click_picker_are_safe() {
    let (mut a, _tasks) = file_app();
    let source = "# [Docs](https://example.org/docs) [Bad](javascript:alert(1))\n\n- first second third fourth fifth sixth\n  continued bullet text\n\nInline $x^2 + \\alpha$ and `\\alpha`\n\n$$\n\\frac{a}{b} + \\sqrt{x}\n$$\n\n~~~text\n[code](https://ignored) $x^2$\n~~~";
    let b = a.browser.as_mut().unwrap();
    b.preview = Some(source.into());
    b.preview_path = Some("/fixture/document.md".into());
    b.preview_rich = Some(RichPreview::from_value(
        &serde_json::json!({"kind":"markdown", "text":source}),
    ));
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    terminal.draw(|f| render(f, &a)).unwrap();
    let b = a.browser.as_ref().unwrap();
    assert!(!b.preview_link_cells.borrow().is_empty());
    let position = b.preview_link_cells.borrow()[0];
    assert!(a.preview_mouse(
        MouseEvent {
            kind: MouseEventKind::Down(event::MouseButton::Left),
            column: position.0,
            row: position.1,
            modifiers: KeyModifiers::NONE
        },
        Rect::new(0, 0, 80, 24)
    ));
    assert!(
        matches!(&a.dialog,Some(Dialog::Links(links)) if links.len()==2 && links[0].target=="https://example.org/docs")
    );
    a.dialog_selected = 1;
    a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(a.notice.contains("unsupported URI scheme"));
    a.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    let b = a.browser.as_ref().unwrap();
    let display = preview_display_lines(b, source)
        .iter()
        .map(|l| {
            l.spans
                .iter()
                .map(|s| s.content.as_ref())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");
    if !ascii() {
        assert!(display.contains("x² + α"), "{display}");
        assert!(display.contains("√"), "{display}");
    }
    assert!(display.contains("[code](https://ignored) $x^2$"));
    assert!(!display.contains("[Docs]"));
    assert!(a.browser.as_ref().unwrap().preview.is_some());
    a.text = "Inline".into();
    a.update_preview_search();
    terminal.draw(|f| render(f, &a)).unwrap();
    let buffer = terminal.backend().buffer();
    let mut nonlink = None;
    for y in 0..24 {
        for x in 0..80 {
            if buffer[(x, y)].symbol() == "I"
                && buffer[(x, y)].modifier.contains(Modifier::UNDERLINED)
            {
                nonlink = Some((x, y));
            }
        }
    }
    let (x, y) = nonlink.expect("highlighted ordinary search text");
    assert!(!a.preview_mouse(
        MouseEvent {
            kind: MouseEventKind::Down(event::MouseButton::Left),
            column: x,
            row: y,
            modifiers: KeyModifiers::NONE
        },
        Rect::new(0, 0, 80, 24)
    ));
    assert!(a.dialog.is_none());
}
#[test]
fn markdown_lists_continuations_unicode_and_code_keep_layout() {
    for width in [12, 20, 40] {
        let mut b = Browser::new(0, "/fixture".into());
        let source="1. 漢字 one two three four five six\n   continued words below\n  - nested words one two three four\n\n~~~sh\n- literal code bullet\n~~~";
        b.preview_rich = Some(RichPreview::from_value(
            &serde_json::json!({"kind":"markdown","text":source}),
        ));
        b.preview_viewport.set((width, 20));
        let lines = preview_display_lines(&b, source);
        let contents = lines
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();
        assert!(
            contents.iter().any(|l| l.starts_with("   continued")),
            "{contents:?}"
        );
        assert!(contents.iter().any(|l| l.contains("literal code bullet")));
        assert!(lines.iter().all(|l| l.width() <= usize::from(width)
            || l.spans.iter().any(|s| s.content.contains("literal code"))));
    }
    let raw = preview_inline(
        "Unsupported $\\begin{matrix}x_1\\end{matrix}$ and escaped \\$x^2\\$ and `x^2`",
    );
    let content = raw.iter().map(|s| s.content.as_ref()).collect::<String>();
    assert!(content.contains("$\\begin{matrix}x_1\\end{matrix}$"));
    assert!(content.contains("\\$x^2\\$"));
    assert_eq!(
        preview_inline("Costs $5 and $10")
            .iter()
            .map(|s| s.content.as_ref())
            .collect::<String>(),
        "Costs $5 and $10"
    );
}

#[test]
fn markdown_bullet_wrap_has_hanging_indent() {
    let mut b = Browser::new(0, "/fixture".into());
    b.preview_viewport.set((16, 8));
    let source = "- one two three four five six";
    b.preview_rich = Some(RichPreview::from_value(
        &serde_json::json!({"kind":"markdown", "text":source}),
    ));
    let lines = preview_display_lines(&b, source);
    let mut terminal = Terminal::new(TestBackend::new(16, 8)).unwrap();
    terminal
        .draw(|f| f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), f.area()))
        .unwrap();
    let buffer = terminal.backend().buffer();
    assert_eq!(buffer[(0, 1)].symbol(), " ");
    assert_eq!(buffer[(1, 1)].symbol(), " ");
    assert_ne!(buffer[(2, 1)].symbol(), " ");
}

#[test]
fn network_observer_tree_preserves_every_distinct_route_without_merging() {
    let (mut a, _rx) = queued_app();
    observer_network_fixture(&mut a);
    assert_eq!(a.network_rows().len(), 2, "observers start collapsed");
    let before = a.network_candidates();
    expand_network_observers(&mut a);
    let rows = a.network_rows();
    let routes: Vec<_> = rows.iter().filter(|r| r["address"].is_string()).collect();
    assert_eq!(routes.len(), before.len());
    assert_eq!(
        routes
            .iter()
            .map(|r| r["_key"].to_string())
            .collect::<BTreeSet<_>>()
            .len(),
        before.len()
    );
    assert_eq!(
        routes
            .iter()
            .filter(|r| r["link_state"] == "FAILED")
            .count(),
        11
    );
    assert_eq!(
        routes
            .iter()
            .filter(|r| r["address"] == "192.0.2.20")
            .count(),
        2,
        "same address from separate observers stays separate"
    );
    assert!(routes.iter().all(|r| !r["_group"].is_string()));
    assert_eq!(a.network_candidates(), before);
    a.device = 2;
    assert!(a
        .network_rows()
        .iter()
        .filter(|r| !r["_peer"].is_u64())
        .all(|r| r["_device"] == 1));
}
#[test]
fn network_friendly_names_require_unique_fresh_authenticated_mac_and_ip() {
    let (mut a, _rx) = queued_app();
    observer_network_fixture(&mut a);
    let authenticated = serde_json::json!({"interfaces":{"data":[{"address":"02:11:22:33:44:55","addr_info":[{"local":"192.0.2.20"},{"local":"2001:db8::20"}]}]},"internet":{"state":"unknown"}});
    a.apply(Reply {
        device: 1,
        generation: a.generation,
        op: Operation::Network,
        result: Ok(authenticated.clone()),
        preview: None,
    });
    let c = a
        .network_candidates()
        .into_iter()
        .find(|c| c["_device"] == 0 && c["address"] == "192.0.2.20")
        .unwrap();
    assert_eq!(a.known_neighbor(&c), Some(1));
    let mut wrong_mac = c.clone();
    wrong_mac["lladdr"] = serde_json::json!("02:aa:bb:cc:dd:ee");
    assert_eq!(a.known_neighbor(&wrong_mac), None);
    let mut absent_mac = c.clone();
    absent_mac["lladdr"] = Value::Null;
    assert_eq!(a.known_neighbor(&absent_mac), None);
    expand_network_observers(&mut a);
    assert!(a.network_rows().iter().any(|r| r["_known_peer"] == 1));
    a.apply(Reply {
        device: 0,
        generation: a.generation,
        op: Operation::Network,
        result: Ok(authenticated),
        preview: None,
    });
    assert_eq!(
        a.known_neighbor(&c),
        None,
        "shared authenticated MAC+IP is ambiguous"
    );
    a.network.get_mut(&0).unwrap()["_authenticated_at"] = Value::Null;
    a.network.get_mut(&1).unwrap()["_authenticated_at"] =
        serde_json::json!(transport::now().saturating_sub(91));
    assert_eq!(a.known_neighbor(&c), None);
    a.network.get_mut(&1).unwrap()["_authenticated_at"] = serde_json::json!(transport::now());
    let mut stale = c;
    stale["_snapshot"] = serde_json::json!(transport::now().saturating_sub(91));
    assert_eq!(a.known_neighbor(&stale), None);
}
#[test]
fn network_device_has_direct_terminal_without_creating_a_tmux_session() {
    let (mut a, rx) = queued_app();
    observer_network_fixture(&mut a);
    a.dialog = Some(Dialog::Peer(1));
    a.dialog_selected = 4;
    a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(
        a.pending_terminal.as_ref().map(|d| d.id.as_str()),
        Some("remote")
    );
    assert!(rx.try_recv().is_err());
}

#[test]
fn named_neighbors_are_visible_and_manual_add_preserves_selected_observer() {
    let (mut a, _rx) = queued_app();
    observer_network_fixture(&mut a);
    a.network.get_mut(&1).unwrap()["candidates"][0]["hostname"] =
        serde_json::json!("blastoise-odroid.local");
    expand_network_observers(&mut a);
    let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
    terminal.draw(|f| render(f, &a)).unwrap();
    let text = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect::<String>();
    assert!(
        text.contains("blastoise-odroid.local"),
        "hostname missing from network rows"
    );
    a.device = 2;
    select_network_peer(&mut a, 1);
    a.execute(Action::Add);
    for c in "roboboat@blastoise-odroid.local".chars() {
        press(&mut a, c);
    }
    a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(
        a.pending_add_via.as_ref().map(|d| d.id.as_str()),
        Some("remote")
    );
}

#[test]
fn network_tree_keys_expand_collapse_and_capture_exact_child_route() {
    let (mut a, rx) = queued_app();
    observer_network_fixture(&mut a);
    select_network_peer(&mut a, 1);
    press(&mut a, 'l');
    assert!(a.network_expanded.contains(&a.peer_key(1)));
    press(&mut a, 'l');
    assert_eq!(
        a.network_rows()[a.network_selected]["address"],
        "192.0.2.20"
    );
    press(&mut a, 'j');
    assert_eq!(
        a.network_rows()[a.network_selected]["address"],
        "192.0.2.22"
    );
    let selected = a.network_selection();
    press(&mut a, 'h');
    assert_eq!(a.network_rows()[a.network_selected]["_peer"], 1);
    press(&mut a, 'h');
    assert!(!a.network_expanded.contains(&a.peer_key(1)));
    a.restore_network_selection(selected);
    assert_eq!(a.network_rows()[a.network_selected]["_peer"], 1);
    press(&mut a, 'l');
    select_network_route(&mut a, 1, "192.0.2.22");
    a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(a.network_add_target, Some((1, "192.0.2.22".into())));
    a.device = 1;
    for c in "alice".chars() {
        press(&mut a, c);
    }
    a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(a.pending_add.as_deref(), Some("alice@192.0.2.22"));
    assert_eq!(a.pending_add_via.as_ref().unwrap().id, "remote");
    assert!(rx.try_recv().is_err());
}
#[test]
fn network_refresh_preserves_observer_expansion_and_selection_identity() {
    let (mut a, _rx) = queued_app();
    observer_network_fixture(&mut a);
    expand_network_observers(&mut a);
    select_network_route(&mut a, 0, "2001:db8::20");
    let mut candidates = a.network[&0]["candidates"].as_array().unwrap().clone();
    candidates.reverse();
    a.apply(Reply {
        device: 0,
        generation: a.generation,
        op: Operation::NetworkCandidates,
        result: Ok(serde_json::json!({"observed_at":transport::now(),"candidates":candidates})),
        preview: None,
    });
    assert_eq!(
        a.network_rows()[a.network_selected]["address"],
        "2001:db8::20"
    );
    assert!(a.network_expanded.contains(&a.peer_key(0)));
    a.view = View::Work;
    a.view = View::Network;
    assert!(a.network_expanded.contains(&a.peer_key(0)));
    assert_eq!(
        a.network_rows()[a.network_selected]["address"],
        "2001:db8::20"
    );
    a.focus = Focus::Devices;
    a.device = 1;
    a.move_selection(1);
    assert_eq!(
        a.network_rows()[a.network_selected]["address"],
        "192.0.2.22",
        "new observer scope contains only its own neighbors"
    );
}
#[test]
fn network_route_details_separate_discovery_from_saved_jump_hops() {
    let (mut a, _rx) = queued_app();
    observer_network_fixture(&mut a);
    expand_network_observers(&mut a);
    select_network_route(&mut a, 1, "192.0.2.22");
    a.network_jump_routes
        .insert(1, Some(vec!["saved-bastion".into()]));
    let row = a.network_rows()[a.network_selected].clone();
    let detail = a.network_detail(&row);
    assert!(detail.contains("Route found: viewer"));
    assert!(detail.contains("laptop"));
    assert!(detail.contains("192.0.2.22 / eth0"));
    assert!(detail.contains("SSH jumps to observer (saved): saved-bastion"));
    assert!(detail.contains("authentication unknown"));
    assert!(!detail.contains("authenticated transit"));
    a.network_jump_routes.insert(
        1,
        Some((0..8).map(|i| format!("long-saved-gateway-{i}")).collect()),
    );
    let before = capture_app(&a, 48);
    assert!(before.contains("Route"));
    for _ in 0..30 {
        press(&mut a, 'J');
    }
    let after = capture_app(&a, 48);
    assert!(after.contains("authentication"));
    assert!(after.contains("Internet unknown"));
    press(&mut a, 'j');
    assert_eq!(a.network_detail_scroll, 0);
}
#[test]
fn network_observer_capture_matrix_has_no_evidence_column() {
    for width in [48, 80, 120] {
        for height in [24, 40] {
            for state in ["collapsed", "expanded", "route", "unresolved"] {
                let (mut a, _rx) = queued_app();
                observer_network_fixture(&mut a);
                if state != "collapsed" {
                    expand_network_observers(&mut a);
                }
                if state == "route" {
                    select_network_route(&mut a, 1, "192.0.2.22");
                }
                if state == "unresolved" {
                    select_network_route(&mut a, 0, "192.0.2.100");
                }
                let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
                terminal.draw(|f| render(f, &a)).unwrap();
                let text = terminal
                    .backend()
                    .buffer()
                    .content
                    .chunks(width as usize)
                    .map(|row| row.iter().map(|c| c.symbol()).collect::<String>())
                    .collect::<Vec<_>>()
                    .join("\n");
                assert!(!text.contains("Evidence"));
                assert!(text.contains("Network"));
                if let Some(directory) = std::env::var_os("CX_NETWORK_CAPTURE_DIR") {
                    let directory = std::path::PathBuf::from(directory);
                    std::fs::create_dir_all(&directory).unwrap();
                    std::fs::write(
                        directory.join(format!("tree-{width}x{height}-{state}.txt")),
                        text,
                    )
                    .unwrap();
                }
            }
        }
    }
}
#[test]
fn network_enrolled_peers_exist_without_lan_and_capture_execution_identity() {
    let (mut a, rx) = queued_app();
    a.view = View::Network;
    a.device = 1;
    assert!(a.network_candidates().is_empty());
    let rows = a.network_rows();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["_peer"], 0);
    a.device = 2;
    assert_eq!(a.network_rows()[0]["_peer"], 1);
    a.network_selected = 0;
    a.open_neighbor();
    assert!(matches!(a.dialog, Some(Dialog::Peer(1))));
    a.device = 2; // Action identity was captured before scope changed.
    a.dialog_selected = 1;
    a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(a.browser.as_ref().unwrap().device, 1);
    assert!(rx
        .try_iter()
        .any(|t| t.device == 1 && matches!(t.op, Operation::List { .. })));
    assert!(a.pending_add.is_none());
}
#[test]
fn network_peer_checks_bounded_cached_offline_and_no_redraw_requests() {
    let (mut a, rx) = queued_app();
    a.view = View::Network;
    a.device = 2;
    a.providers.clear();
    for index in 2..7 {
        let mut d = a.devices[1].clone();
        d.id = format!("peer-{index}");
        a.devices.push(d);
        a.work.push(Cached {
            sessions: vec![],
            loading: false,
            error: None,
            fetched: 0,
        });
    }
    a.refresh();
    let tasks: Vec<_> = rx.try_iter().collect();
    assert_eq!(
        tasks
            .iter()
            .filter(|t| matches!(t.op, Operation::Info))
            .count(),
        1
    );
    assert!(tasks
        .iter()
        .filter(|t| matches!(t.op, Operation::Network | Operation::NetworkCandidates))
        .all(|t| t.device < a.devices.len()));
    assert_eq!(a.peer_inflight.len(), 1);
    a.apply(Reply {
        device: 1,
        generation: a.generation,
        op: Operation::Info,
        result: Err(anyhow::anyhow!("offline")),
        preview: None,
    });
    assert_eq!(a.peer_inflight.len(), 0);
    assert!(a.peer_status(1).contains("unavailable"));
    let next: Vec<_> = rx.try_iter().collect();
    assert!(next.is_empty());
    a.peer_evidence
        .insert(1, (transport::now().saturating_sub(100), true));
    assert!(a.peer_status(1).contains("cached helper"));
    a.network_selected = 0;
    let text = capture_app(&a, 120);
    assert!(text.contains("laptop"));
    assert!(text.contains("helper evidence from viewer"));
    assert!(text.contains("Via another observer: unknown"));
    assert!(rx.try_recv().is_err());
}
#[test]
fn network_peer_selection_survives_lan_reordering_and_actions_target_peer() {
    let (mut a, rx) = queued_app();
    network_fixture(&mut a);
    select_network_peer(&mut a, 1);
    a.apply(Reply {
        device: 0,
        generation: a.generation,
        op: Operation::NetworkCandidates,
        result: Ok(serde_json::json!({"candidates":[{"address":"192.0.2.0","interface":"eth0"}]})),
        preview: None,
    });
    assert_eq!(a.network_rows()[a.network_selected]["_peer"], 1);
    assert_eq!(a.command_context(), Some((1, "~".into())));
    a.execute(Action::New);
    assert!(matches!(a.dialog, Some(Dialog::Provider(1, _))));
    a.dialog = None;
    a.execute(Action::Files);
    assert_eq!(a.browser.as_ref().unwrap().device, 1);
    assert!(rx
        .try_iter()
        .any(|t| t.device == 1 && matches!(t.op, Operation::List { .. })));
}
#[test]
fn add_device_modal_is_compact_and_keeps_contextual_keys() {
    let (mut a, _rx) = queued_app();
    a.execute(Action::Add);
    let text = capture_app(&a, 100);
    assert!(text.contains("Add by SSH address"));
    assert!(!text.contains("Enter confirm"));
    assert!(!text.contains("Escape cancel"));
    a.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(a.input.is_none());
    assert!(a.pending_add.is_none());
}
#[test]
fn network_numeric_order_stale_port_and_linklocal_enrollment() {
    let (mut a, _rx) = queued_app();
    network_fixture(&mut a);
    let stale = serde_json::json!({"address":"fe80::1","interface":"eth0","observed_at":transport::now().saturating_sub(100),"ssh":{"state":"tcp_reachable"}});
    assert_eq!(neighbor_ssh(&stale), "unknown");
    assert!(!neighbor_connectable(&stale));
    a.dialog = Some(Dialog::Neighbor(0, stale));
    a.dialog_selected = 1;
    a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(a.input.is_none());
    assert!(a.pending_add.is_none());
    a.network.insert(0, serde_json::json!({"candidates":[{"address":"192.0.2.100","interface":"eth0"},{"address":"192.0.2.2","interface":"eth0"}]}));
    assert_eq!(a.network_candidates()[0]["address"], "192.0.2.2");
}
#[test]
fn network_browser_capture_matrix() {
    for width in [48, 80, 120] {
        for height in [24, 40] {
            for state in ["all", "unknown", "menu", "account"] {
                let (mut a, _rx) = queued_app();
                network_fixture(&mut a);
                a.network.insert(1, serde_json::json!({"internet":{"state":"unknown"},"candidates":[{"address":"192.0.2.20","interface":"wlan0","source":"neighbor","link_state":"STALE","ssh":{"state":"unknown"}}]}));
                if state == "unknown" {
                    a.device = 2;
                }
                if state == "menu" || state == "account" {
                    select_network_route(&mut a, 1, "192.0.2.20");
                    a.open_neighbor();
                }
                if state == "account" {
                    press(&mut a, 'u');
                }
                let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
                terminal.draw(|f| render(f, &a)).unwrap();
                let text = terminal
                    .backend()
                    .buffer()
                    .content
                    .chunks(width as usize)
                    .map(|row| row.iter().map(|c| c.symbol()).collect::<String>())
                    .collect::<Vec<_>>()
                    .join("\n");
                assert!(text.contains("Network"));
                if let Some(directory) = std::env::var_os("CX_NETWORK_CAPTURE_DIR") {
                    let directory = std::path::PathBuf::from(directory);
                    std::fs::create_dir_all(&directory).unwrap();
                    std::fs::write(
                        directory.join(format!("{width}x{height}-{state}.txt")),
                        text,
                    )
                    .unwrap();
                }
            }
        }
    }
}
#[test]
fn new_session_shortcut_chooses_before_launching_at_home() {
    let (mut a, rx) = queued_app();
    a.device = 2;
    a.view = View::Work;
    press(&mut a, 'n');
    assert!(a.view == View::Work);
    assert!(matches!(&a.dialog,Some(Dialog::SessionChooser(1,path)) if path == "~"));
    assert!(a.browser.is_none());
    assert!(rx.try_recv().is_err());
    press(&mut a, 's');
    assert!(rx.try_iter().any(|task| task.device == 1 && matches!(task.op, Operation::Create(ref c) if c.directory == "~" && c.provider == "shell")));
}
#[test]
fn palette_files_always_asks_for_execution_device() {
    let (mut a, rx) = queued_app();
    a.device = 2;
    a.view = View::Work;
    a.input = Some(Input::Palette);
    a.text = "Files".into();
    a.palette_selected = 0;
    a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(matches!(
        a.dialog,
        Some(Dialog::Device(ChooseDevice::Files))
    ));
    assert!(a.browser.is_none());
    assert!(rx.try_recv().is_err());
}
#[test]
fn palette_network_picker_includes_all_and_scopes_queries_after_choice() {
    let (mut a, rx) = queued_app();
    a.device = 1;
    a.view = View::Network;
    a.execute_palette(Action::Network);
    assert!(matches!(
        a.dialog,
        Some(Dialog::Device(ChooseDevice::Network))
    ));
    assert_eq!(
        a.device_choices(ChooseDevice::Network),
        vec![None, Some(0), Some(1)]
    );
    assert!(rx.try_recv().is_err());
    a.dialog_selected = 2;
    a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(a.device, 2);
    assert!(a.view == View::Network);
    assert!(rx.try_iter().all(|task| task.device == 1));
    a.execute_palette(Action::Network);
    a.dialog_selected = 0;
    a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(a.device, 0);
}
#[test]
fn palette_add_gateway_and_cancel_keep_the_selected_view() {
    let (mut a, rx) = queued_app();
    a.device = 1;
    a.execute_palette(Action::Files);
    a.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(a.device, 1);
    assert!(a.browser.is_none());
    assert!(rx.try_recv().is_err());
    a.execute_palette(Action::Add);
    assert_eq!(
        a.device_choices(ChooseDevice::AddGateway),
        vec![None, Some(1)]
    );
    a.dialog_selected = 1;
    a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(a.pending_add_via.as_ref().unwrap().id, a.devices[1].id);
    assert!(a.input == Some(Input::Add));
    a.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    a.execute_palette(Action::Add);
    a.dialog_selected = 0;
    a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(a.pending_add_via.is_none());
}
#[test]
fn palette_scope_and_device_chooser_render_at_terminal_widths() {
    for width in [48, 80, 120] {
        let (mut a, _) = queued_app();
        a.device = 2;
        a.input = Some(Input::Palette);
        a.text = "Network".into();
        assert!(capture_app(&a, width).contains("Next: choose device"));
        a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        let text = capture_app(&a, width);
        assert!(text.contains("All devices"));
        assert!(text.contains("laptop"));
    }
}
#[test]
fn focused_network_only_lists_and_refreshes_the_selected_observer() {
    let (mut a, rx) = queued_app();
    observer_network_fixture(&mut a);
    a.device = 2;
    let rows = a.network_rows();
    assert!(
        rows.iter().all(|r| r["_device"] == 1),
        "focused view leaked other devices"
    );
    assert!(
        rows.iter().any(|r| r["address"] == "192.0.2.22"),
        "focused neighbors hidden behind collapsed host"
    );
    a.refresh();
    let tasks: Vec<_> = rx.try_iter().collect();
    assert!(!tasks.is_empty());
    assert!(
        tasks.iter().all(|task| task.device == 1),
        "focused refresh queried another host"
    );
    assert!(a.peer_checks.iter().all(|d| *d == 1));
    a.device = 0;
    assert_eq!(
        a.network_rows()
            .iter()
            .filter(|r| r["_peer"].is_u64())
            .count(),
        a.devices.len()
    );
}

#[test]
fn network_all_scope_batches_every_host_and_captures_neighbor_owner() {
    let (mut a, rx) = queued_app();
    network_fixture(&mut a);
    let template = a.devices[1].clone();
    for i in 2..7 {
        let mut d = template.clone();
        d.id = format!("remote-{i}");
        a.devices.push(d);
    }
    a.refresh();
    let tasks: Vec<_> = rx.try_iter().collect();
    assert_eq!(tasks.len(), 8);
    assert_eq!(a.network_inflight.len(), 4);
    assert_eq!(a.network_refresh_queue.len(), 3);
    for op in [Operation::Network, Operation::NetworkCandidates] {
        a.apply(Reply {
            device: 0,
            generation: a.generation,
            op,
            result: Err(anyhow::anyhow!("offline")),
            preview: None,
        });
    }
    let next: Vec<_> = rx.try_iter().collect();
    assert_eq!(next.len(), 2);
    assert!(next.iter().all(|t| t.device == 4));
    assert_eq!(a.network_inflight.len(), 4);
    a.network.insert(1, serde_json::json!({"candidates":[{"address":"192.0.2.1","interface":"eth0","source":"neighbor","ssh":{"state":"unknown"}}]}));
    select_network_route(&mut a, 1, "192.0.2.1");
    a.open_neighbor();
    assert_eq!(a.network_add_target, Some((1, "192.0.2.1".into())));
    a.device = 1;
    press(&mut a, 'u');
    a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(a.pending_add.as_deref(), Some("u@192.0.2.1"));
    assert_eq!(a.pending_add_via.as_ref().unwrap().id, "remote");
    assert!(rx.try_recv().is_err());
}
#[test]
fn network_fleet_reply_preserves_selected_owner_at_duplicate_address() {
    let (mut a, _rx) = queued_app();
    network_fixture(&mut a);
    a.network.insert(1, serde_json::json!({"candidates":[{"address":"192.0.2.1","interface":"eth0","source":"neighbor"}]}));
    select_network_route(&mut a, 1, "192.0.2.1");
    a.apply(Reply {device:0, generation:a.generation, op:Operation::NetworkCandidates, result:Ok(serde_json::json!({"candidates":[{"address":"192.0.2.0","interface":"eth0","source":"neighbor"}]})), preview:None});
    assert_eq!(a.network_rows()[a.network_selected]["address"], "192.0.2.1");
    assert_eq!(a.network_device(), Some(1));
}
#[test]
fn network_neighbors_sorted_and_unknown_is_honest() {
    let mut a = app();
    network_fixture(&mut a);
    assert_eq!(a.network_candidates()[0]["address"], "192.0.2.1");
    let text = capture_app(&a, 100);
    assert!(text.contains("Internet unknown"));
    assert!(text.contains("port open"));
    assert!(text.contains("authentication unknown"));
    assert!(text.contains("Internet unknown · source neighbor"));
}
#[test]
fn network_neighbor_enter_starts_account_without_manual_check() {
    let (mut a, rx) = queued_app();
    network_fixture(&mut a);
    a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(rx.try_recv().is_err());
    assert!(a.input == Some(Input::Add));
    assert!(a.dialog.is_none());
    assert_eq!(a.network_add_target, Some((0, "192.0.2.1".into())));
    assert!(!capture_app(&a, 100).contains("Check SSH"));
    a.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(a.pending_add.is_none());
}
#[test]
fn network_auto_ssh_is_bounded_cached_and_scoped_without_redraw_work() {
    let (mut a, rx) = queued_app();
    a.view = View::Network;
    a.device = 1;
    let candidates: Vec<_> = (1..=50).map(|i| serde_json::json!({"address":format!("192.0.2.{i}"), "interface":"eth0", "ssh":{"state":"unknown"}})).collect();
    let snapshot = serde_json::json!({"observed_at":transport::now(), "candidates":candidates});
    a.apply(Reply {
        device: 0,
        generation: a.generation,
        op: Operation::NetworkCandidates,
        result: Ok(snapshot.clone()),
        preview: None,
    });
    let tasks: Vec<_> = rx.try_iter().collect();
    assert_eq!(tasks.len(), 4);
    assert!(tasks
        .iter()
        .all(|t| t.device == 0 && matches!(t.op, Operation::ProbeCandidate { .. })));
    assert_eq!(a.neighbor_queue.len(), 28);
    assert_eq!(a.neighbor_budget, 0);
    a.queue_neighbor_checks();
    a.pump_neighbor_checks();
    capture_app(&a, 120);
    assert!(rx.try_recv().is_err());
    let task = &tasks[0];
    let Operation::ProbeCandidate { address, interface } = &task.op else {
        panic!("probe");
    };
    let evidence = serde_json::json!({"address":address, "interface":interface, "observed_at":transport::now(), "ssh":{"state":"tcp_reachable"}});
    a.apply(Reply {
        device: 0,
        generation: a.generation,
        op: task.op.clone(),
        result: Ok(evidence),
        preview: None,
    });
    assert_eq!(rx.try_iter().count(), 1);
    assert_eq!(a.neighbor_probes.len(), 4);
    a.device = 2;
    a.apply(Reply {
        device: 0,
        generation: a.generation + 1,
        op: tasks[1].op.clone(),
        result: Err(anyhow::anyhow!("offline")),
        preview: None,
    });
    assert!(rx.try_recv().is_err());
    assert!(a.input.is_none());
    assert!(a.dialog.is_none());
    assert!(a.neighbor_queue.is_empty());
    a.device = 1;
    a.neighbor_budget = 32;
    a.apply(Reply {
        device: 0,
        generation: a.generation,
        op: Operation::NetworkCandidates,
        result: Ok(snapshot),
        preview: None,
    });
    assert!(a
        .network_candidates()
        .iter()
        .any(|c| c["address"] == *address && neighbor_ssh(c) == "port open"));
    assert!(!rx
        .try_iter()
        .any(|t| matches!(t.op, Operation::ProbeCandidate {address:ref ip,..} if ip == address)));
}
#[test]
fn network_auto_ssh_ignores_stale_unnumeric_and_missing_interface() {
    let (mut a, rx) = queued_app();
    a.view = View::Network;
    a.apply(Reply {device:0, generation:a.generation, op:Operation::NetworkCandidates, result:Ok(serde_json::json!({"observed_at":transport::now().saturating_sub(91),"candidates":[{"address":"192.0.2.1","interface":"eth0"}]})), preview:None});
    a.apply(Reply {device:0, generation:a.generation, op:Operation::NetworkCandidates, result:Ok(serde_json::json!({"observed_at":transport::now(),"candidates":[{"address":"example.test","interface":"eth0"},{"address":"192.0.2.2"},{"address":"192.0.2.3","interface":"bad iface"}]})), preview:None});
    assert!(rx.try_recv().is_err());
    assert!(a.neighbor_probes.is_empty());
}
#[test]
fn queued_neighbor_check_does_not_outlive_snapshot() {
    let (mut a, rx) = queued_app();
    a.view = View::Network;
    a.network.insert(0, serde_json::json!({"candidates_observed_at":transport::now().saturating_sub(91),"candidates":[{"address":"192.0.2.1","interface":"eth0"}]}));
    let candidate = a.network_candidates()[0].clone();
    a.neighbor_queue.push_back((0, candidate));
    a.pump_neighbor_checks();
    assert!(rx.try_recv().is_err());
    assert!(a.neighbor_queue.is_empty());
}
#[test]
fn network_connect_account_input_owns_keys_and_scope() {
    let (mut a, _rx) = queued_app();
    network_fixture(&mut a);
    a.open_neighbor();
    for c in "root".chars() {
        press(&mut a, c);
    }
    assert_eq!(a.text, "root");
    assert!(a.dialog.is_none());
    a.device = 2;
    a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(a.pending_add.as_deref(), Some("root@192.0.2.1"));
    assert_eq!(a.pending_add_via.as_ref().unwrap().id, "local");
}
#[test]
fn network_offline_invalidates_previous_evidence_and_scope_does_not_block() {
    let (mut a, rx) = queued_app();
    network_fixture(&mut a);
    a.refresh();
    a.device = 2;
    a.refresh();
    let tasks: Vec<_> = rx.try_iter().collect();
    assert!(tasks
        .iter()
        .any(|t| t.device == 0 && matches!(t.op, Operation::Network)));
    assert!(tasks
        .iter()
        .any(|t| t.device == 1 && matches!(t.op, Operation::Network)));
    a.apply(Reply {
        device: 0,
        generation: a.generation + 1,
        op: Operation::Network,
        result: Err(anyhow::anyhow!("offline")),
        preview: None,
    });
    assert_eq!(a.network[&0]["internet"]["state"], "unknown");
    assert_eq!(a.network[&0]["candidates"][1]["ssh"]["state"], "unknown");
    assert!(!a.network_inflight.contains(&0));
    assert!(a.network_inflight.contains(&1));
}
#[test]
fn failed_cached_session_keeps_refresh_visible_at_small_widths() {
    let (mut a, _) = queued_app();
    a.view = View::Work;
    a.device = 1;
    a.focus = Focus::Workspace;
    a.work[0].sessions.push(disposable_shell());
    a.work[0].error = Some("account@execution-device: Device unreachable over SSH · check its connection, then Refresh: helper closed before response".into());
    for width in [48, 80, 120] {
        let text = capture_app(&a, width);
        assert!(text.contains("Ctrl+P → Refresh"), "{width}: {text}");
        if width >= 80 {
            assert!(text.contains("cached"), "{width}: {text}");
        }
    }
    a.apply(Reply {
        device: 0,
        op: Operation::Sessions,
        generation: a.generation,
        result: Ok(serde_json::json!([])),
        preview: None,
    });
    assert!(a.work[0].sessions.is_empty() && a.work[0].error.is_none());
}
#[test]
fn shell_stop_requires_confirmation_and_rejects_external_and_agent_rows() {
    let (mut a, rx) = queued_app();
    let session: Session = serde_json::from_value(serde_json::json!({
            "id":"cx-fixture","name":"shell · fixture","directory":"/fixture","provider":"shell",
            "account":"tester","host":"workstation","pid":123,"started":"start","boot_id":"boot","external":false
        })).unwrap();
    a.work[0].sessions.push(session);
    a.view = View::Work;
    a.focus = Focus::Workspace;
    press(&mut a, 'd');
    assert!(matches!(a.dialog, Some(Dialog::StopShell(..))));
    assert!(rx.try_recv().is_err());
    press(&mut a, 'n');
    assert!(a.dialog.is_none());
    press(&mut a, 'd');
    press(&mut a, 'y');
    assert!(matches!(
        rx.try_recv().unwrap().op,
        Operation::StopSession { pid: 123, .. }
    ));
    a.work[0].sessions[0].external = true;
    press(&mut a, 'd');
    assert!(a.dialog.is_none());
    assert!(rx.try_recv().is_err());
    a.work[0].sessions[0].external = false;
    a.work[0].sessions[0].provider = "codex".into();
    press(&mut a, 'd');
    assert!(matches!(a.dialog, Some(Dialog::StopShell(..))));
    press(&mut a, 'n');
    assert!(rx.try_recv().is_err());
}

#[test]
fn file_device_selection_retargets_only_the_focused_location() {
    let (mut a, rx) = file_app();
    a.device = 1;
    a.focus = Focus::Devices;
    a.other_browser = Some(Browser::new(0, "/destination".into()));
    press(&mut a, 'j');
    assert_eq!(a.device, 2);
    assert_eq!(
        a.browser.as_ref().unwrap().device,
        1,
        "Files must follow selected device"
    );
    assert_eq!(a.browser.as_ref().unwrap().path, "~");
    assert_eq!(a.other_browser.as_ref().unwrap().path, "/destination");
    assert!(a.focus == Focus::Devices);
    assert!(rx
        .try_iter()
        .any(|t| t.device == 1 && matches!(t.op, Operation::List { .. })));
    a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(a.focus == Focus::Workspace);
    assert_eq!(a.browser.as_ref().unwrap().device, 1);
}
#[test]
fn transfer_shortcut_captures_selection_and_opens_destination_picker() {
    let (mut a, _rx) = file_app();
    press(&mut a, ' ');
    press(&mut a, 't');
    assert!(matches!(
        a.dialog,
        Some(Dialog::Device(ChooseDevice::Destination))
    ));
    assert_eq!(a.clipboard.as_ref().unwrap().entries[0].name, "alpha.txt");
    assert!(!a.clipboard.as_ref().unwrap().cut);
    assert_eq!(a.browser.as_ref().unwrap().device, 0);
    a.chosen_device(1, ChooseDevice::Destination);
    assert_eq!(a.browser.as_ref().unwrap().device, 1);
    assert_eq!(a.other_browser.as_ref().unwrap().device, 0);
    assert_eq!(a.device, 2);
    a.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    a.view = View::Files;
    a.focus = Focus::Workspace;
    press(&mut a, 'T');
    assert!(matches!(a.dialog, Some(Dialog::Jobs)));
}
#[test]
fn device_switch_restores_folder_and_does_not_consume_cut_clipboard() {
    let (mut a, _rx) = file_app();
    press(&mut a, 'x');
    a.device = 1;
    a.focus = Focus::Devices;
    a.browser.as_mut().unwrap().preview = Some("preview".into());
    press(&mut a, 'j');
    assert_eq!(a.browser.as_ref().unwrap().device, 1);
    press(&mut a, 'k');
    assert_eq!(a.browser.as_ref().unwrap().device, 0);
    assert_eq!(a.browser.as_ref().unwrap().path, "/files");
    assert!(a.clipboard.as_ref().unwrap().cut);
    assert_eq!(a.clipboard.as_ref().unwrap().device, 0);
    a.execute(Action::TransferTo);
    assert!(a.clipboard.as_ref().unwrap().cut);
    assert!(matches!(
        a.dialog,
        Some(Dialog::Device(ChooseDevice::Destination))
    ));
}
#[test]
fn escape_clearing_selection_does_not_discard_inflight_directory_listing() {
    let (mut a, _rx) = file_app();
    let b = a.browser.as_mut().unwrap();
    b.loading = true;
    b.marked.insert("/files/alpha.txt".into());
    let generation = a.generation;
    a.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    a.apply(Reply { preview:None, device: 0, op: Operation::List { path: "/files".into() }, generation,
            result: Ok(serde_json::json!({"path":"/files", "entries":[{"name":"fresh.txt","path":"/files/fresh.txt","kind":"file","size":1}]})) });
    assert!(!a.browser.as_ref().unwrap().loading);
    assert_eq!(a.visible_entries()[0].name, "fresh.txt");
}
#[test]
fn opening_remote_files_from_all_devices_keeps_execution_context_on_enter() {
    let (mut a, rx) = queued_app();
    a.device = 0;
    a.open_browser(1, "/project".into());
    assert_eq!(a.device, 2);
    a.focus = Focus::Devices;
    a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(a.browser.as_ref().unwrap().device, 1);
    assert_eq!(a.browser.as_ref().unwrap().path, "/project");
    assert!(!rx
        .try_iter()
        .any(|t| t.device == 0 && matches!(t.op, Operation::List { .. })));
}
#[test]
fn menu_navigation_counts_edges_and_text_ownership() {
    let (mut a, _rx) = queued_app();
    a.focus = Focus::Devices;
    press(&mut a, '2');
    press(&mut a, 'j');
    assert_eq!(a.device, 2);
    press(&mut a, 'g');
    press(&mut a, 'g');
    assert_eq!(a.device, 0);
    press(&mut a, 'G');
    assert_eq!(a.device, 2);
    press(&mut a, '2');
    press(&mut a, 'G');
    assert_eq!(a.device, 1);
    a.focus = Focus::Actions;
    press(&mut a, '3');
    press(&mut a, 'j');
    assert_eq!(a.side_selected, 3);
    press(&mut a, 'g');
    press(&mut a, 'g');
    assert_eq!(a.side_selected, 0);
    a.focus = Focus::Workspace;
    a.view = View::Work;
    a.device = 0;
    for i in 0..4 {
        let mut session = disposable_shell();
        session.id = format!("navigation-{i}");
        a.work[0].sessions.push(session);
    }
    press(&mut a, '3');
    press(&mut a, 'j');
    assert_eq!(a.selected, 3);
    press(&mut a, 'g');
    press(&mut a, 'g');
    assert_eq!(a.selected, 0);
    press(&mut a, 'G');
    assert_eq!(a.selected, 3);
    a.focus = Focus::Workspace;
    a.view = View::Containers;
    a.containers.insert(
        0,
        vec![container_fixture('a', true), container_fixture('b', true)],
    );
    press(&mut a, '2');
    press(&mut a, 'j');
    assert_eq!(a.container_selected, 2);
    press(&mut a, 'g');
    press(&mut a, 'g');
    assert_eq!(a.container_selected, 0);
    press(&mut a, 'G');
    assert_eq!(a.container_selected, a.container_rows().len() - 1);
    network_fixture(&mut a);
    press(&mut a, 'G');
    assert_eq!(a.network_selected, a.network_rows().len() - 1);
    press(&mut a, 'g');
    press(&mut a, 'g');
    assert_eq!(a.network_selected, 0);
    press(&mut a, '2');
    press(&mut a, 'j');
    assert_eq!(a.network_selected, 2.min(a.network_rows().len() - 1));
    a.dialog = Some(Dialog::SessionChooser(0, "~".into()));
    a.dialog_selected = 0;
    press(&mut a, '2');
    press(&mut a, 'l');
    assert_eq!(a.dialog_selected, 2);
    press(&mut a, 'g');
    press(&mut a, 'g');
    assert_eq!(a.dialog_selected, 0);
    press(&mut a, 'G');
    assert_eq!(a.dialog_selected, 3);
    a.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    a.focus = Focus::Devices;
    press(&mut a, '5');
    press(&mut a, 'f');
    for c in "laptop".chars() {
        press(&mut a, c);
    }
    assert_eq!(a.device_filter, "laptop");
    assert_eq!(a.device_rows(), vec![0, 2]);
    a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(a.focus == Focus::Devices);
    press(&mut a, 'j');
    assert_eq!(a.device, 2);
    a.key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE));
    a.key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
    for c in "2gg".chars() {
        press(&mut a, c);
    }
    assert_eq!(a.text, "2gg");
    assert!(a.navigation.is_idle());
}
#[test]
fn transfer_filter_and_counted_detail_navigation_preserve_job_identity() {
    let (mut a, rx) = queued_app();
    a.jobs.insert(0,serde_json::json!({"jobs":[
            {"key":"a","updated":3,"status":"failed","source_display":"alpha.txt","actual_destinations":["/target/alpha.txt"],"error":"retry later"},
            {"key":"b","updated":2,"status":"complete","source_display":"beta.txt"},
            {"key":"c","updated":1,"status":"running","source_display":"gamma.txt"}]}));
    a.dialog = Some(Dialog::Jobs);
    press(&mut a, '2');
    press(&mut a, 'j');
    assert_eq!(a.dialog_selected, 2);
    press(&mut a, 'g');
    press(&mut a, 'g');
    assert_eq!(a.dialog_selected, 0);
    press(&mut a, 'G');
    assert_eq!(a.dialog_selected, 2);
    press(&mut a, '/');
    for c in "alpha".chars() {
        press(&mut a, c);
    }
    assert_eq!(a.job_rows().len(), 1);
    assert_eq!(a.job_rows()[0].1["key"], "a");
    a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(matches!(a.dialog, Some(Dialog::Jobs)));
    assert!(rx.try_recv().is_err());
    capture_app(&a, 48);
    a.key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    press(&mut a, 'G');
    assert_eq!(a.dialog_scroll, a.dialog_scroll_max.get());
    press(&mut a, 'g');
    press(&mut a, 'g');
    assert_eq!(a.dialog_scroll, 0);
    press(&mut a, '3');
    press(&mut a, 'j');
    assert_eq!(a.dialog_scroll, 3.min(a.dialog_scroll_max.get()));
    assert_eq!(a.dialog_selected, 0);
    assert!(rx.try_recv().is_err());
}
#[test]
fn yazi_motion_counts_and_delete_yes_no_do_not_leak_into_inputs() {
    let (mut a, rx) = file_app();
    press(&mut a, '2');
    press(&mut a, 'j');
    assert_eq!(a.browser.as_ref().unwrap().selected, 2);
    press(&mut a, 'g');
    press(&mut a, 'g');
    assert_eq!(a.browser.as_ref().unwrap().selected, 0);
    press(&mut a, 'G');
    assert_eq!(a.browser.as_ref().unwrap().selected, 2);
    press(&mut a, 'd');
    press(&mut a, 'n');
    assert!(a.dialog.is_none() && rx.try_recv().is_err());
    press(&mut a, 'd');
    press(&mut a, 'y');
    assert!(matches!(
        rx.try_recv().unwrap().op,
        Operation::Remove { .. }
    ));
    press(&mut a, 'f');
    press(&mut a, '5');
    press(&mut a, 'j');
    assert_eq!(a.text, "5j");
}
#[test]
fn space_advances_range_shrinks_and_escape_preserves_then_clears_marks() {
    let (mut a, _) = file_app();
    press(&mut a, ' ');
    assert_eq!(a.browser.as_ref().unwrap().selected, 1);
    press(&mut a, 'v');
    press(&mut a, 'j');
    assert_eq!(a.browser.as_ref().unwrap().marked.len(), 3);
    press(&mut a, 'k');
    assert_eq!(a.browser.as_ref().unwrap().marked.len(), 2);
    a.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(a.browser.as_ref().unwrap().visual_anchor.is_none());
    assert_eq!(a.browser.as_ref().unwrap().marked.len(), 2);
    a.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(a.browser.as_ref().unwrap().marked.is_empty() && a.view == View::Files);
}
#[test]
fn hidden_and_filter_do_not_lose_selected_paths_or_turn_text_into_actions() {
    let (mut a, _) = file_app();
    assert_eq!(a.visible_entries().len(), 3);
    press(&mut a, '.');
    assert_eq!(a.visible_entries().len(), 4);
    a.browser.as_mut().unwrap().selected = 3;
    press(&mut a, ' ');
    press(&mut a, '.');
    assert_eq!(a.chosen_entries()[0].name, ".secret");
    press(&mut a, 'f');
    press(&mut a, 'b');
    assert_eq!(a.visible_entries().len(), 1);
    assert!(a
        .browser
        .as_ref()
        .unwrap()
        .marked
        .contains("/files/.secret"));
    press(&mut a, 'd');
    assert!(a.dialog.is_none() && a.input == Some(Input::Filter));
    a.key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
    assert_eq!(a.visible_entries().len(), 3);
    a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(a.input.is_none() && a.browser.as_ref().unwrap().preview.is_none());
}
#[test]
fn delete_defaults_to_cancel_and_uses_captured_identity() {
    let (mut a, rx) = file_app();
    press(&mut a, 'd');
    a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(rx.try_recv().is_err());
    press(&mut a, 'd');
    a.browser.as_mut().unwrap().entries[0].identity = Some("replacement".into());
    press(&mut a, 'j');
    a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(
        matches!(rx.try_recv().unwrap().op, Operation::Remove { path, expected_identity: Some(id) }
            if path == "/files/alpha.txt" && id == "identity-alpha.txt")
    );
}
#[test]
fn rename_is_one_component_and_input_owns_file_shortcuts() {
    let (mut a, rx) = file_app();
    press(&mut a, 'r');
    assert_eq!(a.text, "alpha.txt");
    a.key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
    for c in "../oops".chars() {
        press(&mut a, c);
    }
    a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(a.input == Some(Input::Rename) && rx.try_recv().is_err());
    a.key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
    for c in "cxd.txt".chars() {
        press(&mut a, c);
    }
    a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(
        matches!(rx.try_recv().unwrap().op, Operation::Rename { name, expected_identity: Some(_), .. } if name == "cxd.txt")
    );
    assert!(a.clipboard.is_none());
}
#[test]
fn copy_batch_stays_in_folder_and_paste_is_nonmodal_and_serialized() {
    let (mut a, rx) = file_app();
    press(&mut a, ' ');
    press(&mut a, ' ');
    press(&mut a, 'c');
    assert!(a.dialog.is_none());
    assert_eq!(a.clipboard.as_ref().unwrap().entries.len(), 2);
    a.browser.as_mut().unwrap().path = "/destination".into();
    press(&mut a, 'p');
    let first = rx.try_recv().unwrap();
    let Operation::Transfer(ref spec) = first.op else {
        panic!()
    };
    assert_eq!(spec.source_path, "/files/alpha.txt");
    assert!(!spec.cut);
    assert_eq!(spec.destination_path, "/destination");
    assert_eq!(spec.conflict, "rename");
    assert_eq!(a.file_queue.len(), 1);
    assert!(a.transfer_drawer && a.dialog.is_none());
    a.apply(Reply {
        preview: None,
        device: 0,
        op: first.op,
        generation: a.generation,
        result: Ok(serde_json::json!({"status":"queued"})),
    });
    assert!(
        matches!(rx.try_recv().unwrap().op, Operation::Transfer(s) if s.source_path=="/files/beta.txt")
    );
}
#[test]
fn cut_captures_identity_and_queued_quit_requires_intent() {
    let (mut a, rx) = file_app();
    press(&mut a, 'x');
    a.browser.as_mut().unwrap().path = "/destination".into();
    press(&mut a, 'p');
    assert!(
        matches!(rx.try_recv().unwrap().op, Operation::Transfer(s) if s.cut && s.source_identity.as_deref()==Some("identity-alpha.txt"))
    );
    a.request_quit();
    assert!(!a.quit);
    assert!(matches!(a.dialog, Some(Dialog::PendingExit(_))));
    a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(!a.quit);
}
#[test]
fn restart_restores_browser_host_search_focus_and_copy_without_preview_content() {
    let fixture = tempfile::tempdir().unwrap();
    let (mut a, _rx) = queued_app();
    a.device = 2;
    a.view = View::Files;
    a.focus = Focus::Actions;
    a.browser = Some(Browser::new(1, "/recordings/日本語".into()));
    let browser = a.browser.as_mut().unwrap();
    browser.search = "record".into();
    browser.preview = Some("SYNTHETIC_PRIVATE_PREVIEW".into());
    browser.entries.push(Entry {
        name: "record.bin".into(),
        path: "/recordings/日本語/record.bin".into(),
        kind: "file".into(),
        size: 5,
        identity: None,
        hidden: false,
        rename_name: None,
    });
    a.clipboard = Some(Clipboard {
        container: None,
        id: "fixture".into(),
        device: 1,
        entries: vec![browser.entries[0].clone()],
        cut: false,
        source_label: "fixture".into(),
    });
    let name = save_restart_at(&a, fixture.path()).unwrap();
    let bytes = std::fs::read(fixture.path().join(&name)).unwrap();
    assert!(!String::from_utf8_lossy(&bytes).contains("SYNTHETIC_PRIVATE_PREVIEW"));
    let (mut restored, _) = queued_app();
    restored.devices.swap(0, 1);
    restore_restart_at(&mut restored, &name, fixture.path()).unwrap();
    assert_eq!(restored.device, 1);
    assert!(restored.view == View::Files && restored.focus == Focus::Actions);
    let browser = restored.browser.as_ref().unwrap();
    assert_eq!(browser.device, 0);
    assert_eq!(browser.search, "record");
    assert_eq!(browser.path, "/recordings/日本語");
    assert!(browser.preview.is_none());
    assert_eq!(restored.clipboard.as_ref().unwrap().device, 0);
    assert!(!fixture.path().join(name).exists());
}
#[test]
fn rejected_cut_retains_clipboard_and_late_mutation_errors_remain_visible() {
    let (mut a, rx) = file_app();
    press(&mut a, 'x');
    press(&mut a, 'p');
    let task = rx.try_recv().unwrap();
    a.generation += 1;
    a.apply(Reply {
        preview: None,
        device: 0,
        op: task.op,
        generation: task.generation,
        result: Err(anyhow::anyhow!(
            "Unsupported directory move; source retained"
        )),
    });
    assert!(a
        .clipboard
        .as_ref()
        .is_some_and(|c| c.cut && c.entries.len() == 1));
    assert!(a.notice.contains("source retained"));
    let old = a.generation;
    a.generation += 1;
    a.apply(Reply {
        preview: None,
        device: 0,
        op: Operation::Rename {
            path: "/files/alpha.txt".into(),
            name: "beta.txt".into(),
            expected_identity: None,
        },
        generation: old,
        result: Err(anyhow::anyhow!("destination exists")),
    });
    assert!(a.notice.contains("destination exists"));
}
fn capture_app(a: &App, width: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, 24)).unwrap();
    terminal.draw(|f| render(f, a)).unwrap();
    terminal
        .backend()
        .buffer()
        .content
        .chunks(width as usize)
        .map(|row| row.iter().map(|c| c.symbol()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}
#[test]
fn update_menu_and_notices_identify_the_running_binary() {
    let label = ACTIONS
        .iter()
        .find(|(action, _)| *action == Action::Update)
        .unwrap()
        .1;
    assert!(label.contains(concat!("cx v", env!("CARGO_PKG_VERSION"))));
    for width in [48, 80, 120] {
        let (mut a, _rx) = file_app();
        a.input = Some(Input::Palette);
        a.text = "update".into();
        let rendered = capture_app(&a, width);
        assert!(
            rendered.contains(concat!("Update cx v", env!("CARGO_PKG_VERSION"))),
            "{rendered}"
        );
    }
    for result in [
        Ok(crate::update::CheckOutcome::Current),
        Ok(crate::update::CheckOutcome::Offline),
        Ok(crate::update::CheckOutcome::Skipped),
        Err(anyhow::anyhow!(
            "synthetic failure; no raw detail displayed"
        )),
    ] {
        let text = update_check_notice(&result);
        assert!(text.starts_with(concat!("cx v", env!("CARGO_PKG_VERSION"), " ")));
        assert!(!text.contains("synthetic failure"));
    }
    assert!(update_check_notice(&Ok(crate::update::CheckOutcome::Offline)).contains("retry"));
    assert!(update_check_notice(&Ok(crate::update::CheckOutcome::Current)).contains("up to date"));
    assert_eq!(
        notifications::area("notice", Rect::new(0, 0, 2, 1)).width,
        0
    );
}
#[test]
fn update_notice_capture_matrix_aligns_and_preserves_default_background() {
    for width in [48, 80, 120] {
        for search in [false, true] {
            for (state, result) in [
                ("current", Ok(crate::update::CheckOutcome::Current)),
                ("offline", Ok(crate::update::CheckOutcome::Offline)),
            ] {
                let (mut a, _rx) = file_app();
                a.view = View::Work;
                a.browser = None;
                a.search = if search {
                    "no-match".into()
                } else {
                    String::new()
                };
                a.set_notice_as(update_notice_kind(&result), update_check_notice(&result));
                let mut terminal = Terminal::new(TestBackend::new(width, 24)).unwrap();
                terminal.draw(|frame| render(frame, &a)).unwrap();
                let buffer = terminal.backend().buffer();
                let text = buffer
                    .content
                    .chunks(width as usize)
                    .map(|cells| cells.iter().map(|cell| cell.symbol()).collect::<String>())
                    .collect::<Vec<_>>()
                    .join("\n");
                assert!(
                    text.contains(if state == "current" {
                        "up to date"
                    } else {
                        "retry"
                    }),
                    "{width} {state}: {text}"
                );
                assert!(text.contains(if state == "current" {
                    "Success"
                } else {
                    "Warning"
                }));
                let available = Rect::new(17, 2, width - 17, if search { 15 } else { 18 });
                let popup = notifications::area(&a.notice, available);
                assert_eq!(popup.right(), width - 1);
                assert_eq!(popup.bottom(), available.bottom() - 1);
                assert!(text.lines().last().unwrap().trim().is_empty());
                assert!(buffer.content.iter().all(|cell| cell.bg == Color::Reset));
                if let Some(directory) = std::env::var_os("CX_UPDATE_CAPTURE_DIR") {
                    let directory = std::path::PathBuf::from(directory);
                    std::fs::create_dir_all(&directory).unwrap();
                    let text = buffer
                        .content
                        .chunks(width as usize)
                        .map(|cells| cells.iter().map(|cell| cell.symbol()).collect::<String>())
                        .collect::<Vec<_>>()
                        .join("\n");
                    std::fs::write(
                        directory.join(format!("{width}-{state}-search-{search}.txt")),
                        text,
                    )
                    .unwrap();
                }
            }
        }
    }
}
#[test]
fn long_failed_transfer_details_scroll_and_modal_keys_own_footer() {
    let (mut a, _rx) = file_app();
    let destination = format!("/output/{}/recording.copy-1", "long-path/".repeat(30));
    a.jobs.insert(0, serde_json::json!({"jobs":[{"key":"fixture","status":"failed","error":"Disk full · free space then retry","route":"fixture route","source_host":"tester@workstation","destination_host":"peace@laptop","source_display":format!("/source/{}", "nested/".repeat(30)),"actual_destinations":[destination]}]}));
    a.dialog = Some(Dialog::Jobs);
    for width in [48, 80] {
        let first = capture_app(&a, width);
        assert!(first.contains("Disk full") && first.contains("Route:"));
        assert!(!first
            .lines()
            .skip(21)
            .collect::<String>()
            .contains("Copy / cut"));
        let mut reached = false;
        for _ in 0..30 {
            if capture_app(&a, width).contains("recording.copy-1") {
                reached = true;
                break;
            }
            a.key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE));
        }
        assert!(
            reached,
            "actual renamed destination must be reachable by scrolling"
        );
        a.dialog_scroll = 0;
    }
}
#[test]
fn recursive_delete_warning_is_visible_and_detail_scrolling_cannot_confirm() {
    let (mut a, rx) = file_app();
    let b = a.browser.as_mut().unwrap();
    b.entries[0].kind = "directory".into();
    b.marked
        .extend(["/files/alpha.txt".into(), "/files/beta.txt".into()]);
    press(&mut a, 'd');
    let text = capture_app(&a, 48);
    assert!(
        text.contains("No undo")
            && text.contains("folders include their contents.")
            && text.contains("Cancel")
    );
    assert!(!text
        .lines()
        .skip(21)
        .collect::<String>()
        .contains("Copy / cut"));
    a.key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    press(&mut a, 'j');
    a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(a.dialog.is_some() && !a.dialog_detail_focus && rx.try_recv().is_err());
    a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(a.dialog.is_none() && rx.try_recv().is_err());
    press(&mut a, 'd');
    a.key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    a.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(a.dialog.is_none() && !a.dialog_detail_focus && rx.try_recv().is_err());
}
#[test]
fn changing_session_labels_preserves_row_order_and_selection() {
    let (mut a, _) = queued_app();
    for (id, name) in [("one", "alpha"), ("two", "zeta")] {
        a.work[0].sessions.push(
            serde_json::from_value(serde_json::json!({
                "id":id,"name":name,"directory":"/tmp","provider":"shell",
                "account":a.devices[0].account,"host":a.devices[0].host,
                "pid":1,"started":"x","boot_id":"boot","external":false,"socket":null
            }))
            .unwrap(),
        );
    }
    a.selected = 1;
    let before: Vec<_> = a.session_rows().iter().map(|(_, s)| s.id.clone()).collect();
    a.work[0].sessions[0].name = "running zsh".into();
    a.work[0].sessions[1].name = "A native task name".into();
    assert_eq!(
        before,
        a.session_rows()
            .iter()
            .map(|(_, s)| s.id.clone())
            .collect::<Vec<_>>()
    );
    assert_eq!(a.selected_session().unwrap().1.id, "two");
}
#[test]
fn restart_preserves_selected_session_identity_when_devices_reorder() {
    let fixture = tempfile::tempdir().unwrap();
    let (mut a, _) = queued_app();
    for d in 0..2 {
        a.work[d].sessions.push(serde_json::from_value(serde_json::json!({"id":format!("session-{d}"),"name":format!("session-{d}"),"directory":"/tmp","provider":"shell","account":a.devices[d].account,"host":a.devices[d].host,"pid":1,"started":"x","boot_id":"boot","external":false,"socket":null})).unwrap());
    }
    a.search = "session-1".into();
    a.selected = a
        .session_rows()
        .iter()
        .position(|(_, s)| s.id == "session-1")
        .unwrap();
    let name = save_restart_at(&a, fixture.path()).unwrap();
    let (mut restored, _) = queued_app();
    restored.devices.swap(0, 1);
    restored.work[0].sessions = a.work[1].sessions.clone();
    restored.work[1].sessions = a.work[0].sessions.clone();
    restore_restart_at(&mut restored, &name, fixture.path()).unwrap();
    assert_eq!(restored.selected_session().unwrap().1.id, "session-1");
}
#[test]
fn restored_file_identity_survives_page_one_clamping() {
    let fixture = tempfile::tempdir().unwrap();
    let (mut a, _) = queued_app();
    let mut browser = Browser::new(0, "/recordings".into());
    browser.entries = (0..1200)
        .map(|i| Entry {
            name: format!("f{i:04}"),
            path: format!("/recordings/f{i:04}"),
            kind: "file".into(),
            size: 1,
            identity: None,
            hidden: false,
            rename_name: None,
        })
        .collect();
    browser.selected = 1100;
    a.browser = Some(browser);
    a.view = View::Files;
    let name = save_restart_at(&a, fixture.path()).unwrap();
    let (mut restored, _rx) = queued_app();
    restore_restart_at(&mut restored, &name, fixture.path()).unwrap();
    for (offset, end, next) in [(0, 1000, Some(1000)), (1000, 1200, None)] {
        let entries=(offset..end).map(|i|serde_json::json!({"name":format!("f{i:04}"),"path":format!("/recordings/f{i:04}"),"kind":"file","size":1})).collect::<Vec<_>>();
        restored.apply(Reply {
            preview: None,
            device: 0,
            op: Operation::ListPage {
                path: "/recordings".into(),
                offset,
                limit: 1000,
            },
            generation: 0,
            result: Ok(
                serde_json::json!({"path":"/recordings","entries":entries,"next_offset":next}),
            ),
        });
    }
    let browser = restored.browser.as_ref().unwrap();
    assert_eq!(browser.entries[browser.selected].path, "/recordings/f1100");
}
#[test]
fn restart_rejects_unsafe_names_symlinks_and_expires() {
    let fixture = tempfile::tempdir().unwrap();
    let (a, _) = queued_app();
    let name = save_restart_at(&a, fixture.path()).unwrap();
    let (mut restored, _) = queued_app();
    assert!(restore_restart_at(&mut restored, "../anything", fixture.path()).is_err());
    let link = "viewer-restart-123.json";
    std::os::unix::fs::symlink(fixture.path().join(&name), fixture.path().join(link)).unwrap();
    assert!(restore_restart_at(&mut restored, link, fixture.path()).is_err());
    let mut value: Value =
        serde_json::from_slice(&std::fs::read(fixture.path().join(&name)).unwrap()).unwrap();
    value["expires_at"] = serde_json::json!(0);
    std::fs::write(
        fixture.path().join(&name),
        serde_json::to_vec(&value).unwrap(),
    )
    .unwrap();
    assert!(restore_restart_at(&mut restored, &name, fixture.path()).is_err());
}
#[test]
fn queued_palette_input_prevents_restart_before_and_after_dispatch() {
    let (mut a, _) = queued_app();
    assert!(restart_ready(&a, Duration::from_secs(4), false));
    assert!(!restart_ready(&a, Duration::from_secs(4), true));
    a.key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL));
    assert!(a.input == Some(Input::Palette));
    assert!(!restart_ready(&a, Duration::from_secs(4), false));
}
#[test]
fn later_directory_pages_cannot_retarget_the_selected_file() {
    let (mut a, _rx) = queued_app();
    let mut browser = Browser::new(0, "/files".into());
    browser.restore_selection = Some("/files/z".into());
    a.browser = Some(browser);
    a.view = View::Files;
    for (offset, name, next) in [(0, "z", Some(1)), (1, "a", None)] {
        a.apply(Reply{preview:None,device:0,op:Operation::ListPage{path:"/files".into(),offset,limit:1},generation:0,result:Ok(serde_json::json!({"path":"/files","entries":[{"name":name,"path":format!("/files/{name}"),"kind":"file","size":1}],"next_offset":next}))});
    }
    let browser = a.browser.as_ref().unwrap();
    assert_eq!(browser.entries[browser.selected].path, "/files/z");
}
#[test]
fn restart_waits_for_inputs_dialogs_and_mutation_responses() {
    let (mut a, _) = queued_app();
    assert!(can_restart(&a));
    a.input = Some(Input::Search);
    assert!(!can_restart(&a));
    a.input = None;
    a.dialog = Some(Dialog::Provider(0, None));
    assert!(!can_restart(&a));
    a.dialog = None;
    a.creating = true;
    assert!(!can_restart(&a));
    a.creating = false;
    a.pending_requests.set(1);
    assert!(!can_restart(&a));
}
#[test]
fn progress_reordering_preserves_job_cancellation_identity() {
    let (mut a, rx) = queued_app();
    a.dialog = Some(Dialog::Jobs);
    a.jobs.insert(
        0,
        serde_json::json!({"jobs":[
            {"key":"a", "status":"running", "updated":2},
            {"key":"b", "status":"running", "updated":1}]}),
    );
    assert_eq!(a.job_rows()[a.dialog_selected].1["key"], "a");
    a.apply(Reply {
        preview: None,
        device: 0,
        op: Operation::TransferJobs,
        generation: 0,
        result: Ok(serde_json::json!({"jobs":[
                {"key":"a", "status":"running", "updated":2},
                {"key":"b", "status":"running", "updated":3}]})),
    });
    a.key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE));
    assert!(matches!(rx.try_recv().unwrap().op, Operation::TransferCancel {key} if key == "a"));
}
#[test]
fn obsolete_network_reply_releases_loading_on_success_and_error() {
    let (mut a, _rx) = queued_app();
    a.generation = 5;
    for result in [Ok(serde_json::json!({})), Err(anyhow::anyhow!("offline"))] {
        a.network_loading = true;
        a.apply(Reply {
            preview: None,
            device: 1,
            op: Operation::Network,
            generation: 0,
            result,
        });
        assert!(!a.network_loading);
    }
}
#[test]
fn expired_agent_selection_cannot_turn_into_shell_launch() {
    let (mut a, rx) = queued_app();
    a.dialog = Some(Dialog::Provider(1, None));
    a.dialog_selected = 2;
    a.providers.insert(
        1,
        (
            vec!["claude".into(), "codex".into()],
            transport::now().saturating_sub(61),
        ),
    );
    a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(a.launch_provider.is_none());
    assert!(matches!(a.dialog, Some(Dialog::Provider(1, None))));
    assert!(matches!(rx.try_recv().unwrap().op, Operation::Info));
    a.apply(Reply {
        preview: None,
        device: 1,
        op: Operation::Info,
        generation: 0,
        result: Ok(serde_json::json!({"capabilities":["claude", "codex"]})),
    });
    assert_eq!(a.dialog_selected, 2);
}
#[test]
fn launch_choices_follow_execution_device_and_unknown_hides_agents() {
    let (mut a, _rx) = queued_app();
    a.providers
        .insert(1, (vec!["codex".into()], transport::now()));
    assert_eq!(a.provider_choices(0), vec!["shell", "claude", "codex"]);
    assert_eq!(a.provider_choices(1), vec!["shell", "codex"]);
    a.open_browser(1, "~".into());
    assert!(a.provider_choices(1).contains(&"codex"));
    assert!(!a.provider_choices(1).contains(&"claude"));
    a.dialog = Some(Dialog::Provider(1, None));
    a.dialog_selected = 1;
    a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(a.launch_provider.as_deref(), Some("codex"));
    a.providers.insert(1, (vec![], transport::now()));
    assert_eq!(a.provider_choices(1), vec!["shell"]);
    assert!(!a.provider_choices(1).contains(&"codex"));
    assert!(a.provider_choices(1).contains(&"shell"));
    a.providers.remove(&1);
    assert_eq!(a.provider_choices(1), vec!["shell"]);
    a.providers.insert(
        1,
        (vec!["codex".into()], transport::now().saturating_sub(61)),
    );
    assert_eq!(a.provider_choices(1), vec!["shell"]);
}
#[test]
fn provider_results_survive_navigation_but_failed_checks_hide_agents() {
    let (mut a, _rx) = queued_app();
    a.generation = 8;
    a.apply(Reply {
        preview: None,
        device: 1,
        op: Operation::Info,
        generation: 0,
        result: Ok(serde_json::json!({"capabilities":["tmux", "codex"]})),
    });
    assert_eq!(a.provider_choices(1), vec!["shell", "codex"]);
    a.apply(Reply {
        preview: None,
        device: 1,
        op: Operation::Info,
        generation: 1,
        result: Err(anyhow::anyhow!("unreachable")),
    });
    assert_eq!(a.provider_choices(1), vec!["shell"]);
}
#[test]
fn new_from_files_starts_current_execution_location_and_attaches_on_reply() {
    let (mut a, rx) = queued_app();
    a.browser = Some(Browser::new(1, "/project/remote".into()));
    a.browser.as_mut().unwrap().search = "recording".into();
    a.view = View::Files;
    a.focus = Focus::Actions;
    a.execute(Action::New);
    assert!(
        matches!(&a.dialog, Some(Dialog::Provider(1, Some(path))) if path == "/project/remote")
    );
    a.dialog_selected = 2;
    a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(matches!(a.dialog, Some(Dialog::AgentStart(..))));
    a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(matches!(a.dialog, Some(Dialog::Permissions(..))));
    a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    let task = rx.try_recv().unwrap();
    assert_eq!(task.device, 1);
    let Operation::Create(spec) = &task.op else {
        panic!("expected direct create, not browser reset")
    };
    assert_eq!(spec.directory, "/project/remote");
    assert_eq!(spec.provider, "codex");
    let session = serde_json::json!({"id":"new","name":"Codex","directory":"/project/remote","provider":"codex","account":"peace","host":"laptop","pid":1,"started":"x","boot_id":"boot","external":false,"socket":null});
    a.apply(Reply {
        preview: None,
        device: 1,
        op: task.op,
        generation: 0,
        result: Ok(session),
    });
    assert!(a.pending_attach.is_some());
    assert_eq!(a.browser.as_ref().unwrap().search, "recording");
    assert!(a.view == View::Files);
}
#[test]
fn actions_are_workspace_only_and_do_not_duplicate_shell_launch() {
    let (mut a, _) = queued_app();
    a.view = View::Files;
    a.browser = Some(Browser::new(0, "/project".into()));
    for (action, _) in sidebar_actions(&a) {
        assert!(workspace_action(action));
    }
    assert!(a
        .palette()
        .iter()
        .any(|(action, _)| *action == Action::Command));
    assert!(a
        .palette()
        .iter()
        .any(|(action, _)| *action == Action::Mkdir));
    assert!(sidebar_actions(&a)
        .iter()
        .any(|(_, label)| *label == "Sessions"));
    assert_eq!(
        a.palette()
            .iter()
            .filter(|(a, _)| *a == Action::New)
            .count(),
        1
    );
    assert!(!a
        .palette()
        .iter()
        .any(|(_, label)| label.contains("Start here")));
    assert!(a.palette_scope().contains("Next:") || a.palette_scope().contains("Current:"));
    press(&mut a, ':');
    assert!(a.input == Some(Input::Command));
}
#[test]
fn new_shortcut_chooses_profile_and_preserves_focused_folder() {
    let (mut a, rx) = queued_app();
    a.view = View::Files;
    a.browser = Some(Browser::new(1, "/projects/robot".into()));
    a.device = 1;
    a.focus = Focus::Workspace;
    press(&mut a, 'n');
    assert!(matches!(&a.dialog, Some(Dialog::SessionChooser(1,path)) if path == "/projects/robot"));
    assert!(rx.try_recv().is_err());
    press(&mut a, 's');
    assert!(a.dialog.is_none());
    assert!(rx.try_iter().any(|task| task.device == 1 && matches!(task.op, Operation::Create(ref c) if c.directory == "/projects/robot" && c.provider == "shell")));
}
#[test]
fn moving_to_client_only_host_drops_unavailable_launch_profile() {
    let (mut a, rx) = queued_app();
    a.view = View::Files;
    a.focus = Focus::Workspace;
    a.browser = Some(Browser::new(1, "/server/files".into()));
    a.launch_provider = Some("codex".into());
    a.providers
        .insert(1, (vec!["shell".into()], transport::now()));
    press(&mut a, 'n');
    assert!(matches!(&a.dialog, Some(Dialog::SessionChooser(1, path)) if path == "/server/files"));
    assert!(a.launch_provider.is_none());
    assert!(rx.try_recv().is_err());
    assert_eq!(a.provider_choices(1), vec!["shell"]);
}
#[test]
fn command_input_pins_host_folder_and_owns_printable_shortcuts() {
    let (mut a, _) = queued_app();
    a.view = View::Files;
    a.browser = Some(Browser::new(0, "/projects/quoted ' robot".into()));
    press(&mut a, ':');
    assert_eq!(
        a.command_target,
        Some((0, "/projects/quoted ' robot".into()))
    );
    for c in "printf n:hjl".chars() {
        press(&mut a, c);
    }
    assert_eq!(a.text, "printf n:hjl");
    a.browser = Some(Browser::new(1, "/elsewhere".into()));
    a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    let (device, command) = a.pending_command.as_ref().unwrap();
    assert_eq!(device.id, a.devices[0].id);
    assert_eq!(command.directory, "/projects/quoted ' robot");
    assert_eq!(command.command, "printf n:hjl");
    assert!(a.input.is_none() && a.text.is_empty());
    assert!(!can_restart(&a));
}
#[test]
fn long_command_scrolls_and_supports_cursor_edits() {
    let (mut a, _) = queued_app();
    a.view = View::Files;
    a.browser = Some(Browser::new(0, "/files".into()));
    press(&mut a, ':');
    for c in "x".repeat(200).chars() {
        press(&mut a, c);
    }
    for c in "TAIL".chars() {
        press(&mut a, c);
    }
    let text = capture_app(&a, 80);
    assert!(text.contains("TAIL"));
    a.key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
    a.key(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE));
    assert!(a.text.ends_with("TAI"));
    a.key(KeyEvent::new(KeyCode::Home, KeyModifiers::NONE));
    press(&mut a, '☃');
    assert!(a.text.starts_with('☃'));
    a.key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
    a.key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
    assert!(a.text.starts_with("☃x"));
}
fn container_fixture(tag: char, dev: bool) -> crate::containers::Container {
    serde_json::from_value(serde_json::json!({"engine":"fixture","id":tag.to_string().repeat(64),"name":if dev{"roboboat_dev"}else{"server_service"},"image":"fixture:latest","state":"running","started_at":"fingerprint","devcontainer":dev,"evidence":if dev{"Dev Containers labels"}else{"Docker container"},"user":"robot","folder":"/workspace","workspace":null,"config":null,"network":"host","networks":["host"],"ports":{},"allowed":false})).unwrap()
}
#[test]
fn container_mutations_pin_scope_while_commands_stay_in_terminal() {
    let (mut a, rx) = file_app();
    let scope = container_fixture('a', true).scope();
    a.open_container_browser(0, scope.clone());
    let _ = rx.try_iter().collect::<Vec<_>>();
    assert!(!a.action_enabled(Action::Command));
    assert!(a.command_context().is_none());
    a.send(
        0,
        Operation::Mkdir {
            path: "/workspace/new".into(),
        },
    );
    assert!(
        matches!(rx.try_recv().unwrap().op, Operation::ContainerFileAction { scope: c, .. } if c == scope)
    );
    a.queue_file_actions(vec![
        (
            0,
            Operation::Remove {
                path: "/workspace/a".into(),
                expected_identity: Some("proof".into()),
            },
        ),
        (
            0,
            Operation::Rename {
                path: "/workspace/b".into(),
                name: "c".into(),
                expected_identity: Some("proof".into()),
            },
        ),
    ]);
    assert!(
        matches!(rx.try_recv().unwrap().op, Operation::ContainerFileAction { scope: c, .. } if c == scope)
    );
    a.open_host_browser(0, "/workspace".into());
    let _ = rx.try_iter().collect::<Vec<_>>();
    a.file_busy = false;
    a.start_next_file_action();
    assert!(
        matches!(rx.try_recv().unwrap().op, Operation::ContainerFileAction { scope: c, .. } if c == scope)
    );
}
#[test]
fn identical_host_and_container_paths_use_independent_locations_and_cache() {
    let (mut a, rx) = queued_app();
    a.open_host_browser(0, "/workspace".into());
    let host_generation = a.generation;
    let sa = container_fixture('a', true).scope();
    let sb = container_fixture('b', true).scope();
    a.open_container_browser(0, sa.clone());
    a.open_browser(0, "/workspace/sub".into());
    assert_eq!(a.browser.as_ref().unwrap().container.as_ref(), Some(&sa));
    a.open_container_browser(0, sb.clone());
    assert_eq!(a.browser.as_ref().unwrap().path, "/workspace");
    a.open_host_browser(0, "/workspace".into());
    assert!(a.browser.as_ref().unwrap().container.is_none());
    assert!(a.generation > host_generation);
    a.open_container_browser(0, sa);
    assert_eq!(a.browser.as_ref().unwrap().path, "/workspace/sub");
    let tasks = rx.try_iter().collect::<Vec<_>>();
    assert!(tasks.iter().any(|t| matches!(t.op, Operation::List { .. })));
    assert!(tasks
        .iter()
        .any(|t| matches!(&t.op,Operation::ContainerFiles{scope,..} if scope.id==sb.id)));
}
#[test]
fn stale_container_reply_cannot_replace_current_scope_even_with_same_generation() {
    let (mut a, _rx) = queued_app();
    let sa = container_fixture('a', true).scope();
    let sb = container_fixture('b', true).scope();
    a.open_container_browser(0, sb.clone());
    a.apply(Reply {
        preview: None,
        device: 0,
        generation: a.generation,
        op: Operation::ContainerFiles {
            scope: sa,
            operation: Box::new(Operation::List {
                path: "/workspace".into(),
            }),
        },
        result: Ok(serde_json::json!({"path":"/WRONG","entries":[]})),
    });
    assert_eq!(a.browser.as_ref().unwrap().path, "/workspace");
    assert_eq!(a.browser.as_ref().unwrap().container.as_ref(), Some(&sb));
}
#[test]
fn container_tree_collapses_and_ordinary_containers_only_offer_opt_in() {
    let (mut a, rx) = queued_app();
    a.view = View::Containers;
    a.focus = Focus::Workspace;
    a.device = 1;
    a.containers.insert(
        0,
        vec![container_fixture('a', true), container_fixture('b', false)],
    );
    assert_eq!(a.container_rows().len(), 3);
    a.container_selected = 2;
    a.open_container_actions();
    assert!(matches!(&a.dialog,Some(Dialog::ContainerActions(_,c)) if !c.devcontainer));
    assert_eq!(
        container_action_labels(&container_fixture('b', false)),
        vec!["Enable access"]
    );
    a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(matches!(a.dialog, Some(Dialog::ContainerConfirm(..))) && a.dialog_selected == 0);
    a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(rx.try_recv().is_err());
    a.container_tree_motion(false);
    assert_eq!(a.container_rows().len(), 1);
    a.container_tree_motion(true);
    assert_eq!(a.container_rows().len(), 3);
    a.move_selection(1);
    assert_eq!(a.container_selected, 1);
    a.move_selection(1);
    assert_eq!(a.container_selected, 2);
}
#[test]
fn container_menus_only_launch_shells() {
    let (mut a, rx) = queued_app();
    let c = container_fixture('a', true);
    assert_eq!(
        container_action_labels(&c),
        vec!["Devcontainer terminal", "Files", "Stop"]
    );
    a.open_container_browser(0, c.scope());
    let _ = rx.try_iter().collect::<Vec<_>>();
    a.browser.as_mut().unwrap().path = "/workspace/src".into();
    a.start_shell();
    assert!(
        matches!(rx.try_recv().unwrap().op,Operation::ContainerCreate{scope,request,yolo:false} if scope.folder=="/workspace/src"&&request.provider=="shell")
    );
    a.creating = false;
    let mut session = disposable_shell();
    session.container = Some(c.scope());
    a.work[0].sessions.push(session);
    a.view = View::Work;
    a.focus = Focus::Workspace;
    press(&mut a, 'n');
    assert!(matches!(a.dialog, Some(Dialog::ContainerProvider(..))));
    a.dialog = None;
    a.view = View::Files;
    a.execute(Action::New);
    assert!(matches!(a.dialog, Some(Dialog::ContainerProvider(..))));
    press(&mut a, 'x');
    press(&mut a, 'c');
    assert!(rx.try_recv().is_err());
    press(&mut a, 's');
    assert!(
        matches!(rx.try_recv().unwrap().op,Operation::ContainerCreate{request,yolo:false,..} if request.provider=="shell")
    );
}
#[test]
fn devcontainer_startup_captures_host_folder_and_requires_confirmation() {
    let (mut a, rx) = file_app();
    assert!(a.action_enabled(Action::DevcontainerUp));
    a.execute(Action::DevcontainerUp);
    assert!(matches!(&a.dialog, Some(Dialog::DevcontainerUp(0, path)) if path == "/files"));
    assert_eq!(a.dialog_selected, 0);
    a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(rx.try_recv().is_err());
    a.execute(Action::DevcontainerUp);
    a.dialog_selected = 1;
    a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(
        matches!(rx.try_recv().unwrap().op, Operation::DevcontainerUp { workspace } if workspace == "/files")
    );
    a.open_container_browser(0, container_fixture('a', true).scope());
    assert!(!a.action_enabled(Action::DevcontainerUp));
}
#[test]
fn browser_cache_distinguishes_docker_engines() {
    let a = container_fixture('a', true).scope();
    let mut b = a.clone();
    b.engine = "other-engine".into();
    assert_ne!(browser_scope_key(Some(&a)), browser_scope_key(Some(&b)));
}
#[test]
fn session_chooser_device_focus_never_inherits_other_browser_or_search() {
    let (mut a, rx) = file_app();
    a.device = 2;
    a.focus = Focus::Devices;
    a.browser.as_mut().unwrap().search = "alpha".into();
    press(&mut a, 'n');
    assert!(matches!(&a.dialog,Some(Dialog::SessionChooser(1,path)) if path=="~"));
    assert!(rx.try_recv().is_err());
    press(&mut a, 'l');
    assert_eq!(a.dialog_selected, 1);
    press(&mut a, 'h');
    assert_eq!(a.dialog_selected, 0);
    press(&mut a, 'x');
    assert!(matches!(a.dialog, Some(Dialog::AgentStart(..))));
    a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(
        matches!(&a.dialog,Some(Dialog::Permissions(1,path,provider, _)) if path=="~" && provider=="codex")
    );
}
#[test]
fn session_chooser_missing_provider_stays_open_and_devcontainer_routes_to_tree() {
    let (mut a, rx) = queued_app();
    a.device = 2;
    a.providers
        .insert(1, (vec!["containers-v1".into()], transport::now()));
    press(&mut a, 'n');
    press(&mut a, 'c');
    assert!(matches!(a.dialog, Some(Dialog::SessionChooser(..))));
    assert!(rx.try_recv().is_err());
    press(&mut a, 'd');
    assert!(a.view == View::Containers);
    assert_eq!(a.device, 2);
    assert!(rx
        .try_iter()
        .any(|t| t.device == 1 && matches!(t.op, Operation::Containers)));
}
#[test]
fn watch_key_attaches_highlighted_session_read_only_and_is_contextual() {
    let (mut a, _rx) = queued_app();
    a.device = 2;
    a.focus = Focus::Workspace;
    a.view = View::Work;
    a.work[1].sessions=vec![serde_json::from_value(serde_json::json!({"id":"watch-proof","name":"fixture","directory":"/workspace","provider":"shell","account":"peace","host":"laptop","pid":123,"started":"proof","boot_id":"boot","external":false,"socket":null})).unwrap()];
    press(&mut a, 'w');
    assert!(matches!(&a.pending_attach,Some((1,s,true)) if s.id=="watch-proof"));
    a.pending_attach = None;
    a.focus = Focus::Devices;
    press(&mut a, 'w');
    assert!(a.pending_attach.is_none());
    a.focus = Focus::Workspace;
    a.view = View::Files;
    press(&mut a, 'w');
    assert!(a.pending_attach.is_none());
    assert!(!ACTIONS
        .iter()
        .any(|(action, _)| *action == Action::Terminal));
    assert!(!sidebar_actions(&a)
        .iter()
        .any(|(action, _)| *action == Action::Terminal));
}
#[test]
fn host_launch_does_not_offer_to_resume_same_path_container_session() {
    let (mut a, rx) = queued_app();
    let mut s:Session=serde_json::from_value(serde_json::json!({"id":"container-proof","name":"fixture","directory":"/workspace","provider":"shell","account":"tester","host":"workstation","pid":123,"started":"proof","boot_id":"boot","external":false,"socket":null})).unwrap();
    s.container = Some(container_fixture('a', true).scope());
    a.work[0].sessions = vec![s];
    a.start_at(0, "/workspace".into(), "shell".into());
    assert!(!matches!(a.dialog, Some(Dialog::Matching(..))));
    assert!(rx
        .try_iter()
        .any(|t| matches!(t.op,Operation::Create(ref c) if c.directory=="/workspace")));
}
#[test]
fn containers_ui_capture_matrix_and_default_background() {
    for (width, height) in [(48, 24), (80, 24), (120, 40)] {
        let (mut a, _rx) = queued_app();
        a.view = View::Containers;
        a.focus = Focus::Workspace;
        a.containers.insert(
            0,
            vec![container_fixture('a', true), container_fixture('b', false)],
        );
        a.container_selected = 1;
        for mode in ["tree", "actions", "files", "chooser", "container-chooser"] {
            if mode == "container-chooser" {
                a.dialog = Some(Dialog::ContainerProvider(
                    0,
                    container_fixture('a', true).scope(),
                ));
                a.dialog_selected = 0;
            } else if mode == "chooser" {
                a.dialog = Some(Dialog::SessionChooser(0, "~/robot/workspace".into()));
                a.dialog_selected = 2;
            } else if mode == "actions" {
                a.open_container_actions();
            } else if mode == "files" {
                a.dialog = None;
                a.open_container_browser(0, container_fixture('a', true).scope());
                let b = a.browser.as_mut().unwrap();
                b.loading = false;
                b.entries = vec![
                    Entry {
                        name: "src".into(),
                        path: "/workspace/src".into(),
                        kind: "directory".into(),
                        size: 0,
                        identity: None,
                        hidden: false,
                        rename_name: None,
                    },
                    Entry {
                        name: "README.md".into(),
                        path: "/workspace/README.md".into(),
                        kind: "file".into(),
                        size: 256,
                        identity: None,
                        hidden: false,
                        rename_name: None,
                    },
                    Entry {
                        name: "devcontainer.json".into(),
                        path: "/workspace/devcontainer.json".into(),
                        kind: "file".into(),
                        size: 128,
                        identity: None,
                        hidden: false,
                        rename_name: None,
                    },
                ];
            }
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal.draw(|f| render(f, &a)).unwrap();
            let buffer = terminal.backend().buffer();
            let text = buffer
                .content
                .chunks(usize::from(width))
                .map(|r| r.iter().map(|c| c.symbol()).collect::<String>())
                .collect::<Vec<_>>()
                .join("\n");
            if mode == "container-chooser" {
                assert!(text.contains("[s] Devcontainer"));
                assert!(
                    !text.contains("[c] Claude")
                        && !text.contains("[x] Codex")
                        && !text.contains("Agent session")
                );
            } else if mode == "chooser" {
                for label in ["[c] Claude", "[x] Codex", "[s] Shell", "[d] Devcontainer"] {
                    assert!(text.contains(label), "{width}: missing {label}");
                }
                let rows = if width < 68 { 2 } else { 1 };
                let dialog = popup(Rect::new(0, 0, width, height), 78, 7 + rows * 3);
                for y in dialog.y..dialog.bottom() {
                    for x in dialog.x..dialog.right() {
                        assert!(
                            !buffer[(x, y)].modifier.contains(Modifier::UNDERLINED),
                            "chooser must not underline button borders at {width}: {x},{y}"
                        );
                    }
                }
            } else {
                assert!(text.contains("roboboat_dev"), "{width} {mode}");
            }
            assert!(buffer
                .content
                .iter()
                .filter(|c| c.symbol() == " ")
                .all(|c| c.bg == Color::Reset));
            if let Some(directory) = std::env::var_os("CX_CONTAINER_CAPTURE_DIR") {
                let path = std::path::PathBuf::from(directory);
                std::fs::create_dir_all(&path).unwrap();
                std::fs::write(path.join(format!("{width}x{height}-{mode}.txt")), text).unwrap();
                let cells=buffer.content.iter().map(|cell|serde_json::json!({"text":cell.symbol(),"fg":format!("{:?}",cell.fg),"bg":format!("{:?}",cell.bg),"modifier":format!("{:?}",cell.modifier)})).collect::<Vec<_>>();
                std::fs::write(path.join(format!("{width}x{height}-{mode}.json")),serde_json::to_vec(&serde_json::json!({"width":width,"height":height,"cells":cells,"backend":"Ratatui TestBackend fixture"})).unwrap()).unwrap();
            }
        }
    }
}
#[test]
fn real_info_response_retains_native_command_capability() {
    let (mut a, _) = queued_app();
    a.view = View::Files;
    a.browser = Some(Browser::new(1, "/remote/folder".into()));
    a.apply(Reply { preview:None, device: 1, op: Operation::Info, generation: a.generation,
            result: Ok(serde_json::json!({"capabilities": ["shell", "tmux", "native-command-v1", "unrecognized"]})) });
    assert_eq!(a.provider_choices(1), vec!["shell"]);
    assert_eq!(a.providers[&1].0, vec!["native-command-v1"]);
    press(&mut a, ':');
    assert!(a.input == Some(Input::Command));
    assert_eq!(a.command_target, Some((1, "/remote/folder".into())));
}
#[test]
fn command_cancel_and_unsupported_remote_never_dispatch() {
    let (mut a, _) = queued_app();
    a.view = View::Files;
    a.browser = Some(Browser::new(0, "/files".into()));
    press(&mut a, ':');
    press(&mut a, 'n');
    a.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(a.pending_command.is_none() && a.command_target.is_none());
    a.browser = Some(Browser::new(1, "/remote".into()));
    a.providers.remove(&1);
    press(&mut a, ':');
    assert!(a.input.is_none() && a.pending_command.is_none());
    assert!(a.notice.contains("Checking"));
    a.providers
        .insert(1, (vec!["native-command-v1".into()], transport::now()));
    press(&mut a, ':');
    assert!(a.input == Some(Input::Command));
}
#[test]
fn new_session_from_all_explicitly_selects_execution_provider_directory() {
    let (mut a, rx) = queued_app();
    assert!(a.action_enabled(Action::New));
    assert!(!a.action_enabled(Action::Work));
    assert_eq!(
        ACTIONS
            .iter()
            .filter(|(_, label)| label.starts_with("Files"))
            .count(),
        1
    );
    a.execute(Action::New);
    assert!(matches!(a.dialog, Some(Dialog::Device(ChooseDevice::New))));
    a.dialog_selected = 1;
    a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(matches!(a.dialog, Some(Dialog::Provider(1, None))));
    a.dialog_selected = 2;
    a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(a.browser.as_ref().unwrap().device, 1);
    assert_eq!(a.launch_provider.as_deref(), Some("codex"));
    let sessions = rx.try_recv().unwrap();
    assert!(matches!(sessions.op, Operation::Sessions));
    let list = rx.try_recv().unwrap();
    assert!(matches!(list.op, Operation::List { .. }));
    a.browser.as_mut().unwrap().path = "/projects/test".into();
    press(&mut a, 'n');
    assert!(matches!(a.dialog, Some(Dialog::SessionChooser(..))));
    press(&mut a, 'x');
    assert!(matches!(a.dialog, Some(Dialog::AgentStart(..))));
    a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(matches!(a.dialog, Some(Dialog::Permissions(..))));
    a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    let task = rx.try_recv().unwrap();
    assert_eq!(task.device, 1);
    let Operation::Create(spec) = task.op else {
        panic!("expected create")
    };
    assert_eq!(spec.provider, "codex");
    assert_eq!(spec.directory, "/projects/test");
}
#[test]
fn agent_permissions_are_per_launch_cancelable_and_yolo_dispatches_distinct_operation() {
    let (mut a, rx) = queued_app();
    for provider in ["codex", "claude"] {
        a.create_at(0, "/folder".into(), provider.into());
        assert!(matches!(a.dialog, Some(Dialog::Permissions(..))));
        assert_eq!(a.dialog_selected, 0);
        a.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(rx.try_recv().is_err());
        a.create_at(0, "/folder".into(), provider.into());
        a.dialog_selected = 1;
        a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(
            matches!(rx.try_recv().unwrap().op, Operation::CreateYolo(c) if c.provider == provider && c.directory == "/folder")
        );
        a.create_at(0, "/folder".into(), provider.into());
        assert_eq!(a.dialog_selected, 0);
        a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(matches!(rx.try_recv().unwrap().op, Operation::Create(_)));
    }
}
#[test]
fn permission_dialog_render_sizes_and_old_helper_refusal() {
    let (mut a, rx) = queued_app();
    for (width, height) in [(48, 24), (80, 24), (120, 40)] {
        a.create_at(1, "/robot/networking".into(), "codex".into());
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|f| render(f, &a)).unwrap();
        let text = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<String>();
        assert!(text.contains("Default") && text.contains("YOLO"));
        assert!(text.contains("sandbox"));
        if let Some(directory) = std::env::var_os("CX_PERMISSIONS_CAPTURE_DIR") {
            let directory = std::path::PathBuf::from(directory);
            std::fs::create_dir_all(&directory).unwrap();
            let buffer = terminal.backend().buffer();
            let rows = buffer
                .content
                .chunks(usize::from(width))
                .map(|row| row.iter().map(|c| c.symbol()).collect::<String>())
                .collect::<Vec<_>>()
                .join("\n");
            std::fs::write(directory.join(format!("{width}x{height}.txt")), rows).unwrap();
        }
    }
    a.providers
        .insert(1, (vec!["codex".into()], transport::now()));
    a.dialog_selected = 1;
    a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(matches!(a.dialog, Some(Dialog::Permissions(..))));
    assert!(!rx
        .try_iter()
        .any(|t| matches!(t.op, Operation::CreateYolo(_))));
    a.providers.insert(
        1,
        (
            vec!["codex".into(), "session-yolo-v1".into()],
            transport::now(),
        ),
    );
    a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(matches!(
        rx.try_recv().unwrap().op,
        Operation::CreateYolo(_)
    ));
}
#[test]
fn agent_stop_confirms_exact_provider_and_refuses_old_helper() {
    let (mut a, rx) = queued_app();
    a.view = View::Work;
    a.focus = Focus::Workspace;
    let mut session = disposable_shell();
    session.provider = "claude".into();
    a.work[1].sessions.push(session);
    a.device = 2;
    a.providers
        .insert(1, (vec!["stop-session-v1".into()], transport::now()));
    press(&mut a, 'd');
    assert!(a.dialog.is_none());
    let _ = rx.try_iter().collect::<Vec<_>>();
    a.providers
        .insert(1, (vec!["stop-agent-session-v1".into()], transport::now()));
    press(&mut a, 'd');
    assert!(matches!(a.dialog, Some(Dialog::StopShell(..))));
    press(&mut a, 'y');
    assert!(
        matches!(rx.try_recv().unwrap().op, Operation::StopAgentSession {provider, ..} if provider == "claude")
    );
}
#[test]
fn matching_live_session_offers_reuse_and_explicit_new() {
    let (mut a, rx) = queued_app();
    let session: Session=serde_json::from_value(serde_json::json!({"id":"same","name":"existing","directory":"/project","provider":"codex","account":"peace","host":"laptop","pid":1,"started":"x","boot_id":"boot","external":false,"socket":null})).unwrap();
    a.work[1].sessions.push(session);
    a.start_at(1, "/project".into(), "codex".into());
    assert!(matches!(a.dialog, Some(Dialog::Matching(..))));
    assert!(a.pending_attach.is_none());
    a.dialog_selected = 1;
    a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(matches!(a.dialog, Some(Dialog::Permissions(_, _, _, true))));
    a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    let Operation::Create(resume) = rx.try_recv().unwrap().op else {
        panic!("expected native resume create")
    };
    assert!(resume.resume);

    a.start_at(1, "/project".into(), "codex".into());
    a.dialog_selected = 2;
    a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(matches!(
        a.dialog,
        Some(Dialog::Permissions(_, _, _, false))
    ));
    a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    let Operation::Create(fresh) = rx.try_recv().unwrap().op else {
        panic!("expected fresh create")
    };
    assert!(!fresh.resume);
}
fn destination_transfer_fixture() -> (App, mpsc::Receiver<Task>) {
    let (mut a, rx) = queued_app();
    a.view = View::Files;
    a.focus = Focus::Workspace;
    let mut source = Browser::new(1, "/source".into());
    source.loading = false;
    source.entries.push(Entry {
        name: "sample.txt".into(),
        path: "/source/sample.txt".into(),
        kind: "file".into(),
        size: 12,
        identity: None,
        hidden: false,
        rename_name: None,
    });
    a.browser = Some(source);
    a.execute(Action::Copy);
    a.other_browser = a.browser.take();
    let mut destination = Browser::new(0, "/receive".into());
    destination.loading = false;
    destination.entries.push(Entry {
        name: "nested".into(),
        path: "/receive/nested".into(),
        kind: "directory".into(),
        size: 0,
        identity: None,
        hidden: false,
        rename_name: None,
    });
    a.browser = Some(destination);
    a.destination_active = true;
    (a, rx)
}
#[test]
fn destination_enter_transfers_displayed_folder_once_and_releases_on_failure() {
    let (mut a, rx) = destination_transfer_fixture();
    a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    let task = rx.try_recv().unwrap();
    let Operation::Transfer(spec) = &task.op else {
        panic!("expected transfer")
    };
    assert_eq!(spec.destination_path, "/receive");
    assert_eq!(a.browser.as_ref().unwrap().path, "/receive");
    assert_eq!(a.active_transfer_keys().len(), 1);
    a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(rx.try_recv().is_err(), "double Enter must not submit again");
    a.apply(Reply {
        preview: None,
        device: task.device,
        generation: task.generation,
        op: task.op,
        result: Err(anyhow::anyhow!("fixture failure")),
    });
    assert!(a.active_transfer_keys().is_empty());
    a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(matches!(rx.try_recv().unwrap().op, Operation::Transfer(_)));
}
#[test]
fn destination_navigation_and_non_destination_enter_keep_their_meaning() {
    let (mut a, rx) = destination_transfer_fixture();
    a.key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE));
    assert_eq!(a.browser.as_ref().unwrap().path, "/receive/nested");
    assert!(!matches!(rx.try_recv().unwrap().op, Operation::Transfer(_)));
    while rx.try_recv().is_ok() {}
    a.browser.as_mut().unwrap().loading = true;
    a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(rx.try_recv().is_err());
    a.destination_active = false;
    a.browser.as_mut().unwrap().loading = false;
    a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(a.pending_transfers.is_empty());
}
#[test]
fn transfer_sidebar_animation_frames_are_transparent_and_fit() {
    let (mut a, rx) = destination_transfer_fixture();
    a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    let task = rx.try_recv().unwrap();
    let Operation::Transfer(spec) = &task.op else {
        panic!()
    };
    let key = spec.key.clone();
    a.apply(Reply {
        preview: None,
        device: 0,
        generation: a.generation,
        op: Operation::TransferJobs,
        result: Ok(
            serde_json::json!({"jobs":[{"key":key,"status":"running","bytes":12,"total":120}]}),
        ),
    });
    assert!(a.pending_transfers.is_empty());
    assert_eq!(a.active_transfer_keys().len(), 1);
    for (width, height) in [(48, 24), (80, 24), (120, 40)] {
        for phase in [0, 3, 6, 9] {
            a.transfer_frame = phase;
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal.draw(|f| render(f, &a)).unwrap();
            let buffer = terminal.backend().buffer();
            assert!(buffer.content.iter().all(|c| c.bg == Color::Reset));
            let text = buffer
                .content
                .chunks(width as usize)
                .map(|row| row.iter().map(|c| c.symbol()).collect::<String>())
                .collect::<Vec<_>>()
                .join("\n");
            assert!(text.contains(transfer_spinner(phase, ascii())), "{text}");
            assert!(text.contains("Transfers"), "{text}");
            if let Some(dir) = std::env::var_os("CX_TRANSFER_CAPTURE_DIR") {
                let dir = std::path::PathBuf::from(dir);
                std::fs::create_dir_all(&dir).unwrap();
                let stem = format!("{width}x{height}-frame{phase}");
                std::fs::write(dir.join(format!("{stem}.txt")), text).unwrap();
                let cells = buffer
                    .content
                    .iter()
                    .map(|c| {
                        serde_json::json!({
                            "text":c.symbol(),"fg":format!("{:?}",c.fg),"bg":format!("{:?}",c.bg),
                            "modifier":format!("{:?}",c.modifier)
                        })
                    })
                    .collect::<Vec<_>>();
                std::fs::write(
                    dir.join(format!("{stem}.json")),
                    serde_json::to_vec(
                        &serde_json::json!({"width":width,"height":height,"cells":cells,
                            "backend":"Ratatui TestBackend fixture; not physical terminal"}),
                    )
                    .unwrap(),
                )
                .unwrap();
            }
        }
    }
    a.apply(Reply {
        preview: None,
        device: 0,
        generation: a.generation,
        op: Operation::TransferJobs,
        result: Ok(serde_json::json!({"jobs":[{"key":key,"status":"complete"}]})),
    });
    assert!(a.active_transfer_keys().is_empty());
    assert_eq!(a.notice_kind, NoticeKind::Success);
    for plain in [false, true] {
        let period = if plain { 4 } else { 10 };
        assert_eq!(transfer_spinner(0, plain), transfer_spinner(period, plain));
        assert_ne!(transfer_spinner(0, plain), transfer_spinner(1, plain));
    }
}

#[test]
fn transfer_locations_stay_independent_and_submit_real_spec() {
    let (mut a, rx) = queued_app();
    a.view = View::Files;
    let mut source = Browser::new(1, "/recordings".into());
    source.entries.push(Entry {
        name: "test.bin".into(),
        path: "/recordings/test.bin".into(),
        kind: "file".into(),
        size: 3,
        identity: None,
        hidden: false,
        rename_name: None,
    });
    a.browser = Some(source);
    a.execute(Action::Copy);
    assert!(a.dialog.is_none());
    assert_eq!(a.browser.as_ref().unwrap().path, "/recordings");
    a.execute(Action::Destination);
    a.dialog_selected = 0;
    a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(a.destination_active);
    assert_eq!(a.other_browser.as_ref().unwrap().device, 1);
    a.browser.as_mut().unwrap().path = "/receive".into();
    while rx.try_recv().is_ok() {}
    assert_eq!(a.conflict_policy(), "rename");
    a.execute(Action::Conflict);
    assert_eq!(a.conflict_policy(), "skip");
    a.execute(Action::Conflict);
    assert_eq!(a.conflict_policy(), "overwrite");
    a.execute(Action::Paste);
    let task = rx.try_recv().unwrap();
    assert_eq!(task.device, 0);
    let Operation::Transfer(spec) = task.op else {
        panic!("expected durable transfer")
    };
    assert_eq!(spec.source.id, "remote");
    assert_eq!(spec.destination.id, "local");
    assert_eq!(spec.source_path, "/recordings/test.bin");
    assert_eq!(spec.destination_path, "/receive");
    assert_eq!(spec.conflict, "overwrite");
    assert!(a.dialog.is_none() && a.transfer_drawer);
    a.dialog = None;
    a.focus = Focus::Workspace;
    a.cycle_focus(true);
    assert!(!a.destination_active);
    assert_eq!(a.browser.as_ref().unwrap().path, "/recordings");
    a.cycle_focus(false);
    assert!(a.destination_active);
    a.cycle_focus(false);
    assert!(a.focus == Focus::Devices);
}
#[test]
fn jobs_cancel_owner_and_retry_same_idempotency_spec() {
    let (mut a, rx) = queued_app();
    let spec = crate::model::TransferSpec {
        source_container: None,
        destination_container: None,
        cut: false,
        source_identity: None,
        source: a.devices[1].clone(),
        source_path: "/data/source".into(),
        destination: a.devices[0].clone(),
        destination_path: "/destination".into(),
        conflict: "skip".into(),
        key: "test-transfer".into(),
    };
    a.submitted.insert(spec.key.clone(), spec);
    a.jobs.insert(0,serde_json::json!({"jobs":[{"key":"test-transfer","status":"running","source_host":"peace@laptop","destination_host":"tester@workstation","route":"direct on laptop","bytes":2,"total":10}]}));
    a.dialog = Some(Dialog::Jobs);
    a.key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE));
    let task = rx.try_recv().unwrap();
    assert_eq!(task.device, 0);
    assert!(matches!(task.op,Operation::TransferCancel { key } if key=="test-transfer"));
    a.jobs.get_mut(&0).unwrap()["jobs"][0]["status"] = serde_json::json!("failed");
    a.key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE));
    let Operation::TransferRetry { key } = rx.try_recv().unwrap().op else {
        panic!("real retry operation required")
    };
    assert_eq!(key, "test-transfer");
}
#[test]
fn directional_panel_focus_obeys_geometry_and_input_ownership() {
    let (mut a, _rx) = queued_app();
    a.panels.borrow_mut().extend([
        (Focus::Devices, false, Rect::new(0, 0, 20, 8)),
        (Focus::Actions, false, Rect::new(0, 8, 20, 8)),
        (Focus::Workspace, false, Rect::new(20, 0, 60, 16)),
    ]);
    a.key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::CONTROL));
    assert!(a.focus == Focus::Devices);
    a.key(KeyEvent::new(KeyCode::Down, KeyModifiers::CONTROL));
    assert!(a.focus == Focus::Actions);
    a.key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::CONTROL));
    assert!(a.focus == Focus::Workspace);
    a.input = Some(Input::Search);
    a.key(KeyEvent::new(KeyCode::Left, KeyModifiers::CONTROL));
    assert!(a.focus == Focus::Workspace);
}
#[test]
fn internet_evidence_is_not_https_or_stale_readiness() {
    let value = serde_json::json!({"internet":{"state":"reachable","stale":false,"observed_at":transport::now()},"sharing":{"state":"unsupported"}});
    let text = network_summary(&value);
    assert!(text.contains("reachable (ICMP)"));
    assert!(text.contains("DNS / HTTPS"));
    let mut stale = value;
    stale["internet"]["observed_at"] = serde_json::json!(transport::now() - 120);
    assert!(network_summary(&stale).contains("not confirmed"));
}
#[test]
fn reverse_tab_cycles_focus_and_leaves_search() {
    let mut a = app();
    a.focus = Focus::Workspace;
    a.key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT));
    assert!(a.focus == Focus::Actions);
    a.key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT));
    assert!(a.focus == Focus::Devices);
    a.key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT));
    assert!(a.focus == Focus::Workspace);
    a.input = Some(Input::Search);
    a.text = "abc".into();
    a.key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT));
    assert!(a.input.is_none());
    assert!(a.focus == Focus::Actions);
}
#[test]
fn horizontal_folder_keys_enter_and_leave_without_previewing_files() {
    for (forward, backward) in [
        (KeyCode::Right, KeyCode::Left),
        (KeyCode::Char('l'), KeyCode::Char('h')),
    ] {
        let mut a = app();
        a.view = View::Files;
        a.focus = Focus::Workspace;
        a.browser = Some(Browser::new(0, "/fixture".into()));
        a.browser.as_mut().unwrap().entries = vec![Entry {
            name: "child".into(),
            path: "/fixture/child".into(),
            kind: "directory".into(),
            size: 0,
            identity: None,
            hidden: false,
            rename_name: None,
        }];
        a.key(KeyEvent::new(forward, KeyModifiers::NONE));
        assert_eq!(a.browser.as_ref().unwrap().path, "/fixture/child");
        a.browser.as_mut().unwrap().parent = Some("/fixture".into());
        a.key(KeyEvent::new(backward, KeyModifiers::NONE));
        assert_eq!(a.browser.as_ref().unwrap().path, "/fixture");
        a.browser.as_mut().unwrap().entries[0].kind = "file".into();
        let generation = a.generation;
        a.key(KeyEvent::new(forward, KeyModifiers::NONE));
        assert_eq!(a.generation, generation);
        assert!(a.browser.as_ref().unwrap().preview.is_none());
    }
}
#[test]
fn clipped_directory_find_has_a_distinct_noncolor_modifier() {
    let (mut a, _tasks) = file_app();
    a.focus = Focus::Workspace;
    let b = a.browser.as_mut().unwrap();
    b.search = "needle".into();
    b.entries.truncate(1);
    let matching = b.entries[0].clone();
    b.entries[0].name = "ordinary".into();
    b.entries[0].kind = "directory".into();
    let mut matching = matching;
    matching.name = format!("visible-{}-needle", "élong".repeat(30));
    matching.kind = "directory".into();
    b.entries.push(matching);
    b.selected = 0;
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    terminal.draw(|frame| render(frame, &a)).unwrap();
    let buffer = terminal.backend().buffer();
    let found = (0..24).any(|y| {
        (0..72).any(|x| {
            (0..8)
                .map(|i| buffer[(x + i, y)].symbol())
                .collect::<String>()
                == "visible-"
                && (0..8).all(|i| buffer[(x + i, y)].modifier.contains(Modifier::UNDERLINED))
        })
    });
    assert!(
        found,
        "clipped directory match must remain visibly distinct without color"
    );
}

#[test]
fn long_unicode_file_find_keeps_visible_match_positions() {
    let (mut a, _tasks) = file_app();
    a.focus = Focus::Workspace;
    let b = a.browser.as_mut().unwrap();
    b.search = "needle".into();
    b.entries.truncate(1);
    b.entries[0].name = format!("needle-{}-終.txt", "élong".repeat(30));
    b.selected = 0;
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    terminal.draw(|frame| render(frame, &a)).unwrap();
    let buffer = terminal.backend().buffer();
    let found = (0..24).any(|y| {
        (0..74).any(|x| {
            (0..6)
                .map(|i| buffer[(x + i, y)].symbol())
                .collect::<String>()
                == "needle"
                && (0..6).all(|i| buffer[(x + i, y)].modifier.contains(Modifier::UNDERLINED))
        })
    });
    assert!(
        found,
        "visible filename match lost its highlight after clipping"
    );
}

#[test]
fn file_find_highlights_without_filtering_and_wraps_between_files() {
    let (mut a, _tasks) = file_app();
    a.focus = Focus::Workspace;
    let b = a.browser.as_mut().unwrap();
    b.entries = ["alpha-alpha.txt", "other.txt", "a-long-pha.txt", ".alpha"]
        .iter()
        .enumerate()
        .map(|(i, name)| Entry {
            name: (*name).into(),
            path: format!("/fixture/{i}"),
            kind: "file".into(),
            size: 0,
            identity: None,
            hidden: name.starts_with('.'),
            rename_name: None,
        })
        .collect();
    b.marked.insert("/fixture/1".into());
    let before: Vec<_> = a.visible_entries().iter().map(|e| e.path.clone()).collect();
    a.key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE));
    for c in "alpha".chars() {
        a.key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
    }
    assert_eq!(
        a.visible_entries()
            .iter()
            .map(|e| e.path.clone())
            .collect::<Vec<_>>(),
        before
    );
    assert_eq!(file_search_matches(&a.visible_entries(), "alpha").len(), 2);
    let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
    terminal.draw(|frame| render(frame, &a)).unwrap();
    assert!(
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .filter(|cell| cell.modifier.contains(Modifier::UNDERLINED))
            .count()
            >= 10
    );
    a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(a.input.is_none());
    assert!(a.browser.as_ref().unwrap().preview.is_none());
    for (key, expected) in [('n', 2), ('n', 0), ('N', 2), ('N', 0)] {
        a.key(KeyEvent::new(KeyCode::Char(key), KeyModifiers::NONE));
        assert_eq!(a.browser.as_ref().unwrap().selected, expected);
        assert!(!a.creating);
    }
    a.browser.as_mut().unwrap().search = "missing".into();
    a.key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE));
    assert_eq!(a.visible_entries().len(), 3);
    assert_eq!(a.browser.as_ref().unwrap().selected, 0);
    assert!(a.browser.as_ref().unwrap().marked.contains("/fixture/1"));
    a.browser.as_mut().unwrap().filter = "other".into();
    assert_eq!(a.visible_entries().len(), 1);
    a.browser.as_mut().unwrap().filter.clear();
    a.browser.as_mut().unwrap().show_hidden = true;
    assert_eq!(
        file_search_matches(&a.visible_entries(), "missing").len(),
        0
    );
    a.browser.as_mut().unwrap().search = "alpha".into();
    assert_eq!(file_search_matches(&a.visible_entries(), "alpha").len(), 3);
}

#[test]
fn fuzzy_search_is_live_and_selection_survives_arrow_keys() {
    let mut a = app();
    a.view = View::Files;
    a.browser = Some(Browser::new(0, "/fixture".into()));
    a.browser.as_mut().unwrap().entries = vec![
        Entry {
            name: "recording-first.bin".into(),
            path: "/fixture/a".into(),
            kind: "file".into(),
            size: 0,
            identity: None,
            hidden: false,
            rename_name: None,
        },
        Entry {
            name: "recording-second.bin".into(),
            path: "/fixture/b".into(),
            kind: "file".into(),
            size: 0,
            identity: None,
            hidden: false,
            rename_name: None,
        },
        Entry {
            name: "other".into(),
            path: "/fixture/c".into(),
            kind: "file".into(),
            size: 0,
            identity: None,
            hidden: false,
            rename_name: None,
        },
    ];
    a.key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE));
    for c in "rcb".chars() {
        a.key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
    }
    assert_eq!(a.visible_entries().len(), 3);
    assert_eq!(a.browser.as_ref().unwrap().search, "rcb");
    a.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    assert_eq!(a.browser.as_ref().unwrap().selected, 1);
    let mut t = Terminal::new(TestBackend::new(80, 24)).unwrap();
    t.draw(|f| render(f, &a)).unwrap();
    let bottom = t
        .backend()
        .buffer()
        .content
        .iter()
        .skip(80 * 18)
        .map(|c| c.symbol())
        .collect::<String>();
    assert!(bottom.contains("Search"));
    assert!(bottom.contains("rcb"));
}
#[test]
fn ctrl_c_exits_from_every_modal() {
    for input in [
        None,
        Some(Input::Search),
        Some(Input::Palette),
        Some(Input::Add),
        Some(Input::Mkdir),
    ] {
        let mut a = app();
        a.input = input;
        a.help = true;
        a.key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert!(a.quit);
    }
}
#[test]
fn printable_navigation_is_text_in_inputs() {
    let mut a = app();
    a.input = Some(Input::Search);
    for c in "hjkl?q/".chars() {
        a.key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
    }
    assert_eq!(a.text, "hjkl?q/");
    assert!(!a.quit);
    assert!(!a.help);
}
#[test]
fn enter_file_previews_never_launches() {
    let mut a = app();
    a.view = View::Files;
    a.browser = Some(Browser::new(0, "/tmp".into()));
    a.browser.as_mut().unwrap().entries = vec![Entry {
        name: "file".into(),
        path: "/tmp/file".into(),
        kind: "file".into(),
        size: 1,
        identity: None,
        hidden: false,
        rename_name: None,
    }];
    a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(a.pending_attach.is_none());
}
#[test]
fn hostile_controls_are_not_rendered() {
    assert_eq!(safe_text("\x1b]52;secret\x07"), "�]52;secret�");
}
#[test]
fn stale_directory_reply_cannot_retarget_browser() {
    let mut a = app();
    a.browser = Some(Browser::new(0, "/new".into()));
    a.generation = 2;
    a.apply(Reply {
        preview: None,
        device: 0,
        op: Operation::List {
            path: "/old".into(),
        },
        generation: 1,
        result: Ok(serde_json::json!({"path":"/old","entries":[]})),
    });
    assert_eq!(a.browser.unwrap().path, "/new");
}
#[test]
fn empty_session_action_respects_loading_errors_search_and_focus() {
    let (mut a, _rx) = queued_app();
    a.devices.truncate(1);
    a.work.truncate(1);
    a.device = 0;
    a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(matches!(
        a.dialog,
        Some(Dialog::Device(ChooseDevice::Shell))
    ));
    for state in ["loading", "error", "search", "creating"] {
        let mut a = app();
        match state {
            "loading" => a.work[0].loading = true,
            "error" => a.work[0].error = Some("offline".into()),
            "search" => a.search = "missing".into(),
            "creating" => a.creating = true,
            _ => unreachable!(),
        }
        a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(a.dialog.is_none(), "{state}");
    }
    let mut a = app();
    press(&mut a, 'a');
    assert!(a.input == Some(Input::Add));
    a.text.clear();
    press(&mut a, 'a');
    assert_eq!(a.text, "a");
    let (mut a, _rx) = queued_app();
    press(&mut a, 'a');
    assert!(a.input.is_none());
}
#[test]
fn selected_file_context_follows_focused_pane_and_keeps_destination() {
    let (mut a, _rx) = file_app();
    a.other_browser = Some(Browser::new(1, "/output".into()));
    for (switch, host) in [(false, "workstation"), (true, "laptop")] {
        if switch {
            a.switch_pane();
        }
        let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
        terminal.draw(|f| render(f, &a)).unwrap();
        let text = terminal
            .backend()
            .buffer()
            .content
            .chunks(120)
            .map(|row| row.iter().map(|c| c.symbol()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n");
        let sidebar = text
            .lines()
            .map(|line| line.chars().take(20).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(sidebar.contains(host), "{sidebar}");
        assert!(sidebar.contains("To laptop"), "{sidebar}");
    }
}
#[test]
fn project_headers_render_immediately_above_sessions_with_space_before_later_group() {
    let (mut a, _rx) = queued_app();
    a.device = 1;
    for (id, name, directory) in [
        ("a", "alpha-session", "/alpha"),
        ("b", "beta-session", "/beta"),
    ] {
        let mut session = disposable_shell();
        session.id = id.into();
        session.name = name.into();
        session.directory = directory.into();
        a.work[0].sessions.push(session);
    }
    a.work[0].fetched = transport::now();
    for width in [80, 120] {
        let rendered = capture_app(&a, width);
        let lines: Vec<_> = rendered.lines().collect();
        let first = lines
            .iter()
            .position(|line| line.contains("alpha-session"))
            .expect("first session");
        let second = lines
            .iter()
            .position(|line| line.contains("beta-session"))
            .expect("second session");
        assert!(lines[first - 1].contains("/alpha"), "{rendered}");
        assert!(lines[second - 1].contains("/beta"), "{rendered}");
        // Ignore the sidebar; the workspace column must be blank before its later header.
        let workspace_start = lines[second - 1][..lines[second - 1].find("/beta").unwrap()]
            .chars()
            .count();
        assert!(
            lines[second - 2]
                .chars()
                .skip(workspace_start)
                .all(|c| c.is_whitespace() || matches!(c, '│' | '┃' | '|')),
            "{rendered}"
        );
    }
}
#[test]
fn workspace_polish_capture_matrix() {
    for width in [48, 80, 120] {
        for height in [24, 40] {
            for state in [
                "first-run",
                "checking",
                "unavailable",
                "search",
                "sessions",
                "destination",
                "files",
                "files-inactive",
                "files-marked",
            ] {
                let (mut a, _rx) = file_app();
                if !matches!(
                    state,
                    "destination" | "files" | "files-inactive" | "files-marked"
                ) {
                    a.view = View::Work;
                    a.browser = None;
                    a.devices.truncate(1);
                    a.work.truncate(1);
                }
                match state {
                    "checking" => a.work[0].loading = true,
                    "unavailable" => a.work[0].error = Some("Device unreachable over SSH".into()),
                    "search" => a.search = "no-match".into(),
                    "sessions" => {
                        a.work[0].sessions.push(disposable_shell());
                        a.work[0].fetched = transport::now();
                    }
                    "destination" => a.other_browser = Some(Browser::new(1, "/output".into())),
                    "files" | "files-inactive" | "files-marked" => {
                        let b = a.browser.as_mut().unwrap();
                        b.entries[0].name = "recordings".into();
                        b.entries[0].kind = "directory".into();
                        b.selected = 1;
                        if state == "files-inactive" {
                            a.focus = Focus::Devices;
                        }
                        if state == "files-marked" {
                            b.marked.insert(b.entries[1].path.clone());
                        }
                    }
                    _ => {}
                }
                let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
                terminal.draw(|f| render(f, &a)).unwrap();
                let buffer = terminal.backend().buffer();
                let text = buffer
                    .content
                    .chunks(width as usize)
                    .map(|row| row.iter().map(|c| c.symbol()).collect::<String>())
                    .collect::<Vec<_>>()
                    .join("\n");
                assert!(
                    buffer.content.iter().all(|c| c.bg == Color::Reset),
                    "{state} {width}x{height}: opaque background"
                );
                if state == "sessions" {
                    assert!(text.contains("STATE"), "{width}x{height}: {text}");
                    assert!(text.contains("live"), "{width}x{height}: {text}");
                }
                if state == "first-run" {
                    assert!(text.contains("New shell"), "{width}x{height}: {text}");
                    assert!(
                        text.contains("Add by SSH address"),
                        "{width}x{height}: {text}"
                    );
                }
                if let Some(directory) = std::env::var_os("CX_WORKSPACE_CAPTURE_DIR") {
                    let directory = std::path::PathBuf::from(directory);
                    std::fs::create_dir_all(&directory).unwrap();
                    let file = format!("{width}x{height}-{state}");
                    std::fs::write(directory.join(format!("{file}.txt")), text).unwrap();
                    let cells = buffer
                        .content
                        .iter()
                        .map(|c| {
                            serde_json::json!({
                                "text": c.symbol(), "fg": format!("{:?}", c.fg),
                                "bg": format!("{:?}", c.bg), "modifier": format!("{:?}", c.modifier)
                            })
                        })
                        .collect::<Vec<_>>();
                    std::fs::write(
                        directory.join(format!("{file}.json")),
                        serde_json::to_vec(&serde_json::json!({
                            "width":width, "height":height, "cells":cells,
                            "backend":"Ratatui TestBackend fixture; not a physical terminal"
                        }))
                        .unwrap(),
                    )
                    .unwrap();
                }
            }
        }
    }
}
#[test]
fn stacked_path_is_only_in_sidebar_and_keeps_current_folder_visible() {
    for (width, height) in [(48, 24), (80, 24), (120, 40)] {
        for split in [false, true] {
            let (mut a, _rx) = file_app();
            a.browser.as_mut().unwrap().display_path =
                "/home/tester/projects/robotics/recordings".into();
            if split {
                a.other_browser = Some(Browser::new(1, "/output".into()));
            }
            let mut t = Terminal::new(TestBackend::new(width, height)).unwrap();
            t.draw(|f| render(f, &a)).unwrap();
            let sidebar_width = if width < 60 { 14 } else { 21 };
            let left = t
                .backend()
                .buffer()
                .content
                .chunks(width as usize)
                .map(|row| {
                    row[..sidebar_width]
                        .iter()
                        .map(|c| c.symbol())
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
                .join("\n");
            let right = t
                .backend()
                .buffer()
                .content
                .chunks(width as usize)
                .map(|row| {
                    row[sidebar_width..]
                        .iter()
                        .map(|c| c.symbol())
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
                .join("\n");
            assert!(left.contains("recordings"), "{width}: {left}");
            assert!(!right.contains("recordings"), "{right}");
            assert!(right.contains("alpha.txt"), "file list obscured: {right}");
            if let Some(dir) = std::env::var_os("CX_HIERARCHY_CAPTURE_DIR") {
                let dir = std::path::PathBuf::from(dir);
                std::fs::create_dir_all(&dir).unwrap();
                let text = t
                    .backend()
                    .buffer()
                    .content
                    .chunks(width as usize)
                    .map(|row| row.iter().map(|c| c.symbol()).collect::<String>())
                    .collect::<Vec<_>>()
                    .join("\n");
                std::fs::write(
                    dir.join(format!(
                        "{width}x{height}-{}.txt",
                        if split { "split" } else { "single" }
                    )),
                    text,
                )
                .unwrap();
            }
        }
    }
}
#[test]
fn selected_context_is_a_bounded_folder_and_item_hierarchy() {
    let entry = Entry {
        name: "log.bag".into(),
        path: "/a/b/c/log.bag".into(),
        kind: "file".into(),
        size: 0,
        hidden: false,
        identity: None,
        rename_name: None,
    };
    for height in [3, 4, 6, 12] {
        let lines = file_context_hierarchy(
            "robot",
            "/home/roboboat/robotics/recordings",
            Some(&entry),
            22,
            height,
        );
        let text = lines.iter().map(ToString::to_string).collect::<Vec<_>>();
        assert!(text.len() <= height);
        assert!(text[0].contains("robot"));
        assert!(text[text.len() - 2].contains("recordings/"));
        assert!(text.last().unwrap().contains("log.bag"));
        assert!(text.iter().all(|line| Span::raw(line).width() <= 22));
    }
}
#[test]
fn terminal_sizes_render_without_panic() {
    for (w, h) in [(80, 24), (120, 40), (40, 15), (25, 8)] {
        let a = app();
        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
        t.draw(|f| render(f, &a)).unwrap();
        let content = t
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<String>();
        assert!(content.contains("cx"));
    }
}
#[test]
fn opaque_parent_and_browser_state_are_preserved() {
    let mut a = app();
    a.open_browser(0, "/recordings".into());
    let b = a.browser.as_mut().unwrap();
    b.selected = 7;
    b.search = "robot".into();
    b.parent = Some("cx-bytes:Lw==".into());
    a.open_browser(0, "/other".into());
    a.open_browser(0, "/recordings".into());
    assert_eq!(a.browser.as_ref().unwrap().selected, 7);
    assert_eq!(a.browser.as_ref().unwrap().search, "robot");
    a.parent_directory();
    assert_eq!(a.browser.as_ref().unwrap().path, "cx-bytes:Lw==");
}
#[test]
fn bounded_queue_never_blocks_or_leaves_checking() {
    let (tx, _rx) = mpsc::sync_channel(1);
    let mut a = app();
    a.tx = tx;
    assert!(a.send(0, Operation::Info));
    a.refresh_work();
    assert!(!a.work[0].loading);
    assert!(a.notice.contains("busy"));
}
#[test]
fn network_observation_uses_real_shape_and_honest_sharing() {
    let text = network_summary(
        &serde_json::json!({"interfaces":{"state":"observed","data":[{"ifname":"eth0","operstate":"UP"}]},"sharing":{"state":"unsupported","reason":"No privileges requested"}}),
    );
    assert!(text.contains("eth0 · UP"));
    assert!(text.contains("Sharing: unsupported"));
    assert!(text.contains("unknown"));
}
#[test]
fn labels_cannot_spoof_rows_or_direction() {
    assert_eq!(safe_label("name\n\t\u{202e}abc"), "name���abc");
}
#[test]
fn maximum_png_preview_is_visible() {
    use base64::Engine;
    let image = image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
        1280,
        960,
        image::Rgba([220, 30, 20, 255]),
    ));
    let mut png = std::io::Cursor::new(Vec::new());
    image.write_to(&mut png, image::ImageFormat::Png).unwrap();
    let value = serde_json::json!({"kind":"image", "title":"PNG",
            "image":{"width":1280,"height":960,"png":base64::engine::general_purpose::STANDARD.encode(png.into_inner())}});
    assert!(
        RichPreview::from_value(&value).raster.is_some(),
        "valid maximum-size preview disappeared"
    );
}
#[test]
fn preview_loading_does_not_search_placeholder() {
    let (mut a, _rx) = file_app();
    a.focus = Focus::Workspace;
    let b = a.browser.as_mut().unwrap();
    b.preview = Some("Loading preview…".into());
    b.preview_pending_page = Some(1);
    press(&mut a, '/');
    assert!(a.input.is_none());
    assert!(a.notice.contains("loading"));
    assert!(a.browser.as_ref().unwrap().preview_find.query.is_empty());
    a.browser.as_mut().unwrap().preview_pending_page = None;
    press(&mut a, '/');
    assert!(a.input == Some(Input::PreviewSearch));
}
#[test]
fn preview_search_owns_keys_and_preserves_browser_state() {
    let (mut a, rx) = file_app();
    a.focus = Focus::Workspace;
    let b = a.browser.as_mut().unwrap();
    b.search = "alpha".into();
    b.preview = Some("opening\nneedle needle\nlast n e e d l e".into());
    let _ = capture_app(&a, 80);
    press(&mut a, '/');
    for c in "needle".chars() {
        press(&mut a, c);
    }
    let b = a.browser.as_ref().unwrap();
    assert_eq!(b.preview_find.matches.len(), 3);
    assert_eq!(b.search, "alpha");
    assert!(capture_app(&a, 80).contains("Search"));
    a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(a.browser.as_ref().unwrap().preview.is_some());
    press(&mut a, 'n');
    assert_eq!(a.browser.as_ref().unwrap().preview_find.selected, 1);
    press(&mut a, 'n');
    press(&mut a, 'n');
    assert_eq!(a.browser.as_ref().unwrap().preview_find.selected, 0);
    press(&mut a, 'N');
    assert_eq!(a.browser.as_ref().unwrap().preview_find.selected, 2);
    assert!(
        rx.try_recv().is_err(),
        "preview n must not create a session or reopen a file"
    );
    a.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(a.browser.as_ref().unwrap().preview.is_some());
    assert!(a.browser.as_ref().unwrap().preview_find.query.is_empty());
    a.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(a.browser.as_ref().unwrap().preview.is_none());
    assert_eq!(a.browser.as_ref().unwrap().search, "alpha");
    assert_eq!(a.browser.as_ref().unwrap().selected, 0);
}
#[test]
fn preview_search_unicode_styles_and_missing_matches() {
    let mut lines = vec![Line::from(vec![
        Span::styled("東京 ", Style::default()),
        Span::styled("Café café", Style::default().fg(Color::Green)),
    ])];
    let matches = preview_matches(&lines, "CAFÉ");
    assert_eq!(
        matches,
        vec![(0, vec![3, 4, 5, 6]), (0, vec![8, 9, 10, 11])]
    );
    let find = PreviewFind {
        query: "CAFÉ".into(),
        matches,
        selected: 1,
        reveal: std::cell::Cell::new(false),
    };
    highlight_preview_matches(&mut lines, &find);
    assert!(lines[0]
        .spans
        .iter()
        .any(|span| span.style.add_modifier.contains(Modifier::UNDERLINED)));
    assert!(lines[0].spans.iter().all(|span| span.style.bg.is_none()));
    assert!(preview_matches(&lines, "unavailable").is_empty());
    assert!(preview_matches(&lines, "").is_empty());
    assert_eq!(
        lines[0]
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect::<String>(),
        "東京 Café café"
    );
}
#[test]
fn preview_g_gg_clamp_wrapping_and_resize() {
    for width in [48, 80, 120] {
        let (mut a, _) = file_app();
        a.focus = Focus::Workspace;
        a.browser.as_mut().unwrap().preview = Some(
            "FIRST WRAPPED LINE\n".to_owned() + &"東京 word ".repeat(300) + "\nWRAPPED LAST LINE",
        );
        let _ = capture_app(&a, width);
        press(&mut a, 'G');
        assert!(capture_app(&a, width).contains("WRAPPED LAST LINE"));
        press(&mut a, 'j');
        assert!(capture_app(&a, width).contains("WRAPPED LAST LINE"));
        let _ = capture_app(&a, 120);
        assert!(capture_app(&a, 120).contains("WRAPPED LAST LINE"));
        press(&mut a, 'g');
        press(&mut a, 'g');
        assert_eq!(a.browser.as_ref().unwrap().preview_scroll.get(), 0);
        assert!(capture_app(&a, width).contains("FIRST WRAPPED LINE"));
    }
}
#[test]
fn preview_search_survives_refresh_and_reveals_on_resize() {
    let (mut a, _) = file_app();
    a.focus = Focus::Workspace;
    let b = a.browser.as_mut().unwrap();
    b.preview_path = Some("/files/alpha.txt".into());
    b.preview = Some("word ".repeat(350) + "needle");
    let _ = capture_app(&a, 120);
    press(&mut a, '/');
    for c in "needle".chars() {
        press(&mut a, c);
    }
    assert!(capture_app(&a, 120).contains("needle"));
    assert!(capture_app(&a, 48).contains("needle"));
    a.browser.as_mut().unwrap().preview_pending_page = Some(1);
    a.apply(Reply {
        device: 0,
        generation: a.generation,
        op: Operation::Preview {
            path: "/files/alpha.txt".into(),
        },
        preview: None,
        result: Ok(serde_json::json!({"kind":"text","text":"new needle contents"})),
    });
    assert!(a.input == Some(Input::PreviewSearch));
    assert_eq!(a.browser.as_ref().unwrap().preview_find.query, "needle");
    assert_eq!(a.browser.as_ref().unwrap().preview_find.matches.len(), 1);
    assert!(capture_app(&a, 80).contains("new needle contents"));
}
#[test]
fn preview_g_keeps_last_lines_visible() {
    let (mut a, _) = file_app();
    a.focus = Focus::Workspace;
    a.browser.as_mut().unwrap().preview = Some(
        (0..100)
            .map(|i| format!("line {i:03}\n"))
            .collect::<String>()
            + "FINAL PREVIEW LINE",
    );
    assert!(capture_app(&a, 80).contains("line 000"));
    press(&mut a, 'G');
    assert!(
        capture_app(&a, 80).contains("FINAL PREVIEW LINE"),
        "G must reach visible content rather than a blank preview"
    );
    assert_eq!(a.browser.as_ref().unwrap().selected, 0);
}
#[test]
fn pdf_pages_coalesce_keys_and_preserve_last_page_on_failure() {
    let (mut a, rx) = file_app();
    let b = a.browser.as_mut().unwrap();
    b.preview_path = Some("/files/book.pdf".into());
    b.preview = Some("".into());
    b.preview_rich = Some(RichPreview::from_value(
        &serde_json::json!({"kind":"pdf","page":1,"pages":3,"image":{"width":1,"height":1,"rgba":"/////w=="}}),
    ));
    a.key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE));
    assert!(matches!(
        rx.try_recv().unwrap().op,
        Operation::PreviewPage { page: 2, .. }
    ));
    press(&mut a, 'j');
    press(&mut a, 'j');
    assert!(
        rx.try_recv().is_err(),
        "only one page render may be in flight"
    );
    assert_eq!(a.browser.as_ref().unwrap().preview_requested_page, 3);
    a.apply(Reply {device:0,generation:a.generation,op:Operation::PreviewPage {path:"/files/book.pdf".into(),page:2},preview:None,result:Ok(serde_json::json!({"kind":"pdf","page":2,"pages":3,"text":"","image":{"width":1,"height":1,"rgba":"/////w=="}}))});
    assert!(matches!(
        rx.try_recv().unwrap().op,
        Operation::PreviewPage { page: 3, .. }
    ));
    assert_eq!(
        a.browser
            .as_ref()
            .unwrap()
            .preview_rich
            .as_ref()
            .unwrap()
            .page,
        1
    );
    a.apply(Reply {
        device: 0,
        generation: a.generation,
        op: Operation::PreviewPage {
            path: "/files/book.pdf".into(),
            page: 3,
        },
        preview: None,
        result: Err(anyhow::anyhow!("renderer unavailable")),
    });
    assert_eq!(
        a.browser
            .as_ref()
            .unwrap()
            .preview_rich
            .as_ref()
            .unwrap()
            .page,
        1
    );
    assert!(a.browser.as_ref().unwrap().preview_pending_page.is_none());
    let generation = a.generation;
    a.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    a.apply(Reply {
        device: 0,
        generation,
        op: Operation::PreviewPage {
            path: "/files/book.pdf".into(),
            page: 3,
        },
        preview: None,
        result: Ok(serde_json::json!({"kind":"pdf","page":3,"pages":3})),
    });
    assert!(a.browser.as_ref().unwrap().preview.is_none());
}
#[test]
fn key_release_does_not_advance_selection_twice() {
    let (mut a, _rx) = file_app();
    a.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    assert_eq!(a.browser.as_ref().unwrap().selected, 1);
    a.key(KeyEvent::new_with_kind(
        KeyCode::Down,
        KeyModifiers::NONE,
        event::KeyEventKind::Release,
    ));
    assert_eq!(a.browser.as_ref().unwrap().selected, 1);
    a.key(KeyEvent::new_with_kind(
        KeyCode::Down,
        KeyModifiers::NONE,
        event::KeyEventKind::Repeat,
    ));
    assert_eq!(a.browser.as_ref().unwrap().selected, 2);
}

#[test]
fn list_mouse_navigation_uses_panel_and_modal_ownership() {
    let (mut a, _rx) = file_app();
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    terminal.draw(|f| render(f, &a)).unwrap();
    let wheel = MouseEvent {
        kind: MouseEventKind::ScrollDown,
        column: 40,
        row: 8,
        modifiers: KeyModifiers::NONE,
    };
    assert!(a.mouse(wheel, Rect::new(0, 0, 80, 24)));
    assert_eq!(a.browser.as_ref().unwrap().selected, 1);
    assert!(!a.mouse(MouseEvent { row: 0, ..wheel }, Rect::new(0, 0, 80, 24)));
    assert_eq!(a.browser.as_ref().unwrap().selected, 1);
    assert!(!a.mouse(
        MouseEvent {
            modifiers: KeyModifiers::CONTROL,
            ..wheel
        },
        Rect::new(0, 0, 80, 24)
    ));
    a.help = true;
    assert!(a.mouse(wheel, Rect::new(0, 0, 80, 24)));
    assert_eq!(a.help_scroll, 1);
    assert_eq!(a.browser.as_ref().unwrap().selected, 1);
    a.help = false;
    a.input = Some(Input::Rename);
    a.text = "name".into();
    assert!(a.mouse(wheel, Rect::new(0, 0, 80, 24)));
    assert_eq!(a.text, "name");
    assert_eq!(a.browser.as_ref().unwrap().selected, 1);
}

#[test]
fn wheel_bursts_keep_direction_position_modifiers_and_keyboard_distinct() {
    let mut reports = WheelReports::default();
    let start = Instant::now();
    let down = MouseEvent {
        kind: MouseEventKind::ScrollDown,
        column: 40,
        row: 8,
        modifiers: KeyModifiers::NONE,
    };
    assert!(reports.accepts(&Event::Mouse(down), start));
    assert!(!reports.accepts(&Event::Mouse(down), start + Duration::from_micros(20)));
    let up = MouseEvent {
        kind: MouseEventKind::ScrollUp,
        ..down
    };
    assert!(reports.accepts(&Event::Mouse(up), start + Duration::from_micros(30)));
    let other = MouseEvent { row: 9, ..up };
    assert!(reports.accepts(&Event::Mouse(other), start + Duration::from_micros(40)));
    let modified = MouseEvent {
        modifiers: KeyModifiers::SHIFT,
        ..other
    };
    assert!(reports.accepts(&Event::Mouse(modified), start + Duration::from_micros(50)));
    let key = Event::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    assert!(reports.accepts(&key, start + Duration::from_micros(60)));
    assert!(reports.accepts(&Event::Mouse(modified), start + Duration::from_micros(70)));
    assert!(reports.accepts(
        &Event::Mouse(modified),
        start + WHEEL_BURST + Duration::from_micros(70)
    ));
    // The cutoff is anchored to the accepted tick, not extended by duplicates.
    assert!(!reports.accepts(
        &Event::Mouse(modified),
        start + WHEEL_BURST + Duration::from_micros(80)
    ));
    assert!(reports.accepts(
        &Event::Mouse(modified),
        start + 2 * WHEEL_BURST + Duration::from_micros(70)
    ));
}

#[test]
fn physical_wheel_trace_selects_one_adjacent_row_per_tick_in_both_lists() {
    // User capture: ten physical down ticks produced sixteen reports.
    let milliseconds = [
        2259.40, 3031.42, 3031.44, 3675.38, 3675.39, 4445.36, 5156.38, 5156.40, 5878.41, 5878.42,
        6620.37, 7374.35, 7374.36, 7993.32, 7993.32, 8694.36,
    ];
    for view in [View::Files, View::Work] {
        let (mut a, _rx) = file_app();
        if view == View::Files {
            let template = a.browser.as_ref().unwrap().entries[0].clone();
            a.browser.as_mut().unwrap().entries = (0..24)
                .map(|i| {
                    let mut entry = template.clone();
                    entry.name = format!("file{i:02}");
                    entry.path = format!("/files/file{i:02}");
                    entry
                })
                .collect();
        } else {
            a.view = View::Work;
            a.work[0].sessions = (0..24)
                .map(|i| {
                    let mut session = disposable_shell();
                    session.id = format!("wheel-{i:02}");
                    session.name = session.id.clone();
                    session
                })
                .collect();
        }
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal.draw(|f| render(f, &a)).unwrap();
        let mut reports = WheelReports::default();
        let start = Instant::now();
        let wheel = MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 40,
            row: 8,
            modifiers: KeyModifiers::NONE,
        };
        for ms in milliseconds {
            if reports.accepts(
                &Event::Mouse(wheel),
                start + Duration::from_secs_f64(ms / 1000.0),
            ) {
                a.mouse(wheel, Rect::new(0, 0, 80, 24));
            }
        }
        let selected = if view == View::Files {
            a.browser.as_ref().unwrap().selected
        } else {
            a.selected
        };
        assert_eq!(selected, 10, "ten physical ticks must move ten rows");
        let up_ms = [
            9492.34, 10121.33, 10706.35, 10706.36, 11295.31, 11936.35, 11936.36, 12548.32,
            12548.33, 13222.01, 14015.34, 14015.36, 14672.30, 14672.32, 14715.27, 14715.29,
        ];
        for ms in up_ms {
            let up = MouseEvent {
                kind: MouseEventKind::ScrollUp,
                ..wheel
            };
            if reports.accepts(
                &Event::Mouse(up),
                start + Duration::from_secs_f64(ms / 1000.0),
            ) {
                a.mouse(up, Rect::new(0, 0, 80, 24));
            }
        }
        let selected = if view == View::Files {
            a.browser.as_ref().unwrap().selected
        } else {
            a.selected
        };
        assert_eq!(
            selected, 0,
            "ten physical up ticks, including 43 ms apart, must return to the first row"
        );
    }
}

#[test]
fn file_search_wheel_moves_adjacent_rows_without_jumping_matches() {
    let (mut a, _rx) = file_app();
    let b = a.browser.as_mut().unwrap();
    b.entries[2].name = "zalpha.txt".into();
    b.search = "alpha".into();
    a.input = Some(Input::Search);
    a.text = "alpha".into();
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    terminal.draw(|f| render(f, &a)).unwrap();
    let wheel = MouseEvent {
        kind: MouseEventKind::ScrollDown,
        column: 40,
        row: 8,
        modifiers: KeyModifiers::NONE,
    };
    assert!(a.mouse(wheel, Rect::new(0, 0, 80, 24)));
    assert_eq!(
        a.browser.as_ref().unwrap().selected,
        1,
        "one wheel tick must select beta, even between search matches"
    );
    assert_eq!(a.text, "alpha");
    assert!(a.input == Some(Input::Search));
    assert_eq!(a.browser.as_ref().unwrap().search, "alpha");
    assert!(a.mouse(
        MouseEvent {
            kind: MouseEventKind::ScrollUp,
            ..wheel
        },
        Rect::new(0, 0, 80, 24)
    ));
    assert_eq!(a.browser.as_ref().unwrap().selected, 0);
    a.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    assert_eq!(
        a.browser.as_ref().unwrap().selected,
        2,
        "search keyboard navigation still selects the next match"
    );
}

#[test]
fn confirmation_wheel_does_not_arm_delete_or_navigate_files() {
    let (mut a, rx) = file_app();
    a.execute(Action::Delete);
    assert_eq!(a.dialog_selected, 0);
    assert!(a.mouse(
        MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 40,
            row: 10,
            modifiers: KeyModifiers::NONE
        },
        Rect::new(0, 0, 80, 24)
    ));
    assert_eq!(a.dialog_selected, 0);
    assert_eq!(a.browser.as_ref().unwrap().selected, 0);
    a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(a.dialog.is_none());
    assert!(
        rx.try_recv().is_err(),
        "wheel plus Enter must retain default Cancel"
    );
}

#[test]
fn fullscreen_pdf_from_two_locations_has_mouse_target() {
    let (mut a, rx) = file_app();
    a.other_browser = Some(Browser::new(1, "/destination".into()));
    let b = a.browser.as_mut().unwrap();
    b.preview = Some("".into());
    b.preview_rich = Some(RichPreview::from_value(
        &serde_json::json!({"kind":"pdf","page":1,"pages":3,"image":{"width":1,"height":1,"rgba":"/////w=="}}),
    ));
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    terminal.draw(|f| render(f, &a)).unwrap();
    assert!(a.mouse(
        MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 40,
            row: 10,
            modifiers: KeyModifiers::NONE
        },
        Rect::new(0, 0, 80, 24)
    ));
    assert!(matches!(
        rx.try_recv().unwrap().op,
        Operation::PreviewPage { page: 2, .. }
    ));
}

#[test]
fn preview_click_focus_and_wheel_keep_file_selection() {
    let (mut a, _rx) = file_app();
    a.browser.as_mut().unwrap().preview = Some((0..100).map(|i| format!("line {i}\n")).collect());
    capture_app(&a, 80);
    let selected = a.browser.as_ref().unwrap().selected;
    a.focus = Focus::Devices;
    let mut mouse = MouseEvent {
        kind: MouseEventKind::Down(event::MouseButton::Left),
        column: 40,
        row: 10,
        modifiers: KeyModifiers::NONE,
    };
    assert!(a.mouse(mouse, Rect::new(0, 0, 80, 24)));
    assert!(a.focus == Focus::Workspace);
    mouse.kind = MouseEventKind::ScrollDown;
    assert!(a.mouse(mouse, Rect::new(0, 0, 80, 24)));
    assert_eq!(a.browser.as_ref().unwrap().preview_scroll.get(), 1);
    assert_eq!(a.browser.as_ref().unwrap().selected, selected);
    a.key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE));
    assert!(a.browser.as_ref().unwrap().preview_scroll.get() > 1);
}

#[test]
fn pending_pdf_keeps_full_workspace_panel() {
    let (mut a, _rx) = file_app();
    a.other_browser = Some(Browser::new(1, "/destination".into()));
    let b = a.browser.as_mut().unwrap();
    b.preview = Some("PDF".into());
    b.preview_rich = Some(RichPreview::from_value(
        &serde_json::json!({"kind":"pdf","page":1,"pages":3,"image":{"width":1,"height":1,"rgba":"/////w=="}}),
    ));
    let before = capture_app(&a, 80);
    a.browser.as_mut().unwrap().preview_pending_page = Some(2);
    let pending = capture_app(&a, 80);
    assert!(!before.contains("NORMAL"));
    assert!(!pending.contains("NORMAL"), "{pending}");
    assert!(pending.contains("page 2"));
}

#[test]
fn pending_pdf_page_down_remains_single_page() {
    let (mut a, rx) = file_app();
    let b = a.browser.as_mut().unwrap();
    b.preview = Some("PDF".into());
    b.preview_path = Some("/book.pdf".into());
    b.preview_requested_page = 2;
    b.preview_pending_page = Some(2);
    b.preview_rich = Some(RichPreview::from_value(
        &serde_json::json!({"kind":"pdf","page":1,"pages":12,"image":{"width":1,"height":1,"rgba":"/////w=="}}),
    ));
    a.key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE));
    assert_eq!(a.browser.as_ref().unwrap().preview_requested_page, 3);
    assert!(rx.try_recv().is_err());
}

#[test]
fn pdf_mouse_wheel_is_scoped_to_preview() {
    let (mut a, rx) = file_app();
    let b = a.browser.as_mut().unwrap();
    b.preview = Some("".into());
    b.preview_rich = Some(RichPreview::from_value(
        &serde_json::json!({"kind":"pdf","page":1,"pages":3,"image":{"width":1,"height":1,"rgba":"/////w=="}}),
    ));
    let mut event = MouseEvent {
        kind: MouseEventKind::ScrollDown,
        column: 1,
        row: 3,
        modifiers: KeyModifiers::NONE,
    };
    assert!(!a.preview_mouse(event, Rect::new(0, 0, 80, 24)));
    assert!(rx.try_recv().is_err());
    event.column = 40;
    event.row = 10;
    assert!(a.preview_mouse(event, Rect::new(0, 0, 80, 24)));
    assert!(matches!(
        rx.try_recv().unwrap().op,
        Operation::PreviewPage { page: 2, .. }
    ));
    a.help = true;
    assert!(!a.preview_mouse(event, Rect::new(0, 0, 80, 24)));
}
#[test]
fn host_update_reloads_metadata_and_preview_without_replaying_work() {
    let (mut a, rx) = file_app();
    let mut b = Browser::new(1, "/remote".into());
    b.preview = Some("".into());
    b.preview_path = Some("/remote/book.pdf".into());
    b.preview_requested_page = 2;
    b.preview_rich = Some(RichPreview::from_value(
        &serde_json::json!({"kind":"pdf","page":2,"pages":3}),
    ));
    a.browser = Some(b);
    a.host_updated(transport::HostUpdate {
        target: "laptop".into(),
        version: "0.1.10".into(),
    });
    let tasks = rx.try_iter().collect::<Vec<_>>();
    assert!(tasks.iter().all(|t| t.device == 1));
    assert!(tasks.iter().any(|t| matches!(t.op, Operation::Info)));
    assert!(tasks
        .iter()
        .any(|t| matches!(t.op, Operation::PreviewPage { page: 2, .. })));
    assert!(tasks.iter().all(|t| matches!(
        t.op,
        Operation::Info
            | Operation::Sessions
            | Operation::TransferJobs
            | Operation::PreviewPage { .. }
    )));
    assert_eq!(a.browser.as_ref().unwrap().path, "/remote");
    assert!(a.pending_attach.is_none() && !a.creating);
}
#[test]
fn old_helper_page_check_does_not_loop() {
    let (mut a, rx) = file_app();
    a.browser = Some(Browser::new(1, "/remote".into()));
    let b = a.browser.as_mut().unwrap();
    b.preview = Some("".into());
    b.preview_requested_page = 2;
    b.preview_rich = Some(RichPreview::from_value(
        &serde_json::json!({"kind":"pdf","page":1}),
    ));
    a.apply(Reply {
        device: 1,
        generation: a.generation,
        op: Operation::Info,
        preview: None,
        result: Ok(serde_json::json!({"version":"0.1.9","capabilities":["codex"]})),
    });
    assert!(
        rx.try_recv().is_err(),
        "unsupported page capability must not create an Info retry loop"
    );
}
#[test]
fn pdf_scroll_requests_the_next_page() {
    let (mut a, rx) = file_app();
    let b = a.browser.as_mut().unwrap();
    b.preview = Some("PDF".into());
    b.preview_rich = Some(RichPreview::from_value(
        &serde_json::json!({"kind":"pdf","title":"PDF · page 1","page":1,"pages":3}),
    ));
    a.key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));
    assert!(
        rx.try_iter()
            .any(|t| matches!(t.op, Operation::PreviewPage { page: 2, .. })),
        "PDF scrolling never requests another page"
    );
}
#[test]
fn rich_preview_rejects_bad_images_and_keeps_transparent_pixels_default() {
    use base64::Engine;
    let valid = serde_json::json!({"kind":"image","title":"PNG 2x2", "image":{"width":2,"height":2,"rgba":base64::engine::general_purpose::STANDARD.encode([255u8,0,0,255,0,0,0,0,0,255,0,255,0,0,255,255])}});
    let rich = RichPreview::from_value(&valid);
    let (w, h, bytes) = rich.raster.unwrap();
    let lines = raster_lines(w, h, &bytes, Rect::new(0, 0, 2, 1));
    assert_eq!(lines[0].spans[0].style.fg, Some(Color::Rgb(255, 0, 0)));
    assert_eq!(lines[0].spans[0].style.bg, Some(Color::Rgb(0, 255, 0)));
    assert_eq!(lines[0].spans[1].content, "▄");
    assert_eq!(lines[0].spans[1].style.bg, None);
    for image in [
        serde_json::json!({"width":0,"height":1,"rgba":""}),
        serde_json::json!({"width":161,"height":1,"rgba":""}),
        serde_json::json!({"width":1,"height":1,"rgba":"bad"}),
        serde_json::json!({"width":1,"height":1,"rgba":"AA=="}),
    ] {
        assert!(RichPreview::from_value(&serde_json::json!({"image":image}))
            .raster
            .is_none());
    }
    assert!(raster_lines(w, h, &bytes, Rect::new(0, 0, 0, 0)).is_empty());
}
#[test]
fn markdown_table_search_clamps_stored_selection_after_resize() {
    let (mut a, _tasks) = file_app();
    let source = "| Key | Note |\n| --- | --- |\n| Key | A |\n| Key | B |";
    let b = a.browser.as_mut().unwrap();
    b.preview = Some(source.into());
    b.preview_rich = Some(RichPreview::from_value(
        &serde_json::json!({"kind":"markdown", "text":source}),
    ));
    b.preview_viewport.set((8, 20));
    b.preview_find.query = "Key".into();
    b.preview_find.matches = preview_matches(&preview_display_lines(b, source), "Key");
    assert_eq!(b.preview_find.matches.len(), 5);
    b.preview_find.selected = 4;
    b.preview_viewport.set((80, 20));
    a.next_preview_match(1);
    assert_eq!(a.browser.as_ref().unwrap().preview_find.matches.len(), 3);
    assert_eq!(a.browser.as_ref().unwrap().preview_find.selected, 0);
    a.browser.as_mut().unwrap().preview_find.selected = 4;
    a.next_preview_match(-1);
    assert_eq!(a.browser.as_ref().unwrap().preview_find.selected, 1);
}

#[test]
fn table_body_stops_before_a_pipe_in_a_code_fence() {
    let (mut a, _tasks) = file_app();
    let source = "| A | B |\n| --- | --- |\n```python | extra\n| literal | value |\n```";
    let b = a.browser.as_mut().unwrap();
    b.preview = Some(source.into());
    b.preview_rich = Some(RichPreview::from_value(
        &serde_json::json!({"kind":"markdown", "text":source}),
    ));
    b.preview_viewport.set((80, 20));
    let lines = preview_display_lines(b, source);
    let opening = lines[2]
        .spans
        .iter()
        .map(|s| s.content.as_ref())
        .collect::<String>();
    let code = lines[3]
        .spans
        .iter()
        .map(|s| s.content.as_ref())
        .collect::<String>();
    assert!(opening.contains("python") && !opening.contains("```"));
    assert!(code.contains("| literal | value |"));
}

#[test]
fn markdown_table_cells_keep_inline_emphasis_and_width() {
    let (mut a, _tasks) = file_app();
    let source = "| Name | Note |\n| --- | --- |\n| _italic_ | `a|b` |";
    let b = a.browser.as_mut().unwrap();
    b.preview = Some(source.into());
    b.preview_rich = Some(RichPreview::from_value(
        &serde_json::json!({"kind":"markdown", "text":source}),
    ));
    b.preview_viewport.set((80, 20));
    let lines = preview_display_lines(b, source);
    assert!(lines[2]
        .spans
        .iter()
        .any(|s| s.content == "italic" && s.style.add_modifier.contains(Modifier::ITALIC)));
    let text = lines[2]
        .spans
        .iter()
        .map(|s| s.content.as_ref())
        .collect::<String>();
    assert!(text.contains("a|b") && !text.contains('`') && !text.contains('_'));
    b.preview_viewport.set((8, 20));
    let narrow = preview_display_lines(b, source);
    assert!(narrow[2]
        .spans
        .iter()
        .any(|s| s.content == "italic" && s.style.add_modifier.contains(Modifier::ITALIC)));
}

#[test]
fn markdown_tables_resize_and_code_blocks_remain_literal() {
    let (mut a, _tasks) = file_app();
    let source = "| Device | Count |\n| :--- | ---: |\n| workstation | 42 |\n\n~~~python\nprint(\"hello\")\n| not | a table |\n| --- | --- |\n~~~";
    let b = a.browser.as_mut().unwrap();
    b.preview = Some(source.into());
    b.preview_rich = Some(RichPreview::from_value(
        &serde_json::json!({"kind":"markdown", "text":source}),
    ));
    b.preview_viewport.set((80, 20));
    let wide = preview_display_lines(b, source);
    assert_eq!(wide.len(), source.lines().count());
    let rendered = wide
        .iter()
        .map(|l| {
            l.spans
                .iter()
                .map(|s| s.content.as_ref())
                .collect::<String>()
        })
        .collect::<Vec<_>>();
    assert!(!rendered[0].starts_with('|'));
    assert!(rendered[0].contains("Device") && rendered[2].contains("42"));
    assert!(rendered[4].contains("python") && !rendered[4].contains("~~~"));
    assert!(rendered[6].contains("| not | a table |"));
    b.preview_viewport.set((12, 20));
    let narrow = preview_display_lines(b, source);
    let row = narrow[2]
        .spans
        .iter()
        .map(|s| s.content.as_ref())
        .collect::<String>();
    assert!(row.contains("Device: workstation") && row.contains("Count: 42"));
    assert_eq!(narrow.len(), wide.len());
    b.preview_find.query = "42".into();
    b.preview_find.matches = preview_matches(&narrow, "42");
    b.preview_viewport.set((80, 20));
    a.next_preview_match(1);
    assert_eq!(a.browser.as_ref().unwrap().preview_find.matches.len(), 1);
    assert_eq!(a.browser.as_ref().unwrap().preview_find.matches[0].0, 2);
}

#[test]
fn markdown_underscore_emphasis_preserves_identifiers_and_code() {
    let lines = preview_lines("_italic words_ and __strong words__\nfile_name_here and `_literal_`\n\\_escaped_ and _ unmatched\n_élégant_", "markdown");
    assert!(lines[0]
        .spans
        .iter()
        .any(|s| s.content == "italic words" && s.style.add_modifier.contains(Modifier::ITALIC)));
    assert!(lines[0]
        .spans
        .iter()
        .any(|s| s.content == "strong words" && s.style.add_modifier.contains(Modifier::BOLD)));
    assert_eq!(
        lines[1]
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect::<String>(),
        "file_name_here and _literal_"
    );
    assert!(!lines[1]
        .spans
        .iter()
        .any(|s| s.style.add_modifier.contains(Modifier::ITALIC)));
    assert!(!lines[2]
        .spans
        .iter()
        .any(|s| s.style.add_modifier.contains(Modifier::ITALIC)));
    assert!(lines[3]
        .spans
        .iter()
        .any(|s| s.content == "élégant" && s.style.add_modifier.contains(Modifier::ITALIC)));
}

#[test]
fn markdown_preview_styles_without_controls_or_active_content() {
    let lines=preview_lines("# Heading\n- **strong** and `code`\n```rust\nfn main() {}\n```\n<img src='https://example.test'>\n\x1b[31m", "markdown");
    assert!(lines[0].spans[0]
        .style
        .add_modifier
        .contains(Modifier::BOLD));
    assert!(lines[1]
        .spans
        .iter()
        .any(|s| s.content == "strong" && s.style.add_modifier.contains(Modifier::BOLD)));
    let rendered = lines
        .iter()
        .flat_map(|l| l.spans.iter())
        .map(|s| s.content.as_ref())
        .collect::<String>();
    assert!(!rendered.contains('\x1b'));
    assert!(rendered.contains("https://example.test"));
}
#[test]
fn preview_reply_is_scoped_and_content_is_not_serialized() {
    let (mut a, _rx) = file_app();
    let reply = || serde_json::json!({"text":"# Safe","kind":"markdown","title":"Heading"});
    a.apply(Reply {
        preview: None,
        device: 1,
        op: Operation::Preview {
            path: "/remote".into(),
        },
        generation: a.generation,
        result: Ok(reply()),
    });
    assert!(a.browser.as_ref().unwrap().preview.is_none());
    a.apply(Reply {
        preview: None,
        device: 0,
        op: Operation::Preview {
            path: "/files/a".into(),
        },
        generation: a.generation,
        result: Ok(reply()),
    });
    assert!(a.browser.as_ref().unwrap().preview_rich.is_some());
    let encoded = serde_json::to_string(a.browser.as_ref().unwrap()).unwrap();
    assert!(!encoded.contains("Heading") && !encoded.contains("# Safe"));
    a.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(a.browser.as_ref().unwrap().preview_rich.is_none());
    a.apply(Reply {
        preview: None,
        device: 0,
        op: Operation::Preview {
            path: "/files/a".into(),
        },
        generation: a.generation - 1,
        result: Ok(reply()),
    });
    assert!(a.browser.as_ref().unwrap().preview.is_none());
}
fn disposable_shell() -> Session {
    serde_json::from_value(serde_json::json!({"id":"cx-disposable", "name":"Disposable shell", "directory":"/tmp/cx-disposable", "provider":"shell", "host":"workstation", "account":"tester", "pid":1234,"started":"100", "boot_id":"test-boot", "external":false,"socket":"cx"})).unwrap()
}
#[test]
fn stop_shell_confirmation_protects_agents_and_captures_runtime_identity() {
    let (mut a, rx) = queued_app();
    a.work[0].sessions = vec![disposable_shell()];
    a.providers
        .insert(0, (vec!["stop-session-v1".into()], transport::now()));
    press(&mut a, 'd');
    assert!(matches!(a.dialog, Some(Dialog::StopShell(..))) && a.dialog_selected == 0);
    a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(rx.try_recv().is_err());
    press(&mut a, 'd');
    a.work[0].sessions[0].pid = 9999;
    press(&mut a, 'y');
    assert!(
        matches!(rx.try_recv().unwrap().op,Operation::StopSession{id,pid:1234,started,boot_id} if id=="cx-disposable" && started=="100" && boot_id=="test-boot")
    );
    assert!(a.pending_attach.is_none());
    a.work[0].sessions[0].provider = "codex".into();
    press(&mut a, 'd');
    assert!(matches!(a.dialog, Some(Dialog::StopShell(..))));
    press(&mut a, 'n');
    a.work[0].sessions[0].provider = "shell".into();
    a.work[0].sessions[0].external = true;
    press(&mut a, 'd');
    assert!(a.dialog.is_none());
}
#[test]
fn preview_and_confirmation_capture_sizes() {
    use base64::Engine;
    for (width, height) in [(120, 40), (80, 24), (48, 24)] {
        let (mut a, _rx) = file_app();
        a.browser.as_mut().unwrap().preview =
            Some("# Recording\n- **Robot** capture\n```rust\nfn main() {}\n```".into());
        a.browser.as_mut().unwrap().preview_rich = Some(RichPreview::from_value(
            &serde_json::json!({"kind":"markdown","title":"README.md"}),
        ));
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        let write = |terminal: &Terminal<TestBackend>, name: &str| {
            if let Some(directory) = std::env::var_os("CX_PREVIEW_CAPTURE_DIR") {
                let directory = std::path::PathBuf::from(directory);
                std::fs::create_dir_all(&directory).unwrap();
                let buffer = terminal.backend().buffer();
                let text = buffer
                    .content
                    .chunks(usize::from(width))
                    .map(|row| row.iter().map(|c| c.symbol()).collect::<String>())
                    .collect::<Vec<_>>()
                    .join("\n");
                std::fs::write(directory.join(format!("{width}x{height}-{name}.txt")), text)
                    .unwrap();
                let cells=buffer.content.iter().map(|cell|serde_json::json!({"text":cell.symbol(),"fg":format!("{:?}",cell.fg),"bg":format!("{:?}",cell.bg),"modifier":format!("{:?}",cell.modifier)})).collect::<Vec<_>>();
                std::fs::write(directory.join(format!("{width}x{height}-{name}.json")),serde_json::to_vec(&serde_json::json!({"width":width,"height":height,"cells":cells,"backend":"Ratatui TestBackend fixture; not physical emulator"})).unwrap()).unwrap();
            }
        };
        terminal.draw(|f| render(f, &a)).unwrap();
        write(&terminal, "markdown");
        let rgba = (0..32 * 20)
            .flat_map(|i| {
                [
                    if i % 32 < 16 { 255 } else { 0 },
                    if i / 32 < 10 { 180 } else { 0 },
                    180,
                    if i % 5 == 0 { 0 } else { 255 },
                ]
            })
            .collect::<Vec<u8>>();
        a.browser.as_mut().unwrap().preview_rich = Some(RichPreview::from_value(
            &serde_json::json!({"kind":"image","title":"PNG 32x20","image":{"width":32,"height":20,"rgba":base64::engine::general_purpose::STANDARD.encode(rgba)}}),
        ));
        terminal.draw(|f| render(f, &a)).unwrap();
        write(&terminal, "image");
        a.browser.as_mut().unwrap().preview = None;
        a.dialog = Some(Dialog::Delete(
            0,
            vec![a.browser.as_ref().unwrap().entries[0].clone()],
        ));
        terminal.draw(|f| render(f, &a)).unwrap();
        write(&terminal, "delete");
        a.dialog = Some(Dialog::StopShell(0, disposable_shell()));
        terminal.draw(|f| render(f, &a)).unwrap();
        write(&terminal, "stop-shell");
    }
}
#[test]
fn boxed_confirmation_variant_captures() {
    for (width, height) in [(120, 40), (80, 24), (48, 24), (36, 24)] {
        for (variant, name) in [
            (ConfirmationLayout::Separate, "a-separate"),
            (ConfirmationLayout::Joined, "b-joined"),
            (ConfirmationLayout::Compact, "c-compact"),
        ] {
            for stop in [false, true] {
                for selected in [0, 1] {
                    let (mut a, _rx) = file_app();
                    a.dialog = Some(if stop {
                        Dialog::StopShell(0, disposable_shell())
                    } else {
                        Dialog::Delete(0, vec![a.browser.as_ref().unwrap().entries[0].clone()])
                    });
                    a.dialog_selected = selected;
                    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
                    terminal
                            .draw(|frame| {
                                render(frame, &a);
                                let detail = if stop {"Running work in this session will end.\ntester@workstation\nDisposable shell\n/tmp/cx-disposable"} else {"Deletion cannot be undone.\ntester@workstation\n• alpha.txt"};
                            let rect = confirmation_popup(frame.area(), detail);
                                let inner = Rect::new(
                                    rect.x + 2,
                                    rect.y + 1,
                                    rect.width.saturating_sub(4),
                                    rect.height.saturating_sub(2),
                                );
                                let rows = Layout::default()
                                    .direction(Direction::Vertical)
                                    .constraints([
                                        Constraint::Min(2),
                                        Constraint::Length(1),
                                        Constraint::Length(3),
                                        Constraint::Length(1),
                                    ])
                                    .split(inner);
                                confirmation_buttons(
                                    frame, rows[2], stop, selected, false, variant,
                                );
                            })
                            .unwrap();
                    let buffer = terminal.backend().buffer();
                    let text = buffer
                        .content
                        .chunks(usize::from(width))
                        .map(|row| row.iter().map(|c| c.symbol()).collect::<String>())
                        .collect::<Vec<_>>()
                        .join("\n");
                    assert!(text.contains(if stop {
                        "Running work"
                    } else {
                        "Deletion cannot be"
                    }));
                    assert!(text.contains("tester@workstation"));
                    assert!(
                        text.contains(if stop {
                            if width < 48 {
                                "Keep"
                            } else {
                                "Keep session"
                            }
                        } else {
                            "Cancel"
                        }) && text.contains(if stop { "Stop session" } else { "Delete" })
                    );
                    if let Some(directory) = std::env::var_os("CX_CONFIRMATION_CAPTURE_DIR") {
                        let directory = std::path::PathBuf::from(directory);
                        std::fs::create_dir_all(&directory).unwrap();
                        let file = format!(
                            "{width}x{height}-{name}-{}-{}",
                            if stop { "stop-shell" } else { "delete" },
                            if selected == 0 { "cancel" } else { "confirm" }
                        );
                        std::fs::write(directory.join(format!("{file}.txt")), text).unwrap();
                        let cells=buffer.content.iter().map(|cell|serde_json::json!({"text":cell.symbol(),"fg":format!("{:?}",cell.fg),"bg":format!("{:?}",cell.bg),"modifier":format!("{:?}",cell.modifier)})).collect::<Vec<_>>();
                        std::fs::write(directory.join(format!("{file}.json")),serde_json::to_vec(&serde_json::json!({"width":width,"height":height,"cells":cells,"backend":"TestBackend fixture; not physical emulator"})).unwrap()).unwrap();
                    }
                }
            }
        }
    }
}
#[test]
fn larger_image_preview_uses_workspace_without_changing_locations() {
    use base64::Engine;
    let (mut a, _rx) = file_app();
    a.other_browser = Some(Browser::new(1, "/other-device".into()));
    let browser = a.browser.as_mut().unwrap();
    browser.selected = 1;
    browser.preview = Some("Image preview".into());
    browser.preview_rich = Some(RichPreview::from_value(
        &serde_json::json!({"kind":"image","title":"Photo","image":{"width":2,"height":2,"rgba":base64::engine::general_purpose::STANDARD.encode([255u8;16])}}),
    ));
    let text = capture_app(&a, 80);
    assert!(
        text.contains("tester@workstation") && text.contains("Photo") && !text.contains("NORMAL")
    );
    assert_eq!(a.browser.as_ref().unwrap().selected, 1);
    assert_eq!(a.other_browser.as_ref().unwrap().path, "/other-device");
    a.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(a.browser.as_ref().unwrap().preview.is_none());
    assert_eq!(a.browser.as_ref().unwrap().selected, 1);
    assert_eq!(a.other_browser.as_ref().unwrap().device, 1);
    let lines = raster_lines(2, 2, &[255u8; 16], Rect::new(0, 0, 20, 10));
    assert_eq!(lines.len(), 10);
    assert_eq!(lines[0].spans.len(), 20);
}
#[test]
fn native_bitmap_geometry_tracks_workspace_and_suppresses_overlays() {
    use base64::Engine;
    let (mut a, _rx) = file_app();
    let browser = a.browser.as_mut().unwrap();
    browser.preview = Some("Image".into());
    browser.preview_rich = Some(RichPreview::from_value(
        &serde_json::json!({"kind":"image","title":"PNG","image":{"width":2,"height":2,"rgba":base64::engine::general_purpose::STANDARD.encode([255u8;16])}}),
    ));
    let screen = Rect::new(0, 0, 80, 24);
    assert_eq!(
        native_preview_area(&a, screen),
        Some(Rect::new(22, 3, 57, 16))
    );
    a.set_notice("Preview notification".into());
    assert!(native_preview_area(&a, screen).is_none());
    assert!(a.expire_notice(a.notice_deadline.unwrap()));
    a.transfer_drawer = true;
    assert_eq!(
        native_preview_area(&a, screen),
        Some(Rect::new(22, 3, 57, 12))
    );
    a.browser.as_mut().unwrap().search = "photo".into();
    assert_eq!(
        native_preview_area(&a, screen),
        Some(Rect::new(22, 3, 57, 9))
    );
    a.transfer_drawer = false;
    a.browser.as_mut().unwrap().search.clear();
    assert_eq!(
        native_preview_area(&a, Rect::new(0, 0, 48, 24)),
        Some(Rect::new(15, 3, 32, 16))
    );
    a.help = true;
    assert!(native_preview_area(&a, screen).is_none());
    a.help = false;
    for input in [Input::Palette, Input::Search, Input::Filter, Input::Rename] {
        a.input = Some(input);
        assert!(native_preview_area(&a, screen).is_none());
    }
    a.input = None;
    a.dialog = Some(Dialog::Jobs);
    assert!(native_preview_area(&a, screen).is_none());
    a.dialog = None;
    a.view = View::Work;
    assert!(native_preview_area(&a, screen).is_none());
    a.view = View::Files;
    assert!(native_preview_area(&a, Rect::new(0, 0, 35, 24)).is_none());
    assert!(native_preview_area(&a, Rect::new(0, 0, 80, 9)).is_none());
}
#[test]
fn accepted_preview_responses_change_native_identity_even_when_metadata_matches() {
    let (mut a, _rx) = file_app();
    for revision in [1, 2] {
        a.apply(Reply {
            device: 0,
            op: Operation::Preview {
                path: "/files/alpha.txt".into(),
            },
            generation: a.generation,
            preview: None,
            result: Ok(serde_json::json!({"kind":"text","title":"same","text":"same"})),
        });
        assert_eq!(a.browser.as_ref().unwrap().preview_revision, revision);
    }
    let encoded = serde_json::to_string(a.browser.as_ref().unwrap()).unwrap();
    assert!(!encoded.contains("preview_revision"));
    let before = a.browser.as_ref().unwrap().preview_revision;
    a.apply(Reply {
        device: 1,
        op: Operation::Preview {
            path: "/remote".into(),
        },
        generation: a.generation,
        preview: None,
        result: Ok(serde_json::json!({"text":"stale"})),
    });
    assert_eq!(a.browser.as_ref().unwrap().preview_revision, before);
}

#[test]
fn scoped_sessions_have_evidence_based_devcontainer_labels_at_all_widths() {
    let (mut a, _rx) = queued_app();
    let c = container_fixture('a', true);
    a.containers.insert(0, vec![c.clone()]);
    let mut session = disposable_shell();
    session.container = Some(c.scope());
    session.name = "roboboat_dev · shell".into();
    session.directory = c.folder.clone();
    a.work[0].sessions.push(session.clone());
    a.focus = Focus::Workspace;
    for width in [48, 80, 120] {
        let text = capture_app(&a, width);
        if let Some(dir) = std::env::var_os("CX_SESSION_LABEL_CAPTURE_DIR") {
            let dir = std::path::PathBuf::from(dir);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join(format!("{width}-sessions.txt")), &text).unwrap();
        }
        assert!(text.contains("devcontainer"), "{width}: {text}");
        assert!(!text.contains("[shell]"));
    }
    assert_eq!(session.provider, "shell");
    let mut ordinary = c.clone();
    ordinary.devcontainer = false;
    a.containers.insert(0, vec![ordinary]);
    assert_eq!(menus::session_label(&a, 0, &session), "Docker");
    a.containers.clear();
    assert_eq!(menus::session_label(&a, 0, &session), "container");
    a.containers.insert(0, vec![c]);
    session.container.as_mut().unwrap().engine = "different-engine".into();
    assert_eq!(menus::session_label(&a, 0, &session), "container");
    session.provider = "codex".into();
    assert_eq!(menus::session_label(&a, 0, &session), "codex");
}
#[test]
fn scoped_session_refresh_requests_inventory_once_for_labels_without_launching() {
    let (mut a, rx) = queued_app();
    let mut session = disposable_shell();
    session.container = Some(container_fixture('a', true).scope());
    for _ in 0..2 {
        a.apply(Reply {
            device: 0,
            op: Operation::Sessions,
            generation: a.generation,
            preview: None,
            result: Ok(serde_json::to_value(vec![session.clone()]).unwrap()),
        });
    }
    let tasks = rx.try_iter().collect::<Vec<_>>();
    assert_eq!(tasks.len(), 1);
    assert!(matches!(tasks[0].op, Operation::Containers));
    assert!(a.pending_attach.is_none() && !a.creating);
}

#[test]
fn notices_expire_and_repeated_messages_restart_the_timeout() {
    let (mut app, _rx) = file_app();
    app.set_notice("Transfer complete".into());
    let first = app.notice_deadline.unwrap();
    assert!(!app.expire_notice(first - Duration::from_millis(1)));
    // Re-emitting the same text must not inherit the old deadline.
    app.notice_deadline = Some(Instant::now() - Duration::from_secs(1));
    app.set_notice("Transfer complete".into());
    let renewed = app.notice_deadline.unwrap();
    assert!(!app.expire_notice(renewed - Duration::from_millis(1)));
    assert!(app.expire_notice(renewed));
    assert!(app.notice.is_empty());
    assert!(!app.expire_notice(renewed + Duration::from_secs(10)));
    app.set_notice(String::new());
    assert!(app.notice_deadline.is_none());
}

#[test]
fn floating_notifications_are_typed_bounded_and_restore_the_scene() {
    for width in [48, 80, 120] {
        for (kind, label, color) in [
            (NoticeKind::Info, "Info", Color::Rgb(80, 200, 210)),
            (NoticeKind::Success, "Success", Color::Rgb(110, 200, 120)),
            (NoticeKind::Warning, "Warning", Color::Rgb(220, 180, 70)),
            (NoticeKind::Error, "Error", Color::Rgb(220, 90, 90)),
        ] {
            let (mut app, _rx) = file_app();
            let baseline = capture_app(&app, width);
            app.set_notice_as(kind, "A notification with a clear type".into());
            let mut terminal = Terminal::new(TestBackend::new(width, 24)).unwrap();
            terminal.draw(|frame| render(frame, &app)).unwrap();
            let buffer = terminal.backend().buffer();
            let text = capture_app(&app, width);
            assert!(text.contains(label));
            assert!(text.contains("notification") && text.contains("type"));
            assert!(buffer.content.iter().all(|cell| cell.bg == Color::Reset));
            if std::env::var_os("NO_COLOR").is_none() {
                assert!(buffer.content.iter().any(|cell| cell.fg == color));
            }
            if let Some(dir) = std::env::var_os("CX_NOTIFICATION_CAPTURE_DIR") {
                let dir = std::path::PathBuf::from(dir);
                std::fs::create_dir_all(&dir).unwrap();
                std::fs::write(dir.join(format!("{width}-{label}.txt")), &text).unwrap();
                let cells = buffer.content.iter().map(|cell| serde_json::json!({
                    "text": cell.symbol(), "fg": format!("{:?}", cell.fg),
                    "bg": format!("{:?}", cell.bg), "modifier": format!("{:?}", cell.modifier)
                })).collect::<Vec<_>>();
                std::fs::write(
                    dir.join(format!("{width}-{label}.json")),
                    serde_json::to_vec(
                        &serde_json::json!({"width": width, "height": 24, "cells": cells}),
                    )
                    .unwrap(),
                )
                .unwrap();
            }
            assert!(app.expire_notice(app.notice_deadline.unwrap()));
            assert_eq!(capture_app(&app, width), baseline);
        }
    }
    for size in [(2, 1), (36, 10), (48, 24), (120, 40)] {
        let available = Rect::new(5, 3, size.0, size.1);
        let text = "測試 robot_error_no_spaces_".repeat(100);
        let rect = notifications::area(&text, available);
        if !rect.is_empty() {
            assert!(rect.x >= available.x && rect.y >= available.y);
            assert!(rect.right() <= available.right() && rect.bottom() <= available.bottom());
            assert!(rect.height <= 8 && rect.width <= 60);
        }
    }
}
#[test]
fn notification_types_follow_results_and_typed_inputs_keep_ownership() {
    assert_eq!(
        update_notice_kind(&Ok(crate::update::CheckOutcome::Offline)),
        NoticeKind::Warning
    );
    assert_eq!(
        update_notice_kind(&Ok(crate::update::CheckOutcome::Current)),
        NoticeKind::Success
    );
    assert_eq!(
        update_notice_kind(&Err(anyhow::anyhow!("fixture"))),
        NoticeKind::Error
    );
    let (mut app, _rx) = file_app();
    app.apply(Reply {
        device: 0,
        generation: app.generation,
        preview: None,
        op: Operation::List {
            path: "/files".into(),
        },
        result: Err(anyhow::anyhow!("fixture file request failed")),
    });
    assert_eq!(app.notice_kind, NoticeKind::Error);
    app.input = Some(Input::Search);
    app.text = "folder".into();
    app.set_notice_as(NoticeKind::Warning, "Still searching".into());
    press(&mut app, 'd');
    assert_eq!(app.text, "folderd");
    assert!(app.input == Some(Input::Search));
    assert!(capture_app(&app, 80).contains("Still searching"));
}

#[test]
fn notification_animation_eases_in_out_and_stops_polling_fast_when_settled() {
    let start = Instant::now();
    let end = start + Duration::from_secs(5);
    let samples = [0, 65, 130, 195, 260, 2500, 4740, 4805, 4870, 4935, 5000];
    let mut captures = Vec::new();
    let (mut app, _rx) = file_app();
    app.set_notice_as(NoticeKind::Success, "Transfer complete".into());
    app.notice_started = Some(start);
    app.notice_deadline = Some(end);
    let baseline = {
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        app.notice.clear();
        terminal
            .draw(|frame| rendering::render_at(frame, &app, None, start))
            .unwrap();
        let b = terminal.backend().buffer().clone();
        app.notice = "Transfer complete".into();
        b
    };
    for ms in samples {
        let now = start + Duration::from_millis(ms);
        let appearance = notifications::appearance(Some(start), Some(end), now);
        assert!((0.0..=1.0).contains(&appearance.opacity));
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal
            .draw(|frame| rendering::render_at(frame, &app, None, now))
            .unwrap();
        let b = terminal.backend().buffer();
        if ms == 0 || ms == 5000 {
            assert_eq!(b, &baseline);
        }
        captures.push(b.clone());
        if let Some(dir) = std::env::var_os("CX_ANIMATION_CAPTURE_DIR") {
            let dir = std::path::PathBuf::from(dir);
            std::fs::create_dir_all(&dir).unwrap();
            let cells = b.content.iter().map(|c| serde_json::json!({"text":c.symbol(),"fg":format!("{:?}",c.fg),"bg":format!("{:?}",c.bg)})).collect::<Vec<_>>();
            std::fs::write(
                dir.join(format!("{ms:04}.json")),
                serde_json::to_vec(&serde_json::json!({"width":80,"height":24,"cells":cells}))
                    .unwrap(),
            )
            .unwrap();
        }
    }
    assert_ne!(captures[1], captures[2]);
    assert_ne!(captures[8], captures[9]);
    assert_eq!(captures[4], captures[5]);
    assert_eq!(
        notifications::poll_interval(Some(start), Some(end), start + Duration::from_millis(100)),
        Duration::from_millis(33)
    );
    assert_eq!(
        notifications::poll_interval(Some(start), Some(end), start + Duration::from_secs(1)),
        Duration::from_millis(100)
    );
    assert!(!notifications::animating(Some(start), Some(end), end));
    // All phases also stay bounded in a narrow terminal.
    for ms in samples {
        let mut terminal = Terminal::new(TestBackend::new(48, 24)).unwrap();
        terminal
            .draw(|frame| {
                rendering::render_at(frame, &app, None, start + Duration::from_millis(ms))
            })
            .unwrap();
    }
}

#[test]
fn unreachable_notices_are_once_per_device_until_successful_contact() {
    let (mut app, _rx) = queued_app();
    let failure = |op, device, generation| Reply {
        op,
        device,
        generation,
        preview: None,
        result: Err(anyhow::Error::new(
            transport::ConnectionFailure::Unreachable,
        )),
    };
    app.apply(failure(Operation::Sessions, 1, app.generation));
    assert!(app.notice.contains("unreachable"));
    assert!(app.work[1].error.is_some());
    let end = app.notice_deadline.unwrap();
    app.apply(failure(Operation::Info, 1, app.generation));
    assert_eq!(app.notice_deadline, Some(end));
    assert!(app.expire_notice(end));
    for op in [
        Operation::Info,
        Operation::Sessions,
        Operation::Network,
        Operation::Containers,
    ] {
        app.apply(failure(op, 1, app.generation));
        assert!(app.notice.is_empty());
    }
    assert!(app.container_errors.contains_key(&1));
    app.apply(failure(Operation::Sessions, 0, app.generation));
    assert!(app.notice.contains("unreachable"));
    app.apply(Reply {
        op: Operation::List {
            path: "/files".into(),
        },
        device: 1,
        generation: app.generation,
        preview: None,
        result: Err(anyhow::anyhow!("Permission denied for this file operation")),
    });
    assert!(app.notice.contains("Permission denied"));
    assert!(app.unavailable_notified.contains(&app.devices[1].id));
    // A genuinely successful helper response re-arms future outages.
    app.apply(Reply {
        op: Operation::Sessions,
        device: 1,
        generation: app.generation,
        preview: None,
        result: Ok(serde_json::json!([])),
    });
    assert!(!app.unavailable_notified.contains(&app.devices[1].id));
    app.set_notice(String::new());
    app.apply(failure(Operation::Sessions, 1, app.generation));
    assert!(app.notice.contains("unreachable"));
}

#[test]
fn stale_errors_and_quiet_neighbor_checks_do_not_consume_the_first_device_notice() {
    let (mut app, _rx) = queued_app();
    app.generation = 2;
    for (op, generation) in [
        (
            Operation::List {
                path: "/stale".into(),
            },
            1,
        ),
        (
            Operation::ProbeCandidate {
                address: "192.0.2.1".into(),
                interface: None,
            },
            2,
        ),
    ] {
        app.apply(Reply {
            op,
            device: 1,
            generation,
            preview: None,
            result: Err(anyhow::Error::new(transport::ConnectionFailure::Timeout)),
        });
        assert!(app.notice.is_empty());
        assert!(app.unavailable_notified.is_empty());
    }
    app.apply(Reply {
        op: Operation::Info,
        device: 1,
        generation: 2,
        preview: None,
        result: Err(anyhow::Error::new(transport::ConnectionFailure::Timeout)),
    });
    assert!(app.notice.contains("timed out"));
    let deadline = app.notice_deadline;
    app.apply(Reply {
        op: Operation::Sessions,
        device: 1,
        generation: 1,
        preview: None,
        result: Ok(serde_json::json!([])),
    });
    assert!(app.unavailable_notified.contains(&app.devices[1].id));
    app.apply(Reply {
        op: Operation::Sessions,
        device: 1,
        generation: 2,
        preview: None,
        result: Err(anyhow::Error::new(
            transport::ConnectionFailure::Unreachable,
        )),
    });
    assert_eq!(app.notice_deadline, deadline);
}

#[test]
fn container_clipboard_survives_navigation_and_submits_both_scopes() {
    for cut in [false, true] {
        let (mut a, rx) = file_app();
        let source = container_fixture('a', true).scope();
        let destination = container_fixture('b', true).scope();
        a.browser.as_mut().unwrap().container = Some(source.clone());
        a.execute(if cut { Action::Cut } else { Action::Copy });
        assert_eq!(
            a.clipboard.as_ref().unwrap().container.as_ref(),
            Some(&source)
        );
        assert!(a
            .clipboard
            .as_ref()
            .unwrap()
            .source_label
            .contains("roboboat_dev"));
        a.open_scoped_browser(1, "/files".into(), Some(destination.clone()));
        let _ = rx.try_iter().collect::<Vec<_>>();
        a.browser.as_mut().unwrap().loading = false;
        a.destination_active = true;
        a.focus = Focus::Workspace;
        a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        let task = rx.try_recv().unwrap();
        let Operation::ScopedTransfer(spec) = task.op else {
            panic!("container paths must use the scoped protocol")
        };
        assert_eq!(spec.source_container, Some(source));
        assert_eq!(spec.destination_container, Some(destination));
        assert_eq!(spec.source_path, "/files/alpha.txt");
        assert_eq!(spec.destination_path, "/files");
        assert_eq!(spec.cut, cut);
        assert_eq!(spec.source_identity.is_some(), cut);
        assert_eq!(task.device, 0, "viewer owns relay submission");
        assert!(a.transfer_drawer);
    }
}
#[test]
fn destination_picker_separates_host_from_allowed_running_containers() {
    let (mut a, rx) = file_app();
    a.execute(Action::Copy);
    let a_container = container_fixture('a', true);
    let ordinary = container_fixture('b', false);
    let mut stopped = container_fixture('c', true);
    stopped.state = "exited".into();
    a.containers
        .insert(1, vec![stopped, ordinary, a_container.clone()]);
    a.chosen_device(1, ChooseDevice::Destination);
    assert!(matches!(a.dialog, Some(Dialog::DestinationScope(1, _))));
    assert_eq!(a.destination_containers(1).len(), 1);
    a.dialog_selected = 1;
    a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(a.destination_active && a.other_browser.is_some());
    assert_eq!(
        a.browser.as_ref().unwrap().container,
        Some(a_container.scope())
    );
    assert!(a.clipboard.is_some());
    assert!(rx
        .try_iter()
        .any(|t| matches!(t.op, Operation::ContainerFiles { .. })));
}
#[test]
fn scoped_mutation_failure_releases_queue_after_navigation_and_preserves_clipboard() {
    let (mut a, rx) = file_app();
    a.execute(Action::Copy);
    let scope = container_fixture('a', true).scope();
    a.browser.as_mut().unwrap().container = Some(scope.clone());
    a.queue_file_actions(vec![(
        0,
        Operation::Remove {
            path: "/files/alpha.txt".into(),
            expected_identity: Some("original".into()),
        },
    )]);
    let task = rx.try_recv().unwrap();
    a.open_host_browser(1, "/output".into());
    a.apply(Reply {
        device: task.device,
        generation: task.generation,
        op: task.op,
        preview: None,
        result: Err(anyhow::anyhow!("Permission denied inside container")),
    });
    assert!(!a.file_busy);
    assert!(a.notice.contains("Permission denied"));
    assert!(a.clipboard.is_some());
    assert!(a.browser.as_ref().unwrap().container.is_none());
}
#[test]
fn rebuild_is_devcontainer_only_explicit_and_refreshes_after_failure() {
    let (mut a, rx) = queued_app();
    let mut c = container_fixture('a', true);
    c.workspace = Some("/robot/project".into());
    c.config = Some("/robot/project/.devcontainer/devcontainer.json".into());
    assert!(container_action_labels(&c).contains(&"Rebuild"));
    assert!(!container_action_labels(&container_fixture('b', false)).contains(&"Rebuild"));
    a.dialog = Some(Dialog::ContainerConfirm(0, c.clone(), "Rebuild".into()));
    a.dialog_selected = 0;
    a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(rx.try_recv().is_err());
    a.dialog = Some(Dialog::ContainerConfirm(0, c, "Rebuild".into()));
    a.dialog_selected = 1;
    a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    let task = rx.try_recv().unwrap();
    assert!(matches!(task.op, Operation::DevcontainerRebuild { .. }));
    a.apply(Reply {
        device: task.device,
        generation: task.generation,
        op: task.op,
        preview: None,
        result: Err(anyhow::anyhow!("CLI build failed")),
    });
    assert!(a.notice.contains("CLI build failed"));
    assert!(
        rx.try_iter()
            .all(|t| !matches!(t.op, Operation::DevcontainerRebuild { .. })),
        "failure must not replay rebuild"
    );
}
#[test]
fn container_transfer_and_rebuild_layouts_are_bounded_at_supported_widths() {
    for width in [48, 80, 120] {
        let (mut a, _) = file_app();
        let mut c = container_fixture('a', true);
        c.workspace = Some("/robot/project".into());
        c.config = Some("/robot/project/.devcontainer/devcontainer.json".into());
        a.containers.insert(0, vec![c.clone()]);
        for (name, dialog) in [
            ("destination", Dialog::DestinationScope(0, vec![c.clone()])),
            (
                "rebuild",
                Dialog::ContainerConfirm(0, c.clone(), "Rebuild".into()),
            ),
        ] {
            a.dialog = Some(dialog);
            let text = capture_app(&a, width);
            assert!(text
                .lines()
                .all(|line| Line::from(line).width() <= width as usize));
            assert!(text.contains(if name == "destination" {
                "Host files"
            } else {
                "Rebuild"
            }));
            if let Some(dir) = std::env::var_os("CX_CONTAINER_TRANSFER_CAPTURE_DIR") {
                let dir = std::path::PathBuf::from(dir);
                std::fs::create_dir_all(&dir).unwrap();
                std::fs::write(dir.join(format!("{name}-{width}.txt")), text).unwrap();
                let mut terminal = Terminal::new(TestBackend::new(width, 24)).unwrap();
                terminal.draw(|frame| render(frame, &a)).unwrap();
                let cells = terminal.backend().buffer().content.iter().map(|c| serde_json::json!({"text":c.symbol(),"fg":format!("{:?}",c.fg),"bg":format!("{:?}",c.bg)})).collect::<Vec<_>>();
                std::fs::write(
                    dir.join(format!("{name}-{width}.json")),
                    serde_json::to_vec(
                        &serde_json::json!({"width":width,"height":24,"cells":cells}),
                    )
                    .unwrap(),
                )
                .unwrap();
            }
        }
    }
}

#[test]
fn narrow_rebuild_details_scroll_without_changing_confirmation() {
    let (mut a, _) = queued_app();
    a.dialog = Some(Dialog::ContainerConfirm(
        0,
        container_fixture('a', true),
        "Rebuild".into(),
    ));
    let initial = capture_app(&a, 48);
    assert!(a.dialog_scroll_max.get() > 0);
    a.key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE));
    assert!(a.dialog_scroll > 0);
    assert_eq!(a.dialog_selected, 0);
    let scrolled = capture_app(&a, 48);
    assert_ne!(scrolled, initial);
}
