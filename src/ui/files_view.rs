//! File browser rendering and the transfer drawer labels.
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
                    tint(if b.visual_anchor.is_some() {
                        Color::Magenta
                    } else {
                        Color::Green
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
                    if b.marked.is_empty() {
                        muted()
                    } else {
                        tint(Color::Yellow)
                    },
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
                        tint(Color::Yellow).add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
                    } else if e.kind == "directory" {
                        tint(Color::Blue).add_modifier(Modifier::BOLD)
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
                        (if ascii() { "*" } else { "●" }, tint(Color::Yellow))
                    } else if let Some(c) = clip {
                        (
                            if c.cut { "x" } else { "c" },
                            tint(if c.cut { Color::Magenta } else { Color::Green }),
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
                        tint(Color::Blue).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default()
                    };
                    let cursor = if selected {
                        if ascii() {
                            "> "
                        } else {
                            "▸ "
                        }
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
                .row_highlight_style(Style::default()),
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
        Paragraph::new(bottom).style(if clipboard.is_some() {
            tint(Color::Yellow)
        } else {
            muted()
        }),
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
