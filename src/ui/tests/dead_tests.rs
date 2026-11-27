//! A terminal that no longer exists is never offered, and a failed attach never
//! moves the viewer (decision 2026-11-23).
use super::*;

fn shell(id: &str, name: &str) -> Session {
    let mut s = disposable_shell();
    s.id = id.into();
    s.name = name.into();
    s
}
fn listed(a: &mut App, d: usize, rows: &[Session]) {
    a.apply(reply(
        a,
        d,
        Operation::Sessions,
        Ok(serde_json::json!(rows)),
    ));
}
fn enter(a: &mut App) {
    a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
}
fn adopt_caps(a: &mut App, d: usize) {
    a.providers.insert(
        d,
        (
            vec![
                "claude".into(),
                "adopt-session-v1".into(),
                "adopt-reptyr-v1".into(),
            ],
            transport::now(),
        ),
    );
}
/// The device's checked attach found the terminal gone (its dedicated refusal).
fn refused() -> anyhow::Error {
    sessions::Gone.into()
}

/// The device refused the attach (the terminal ended behind the viewer's back): the
/// device screen stays, the tile goes with a quiet notice, the neighbour is selected.
#[test]
fn refused_attach_stays_on_the_device_and_removes_the_tile() {
    let (mut a, _rx) = queued_app();
    listed(&mut a, 1, &[shell("cx-one", "one"), shell("cx-two", "two")]);
    a.open_device(1);
    let gone = a.selected_session().unwrap().1;
    assert_eq!(gone.id, "cx-one");
    a.attach_finished(1, &gone, Err(refused()), None);
    assert_eq!(a.view, View::Work);
    assert_eq!(a.device, 2);
    assert!(a.work[1].sessions.iter().all(|s| s.id != "cx-one"));
    assert_eq!(
        a.selected_session().map(|(_, s)| s.id),
        Some("cx-two".into())
    );
    assert!(a.notice.contains("ended · removed"), "{}", a.notice);
    assert_eq!(a.notice_kind, NoticeKind::Info);
    // Not something to detach back to.
    assert!(a.last_session.is_none());
}

/// Any other failure reports, but still never navigates and keeps the tile (the
/// refresh decides).
#[test]
fn other_attach_failures_report_without_navigating() {
    let (mut a, _rx) = queued_app();
    listed(&mut a, 1, &[shell("cx-one", "one")]);
    a.open_device(1);
    let s = a.work[1].sessions[0].clone();
    a.attach_finished(1, &s, Err(anyhow::anyhow!("pty allocation failed")), None);
    assert_eq!((a.view, a.device), (View::Work, 2));
    assert_eq!(a.work[1].sessions.len(), 1);
    assert!(a.notice.contains("Attachment failed"), "{}", a.notice);
}

/// An older helper's refusal (no dedicated status) or any other failure that only
/// reads like one is a failure, never proof the terminal ended: the tile stays.
#[test]
fn a_refusal_without_the_gone_status_keeps_the_tile() {
    let (mut a, _rx) = queued_app();
    listed(&mut a, 1, &[shell("cx-one", "one")]);
    a.open_device(1);
    let s = a.work[1].sessions[0].clone();
    let old = anyhow::anyhow!("Session changed or ended; refresh before attaching");
    a.attach_finished(1, &s, Err(old), None);
    assert_eq!((a.view, a.device), (View::Work, 2));
    assert_eq!(a.work[1].sessions.len(), 1);
    assert!(a.ended.is_empty());
    assert!(a.notice.contains("Attachment failed"), "{}", a.notice);
}

/// A detach lands on the session's device with its tile selected (`⏎` re-attaches).
#[test]
fn detach_returns_to_the_device_with_the_tile_selected() {
    let (mut a, _rx) = queued_app();
    listed(&mut a, 1, &[shell("cx-one", "one"), shell("cx-two", "two")]);
    a.open_device(1);
    let s = a.work[1].sessions[1].clone();
    a.attach_finished(1, &s, Ok(()), None);
    assert_eq!((a.view, a.device), (View::Work, 2));
    assert_eq!(
        a.selected_session().map(|(_, s)| s.id),
        Some("cx-two".into())
    );
    assert!(a.notice.contains("Returned from"), "{}", a.notice);
}

/// A move refused because the outside terminal closed removes its tile and asks the
/// device for its outside terminals again.
#[test]
fn failed_adopt_of_a_closed_terminal_removes_it_and_rediscovers() {
    let (mut a, rx) = queued_app();
    adopt_caps(&mut a, 1);
    listed(&mut a, 1, &[]);
    a.apply(reply(
        &a,
        1,
        Operation::ExternalSessions,
        Ok(serde_json::json!([external_agent()])),
    ));
    a.open_device(1);
    // (Opening it scanned it: that answer comes first.)
    a.apply(reply(
        &a,
        1,
        Operation::ExternalSessions,
        Ok(serde_json::json!([external_agent()])),
    ));
    let _ = rx.try_iter().count();
    let s = external_agent();
    let op = Operation::AdoptSession {
        id: s.id.clone(),
        origin: s.origin.clone().unwrap(),
        boot_id: s.boot_id.clone(),
    };
    a.apply(reply(
        &a,
        1,
        op,
        Err(anyhow::anyhow!(
            "terminal changed or closed; refresh sessions"
        )),
    ));
    assert!(a.work[1].sessions.is_empty());
    assert_eq!((a.view, a.device), (View::Work, 2));
    assert!(a.notice.contains("ended · removed"), "{}", a.notice);
    assert!(rx
        .try_iter()
        .any(|t| t.device == 1 && matches!(t.op, Operation::ExternalSessions)));
}

/// Outside terminals are bound to a process: one restored from the disk cache is not
/// drawn, listed or selectable until the device confirms it again.
#[test]
fn cached_outside_terminals_stay_hidden_until_confirmed() {
    let (mut a, _rx) = queued_app();
    listed(&mut a, 1, &[shell("cx-one", "one")]);
    a.apply(reply(
        &a,
        1,
        Operation::ExternalSessions,
        Ok(serde_json::json!([external_agent()])),
    ));
    assert_eq!(a.work[1].sessions.len(), 2);
    let snapshot = workspaces::NavigationCache::snapshot(&a);
    // The cache on disk keeps what it always kept.
    assert!(serde_json::to_string(&snapshot)
        .unwrap()
        .contains("ext-0123456789abcdef"));
    let (mut b, _rx) = queued_app();
    snapshot.restore(&mut b);
    assert!(
        b.work[1].sessions.iter().all(|s| !s.external),
        "{:?}",
        b.work[1].sessions
    );
    assert_eq!(b.work[1].sessions.len(), 1);
    b.open_device(1);
    assert!(!draw_h(&b, 120, 30).contains("outside cx"));
    b.apply(reply(
        &b,
        1,
        Operation::ExternalSessions,
        Ok(serde_json::json!([external_agent()])),
    ));
    assert!(b.work[1].sessions.iter().any(|s| s.external));
}

/// The open device asks for its outside terminals when opened, then on every other 3 s
/// list poll (6 s: a scan costs ~0.3 s on the lane the list shares), one in flight at a
/// time; the picker asks for none.
#[test]
fn the_open_device_polls_its_outside_terminals() {
    let (mut a, rx) = queued_app();
    adopt_caps(&mut a, 1);
    let externals = |rx: &mpsc::Receiver<Task>| {
        rx.try_iter()
            .filter(|t| t.device == 1 && matches!(t.op, Operation::ExternalSessions))
            .count()
    };
    let answer = |a: &mut App| {
        a.apply(reply(
            a,
            1,
            Operation::ExternalSessions,
            Ok(serde_json::json!([])),
        ))
    };
    a.open_picker();
    let _ = rx.try_iter().count();
    for _ in 0..4 {
        a.poll_sessions();
    }
    assert_eq!(externals(&rx), 0);
    a.open_device(1);
    assert_eq!(externals(&rx), 1, "opening a device scans it at once");
    answer(&mut a);
    let mut sent = 0;
    for _ in 0..4 {
        a.poll_sessions();
        let n = externals(&rx);
        sent += n;
        if n > 0 {
            answer(&mut a);
        }
    }
    assert_eq!(sent, 2, "every other poll");
    // One in flight: the polls (and the fleet refresh) wait for its answer.
    for _ in 0..4 {
        a.poll_sessions();
    }
    a.refresh_work();
    assert_eq!(externals(&rx), 1, "one in flight");
    answer(&mut a);
    // Without the capability there is nothing to ask.
    a.providers
        .insert(1, (vec!["claude".into()], transport::now()));
    for _ in 0..4 {
        a.poll_sessions();
    }
    assert_eq!(externals(&rx), 0);
}

/// A scan that started before a move (or a stop) on its device may still list the
/// moved terminal: its answer is dropped and a fresh one asked for, so the moved
/// terminal never comes back as an outside tile.
#[test]
fn an_outside_scan_from_before_a_move_never_resurrects_the_moved_terminal() {
    let (mut a, rx) = queued_app();
    adopt_caps(&mut a, 1);
    listed(&mut a, 1, &[]);
    a.open_device(1);
    a.request_externals(1);
    let externals = |rx: &mpsc::Receiver<Task>| {
        rx.try_iter()
            .filter(|t| t.device == 1 && matches!(t.op, Operation::ExternalSessions))
            .count()
    };
    assert_eq!(externals(&rx), 1);
    let outside = external_agent();
    let mut moved = shell("cx-moved", "claude · api");
    moved.provider = "claude".into();
    a.apply(reply(
        &a,
        1,
        Operation::AdoptSession {
            id: outside.id.clone(),
            origin: outside.origin.clone().unwrap(),
            boot_id: outside.boot_id.clone(),
        },
        Ok(serde_json::json!(moved)),
    ));
    let _ = a.pending_attach.take();
    // The scan from before the move answers with the terminal still outside.
    a.apply(reply(
        &a,
        1,
        Operation::ExternalSessions,
        Ok(serde_json::json!([external_agent()])),
    ));
    assert!(
        a.work[1].sessions.iter().all(|s| !s.external),
        "resurrected"
    );
    assert_eq!(externals(&rx), 1, "a fresh scan");
    // The fresh one is applied.
    a.apply(reply(
        &a,
        1,
        Operation::ExternalSessions,
        Ok(serde_json::json!([])),
    ));
    assert!(a.work[1].sessions.iter().any(|s| s.id == "cx-moved"));
}

/// An unreachable device keeps its cached tiles (with why), but Enter on one only says
/// why: no attach, no check, no move.
#[test]
fn enter_on_an_unreachable_cached_tile_only_says_why() {
    let (mut a, rx) = queued_app();
    adopt_caps(&mut a, 1);
    listed(&mut a, 1, &[shell("cx-one", "one")]);
    a.apply(reply(
        &a,
        1,
        Operation::ExternalSessions,
        Ok(serde_json::json!([external_agent()])),
    ));
    a.apply(reply(
        &a,
        1,
        Operation::Sessions,
        Err(anyhow::Error::new(transport::ConnectionFailure::Refused)),
    ));
    a.open_device(1);
    assert_eq!(a.work[1].sessions.len(), 2);
    for i in 0..2 {
        a.selected = i;
        let _ = rx.try_iter().count();
        a.notice.clear();
        enter(&mut a);
        assert!(a.pending_attach.is_none());
        assert!(a.attach_check.is_none());
        assert!(a.pending_adoption.is_none() && a.pending_ptrace.is_none());
        assert!(!a.creating);
        assert!(rx
            .try_iter()
            .all(|t| !matches!(t.op, Operation::AdoptSession { .. })));
        assert_eq!((a.view, a.device), (View::Work, 2));
        assert!(a.notice.contains("refused"), "{}", a.notice);
    }
}
