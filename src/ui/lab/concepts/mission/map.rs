//! The network schematic, drawn as a left-to-right flow: this device is a pill on the
//! left, the devices it reaches directly form the next column and devices reached through
//! them the one after. Links are smooth braille S-curves (ASCII: dotted strokes) coloured
//! by network, with brighter "packets" travelling the directions access is allowed.
//! Nothing here dispatches anything.
use super::{fmt, mix, Cx, Rgb, Size};
use crate::ui::lab::{Access, DeviceStatus, EdgeKind, LabEdge, Tok};
use ratatui::{buffer::Buffer, layout::Rect, style::Modifier};

/// What the schematic highlights.
#[derive(Clone, Copy)]
pub(super) enum Sel<'a> {
    /// A device; `true` when the schematic has keyboard focus.
    Node(&'a str, bool),
    Edge(&'a str),
}

#[derive(Clone, Copy, PartialEq, PartialOrd, Debug)]
enum Mode {
    Chip,
    Name,
    Full,
}

/// A placed device: status dot at `x`, label from `x + 2`, `w` columns wide in total.
struct Node {
    id: String,
    col: usize,
    x: u16,
    y: u16,
    w: u16,
}

struct Plan {
    nodes: Vec<Node>,
    pill: Rect,
    /// Two-row nodes (name, then evidence and mesh badges).
    tall: bool,
    mode: Mode,
    /// "+N" markers: (x, y, hidden count).
    more: Vec<(u16, u16, usize)>,
}

/// Device ids in selection order: column by column, top to bottom, the viewer last.
pub(super) fn order(cx: &Cx, band: Rect) -> Vec<String> {
    let mut v: Vec<String> = plan(cx, band).nodes.into_iter().map(|n| n.id).collect();
    v.push(cx.s.viewer.clone());
    v
}

pub(super) fn mesh_tok(cx: &Cx, id: &str) -> Tok {
    match cx.s.meshes.iter().position(|m| m.id == id) {
        Some(0) | None => Tok::Mesh,
        Some(_) => Tok::Id(6),
    }
}

/// Jump edges are drawn through the hop they use, not as their own line.
fn routed(e: &LabEdge) -> bool {
    !matches!(e.kind, EdgeKind::Jump(_))
}

fn linked(cx: &Cx, a: &str, b: &str) -> bool {
    cx.s.edges
        .iter()
        .any(|e| routed(e) && ((e.from == a && e.to == b) || (e.from == b && e.to == a)))
}

fn badges_w(cx: &Cx, id: &str) -> u16 {
    cx.s.device(id).map_or(0, |d| d.meshes.len() as u16)
}

/// Room for the cpu sparkline on a tall node's evidence row.
fn spark_w(cx: &Cx, id: &str) -> u16 {
    let live =
        cx.s.device(id)
            .is_some_and(|d| d.metrics.is_some() && cx.up(d));
    if live && cx.size() >= Size::M {
        7
    } else {
        0
    }
}

fn label_w(cx: &Cx, id: &str, mode: Mode, tall: bool, nmax: u16) -> u16 {
    let Some(d) = cx.s.device(id) else { return 0 };
    let name = (d.name.chars().count() as u16).min(nmax);
    let meta = cx.meta(d).chars().count() as u16;
    match mode {
        Mode::Chip => 5,
        Mode::Name => 6 + name,
        Mode::Full if tall => 6 + name.max(meta + 1 + badges_w(cx, id) + spark_w(cx, id)),
        Mode::Full => 7 + name + meta,
    }
}

fn pill_w(cx: &Cx, mode: Mode) -> u16 {
    let name =
        cx.s.device(&cx.s.viewer)
            .map_or(4, |d| d.name.chars().count() as u16);
    match mode {
        Mode::Chip => 5,
        Mode::Name => 7 + name,
        Mode::Full => 7 + name + 14,
    }
}

/// Columns by hop count from the viewer (at most three), ordered to limit crossings.
fn columns(cx: &Cx) -> Vec<Vec<String>> {
    let s = cx.s;
    let v = s.viewer.as_str();
    let mut cols: Vec<Vec<String>> = Vec::new();
    let mut seen: Vec<&str> = vec![v];
    let mut frontier: Vec<&str> = vec![v];
    while !frontier.is_empty() && cols.len() < 3 {
        let mut next: Vec<&str> = Vec::new();
        for d in &s.devices {
            let id = d.id.as_str();
            if !seen.contains(&id) && frontier.iter().any(|p| linked(cx, p, id)) {
                next.push(id);
            }
        }
        if next.is_empty() {
            break;
        }
        seen.extend(next.iter().copied());
        cols.push(next.iter().map(|x| x.to_string()).collect());
        frontier = next;
    }
    // Anything unreached (no routed link at all) joins the last column.
    let rest: Vec<String> = s
        .devices
        .iter()
        .map(|d| d.id.clone())
        .filter(|id| !seen.contains(&id.as_str()))
        .collect();
    if !rest.is_empty() {
        if cols.is_empty()
            || cols.len() < 3 && cols.last().is_some_and(|c| c.len() > rest.len() * 2)
        {
            cols.push(rest);
        } else if let Some(last) = cols.last_mut() {
            last.extend(rest);
        }
    }
    // First column grouped by mesh; later columns follow their parents.
    let group = |d: &str| {
        s.meshes
            .iter()
            .position(|m| m.members.iter().any(|x| x == d))
            .unwrap_or(s.meshes.len())
    };
    let idx = |d: &str| s.devices.iter().position(|x| x.id == d).unwrap_or(0);
    if let Some(c) = cols.first_mut() {
        c.sort_by_key(|d| (group(d), idx(d)));
    }
    for k in 1..cols.len() {
        let prev = cols[k - 1].clone();
        let parent = |d: &str| {
            prev.iter()
                .position(|p| linked(cx, p, d))
                .unwrap_or(prev.len())
        };
        cols[k].sort_by_key(|d| (parent(d), idx(d)));
    }
    cols
}

fn plan(cx: &Cx, band: Rect) -> Plan {
    let cols = columns(cx);
    let h = band.height;
    let most = cols.iter().map(|c| c.len()).max().unwrap_or(0) as u16;
    let tall = cx.size() >= Size::S && most > 0 && h + 1 >= most * 3;
    let cap = |n: usize| -> usize {
        let fit = if tall {
            usize::from((h + 1) / 3)
        } else {
            usize::from(h)
        };
        n.min(fit.max(1))
    };
    // Richest label mode whose columns fit with breathing room between them.
    let mut mode = Mode::Chip;
    let mut nmax = 8;
    let mut widths: Vec<u16> = Vec::new();
    let tries = [
        (Mode::Full, 22),
        (Mode::Full, 16),
        (Mode::Name, 22),
        (Mode::Name, 16),
        (Mode::Name, 12),
        (Mode::Chip, 8),
    ];
    for (m, nm) in tries {
        mode = m;
        nmax = nm;
        widths = cols
            .iter()
            .map(|c| {
                c.iter()
                    .take(cap(c.len()))
                    .map(|id| label_w(cx, id, m, tall, nmax))
                    .max()
                    .unwrap_or(0)
                    .max(4)
            })
            .collect();
        let min_gap = if m == Mode::Chip { 4 } else { 8 };
        let need = 2 + pill_w(cx, m) + widths.iter().sum::<u16>() + min_gap * widths.len() as u16;
        if need <= band.width {
            break;
        }
    }
    let pw = pill_w(cx, mode).min(band.width);
    let fixed = pw + widths.iter().sum::<u16>();
    let gaps = widths.len() as u16;
    let free = band.width.saturating_sub(fixed);
    let gap = if gaps == 0 {
        0
    } else {
        (free / (gaps + 1)).clamp(3, 34)
    };
    let margin = free.saturating_sub(gap * gaps) / 2;
    let cy = band.y + h / 2;
    let pill_h = if h >= 5 { 3 } else { 1 };
    let pill = Rect::new(band.x + margin, cy.saturating_sub(pill_h / 2), pw, pill_h);

    let mut nodes = Vec::new();
    let mut more = Vec::new();
    let mut x = pill.right() + gap;
    let rows = if tall { 2 } else { 1 };
    for (k, c) in cols.iter().enumerate() {
        let shown = if cap(c.len()) < c.len() {
            cap(c.len()).saturating_sub(1)
        } else {
            c.len()
        };
        let slots = shown + usize::from(shown < c.len());
        let n = slots.max(1) as u16;
        let min_sp = if tall { 3 } else { 1 };
        let sp = ((h + 1) / n).clamp(min_sp, if tall { 5 } else { 3 });
        let span = (n - 1) * sp + rows;
        let y0 = band.y + h.saturating_sub(span) / 2;
        for (i, id) in c.iter().take(shown).enumerate() {
            let w = label_w(cx, id, mode, tall, nmax);
            nodes.push(Node {
                id: id.clone(),
                col: k + 1,
                x,
                y: y0 + i as u16 * sp,
                w,
            });
        }
        if shown < c.len() {
            more.push((x + 2, y0 + shown as u16 * sp, c.len() - shown));
        }
        x += widths.get(k).copied().unwrap_or(0) + gap;
    }
    Plan {
        nodes,
        pill,
        tall,
        mode,
        more,
    }
}

/// Braille canvas (2x4 dots per cell); in ASCII it keeps one stroke char per cell.
struct Dots {
    r: Rect,
    mask: Vec<u8>,
    lvl: Vec<u8>,
    tok: Vec<Tok>,
    ch: Vec<char>,
}

impl Dots {
    fn new(r: Rect) -> Self {
        let n = usize::from(r.width) * usize::from(r.height);
        Self {
            r,
            mask: vec![0; n],
            lvl: vec![0; n],
            tok: vec![Tok::Faint; n],
            ch: vec![' '; n],
        }
    }
    /// Plot subpixel (px, py) in frame coordinates (x*2, y*4).
    fn dot(&mut self, px: i32, py: i32, lvl: u8, tok: Tok, slope: (f32, f32)) {
        let (cx, cy) = (px.div_euclid(2), py.div_euclid(4));
        if cx < i32::from(self.r.x) || cy < i32::from(self.r.y) {
            return;
        }
        let (cx, cy) = (cx as u16, cy as u16);
        if cx >= self.r.right() || cy >= self.r.bottom() {
            return;
        }
        let i = usize::from(cy - self.r.y) * usize::from(self.r.width) + usize::from(cx - self.r.x);
        let (sx, sy) = (px.rem_euclid(2), py.rem_euclid(4));
        const BITS: [[u8; 4]; 2] = [[0x01, 0x02, 0x04, 0x40], [0x08, 0x10, 0x20, 0x80]];
        self.mask[i] |= BITS[sx as usize][sy as usize];
        if lvl >= self.lvl[i] {
            self.lvl[i] = lvl;
            self.tok[i] = tok;
            let (dx, dy) = slope;
            let (ax, ay) = (dx.abs(), dy.abs() * 2.0);
            self.ch[i] = if lvl <= 1 {
                '.'
            } else if ax > ay * 2.0 {
                '-'
            } else if ay > ax * 2.0 {
                '|'
            } else if (dx > 0.0) == (dy > 0.0) {
                '\\'
            } else {
                '/'
            };
        }
    }
    fn flush(&self, cx: &Cx, buf: &mut Buffer) {
        for (i, &m) in self.mask.iter().enumerate() {
            if m == 0 {
                continue;
            }
            let x = self.r.x + (i % usize::from(self.r.width)) as u16;
            let y = self.r.y + (i / usize::from(self.r.width)) as u16;
            let lvl = self.lvl[i];
            let st = match lvl {
                0 => cx.ink(self.tok[i], 0.16),
                1 => cx.ink(self.tok[i], 0.42),
                2 => cx.ink(self.tok[i], 0.7),
                _ => cx.ink(self.tok[i], 1.0).add_modifier(Modifier::BOLD),
            };
            let sym = if cx.uni() {
                char::from_u32(0x2800 + u32::from(m)).unwrap_or(' ')
            } else if lvl == 0 {
                continue;
            } else {
                self.ch[i]
            };
            let c = &mut buf[(x, y)];
            c.set_char(sym);
            c.set_style(st);
        }
    }

    /// Cubic bezier in subpixels; dotted base plus moving packets.
    #[allow(clippy::too_many_arguments)]
    fn curve(
        &mut self,
        p: [(f32, f32); 4],
        tok: Tok,
        base: u8,
        every: u32,
        packets: &[(bool, u32)],
        phase: u32,
    ) {
        let at = |t: f32| {
            let u = 1.0 - t;
            let w = [u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t];
            (
                w[0] * p[0].0 + w[1] * p[1].0 + w[2] * p[2].0 + w[3] * p[3].0,
                w[0] * p[0].1 + w[1] * p[1].1 + w[2] * p[2].1 + w[3] * p[3].1,
            )
        };
        // Arc-length samples, one per subpixel.
        let mut pts: Vec<(i32, i32, f32, f32)> = Vec::new();
        let steps = 400;
        let mut prev = at(0.0);
        let mut last = (i32::MIN, i32::MIN);
        for k in 0..=steps {
            let q = at(k as f32 / steps as f32);
            let (ix, iy) = (q.0.round() as i32, q.1.round() as i32);
            if (ix, iy) != last {
                pts.push((ix, iy, q.0 - prev.0, q.1 - prev.1));
                last = (ix, iy);
                prev = q;
            }
        }
        let n = pts.len() as u32;
        // Skip a few dots at each end so the curve meets labels cleanly.
        let trim = 2;
        for (i, &(x, y, dx, dy)) in pts.iter().enumerate() {
            let i = i as u32;
            if i < trim || i + trim >= n {
                continue;
            }
            if i % every == 0 {
                self.dot(x, y, base, tok, (dx, dy));
            }
            for &(fwd, gap) in packets {
                let pos = if fwd { i } else { n - 1 - i };
                let k = (pos + gap * 4 - phase % gap) % gap;
                if k < 5 {
                    let lvl = if k < 2 { 3 } else { 2 };
                    let s = if fwd { (dx, dy) } else { (-dx, -dy) };
                    self.dot(x, y, lvl, tok, s);
                }
            }
        }
    }
}

fn edge_tok(cx: &Cx, e: &LabEdge) -> Tok {
    if !e.up {
        return Tok::Err;
    }
    match &e.kind {
        EdgeKind::Mesh(m) => mesh_tok(cx, m),
        EdgeKind::Lan => Tok::Lan,
        EdgeKind::Direct | EdgeKind::Jump(_) => Tok::Info,
    }
}

/// Policy shorthand, with the arrow pointing the way access is allowed on screen.
pub(super) fn policy(cx: &Cx, e: &LabEdge, from_left: bool) -> String {
    let (f, r) = (e.forward, e.reverse);
    let (to_right, to_left) = if from_left { (f, r) } else { (r, f) };
    let word = |a: Access| match a {
        Access::Key => "key",
        Access::Password => "pwd",
        Access::Denied => "off",
    };
    match (to_right, to_left) {
        (Access::Denied, Access::Denied) => format!("{} off", cx.g("⊘", "x")),
        (a, Access::Denied) => format!("{} {}", cx.g("→", "->"), word(a)),
        (Access::Denied, a) => format!("{} {}", cx.g("←", "<-"), word(a)),
        (a, b) if a == b => format!("{} {}", cx.g("⇄", "<>"), word(a)),
        (a, b) => format!("{} {}/{}", cx.g("⇄", "<>"), word(a), word(b)),
    }
}

pub(super) fn draw(cx: &Cx, buf: &mut Buffer, band: Rect, sel: Sel) {
    if band.width < 16 || band.height < 3 {
        return;
    }
    let s = cx.s;
    let p = plan(cx, band);
    let find = |id: &str| p.nodes.iter().find(|n| n.id == id);
    let sub = |x: u16, y: u16| (f32::from(x) * 2.0 + 1.0, f32::from(y) * 4.0 + 2.0);
    let pill_mid = p.pill.y + p.pill.height / 2;
    let hl = |e: &LabEdge| match sel {
        Sel::Node(id, _) => e.from == id || e.to == id,
        Sel::Edge(id) => e.id == id,
    };

    // A faint blueprint grid gives the band depth without drawing boxes.
    if cx.uni() && cx.tc() && band.height >= 7 {
        let st = cx.ink(Tok::Faint, if cx.dark() { 0.55 } else { 0.7 });
        for y in (band.y..band.bottom()).step_by(2) {
            for x in (band.x + 2..band.right().saturating_sub(1)).step_by(4) {
                cx.put(buf, x, y, 1, "·", st);
            }
        }
    }

    let mut dots = Dots::new(band);
    let transfers: Vec<(&str, &str)> = s
        .transfers
        .iter()
        .map(|t| (t.from.as_str(), t.to.as_str()))
        .collect();
    let mut chips: Vec<(u16, u16, String, Tok, String)> = Vec::new();
    for e in s.edges.iter().filter(|e| routed(e)) {
        // (anchor, column) for each end, in from/to order.
        let end = |id: &str, outgoing: bool| -> Option<((f32, f32), usize)> {
            if id == s.viewer {
                return Some((sub(p.pill.right(), pill_mid), 0));
            }
            let n = find(id)?;
            let x = if outgoing {
                n.x + n.w + 1
            } else {
                n.x.saturating_sub(2)
            };
            Some((sub(x, n.y), n.col))
        };
        let (Some(fa), Some(ta)) = (end(&e.from, true), end(&e.to, true)) else {
            continue;
        };
        // Lower column on the left; the curve leaves its right side and enters the dot.
        let from_left = fa.1 <= ta.1;
        let (l, r) = if from_left {
            (&e.from, &e.to)
        } else {
            (&e.to, &e.from)
        };
        let (Some(la), Some(ra)) = (end(l, true), end(r, false)) else {
            continue;
        };
        let (p0, p3, p1, p2) = if la.1 == ra.1 {
            // Same column: an arc out to the right of both labels.
            let (a, b) = (la.0, end(r, true).map_or(ra.0, |x| x.0));
            let dy = (a.1 - b.1).abs() / 4.0;
            let xr = a.0.max(b.0) + 4.0 + dy * 1.3;
            (a, b, (xr, a.1), (xr, b.1))
        } else {
            let (a, b) = (la.0, ra.0);
            let k = (b.0 - a.0) * 0.5;
            (a, b, (a.0 + k, a.1), (b.0 - k, b.1))
        };
        let pts = if *l == e.from {
            [p0, p1, p2, p3]
        } else {
            [p3, p2, p1, p0]
        };
        let tok = edge_tok(cx, e);
        let lit = hl(e);
        let busy = transfers
            .iter()
            .any(|(f, t)| (*f == e.from && *t == e.to) || (*f == e.to && *t == e.from));
        let gap = if busy { 10 } else { 22 };
        let mut packets = Vec::new();
        if e.up {
            if e.forward != Access::Denied {
                packets.push((true, gap));
            }
            if e.reverse != Access::Denied {
                packets.push((false, gap + 3));
            }
        }
        let closed = !e.up || (e.forward == Access::Denied && e.reverse == Access::Denied);
        let every = if closed { 4 } else { 2 };
        let edge_mode = matches!(sel, Sel::Edge(_));
        let base = if lit {
            2
        } else if edge_mode {
            0
        } else {
            1
        };
        if edge_mode && !lit {
            packets.clear();
        }
        let seed =
            e.id.bytes()
                .fold(0u32, |h, b| h.wrapping_mul(31).wrapping_add(u32::from(b)));
        dots.curve(pts, tok, base, every, &packets, cx.c.frame * 3 + seed % 17);
        let mid = (
            (p0.0 + 3.0 * p1.0 + 3.0 * p2.0 + p3.0) / 8.0,
            (p0.1 + 3.0 * p1.1 + 3.0 * p2.1 + p3.1) / 8.0,
        );
        let (mx, my) = ((mid.0 / 2.0) as u16, (mid.1 / 4.0) as u16);
        if !e.up {
            chips.push((mx, my, cx.g("✕", "x").into(), Tok::Err, e.id.clone()));
        } else if lit && (cx.size() >= Size::M || matches!(sel, Sel::Edge(_))) {
            chips.push((mx, my, policy(cx, e, *l == e.from), tok, e.id.clone()));
        }
    }
    dots.flush(cx, buf);

    for (mx, my, text, tok, id) in chips {
        let w = super::width(&text, cx.uni()) + 2;
        let x = mx.saturating_sub(w / 2).max(band.x);
        let st = if cx.has_surface() {
            cx.rgb_bg(mix(cx.bg0(), cx.rgb(tok), 0.2))
                .patch(cx.ink(tok, 1.0))
        } else {
            cx.ink(tok, 1.0).add_modifier(Modifier::BOLD)
        };
        let n = cx.put(buf, x, my, w, &format!(" {text} "), st);
        cx.hit(Rect::new(x, my, n, 1), format!("edge:{id}"));
    }

    draw_pill(cx, buf, p.pill, p.mode);

    let (sel_id, focused) = match sel {
        Sel::Node(id, f) => (Some(id), f),
        Sel::Edge(_) => (None, false),
    };
    for n in &p.nodes {
        let Some(d) = s.device(&n.id) else { continue };
        let rows = if p.tall { 2 } else { 1 };
        let area = Rect::new(n.x, n.y, n.w, rows);
        let picked = sel_id == Some(n.id.as_str());
        if picked && cx.has_surface() {
            let r = Rect::new(n.x.saturating_sub(1), n.y, n.w + 2, rows);
            cx.fill(
                buf,
                r,
                if focused {
                    cx.st(Tok::AccentBg)
                } else {
                    cx.st(Tok::SelectDimBg)
                },
            );
        }
        if picked {
            if !cx.has_surface() {
                let st = if focused {
                    cx.st(Tok::Accent).add_modifier(Modifier::BOLD)
                } else {
                    cx.st(Tok::Muted)
                };
                cx.put(buf, n.x.saturating_sub(2), n.y, 1, cx.g("▸", ">"), st);
            }
        }
        let (g, gst) = cx.status(d.status);
        cx.put(buf, n.x, n.y, 1, g, gst);
        let mut x = n.x + 2;
        x += cx.dchip(buf, x, n.y, d, false);
        let end = n.x + n.w;
        if p.mode > Mode::Chip {
            x += 1;
            let up = cx.up(d);
            let nst = if up {
                cx.ink(Tok::Fg, 1.0).add_modifier(Modifier::BOLD)
            } else {
                cx.ink(Tok::Muted, 0.9)
            };
            // Without tints the selected name itself carries the highlight.
            let nst = if picked && focused && !cx.has_surface() {
                nst.add_modifier(Modifier::REVERSED)
            } else {
                nst
            };
            let meta = cx.meta(d);
            let inline = p.mode == Mode::Full && !p.tall;
            let room = end.saturating_sub(
                x + if inline {
                    meta.chars().count() as u16 + 1
                } else {
                    0
                },
            );
            x += cx.put(
                buf,
                x,
                n.y,
                room,
                &fmt::fit(&d.name, room as usize, cx.uni()),
                nst,
            );
            let mst = match d.status {
                DeviceStatus::Down => cx.ink(Tok::Err, 0.85),
                DeviceStatus::Stale => cx.ink(Tok::Warn, 0.9),
                _ => cx.ink(Tok::Muted, 0.95),
            };
            if inline {
                cx.put(buf, x + 1, n.y, end.saturating_sub(x + 1), &meta, mst);
            } else if p.tall && p.mode == Mode::Full {
                let y = n.y + 1;
                let mut mx = n.x + 6;
                mx += cx.put(buf, mx, y, end.saturating_sub(mx), &meta, mst);
                mx += 1;
                mx += super::tiles::mesh_badges(cx, buf, mx, y, d);
                if let Some(m) = d.metrics.as_ref().filter(|_| spark_w(cx, &d.id) > 0) {
                    let v: Vec<Option<f32>> = m.cpu_hist.iter().map(|v| Some(*v)).collect();
                    let x0 = end.saturating_sub(6).max(mx + 1);
                    super::tiles::spark(cx, buf, x0, y, end.saturating_sub(x0), &v, cx.heat(m.cpu));
                }
            }
        }
        cx.hit(area, format!("device:{}", d.id));
    }

    for &(x, y, m) in &p.more {
        let t = format!("+{m} more");
        let w = cx.put(
            buf,
            x,
            y,
            band.right().saturating_sub(x),
            &t,
            cx.st(Tok::Muted),
        );
        cx.hit(Rect::new(x, y, w, 1), "palette:devices");
    }
}

/// This device: a soft accent capsule (half-block caps; a rounded box without tints).
fn draw_pill(cx: &Cx, buf: &mut Buffer, pill: Rect, mode: Mode) {
    let Some(d) = cx.s.device(&cx.s.viewer) else {
        return;
    };
    let tint: Rgb = mix(
        cx.bg0(),
        cx.rgb(Tok::Accent),
        if cx.dark() { 0.22 } else { 0.16 },
    );
    let mid = pill.y + pill.height / 2;
    if pill.height >= 3 {
        if cx.has_surface() {
            let half = cx.rgb_fg(tint);
            for x in pill.x..pill.right() {
                cx.put(buf, x, pill.y, 1, "▄", half);
                cx.put(buf, x, pill.bottom() - 1, 1, "▀", half);
            }
        } else {
            let st = cx.st(Tok::Accent);
            let w = pill.width.saturating_sub(2) as usize;
            let (tl, tr, bl, br, hz) = if cx.uni() {
                ("╭", "╮", "╰", "╯", "─")
            } else {
                ("+", "+", "+", "+", "-")
            };
            cx.put(
                buf,
                pill.x,
                pill.y,
                pill.width,
                &format!("{tl}{}{tr}", hz.repeat(w)),
                st,
            );
            cx.put(
                buf,
                pill.x,
                pill.bottom() - 1,
                pill.width,
                &format!("{bl}{}{br}", hz.repeat(w)),
                st,
            );
            cx.put(buf, pill.x, mid, 1, cx.g("│", "|"), st);
            cx.put(buf, pill.right() - 1, mid, 1, cx.g("│", "|"), st);
        }
    }
    if cx.has_surface() {
        cx.fill(buf, Rect::new(pill.x, mid, pill.width, 1), cx.rgb_bg(tint));
    }
    let mut x = pill.x + 1 + u16::from(mode > Mode::Chip);
    x += cx.dchip(buf, x, mid, d, false);
    if mode > Mode::Chip {
        x += 1;
        let room = pill.right().saturating_sub(x + 1);
        x += cx.put(
            buf,
            x,
            mid,
            room,
            &d.name,
            cx.ink(Tok::Fg, 1.0).add_modifier(Modifier::BOLD),
        );
        if mode == Mode::Full {
            cx.put(
                buf,
                x,
                mid,
                pill.right().saturating_sub(x + 1),
                " · this device",
                cx.ink(Tok::Muted, 0.95),
            );
        }
    }
    cx.hit(pill, format!("device:{}", d.id));
}
