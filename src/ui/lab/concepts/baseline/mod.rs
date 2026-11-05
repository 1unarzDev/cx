//! `baseline`: a rough copy of the v0.1.95 layout (tabs, Devices/Actions sidebar,
//! bordered list + preview, three footer rows). It exists to prove the lab pipeline
//! end to end and to give new concepts something to beat; it is not a design.
use crate::ui::lab::{
    ansi_lines, fmt, Access, Concept, DeviceStatus, FileKind, LabCaps, LabState, LabView, Pane,
    SessionState, Tok,
};
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    symbols::border,
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Widget},
};

pub struct Baseline;

const ASCII_BORDER: border::Set = border::Set {
    top_left: "+",
    top_right: "+",
    bottom_left: "+",
    bottom_right: "+",
    vertical_left: "|",
    vertical_right: "|",
    horizontal_top: "-",
    horizontal_bottom: "-",
};

fn block<'a>(caps: &LabCaps, title: &str, focused: bool) -> Block<'a> {
    let set = if caps.unicode {
        border::ROUNDED
    } else {
        ASCII_BORDER
    };
    let mark = if focused {
        caps.glyph("▸ ", "> ")
    } else {
        ""
    };
    let mut b = Block::default()
        .borders(Borders::ALL)
        .border_set(set)
        .title(Span::styled(
            format!(" {mark}{title} "),
            caps.tok(Tok::Accent),
        ));
    if focused {
        b = b.border_style(Style::default().add_modifier(Modifier::BOLD));
    }
    b
}

fn status_glyph(caps: &LabCaps, s: DeviceStatus) -> Span<'static> {
    let (u, a, t) = match s {
        DeviceStatus::Local => ("●", "*", Tok::Ok),
        DeviceStatus::Online => ("●", "*", Tok::Ok),
        DeviceStatus::Stale => ("◐", "~", Tok::Warn),
        DeviceStatus::Down => ("○", "x", Tok::Err),
        DeviceStatus::Unknown => ("◌", "?", Tok::Muted),
    };
    Span::styled(caps.glyph(u, a), caps.tok(t))
}

fn row_line(caps: &LabCaps, selected: bool, text: Vec<Span<'static>>) -> Line<'static> {
    let mut spans = vec![Span::raw(if selected {
        caps.glyph("› ", "> ")
    } else {
        "  "
    })];
    spans.extend(text);
    let mut l = Line::from(spans);
    if selected {
        l = l.style(caps.tok(Tok::AccentBg));
    }
    l
}

fn devices(state: &LabState, caps: &LabCaps, area: Rect, buf: &mut Buffer, focused: bool) {
    let w = usize::from(area.width.saturating_sub(7));
    let mut lines = vec![row_line(caps, false, vec![Span::raw("All devices")])];
    for (i, d) in state.devices.iter().enumerate() {
        lines.push(row_line(
            caps,
            focused && i == caps.selected,
            vec![
                status_glyph(caps, d.status),
                Span::raw(" "),
                Span::raw(fmt::fit(&d.name, w, caps.unicode)),
            ],
        ));
    }
    Paragraph::new(lines)
        .block(block(caps, "Devices", focused))
        .render(area, buf);
}

fn session_state(caps: &LabCaps, s: SessionState) -> Span<'static> {
    let (t, tok) = match s {
        SessionState::Working => ("working", Tok::Accent),
        SessionState::AwaitingInput => ("input", Tok::Warn),
        SessionState::Running => ("running", Tok::Ok),
        SessionState::Idle => ("idle", Tok::Muted),
        SessionState::Exited => ("exited", Tok::Err),
    };
    Span::styled(t, caps.tok(tok))
}

fn sessions(
    state: &LabState,
    caps: &LabCaps,
    device: Option<&str>,
    area: Rect,
    buf: &mut Buffer,
    focused: bool,
) {
    let inner_w = usize::from(area.width.saturating_sub(4));
    let mut lines = vec![Line::styled(
        format!("  {:<w$}STATE", "SESSION", w = inner_w.saturating_sub(9)),
        caps.tok(Tok::Muted),
    )];
    let list = state
        .sessions
        .iter()
        .filter(|s| device.is_none_or(|d| s.device == d));
    for (i, s) in list.enumerate() {
        lines.push(Line::styled(
            format!("  {}  {}", s.device, s.cwd),
            caps.tok(Tok::Muted),
        ));
        let name = fmt::fit(&s.title, inner_w.saturating_sub(12), caps.unicode);
        let pad = inner_w.saturating_sub(name.chars().count() + 10);
        lines.push(row_line(
            caps,
            i == caps.selected,
            vec![
                Span::raw(name),
                Span::raw(" ".repeat(pad)),
                session_state(caps, s.state),
            ],
        ));
    }
    let title = format!("Sessions · {}", state.sessions.len());
    Paragraph::new(lines)
        .block(block(caps, &title, focused))
        .render(area, buf);
}

fn preview(state: &LabState, caps: &LabCaps, id: Option<&str>, area: Rect, buf: &mut Buffer) {
    let s = id
        .and_then(|i| state.session(i))
        .or_else(|| state.sessions.get(caps.selected))
        .or(state.sessions.first());
    let Some(s) = s else {
        block(caps, "Preview", false).render(area, buf);
        return;
    };
    let b = block(caps, &format!("{} · {}", s.title, s.device), false);
    let inner = b.inner(area);
    b.render(area, buf);
    let mut lines = vec![Line::styled(
        "Enter · focus terminal · Wheel · scroll",
        caps.tok(Tok::Muted),
    )];
    lines.extend(ansi_lines(&s.screen, inner.width));
    Paragraph::new(lines).render(inner, buf);
}

fn files(state: &LabState, caps: &LabCaps, area: Rect, buf: &mut Buffer) {
    let f = &state.files;
    let w = usize::from(area.width.saturating_sub(14));
    let lines: Vec<Line> = f
        .entries
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let indent = "  ".repeat(usize::from(e.depth));
            let (name, tok) = match e.kind {
                FileKind::Dir => (format!("{}/", e.name), Tok::Dir),
                FileKind::Symlink => (format!("{}@", e.name), Tok::Info),
                FileKind::File => (e.name.clone(), Tok::Fg),
            };
            let size = if e.kind == FileKind::Dir {
                String::new()
            } else {
                fmt::bytes(e.size)
            };
            let left = fmt::fit(&format!("{indent}{name}"), w, caps.unicode);
            let pad = w.saturating_sub(left.chars().count());
            row_line(
                caps,
                i == f.selected,
                vec![
                    Span::styled(left, caps.tok(tok)),
                    Span::raw(" ".repeat(pad)),
                    Span::styled(format!("{size:>8}"), caps.tok(Tok::Muted)),
                ],
            )
        })
        .collect();
    Paragraph::new(lines)
        .block(block(
            caps,
            &format!("Files · {}:{}", f.device, f.cwd),
            true,
        ))
        .render(area, buf);
}

fn file_preview(state: &LabState, caps: &LabCaps, area: Rect, buf: &mut Buffer) {
    let Some(p) = state.preview() else { return };
    let lines: Vec<Line> = p
        .text
        .lines()
        .enumerate()
        .map(|(i, l)| {
            Line::from(vec![
                Span::styled(format!("{:>3} ", i + 1), caps.tok(Tok::Faint)),
                Span::raw(l.to_string()),
            ])
        })
        .collect();
    Paragraph::new(lines)
        .block(block(caps, &p.path, true))
        .render(area, buf);
}

fn access(a: Access) -> &'static str {
    match a {
        Access::Key => "key",
        Access::Password => "password",
        Access::Denied => "none",
    }
}

fn network(state: &LabState, caps: &LabCaps, area: Rect, buf: &mut Buffer) {
    let mut lines = Vec::new();
    for (i, d) in state.devices.iter().enumerate() {
        let link = state
            .edges_of(&d.id)
            .find(|e| e.from == state.viewer || e.to == state.viewer)
            .map(|e| format!("{:?}", e.kind).to_lowercase())
            .unwrap_or_default();
        lines.push(row_line(
            caps,
            i == caps.selected,
            vec![
                status_glyph(caps, d.status),
                Span::raw(format!(" {:<20} ", fmt::fit(&d.name, 20, caps.unicode))),
                Span::styled(format!("{:<22} ", d.evidence), caps.tok(Tok::Muted)),
                Span::styled(fmt::fit(&link, 20, caps.unicode), caps.tok(Tok::Mesh)),
            ],
        ));
    }
    for m in &state.meshes {
        lines.push(Line::raw(""));
        lines.push(Line::styled(
            format!("  mesh {} · {}", m.name, m.detail),
            caps.tok(Tok::Mesh),
        ));
    }
    Paragraph::new(lines)
        .block(block(caps, "Network", true))
        .render(area, buf);
}

fn simple_list(
    caps: &LabCaps,
    title: &str,
    focused: bool,
    lines: Vec<Line<'static>>,
    area: Rect,
    buf: &mut Buffer,
) {
    Paragraph::new(lines)
        .block(block(caps, title, focused))
        .render(area, buf);
}

fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width);
    let h = h.min(area.height);
    Rect::new(
        area.x + (area.width - w) / 2,
        area.y + (area.height - h) / 2,
        w,
        h,
    )
}

fn qr(state: &LabState, caps: &LabCaps) -> Vec<Line<'static>> {
    let Some(inv) = &state.invite else {
        return vec![Line::raw("no pending invite")];
    };
    let mut lines = vec![
        Line::styled(format!("code {}", inv.code), caps.tok(Tok::Accent)),
        Line::raw(inv.oneliner.clone()),
        Line::raw(""),
    ];
    // Half-block QR: two module rows per cell row.
    for pair in inv.qr.chunks(2) {
        let row: String = (0..pair[0].len())
            .map(|c| {
                let top = pair[0][c];
                let bot = pair.get(1).is_some_and(|r| r[c]);
                match (top, bot, caps.unicode) {
                    (true, true, true) => '█',
                    (true, false, true) => '▀',
                    (false, true, true) => '▄',
                    (true, _, false) | (_, true, false) => '#',
                    _ => ' ',
                }
            })
            .collect();
        lines.push(Line::raw(row));
    }
    lines
}

impl Concept for Baseline {
    fn name(&self) -> &str {
        "baseline"
    }

    fn summary(&self) -> &str {
        "v0.1.95 layout, roughly: tabs, sidebar, bordered panels, three footer rows"
    }

    fn render(
        &self,
        view: LabView,
        state: &LabState,
        caps: &LabCaps,
        area: Rect,
        buf: &mut Buffer,
    ) {
        if area.width < 4 || area.height < 4 {
            return;
        }
        let [tabs, _, body, status, footer] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
            Constraint::Length(2),
        ])
        .areas(area);
        let tab = match view {
            LabView::Files | LabView::FilePreview => 1,
            LabView::Hub | LabView::EdgeEditor(_) | LabView::JoinCode => 2,
            _ => 0,
        };
        let mut t = vec![Span::styled(
            " cx  ",
            caps.tok(Tok::Accent).add_modifier(Modifier::BOLD),
        )];
        for (i, name) in ["Sessions", "Files", "Network"].iter().enumerate() {
            t.push(if i == tab {
                Span::styled(
                    format!(" [{name}] "),
                    Style::default().add_modifier(Modifier::BOLD),
                )
            } else {
                Span::styled(format!("  {name}  "), caps.tok(Tok::Muted))
            });
        }
        Paragraph::new(Line::from(t)).render(tabs, buf);

        let side_w = if body.width >= 60 { 21 } else { 0 };
        let [side, main] =
            Layout::horizontal([Constraint::Length(side_w), Constraint::Min(1)]).areas(body);
        if side_w > 0 {
            let [devs, acts] =
                Layout::vertical([Constraint::Min(3), Constraint::Length(6)]).areas(side);
            devices(state, caps, devs, buf, caps.is_focused(Pane::Rail));
            simple_list(
                caps,
                "Actions",
                false,
                ["Files", "Network", "Containers", "Transfers"]
                    .map(|s| Line::raw(format!(" {s}")))
                    .to_vec(),
                acts,
                buf,
            );
        }
        let split = |r: Rect| {
            Layout::vertical([Constraint::Percentage(45), Constraint::Percentage(55)]).areas::<2>(r)
        };
        let base = |buf: &mut Buffer| {
            let [l, p] = split(main);
            sessions(state, caps, None, l, buf, !caps.is_focused(Pane::Rail));
            preview(state, caps, None, p, buf);
        };
        match &view {
            LabView::Hub | LabView::JoinCode | LabView::EdgeEditor(_) => {
                network(state, caps, main, buf)
            }
            LabView::DeviceFocus(d) => {
                let [l, p] = split(main);
                sessions(state, caps, Some(d), l, buf, true);
                preview(state, caps, None, p, buf);
            }
            LabView::SessionPeek(id) => {
                let [l, p] = split(main);
                sessions(state, caps, None, l, buf, false);
                preview(state, caps, Some(id), p, buf);
            }
            LabView::Files => files(state, caps, main, buf),
            LabView::FilePreview => file_preview(state, caps, main, buf),
            LabView::Secrets => {
                let lines = state
                    .secrets
                    .iter()
                    .map(|s| {
                        let v = s.masked.clone().unwrap_or_else(|| "(missing)".into());
                        Line::raw(format!("  {:<24} {v}", s.key))
                    })
                    .collect();
                simple_list(caps, "Secrets", true, lines, main, buf);
            }
            _ => base(buf),
        }
        // Overlays.
        let overlay = |title: &str, lines: Vec<Line<'static>>, w: u16, h: u16, buf: &mut Buffer| {
            let r = centered(area, w, h);
            Clear.render(r, buf);
            simple_list(caps, title, true, lines, r, buf);
        };
        match &view {
            LabView::Palette(q) => {
                let mut lines = vec![Line::raw(format!("> {q}")), Line::raw("")];
                for d in state.devices.iter().filter(|d| d.name.contains(q.as_str())) {
                    lines.push(Line::raw(format!("  go to {}", d.name)));
                }
                overlay("Command", lines, 50, 12, buf);
            }
            LabView::Onboarding => overlay(
                "Welcome",
                vec![
                    Line::raw("No mesh yet."),
                    Line::raw(""),
                    Line::styled("[ Connect Cloudflare ]", caps.tok(Tok::Accent)),
                ],
                40,
                7,
                buf,
            ),
            LabView::JoinCode => overlay("Join code", qr(state, caps), 56, 20, buf),
            LabView::EdgeEditor(id) => {
                let lines = match state.edge(id) {
                    Some(e) => vec![
                        Line::raw(format!("{} -> {}: {}", e.from, e.to, access(e.forward))),
                        Line::raw(format!("{} -> {}: {}", e.to, e.from, access(e.reverse))),
                    ],
                    None => vec![Line::raw("unknown edge")],
                };
                overlay("Edge policy", lines, 50, 6, buf)
            }
            LabView::Help => overlay(
                "Help",
                [
                    "Enter  open",
                    "n      new session",
                    "w      watch",
                    "p      pin",
                    "Ctrl+O attach",
                    "Ctrl+] return",
                    "?      help",
                ]
                .map(Line::raw)
                .to_vec(),
                40,
                10,
                buf,
            ),
            _ => {}
        }

        let online = state
            .devices
            .iter()
            .filter(|d| matches!(d.status, DeviceStatus::Online | DeviceStatus::Local))
            .count();
        Paragraph::new(Line::styled(
            format!(
                " {} {online}/{} devices",
                caps.glyph("●", "*"),
                state.devices.len()
            ),
            caps.tok(Tok::Warn),
        ))
        .render(status, buf);
        Paragraph::new(vec![
            Line::raw(" Enter Open     n New session     w Watch · read-only"),
            Line::raw(" p Pin / unpin  s Agent tasks     d Stop · confirm"),
        ])
        .render(footer, buf);
    }
}
