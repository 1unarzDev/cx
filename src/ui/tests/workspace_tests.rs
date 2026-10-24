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
        matches!(a.dialog, Some(Dialog::Permissions(_, _, _, false))),
        "new agent goes directly to explicit launch permissions"
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
fn folder_marks_count_agents_in_exact_scope_and_show_staleness() {
    let (mut a, _) = queued_app();
    let mut b = Browser::new(0, "/".into());
    let mut s = disposable_shell();
    s.directory = "/project".into();
    s.provider = "codex".into();
    a.work[0].sessions = vec![s.clone(), s.clone()];
    assert_eq!(a.folder_agents(&b)["/project"], "Codex 1 ?");
    a.work[0].fetched = transport::now();
    assert_eq!(a.folder_agents(&b)["/project"], "Codex 1");
    b.container = Some(container_fixture('a', true).scope());
    assert!(a.folder_agents(&b).is_empty());
    a.work[0].sessions[0].container = b.container.clone();
    assert_eq!(a.folder_agents(&b)["/project"], "Codex 1");
    b.container.as_mut().unwrap().started_at = "replacement".into();
    assert!(a.folder_agents(&b).is_empty());
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

#[test]
fn workspace_capture_matrix() {
    let (mut a, _) = queued_app();
    a.device = 1;
    a.work[0].fetched = transport::now();
    for (id, name, provider) in [
        ("first", "Fix session discovery", "codex"),
        ("second", "Review terminal lifecycle", "claude"),
        ("shell", "Project shell", "shell"),
    ] {
        let mut s = disposable_shell();
        s.id = id.into();
        s.name = name.into();
        s.directory = "/workspace/cx".into();
        s.provider = provider.into();
        a.work[0].sessions.push(s);
    }
    let pinned = a.work[0].sessions[0].clone();
    pin(&mut a, 0, &pinned);
    let mut ended = disposable_shell();
    ended.name = "Compiler experiment".into();
    ended.id = "ended".into();
    pin(&mut a, 0, &ended);
    for width in [48, 80, 120] {
        let text = capture_app(&a, width);
        assert!(text.contains("not seen") && text.contains("* "));
        if let Some(dir) = std::env::var_os("CX_SESSION_WORKSPACE_CAPTURE_DIR") {
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(
                std::path::Path::new(&dir).join(format!("sessions-{width}.txt")),
                text,
            )
            .unwrap();
        }
    }
}
