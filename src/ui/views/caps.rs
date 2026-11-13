//! Terminal capabilities a concept renders for, plus the spec's semantic tokens
//! (docs/design/ui-revamp.md section 4) resolved per color level and theme.
use ratatui::style::{Color, Modifier, Style};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorLevel {
    Truecolor,
    C256,
    C16,
    /// NO_COLOR: attributes only.
    None,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Theme {
    Dark,
    Light,
}

/// Which region holds keyboard focus. Concepts may ignore panes they do not have.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pane {
    Graph,
    Sessions,
    Files,
    Dialog,
    Custom(&'static str),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LabCaps {
    pub color: ColorLevel,
    pub unicode: bool,
    pub theme: Theme,
    /// None: the concept picks its natural focus for the view.
    pub focus: Option<Pane>,
    /// Cursor index within the focused pane's list (concept-defined meaning).
    pub selected: usize,
    /// Pointer cell, for hover affordances. Hover must never change state.
    pub hover: Option<(u16, u16)>,
    /// Animation frame counter (spinners, pulses); captures use 0 unless CX_LAB_FRAME is set.
    pub frame: u32,
}

impl Default for LabCaps {
    fn default() -> Self {
        Self {
            color: ColorLevel::Truecolor,
            unicode: true,
            theme: Theme::Dark,
            focus: None,
            selected: 0,
            hover: None,
            frame: 0,
        }
    }
}

/// Semantic tokens. `Id(n)` is the per-device identity hue (0..8).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tok {
    Fg,
    Muted,
    Faint,
    Accent,
    /// Selected row in the focused pane (background).
    AccentBg,
    /// Selected row in an unfocused pane (background).
    SelectDimBg,
    /// A second, stronger surface tint (background).
    Surface2,
    Ok,
    Warn,
    Err,
    Info,
    Mesh,
    Lan,
    Dir,
    Claude,
    Codex,
    Shell,
    Id(u8),
}

impl Tok {
    pub fn is_bg(self) -> bool {
        matches!(self, Tok::AccentBg | Tok::SelectDimBg | Tok::Surface2)
    }
}

type Rgb = (u8, u8, u8);
/// (dark rgb, light rgb, 256 dark, 256 light, 16-color)
type Row = (Rgb, Rgb, u8, u8, Option<Color>);

fn row(t: Tok) -> Option<Row> {
    use crate::ui::theme::{spec, Tok as T};
    // The tokens the rest of the UI draws too come from its table (`theme::spec`), so a
    // state has one colour everywhere.
    let shared = |t: T| {
        let r = spec(t);
        Some((r.dark, r.light, r.x256.0, r.x256.1, r.ansi))
    };
    match t {
        Tok::Fg => None,
        Tok::Muted => shared(T::Muted),
        Tok::Faint => shared(T::Faint),
        Tok::Accent => shared(T::Accent),
        Tok::AccentBg => shared(T::AccentBg),
        Tok::SelectDimBg => shared(T::SelectDimBg),
        Tok::Ok => shared(T::Ok),
        Tok::Warn => shared(T::Warn),
        Tok::Err => shared(T::Err),
        Tok::Info => shared(T::Info),
        Tok::Id(n) => shared(T::Id(n)),
        Tok::Surface2 => Some(((0x23, 0x28, 0x30), (0xe6, 0xe9, 0xed), 235, 254, None)),
        Tok::Mesh => Some((
            (0xc4, 0x9c, 0xf0),
            (0x70, 0x48, 0xc0),
            177,
            97,
            Some(Color::Magenta),
        )),
        Tok::Lan => Some((
            (0x7f, 0xc9, 0xb4),
            (0x0f, 0x7a, 0x62),
            79,
            29,
            Some(Color::Cyan),
        )),
        Tok::Dir => Some((
            (0x8f, 0xb7, 0xf0),
            (0x1d, 0x4f, 0x91),
            111,
            25,
            Some(Color::LightBlue),
        )),
        Tok::Claude => Some((
            (0xe0, 0xa3, 0x7a),
            (0xa4, 0x53, 0x2a),
            173,
            130,
            Some(Color::Yellow),
        )),
        Tok::Codex => Some((
            (0x9e, 0xc5, 0xf8),
            (0x26, 0x59, 0xa8),
            111,
            25,
            Some(Color::Blue),
        )),
        Tok::Shell => Some(((0xaa, 0xb2, 0xbf), (0x4b, 0x55, 0x63), 249, 240, None)),
    }
}

impl LabCaps {
    /// The color a token resolves to at this level, or None for the terminal default.
    pub fn color(&self, t: Tok) -> Option<Color> {
        let (dark, light, d256, l256, c16) = row(t)?;
        let dark_theme = self.theme == Theme::Dark;
        match self.color {
            ColorLevel::Truecolor => {
                let (r, g, b) = if dark_theme { dark } else { light };
                Some(Color::Rgb(r, g, b))
            }
            ColorLevel::C256 => Some(Color::Indexed(if dark_theme { d256 } else { l256 })),
            ColorLevel::C16 => c16,
            ColorLevel::None => None,
        }
    }

    /// A ready style for a token: foreground for text tokens, background for surface
    /// tokens, with the spec's NO_COLOR / 16-color attribute substitutes applied.
    pub fn tok(&self, t: Tok) -> Style {
        let c = self.color(t);
        let mut s = Style::default();
        if t.is_bg() {
            match c {
                Some(c) => s = s.bg(c),
                None if matches!(t, Tok::AccentBg) => s = s.add_modifier(Modifier::REVERSED),
                None => {}
            }
            return s;
        }
        if let Some(c) = c {
            s = s.fg(c);
        }
        if self.color == ColorLevel::None {
            s = match t {
                Tok::Muted | Tok::Faint => s.add_modifier(Modifier::DIM),
                Tok::Accent | Tok::Err | Tok::Dir => s.add_modifier(Modifier::BOLD),
                _ => s,
            };
        } else if self.color == ColorLevel::C16 && matches!(t, Tok::Faint) {
            s = s.add_modifier(Modifier::DIM);
        }
        s
    }

    /// Unicode glyph or its ASCII stand-in.
    pub fn glyph(&self, unicode: &'static str, ascii: &'static str) -> &'static str {
        if self.unicode {
            unicode
        } else {
            ascii
        }
    }

    pub fn has_color(&self) -> bool {
        self.color != ColorLevel::None
    }

    /// Parse a variant label such as `dark-truecolor`, `light-256`, `dark-16`, `nocolor`,
    /// `ascii` (dark, no color, no unicode) or `ascii-16`.
    #[cfg(test)]
    pub fn from_variant(label: &str) -> Option<Self> {
        let mut caps = Self::default();
        let mut color_set = false;
        let mut ascii = false;
        for part in label.split('-') {
            match part {
                "dark" => caps.theme = Theme::Dark,
                "light" => caps.theme = Theme::Light,
                "truecolor" | "24bit" | "tc" => {
                    caps.color = ColorLevel::Truecolor;
                    color_set = true
                }
                "256" => {
                    caps.color = ColorLevel::C256;
                    color_set = true
                }
                "16" => {
                    caps.color = ColorLevel::C16;
                    color_set = true
                }
                "nocolor" | "mono" => {
                    caps.color = ColorLevel::None;
                    color_set = true
                }
                "ascii" => ascii = true,
                "unicode" => caps.unicode = true,
                _ => return None,
            }
        }
        if ascii {
            caps.unicode = false;
            if !color_set {
                caps.color = ColorLevel::None;
            }
        }
        Some(caps)
    }
}

/// Standard xterm 16-color values used to quantize (the PNG renderer shows a nicer palette).
pub const XTERM16: [Rgb; 16] = [
    (0, 0, 0),
    (205, 0, 0),
    (0, 205, 0),
    (205, 205, 0),
    (0, 0, 238),
    (205, 0, 205),
    (0, 205, 205),
    (229, 229, 229),
    (127, 127, 127),
    (255, 0, 0),
    (0, 255, 0),
    (255, 255, 0),
    (92, 92, 255),
    (255, 0, 255),
    (0, 255, 255),
    (255, 255, 255),
];

pub fn index_rgb(i: u8) -> Rgb {
    match i {
        0..=15 => XTERM16[usize::from(i)],
        16..=231 => {
            let i = i - 16;
            let lv = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
            (lv(i / 36), lv((i / 6) % 6), lv(i % 6))
        }
        _ => {
            let v = 8 + (i - 232) * 10;
            (v, v, v)
        }
    }
}

fn dist(a: Rgb, b: Rgb) -> u32 {
    let d = |x: u8, y: u8| (i32::from(x) - i32::from(y)).pow(2) as u32;
    // Weighted for perceived luminance.
    2 * d(a.0, b.0) + 4 * d(a.1, b.1) + 3 * d(a.2, b.2)
}

pub fn rgb_to_256(r: u8, g: u8, b: u8) -> u8 {
    (16..=255u8)
        .min_by_key(|&i| dist(index_rgb(i), (r, g, b)))
        .unwrap_or(16)
}

pub fn rgb_to_16(r: u8, g: u8, b: u8) -> u8 {
    (0..16u8)
        .min_by_key(|&i| dist(XTERM16[usize::from(i)], (r, g, b)))
        .unwrap_or(7)
}

pub fn named_index(c: Color) -> Option<u8> {
    Some(match c {
        Color::Black => 0,
        Color::Red => 1,
        Color::Green => 2,
        Color::Yellow => 3,
        Color::Blue => 4,
        Color::Magenta => 5,
        Color::Cyan => 6,
        Color::Gray => 7,
        Color::DarkGray => 8,
        Color::LightRed => 9,
        Color::LightGreen => 10,
        Color::LightYellow => 11,
        Color::LightBlue => 12,
        Color::LightMagenta => 13,
        Color::LightCyan => 14,
        Color::White => 15,
        Color::Indexed(i) => i,
        _ => return None,
    })
}

pub fn indexed_16(i: u8) -> Color {
    [
        Color::Black,
        Color::Red,
        Color::Green,
        Color::Yellow,
        Color::Blue,
        Color::Magenta,
        Color::Cyan,
        Color::Gray,
        Color::DarkGray,
        Color::LightRed,
        Color::LightGreen,
        Color::LightYellow,
        Color::LightBlue,
        Color::LightMagenta,
        Color::LightCyan,
        Color::White,
    ][usize::from(i % 16)]
}

/// What a terminal at `level` can show of `c`; also counts colors the concept leaked above it.
#[cfg(test)]
pub fn degrade(c: Color, level: ColorLevel, leaked: &mut u32) -> Color {
    match (level, c) {
        (_, Color::Reset) => Color::Reset,
        (ColorLevel::Truecolor, c) => c,
        (ColorLevel::None, _) => {
            *leaked += 1;
            Color::Reset
        }
        (ColorLevel::C256, Color::Rgb(r, g, b)) => {
            *leaked += 1;
            Color::Indexed(rgb_to_256(r, g, b))
        }
        (ColorLevel::C256, c) => c,
        (ColorLevel::C16, Color::Rgb(r, g, b)) => {
            *leaked += 1;
            indexed_16(rgb_to_16(r, g, b))
        }
        (ColorLevel::C16, Color::Indexed(i)) if i >= 16 => {
            *leaked += 1;
            let (r, g, b) = index_rgb(i);
            indexed_16(rgb_to_16(r, g, b))
        }
        (ColorLevel::C16, c) => c,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variants_parse() {
        let a = LabCaps::from_variant("ascii").unwrap();
        assert!(!a.unicode && a.color == ColorLevel::None);
        let b = LabCaps::from_variant("light-16").unwrap();
        assert!(b.unicode && b.color == ColorLevel::C16 && b.theme == Theme::Light);
        assert!(LabCaps::from_variant("ascii-16").unwrap().color == ColorLevel::C16);
        assert!(LabCaps::from_variant("bogus").is_none());
    }

    #[test]
    fn quantizers_hit_exact_palette_entries() {
        assert_eq!(rgb_to_256(0, 0, 0), 16);
        assert_eq!(rgb_to_256(255, 255, 255), 231);
        assert_eq!(rgb_to_16(250, 10, 10), 9);
        let mut n = 0;
        assert_eq!(
            degrade(Color::Rgb(0, 200, 200), ColorLevel::C16, &mut n),
            Color::Cyan
        );
        assert_eq!(n, 1);
    }
}
