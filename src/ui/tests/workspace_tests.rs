use super::*;

#[test]
fn workspace_picker_shows_every_runtime_in_the_same_folder() {
    let (mut a, _) = queued_app();
    for (id, name) in [("first", "Fix parser"), ("second", "Review renderer")] {
        let mut s = disposable_shell();
        s.id = id.into();
        s.name = name.into();
        s.directory = "/project".into();
        s.provider = "claude".into();
        a.work[0].sessions.push(s);
    }
    a.start_at(0, "/project".into(), "claude".into());
    let screen = capture_app(&a, 120);
    assert!(
        screen.contains("Fix parser") && screen.contains("Review renderer"),
        "both actual sessions must be visible"
    );
}

#[test]
fn workspace_new_agent_does_not_ask_about_unknown_latest_history() {
    let (mut a, _) = queued_app();
    a.start_at(0, "/empty".into(), "claude".into());
    assert!(
        matches!(a.dialog, Some(Dialog::SessionPicker(..))),
        "agent picker offers explicit fresh and history choices"
    );
}

#[test]
fn workspace_duplicate_runtime_reply_is_presented_once() {
    let (mut a, _) = queued_app();
    let s = disposable_shell();
    a.work[0].sessions = vec![s.clone(), s];
    assert_eq!(
        a.session_rows().len(),
        1,
        "duplicate observation must not produce another row"
    );
}

fn pin(a: &mut App, d: usize, s: &Session) {
    a.pins.push(store::SessionPin {
        device: a.devices[d].clone(),
        session: s.clone(),
        pinned_at: 1,
    });
}
fn observed_reply(a: &mut App, task: Task, sessions: Vec<Session>) {
    a.apply(Reply {
        device_id: None,
        execution_target: None,
        preview: None,
        device: task.device,
        op: task.op,
        generation: task.generation,
        result: Ok(serde_json::to_value(sessions).unwrap()),
    });
}

#[test]
fn pins_promote_one_runtime_survive_exit_and_never_restart_it() {
    let (mut a, rx) = queued_app();
    let mut s = disposable_shell();
    s.directory = "/z-pinned".into();
    let mut other = s.clone();
    other.id = "other".into();
    other.directory = "/a-other".into();
    a.work[0].sessions = vec![other, s.clone()];
    a.work[0].fetched = transport::now();
    pin(&mut a, 0, &s);
    assert_eq!(a.session_rows().len(), 2);
    assert_eq!(a.session_rows()[0].1.id, s.id);
    a.work[0].sessions.clear();
    assert_eq!(a.session_rows().len(), 1);
    assert_eq!(a.session_status(0, &s), "not seen");
    a.open_session(0, s, false);
    assert!(matches!(a.dialog, Some(Dialog::PinnedEnded(..))));
    assert!(a.pending_attach.is_none() && rx.try_recv().is_err());
    press(&mut a, 'd');
    assert!(rx.try_recv().is_err());
}

#[test]
fn cached_attachment_rejects_reused_id_and_earlier_refresh() {
    let (mut a, rx) = queued_app();
    let s = disposable_shell();
    a.work[0].sessions.push(s.clone());
    a.refresh_sessions_on(0);
    let old = rx.try_recv().unwrap();
    a.open_session(0, s.clone(), false);
    let check = rx.try_recv().unwrap();
    assert!(a.pending_attach.is_none());
    observed_reply(&mut a, old, vec![s.clone()]);
    assert!(
        a.pending_attach.is_none(),
        "older queued observation cannot satisfy click-time verification"
    );
    let mut replacement = s.clone();
    replacement.started = "replacement".into();
    observed_reply(&mut a, check, vec![replacement]);
    assert!(a.pending_attach.is_none() && a.attach_check.is_none());
    assert!(a.notice.contains("ended or changed"));
}

#[test]
fn attachment_checks_real_runtime_before_watch() {
    let (mut a, rx) = queued_app();
    let s = disposable_shell();
    a.work[1].sessions.push(s.clone());
    a.open_session(1, s.clone(), true);
    assert!(a.pending_attach.is_none());
    observed_reply(&mut a, rx.try_recv().unwrap(), vec![s]);
    assert!(matches!(a.pending_attach, Some((1, _, true))));
}

#[test]
fn failed_pin_refresh_preserves_cached_work_without_attaching() {
    let (mut a, rx) = queued_app();
    let s = disposable_shell();
    pin(&mut a, 0, &s);
    a.work[0].sessions.push(s.clone());
    a.open_session(0, s.clone(), false);
    let task = rx.try_recv().unwrap();
    a.apply(Reply {
        device_id: None,
        execution_target: None,
        preview: None,
        device: task.device,
        op: task.op,
        generation: task.generation,
        result: Err(anyhow::anyhow!("fixture offline")),
    });
    assert!(a.pending_attach.is_none() && a.attach_check.is_none());
    assert_eq!(a.session_rows().len(), 1);
    assert_eq!(a.session_status(0, &s), "? stale");
    assert!(a.notice.contains("no attachment started"));
    let mut restored = queued_app().0;
    workspaces::NavigationCache::snapshot(&a).restore(&mut restored);
    assert_eq!(restored.work[0].sessions.len(), 1);
}

#[test]
fn removing_device_discards_indexed_cache_and_old_replies() {
    let (mut a, _) = queued_app();
    let mut third = a.devices[1].clone();
    third.id = "third".into();
    third.target = Some("third-fixture".into());
    a.devices.push(third);
    a.work.push(Cached {
        sessions: vec![],
        loading: false,
        error: None,
        fetched: 0,
    });
    a.policies.push(store::DevicePolicy::default());
    let b = Browser::new(1, "/only-on-removed-device".into());
    a.browser_cache
        .insert((1, b.path.clone(), String::new()), b);
    let old_generation = a.generation;
    let session = disposable_shell();
    a.forget_device_index(1);
    assert_eq!(a.devices[1].id, "third");
    assert!(a.browser_cache.is_empty() && a.clipboard.is_none());
    for old_index in [1, 2] {
        a.apply(Reply {
            device_id: None,
            execution_target: None,
            preview: None,
            device: old_index,
            op: Operation::Sessions,
            generation: old_generation,
            result: Ok(serde_json::to_value(vec![session.clone()]).unwrap()),
        });
    }
    assert!(a.work[1].sessions.is_empty());
    let snapshot = serde_json::to_string(&workspaces::NavigationCache::snapshot(&a)).unwrap();
    assert!(!snapshot.contains("only-on-removed-device"));
}

#[test]
fn unavailable_pin_refresh_targets_its_own_device_even_when_marked_down() {
    let (mut a, rx) = queued_app();
    a.device = 1;
    a.policies[1].availability = store::Availability::UsuallyDown;
    a.dialog = Some(Dialog::PinnedEnded(1, disposable_shell()));
    a.dialog_selected = 1;
    a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    let task = rx.try_recv().unwrap();
    assert_eq!(task.device, 1);
    assert!(matches!(task.op, Operation::Sessions));
    assert!(rx.try_recv().is_err());
}

#[test]
fn pending_attachment_is_cancelled_by_escape_and_navigation() {
    for key in [KeyCode::Esc, KeyCode::Char('j'), KeyCode::Char('/')] {
        let (mut a, rx) = queued_app();
        let s = disposable_shell();
        a.work[0].sessions.push(s.clone());
        a.open_session(0, s.clone(), false);
        let task = rx.try_recv().unwrap();
        a.key(KeyEvent::new(key, KeyModifiers::NONE));
        assert!(a.attach_check.is_none());
        observed_reply(&mut a, task, vec![s]);
        assert!(
            a.pending_attach.is_none(),
            "late replies must not take the terminal after cancellation"
        );
    }
}

#[test]
fn container_folder_picker_remains_folder_scoped_after_refresh() {
    let (mut a, rx) = queued_app();
    let scope = container_fixture('a', true).scope();
    let mut b = Browser::new(0, "/project-a".into());
    b.container = Some(scope.clone());
    a.browser = Some(b);
    a.view = View::Files;
    let mut s = disposable_shell();
    s.directory = "/project-a".into();
    s.container = Some(scope.clone());
    let mut other = s.clone();
    other.directory = "/project-b".into();
    other.id = "other".into();
    a.work[0].sessions = vec![s.clone(), other.clone()];
    a.sessions_here();
    observed_reply(
        &mut a,
        rx.try_recv().unwrap(),
        vec![s.clone(), other.clone()],
    );
    let Some(Dialog::SessionPicker(_, _, _, _, rows)) = &a.dialog else {
        panic!("picker");
    };
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].directory, "/project-a");
    a.open_container_sessions(0, scope);
    observed_reply(&mut a, rx.try_recv().unwrap(), vec![s, other]);
    let Some(Dialog::SessionPicker(_, _, _, _, rows)) = &a.dialog else {
        panic!("picker");
    };
    assert_eq!(rows.len(), 2, "container-wide picker includes both folders");
}

#[test]
fn identity_ignores_title_provider_and_active_pane_when_terminal_is_verified() {
    let mut s = disposable_shell();
    s.terminal = Some(crate::model::TerminalIdentity {
        server_pid: 8,
        server_start: "91".into(),
        session_id: "$0".into(),
        created: "42".into(),
    });
    let mut moved = s.clone();
    moved.pid += 1;
    moved.started = "42:900".into();
    moved.provider = "claude".into();
    moved.name = "new title".into();
    assert_eq!(workspaces::runtime_key(&s), workspaces::runtime_key(&moved));
    moved.terminal.as_mut().unwrap().server_start = "92".into();
    assert_ne!(workspaces::runtime_key(&s), workspaces::runtime_key(&moved));
}

#[test]
fn same_paths_and_ids_on_different_scopes_do_not_merge() {
    let (mut a, _) = queued_app();
    let s = disposable_shell();
    let mut container = s.clone();
    container.container = Some(container_fixture('a', true).scope());
    let mut socket = s.clone();
    socket.socket = Some("/another/socket".into());
    a.work[0].sessions = vec![s.clone(), container, socket];
    a.work[1].sessions = vec![s];
    assert_eq!(a.session_rows().len(), 4);
}

#[test]
fn persistent_navigation_cache_is_instant_scoped_and_not_mutation_authority() {
    let (mut a, _) = file_app();
    a.browser.as_mut().unwrap().preview = Some("not persisted".into());
    a.browser
        .as_mut()
        .unwrap()
        .marked
        .insert("/files/alpha.txt".into());
    let bytes = serde_json::to_vec(&workspaces::NavigationCache::snapshot(&a)).unwrap();
    assert!(!String::from_utf8_lossy(&bytes).contains("not persisted"));
    let (mut b, rx) = queued_app();
    let cache: workspaces::NavigationCache = serde_json::from_slice(&bytes).unwrap();
    cache.restore(&mut b);
    b.open_host_browser(0, "/files".into());
    let browser = b.browser.as_ref().unwrap();
    assert_eq!(browser.entries.len(), 4);
    assert!(browser.loading && browser.marked.is_empty() && browser.preview.is_none());
    assert!(browser.entries.iter().all(|e| e.identity.is_none()));
    assert!(rx
        .try_iter()
        .any(|t| matches!(t.op, Operation::List { .. })));
    let (mut changed, _) = queued_app();
    changed.devices[0].account = "different".into();
    serde_json::from_slice::<workspaces::NavigationCache>(&bytes)
        .unwrap()
        .restore(&mut changed);
    assert!(changed.browser_cache.is_empty());
}

#[test]
fn scheduler_supersedes_navigation_without_discarding_mutations() {
    let (a, _) = queued_app();
    let make = |generation, op| Task {
        device: 0,
        execution: a.devices[0].clone(),
        generation,
        op,
    };
    let mut tasks = VecDeque::new();
    for n in 1..20 {
        tasks.push_back(make(
            n,
            Operation::List {
                path: format!("/{n}"),
            },
        ));
    }
    tasks.insert(
        5,
        make(
            1,
            Operation::Mkdir {
                path: "/keep".into(),
            },
        ),
    );
    let incoming = make(20, Operation::List { path: "/20".into() });
    let discarded = workspaces::superseded_reads(&mut tasks, &incoming);
    assert_eq!(discarded.len(), 19);
    assert!(matches!(&tasks[0].op, Operation::Mkdir { path } if path == "/keep"));
    assert_eq!(tasks.len(), 1);
    tasks.push_back(make(20, Operation::List { path: "/20".into() }));
    assert!(workspaces::superseded_reads(&mut tasks, &incoming).is_empty());
}

#[test]
fn picker_refresh_keeps_disappeared_selection_instead_of_selecting_new_work() {
    let (mut a, rx) = queued_app();
    let mut s = disposable_shell();
    s.directory = "/project".into();
    s.provider = "codex".into();
    a.work[0].sessions = vec![s.clone()];
    a.start_at(0, "/project".into(), "codex".into());
    let task = rx.try_recv().unwrap();
    let mut other = s.clone();
    other.id = "other".into();
    observed_reply(&mut a, task, vec![other]);
    let Some(Dialog::SessionPicker(_, _, _, _, rows)) = &a.dialog else {
        panic!("picker");
    };
    assert_eq!(rows[a.dialog_selected].id, s.id);
    a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(matches!(a.dialog, Some(Dialog::PinnedEnded(..))));
    assert!(a.pending_attach.is_none());
}

fn identified_shell() -> Session {
    let mut s = disposable_shell();
    s.terminal = Some(crate::model::TerminalIdentity {
        server_pid: 1,
        server_start: "1".into(),
        session_id: "$1".into(),
        created: "1".into(),
    });
    s
}

#[test]
fn identity_checked_sessions_attach_on_enter_without_a_round_trip() {
    // Spec 11 flow 1: Enter attaches directly. The checked attach re-verifies
    // the runtime on the execution host, so no click-time listing is sent.
    let (mut a, rx) = queued_app();
    a.work[0].sessions.push(identified_shell());
    a.work[0].fetched = transport::now();
    a.open_device(0);
    let _ = rx.try_iter().count();
    a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(matches!(a.pending_attach, Some((0, _, false))));
    assert!(a.attach_check.is_none() && !a.pane_focused());
    assert!(rx.try_iter().all(|t| !matches!(t.op, Operation::Sessions)));
    // `w` watches read-only through the same direct path.
    a.pending_attach = None;
    press(&mut a, 'w');
    assert!(matches!(a.pending_attach, Some((0, _, true))));
    // Legacy rows without terminal identity keep the verification round trip.
    let (mut b, rx) = queued_app();
    b.work[0].sessions.push(disposable_shell());
    b.view = View::Work;
    b.device = 1;
    b.selected = 0;
    b.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(b.pending_attach.is_none() && b.attach_check.is_some());
    assert!(matches!(rx.try_recv().unwrap().op, Operation::Sessions));
}

/// The view-layer target whose action is `action` in the last frame.
fn view_target(action: &str) -> Rect {
    super::hitmap::with(|m| m.all())
        .into_iter()
        .find(|(_, h)| {
            matches!(h, super::hitmap::Hit::View(i) if super::hitmap::action(*i).as_deref() == Some(action))
        })
        .map(|(r, _)| r)
        .unwrap_or_else(|| panic!("no target {action}"))
}
fn tap_at(a: &mut App, column: u16, row: u16) {
    for kind in [
        MouseEventKind::Down(event::MouseButton::Left),
        MouseEventKind::Up(event::MouseButton::Left),
    ] {
        let event = MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        };
        a.mouse(event, Rect::new(0, 0, 160, 30));
    }
}

#[test]
fn tapping_a_session_tile_attaches_it_like_its_slot_key() {
    let (mut a, _) = queued_app();
    let mut second = identified_shell();
    second.id = "cx-second".into();
    second.name = "second shell".into();
    second.terminal.as_mut().unwrap().created = "2".into();
    a.work[0].sessions = vec![identified_shell(), second];
    a.work[0].fetched = transport::now();
    a.open_device(0);
    let mut terminal = ratatui::Terminal::new(TestBackend::new(160, 30)).unwrap();
    terminal.draw(|f| render(f, &a)).unwrap();
    let key = workspaces::runtime_key(&a.work[0].sessions[1]);
    let rect = view_target(&format!("session:0:{key}"));
    tap_at(&mut a, rect.x + 4, rect.y + 1);
    assert!(matches!(&a.pending_attach, Some((0, s, false)) if s.id == "cx-second"));
}

#[test]
fn peek_shows_the_live_screen_and_a_tap_on_it_types_inline() {
    let (mut a, rx) = queued_app();
    a.work[0].sessions.push(identified_shell());
    a.work[0].fetched = transport::now();
    a.providers
        .get_mut(&0)
        .unwrap()
        .0
        .push("live-session-pane-v1".into());
    a.open_device(0);
    let _ = rx.try_iter().count();
    assert!(a.refresh_session_preview());
    let task = rx.try_recv().unwrap();
    a.apply(Reply {
        device_id: Some("local".into()),
        execution_target: Some(None),
        preview: None,
        device: 0,
        op: task.op,
        generation: task.generation,
        result: Ok(serde_json::json!({"live":true,"pane_id":"%1","styled_lines":["peeked screen"],"cols":80,"rows":24})),
    });
    press(&mut a, ' ');
    assert!(a.peek);
    let mut terminal = ratatui::Terminal::new(TestBackend::new(160, 30)).unwrap();
    terminal.draw(|f| render(f, &a)).unwrap();
    let text: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect();
    assert!(text.contains("peeked screen"), "{text}");
    // A tap on the peeked screen types into it; Ctrl+\ leaves without sending.
    let screen = a.preview_screen();
    let tap = |column, row| MouseEvent {
        kind: MouseEventKind::Down(event::MouseButton::Left),
        column,
        row,
        modifiers: KeyModifiers::NONE,
    };
    let area = Rect::new(0, 0, 160, 30);
    a.mouse(tap(screen.x + 1, screen.y), area);
    assert!(a.pane_focused());
    a.key(KeyEvent::new(KeyCode::Char('\\'), KeyModifiers::CONTROL));
    assert!(!a.pane_focused());
    // While typing, a tap outside the screen stops it: the bar's `ctrl+\` chip
    // (bottom row) and the crumb (top row, toward the hub) alike.
    for row in [29, 0] {
        a.mouse(tap(screen.x + 1, screen.y), area);
        assert!(a.pane_focused());
        a.mouse(tap(2, row), area);
        assert!(!a.pane_focused() && a.peek, "row {row}");
    }
    assert!(rx
        .try_iter()
        .all(|t| !matches!(t.op, Operation::SessionPaneInput { .. })));
    // One Esc is the agent's; Esc Esc stops typing, and one more Esc leaves the peek.
    a.mouse(tap(screen.x + 1, screen.y), area);
    let esc = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
    a.key(esc);
    assert!(a.pane_focused());
    a.key(esc);
    assert!(!a.pane_focused() && a.peek);
    assert_eq!(
        rx.try_iter()
            .filter(|t| matches!(t.op, Operation::SessionPaneInput { .. }))
            .count(),
        1,
        "only the first Esc reached the session"
    );
    a.key(esc);
    assert!(!a.peek);
}

fn live_preview_app() -> (App, mpsc::Receiver<Task>) {
    let (mut a, rx) = queued_app();
    a.work[0].sessions.push(identified_shell());
    a.work[0].fetched = transport::now();
    a.providers
        .get_mut(&0)
        .unwrap()
        .0
        .push("live-session-pane-v1".into());
    a.view = View::Work;
    a.device = 1;
    a.selected = 0;
    assert!(a.refresh_session_preview());
    let task = rx.try_recv().unwrap();
    a.apply(Reply {
        device_id: Some("local".into()),
        execution_target: Some(None),
        preview: None,
        device: 0,
        op: task.op,
        generation: task.generation,
        result: Ok(serde_json::json!({"live":true,"pane_id":"%1","styled_lines":["screen"],"cols":80,"rows":24})),
    });
    (a, rx)
}

#[test]
fn preview_clicks_leave_an_open_text_input_in_charge() {
    let (mut a, rx) = live_preview_app();
    press(&mut a, ' ');
    let mut terminal = ratatui::Terminal::new(TestBackend::new(160, 30)).unwrap();
    terminal.draw(|f| render(f, &a)).unwrap();
    let screen = a.preview_screen();
    // The palette is open over the peek: a tap on the screen beneath it only
    // closes the palette, never focuses the pane or sends the query to it.
    press(&mut a, '/');
    press(&mut a, 'l');
    assert!(a.input == Some(Input::Palette));
    terminal.draw(|f| render(f, &a)).unwrap();
    let tap = |column, row| MouseEvent {
        kind: MouseEventKind::Down(event::MouseButton::Left),
        column,
        row,
        modifiers: KeyModifiers::NONE,
    };
    a.mouse(tap(screen.x, screen.bottom() - 1), Rect::new(0, 0, 160, 30));
    assert!(!a.pane_focused());
    assert!(rx
        .try_iter()
        .all(|t| !matches!(t.op, Operation::SessionPaneInput { .. })));
}

#[test]
fn palette_sends_a_literal_ctrl_backslash_to_the_last_session_and_reattaches() {
    let (mut a, rx) = live_preview_app();
    assert!(
        !a.action_enabled(Action::SendCtrlBackslash),
        "nothing detached yet"
    );
    // A detach lands on the hub with the session selected: the action is there.
    let s = a.work[0].sessions[0].clone();
    a.returned_from(0, &s);
    assert!(a.action_enabled(Action::SendCtrlBackslash));
    a.refresh_session_preview();
    if let Some(task) = rx
        .try_iter()
        .find(|t| matches!(t.op, Operation::SessionPreview { .. }))
    {
        a.apply(Reply {
            device_id: Some("local".into()),
            execution_target: Some(None),
            preview: None,
            device: 0,
            op: task.op,
            generation: task.generation,
            result: Ok(serde_json::json!({"live":true,"pane_id":"%1","styled_lines":["screen"],"cols":80,"rows":24})),
        });
    }
    a.execute(Action::SendCtrlBackslash);
    let sent: Vec<_> = rx
        .try_iter()
        .filter_map(|t| match t.op {
            Operation::SessionPaneInput { hex, pane_id, .. } => Some((hex, pane_id)),
            _ => None,
        })
        .collect();
    assert_eq!(sent, [("1c".to_string(), "%1".to_string())]);
    // Re-attached, so the follow-up key (nvim Ctrl+\ Ctrl+N) is typed natively.
    assert!(a.pending_attach.is_some() || a.attach_check.is_some());
    assert!(!a.pane_focused());
}

#[test]
fn passive_reports_never_postpone_attachment_but_keys_do() {
    use crossterm::event::{MouseButton, MouseEvent as Mouse};
    let mouse = |kind| {
        Event::Mouse(Mouse {
            kind,
            column: 1,
            row: 1,
            modifiers: KeyModifiers::NONE,
        })
    };
    for passive in [
        mouse(MouseEventKind::Moved),
        mouse(MouseEventKind::Up(MouseButton::Left)),
        Event::FocusLost,
        Event::Resize(80, 24),
    ] {
        assert!(runtime::passive_input(&passive));
    }
    for owning in [
        Event::Key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE)),
        Event::Paste("x".into()),
        mouse(MouseEventKind::Down(MouseButton::Left)),
        mouse(MouseEventKind::ScrollDown),
    ] {
        assert!(!runtime::passive_input(&owning));
    }
}

#[test]
fn control_keys_reach_the_pane_with_their_legacy_bytes() {
    let ctrl = |c| encode_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL));
    assert_eq!(ctrl(']'), Some(vec![0x1d]));
    assert_eq!(ctrl('5'), Some(vec![0x1d]));
    assert_eq!(ctrl('4'), Some(vec![0x1c]));
    assert_eq!(ctrl('7'), Some(vec![0x1f]));
    assert_eq!(ctrl(' '), Some(vec![0]));
    assert_eq!(ctrl('o'), Some(vec![0x0f]));
    assert!(session_preview::detach_key(&KeyEvent::new(
        KeyCode::Char('\\'),
        KeyModifiers::CONTROL
    )));
    assert!(session_preview::detach_key(&KeyEvent::new(
        KeyCode::Char('4'),
        KeyModifiers::CONTROL
    )));
    assert!(!session_preview::detach_key(&KeyEvent::new(
        KeyCode::Char(']'),
        KeyModifiers::CONTROL
    )));
}

#[test]
fn keystroke_echo_polls_immediately_and_parsed_screen_is_cached() {
    let (mut a, rx) = queued_app();
    a.work[0].sessions.push(identified_shell());
    a.work[0].fetched = transport::now();
    a.providers
        .get_mut(&0)
        .unwrap()
        .0
        .push("live-session-pane-v1".into());
    a.focus = Focus::Workspace;
    assert!(a.refresh_session_preview());
    let task = rx.try_recv().unwrap();
    let reply = |task: Task, line: &str| Reply {
        device_id: Some("local".into()),
        execution_target: Some(None),
        preview: None,
        device: 0,
        op: task.op,
        generation: task.generation,
        result: Ok(
            serde_json::json!({"live":true,"pane_id":"%1","styled_lines":[line],"cols":80,"rows":24}),
        ),
    };
    a.apply(reply(task, "\u{1b}[31mfirst"));
    let r1 = a.session_preview_revision();
    let _ = capture_app(&a, 160);
    let _ = capture_app(&a, 160);
    assert_eq!(a.session_preview_revision(), r1, "drawing never reparses");
    // A tap on the peeked screen (the same entry point).
    a.type_into_preview();
    assert!(a.pane_focused());
    // Focusing takes one fresh capture; after it lands nothing is due yet.
    a.refresh_session_preview();
    let focus = rx.try_recv().unwrap();
    a.apply(reply(focus, "\u{1b}[31mfirst"));
    assert_eq!(
        a.session_preview_revision(),
        r1,
        "identical capture: no redraw"
    );
    a.refresh_session_preview();
    assert!(rx.try_recv().is_err());
    press(&mut a, 'x');
    assert!(matches!(
        rx.try_recv().unwrap().op,
        Operation::SessionPaneInput { .. }
    ));
    // A keystroke makes the echo capture due at once, without waiting 100 ms.
    a.refresh_session_preview();
    let echo = rx.try_recv().unwrap();
    assert!(matches!(echo.op, Operation::SessionPreview { .. }));
    // Another key while that capture is in flight re-polls right after it lands.
    press(&mut a, 'y');
    let _ = rx.try_recv();
    a.apply(reply(echo, "firstx"));
    assert!(a.session_preview_revision() > r1);
    a.refresh_session_preview();
    assert!(matches!(
        rx.try_recv().unwrap().op,
        Operation::SessionPreview { .. }
    ));
    assert_eq!(a.session_preview_wait(), None, "its reply wakes the loop");
}

#[test]
fn focused_scrollback_and_missing_selection_never_spin_the_event_loop() {
    let (mut a, rx) = queued_app();
    a.work[0].sessions.push(identified_shell());
    a.work[0].fetched = transport::now();
    a.providers
        .get_mut(&0)
        .unwrap()
        .0
        .push("live-session-pane-v1".into());
    a.focus = Focus::Workspace;
    let reply = |task: Task| {
        let lines: Vec<String> = (0..200).map(|n| format!("line {n}")).collect();
        Reply {
            device_id: Some("local".into()),
            execution_target: Some(None),
            preview: None,
            device: 0,
            op: task.op,
            generation: task.generation,
            result: Ok(serde_json::json!({
                "live": true, "pane_id": "%1", "styled_lines": lines,
                "cols": 80, "rows": 24, "history_total": 176, "line_start": 0,
                "cursor_y": 23
            })),
        }
    };
    assert!(a.refresh_session_preview());
    a.apply(reply(rx.try_recv().unwrap()));
    // A tap on the peeked screen (the same entry point).
    a.type_into_preview();
    assert!(a.pane_focused());
    a.refresh_session_preview();
    a.apply(reply(rx.try_recv().unwrap()));
    // Type, so echo polling runs at its fastest interval.
    press(&mut a, 'x');
    let _ = rx.try_recv();
    a.refresh_session_preview();
    if let Ok(echo) = rx.try_recv() {
        a.apply(reply(echo));
    }
    // Lay out the peek (where typing starts) so the wheel lands on its screen.
    a.peek = true;
    let _ = capture_app(&a, 160);
    let (column, row) = (100, 10);
    for _ in 0..5 {
        a.mouse(
            MouseEvent {
                kind: MouseEventKind::ScrollUp,
                column,
                row,
                modifiers: KeyModifiers::NONE,
            },
            Rect::new(0, 0, 160, 24),
        );
    }
    // The wheel requests one history page; once it lands nothing is due.
    a.refresh_session_preview();
    let page = rx.try_recv().unwrap();
    assert!(matches!(
        page.op,
        Operation::SessionPreview {
            scrollback: true,
            ..
        }
    ));
    a.apply(reply(page));
    a.refresh_session_preview();
    assert!(rx.try_recv().is_err(), "a loaded page is not re-requested");
    // Typing while scrolled back must not make the loop poll with a zero wait.
    std::thread::sleep(Duration::from_millis(20));
    let wait = a.session_preview_wait();
    assert!(
        wait.is_none_or(|w| w > Duration::ZERO),
        "focused scrollback wait {wait:?}"
    );
    // A focused preview whose session vanished requests nothing either.
    a.work[0].sessions.clear();
    assert!(a.session_preview_wait().is_none());
}

#[test]
fn unreachable_or_cached_devices_keep_the_in_cx_round_trip() {
    // An errored device, a startup-cache row (never fetched) and a listing
    // older than a minute must not suspend the viewer into a raw ssh.
    for case in 0..3 {
        let (mut a, rx) = queued_app();
        a.work[0].sessions.push(identified_shell());
        match case {
            0 => {
                a.work[0].fetched = transport::now();
                a.work[0].error = Some("ssh: connect: Connection refused".into());
            }
            1 => a.work[0].fetched = 0,
            _ => a.work[0].fetched = transport::now().saturating_sub(120),
        }
        // The device's own screen, its first session selected.
        a.view = View::Work;
        a.device = 1;
        a.selected = 0;
        a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(
            a.pending_attach.is_none(),
            "case {case} suspended the viewer"
        );
        assert!(a.attach_check.is_some(), "case {case}");
        assert!(matches!(rx.try_recv().unwrap().op, Operation::Sessions));
    }
}

#[test]
fn a_helper_watch_replaces_polling_and_polling_returns_when_it_ends() {
    let (mut a, rx) = queued_app();
    a.work[0].sessions.push(identified_shell());
    a.work[0].fetched = transport::now();
    a.providers
        .get_mut(&0)
        .unwrap()
        .0
        .push("session-preview-v1".into());
    a.focus = Focus::Workspace;
    a.refresh_session_preview();
    a.sync_watches();
    let tasks: Vec<Task> = rx.try_iter().collect();
    // An old remote helper (no watch-v1) is never sent a Watch it cannot decode.
    assert!(tasks
        .iter()
        .all(|t| t.device == 0 || !matches!(t.op, Operation::Watch { .. })));
    let watch = tasks
        .into_iter()
        .find(|t| {
            matches!(
                &t.op,
                Operation::Watch {
                    sessions: true,
                    pane: Some(_)
                }
            )
        })
        .expect("the selected local session is watched");
    let screen = |line: &str| serde_json::json!({"live":true,"pane_id":"%1","styled_lines":[line],"cols":80,"rows":24});
    // The reply carries the first screen.
    a.watch_replied(Reply {
        device_id: Some("local".into()),
        execution_target: Some(None),
        preview: None,
        device: 0,
        op: watch.op,
        generation: watch.generation,
        result: Ok(serde_json::json!({"watching": true, "pane": screen("first")})),
    });
    assert!(a.sessions_pushed(0));
    let revision = a.session_preview_revision();
    // Watched: nothing is polled, however long since the last capture.
    std::thread::sleep(Duration::from_millis(300));
    a.refresh_session_preview();
    a.sync_watches();
    assert!(rx.try_recv().is_err(), "a watched screen is never polled");
    // A pushed screen for the selected session applies like a polled one.
    let mut pushed = screen("second");
    pushed["kind"] = "pane".into();
    pushed["session_id"] = "$1".into();
    let (redraw, _) = a.watch_event(None, pushed.clone());
    assert!(redraw && a.session_preview_revision() > revision);
    // One for another session is ignored.
    pushed["session_id"] = "$2".into();
    pushed["styled_lines"] = serde_json::json!(["other"]);
    assert_eq!(a.watch_event(None, pushed), (false, false));
    // A pushed list replaces the device's list.
    let mut more = identified_shell();
    more.id = format!("cx-{}", "b".repeat(64));
    let list = serde_json::json!({"kind": "sessions", "sessions": [identified_shell(), more]});
    assert_eq!(a.watch_event(None, list), (true, true));
    assert_eq!(a.work[0].sessions.len(), 2);
    // The watch ends (control client killed, helper gone): polling resumes at once.
    a.watch_event(None, serde_json::json!({"kind": "closed"}));
    assert!(!a.sessions_pushed(0));
    a.refresh_session_preview();
    assert!(matches!(
        rx.try_recv().unwrap().op,
        Operation::SessionPreview { .. }
    ));
}

fn watched_local_app() -> (App, mpsc::Receiver<Task>, Task) {
    let (mut a, rx) = queued_app();
    a.work[0].sessions.push(identified_shell());
    a.work[0].fetched = transport::now();
    a.providers
        .get_mut(&0)
        .unwrap()
        .0
        .push("session-preview-v1".into());
    a.focus = Focus::Workspace;
    a.refresh_session_preview();
    a.sync_watches();
    let watch = rx
        .try_iter()
        .find(|t| matches!(t.op, Operation::Watch { pane: Some(_), .. }))
        .expect("the selected local session is watched");
    (a, rx, watch)
}

#[test]
fn a_terminal_handoff_ends_the_watch_and_return_watches_again() {
    let (mut a, rx, _) = watched_local_app();
    // Attached, nothing reads helper events: the pane must not keep streaming.
    a.stop_watches();
    let ops: Vec<_> = rx.try_iter().map(|t| t.op).collect();
    assert!(
        matches!(
            ops[..],
            [Operation::Watch {
                sessions: false,
                pane: None
            }]
        ),
        "{ops:?}"
    );
    assert!(!a.sessions_pushed(0));
    a.sync_watches();
    assert!(rx.try_iter().any(|t| matches!(
        t.op,
        Operation::Watch {
            sessions: true,
            pane: Some(_)
        }
    )));
}

#[test]
fn a_pushed_screen_that_overtakes_its_watch_reply_is_kept() {
    let (mut a, _rx, watch) = watched_local_app();
    let screen = |line: &str| serde_json::json!({"live":true,"pane_id":"%1","styled_lines":[line],"cols":80,"rows":24});
    // In-process, the watch thread's first change can reach the loop before
    // the task worker's reply.
    let mut pushed = screen("NEWER");
    pushed["kind"] = "pane".into();
    pushed["session_id"] = "$1".into();
    assert!(a.watch_event(None, pushed).0);
    a.watch_replied(Reply {
        device_id: Some("local".into()),
        execution_target: Some(None),
        preview: None,
        device: 0,
        op: watch.op,
        generation: watch.generation,
        result: Ok(serde_json::json!({"watching": true, "pane": screen("OLDER")})),
    });
    assert_eq!(a.session_preview_lines(), ["NEWER"]);
    assert!(a.sessions_pushed(0));
}

/// A frame the replaced watch sent before the helper ended it (it is drained only
/// after the new watch is asked for) never hides the new watch's first screen, and
/// once the new reply names its watch, the old one's frames are ignored.
#[test]
fn a_replaced_watchs_late_frame_never_overrides_the_new_reply() {
    let (mut a, rx, watch) = watched_local_app();
    let screen = |line: &str, id: u64| serde_json::json!({"live":true,"pane_id":"%1","styled_lines":[line],"cols":80,"rows":24,"watch":id});
    let frame = |line: &str, id: u64| {
        let mut v = screen(line, id);
        v["kind"] = "pane".into();
        v["session_id"] = "$1".into();
        v
    };
    let reply = |a: &mut App, task: Task, value: Value| {
        a.watch_replied(Reply {
            device_id: Some("local".into()),
            execution_target: Some(None),
            preview: None,
            device: 0,
            op: task.op,
            generation: task.generation,
            result: Ok(value),
        })
    };
    reply(
        &mut a,
        watch,
        serde_json::json!({"watching": true, "watch": 1, "pane": screen("BEFORE", 1)}),
    );
    a.stop_watches();
    a.sync_watches();
    let again = rx
        .try_iter()
        .find(|t| matches!(t.op, Operation::Watch { pane: Some(_), .. }))
        .expect("watched again");
    a.watch_event(None, frame("STALE", 1));
    reply(
        &mut a,
        again,
        serde_json::json!({"watching": true, "watch": 2, "pane": screen("FRESH", 2)}),
    );
    assert_eq!(a.session_preview_lines(), ["FRESH"]);
    assert_eq!(a.watch_event(None, frame("STALE AGAIN", 1)), (false, false));
    assert_eq!(a.session_preview_lines(), ["FRESH"]);
    assert!(a.watch_event(None, frame("NEXT", 2)).0);
    assert_eq!(a.session_preview_lines(), ["NEXT"]);
}

#[test]
fn a_refused_watch_falls_back_to_polling() {
    let (mut a, rx) = queued_app();
    a.work[0].sessions.push(identified_shell());
    a.work[0].fetched = transport::now();
    a.providers
        .get_mut(&0)
        .unwrap()
        .0
        .push("session-preview-v1".into());
    a.focus = Focus::Workspace;
    a.sync_watches();
    let watch = rx
        .try_iter()
        .find(|t| matches!(t.op, Operation::Watch { .. }))
        .unwrap();
    a.watch_replied(Reply {
        device_id: Some("local".into()),
        execution_target: Some(None),
        preview: None,
        device: 0,
        op: watch.op,
        generation: watch.generation,
        result: Err(anyhow::anyhow!("tmux control mode unavailable")),
    });
    assert!(!a.sessions_pushed(0));
    a.refresh_session_preview();
    a.sync_watches();
    let ops: Vec<_> = rx.try_iter().map(|t| t.op).collect();
    assert!(ops
        .iter()
        .any(|op| matches!(op, Operation::SessionPreview { .. })));
    assert!(
        !ops.iter().any(|op| matches!(op, Operation::Watch { .. })),
        "no retry before the backoff"
    );
}

#[test]
fn a_hub_tile_keeps_its_pushed_screen_after_the_selection_moves() {
    let (mut a, rx) = queued_app();
    let first = identified_shell();
    let mut second = identified_shell();
    second.id = format!("cx-{}", "b".repeat(64));
    if let Some(t) = second.terminal.as_mut() {
        t.session_id = "$2".into();
        t.created = "2".into();
    }
    a.work[0].sessions = vec![first.clone(), second.clone()];
    a.work[0].fetched = transport::now();
    a.providers
        .get_mut(&0)
        .unwrap()
        .0
        .push("session-preview-v1".into());
    a.go_hub();
    a.hub_sessions = true;
    a.reconcile_session_selection(Some((0, workspaces::runtime_key(&first))));
    a.refresh_session_preview();
    a.sync_watches();
    let watch = rx
        .try_iter()
        .find(|t| matches!(t.op, Operation::Watch { pane: Some(_), .. }))
        .expect("the selected tile's screen is watched");
    a.watch_replied(Reply {
        device_id: Some("local".into()),
        execution_target: Some(None),
        preview: None,
        device: 0,
        op: watch.op,
        generation: watch.generation,
        result: Ok(serde_json::json!({"watching": true, "pane": {
            "live": true, "pane_id": "%1", "styled_lines": ["PUSHED-SCREEN"],
            "cols": 80, "rows": 24}})),
    });
    // Another tile selected: the first keeps its last frame from memory, unpolled.
    a.reconcile_session_selection(Some((0, workspaces::runtime_key(&second))));
    a.refresh_session_preview();
    let mut t = Terminal::new(TestBackend::new(120, 40)).unwrap();
    t.draw(|f| render(f, &a)).unwrap();
    let text: String = t
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect();
    assert!(text.contains("PUSHED-SCREEN"), "the tile lost its frame");
}
