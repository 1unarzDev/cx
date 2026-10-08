//! Session menu definitions are shared by dispatch, shortcut lookup and rendering.
use super::{
    accent, ascii, block, muted, popup, App, Block, BorderType, Borders, Clear, Dialog, Frame,
    Modifier, Paragraph, Rect, Wrap,
};

pub(super) struct SessionChoice {
    pub label: &'static str,
    pub provider: &'static str,
    pub key: char,
}
pub(super) const HOST: &[SessionChoice] = &[
    SessionChoice {
        label: "[c] Claude",
        provider: "claude",
        key: 'c',
    },
    SessionChoice {
        label: "[x] Codex",
        provider: "codex",
        key: 'x',
    },
    SessionChoice {
        label: "[s] Shell",
        provider: "shell",
        key: 's',
    },
    SessionChoice {
        label: "[d] Devcontainer",
        provider: "container",
        key: 'd',
    },
];
pub(super) const CONTAINER: &[SessionChoice] = &[SessionChoice {
    label: "[s] Shell",
    provider: "shell",
    key: 's',
}];
pub(super) fn session_choices(dialog: &Dialog) -> Option<&'static [SessionChoice]> {
    match dialog {
        Dialog::SessionChooser(..) => Some(HOST),
        Dialog::ContainerProvider(..) => Some(CONTAINER),
        _ => None,
    }
}

pub(super) fn container_action_labels(c: &crate::containers::Container) -> Vec<&'static str> {
    if !c.devcontainer && !c.allowed {
        return vec!["Enable access"];
    }
    let mut actions = if c.state == "running" {
        vec![
            if c.devcontainer {
                "Devcontainer terminal"
            } else {
                "Shell"
            },
            "Files · read-only",
            "Stop",
        ]
    } else {
        vec!["Start"]
    };
    if !c.devcontainer {
        actions.push("Disable access");
    }
    actions
}
pub(super) fn render_session_chooser(
    frame: &mut Frame<'_>,
    app: &App,
    area: Rect,
    dialog: &Dialog,
    title: &str,
    detail: &str,
) {
    let host = matches!(dialog, Dialog::SessionChooser(..));
    let choices = session_choices(dialog).expect("session chooser dialog");
    let columns = if area.width < 68 { 2 } else { choices.len() };
    let rows = choices.len().div_ceil(columns);
    let rect = popup(area, 78, (7 + rows * 3) as u16);
    frame.render_widget(Clear, rect);
    frame.render_widget(block(title.into(), true), rect);
    let inner = Rect::new(
        rect.x + 2,
        rect.y + 1,
        rect.width.saturating_sub(4),
        rect.height.saturating_sub(2),
    );
    frame.render_widget(
        Paragraph::new(detail).wrap(Wrap { trim: false }),
        Rect::new(inner.x, inner.y, inner.width, 3),
    );
    for (i, choice) in choices.iter().enumerate() {
        let column = (i % columns) as u16;
        let start = inner.width * column / columns as u16;
        let end = inner.width * (column + 1) / columns as u16;
        let cell = Rect::new(
            inner.x + start,
            inner.y + 3 + (i / columns) as u16 * 3,
            (end - start).saturating_sub(1),
            3,
        );
        let available = match dialog {
            Dialog::SessionChooser(d, _) => app.provider_choices(*d).contains(&choice.provider),
            _ => true,
        };
        let active = app.dialog_selected == i;
        let style = if active {
            accent().add_modifier(Modifier::BOLD)
        } else if available {
            accent()
        } else {
            muted()
        };
        let label = match dialog {
            Dialog::ContainerProvider(device, scope) => {
                format!("[s] {}", container_label(app, *device, scope))
            }
            _ => choice.label.to_string(),
        };
        frame.render_widget(
            Paragraph::new(label)
                .alignment(ratatui::layout::Alignment::Center)
                .style(style)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(if active {
                            if ascii() {
                                " > "
                            } else {
                                " › "
                            }
                        } else {
                            ""
                        })
                        .border_type(if ascii() {
                            BorderType::Plain
                        } else {
                            BorderType::Rounded
                        })
                        .border_style(if active { style } else { muted() }),
                ),
            cell,
        );
    }
    let help = if !host {
        "s shell · Enter start · Esc cancel"
    } else if columns == 2 {
        "h/l choose · Enter start · Esc cancel"
    } else {
        "c/x/s/d quick pick · h/l choose · Enter · Esc"
    };
    frame.render_widget(
        Paragraph::new(help).style(muted()),
        Rect::new(inner.x, inner.y + 3 + (rows as u16) * 3, inner.width, 1),
    );
}

/// Container labels use discovery evidence, never a container's name or the agent provider field.
pub(super) fn container_label(
    app: &App,
    device: usize,
    scope: &super::ContainerScope,
) -> &'static str {
    match app.containers.get(&device).and_then(|items| {
        items.iter().find(|c| {
            c.engine == scope.engine && c.id == scope.id && c.started_at == scope.started_at
        })
    }) {
        Some(c) if c.devcontainer => "Devcontainer",
        Some(_) => "Docker",
        None => "Container",
    }
}
pub(super) fn session_label<'a>(app: &App, device: usize, session: &'a super::Session) -> &'a str {
    if session.provider == "shell" {
        if let Some(scope) = &session.container {
            return match container_label(app, device, scope) {
                "Devcontainer" => "devcontainer",
                "Container" => "container",
                other => other,
            };
        }
    }
    &session.provider
}
