//! Unified keymap, hit map, palette, view switching, phone layout and frame
//! output (spec sections 3, 6, 6.6, 8, 12).
use super::*;
use crate::ui::hitmap::Hit;
use crate::ui::keymap::Cmd;
use ratatui::buffer::Buffer;

fn draw(a: &App, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
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
fn click(a: &mut App, x: u16, y: u16, area: Rect) {
    for kind in [
        MouseEventKind::Down(event::MouseButton::Left),
        MouseEventKind::Up(event::MouseButton::Left),
    ] {
        a.mouse(
            MouseEvent {
                kind,
                column: x,
                row: y,
                modifiers: KeyModifiers::NONE,
            },
            area,
        );
    }
}
/// The rect of the view-layer target whose action is `action` (last frame).
fn act(action: &str) -> Rect {
    hitmap::with(|m| m.all())
        .into_iter()
        .find(|(_, h)| matches!(h, Hit::View(i) if hitmap::action(*i).as_deref() == Some(action)))
        .map(|(r, _)| r)
        .unwrap_or_else(|| panic!("no view target {action}"))
}
fn letter(a: &App, d: usize) -> char {
    a.hotkeys()[&a.devices[d].id]
}
fn target(hit: Hit) -> Rect {
    hitmap::with(|m| m.all())
        .into_iter()
        .find(|(_, h)| *h == hit)
        .map(|(r, _)| r)
        .unwrap_or_else(|| panic!("no hit target {hit:?}"))
}
fn sessions(a: &mut App, device: usize, count: usize) {
    a.work[device].sessions = (0..count)
        .map(|i| {
            let mut s = disposable_shell();
            s.id = format!("nav-{device}-{i:02}");
            s.name = format!("nav {device} {i:02}");
            s
        })
        .collect();
    a.work[device].fetched = transport::now();
}

#[test]
fn hub_letters_open_devices_and_shift_h_returns_from_anywhere() {
    let (mut a, _rx) = file_app();
    sessions(&mut a, 1, 2);
    a.go_hub();
    assert!(a.view == View::Network);
    // A device letter opens that device, ready to attach or start.
    let l = letter(&a, 1);
    press(&mut a, l);
    assert!(a.view == View::Work && a.device == 2);
    assert!(a.dialog.is_none(), "a letter opened a chooser (P13)");
    // `f` there is its files; Shift+H is the hub from any screen (a 65% keyboard has
    // no backtick key of its own).
    let shift_h = KeyEvent::new(KeyCode::Char('H'), KeyModifiers::SHIFT);
    a.key(shift_h);
    assert!(a.view == View::Network && a.device == 0);
    a.view = View::Files;
    a.key(shift_h);
    assert!(a.view == View::Network);
    // `` ` `` (v0.2.1's hub key) still works for one release, unlisted.
    press(&mut a, l);
    press(&mut a, '`');
    assert!(a.view == View::Network);
    // Help, the footers and the palette name `H`, never the backtick.
    press(&mut a, l);
    let shown = draw(&a, 120, 40);
    assert!(
        shown.contains("H hub") && !shown.contains("` hub"),
        "{shown}"
    );
    a.help = true;
    let help = draw(&a, 120, 40);
    assert!(!help.contains('`'), "{help}");
    a.help = false;
    // A double Esc also reaches the hub.
    let l = letter(&a, 1);
    press(&mut a, l);
    a.key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE));
    assert!(a.view == View::Files);
    // In Files H is the hub.
    press(&mut a, 'H');
    assert!(a.view == View::Network);
    // `e` then a letter opens the link to that device.
    press(&mut a, 'e');
    let l = letter(&a, 1);
    press(&mut a, l);
    assert!(a.edge.as_ref().is_some_and(|e| e.id == "0-1"));
    a.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(a.edge.is_none());
}

#[test]
fn hub_moves_never_refresh_and_keep_the_session_by_identity() {
    let (mut a, rx) = queued_app();
    sessions(&mut a, 0, 3);
    sessions(&mut a, 1, 2);
    a.go_hub();
    while rx.try_recv().is_ok() {}
    draw(&a, 120, 40);
    let id = a.selected_session().map(|(_, s)| s.id);
    a.key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
    assert!(a.hub_device.is_some());
    assert!(rx.try_recv().is_err(), "moving on the map did I/O (P17)");
    assert!(
        a.providers.contains_key(&0),
        "moving cleared provider caches"
    );
    assert_eq!(
        a.selected_session().map(|(_, s)| s.id),
        id,
        "selection jumped"
    );
    // The sessions zone moves by identity through the ranked tiles.
    a.key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    assert!(a.hub_sessions);
    let first = a.selected_session().unwrap().1.id;
    a.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    assert_ne!(a.selected_session().unwrap().1.id, first);
    assert!(rx.try_recv().is_err());
}

#[test]
fn device_slots_attach_and_moving_clamps_without_wrapping() {
    let (mut a, _rx) = queued_app();
    sessions(&mut a, 0, 3);
    a.open_device(0);
    // Slot `d` is the second session: it attaches at once.
    press(&mut a, 'd');
    let attached = a
        .pending_attach
        .as_ref()
        .map(|(_, s, _)| s.id.clone())
        .or_else(|| a.attach_check.as_ref().map(|(_, s, _, _)| s.id.clone()));
    assert_eq!(attached.as_deref(), Some("nav-0-01"));
    a.pending_attach = None;
    a.attach_check = None;
    // Up at the top stays; down past the last session lands on the new tile.
    a.open_device(0);
    a.key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
    assert_eq!(
        a.selected_session().unwrap().1.id,
        "nav-0-00",
        "Up at the top wrapped"
    );
    for _ in 0..5 {
        a.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    }
    assert!(
        a.selected_session().is_none(),
        "the new-session tile ends the row"
    );
}

/// Every session on a device holds its own key (19 of them, home row first; `a` and `o`
/// start agents) and the key attaches exactly that session; a session keeps its key
/// while others end and a new one takes the freed key; the 20th has none and a digit
/// nobody holds stays a count.
#[test]
fn every_session_key_attaches_its_session_and_keys_are_stable() {
    let (mut a, _rx) = queued_app();
    sessions(&mut a, 0, 21);
    let keys: Vec<(String, Option<char>)> = a
        .slot_keys(0)
        .into_iter()
        .map(|(s, k)| (s.id.clone(), k))
        .collect();
    let given: String = keys.iter().filter_map(|(_, k)| *k).collect();
    assert_eq!(given, "sdgquiyzvm123456789");
    assert_eq!(keys.iter().filter(|(_, k)| k.is_none()).count(), 2);
    for (id, k) in keys.iter().filter_map(|(id, k)| k.map(|k| (id, k))) {
        a.open_device(0);
        press(&mut a, k);
        let attached = a
            .pending_attach
            .take()
            .map(|(_, s, _)| s.id)
            .or_else(|| a.attach_check.take().map(|(_, s, _, _)| s.id));
        assert_eq!(attached.as_deref(), Some(id.as_str()), "key {k}");
    }
    // `d` ends: the others keep theirs; the first without a key takes `d`.
    let s_id = keys[1].0.clone();
    a.work[0].sessions.retain(|s| s.id != s_id);
    let after: Vec<(String, Option<char>)> = a
        .slot_keys(0)
        .into_iter()
        .map(|(s, k)| (s.id.clone(), k))
        .collect();
    for (id, k) in &keys {
        if *id != s_id && k.is_some() {
            assert!(after.contains(&(id.clone(), *k)), "{id} kept {k:?}");
        }
    }
    let heir = &keys.iter().find(|(_, k)| k.is_none()).unwrap().0;
    assert!(after.contains(&(heir.clone(), Some('d'))));
}

/// A device with nothing running: a session that ended while cx watched is its resume
/// row (`s` continues the agent's latest conversation in that folder) and recent
/// folders follow on `d g ...` (each opens the launch chooser there); taps do the same.
#[test]
fn an_ended_session_resumes_and_recent_folders_start_with_one_key() {
    let (mut a, rx) = queued_app();
    sessions(&mut a, 0, 1);
    a.work[0].sessions[0].provider = "claude".into();
    a.work[0].sessions[0].directory = "/srv/proj".into();
    a.file_locations.insert(0, "/srv/notes".into());
    a.apply(Reply {
        device_id: Some(a.devices[0].id.clone()),
        execution_target: None,
        device: 0,
        op: Operation::Sessions,
        generation: a.generation,
        preview: None,
        result: Ok(serde_json::json!([])),
    });
    a.open_device(0);
    let text = draw(&a, 120, 40);
    assert!(text.contains("nothing running"), "{text}");
    let row = |w: &str| {
        text.lines()
            .find(|l| l.contains(w))
            .unwrap_or("")
            .to_string()
    };
    assert!(
        row("resume").contains(" s ") && row("resume").contains("/srv/proj"),
        "{text}"
    );
    assert!(
        row("recent").contains(" d ") && row("recent").contains("/srv/notes"),
        "{text}"
    );
    let _ = rx.try_iter().count();
    press(&mut a, 's');
    assert!(rx.try_iter().any(|t| matches!(&t.op,
        Operation::Create(c) if c.resume && !c.resume_pick && c.directory == "/srv/proj" && c.provider == "claude")));
    a.creating = false;
    a.open_device(0);
    press(&mut a, 'd');
    assert!(matches!(&a.dialog, Some(Dialog::SessionChooser(0, p)) if p == "/srv/notes"));
    a.dialog = None;
    // The row's tap is its key.
    draw(&a, 120, 40);
    let r = act("launch:0:d");
    click(&mut a, r.x + 1, r.y, Rect::new(0, 0, 120, 40));
    assert!(matches!(&a.dialog, Some(Dialog::SessionChooser(0, p)) if p == "/srv/notes"));
}

/// Attaching from a tile plays the expand from where the tile was drawn: the loop
/// keeps drawing frames for it (and no longer once it is done), any key ends it, and
/// reduced motion never starts one; a detach plays the shrink back into the tile.
#[test]
fn attaching_from_a_tile_expands_it_and_a_key_skips_it() {
    let (mut a, _rx) = queued_app();
    sessions(&mut a, 0, 2);
    a.open_device(0);
    draw(&a, 120, 40);
    press(&mut a, 'd');
    let e = a.expand.clone().expect("the tile grows");
    assert!(e.grow && e.from.is_some_and(|r| r.width < 120), "{e:?}");
    let t0 = e.start;
    let (id, tile, t) = a.expand_frame(t0).unwrap();
    assert_eq!((tile, t), (e.from.unwrap(), 0.0));
    let area = Rect::new(0, 0, 120, 40);
    assert!(a.attach_waits(t0, area) && !a.attach_waits(e.lands_at(area), area));
    assert_eq!(id, workspaces::runtime_key(a.slot_sessions(0)[1]));
    assert!(runtime::loop_wait(&a, t0, None) <= runtime::FRAME_INTERVAL);
    let done = t0 + motion::EXPAND;
    assert!(!a.expanding(done) && a.expand_frame(done).is_none());
    // A key ends it at once.
    a.key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));
    assert!(a.expand.is_none());
    // The shrink after a detach finds its tile on the frame it draws.
    let s = a.slot_sessions(0)[1].clone();
    a.start_expand(0, &s, false);
    let e = a.expand.clone().unwrap();
    assert!(!e.grow && e.from.is_none());
    let (_, _, t) = a.expand_frame(e.start).unwrap();
    assert_eq!(t, 1.0, "the shrink starts at full screen");
    assert!(
        a.expand_frame(Instant::now()).unwrap().2 < 0.7,
        "its first drawn frame shows the hub"
    );
    // Reduced motion: nothing plays.
    a.expand = None;
    a.motion = motion::Clock::new(true);
    a.start_expand(0, &s, true);
    assert!(a.expand.is_none());
}

#[test]
fn nodes_tiles_and_chips_are_tap_targets_with_key_semantics() {
    let (mut a, _rx) = queued_app();
    sessions(&mut a, 0, 3);
    a.go_hub();
    let area = Rect::new(0, 0, 120, 40);
    // A node opens its device, like its letter.
    draw(&a, 120, 40);
    let node = act("device:1");
    click(&mut a, node.x + 1, node.y, area);
    assert!(a.view == View::Work && a.device == 2 && a.dialog.is_none());
    // The footer's `H hub` chip, then `? help`.
    draw(&a, 120, 40);
    let hub = act("hub");
    click(&mut a, hub.x, hub.y, area);
    assert!(a.view == View::Network);
    draw(&a, 120, 40);
    let help = act("help");
    click(&mut a, help.x, help.y, area);
    assert!(a.help);
    a.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    // `/ find` opens the palette; a tap beside the sheet closes it.
    draw(&a, 120, 40);
    let find = act("palette");
    click(&mut a, find.x, find.y, area);
    assert!(a.input == Some(Input::Palette));
    draw(&a, 120, 40);
    click(&mut a, 0, 0, area);
    assert!(a.input.is_none());
    // A tap focuses a session tile; a tap on the focused tile attaches it (as `⏎`).
    draw(&a, 120, 40);
    let key = workspaces::runtime_key(&a.work[0].sessions[1]);
    let tile = act(&format!("tile:0:{key}"));
    click(&mut a, tile.x + 2, tile.y + 1, area);
    assert!(a.pending_attach.is_none() && a.attach_check.is_none());
    assert!(a.hub_sessions);
    assert_eq!(
        a.selected_session()
            .map(|(_, s)| workspaces::runtime_key(&s)),
        Some(key.clone())
    );
    draw(&a, 120, 40);
    let tile = act(&format!("tile:0:{key}"));
    click(&mut a, tile.x + 2, tile.y + 1, area);
    assert!(a.pending_attach.is_some() || a.attach_check.is_some());
}

#[test]
fn right_click_and_long_press_open_the_keymap_context_menu() {
    let (mut a, _rx) = queued_app();
    sessions(&mut a, 0, 3);
    a.open_device(0);
    let area = Rect::new(0, 0, 120, 40);
    draw(&a, 120, 40);
    let key = workspaces::runtime_key(&a.work[0].sessions[2]);
    let tile = act(&format!("tile:0:{key}"));
    a.mouse(
        MouseEvent {
            kind: MouseEventKind::Down(event::MouseButton::Right),
            column: tile.x + 2,
            row: tile.y,
            modifiers: KeyModifiers::NONE,
        },
        area,
    );
    assert_eq!(a.selected_session().unwrap().1.id, "nav-0-02");
    let menu = a.context_menu.as_ref().expect("right-click menu");
    let cmds: Vec<_> = menu.items.iter().map(|i| i.2).collect();
    assert!(
        cmds.contains(&Cmd::Watch)
            && cmds.contains(&Cmd::Pin)
            && cmds.contains(&Cmd::View("attach"))
    );
    let text = draw(&a, 120, 40);
    // Short chip labels, so rows fit the box.
    assert!(text.contains("Attach") && text.contains("Claude"), "{text}");
    a.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(a.context_menu.is_none() && a.dialog.is_none());
    // Long press (touch): hold the tile past the threshold.
    draw(&a, 120, 40);
    let key = workspaces::runtime_key(&a.work[0].sessions[1]);
    let tile = act(&format!("tile:0:{key}"));
    let down = MouseEvent {
        kind: MouseEventKind::Down(event::MouseButton::Left),
        column: tile.x + 2,
        row: tile.y + 1,
        modifiers: KeyModifiers::NONE,
    };
    assert!(a.mouse(down, area));
    let deadline = a.press_deadline().unwrap();
    assert!(!a.tick_press(deadline - Duration::from_millis(1)));
    assert!(a.tick_press(deadline));
    assert!(a.context_menu.is_some());
    // Releasing after a long press does not also attach.
    a.mouse(
        MouseEvent {
            kind: MouseEventKind::Up(event::MouseButton::Left),
            ..down
        },
        area,
    );
    assert!(a.attach_check.is_none() && a.pending_attach.is_none());
    // Choosing a menu row runs its command.
    draw(&a, 120, 40);
    let pin = a
        .context_menu
        .as_ref()
        .unwrap()
        .items
        .iter()
        .position(|i| i.2 == Cmd::Pin)
        .unwrap();
    let rect = target(Hit::MenuRow(pin));
    let pins = a.pins.len();
    click(&mut a, rect.x, rect.y, area);
    assert!(a.context_menu.is_none());
    assert_ne!(a.pins.len(), pins, "menu Pin did not pin");
}

#[test]
fn hover_and_motion_never_select_or_redraw() {
    let (mut a, _rx) = queued_app();
    sessions(&mut a, 0, 5);
    a.open_device(0);
    draw(&a, 120, 40);
    let key = workspaces::runtime_key(&a.work[0].sessions[3]);
    let tile = act(&format!("tile:0:{key}"));
    for kind in [MouseEventKind::Moved, MouseEventKind::Moved] {
        assert!(!a.mouse(
            MouseEvent {
                kind,
                column: tile.x + 1,
                row: tile.y,
                modifiers: KeyModifiers::NONE,
            },
            Rect::new(0, 0, 120, 40),
        ));
    }
    assert_eq!(a.selected_session().unwrap().1.id, "nav-0-00");
}

#[test]
fn palette_switcher_ranks_fuzzy_then_recent_and_attaches_on_enter() {
    let (mut a, rx) = queued_app();
    sessions(&mut a, 0, 2);
    let mut robot = disposable_shell();
    robot.id = "robot-logs".into();
    robot.name = "robot logs".into();
    a.work[1].sessions = vec![robot];
    a.work[1].fetched = transport::now();
    a.view = View::Files;
    a.device = 1; // scoped to the local device: the switcher still spans all
    a.key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL));
    assert!(a.input == Some(Input::Palette));
    for c in "robot".chars() {
        press(&mut a, c);
    }
    let view = a.palette_view();
    assert_eq!(view[0].section, "Sessions");
    assert_eq!(view[0].label, "robot logs");
    let text = draw(&a, 80, 24);
    assert!(
        text.contains("robot logs") && text.contains("SESSIONS"),
        "{text}"
    );
    a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(a.input.is_none());
    assert!(a.view == View::Work);
    assert_eq!(a.selected_session().unwrap().1.id, "robot-logs");
    assert!(
        a.attach_check.is_some() || rx.try_iter().any(|t| matches!(t.op, Operation::Sessions)),
        "Enter on a session did not attach"
    );
    // Recency: an empty query lists the last pick first.
    a.attach_check = None;
    a.key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL));
    assert_eq!(a.palette_view()[0].id, a.palette_mru[0]);
    // Goto entries go there with no chooser.
    for c in "hub".chars() {
        press(&mut a, c);
    }
    let index = a
        .palette_view()
        .iter()
        .position(|i| i.section == "Go to")
        .unwrap();
    a.palette_selected = index;
    a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(a.view == View::Network && a.dialog.is_none());
}

#[test]
fn palette_tab_jumps_sections_and_is_one_box() {
    let (mut a, _rx) = queued_app();
    sessions(&mut a, 0, 2);
    a.key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL));
    let first = a.palette_view()[a.palette_selected].section;
    a.key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    let second = a.palette_view()[a.palette_selected].section;
    assert_ne!(first, second);
    a.key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT));
    assert_eq!(a.palette_view()[a.palette_selected].section, first);
    let text = draw(&a, 80, 24);
    // A single border box: exactly one top-left corner inside the overlay rows.
    let corners = text.matches('╭').count() + text.matches('+').count();
    assert!(corners >= 1);
    assert!(
        !text.contains("Actions · type to filter"),
        "stacked palette boxes remain"
    );
}

#[test]
fn help_and_footer_teach_the_design_keys_and_each_chip_runs_its_key() {
    let (mut a, _rx) = queued_app();
    sessions(&mut a, 0, 2);
    a.go_hub();
    a.help = true;
    let text = draw(&a, 120, 40);
    for key in [
        "ON THE HUB",
        "s d g q",
        "c x n",
        "ctrl+\\",
        "find",
        "secrets",
    ] {
        assert!(text.contains(key), "help lacks {key}: {text}");
    }
    a.help = false;
    // Footer chips are tap targets running the same verb as their key.
    draw(&a, 120, 40);
    let last = act("last");
    let zone = act("zone");
    assert_eq!(last.y, zone.y, "both on the footer row");
    click(&mut a, zone.x, zone.y, Rect::new(0, 0, 120, 40));
    assert!(a.hub_sessions, "tab chip switched zones like Tab");
    // The keymap still names the device verbs (context menus, `?`).
    a.open_device(0);
    for (key, cmd) in [('w', Cmd::Watch), ('p', Cmd::Pin), ('?', Cmd::Help)] {
        assert_eq!(
            keymap::resolve(&a, &KeyEvent::new(KeyCode::Char(key), KeyModifiers::NONE)),
            Some(cmd)
        );
    }
}

#[test]
fn phone_hub_is_the_device_tree_with_a_wrapping_footer() {
    let (mut a, _rx) = queued_app();
    sessions(&mut a, 0, 3);
    a.go_hub();
    let text = draw(&a, 40, 20);
    let lines: Vec<_> = text.lines().collect();
    assert!(lines[0].contains("cx"), "{text}");
    // (No `NETWORK` header strip: the tree starts under the top bar.)
    assert!(
        !text.contains("NETWORK") && text.contains("laptop"),
        "{text}"
    );
    // Phone keys are words: `ret`, never a missing glyph.
    assert!(text.contains("ret"), "{text}");
    // Each tree row is the device's tap target, three rows by seven columns at least.
    draw(&a, 40, 20);
    let row = act("device:1");
    assert!(row.width >= 7 && row.height >= 1, "{row:?}");
    click(&mut a, row.x + 1, row.y, Rect::new(0, 0, 40, 20));
    assert!(a.view == View::Work && a.device == 2);
}

#[test]
fn idle_frames_send_nothing_and_cursor_changes_send_only_the_cursor() {
    use ratatui::backend::Backend;
    // What ratatui's crossterm backend writes after any frame's cells.
    let mut resets = Vec::new();
    {
        let mut backend = ratatui::backend::CrosstermBackend::new(&mut resets);
        backend.draw(std::iter::empty()).unwrap();
    }
    assert!(!resets.is_empty());
    let mut tail = Vec::new();
    let with = |prefix: &[u8], suffix: &[u8]| {
        let mut v = prefix.to_vec();
        v.extend_from_slice(&resets);
        v.extend_from_slice(suffix);
        v
    };
    // A frame with cell updates is sent whole.
    let frame = with(b"\x1b[3;4Hx", b"\x1b[?25l");
    assert_eq!(frame_payload(&frame, &mut tail).unwrap(), frame);
    // No cell updates and the same cursor state: nothing at all.
    let idle = with(b"", b"\x1b[?25l");
    assert!(frame_payload(&idle, &mut tail).is_none());
    assert!(frame_payload(&idle, &mut tail).is_none());
    // Cursor moved (input field): only the cursor bytes.
    let moved = with(b"", b"\x1b[?25h\x1b[5;6H");
    assert_eq!(
        frame_payload(&moved, &mut tail).unwrap(),
        b"\x1b[?25h\x1b[5;6H"
    );
    assert!(frame_payload(&moved, &mut tail).is_none());
}

/// A terminal for what `encode` emits: CUP, CHA, ECH and SGR. Cells keep the
/// symbol and the SGR state they were written with (as crossterm spells it).
struct Replay {
    width: u16,
    cells: Vec<(String, [String; 3], Modifier)>,
}
impl Replay {
    fn new(width: u16, height: u16) -> Self {
        let blank = (" ".to_string(), Default::default(), Modifier::empty());
        Replay {
            width,
            cells: vec![blank; width as usize * height as usize],
        }
    }
    fn feed(&mut self, bytes: &[u8]) {
        let text = String::from_utf8(bytes.to_vec()).unwrap();
        let (mut x, mut y, mut style, mut modifier) =
            (0u16, 0u16, <[String; 3]>::default(), Modifier::empty());
        let mut rest = text.as_str();
        while !rest.is_empty() {
            if let Some(seq) = rest.strip_prefix("\x1b[") {
                let end = seq.find(|c: char| c.is_ascii_alphabetic()).unwrap();
                let (params, cmd) = (&seq[..end], &seq[end..=end]);
                let n = |i: usize| {
                    params
                        .split(';')
                        .nth(i)
                        .and_then(|v| v.parse::<u16>().ok())
                        .unwrap_or(1)
                };
                match cmd {
                    "H" => (y, x) = (n(0) - 1, n(1) - 1),
                    "G" => x = n(0) - 1,
                    "X" => {
                        for k in x..x + n(0) {
                            self.cells[(y * self.width + k) as usize] = (
                                " ".into(),
                                [String::new(), style[1].clone(), String::new()],
                                Modifier::empty(),
                            );
                        }
                    }
                    "m" => {
                        let mut parts = params.split(';').peekable();
                        while let Some(p) = parts.next() {
                            let color = |parts: &mut std::iter::Peekable<std::str::Split<char>>,
                                         head: &str| {
                                let k = if parts.peek() == Some(&"5") { 2 } else { 4 };
                                std::iter::once(head.to_string())
                                    .chain(parts.take(k).map(str::to_string))
                                    .collect::<Vec<_>>()
                                    .join(";")
                            };
                            match p {
                                "38" => style[0] = color(&mut parts, p),
                                "48" => style[1] = color(&mut parts, p),
                                "58" => style[2] = color(&mut parts, p),
                                "39" => style[0] = String::new(),
                                "49" => style[1] = String::new(),
                                "59" => style[2] = String::new(),
                                "0" => (style, modifier) = (Default::default(), Modifier::empty()),
                                "1" => modifier |= Modifier::BOLD,
                                "2" => modifier |= Modifier::DIM,
                                "3" => modifier |= Modifier::ITALIC,
                                "4" => modifier |= Modifier::UNDERLINED,
                                "7" => modifier |= Modifier::REVERSED,
                                "9" => modifier |= Modifier::CROSSED_OUT,
                                "22" => modifier -= Modifier::BOLD | Modifier::DIM,
                                "23" => modifier -= Modifier::ITALIC,
                                "24" => modifier -= Modifier::UNDERLINED,
                                "27" => modifier -= Modifier::REVERSED,
                                "29" => modifier -= Modifier::CROSSED_OUT,
                                other => panic!("unexpected SGR {other}"),
                            }
                        }
                    }
                    other => panic!("unexpected CSI {other}"),
                }
                rest = &seq[end + 1..];
            } else {
                let end = rest.find('\x1b').unwrap_or(rest.len());
                for g in ratatui::text::Span::raw(&rest[..end]).styled_graphemes(Style::default()) {
                    self.cells[(y * self.width + x) as usize] =
                        (g.symbol.to_string(), style.clone(), modifier);
                    x += ratatui::text::Span::raw(g.symbol).width() as u16;
                }
                rest = &rest[end..];
            }
        }
    }
    /// Every cell matches `want` (cells under a wide symbol aside; a blank cell's
    /// foreground does not show).
    fn assert_shows(&self, want: &Buffer) {
        fn spelled(c: Color, command: impl crossterm::Command) -> String {
            let mut out = String::new();
            if c != Color::Reset {
                command.write_ansi(&mut out).unwrap();
            }
            out.trim_start_matches("\x1b[")
                .trim_end_matches('m')
                .to_string()
        }
        let width = |s: &str| ratatui::text::Span::raw(s).width();
        let mut covered = false;
        for (i, (cell, got)) in want.content.iter().zip(&self.cells).enumerate() {
            if !std::mem::replace(&mut covered, width(cell.symbol()) > 1) {
                let style = [
                    spelled(
                        cell.fg,
                        crossterm::style::SetForegroundColor(cell.fg.into()),
                    ),
                    spelled(
                        cell.bg,
                        crossterm::style::SetBackgroundColor(cell.bg.into()),
                    ),
                    spelled(
                        cell.underline_color,
                        crossterm::style::SetUnderlineColor(cell.underline_color.into()),
                    ),
                ];
                let blank = cell.symbol() == " " && cell.modifier.is_empty();
                assert_eq!(got.0, cell.symbol(), "symbol at {i}");
                assert_eq!(got.1[1], style[1], "background at {i}");
                assert_eq!(got.2, cell.modifier, "modifiers at {i}");
                if !blank {
                    assert_eq!(got.1, style, "colours at {i}");
                }
            }
        }
    }
}

#[test]
fn encoded_frames_show_exactly_the_buffer_in_fewer_bytes() {
    use ratatui::backend::Backend as _;
    use ratatui::style::Stylize;
    let area = Rect::new(0, 0, 40, 6);
    // Varied rows: colour and attribute changes, wide symbols, gaps and long blanks.
    let frame = |seed: u64| {
        let mut buffer = Buffer::empty(area);
        let mut state = seed;
        let mut next = || {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (state >> 33) as usize
        };
        for y in 0..area.height {
            let mut x = next() % 6;
            while x < 36 {
                let word = ["ab", "漢字", "│ ", "x", "  ", "▌n"][next() % 6];
                let style = [
                    Style::new(),
                    Style::new().blue(),
                    Style::new().fg(Color::Indexed(8)).bold(),
                    Style::new().fg(Color::Rgb(1, 2, 3)).on_red(),
                    Style::new()
                        .dim()
                        .underlined()
                        .underline_color(Color::Green),
                    Style::new().reversed().italic(),
                ][next() % 6];
                buffer.set_string(x as u16, y, word, style);
                x += word.chars().count() + next() % 9;
            }
        }
        buffer
    };
    for seed in 0..60 {
        let (before, after) = (frame(seed), frame(seed + 1000));
        let mut screen = Replay::new(area.width, area.height);
        let mut first = Vec::new();
        encode(&mut first, Buffer::empty(area).diff(&before)).unwrap();
        screen.feed(&first);
        screen.assert_shows(&before);
        let (mut ours, mut theirs) = (Vec::new(), Vec::new());
        encode(&mut ours, before.diff(&after)).unwrap();
        ratatui::backend::CrosstermBackend::new(&mut theirs)
            .draw(before.diff(&after).into_iter())
            .unwrap();
        assert!(
            ours.len() <= theirs.len(),
            "seed {seed}: {} > {}",
            ours.len(),
            theirs.len()
        );
        screen.feed(&ours);
        screen.assert_shows(&after);
    }
    // A long blank run is erased in place, a jump along the row moves by column.
    let mut wide = Buffer::empty(area);
    wide.set_string(0, 0, "x".repeat(30), Style::new());
    let mut bytes = Vec::new();
    encode(&mut bytes, wide.diff(&Buffer::empty(area))).unwrap();
    assert!(bytes.starts_with(b"\x1b[1;1H\x1b[30X"), "{bytes:?}");
    let mut gap = Buffer::empty(area);
    gap.set_string(2, 1, "a", Style::new());
    gap.set_string(9, 1, "b", Style::new());
    bytes.clear();
    encode(&mut bytes, Buffer::empty(area).diff(&gap)).unwrap();
    assert!(bytes.starts_with(b"\x1b[2;3Ha\x1b[10Gb"), "{bytes:?}");
}

#[test]
fn secrets_render_everywhere_and_never_move_the_session_selection() {
    let (mut a, _rx) = queued_app();
    a.view = View::Secrets;
    for (w, h) in [(40, 20), (80, 24), (120, 40), (200, 60)] {
        let text = draw(&a, w, h);
        assert!(
            text.contains("SECRETS") || text.contains("secrets"),
            "{w}x{h}: {text}"
        );
    }
    // Moving in Secrets does not touch the session selection.
    sessions(&mut a, 0, 4);
    a.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    assert_eq!(a.selected, 0);
}

#[test]
fn colon_runs_in_every_view_even_with_a_file_search_active() {
    // Regression: `:` was gated on any file search, so it silently did
    // nothing after `/` in Files or after switching to Sessions.
    let (mut a, _rx) = file_app();
    a.focus = Focus::Workspace;
    a.browser.as_mut().unwrap().search = "alpha".into();
    press(&mut a, ':');
    assert!(a.palette_run_mode(), "Files + search");
    a.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(a.input.is_none() && a.command_target.is_none());
    a.open_device(0);
    assert!(a.view == View::Work);
    press(&mut a, ':');
    assert!(a.palette_run_mode(), "a device after search");
}

#[test]
fn colon_is_the_palette_in_run_mode_and_ctrl_p_never_lists_run() {
    let (mut a, _rx) = file_app();
    a.focus = Focus::Workspace;
    press(&mut a, ':');
    assert_eq!(a.text, "run: ");
    let view = a.palette_view();
    assert_eq!(view.len(), 1);
    assert_eq!(view[0].section, "Run command");
    for c in "echo hi".chars() {
        press(&mut a, c);
    }
    let text = draw(&a, 120, 40);
    assert!(
        text.contains("run: echo hi") && text.contains("RUN COMMAND"),
        "{text}"
    );
    assert!(text.contains("/files"), "run row shows its folder: {text}");
    a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    let (device, command) = a.pending_command_panel.take().expect("dispatched");
    assert_eq!((device, command.command.as_str()), (0, "echo hi"));
    assert_eq!(command.directory, "/files");
    assert!(a.input.is_none() && a.command_target.is_none());
    // Backspacing past the prefix turns it back into the ordinary palette.
    press(&mut a, ':');
    for _ in 0..5 {
        a.key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
    }
    assert!(!a.palette_run_mode() && a.input == Some(Input::Palette));
    assert!(a.palette_view().iter().all(|i| i.section != "Run command"));
    // Ctrl+P keeps the user's direction: no run-command entry.
    a.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    a.key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL));
    assert!(a
        .palette_view()
        .iter()
        .all(|i| i.target != super::super::actions::PaletteTarget::Run));
}

#[test]
fn palette_fits_its_results_aligns_details_and_dims_the_backdrop() {
    let (mut a, _rx) = queued_app();
    sessions(&mut a, 0, 2);
    a.file_locations.insert(0, "/home/user/projects/nav".into());
    let base = draw(&a, 120, 40);
    a.key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL));
    for c in "nav".chars() {
        press(&mut a, c);
    }
    let items = a.palette_view();
    let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
    terminal.draw(|f| render(f, &a)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    let text = draw(&a, 120, 40);
    // Every result fits and shows; a section heading names each group.
    assert!(text.contains("/ nav"), "{text}");
    for item in &items {
        assert!(
            text.contains(&item.label),
            "{} missing:\n{text}",
            item.label
        );
    }
    assert!(text.contains("SESSIONS"), "{text}");
    assert!(!text.contains("more ↓"), "nothing cut: {text}");
    // The backdrop is faded: the top bar's bold `cx` loses its weight.
    let x = base.lines().next().unwrap().find("cx").unwrap() as u16;
    let mut plain = Terminal::new(TestBackend::new(120, 40)).unwrap();
    a.input = None;
    plain.draw(|f| render(f, &a)).unwrap();
    assert!(plain.backend().buffer()[(x, 0)]
        .modifier
        .contains(Modifier::BOLD));
    assert!(!buffer[(x, 0)].modifier.contains(Modifier::BOLD));
}

#[test]
fn help_covers_the_frame_below_100_columns() {
    let (mut a, _rx) = queued_app();
    sessions(&mut a, 0, 3);
    a.go_hub();
    a.help = true;
    let text = draw(&a, 80, 24);
    let lines: Vec<_> = text.lines().collect();
    assert!(lines[0].contains("cx › help"), "{text}");
    // Help owns the frame: nothing of the hub bleeds around it.
    assert!(
        !text.contains("NETWORK") && !text.contains("SESSIONS"),
        "{text}"
    );
    assert!(text.contains("esc") && text.contains("close"), "{text}");
}

#[test]
fn context_menu_rows_use_short_labels() {
    let (mut a, _rx) = queued_app();
    sessions(&mut a, 0, 2);
    a.open_device(0);
    let labels: Vec<_> = keymap::context_menu(&a)
        .into_iter()
        .map(|(_, label, _)| label)
        .collect();
    assert!(labels.contains(&"Agent tasks"), "{labels:?}");
    assert!(labels.iter().all(|l| l.chars().count() <= 24), "{labels:?}");
}

#[test]
fn palette_spans_the_frame_below_100_columns() {
    let (mut a, _rx) = queued_app();
    sessions(&mut a, 0, 3);
    a.open_palette("");
    // Phones: a full sheet on a cleared backdrop, its chips on the last row.
    let text = draw(&a, 40, 20);
    assert!(!text.contains("NETWORK"), "{text}");
    assert!(text.lines().nth(2).unwrap().contains("/ "), "{text}");
    assert!(text.lines().last().unwrap().contains("esc"), "{text}");
}

#[test]
fn palette_query_ranks_on_labels_and_ignores_scattered_path_letters() {
    let (mut a, _rx) = queued_app();
    let mut watcher = disposable_shell();
    watcher.id = "watcher".into();
    watcher.name = "build watcher".into();
    watcher.directory = "/home/tester/projects/robot-logs".into();
    let mut core = disposable_shell();
    core.id = "core".into();
    core.name = "ros core".into();
    a.work[0].sessions = vec![watcher, core];
    a.work[0].fetched = transport::now();
    a.key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL));
    for c in "ros".chars() {
        press(&mut a, c);
    }
    let labels: Vec<_> = a.palette_view().into_iter().map(|i| i.label).collect();
    assert_eq!(
        labels.first().map(String::as_str),
        Some("ros core"),
        "{labels:?}"
    );
    // 'r..o..s' scattered across "projects/robot-logs" is not a match.
    assert!(!labels.iter().any(|l| l == "build watcher"), "{labels:?}");
    // A contiguous detail match (the device name) still finds the session.
    a.text.clear();
    for c in "robot-logs".chars() {
        press(&mut a, c);
    }
    let labels: Vec<_> = a.palette_view().into_iter().map(|i| i.label).collect();
    assert!(labels.iter().any(|l| l == "build watcher"), "{labels:?}");
}

#[test]
fn a_pending_reply_never_makes_the_event_loop_poll() {
    // Replies wake the loop through the shared channel; the wait stays idle.
    let (mut a, _rx) = queued_app();
    a.view = View::Files;
    let now = Instant::now();
    let idle = loop_wait(&a, now, None);
    assert!(idle >= Duration::from_millis(50), "{idle:?}");
    assert!(a.send(0, Operation::Sessions));
    assert_eq!(loop_wait(&a, now, None), idle);
}

/// `+`'s screen with a stand-in for the listener thread: the invite shows its code, the
/// `cx join` one-liner, the other addresses it listens on and a QR code; `y` / `Y` (or a tap)
/// copy the code / the command; Esc drops it, which tells the listener to close.
#[test]
fn the_hub_invite_shows_its_code_and_esc_closes_its_listener() {
    use std::sync::{atomic::AtomicBool, atomic::Ordering, Arc};
    let (mut a, _rx) = queued_app();
    a.go_hub();
    let (tx, rx) = mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    a.invite = Some(screens::InviteRun {
        rx,
        cancel: cancel.clone(),
        shown: None,
        drawn_secs: Default::default(),
    });
    assert!(draw(&a, 120, 40).contains("Opening an invite"));
    tx.send(Ok(screens::InviteEvent::Ready(crate::mesh::Invite {
        backend: "ssh-overlay".into(),
        code: "7K2Q-M9XD-4HT".into(),
        one_liner_sh: "cx join 7K2Q-M9XD-4HT 192.168.1.5".into(),
        expires: "2026-11-11T23:30:00Z".into(),
        uses: 1,
        hosts: vec!["192.168.1.5".into(), "100.101.2.3".into()],
        ..Default::default()
    })))
    .unwrap();
    assert!(a.poll_invite());
    let text = draw(&a, 120, 40);
    assert!(text.contains("cx join 7K2Q-M9XD-4HT 192.168.1.5"), "{text}");
    assert!(text.contains("also listening on 100.101.2.3"), "{text}");
    assert!(
        text.contains("7 K 2 Q") && text.contains("expires in 29m"),
        "{text}"
    );
    assert!(text.contains('▀'), "a QR code: {text}");
    let vm = viewmodel::ViewModel::from(&a);
    let qr = &vm.state.invite.as_ref().unwrap().qr;
    assert!(
        qr.len() >= 21 && qr.iter().all(|r| r.len() == qr.len()),
        "a real QR matrix"
    );
    press(&mut a, 'y');
    assert_eq!(a.copy_out.take().as_deref(), Some("7K2Q-M9XD-4HT"));
    assert!(
        a.notice
            .contains("if your terminal allows clipboard access"),
        "{}",
        a.notice
    );
    a.key(KeyEvent::new(KeyCode::Char('Y'), KeyModifiers::SHIFT));
    assert_eq!(
        a.copy_out.take().as_deref(),
        Some("cx join 7K2Q-M9XD-4HT 192.168.1.5")
    );
    a.view_action("copy-code");
    assert_eq!(a.copy_out.take().as_deref(), Some("7K2Q-M9XD-4HT"));
    a.view_action("copy-cmd");
    assert!(a.copy_out.take().is_some_and(|c| c.starts_with("cx join ")));
    assert!(a.invite.is_some(), "copying keeps the invite open");
    a.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(a.invite.is_none());
    assert!(
        cancel.load(Ordering::Relaxed),
        "the listener was told to close"
    );
    // A join closes the screen and says who joined.
    let (tx, rx) = mpsc::channel();
    a.invite = Some(screens::InviteRun {
        rx,
        cancel: Arc::new(AtomicBool::new(false)),
        shown: None,
        drawn_secs: Default::default(),
    });
    tx.send(Ok(screens::InviteEvent::Joined(
        serde_json::json!({"name": "rover"}),
    )))
    .unwrap();
    assert!(a.poll_invite() && a.invite.is_none());
    assert!(draw(&a, 120, 40).contains("rover joined"));
}

/// The keys screen's entry: the value is never drawn, only counted; Esc wipes it.
#[test]
fn a_secret_value_is_typed_masked_and_never_drawn() {
    let (mut a, _rx) = queued_app();
    a.view = View::Secrets;
    // The Cloudflare token's missing card (the first): its slot key opens the entry.
    press(&mut a, 's');
    assert!(a.secret_entry.as_ref().is_some_and(|e| !e.naming));
    for c in "hunter2-s3cret".chars() {
        press(&mut a, c);
    }
    a.secret_paste("-pasted\n");
    let text = draw(&a, 100, 30);
    assert!(
        !text.contains("hunter2") && !text.contains("pasted"),
        "{text}"
    );
    assert!(
        text.contains("•••••") && text.contains("21 characters"),
        "{text}"
    );
    a.key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
    assert_eq!(a.secret_entry.as_ref().unwrap().value.len(), 20);
    a.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(
        a.secret_entry.is_none() && a.secret_job.is_none(),
        "nothing saved"
    );
    // `h`: a new secret, its name first (validated), then its value.
    press(&mut a, 'h');
    for c in "bad name".chars() {
        press(&mut a, c);
    }
    a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(
        a.secret_entry.as_ref().unwrap().naming,
        "an invalid name is refused"
    );
    assert!(
        draw(&a, 100, 30).contains("bad name"),
        "the name is shown as typed"
    );
}

/// The keys screen's `e` exposes the selected secret to new sessions here (off by default) and
/// `v` edits its variable; the card says which variable, never the value.
#[test]
fn e_exposes_a_secret_to_new_sessions_and_v_names_its_variable() {
    let (mut a, _rx) = queued_app();
    let name = "ui.expose-test";
    let store = crate::secrets::Secrets::at(store::ensure().unwrap(), None);
    store.set(name, "ui-expose-canary").unwrap();
    a.open_secrets();
    a.secret_selected = a.secret_list.iter().position(|m| m.name == name).unwrap();
    let meta = |a: &App| {
        a.secret_list
            .iter()
            .find(|m| m.name == name)
            .cloned()
            .unwrap()
    };
    assert!(!meta(&a).expose, "off by default");
    press(&mut a, 'e');
    assert!(meta(&a).expose && store.meta(name).unwrap().unwrap().expose);
    let text = draw(&a, 120, 40);
    assert!(text.contains("new sessions as $UI_EXPOSE_TEST"), "{text}");
    press(&mut a, 'v');
    assert_eq!(
        a.secret_entry.as_ref().unwrap().var.as_deref(),
        Some("UI_EXPOSE_TEST")
    );
    a.key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
    for c in "MY_TOKEN".chars() {
        press(&mut a, c);
    }
    assert!(draw(&a, 120, 40).contains("$MY_TOKEN"));
    a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(a.secret_entry.is_none());
    assert_eq!(meta(&a).env.as_deref(), Some("MY_TOKEN"));
    assert!(meta(&a).expose, "renaming the variable keeps the choice");
    let text = draw(&a, 120, 40);
    assert!(
        text.contains("$MY_TOKEN") && !text.contains("ui-expose-canary"),
        "{text}"
    );
    press(&mut a, 'e');
    assert!(!meta(&a).expose);
    store.delete(name).unwrap();
}

/// After a detach the left session is selected on the hub even when it ranks below the
/// tile row: its tile takes the row's last cell.
#[test]
fn the_selected_session_always_has_a_hub_tile() {
    let (mut a, _rx) = queued_app();
    sessions(&mut a, 0, 12);
    a.go_hub();
    a.hub_sessions = true;
    let last = workspaces::runtime_key(&a.work[0].sessions[11].clone());
    a.reconcile_session_selection(Some((0, last)));
    let (_, s) = a.selected_session().unwrap();
    assert_eq!(s.name, "nav 0 11");
    let text = draw(&a, 120, 40);
    assert!(text.contains("more"), "tiles are hidden: {text}");
    assert!(text.contains(&s.name), "{} has no tile: {text}", s.name);
}

/// `H` (and `` ` ``) `-` `+` `K` dispatch through the keymap's Global rows, not a parallel match: the
/// design's screen keys leave them alone, and they still reach their views.
#[test]
fn global_view_keys_go_through_the_keymap() {
    let (mut a, _rx) = queued_app();
    a.go_hub();
    for c in ['H', '`', '-', '+', 'K'] {
        let key = KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE);
        assert!(!a.screen_key(key), "{c} was caught before the keymap");
        assert!(keymap::resolve(&a, &key).is_some(), "{c} has a binding");
    }
    press(&mut a, 'K');
    assert!(a.view == View::Secrets);
    press(&mut a, 'H');
    assert!(a.view == View::Network);
}

// ------------------------------------------------------------------ v0.2.1 review

fn chord(a: &mut App, c: char) {
    a.key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL));
}
fn esc(a: &mut App) {
    a.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    // (Past the double-Esc window, so the next Esc is a single one.)
    a.last_esc = None;
}
/// An invite as its listener reports it, with a stand-in thread that ends on `go`.
fn open_invite(a: &mut App) -> mpsc::Sender<()> {
    let (_tx, rx) = mpsc::channel();
    let (go, wait) = mpsc::channel::<()>();
    a.invite = Some(screens::InviteRun {
        rx,
        cancel: Default::default(),
        shown: Some((
            crate::mesh::Invite {
                backend: "ssh-overlay".into(),
                code: "7K2Q-M9XD-4HT".into(),
                one_liner_sh: "cx join 7K2Q-M9XD-4HT 192.168.1.5".into(),
                ..Default::default()
            },
            Instant::now() + screens::INVITE_TTL,
        )),
        drawn_secs: Default::default(),
    });
    a.invite_thread = Some(thread::spawn(move || {
        let _ = wait.recv();
    }));
    go
}

/// Chords in the link editor are the keymap's: Ctrl+O never refuses a direction, and
/// Ctrl+P still opens the palette.
#[test]
fn the_link_editor_leaves_chords_to_the_keymap() {
    let (mut a, _rx) = queued_app();
    a.go_hub();
    press(&mut a, 'e');
    let l = letter(&a, 1);
    press(&mut a, l);
    assert!(a.edge.is_some());
    for c in ['o', 'a'] {
        chord(&mut a, c);
        assert!(a.edge_job.is_none(), "Ctrl+{c} changed a policy");
    }
    assert!(a.edge.as_ref().unwrap().undo.is_empty());
    chord(&mut a, 'p');
    assert!(a.input == Some(Input::Palette));
}

/// `?`, `K` and `` ` `` work in the link editor; `?` shows the link's own keys.
#[test]
fn global_keys_reach_past_the_link_editor() {
    let (mut a, _rx) = queued_app();
    a.go_hub();
    let l = letter(&a, 1);
    for k in ['e', l, '?'] {
        press(&mut a, k);
    }
    assert!(a.help && a.edge.is_some());
    assert!(draw(&a, 120, 40).contains("ON A LINK"));
    esc(&mut a);
    assert!(
        !a.help && a.edge.is_some(),
        "help closes back onto the link"
    );
    press(&mut a, 'K');
    assert!(a.view == View::Secrets && a.edge.is_none());
    a.go_hub();
    for k in ['e', l, 'H'] {
        press(&mut a, k);
    }
    assert!(a.view == View::Network && a.edge.is_none());
}

/// The link editor draws the link its keys change, also when nothing is recorded for
/// that pair yet (never the first stored link, never a blank sheet).
#[test]
fn the_link_editor_draws_the_pair_it_was_opened_on() {
    let mut a = viewmodel::tests::hub_app();
    a.open_device(1);
    a.view_action("edge:1-2");
    assert_eq!(a.edge.as_ref().unwrap().id, "1-2");
    let text = draw(&a, 140, 45);
    assert!(
        text.contains("innovation") && text.contains("nas"),
        "{text}"
    );
    assert!(text.contains("password"), "{text}");
}

/// One policy change at a time: a second key while the first is saving is refused,
/// so no result is dropped unseen and undo keeps one entry per change.
#[test]
fn a_policy_key_waits_for_the_change_being_saved() {
    let (mut a, _rx) = queued_app();
    a.go_hub();
    let l = letter(&a, 1);
    for k in ['e', l] {
        press(&mut a, k);
    }
    let (_tx, rx) = mpsc::channel();
    a.edge_job = Some(rx);
    for k in ['o', 'u'] {
        press(&mut a, k);
    }
    assert!(a.edge.as_ref().unwrap().undo.is_empty());
    assert!(a.notice.contains("Still saving"), "{}", a.notice);
}

/// The invite: `y` copies the code, `Y` the one-liner (OSC 52, a hedged toast), `?` is
/// help, and Esc closes it without waiting for the listener's thread.
#[test]
fn the_invite_copies_and_leaves_global_keys_working() {
    let (mut a, _rx) = queued_app();
    a.go_hub();
    let go = open_invite(&mut a);
    assert!(draw(&a, 120, 40).contains("command"), "footer chip");
    press(&mut a, 'y');
    assert_eq!(a.copy_out.take().as_deref(), Some("7K2Q-M9XD-4HT"));
    assert!(a.notice.contains("if your terminal allows"), "{}", a.notice);
    a.key(KeyEvent::new(KeyCode::Char('Y'), KeyModifiers::SHIFT));
    assert_eq!(
        a.copy_out.take().as_deref(),
        Some("cx join 7K2Q-M9XD-4HT 192.168.1.5")
    );
    a.view_action("copy-code");
    assert_eq!(a.copy_out.take().as_deref(), Some("7K2Q-M9XD-4HT"));
    press(&mut a, '?');
    assert!(a.help && a.invite.is_some());
    esc(&mut a);
    esc(&mut a);
    assert!(a.invite.is_none());
    let closing = a
        .invite_thread
        .as_ref()
        .expect("the thread is kept, not joined");
    assert!(!closing.is_finished());
    drop(go);
}

/// Help is the table for the screen it was opened over, and a device's lists every
/// device verb the keymap dispatches there.
#[test]
fn help_is_the_table_of_the_screen_below() {
    let (mut a, _rx) = queued_app();
    a.go_hub();
    let l = letter(&a, 1);
    for k in [l, '?'] {
        press(&mut a, k);
    }
    let text = draw(&a, 120, 40);
    assert!(text.contains("IN A DEVICE"), "{text}");
    assert!(
        text.contains("C X") && text.contains("resume claude"),
        "{text}"
    );
    assert!(text.contains("b E D") && text.contains("w p t"), "{text}");
    // Files is yazi's: `~` opens its help, the keymap with the user's prepends first.
    let (mut a, _rx) = file_app();
    press(&mut a, '~');
    let text = draw(&a, 120, 40);
    assert!(
        text.contains("mgr.help") && text.contains("Quit Files") && !text.contains("not in cx"),
        "{text}"
    );
}

/// Files' `c` opens the which-key panel and `cc` copies the path (OSC 52); `y` yanks
/// for a paste (a transfer when pasted on another device).
#[test]
fn files_cc_copies_the_path_and_y_yanks() {
    let (mut a, _rx) = file_app();
    press(&mut a, 'c');
    let text = draw(&a, 120, 40);
    assert!(
        text.contains("Copy file path") && text.contains("Copy directory path"),
        "{text}"
    );
    press(&mut a, 'c');
    assert_eq!(a.copy_out.take().as_deref(), Some("/files/alpha.txt"));
    assert!(a.clipboard.is_none(), "no transfer copy");
    press(&mut a, 'y');
    assert!(a.clipboard.is_some());
    assert!(a.copy_out.is_none());
}

/// An `e` that is not followed by a device letter is dropped, and that key does what
/// it always does.
#[test]
fn a_pending_link_prefix_never_swallows_the_next_key() {
    let (mut a, _rx) = queued_app();
    sessions(&mut a, 1, 2);
    a.go_hub();
    press(&mut a, 'e');
    esc(&mut a);
    let l = letter(&a, 1);
    press(&mut a, l);
    assert!(
        a.edge.is_none() && a.view == View::Work,
        "the letter opened the device"
    );
    sessions(&mut a, 0, 3);
    a.go_hub();
    a.hub_sessions = true;
    a.screen_move(0);
    let before = a.selected_session().unwrap().1.id;
    press(&mut a, 'e');
    press(&mut a, 'j');
    assert_ne!(a.selected_session().unwrap().1.id, before, "`j` moved");
}

/// `gg` is the first row on the hub, also with a device named for `g` (it takes another
/// letter: `g` is late, like `h j k l`).
#[test]
fn gg_is_the_first_row_when_no_device_has_g() {
    let (mut a, _rx) = queued_app();
    a.devices[1].name = "gateway".into();
    a.hotkeys.borrow_mut().clear();
    assert!(a.hotkeys().values().all(|c| *c != 'g'), "{:?}", a.hotkeys());
    sessions(&mut a, 0, 3);
    a.go_hub();
    a.hub_sessions = true;
    a.key(KeyEvent::new(KeyCode::Char('G'), KeyModifiers::SHIFT));
    let last = a.selected_session().unwrap().1.id;
    press(&mut a, 'g');
    press(&mut a, 'g');
    assert_ne!(a.selected_session().unwrap().1.id, last);
}

/// In a device a slot key moves the stop question to that session: `⌫ d ⌫`.
#[test]
fn a_slot_key_retargets_an_armed_stop() {
    let (mut a, rx) = queued_app();
    sessions(&mut a, 0, 2);
    a.providers
        .insert(0, (vec!["stop-session-v1".into()], transport::now()));
    a.open_device(0);
    let _ = rx.try_iter().count();
    bksp(&mut a);
    press(&mut a, 'd');
    let (_, armed) = a.stop_armed.clone().expect("the question moved");
    assert_eq!(armed.id, a.slot_sessions(0)[1].id);
    bksp(&mut a);
    assert!(rx
        .try_iter()
        .any(|t| matches!(t.op, Operation::StopSession { .. })));
}

/// A device still being checked is not drawn as down (no red `✕`).
#[test]
fn an_unchecked_device_is_not_down() {
    let (mut a, _rx) = queued_app();
    a.open_device(1);
    a.work[1].fetched = 0;
    let text = draw(&a, 120, 40);
    assert!(
        text.contains("checking") || text.contains("not checked"),
        "{text}"
    );
    assert!(!text.contains('✕'), "{text}");
}

/// Onboarding's `⏎ Connect Cloudflare` (key or tap) opens the token's masked entry.
#[test]
fn onboarding_enter_opens_the_cloudflare_token_entry() {
    let mut a = app();
    assert!(a.onboarding());
    a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(a.view == View::Secrets);
    let e = a.secret_entry.as_ref().expect("the masked entry");
    assert_eq!(e.name, crate::secrets::CLOUDFLARE_TOKEN);
    let mut a = app();
    draw(&a, 120, 40);
    let r = act("connect-cloudflare");
    click(&mut a, r.x, r.y, Rect::new(0, 0, 120, 40));
    assert!(a.secret_entry.is_some());
}

/// The hub's vitals chips run their keys: `f files` opens that device's files.
#[test]
fn hub_vitals_chips_run_their_keys() {
    let (mut a, _rx) = queued_app();
    a.go_hub();
    draw(&a, 120, 40);
    let r = act("files:0");
    click(&mut a, r.x, r.y, Rect::new(0, 0, 120, 40));
    assert!(a.view == View::Files);
}

/// Every target a production frame registers is an action `view_action` knows.
#[test]
fn every_drawn_target_is_a_known_action() {
    type Setup = fn() -> App;
    let setups: [(&str, Setup); 9] = [
        ("hub", || {
            let (mut a, _) = queued_app();
            sessions(&mut a, 0, 2);
            a.go_hub();
            a
        }),
        ("hub sessions", || {
            let (mut a, _) = queued_app();
            sessions(&mut a, 0, 2);
            a.work[1].error = Some("peace@laptop: timeout".into());
            a.go_hub();
            a.hub_sessions = true;
            a
        }),
        ("device", || {
            let (mut a, _) = queued_app();
            sessions(&mut a, 1, 2);
            a.open_device(1);
            a
        }),
        ("peek", || {
            let (mut a, _) = queued_app();
            sessions(&mut a, 1, 2);
            a.open_device(1);
            a.peek = true;
            a
        }),
        ("files", || file_app().0),
        ("secrets", || {
            let (mut a, _) = queued_app();
            a.open_secrets();
            a
        }),
        ("onboarding", app),
        ("link", || {
            let (mut a, _) = queued_app();
            a.go_hub();
            a.view_action("edge:0-1");
            a
        }),
        ("invite", || {
            let (mut a, _) = queued_app();
            a.go_hub();
            drop(open_invite(&mut a));
            a
        }),
    ];
    // (These start a listener or enforce a policy over SSH: not run here.)
    let live = |act: &str| {
        ["invite", "join", "new-code", "policy:"]
            .iter()
            .any(|p| act.starts_with(p))
    };
    for (name, setup) in setups {
        for (w, h) in [(120, 40), (60, 30)] {
            draw(&setup(), w, h);
            let actions: Vec<String> = hitmap::with(|m| m.all())
                .into_iter()
                .filter_map(|(_, hit)| match hit {
                    Hit::View(i) => hitmap::action(i),
                    _ => None,
                })
                .collect();
            for act in actions.iter().filter(|a| !live(a)) {
                assert!(
                    setup().view_action(act),
                    "{name} {w}x{h}: `{act}` does nothing"
                );
            }
        }
    }
}

/// A session whose device is unreachable, or whose listing is old, is unknown: never
/// drawn as exited or idle.
#[test]
fn sessions_without_current_evidence_are_unknown() {
    let (mut a, _rx) = queued_app();
    sessions(&mut a, 1, 1);
    let state = |a: &App| viewmodel::ViewModel::from(a).state.sessions[0].state;
    assert_ne!(state(&a), views::SessionState::Unknown);
    a.work[1].error = Some("peace@laptop: timeout".into());
    assert_eq!(state(&a), views::SessionState::Unknown);
    a.work[1].error = None;
    a.work[1].fetched = transport::now() - 120;
    assert_eq!(state(&a), views::SessionState::Unknown);
    a.open_device(1);
    a.work[1].fetched = transport::now() - 120;
    assert!(draw(&a, 120, 40).contains("unknown"));
}

/// The ticker says which device and why, once: no transport advice, no "Refresh".
#[test]
fn the_ticker_names_the_device_and_a_short_reason() {
    let (mut a, _rx) = queued_app();
    a.work[1].error = Some(
        "tester@workstation: peace@laptop: Device unreachable over SSH · check its connection, then Refresh"
            .into(),
    );
    let vm = viewmodel::ViewModel::from(&a);
    let n = &vm.state.notifications[0];
    assert_eq!(n.text, "laptop: Device unreachable over SSH");
    assert_eq!(n.device.as_deref(), Some("1"));
}

/// A device keeps its identity hue (its id's hash) when another device is removed;
/// only one bumped aside by a collision with the removed one moves back to its own.
#[test]
fn a_device_keeps_its_hue_when_another_leaves() {
    let mut a = viewmodel::tests::hub_app();
    let hue = |a: &App, name: &str| {
        let vm = viewmodel::ViewModel::from(a);
        vm.state
            .devices
            .iter()
            .find(|d| d.name == name)
            .unwrap()
            .hue
    };
    let local = hue(&a, "tranquility");
    assert_eq!(local, crate::ui::theme::swatch_index("local"));
    a.devices.remove(1);
    a.work.remove(1);
    assert_eq!(hue(&a, "tranquility"), local);
    assert_eq!(hue(&a, "nas"), crate::ui::theme::swatch_index("nas"));
}

/// The design's tokens are the theme's: one light `ok`/`warn` (4.5:1 on #f0f0f0), and
/// the first identity hue is not the focus accent.
#[test]
fn the_design_and_the_theme_share_one_token_table() {
    use views::{ColorLevel, LabCaps, Theme, Tok};
    let light = LabCaps {
        color: ColorLevel::Truecolor,
        theme: Theme::Light,
        ..LabCaps::default()
    };
    assert_eq!(light.color(Tok::Ok), Some(Color::Rgb(0x23, 0x7a, 0x35)));
    assert_eq!(light.color(Tok::Warn), Some(Color::Rgb(0x8f, 0x5b, 0x00)));
    assert_ne!(light.color(Tok::Id(0)), light.color(Tok::Accent));
}

/// UI tests keep their state in a temporary directory, never the developer's.
#[test]
fn ui_tests_never_write_the_real_state_dir() {
    let _ = app();
    let home = std::path::PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    assert!(!store::state_dir().starts_with(home.join(".local/state")));
    if let Some(x) = std::env::var_os("XDG_STATE_HOME") {
        assert!(!store::state_dir().starts_with(x));
    }
}

/// A link this device reaches through an enrolled relay is a jump through it.
#[test]
fn a_routed_link_is_a_jump_through_its_relay() {
    let mut a = viewmodel::tests::hub_app();
    let dir = |from: &str, to: &str| crate::edges::DirectionView {
        from: from.into(),
        to: to.into(),
        access: crate::edges::Access::Key,
        source: String::new(),
        enforced: String::new(),
    };
    a.edge_views = vec![crate::edges::EdgeView {
        from: crate::edges::LOCAL.into(),
        to: "nas".into(),
        from_name: String::new(),
        to_name: String::new(),
        label: String::new(),
        forward: dir(crate::edges::LOCAL, "nas"),
        reverse: dir("nas", crate::edges::LOCAL),
        routes_over: 0,
    }];
    a.network_jump_routes
        .insert(2, Some(vec!["innovation".into()]));
    let vm = viewmodel::ViewModel::from(&a);
    assert_eq!(vm.state.edges[0].kind, views::EdgeKind::Jump("1".into()));
}

/// The rect a navigable item (`device:<id>`, `session:<d>:<key>`) was drawn at.
fn nav_rect(a: &App, item: &str) -> Rect {
    a.nav
        .borrow()
        .iter()
        .find(|(_, x)| x == item)
        .map(|(r, _)| *r)
        .unwrap_or_else(|| panic!("{item} not drawn"))
}
fn selected_rect(a: &App) -> Rect {
    let (d, s) = a.selected_session().unwrap();
    nav_rect(a, &format!("session:{d}:{}", workspaces::runtime_key(&s)))
}

/// The hub's tiles are a grid the arrows and `h j k l` walk in 2D; Down from the map
/// enters the tiles and Up from the top row returns to it.
#[test]
fn the_hub_tiles_are_a_grid_the_arrows_walk() {
    let (mut a, _rx) = queued_app();
    sessions(&mut a, 0, 5);
    a.go_hub();
    draw(&a, 190, 50);
    a.key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    assert!(a.hub_sessions);
    draw(&a, 190, 50);
    let start = selected_rect(&a);
    for (key, check) in [
        (
            KeyCode::Down,
            &(|r: Rect, s: Rect| r.y > s.y) as &dyn Fn(Rect, Rect) -> bool,
        ),
        (KeyCode::Right, &|r: Rect, s: Rect| r.x > s.x && r.y == s.y),
        (KeyCode::Left, &|r: Rect, s: Rect| r.x < s.x && r.y == s.y),
        (KeyCode::Up, &|r: Rect, s: Rect| r.y < s.y),
    ] {
        let before = selected_rect(&a);
        a.key(KeyEvent::new(key, KeyModifiers::NONE));
        draw(&a, 190, 50);
        let after = selected_rect(&a);
        assert!(check(after, before), "{key:?}: {before:?} -> {after:?}");
    }
    // Back on the first row: `k` (Up) leaves the tiles for the map, `j` returns.
    assert_eq!(selected_rect(&a).y, start.y);
    press(&mut a, 'k');
    assert!(!a.hub_sessions, "up from the top row is the map");
    press(&mut a, 'j');
    assert!(a.hub_sessions, "down from the map is the tiles");
    // `l` is Right; Enter attaches the focused tile; `␣` peeks it.
    draw(&a, 190, 50);
    let before = selected_rect(&a);
    press(&mut a, 'l');
    draw(&a, 190, 50);
    assert!(selected_rect(&a).x > before.x);
    press(&mut a, ' ');
    assert!(a.peek);
    a.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(a.pending_attach.is_some() || a.attach_check.is_some());
}

/// In the map the arrows move to the nearest node that way (here the lone peer sits
/// right of this device), and Down past the map's last node enters the tiles.
#[test]
fn the_map_arrows_move_between_nodes_by_place() {
    let (mut a, _rx) = queued_app();
    sessions(&mut a, 0, 2);
    a.go_hub();
    let here = |a: &App| {
        let id = a
            .hub_device
            .clone()
            .unwrap_or_else(|| viewmodel::dev_id(a.local_index()));
        nav_rect(a, &format!("device:{id}"))
    };
    draw(&a, 190, 50);
    let home = here(&a);
    let others: Vec<Rect> = a
        .nav
        .borrow()
        .iter()
        .filter(|(r, x)| x.starts_with("device:") && *r != home)
        .map(|(r, _)| *r)
        .collect();
    // A lone peer sits beside the root on the orbit's right half.
    assert!(others.iter().all(|r| r.x >= home.right()), "{others:?}");
    a.key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
    draw(&a, 190, 50);
    assert!(here(&a).x >= home.right() && !a.hub_sessions);
    press(&mut a, 'h');
    draw(&a, 190, 50);
    assert_eq!(here(&a), home);
    // Up has nowhere to go (nothing moves); down leaves the map for the tiles.
    a.key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
    draw(&a, 190, 50);
    assert_eq!(here(&a), home);
    a.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    assert!(a.hub_sessions, "down from the map's last node is the tiles");
}

/// In a device the tiles are a grid too; a tap focuses a tile.
#[test]
fn a_device_grid_walks_with_arrows_and_taps_focus() {
    let (mut a, _rx) = queued_app();
    sessions(&mut a, 0, 4);
    a.open_device(0);
    draw(&a, 190, 50);
    let first = selected_rect(&a);
    a.key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
    draw(&a, 190, 50);
    let right = selected_rect(&a);
    assert!(right.x > first.x && right.y == first.y);
    press(&mut a, 'j');
    draw(&a, 190, 50);
    assert!(selected_rect(&a).y > right.y);
    press(&mut a, 'k');
    draw(&a, 190, 50);
    assert_eq!(selected_rect(&a).y, first.y);
}

/// The palette's arrows and Ctrl+N/P move the drawn selection (and the list scrolls
/// to it); Left/Right move the text cursor.
#[test]
fn palette_arrows_move_the_drawn_selection_and_the_cursor() {
    let (mut a, _rx) = queued_app();
    sessions(&mut a, 0, 3);
    a.go_hub();
    a.open_palette("");
    let marked = |a: &App| -> String {
        draw(a, 120, 40)
            .lines()
            .find(|l| l.contains("│▌"))
            .unwrap_or_default()
            .to_string()
    };
    let rows = a.palette_view();
    assert!(marked(&a).contains(&rows[0].label));
    a.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    assert_eq!(a.palette_selected, 1);
    assert!(marked(&a).contains(&rows[1].label), "{}", marked(&a));
    a.key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::CONTROL));
    assert!(marked(&a).contains(&rows[2].label));
    a.key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL));
    assert_eq!(a.palette_selected, 1);
    // Past what the sheet holds, the rows scroll with the selection.
    let last = rows.len() - 1;
    for _ in 0..last {
        a.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    }
    assert_eq!(a.palette_selected, last);
    assert!(
        marked(&a).contains(&rows[last].label),
        "{}",
        draw(&a, 120, 40)
    );
    // Left/Right move the cursor; typing goes where it is.
    for c in "ab".chars() {
        press(&mut a, c);
    }
    a.key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
    press(&mut a, 'X');
    assert_eq!(a.text, "aXb");
    a.key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
    press(&mut a, 'Y');
    assert_eq!(a.text, "aXbY");
    assert!(draw(&a, 120, 40).contains("aXbY▏"));
    a.key(KeyEvent::new(KeyCode::Home, KeyModifiers::NONE));
    press(&mut a, 'Z');
    assert_eq!(a.text, "ZaXbY");
    assert!(draw(&a, 120, 40).contains("Z▏aXbY"));
}

/// One key label set: `␣ ⏎ ⌫` in Unicode, `spc ret bksp` in ASCII.
#[test]
fn keys_are_labelled_with_one_glyph_set() {
    let (mut a, _rx) = queued_app();
    sessions(&mut a, 0, 2);
    a.open_device(0);
    let text = draw(&a, 190, 50);
    let footer = text.lines().last().unwrap();
    for k in ["⏎", "␣", "⌫"] {
        assert!(footer.contains(k), "{k}: {footer}");
    }
    for word in ["spc", "bksp", "enter"] {
        assert!(!footer.contains(word), "{word}: {footer}");
    }
    let ascii = caps::Caps {
        unicode: false,
        ..caps::Caps::default()
    };
    let text = caps::with_caps(ascii, || draw(&a, 190, 50));
    let footer = text.lines().last().unwrap();
    for k in ["ret", "spc", "bksp"] {
        assert!(footer.contains(k), "{k}: {footer}");
    }
}

/// The App's own overlays (command panel, New session, dialogs, menus, find, transfer
/// and typing bars, containers, the toast, the file preview body, the palette) over
/// each view, drawn through `render` at every size from 1x1. Panic sites with their
/// first case.
fn sweep_app(sizes: &[(u16, u16)]) -> Vec<String> {
    type Setup = fn(&mut App);
    let overlays: &[(&str, Setup)] = &[
        ("hub", |a| a.go_hub()),
        ("device", |a| a.open_device(1)),
        ("help", |a| a.help = true),
        ("command-panel", |a| {
            a.command_panel = Some(CommandPanel {
                device: 0,
                id: "run".into(),
                cols: 80,
                rows: 24,
                lines: (0..40)
                    .map(|i| format!("\x1b[32mline {i}\x1b[0m {}", "x".repeat(i * 3)))
                    .collect(),
                alive: true,
                status: None,
                inflight: false,
                last_poll: std::time::Instant::now(),
                label: "echo a long command label that runs".into(),
                quiet: false,
            })
        }),
        ("command-panel-empty", |a| {
            a.command_panel = Some(CommandPanel {
                device: 1,
                id: "run".into(),
                cols: 80,
                rows: 24,
                lines: vec![],
                alive: false,
                status: Some(1),
                inflight: true,
                last_poll: std::time::Instant::now(),
                label: "echo hi".into(),
                quiet: false,
            })
        }),
        ("creating", |a| a.creating = true),
        ("toast", |a| {
            a.set_notice_as(
                NoticeKind::Error,
                "laptop: Device unreachable over SSH ".repeat(4),
            )
        }),
        ("palette-run", |a| {
            a.input = Some(Input::Palette);
            a.text = "run: echo hi".into();
            a.rename_cursor = a.text.len();
        }),
        ("rename", |a| {
            a.input = Some(Input::Rename);
            a.text = "a-very-long-name-".repeat(5);
            a.rename_cursor = a.text.len();
        }),
        ("add", |a| {
            a.input = Some(Input::Add);
            a.text = "user@host".into();
        }),
        ("mkdir", |a| a.input = Some(Input::Mkdir)),
        ("peer", |a| a.dialog = Some(Dialog::Peer(1))),
        ("jobs", |a| a.dialog = Some(Dialog::Jobs)),
        ("provider", |a| a.dialog = Some(Dialog::Provider(1, None))),
        ("chooser", |a| {
            a.dialog = Some(Dialog::SessionChooser(0, "~".into()))
        }),
        ("remove", |a| a.dialog = Some(Dialog::RemoveDevice(1))),
        ("edit", |a| a.dialog = Some(Dialog::EditDevice(1))),
        ("reset", |a| a.dialog = Some(Dialog::Reset)),
        ("exit", |a| a.dialog = Some(Dialog::PendingExit(3))),
        ("devices", |a| {
            a.dialog = Some(Dialog::Device(ChooseDevice::Files))
        }),
        ("menu", |a| {
            a.context_menu = Some(hitmap::ContextMenu {
                items: keymap::context_menu(a),
                selected: 0,
                anchor: (3, 3),
            })
        }),
        ("containers", |a| a.view = View::Containers),
        ("secrets", |a| a.view = View::Secrets),
        ("typing", |a| {
            a.session_preview.focused =
                Some((a.devices[0].clone(), disposable_shell(), "cx".into()))
        }),
        ("files", |a| a.view = View::Files),
        ("files-find", |a| {
            a.view = View::Files;
            a.input = Some(Input::Search);
            a.text = "alp".into();
            a.rename_cursor = 3;
        }),
        ("files-filter", |a| {
            a.view = View::Files;
            a.input = Some(Input::Filter);
        }),
        ("files-preview", |a| {
            a.view = View::Files;
            let text = "# Title\n\n```rust\nfn main() {}\n```\n- one two three four five six\n";
            let b = a.browser.as_mut().unwrap();
            b.preview_path = Some("/files/alpha.md".into());
            b.preview = Some(text.into());
            b.preview_rich = Some(RichPreview::from_value(
                &serde_json::json!({"kind":"markdown","text":text}),
            ));
        }),
        ("files-transfer", |a| {
            a.view = View::Files;
            let entries = a.browser.as_ref().unwrap().entries[..1].to_vec();
            a.clipboard = Some(Clipboard {
                container: None,
                id: "fixture".into(),
                device: 1,
                entries,
                cut: true,
                source_label: "fixture".into(),
            });
            a.destination_active = true;
        }),
    ];
    let ascii = caps::Caps {
        unicode: false,
        ..caps::Caps::default()
    };
    let prev = std::panic::take_hook();
    thread_local! {
        static AT: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
    }
    std::panic::set_hook(Box::new(|info| {
        // The first frame in cx's own source (a ratatui index panic names ratatui).
        let trace = std::backtrace::Backtrace::force_capture().to_string();
        let ours = trace
            .lines()
            .find(|l| l.contains("at ./src/") && !l.contains("/tests"))
            .map(|l| l.trim().to_string());
        let at = info.location().map(|l| l.to_string()).unwrap_or_default();
        AT.with(|w| *w.borrow_mut() = ours.map_or(at.clone(), |o| format!("{o} ({at})")));
    }));
    let mut fails = std::collections::BTreeMap::<String, (usize, String)>::new();
    for (name, setup) in overlays {
        let (mut a, _rx) = file_app();
        sessions(&mut a, 0, 5);
        sessions(&mut a, 1, 3);
        setup(&mut a);
        for tier in [caps::Caps::default(), ascii] {
            for &(w, h) in sizes {
                let drew = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    caps::with_caps(tier, || {
                        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
                        t.draw(|f| render(f, &a)).unwrap();
                    })
                }));
                if let Err(e) = drew {
                    let what = e
                        .downcast_ref::<String>()
                        .cloned()
                        .or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string()))
                        .unwrap_or_default();
                    let at = AT.with(|w| w.borrow().clone());
                    let what: String = what.chars().take(60).collect();
                    let f = fails
                        .entry(format!("{at} {what}"))
                        .or_insert((0, String::new()));
                    f.0 += 1;
                    if f.1.is_empty() {
                        f.1 = format!("{name} {w}x{h} unicode={}", tier.unicode);
                    }
                }
            }
        }
    }
    std::panic::set_hook(prev);
    fails
        .into_iter()
        .map(|(what, (n, first))| format!("{what}: {n}x, first {first}"))
        .collect()
}

/// The suite's share of the overlay sweep: tiny, phone, half and full terminals.
#[test]
fn every_overlay_renders_across_sizes() {
    // (A light share: it runs beside the suite's timing-sensitive tests.)
    let ws = [1, 5, 10, 16, 19, 20, 30, 40, 60, 80, 95, 120, 190, 250];
    let hs = [1, 3, 5, 8, 16, 24, 36, 50, 80];
    let sizes: Vec<(u16, u16)> = ws.iter().flat_map(|&w| hs.map(|h| (w, h))).collect();
    let fails = sweep_app(&sizes);
    assert!(fails.is_empty(), "{} panic sites: {fails:#?}", fails.len());
}

/// The whole overlay matrix: 1x1 to 250x80, every width to 140 and every height to 60.
#[test]
#[ignore]
fn every_overlay_renders_at_every_size() {
    let ws = (1..=140).chain((150..=250).step_by(10));
    let sizes: Vec<(u16, u16)> = ws
        .flat_map(|w| (1..=60).chain([70, 80]).map(move |h| (w, h)))
        .collect();
    let fails = sweep_app(&sizes);
    assert!(fails.is_empty(), "{} panic sites: {fails:#?}", fails.len());
}

/// A highlighted folder shows its own entries in the preview column (yazi's third
/// column), says yazi's `No items` when it is empty, and nothing while it is listed.
#[test]
fn a_highlighted_folder_previews_its_entries() {
    let (mut a, _rx) = file_app();
    let entry = |dir: &str, name: &str, kind: &str| Entry {
        name: name.into(),
        path: format!("{dir}/{name}"),
        kind: kind.into(),
        size: 3,
        identity: None,
        hidden: false,
        rename_name: None,
        modified: None,
        mode: None,
    };
    let mut child = Browser::new(0, "/files/docs".into());
    child.entries = vec![
        entry("/files/docs", "guides", "directory"),
        entry("/files/docs", "inner-note.md", "file"),
    ]
    .into();
    let b = a.browser.as_mut().unwrap();
    let mut entries: Vec<Entry> = b.entries.iter().cloned().collect();
    entries.insert(0, entry("/files", "docs", "directory"));
    b.entries = entries.into();
    b.selected = 0;
    b.peek = Some(Box::new(child));
    let text = draw(&a, 190, 50);
    assert!(
        text.contains("guides") && text.contains("inner-note.md"),
        "{text}"
    );
    let b = a.browser.as_mut().unwrap();
    b.peek.as_mut().unwrap().entries = Vec::new().into();
    assert!(draw(&a, 190, 50).contains("No items"));
    // Still listing: nothing claims the folder is empty.
    a.browser.as_mut().unwrap().peek.as_mut().unwrap().loading = true;
    assert!(!draw(&a, 190, 50).contains("No items"));
}

/// Every map move is undone by the opposite arrow, and a run of moves by the opposite
/// run (Down Down, then Up Up), from every node of maps of four to eight nodes.
#[test]
fn the_opposite_arrow_undoes_each_map_move() {
    for n in 3..=7 {
        undo_map_moves(n);
    }
}
fn undo_map_moves(others: usize) {
    crate::store::isolate_test_state();
    let (tx, _rx) = mpsc::sync_channel(64);
    let mut devices = app().devices;
    let names = ["north", "east", "south", "west", "relay", "robot", "nas"];
    for &name in &names[..others] {
        let mut d = devices[0].clone();
        d.id = name.into();
        d.name = name.into();
        d.target = Some(format!("{name}.invalid"));
        devices.push(d);
    }
    let mut a = App::new(devices, tx);
    // (Sessions, so Down past the map lands on tiles and the arrows' memory meets them.)
    sessions(&mut a, 0, 2);
    a.go_hub();
    let node = |a: &App| a.hub_device.clone().unwrap_or_else(|| viewmodel::dev_id(0));
    let dirs = [KeyCode::Up, KeyCode::Down, KeyCode::Left, KeyCode::Right];
    let opposite = |k: KeyCode| match k {
        KeyCode::Up => KeyCode::Down,
        KeyCode::Down => KeyCode::Up,
        KeyCode::Left => KeyCode::Right,
        _ => KeyCode::Left,
    };
    for (w, h) in [(80, 48), (120, 40), (190, 50)] {
        for start in 0..a.devices.len() {
            for first in dirs {
                for second in dirs {
                    a.go_hub();
                    a.hub_device = Some(viewmodel::dev_id(start));
                    draw(&a, w, h);
                    let mut path = vec![node(&a)];
                    let mut keys = Vec::new();
                    for k in [first, second] {
                        a.key(KeyEvent::new(k, KeyModifiers::NONE));
                        draw(&a, w, h);
                        if a.hub_sessions {
                            // Down past the map is the tiles; Up from them, the map again.
                            a.key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
                            draw(&a, w, h);
                            assert!(!a.hub_sessions);
                            assert_eq!(node(&a), *path.last().unwrap());
                            break;
                        }
                        if node(&a) == *path.last().unwrap() {
                            continue; // (Nothing that way: it stays put.)
                        }
                        path.push(node(&a));
                        keys.push(k);
                    }
                    for k in keys.iter().rev() {
                        let expected = path[path.len() - 2].clone();
                        a.key(KeyEvent::new(opposite(*k), KeyModifiers::NONE));
                        draw(&a, w, h);
                        path.pop();
                        assert_eq!(
                            node(&a),
                            expected,
                            "{others} others {w}x{h} from {start}: {first:?} {second:?} not undone"
                        );
                    }
                }
            }
        }
    }
}

/// Arrows never dismiss a popup: in the new-session chooser Left and Right move between
/// its choices, in the agent step Left goes back to the step that opened it (the device
/// chooser or the device menu), and only Esc closes either.
#[test]
fn arrows_move_in_popups_and_only_esc_closes_them() {
    let (mut a, _rx) = queued_app();
    let arrow = |a: &mut App, code| a.key(KeyEvent::new(code, KeyModifiers::NONE));
    for dialog in [
        Dialog::SessionChooser(0, "~".into()),
        Dialog::Provider(1, None),
        Dialog::Peer(1),
        Dialog::Jobs,
        Dialog::Device(ChooseDevice::New),
        Dialog::RemoveDevice(1),
        Dialog::EditDevice(1),
        Dialog::Reset,
        Dialog::PendingExit(1),
    ] {
        a.dialog = Some(dialog);
        a.dialog_back = None;
        for code in [KeyCode::Left, KeyCode::Right, KeyCode::Up, KeyCode::Down] {
            arrow(&mut a, code);
            assert!(a.dialog.is_some(), "{code:?} dismissed a popup");
        }
        arrow(&mut a, KeyCode::Esc);
        assert!(a.dialog.is_none(), "Esc kept a popup open");
    }
    // The chooser's choices sit in a row: Right, then Left, moves along it.
    a.dialog = Some(Dialog::SessionChooser(0, "~".into()));
    a.dialog_selected = 0;
    arrow(&mut a, KeyCode::Right);
    assert_eq!(a.dialog_selected, 1);
    arrow(&mut a, KeyCode::Left);
    assert_eq!(a.dialog_selected, 0);
    assert!(matches!(a.dialog, Some(Dialog::SessionChooser(..))));
    // New session -> a device -> the agent step; Left returns to the device, selected.
    a.dialog = None;
    a.device = 0;
    a.choose_device(ChooseDevice::New);
    assert!(matches!(a.dialog, Some(Dialog::Device(ChooseDevice::New))));
    let row = a
        .device_choices(ChooseDevice::New)
        .iter()
        .position(|c| *c == Some(1))
        .unwrap();
    a.dialog_selected = row;
    arrow(&mut a, KeyCode::Enter);
    assert!(matches!(a.dialog, Some(Dialog::Provider(1, None))));
    arrow(&mut a, KeyCode::Left);
    assert!(matches!(a.dialog, Some(Dialog::Device(ChooseDevice::New))));
    assert_eq!(a.dialog_selected, row);
    // The device menu's agent row, likewise.
    a.dialog = Some(Dialog::Peer(1));
    a.dialog_selected = 3;
    arrow(&mut a, KeyCode::Enter);
    assert!(matches!(a.dialog, Some(Dialog::Provider(1, None))));
    arrow(&mut a, KeyCode::Left);
    assert!(matches!(a.dialog, Some(Dialog::Peer(1))) && a.dialog_selected == 3);
}
/// The find palette and the link editor are rounded overlays (`╭╮╰╯`), like every other
/// frame: never a square corner, at phone, half and full sizes.
#[test]
fn the_palette_and_the_link_editor_have_rounded_corners() {
    let mut a = viewmodel::tests::hub_app();
    let square = ['┌', '┐', '└', '┘'];
    for (w, h) in [
        (40u16, 20u16),
        (80, 24),
        (80, 48),
        (95, 50),
        (120, 40),
        (190, 50),
    ] {
        a.go_hub();
        a.open_palette("");
        let t = draw(&a, w, h);
        assert!(t.contains('╭') && t.contains('╯'), "palette {w}x{h}:\n{t}");
        assert!(!t.contains(square), "palette {w}x{h}:\n{t}");
        a.input = None;
        a.text.clear();
        a.view_action("edge:1-2");
        let t = draw(&a, w, h);
        assert!(a.edge.is_some() && t.contains('╭'), "link {w}x{h}:\n{t}");
        assert!(!t.contains(square), "link {w}x{h}:\n{t}");
        a.edge = None;
    }
}

/// Arrows never close a popup, the Files which-key panel included: after a prefix key
/// an arrow leaves the chord waiting; Esc cancels it.
#[test]
fn an_arrow_keeps_the_files_which_key_panel() {
    let (mut a, _rx) = file_app();
    press(&mut a, 'g');
    assert_eq!(a.yz.pending, ["g"]);
    for code in [KeyCode::Left, KeyCode::Right, KeyCode::Up, KeyCode::Down] {
        a.key(KeyEvent::new(code, KeyModifiers::NONE));
        assert_eq!(a.yz.pending, ["g"], "{code:?} closed the which-key panel");
    }
    a.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(a.yz.pending.is_empty());
}

/// Every keycap the launch panel prints (device view, and the hub with nothing running,
/// where the device's letter comes first) does exactly what a tap on its row does.
#[test]
fn every_launch_panel_keycap_does_what_its_tap_does() {
    let (mut a, rx) = queued_app();
    sessions(&mut a, 0, 1);
    a.work[0].sessions[0].provider = "claude".into();
    a.work[0].sessions[0].directory = "/srv/proj".into();
    a.file_locations.insert(0, "/srv/notes".into());
    a.apply(Reply {
        device_id: Some(a.devices[0].id.clone()),
        execution_target: None,
        device: 0,
        op: Operation::Sessions,
        generation: a.generation,
        preview: None,
        result: Ok(serde_json::json!([])),
    });
    let area = Rect::new(0, 0, 120, 40);
    // What an action did: the dialog it opened and the sessions it asked for.
    let effect = |a: &mut App| {
        let ops: Vec<String> = rx
            .try_iter()
            .filter_map(|t| match &t.op {
                Operation::Create(c) | Operation::CreateYolo(c) => {
                    Some(format!("{} {} {}", c.provider, c.directory, c.resume))
                }
                _ => None,
            })
            .collect();
        let dialog = match a.dialog.take() {
            Some(Dialog::SessionChooser(d, p)) => format!("chooser {d} {p}"),
            Some(Dialog::Permissions(d, dir, p, ..)) => format!("permissions {d} {dir} {p}"),
            Some(Dialog::SessionPicker(d, dir, _, p, _)) => format!("picker {d} {dir} {p}"),
            Some(_) => "other".into(),
            None => "None".into(),
        };
        a.creating = false;
        (dialog, ops)
    };
    for hub in [false, true] {
        let open = |a: &mut App| {
            a.dialog = None;
            if hub {
                a.go_hub();
            } else {
                a.open_device(0);
            }
        };
        open(&mut a);
        let text = draw(&a, 120, 40);
        let rows: Vec<&str> = text.lines().collect();
        let targets: Vec<(Rect, String)> = hitmap::with(|m| m.all())
            .into_iter()
            .filter_map(|(r, h)| match h {
                Hit::View(i) => hitmap::action(i).map(|x| (r, x)),
                _ => None,
            })
            .filter(|(_, x)| {
                x.strip_prefix("new:")
                    .or(x.strip_prefix("launch:"))
                    .is_some_and(|r| r.starts_with(&format!("{}:", viewmodel::dev_id(0))))
            })
            .collect();
        assert!(targets.len() >= 5, "hub {hub}: {targets:?}\n{text}");
        for (r, action) in targets {
            // The key: the last one-letter cap before the row's words.
            let cells: String = rows[r.y as usize]
                .chars()
                .skip(r.x as usize)
                .take(r.width as usize)
                .collect();
            let key = cells
                .split_whitespace()
                .take_while(|t| t.chars().count() == 1)
                .last()
                .and_then(|t| t.chars().next())
                .unwrap_or_else(|| panic!("hub {hub}: no key on {action}: {cells:?}"));
            open(&mut a);
            let _ = effect(&mut a);
            if hub {
                let l = letter(&a, 0);
                press(&mut a, l);
            }
            press(&mut a, key);
            let by_key = effect(&mut a);
            open(&mut a);
            draw(&a, 120, 40);
            click(&mut a, r.x + 1, r.y, area);
            if hub {
                // (A tap acts at once; the key went through the device first.)
                assert_eq!(a.view, View::Network, "{action}");
            }
            let by_tap = effect(&mut a);
            assert!(
                by_key.0 != "None" || !by_key.1.is_empty(),
                "hub {hub}: {key} did nothing ({action})"
            );
            assert_eq!(by_key, by_tap, "hub {hub}: key {key} vs tap {action}");
        }
    }
}

/// The only device: its launch keys work on the hub without its letter first.
#[test]
fn a_lone_device_starts_from_the_hub_with_one_key() {
    let (tx, rx) = mpsc::sync_channel(64);
    let mut a = App::new(app().devices.into_iter().take(1).collect(), tx);
    a.providers
        .insert(0, (vec!["claude".into(), "codex".into()], transport::now()));
    a.onboarding_dismissed = true;
    a.go_hub();
    let text = draw(&a, 120, 40);
    assert!(text.contains(" x  codex"), "{text}");
    let _ = rx.try_iter().count();
    press(&mut a, 'x');
    let started = rx
        .try_iter()
        .any(|t| matches!(&t.op, Operation::Create(c) if c.provider == "codex"));
    assert!(started || matches!(&a.dialog, Some(Dialog::SessionChooser(0, _))));
}

/// `⌫` on the hub's sessions zone and in peek asks visibly (the next key used to
/// cancel a question only the device view drew); a second `⌫` stops. From the map,
/// where no session is in view, `⌫` asks nothing.
#[test]
fn hub_and_peek_show_the_stop_question_and_a_second_backspace_stops() {
    let (mut a, rx) = queued_app();
    sessions(&mut a, 0, 2);
    a.go_hub();
    a.hub_sessions = false;
    bksp(&mut a);
    assert!(a.stop_armed.is_none(), "the map has no session in view");
    a.hub_sessions = true;
    let _ = rx.try_iter().count();
    bksp(&mut a);
    let (_, armed) = a.stop_armed.clone().expect("armed");
    let screen = draw(&a, 160, 48);
    assert!(
        screen.contains(&format!("stop shell {} · /tmp/cx-disposable?", armed.name))
            && screen.contains("again to stop"),
        "{screen}"
    );
    assert!(screen.contains("stop it"), "footer: {screen}");
    bksp(&mut a);
    assert!(rx
        .try_iter()
        .any(|t| matches!(t.op, Operation::StopSession { id, .. } if id == armed.id)));

    press(&mut a, ' ');
    assert!(a.peek);
    bksp(&mut a);
    let (_, armed) = a.stop_armed.clone().expect("armed in peek");
    let screen = draw(&a, 160, 48);
    assert!(
        screen.contains(&format!("stop shell {} · /tmp/cx-disposable?", armed.name))
            && screen.contains("stop it"),
        "{screen}"
    );
    bksp(&mut a);
    assert!(rx
        .try_iter()
        .any(|t| matches!(t.op, Operation::StopSession { id, .. } if id == armed.id)));
}

/// A remote capability check older than a minute no longer costs a third `⌫`: the
/// question is asked at once while Info refreshes, and only an answer that says
/// "unsupported" refuses at the confirm.
#[test]
fn one_double_backspace_stops_a_remote_session_with_a_stale_check() {
    let (mut a, rx) = queued_app();
    let mut s = disposable_shell();
    s.provider = "claude".into();
    a.work[1].sessions = vec![s];
    a.work[1].fetched = transport::now();
    a.providers.insert(
        1,
        (vec!["stop-agent-session-v1".into()], transport::now() - 120),
    );
    a.open_device(1);
    let _ = rx.try_iter().count();
    bksp(&mut a);
    assert!(a.stop_armed.is_some(), "asked at once");
    bksp(&mut a);
    let ops: Vec<_> = rx.try_iter().map(|t| t.op).collect();
    assert!(ops.iter().any(|o| matches!(o, Operation::Info)), "{ops:?}");
    assert!(
        ops.iter().any(
            |o| matches!(o, Operation::StopAgentSession { provider, .. } if provider == "claude")
        ),
        "{ops:?}"
    );
    // An old helper: the refreshed answer arrives between the two presses.
    a.providers
        .insert(1, (vec!["claude".into()], transport::now() - 120));
    bksp(&mut a);
    assert!(a.stop_armed.is_some());
    a.providers
        .insert(1, (vec!["claude".into()], transport::now()));
    bksp(&mut a);
    assert!(!rx
        .try_iter()
        .any(|t| matches!(t.op, Operation::StopAgentSession { .. })));
}

/// An agent in a terminal outside tmux stops through StopExternal (identity checked
/// on its host) when that host's cx can; otherwise the old refusal stays.
#[test]
fn backspace_stops_an_outside_agent_through_stop_external() {
    let (mut a, rx) = queued_app();
    let outside: Session = serde_json::from_value(serde_json::json!({
        "id":"ext-0123456789abcdef","name":"claude · proj","directory":"/proj","provider":"claude",
        "host":"laptop","account":"peace","pid":700,"started":"ext:900","boot_id":"boot",
        "external":true,"socket":null,
        "origin":{"kind":"terminal","tty":"/dev/pts/4","leader":{"pid":700,"start_ticks":"900"},
                  "holder":{"pid":699,"start_ticks":"899"}}
    }))
    .unwrap();
    a.work[1].sessions = vec![outside];
    a.work[1].fetched = transport::now();
    a.providers
        .insert(1, (vec!["stop-agent-session-v1".into()], transport::now()));
    a.open_device(1);
    let _ = rx.try_iter().count();
    bksp(&mut a);
    assert!(a.stop_armed.is_none(), "an older helper cannot stop it");
    a.providers
        .insert(1, (vec!["stop-external-v1".into()], transport::now()));
    bksp(&mut a);
    let screen = draw(&a, 160, 48);
    assert!(screen.contains("stop proj claude · /proj?"), "{screen}");
    bksp(&mut a);
    assert!(rx.try_iter().any(|t| matches!(t.op,
        Operation::StopExternal { id, origin, boot_id }
            if id == "ext-0123456789abcdef" && origin.leader.pid == 700 && boot_id == "boot")));
    // A shell outside tmux is not an agent: nothing to stop.
    a.work[1].sessions[0].provider = "shell".into();
    bksp(&mut a);
    assert!(a.stop_armed.is_none());
}

/// The hub says where its bare `c x n` start, quietly on the SESSIONS rule, at every
/// size (phones included), the folder cut from the left; a tap on its key launches as
/// the key does.
#[test]
fn hub_shows_where_bare_launch_keys_start_at_every_size() {
    let (tx, rx) = mpsc::sync_channel(64);
    let mut devices = app().devices;
    let mut remote = devices[0].clone();
    remote.id = "remote".into();
    remote.name = "laptop".into();
    remote.target = Some("laptop".into());
    devices.push(remote);
    let mut a = App::new(devices, tx);
    a.onboarding_dismissed = true;
    a.here = Some("/srv/work/projects/robot-arm".into());
    a.go_hub();
    for (w, h) in [(120, 40), (80, 24), (60, 30), (40, 20), (36, 40)] {
        let text = draw(&a, w, h);
        let row = text
            .lines()
            .find(|l| l.contains("SESSIONS"))
            .unwrap_or_else(|| panic!("{w}x{h}: no SESSIONS rule\n{text}"));
        assert!(row.contains("here "), "{w}x{h}: {row}\n{text}");
        assert!(row.contains("robot-arm"), "{w}x{h}: {row}");
        if std::env::var_os("CX_PRINT").is_some() {
            println!("{w}x{h}\n{text}\n");
        }
    }
    // Wide: the whole folder and the keys (this device selected and idle: its launch
    // row under the rule shows the keys, so the rule shows the folder alone).
    let text = draw(&a, 120, 40);
    assert!(text.contains("here /srv/work/projects/robot-arm"), "{text}");
    assert!(text.contains(" c  claude"), "{text}");
    // With sessions, the rule carries the keys as well, and each taps as its key.
    sessions(&mut a, 1, 2);
    for (w, h) in [(120, 40), (80, 24), (40, 20)] {
        let text = draw(&a, w, h);
        let row = text.lines().find(|l| l.contains("SESSIONS")).unwrap();
        assert!(
            row.contains(" n  here ") && row.contains("robot-arm"),
            "{w}x{h}: {row}\n{text}"
        );
        if std::env::var_os("CX_PRINT").is_some() {
            println!("{w}x{h}\n{text}\n");
        }
    }
    let text = draw(&a, 120, 40);
    let (y, row) = text
        .lines()
        .enumerate()
        .find(|(_, l)| l.contains("SESSIONS"))
        .unwrap();
    let x = row.chars().collect::<String>().find(" n  here").unwrap();
    let x = row[..x].chars().count() as u16 + 1;
    let _ = rx.try_iter().count();
    click(&mut a, x, y as u16, Rect::new(0, 0, 120, 40));
    assert!(
        rx.try_iter().any(|t| matches!(&t.op, Operation::Create(c)
            if c.provider == "shell" && c.directory == "/srv/work/projects/robot-arm")),
        "a tap on the rule's n starts a shell here"
    );
    a.creating = false;
    // This device's screen: its launch row says it too, where the row has room.
    a.open_device(0);
    for (w, h) in [(120, 40), (80, 24)] {
        let text = draw(&a, w, h);
        assert!(text.contains("robot-arm"), "{w}x{h}\n{text}");
        if std::env::var_os("CX_PRINT").is_some() {
            println!("{w}x{h}\n{text}\n");
        }
    }
}
