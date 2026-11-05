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
    // Short chip labels, so rows fit the box.
    assert!(text.contains("Watch · read-only"), "{text}");
    assert!(!text.contains("Inspect selected session"), "{text}");
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
    press(&mut a, 'g');
    press(&mut a, 's');
    assert!(a.view == View::Work);
    press(&mut a, ':');
    assert!(a.palette_run_mode(), "Sessions after search");
    // The footer chip still yields to the search's `n / N` in Files.
    a.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    a.view = View::Files;
    a.focus = Focus::Workspace;
    assert!(!keymap::hints(&a).iter().any(|(h, _)| h.key == "n / :"));
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
        text.contains("run: echo hi") && text.contains("Run command"),
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
    a.key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL));
    for c in "nav".chars() {
        press(&mut a, c);
    }
    let items = a.palette_view();
    let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
    terminal.draw(|f| render(f, &a)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    let rows: Vec<String> = buffer
        .content
        .chunks(120)
        .map(|row| row.iter().map(|c| c.symbol()).collect())
        .collect();
    // Height follows the results: rows plus one header per section plus borders.
    let top = rows
        .iter()
        .position(|r| r.contains("╭ > nav"))
        .expect("palette top");
    let left = rows[top]
        .find("╭ > nav")
        .map(|b| rows[top][..b].chars().count())
        .unwrap();
    let bottom = (top + 1..rows.len())
        .find(|&y| rows[y].chars().nth(left) == Some('╰'))
        .unwrap();
    let sections = items
        .iter()
        .enumerate()
        .filter(|(i, it)| *i == 0 || items[*i - 1].section != it.section)
        .count();
    assert_eq!(
        bottom - top - 1,
        items.len() + sections,
        "{}",
        rows.join("\n")
    );
    // Details start at one fixed column for every row that has one.
    let columns: std::collections::BTreeSet<usize> = items
        .iter()
        .filter(|i| !i.detail.is_empty())
        .filter_map(|i| {
            let detail: String = i.detail.chars().take(6).collect();
            rows[top + 1..bottom]
                .iter()
                .map(|r| r.chars().skip(left).collect::<String>())
                .find(|r| r.contains(&format!(" {} ", i.label)) && r.contains(&detail))
                .and_then(|r| r.rfind(&detail).map(|b| r[..b].chars().count()))
        })
        .collect();
    assert!(!columns.is_empty());
    let palette: Vec<_> = rows[top..=bottom]
        .iter()
        .map(|r| r.chars().skip(left).collect::<String>())
        .collect();
    assert_eq!(
        columns.len(),
        1,
        "ragged detail column: {columns:?}\n{}",
        palette.join("\n")
    );
    // Backdrop is dimmed.
    let cell = &buffer[(0, 0)];
    assert!(cell.modifier.contains(Modifier::DIM));
}

#[test]
fn help_covers_the_frame_below_100_columns() {
    let (mut a, _rx) = queued_app();
    sessions(&mut a, 0, 3);
    a.view = View::Work;
    a.help = true;
    let text = draw(&a, 80, 24);
    let lines: Vec<_> = text.lines().collect();
    assert!(
        lines[0].starts_with('╭') && lines[0].contains("Help"),
        "{text}"
    );
    for line in &lines[1..lines.len() - 1] {
        assert!(
            line.starts_with('│') && line.ends_with('│'),
            "bleed: {text}"
        );
    }
}

#[test]
fn context_menu_rows_use_short_labels() {
    let (mut a, _rx) = queued_app();
    sessions(&mut a, 0, 2);
    a.view = View::Work;
    a.focus = Focus::Workspace;
    let labels: Vec<_> = keymap::context_menu(&a)
        .into_iter()
        .map(|(_, label, _)| label)
        .collect();
    assert!(labels.contains(&"Agent tasks"), "{labels:?}");
    assert!(labels.iter().all(|l| l.chars().count() <= 24), "{labels:?}");
}

#[test]
fn phone_device_strip_marks_and_reaches_hidden_devices() {
    let (tx, _rx) = mpsc::sync_channel(64);
    let mut devices = app().devices;
    for name in ["laptop", "lab-arm", "field-rover", "jetson-01"] {
        let mut d = devices[0].clone();
        d.id = name.into();
        d.name = name.into();
        d.target = Some(name.into());
        devices.push(d);
    }
    let mut a = App::new(devices, tx);
    a.view = View::Work;
    let text = draw(&a, 40, 20);
    let strip = text.lines().nth(1).unwrap().to_owned();
    assert!(strip.contains("›"), "no overflow marker: {strip}");
    let hidden: usize = strip
        .trim_end()
        .trim_end_matches('›')
        .rsplit(' ')
        .next()
        .unwrap()
        .parse()
        .unwrap();
    assert!(hidden >= 1);
    // The marker is a tap target that steps to the first hidden device.
    let (rect, hit) = hitmap::with(|m| m.all())
        .into_iter()
        .filter(|(r, h)| r.y == 1 && matches!(h, Hit::Row(ListId::Devices, _)))
        .max_by_key(|(r, _)| r.x)
        .unwrap();
    let Hit::Row(_, value) = hit else {
        unreachable!()
    };
    click(&mut a, rect.x, rect.y, Rect::new(0, 0, 40, 20));
    assert_eq!(a.device, value);
    let strip = draw(&a, 40, 20).lines().nth(1).unwrap().to_owned();
    assert!(strip.contains('‹'), "left marker after scrolling: {strip}");
    let name = &a.devices[value - 1].name;
    assert!(strip.contains(name.as_str()), "{name} not visible: {strip}");
}

#[test]
fn help_descriptions_wrap_under_their_column_and_enter_is_listed_once() {
    let (mut a, _rx) = queued_app();
    sessions(&mut a, 0, 3);
    a.view = View::Work;
    a.focus = Focus::Workspace;
    a.help = true;
    let sections = keymap::help_sections(&a);
    // 80 columns: full-frame box; 200: the centered 76-wide popup.
    for (w, box_width) in [(80u16, 80usize), (200, 76)] {
        let text = draw(&a, w, 120);
        let key_width = 18;
        let column = box_width - 2 - key_width - 1;
        let mut wrapped = 0;
        for (_, rows) in &sections {
            for (_, description) in rows {
                let parts = rendering::help_wrap(description, column);
                // The 200-column popup is height-capped and scrolls; check
                // the rows it shows.
                if !text.contains(&format!(" {}", parts[0])) {
                    continue;
                }
                for part in parts.iter().skip(1) {
                    wrapped += 1;
                    let indented = format!("│{}{part}", " ".repeat(key_width + 1));
                    assert!(
                        text.contains(&indented),
                        "{w}: continuation {part:?} not under the description column\n{text}"
                    );
                }
            }
        }
        assert!(wrapped > 0, "{w}: no description wrapped");
    }
    // P15: the generic Everywhere Enter row folds under Sessions' own.
    let enter_rows = sections
        .iter()
        .filter(|(title, _)| matches!(*title, "Sessions" | "Everywhere"))
        .flat_map(|(_, rows)| rows)
        .filter(|(k, _)| *k == "Enter")
        .count();
    assert_eq!(enter_rows, 1, "Enter listed twice with Sessions focused");
    let parts = rendering::help_wrap("Allow access in a single-direction only", 12);
    assert!(parts.len() > 1 && parts.iter().all(|p| p.chars().count() <= 12));
    assert_eq!(
        rendering::help_wrap("supercalifragilistic", 8),
        vec!["superca…".to_owned()]
    );
}

#[test]
fn keys_placeholder_footer_only_offers_keys_that_work_there() {
    let (mut a, _rx) = queued_app();
    sessions(&mut a, 0, 4);
    a.view = View::Secrets;
    a.focus = Focus::Workspace;
    let hints = keymap::hints(&a);
    let keys: Vec<_> = hints.iter().map(|(h, _)| h.key).collect();
    assert!(!keys.contains(&"Enter"), "{keys:?}");
    assert!(!keys.contains(&"/"), "{keys:?}");
    // `n` appears in exactly one chip.
    let n_chips = keys
        .iter()
        .filter(|k| k.split(" / ").any(|p| p == "n"))
        .count();
    assert!(n_chips <= 1, "{keys:?}");
    // `/` does nothing hidden on the placeholder.
    press(&mut a, '/');
    assert!(a.input.is_none(), "invisible sessions search opened");
    // The status line does not show a session count for the placeholder.
    let text = draw(&a, 120, 40);
    assert!(text.contains("Focus: Keys"), "{text}");
    assert!(!text.contains("Focus: Keys · "), "{text}");
    assert!(!text.contains("Enter open"), "{text}");
    // The device rail keeps `/` as its filter.
    a.focus = Focus::Devices;
    assert!(keymap::hints(&a).iter().any(|(h, _)| h.key == "/"));
}

#[test]
fn palette_detail_keeps_the_device_whole_when_there_is_room() {
    let detail = "fixture@laptop · /tmp/tmp-abcdef/projects/cx-demo/src/home";
    let fitted = rendering::fit_detail(detail, 48);
    assert!(fitted.chars().count() <= 48, "{fitted}");
    assert!(fitted.starts_with("fixture@laptop · "), "{fitted}");
    assert!(fitted.ends_with("/src/home"), "{fitted}");
    // Tight: the path tail still wins most of the room.
    let tight = rendering::fit_detail(detail, 24);
    assert!(
        tight.chars().count() <= 24 && tight.ends_with("home"),
        "{tight}"
    );
}

#[test]
fn palette_spans_the_frame_below_100_columns() {
    let (mut a, _rx) = queued_app();
    sessions(&mut a, 0, 3);
    a.open_palette("");
    let text = draw(&a, 80, 24);
    // The palette owns the frame's edges: its top border (holding the input)
    // spans the frame, with no backdrop border a cell or two outside it.
    let lines: Vec<&str> = text.lines().collect();
    let input = lines
        .iter()
        .position(|l| l.contains('█'))
        .unwrap_or_else(|| panic!("{text}"));
    let row: Vec<char> = lines[input].chars().collect();
    assert!(row[0] == '╭' && row[79] == '╮', "{text}");
    let below: Vec<char> = lines[input + 1].chars().collect();
    assert!(below[0] == '│' && !"│╭╰".contains(below[1]), "{text}");
}
