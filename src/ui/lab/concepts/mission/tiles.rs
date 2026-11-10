//! Tiles: device tiles with micro-charts and session thumbnails that show the live tail
//! of the real screen in its own colours, so sessions are recognised by their look.
use super::{fit, fmt, mix, to_rgb, Cx, Rgb};
use crate::ui::lab::{ansi_lines, DeviceStatus, LabDevice, LabSession, Tok};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
};

/// Selection state of a tile: None, Some(false) = selected in an unfocused zone,
/// Some(true) = selected in the focused zone.
pub(super) type Pick = Option<bool>;

pub(super) fn tile_bg(cx: &Cx, pick: Pick) -> Style {
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
        for y in r.y..r.bottom() {
            cx.put(buf, r.x, y, 1, cx.g("▌", if f { ">" } else { " " }), st);
        }
    }
}

/// Eighth-block area chart (newest on the right), shaded by height.
pub(super) fn chart(cx: &Cx, buf: &mut Buffer, r: Rect, vals: &[f32], tok: Option<Tok>) {
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
        let st = match tok {
            Some(t) => cx.ink(t, 0.85),
            None => cx.heat(v),
        };
        for row in 0..r.height {
            let y = r.bottom() - 1 - row;
            let fill = (total - i32::from(row) * 8).clamp(0, 8) as usize;
            let s = if cx.uni() {
                E[fill]
            } else if fill >= 6 {
                "#"
            } else if fill >= 2 {
                if row == 0 {
                    "="
                } else {
                    ":"
                }
            } else if row == 0 {
                "_"
            } else {
                " "
            };
            let shade = if cx.tc() && fill > 0 {
                let a = 0.45 + 0.55 * (f32::from(row) + 1.0) / f32::from(r.height);
                match tok {
                    Some(t) => cx.ink(t, a),
                    None => st,
                }
            } else {
                st
            };
            if fill == 0 && row == 0 && cx.uni() {
                cx.put(buf, r.x + i as u16, y, 1, "▁", cx.ink(Tok::Faint, 0.8));
            } else if fill > 0 || !cx.uni() {
                cx.put(buf, r.x + i as u16, y, 1, s, shade);
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
    if w == 0 || vals.is_empty() {
        return;
    }
    // Scale between the window's min and max so variation shows, not a solid block.
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
    cx.put(buf, x, y, w as u16, &s, st);
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

pub(super) fn mesh_badges(cx: &Cx, buf: &mut Buffer, x: u16, y: u16, d: &LabDevice) -> u16 {
    let mut w = 0;
    for m in &d.meshes {
        let i = cx.s.meshes.iter().position(|x| &x.id == m).unwrap_or(0);
        let tok = if i == 0 { Tok::Mesh } else { Tok::Id(6) };
        w += cx.put(buf, x + w, y, 1, cx.g("◆", "*"), cx.st(tok));
    }
    w
}

pub(super) fn pips(
    cx: &Cx,
    buf: &mut Buffer,
    right: u16,
    y: u16,
    d: &LabDevice,
    max: usize,
) -> u16 {
    let ss: Vec<&LabSession> = cx.s.sessions_on(&d.id).collect();
    if ss.is_empty() {
        return 0;
    }
    let shown = ss.len().min(max);
    let mut x = right.saturating_sub(shown as u16 + if ss.len() > max { 2 } else { 0 });
    let x0 = x;
    for s in ss.iter().take(shown) {
        let (_, _, st) = cx.sstate(s);
        x += cx.put(buf, x, y, 1, cx.g("▮", "#"), st);
    }
    if ss.len() > max {
        cx.put(
            buf,
            x,
            y,
            2,
            &format!("+{}", ss.len() - max),
            cx.st(Tok::Muted),
        );
    }
    right - x0
}

/// Device tile: identity, cpu history, rtt / temperature and session pips.
pub(super) fn device_tile(cx: &Cx, buf: &mut Buffer, r: Rect, d: &LabDevice, pick: Pick) {
    if r.width < 8 || r.height < 2 {
        return;
    }
    cx.fill(buf, r, tile_bg(cx, pick));
    pick_bar(cx, buf, r, pick);
    let x = r.x + 1;
    let right = r.right().saturating_sub(1);
    let mut cx0 = x;
    cx0 += cx.dchip(buf, cx0, r.y, d, false);
    cx0 += 1;
    let (g, gst) = cx.status(d.status);
    let up = cx.up(d);
    let glyph = r.width >= 20 || !up;
    let name_room = if glyph {
        right.saturating_sub(cx0 + 2)
    } else {
        r.right().saturating_sub(cx0)
    };
    let nst = if up {
        cx.ink(Tok::Fg, 1.0).add_modifier(Modifier::BOLD)
    } else {
        cx.st(Tok::Muted)
    };
    cx.put(
        buf,
        cx0,
        r.y,
        name_room,
        &fit(&d.name, name_room as usize, cx.uni()),
        nst,
    );
    if glyph {
        cx.put(buf, right - 1, r.y, 1, g, gst);
    }
    cx.hit(r, format!("device:{}", d.id));
    let body = r.height.saturating_sub(2);
    let foot = r.bottom() - 1;
    if r.height >= 3 {
        match &d.metrics {
            Some(m) if up && body > 0 => {
                let pct = format!("{:.0}%", m.cpu * 100.0);
                let cw = r.width.saturating_sub(pct.len() as u16 + 3);
                chart(cx, buf, Rect::new(x, r.y + 1, cw, body), &m.cpu_hist, None);
                cx.put_r(
                    buf,
                    right,
                    r.y + 1,
                    &pct,
                    cx.heat(m.cpu).add_modifier(Modifier::BOLD),
                );
                if body >= 2 {
                    cx.put_r(buf, right, r.y + 2, "cpu", cx.ink(Tok::Muted, 0.8));
                }
            }
            _ if body > 0 => {
                let why = if d.status == DeviceStatus::Unknown {
                    "not probed yet"
                } else {
                    "unreachable"
                };
                cx.put(buf, x, r.y + 1, r.width - 2, why, cx.ink(Tok::Err, 0.9));
                if body >= 2 {
                    let ev = fit(&d.evidence, usize::from(r.width - 2), cx.uni());
                    cx.put(buf, x, r.y + 2, r.width - 2, &ev, cx.st(Tok::Muted));
                }
            }
            _ => {}
        }
    }
    if r.height >= 2 && foot > r.y {
        let mut fx = x;
        fx += mesh_badges(cx, buf, fx, foot, d);
        if fx > x {
            fx += 1;
        }
        let meta = cx.meta(d);
        let mst = if up {
            cx.ink(Tok::Fg, 0.85)
        } else {
            cx.ink(Tok::Err, 0.8)
        };
        fx += cx.put(buf, fx, foot, right.saturating_sub(fx), &meta, mst);
        if let Some(t) = d.metrics.as_ref().and_then(|m| m.temp_c).filter(|_| up) {
            let s = format!(" {t:.0}{}", cx.g("°", "C"));
            let hv = ((t - 35.0) / 50.0).clamp(0.0, 1.0);
            if fx + s.chars().count() as u16 + 4 < right {
                fx += cx.put(buf, fx, foot, 6, &s, cx.heat(hv));
            }
        }
        let _ = fx;
        pips(cx, buf, right, foot, d, 4);
    }
}

/// Compact one-row device line for phones and narrow strips.
pub(super) fn device_row(cx: &Cx, buf: &mut Buffer, r: Rect, d: &LabDevice, pick: Pick) {
    if r.width < 10 {
        return;
    }
    let bg = match pick {
        Some(_) => tile_bg(cx, pick),
        None => Style::default(),
    };
    cx.fill(buf, r, bg);
    pick_bar(cx, buf, Rect::new(r.x, r.y, 1, 1), pick);
    let mut x = r.x + 1;
    x += cx.dchip(buf, x, r.y, d, false);
    x += 1;
    let (g, gst) = cx.status(d.status);
    let right = r.right().saturating_sub(1);
    let tail = 13u16.min(r.width / 2);
    let up = cx.up(d);
    let nst = if up {
        cx.ink(Tok::Fg, 1.0)
    } else {
        cx.st(Tok::Muted)
    };
    let room = right.saturating_sub(x + tail + 2);
    x += cx.put(
        buf,
        x,
        r.y,
        room,
        &fit(&d.name, room as usize, cx.uni()),
        nst,
    );
    let _ = x;
    let tx = right.saturating_sub(tail);
    cx.put(buf, tx, r.y, 1, g, gst);
    let meta = cx.meta(d);
    let mw = cx.put(buf, tx + 2, r.y, 6, &meta, cx.st(Tok::Muted));
    let pw = pips(cx, buf, right, r.y, d, 3);
    if let Some(m) = d.metrics.as_ref().filter(|_| up) {
        let sx = tx + 2 + mw + 1;
        let sw = right.saturating_sub(sx + pw + 1);
        if sw >= 3 {
            let v: Vec<Option<f32>> = m.cpu_hist.iter().map(|v| Some(*v)).collect();
            spark(cx, buf, sx, r.y, sw, &v, cx.heat(m.cpu));
        }
    }
    cx.hit(r, format!("device:{}", d.id));
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
    let text = |l: &ratatui::text::Line| {
        l.spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect::<String>()
    };
    let used = lines
        .iter()
        .rposition(|l| !boring(text(l).trim()))
        .map_or(0, |i| i + 1);
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
    if cx.c.color == crate::ui::lab::ColorLevel::C16 {
        // No tinted backgrounds in 16 colours: a coloured background (a diff row) becomes
        // ink of its hue instead, so + stays green and - stays red without a loud block.
        let ink = |c: ratatui::style::Color| -> Option<ratatui::style::Color> {
            if crate::ui::lab::caps::named_index(c).is_some_and(|i| i < 16) {
                return Some(c);
            }
            to_rgb(c).map(|c| crate::ui::lab::caps::indexed_16(ansi16(c, false)))
        };
        let from_bg = s.bg.and_then(to_rgb).and_then(|c| {
            let (hi, lo) = (c.0.max(c.1).max(c.2), c.0.min(c.1).min(c.2));
            (u16::from(hi - lo) * 4 > u16::from(hi) && hi > 24).then(|| {
                let full = |v: u8| ((u16::from(v - lo) * 255) / u16::from(hi - lo).max(1)) as u8;
                crate::ui::lab::caps::indexed_16(ansi16((full(c.0), full(c.1), full(c.2)), true))
            })
        });
        let fg = s.fg.and_then(ink);
        let grey = |c: Option<ratatui::style::Color>| {
            c.and_then(crate::ui::lab::caps::named_index)
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

/// Session thumbnail: header (keys, title, state), the screen's live tail and a caption
/// (provider, folder, activity, age).
pub(super) fn thumb(cx: &Cx, buf: &mut Buffer, r: Rect, x: &LabSession, pick: Pick) {
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
    hx += addr(cx, buf, hx, r.y, x);
    hx += 1;
    let (g, word, sst) = cx.sstate(x);
    let state = if x.state == crate::ui::lab::SessionState::Idle {
        format!("{word} {}", fmt::age(x.last_activity_secs))
    } else {
        word.to_string()
    };
    let show_state = r.width >= 30;
    let sw = super::width(&state, cx.uni()) + 2;
    let room = right.saturating_sub(hx + if show_state { sw + 1 } else { 2 });
    let tst = cx.ink(Tok::Fg, 1.0).add_modifier(Modifier::BOLD);
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
            let tok = if x.state == crate::ui::lab::SessionState::Idle {
                Tok::Faint
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
