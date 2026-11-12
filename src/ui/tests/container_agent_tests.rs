use super::*;

fn evidence(a: &mut App, d: usize, scope: &ContainerScope, profiles: &[&str]) {
    a.apply_container_profiles(
        d,
        scope,
        Ok(serde_json::json!({"scope":scope,"providers":profiles,"observed_at":1})),
    );
}
#[test]
fn container_profiles_are_scoped_and_preserve_selected_shell_on_arrival() {
    let (mut a, rx) = queued_app();
    let scope = container_fixture('a', true).scope();
    a.open_container_chooser(0, scope.clone());
    assert!(matches!(
        rx.try_recv().unwrap().op,
        Operation::ContainerProviders { .. }
    ));
    assert_eq!(
        a.container_session_choices(0, &scope).len(),
        1,
        "host agents must not leak into container"
    );
    evidence(&mut a, 0, &scope, &["shell", "claude", "codex"]);
    assert_eq!(a.container_session_choices(0, &scope).len(), 3);
    assert_eq!(
        a.container_session_choices(0, &scope)[a.dialog_selected].provider,
        "shell"
    );
    let mut changed = scope.clone();
    changed.started_at.push('x');
    assert_eq!(a.container_session_choices(0, &changed).len(), 1);
    changed = scope.clone();
    changed.user = "another-user".into();
    assert_eq!(a.container_session_choices(0, &changed).len(), 1);
    changed = scope.clone();
    changed.folder = "/another-folder".into();
    assert_eq!(a.container_session_choices(0, &changed).len(), 1);
    assert_eq!(a.container_session_choices(1, &scope).len(), 1);
}
#[test]
fn container_history_and_new_preserve_scope_permissions_and_intent() {
    for provider in ["claude", "codex"] {
        for resume in [false, true] {
            for yolo in [false, true] {
                let (mut a, rx) = queued_app();
                let mut scope = container_fixture('a', true).scope();
                scope.folder = "/workspace/nested folder".into();
                evidence(&mut a, 0, &scope, &["shell", provider]);
                a.choose_container_profile(0, scope.clone(), provider.into());
                assert!(
                    matches!(&a.dialog, Some(Dialog::SessionPicker(_,_,_,_,rows)) if rows.is_empty())
                );
                a.dialog_selected = usize::from(resume);
                a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
                assert!(
                    matches!(&a.dialog, Some(Dialog::ContainerPermissions(_, s, p, r)) if s == &scope && p == provider && *r == resume)
                );
                a.dialog_selected = usize::from(yolo);
                a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
                let creates: Vec<_> = rx
                    .try_iter()
                    .filter(|t| matches!(t.op, Operation::ContainerAgentCreate { .. }))
                    .collect();
                assert_eq!(creates.len(), 1);
                assert!(
                    matches!(&creates[0].op, Operation::ContainerAgentCreate {scope:s,request,yolo:y} if s == &scope && request.directory == scope.folder && request.provider == provider && request.resume == resume && *y == yolo)
                );
            }
        }
    }
}
#[test]
fn container_history_selection_survives_runtime_refresh() {
    let (mut a, rx) = queued_app();
    let scope = container_fixture('a', true).scope();
    evidence(&mut a, 0, &scope, &["shell", "codex"]);
    a.choose_container_profile(0, scope.clone(), "codex".into());
    a.dialog_selected = 1; // History, after New.
    let task = rx.try_recv().unwrap();
    let mut s = disposable_shell();
    s.provider = "codex".into();
    s.directory = scope.folder.clone();
    s.container = Some(scope);
    a.apply(Reply {
        device_id: None,
        execution_target: None,
        preview: None,
        device: task.device,
        generation: task.generation,
        op: task.op,
        result: Ok(serde_json::to_value(vec![s]).unwrap()),
    });
    assert_eq!(a.dialog_selected, 2);
    a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(matches!(
        a.dialog,
        Some(Dialog::ContainerPermissions(_, _, _, true))
    ));
}
#[test]
fn container_failed_or_mismatched_evidence_never_authorizes_launch() {
    let (mut a, rx) = queued_app();
    let scope = container_fixture('a', true).scope();
    let mut different = scope.clone();
    different.started_at.push('x');
    a.apply_container_profiles(
        0,
        &scope,
        Ok(serde_json::json!({"scope":different,"providers":["shell","codex"],"observed_at":1})),
    );
    a.launch_container_agent(0, scope.clone(), "codex".into(), true, true);
    assert!(!rx
        .try_iter()
        .any(|t| matches!(t.op, Operation::ContainerAgentCreate { .. })));
    evidence(&mut a, 0, &scope, &["shell", "codex"]);
    a.container_permissions(0, scope.clone(), "codex".into(), true);
    a.apply_container_profiles(0, &scope, Err(anyhow::anyhow!("fixture unavailable")));
    a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(!rx
        .try_iter()
        .any(|t| matches!(t.op, Operation::ContainerAgentCreate { .. })));
    assert!(a.pending_attach.is_none());
}
#[test]
fn container_agent_choice_keeps_existing_runtime_separate_from_native_history() {
    let (mut a, rx) = queued_app();
    let scope = container_fixture('a', true).scope();
    evidence(&mut a, 0, &scope, &["shell", "claude"]);
    let mut s = disposable_shell();
    s.provider = "claude".into();
    s.container = Some(scope.clone());
    s.directory = scope.folder.clone();
    a.work[0].sessions.push(s);
    a.choose_container_profile(0, scope, "claude".into());
    assert!(matches!(&a.dialog, Some(Dialog::SessionPicker(_,_,_,_,rows)) if rows.len() == 1));
    a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(a.attach_check.is_some());
    assert!(rx.try_iter().all(|t| !matches!(
        t.op,
        Operation::ContainerCreate { .. } | Operation::ContainerAgentCreate { .. }
    )));
}
#[test]
fn container_profile_captures() {
    let (mut a, _) = queued_app();
    let scope = container_fixture('a', true).scope();
    evidence(&mut a, 0, &scope, &["shell", "codex", "claude"]);
    a.open_container_chooser(0, scope.clone());
    for width in [48, 80, 120] {
        let text = capture_app(&a, width);
        assert!(text.contains("Claude") && text.contains("Codex"));
        if let Some(dir) = std::env::var_os("CX_CONTAINER_CAPTURE_DIR") {
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(
                std::path::Path::new(&dir).join(format!("profiles-{width}.txt")),
                text,
            )
            .unwrap();
        }
    }
}

#[test]
fn remote_container_capability_probe_is_explicit_and_requires_new_helper() {
    let (mut a, rx) = queued_app();
    let scope = container_fixture('a', true).scope();
    a.providers.remove(&1);
    a.policies[1].availability = store::Availability::UsuallyDown;
    a.open_container_chooser(1, scope.clone());
    let t = rx.try_recv().unwrap();
    assert!(matches!(t.op, Operation::Info));
    a.apply(Reply {
        device_id: None,
        execution_target: None,
        preview: None,
        device: 1,
        generation: t.generation,
        op: t.op,
        result: Ok(serde_json::json!({"capabilities":["codex","container-agents-v1"]})),
    });
    let t = rx.try_recv().unwrap();
    assert!(matches!(t.op, Operation::ContainerProviders { .. }));
    a.apply(Reply {
        device_id: None,
        execution_target: None,
        preview: None,
        device: 1,
        generation: t.generation,
        op: t.op,
        result: Ok(
            serde_json::json!({"scope":scope,"providers":["shell","claude"],"observed_at":1}),
        ),
    });
    assert_eq!(
        a.container_session_choices(1, &scope)
            .iter()
            .map(|c| c.provider)
            .collect::<Vec<_>>(),
        vec!["shell", "claude"]
    );
    a.providers.get_mut(&1).unwrap().0 = vec!["codex".into()];
    assert_eq!(
        a.container_session_choices(1, &scope).len(),
        1,
        "old helper cannot receive new agent operation"
    );
}
#[test]
fn container_missing_observation_time_is_unknown() {
    let (mut a, _) = queued_app();
    let scope = container_fixture('a', true).scope();
    for time in [Value::Null, serde_json::json!(0), serde_json::json!("bad")] {
        a.apply_container_profiles(
            0,
            &scope,
            Ok(serde_json::json!({"scope":scope,"providers":["shell","codex"],"observed_at":time})),
        );
        assert_eq!(a.container_session_choices(0, &scope).len(), 1);
    }
}
