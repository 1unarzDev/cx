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
    begin_frame(overlay_owns_input(app));
    render_frame(frame, app, native, now);
    let area = frame.area();
    fit_titles(frame.buffer_mut(), area);
    if ascii() {
        // ASCII parity (spec C11): UI glyphs from every view, in one place.
        asciify(frame.buffer_mut(), area);
    }
}
/// True while a modal overlay (help, a dialog, the palette or another input
/// popup, the command panel, the "New session" popup) owns input. Inline
/// search and filter keep the list focused.
pub(super) fn overlay_owns_input(app: &App) -> bool {
    app.help
        || app.dialog.is_some()
        || app.command_panel.is_some()
        || app.creating
        || app
            .input
            .is_some_and(|i| !matches!(i, Input::Search | Input::PreviewSearch | Input::Filter))
}
fn render_frame(
    frame: &mut Frame<'_>,
    app: &App,
    native: Option<&mut crate::terminal_preview::NativePreview>,
    now: Instant,
) {
    let area = frame.area();
    hitmap::begin();
    app.preview_rect.set(None);
    let vm = viewmodel::ViewModel::from(app);
    let drawn = views::mission::render(&vm.view, &vm.state, &vm.caps, area, frame.buffer_mut());
    if vm.view == views::LabView::Hub {
        *app.hub_order.borrow_mut() = drawn.order;
    }
    // Phones keep a spacer row above their footer (views::mission::views::draw).
    let spare = u16::from(area.width < 70);
    let body = Rect::new(
        area.x,
        area.y + 1,
        area.width,
        area.height.saturating_sub(2 + spare),
    );
    // One workspace panel takes the wheel (and the open preview's clicks).
    *app.panels.borrow_mut() = vec![(Focus::Workspace, false, body)];
    let files = hitmap::file_rows(app);
    let mut preview = None;
    for t in &drawn.targets {
        // Two surfaces the app draws and routes itself (preview links and wheel,
        // typing into a peek): geometry only, not tap targets.
        match t.action.as_str() {
            "preview-body" => preview = Some(t.area),
            "peek-screen" => app.set_preview_screen(t.area),
            _ => hitmap::register_action(t.area, &t.action, &files),
        }
    }
    // The real file preview draws its body where the design put the preview surface.
    if let (Some(b), Some(r)) = (app.active_browser(), preview) {
        let r = Rect::new(r.x, r.y, r.width, body.bottom().saturating_sub(r.y));
        let shown = b.preview.as_ref().map(|t| (b, t.as_str())).or_else(|| {
            b.peek
                .as_deref()
                .and_then(|p| p.preview.as_deref().map(|t| (p, t)))
        });
        if let Some((pb, text)) = shown {
            frame.render_widget(Clear, r);
            if b.preview.is_some() {
                app.preview_rect.set(Some(r));
            }
            render_preview_body(frame, r, pb, text, native);
            // The inline peek of a file no longer selected stays, dimmed, until
            // the new one arrives (a frame from memory, never a blank).
            if b.preview.is_none() && pb.preview_pending_page.is_some() {
                frame
                    .buffer_mut()
                    .set_style(r, Style::default().add_modifier(Modifier::DIM));
            }
            // A PDF says its page (and a page still rendering) on the label row.
            if let Some(p) = pb.preview_rich.as_ref().filter(|p| p.kind == "pdf") {
                let status = format!(
                    " PDF · {}/{}{} ",
                    p.page,
                    p.pages.map_or("?".into(), |n| n.to_string()),
                    pb.preview_pending_page
                        .map(|n| format!(" · {} page {n}", if ascii() { "~" } else { "◌" }))
                        .unwrap_or_default()
                );
                let w = (status.chars().count() as u16).min(r.width);
                let row = Rect::new(r.right() - w, r.y.saturating_sub(1), w, 1);
                frame.render_widget(Paragraph::new(Span::styled(status, muted())), row);
            }
        }
    }
    // The design shows the highlighted file inline from 100 columns (Size M).
    if let Some(b) = app.active_browser().filter(|_| app.view == View::Files) {
        b.peek_shown.set(b.preview.is_none() && area.width >= 100);
    }
    if app.view == View::Containers && vm.view == views::LabView::Hub {
        frame.render_widget(Clear, body);
        hitmap::begin();
        render_containers(frame, app, body);
    }
    if app.view == View::Files {
        file_matches(frame, app, &drawn.targets);
    }
    destination_bar(frame, app, area);
    find_bar(frame, app, area);
    if app.pane_focused() {
        typing_bar(frame, area);
    }
    if let Some(query) = &vm.palette {
        hitmap::begin();
        for t in
            views::mission::render_palette(&vm.state, &vm.caps, area, frame.buffer_mut(), query)
        {
            hitmap::register_action(t.area, &t.action, &files);
        }
    }
    let workspace = body;
    let phone = area.width < 70;
    let _ = phone;
    // Overlays from here on: they own the focused treatment.
    set_panels_dimmed(false);
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
        if input != Input::Palette {
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
                    ChooseDevice::New => "New session · execution device",
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
                vec!["SSH address".into(), "Access · edit the link policy".into(), "Remove device".into(), "Cancel".into()],
                format!("Current target: {}\nEdit the SSH alias or user@host. The device identity and posture stay unchanged.", safe_label(app.devices[*d].target.as_deref().unwrap_or("unknown"))),
            ),
            Dialog::Reset => (
                "Reset managed sessions?".into(),
                vec!["Cancel · keep running sessions".into(), "Reset tmux and agents".into()],
                "This forcefully terminates all CX-managed tmux sessions on this execution device, including Claude and Codex processes running inside them. External tmux sessions are untouched.\nEnter confirms · Escape cancels".into(),
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
            Dialog::Delete(..) | Dialog::RemoveDevice(..) | Dialog::Reset
        ) {
            let labels = if matches!(dialog, Dialog::RemoveDevice(..)) {
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
                    .border_set(overlay_border())
                    .border_style(err())
                    .title(Line::from(Span::styled(
                        format!(" {title} "),
                        err().add_modifier(Modifier::BOLD),
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
            let selection_style = selection(!app.dialog_detail_focus);
            if matches!(dialog, Dialog::Jobs) {
                let progress_width = if parts[0].width < 60 { 10 } else { 18 };
                let file_width = parts[0].width.saturating_sub(progress_width + 15) as usize;
                let rows = app
                    .job_rows()
                    .into_iter()
                    .map(|(_, job)| {
                        let status_style = match job["status"].as_str() {
                            Some("failed" | "incomplete") => err(),
                            Some("complete") => ok(),
                            _ => warn(),
                        };
                        let operation = if job["operation"] == "move" {
                            "move"
                        } else {
                            "copy"
                        };
                        Row::new(vec![
                            Cell::from(Span::styled(
                                safe_label(job["status"].as_str().unwrap_or("unknown")),
                                status_style,
                            )),
                            Cell::from(Line::from(vec![
                                Span::styled(
                                    format!("{operation} "),
                                    if operation == "move" { warn() } else { info() },
                                ),
                                Span::raw(fit_path(
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
                    .block(block(title, !app.dialog_detail_focus))
                    .row_highlight_style(selection_style),
                    parts[0],
                    &mut state,
                );
            } else {
                let mut state =
                    ratatui::widgets::ListState::default().with_selected(Some(app.dialog_selected));
                frame.render_stateful_widget(
                    List::new(labels.into_iter().map(ListItem::new).collect::<Vec<_>>())
                        .block(block(title, !app.dialog_detail_focus))
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
    hitmap::render_context_menu(frame, app, area);
}
/// Key that returns from a managed session. The sessions wave moves this to
/// Ctrl+\ together with the tmux binding; help reads it from here.
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
                let state = if c.state == "running" { ok() } else { warn() };
                ListItem::new(Line::from(vec![
                    Span::raw(if ascii() { "  |- " } else { "  ├─ " }),
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
                        state,
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
            .highlight_style(selection(app.focus == Focus::Workspace)),
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
/// Files search: matching characters underlined (a non-colour cue), or the whole
/// visible name when every match position is cut off.
fn file_matches(frame: &mut Frame<'_>, app: &App, targets: &[views::LabTarget]) {
    let Some(b) = app.active_browser().filter(|b| !b.search.is_empty()) else {
        return;
    };
    let matches = browser_matches(b);
    let rows = browser_entries(b);
    let buf = frame.buffer_mut();
    for t in targets {
        let Some(name) = t.action.strip_prefix("file:") else {
            continue;
        };
        let Some((_, positions)) = matches
            .iter()
            .find(|(row, _)| rows.get(*row).is_some_and(|e| safe_label(&e.name) == name))
        else {
            continue;
        };
        // The name starts after the selection bar and the fold/type glyph.
        let mut cells = Vec::new();
        let mut x = t.area.x + 3;
        for c in name.chars() {
            if x >= t.area.right() {
                break;
            }
            cells.push(x);
            x += Span::raw(c.to_string()).width().max(1) as u16;
        }
        // The last visible cell may be the cut's ellipsis.
        let shown = cells.len().saturating_sub(1);
        let visible: Vec<u16> = positions
            .iter()
            .filter(|p| **p < shown)
            .map(|p| cells[*p])
            .collect();
        let underline = if visible.is_empty() {
            cells[..shown].to_vec()
        } else {
            visible
        };
        for x in underline {
            buf[(x, t.area.y)].modifier.insert(Modifier::UNDERLINED);
        }
    }
}
/// The find / filter field on the footer row: while it is typed, and while a
/// search stays active (its match position and `n/N`).
fn find_bar(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let b = app.browser.as_ref().filter(|_| app.view == View::Files);
    let typing = app
        .input
        .filter(|i| matches!(i, Input::Search | Input::PreviewSearch | Input::Filter));
    let preview = b.is_some_and(|b| b.preview.is_some());
    let (key, text, label) = match (typing, b) {
        (Some(Input::Filter), _) => (" f ", app.text.clone(), "Filter".to_string()),
        (Some(Input::PreviewSearch), Some(b)) | (None, Some(b))
            if preview && (typing.is_some() || !b.preview_find.query.is_empty()) =>
        {
            let count = doc_matches(b, &b.preview_find.query).len();
            let at = if count == 0 {
                0
            } else {
                (b.preview_find.selected + 1).min(count)
            };
            let query = if typing.is_some() {
                app.text.clone()
            } else {
                b.preview_find.query.clone()
            };
            (" / ", query, format!("Search · {at}/{count}"))
        }
        (Some(_), Some(b)) | (None, Some(b)) if typing.is_some() || !b.search.is_empty() => {
            let matches = browser_matches(b);
            let at = matches
                .iter()
                .position(|(row, _)| *row == b.selected)
                .map_or(0, |i| i + 1);
            let query = if typing.is_some() {
                app.text.clone()
            } else {
                b.search.clone()
            };
            (
                " / ",
                query,
                format!("Search · {at}/{} · n/N matches", matches.len()),
            )
        }
        (Some(_), _) => (" / ", app.text.clone(), "Search".to_string()),
        _ => return,
    };
    let row = Rect::new(area.x, area.bottom().saturating_sub(1), area.width, 1);
    frame.render_widget(Clear, row);
    let text = safe_label(&text);
    let width = Span::raw(text.as_str()).width() as u16;
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(key, accent().add_modifier(Modifier::BOLD)),
            Span::raw(" "),
            Span::raw(text),
        ])),
        row,
    );
    let lw = Span::raw(label.as_str()).width() as u16;
    if row.width > width + lw + 6 {
        frame.render_widget(
            Paragraph::new(Span::styled(label, muted())),
            Rect::new(row.right() - lw - 1, row.y, lw, 1),
        );
    }
    if typing.is_some() {
        frame.set_cursor_position((row.x + 4 + width.min(row.width.saturating_sub(5)), row.y));
    }
}
/// Typing into a peeked session: the footer says how to leave (`ctrl+\`).
fn typing_bar(frame: &mut Frame<'_>, area: Rect) {
    let row = Rect::new(area.x, area.bottom().saturating_sub(1), area.width, 1);
    frame.render_widget(Clear, row);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::raw(" "),
            Span::styled(
                " ctrl+\\ ",
                Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD),
            ),
            Span::styled(" detach  typing into the session", muted()),
        ])),
        row,
    );
}
/// Choosing a transfer's destination folder: the footer says what Enter does here.
fn destination_bar(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let Some(c) = app.clipboard.as_ref().filter(|_| {
        app.view == View::Files
            && app.destination_active
            && app.input.is_none()
            && app.dialog.is_none()
    }) else {
        return;
    };
    let row = Rect::new(area.x, area.bottom().saturating_sub(1), area.width, 1);
    let key = Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD);
    let n = c.entries.len();
    frame.render_widget(Clear, row);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::raw(" "),
            Span::styled(if ascii() { " ret " } else { " ⏎ " }, key),
            Span::styled(" transfer here  ", muted()),
            Span::styled(" esc ", key),
            Span::styled(" cancel  ", muted()),
            Span::styled(
                format!(
                    "{n} item{} from {}",
                    if n == 1 { "" } else { "s" },
                    safe_label(&c.source_label)
                ),
                muted(),
            ),
        ])),
        row,
    );
}
