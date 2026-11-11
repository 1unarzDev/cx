//! Buffer -> cell/style JSON for tests/lab_png.py, after applying what the terminal tier
//! can really show (colors quantized or dropped) and linting capability leaks.
//!
//! Cell format: `[symbol, fg, bg, modifiers]` plus a 5th element `width` for wide
//! graphemes; the cell after a wide grapheme is `["", ...]` (continuation).
//! Colors: null (terminal default), 0..=255 (palette index; named colors map to 0..15),
//! or "#rrggbb". Modifiers are ratatui's bit flags (BOLD 1, DIM 2, ITALIC 4,
//! UNDERLINED 8, REVERSED 64, HIDDEN 128, CROSSED_OUT 256).
use super::caps::{degrade, named_index, LabCaps};
use ratatui::{buffer::Buffer, style::Color, text::Span};
use serde_json::{json, Value};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Lint {
    /// Cells drawn with a non-ASCII symbol while caps.unicode is false.
    pub non_ascii: u32,
    /// Colors above the caps' color level (they were quantized/dropped for the dump).
    pub color_leaks: u32,
    /// First few offending symbols, for the report.
    pub samples: Vec<String>,
}

pub fn color_json(c: Color) -> Value {
    match c {
        Color::Reset => Value::Null,
        Color::Rgb(r, g, b) => json!(format!("#{r:02x}{g:02x}{b:02x}")),
        c => named_index(c).map(|i| json!(i)).unwrap_or(Value::Null),
    }
}

/// Degrade colors in place to the caps tier and count leaks.
pub fn enforce(buf: &mut Buffer, caps: &LabCaps) -> Lint {
    let mut lint = Lint::default();
    let area = buf.area;
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            let cell = &mut buf[(x, y)];
            cell.fg = degrade(cell.fg, caps.color, &mut lint.color_leaks);
            cell.bg = degrade(cell.bg, caps.color, &mut lint.color_leaks);
            if !caps.unicode && !cell.symbol().is_ascii() {
                lint.non_ascii += 1;
                if lint.samples.len() < 12 && !lint.samples.iter().any(|s| s == cell.symbol()) {
                    lint.samples.push(cell.symbol().to_string());
                }
            }
        }
    }
    lint
}

/// Enforce caps and serialize the buffer rows.
pub fn dump(buf: &Buffer, caps: &LabCaps) -> (Value, Lint) {
    let mut buf = buf.clone();
    let lint = enforce(&mut buf, caps);
    let area = buf.area;
    let mut rows = Vec::with_capacity(usize::from(area.height));
    for y in area.top()..area.bottom() {
        let mut row = Vec::with_capacity(usize::from(area.width));
        let mut skip = 0u16;
        for x in area.left()..area.right() {
            let cell = &buf[(x, y)];
            let fg = color_json(cell.fg);
            let bg = color_json(cell.bg);
            let mods = cell.modifier.bits();
            if skip > 0 {
                skip -= 1;
                row.push(json!(["", fg, bg, mods]));
                continue;
            }
            let sym = cell.symbol();
            let w = Span::raw(sym).width() as u16;
            if w > 1 {
                // Clip a wide grapheme that would overrun the row, like a terminal does.
                if x + w > area.right() {
                    row.push(json!([" ", fg, bg, mods]));
                    continue;
                }
                skip = w - 1;
                row.push(json!([sym, fg, bg, mods, w]));
            } else {
                row.push(json!([sym, fg, bg, mods]));
            }
        }
        rows.push(Value::Array(row));
    }
    (Value::Array(rows), lint)
}

/// Plain text of a buffer (one line per row), for quick diffs and tests.
pub fn text(buf: &Buffer) -> String {
    let area = buf.area;
    let mut out = String::new();
    for y in area.top()..area.bottom() {
        let mut skip = 0;
        for x in area.left()..area.right() {
            if skip > 0 {
                skip -= 1;
                continue;
            }
            let s = buf[(x, y)].symbol();
            skip = Span::raw(s).width().saturating_sub(1);
            out.push_str(s);
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::lab::caps::ColorLevel;
    use ratatui::{layout::Rect, style::Style};

    #[test]
    fn wide_cells_and_degradation() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 6, 1));
        buf.set_string(0, 0, "日a", Style::default().fg(Color::Rgb(250, 0, 0)));
        let caps = LabCaps {
            color: ColorLevel::C16,
            unicode: false,
            ..LabCaps::default()
        };
        let (v, lint) = dump(&buf, &caps);
        let row = v[0].as_array().unwrap();
        assert_eq!(row[0][0], "日");
        assert_eq!(row[0][4], 2);
        assert_eq!(row[1][0], "");
        assert_eq!(row[2][0], "a");
        assert_eq!(row[0][1], 9);
        assert_eq!(lint.non_ascii, 1);
        assert_eq!(lint.color_leaks, 2);
        assert_eq!(text(&buf).trim_end(), "日a");
    }
}
