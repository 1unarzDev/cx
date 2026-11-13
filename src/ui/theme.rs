//! Semantic theme tokens, glyph sets and the focus treatment (design spec
//! sections 4 and 5). Widgets ask for a token ([`Tok`]); this module maps it to
//! a color for the detected depth and background ([`super::caps`]), or to an
//! attribute substitute under `NO_COLOR`. `bg` is always the terminal default:
//! only selections paint a background, so transparency keeps working.
use super::caps::{caps, Background, Caps, ColorDepth};
use super::*;
use ratatui::symbols::border;

/// Semantic color tokens (spec table 4.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Tok {
    /// Secondary text, ages.
    Muted,
    /// Borders, separators, hints.
    Faint,
    /// Focus border, focused title chip, cursor bar.
    Accent,
    /// Selected row in the focused panel (background).
    AccentBg,
    /// Selected row in an unfocused panel (background).
    SelectDimBg,
    Ok,
    Warn,
    Err,
    Info,
    /// Inline code and rendered math in previews.
    Code,
    /// Search matches that are not the current one.
    Match,
    /// Per-device / per-session identity hue, `0..SWATCHES`.
    Id(u8),
}

/// Number of identity hues.
pub(super) const SWATCHES: u8 = 8;

type Rgb = (u8, u8, u8);
/// One token row: truecolor dark/light, xterm-256 dark/light, 16-color.
#[derive(Clone, Copy)]
pub(super) struct Row {
    pub(super) dark: Rgb,
    pub(super) light: Rgb,
    pub(super) x256: (u8, u8),
    /// `None` keeps the terminal default (or relies on an attribute).
    pub(super) ansi: Option<Color>,
}

const fn row(dark: u32, light: u32, x256: (u8, u8), ansi: Option<Color>) -> Row {
    Row {
        dark: ((dark >> 16) as u8, (dark >> 8) as u8, dark as u8),
        light: ((light >> 16) as u8, (light >> 8) as u8, light as u8),
        x256,
        ansi,
    }
}

/// Identity hues: OKLCH L 0.74 / C 0.10 (dark) and L 0.47 / C 0.11 (light),
/// hue 25..340 in 8 steps; every one reaches 4.5:1 on #282c34 and #f0f0f0.
/// The 16-color column is the nearest ANSI hue of each swatch (red, yellow,
/// green, cyan, bright cyan, bright blue, magenta, bright magenta), so a device
/// keeps the same hue family at every depth. It uses the same eight ANSI slots
/// as the spec's rotation except bright blue for plain blue, which fails
/// contrast on dark backgrounds (P9).
const IDS: [Row; 8] = [
    row(0xe3928b, 0x8e3e3a, (174, 131), Some(Color::Red)),
    row(0xd3a062, 0x814d00, (179, 130), Some(Color::Yellow)),
    row(0xa3b46b, 0x536304, (143, 64), Some(Color::Green)),
    row(0x68bf9b, 0x006d4b, (79, 29), Some(Color::Cyan)),
    row(0x51bcce, 0x006a7d, (74, 30), Some(Color::LightCyan)),
    row(0x79b0e8, 0x215d96, (110, 25), Some(Color::LightBlue)),
    row(0xaf9ee4, 0x604c91, (146, 61), Some(Color::Magenta)),
    row(0xd692bf, 0x823f6e, (175, 125), Some(Color::LightMagenta)),
];

/// The one table of the shared tokens (the design's views read it too).
pub(super) fn spec(t: Tok) -> Row {
    match t {
        Tok::Muted => row(0x8b93a1, 0x5c6470, (246, 241), Some(Color::DarkGray)),
        Tok::Faint => row(0x4a515c, 0xc3c8d0, (239, 251), Some(Color::DarkGray)),
        Tok::Accent => row(0x6cc4d6, 0x0b7285, (80, 30), Some(Color::Cyan)),
        // 16-color selections use REVERSED instead of a background.
        Tok::AccentBg => row(0x1d3a42, 0xd3eef3, (23, 195), None),
        Tok::SelectDimBg => row(0x262b33, 0xeceff3, (236, 255), None),
        // Light ok/warn are darker than the draft table so they reach 4.5:1 on #f0f0f0.
        Tok::Ok => row(0x8fcf9d, 0x237a35, (114, 28), Some(Color::Green)),
        Tok::Warn => row(0xe3c275, 0x8f5b00, (179, 130), Some(Color::Yellow)),
        Tok::Err => row(0xe8837f, 0xc92a2a, (210, 160), Some(Color::Red)),
        // 16-color: bright blue, plain blue is unreadable on dark backgrounds (P9).
        Tok::Info => row(0x8fb0e0, 0x1c64b8, (110, 25), Some(Color::LightBlue)),
        // Plain blue fails contrast on dark backgrounds (P9): bright blue + bold.
        Tok::Code => row(0xd7a6c9, 0x8a3d73, (182, 96), Some(Color::Magenta)),
        Tok::Match => row(0xf0c674, 0x8a5a00, (221, 94), Some(Color::Yellow)),
        Tok::Id(i) => IDS[(i % SWATCHES) as usize],
    }
}

/// The color for `t` under `caps`, or `None` when the depth has no color for it.
pub(super) fn token_color(t: Tok, caps: Caps) -> Option<Color> {
    let r = spec(t);
    let light = caps.background == Background::Light;
    match caps.color {
        ColorDepth::None => None,
        ColorDepth::Ansi16 => r.ansi,
        ColorDepth::Ansi256 => Some(Color::Indexed(if light { r.x256.1 } else { r.x256.0 })),
        ColorDepth::TrueColor => {
            let (red, green, blue) = if light { r.light } else { r.dark };
            Some(Color::Rgb(red, green, blue))
        }
    }
}

/// Truecolor RGB of a token for the current background (for blends and fades).
pub(super) fn token_rgb(t: Tok) -> (u8, u8, u8) {
    let r = spec(t);
    if caps().background == Background::Light {
        r.light
    } else {
        r.dark
    }
}

/// Attribute that stands in for `t` when the depth cannot show its color.
fn substitute(t: Tok, depth: ColorDepth) -> Modifier {
    match (t, depth) {
        (Tok::Muted | Tok::Faint, ColorDepth::None) => Modifier::DIM,
        (Tok::Accent | Tok::Err, ColorDepth::None) => Modifier::BOLD,
        (Tok::Info, ColorDepth::Ansi16) => Modifier::BOLD,
        // Directories are always bold, so they never rely on hue alone.
        _ => Modifier::empty(),
    }
}

/// Foreground style for a token, with its attribute substitute.
pub(super) fn tok_for(t: Tok, caps: Caps) -> Style {
    let style = Style::default().add_modifier(substitute(t, caps.color));
    match token_color(t, caps) {
        Some(color) => style.fg(color),
        None => style,
    }
}
pub(super) fn tok(t: Tok) -> Style {
    tok_for(t, caps())
}

pub(super) fn accent() -> Style {
    tok(Tok::Accent)
}
pub(super) fn muted() -> Style {
    tok(Tok::Muted)
}
pub(super) fn faint() -> Style {
    tok(Tok::Faint)
}
pub(super) fn ok() -> Style {
    tok(Tok::Ok)
}
pub(super) fn warn() -> Style {
    tok(Tok::Warn)
}
pub(super) fn err() -> Style {
    tok(Tok::Err)
}
pub(super) fn info() -> Style {
    tok(Tok::Info)
}
/// Stable identity hue for a key (device id, session id): FNV-1a, so a device
/// keeps its color across views and restarts.
pub(super) fn swatch_index(key: &str) -> u8 {
    let hash = key.bytes().fold(0xcbf29ce484222325u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x100000001b3)
    });
    (hash % u64::from(SWATCHES)) as u8
}

/// Selected-row style: `accent.bg` in the focused panel, `select.dim.bg` elsewhere.
/// 16-color and NO_COLOR use REVERSED for the focused row and nothing (the bar
/// marker carries it) for unfocused ones. The reversed row also resets the
/// foreground, so token-colored cells (an accent state, a dim age) do not turn
/// into colored blocks inside the bar: the row reads as one uniform chip.
///
/// v0.2.2: no background in any tier (terminals are often transparent, and a painted
/// row reads as a slab): the focused row is bold accent ink (bold and underlined
/// without colour), an unfocused one is plain (its bar marker carries it).
pub(super) fn selection_for(focused: bool, caps: Caps) -> Style {
    if !focused {
        return Style::default();
    }
    match token_color(Tok::Accent, caps) {
        Some(c) => Style::default().fg(c).add_modifier(Modifier::BOLD),
        None => Style::default().add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
    }
}
/// Selection style for a panel row; `focused` is downgraded while an overlay
/// owns input (see [`set_panels_dimmed`]).
pub(super) fn selection(focused: bool) -> Style {
    selection_for(focused && !panels_dimmed(), caps())
}
/// The focused selection (lists that only show a selection while focused).
/// Still dims behind an overlay.
pub(super) fn selected_style() -> Style {
    selection(true)
}

/// Per-frame render state shared by the theme helpers.
#[derive(Default)]
struct FrameState {
    /// An overlay, dialog or palette owns input: panels behind it draw unfocused.
    dimmed: bool,
    /// Collect block titles for [`fit_titles`] (only inside a managed frame).
    collecting: bool,
    /// Focused/unfocused titles created this frame, as rendered cell symbols.
    titles: Vec<Vec<String>>,
}
thread_local! {
    static FRAME: std::cell::RefCell<FrameState> = std::cell::RefCell::new(FrameState::default());
}
/// While `true`, [`block`], [`selection`] and [`selection_symbol`] treat every
/// caller as unfocused, so the panels behind a modal overlay never look focused
/// next to it (spec 5: exactly one focused region). Overlays are drawn after
/// this is reset to `false`.
pub(super) fn set_panels_dimmed(dimmed: bool) {
    FRAME.with(|f| f.borrow_mut().dimmed = dimmed);
}
pub(super) fn panels_dimmed() -> bool {
    FRAME.with(|f| f.borrow().dimmed)
}
/// Start a managed frame: forget titles and content regions, start collecting
/// titles for [`fit_titles`].
pub(super) fn begin_frame(dimmed: bool) {
    clear_protected_content();
    FRAME.with(|f| {
        *f.borrow_mut() = FrameState {
            dimmed,
            collecting: true,
            titles: Vec::new(),
        }
    });
}
/// End a managed frame: stop collecting and return the collected titles.
fn end_frame() -> Vec<Vec<String>> {
    FRAME.with(|f| {
        let mut f = f.borrow_mut();
        f.dimmed = false;
        f.collecting = false;
        std::mem::take(&mut f.titles)
    })
}
fn register_title(title: &Line<'_>) {
    FRAME.with(|f| {
        let mut f = f.borrow_mut();
        if !f.collecting || f.titles.len() >= 256 {
            return;
        }
        let width = title.width();
        if width == 0 || width > 512 {
            return;
        }
        let mut buf = ratatui::buffer::Buffer::empty(Rect::new(0, 0, width as u16, 1));
        ratatui::widgets::Widget::render(title.clone(), buf.area, &mut buf);
        f.titles
            .push(buf.content.iter().map(|c| c.symbol().to_owned()).collect());
    });
}
/// Horizontal border symbol continuing a top-left corner, for our border sets.
fn top_run(corner: &str) -> Option<(&'static str, &'static [&'static str])> {
    Some(match corner {
        "┏" => ("━", &["┓"]),
        "╭" => ("─", &["╮"]),
        "┌" => ("─", &["┐"]),
        "#" => ("=", &["#"]),
        "+" => ("-", &["+"]),
        _ => return None,
    })
}
/// Keep panel titles from swallowing the top border at narrow widths: a title
/// wider than the block minus 4 cells is cut to that width and ends with the
/// glyph-set ellipsis, so border cells stay visible after the chip. Runs on the
/// finished frame (titles are clipped by ratatui without an ellipsis).
pub(super) fn fit_titles(buf: &mut ratatui::buffer::Buffer, area: Rect) {
    let titles = end_frame();
    if titles.is_empty() {
        return;
    }
    let ellipsis = glyphs().ellipsis;
    for y in area.top()..area.bottom() {
        let mut x = area.left();
        while x + 1 < area.right() {
            let Some((run, rights)) = top_run(buf[(x, y)].symbol()) else {
                x += 1;
                continue;
            };
            let start = x + 1;
            // Longest registered title drawn right after this corner.
            let mut best: Option<(u16, usize)> = None;
            for t in &titles {
                let mut k = 0u16;
                while usize::from(k) < t.len()
                    && start + k < area.right()
                    && buf[(start + k, y)].symbol() == t[usize::from(k)]
                {
                    k += 1;
                }
                if k >= 2 && best.is_none_or(|(bk, _)| k > bk) {
                    best = Some((k, t.len()));
                }
            }
            let Some((k, full)) = best else {
                x += 1;
                continue;
            };
            // Find the right corner after the drawn title.
            let mut end = start + k;
            while end < area.right() && buf[(end, y)].symbol() == run {
                end += 1;
            }
            if end >= area.right() || !rights.contains(&buf[(end, y)].symbol()) {
                x = start + k;
                continue;
            }
            let width = end - x + 1;
            let cap = width.saturating_sub(4);
            if usize::from(k) < full || k > cap {
                let style = buf[(x, y)].style();
                let keep = cap.saturating_sub(1);
                // Do not split a wide character: back off over continuation cells.
                let mut cut = start + keep;
                while cut > start && buf[(cut, y)].symbol().is_empty() {
                    cut -= 1;
                }
                if cap > 0 {
                    let chip = buf[(cut.saturating_sub(1).max(start), y)].style();
                    buf[(cut, y)].set_symbol(ellipsis).set_style(chip);
                    cut += 1;
                }
                for cx in cut..end {
                    buf[(cx, y)].reset();
                    buf[(cx, y)].set_symbol(run).set_style(style);
                }
            }
            x = end + 1;
        }
    }
}
/// Glyph set (spec table 4.2). ASCII is chosen by `CX_ASCII`, `TERM=dumb|linux`
/// or a non-UTF-8 locale.
#[allow(dead_code)]
pub(super) struct Glyphs {
    /// ok, warn, err, info, unknown.
    pub status: [&'static str; 5],
    pub bar_focused: &'static str,
    pub bar_dim: &'static str,
    pub both: &'static str,
    pub right: &'static str,
    pub left: &'static str,
    pub via: &'static str,
    pub blocked: &'static str,
    pub mesh: &'static str,
    pub jump: &'static str,
    pub lan: &'static str,
    pub local: &'static str,
    pub spark: &'static [&'static str],
    pub bars: &'static [&'static str],
    pub spinner: &'static [&'static str],
    pub tree_mid: &'static str,
    pub tree_end: &'static str,
    pub tree_pipe: &'static str,
    pub ellipsis: &'static str,
    pub sep: &'static str,
}
const UNICODE: Glyphs = Glyphs {
    status: ["●", "◐", "✕", "◌", "·"],
    bar_focused: "▌ ",
    bar_dim: "▏ ",
    both: "⇄",
    right: "→",
    left: "←",
    via: "⇢",
    blocked: "⊘",
    mesh: "◆",
    jump: "↳",
    lan: "~",
    local: "⌂",
    spark: &["▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"],
    bars: &["▏", "▎", "▍", "▌", "▋", "▊", "▉", "█"],
    spinner: &["◐", "◓", "◑", "◒"],
    tree_mid: "├",
    tree_end: "└",
    tree_pipe: "│",
    ellipsis: "…",
    sep: "·",
};
const ASCII: Glyphs = Glyphs {
    status: ["+", "~", "x", ".", "-"],
    bar_focused: "> ",
    bar_dim: ": ",
    both: "<>",
    right: "->",
    left: "<-",
    via: "~>",
    blocked: "x",
    mesh: "*",
    jump: "^",
    lan: "~",
    local: "@",
    spark: &["_", ".", "-", "=", "#"],
    bars: &["#"],
    spinner: &["-", "\\", "|", "/"],
    tree_mid: "|-",
    tree_end: "`-",
    tree_pipe: "|",
    ellipsis: "~",
    sep: "|",
};
pub(super) fn glyphs_for(caps: Caps) -> &'static Glyphs {
    if caps.unicode {
        &UNICODE
    } else {
        &ASCII
    }
}
pub(super) fn glyphs() -> &'static Glyphs {
    glyphs_for(caps())
}

/// ASCII borders: focused `#=#`, unfocused `+-+`.
const ASCII_FOCUSED: border::Set = border::Set {
    top_left: "#",
    top_right: "#",
    bottom_left: "#",
    bottom_right: "#",
    vertical_left: "#",
    vertical_right: "#",
    horizontal_top: "=",
    horizontal_bottom: "=",
};
const ASCII_PLAIN: border::Set = border::Set {
    top_left: "+",
    top_right: "+",
    bottom_left: "+",
    bottom_right: "+",
    vertical_left: "|",
    vertical_right: "|",
    horizontal_top: "-",
    horizontal_bottom: "-",
};

/// Border glyphs for a panel: heavy when focused, rounded otherwise.
pub(super) fn border_set_for(focused: bool, caps: Caps) -> border::Set {
    match (caps.unicode, focused) {
        (true, true) => border::THICK,
        (true, false) => border::ROUNDED,
        (false, true) => ASCII_FOCUSED,
        (false, false) => ASCII_PLAIN,
    }
}
pub(super) fn border_set(focused: bool) -> border::Set {
    border_set_for(focused && !panels_dimmed(), caps())
}
/// Border glyphs for overlays and toasts (never "focused" panels).
pub(super) fn overlay_border() -> border::Set {
    border_set_for(false, caps())
}

/// Panel frame with the four-cue focus treatment (spec section 5): heavy accent
/// border + reversed title chip when focused; rounded faint border + muted title
/// otherwise. ASCII: `#=#` + `[ title ]` reversed vs `+-+`.
pub(super) fn block_for(title: String, focused: bool, caps: Caps) -> Block<'static> {
    let (title, title_style, border_style) = if focused {
        // (Bold underlined accent ink: a reversed chip is a painted cell.)
        let chip = tok_for(Tok::Accent, caps).add_modifier(Modifier::UNDERLINED | Modifier::BOLD);
        let text = if caps.unicode {
            format!(" {title} ")
        } else {
            format!("[ {title} ]")
        };
        let border = match token_color(Tok::Accent, caps) {
            Some(c) => Style::default().fg(c),
            None => Style::default().add_modifier(Modifier::BOLD),
        };
        (text, chip, border)
    } else {
        (
            format!(" {title} "),
            tok_for(Tok::Muted, caps),
            tok_for(Tok::Faint, caps),
        )
    };
    let title = Line::from(Span::styled(title, title_style));
    register_title(&title);
    Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_set(border_set_for(focused, caps))
        .border_style(border_style)
}
/// Panel frame for the current capabilities. `focused` is downgraded while an
/// overlay owns input, so only the overlay shows the focused treatment.
pub(super) fn block(title: String, focused: bool) -> Block<'static> {
    block_for(title, focused && !panels_dimmed(), caps())
}

/// ASCII substitute for a UI glyph, or `None` when the symbol is not a known
/// UI glyph (user content is left alone).
pub(super) fn ascii_glyph(symbol: &str) -> Option<&'static str> {
    Some(match symbol {
        "·" | "•" | "∙" => ".",
        "…" => "~",
        "●" | "◆" => "*",
        "○" | "◌" | "◦" => "o",
        "◐" | "◓" | "◑" | "◒" => "~",
        "✕" | "×" | "⊘" => "x",
        "→" | "⇢" | "›" | "▸" | "▶" => ">",
        "←" | "‹" | "◂" => "<",
        "↑" => "^",
        "↓" => "v",
        "↔" | "⇄" => "~",
        "↳" => "^",
        "⌂" => "@",
        "─" | "━" | "═" | "╌" => "-",
        "│" | "┃" | "║" | "╎" => "|",
        "┌" | "┐" | "└" | "┘" | "╭" | "╮" | "╰" | "╯" | "┏" | "┓" | "┗" | "┛" | "├" | "┤" | "┬"
        | "┴" | "┼" | "┣" | "┫" | "┳" | "┻" | "╋" => "+",
        "▌" | "▏" | "▎" | "▍" | "▋" | "▊" | "▉" | "█" | "▇" | "▆" | "▅" => {
            "#"
        }
        "▁" | "▂" => "_",
        "▃" | "▄" => "=",
        "✓" | "✔" => "+",
        "⏏" => "^",
        "—" | "–" => "-",
        "“" | "”" => "\"",
        "‘" | "’" => "'",
        _ => return None,
    })
}
thread_local! {
    /// Content regions of the frame being drawn, with the symbols they held
    /// right after the content was rendered (see [`protect_content`]).
    static CONTENT: std::cell::RefCell<Vec<(Rect, Vec<String>)>> =
        const { std::cell::RefCell::new(Vec::new()) };
}
/// Mark `area` as user content (file preview text, the live terminal preview):
/// the ASCII pass leaves it alone, so quotes, dashes and box drawing inside a
/// document or a remote screen are shown as written. The symbols are
/// snapshotted, so chrome drawn over the region later (an overlay, a toast) is
/// still converted. Content renderers emit their own decorations in ASCII
/// (markdown rules and bullets check [`ascii`]). No-op outside ASCII mode.
pub(super) fn protect_content(buf: &ratatui::buffer::Buffer, area: Rect) {
    if !ascii() {
        return;
    }
    let area = area.intersection(buf.area);
    if area.is_empty() {
        return;
    }
    let mut symbols = Vec::with_capacity(usize::from(area.width) * usize::from(area.height));
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            symbols.push(buf[(x, y)].symbol().to_owned());
        }
    }
    CONTENT.with(|c| c.borrow_mut().push((area, symbols)));
}
/// Forget content regions (start of a frame).
pub(super) fn clear_protected_content() {
    CONTENT.with(|c| c.borrow_mut().clear());
}
/// Rewrite known UI glyphs in a rendered buffer to ASCII, except cells of
/// regions registered with [`protect_content`] that still hold their content.
/// Consumes the registered regions.
pub(super) fn asciify(buf: &mut ratatui::buffer::Buffer, area: Rect) {
    let content = CONTENT.with(|c| std::mem::take(&mut *c.borrow_mut()));
    let is_content = |x: u16, y: u16, symbol: &str| {
        content.iter().any(|(r, symbols)| {
            r.contains(ratatui::layout::Position { x, y })
                && symbols[usize::from(y - r.y) * usize::from(r.width) + usize::from(x - r.x)]
                    == symbol
        })
    };
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            let cell = &buf[(x, y)];
            if let Some(sub) = ascii_glyph(cell.symbol()) {
                if !is_content(x, y, cell.symbol()) {
                    buf[(x, y)].set_symbol(sub);
                }
            }
        }
    }
}
pub(super) fn ascii() -> bool {
    !caps().unicode
}
/// True when colors are off (`NO_COLOR`, any value, or `TERM=dumb`).
pub(super) fn no_color() -> bool {
    caps().color == ColorDepth::None
}
/// Blend a token toward the background by `opacity` (1.0 = the token itself).
/// Only truecolor terminals fade; other depths show the token as is.
pub(super) fn faded(t: Tok, opacity: f32) -> Style {
    let caps = caps();
    if caps.color != ColorDepth::TrueColor || opacity >= 1.0 {
        return tok(t);
    }
    let base: f32 = if caps.background == Background::Light {
        240.0
    } else {
        64.0
    };
    let (r, g, b) = token_rgb(t);
    let blend = |v: u8| (base + (f32::from(v) - base) * opacity.max(0.0)).round() as u8;
    tok(t).fg(Color::Rgb(blend(r), blend(g), blend(b)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::caps::with_caps;

    fn at(color: ColorDepth, background: Background) -> Caps {
        Caps {
            color,
            background,
            ..Caps::default()
        }
    }
    const ALL: [Tok; 11] = [
        Tok::Code,
        Tok::Match,
        Tok::Muted,
        Tok::Faint,
        Tok::Accent,
        Tok::AccentBg,
        Tok::SelectDimBg,
        Tok::Ok,
        Tok::Warn,
        Tok::Err,
        Tok::Info,
    ];

    #[test]
    fn tokens_downgrade_by_depth() {
        use ColorDepth::*;
        for t in ALL.into_iter().chain((0..SWATCHES).map(Tok::Id)) {
            for bg in [Background::Dark, Background::Light] {
                assert!(
                    matches!(token_color(t, at(TrueColor, bg)), Some(Color::Rgb(..))),
                    "{t:?}"
                );
                assert!(
                    matches!(token_color(t, at(Ansi256, bg)), Some(Color::Indexed(_))),
                    "{t:?}"
                );
                let c16 = token_color(t, at(Ansi16, bg));
                assert!(
                    !matches!(c16, Some(Color::Rgb(..) | Color::Indexed(_))),
                    "{t:?} 16-color must use a named ANSI color: {c16:?}"
                );
                assert_eq!(token_color(t, at(None, bg)), Option::None, "{t:?}");
            }
        }
        assert_eq!(
            token_color(Tok::Accent, at(TrueColor, Background::Dark)),
            Some(Color::Rgb(0x6c, 0xc4, 0xd6))
        );
        assert_eq!(
            token_color(Tok::Accent, at(TrueColor, Background::Light)),
            Some(Color::Rgb(0x0b, 0x72, 0x85))
        );
        assert_eq!(
            token_color(Tok::Accent, at(Ansi256, Background::Dark)),
            Some(Color::Indexed(80))
        );
        assert_eq!(
            token_color(Tok::Accent, at(Ansi256, Background::Light)),
            Some(Color::Indexed(30))
        );
        // NO_COLOR substitutes keep the information as attributes.
        let none = at(None, Background::Dark);
        assert!(tok_for(Tok::Muted, none)
            .add_modifier
            .contains(Modifier::DIM));
        assert!(tok_for(Tok::Accent, none)
            .add_modifier
            .contains(Modifier::BOLD));
        assert!(tok_for(Tok::Err, none)
            .add_modifier
            .contains(Modifier::BOLD));
        assert!(selection_for(true, none)
            .add_modifier
            .contains(Modifier::BOLD | Modifier::UNDERLINED));
        assert!(selection_for(true, at(Ansi16, Background::Dark))
            .bg
            .is_none());
        assert_eq!(selection_for(false, none), Style::default());
        assert_eq!(
            selection_for(true, at(TrueColor, Background::Dark)).bg,
            Option::<Color>::None
        );
    }

    fn luminance((r, g, b): Rgb) -> f64 {
        let ch = |v: u8| {
            let v = f64::from(v) / 255.0;
            if v <= 0.03928 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * ch(r) + 0.7152 * ch(g) + 0.0722 * ch(b)
    }
    fn contrast(a: Rgb, b: Rgb) -> f64 {
        let (x, y) = (luminance(a), luminance(b));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    }

    #[test]
    fn text_tokens_reach_wcag_aa_on_both_background_ranges() {
        let text = [
            Tok::Muted,
            Tok::Accent,
            Tok::Ok,
            Tok::Warn,
            Tok::Err,
            Tok::Info,
            Tok::Code,
            Tok::Match,
        ];
        for t in text.into_iter().chain((0..SWATCHES).map(Tok::Id)) {
            let r = spec(t);
            for bg in [(0, 0, 0), (0x28, 0x2c, 0x34)] {
                assert!(
                    contrast(r.dark, bg) >= 4.5,
                    "{t:?} dark {:.2}",
                    contrast(r.dark, bg)
                );
            }
            for bg in [(0xff, 0xff, 0xff), (0xf0, 0xf0, 0xf0)] {
                assert!(
                    contrast(r.light, bg) >= 4.5,
                    "{t:?} light {:.2}",
                    contrast(r.light, bg)
                );
            }
        }
    }

    #[test]
    fn swatches_are_distinct_and_stable() {
        let dark: BTreeSet<_> = IDS.iter().map(|r| r.dark).collect();
        let x256: BTreeSet<_> = IDS.iter().map(|r| r.x256.0).collect();
        let x256l: BTreeSet<_> = IDS.iter().map(|r| r.x256.1).collect();
        let ansi: BTreeSet<_> = IDS.iter().map(|r| format!("{:?}", r.ansi)).collect();
        assert_eq!(
            (dark.len(), x256.len(), x256l.len(), ansi.len()),
            (8, 8, 8, 8)
        );
        assert_eq!(swatch_index("laptop"), swatch_index("laptop"));
        let spread: BTreeSet<_> = [
            "laptop",
            "lab-arm",
            "field-rover",
            "odroid",
            "jetson",
            "creation",
        ]
        .iter()
        .map(|k| swatch_index(k))
        .collect();
        assert!(spread.len() >= 3, "{spread:?}");
    }

    fn draw(caps: Caps, focused: bool) -> (String, ratatui::buffer::Buffer) {
        with_caps(caps, || {
            let mut t = Terminal::new(ratatui::backend::TestBackend::new(16, 3)).unwrap();
            t.draw(|f| f.render_widget(block("Files".into(), focused), f.area()))
                .unwrap();
            let b = t.backend().buffer().clone();
            let text = b
                .content
                .chunks(16)
                .map(|r| r.iter().map(|c| c.symbol()).collect::<String>())
                .collect::<Vec<_>>()
                .join("\n");
            (text, b)
        })
    }

    #[test]
    fn focus_treatment_survives_every_capability_level() {
        let dark = Caps::default();
        let (text, buf) = draw(dark, true);
        assert!(
            text.starts_with("┏ Files ━") && text.contains('┃') && text.contains("┗"),
            "{text}"
        );
        let chip = &buf[(2, 0)];
        assert_eq!(chip.symbol(), "F");
        assert!(
            chip.modifier
                .contains(Modifier::UNDERLINED | Modifier::BOLD)
                && !chip.modifier.contains(Modifier::REVERSED),
            "title chip is bold underlined ink"
        );
        assert_eq!(chip.fg, Color::Rgb(0x6c, 0xc4, 0xd6));
        assert_eq!(chip.bg, Color::Reset, "chips never paint a background");
        assert_eq!(buf[(0, 1)].fg, Color::Rgb(0x6c, 0xc4, 0xd6));

        let (text, buf) = draw(dark, false);
        assert!(
            text.starts_with("╭ Files ─") && text.contains('╰'),
            "{text}"
        );
        assert!(!buf[(2, 0)].modifier.contains(Modifier::REVERSED));
        assert_eq!(buf[(2, 0)].fg, Color::Rgb(0x8b, 0x93, 0xa1));
        assert_eq!(buf[(0, 1)].fg, Color::Rgb(0x4a, 0x51, 0x5c));

        // NO_COLOR keeps heavy border + bold underlined title.
        let mono = Caps {
            color: ColorDepth::None,
            ..dark
        };
        let (text, buf) = draw(mono, true);
        assert!(text.starts_with("┏ Files"));
        assert!(buf[(2, 0)].modifier.contains(Modifier::UNDERLINED));
        assert!(buf
            .content
            .iter()
            .all(|c| c.fg == Color::Reset && c.bg == Color::Reset));

        // ASCII: `#=#` + `[ Files ]` underlined vs `+-+`, nothing outside ASCII.
        let ascii = Caps {
            unicode: false,
            color: ColorDepth::None,
            ..dark
        };
        let (text, buf) = draw(ascii, true);
        assert!(
            text.starts_with("#[ Files ]====") && text.contains("#\n#"),
            "{text}"
        );
        assert!(buf[(3, 0)].modifier.contains(Modifier::UNDERLINED));
        let (plain, _) = draw(ascii, false);
        assert!(
            plain.starts_with("+ Files ---") && plain.contains("+\n|"),
            "{plain}"
        );
        assert!(text.is_ascii() && plain.is_ascii());

        // Light 256: the same structure with indexed colors.
        let light = Caps {
            color: ColorDepth::Ansi256,
            background: Background::Light,
            ..dark
        };
        let (_, buf) = draw(light, true);
        assert_eq!(buf[(0, 1)].fg, Color::Indexed(30));
    }

    #[test]
    fn glyph_sets_are_parallel_and_ascii_is_ascii() {
        let a = &ASCII;
        for s in a
            .status
            .iter()
            .chain(a.spark)
            .chain(a.bars)
            .chain(a.spinner)
            .chain(
                [
                    a.bar_focused,
                    a.bar_dim,
                    a.both,
                    a.right,
                    a.left,
                    a.via,
                    a.blocked,
                    a.mesh,
                    a.jump,
                    a.lan,
                    a.local,
                    a.tree_mid,
                    a.tree_end,
                    a.tree_pipe,
                    a.ellipsis,
                    a.sep,
                ]
                .iter(),
            )
        {
            assert!(s.is_ascii(), "{s}");
        }
        assert_eq!(UNICODE.status.len(), ASCII.status.len());
        assert_eq!(
            Span::raw(UNICODE.bar_focused).width(),
            Span::raw(ASCII.bar_focused).width()
        );
        for set in [ASCII_FOCUSED, ASCII_PLAIN] {
            assert!([set.top_left, set.horizontal_top, set.vertical_left]
                .iter()
                .all(|s| s.is_ascii()));
        }
    }

    #[test]
    fn asciify_maps_ui_glyphs_and_leaves_content() {
        let mut buf = ratatui::buffer::Buffer::empty(Rect::new(0, 0, 8, 1));
        buf.set_string(0, 0, "┏━·…●é→", Style::default());
        asciify(&mut buf, Rect::new(0, 0, 8, 1));
        let text: String = buf.content.iter().map(|c| c.symbol()).collect();
        assert_eq!(text, "+-.~*é> ");
    }

    #[test]
    fn asciify_keeps_protected_content_but_not_chrome_drawn_over_it() {
        let ascii = Caps {
            unicode: false,
            ..Caps::default()
        };
        with_caps(ascii, || {
            let mut buf = ratatui::buffer::Buffer::empty(Rect::new(0, 0, 10, 2));
            buf.set_string(0, 0, "│“x”—→", Style::default());
            buf.set_string(0, 1, "│ ── ", Style::default());
            clear_protected_content();
            protect_content(&buf, Rect::new(1, 0, 9, 2));
            // An overlay later draws chrome over part of the content region.
            buf.set_string(6, 1, "╭─", Style::default());
            asciify(&mut buf, Rect::new(0, 0, 10, 2));
            let row = |buf: &ratatui::buffer::Buffer, y: u16| -> String {
                (0..10).map(|x| buf[(x, y)].symbol()).collect()
            };
            assert_eq!(row(&buf, 0), "|“x”—→    ");
            assert_eq!(row(&buf, 1), "| ──  +-  ");
            // Regions are per frame: the next pass converts everything.
            asciify(&mut buf, Rect::new(0, 0, 10, 2));
            assert_eq!(row(&buf, 0), "|\"x\"->    ");
        });
        // Outside ASCII mode nothing is recorded.
        with_caps(Caps::default(), || {
            let buf = ratatui::buffer::Buffer::empty(Rect::new(0, 0, 2, 1));
            protect_content(&buf, buf.area);
            assert!(CONTENT.with(|c| c.borrow().is_empty()));
        });
    }

    #[test]
    fn no_color_selection_is_marked_only_when_focused() {
        let mono = Caps {
            color: ColorDepth::None,
            ..Caps::default()
        };
        assert!(selection_for(true, mono)
            .add_modifier
            .contains(Modifier::UNDERLINED));
        assert_eq!(selection_for(false, mono), Style::default());
    }

    #[test]
    fn identity_hues_keep_their_family_at_16_colors() {
        // The truecolor hue angle must fall in the ANSI hue's range.
        let range = |c: Color| match c {
            Color::Red => (-30.0, 30.0),
            Color::Yellow => (25.0, 70.0),
            Color::Green => (60.0, 150.0),
            Color::Cyan | Color::LightCyan => (145.0, 210.0),
            Color::LightBlue => (195.0, 250.0),
            Color::Magenta | Color::LightMagenta => (240.0, 340.0),
            other => panic!("{other:?}"),
        };
        let hue = |(r, g, b): Rgb| {
            let (r, g, b) = (f64::from(r), f64::from(g), f64::from(b));
            let h = (3f64.sqrt() * (g - b))
                .atan2(2.0 * r - g - b)
                .to_degrees()
                .rem_euclid(360.0);
            if h > 330.0 {
                h - 360.0
            } else {
                h
            }
        };
        let mut seen = Vec::new();
        for i in 0..SWATCHES {
            let r = spec(Tok::Id(i));
            let ansi = r.ansi.unwrap();
            let (lo, hi) = range(ansi);
            for rgb in [r.dark, r.light] {
                let h = hue(rgb);
                assert!(lo <= h && h <= hi, "id.{i} {rgb:?} hue {h:.0} vs {ansi:?}");
            }
            assert!(!seen.contains(&ansi), "id.{i} duplicates {ansi:?}");
            seen.push(ansi);
        }
    }

    #[test]
    fn fades_only_in_truecolor() {
        with_caps(Caps::default(), || {
            assert_eq!(faded(Tok::Warn, 1.0).fg, Some(Color::Rgb(0xe3, 0xc2, 0x75)));
            assert_eq!(faded(Tok::Warn, 0.0).fg, Some(Color::Rgb(64, 64, 64)));
        });
        with_caps(
            Caps {
                color: ColorDepth::Ansi256,
                ..Caps::default()
            },
            || {
                assert_eq!(faded(Tok::Warn, 0.3).fg, Some(Color::Indexed(179)));
            },
        );
    }
}
