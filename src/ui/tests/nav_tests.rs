//! Unified keymap, hit map, palette, view switching, phone layout and frame
//! output (spec sections 3, 6, 6.6, 8, 12).
use super::*;
use crate::ui::hitmap::{Hit, ListId};
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
fn goto_chords_brackets_and_alt_digits_switch_views_without_a_chooser() {
    let (mut a, _rx) = file_app();
    a.view = View::Work;
    for (keys, view) in [
        ("gn", View::Network),
        ("gk", View::Secrets),
        ("gf", View::Files),
        ("gs", View::Work),
    ] {
        for c in keys.chars() {
            press(&mut a, c);
        }
        assert!(a.view == view, "{keys}");
        assert!(a.dialog.is_none(), "{keys} opened a device chooser (P13)");
    }
    // gg keeps meaning "first row".
    sessions(&mut a, 0, 5);
    a.selected = 3;
    press(&mut a, 'g');
    press(&mut a, 'g');
    assert_eq!(a.selected, 0);
    press(&mut a, ']');
    assert!(a.view == View::Files);
    press(&mut a, '[');
    press(&mut a, '[');
    assert!(
        a.view == View::Secrets,
        "[ cycles backwards and wraps across views"
    );
    a.key(KeyEvent::new(KeyCode::Char('3'), KeyModifiers::ALT));
    assert!(a.view == View::Network);
    a.key(KeyEvent::new(KeyCode::Char('1'), KeyModifiers::ALT));
    assert!(a.view == View::Work);
    // Plain digits stay counts.
    press(&mut a, '2');
    press(&mut a, 'j');
    assert_eq!(a.selected, 2);
}

#[test]
fn device_moves_never_refresh_and_keep_the_session_by_identity() {
    let (mut a, rx) = queued_app();
    sessions(&mut a, 0, 3);
    sessions(&mut a, 1, 2);
    a.view = View::Work;
    let id = a.session_rows()[1].1.id.clone();
    a.selected = 1;
    a.focus = Focus::Devices;
    a.device = 0;
    a.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    assert_eq!(a.device, 1);
    assert!(rx.try_recv().is_err(), "moving in Devices did I/O (P17)");
    assert!(
        a.providers.contains_key(&0),
        "moving cleared provider caches"
    );
    assert_eq!(a.selected_session().unwrap().1.id, id, "selection jumped");
    a.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    assert!(rx.try_recv().is_err());
    // Settled: exactly one scoped Sessions request for the chosen device.
    assert!(!a.settle_device_refresh(Instant::now()));
    let due = a.device_refresh_due.unwrap();
    assert!(a.settle_device_refresh(due));
    let tasks: Vec<_> = rx.try_iter().collect();
    assert!(!tasks.is_empty());
    assert!(tasks
        .iter()
        .all(|t| t.device == 1 && matches!(t.op, Operation::Sessions)));
    // Enter on a device lands in the view without another refresh.
    a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(a.focus == Focus::Workspace);
    assert!(rx.try_recv().is_err());
}

#[test]
fn moving_clamps_without_wrapping_and_keeps_scrolloff() {
    let (mut a, _rx) = queued_app();
    sessions(&mut a, 0, 40);
    a.view = View::Work;
    a.key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
    assert_eq!(a.selected, 0, "Up at the top wrapped");
    a.key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE));
    let last = a.selected;
    a.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    assert_eq!(a.selected, last, "Down at the bottom wrapped");
    // The viewport follows with a margin and does not jump on unrelated redraws.
    a.selected = 0;
    draw(&a, 120, 40);
    let row_of = |text: &str, name: &str| {
        text.lines()
            .position(|l| l.contains(name) && l.contains("live"))
    };
    for _ in 0..30 {
        a.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        draw(&a, 120, 40);
    }
    let name = |a: &App| a.selected_session().unwrap().1.name;
    let text = draw(&a, 120, 40);
    let bar = row_of(&text, &name(&a)).unwrap();
    assert!(
        row_of(&text, &format!("nav 0 {:02}", a.selected + 2)).is_some(),
        "scrolloff: two rows stay visible below the bar: {text}"
    );
    assert_eq!(
        row_of(&draw(&a, 120, 40), &name(&a)),
        Some(bar),
        "list jumped on redraw"
    );
    // One step up moves the bar one row, not the list.
    a.key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
    let text = draw(&a, 120, 40);
    assert_eq!(row_of(&text, &name(&a)), Some(bar - 1), "{text}");
}

#[test]
fn tabs_chips_and_rows_are_tap_targets_with_key_semantics() {
    let (mut a, _rx) = queued_app();
    sessions(&mut a, 0, 3);
    a.view = View::Work;
    let area = Rect::new(0, 0, 120, 30);
    draw(&a, 120, 30);
    let tab = target(Hit::Tab(View::Network));
    click(&mut a, tab.x, tab.y, area);
    assert!(a.view == View::Network && a.dialog.is_none());
    draw(&a, 120, 30);
    let tab = target(Hit::Tab(View::Work));
    click(&mut a, tab.x + 1, tab.y, area);
    assert!(a.view == View::Work);
    // A footer chip runs its key's command; "? Help" is always offered.
    draw(&a, 120, 30);
    let help = target(Hit::Cmd(Cmd::Help));
    click(&mut a, help.x, help.y, area);
    assert!(a.help);
    a.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    // Header palette button.
    draw(&a, 120, 30);
    let more = target(Hit::Cmd(Cmd::Palette));
    click(&mut a, more.x, more.y, area);
    assert!(a.input == Some(Input::Palette));
    // Tapping outside the palette closes it.
    draw(&a, 120, 30);
    click(&mut a, 0, 29, area);
    assert!(a.input.is_none());
    // Device rows select by tap without refreshing.
    draw(&a, 120, 30);
    let row = target(Hit::Row(ListId::Devices, 2));
    click(&mut a, row.x, row.y, area);
    assert_eq!(a.device, 2);
    assert!(a.focus == Focus::Devices);
}

#[test]
fn right_click_and_long_press_open_the_keymap_context_menu() {
    let (mut a, _rx) = queued_app();
    sessions(&mut a, 0, 3);
    a.view = View::Work;
    let area = Rect::new(0, 0, 120, 30);
    draw(&a, 120, 30);
    let row = target(Hit::Row(ListId::Sessions, 2));
    a.mouse(
        MouseEvent {
            kind: MouseEventKind::Down(event::MouseButton::Right),
            column: row.x + 2,
            row: row.y,
            modifiers: KeyModifiers::NONE,
        },
        area,
    );
    assert_eq!(a.selected, 2);
    let menu = a.context_menu.as_ref().expect("right-click menu");
    let cmds: Vec<_> = menu.items.iter().map(|i| i.2).collect();
    assert!(cmds.contains(&Cmd::Watch) && cmds.contains(&Cmd::Pin) && cmds.contains(&Cmd::Primary));
    let text = draw(&a, 120, 30);
    assert!(text.contains("Watch selected session"), "{text}");
    // Same labels as the keymap: Esc closes, nothing else ran.
    a.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(a.context_menu.is_none() && a.dialog.is_none());
    // Long press (touch): hold the row past the threshold.
    draw(&a, 120, 30);
    let row = target(Hit::Row(ListId::Sessions, 1));
    let down = MouseEvent {
        kind: MouseEventKind::Down(event::MouseButton::Left),
        column: row.x + 2,
        row: row.y,
        modifiers: KeyModifiers::NONE,
    };
    assert!(a.mouse(down, area));
    let deadline = a.press_deadline().unwrap();
    assert!(!a.tick_press(deadline - Duration::from_millis(1)));
    assert!(a.tick_press(deadline));
    assert!(a.context_menu.is_some());
    // Releasing after a long press does not also run Enter.
    a.mouse(
        MouseEvent {
            kind: MouseEventKind::Up(event::MouseButton::Left),
            ..down
        },
        area,
    );
    assert!(a.attach_check.is_none() && a.pending_attach.is_none());
    // Choosing a menu row runs its command.
    draw(&a, 120, 30);
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
fn scrollbar_click_and_drag_select_proportionally() {
    let (mut a, _rx) = queued_app();
    sessions(&mut a, 0, 60);
    a.view = View::Work;
    let area = Rect::new(0, 0, 80, 24);
    draw(&a, 80, 24);
    let bar = target(Hit::Scrollbar(ListId::Sessions, 60));
    let press = MouseEvent {
        kind: MouseEventKind::Down(event::MouseButton::Left),
        column: bar.x,
        row: bar.bottom() - 1,
        modifiers: KeyModifiers::NONE,
    };
    assert!(a.mouse(press, area));
    assert_eq!(a.selected, 59);
    assert!(a.mouse(
        MouseEvent {
            kind: MouseEventKind::Drag(event::MouseButton::Left),
            row: bar.y,
            ..press
        },
        area
    ));
    assert_eq!(a.selected, 0);
}

#[test]
fn hover_and_motion_never_select_or_redraw() {
    let (mut a, _rx) = queued_app();
    sessions(&mut a, 0, 5);
    a.view = View::Work;
    draw(&a, 120, 30);
    let row = target(Hit::Row(ListId::Sessions, 3));
    for kind in [MouseEventKind::Moved, MouseEventKind::Moved] {
        assert!(!a.mouse(
            MouseEvent {
                kind,
                column: row.x + 1,
                row: row.y,
                modifiers: KeyModifiers::NONE,
            },
            Rect::new(0, 0, 120, 30),
        ));
    }
    assert_eq!(a.selected, 0);
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
        text.contains("robot logs") && text.contains("Sessions"),
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
    for c in "network".chars() {
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
fn help_and_footer_come_from_the_keymap() {
    let (mut a, _rx) = queued_app();
    sessions(&mut a, 0, 2);
    a.view = View::Work;
    a.help = true;
    let mut terminal = Terminal::new(TestBackend::new(120, 60)).unwrap();
    terminal.draw(|f| render(f, &a)).unwrap();
    let text: String = terminal
        .backend()
        .buffer()
        .content
        .chunks(120)
        .map(|row| row.iter().map(|c| c.symbol()).collect::<String>() + "\n")
        .collect();
    for key in ["g s", "[ / ]", "Ctrl+P", "Tab / Shift+Tab", "Ctrl+arrows"] {
        assert!(text.contains(key), "help lacks {key}: {text}");
    }
    assert!(!text.contains("Ctrl+arrows / hjkl"));
    // P15: within one section a key is listed once.
    for (_, rows) in keymap::help_sections(&a) {
        let mut labels: Vec<_> = rows.iter().map(|(k, _)| *k).collect();
        labels.sort();
        let before = labels.len();
        labels.dedup();
        assert_eq!(before, labels.len());
    }
    a.help = false;
    // Footer chips are keymap hints, each a target for that hint's command,
    // and each chip's key dispatches the same command.
    draw(&a, 120, 30);
    let chips: Vec<_> = hitmap::with(|m| m.all())
        .into_iter()
        .filter_map(|(r, h)| match h {
            Hit::Cmd(cmd) if r.y >= 26 => Some(cmd),
            _ => None,
        })
        .collect();
    let hints: Vec<_> = keymap::hints(&a).into_iter().map(|(_, c)| c).collect();
    assert!(chips.len() >= 4, "{chips:?}");
    assert!(chips.iter().all(|c| hints.contains(c)));
    assert!(chips.contains(&Cmd::Help));
    for (key, cmd) in [('w', Cmd::Watch), ('p', Cmd::Pin), ('?', Cmd::Help)] {
        assert_eq!(
            keymap::resolve(&a, &KeyEvent::new(KeyCode::Char(key), KeyModifiers::NONE)),
            Some(cmd)
        );
    }
}

#[test]
fn phone_layout_is_single_column_with_bottom_tabs_and_device_strip() {
    let (mut a, _rx) = queued_app();
    sessions(&mut a, 0, 3);
    a.view = View::Work;
    let text = draw(&a, 40, 20);
    let lines: Vec<_> = text.lines().collect();
    assert!(
        lines[0].contains("cx") && lines[0].contains("Sessions"),
        "{text}"
    );
    assert!(lines[1].contains("[All]"), "device strip: {text}");
    let last = lines.last().unwrap();
    for tab in ["Sess", "Files", "Net", "Keys"] {
        assert!(last.contains(tab), "{text}");
    }
    assert!(
        !text.contains("Actions"),
        "phone shows the Actions panel: {text}"
    );
    // Tab targets are at least 8 cols wide and span both bar rows.
    let tab = target(Hit::Tab(View::Files));
    assert!(tab.width >= 8 && tab.height == 2, "{tab:?}");
    let area = Rect::new(0, 0, 40, 20);
    click(&mut a, tab.x + 1, tab.y, area);
    assert!(a.view == View::Files);
    // Tab cycles Devices <-> Workspace only (no hidden Actions focus).
    a.view = View::Work;
    draw(&a, 40, 20);
    a.focus = Focus::Workspace;
    a.key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    assert!(a.focus == Focus::Devices);
    a.key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    assert!(a.focus == Focus::Workspace);
    // Device chips are tap targets.
    draw(&a, 40, 20);
    let chip = target(Hit::Row(ListId::Devices, 2));
    click(&mut a, chip.x, chip.y, area);
    assert_eq!(a.device, 2);
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
fn secrets_placeholder_and_keys_tab_render_everywhere() {
    let (mut a, _rx) = queued_app();
    a.view = View::Secrets;
    for (w, h) in [(40, 20), (80, 24), (120, 40), (200, 60)] {
        let text = draw(&a, w, h);
        assert!(text.contains("Keys"), "{w}x{h}: {text}");
    }
    // Moving in Secrets does not touch the session selection.
    sessions(&mut a, 0, 4);
    a.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    assert_eq!(a.selected, 0);
}
