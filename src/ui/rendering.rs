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
    let query = filters::visible_query(app);
    let show_search = matches!(
        app.input,
        Some(Input::Search | Input::PreviewSearch | Input::Filter)
    ) || !query.is_empty();
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(5),
            Constraint::Length(if show_search { 7 } else { 4 }),
        ])
        .split(area);
    let label = match (app.view, area.width < 70) {
        (View::Work, true) => "Sessions",
        (View::Files, true) => "Files",
        (View::Containers, true) => "Containers",
        (View::Containers, false) => "Sessions  Network  [Containers]",
        (View::Network, true) => "Network",
        (View::Work, false) => "[Sessions]  Network",
        (View::Files, false) => "Sessions / Files  Network",
        (View::Network, false) => "Sessions  [Network]",
    };
    let viewer = app
        .devices
        .iter()
        .find(|d| d.target.is_none())
        .map(identity)
        .unwrap_or_else(|| "local viewer".into());
    let top = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Min(20),
            Constraint::Length(viewer.len().min(40) as u16 + 8),
        ])
        .split(vertical[0]);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" cx ", accent().add_modifier(Modifier::BOLD)),
            Span::raw("  "),
            Span::styled(label, Style::default().add_modifier(Modifier::BOLD)),
        ])),
        top[0],
    );
    frame.render_widget(
        Paragraph::new(format!("viewing {viewer}"))
            .style(muted())
            .alignment(ratatui::layout::Alignment::Right),
        top[1],
    );
    let sidebar_width: u16 =
        if area.width < 60 && app.view == View::Files && app.focus == Focus::Workspace {
            14
        } else if area.width < 70 {
            17
        } else {
            21
        };
    let content = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(sidebar_width), Constraint::Min(15)])
        .split(vertical[1]);
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
            _ => "?",
        };
        let dot_style = if std::env::var_os("NO_COLOR").is_some() {
            Style::default()
        } else {
            Style::default().fg(match status {
                "ready" => Color::Green,
                "checking" => Color::Cyan,
                "unavailable" => Color::Yellow,
                _ => Color::Reset,
            })
        };
        let access_marker = match app.device_policy(i).access {
            store::AccessMode::Core if ascii() => "<->",
            store::AccessMode::Core => "↔",
            store::AccessMode::Directed if ascii() => "->",
            store::AccessMode::Directed => "→",
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
    *app.panels.borrow_mut() = vec![
        (Focus::Devices, false, sidebar[0]),
        (Focus::Actions, false, sidebar[1]),
    ];
    if app.view != View::Files || app.other_browser.is_none() {
        app.panels
            .borrow_mut()
            .push((Focus::Workspace, app.destination_active, content[1]));
    }
    let mut state = ratatui::widgets::ListState::default()
        .with_selected(app.device_rows().iter().position(|d| *d == app.device));
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
                safe_label(&s.directory)
            )
        } else {
            format!(
                "Host {}\nFolder {}\nEnter open · n new",
                identity(&app.devices[i]),
                safe_label(&s.directory)
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
                            "[ Enter / n  New shell ]"
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
                let compact = workspace.width < 50;
                let mut display_rows = Vec::new();
                let mut selected_row = 0;
                let mut last_project = None;
                for (index, (i, s)) in rows.iter().enumerate() {
                    if last_project != Some(s.directory.as_str()) {
                        if last_project.is_some() {
                            display_rows.push(Row::new(vec![Cell::from("")]).height(1));
                        }
                        display_rows.push(
                            Row::new(if compact {
                                vec![Cell::from(safe_label(&s.directory)), Cell::from("")]
                            } else {
                                vec![
                                    Cell::from(""),
                                    Cell::from(safe_label(&s.directory)),
                                    Cell::from(""),
                                    Cell::from(""),
                                ]
                            })
                            .style(muted().add_modifier(Modifier::BOLD))
                            .height(1),
                        );
                        last_project = Some(s.directory.as_str());
                    }
                    if index == app.selected {
                        selected_row = display_rows.len();
                    }
                    let fresh = app.work[*i].fetched > 0
                        && transport::now().saturating_sub(app.work[*i].fetched) <= 60
                        && app.work[*i].error.is_none();
                    let badge = format!("[{}]", safe_label(menus::session_label(app, *i, s)));
                    let cells = vec![
                        Cell::from(badge).style(if std::env::var_os("NO_COLOR").is_some() {
                            Style::default()
                        } else {
                            Style::default().fg(match s.provider.as_str() {
                                "claude" => Color::Yellow,
                                "codex" => Color::Cyan,
                                _ => Color::Gray,
                            })
                        }),
                        Cell::from(if compact && s.container.is_some() {
                            format!(
                                "{} · {}",
                                menus::session_label(app, *i, s),
                                safe_label(&s.name)
                            )
                        } else {
                            safe_label(&s.name)
                        }),
                        Cell::from(identity(&app.devices[*i])),
                        Cell::from(if !fresh {
                            "? cached"
                        } else if s.external {
                            if ascii() {
                                "+ ext"
                            } else {
                                "● ext"
                            }
                        } else {
                            if ascii() {
                                "+ live"
                            } else {
                                "● live"
                            }
                        })
                        .style(if !fresh { muted() } else { accent() }),
                    ];
                    display_rows.push(Row::new(if compact {
                        vec![cells[1].clone(), cells[3].clone()]
                    } else {
                        cells
                    }));
                }
                let mut state = TableState::default().with_selected(Some(selected_row));
                let table = Table::new(
                    display_rows,
                    if compact {
                        vec![Constraint::Min(10), Constraint::Length(8)]
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
                            Constraint::Length(8),
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
                    );
                    render_browser(
                        frame,
                        destination,
                        &app.devices[destination.device],
                        panes[1],
                        app.destination_active && app.focus == Focus::Workspace,
                        &format!("Destination · {}", app.conflict_policy()),
                        app.clipboard.as_ref(),
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
                    );
                }
            }
        }
        View::Containers => render_containers(frame, app, workspace),
        View::Network => render_network(frame, app, workspace),
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
    let footer = Layout::default()
        .direction(Direction::Vertical)
        .constraints(if show_search {
            vec![
                Constraint::Length(3),
                Constraint::Length(1),
                Constraint::Length(2),
                Constraint::Length(1),
            ]
        } else {
            vec![
                Constraint::Length(1),
                Constraint::Length(2),
                Constraint::Length(1),
            ]
        })
        .split(vertical[2]);
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
        app.work
            .iter()
            .enumerate()
            .filter(|(i, _)| app.device == 0 || app.device == i + 1)
            .map(|(_, w)| w.sessions.len())
            .sum()
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
    let healthy = if std::env::var_os("NO_COLOR").is_some() {
        Style::default()
    } else {
        Style::default().fg(Color::Green)
    };
    let warning = if std::env::var_os("NO_COLOR").is_some() {
        Style::default()
    } else {
        Style::default().fg(Color::Yellow)
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            if errors > 0 {
                format!(
                    " {} {errors} unavailable  ",
                    if ascii() { "!" } else { "◆" }
                )
            } else {
                format!(
                    " {} {ready}/{} devices  ",
                    if ascii() { "+" } else { "●" },
                    app.devices.len()
                )
            },
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
    let transfer = if active > 0 {
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
    let mut hints = if app.help {
        vec![("↑↓", "Scroll"), ("PgUpDn", "Page"), ("Esc", "Close")]
    } else if let Some(dialog) = &app.dialog {
        if matches!(dialog, Dialog::Jobs) {
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
        } else if matches!(dialog, Dialog::Delete(..) | Dialog::StopShell(..)) {
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
        Some(Input::Rename | Input::Mkdir | Input::Palette | Input::Command)
    ) {
        vec![
            (
                "Enter",
                if app.input == Some(Input::Command) {
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
    } else if app.view == View::Files
        && app.focus == Focus::Workspace
        && app.browser.as_ref().is_some_and(|b| b.preview.is_some())
    {
        let mut hints = vec![
            (
                "j/k",
                if app.pdf_preview_active() {
                    "Pages"
                } else {
                    "Scroll"
                },
            ),
            ("gg / G", "Top / bottom"),
            ("Esc", "Back"),
            ("?", "Help"),
        ];
        if app.text_preview_active() {
            hints.insert(2, ("/", "Search"));
            hints.insert(3, ("n / N", "Next / previous"));
            if app.markdown_preview_active() {
                hints[1] = ("o", "Links");
            }
        }
        hints
    } else if app.view == View::Network && app.focus == Focus::Workspace {
        vec![
            ("j/k", "Devices / LAN"),
            ("Enter", "Open / connect"),
            if app.device == 0 {
                ("h / l", "Fold / expand")
            } else {
                ("Home", "Host details")
            },
            ("J / K", "Scroll details"),
            ("Ctrl P", "Actions"),
            ("?", "Help"),
            ("Ctrl C", "Quit"),
        ]
    } else if app.view == View::Containers && app.focus == Focus::Workspace {
        vec![
            ("Enter", "Actions"),
            ("n", "New session"),
            ("h / l", "Fold / expand"),
            ("r", "Refresh"),
            ("/", "Search"),
            ("Ctrl P", "Actions"),
        ]
    } else if app.view == View::Files
        && app.focus == Focus::Workspace
        && app.destination_active
        && app.clipboard.is_some()
    {
        vec![
            ("Enter", "Transfer here"),
            ("h / l", "Parent / open"),
            ("Tab", "Source"),
            ("o", "Conflict policy"),
            ("T", "Progress"),
            ("?", "All keys"),
        ]
    } else if app.view == View::Files && app.focus == Focus::Workspace {
        vec![
            ("Space", "Select"),
            ("c / x", "Copy / cut"),
            ("p / t", "Paste / transfer"),
            (
                if app.browser.as_ref().is_some_and(|b| !b.search.is_empty()) {
                    "n / N"
                } else {
                    "n / :"
                },
                if app.browser.as_ref().is_some_and(|b| !b.search.is_empty()) {
                    "Next / previous"
                } else {
                    "Session / command"
                },
            ),
            ("r / d", "Rename / delete"),
            ("?", "All keys"),
        ]
    } else if empty_work_can_create(app) && app.focus == Focus::Workspace {
        vec![
            ("Enter/n", "New shell"),
            (
                if local_only(app) { "a" } else { "/" },
                if local_only(app) {
                    "Add by SSH address"
                } else {
                    "Search"
                },
            ),
            ("Tab", "Focus"),
            ("Ctrl P", "Actions"),
            ("Ctrl C", "Quit"),
            ("?", "Help"),
        ]
    } else if app.view == View::Work && app.focus == Focus::Workspace {
        vec![
            ("Enter", "Open"),
            ("n", "New session"),
            ("w", "Watch · read-only"),
            ("d", "Stop · confirm"),
            ("Ctrl P", "Actions"),
            ("?", "Help"),
        ]
    } else {
        vec![
            (
                "Enter",
                if app.focus == Focus::Actions {
                    "Run"
                } else {
                    "Open"
                },
            ),
            ("/", "Search"),
            ("Tab", "Focus"),
            ("Ctrl P", "Actions"),
            ("?", "Help"),
            ("Ctrl C", "Quit"),
        ]
    };
    let columns = if footer[key_row].width >= 60 { 3 } else { 2 };
    if columns == 2
        && !matches!(
            app.input,
            Some(Input::Search | Input::PreviewSearch | Input::Filter)
        )
    {
        hints.retain(|(_, label)| !matches!(*label, "Search" | "Focus"));
        hints.truncate(4);
        if app.dialog.is_none()
            && app.input.is_none()
            && !app.help
            && app.view == View::Files
            && app.browser.as_ref().is_some_and(|b| b.preview.is_none())
        {
            hints[3] = if app.destination_active && app.clipboard.is_some() {
                ("T", "Progress")
            } else {
                ("t / T", "Send / jobs")
            };
        }
    }
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1)])
        .split(footer[key_row]);
    for (i, (key, label)) in hints.into_iter().take(columns * 2).enumerate() {
        let key = if ascii() && key == "↑↓" {
            "j/k"
        } else {
            key
        };
        let cells = Layout::default()
            .direction(Direction::Horizontal)
            .constraints(vec![Constraint::Ratio(1, columns as u32); columns])
            .split(rows[i / columns]);
        let key_style = if std::env::var_os("NO_COLOR").is_some() {
            Style::default().add_modifier(Modifier::BOLD)
        } else {
            accent().add_modifier(Modifier::BOLD)
        };
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(format!(" {key} "), key_style),
                Span::raw(label),
            ])),
            cells[i % columns],
        );
    }
    notifications::render(
        frame,
        &app.notice,
        app.notice_kind,
        workspace,
        notifications::appearance(app.notice_started, app.notice_deadline, now),
    );
    if let Some(input) = app
        .input
        .filter(|i| !matches!(*i, Input::Search | Input::PreviewSearch | Input::Filter))
    {
        let rect = popup(
            area,
            if input == Input::Add { 64 } else { 76 },
            if input == Input::Palette {
                19
            } else if matches!(input, Input::Rename | Input::Command | Input::Add) {
                3
            } else {
                5
            },
        );
        frame.render_widget(Clear, rect);
        if input == Input::Palette {
            let parts = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(3),
                    Constraint::Min(1),
                    Constraint::Length(3),
                ])
                .split(rect);
            frame.render_widget(
                Paragraph::new(format!("> {}", safe_text(&app.text)))
                    .block(block("Actions · type to filter".into(), true)),
                parts[0],
            );
            let items = app
                .palette()
                .iter()
                .map(|(_, s)| ListItem::new(*s))
                .collect::<Vec<_>>();
            let mut state =
                ratatui::widgets::ListState::default().with_selected(Some(app.palette_selected));
            frame.render_stateful_widget(
                List::new(items)
                    .block(Block::default().borders(Borders::ALL))
                    .highlight_style(selected_style()),
                parts[1],
                &mut state,
            );
            frame.render_widget(
                Paragraph::new(app.palette_scope())
                    .wrap(Wrap { trim: false })
                    .block(Block::default().borders(Borders::ALL)),
                parts[2],
            );
        } else {
            let editing = matches!(input, Input::Rename | Input::Command | Input::Add);
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
                Paragraph::new(
                    if matches!(input, Input::Rename | Input::Command | Input::Add) {
                        format!("> {}", safe_label(&app.text))
                    } else {
                        format!("> {}\nEnter confirm · Escape cancel", safe_label(&app.text))
                    },
                )
                .scroll((0, scroll))
                .block(block(
                    if input == Input::Command {
                        app.command_target
                            .as_ref()
                            .map(|(d, path)| {
                                format!(
                                    "Run command · {} · {}",
                                    identity(&app.devices[*d]),
                                    safe_label(
                                        app.browser
                                            .as_ref()
                                            .filter(|b| b.device == *d && b.path == *path)
                                            .map(|b| b.display_path.as_str())
                                            .unwrap_or(path)
                                    )
                                )
                            })
                            .unwrap_or_else(|| "Run command".into())
                    } else if input == Input::Add && app.network_add_target.is_some() {
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
                            Input::Command => "Run command",
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
            Dialog::ContainerProvider(d,scope)=>(format!("Session · {} / {}",safe_label(&app.devices[*d].name),safe_label(&scope.name)),vec!["Shell · container user".into()],"Runtime availability is checked before launch. No packages are installed. Enter choose · Escape cancel".into()),
            Dialog::ContainerConfirm(d,c,action)=>(format!("{} {}?",action,safe_label(&c.name)),vec!["Cancel · keep current state".into(),format!("{} selected container",action)],format!("{}\n{}\n{}\n{}\n{}",safe_label(&app.devices[*d].name),safe_label(&c.name),safe_label(&c.id[..12]),if action=="Rebuild"{"The CLI replaces the container and its writable layer and ends sessions/transfers using it. Back up container-only files first. Workspace hooks may run and Compose services may restart. Mounted data follows the workspace configuration; no volume removal flag is used. Allow up to 10 minutes."}else if action=="Stop"{"Running work in this container will end."}else if action=="Enable access"{"Enable this container for terminals, files, transfers and lifecycle actions on this host account."}else{"Existing image, volumes and ports are preserved."},if action=="Rebuild"{"The selected workspace must uniquely match this container. Rebuild follows its declared configuration."}else{"Only this exact container ID is targeted. No rebuild/removal or network changes."})),
            Dialog::Device(purpose) => (
                match purpose {
                    ChooseDevice::Containers => "Containers · choose device or All devices",
                    ChooseDevice::Work => "Sessions · choose device or All devices",
                    ChooseDevice::Network => "Network · choose device or All devices",
                    ChooseDevice::AddGateway => "Add by SSH address · choose gateway",
                    ChooseDevice::SessionChooser => "New session · choose device",
                    ChooseDevice::Shell => "New shell · choose device · starts in home",
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
            Dialog::SessionChooser(d,path) => (
                format!("New session · {}",safe_label(&app.devices[*d].name)),
                menus::HOST.iter().map(|choice|choice.label.into()).collect(),
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
            Dialog::Permissions(d, path, provider, resume) => (
                format!("{} permissions · {}", provider, identity(&app.devices[*d])),
                vec!["Default · existing host permissions".into(), "YOLO · bypass approvals".into()],
                format!("{}\n{}Applies to this session only.\nYOLO bypasses approval prompts; Codex also disables its sandbox.\nEnter starts · Escape cancels", safe_label(path), if *resume { "Native resume: " } else { "Fresh session: " }),
            ),
            Dialog::AgentStart(d, path, provider) => (
                format!("{} session · {}", provider, identity(&app.devices[*d])),
                vec!["Fresh session".into(), "Resume latest session".into()],
                format!("{}\nFresh starts a new conversation. Resume uses the provider's native {} resume command in this folder.\nEnter chooses · Escape cancels", safe_label(path), if provider == "codex" { "Codex" } else { "Claude" }),
            ),
            Dialog::StopShell(d, session) => (
                format!("Stop {} session?", session.provider), vec!["Keep session".into(), "Stop session".into()],
                format!("Running work in this session will end.\n{}\n{}\n{}", identity(&app.devices[*d]), safe_label(&session.name), safe_label(&session.directory)),
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
            Dialog::Matching(d, path, provider, s) => (
                format!("Matching {provider} · {}", identity(&app.devices[*d])),
                vec![
                    format!("Open existing · {}", safe_label(&s.name)),
                    "Resume latest session".into(),
                    "Create fresh session".into(),
                ],
                format!("{}\nResume uses the provider's native command in this folder; fresh starts a new conversation.", safe_label(path)),
            ),
            Dialog::Peer(d) => (
                format!("Device · {}", safe_label(&app.devices[*d].name)),
                vec!["Open sessions".into(), "Browse files".into(), "New persistent shell · tmux".into(), "New session".into(), "SSH terminal · exit returns".into()],
                format!("{}\n{}\nFrom viewer · helper evidence\nVia selected device: unknown", identity(&app.devices[*d]), app.peer_status(*d)),
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
        } else if matches!(dialog, Dialog::Delete(..) | Dialog::StopShell(..)) {
            let stop = matches!(dialog, Dialog::StopShell(..));
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
        let rect = popup(area, 76, 20);
        frame.render_widget(Clear, rect);
        let key_style = accent().add_modifier(Modifier::BOLD);
        let key_row = |keys: &str, description: &str| {
            Line::from(vec![
                Span::styled(format!("{keys:<22} "), key_style),
                Span::raw(description.to_owned()),
            ])
        };
        let mut help = vec![
            key_row("Arrows / h j k l", "Navigate"),
            key_row("Ctrl+arrows / hjkl", "Move panel focus (hold Ctrl)"),
            key_row("Enter", "Open / enter session with input"),
            key_row("Escape", "Back / clear current mode"),
            key_row("Tab / Shift+Tab", "Next / previous panel"),
            key_row("/", "Search"),
            key_row("Ctrl+P", "Actions"),
            key_row(
                "n",
                if app.view == View::Files
                    && app.browser.as_ref().is_some_and(|b| !b.search.is_empty())
                {
                    "Next matching file; clear search for New session"
                } else {
                    "New session in current folder"
                },
            ),
            key_row(":", "Run command in current folder"),
            key_row("Ctrl+C", "Quit cx; work keeps running"),
        ];
        if app.view == View::Work {
            help.push(key_row("d", "Stop selected CX-managed session · confirm"));
            help.push(key_row("w", "Watch selected session · read-only"));
        }
        if app.focus == Focus::Devices {
            help.push(key_row(
                "b",
                "Toggle selected remote device: ↔ bidirectional / → viewer-to-device",
            ));
        }
        if app.view == View::Files && app.browser.as_ref().is_some_and(|b| b.preview.is_some()) {
            help.extend([
                key_row("gg / G", "Preview top / bottom (first/last PDF page)"),
                key_row("j/k · PgUp/PgDn", "Scroll preview / change PDF page"),
                key_row(
                    "Escape",
                    "Finish search / clear highlights / return to files",
                ),
            ]);
            if app.text_preview_active() {
                help.extend([
                    key_row("/", "Search preview text, live fuzzy matches"),
                    key_row("n / N", "Next / previous match, wrap around"),
                    key_row(
                        "o / click link",
                        "Inspect links; Enter opens viewer browser",
                    ),
                ]);
            }
        } else if app.view == View::Files {
            help.push(Line::from(Span::styled("Files", key_style)));
            for (keys, description) in [
                ("Left/h · Right/l", "Parent / enter directory"),
                ("Space", "Select and advance"),
                ("v · u", "Visual range / clear selection"),
                (".", "Show / hide hidden files"),
                ("/ · n / N", "Highlight names / next / previous match"),
                ("f", "Filter names"),
                (
                    "c/y · x",
                    "Copy / cut, then choose another folder or device",
                ),
                ("p · Y", "Paste here / clear clipboard"),
                (
                    "t",
                    "Transfer to… choose device, folder, then Enter (h/l navigate)",
                ),
                ("T", "Transfer jobs and results"),
                ("r · d", "Rename / confirm permanent deletion"),
                ("M · o", "New folder / cycle copy conflict policy"),
                ("gg / G · 5j / 5k", "First/last file / counted movement"),
            ] {
                help.push(key_row(keys, description));
            }
        }
        if matches!(app.dialog, Some(Dialog::Jobs)) {
            help.push(Line::from(Span::styled("Transfer jobs", key_style)));
            help.extend([
                key_row("c · r", "Cancel / retry saved job"),
                key_row("Enter / Tab", "Focus details"),
                key_row("PgUp/PgDn", "Scroll details"),
            ]);
        }
        help.push(Line::default());
        if app.view == View::Work
            && app
                .selected_session()
                .is_some_and(|(_, s)| !s.external && s.provider == "shell")
        {
            help.push(key_row(
                "d",
                "Stop selected CX-managed session · confirmation",
            ));
        }
        help.push(Line::raw("Native terminals own their input."));
        help.push(key_row("Ctrl+]", "Return from a managed session"));
        help.push(Line::raw("External sessions keep their tmux bindings."));
        help.push(Line::default());
        help.push(Line::from(Span::styled(
            "Available actions · Ctrl+P",
            key_style,
        )));
        for (action, label) in ACTIONS
            .iter()
            .filter(|(a, _)| workspace_action(*a) && app.action_enabled(*a))
        {
            let keys = match action {
                Action::TransferTo => "t",
                Action::Jobs => "T",
                Action::Copy => "c / y",
                Action::Cut => "x",
                Action::Rename => "r",
                Action::Delete => "d",
                Action::Hidden => ".",
                Action::Filter => "f",
                Action::Select => "Space",
                Action::Visual => "v",
                Action::Paste => "p",
                Action::Help => "? / F1",
                Action::Quit => "Ctrl+C",
                _ => "",
            };
            let description = if matches!(
                action,
                Action::TransferTo
                    | Action::Copy
                    | Action::Cut
                    | Action::Rename
                    | Action::Delete
                    | Action::Hidden
                    | Action::Filter
                    | Action::Select
                    | Action::Visual
            ) {
                label
                    .rsplit_once(" · ")
                    .map(|(description, _)| description)
                    .unwrap_or(label)
            } else {
                label
            };
            help.push(key_row(keys, description));
        }
        frame.render_widget(
            Paragraph::new(help)
                .wrap(Wrap { trim: false })
                .scroll((app.help_scroll, 0))
                .block(block("Help".into(), true)),
            rect,
        );
    }
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
