//! `glyphs`: renderer test card. Every view draws the same card of the cell techniques
//! concepts rely on (box lines, eighth blocks, half-block pixels, braille canvas, wide
//! glyphs, attributes, palette), so `tests/lab_png.py` fidelity can be checked by eye.
use crate::ui::lab::{caps::indexed_16, fmt, Concept, LabCaps, LabState, LabView, Tok};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    symbols::Marker,
    text::{Line, Span},
    widgets::{
        canvas::{Canvas, Circle, Line as CLine},
        Paragraph, Widget, Wrap,
    },
};

pub struct Glyphs;

impl Concept for Glyphs {
    fn name(&self) -> &str {
        "glyphs"
    }

    fn summary(&self) -> &str {
        "renderer test card: boxes, blocks, braille, wide glyphs, attributes, palette"
    }

    fn render(
        &self,
        _view: LabView,
        state: &LabState,
        caps: &LabCaps,
        area: Rect,
        buf: &mut Buffer,
    ) {
        buf.set_style(area, caps.tok(Tok::Surface));
        let u = caps.unicode;
        let m = caps.tok(Tok::Muted);
        let ramp: Vec<Option<f32>> = (0..24).map(|i| Some(i as f32 / 23.0)).collect();
        let mut lines = vec![
            Line::from(vec![
                Span::styled("box  ", m),
                Span::raw(caps.glyph("┌─┬─┐ ╭─╮ ┏━┓ ╔═╗ ┄┆ ├┼┤", "+-+-+ +-+ +-+ +=+ .: |+|")),
            ]),
            Line::from(vec![
                Span::styled("join ", m),
                Span::raw(caps.glyph("└─┴─┘ ╰─╯ ┗━┛ ╚═╝ ╌╎ ╶╴╷╵", "+-+-+ +-+ +-+ +=+ .: -.'")),
            ]),
            Line::from(vec![
                Span::styled("spark", m),
                Span::raw(" "),
                Span::styled(fmt::spark(&ramp, u), caps.tok(Tok::Accent)),
                Span::raw(" "),
                Span::raw(caps.glyph("▏▎▍▌▋▊▉█ ░▒▓", "|||||||# .:#")),
            ]),
            Line::from(vec![
                Span::styled("half ", m),
                Span::raw(caps.glyph(
                    "▀▄█▌▐ ▖▗▘▝▚▞ ⣿⡇⠿⢸⣀ ●◐○◌ ✓✗ ⏎ ⋯",
                    "^_#[] .,'`\\/ #|=|_ *~ox? vx < ...",
                )),
            ]),
            Line::from(vec![
                Span::styled("wide ", m),
                Span::raw(if u {
                    "日本語 한글 ⚡🤖 ok"
                } else {
                    "(wide glyphs: unicode only)"
                }),
            ]),
            Line::from(vec![
                Span::styled("attr ", m),
                Span::styled("bold", Style::default().add_modifier(Modifier::BOLD)),
                Span::raw(" "),
                Span::styled("dim", Style::default().add_modifier(Modifier::DIM)),
                Span::raw(" "),
                Span::styled("italic", Style::default().add_modifier(Modifier::ITALIC)),
                Span::raw(" "),
                Span::styled("under", Style::default().add_modifier(Modifier::UNDERLINED)),
                Span::raw(" "),
                Span::styled("rev", Style::default().add_modifier(Modifier::REVERSED)),
                Span::raw(" "),
                Span::styled(
                    "strike",
                    Style::default().add_modifier(Modifier::CROSSED_OUT),
                ),
            ]),
        ];
        let mut toks = vec![Span::styled("toks ", m)];
        for (t, n) in [
            (Tok::Accent, "acc"),
            (Tok::Ok, "ok"),
            (Tok::Warn, "warn"),
            (Tok::Err, "err"),
            (Tok::Mesh, "mesh"),
            (Tok::Lan, "lan"),
            (Tok::Claude, "cl"),
            (Tok::Codex, "cx"),
        ] {
            toks.push(Span::styled(format!("{n} "), caps.tok(t)));
        }
        lines.push(Line::from(toks));
        let mut pal = vec![Span::styled("ansi ", m)];
        for i in 0..16u8 {
            pal.push(Span::styled(
                format!("{i:x}"),
                Style::default()
                    .bg(indexed_16(i))
                    .fg(indexed_16(if i < 8 { 15 } else { 0 })),
            ));
        }
        pal.push(Span::raw(" "));
        for i in 0..16u16 {
            let v = (i * 17) as u8;
            pal.push(Span::styled(
                " ",
                Style::default().bg(Color::Rgb(v, 255 - v, 128)),
            ));
        }
        lines.push(Line::from(pal));
        lines.push(Line::from(vec![
            Span::styled("ids  ", m),
            Span::styled(
                state
                    .devices
                    .iter()
                    .map(|d| d.name.as_str())
                    .collect::<Vec<_>>()
                    .join(" "),
                caps.tok(Tok::Fg),
            ),
        ]));
        let n = lines.len() as u16;
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .render(area, buf);

        // Braille canvas + half-block pixels in the remaining space.
        if area.height > n + 3 && u {
            let rest = Rect::new(area.x, area.y + n + 1, area.width, area.height - n - 1);
            let half = rest.width / 2;
            let left = Rect::new(rest.x, rest.y, half, rest.height);
            let right = Rect::new(rest.x + half, rest.y, rest.width - half, rest.height);
            let accent = caps.color(Tok::Accent).unwrap_or(Color::Reset);
            let warn = caps.color(Tok::Warn).unwrap_or(Color::Reset);
            Canvas::default()
                .marker(Marker::Braille)
                .x_bounds([-1.0, 1.0])
                .y_bounds([-1.0, 1.0])
                .paint(|ctx| {
                    ctx.draw(&Circle {
                        x: 0.0,
                        y: 0.0,
                        radius: 0.8,
                        color: accent,
                    });
                    ctx.draw(&CLine {
                        x1: -1.0,
                        y1: -1.0,
                        x2: 1.0,
                        y2: 1.0,
                        color: warn,
                    });
                    ctx.draw(&CLine {
                        x1: -1.0,
                        y1: 0.6,
                        x2: 1.0,
                        y2: -0.2,
                        color: accent,
                    });
                })
                .render(left, buf);
            // Half-block gradient: each cell carries two vertical pixels (fg top, bg bottom).
            for y in 0..right.height {
                for x in 0..right.width {
                    let px = |row: u16| {
                        let t = f32::from(x) / f32::from(right.width.max(1));
                        let s = f32::from(row) / f32::from((right.height * 2).max(1));
                        caps.rgb(
                            ((40.0 + 180.0 * t) as u8, (60.0 + 120.0 * s) as u8, 160),
                            ((200.0 - 120.0 * t) as u8, (220.0 - 90.0 * s) as u8, 240),
                            None,
                        )
                        .unwrap_or(Color::Reset)
                    };
                    let cell = &mut buf[(right.x + x, right.y + y)];
                    cell.set_symbol("▀").set_fg(px(y * 2)).set_bg(px(y * 2 + 1));
                }
            }
        }
    }
}
