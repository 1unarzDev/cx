//! Layout and rendering only; action dispatch and panel effects remain in the controller.
use super::*;

#[cfg(test)]
pub(super) fn render(frame: &mut Frame<'_>, app: &App) {
    render_at(
        frame,
        app,
        None,
        app.notice_started
            .map_or_else(Instant::now, |start| start + notifications::TRANSITION),
    )
}
pub(super) fn render_with_native(
    frame: &mut Frame<'_>,
    app: &App,
    native: Option<&mut crate::terminal_preview::NativePreview>,
) {
    render_at(frame, app, native, Instant::now());
}
pub(super) fn render_at(
    frame: &mut Frame<'_>,
    app: &App,
    native: Option<&mut crate::terminal_preview::NativePreview>,
    now: Instant,
) {
    let area = frame.area();
    if area.width < 36 || area.height < 10 {
        frame.render_widget(
            Paragraph::new("cx · enlarge terminal\nCtrl+P actions · Ctrl+C quit")
                .wrap(Wrap { trim: false }),
            area,
        );
        return;
    }
    hitmap::begin();
    let areas = frame_areas(app, area);
    let phone = areas.phone;
    let show_search = areas.show_search;
    let query = filters::visible_query(app);
    render_header(frame, app, areas.header);
    let sidebar_width = areas.sidebar_width;
    let content = [areas.rail, areas.main];
    let mut device_items = vec![ListItem::new("All devices")];
    for (i, d) in app
        .devices
        .iter()
        .enumerate()
        .filter(|(i, _)| app.device_matches(*i))
    {
        let status = if app.work[i].loading {
            "checking"
        } else if app.work[i].error.is_some() {
            "unavailable"
        } else if app.work[i].fetched == 0
            || transport::now().saturating_sub(app.work[i].fetched) > 60
        {
            "unknown"
        } else {
            "ready"
        };
        let indicator = match status {
            "ready" => {
                if ascii() {
                    "+"
                } else {
                    "●"
                }
            }
            "checking" => {
                if ascii() {
                    "~"
                } else {
                    "◌"
                }
            }
            "unavailable" => {
                if ascii() {
                    "x"
                } else {
                    "○"
                }
            }
            "down" => {
                if ascii() {
                    "o"
                } else {
                    "○"
                }
            }
            _ => "?",
        };
        let dot_style = if no_color() {
            Style::default()
        } else {
            Style::default().fg(match status {
                "ready" => Color::Green,
                "checking" => Color::Cyan,
                "unavailable" => Color::Yellow,
                "down" => Color::DarkGray,
                _ => Color::Reset,
            })
        };
        let access_marker = match app.device_policy(i).access {
            store::AccessMode::Core => "",
            store::AccessMode::Directed => " *",
        };
        device_items.push(ListItem::new(Line::from(vec![
            Span::styled(format!("{indicator} "), dot_style),
            Span::raw(fit_label(
                &format!("{access_marker} {}", d.name),
                sidebar_width.saturating_sub(8) as usize,
            )),
        ])));
    }
    let sidebar = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length((app.devices.len() as u16 + 3).min(content[0].height / 2)),
            Constraint::Length(
                (sidebar_actions(app).len() as u16 + 2)
                    .min(content[0].height.saturating_sub(
                        (app.devices.len() as u16 + 3).min(content[0].height / 2) + 4,
                    ))
                    .max(5),
            ),
            Constraint::Min(0),
        ])
        .split(content[0]);
    *app.panels.borrow_mut() = if phone {
        vec![(Focus::Devices, false, areas.strip)]
    } else {
        vec![
            (Focus::Devices, false, sidebar[0]),
            (Focus::Actions, false, sidebar[1]),
        ]
    };
    if phone {
        render_device_strip(frame, app, areas.strip);
    }
    if app.view != View::Files || app.other_browser.is_none() {
        app.panels
            .borrow_mut()
            .push((Focus::Workspace, app.destination_active, content[1]));
    }
    let device_rows = app.device_rows();
    let device_selected = device_rows.iter().position(|d| *d == app.device);
    let device_height = sidebar[0].height.saturating_sub(2) as usize;
    let device_offset = navigation::viewport(
        "devices",
        device_selected.unwrap_or(0),
        device_rows.len(),
        device_height,
    );
    let mut state = ratatui::widgets::ListState::default()
        .with_selected(device_selected)
        .with_offset(device_offset);
    if !phone {
        let inner = Rect::new(
            sidebar[0].x + 1,
            sidebar[0].y + 1,
            sidebar[0].width.saturating_sub(2),
            device_height as u16,
        );
        for (line, index) in (device_offset..device_rows.len()).enumerate() {
            if line >= device_height {
                break;
            }
            hitmap::register(
                Rect::new(inner.x, inner.y + line as u16, inner.width, 1),
                hitmap::Hit::Row(hitmap::ListId::Devices, device_rows[index]),
            );
        }
    }
    frame.render_stateful_widget(
        List::new(device_items)
            .block(
                block(
                    if app.device_filter.is_empty() {
                        "Devices".into()
                    } else {
                        "Devices · filter".into()
                    },
                    app.focus == Focus::Devices,
                )
                .padding(Padding::horizontal(1)),
            )
            .highlight_style(if app.focus == Focus::Devices {
                selected_style()
            } else {
                accent().add_modifier(Modifier::BOLD)
            })
            .highlight_symbol(if ascii() { "> " } else { "› " }),
        sidebar[0],
        &mut state,
    );
    if !phone {
        hitmap::scrollbar(
            frame,
            hitmap::ListId::Devices,
            Rect::new(
                sidebar[0].x,
                sidebar[0].y + 1,
                sidebar[0].width,
                device_height as u16,
            ),
            state.offset(),
            device_rows.len(),
            device_height,
            device_rows.len(),
        );
    }
    let actions = sidebar_actions(app);
    let items = actions
        .iter()
        .map(|(action, label)| {
            let active = app.active_transfer_keys().len();
            if *action == Action::Jobs && active > 0 {
                ListItem::new(Line::styled(
                    if sidebar_width < 18 {
                        format!("{} Sending", transfer_spinner(app.transfer_frame, ascii()))
                    } else {
                        format!(
                            "{} Transfers {active}",
                            transfer_spinner(app.transfer_frame, ascii())
                        )
                    },
                    accent(),
                ))
            } else {
                ListItem::new(*label)
            }
        })
        .collect::<Vec<_>>();
    let mut state =
        ratatui::widgets::ListState::default().with_selected(if app.focus == Focus::Actions {
            Some(app.side_selected.min(actions.len().saturating_sub(1)))
        } else {
            None
        });
    frame.render_stateful_widget(
        List::new(items)
            .block(
                block("Actions".into(), app.focus == Focus::Actions)
                    .padding(Padding::horizontal(1)),
            )
            .highlight_style(selected_style()),
        sidebar[1],
        &mut state,
    );
    if !phone {
        for (line, (action, _)) in actions.iter().enumerate() {
            let y = sidebar[1].y + 1 + line as u16;
            if y + 1 >= sidebar[1].bottom() {
                break;
            }
            hitmap::register(
                Rect::new(sidebar[1].x + 1, y, sidebar[1].width.saturating_sub(2), 1),
                hitmap::Hit::Cmd(keymap::Cmd::Do(*action)),
            );
        }
    }
    let details = if app.view == View::Containers {
        app.selected_container()
            .map(|(d, c)| {
                format!(
                    "{}\n{}\n{}\n{}\nNetwork {}\nUser {}\n{}\nEnter actions · n shell",
                    safe_label(&app.devices[d].name),
                    safe_label(&c.name),
                    if c.devcontainer {
                        "Devcontainer"
                    } else {
                        "Docker · inspection"
                    },
                    safe_label(&c.state),
                    safe_label(&c.network),
                    safe_label(&c.user),
                    safe_label(&c.folder)
                )
            })
            .unwrap_or_else(|| "Choose a container\nEnter expands device\nr refresh".into())
    } else if app.view == View::Network {
        app.network_rows()
            .get(app.network_selected)
            .map(|row| {
                let owner = row["_device"].as_u64().unwrap_or(0) as usize;
                let observer = safe_label(&app.devices[owner].name);
                let arrow = if ascii() { "->" } else { "→" };
                let mut lines = vec!["Route".to_owned(), format!("viewer {arrow} {observer}")];
                if !row["_peer"].is_u64() {
                    lines.push(format!(
                        "{arrow} {}",
                        safe_label(row["address"].as_str().unwrap_or("unknown"))
                    ));
                    lines.push(format!(
                        "Link {}",
                        safe_label(row["interface"].as_str().unwrap_or("unknown"))
                    ));
                }
                if let Some(Some(hops)) = app.network_jump_routes.get(&owner) {
                    if !hops.is_empty() {
                        lines.push("SSH hops (saved)".into());
                        lines.extend(hops.iter().map(|hop| safe_label(hop)));
                    }
                }
                lines.join("\n")
            })
            .unwrap_or_else(|| "No devices or LAN observations".into())
    } else if app.view == View::Files {
        String::new() // Render the selected file context as styled hierarchy below.
    } else if let Some((i, s)) = app.selected_session() {
        if let Some(scope) = &s.container {
            format!(
                "Host {}\n{} {}\nFolder {}\nEnter open · n new",
                identity(&app.devices[i]),
                menus::session_label(app, i, &s),
                safe_label(&scope.name),
                session_folder_label(&s.directory)
            )
        } else {
            format!(
                "Host {}\nFolder {}\nEnter open · n new",
                identity(&app.devices[i]),
                session_folder_label(&s.directory)
            )
        }
    } else if let Some(i) = app.actual_device() {
        format!(
            "Host {}\n{} sessions\nn new · Ctrl+P actions",
            identity(&app.devices[i]),
            app.work[i].sessions.len()
        )
    } else {
        "Choose a device\nn new · Ctrl+P actions".into()
    };
    if sidebar[2].height > 3 {
        let details = if app.view == View::Network {
            details
        } else {
            details
                .lines()
                .map(|line| fit_label(line, sidebar_width.saturating_sub(2) as usize))
                .collect::<Vec<_>>()
                .join("\n")
        };
        let selected_content =
            if let Some(b) = app.browser.as_ref().filter(|_| app.view == View::Files) {
                let entries = app.visible_entries();
                let width = sidebar_width.saturating_sub(2) as usize;
                let height = sidebar[2].height.saturating_sub(1) as usize;
                let mut lines = file_context_hierarchy(
                    &app.devices[b.device].name,
                    &b.display_path,
                    entries.get(b.selected),
                    width,
                    height.saturating_sub(if b.container.is_some() { 2 } else { 0 }),
                );
                if let Some(scope) = &b.container {
                    lines.insert(
                        1,
                        Line::styled(
                            fit_label(&scope.name, width),
                            accent().add_modifier(Modifier::BOLD),
                        ),
                    );
                    lines.insert(
                        2,
                        Line::styled(
                            fit_label(
                                if width < 16 {
                                    "Container"
                                } else {
                                    "Container files"
                                },
                                width,
                            ),
                            muted(),
                        ),
                    );
                }
                if b.marked.len() > 0 {
                    lines.push(Line::styled(
                        format!("{} marked · t send", b.marked.len()),
                        muted(),
                    ));
                }
                if let Some(other) = &app.other_browser {
                    let destination = if app.destination_active { b } else { other };
                    lines.push(Line::styled(
                        format!("To {}", safe_label(&app.devices[destination.device].name)),
                        muted(),
                    ));
                    lines.push(Line::raw(compact_path(
                        &destination.display_path,
                        sidebar_width.saturating_sub(2) as usize,
                    )));
                }
                Paragraph::new(lines)
            } else {
                Paragraph::new(details)
            };
        frame.render_widget(
            selected_content.wrap(Wrap { trim: false }).block(
                Block::default()
                    .padding(Padding::horizontal(1))
                    .title(Line::from(Span::styled(
                        " Selected ",
                        accent().add_modifier(Modifier::BOLD),
                    )))
                    .borders(Borders::TOP),
            ),
            sidebar[2],
        );
    }
    let split_workspace = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(5),
            Constraint::Length(
                if app.view == View::Files && app.transfer_drawer && content[1].height >= 12 {
                    4
                } else {
                    0
                },
            ),
        ])
        .split(content[1]);
    let workspace = split_workspace[0];
    match app.view {
        View::Work => {
            let workspace = session_preview::render(frame, app, workspace);
            // Only the list owns wheel navigation; the snapshot and metadata
            // footer must never move the selected session behind the pointer.
            app.panels
                .borrow_mut()
                .retain(|(focus, _, _)| *focus != Focus::Workspace);
            app.panels
                .borrow_mut()
                .push((Focus::Workspace, false, workspace));
            let rows = app.session_rows();
            if rows.is_empty() {
                let checking = app
                    .work
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| app.device == 0 || app.device == i + 1)
                    .any(|(_, w)| w.loading);
                let errors: Vec<_> = app
                    .work
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| app.device == 0 || app.device == i + 1)
                    .filter_map(|(_, w)| w.error.as_deref())
                    .collect();
                let mut lines = vec![Line::from("")];
                if checking {
                    lines.push(Line::from(Span::styled("Checking sessions…", accent())));
                    lines.push(Line::from("Waiting for device evidence"));
                } else if !errors.is_empty() {
                    lines.push(Line::from(Span::styled(
                        "Device unavailable",
                        tint(Color::Yellow),
                    )));
                    for error in errors {
                        lines.push(Line::from(safe_text(error)));
                    }
                    lines.push(Line::from("Ctrl+P → Refresh"));
                } else if !app.search.is_empty() {
                    lines.push(Line::from("No matching sessions"));
                    lines.push(Line::from("Esc clears search"));
                    lines.push(Line::from("n starts a new session"));
                } else {
                    lines.push(Line::from(Span::styled(
                        "No live sessions",
                        Style::default().add_modifier(Modifier::BOLD),
                    )));
                    lines.push(Line::from("Start a shell in this device's home."));
                    lines.push(Line::from(""));
                    lines.push(Line::from(Span::styled(
                        if app.creating {
                            "Creating session…"
                        } else {
                            "[ Enter / n  New session ]"
                        },
                        if app.focus == Focus::Workspace && !app.creating {
                            selected_style()
                        } else {
                            accent().add_modifier(Modifier::BOLD)
                        },
                    )));
                    if local_only(app) {
                        lines.push(Line::from(""));
                        lines.push(Line::from("Use this computer now."));
                        lines.push(Line::from(Span::styled(
                            "Network · discover reachable devices",
                            accent().add_modifier(Modifier::BOLD),
                        )));
                        lines.push(Line::from("Ctrl+P · Add by SSH address"));
                    }
                }
                frame.render_widget(
                    Paragraph::new(lines).wrap(Wrap { trim: false }).block(
                        block("Sessions".into(), app.focus == Focus::Workspace)
                            .padding(Padding::horizontal(1)),
                    ),
                    workspace,
                );
            } else {
                let selected_error = rows
                    .get(app.selected)
                    .and_then(|(device, _)| app.work[*device].error.as_deref());
                let table_area = if let Some(error) = selected_error {
                    let areas = Layout::default()
                        .direction(Direction::Vertical)
                        .constraints([Constraint::Min(3), Constraint::Length(2)])
                        .split(workspace);
                    frame.render_widget(
                        Paragraph::new(format!(
                            "{}\nCtrl+P → Refresh · state unconfirmed",
                            fit_label(&safe_text(error), usize::from(areas[1].width))
                        ))
                        .style(muted()),
                        areas[1],
                    );
                    areas[0]
                } else {
                    workspace
                };
                let compact = workspace.width < 70;
                let state_width = if rows.iter().any(|(_, s)| s.activity.is_some()) {
                    10
                } else {
                    8
                };
                let mut display_rows = Vec::new();
                let mut session_display_rows = Vec::new();
                let mut selected_row = 0;
                let mut last_project = None;
                for (index, (i, s)) in rows.iter().enumerate() {
                    if last_project != Some(s.directory.as_str()) {
                        if last_project.is_some() {
                            display_rows.push(Row::new(vec![Cell::from("")]).height(1));
                        }
                        display_rows.push(
                            Row::new(if compact {
                                vec![
                                    Cell::from(session_folder_label(&s.directory)),
                                    Cell::from(""),
                                ]
                            } else {
                                vec![
                                    Cell::from(""),
                                    Cell::from(session_folder_label(&s.directory)),
                                    Cell::from(""),
                                    Cell::from(""),
                                ]
                            })
                            .style(muted().add_modifier(Modifier::BOLD))
                            .height(1),
                        );
                        last_project = Some(s.directory.as_str());
                    }
                    session_display_rows.push((display_rows.len(), index));
                    if index == app.selected {
                        selected_row = display_rows.len();
                    }
                    let status = app.session_status(*i, s);
                    let fresh = status.starts_with('+');
                    let badge = format!("[{}]", safe_label(menus::session_label(app, *i, s)));
                    let name = if s.agent_tasks.is_empty() {
                        session_preview::label(s)
                    } else {
                        format!(
                            "{} · {} tasks",
                            session_preview::label(s),
                            s.agent_tasks.len()
                        )
                    };
                    let name = format!("{}{}", if app.pinned(*i, s) { "* " } else { "" }, name);
                    let name = match s.origin.as_ref().filter(|_| s.id.starts_with("ext-")) {
                        Some(origin) => format!("{name} · {}", origin_label(&origin.kind)),
                        None => name,
                    };
                    let cells = vec![
                        Cell::from(badge).style(if no_color() {
                            Style::default()
                        } else {
                            Style::default().fg(match s.provider.as_str() {
                                "claude" => Color::Yellow,
                                "codex" => Color::Cyan,
                                _ => Color::Gray,
                            })
                        }),
                        Cell::from(if compact && s.container.is_some() {
                            format!("{} · {}", menus::session_label(app, *i, s), name)
                        } else {
                            name
                        }),
                        Cell::from(identity(&app.devices[*i])),
                        Cell::from(if !fresh {
                            status.to_string()
                        } else if s.activity.is_some() && !session_activity_current(&app.work[*i]) {
                            "? state".into()
                        } else if session_busy(s) {
                            let frames = if ascii() {
                                ["|", "/", "-", "\\"]
                            } else {
                                ["◐", "◓", "◑", "◒"]
                            };
                            format!(
                                "{} {}",
                                frames[app.transfer_frame % frames.len()],
                                s.activity.as_deref().unwrap_or("")
                            )
                        } else if let Some(activity) = &s.activity {
                            safe_label(activity)
                        } else if s.external {
                            if ascii() {
                                "+ ext".into()
                            } else {
                                "● ext".into()
                            }
                        } else {
                            if ascii() {
                                "+ live".into()
                            } else {
                                "● live".into()
                            }
                        })
                        .style(if !fresh { muted() } else { accent() }),
                    ];
                    display_rows.push(
                        Row::new(if compact {
                            vec![cells[1].clone(), cells[3].clone()]
                        } else {
                            cells
                        })
                        .style(if fresh {
                            Style::default()
                        } else {
                            muted()
                        }),
                    );
                }
                // Remembered viewport with scrolloff: the list moves only when
                // the bar nears an edge, never per frame (spec 12.4).
                let body_height = table_area.height.saturating_sub(4) as usize;
                let offset =
                    navigation::viewport("sessions", selected_row, display_rows.len(), body_height);
                let display_len = display_rows.len();
                let mut state = TableState::default()
                    .with_selected((app.selected < rows.len()).then_some(selected_row))
                    .with_offset(offset);
                let table = Table::new(
                    display_rows,
                    if compact {
                        vec![Constraint::Min(10), Constraint::Length(state_width)]
                    } else {
                        vec![
                            Constraint::Length(
                                if rows.iter().any(|(_, s)| s.container.is_some()) {
                                    14
                                } else {
                                    9
                                },
                            ),
                            Constraint::Min(10),
                            Constraint::Length(if workspace.width > 65 { 24 } else { 18 }),
                            Constraint::Length(state_width),
                        ]
                    },
                )
                .header(
                    Row::new(if compact {
                        vec!["SESSION", "STATE"]
                    } else {
                        vec!["AGENT", "SESSION", "EXECUTION", "STATE"]
                    })
                    .style(muted())
                    .bottom_margin(1),
                )
                .column_spacing(if compact { 1 } else { 2 })
                .block(block(
                    format!("Sessions · {} sessions", rows.len()),
                    app.focus == Focus::Workspace,
                ))
                .row_highlight_style(selected_style())
                .highlight_symbol(if ascii() { "> " } else { "› " });
                frame.render_stateful_widget(table, table_area, &mut state);
                // Table offset includes group headers/spacers. Hit boxes must use
                // the rendered rows, not index arithmetic over logical sessions.
                let body_y = table_area.y.saturating_add(3); // border + header + margin
                let bottom = table_area.bottom().saturating_sub(1);
                for (row, index) in session_display_rows {
                    if row < state.offset() {
                        continue;
                    }
                    let y = body_y.saturating_add((row - state.offset()) as u16);
                    if y < bottom {
                        hitmap::register(
                            Rect::new(table_area.x + 1, y, table_area.width.saturating_sub(2), 1),
                            hitmap::Hit::Row(hitmap::ListId::Sessions, index),
                        );
                    }
                }
                hitmap::scrollbar(
                    frame,
                    hitmap::ListId::Sessions,
                    Rect::new(table_area.x, body_y, table_area.width, body_height as u16),
                    state.offset(),
                    display_len,
                    body_height,
                    rows.len(),
                );
            }
        }
        View::Files => {
            if let Some(b) = &app.browser {
                if b.preview.is_some()
                    && b.preview_rich
                        .as_ref()
                        .is_some_and(|p| matches!(p.kind.as_str(), "image" | "pdf"))
                {
                    if app.other_browser.is_some() {
                        app.panels.borrow_mut().push((
                            Focus::Workspace,
                            app.destination_active,
                            workspace,
                        ));
                    }
                    render_preview_with_native(
                        frame,
                        workspace,
                        b,
                        b.preview.as_deref().unwrap_or(""),
                        app.focus == Focus::Workspace,
                        native,
                    );
                } else if let Some(other) = &app.other_browser {
                    let panes = Layout::default()
                        .direction(if workspace.width >= 52 {
                            Direction::Horizontal
                        } else {
                            Direction::Vertical
                        })
                        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                        .split(workspace);
                    app.panels.borrow_mut().extend([
                        (Focus::Workspace, false, panes[0]),
                        (Focus::Workspace, true, panes[1]),
                    ]);
                    let (source, destination) = if app.destination_active {
                        (other, b)
                    } else {
                        (b, other)
                    };
                    render_browser(
                        frame,
                        source,
                        &app.devices[source.device],
                        panes[0],
                        !app.destination_active && app.focus == Focus::Workspace,
                        "Source",
                        app.clipboard.as_ref(),
                        &app.folder_agents(source),
                    );
                    render_browser(
                        frame,
                        destination,
                        &app.devices[destination.device],
                        panes[1],
                        app.destination_active && app.focus == Focus::Workspace,
                        &format!("Destination · {}", app.conflict_policy()),
                        app.clipboard.as_ref(),
                        &app.folder_agents(destination),
                    );
                } else {
                    let label = app
                        .launch_provider
                        .as_ref()
                        .map(|p| format!("Start {p} here · n"))
                        .unwrap_or_else(|| "Files".into());
                    render_browser(
                        frame,
                        b,
                        &app.devices[b.device],
                        workspace,
                        app.focus == Focus::Workspace,
                        &label,
                        app.clipboard.as_ref(),
                        &app.folder_agents(b),
                    );
                }
            }
        }
        View::Containers => render_containers(frame, app, workspace),
        View::Network => render_network(frame, app, workspace),
        View::Secrets => render_secrets(frame, app, workspace),
    }
    if split_workspace[1].height > 0 {
        let rows = app.job_rows();
        let mut lines = rows
            .iter()
            .take(2)
            .map(|(_, j)| {
                Line::from(vec![
                    Span::styled(
                        format!(" {} ", transfer_status(j)),
                        tint(if j["status"] == "failed" {
                            Color::Red
                        } else if j["status"] == "complete" {
                            Color::Green
                        } else {
                            Color::Yellow
                        }),
                    ),
                    Span::raw(format!(
                        "{} {}",
                        if j["operation"] == "move" {
                            "move"
                        } else {
                            "copy"
                        },
                        transfer_name(j)
                    )),
                    Span::styled(
                        format!(
                            " · {}",
                            if j["source_host"] == j["destination_host"] {
                                safe_label(j["source_host"].as_str().unwrap_or("?"))
                            } else {
                                format!(
                                    "{} → {}",
                                    safe_label(j["source_host"].as_str().unwrap_or("?")),
                                    safe_label(j["destination_host"].as_str().unwrap_or("?"))
                                )
                            }
                        ),
                        muted(),
                    ),
                ])
            })
            .collect::<Vec<_>>();
        if !app.file_errors.is_empty() {
            lines.insert(
                0,
                Line::from(Span::styled(
                    format!(
                        " × {} actions rejected · {}",
                        app.file_errors.len(),
                        app.file_errors.last().unwrap()
                    ),
                    tint(Color::Red),
                )),
            );
            lines.truncate(2);
        }
        frame.render_widget(
            Paragraph::new(if lines.is_empty() {
                vec![Line::from(" Submitting selected items…")]
            } else {
                lines
            })
            .block(block("Transfers · T details".into(), false)),
            split_workspace[1],
        );
    }
    // Phone: one chip row, then the bottom tab bar (separator + tabs).
    let (key_rows, tail) = if phone { (1, 2) } else { (2, 1) };
    let footer = Layout::default()
        .direction(Direction::Vertical)
        .constraints(if show_search {
            vec![
                Constraint::Length(3),
                Constraint::Length(1),
                Constraint::Length(key_rows),
                Constraint::Length(tail),
            ]
        } else {
            vec![
                Constraint::Length(1),
                Constraint::Length(key_rows),
                Constraint::Length(tail),
            ]
        })
        .split(areas.footer);
    if phone {
        render_tab_bar(frame, app, footer[footer.len() - 1]);
    }
    let status_row = if show_search { 1 } else { 0 };
    let key_row = if show_search { 2 } else { 1 };
    let mut unique_jobs = std::collections::BTreeMap::new();
    for j in app
        .jobs
        .values()
        .filter_map(|v| v["jobs"].as_array())
        .flatten()
    {
        unique_jobs.insert(
            (
                j["key"].as_str().unwrap_or(""),
                j["source_host"].as_str().unwrap_or(""),
                j["destination_host"].as_str().unwrap_or(""),
            ),
            j,
        );
    }
    let jobs = unique_jobs.values().copied().collect::<Vec<_>>();
    let active = jobs
        .iter()
        .filter(|j| {
            matches!(
                j["status"].as_str(),
                Some("running" | "queued" | "submitting")
            )
        })
        .count();
    let failed = jobs.iter().filter(|j| j["status"] == "failed").count();
    let ready = app
        .work
        .iter()
        .filter(|w| {
            w.fetched > 0 && transport::now().saturating_sub(w.fetched) < 60 && w.error.is_none()
        })
        .count();
    let errors = app.work.iter().filter(|w| w.error.is_some()).count();
    let ready = if app.view == View::Containers {
        app.containers
            .keys()
            .filter(|d| app.device == 0 || app.device == **d + 1)
            .count()
    } else {
        ready
    };
    let errors = if app.view == View::Containers {
        app.container_errors
            .keys()
            .filter(|d| app.device == 0 || app.device == **d + 1)
            .count()
    } else {
        errors
    };
    let shown = if app.view == View::Containers {
        app.container_rows()
            .iter()
            .filter(|(_, c)| c.is_some())
            .count()
    } else if app.view == View::Network {
        app.network_rows().len()
    } else if app.view == View::Files {
        app.visible_entries().len()
    } else {
        app.session_rows().len()
    };
    let total = if app.view == View::Containers {
        app.containers
            .iter()
            .filter(|(d, _)| app.device == 0 || app.device == **d + 1)
            .map(|(_, c)| c.len())
            .sum()
    } else if app.view == View::Network {
        shown
    } else if app.view == View::Files {
        app.browser.as_ref().map(|b| b.entries.len()).unwrap_or(0)
    } else {
        app.session_rows_for("").len()
    };
    let status_sections = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(if area.width < 60 {
            vec![
                Constraint::Length(0),
                Constraint::Percentage(55),
                Constraint::Percentage(45),
            ]
        } else {
            vec![
                Constraint::Percentage(30),
                Constraint::Percentage(30),
                Constraint::Percentage(40),
            ]
        })
        .split(footer[status_row]);
    let healthy = if no_color() {
        Style::default()
    } else {
        Style::default().fg(Color::Green)
    };
    let warning = if no_color() {
        Style::default()
    } else {
        Style::default().fg(Color::Yellow)
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            format!(
                " {} {ready}/{} devices  ",
                if ascii() { "+" } else { "●" },
                app.devices.len()
            ),
            if errors > 0 {
                warning
            } else if ready == 0 {
                muted()
            } else {
                healthy
            },
        )])),
        status_sections[0],
    );
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                format!("Focus: {}", app.focus_label()),
                accent().add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                if app.dialog.is_some() || app.help {
                    String::new()
                } else {
                    format!(" · {shown}/{total}")
                },
                muted(),
            ),
        ]))
        .alignment(ratatui::layout::Alignment::Center),
        status_sections[1],
    );
    let transfer = if active > 0 && phone {
        // The phone layout has no Actions panel; the status line animates.
        format!(
            "{} Transfers {active} ",
            transfer_spinner(app.transfer_frame, ascii())
        )
    } else if active > 0 {
        format!("{} {active} active", if ascii() { "~" } else { "↔" })
    } else if failed > 0 {
        format!("{} {failed} failed", if ascii() { "x" } else { "×" })
    } else if !jobs.is_empty() {
        let completed = jobs.iter().filter(|j| j["status"] == "complete").count();
        let skipped = jobs
            .iter()
            .map(|j| j["skipped"].as_u64().unwrap_or(0))
            .sum::<u64>();
        if skipped > 0 {
            format!(
                "{} {completed} done · {skipped} skipped",
                if ascii() { "+" } else { "●" }
            )
        } else {
            format!("{} {completed} complete", if ascii() { "+" } else { "●" })
        }
    } else {
        format!("{} idle", if ascii() { "~" } else { "↔" })
    };
    frame.render_widget(
        Paragraph::new(transfer)
            .style(if failed > 0 { warning } else { accent() })
            .alignment(ratatui::layout::Alignment::Right),
        status_sections[2],
    );
    if show_search {
        let editing = matches!(
            app.input,
            Some(Input::Search | Input::PreviewSearch | Input::Filter)
        );
        let text = if editing { app.text.as_str() } else { query };
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(
                    if app.input == Some(Input::Filter) {
                        " f  "
                    } else {
                        " /  "
                    },
                    accent().add_modifier(Modifier::BOLD),
                ),
                Span::raw(safe_label(text)),
            ]))
            .block(block(
                if app.input == Some(Input::Filter) {
                    "Filter".into()
                } else if let Some(b) = app
                    .browser
                    .as_ref()
                    .filter(|b| b.preview.is_some() && app.view == View::Files)
                {
                    let count = preview_matches(
                        &preview_display_lines(b, b.preview.as_deref().unwrap_or("")),
                        &b.preview_find.query,
                    )
                    .len();
                    format!(
                        "Search · {}/{}",
                        if count == 0 {
                            0
                        } else {
                            (b.preview_find.selected + 1).min(count)
                        },
                        count
                    )
                } else {
                    "Search".into()
                },
                editing,
            )),
            footer[0],
        );
        if editing {
            let width = Span::raw(safe_label(text)).width() as u16;
            frame.set_cursor_position((
                footer[0].x + 5 + width.min(footer[0].width.saturating_sub(7)),
                footer[0].y + 1,
            ));
        }
    }
    let mut hints = if app.pane_focused() {
        vec![("Ctrl+]", "Sessions"), ("Ctrl+O", "Expand terminal")]
    } else if app.help {
        vec![("↑↓", "Scroll"), ("PgUpDn", "Page"), ("Esc", "Close")]
    } else if let Some(dialog) = &app.dialog {
        if matches!(dialog, Dialog::AgentTasks(..)) {
            vec![
                ("↑↓", "Choose"),
                ("Enter", "Details"),
                ("Tab", "Focus"),
                ("PgUpDn", "Scroll"),
                ("Esc", "Close"),
            ]
        } else if matches!(dialog, Dialog::Jobs) {
            vec![
                ("c", "Cancel job"),
                ("r", "Retry job"),
                ("Esc", "Back"),
                ("Tab", "Details"),
                ("PgUpDn", "Scroll"),
                (
                    "Enter",
                    if app.dialog_detail_focus {
                        "Back"
                    } else {
                        "Details"
                    },
                ),
            ]
        } else if matches!(
            dialog,
            Dialog::Delete(..) | Dialog::StopShell(..) | Dialog::RemoveDevice(..) | Dialog::Reset
        ) {
            vec![
                (
                    "↑↓",
                    if app.dialog_detail_focus {
                        "Scroll"
                    } else {
                        "Choose"
                    },
                ),
                (
                    "Enter",
                    if app.dialog_detail_focus {
                        "Back"
                    } else {
                        "Confirm"
                    },
                ),
                (
                    "y / n",
                    if matches!(dialog, Dialog::StopShell(..)) {
                        "Stop / keep"
                    } else if matches!(dialog, Dialog::RemoveDevice(..)) {
                        "Remove / keep"
                    } else if matches!(dialog, Dialog::Reset) {
                        "Reset / cancel"
                    } else {
                        "Delete / cancel"
                    },
                ),
                ("Tab", "Details"),
                ("PgUpDn", "Scroll"),
                ("Home", "Top"),
            ]
        } else {
            vec![("↑↓", "Choose"), ("Enter", "Confirm"), ("Esc", "Cancel")]
        }
    } else if app.input == Some(Input::Add) {
        vec![("Enter", "Connect"), ("Esc", "Cancel"), ("Ctrl U", "Clear")]
    } else if matches!(
        app.input,
        Some(Input::Rename | Input::Mkdir | Input::Palette)
    ) {
        vec![
            (
                "Enter",
                if app.palette_run_mode() {
                    "Run"
                } else {
                    "Confirm"
                },
            ),
            ("Esc", "Cancel"),
            ("Ctrl U", "Clear"),
        ]
    } else if app.input == Some(Input::PreviewSearch)
        || (app.input == Some(Input::Search) && app.view == View::Files)
    {
        vec![("↑↓", "Match"), ("Enter", "Done"), ("Esc", "Done")]
    } else if matches!(
        app.input,
        Some(Input::Search | Input::PreviewSearch | Input::Filter)
    ) {
        vec![
            ("↑↓", "Select"),
            (
                "Enter",
                if app.input == Some(Input::Filter) {
                    "Done"
                } else {
                    "Open"
                },
            ),
            ("Esc", "Done"),
            ("Tab", "Focus"),
        ]
    } else {
        Vec::new()
    };
    // Workspace hints come from the keymap, so they match dispatch and help
    // and each chip is a tap target running the same command as its key.
    let modal = !hints.is_empty();
    let mut chips: Vec<(&str, &str, Option<keymap::Cmd>)> = if modal {
        hints.drain(..).map(|(k, l)| (k, l, None)).collect()
    } else {
        keymap::hints(app)
            .into_iter()
            .map(|(hint, cmd)| (hint.key, hint.label, Some(cmd)))
            .collect()
    };
    let columns = if phone {
        3
    } else if footer[key_row].width >= 60 {
        3
    } else {
        2
    };
    let capacity = columns * usize::from(key_rows);
    if columns == 2
        && !matches!(
            app.input,
            Some(Input::Search | Input::PreviewSearch | Input::Filter)
        )
    {
        chips.retain(|(_, label, _)| !matches!(*label, "Search" | "Focus"));
    }
    // Help stays reachable by tap even when the chips overflow.
    if !modal && chips.len() > capacity && capacity > 1 {
        if let Some(help) = chips
            .iter()
            .position(|(_, _, cmd)| *cmd == Some(keymap::Cmd::Help))
        {
            let help = chips.remove(help);
            chips.truncate(capacity - 1);
            chips.push(help);
        }
    }
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints(vec![Constraint::Length(1); usize::from(key_rows)])
        .split(footer[key_row]);
    for (i, (key, label, cmd)) in chips.into_iter().take(capacity).enumerate() {
        let key = if ascii() && key == "↑↓" {
            "j/k"
        } else {
            key
        };
        let cells = Layout::default()
            .direction(Direction::Horizontal)
            .constraints(vec![Constraint::Ratio(1, columns as u32); columns])
            .split(rows[i / columns]);
        let key_style = if no_color() {
            Style::default().add_modifier(Modifier::BOLD)
        } else {
            accent().add_modifier(Modifier::BOLD)
        };
        let cell = cells[i % columns];
        let text = if phone && !modal {
            // Touch targets: one short word per chip, bracketed like a button.
            let word = label
                .split([' ', '/', '·'])
                .find(|w| !w.is_empty())
                .unwrap_or(label);
            Line::from(vec![Span::styled(format!(" [{word}] "), key_style)])
                .alignment(ratatui::layout::Alignment::Center)
        } else {
            Line::from(vec![
                Span::styled(format!(" {key} "), key_style),
                Span::raw(label),
            ])
        };
        if let Some(cmd) = cmd.filter(|c| *c != keymap::Cmd::Motion) {
            // Phone chips take their whole cell (larger touch targets).
            let width = if phone {
                cell.width
            } else {
                (text.width() as u16 + 1).min(cell.width)
            };
            hitmap::register(Rect::new(cell.x, cell.y, width, 1), hitmap::Hit::Cmd(cmd));
        }
        frame.render_widget(Paragraph::new(text), cell);
    }
    if app.creating && app.dialog.is_none() && app.input.is_none() {
        let rect = popup(workspace, 44, 6);
        frame.render_widget(Clear, rect);
        let outer = block("New session".into(), true);
        let inner = outer.inner(rect);
        frame.render_widget(outer, rect);
        let body = Rect::new(
            inner.x,
            inner.y + inner.height.saturating_sub(2) / 2,
            inner.width,
            inner.height.min(2),
        );
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(Span::styled(
                    format!(
                        "{} Starting terminal",
                        transfer_spinner(app.transfer_frame, ascii())
                    ),
                    accent().add_modifier(Modifier::BOLD),
                )),
                Line::from("Preparing the first screen…"),
            ])
            .alignment(ratatui::layout::Alignment::Center),
            body,
        );
    }
    render_command_panel(frame, app, area);
    if !app.creating {
        notifications::render(
            frame,
            &app.notice,
            app.notice_kind,
            workspace,
            notifications::appearance(app.notice_started, app.notice_deadline, now),
        );
    }
    if let Some(input) = app
        .input
        .filter(|i| !matches!(*i, Input::Search | Input::PreviewSearch | Input::Filter))
    {
        let rect = popup(
            area,
            if input == Input::Add { 64 } else { 76 },
            if matches!(input, Input::Rename | Input::Add) {
                3
            } else {
                5
            },
        );
        if input == Input::Palette {
            render_palette(frame, app, area, phone);
        } else {
            frame.render_widget(Clear, rect);
            let editing = matches!(input, Input::Rename | Input::Add);
            let cursor_width = if editing {
                Span::raw(safe_label(if input == Input::Add {
                    &app.text
                } else {
                    &app.text[..app.rename_cursor]
                }))
                .width() as u16
                    + 2
            } else {
                0
            };
            let scroll = cursor_width.saturating_sub(rect.width.saturating_sub(3));
            frame.render_widget(
                Paragraph::new(if matches!(input, Input::Rename | Input::Add) {
                    format!("> {}", safe_label(&app.text))
                } else {
                    format!("> {}\nEnter confirm · Escape cancel", safe_label(&app.text))
                })
                .scroll((0, scroll))
                .block(block(
                    if input == Input::Add && app.network_add_target.is_some() {
                        let (d, address) = app.network_add_target.as_ref().unwrap();
                        format!(
                            "SSH account · {} via {}",
                            safe_label(address),
                            safe_label(&app.devices[*d].name)
                        )
                    } else if input == Input::Add && app.pending_add_via.is_some() {
                        format!(
                            "Add by SSH address via {} · user@host",
                            safe_label(&app.pending_add_via.as_ref().unwrap().name)
                        )
                    } else {
                        match input {
                            Input::Search | Input::PreviewSearch => "Search",
                            Input::Add => "Add by SSH address · alias or user@host",
                            Input::Rename => "Rename",
                            _ => "Directory name",
                        }
                        .into()
                    },
                    true,
                )),
                rect,
            );
            if editing {
                frame.set_cursor_position((rect.x + 1 + cursor_width - scroll, rect.y + 1));
            }
        }
    }
    if let Some(dialog) = &app.dialog {
        let (title, labels, detail) = match dialog {
            Dialog::Links(links) => (
                "Links".into(),
                links.iter().map(|l|safe_label(&l.label)).collect(),
                links.get(app.dialog_selected).map(|l|{
                    if crate::markdown_links::web_target(&l.target) {return format!("Viewer browser\n{}",safe_text(&l.target));}
                    app.browser.as_ref().and_then(|b|b.preview_path.as_ref().map(|p|(b,p))).and_then(|(b,p)|crate::files::decode_path(p).ok().and_then(|p|crate::markdown_links::resolve_file_link(&p,&l.target).ok().flatten()).map(|(path,_)|format!("Files · {}\n{}",identity(&app.devices[b.device]),safe_label(&path.to_string_lossy())))).unwrap_or_else(||safe_text(&l.target))
                }).unwrap_or_default(),
            ),
            Dialog::DevcontainerUp(d,path) => ("Start devcontainer workspace?".into(), vec!["Cancel".into(), "Run configuration · start workspace".into()], format!("{}\n{}\nRequires Node Dev Containers CLI on this device.\nThis may build images, start Compose services and run lifecycle hooks declared by the workspace. Review the configuration first; container network settings come from that configuration.", safe_label(&app.devices[*d].name), safe_label(path))),
            Dialog::ContainerActions(d,c) => (format!("{} · {}",safe_label(&c.name),safe_label(&app.devices[*d].name)),container_action_labels(c).iter().map(|s|(*s).into()).collect(),format!("{}\n{} · {}\nNetwork: {} · ports {}\nUser: {} · folder {}\nExisting networks are preserved; no automatic port forwarding.{}",safe_label(&c.evidence),safe_label(&c.state),safe_label(&c.image),safe_label(&c.network),c.ports.as_object().map(|p|p.len()).unwrap_or(0),safe_label(&c.user),safe_label(&c.folder),if !c.devcontainer&&!c.allowed{"\nOrdinary containers are inspection-only until explicitly enabled."}else{""})),
            Dialog::DestinationScope(d, containers) => (
                format!("Transfer to · {}", safe_label(&app.devices[*d].name)),
                std::iter::once("Host files".into()).chain(containers.iter().map(|c| format!("{} · {} · {}", if c.devcontainer { "devcontainer" } else { "container" }, safe_label(&c.name), &c.id[..12]))).collect(),
                format!("Choose the host or a running container, then browse to a folder and press Enter to transfer.{}", if app.containers_loading.contains(d) { "\nChecking containers…" } else if app.container_errors.contains_key(d) { "\nContainer discovery failed · Escape, refresh the device and retry. Host files remain available." } else { "" }),
            ),
            Dialog::ContainerProvider(d,scope)=>(format!("Session · {} / {}",safe_label(&app.devices[*d].name),safe_label(&scope.name)),app.container_session_choices(*d, scope).iter().map(|c| c.label.to_string()).collect(),app.container_profile_detail(*d, scope)),
            Dialog::ContainerPermissions(d,scope,provider,resume) => (
                format!("{} permissions · {} / {}", safe_label(provider), identity(&app.devices[*d]), safe_label(&scope.name)),
                vec!["Default · existing container permissions".into(), "YOLO · bypass approvals".into()],
                format!("User {} · {}\n{}\nYOLO bypasses provider approval prompts; Codex also disables its sandbox.\nSign-in and history belong to this container user.", safe_label(&scope.user), safe_label(&scope.folder), if *resume { "Choose a past conversation in the provider's native picker; no latest selection." } else { "Start a new conversation." }),
            ),
            Dialog::ContainerConfirm(d,c,action)=>(format!("{} {}?",action,safe_label(&c.name)),vec!["Cancel · keep current state".into(),format!("{} selected container",action)],format!("{}\n{}\n{}\n{}\n{}",safe_label(&app.devices[*d].name),safe_label(&c.name),safe_label(&c.id[..12]),if action=="Rebuild"{"The CLI replaces the container and its writable layer and ends sessions/transfers using it. Back up container-only files first. Workspace hooks may run and Compose services may restart. Mounted data follows the workspace configuration; no volume removal flag is used. Allow up to 10 minutes."}else if action=="Stop"{"Running work in this container will end."}else if action=="Enable access"{"Enable this container for terminals, files, transfers and lifecycle actions on this host account."}else{"Existing image, volumes and ports are preserved."},if action=="Rebuild"{"The selected workspace must uniquely match this container. Rebuild follows its declared configuration."}else{"Only this exact container ID is targeted. No rebuild/removal or network changes."})),
            Dialog::Device(purpose) => (
                match purpose {
                    ChooseDevice::Containers => "Containers · choose device or All devices",
                    ChooseDevice::Work => "Sessions · choose device or All devices",
                    ChooseDevice::Network => "Network · choose device or All devices",
                    ChooseDevice::AddGateway => "Add by SSH address · choose gateway",
                    ChooseDevice::SessionChooser => "New session · choose device",
                    ChooseDevice::Shell => "New session · choose device · starts in home",
                    ChooseDevice::New => "New session · execution device",
                    ChooseDevice::Terminal => "SSH terminal · choose device · exit returns",
                    ChooseDevice::Files => "Files · choose device",
                    ChooseDevice::Destination => "Transfer to · destination device",
                }
                .to_string(),
                app.device_choices(*purpose).iter().map(|d| match d {
                    Some(d) => format!("{} · {}", safe_label(&app.devices[*d].name), identity(&app.devices[*d])),
                    None if *purpose == ChooseDevice::AddGateway => "Direct SSH from this viewer".into(),
                    None => "All devices".into(),
                }).collect::<Vec<_>>(),
                if *purpose == ChooseDevice::Destination {
                    app.clipboard
                        .as_ref()
                        .map(|c| {
                            format!(
                                "Source: {} · {}\nChoose device, browse folder, then Enter to transfer · h/l navigate\nEnter choose · Escape cancel",
                                identity(&app.devices[c.device]),
                                format!("{} {} items", if c.cut { "cut" } else { "copy" }, c.entries.len())
                            )
                        })
                        .unwrap_or_else(|| "Enter choose · Escape cancel".into())
                } else {
                    "Enter choose · Escape cancel".to_string()
                },
            ),
            Dialog::AgentTasks(_, session) => {
                let selected = session.agent_tasks.get(app.dialog_selected);
                (
                    format!("Agent tasks · {}", safe_label(&session.name)),
                    session.agent_tasks.iter().map(|task| format!("{} · {}", safe_label(&task.name), safe_label(&task.status))).collect(),
                    selected.map(|task| format!("{}\n{}\n{}\n\nTask status comes from the provider. Open does not imply busy.",
                        safe_label(&task.name), safe_label(&task.status),
                        task.task.as_deref().map(safe_text).unwrap_or_else(|| "Task description unavailable".into())))
                    .unwrap_or_else(|| "No task details reported for this session.".into()),
                )
            },
            Dialog::SessionChooser(d,path) => (
                format!("New session · {}",safe_label(&app.devices[*d].name)),
                app.host_session_choices(*d).iter().map(|choice|choice.label.into()).collect(),
                format!("{}\n{}", safe_label(path), if app.provider_loading.contains(d) { "Checking installed agents…" } else { "Agent permissions are chosen next; devcontainer opens its tree." }),
            ),
            Dialog::Provider(d, path) => (
                format!("New session · {}", identity(&app.devices[*d])),
                app.provider_choices(*d)
                    .iter()
                    .map(|provider| match *provider {
                        "claude" => "Claude · existing host profile".into(),
                        "codex" => "Codex · existing host profile".into(),
                        "container" => "Devcontainer · choose a container workspace".into(),
                        _ => "Shell · ordinary terminal".into(),
                    })
                    .collect(),
                if app.provider_loading.contains(d) {
                    "Checking installed agents… · Shell is available now".into()
                } else if !app
                    .providers
                    .get(d)
                    .is_some_and(|(_, checked)| transport::now().saturating_sub(*checked) < 60)
                {
                    "Agent availability unknown · Escape, then Refresh to retry".into()
                } else if let Some(path) = path {
                    let location = app
                        .browser
                        .as_ref()
                        .filter(|b| b.device == *d && b.path == *path)
                        .map(|b| b.display_path.as_str())
                        .unwrap_or(path);
                    format!(
                        "Enter starts here: {} · Escape cancel",
                        safe_label(location)
                    )
                } else {
                    "Enter choose · next: browse folder, then n to start".into()
                },
            ),
            Dialog::Permissions(d, path, provider, resume, pick) => (
                format!("{} permissions · {}", provider, identity(&app.devices[*d])),
                vec!["Default · existing host permissions".into(), "YOLO · bypass approvals".into()],
                format!("{}\n{}Applies to this session only.\nYOLO bypasses approval prompts; Codex also disables its sandbox.\nEnter starts · Escape cancels", safe_label(path), if *pick { "Native resume (choose): " } else if *resume { "Native resume (latest): " } else { "Fresh session: " }),
            ),
            Dialog::StopShell(d, session) => (
                format!("Stop {} session?", session.provider), vec!["Keep session".into(), "Stop session".into()],
                format!("Running work in this session will end.\n{}\n{}\n{}", identity(&app.devices[*d]), safe_label(&session.name), session_folder_label(&session.directory)),
            ),
            Dialog::Delete(d, entries) => (
                format!("Delete {} {}?", entries.len(), if entries.len() == 1 { "item" } else { "items" }),
                vec!["Cancel · keep files".into(), "Delete permanently".into()],
                format!("{}\n{}\n{}",
                    if entries.iter().any(|e| e.kind == "directory") { "No undo · folders include their contents." } else { "Deletion cannot be undone." }, identity(&app.devices[*d]),
                    entries.iter().map(|e| format!("{} {}", if ascii() { "-" } else { "•" }, safe_label(&e.name))).collect::<Vec<_>>().join("\n")),
            ),
            Dialog::PendingExit(count) => (
                "File actions are still being submitted".into(),
                vec!["Stay · finish submitting".into(), "Quit · discard unsubmitted actions".into()],
                format!("{count} actions have not been submitted.\nAlready detached transfers continue.\nEnter chooses · Escape stays"),
            ),
            Dialog::SessionPicker(d, path, scope, provider, rows) => (
                format!("Sessions here · {}", identity(&app.devices[*d])),
                rows.iter().map(|s| format!("{} · {} · {} · {}", safe_label(&s.provider), session_preview::label(s), app.session_status(*d, s), safe_label(&s.id.chars().rev().take(6).collect::<String>().chars().rev().collect::<String>()))).chain(std::iter::once(if provider.is_empty() && scope.is_some() { "+ New session…".into() } else { format!("+ New {} session", if provider.is_empty() { "shell" } else { provider }) })).chain(container_agents::has_history(scope.as_ref(), provider).then(|| format!("Choose past conversation in {}…", safe_label(provider)))).collect(),
                format!("{}{}\nEnter joins the selected runtime · New creates separate work\nNo conversation is selected by recency.", scope.as_ref().map(|c| format!("{} · ", safe_label(&c.name))).unwrap_or_default(), safe_label(if path.is_empty() { scope.as_ref().map(|c| c.folder.as_str()).unwrap_or("~") } else { path })),
            ),
            Dialog::PinnedEnded(d, s) => (
                format!("{} · {}", if app.pinned(*d, s) { "Pinned work" } else { "Unavailable terminal" }, safe_label(&s.name)),
                vec!["Open saved folder".into(), "Refresh sessions".into()],
                format!("{} · {}\n{}\nOpening the saved folder does not restart this terminal.", identity(&app.devices[*d]), app.session_status(*d, s), safe_label(&s.directory)),
            ),
            Dialog::Peer(d) => (
                format!("Device · {}", safe_label(&app.devices[*d].name)),
                vec!["Open sessions".into(), "Browse files".into(), "New persistent shell · tmux".into(), "New session".into(), "SSH terminal · exit returns".into()],
                format!("{}\n{}\nFrom viewer · helper evidence\nVia selected device: unknown", identity(&app.devices[*d]), app.peer_status(*d)),
            ),
            Dialog::RemoveDevice(d) => (
                format!("Remove {}?", safe_label(&app.devices[*d].name)),
                vec!["Keep device".into(), "Remove from CX".into()],
                format!("This removes the device from this viewer's registry and clears its local policy and saved route.\n{}\nRemote files and sessions are not changed.", identity(&app.devices[*d])),
            ),
            Dialog::EditDevice(d) => (
                format!("Edit {}", safe_label(&app.devices[*d].name)),
                vec!["SSH address".into(), match app.device_policy(*d).access {
                    store::AccessMode::Core => "Access: core (reverse unverified) · switch to single-direction".into(),
                    store::AccessMode::Directed => "Access: single-direction · switch to core".into(),
                }, "Remove device".into(), "Cancel".into()],
                format!("Current target: {}\nEdit the SSH alias or user@host. The device identity and posture stay unchanged.", safe_label(app.devices[*d].target.as_deref().unwrap_or("unknown"))),
            ),
            Dialog::Reset => (
                "Reset managed sessions?".into(),
                vec!["Cancel · keep running sessions".into(), "Reset tmux and agents".into()],
                "This forcefully terminates all CX-managed tmux sessions on this execution device, including Claude and Codex processes running inside them. External tmux sessions are untouched.\nEnter confirms · Escape cancels".into(),
            ),
            Dialog::Neighbor(d, candidate) => (
                format!("Neighbor · {}", safe_label(candidate["address"].as_str().unwrap_or("unknown"))),
                vec![if neighbor_connectable(candidate) { "Connect via this device".into() } else { "Connect unavailable · link-local scope".into() }],
                format!("Via {}\n{}\nEnter chooses · Escape cancels", identity(&app.devices[*d]), neighbor_detail(candidate)),
            ),
            Dialog::Jobs => {
                let jobs = app.job_rows();
                let detail = jobs
                    .get(app.dialog_selected)
                    .map(|(_, j)| {
                        format!(
                            "{}\nRoute: {}\nSource: {} · {}\nDestination: {} · {}",
                            j["error"].as_str().map(safe_label).unwrap_or_else(|| format!("{} · {}", transfer_status(j), if j["operation"] == "move" { "move" } else { "copy" })),
                            safe_label(j["route"].as_str().unwrap_or("route unknown")),
                            safe_label(j["source_host"].as_str().unwrap_or("unknown host")),
                            safe_label(j["source_display"].as_str().or(j["source_path"].as_str()).unwrap_or("")),
                            safe_label(j["destination_host"].as_str().unwrap_or("unknown host")),
                            transfer_destination(j),
                        )
                    })
                    .unwrap_or_else(|| "No transfers yet · c copy / x cut → folder → p paste".into());
                (
                    "Transfers".into(),
                    jobs.iter()
                        .map(|(_, j)| {
                            format!(
                                "{} {} · {} → {} · {} / {}",
                                transfer_status(j),
                                transfer_name(j),
                                safe_label(j["source_host"].as_str().unwrap_or("?")),
                                safe_label(j["destination_host"].as_str().unwrap_or("?")),
                                human_size(j["bytes"].as_u64().unwrap_or(0)),
                                human_size(j["total"].as_u64().unwrap_or(0))
                            )
                        })
                        .collect(),
                    detail,
                )
            }
        };
        if matches!(
            dialog,
            Dialog::SessionChooser(..) | Dialog::ContainerProvider(..)
        ) {
            render_session_chooser(frame, app, area, dialog, &title, &detail);
        } else if matches!(
            dialog,
            Dialog::Delete(..) | Dialog::StopShell(..) | Dialog::RemoveDevice(..) | Dialog::Reset
        ) {
            let stop = matches!(dialog, Dialog::StopShell(..));
            let labels = if stop {
                ["Keep session", "Stop session"]
            } else if matches!(dialog, Dialog::RemoveDevice(..)) {
                ["Keep device", "Remove device"]
            } else if matches!(dialog, Dialog::Reset) {
                ["Cancel", "Reset"]
            } else {
                ["Cancel", "Delete"]
            };
            let rect = confirmation_popup(area, &detail);
            frame.render_widget(Clear, rect);
            frame.render_widget(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(if ascii() {
                        ratatui::widgets::BorderType::Plain
                    } else {
                        ratatui::widgets::BorderType::Rounded
                    })
                    .border_style(tint(Color::Red))
                    .title(Line::from(Span::styled(
                        format!(" {title} "),
                        tint(Color::Red).add_modifier(Modifier::BOLD),
                    ))),
                rect,
            );
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
            let max_scroll = preview_max_scroll(
                &preview_lines(&detail, "text"),
                rows[0].width,
                rows[0].height,
            );
            app.dialog_scroll_max.set(max_scroll);
            frame.render_widget(
                Paragraph::new(detail)
                    .wrap(Wrap { trim: false })
                    .scroll((app.dialog_scroll.min(max_scroll), 0)),
                rows[0],
            );
            confirmation_buttons(
                frame,
                rows[2],
                stop,
                app.dialog_selected,
                app.dialog_detail_focus,
                ConfirmationLayout::Separate,
                labels,
            );
        } else {
            let mut rect = popup(
                area,
                90,
                if matches!(dialog, Dialog::Jobs) {
                    area.height.saturating_sub(6)
                } else {
                    15
                },
            );
            if matches!(dialog, Dialog::Jobs) {
                rect.y = area.y + area.height.saturating_sub(rect.height + 5);
            }
            frame.render_widget(Clear, rect);
            let parts = Layout::default()
                .direction(Direction::Vertical)
                .constraints(if matches!(dialog, Dialog::Jobs) {
                    vec![
                        Constraint::Length((rect.height / 3).max(4)),
                        Constraint::Min(5),
                    ]
                } else {
                    vec![
                        Constraint::Length((labels.len() as u16 + 2).min(6)),
                        Constraint::Min(4),
                    ]
                })
                .split(rect);
            let selection_style = if app.dialog_detail_focus {
                accent()
            } else {
                selected_style()
            };
            if matches!(dialog, Dialog::Jobs) {
                let progress_width = if parts[0].width < 60 { 10 } else { 18 };
                let file_width = parts[0].width.saturating_sub(progress_width + 15) as usize;
                let rows = app
                    .job_rows()
                    .into_iter()
                    .map(|(_, job)| {
                        let status_color = match job["status"].as_str() {
                            Some("failed" | "incomplete") => Color::Red,
                            Some("complete") => Color::Green,
                            _ => Color::Yellow,
                        };
                        let operation = if job["operation"] == "move" {
                            "move"
                        } else {
                            "copy"
                        };
                        Row::new(vec![
                            Cell::from(Span::styled(
                                safe_label(job["status"].as_str().unwrap_or("unknown")),
                                tint(status_color),
                            )),
                            Cell::from(Line::from(vec![
                                Span::styled(
                                    format!("{operation} "),
                                    tint(if operation == "move" {
                                        Color::Magenta
                                    } else {
                                        Color::Cyan
                                    }),
                                ),
                                Span::raw(compact_path(
                                    &transfer_name(&job),
                                    file_width.saturating_sub(5),
                                )),
                            ])),
                            Cell::from(format!(
                                "{}/{}",
                                human_size(job["bytes"].as_u64().unwrap_or(0)),
                                human_size(job["total"].as_u64().unwrap_or(0))
                            )),
                        ])
                    })
                    .collect::<Vec<_>>();
                let mut state = TableState::default().with_selected(Some(app.dialog_selected));
                frame.render_stateful_widget(
                    Table::new(
                        rows,
                        [
                            Constraint::Length(10),
                            Constraint::Min(8),
                            Constraint::Length(progress_width),
                        ],
                    )
                    .header(Row::new(["Status", "File", "Progress"]).style(muted()))
                    .column_spacing(1)
                    .block(block(title, true))
                    .row_highlight_style(selection_style),
                    parts[0],
                    &mut state,
                );
            } else {
                let mut state =
                    ratatui::widgets::ListState::default().with_selected(Some(app.dialog_selected));
                frame.render_stateful_widget(
                    List::new(labels.into_iter().map(ListItem::new).collect::<Vec<_>>())
                        .block(block(title, true))
                        .highlight_style(selection_style),
                    parts[0],
                    &mut state,
                );
            }
            let max_scroll = preview_max_scroll(
                &preview_lines(&detail, "text"),
                parts[1].width.saturating_sub(2),
                parts[1].height.saturating_sub(2),
            );
            app.dialog_scroll_max.set(max_scroll);
            frame.render_widget(
                Paragraph::new(detail)
                    .wrap(Wrap { trim: false })
                    .scroll((app.dialog_scroll.min(max_scroll), 0))
                    .block(block("Details · PgUp/PgDn".into(), app.dialog_detail_focus)),
                parts[1],
            );
        }
    }
    if app.help {
        hitmap::modal();
        // One box over a dimmed backdrop (P12). Below 100 columns a centered
        // box would leave 1-2 column slivers of the panels at the edges, so
        // the overlay takes the full frame there.
        dim_backdrop(frame);
        let rect = if phone || area.width < 100 {
            Rect::new(area.x, area.y, area.width, area.height)
        } else {
            popup(area, 76, area.height.saturating_sub(4).clamp(20, 40))
        };
        frame.render_widget(Clear, rect);
        let key_style = accent().add_modifier(Modifier::BOLD);
        let key_width = if rect.width < 50 { 12 } else { 18 };
        let key_row = |keys: &str, description: &str| {
            Line::from(vec![
                Span::styled(format!("{keys:<key_width$} "), key_style),
                Span::raw(description.to_owned()),
            ])
        };
        // Generated from the keymap: the same table dispatches the keys.
        let mut help = Vec::new();
        for (index, (title, rows)) in keymap::help_sections(app).into_iter().enumerate() {
            if index > 0 {
                help.push(Line::default());
            }
            help.push(Line::from(Span::styled(
                title,
                Style::default().add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
            )));
            for (keys, description) in rows {
                help.push(key_row(keys, description));
            }
        }
        if matches!(app.dialog, Some(Dialog::Jobs)) {
            help.push(Line::default());
            help.push(Line::from(Span::styled("Transfer jobs", key_style)));
            help.extend([
                key_row("c · r", "Cancel / retry saved job"),
                key_row("Enter / Tab", "Focus details"),
                key_row("PgUp/PgDn", "Scroll details"),
            ]);
        }
        help.push(Line::default());
        help.push(Line::from(Span::styled(
            "Inside sessions",
            Style::default().add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        )));
        help.push(Line::raw("Native terminals own their input."));
        help.push(key_row(DETACH_KEY, "Return from a managed session"));
        help.push(Line::raw("External sessions keep their tmux bindings."));
        help.push(Line::default());
        help.push(Line::from(Span::styled(
            "Mouse and touch",
            Style::default().add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        )));
        help.push(key_row("tap", "Select a row; tap it again to open"));
        help.push(key_row(
            "hold / right",
            "Actions for the row (same as its keys)",
        ));
        help.push(key_row("tab · chip", "Go to the view / run the chip's key"));
        help.push(key_row("wheel", "Move one row per detent"));
        frame.render_widget(
            Paragraph::new(help)
                .wrap(Wrap { trim: false })
                .scroll((app.help_scroll, 0))
                .block(block("Help · Esc close".into(), true)),
            rect,
        );
    }
    hitmap::render_context_menu(frame, app, area);
}
/// Key that returns from a managed session. The sessions wave moves this to
/// Ctrl+\ together with the tmux binding; help reads it from here.
pub(super) const DETACH_KEY: &str = "Ctrl+]";
/// Width class XS (phones): single column, device chip strip, bottom tab bar.
pub(super) fn phone(width: u16) -> bool {
    width < 60
}
/// The frame's fixed regions, shared by rendering and the native-preview
/// placement so they cannot disagree.
pub(super) struct Areas {
    pub phone: bool,
    pub show_search: bool,
    pub header: Rect,
    pub rail: Rect,
    /// Device chip strip (phone only).
    pub strip: Rect,
    pub main: Rect,
    pub footer: Rect,
    pub sidebar_width: u16,
}
pub(super) fn frame_areas(app: &App, area: Rect) -> Areas {
    let query = filters::visible_query(app);
    let show_search = matches!(
        app.input,
        Some(Input::Search | Input::PreviewSearch | Input::Filter)
    ) || !query.is_empty();
    let phone = phone(area.width);
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(if phone { 1 } else { 2 }),
            Constraint::Min(5),
            Constraint::Length(if show_search { 7 } else { 4 }),
        ])
        .split(area);
    let sidebar_width: u16 = if phone {
        0
    } else if area.width < 70 {
        17
    } else {
        21
    };
    let content = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(sidebar_width), Constraint::Min(15)])
        .split(vertical[1]);
    let (strip, main) = if phone {
        let parts = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(4)])
            .split(content[1]);
        (parts[0], parts[1])
    } else {
        (Rect::default(), content[1])
    };
    Areas {
        phone,
        show_search,
        header: vertical[0],
        rail: content[0],
        strip,
        main,
        footer: vertical[2],
        sidebar_width,
    }
}
fn tab_label(view: View, phone: bool) -> &'static str {
    keymap::TABS
        .iter()
        .find(|(v, _, _)| *v == view)
        .map(|(_, long, short)| if phone { *short } else { *long })
        .unwrap_or("Sessions")
}
/// Header: product mark, clickable view tabs and the palette button.
fn render_header(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let row = Rect::new(area.x, area.y, area.width, 1);
    let active = keymap::tab_index(app.view);
    let mark = Span::styled(" cx ", accent().add_modifier(Modifier::BOLD));
    // U+2026 is in every common terminal font (U+22EF often is not).
    let more = if ascii() { " ... " } else { " … " };
    let more_width = more.chars().count() as u16;
    let palette = Rect::new(
        row.right().saturating_sub(more_width),
        row.y,
        more_width.min(row.width),
        1,
    );
    frame.render_widget(
        Paragraph::new(Span::styled(more, accent().add_modifier(Modifier::BOLD))),
        palette,
    );
    hitmap::register(palette, hitmap::Hit::Cmd(keymap::Cmd::Palette));
    if phone(area.width) {
        // Tabs move to the bottom bar; the header names the current view.
        let name = if app.view == View::Containers {
            "Containers"
        } else {
            keymap::TABS[active].1
        };
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                mark,
                Span::styled(if ascii() { "- " } else { "· " }, muted()),
                Span::styled(name, Style::default().add_modifier(Modifier::BOLD)),
            ])),
            Rect::new(row.x, row.y, row.width.saturating_sub(more_width), 1),
        );
        return;
    }
    let mut spans = vec![mark, Span::raw(" ")];
    let mut x = row.x + 5;
    for (index, (view, label, _)) in keymap::TABS.iter().enumerate() {
        let selected = index == active;
        let text = format!(
            "{}{label} ",
            if selected {
                if ascii() {
                    ">"
                } else {
                    "▸"
                }
            } else {
                " "
            }
        );
        let width = text.chars().count() as u16;
        if x + width > palette.x {
            break;
        }
        hitmap::register(Rect::new(x, row.y, width, 1), hitmap::Hit::Tab(*view));
        spans.push(Span::styled(
            text,
            if selected {
                accent().add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            },
        ));
        spans.push(Span::raw(" "));
        x += width + 1;
    }
    if app.view == View::Containers && x + 13 <= palette.x {
        spans.push(Span::styled(
            format!("{}Containers", if ascii() { ">" } else { "▸" }),
            accent().add_modifier(Modifier::BOLD),
        ));
        x += 11;
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), row);
    let viewer = app
        .devices
        .iter()
        .find(|d| d.target.is_none())
        .map(identity)
        .unwrap_or_else(|| "local viewer".into());
    let text = format!("viewing {viewer} ");
    let width = text.chars().count() as u16;
    if x + width + 2 <= palette.x {
        frame.render_widget(
            Paragraph::new(text).style(muted()),
            Rect::new(palette.x - width, row.y, width, 1),
        );
    }
}
/// Phone bottom tab bar: a separator row and four equal, tall tab targets.
fn render_tab_bar(frame: &mut Frame<'_>, app: &App, area: Rect) {
    if area.height == 0 {
        return;
    }
    let rule = if ascii() { "=" } else { "━" };
    frame.render_widget(
        Paragraph::new(rule.repeat(area.width as usize)).style(muted()),
        Rect::new(area.x, area.y, area.width, 1),
    );
    let labels = Rect::new(area.x, area.bottom().saturating_sub(1), area.width, 1);
    let cells = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(vec![Constraint::Ratio(1, 4); 4])
        .split(labels);
    let active = keymap::tab_index(app.view);
    for (index, (view, _, _)) in keymap::TABS.iter().enumerate() {
        let selected = index == active;
        let label = format!(
            "{}{}",
            if selected {
                if ascii() {
                    ">"
                } else {
                    "▸"
                }
            } else {
                ""
            },
            tab_label(*view, true)
        );
        frame.render_widget(
            Paragraph::new(label)
                .alignment(ratatui::layout::Alignment::Center)
                .style(if selected {
                    accent().add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                }),
            cells[index],
        );
        hitmap::register(
            Rect::new(cells[index].x, area.y, cells[index].width, area.height),
            hitmap::Hit::Tab(*view),
        );
    }
}
fn device_glyph(app: &App, d: usize) -> (&'static str, Style) {
    let w = &app.work[d];
    let (unicode, plain, color) = if w.loading {
        ("◌", ".", Color::Cyan)
    } else if w.error.is_some() {
        ("✕", "x", Color::Yellow)
    } else if w.fetched > 0 && transport::now().saturating_sub(w.fetched) <= 60 {
        ("●", "+", Color::Green)
    } else {
        ("·", "-", Color::Reset)
    };
    (if ascii() { plain } else { unicode }, tint(color))
}
/// Phone device rail: one row of tappable device chips.
fn render_device_strip(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let rows = app.device_rows();
    let focused = app.focus == Focus::Devices;
    let chips: Vec<(usize, Vec<Span<'static>>, u16)> = rows
        .iter()
        .map(|value| {
            let mut spans = Vec::new();
            if *value == 0 {
                spans.push(Span::raw("All".to_string()));
            } else {
                let (glyph, style) = device_glyph(app, value - 1);
                spans.push(Span::styled(glyph.to_string(), style));
                if app.device_policy(value - 1).access == store::AccessMode::Directed {
                    spans.push(Span::raw("* ".to_string()));
                }
                spans.push(Span::raw(fit_label(&app.devices[value - 1].name, 14)));
            }
            let width = spans.iter().map(|s| s.width() as u16).sum::<u16>() + 2;
            (*value, spans, width)
        })
        .collect();
    let selected = rows.iter().position(|v| *v == app.device).unwrap_or(0);
    // Hidden chips are never silent: `‹N` / `N›` markers count them and are
    // tap targets that step to the nearest hidden device.
    let marker = |n: usize, left: bool| -> String {
        match (left, ascii()) {
            (true, true) => format!("<{n} "),
            (true, false) => format!("‹{n} "),
            (false, true) => format!(" {n}>"),
            (false, false) => format!(" {n}›"),
        }
    };
    let marker_width = |n: usize| marker(n, true).chars().count() as u16;
    // Last chip shown when the strip starts at `first`, leaving room for the
    // markers it needs.
    let fit = |first: usize| -> usize {
        let left = if first > 0 { marker_width(first) } else { 0 };
        let mut x = left;
        let mut last = first;
        for (index, (_, _, width)) in chips.iter().enumerate().skip(first) {
            let remaining = chips.len() - index - 1;
            let reserve = if remaining > 0 {
                marker_width(remaining)
            } else {
                0
            };
            if index > first && x + width + reserve > area.width {
                break;
            }
            last = index;
            x += width + 1;
        }
        last
    };
    // Scroll the strip so the selected chip is always visible.
    let mut first = 0;
    while first < selected && fit(first) < selected {
        first += 1;
    }
    let last = fit(first);
    let mut x = area.x;
    let mut line = Vec::new();
    if first > 0 {
        let text = marker(first, true);
        let width = text.chars().count() as u16;
        line.push(Span::styled(text, accent()));
        hitmap::register(
            Rect::new(x, area.y, width, 1),
            hitmap::Hit::Row(hitmap::ListId::Devices, chips[first - 1].0),
        );
        x += width;
    }
    let hidden_after = chips.len().saturating_sub(last + 1);
    let next_hidden = chips.get(last + 1).map(|(value, _, _)| *value);
    for (index, (value, spans, width)) in chips.into_iter().enumerate().skip(first) {
        if index > last {
            break;
        }
        let width = width.min(area.right().saturating_sub(x));
        let chosen = index == selected;
        let style = if chosen && focused {
            selected_style().add_modifier(Modifier::REVERSED)
        } else if chosen {
            accent().add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        line.push(Span::styled("[", style));
        for span in spans {
            let merged = span.style.patch(style);
            line.push(Span::styled(span.content, merged));
        }
        line.push(Span::styled("]", style));
        line.push(Span::raw(" "));
        hitmap::register(
            Rect::new(x, area.y, width, 1),
            hitmap::Hit::Row(hitmap::ListId::Devices, value),
        );
        x += width + 1;
    }
    if let Some(next) = next_hidden {
        let text = marker(hidden_after, false);
        let width = text.chars().count() as u16;
        let at = area.right().saturating_sub(width);
        let used: u16 = line.iter().map(|s| s.width() as u16).sum();
        let gap = at.saturating_sub(area.x + used);
        line.push(Span::raw(" ".repeat(gap as usize)));
        line.push(Span::styled(text, accent()));
        hitmap::register(
            Rect::new(at, area.y, width, 1),
            hitmap::Hit::Row(hitmap::ListId::Devices, next),
        );
    }
    frame.render_widget(Paragraph::new(Line::from(line)), area);
}
/// Keys / Secrets view placeholder (the inventory lands in a later wave).
fn render_secrets(frame: &mut Frame<'_>, app: &App, area: Rect) {
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(""),
            Line::from(Span::styled(
                "Keys and secrets",
                Style::default().add_modifier(Modifier::BOLD),
            )),
            Line::from("Credentials cx uses will be listed here, always masked."),
            Line::from(Span::styled(
                "Nothing is read yet · provider credentials are never read.",
                muted(),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "g s Sessions · g f Files · g n Network",
                muted(),
            )),
        ])
        .wrap(Wrap { trim: false })
        .block(block("Keys".into(), app.focus == Focus::Workspace).padding(Padding::horizontal(1))),
        area,
    );
}
/// The command palette: one box, dimmed background, sections, MRU ranking.
/// Dim everything already drawn so an overlay reads as the only live layer
/// (P12): DIM plus, with color, the muted fg so terminals that ignore SGR 2
/// still recede.
pub(super) fn dim_backdrop(frame: &mut Frame<'_>) {
    let colored = !no_color();
    for cell in frame.buffer_mut().content.iter_mut() {
        cell.modifier.insert(Modifier::DIM);
        cell.modifier.remove(Modifier::BOLD | Modifier::REVERSED);
        if colored {
            cell.fg = Color::DarkGray;
            cell.bg = Color::Reset;
        }
    }
}
/// Fit a `head · path` detail: the path keeps its tail (spec 7), the head
/// gets what is left.
fn fit_detail(detail: &str, width: usize) -> String {
    let detail = safe_label(detail);
    let tail = |text: &str, width: usize| -> String {
        let count = text.chars().count();
        if count <= width {
            return text.to_owned();
        }
        let keep: String = text.chars().skip(count + 1 - width.max(1)).collect();
        format!("{}{keep}", if ascii() { "~" } else { "…" })
    };
    if detail.chars().count() <= width {
        return detail;
    }
    match detail.rsplit_once(" · ") {
        Some((head, path)) if path.chars().count() + 3 + 4 <= width => {
            let head_width = width - path.chars().count() - 3;
            format!("{} · {path}", fit_label(head, head_width))
        }
        // Long path: the head (device) keeps up to 10 columns, the path
        // keeps its tail in the rest.
        Some((head, path)) if width >= 20 => {
            let head = fit_label(head, head.chars().count().min(10));
            let rest = width - head.chars().count() - 3;
            format!("{head} · {}", tail(path, rest))
        }
        Some((_, path)) => tail(path, width),
        None => tail(&detail, width),
    }
}
/// Tail-preserving fit for paths (spec 7: `…/cx-demo`, never `/home/p…`).
fn fit_path(path: &str, width: usize) -> String {
    let path = safe_label(path);
    let count = path.chars().count();
    if count <= width {
        return path;
    }
    let keep: String = path.chars().skip(count + 1 - width.max(1)).collect();
    format!("{}{keep}", if ascii() { "~" } else { "…" })
}
fn render_palette(frame: &mut Frame<'_>, app: &App, area: Rect, phone: bool) {
    // Dim everything beneath; nothing bleeds through the box edges (P12).
    dim_backdrop(frame);
    hitmap::modal();
    let items = app.palette_view();
    let width = if phone {
        area.width
    } else {
        76.min(area.width.saturating_sub(2))
    };
    // Rows: one per item plus one header per section.
    let sections = items
        .iter()
        .enumerate()
        .filter(|(i, item)| *i == 0 || items[*i - 1].section != item.section)
        .count();
    let content = (items.len() + sections).max(1) as u16;
    let max_height = if phone {
        area.height.saturating_sub(2).min(16)
    } else {
        area.height.saturating_sub(4).min(19)
    };
    let height = (content + 2).min(max_height);
    let rect = if phone {
        Rect::new(area.x, area.bottom() - height, width, height)
    } else {
        // Anchor the top edge where a full box would start, so the box
        // grows downwards as results appear instead of jumping around.
        let full = popup(area, width, max_height);
        Rect::new(full.x, full.y, full.width, height)
    };
    frame.render_widget(Clear, rect);
    let cursor = if ascii() { "_" } else { "█" };
    let search = ">";
    let footer = if app.palette_run_mode() {
        format!(" {} ", app.palette_scope())
    } else {
        format!(" {} · Tab section ", app.palette_scope())
    };
    let at = app.rename_cursor.min(app.text.len());
    let at = if app.text.is_char_boundary(at) {
        at
    } else {
        app.text.len()
    };
    let before = safe_text(&app.text[..at]);
    let after = safe_text(&app.text[at..]);
    // Keep the cursor in view for long commands: show the tail before it.
    let room = (rect.width as usize).saturating_sub(8);
    let before_chars = before.chars().count();
    let before = if before_chars > room {
        before.chars().skip(before_chars - room).collect::<String>()
    } else {
        before
    };
    let after = fit_label(&after, room.saturating_sub(before.chars().count()));
    let outer = Block::default()
        .borders(Borders::ALL)
        .border_type(if ascii() {
            BorderType::Plain
        } else {
            BorderType::Rounded
        })
        .border_style(accent())
        .title(Line::from(vec![
            Span::styled(format!(" {search} "), accent().add_modifier(Modifier::BOLD)),
            Span::raw(before),
            Span::styled(cursor.to_string(), accent()),
            Span::raw(format!("{after} ")),
        ]))
        .title_bottom(Line::styled(
            fit_label(&footer, rect.width.saturating_sub(2) as usize),
            muted(),
        ));
    let inner = outer.inner(rect);
    frame.render_widget(outer, rect);
    if items.is_empty() {
        frame.render_widget(
            Paragraph::new("No matches · Backspace to widen · Esc close").style(muted()),
            inner,
        );
        return;
    }
    // Interleave section headers; remember which line shows which item.
    let mut lines: Vec<(Option<usize>, Line<'static>)> = Vec::new();
    let key_width = 6usize;
    let width = inner.width as usize;
    let room = width.saturating_sub(2 + key_width + 1);
    // One fixed label column for every row with a detail, so details align
    // across sections (spec 7: columns are fixed).
    let label_column = items
        .iter()
        .filter(|i| !i.detail.is_empty())
        .map(|i| Span::raw(safe_label(&i.label)).width())
        .max()
        .unwrap_or(0)
        .clamp(room.min(12), (room * 3 / 5).max(room.min(12)));
    for (index, item) in items.iter().enumerate() {
        if index == 0 || items[index - 1].section != item.section {
            lines.push((
                None,
                Line::styled(
                    format!(" {}", item.section),
                    muted().add_modifier(Modifier::BOLD),
                ),
            ));
        }
        let selected = index == app.palette_selected;
        let bar = if selected {
            if ascii() {
                "> "
            } else {
                "▌ "
            }
        } else {
            "  "
        };
        let label_width = if item.detail.is_empty() {
            room
        } else {
            label_column
        };
        let label = if item.section == "Folders" {
            fit_path(&item.label, label_width)
        } else {
            fit_label(&safe_label(&item.label), label_width)
        };
        let label = if item.detail.is_empty() {
            label
        } else {
            format!("{label:<label_column$}")
        };
        let detail_width = room.saturating_sub(label.chars().count() + 2);
        let detail = if item.detail.is_empty() || detail_width < 4 {
            String::new()
        } else {
            format!("  {}", fit_detail(&item.detail, detail_width))
        };
        let used = 2 + label.chars().count() + detail.chars().count();
        let pad = width.saturating_sub(used + key_width.min(width));
        lines.push((
            Some(index),
            Line::from(vec![
                Span::styled(bar, accent()),
                Span::styled(
                    label,
                    if selected {
                        selected_style()
                    } else {
                        Style::default()
                    },
                ),
                Span::styled(detail, muted()),
                Span::raw(" ".repeat(pad)),
                Span::styled(
                    format!("{:>key_width$}", fit_label(&item.key, key_width)),
                    muted(),
                ),
            ]),
        ));
    }
    let selected_line = lines
        .iter()
        .position(|(i, _)| *i == Some(app.palette_selected))
        .unwrap_or(0);
    let height = inner.height as usize;
    // Keep the section header visible above the first row of a section.
    let offset = navigation::viewport(
        "palette",
        selected_line.saturating_sub(0),
        lines.len(),
        height,
    );
    let visible: Vec<_> = lines.into_iter().skip(offset).take(height).collect();
    for (row, (index, _)) in visible.iter().enumerate() {
        if let Some(index) = index {
            hitmap::register(
                Rect::new(inner.x, inner.y + row as u16, inner.width, 1),
                hitmap::Hit::PaletteRow(*index),
            );
        }
    }
    frame.render_widget(
        Paragraph::new(visible.into_iter().map(|(_, l)| l).collect::<Vec<_>>()),
        inner,
    );
}
fn render_containers(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let rows = app.container_rows();
    let items = rows
        .iter()
        .map(|(d, c)| match c {
            None => {
                let icon = if app.container_collapsed.contains(d) {
                    ">"
                } else {
                    "v"
                };
                let state = if app.containers_loading.contains(d) {
                    " · scanning"
                } else if app.container_errors.contains_key(d) {
                    " · unavailable"
                } else {
                    ""
                };
                ListItem::new(Line::from(vec![
                    Span::styled(
                        format!("{icon} {}", safe_label(&app.devices[*d].name)),
                        accent().add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(state, muted()),
                ]))
            }
            Some(c) => {
                let state = if c.state == "running" {
                    Color::Green
                } else {
                    Color::Yellow
                };
                ListItem::new(Line::from(vec![
                    Span::raw(if ascii() { "  +- " } else { "  ├─ " }),
                    Span::raw(safe_label(&c.name)),
                    Span::styled(
                        format!(
                            " · {}",
                            if c.devcontainer {
                                "Devcontainer"
                            } else {
                                "Docker"
                            }
                        ),
                        muted(),
                    ),
                    Span::styled(
                        format!(
                            " · {}",
                            if app.rebuilding.contains(&(*d, c.id.clone())) {
                                "rebuilding…".into()
                            } else {
                                safe_label(&c.state)
                            }
                        ),
                        tint(state),
                    ),
                    Span::styled(
                        if !c.devcontainer && !c.allowed {
                            " · inspect only"
                        } else {
                            ""
                        },
                        muted(),
                    ),
                ]))
            }
        })
        .collect::<Vec<_>>();
    let mut state =
        ratatui::widgets::ListState::default().with_selected(Some(app.container_selected));
    frame.render_stateful_widget(
        List::new(items)
            .block(block(
                "Containers · Enter actions · n session".into(),
                app.focus == Focus::Workspace,
            ))
            .highlight_style(selected_style()),
        area,
        &mut state,
    );
    if rows.iter().all(|(_, c)| c.is_none()) && area.height > 5 {
        let message = if !app.containers_loading.is_empty() {
            "Discovering Docker containers…".into()
        } else if let Some((d, error)) = app
            .container_errors
            .iter()
            .find(|(d, _)| app.device == 0 || app.device == **d + 1)
        {
            format!(
                "{}: {}",
                safe_label(&app.devices[*d].name),
                safe_label(error)
            )
        } else {
            "No containers found. Existing server containers are never opened automatically.".into()
        };
        frame.render_widget(
            Paragraph::new(message)
                .style(muted())
                .wrap(Wrap { trim: false }),
            Rect::new(
                area.x + 2,
                area.y + 3,
                area.width.saturating_sub(4),
                area.height.saturating_sub(4),
            ),
        );
    }
}

/// Where an outside-CX terminal currently lives, from its master holder.
fn origin_label(kind: &str) -> &'static str {
    match kind {
        "terminal" => "terminal",
        "sshd" => "ssh",
        "tmux" => "tmux",
        "screen" => "screen",
        _ => "external",
    }
}

/// Floating live view of a command running in a managed pane.
/// Printable width of a `capture-pane -e` line, ignoring SGR escapes.
fn visible_width(raw: &str) -> usize {
    let mut width = 0;
    let bytes = raw.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == 0x1b && i + 1 < bytes.len() && bytes[i + 1] == b'[' {
            let mut j = i + 2;
            while j < bytes.len() && !bytes[j].is_ascii_alphabetic() {
                j += 1;
            }
            i = (j + 1).min(bytes.len());
            continue;
        }
        let ch = raw[i..].chars().next().unwrap_or(' ');
        if !ch.is_control() {
            width += if ch.is_ascii() { 1 } else { 2 };
        }
        i += ch.len_utf8();
    }
    width
}

fn render_command_panel(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let Some(panel) = app.command_panel.as_ref() else {
        return;
    };
    // Size the panel to its output, not the whole pane: trim trailing blank
    // lines so a short command gets a small box.
    let mut content: Vec<&String> = panel.lines.iter().collect();
    while content
        .last()
        .is_some_and(|l| visible_width(l) == 0 || l.trim().is_empty())
    {
        content.pop();
    }
    let state = if panel.alive {
        " · running".to_string()
    } else {
        match panel.status {
            Some(0) | None => " · done".into(),
            Some(code) => format!(" · exited {code}"),
        }
    };
    let title = format!("{}{}", safe_label(&panel.label), state);
    let hint = if panel.alive {
        "Esc close · Ctrl+O expand · keys to command"
    } else {
        "Esc close · Ctrl+O full screen"
    };
    // A running command with no output yet shows a compact waiting line.
    let placeholder = content.is_empty();
    let body_rows = if placeholder { 1 } else { content.len() } as u16;
    let content_w = content.iter().map(|l| visible_width(l)).max().unwrap_or(0);
    let inner_w = content_w
        .max(title.chars().count())
        .max(hint.chars().count())
        .clamp(16, area.width.saturating_sub(4) as usize) as u16;
    let inner_h = (body_rows + 1).min(area.height.saturating_sub(4));
    let rect = popup(area, inner_w + 4, inner_h + 2);
    frame.render_widget(Clear, rect);
    let outer = block(title, true).padding(Padding::horizontal(1));
    let inner = outer.inner(rect);
    frame.render_widget(outer, rect);
    let body = Rect {
        height: inner.height.saturating_sub(1),
        ..inner
    };
    let lines: Vec<Line> = if placeholder {
        vec![Line::from(Span::styled("working…", muted()))]
    } else {
        let skip = content.len().saturating_sub(body.height as usize);
        content
            .iter()
            .skip(skip)
            .map(|raw| ansi_line(raw))
            .collect()
    };
    frame.render_widget(Paragraph::new(lines), body);
    frame.render_widget(
        Paragraph::new(hint).style(muted()),
        Rect {
            y: inner.y + inner.height.saturating_sub(1),
            height: 1,
            ..inner
        },
    );
}

/// Parse a tmux `capture-pane -e` line (SGR escapes) into a styled Line.
fn palette_style(mut style: Style, palette8: Option<Color>) -> Style {
    if let Some(colour) = palette8 {
        if matches!(style.fg, Some(Color::DarkGray | Color::Indexed(8))) {
            style.fg = Some(colour);
        }
        if matches!(style.bg, Some(Color::DarkGray | Color::Indexed(8))) {
            style.bg = Some(colour);
        }
    }
    style
}
#[cfg(test)]
pub(super) fn ansi_line_with_palette(raw: &str, palette8: Option<Color>) -> Line<'static> {
    let mut line = ansi_line(raw);
    for span in &mut line.spans {
        span.style = palette_style(span.style, palette8);
    }
    line
}
/// capture-pane emits one continuous SGR stream, including across newline boundaries.
pub(super) fn ansi_screen(
    raw: &[String],
    palette8: Option<Color>,
    cols: u16,
) -> Vec<Line<'static>> {
    let mut style = Style::reset();
    raw.iter()
        .map(|row| {
            let mut line = ansi_line_state(row, &mut style);
            let padding = usize::from(cols).saturating_sub(line.width());
            if padding > 0 {
                line.spans.push(Span::styled(" ".repeat(padding), style));
            }
            for span in &mut line.spans {
                span.style = palette_style(span.style, palette8);
            }
            line
        })
        .collect()
}
pub(super) fn ansi_line(raw: &str) -> Line<'static> {
    ansi_line_state(raw, &mut Style::default())
}
fn ansi_line_state(raw: &str, current: &mut Style) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut style = *current;
    let mut text = String::new();
    let bytes = raw.as_bytes();
    let mut i = 0;
    let flush = |spans: &mut Vec<Span<'static>>, text: &mut String, style: Style| {
        if !text.is_empty() {
            spans.push(Span::styled(std::mem::take(text), style));
        }
    };
    while i < bytes.len() {
        if bytes[i] == 0x1b && i + 1 < bytes.len() && bytes[i + 1] == b'[' {
            let mut j = i + 2;
            while j < bytes.len() && !bytes[j].is_ascii_alphabetic() {
                j += 1;
            }
            if j < bytes.len() {
                if bytes[j] == b'm' {
                    flush(&mut spans, &mut text, style);
                    style = apply_sgr(style, &raw[i + 2..j]);
                }
                // Non-color control sequences are dropped.
                i = j + 1;
                continue;
            }
            break;
        }
        let ch = raw[i..].chars().next().unwrap_or(' ');
        if !ch.is_control() {
            text.push(ch);
        }
        i += ch.len_utf8();
    }
    flush(&mut spans, &mut text, style);
    if spans.is_empty() {
        spans.push(Span::raw(""));
    }
    *current = style;
    Line::from(spans)
}

fn ansi_color(code: u8) -> Color {
    match code {
        0 => Color::Black,
        1 => Color::Red,
        2 => Color::Green,
        3 => Color::Yellow,
        4 => Color::Blue,
        5 => Color::Magenta,
        6 => Color::Cyan,
        7 => Color::Gray,
        8 => Color::DarkGray,
        9 => Color::LightRed,
        10 => Color::LightGreen,
        11 => Color::LightYellow,
        12 => Color::LightBlue,
        13 => Color::LightMagenta,
        14 => Color::LightCyan,
        _ => Color::White,
    }
}

/// Apply one SGR parameter list to a style. Unsupported codes are ignored.
fn apply_sgr(mut style: Style, params: &str) -> Style {
    let parts: Vec<i64> = if params.is_empty() {
        vec![0]
    } else {
        params.split(';').map(|p| p.parse().unwrap_or(0)).collect()
    };
    let mut k = 0;
    while k < parts.len() {
        match parts[k] {
            0 => style = Style::reset(),
            1 => style = style.add_modifier(Modifier::BOLD),
            2 => style = style.add_modifier(Modifier::DIM),
            22 => style = style.remove_modifier(Modifier::BOLD | Modifier::DIM),
            3 => style = style.add_modifier(Modifier::ITALIC),
            4 => style = style.add_modifier(Modifier::UNDERLINED),
            5 => style = style.add_modifier(Modifier::SLOW_BLINK),
            6 => style = style.add_modifier(Modifier::RAPID_BLINK),
            7 => style = style.add_modifier(Modifier::REVERSED),
            8 => style = style.add_modifier(Modifier::HIDDEN),
            9 => style = style.add_modifier(Modifier::CROSSED_OUT),
            23 => style = style.remove_modifier(Modifier::ITALIC),
            24 => style = style.remove_modifier(Modifier::UNDERLINED),
            25 => style = style.remove_modifier(Modifier::SLOW_BLINK | Modifier::RAPID_BLINK),
            27 => style = style.remove_modifier(Modifier::REVERSED),
            28 => style = style.remove_modifier(Modifier::HIDDEN),
            29 => style = style.remove_modifier(Modifier::CROSSED_OUT),
            30..=37 => style = style.fg(ansi_color(parts[k] as u8 - 30)),
            90..=97 => style = style.fg(ansi_color(parts[k] as u8 - 90 + 8)),
            40..=47 => style = style.bg(ansi_color(parts[k] as u8 - 40)),
            100..=107 => style = style.bg(ansi_color(parts[k] as u8 - 100 + 8)),
            39 => style.fg = Some(Color::Reset),
            49 => style.bg = Some(Color::Reset),
            38 | 48 => {
                let fg = parts[k] == 38;
                if parts.get(k + 1) == Some(&5) {
                    if let Some(&n) = parts.get(k + 2) {
                        let color = Color::Indexed(n.clamp(0, 255) as u8);
                        style = if fg { style.fg(color) } else { style.bg(color) };
                    }
                    k += 2;
                } else if parts.get(k + 1) == Some(&2) {
                    if let (Some(&r), Some(&g), Some(&b)) =
                        (parts.get(k + 2), parts.get(k + 3), parts.get(k + 4))
                    {
                        let color = Color::Rgb(r as u8, g as u8, b as u8);
                        style = if fg { style.fg(color) } else { style.bg(color) };
                    }
                    k += 4;
                }
            }
            _ => {}
        }
        k += 1;
    }
    style
}

#[cfg(test)]
mod ansi_tests {
    use super::*;

    #[test]
    fn parses_colors_bold_and_strips_cursor_moves() {
        let line = ansi_line("\x1b[1;31mred\x1b[0m plain\x1b[5;10Hmoved");
        let spans: Vec<(String, Style)> = line
            .spans
            .iter()
            .map(|s| (s.content.to_string(), s.style))
            .collect();
        assert_eq!(spans[0].0, "red");
        assert_eq!(spans[0].1.fg, Some(Color::Red));
        assert!(spans[0].1.add_modifier.contains(Modifier::BOLD));
        // The reset clears bold and color for the following text.
        assert!(
            spans[1].1.fg == Some(Color::Reset)
                && !spans[1].1.add_modifier.contains(Modifier::BOLD)
        );
        // The cursor-move sequence is dropped, its literal text kept.
        let joined: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(joined, "red plainmoved");
    }

    #[test]
    fn parses_256_and_truecolor() {
        let line = ansi_line("\x1b[38;5;200mx\x1b[48;2;1;2;3my");
        assert_eq!(line.spans[0].style.fg, Some(Color::Indexed(200)));
        assert_eq!(line.spans[1].style.bg, Some(Color::Rgb(1, 2, 3)));
    }

    #[test]
    fn plain_text_is_one_span() {
        let line = ansi_line("just text");
        let joined: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(joined, "just text");
    }
}

#[cfg(test)]
mod contrast_regressions {
    use super::*;
    #[test]
    fn captured_rows_share_style_and_paint_trailing_cells() {
        let rows = vec![
            "\x1b[48;5;8mfirst".into(),
            "second".into(),
            "\x1b[49mplain".into(),
        ];
        let lines = ansi_screen(&rows, Some(Color::Rgb(48, 48, 48)), 20);
        for row in &lines[..2] {
            assert_eq!(row.width(), 20);
            assert!(row
                .spans
                .iter()
                .all(|s| s.style.bg == Some(Color::Rgb(48, 48, 48))));
        }
        assert!(lines[2]
            .spans
            .iter()
            .all(|s| s.style.bg == Some(Color::Reset)));
    }

    #[test]
    fn prompt_selection_end_restores_normal_foreground_and_muted_background() {
        let line = ansi_line("\x1b[7mselected\x1b[27;36;48;2;36;39;46mprior prompt");
        let prior = &line.spans[1].style;
        assert!(
            !prior.add_modifier.contains(Modifier::REVERSED),
            "accent foreground is still inverted into the background"
        );
        assert_eq!(prior.fg, Some(Color::Cyan));
        assert_eq!(prior.bg, Some(Color::Rgb(36, 39, 46)));
    }
    #[test]
    fn pane_palette_is_applied_to_both_prompt_background_and_dim_text() {
        let neutral = Color::Rgb(48, 48, 48);
        let line = ansi_line_with_palette("\x1b[48;5;8mprompt\x1b[49;90mdim", Some(neutral));
        assert_eq!(line.spans[0].style.bg, Some(neutral));
        assert_eq!(line.spans[1].style.fg, Some(neutral));
        assert_eq!(line.spans[1].style.bg, Some(Color::Reset));
    }
    #[test]
    fn dim_and_attribute_end_sequences_preserve_native_contrast() {
        let line = ansi_line("\x1b[2mdim\x1b[22mnormal\x1b[3;4;7mselected\x1b[23;24;27mplain");
        assert!(line.spans[0].style.add_modifier.contains(Modifier::DIM));
        assert!(!line.spans[1]
            .style
            .add_modifier
            .intersects(Modifier::DIM | Modifier::BOLD));
        assert!(!line.spans[3]
            .style
            .add_modifier
            .intersects(Modifier::ITALIC | Modifier::UNDERLINED | Modifier::REVERSED));
    }
}
