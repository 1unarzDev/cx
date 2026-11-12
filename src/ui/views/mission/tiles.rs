//! Tiles: micro-charts, phone device rows and session thumbnails that show the live tail
//! of the real screen in its own colours, so sessions are recognised by their look.
use super::{fit, fmt, mix, to_rgb, Cx, Rgb};
use crate::ui::views::{ansi_lines, LabDevice, LabSession, Tok};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
};

/// Selection state of a tile: None, Some(false) = selected in an unfocused zone,
/// Some(true) = selected in the focused zone.
pub(super) type Pick = Option<bool>;

pub(super) fn tile_bg(cx: &Cx, pick: Pick) -> Style {
    // Without tints the bar alone marks the selection (never a reversed block).
    if !cx.has_surface() {
        return Style::default();
    }
    match pick {
        Some(true) => cx.st(Tok::AccentBg),
        Some(false) => cx.st(Tok::SelectDimBg),
        None => cx.surface(),
    }
}

/// Selection edge for tiles: an accent bar on the left (works in every tier).
fn pick_bar(cx: &Cx, buf: &mut Buffer, r: Rect, pick: Pick) {
    if let Some(f) = pick {
        let st = if f {
            cx.st(Tok::Accent)
        } else {
            cx.ink(Tok::Muted, 0.8)
        };
        // ASCII: `>` on the first row only, `|` below it (a column of `>` reads as
        // shell prompts); an unfocused selection is the same bar, dimmer.
        cx.bar_in(buf, r.x, r.y, r.height, st, f);
    }
}

/// Eighth-block area chart (newest on the right), shaded by height.
pub(super) fn chart(cx: &Cx, buf: &mut Buffer, r: Rect, vals: &[f32]) {
    chart_in(cx, buf, r, vals, None);
}

/// A chart in one ink (`None`: each bar's heat).
pub(super) fn chart_in(cx: &Cx, buf: &mut Buffer, r: Rect, vals: &[f32], ink: Option<Style>) {
    if r.width == 0 || r.height == 0 || vals.is_empty() {
        return;
    }
    const E: [&str; 9] = [" ", "▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"];
    let n = vals.len();
    let w = usize::from(r.width);
    for i in 0..w {
        let v = if n >= w {
            vals[n - w + i]
        } else {
            vals[i * n / w]
        }
        .clamp(0.0, 1.0);
        let total = (v * f32::from(r.height) * 8.0).round() as i32;
        let st = ink.unwrap_or_else(|| cx.spark_heat(v));
        for row in 0..r.height {
            let y = r.bottom() - 1 - row;
            let fill = (total - i32::from(row) * 8).clamp(0, 8) as usize;
            let s = if cx.uni() {
                E[fill]
            } else if fill >= 4 {
                // (`#` is the edited link, `:` a password stroke.)
                "="
            } else if row == 0 {
                "_"
            } else {
                " "
            };
            if fill == 0 && row == 0 && cx.uni() {
                cx.put(buf, r.x + i as u16, y, 1, "▁", cx.ink(Tok::Faint, 0.8));
            } else if fill > 0 || !cx.uni() {
                cx.put(buf, r.x + i as u16, y, 1, s, st);
            }
        }
    }
}

/// One-row sparkline with gaps for lost samples.
pub(super) fn spark(
    cx: &Cx,
    buf: &mut Buffer,
    x: u16,
    y: u16,
    w: u16,
    vals: &[Option<f32>],
    st: Style,
) {
    cx.put(buf, x, y, w, &spark_str(cx, vals, w), st);
}

/// The sparkline text, scaled between the window's min and max so variation shows.
fn spark_str(cx: &Cx, vals: &[Option<f32>], w: u16) -> String {
    if w == 0 || vals.is_empty() {
        return String::new();
    }
    let hi = vals.iter().flatten().fold(f32::MIN, |a, b| a.max(*b));
    let lo = vals.iter().flatten().fold(f32::MAX, |a, b| a.min(*b));
    let range = (hi - lo).max(hi.abs() * 0.05).max(1e-3);
    let n = vals.len();
    let w = usize::from(w);
    let pick: Vec<Option<f32>> = (0..w)
        .map(|i| {
            if n >= w {
                vals[n - w + i]
            } else {
                vals[i * n / w]
            }
        })
        .map(|v| v.map(|v| 0.1 + 0.9 * (v - lo) / range))
        .collect();
    let s = fmt::spark(&pick, cx.uni());
    if cx.uni() {
        s
    } else {
        // ASCII sparks use only `_ =`: `.` and `-` are link strokes on the map, `#`
        // the edited link.
        s.replace('.', "_").replace(['-', '#'], "=")
    }
}

/// Thin horizontal gauge with a faint track.
pub(super) fn gauge(cx: &Cx, buf: &mut Buffer, x: u16, y: u16, w: u16, v: f32, st: Style) {
    let v = v.clamp(0.0, 1.0);
    let cells = f32::from(w) * v;
    for i in 0..w {
        let f = (cells - f32::from(i)).clamp(0.0, 1.0);
        let (g, s) = if cx.uni() {
            if f >= 0.75 {
                ("━", st)
            } else if f >= 0.25 {
                ("╸", st)
            } else {
                ("━", cx.ink(Tok::Faint, 0.7))
            }
        } else if f >= 0.5 {
            ("=", st)
        } else {
            ("-", cx.ink(Tok::Faint, 1.0))
        };
        cx.put(buf, x + i, y, 1, g, s);
    }
}

/// A mesh's swatch and a space before its name: a square in its hue; none in ASCII
/// (every printable glyph there already means something; the name says it).
pub(super) fn mesh_glyph(cx: &Cx) -> String {
    cx.g("■ ", "").into()
}

/// Session pips (one slot letter per session in its state colour, space separated),
/// right-aligned at `right`.
pub(super) fn pips(
    cx: &Cx,
    buf: &mut Buffer,
    right: u16,
    y: u16,
    d: &LabDevice,
    max: usize,
) -> u16 {
    let segs = pip_segs(cx, d, max);
    let w: u16 = segs
        .iter()
        .map(|(t, _)| super::width(t, cx.uni()) + 1)
        .sum::<u16>()
        .saturating_sub(1);
    let mut x = right.saturating_sub(w);
    for (t, st) in segs {
        x += cx.put(buf, x, y, 4, &t, st) + 1;
    }
    w
}

/// Pips as text segments: each session's slot letter in its state colour (`a s`), so a
/// node says both how many sessions run there and the key that attaches each one.
pub(super) fn pip_segs(cx: &Cx, d: &LabDevice, max: usize) -> Vec<(String, Style)> {
    let ss: Vec<&LabSession> = cx.s.sessions_on(&d.id).collect();
    let mut v: Vec<(String, Style)> = ss
        .iter()
        .take(max)
        .map(|s| {
            let k = cx.slot(s).unwrap_or(' ').to_string();
            (k, cx.sstate(s).2.add_modifier(Modifier::BOLD))
        })
        .collect();
    if ss.len() > max {
        v.push((format!("+{}", ss.len() - max), cx.st(Tok::Muted)));
    }
    v
}

/// Compact device line for phones: keycap, name, status and evidence on one row; with
/// two rows (a thumb-sized target) the cpu trend, temperature and pips go underneath.
pub(super) fn device_row(cx: &Cx, buf: &mut Buffer, r: Rect, d: &LabDevice, pick: Pick) {
    if r.width < 10 || r.height == 0 {
        return;
    }
    let bg = match pick {
        Some(_) => tile_bg(cx, pick),
        None => Style::default(),
    };
    cx.fill(buf, r, bg);
    pick_bar(cx, buf, r, pick);
    let mut x = r.x + 1;
    x += cx.dchip(buf, x, r.y, d, false);
    x += 1;
    let (g, gst) = cx.status(d.status);
    let right = r.right().saturating_sub(1);
    let two = r.height >= 2;
    let tail = if two { 8 } else { 13u16.min(r.width / 2) };
    let up = cx.up(d);
    let nst = if pick == Some(true) {
        // Focused: the bold accent name that goes with the bar, in every tier.
        cx.st(Tok::Accent).add_modifier(Modifier::BOLD)
    } else if up {
        cx.ink(Tok::Fg, 1.0).add_modifier(Modifier::BOLD)
    } else {
        cx.st(Tok::Muted)
    };
    let room = right.saturating_sub(x + tail + 2);
    cx.put(
        buf,
        x,
        r.y,
        room,
        &fit(&d.name, room as usize, cx.uni()),
        nst,
    );
    let tx = right.saturating_sub(tail);
    cx.put(buf, tx, r.y, 1, g, gst);
    let meta = cx.meta(d);
    let mst = if up {
        cx.st(Tok::Muted)
    } else {
        cx.ink(Tok::Err, 0.85)
    };
    let mw = cx.put(buf, tx + 2, r.y, tail.saturating_sub(2), &meta, mst);
    let (ey, ex) = if two {
        (r.y + 1, r.x + 5)
    } else {
        (r.y, tx + 2 + mw + 1)
    };
    let pw = pips(cx, buf, right, ey, d, 3);
    if let Some(m) = d.metrics.as_ref().filter(|_| up) {
        let sw = right
            .saturating_sub(ex + pw + 1)
            .min(if two { 12 } else { 99 });
        if sw >= 3 {
            let v: Vec<Option<f32>> = m.cpu_hist.iter().map(|v| Some(*v)).collect();
            spark(cx, buf, ex, ey, sw, &v, cx.spark_heat(m.cpu));
            if two {
                let t = match m.temp_c {
                    Some(t) => format!("{:.0}% {t:.0}{}", m.cpu * 100.0, cx.g("°", "C")),
                    None => format!("{:.0}%", m.cpu * 100.0),
                };
                cx.put(buf, ex + sw + 1, ey, 10, &t, cx.st(Tok::Muted));
            }
        }
    }
    cx.hit(r, format!("device:{}", d.id));
}

fn text(l: &ratatui::text::Line) -> String {
    l.spans.iter().map(|s| s.content.as_ref()).collect()
}

/// Screen lines up to the last informative one (trailing chrome dropped).
pub(super) fn used_lines(x: &LabSession) -> usize {
    ansi_lines(&x.screen, x.screen_cols)
        .iter()
        .rposition(|l| !boring(text(l).trim()))
        .map_or(0, |i| i + 1)
}

/// Rows a thumbnail needs to show its whole screen tail: the non-blank lines up to the
/// last informative one (a taller tile would only add blank rows).
pub(super) fn tail_rows(x: &LabSession) -> usize {
    let lines = ansi_lines(&x.screen, x.screen_cols);
    lines[..used_lines(x)]
        .iter()
        .filter(|l| !text(l).trim().is_empty())
        .count()
}

/// The screen's last informative lines, cropped, ending above trailing chrome (input
/// boxes, key hints). Returns the source line range shown.
pub(super) fn screen_text(
    cx: &Cx,
    buf: &mut Buffer,
    r: Rect,
    x: &LabSession,
    color: bool,
) -> (usize, usize) {
    let lines = ansi_lines(&x.screen, x.screen_cols);
    let used = used_lines(x);
    // The last lines that carry something; when the tile is short, blank lines go first.
    let h = usize::from(r.height);
    let start = used.saturating_sub(h);
    let mut pick: Vec<usize> = (start..used).collect();
    if used > h {
        pick = (0..used)
            .rev()
            .filter(|&i| !text(&lines[i]).trim().is_empty())
            .take(h)
            .collect();
        pick.reverse();
    }
    // Narrow tiles drop the indentation the shown lines share.
    let indent = if r.width < x.screen_cols {
        pick.iter()
            .map(|&i| text(&lines[i]))
            .filter(|t| !t.trim().is_empty())
            .map(|t| t.chars().take_while(|c| *c == ' ').count())
            .min()
            .unwrap_or(0)
    } else {
        0
    };
    for (row, &i) in pick.iter().enumerate() {
        let y = r.y + row as u16;
        let mut xx = r.x;
        let mut skip = indent;
        for sp in &lines[i].spans {
            if xx >= r.right() {
                break;
            }
            let st = if color {
                degrade(cx, sp.style)
            } else {
                plain(sp.style)
            };
            // The agent's own reversed key hints (`y Yes`) would look exactly like cx
            // keycaps on the hub, where those letters mean devices: thumbnails draw them
            // in flat ink, no reverse, underline or bold.
            // An emphasised lone letter (`a Always`) would read as a cx key too: it gets
            // the muted caption ink, one step below every cx keycap and name.
            let lone = sp.content.trim().chars().count() == 1
                && sp
                    .style
                    .add_modifier
                    .intersects(Modifier::BOLD | Modifier::REVERSED);
            let st = if lone {
                cx.ink(super::Tok::Muted, 1.0)
            } else if sp.style.add_modifier.contains(Modifier::REVERSED) {
                st.remove_modifier(Modifier::REVERSED | Modifier::BOLD | Modifier::UNDERLINED)
            } else {
                st
            };
            let n = sp.content.chars().count();
            let content: String = if skip > 0 {
                let k = skip.min(n);
                skip -= k;
                sp.content.chars().skip(k).collect()
            } else {
                sp.content.to_string()
            };
            if content.is_empty() {
                continue;
            }
            xx += cx.put(buf, xx, y, r.right() - xx, &content, st);
        }
    }
    (pick.first().copied().unwrap_or(0), used)
}

fn boring(t: &str) -> bool {
    t.is_empty()
        || t.starts_with('╭')
        || t.starts_with('╰')
        || t.starts_with('│')
        || t.starts_with("? for")
        || t.starts_with('⏎')
        || t.starts_with('>')
}

fn plain(s: Style) -> Style {
    let keep = Modifier::BOLD | Modifier::DIM | Modifier::REVERSED;
    Style::default().add_modifier(s.add_modifier & keep)
}

/// Re-express a session screen's style at this terminal's colour level and theme. In 16
/// colours, named colours stay themselves and rgb inks map by hue, so red stays red.
pub(super) fn degrade(cx: &Cx, s: Style) -> Style {
    let mut out = plain(s);
    if cx.c.color == crate::ui::views::ColorLevel::C16 {
        // No tinted backgrounds in 16 colours: a coloured background (a diff row) becomes
        // ink of its hue instead, so + stays green and - stays red without a loud block.
        let ink = |c: ratatui::style::Color| -> Option<ratatui::style::Color> {
            if crate::ui::views::caps::named_index(c).is_some_and(|i| i < 16) {
                return Some(c);
            }
            to_rgb(c).map(|c| crate::ui::views::caps::indexed_16(ansi16(c, false)))
        };
        let from_bg = s.bg.and_then(to_rgb).and_then(|c| {
            let (hi, lo) = (c.0.max(c.1).max(c.2), c.0.min(c.1).min(c.2));
            (u16::from(hi - lo) * 4 > u16::from(hi) && hi > 24).then(|| {
                let full = |v: u8| ((u16::from(v - lo) * 255) / u16::from(hi - lo).max(1)) as u8;
                crate::ui::views::caps::indexed_16(ansi16((full(c.0), full(c.1), full(c.2)), true))
            })
        });
        let fg = s.fg.and_then(ink);
        let grey = |c: Option<ratatui::style::Color>| {
            c.and_then(crate::ui::views::caps::named_index)
                .is_none_or(|i| matches!(i, 0 | 7 | 8 | 15))
        };
        return match (from_bg, fg) {
            (Some(b), f) if grey(f) => out.fg(b),
            (_, Some(f)) => out.fg(f),
            _ => out,
        };
    }
    if let Some(f) =
        s.fg.and_then(to_rgb)
            .and_then(|c| cx.col(adapt(cx, c, false)))
    {
        out = out.fg(f);
    }
    if let Some(b) =
        s.bg.and_then(to_rgb)
            .and_then(|c| cx.col(adapt(cx, c, true)))
    {
        out = out.bg(b);
    }
    out
}

/// Nearest ANSI colour by hue (a nearest-distance match turns muted reds grey).
fn ansi16(c: Rgb, bg: bool) -> u8 {
    let (r, g, b) = (f32::from(c.0), f32::from(c.1), f32::from(c.2));
    let (hi, lo) = (r.max(g).max(b), r.min(g).min(b));
    let light = (hi + lo) / 510.0;
    if hi - lo < 40.0 {
        return match light {
            l if l < 0.2 => 0,
            l if l < 0.5 => 8,
            l if l < 0.8 => 7,
            _ => 15,
        };
    }
    let h = if hi == r {
        ((g - b) / (hi - lo)).rem_euclid(6.0)
    } else if hi == g {
        (b - r) / (hi - lo) + 2.0
    } else {
        (r - g) / (hi - lo) + 4.0
    } * 60.0;
    let base = match h {
        h if !(20.0..330.0).contains(&h) => 1,
        h if h < 70.0 => 3,
        h if h < 160.0 => 2,
        h if h < 200.0 => 6,
        h if h < 260.0 => 4,
        _ => 5,
    };
    // Backgrounds keep the plain hue; bright inks use the bright half.
    if !bg && light > 0.62 {
        base + 8
    } else {
        base
    }
}

/// Screens are mostly authored for dark terminals; on a light theme, dark backgrounds
/// become pale tints that keep their hue (a diff stays green / red) and pale inks deepen.
pub(super) fn adapt(cx: &Cx, c: Rgb, bg: bool) -> Rgb {
    if cx.dark() {
        return c;
    }
    let lum = (0.3 * f32::from(c.0) + 0.59 * f32::from(c.1) + 0.11 * f32::from(c.2)) / 255.0;
    match (bg, lum) {
        (true, l) if l < 0.5 => {
            // Stretch the channels to full saturation, then tint white with it.
            let hi = c.0.max(c.1).max(c.2);
            let lo = c.0.min(c.1).min(c.2);
            if hi - lo < 12 {
                return mix((0xff, 0xff, 0xff), c, 0.12);
            }
            let st = |v: u8| ((u16::from(v - lo) * 255) / u16::from(hi - lo)) as u8;
            mix((0xff, 0xff, 0xff), (st(c.0), st(c.1), st(c.2)), 0.16)
        }
        (false, l) if l > 0.5 => mix(c, (0, 0, 0), 0.42),
        _ => c,
    }
}

/// The most informative last line (status line of an agent, last log line, prompt).
pub(super) fn caption(x: &LabSession) -> String {
    let lines = ansi_lines(&x.screen, x.screen_cols);
    lines
        .iter()
        .map(|l| {
            l.spans
                .iter()
                .map(|s| s.content.as_ref())
                .collect::<String>()
                .trim()
                .to_string()
        })
        .rev()
        .find(|t| !boring(t))
        .unwrap_or_default()
}

/// A session's address, the same everywhere: its device keycap then its slot keycap
/// (`i` `a`), which is exactly what you type from the hub. Returns the width.
pub(super) fn addr(cx: &Cx, buf: &mut Buffer, x: u16, y: u16, s: &LabSession) -> u16 {
    let mut w = 0;
    if let Some(d) = cx.s.device(&s.device) {
        w += cx.dchip(buf, x, y, d, false);
    }
    let k = cx.slot(s).unwrap_or(' ');
    w + cx.put(buf, x + w, y, 3, &format!(" {k} "), cx.key_style())
}

/// A session as a two-row thumb target (phones): its address, title and state, then the
/// provider and the screen's most informative line. What needs you is tinted warn.
pub(super) fn session_row(cx: &Cx, buf: &mut Buffer, r: Rect, x: &LabSession, pick: Pick) {
    if r.width < 12 || r.height == 0 {
        return;
    }
    let ask = x.state == crate::ui::views::SessionState::AwaitingInput;
    let bg = match pick {
        Some(_) => tile_bg(cx, pick),
        None if ask => cx.wash(Tok::Warn, 0.12),
        None => cx.surface(),
    };
    cx.fill(buf, r, bg);
    // Column 0 is the focus bar only; what needs you says so by its tint and `◆`.
    if pick.is_some() {
        pick_bar(cx, buf, r, pick);
    }
    let right = r.right().saturating_sub(1);
    let (g, word, st) = cx.sstate(x);
    let mut xx = r.x + 2;
    xx += addr(cx, buf, xx, r.y, x) + 1;
    let show_word = r.width >= 50 || ask;
    let tail = if show_word {
        format!("{g} {word}")
    } else {
        g.to_string()
    };
    let tw = super::width(&tail, cx.uni());
    let room = right.saturating_sub(xx + tw + 1);
    cx.put(
        buf,
        xx,
        r.y,
        room,
        &fit(&x.title, room as usize, cx.uni()),
        if pick == Some(true) {
            cx.st(Tok::Accent).add_modifier(Modifier::BOLD)
        } else {
            cx.ink(Tok::Fg, 1.0).add_modifier(Modifier::BOLD)
        },
    );
    cx.put_r(buf, right, r.y, &tail, st);
    if r.height >= 2 {
        let (p, ptok) = cx.prov(x);
        let mut x2 = r.x + 2;
        x2 += cx.put(buf, x2, r.y + 1, 7, p, cx.st(ptok)) + 1;
        let cap = if ask {
            format!("is asking {} open to answer", cx.g("·", "-"))
        } else {
            caption(x)
        };
        let room = right.saturating_sub(x2);
        cx.put(
            buf,
            x2,
            r.y + 1,
            room,
            &fit(&cap, room as usize, cx.uni()),
            cx.st(Tok::Muted),
        );
    }
    cx.hit(r, format!("session:{}:{}", x.device, x.id));
}

/// Session thumbnail: header (keys, title, state), the screen's live tail and a caption
/// (provider, folder, activity, age).
pub(super) fn thumb(cx: &Cx, buf: &mut Buffer, r: Rect, x: &LabSession, pick: Pick) {
    thumb_with(cx, buf, r, x, pick, true);
}

/// `keys: false` draws the session's address as plain muted text, for screens where its
/// letters mean something else (the edge editor's `a p o`).
pub(super) fn thumb_with(
    cx: &Cx,
    buf: &mut Buffer,
    r: Rect,
    x: &LabSession,
    pick: Pick,
    keys: bool,
) {
    if r.width < 12 || r.height < 3 {
        return;
    }
    cx.fill(buf, r, tile_bg(cx, pick));
    pick_bar(cx, buf, r, pick);
    let d = cx.s.device(&x.device);
    let left = r.x + 1 + u16::from(pick.is_some());
    let right = r.right().saturating_sub(1);
    // Header: the key that opens it here, its device, title, state.
    let mut hx = left;
    if keys {
        hx += addr(cx, buf, hx, r.y, x);
    } else {
        let a = format!(
            " {} {} ",
            d.map_or(' ', |d| d.hotkey),
            cx.slot(x).unwrap_or(' ')
        );
        hx += cx.put(buf, hx, r.y, 5, &a, cx.st(Tok::Muted));
    }
    hx += 1;
    let (g, word, sst) = cx.sstate(x);
    let state = if x.state == crate::ui::views::SessionState::Idle {
        format!("{word} {}", fmt::age(x.last_activity_secs))
    } else {
        word.to_string()
    };
    let show_state = r.width >= 30;
    let sw = super::width(&state, cx.uni()) + 2;
    let room = right.saturating_sub(hx + if show_state { sw + 1 } else { 2 });
    let tst = if pick == Some(true) {
        cx.st(Tok::Accent).add_modifier(Modifier::BOLD)
    } else {
        cx.ink(Tok::Fg, 1.0).add_modifier(Modifier::BOLD)
    };
    cx.put(
        buf,
        hx,
        r.y,
        room,
        &fit(&x.title, room as usize, cx.uni()),
        tst,
    );
    if show_state {
        cx.put_r(buf, right, r.y, &format!("{g} {state}"), sst);
    } else {
        cx.put(buf, right - 1, r.y, 1, g, sst);
    }
    // Body: a little screen.
    let body = Rect::new(
        left,
        r.y + 1,
        right.saturating_sub(left),
        r.height.saturating_sub(2),
    );
    let base = if cx.dark() {
        (0x0c, 0x0e, 0x11)
    } else {
        (0xff, 0xff, 0xff)
    };
    if body.height > 0 {
        if cx.has_surface() {
            cx.fill(buf, body, cx.rgb_bg(base));
        }
        let text = Rect::new(
            body.x + 1,
            body.y,
            body.width.saturating_sub(2),
            body.height,
        );
        screen_text(cx, buf, text, x, cx.c.has_color());
    }
    // Caption: provider, folder, activity and age.
    let fy = r.bottom() - 1;
    if fy > r.y + 1 {
        let (pname, ptok) = cx.prov(x);
        let mut fx = left;
        fx += cx.put(buf, fx, fy, right.saturating_sub(fx), pname, cx.st(ptok));
        fx += 2;
        let age = fmt::age(x.last_activity_secs);
        let aw = age.len() as u16;
        let sw = if r.width >= 34 { 8 } else { 0 };
        let rest = right.saturating_sub(fx + aw + sw + 2);
        cx.put(
            buf,
            fx,
            fy,
            rest,
            &fit(&x.cwd, rest as usize, cx.uni()),
            cx.ink(Tok::Muted, 0.9),
        );
        cx.put_r(buf, right, fy, &age, cx.ink(Tok::Muted, 0.9));
        if sw > 0 {
            let v: Vec<Option<f32>> = x.activity.iter().map(|v| Some(*v)).collect();
            // A shell's neutral ink is black or white in 16 colours, the heaviest mark on
            // the row: its activity is gray there.
            let tok = if x.state == crate::ui::views::SessionState::Idle {
                Tok::Faint
            } else if ptok == Tok::Shell && !cx.tc() {
                Tok::Muted
            } else {
                ptok
            };
            spark(cx, buf, right - aw - 1 - sw, fy, sw, &v, cx.ink(tok, 0.9));
        }
    }
    if let Some(d) = d {
        cx.hit(r, format!("session:{}:{}", d.id, x.id));
    }
}
