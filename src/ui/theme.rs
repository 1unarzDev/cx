//! Shared styles and terminal capability switches (NO_COLOR, CX_ASCII, dumb TERM).
//! Restyle here; call sites use these helpers instead of reading the environment.
use super::*;

pub(super) fn block(title: String, focused: bool) -> Block<'static> {
    Block::default()
        .title(Line::from(Span::styled(
            if focused {
                format!(" {} {title} ", if ascii() { ">" } else { "▸" })
            } else {
                format!(" {title} ")
            },
            accent().add_modifier(Modifier::BOLD),
        )))
        .border_type(if ascii() {
            BorderType::Plain
        } else {
            BorderType::Rounded
        })
        .borders(Borders::ALL)
        .border_style(if focused {
            Style::default().add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        })
}
pub(super) fn selected_style() -> Style {
    accent().add_modifier(Modifier::UNDERLINED | Modifier::BOLD)
}
pub(super) fn accent() -> Style {
    if no_color() {
        Style::default()
    } else {
        Style::default().fg(Color::Cyan)
    }
}
pub(super) fn muted() -> Style {
    Style::default().add_modifier(Modifier::DIM)
}
pub(super) fn ascii() -> bool {
    std::env::var_os("CX_ASCII").is_some() || std::env::var("TERM").is_ok_and(|v| v == "dumb")
}
pub(super) fn tint(color: Color) -> Style {
    if no_color() {
        Style::default()
    } else {
        Style::default().fg(color)
    }
}
/// True when NO_COLOR is set (any value, per no-color.org): styles drop colors.
pub(super) fn no_color() -> bool {
    std::env::var_os("NO_COLOR").is_some()
}
