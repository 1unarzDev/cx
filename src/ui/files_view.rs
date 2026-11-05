//! File browser: navigation/selection state, transfer submission, `render_browser` and drawer labels.
use super::*;

/// Visible (hidden/filter-applied) rows; cached in the browser's index, so this is
/// cheap to call per frame or per key.
pub(super) fn browser_entries(b: &Browser) -> VisibleRows {
    b.index
        .borrow_mut()
        .visible(&b.entries, b.show_hidden, &b.filter)
}
/// Search matches over the visible rows (one entry per row), cached per query.
pub(super) fn browser_matches(b: &Browser) -> file_index::RowMatches {
    b.index
        .borrow_mut()
        .search(&b.entries, b.show_hidden, &b.filter, &b.search)
}
#[cfg(test)]
pub(super) fn file_search_matches(rows: &VisibleRows, query: &str) -> Vec<(usize, Vec<usize>)> {
    if query.is_empty() {
        return Vec::new();
    }
    let needle: Vec<char> = query.chars().flat_map(char::to_lowercase).collect();
    let mut matches: Vec<(usize, Vec<usize>)> = Vec::new();
    for (row, entry) in rows.iter().enumerate() {
        for positions in line_matches(safe_label(&entry.name).chars(), &needle) {
            if let Some((_, last)) = matches.last_mut().filter(|(last, _)| *last == row) {
                last.extend(positions);
            } else {
                matches.push((row, positions));
            }
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
/// How long a folder or file stays highlighted before its listing/preview is fetched.
const PREFETCH_DELAY: Duration = Duration::from_millis(150);

/// A click/tap target recorded by `render_browser`.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum FileHit {
    /// A visible list row (index into the visible rows).
    Row(usize),
    /// A breadcrumb segment: open this folder.
    Crumb(String),
    /// The inline preview: open the highlighted file.
    Peek,
}

/// File categories drive the type-tag hue and the icon.
#[derive(Clone, Copy, PartialEq)]
enum Category {
    Directory,
    Link,
    Code,
    Doc,
    Data,
    Archive,
    Image,
    Media,
    Other,
}
fn extension(name: &str) -> &str {
    match name.rfind('.') {
        Some(at) if at > 0 && at + 1 < name.len() => &name[at + 1..],
        _ => "",
    }
}
fn category(entry: &Entry) -> Category {
    match entry.kind.as_str() {
        "directory" => return Category::Directory,
        "symlink" => return Category::Link,
        _ => {}
    }
    match extension(&entry.name).to_ascii_lowercase().as_str() {
        "rs" | "py" | "sh" | "bash" | "zsh" | "fish" | "js" | "mjs" | "ts" | "tsx" | "jsx"
        | "c" | "h" | "cc" | "cpp" | "hpp" | "go" | "java" | "kt" | "rb" | "php" | "pl" | "lua"
        | "swift" | "cs" | "html" | "css" | "scss" | "sql" | "vim" | "el" | "zig" | "hs" | "ml"
        | "ex" | "exs" | "r" | "cmake" | "mk" => Category::Code,
        "md" | "markdown" | "txt" | "rst" | "org" | "tex" | "pdf" | "adoc" | "log" => Category::Doc,
        "json" | "toml" | "yaml" | "yml" | "csv" | "tsv" | "xml" | "ini" | "cfg" | "conf"
        | "lock" | "db" | "sqlite" | "env" | "parquet" | "ndjson" | "jsonl" => Category::Data,
        "zip" | "tar" | "gz" | "tgz" | "xz" | "bz2" | "7z" | "zst" | "rar" | "deb" | "rpm"
        | "whl" | "jar" | "iso" => Category::Archive,
        "png" | "jpg" | "jpeg" | "gif" | "svg" | "webp" | "bmp" | "ico" | "tif" | "tiff" => {
            Category::Image
        }
        "mp3" | "wav" | "flac" | "ogg" | "m4a" | "mp4" | "mkv" | "mov" | "webm" | "avi" => {
            Category::Media
        }
        _ if matches!(entry.name.as_str(), "Makefile" | "Dockerfile" | "Justfile") => {
            Category::Code
        }
        _ => Category::Other,
    }
}
fn category_style(category: Category) -> Style {
    match category {
        Category::Directory => dir_style(),
        Category::Link => muted(),
        Category::Code => tint(Color::LightGreen),
        Category::Doc => tint(Color::LightYellow),
        Category::Data => tint(Color::LightCyan),
        Category::Archive => tint(Color::LightRed),
        Category::Image | Category::Media => tint(Color::LightMagenta),
        Category::Other => muted(),
    }
}
/// Directory names: bright blue + bold (plain blue fails contrast on dark backgrounds).
fn dir_style() -> Style {
    tint(Color::LightBlue).add_modifier(Modifier::BOLD)
}
/// Nerd Font icons appear only with the `CX_ICONS=1` escape hatch (wrong guesses are tofu).
fn nerd_icons() -> bool {
    !ascii() && std::env::var("CX_ICONS").is_ok_and(|v| v == "1")
}
fn icon(category: Category) -> &'static str {
    if ascii() {
        return match category {
            Category::Link => "@",
            _ => " ",
        };
    }
    if nerd_icons() {
        return match category {
            Category::Directory => "\u{f07b}",
            Category::Link => "\u{f0c1}",
            Category::Code => "\u{f121}",
            Category::Doc => "\u{f15c}",
            Category::Data => "\u{f1c0}",
            Category::Archive => "\u{f1c6}",
            Category::Image => "\u{f1c5}",
            Category::Media => "\u{f1c8}",
            Category::Other => "\u{f15b}",
        };
    }
    match category {
        Category::Directory => "▸",
        Category::Link => "↪",
        Category::Code => "λ",
        Category::Doc => "≡",
        Category::Data => "▤",
        Category::Archive => "⊞",
        Category::Image => "▣",
        Category::Media => "♪",
        Category::Other => "·",
    }
}
fn type_tag(entry: &Entry, category: Category) -> String {
    match category {
        Category::Directory => "dir".into(),
        Category::Link => "link".into(),
        _ => {
            let ext = extension(&entry.name).to_ascii_lowercase();
            if ext.is_empty() {
                "·".into()
            } else {
                ext.chars().take(4).collect()
            }
        }
    }
}
fn cell_width(text: &str) -> usize {
    Span::raw(text).width()
}
/// Cut `name` in the middle to `width` cells, keeping the extension and some tail.
pub(super) fn middle_ellipsis(name: &str, width: usize) -> String {
    if cell_width(name) <= width {
        return name.to_owned();
    }
    let mark = if ascii() { "~" } else { "…" };
    if width <= 1 {
        return mark.chars().take(width).collect();
    }
    let chars: Vec<char> = name.chars().collect();
    let ext = extension(name).chars().count();
    let tail_cells = (ext + 5).min((width - 1) * 2 / 3).max((width - 1) / 3);
    let mut tail = String::new();
    let mut used = 0;
    for c in chars.iter().rev() {
        let w = cell_width(&c.to_string());
        if used + w > tail_cells {
            break;
        }
        tail.insert(0, *c);
        used += w;
    }
    let mut head = String::new();
    let budget = width - 1 - used;
    let mut head_used = 0;
    for c in &chars {
        let w = cell_width(&c.to_string());
        if head_used + w > budget {
            break;
        }
        head.push(*c);
        head_used += w;
    }
    format!("{head}{mark}{tail}")
}
/// Compact relative age: now, 45s, 12m, 3h, 5d, 3w, 4mo, 2y. Unknown is `·`.
pub(super) fn relative_age(modified: Option<i64>, now: i64) -> String {
    let Some(modified) = modified else {
        return "·".into();
    };
    let age = (now - modified).max(0);
    match age {
        0..=9 => "now".into(),
        10..=59 => format!("{age}s"),
        60..=3599 => format!("{}m", age / 60),
        3600..=86_399 => format!("{}h", age / 3600),
        86_400..=1_209_599 => format!("{}d", age / 86_400),
        1_209_600..=5_183_999 => format!("{}w", age / 604_800),
        5_184_000..=31_535_999 => format!("{}mo", age / 2_592_000),
        _ => format!("{}y", age / 31_536_000),
    }
}
/// Compact size for the size column (≤ 8 cells).
fn short_size(size: u64) -> String {
    if size < 1024 {
        format!("{size} B")
    } else if size < 10 * 1024 * 1024 * 1024 * 1024 {
        let units = ["KiB", "MiB", "GiB", "TiB"];
        let mut value = size as f64 / 1024.0;
        let mut unit = 0;
        while value >= 1024.0 && unit + 1 < units.len() {
            value /= 1024.0;
            unit += 1;
        }
        if value < 10.0 {
            format!("{value:.1} {}", units[unit])
        } else {
            format!("{value:.0} {}", units[unit])
        }
    } else {
        human_size(size)
    }
}
/// One-cell size bar relative to the largest file in the listing.
fn size_bar(size: u64, largest: u64) -> &'static str {
    if size == 0 || largest == 0 {
        return " ";
    }
    let ratio = size as f64 / largest as f64;
    if ascii() {
        return if ratio >= 0.5 { "#" } else { "." };
    }
    const LEVELS: [&str; 8] = ["▏", "▎", "▍", "▌", "▋", "▊", "▉", "█"];
    LEVELS[((ratio * 8.0).ceil() as usize).clamp(1, 8) - 1]
}
fn light_theme() -> bool {
    std::env::var("CX_THEME").is_ok_and(|v| v == "light")
}
/// Selected-row background: `accent.bg` in the focused panel, `select.dim.bg` otherwise.
fn selection_style(focused: bool) -> Style {
    if no_color() {
        return if focused {
            Style::default().add_modifier(Modifier::REVERSED)
        } else {
            Style::default()
        };
    }
    Style::default().bg(Color::Indexed(match (focused, light_theme()) {
        (true, false) => 23,
        (true, true) => 195,
        (false, false) => 236,
        (false, true) => 255,
    }))
}
fn now_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// Breadcrumb segments for the panel title: (label, folder to open on click).
fn breadcrumb(b: &Browser, device: &Device) -> Vec<(String, Option<String>)> {
    let mut segments = vec![(safe_label(&device.name), None)];
    let display = if b.display_path.is_empty() {
        b.path.clone()
    } else {
        b.display_path.clone()
    };
    let home = if device.target.is_none() {
        std::env::var("HOME").ok()
    } else if device.account == "root" {
        Some("/root".into())
    } else {
        Some(format!("/home/{}", device.account))
    }
    .filter(|h| h.len() > 1);
    let decoded = crate::files::decode_path(&b.path)
        .ok()
        .filter(|p| p.is_absolute());
    let components: Vec<String> = display
        .split('/')
        .filter(|p| !p.is_empty())
        .map(safe_label)
        .collect();
    // Ancestors of the decoded path, deepest last, aligned with display components.
    let ancestors: Vec<String> = decoded
        .as_ref()
        .map(|p| {
            let mut list: Vec<String> = p
                .ancestors()
                .filter(|a| a.parent().is_some())
                .map(crate::files::encode_path)
                .collect();
            list.reverse();
            list
        })
        .unwrap_or_default();
    let aligned = ancestors.len() == components.len();
    let home_depth = home
        .as_ref()
        .filter(|h| display == **h || display.starts_with(&format!("{h}/")))
        .map(|h| h.split('/').filter(|p| !p.is_empty()).count());
    let root_target = decoded
        .as_ref()
        .map(|_| crate::files::encode_path(std::path::Path::new("/")));
    match home_depth {
        Some(depth) => {
            segments[0].1 = None;
            segments.push((
                "~".into(),
                aligned
                    .then(|| ancestors.get(depth.wrapping_sub(1)).cloned())
                    .flatten(),
            ));
            for (i, part) in components.iter().enumerate().skip(depth) {
                segments.push((part.clone(), aligned.then(|| ancestors[i].clone())));
            }
        }
        None => {
            segments.push(("/".into(), root_target));
            for (i, part) in components.iter().enumerate() {
                segments.push((part.clone(), aligned.then(|| ancestors[i].clone())));
            }
        }
    }
    // The current folder is not a link to itself.
    if segments.len() > 1 {
        if let Some(last) = segments.last_mut() {
            last.1 = None;
        }
    }
    segments
}
/// Join breadcrumb segments into at most `width` cells, collapsing middle segments
/// (keeping the device and the tail). Returns (text, [(start cell, label, target)]).
fn fit_breadcrumb(
    segments: &[(String, Option<String>)],
    width: usize,
) -> (String, Vec<(usize, String, Option<String>)>) {
    let sep = if ascii() { " > " } else { " › " };
    let ellipsis = if ascii() { "..." } else { "…" };
    let total = |parts: &[&(String, Option<String>)]| {
        parts.iter().map(|p| cell_width(&p.0)).sum::<usize>()
            + parts.len().saturating_sub(1) * cell_width(sep)
    };
    let all: Vec<&(String, Option<String>)> = segments.iter().collect();
    let mut shown: Vec<(String, Option<String>)> = all.iter().map(|p| (*p).clone()).collect();
    if total(&all) > width && segments.len() > 2 {
        // device › … › tail…; add tail segments while they fit.
        let mut tail: Vec<&(String, Option<String>)> = Vec::new();
        let fixed = cell_width(&segments[0].0) + cell_width(ellipsis) + 2 * cell_width(sep);
        let mut used = fixed;
        for part in segments[1..].iter().rev() {
            let cost = cell_width(&part.0) + if tail.is_empty() { 0 } else { cell_width(sep) };
            if used + cost > width && !tail.is_empty() {
                break;
            }
            used += cost;
            tail.insert(0, part);
        }
        shown = vec![segments[0].clone(), (ellipsis.to_owned(), None)];
        shown.extend(tail.into_iter().cloned());
        if shown.len() == segments.len() + 1 {
            shown.remove(1);
        }
        // Still too wide (phones): drop the device too, keeping `… › folder`.
        let refs: Vec<&(String, Option<String>)> = shown.iter().collect();
        if total(&refs) > width && shown.len() > 2 {
            let last = shown.last().cloned().unwrap();
            shown = vec![(ellipsis.to_owned(), None), last];
        }
    }
    let mut text = String::new();
    let mut spans = Vec::new();
    for (i, (label, target)) in shown.iter().enumerate() {
        if i > 0 {
            text.push_str(sep);
        }
        spans.push((cell_width(&text), label.clone(), target.clone()));
        text.push_str(label);
    }
    if cell_width(&text) > width {
        // Keep the deepest folder readable: cut it in the middle, never the separators.
        if let Some((start, label, _)) = spans.last().cloned() {
            let room = width.saturating_sub(start);
            if room >= 4 {
                let cut = middle_ellipsis(&label, room);
                text = format!("{}{cut}", &text[..text.len() - label.len()]);
                if let Some(last) = spans.last_mut() {
                    last.1 = cut;
                }
            }
        }
        if cell_width(&text) > width {
            text = middle_ellipsis(&text, width);
        }
        spans.retain(|(start, label, _)| start + cell_width(label) <= width);
    }
    (text, spans)
}
/// Find `needle` on buffer row `y` within [x0, x1); returns its first column.
fn find_on_row(frame: &mut Frame<'_>, y: u16, x0: u16, x1: u16, needle: &str) -> Option<u16> {
    let buffer = frame.buffer_mut();
    let cells: Vec<(u16, String)> = (x0..x1)
        .map(|x| (x, buffer[(x, y)].symbol().to_owned()))
        .collect();
    let first = needle.chars().next()?.to_string();
    for start in 0..cells.len() {
        if cells[start].1 != first {
            continue;
        }
        let mut text = String::new();
        for (_, symbol) in &cells[start..] {
            text.push_str(symbol);
            if text.len() >= needle.len() {
                break;
            }
        }
        if text.starts_with(needle) {
            return Some(cells[start].0);
        }
    }
    None
}

#[allow(clippy::too_many_arguments)]
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
    b.hits.borrow_mut().clear();
    if b.preview.is_none() && b.doc.borrow().is_some() {
        b.doc.borrow_mut().take();
    }
    let status = browser_status_line(b, label, clipboard);
    let parts = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(3),
            Constraint::Length(u16::from(status.is_some())),
        ])
        .split(area);
    if let Some((text, style)) = status {
        frame.render_widget(Paragraph::new(text).style(style), parts[1]);
    }
    if let Some(preview) = &b.preview {
        render_preview(frame, parts[0], b, preview, focused);
        return;
    }
    // Wide layouts split: list on the left, inline preview of the highlighted entry.
    let dual = label.starts_with("Source") || label.starts_with("Destination");
    let peek = parts[0].width >= 90 && parts[0].height >= 6 && !dual;
    b.peek_shown.set(peek);
    let (list_area, peek_area) = if peek {
        let columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
            .split(parts[0]);
        (columns[0], Some(columns[1]))
    } else {
        (parts[0], None)
    };
    render_list(
        frame, b, device, list_area, focused, label, clipboard, agents,
    );
    if let Some(peek_area) = peek_area {
        render_peek(frame, b, peek_area);
    }
}
/// The single state line under the list, shown only when something is non-default
/// (clipboard, search, filter, transfer destination or the preview's keys).
fn browser_status_line(
    b: &Browser,
    label: &str,
    clipboard: Option<&Clipboard>,
) -> Option<(String, Style)> {
    let text = if b.preview.is_some() {
        return None;
    } else if label.starts_with("Destination") && clipboard.is_some() {
        " Enter transfer here · h/l folders".to_owned()
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
        let matches = browser_matches(b);
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
        return None;
    };
    Some((
        text,
        if clipboard.is_some() {
            tint(Color::Yellow)
        } else {
            muted()
        },
    ))
}
#[allow(clippy::too_many_arguments)]
fn render_list(
    frame: &mut Frame<'_>,
    b: &Browser,
    device: &Device,
    area: Rect,
    focused: bool,
    label: &str,
    clipboard: Option<&Clipboard>,
    agents: &BTreeMap<String, String>,
) {
    // Title: optional context label, then the clickable breadcrumb.
    let prefix = if let Some(c) = &b.container {
        format!(
            "Container · {} · read-only{}",
            safe_label(&c.name),
            if ascii() { " > " } else { " › " }
        )
    } else if label == "Files" {
        String::new()
    } else {
        format!("{} · ", safe_label(label))
    };
    let room = usize::from(area.width).saturating_sub(8 + cell_width(&prefix));
    let (crumbs, segments) = fit_breadcrumb(&breadcrumb(b, device), room);
    let title = format!("{prefix}{crumbs}");
    let border = block(title.clone(), focused);
    let inner = border.inner(area);
    frame.render_widget(border, area);
    if let Some(x) = find_on_row(frame, area.y, area.x, area.right(), &title) {
        let base = x + cell_width(&prefix) as u16;
        let mut hits = b.hits.borrow_mut();
        for (start, text, target) in segments {
            if let Some(target) = target {
                let x = base + start as u16;
                let w = (cell_width(&text) as u16).min(area.right().saturating_sub(x));
                if w > 0 {
                    hits.push((Rect::new(x, area.y, w, 1), FileHit::Crumb(target)));
                }
            }
        }
    }
    let rows = browser_entries(b);
    let (largest, bytes, longest) = {
        let mut index = b.index.borrow_mut();
        (
            index.largest(&b.entries, b.show_hidden, &b.filter),
            index.bytes(),
            index.longest(),
        )
    };
    // Bottom border: counts and only the state that differs from the default.
    let mut state = vec![match b.total.filter(|t| *t as usize > b.entries.len()) {
        // Still paging: loaded/total.
        Some(total) => format!("{}/{} items", b.entries.len(), total),
        None if !b.filter.is_empty() => format!("{}/{} items", rows.len(), b.entries.len()),
        None => format!("{} items", rows.len()),
    }];
    if b.filter.is_empty() && !b.show_hidden && rows.len() < b.entries.len() {
        state.push(format!("{} hidden", b.entries.len() - rows.len()));
    }
    if bytes > 0 && area.width >= 50 {
        state.push(human_size(bytes));
    }
    if !b.marked.is_empty() {
        let shown = rows.iter().filter(|e| b.marked.contains(&e.path)).count();
        state.push(if shown < b.marked.len() {
            format!("{} marked ({shown} shown)", b.marked.len())
        } else {
            format!("{} marked", b.marked.len())
        });
    }
    if b.visual_anchor.is_some() {
        state.push("range".into());
    }
    if b.show_hidden {
        state.push("hidden shown".into());
    }
    if b.loading {
        state.push("loading".into());
    }
    let state = format!(" {} ", state.join(" · "));
    if area.height > 1 && area.width > 6 {
        let w = (cell_width(&state) as u16).min(area.width.saturating_sub(4));
        frame.render_widget(
            Paragraph::new(Span::styled(
                fit_label(&state, usize::from(w)),
                if focused { accent() } else { muted() },
            )),
            Rect::new(area.x + 2, area.bottom() - 1, w, 1),
        );
    }
    if rows.is_empty() {
        frame.render_widget(
            Paragraph::new(if b.loading {
                " Loading…"
            } else if !b.search.is_empty() || !b.filter.is_empty() {
                " No matches · Escape clears filter"
            } else {
                " Empty folder · . shows hidden files"
            })
            .style(muted()),
            inner,
        );
        return;
    }
    // Window: only the visible rows are built. Scrolloff keeps 2 rows of context.
    let height = usize::from(inner.height);
    if height == 0 || inner.width == 0 {
        return;
    }
    let selected = b.selected.min(rows.len() - 1);
    let scrolloff = 2.min(height.saturating_sub(1) / 2);
    let mut offset = b.list_offset.get().min(rows.len().saturating_sub(height));
    if selected < offset + scrolloff {
        offset = selected.saturating_sub(scrolloff);
    }
    if selected + scrolloff >= offset + height {
        offset = selected + scrolloff + 1 - height;
    }
    offset = offset.min(rows.len().saturating_sub(height));
    b.list_offset.set(offset);
    let matches = browser_matches(b);
    let first_match = matches.partition_point(|(row, _)| *row < offset);
    let selected_match = matches
        .iter()
        .position(|(row, _)| *row == selected)
        .unwrap_or(usize::MAX);
    let clip = clipboard
        .filter(|c| c.container == b.container && c.device == b.device)
        .map(|c| (c.cut, file_index::clipboard_paths(&c.entries)));
    // Columns by width: bar, mark, icon, name, [type], [size], [bar], [age].
    let width = usize::from(inner.width);
    let show_type = width >= 64;
    let show_size = width >= 26;
    let show_bar = width >= 72;
    let show_age = width >= 40;
    let agent_width = agents
        .values()
        .map(|s| cell_width(s))
        .max()
        .unwrap_or(0)
        .min(24);
    let size_width = 8.max(agent_width.min(width / 4));
    let fixed = 1
        + 1
        + 2
        + if show_type { 5 } else { 0 }
        + if show_size { size_width + 1 } else { 0 }
        + if show_bar { 2 } else { 0 }
        + if show_age { 5 } else { 0 }
        + 1;
    // Names take what they need (up to the space left), so metadata sits next to them
    // instead of across an empty gap on wide terminals.
    let name_width = width.saturating_sub(fixed).min(longest.max(28) + 2).max(4);
    let now = now_seconds();
    let mut hits = b.hits.borrow_mut();
    let mut lines = Vec::with_capacity(height);
    for (row, entry) in rows.iter().enumerate().skip(offset).take(height) {
        let is_selected = row == selected;
        let category = category(entry);
        let hidden = entry.hidden || entry.name.starts_with('.');
        let base = if is_selected {
            selection_style(focused)
        } else {
            Style::default()
        };
        let mut spans = Vec::with_capacity(12);
        spans.push(Span::styled(
            match (is_selected, focused, ascii()) {
                (true, true, false) => "▌",
                (true, false, false) => "▏",
                (true, true, true) => ">",
                (true, false, true) => ":",
                _ => " ",
            },
            base.patch(accent()),
        ));
        let (mark, mark_style) = if b.marked.contains(&entry.path) {
            (if ascii() { "*" } else { "✓" }, accent())
        } else if let Some((cut, _)) = clip
            .as_ref()
            .filter(|(_, paths)| paths.contains(entry.path.as_str()))
        {
            (
                if *cut { "x" } else { "c" },
                tint(if *cut { Color::Magenta } else { Color::Green }),
            )
        } else {
            (" ", Style::default())
        };
        spans.push(Span::styled(
            mark,
            base.patch(mark_style).add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(
            format!("{} ", icon(category)),
            base.patch(category_style(category)),
        ));
        // Name: directories bold bright blue with a trailing slash; dotfiles dim.
        let name = safe_label(&entry.name);
        let suffix = if category == Category::Directory {
            "/"
        } else {
            ""
        };
        let shown_name = middle_ellipsis(&format!("{name}{suffix}"), name_width);
        let mut name_style = if category == Category::Directory {
            dir_style()
        } else {
            Style::default()
        };
        if hidden {
            name_style = name_style.patch(muted());
        }
        if is_selected && focused {
            name_style = name_style.add_modifier(Modifier::BOLD);
        }
        let mut name_line = Line::from(Span::styled(shown_name.clone(), name_style));
        if !b.search.is_empty() && shown_name == format!("{name}{suffix}") {
            let row_matches: Vec<_> = matches[first_match..]
                .iter()
                .enumerate()
                .take_while(|(_, (r, _))| *r <= row)
                .filter(|(_, (r, _))| *r == row)
                .map(|(i, (_, p))| (first_match + i, p.clone()))
                .collect();
            if let Some((global, positions)) = row_matches.into_iter().next() {
                highlight_preview_matches(
                    std::slice::from_mut(&mut name_line),
                    &PreviewFind {
                        query: b.search.clone(),
                        matches: vec![(0, positions)],
                        selected: if global == selected_match { 0 } else { 1 },
                        reveal: std::cell::Cell::new(false),
                    },
                );
                if global != selected_match {
                    for span in &mut name_line.spans {
                        if span.style.add_modifier.contains(Modifier::UNDERLINED) {
                            span.style = span.style.add_modifier(Modifier::BOLD);
                        }
                    }
                }
            }
        } else if !b.search.is_empty()
            && matches[first_match..]
                .iter()
                .take_while(|(r, _)| *r <= row)
                .any(|(r, _)| *r == row)
        {
            // Clipped names cannot show positions; mark the whole match (NO_COLOR safe).
            for span in &mut name_line.spans {
                span.style = span
                    .style
                    .patch(tint(Color::Yellow))
                    .add_modifier(Modifier::BOLD | Modifier::UNDERLINED);
            }
        }
        let used = cell_width(&shown_name);
        for span in name_line.spans {
            spans.push(Span::styled(span.content, base.patch(span.style)));
        }
        spans.push(Span::styled(
            " ".repeat(name_width.saturating_sub(used) + 1),
            base,
        ));
        if show_type {
            spans.push(Span::styled(
                format!("{:<4} ", type_tag(entry, category)),
                base.patch(if category == Category::Directory {
                    muted()
                } else {
                    category_style(category)
                }),
            ));
        }
        if show_size {
            let (text, style) = if category == Category::Directory {
                match agents.get(&entry.path) {
                    Some(agent) => (agent.clone(), accent()),
                    None => ("·".to_owned(), muted()),
                }
            } else if entry.kind == "file" {
                (short_size(entry.size), muted())
            } else {
                ("·".to_owned(), muted())
            };
            spans.push(Span::styled(
                format!("{:>w$} ", fit_label(&text, size_width), w = size_width),
                base.patch(style),
            ));
        }
        if show_bar {
            spans.push(Span::styled(
                format!(
                    "{} ",
                    if entry.kind == "file" {
                        size_bar(entry.size, largest)
                    } else {
                        " "
                    }
                ),
                base.patch(accent()),
            ));
        }
        if show_age {
            let age = relative_age(entry.modified, now);
            let recent = entry.modified.is_some_and(|m| now - m < 3600);
            spans.push(Span::styled(
                format!("{age:>4} "),
                base.patch(if recent { Style::default() } else { muted() }),
            ));
        }
        // The selection band spans the whole row, not just the columns.
        let used: usize = spans.iter().map(|s| s.width()).sum();
        if is_selected && used < width {
            spans.push(Span::styled(" ".repeat(width - used), base));
        }
        let y = inner.y + lines.len() as u16;
        hits.push((Rect::new(inner.x, y, inner.width, 1), FileHit::Row(row)));
        lines.push(Line::from(spans));
    }
    drop(hits);
    frame.render_widget(Paragraph::new(lines), inner);
}
/// The inline preview: the highlighted file's preview, or the highlighted folder's
/// prefetched listing.
fn render_peek(frame: &mut Frame<'_>, b: &Browser, area: Rect) {
    b.hits.borrow_mut().push((area, FileHit::Peek));
    let rows = browser_entries(b);
    let selected = rows.get(b.selected);
    match (selected, b.peek.as_deref()) {
        (Some(entry), Some(peek))
            if entry.kind == "directory" && peek.path == entry.path && peek.preview.is_none() =>
        {
            let border = block(format!("{}/", safe_label(&entry.name)), false);
            let inner = border.inner(area);
            frame.render_widget(border, area);
            let children = browser_entries(peek);
            let lines: Vec<Line> = if children.is_empty() {
                vec![Line::styled(" Empty folder", muted())]
            } else {
                children
                    .iter()
                    .take(usize::from(inner.height))
                    .map(|child| {
                        let category = category(child);
                        let hidden = child.hidden || child.name.starts_with('.');
                        let mut style = if category == Category::Directory {
                            dir_style()
                        } else {
                            Style::default()
                        };
                        if hidden {
                            style = style.patch(muted());
                        }
                        Line::from(vec![
                            Span::styled(format!(" {} ", icon(category)), category_style(category)),
                            Span::styled(
                                middle_ellipsis(
                                    &format!(
                                        "{}{}",
                                        safe_label(&child.name),
                                        if category == Category::Directory {
                                            "/"
                                        } else {
                                            ""
                                        }
                                    ),
                                    usize::from(inner.width).saturating_sub(3),
                                ),
                                style,
                            ),
                        ])
                    })
                    .collect()
            };
            frame.render_widget(Paragraph::new(lines), inner);
            if children.len() > usize::from(inner.height) && area.height > 1 {
                let more = format!(" +{} more ", children.len() - usize::from(inner.height));
                let w = (cell_width(&more) as u16).min(area.width.saturating_sub(4));
                frame.render_widget(
                    Paragraph::new(Span::styled(more, muted())),
                    Rect::new(area.x + 2, area.bottom() - 1, w, 1),
                );
            }
        }
        (Some(entry), Some(peek))
            if entry.kind != "directory"
                && peek.preview_path.as_ref() == Some(&entry.path)
                && peek.preview.is_some() =>
        {
            render_preview(
                frame,
                area,
                peek,
                peek.preview.as_deref().unwrap_or(""),
                false,
            );
        }
        (Some(entry), _) => {
            let border = block(safe_label(&entry.name), false);
            let inner = border.inner(area);
            frame.render_widget(border, area);
            let mut lines = vec![Line::styled(
                if entry.kind == "directory" {
                    " Folder"
                } else {
                    " Loading preview…"
                },
                muted(),
            )];
            if entry.kind == "file" {
                lines.push(Line::styled(
                    format!(" {}", human_size(entry.size)),
                    muted(),
                ));
            }
            frame.render_widget(Paragraph::new(lines), inner);
        }
        (None, _) => {
            frame.render_widget(block(String::new(), false), area);
        }
    }
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
    pub(super) fn visible_entries(&self) -> VisibleRows {
        self.browser
            .as_ref()
            .map(browser_entries)
            .unwrap_or_else(VisibleRows::empty)
    }
    /// Debounced background work for the Files view, run every loop iteration:
    /// prefetch a folder highlighted for 150 ms, load the inline peek of a highlighted
    /// file, and keep a streaming preview fed. Returns true when a redraw is needed.
    pub(super) fn files_tick(&mut self, now: Instant) -> bool {
        if self.view != View::Files {
            return false;
        }
        self.pump_preview_stream();
        let dual = self.other_browser.is_some();
        let Some(b) = self.browser.as_mut() else {
            return false;
        };
        if b.preview.is_some() {
            return false;
        }
        let Some(entry) = browser_entries(b).get(b.selected).cloned() else {
            b.settle = None;
            return false;
        };
        match &b.settle {
            Some((path, _, _)) if *path == entry.path => {}
            _ => {
                b.settle = Some((entry.path.clone(), now, false));
                return false;
            }
        }
        let (_, since, done) = b.settle.as_ref().unwrap();
        if *done || now.duration_since(*since) < PREFETCH_DELAY {
            return false;
        }
        b.settle.as_mut().unwrap().2 = true;
        let device = b.device;
        let peek = b.peek_shown.get() && !dual;
        let mut dirty = false;
        if entry.kind == "directory" {
            let key = (
                device,
                entry.path.clone(),
                browser_scope_key(b.container.as_ref()),
            );
            let cached = self.browser_cache.get(&key).map(|c| c.entries.clone());
            let b = self.browser.as_mut().unwrap();
            if peek {
                let mut child = Browser::new(device, entry.path.clone());
                child.show_hidden = b.show_hidden;
                if let Some(entries) = &cached {
                    child.entries = entries.clone();
                }
                b.peek = Some(Box::new(child));
                dirty = true;
            }
            if cached.is_none()
                && !b.loading
                && b.prefetch.len() < 4
                && b.prefetch.insert(entry.path.clone())
            {
                let path = entry.path.clone();
                if !self.send(device, Operation::List { path: path.clone() }) {
                    if let Some(b) = self.browser.as_mut() {
                        b.prefetch.remove(&path);
                    }
                }
            }
        } else if peek && entry.kind == "file" {
            let wanted = b
                .peek
                .as_ref()
                .is_none_or(|p| p.preview_path.as_ref() != Some(&entry.path));
            if wanted {
                let mut child = b
                    .peek
                    .take()
                    .filter(|p| p.preview.is_some() && p.path != entry.path)
                    .unwrap_or_else(|| Box::new(Browser::new(device, b.path.clone())));
                child.path = b.path.clone();
                child.entries = vec![entry.clone()].into();
                child.preview_path = Some(entry.path.clone());
                child.preview_pending_page = Some(1);
                child.peek_value = None;
                b.peek = Some(child);
                if !self.send(
                    device,
                    Operation::Preview {
                        path: entry.path.clone(),
                    },
                ) {
                    if let Some(p) = self.browser.as_mut().and_then(|b| b.peek.as_mut()) {
                        p.preview_pending_page = None;
                    }
                }
                dirty = true;
            }
        }
        dirty
    }
    /// Time until `files_tick` has debounced work to do.
    pub(super) fn files_due(&self, now: Instant) -> Option<Duration> {
        if self.view != View::Files {
            return None;
        }
        let b = self.browser.as_ref()?;
        let (_, since, done) = b.settle.as_ref()?;
        (!done).then(|| PREFETCH_DELAY.saturating_sub(now.duration_since(*since)))
    }
    /// A click or tap on a Files target. Returns false when nothing was hit.
    pub(super) fn file_click(&mut self, column: u16, row: u16) -> bool {
        let position = ratatui::layout::Position::new(column, row);
        let find = |b: &Option<Browser>| {
            b.as_ref().and_then(|b| {
                b.hits
                    .borrow()
                    .iter()
                    .find(|(rect, _)| rect.contains(position))
                    .map(|(_, hit)| hit.clone())
            })
        };
        let (other, hit) = match (find(&self.browser), find(&self.other_browser)) {
            (Some(hit), _) => (false, hit),
            (None, Some(hit)) => (true, hit),
            (None, None) => return false,
        };
        if other {
            self.switch_pane();
        }
        self.focus = Focus::Workspace;
        match hit {
            FileHit::Row(index) => {
                let selected = self.browser.as_ref().map_or(0, |b| b.selected);
                if selected == index && !other {
                    self.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
                } else {
                    self.move_selection(index as isize - selected as isize);
                }
            }
            FileHit::Crumb(path) => {
                if let Some(device) = self.browser.as_ref().map(|b| b.device) {
                    self.open_browser(device, path);
                }
            }
            FileHit::Peek => {
                let target = self.browser.as_ref().and_then(|b| {
                    browser_entries(b)
                        .get(b.selected)
                        .map(|e| (b.device, e.clone()))
                });
                match target {
                    Some((device, e)) if e.kind == "directory" => self.open_browser(device, e.path),
                    Some((device, e)) => self.open_preview(device, e.path),
                    None => {}
                }
            }
        }
        true
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
                if !b.search.is_empty() {
                    let matches = browser_matches(b);
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
            let matches = browser_matches(b);
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
