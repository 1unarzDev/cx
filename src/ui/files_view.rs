//! File browser: navigation/selection state, transfer submission, `render_browser` and drawer labels.
use super::*;

pub(super) fn browser_entries(b: &Browser) -> Vec<Entry> {
    b.entries
        .iter()
        .filter(|e| {
            (b.show_hidden || !(e.hidden || e.name.starts_with('.')))
                && e.name.to_lowercase().contains(&b.filter.to_lowercase())
        })
        .cloned()
        .collect()
}
pub(super) fn file_search_matches(rows: &[Entry], query: &str) -> Vec<(usize, Vec<usize>)> {
    if query.is_empty() {
        return Vec::new();
    }
    let lines: Vec<_> = rows
        .iter()
        .map(|e| Line::raw(safe_label(&e.name)))
        .collect();
    let mut matches: Vec<(usize, Vec<usize>)> = Vec::new();
    for (row, positions) in preview_matches(&lines, query) {
        if let Some((_, last_positions)) =
            matches.last_mut().filter(|(last_row, _)| *last_row == row)
        {
            last_positions.extend(positions);
        } else {
            matches.push((row, positions));
        }
    }
    matches
}
pub(super) fn compact_path(path: &str, width: usize) -> String {
    let safe = safe_label(path);
    if Span::raw(&safe).width() <= width {
        return safe;
    }
    let mut tail = String::new();
    let mut used = 1;
    for c in safe.chars().rev() {
        let w = Span::raw(c.to_string()).width();
        if used + w > width {
            break;
        }
        tail.insert(0, c);
        used += w;
    }
    format!("…{tail}")
}
// Keep the selected context in traversal order; reserve the deepest two rows
// before showing ancestors so narrow/short panels never hide the selected item.
pub(super) fn file_context_hierarchy(
    device: &str,
    path: &str,
    entry: Option<&Entry>,
    width: usize,
    height: usize,
) -> Vec<Line<'static>> {
    let safe = safe_label(path);
    let mut parts: Vec<&str> = safe.split('/').filter(|p| !p.is_empty()).collect();
    let folder = parts.pop().unwrap_or("/");
    let required = 2 + usize::from(entry.is_some());
    let parent_count = height.saturating_sub(required).min(parts.len());
    let skipped = parts.len().saturating_sub(parent_count);
    let mut lines = vec![Line::styled(
        fit_label(device, width),
        accent().add_modifier(Modifier::BOLD),
    )];
    let branch = if width < 16 {
        if ascii() {
            ">"
        } else {
            "›"
        }
    } else if ascii() {
        "+- "
    } else {
        "└─ "
    };
    if parent_count > 0 {
        for (depth, parent) in parts.iter().skip(skipped).enumerate() {
            let prefix = if depth == 0 && skipped > 0 {
                if ascii() {
                    ".../"
                } else {
                    "…/"
                }
            } else {
                ""
            };
            let indent = " ".repeat(depth.min(if width < 16 { 1 } else { 3 }));
            lines.push(Line::styled(
                fit_label(&format!("{indent}{prefix}{parent}/"), width),
                muted(),
            ));
        }
    }
    let indent = " ".repeat(if width < 16 { 0 } else { parent_count.min(3) });
    lines.push(Line::styled(
        fit_label(
            &format!(
                "{indent}{branch}{folder}{}",
                if folder == "/" { "" } else { "/" }
            ),
            width,
        ),
        accent().add_modifier(Modifier::BOLD | Modifier::REVERSED),
    ));
    if let Some(entry) = entry {
        lines.push(Line::styled(
            fit_label(
                &format!(
                    "{indent}{}{branch}{}{}",
                    if width < 16 { " " } else { "  " },
                    safe_label(&entry.name),
                    if entry.kind == "directory" { "/" } else { "" }
                ),
                width,
            ),
            Style::default().add_modifier(Modifier::BOLD),
        ));
    }
    lines
}
pub(super) fn render_browser(
    frame: &mut Frame<'_>,
    b: &Browser,
    device: &Device,
    area: Rect,
    focused: bool,
    label: &str,
    clipboard: Option<&Clipboard>,
    agents: &BTreeMap<String, String>,
) {
    let parts = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(area);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                format!(
                    " {}  ",
                    b.container
                        .as_ref()
                        .map(|c| format!("Container · {}", safe_label(&c.name)))
                        .unwrap_or_else(|| label.into())
                ),
                accent().add_modifier(Modifier::BOLD),
            ),
            Span::styled(identity(device), muted()),
        ])),
        parts[0],
    );
    frame.render_widget(
        Paragraph::new(if b.container.is_some() {
            vec![Line::from(vec![
                Span::styled(" READ-ONLY ", accent().add_modifier(Modifier::BOLD)),
                Span::styled(
                    format!(
                        "hidden {}{}",
                        if b.show_hidden { "shown" } else { "off" },
                        if b.loading { " · loading" } else { "" }
                    ),
                    muted(),
                ),
            ])]
        } else {
            vec![Line::from(vec![
                Span::styled(
                    format!(
                        " {} ",
                        if b.visual_anchor.is_some() {
                            "VISUAL"
                        } else {
                            "NORMAL"
                        }
                    ),
                    (if b.visual_anchor.is_some() {
                        tok(Tok::Code)
                    } else {
                        ok()
                    })
                    .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("{} selected{}", b.marked.len(), {
                        let shown = browser_entries(b)
                            .iter()
                            .filter(|e| b.marked.contains(&e.path))
                            .count();
                        if shown < b.marked.len() {
                            format!(" ({shown} shown)")
                        } else {
                            String::new()
                        }
                    }),
                    if b.marked.is_empty() { muted() } else { warn() },
                ),
                Span::styled(
                    format!(
                        " · hidden {}{}",
                        if b.show_hidden { "shown" } else { "off" },
                        if b.loading { " · loading" } else { "" }
                    ),
                    muted(),
                ),
            ])]
        }),
        parts[1],
    );
    if let Some(preview) = &b.preview {
        render_preview(frame, parts[2], b, preview, focused);
    } else {
        let rows = browser_entries(b);
        if rows.is_empty() {
            frame.render_widget(
                Paragraph::new(if b.loading {
                    " Loading…"
                } else if !b.search.is_empty() || !b.filter.is_empty() {
                    " No matches · Escape clears filter"
                } else {
                    " Empty folder · . shows hidden files"
                })
                .style(muted())
                .block(block("Files".into(), focused)),
                parts[2],
            );
        } else {
            let matches = file_search_matches(&rows, &b.search);
            let matched_rows: BTreeSet<_> = matches.iter().map(|(row, _)| *row).collect();
            let mut names: Vec<_> = rows
                .iter()
                .enumerate()
                .map(|(index, e)| {
                    let style = if index == b.selected && focused {
                        accent().add_modifier(Modifier::BOLD)
                    } else if index == b.selected {
                        Style::default().add_modifier(Modifier::BOLD)
                    } else if matched_rows.contains(&index) {
                        tok(Tok::Match).add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
                    } else if e.kind == "directory" {
                        dir()
                    } else {
                        Style::default()
                    };
                    Line::from(Span::styled(
                        if b.search.is_empty() {
                            compact_path(&e.name, area.width.saturating_sub(18) as usize)
                        } else {
                            safe_label(&e.name)
                        },
                        style,
                    ))
                })
                .collect();
            let selected_match = matches
                .iter()
                .position(|(i, _)| *i == b.selected)
                .unwrap_or(usize::MAX);
            highlight_preview_matches(
                &mut names,
                &PreviewFind {
                    query: b.search.clone(),
                    matches,
                    selected: selected_match,
                    reveal: std::cell::Cell::new(false),
                },
            );
            let table_rows = rows
                .iter()
                .enumerate()
                .map(|(index, e)| {
                    let clip = clipboard
                        .filter(|c| c.container == b.container)
                        .filter(|c| {
                            c.device == b.device && c.entries.iter().any(|x| x.path == e.path)
                        });
                    let (marker, style) = if b.marked.contains(&e.path) {
                        (if ascii() { "*" } else { "●" }, warn())
                    } else if let Some(c) = clip {
                        (
                            if c.cut { "x" } else { "c" },
                            if c.cut { warn() } else { info() },
                        )
                    } else {
                        (" ", Style::default())
                    };
                    let selected = index == b.selected;
                    let name_style = if selected && focused {
                        accent().add_modifier(Modifier::BOLD)
                    } else if selected {
                        Style::default().add_modifier(Modifier::BOLD)
                    } else if e.kind == "directory" {
                        dir()
                    } else {
                        Style::default()
                    };
                    let cursor = if selected {
                        selection_symbol(focused)
                    } else {
                        "  "
                    };
                    Row::new(vec![
                        Cell::from(marker).style(style.add_modifier(Modifier::BOLD)),
                        Cell::from(match e.kind.as_str() {
                            "directory" => "/",
                            "symlink" => "@",
                            _ => " ",
                        })
                        .style(accent()),
                        // Keep the cursor attached to its filename instead of a remote rail.
                        // ANSI foreground only: preserve light/dark defaults and transparency.
                        Cell::from(Line::from(
                            vec![Span::styled(
                                cursor,
                                if focused { name_style } else { muted() },
                            )]
                            .into_iter()
                            .chain(names[index].spans.clone())
                            .collect::<Vec<_>>(),
                        )),
                        Cell::from(if e.kind == "file" {
                            human_size(e.size)
                        } else {
                            agents.get(&e.path).cloned().unwrap_or_default()
                        })
                        .style(if selected && focused {
                            Style::default()
                        } else {
                            muted()
                        }),
                    ])
                })
                .collect::<Vec<_>>();
            let mut state = TableState::default().with_selected(Some(b.selected));
            frame.render_stateful_widget(
                Table::new(
                    table_rows,
                    [
                        Constraint::Length(1),
                        Constraint::Length(1),
                        Constraint::Min(1),
                        Constraint::Length(
                            agents
                                .values()
                                .map(|s| s.len() as u16)
                                .max()
                                .unwrap_or(8)
                                .clamp(8, 24)
                                .min(area.width / 3),
                        ),
                    ],
                )
                .column_spacing(1)
                .block(block(format!("{} items", rows.len()), focused))
                .row_highlight_style(selection(focused)),
                parts[2],
                &mut state,
            );
        }
    }
    let bottom = if b.preview.is_some() {
        " Preview · j/k scroll · Escape back".into()
    } else if label.starts_with("Destination") && clipboard.is_some() {
        " Enter transfer here · h/l folders".into()
    } else if let Some(c) = clipboard {
        format!(
            " {} {} · {}{}",
            if c.cut { "CUT" } else { "COPY" },
            c.entries.len(),
            safe_label(&c.source_label),
            if b.filter.is_empty() {
                " · p paste here".into()
            } else {
                format!(" · f {}", safe_label(&b.filter))
            }
        )
    } else if !b.search.is_empty() {
        let matches = file_search_matches(&browser_entries(b), &b.search);
        let current = matches
            .iter()
            .position(|(i, _)| *i == b.selected)
            .map(|i| i + 1)
            .unwrap_or(0);
        format!(
            " / {} · {}/{} · n/N matches{}",
            safe_label(&b.search),
            current,
            matches.len(),
            if b.filter.is_empty() {
                String::new()
            } else {
                format!(" · f {}", safe_label(&b.filter))
            }
        )
    } else if !b.filter.is_empty() {
        format!(" f Filter: {}", safe_label(&b.filter))
    } else {
        if b.container.is_some() {
            " Read-only · . hidden · f filter · n shell".into()
        } else {
            " Space select · v range · . hidden".into()
        }
    };
    frame.render_widget(
        Paragraph::new(bottom).style(if clipboard.is_some() { warn() } else { muted() }),
        parts[3],
    );
}
pub(super) fn human_size(size: u64) -> String {
    if size < 1024 {
        format!("{size} B")
    } else if size < 1024 * 1024 {
        format!("{:.1} KiB", size as f64 / 1024.0)
    } else if size < 1024 * 1024 * 1024 {
        format!("{:.1} MiB", size as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.1} GiB", size as f64 / (1024.0 * 1024.0 * 1024.0))
    }
}
pub(super) fn transfer_spinner(frame: usize, plain: bool) -> &'static str {
    let frames: &[&str] = if plain {
        &["|", "/", "-", "\\"]
    } else {
        &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]
    };
    frames[frame % frames.len()]
}
pub(super) fn transfer_name(job: &Value) -> String {
    let path = job["source_display"]
        .as_str()
        .or(job["source_path"].as_str())
        .unwrap_or("file");
    safe_label(path.rsplit('/').find(|s| !s.is_empty()).unwrap_or(path))
}
pub(super) fn transfer_status(job: &Value) -> String {
    let state = safe_label(job["status"].as_str().unwrap_or("unknown"));
    let skipped = job["skipped"].as_u64().unwrap_or(0);
    if state == "complete" && skipped > 0 {
        format!("complete · {skipped} skipped")
    } else {
        state
    }
}
pub(super) fn transfer_destination(job: &Value) -> String {
    let path = job["actual_destinations"]
        .as_array()
        .and_then(|a| a.first())
        .and_then(Value::as_str)
        .or(job["destination_display"].as_str())
        .or(job["destination_path"].as_str())
        .unwrap_or("");
    match crate::files::decode_path(path) {
        Ok(p) => safe_label(&crate::files::display(&p.to_string_lossy())),
        Err(_) => safe_label(path),
    }
}

impl App {
    pub(super) fn visible_entries(&self) -> Vec<Entry> {
        self.browser
            .as_ref()
            .map(browser_entries)
            .unwrap_or_default()
    }
    pub(super) fn chosen_entries(&self) -> Vec<Entry> {
        let Some(b) = &self.browser else {
            return vec![];
        };
        if b.marked.is_empty() {
            browser_entries(b)
                .get(b.selected)
                .cloned()
                .into_iter()
                .collect()
        } else {
            // Marks are path identities, independent of hidden/filter state.
            b.entries
                .iter()
                .filter(|e| b.marked.contains(&e.path))
                .cloned()
                .collect()
        }
    }
    pub(super) fn queue_file_actions(&mut self, actions: Vec<(usize, Operation)>) {
        if !self.file_busy && self.file_queue.is_empty() {
            self.file_errors.clear();
        }
        for (_, op) in &actions {
            if let Operation::TransferRetry { key } = op {
                self.watched_jobs.insert(key.clone());
            }
        }
        let actions = actions
            .into_iter()
            .map(|(d, op)| (d, self.scoped_file_operation(d, op)))
            .collect::<Vec<_>>();
        self.file_queue.extend(actions);
        self.start_next_file_action();
    }
    pub(super) fn start_next_file_action(&mut self) {
        if !self.file_busy {
            if let Some((d, op)) = self.file_queue.front().cloned() {
                if self.send_explicit(d, op) {
                    self.file_queue.pop_front();
                    self.file_busy = true;
                } else {
                    self.set_notice_as(
                        NoticeKind::Warning,
                        "Request queue busy · file action remains pending".into(),
                    );
                }
            }
        }
    }
    pub(super) fn finish_visual(&mut self) {
        if let Some(b) = &mut self.browser {
            b.visual_anchor = None;
            b.visual_base.clear();
        }
    }
    pub(super) fn live_search(&mut self) {
        if self.view == View::Files {
            if let Some(b) = &mut self.browser {
                b.search = self.text.clone();
                b.restore_selection = None;
                let rows = browser_entries(b);
                if !b.search.is_empty() {
                    let matches = file_search_matches(&rows, &b.search);
                    if let Some(index) = matches
                        .iter()
                        .map(|(i, _)| *i)
                        .find(|i| *i >= b.selected)
                        .or_else(|| matches.first().map(|(i, _)| *i))
                    {
                        b.selected = index;
                    }
                }
            }
        } else {
            self.search = self.text.clone();
            self.selected = 0;
            self.container_selected = 0;
        }
    }
    pub(super) fn next_file_match(&mut self, direction: isize) {
        if let Some(b) = &mut self.browser {
            let matches = file_search_matches(&browser_entries(b), &b.search);
            let index = if direction > 0 {
                matches
                    .iter()
                    .find(|(i, _)| *i > b.selected)
                    .or_else(|| matches.first())
            } else {
                matches
                    .iter()
                    .rev()
                    .find(|(i, _)| *i < b.selected)
                    .or_else(|| matches.last())
            };
            if let Some((index, _)) = index {
                b.selected = *index;
                b.restore_selection = None;
            }
        }
    }
    pub(super) fn open_browser(&mut self, device: usize, path: String) {
        let scope = self
            .browser
            .as_ref()
            .filter(|b| self.view == View::Files && b.device == device)
            .and_then(|b| b.container.clone());
        self.open_scoped_browser(device, path, scope);
    }
    pub(super) fn open_host_browser(&mut self, device: usize, path: String) {
        self.open_scoped_browser(device, path, None);
    }
    pub(super) fn open_container_browser(&mut self, device: usize, scope: ContainerScope) {
        self.other_browser = None;
        self.destination_active = false;
        let path = self
            .container_locations
            .get(&(device, scope.id.clone()))
            .cloned()
            .unwrap_or_else(|| scope.folder.clone());
        self.open_scoped_browser(device, path, Some(scope));
    }
    pub(super) fn open_scoped_browser(
        &mut self,
        device: usize,
        path: String,
        scope: Option<ContainerScope>,
    ) {
        self.device = device + 1;
        let show_hidden = self
            .browser
            .as_ref()
            .or(self.other_browser.as_ref())
            .is_some_and(|b| b.show_hidden);
        self.finish_visual();
        self.generation += 1;
        if let Some(mut old) = self.browser.take() {
            old.preview = None;
            old.preview_rich = None;
            old.preview_history.clear();
            old.preview_restore = None;
            old.preview_anchor = None;
            old.preview_link_cells.borrow_mut().clear();
            if let Some(scope) = &old.container {
                self.container_locations
                    .insert((old.device, scope.id.clone()), old.path.clone());
            } else {
                self.file_locations.insert(old.device, old.path.clone());
            }
            if self.browser_cache.len() >= 64 {
                if let Some(key) = self.browser_cache.keys().next().cloned() {
                    self.browser_cache.remove(&key);
                }
            }
            self.browser_cache.insert(
                (
                    old.device,
                    old.path.clone(),
                    browser_scope_key(old.container.as_ref()),
                ),
                old,
            );
        }
        self.browser = Some(
            self.browser_cache
                .remove(&(device, path.clone(), browser_scope_key(scope.as_ref())))
                .unwrap_or_else(|| Browser::new(device, path)),
        );
        if let Some(b) = &mut self.browser {
            b.show_hidden = show_hidden;
            b.container = scope;
        }
        self.view = View::Files;
        self.focus = Focus::Workspace;
        self.refresh_browser();
        self.check_providers(device);
    }
    pub(super) fn select_file_device(&mut self) {
        // Files always has one execution host; All devices returns to the local location.
        let device = self
            .device
            .checked_sub(1)
            .or_else(|| self.devices.iter().position(|d| d.target.is_none()));
        if let Some(device) = device {
            if self
                .browser
                .as_ref()
                .is_none_or(|b| b.device != device || b.container.is_some())
            {
                let path = self
                    .file_locations
                    .get(&device)
                    .cloned()
                    .unwrap_or_else(|| "~".into());
                self.open_host_browser(device, path);
            }
        }
        self.focus = Focus::Devices;
    }
    pub(super) fn refresh_browser(&mut self) {
        self.generation += 1;
        if let Some(b) = self.browser.as_mut() {
            if b.restore_selection.is_none() {
                b.restore_selection = browser_entries(b)
                    .get(b.selected)
                    .map(|entry| entry.path.clone());
            }
            b.loading = true;
            b.preview = None;
            b.preview_rich = None;
            b.preview_history.clear();
            b.preview_restore = None;
            b.preview_anchor = None;
            b.preview_link_cells.borrow_mut().clear();
            let device = b.device;
            let path = b.path.clone();
            if !self.send(device, Operation::List { path }) {
                if let Some(b) = &mut self.browser {
                    b.loading = false;
                }
                self.set_notice_as(
                    NoticeKind::Warning,
                    "Refresh queue busy · retry shortly".into(),
                );
            }
        }
    }
    pub(super) fn conflict_policy(&self) -> &'static str {
        ["skip", "overwrite", "rename"][self.conflict]
    }
    pub(super) fn active_transfer_keys(&self) -> BTreeSet<String> {
        let mut keys = self.pending_transfers.clone();
        for (_, job) in self.job_rows() {
            if matches!(
                job["status"].as_str(),
                Some("queued" | "running" | "submitting")
            ) {
                if let Some(key) = job["key"].as_str() {
                    keys.insert(key.into());
                }
            }
        }
        keys
    }
    pub(super) fn submit_transfer(&mut self) {
        let (Some(clip), Some(b)) = (self.clipboard.clone(), self.browser.as_ref()) else {
            return;
        };
        let Some(local) = self.devices.iter().position(|d| d.target.is_none()) else {
            return;
        };
        let destination = self.devices[b.device].clone();
        let destination_path = b.path.clone();
        let destination_container = b.container.clone();
        let active = self.active_transfer_keys();
        if self.submitted.iter().any(|(key, spec)| {
            active.contains(key)
                && spec.source.id == self.devices[clip.device].id
                && spec.destination.id == destination.id
                && spec.source_container == clip.container
                && spec.destination_container == destination_container
                && spec.destination_path == destination_path
                && clip.entries.iter().any(|e| e.path == spec.source_path)
        }) {
            self.set_notice_as(
                NoticeKind::Warning,
                "Transfer already pending · T shows progress".into(),
            );
            return;
        }
        let mut actions = Vec::new();
        for entry in &clip.entries {
            let spec = crate::model::TransferSpec {
                source_container: clip.container.clone(),
                destination_container: destination_container.clone(),
                source: self.devices[clip.device].clone(),
                source_path: entry.path.clone(),
                destination: destination.clone(),
                destination_path: destination_path.clone(),
                conflict: self.conflict_policy().into(),
                key: unique_key(),
                cut: clip.cut,
                source_identity: if clip.cut {
                    entry.identity.clone()
                } else {
                    None
                },
            };
            if self.submitted.len() >= 256 {
                if let Some(key) = self.submitted.keys().next().cloned() {
                    self.submitted.remove(&key);
                }
            }
            self.pending_transfers.insert(spec.key.clone());
            self.submitted.insert(spec.key.clone(), spec.clone());
            if clip.cut {
                self.submitted_clipboards
                    .insert(spec.key.clone(), clip.id.clone());
            }
            actions.push((local, spec.operation()));
        }
        self.set_notice(format!(
            "{} {} items · {} → {} · existing: {}",
            if clip.cut { "Moving" } else { "Copying" },
            clip.entries.len(),
            file_endpoint_label(&self.devices[clip.device], &clip.container),
            file_endpoint_label(&destination, &destination_container),
            self.conflict_policy()
        ));
        self.queue_file_actions(actions);
        self.transfer_drawer = true;
    }
    pub(super) fn job_rows(&self) -> Vec<(usize, Value)> {
        let mut unique = HashMap::new();
        for (d, value) in &self.jobs {
            if let Some(jobs) = value["jobs"].as_array() {
                for job in jobs {
                    let key = (
                        job["key"].as_str().unwrap_or("").to_string(),
                        job["source_host"].as_str().unwrap_or("").to_string(),
                        job["destination_host"].as_str().unwrap_or("").to_string(),
                    );
                    let row = unique.entry(key).or_insert((*d, job.clone()));
                    // Prefer the viewer's durable forwarding reference, which knows job ownership.
                    if (self.devices[*d].target.is_none() && self.devices[row.0].target.is_some())
                        || (self.devices[*d].target.is_none()
                            == self.devices[row.0].target.is_none()
                            && *d < row.0)
                    {
                        *row = (*d, job.clone());
                    }
                }
            }
        }
        let mut rows = unique
            .into_values()
            .filter(|(_, job)| {
                fuzzy_score(
                    &self.job_filter,
                    &format!(
                        "{} {} {} {} {} {}",
                        transfer_name(job),
                        transfer_status(job),
                        job["source_host"].as_str().unwrap_or(""),
                        job["destination_host"].as_str().unwrap_or(""),
                        job["source_display"]
                            .as_str()
                            .or(job["source_path"].as_str())
                            .unwrap_or(""),
                        transfer_destination(job)
                    ),
                )
                .is_some()
            })
            .collect::<Vec<_>>();
        rows.sort_by(|(a, x), (b, y)| {
            y["updated"]
                .as_u64()
                .cmp(&x["updated"].as_u64())
                .then_with(|| a.cmp(b))
                .then_with(|| x["key"].as_str().cmp(&y["key"].as_str()))
        });
        rows
    }
    pub(super) fn parent_directory(&mut self) {
        if let Some(b) = &self.browser {
            if let Some(path) = b.parent.clone() {
                self.open_browser(b.device, path);
            }
        }
    }
}
