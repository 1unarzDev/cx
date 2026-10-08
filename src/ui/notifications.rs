//! Non-interactive notifications. Geometry and presentation stay out of action dispatch.
use super::*;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Kind {
    #[default]
    Info,
    Success,
    Warning,
    Error,
}
impl Kind {
    fn label(self) -> &'static str {
        match self {
            Self::Info => "Info",
            Self::Success => "Success",
            Self::Warning => "Warning",
            Self::Error => "Error",
        }
    }
    fn style(self) -> Style {
        tint(match self {
            Self::Info => Color::Cyan,
            Self::Success => Color::Green,
            Self::Warning => Color::Yellow,
            Self::Error => Color::Red,
        })
    }
}

/// The area ends above the footer, so search, status and keyboard hints stay visible.
pub(super) fn area(message: &str, available: Rect) -> Rect {
    let width = (message
        .lines()
        .map(|line| Line::from(line).width())
        .max()
        .unwrap_or(0)
        + 4)
    .clamp(26, 60)
    .min(available.width.saturating_sub(2) as usize) as u16;
    if width < 6 || available.height < 4 {
        return Rect::default();
    }
    let lines = Paragraph::new(message)
        .wrap(Wrap { trim: false })
        .line_count(width - 4);
    let height = (lines.max(1) + 2)
        .min(8)
        .min(available.height.saturating_sub(1) as usize) as u16;
    Rect::new(
        available.right() - width - 1,
        available.bottom() - height - 1,
        width,
        height,
    )
}

pub(super) fn visible(message: &str) -> bool {
    !message.is_empty() && message != "Ctrl+P actions · ? help"
}

pub(super) fn render(frame: &mut Frame<'_>, message: &str, kind: Kind, available: Rect) {
    if !visible(message) {
        return;
    }
    let text = safe_text(message);
    let rect = area(&text, available);
    if rect.is_empty() {
        return;
    }
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(if ascii() {
            BorderType::Plain
        } else {
            BorderType::Rounded
        })
        .border_style(kind.style())
        .title(Line::from(Span::styled(
            format!(" {} ", kind.label()),
            kind.style().add_modifier(Modifier::BOLD),
        )))
        .padding(Padding::horizontal(1));
    let inner = block.inner(rect);
    let paragraph = Paragraph::new(text).wrap(Wrap { trim: false });
    let clipped = paragraph.line_count(inner.width) > inner.height as usize;
    frame.render_widget(Clear, rect);
    frame.render_widget(block, rect);
    frame.render_widget(paragraph, inner);
    if clipped {
        frame.render_widget(
            Paragraph::new(if ascii() { "..." } else { "…" })
                .style(kind.style())
                .alignment(ratatui::layout::Alignment::Right),
            Rect::new(
                inner.right() - 3.min(inner.width),
                inner.bottom() - 1,
                3.min(inner.width),
                1,
            ),
        );
    }
}
