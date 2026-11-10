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
    crate::sessions::viewer_ascii()
}
/// Field separator: ` · `, or ` | ` where only ASCII is safe.
pub(super) fn sep() -> &'static str {
    if ascii() {
        " | "
    } else {
        " · "
    }
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
pub(super) fn ok() -> Style {
    tint(Color::Green)
}
pub(super) fn warn() -> Style {
    tint(Color::Yellow)
}
pub(super) fn err() -> Style {
    tint(Color::Red)
}
pub(super) fn info() -> Style {
    tint(Color::LightBlue)
}
/// Provider badge color by provider id.
pub(super) fn provider(name: &str) -> Style {
    match name {
        "claude" => tint(Color::Yellow),
        "codex" => tint(Color::LightBlue),
        _ => Style::default(),
    }
}
/// Stable identity hue for a key (device id, session id): FNV-1a, so a device
/// or session keeps its color across views and restarts.
pub(super) fn swatch_index(key: &str) -> u8 {
    let hash = key.bytes().fold(0xcbf29ce484222325u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x100000001b3)
    });
    (hash % 8) as u8
}
pub(super) fn swatch(key: &str) -> Style {
    const IDS: [Color; 8] = [
        Color::Red,
        Color::Yellow,
        Color::Green,
        Color::Cyan,
        Color::LightCyan,
        Color::LightBlue,
        Color::Magenta,
        Color::LightMagenta,
    ];
    tint(IDS[usize::from(swatch_index(key))])
}
/// Selected-row style: reversed in the focused panel; unfocused rows rely on
/// the selection bar.
pub(super) fn selection(focused: bool) -> Style {
    if focused {
        Style::default().add_modifier(Modifier::REVERSED)
    } else {
        Style::default()
    }
}
