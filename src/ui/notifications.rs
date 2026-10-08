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

pub(super) const TRANSITION: Duration = Duration::from_millis(260);
const FRAME: Duration = Duration::from_millis(33);

#[derive(Clone, Copy)]
pub(super) struct Appearance {
    pub opacity: f32,
    pub offset: u16,
}
fn ease(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}
pub(super) fn appearance(
    start: Option<Instant>,
    deadline: Option<Instant>,
    now: Instant,
) -> Appearance {
    let opacity = match (start, deadline) {
        (Some(start), Some(end)) => {
            ease(now.saturating_duration_since(start).as_secs_f32() / TRANSITION.as_secs_f32()).min(
                ease(end.saturating_duration_since(now).as_secs_f32() / TRANSITION.as_secs_f32()),
            )
        }
        _ => 1.0,
    };
    Appearance {
        opacity,
        offset: ((1.0 - opacity) * 8.0).round() as u16,
    }
}
pub(super) fn animating(start: Option<Instant>, deadline: Option<Instant>, now: Instant) -> bool {
    matches!((start, deadline), (Some(start), Some(end)) if now < end &&
        (now.saturating_duration_since(start) < TRANSITION || end.saturating_duration_since(now) < TRANSITION))
}
pub(super) fn poll_interval(
    start: Option<Instant>,
    deadline: Option<Instant>,
    now: Instant,
) -> Duration {
    if animating(start, deadline, now) {
        FRAME
    } else if let Some(end) = deadline {
        end.checked_sub(TRANSITION)
            .unwrap_or(end)
            .saturating_duration_since(now)
            .min(Duration::from_millis(100))
            .max(Duration::from_millis(1))
    } else {
        Duration::from_millis(100)
    }
}
fn fade(style: Style, opacity: f32) -> Style {
    if std::env::var_os("NO_COLOR").is_some() {
        return style;
    }
    if opacity >= 1.0 && style.fg.is_none_or(|color| color == Color::Reset) {
        return style;
    }
    let (r, g, b) = match style.fg.unwrap_or(Color::Reset) {
        Color::Cyan => (80, 200, 210),
        Color::Green => (110, 200, 120),
        Color::Yellow => (220, 180, 70),
        Color::Red => (220, 90, 90),
        _ => (208, 208, 208),
    };
    let blend = |v: u8| (64.0 + (v as f32 - 64.0) * opacity).round() as u8;
    style.fg(Color::Rgb(blend(r), blend(g), blend(b)))
}

pub(super) fn render(
    frame: &mut Frame<'_>,
    message: &str,
    kind: Kind,
    available: Rect,
    appearance: Appearance,
) {
    use ratatui::widgets::Widget;
    if !visible(message) || appearance.opacity <= 0.0 {
        return;
    }
    let text = safe_text(message);
    let rect = area(&text, available);
    if rect.is_empty() {
        return;
    }
    // Compose at fixed dimensions, then clip the translated panel to the workspace.
    // Text wrapping stays stable throughout the animation and never touches the sidebar/footer.
    let local = Rect::new(0, 0, rect.width, rect.height);
    let mut buffer = ratatui::buffer::Buffer::empty(local);
    let style = fade(kind.style(), appearance.opacity);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(if ascii() {
            BorderType::Plain
        } else {
            BorderType::Rounded
        })
        .border_style(style)
        .title(Line::from(Span::styled(
            format!(" {} ", kind.label()),
            style.add_modifier(Modifier::BOLD),
        )))
        .padding(Padding::horizontal(1));
    let inner = block.inner(local);
    let paragraph = Paragraph::new(text)
        .wrap(Wrap { trim: false })
        .style(fade(Style::default(), appearance.opacity));
    let clipped = paragraph.line_count(inner.width) > inner.height as usize;
    block.render(local, &mut buffer);
    paragraph.render(inner, &mut buffer);
    if clipped {
        Paragraph::new(if ascii() { "..." } else { "…" })
            .style(style)
            .alignment(ratatui::layout::Alignment::Right)
            .render(
                Rect::new(
                    inner.right() - 3.min(inner.width),
                    inner.bottom() - 1,
                    3.min(inner.width),
                    1,
                ),
                &mut buffer,
            );
    }
    let bounds = available.intersection(frame.area());
    let target = frame.buffer_mut();
    for y in 0..rect.height {
        for x in 0..rect.width {
            let position =
                ratatui::layout::Position::new(rect.x + x + appearance.offset, rect.y + y);
            if bounds.contains(position) {
                target[position] = buffer[(x, y)].clone();
            }
        }
    }
}
