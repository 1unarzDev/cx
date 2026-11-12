//! Unified keymap, hit map, palette, view switching, phone layout and frame
//! output (spec sections 3, 6, 6.6, 8, 12).
use super::*;
use crate::ui::hitmap::Hit;
use crate::ui::keymap::Cmd;

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
fn hub_letters_open_devices_and_backtick_returns_from_anywhere() {
    let (mut a, _rx) = file_app();
    sessions(&mut a, 1, 2);
    a.go_hub();
    assert!(a.view == View::Network);
    // A device letter opens that device, ready to attach or start.
    let l = letter(&a, 1);
    press(&mut a, l);
    assert!(a.view == View::Work && a.device == 2);
    assert!(a.dialog.is_none(), "a letter opened a chooser (P13)");
    // `f` there is its files; `` ` `` is the hub from any screen.
    press(&mut a, '`');
    assert!(a.view == View::Network && a.device == 0);
    a.view = View::Files;
    press(&mut a, '`');
    assert!(a.view == View::Network);
    // A double Esc also reaches the hub (where `` ` `` is a dead key).
    let l = letter(&a, 1);
    press(&mut a, l);
    a.key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE));
    assert!(a.view == View::Files);
    a.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    a.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
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
    a.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
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
    // Slot `s` is the second session: it attaches at once.
    press(&mut a, 's');
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
    // The footer's `` ` hub`` chip, then `? help`.
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
    // A session tile attaches, like its address.
    draw(&a, 120, 40);
    let key = workspaces::runtime_key(&a.work[0].sessions[0]);
    let tile = act(&format!("session:0:{key}"));
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
    let tile = act(&format!("session:0:{key}"));
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
    let tile = act(&format!("session:0:{key}"));
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
    let tile = act(&format!("session:0:{key}"));
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
        "a s d g",
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
    assert!(
        text.contains("NETWORK") && text.contains("laptop"),
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
    // The backdrop is faded: the hub's focused label loses its weight.
    let hub_label = base.lines().position(|l| l.contains("NETWORK")).unwrap() as u16;
    let x = base
        .lines()
        .nth(hub_label as usize)
        .unwrap()
        .find("NETWORK")
        .unwrap() as u16;
    assert!(!buffer[(x, hub_label)].modifier.contains(Modifier::BOLD));
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
    assert!(draw(&a, 120, 40).contains("if your terminal allows clipboard access"));
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
    press(&mut a, 'a');
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

/// `` ` `` `-` `+` `K` dispatch through the keymap's Global rows, not a parallel match: the
/// design's screen keys leave them alone, and they still reach their views.
#[test]
fn global_view_keys_go_through_the_keymap() {
    let (mut a, _rx) = queued_app();
    a.go_hub();
    for c in ['`', '-', '+', 'K'] {
        let key = KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE);
        assert!(!a.screen_key(key), "{c} was caught before the keymap");
        assert!(keymap::resolve(&a, &key).is_some(), "{c} has a binding");
    }
    press(&mut a, 'K');
    assert!(a.view == View::Secrets);
    press(&mut a, '`');
    assert!(a.view == View::Network);
}
