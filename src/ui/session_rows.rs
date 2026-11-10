//! Session list rows (spec 7.1): selection bar, identity swatch, status glyph,
//! provider badge, title, folder, device chip, activity sparkline, idle age and
//! state. Rendering reads state only; hover never selects or fetches.
use super::*;
use std::cell::Cell as StdCell;

/// Per-viewer list state. Hover and scroll offset are presentation only.
#[derive(Default)]
pub(super) struct State {
    /// Logical row under the pointer, for a faint underline. Never a selection.
    pub(super) hover: Option<usize>,
    offset: StdCell<usize>,
    history: HashMap<(String, String), VecDeque<(u64, bool)>>,
}

const HISTORY_SECONDS: u64 = 60;
const HISTORY_KEYS: usize = 512;

impl State {
    /// Record one activity sample per observed session from a fresh listing.
    /// A session is active when it produced output since its previous sample
    /// (refreshes are not evenly spaced: RTT, busy devices and other views
    /// stretch the gap) or reports native work; unknown evidence records nothing.
    pub(super) fn record(&mut self, device: &Device, sessions: &[Session], now: u64) {
        let history = &mut self.history;
        for s in sessions {
            let samples = history
                .entry((device.id.clone(), workspaces::runtime_key(s)))
                .or_default();
            if samples.back().is_some_and(|(t, _)| *t == now) {
                samples.pop_back();
            }
            // No earlier sample: one bucket width, the span the bar covers.
            let gap = samples.back().map_or(15, |(t, _)| now.saturating_sub(*t));
            let active = match (s.idle_seconds, session_busy(s)) {
                (_, true) => true,
                (Some(idle), false) => idle <= gap.max(3),
                (None, false) => continue,
            };
            samples.push_back((now, active));
            while samples
                .front()
                .is_some_and(|(t, _)| now.saturating_sub(*t) > HISTORY_SECONDS)
            {
                samples.pop_front();
            }
        }
        history.retain(|_, samples| {
            samples
                .back()
                .is_some_and(|(t, _)| now.saturating_sub(*t) <= 2 * HISTORY_SECONDS)
        });
        if history.len() > HISTORY_KEYS {
            let mut oldest: Vec<_> = history
                .iter()
                .map(|(k, v)| (v.back().map(|(t, _)| *t).unwrap_or(0), k.clone()))
                .collect();
            oldest.sort();
            for (_, key) in oldest.into_iter().take(history.len() - HISTORY_KEYS) {
                history.remove(&key);
            }
        }
    }
    /// Four-cell sparkline of the last 60 s (15 s buckets), or None without evidence.
    pub(super) fn sparkline(&self, device: &Device, s: &Session, now: u64) -> Option<String> {
        let samples = self
            .history
            .get(&(device.id.clone(), workspaces::runtime_key(s)))?;
        if samples.is_empty() {
            return None;
        }
        let levels: &[&str] = if ascii() {
            &["_", ".", "-", "=", "#"]
        } else {
            &["▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"]
        };
        let mut out = String::new();
        for bucket in 0..4u64 {
            let (start, end) = (
                now.saturating_sub(HISTORY_SECONDS - bucket * 15),
                now.saturating_sub(HISTORY_SECONDS - (bucket + 1) * 15),
            );
            let in_bucket: Vec<bool> = samples
                .iter()
                .filter(|(t, _)| *t > start && *t <= end.max(start + 1))
                .map(|(_, a)| *a)
                .collect();
            if in_bucket.is_empty() {
                // No snapshot in this bucket: unknown, not idle.
                out.push_str(if ascii() { " " } else { "·" });
                continue;
            }
            let active = in_bucket.iter().filter(|a| **a).count();
            let level = (active * (levels.len() - 1)).div_ceil(in_bucket.len());
            out.push_str(levels[level.min(levels.len() - 1)]);
        }
        Some(out)
    }
}

/// Provider badge text and hue: SH / CL / CX, DC / CT for container shells.
pub(super) fn badge(app: &App, d: usize, s: &Session) -> (String, Style) {
    let style = provider(&s.provider).add_modifier(Modifier::BOLD);
    let text = match s.provider.as_str() {
        "claude" => "CL".to_string(),
        "codex" => "CX".to_string(),
        "shell" if s.container.is_some() => match menus::session_label(app, d, s) {
            "devcontainer" => "DC".into(),
            _ => "CT".into(),
        },
        "shell" => "SH".into(),
        other => other
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .take(2)
            .collect::<String>()
            .to_ascii_uppercase(),
    };
    (format!("{text:<2}"), style)
}
/// Two-letter device initials, unique across `names` (in list order).
/// Each name keeps its first letter and takes the next letter that no
/// earlier device claimed (`laptop` la, `lab-arm` lb), then a digit, so
/// chips stay distinct without color (NO_COLOR, color-blind viewers).
pub(super) fn device_initials(names: &[&str]) -> Vec<String> {
    let mut taken: Vec<String> = Vec::with_capacity(names.len());
    for name in names {
        let letters: Vec<char> = name
            .chars()
            .filter(|c| c.is_alphanumeric())
            .flat_map(|c| c.to_lowercase())
            .collect();
        let Some(&first) = letters.first() else {
            taken.push(String::new());
            continue;
        };
        let candidates = letters[1..]
            .iter()
            .copied()
            .chain('2'..='9')
            .chain('a'..='z')
            .map(|second| format!("{first}{second}"));
        let mut chosen = None;
        let mut fallback = None;
        for candidate in candidates {
            fallback.get_or_insert_with(|| candidate.clone());
            if !taken.contains(&candidate) {
                chosen = Some(candidate);
                break;
            }
        }
        taken.push(chosen.or(fallback).unwrap_or_else(|| first.to_string()));
    }
    taken
}
/// Device chip from precomputed unique initials, colored by device identity.
pub(super) fn device_chip(device: &Device, initials: &str) -> (String, Style) {
    let initials = if initials.is_empty() {
        "··"
    } else {
        initials
    };
    (format!("[{}]", safe_label(initials)), swatch(&device.id))
}
/// Seconds rendered as a compact age: 4s, 9m, 2h, 3d.
pub(super) fn age(seconds: u64) -> String {
    match seconds {
        0..=59 => format!("{seconds}s"),
        60..=3599 => format!("{}m", seconds / 60),
        3600..=86_399 => format!("{}h", seconds / 3600),
        _ => format!("{}d", seconds / 86_400),
    }
}
/// Idle age now: the helper's observation plus time since that listing.
pub(super) fn idle_now(cache: &Cached, s: &Session) -> Option<u64> {
    let idle = s.idle_seconds?;
    (cache.fetched > 0).then(|| idle.saturating_add(transport::now().saturating_sub(cache.fetched)))
}
/// The device's home folder for `~` display: the viewer's HOME, so only for
/// the local device. No helper reports a remote HOME, so remote paths stay
/// absolute rather than guessing `/home/<account>`.
pub(super) fn device_home(device: &Device) -> Option<String> {
    device
        .target
        .is_none()
        .then(|| std::env::var("HOME").ok().filter(|h| h.len() > 1))
        .flatten()
}
/// Folder path, ~-relative to `home`, with a tail-preserving ellipsis (`…/cx-demo`).
pub(super) fn folder_tail(path: &str, home: Option<&str>, width: usize) -> String {
    let path = if path.is_empty() {
        // Unknown evidence, never a path a command could run in.
        "? folder".to_string()
    } else {
        safe_label(path)
    };
    let home_relative = home
        .filter(|h| h.len() > 1)
        .and_then(|h| {
            path.strip_prefix(h.trim_end_matches('/'))
                .filter(|rest| rest.is_empty() || rest.starts_with('/'))
                .map(|rest| format!("~{rest}"))
        })
        .unwrap_or(path);
    if text_width(&home_relative) <= width {
        return home_relative;
    }
    // Too long: keep as many trailing components as fit behind `…/`, so a cut
    // path never reads like a relative name. A basename too long for the cell
    // keeps its own tail.
    let marker = if ascii() { ".../" } else { "…/" };
    let trimmed = home_relative.trim_end_matches('/');
    let parts: Vec<&str> = trimmed.split('/').filter(|p| !p.is_empty()).collect();
    let mut kept = String::new();
    for part in parts.iter().rev() {
        let candidate = if kept.is_empty() {
            part.to_string()
        } else {
            format!("{part}/{kept}")
        };
        if text_width(marker) + text_width(&candidate) > width {
            break;
        }
        kept = candidate;
    }
    if kept.is_empty() {
        tail(parts.last().copied().unwrap_or(trimmed), width)
    } else {
        format!("{marker}{kept}")
    }
}
fn text_width(text: &str) -> usize {
    Span::raw(text).width()
}
fn char_width(c: char) -> usize {
    let mut buf = [0u8; 4];
    text_width(c.encode_utf8(&mut buf))
}
fn tail(text: &str, width: usize) -> String {
    if text_width(text) <= width {
        return text.to_string();
    }
    if width == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut used = 1;
    for c in text.chars().rev() {
        let w = char_width(c);
        if used + w > width {
            break;
        }
        used += w;
        out.insert(0, c);
    }
    format!("{}{out}", if ascii() { "~" } else { "…" })
}
fn head(text: &str, width: usize) -> String {
    let full = text_width(text);
    if full <= width {
        return format!("{text}{}", " ".repeat(width - full));
    }
    if width == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut used = 1;
    for c in text.chars() {
        let w = char_width(c);
        if used + w > width {
            break;
        }
        used += w;
        out.push(c);
    }
    out.push_str(if ascii() { "~" } else { "…" });
    let pad = width.saturating_sub(used);
    format!("{out}{}", " ".repeat(pad))
}
fn right(text: &str, width: usize) -> String {
    let w = text_width(text);
    if w >= width {
        text.to_string()
    } else {
        format!("{}{text}", " ".repeat(width - w))
    }
}

/// Status glyph per spec: running (ok), stale/cached (warn), ended (err).
fn status_glyph(status: &str) -> (&'static str, Style) {
    let a = ascii();
    if status.starts_with('+') {
        (if a { "+" } else { "●" }, ok())
    } else if status == "not seen" {
        (
            if a { "x" } else { "✕" },
            err().add_modifier(Modifier::BOLD),
        )
    } else {
        (if a { "~" } else { "◐" }, warn())
    }
}
/// State text, shown only when it is not idle. Unknown evidence is `·`.
fn state_text(app: &App, d: usize, s: &Session, status: &str) -> (String, Style) {
    if !status.starts_with('+') {
        return match status {
            "not seen" => ("ended".into(), err()),
            "? cached" => ("cached".into(), warn()),
            _ => ("stale".into(), warn()),
        };
    }
    if s.activity.is_some() && !session_activity_current(&app.work[d]) {
        return ("? state".into(), muted());
    }
    if session_busy(s) {
        let frames = if ascii() {
            ["|", "/", "-", "\\"]
        } else {
            ["◐", "◓", "◑", "◒"]
        };
        let word = match s.activity.as_deref() {
            Some("output") => "active",
            _ => "working",
        };
        return (
            format!("{} {word}", frames[app.transfer_frame % frames.len()]),
            info(),
        );
    }
    match s.activity.as_deref() {
        Some("waiting") => ("awaiting".into(), warn().add_modifier(Modifier::BOLD)),
        _ => (String::new(), Style::default()),
    }
}
fn title(app: &App, d: usize, s: &Session) -> String {
    let mut name = session_preview::label(s);
    if !s.agent_tasks.is_empty() {
        name = format!("{name}{}{} tasks", sep(), s.agent_tasks.len());
    }
    if let Some(origin) = s.origin.as_ref().filter(|_| s.id.starts_with("ext-")) {
        name = format!("{name}{}{}", sep(), origin_label(&origin.kind));
    }
    if let Some(scope) = s.container.as_ref().filter(|_| s.provider != "shell") {
        name = format!("{name}{}{}", sep(), menus::container_label(app, d, scope));
    }
    name
}
/// The title the list shows for `(d, s)`, ordinal included, so the preview
/// border (and zoom, where the list is hidden) names the same session.
pub(super) fn display_title(app: &App, d: usize, s: &Session) -> String {
    let rows = app.session_rows();
    let key = workspaces::runtime_key(s);
    match rows
        .iter()
        .position(|(rd, rs)| *rd == d && workspaces::runtime_key(rs) == key)
    {
        Some(i) => numbered(rows.iter().map(|(d, s)| title(app, *d, s)).collect()).swap_remove(i),
        None => title(app, d, s),
    }
}
/// Equal titles get an ordinal in list order (`robot-logs shell 2`), so two
/// shells in one folder stay distinguishable without exposing tmux ids.
fn numbered(titles: Vec<String>) -> Vec<String> {
    let mut seen: HashMap<String, usize> = HashMap::new();
    let total: HashMap<String, usize> = titles.iter().fold(HashMap::new(), |mut m, t| {
        *m.entry(t.clone()).or_default() += 1;
        m
    });
    titles
        .into_iter()
        .map(|t| {
            let n = seen.entry(t.clone()).or_default();
            *n += 1;
            if total[&t] > 1 && *n > 1 {
                format!("{t} {n}")
            } else {
                t
            }
        })
        .collect()
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

struct Columns {
    two_rows: bool,
    folder: usize,
    chip: bool,
    spark: bool,
    state: usize,
}
fn columns(width: usize, phone: bool, many_devices: bool) -> Columns {
    // bar 1 · swatch 2 · status 2 · badge 3 · title · folder · chip 5 · spark 5 · age 4 · state
    if phone || width < 30 {
        return Columns {
            two_rows: true,
            folder: 0,
            // The chip needs room next to a readable title.
            chip: many_devices && width >= 30,
            spark: false,
            state: 0,
        };
    }
    let folder = if width >= 110 {
        ((width - 40) / 3).clamp(24, 40)
    } else if width >= 80 {
        // Room for `~/projects/<name>` before the tail ellipsis is needed.
        ((width - 40) / 2).clamp(18, 28)
    } else if width >= 64 {
        12
    } else {
        10
    };
    Columns {
        two_rows: false,
        folder,
        chip: many_devices,
        spark: width >= 72,
        // Narrow side-by-side lists keep the status glyph as their evidence.
        state: if width >= 64 {
            10
        } else if width >= 50 {
            9
        } else {
            0
        },
    }
}

/// Render the non-empty session list into `area` and register row hit boxes.
pub(super) fn render(frame: &mut Frame<'_>, app: &App, area: Rect, rows: &[(usize, &Session)]) {
    let focused = app.focus == Focus::Workspace;
    let block = block(format!("Sessions {}", rows.len()), focused);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let many_devices = app.device == 0 && app.devices.len() > 1;
    // Phones (spec XS, under 60 columns) use two rows per session.
    let cols = columns(
        usize::from(inner.width),
        frame.area().width < 60,
        many_devices,
    );
    let row_height = if cols.two_rows { 2 } else { 1 };
    let visible = (usize::from(inner.height) / row_height).max(1);
    // Scrolloff 2: the list scrolls before the bar reaches an edge, never recentres.
    let selected = app.selected.min(rows.len().saturating_sub(1));
    let mut offset = app.session_list.offset.get();
    let margin = 2.min(visible.saturating_sub(1) / 2);
    if selected < offset + margin {
        offset = selected.saturating_sub(margin);
    }
    if selected + margin >= offset + visible {
        offset = (selected + margin + 1).saturating_sub(visible);
    }
    offset = offset.min(rows.len().saturating_sub(visible));
    app.session_list.offset.set(offset);
    let now = transport::now();
    let a = ascii();
    let titles = numbered(rows.iter().map(|(d, s)| title(app, *d, s)).collect());
    let initials = if cols.chip {
        device_initials(
            &app.devices
                .iter()
                .map(|d| d.name.as_str())
                .collect::<Vec<_>>(),
        )
    } else {
        Vec::new()
    };
    let mut lines = Vec::new();
    let mut hits = app.session_hits.borrow_mut();
    for (index, (d, s)) in rows.iter().enumerate().skip(offset).take(visible) {
        let (d, s) = (*d, *s);
        let device = &app.devices[d];
        let home = device_home(device);
        let is_selected = index == app.selected;
        let row_style = if is_selected {
            selection(focused)
        } else {
            Style::default()
        };
        let hover = app.session_list.hover == Some(index) && !is_selected;
        let status = app.session_status(d, s);
        let fresh = status.starts_with('+');
        let text_style = if fresh { Style::default() } else { muted() };
        let text_style = if hover {
            text_style.add_modifier(Modifier::UNDERLINED)
        } else {
            text_style
        };
        let bar = match (is_selected, focused, a) {
            (true, true, false) => Span::styled("▌", accent()),
            (true, false, false) => Span::styled("▏", muted()),
            (true, true, true) => Span::styled(">", accent().add_modifier(Modifier::BOLD)),
            (true, false, true) => Span::styled(":", muted()),
            _ => Span::raw(" "),
        };
        // Phone rows (spec XS) drop the swatch so the title keeps its room.
        let swatch_span = if cols.two_rows {
            Span::raw("")
        } else if no_color() {
            Span::raw("  ")
        } else {
            Span::styled(if a { "# " } else { "■ " }, swatch(&s.id))
        };
        let (glyph, glyph_style) = status_glyph(status);
        let (badge_text, badge_style) = badge(app, d, s);
        let pin = if app.pinned(d, s) {
            if a {
                "!"
            } else {
                "⚑"
            }
        } else {
            " "
        };
        let idle = idle_now(&app.work[d], s)
            .map(age)
            .unwrap_or_else(|| "·".into());
        let chip = cols.chip.then(|| device_chip(device, &initials[d]));
        let (state, state_style) = state_text(app, d, s, status);
        let name = titles[index].clone();
        let fixed = 1 + if cols.two_rows { 0 } else { 2 } + 2 + 3;
        let tail_width = if cols.two_rows {
            chip.as_ref().map(|_| 5).unwrap_or(0) + 5
        } else {
            (if cols.folder > 0 { cols.folder + 1 } else { 0 })
                + chip.as_ref().map(|_| 5).unwrap_or(0)
                + if cols.spark { 5 } else { 0 }
                + 5
                + if cols.state > 0 { cols.state + 1 } else { 0 }
        };
        let title_width = usize::from(inner.width)
            .saturating_sub(fixed + tail_width)
            .max(1);
        let mut spans = vec![
            bar,
            swatch_span,
            Span::styled(glyph, glyph_style),
            Span::styled(pin, accent()),
            Span::styled(format!("{badge_text} "), badge_style),
            Span::styled(
                head(&name, title_width),
                text_style.add_modifier(Modifier::BOLD),
            ),
        ];
        if !cols.two_rows && cols.folder > 0 {
            spans.push(Span::raw(" "));
            // Narrow lists without a state column show non-idle state in the
            // folder cell: evidence outranks the folder, which the preview shows.
            spans.push(if cols.state == 0 && !state.is_empty() {
                Span::styled(head(&state, cols.folder), state_style)
            } else {
                Span::styled(
                    head(
                        &folder_tail(&s.directory, home.as_deref(), cols.folder),
                        cols.folder,
                    ),
                    muted(),
                )
            });
        }
        if let Some((chip, style)) = &chip {
            spans.push(Span::raw(" "));
            spans.push(Span::styled(chip.clone(), *style));
        }
        if cols.spark {
            let spark = app
                .session_list
                .sparkline(device, s, now)
                .unwrap_or_else(|| "   ·".into());
            spans.push(Span::raw(" "));
            spans.push(Span::styled(spark, info()));
        }
        spans.push(Span::styled(right(&idle, 5), muted()));
        if !cols.two_rows && cols.state > 0 {
            spans.push(Span::raw(" "));
            spans.push(Span::styled(head(&state, cols.state), state_style));
        }
        let y = inner.y + ((index - offset) * row_height) as u16;
        hits.push((Rect::new(inner.x, y, inner.width, row_height as u16), index));
        lines.push(Line::from(spans).style(row_style));
        if cols.two_rows {
            // The path tail sits under the status glyph (spec 6.1 XS).
            const INDENT: usize = 3;
            let lead = if is_selected && focused && !a {
                Span::styled("▌  ", accent())
            } else {
                Span::raw("   ")
            };
            let w = usize::from(inner.width).saturating_sub(
                INDENT
                    + if state.is_empty() {
                        0
                    } else {
                        text_width(&state) + 1
                    },
            );
            let mut second = vec![
                lead,
                Span::styled(
                    head(&folder_tail(&s.directory, home.as_deref(), w), w),
                    muted(),
                ),
            ];
            if !state.is_empty() {
                // Narrow rows keep the state on the folder line, right-aligned.
                second.push(Span::raw(" "));
                second.push(Span::styled(state.clone(), state_style));
            }
            lines.push(Line::from(second).style(row_style));
        }
    }
    drop(hits);
    frame.render_widget(Paragraph::new(lines), inner);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn swatches_are_stable_and_spread() {
        assert_eq!(swatch_index("cx-a"), swatch_index("cx-a"));
        let hues: BTreeSet<u8> = (0..64).map(|i| swatch_index(&format!("cx-{i}"))).collect();
        assert_eq!(hues.len(), 8);
    }
    #[test]
    fn device_initials_stay_unique_without_color() {
        assert_eq!(
            device_initials(&["laptop", "lab-arm", "field-rover"]),
            ["la", "lb", "fi"]
        );
        // Same letters throughout fall back to a digit, never a duplicate.
        assert_eq!(
            device_initials(&["aa", "aa", "a", ""]),
            ["aa", "a2", "a3", ""]
        );
        let many: Vec<String> = (0..30).map(|i| format!("lab{i}")).collect();
        let refs: Vec<&str> = many.iter().map(String::as_str).collect();
        let chips = device_initials(&refs);
        assert_eq!(chips.iter().collect::<BTreeSet<_>>().len(), chips.len());
    }
    #[test]
    fn folders_are_relative_to_the_device_home() {
        assert_eq!(folder_tail("/home/ana/x", Some("/home/ana"), 20), "~/x");
        assert_eq!(folder_tail("/home/ana", Some("/home/ana/"), 20), "~");
        assert_eq!(
            folder_tail("/home/anab/x", Some("/home/ana"), 20),
            "/home/anab/x"
        );
        // Cut paths keep a visible `…/` so they never pass for relative names.
        assert_eq!(folder_tail("/srv/a/b/cx-demo", None, 12), "…/b/cx-demo");
        assert_eq!(folder_tail("/srv/a/b/cx-demo", None, 9), "…/cx-demo");
        assert_eq!(
            folder_tail("/home/ana/projects/robot-logs", Some("/home/ana"), 14),
            "…/robot-logs"
        );
        assert_eq!(folder_tail("/srv/a/b/cx-demo", None, 8), "cx-demo");
        assert_eq!(folder_tail("/srv/a/b/cx-demo", None, 5), "…demo");
    }
    fn device(target: Option<&str>) -> Device {
        Device {
            id: "d".into(),
            name: "robot".into(),
            target: target.map(Into::into),
            account: "ana".into(),
            host: "robot".into(),
            status: String::new(),
            observed_at: 0,
        }
    }
    fn shell(id: &str, idle: Option<u64>) -> Session {
        let mut s: Session = serde_json::from_value(serde_json::json!({"id": id, "name": "shell", "directory": "/", "provider": "shell", "host": "h", "account": "a", "pid": 1, "started": "1", "boot_id": "b", "external": false, "socket": "cx"})).unwrap();
        s.idle_seconds = idle;
        s
    }
    #[test]
    fn remote_folders_stay_absolute_without_an_observed_home() {
        assert_eq!(device_home(&device(Some("ana@robot"))), None);
    }
    #[test]
    fn sparkline_records_output_since_the_previous_sample() {
        let (low, unknown) = if ascii() { ("_", " ") } else { ("▁", "·") };
        let d = device(None);
        let mut state = State::default();
        assert_eq!(state.sparkline(&d, &shell("cx-a", None), 1000), None);
        // No idle evidence records nothing.
        state.record(&d, &[shell("cx-a", None)], 1000);
        assert_eq!(state.sparkline(&d, &shell("cx-a", None), 1000), None);
        state.record(&d, &[shell("cx-a", Some(100))], 1000);
        let idle = state.sparkline(&d, &shell("cx-a", None), 1000).unwrap();
        assert_eq!(idle, format!("{0}{0}{0}{low}", unknown));
        // Output 5 s ago, 10 s after the previous sample: active, even though
        // the refresh gap was longer than the nominal 3 s.
        state.record(&d, &[shell("cx-a", Some(5))], 1010);
        let spark = state.sparkline(&d, &shell("cx-a", None), 1010).unwrap();
        assert!(!spark.ends_with(low), "{spark}");
        // A repeated timestamp replaces its sample rather than adding one.
        state.record(&d, &[shell("cx-a", Some(100))], 1010);
        state.record(&d, &[shell("cx-a", Some(100))], 1010);
        assert_eq!(state.history.values().next().unwrap().len(), 2);
        // Samples older than the window drop out; the bar is unknown again.
        state.record(&d, &[shell("cx-a", Some(100))], 1100);
        assert_eq!(state.history.values().next().unwrap().len(), 1);
        // A key unseen for two windows is forgotten.
        state.record(&d, &[], 1300);
        assert!(state.history.is_empty());
    }
    #[test]
    fn sparkline_history_evicts_the_oldest_keys_first() {
        let d = device(None);
        let mut state = State::default();
        state.record(&d, &[shell("cx-old", Some(0))], 1000);
        let newer: Vec<_> = (0..HISTORY_KEYS)
            .map(|i| shell(&format!("cx-{i}"), Some(0)))
            .collect();
        state.record(&d, &newer, 1001);
        assert_eq!(state.history.len(), HISTORY_KEYS);
        assert_eq!(state.sparkline(&d, &shell("cx-old", None), 1001), None);
        assert!(state.sparkline(&d, &newer[0], 1001).is_some());
    }
    #[test]
    fn equal_titles_are_numbered_in_list_order() {
        let titles = numbered(vec!["a shell".into(), "b".into(), "a shell".into()]);
        assert_eq!(titles, ["a shell", "b", "a shell 2"]);
    }
    #[test]
    fn ages_and_tails_keep_the_distinguishing_end() {
        assert_eq!(age(4), "4s");
        assert_eq!(age(61), "1m");
        assert_eq!(age(7200), "2h");
        assert_eq!(age(3 * 86_400), "3d");
        assert_eq!(
            tail("/tmp/cx-capture/projects/cx-demo", 10).chars().count(),
            10
        );
        assert!(tail("/tmp/cx-capture/projects/cx-demo", 10).ends_with("cx-demo"));
        assert_eq!(tail("short", 10), "short");
    }
}
