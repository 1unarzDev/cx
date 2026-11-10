//! Screens: hub, device, session peek, files, preview, palette, keys, onboarding, join,
//! link policy and help. Each is a plain function over the shared `Cx`.
use super::{centered, fit, fmt, inset, mix, orbit, tiles, Cx, Size, SLOTS};
use crate::ui::lab::{
    caps::Pane, Access, DeviceStatus, EdgeKind, FileKind, LabDevice, LabEdge, LabSession, LabView,
    PreviewKind, Tok,
};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
};

type Keys<'k> = Vec<(&'k str, &'k str, &'k str)>;

pub(super) fn draw(cx: &Cx, view: &LabView, buf: &mut Buffer) {
    let a = cx.area;
    // Phones keep a spacer row above the footer so its chips are two rows tall to a thumb.
    let spare = u16::from(cx.touch());
    let body = Rect::new(a.x, a.y + 1, a.width, a.height.saturating_sub(2 + spare));
    let foot = Rect::new(a.x, a.bottom() - 1, a.width, 1);
    match view {
        LabView::Hub => {
            top(cx, buf, &[], true);
            super::hub::hub(cx, buf, body);
            footer(cx, buf, foot, hub_keys(cx));
        }
        LabView::DeviceFocus(id) => {
            let d = cx.s.device(id).or(cx.s.devices.first());
            top(
                cx,
                buf,
                &d.map(Crumb::Dev).into_iter().collect::<Vec<_>>(),
                true,
            );
            if let Some(d) = d {
                device(cx, buf, body, d);
            }
            let n = d.map_or(0, |d| cx.s.sessions_on(&d.id).count());
            // `⏎` and the slot keys both attach, so both are on the one chip.
            let slots: String = std::iter::once(cx.ret().to_string())
                .chain(SLOTS[..n.clamp(1, 3)].iter().map(|c| c.to_string()))
                .collect::<Vec<_>>()
                .join(" ");
            let mut items: Keys = Vec::new();
            if cx.touch() {
                // The primary verb first, then the way back; tiles also attach on a tap.
                if n > 0 {
                    items.push(("⏎", "attach", "attach"));
                }
                items.extend([("esc", "back", "back"), ("f", "files", "files")]);
                if n > 0 {
                    items.push(("⌫", "stop", "stop"));
                }
                items.extend([("e", "link", "links"), ("`", "hub", "hub")]);
            } else {
                if n > 0 {
                    items.extend([(slots.as_str(), "attach", "attach"), ("␣", "peek", "peek")]);
                }
                items.extend([("f", "files", "files"), ("e", "link", "links")]);
                if n > 0 {
                    items.push(("⌫", "stop", "stop"));
                }
                items.extend([("/", "find", "palette"), ("esc", "back", "back")]);
            }
            footer(cx, buf, foot, items);
        }
        LabView::SessionPeek(id) if cx.c.focus != Some(Pane::Custom("peek")) => {
            if let Some(x) = cx.s.session(id).or(cx.s.sessions.first()) {
                attached(cx, buf, x);
            }
        }
        LabView::SessionPeek(id) => {
            let x = cx.s.session(id).or(cx.s.sessions.first());
            let d = x.and_then(|x| cx.s.device(&x.device));
            let mut crumbs: Vec<Crumb> = d.map(Crumb::Dev).into_iter().collect();
            if let Some(x) = x {
                crumbs.push(Crumb::Text(x.title.clone()));
            }
            top(cx, buf, &crumbs, true);
            if let Some(x) = x {
                peek(cx, buf, body, x);
            }
            footer(
                cx,
                buf,
                foot,
                if cx.touch() {
                    // The filmstrip tiles are tap targets, so `tab next` is not needed.
                    vec![
                        ("⏎", "attach", "attach"),
                        ("esc", "back", "back"),
                        ("⌫", "stop", "stop"),
                        ("`", "hub", "hub"),
                    ]
                } else {
                    vec![
                        ("⏎", "attach", "attach"),
                        ("t a…", "jump", "jump"),
                        ("tab", "next", "peek-next"),
                        ("⌫", "stop", "stop"),
                        ("esc", "back", "back"),
                        ("`", "hub", "hub"),
                    ]
                },
            );
        }
        LabView::Files | LabView::FilePreview => {
            let d = cx.s.device(&cx.s.files.device);
            let mut crumbs: Vec<Crumb> = d.map(Crumb::Dev).into_iter().collect();
            crumbs.push(Crumb::Text(cx.s.files.cwd.clone()));
            if matches!(view, LabView::FilePreview) {
                crumbs.push(Crumb::Text(cx.s.files.preview_path.clone()));
            }
            top(cx, buf, &crumbs, true);
            if matches!(view, LabView::Files) && cx.c.focus == Some(Pane::Dialog) {
                // `s` (send to): the network as the destination picker.
                files(cx, buf, body);
                send_to(cx, buf, body);
                footer(
                    cx,
                    buf,
                    foot,
                    vec![("a-z", "send there", "send"), ("esc", "cancel", "back")],
                );
            } else if matches!(view, LabView::Files) {
                files(cx, buf, body);
                footer(
                    cx,
                    buf,
                    foot,
                    if cx.size() >= Size::M {
                        // The preview is already inline: the footer teaches the motion keys.
                        vec![
                            ("⏎", "open", "open"),
                            ("j k", "move", "move"),
                            ("l h", "fold", "fold"),
                            ("s", "send to", "send"),
                            ("y", "copy", "copy"),
                            ("/", "find", "palette"),
                            ("esc", "back", "back"),
                            ("`", "hub", "hub"),
                        ]
                    } else {
                        // Tapping a row previews it, so phones keep `s send` instead.
                        vec![
                            ("⏎", "open", "open"),
                            ("esc", "back", "back"),
                            ("s", "send", "send"),
                            ("␣", "preview", "preview"),
                            ("/", "find", "palette"),
                            ("`", "hub", "hub"),
                        ]
                    },
                );
            } else {
                file_preview(cx, buf, body, true);
                footer(
                    cx,
                    buf,
                    foot,
                    vec![
                        ("esc", "back", "back"),
                        ("/", "find", "find"),
                        ("s", "send to", "send"),
                        ("y", "copy", "copy"),
                        ("`", "hub", "hub"),
                    ],
                );
            }
        }
        LabView::Palette(q) => {
            top(cx, buf, &[], true);
            super::hub::hub(cx, buf, body);
            footer(cx, buf, foot, hub_keys(cx));
            cx.dim(buf, a);
            palette(cx, buf, a, q);
        }
        LabView::Secrets => {
            top(cx, buf, &[Crumb::Text("keys".into())], true);
            secrets(cx, buf, body);
            footer(
                cx,
                buf,
                foot,
                vec![
                    ("a s d g", "open", "edit"),
                    ("y", "copy", "copy"),
                    ("r", "reveal 10s", "reveal"),
                    ("esc", "back", "back"),
                ],
            );
        }
        LabView::Onboarding => {
            top(cx, buf, &[Crumb::Text("welcome".into())], false);
            onboarding(cx, buf, body);
            footer(
                cx,
                buf,
                foot,
                vec![
                    ("⏎", "connect", "connect"),
                    ("j", "join code", "join"),
                    ("esc", "skip", "skip"),
                ],
            );
        }
        LabView::JoinCode => {
            top(cx, buf, &[Crumb::Text("invite".into())], true);
            join(cx, buf, body);
            footer(
                cx,
                buf,
                foot,
                vec![
                    ("y", "code", "copy-code"),
                    ("Y", "command", "copy-cmd"),
                    ("esc", "done", "back"),
                    ("r", "new code", "renew"),
                ],
            );
        }
        LabView::EdgeEditor(id) => {
            let e = cx.s.edge(id).or(cx.s.edges.first());
            let ends = e.and_then(|e| Some((cx.s.device(&e.from)?, cx.s.device(&e.to)?)));
            let crumbs: Vec<Crumb> = ends.map(|(a, b)| Crumb::Link(a, b)).into_iter().collect();
            top(cx, buf, &crumbs, true);
            edge_editor(cx, buf, body, id);
            let mut items: Keys = vec![
                ("k p o", "out", "policy-out"),
                ("K P O", "in", "policy-in"),
                ("u", "undo", "undo"),
                ("esc", "done", "back"),
            ];

            if cx.size() >= Size::M {
                items[0].1 = "key pass off, out";
                items[1].1 = "the same, in";
            }
            footer(cx, buf, foot, items);
        }
        LabView::Help => {
            top(cx, buf, &[Crumb::Text("help".into())], true);
            help(cx, buf, body);
            footer(cx, buf, foot, vec![("esc", "close", "back")]);
        }
    }
}

enum Crumb<'a> {
    Dev(&'a LabDevice),
    /// A link: both ends' keycaps around `⇄`.
    Link(&'a LabDevice, &'a LabDevice),
    Text(String),
}

/// Top bar: wordmark, breadcrumb (every segment is a tap target), then global evidence.
fn top(cx: &Cx, buf: &mut Buffer, crumbs: &[Crumb], status: bool) {
    let a = cx.area;
    let y = a.y;
    let mut x = a.x + 1;
    x += cx.put(
        buf,
        x,
        y,
        2,
        "cx",
        cx.st(Tok::Accent).add_modifier(Modifier::BOLD),
    );
    cx.hit(Rect::new(a.x, y, 3, 1), "hub");
    let right_w = if status { right_status(cx, buf, y) } else { 0 };
    let limit = a.right().saturating_sub(right_w + 2);
    for (i, c) in crumbs.iter().enumerate() {
        if x + 4 >= limit {
            break;
        }
        x += cx.put(buf, x, y, 3, cx.g(" › ", " / "), cx.ink(Tok::Faint, 1.0));
        let x0 = x;
        match c {
            // On a phone a device crumb before the last one is its keycap alone.
            Crumb::Dev(d) if cx.size() == Size::Xs && i + 1 < crumbs.len() => {
                x += cx.dchip(buf, x, y, d, false);
                cx.hit(Rect::new(x0, y, x - x0, 1), format!("device:{}", d.id));
            }
            Crumb::Dev(d) => {
                x += cx.dchip(buf, x, y, d, false);
                x += 1;
                let room = limit.saturating_sub(x);
                x += cx.put(
                    buf,
                    x,
                    y,
                    room,
                    &fit(&d.name, room as usize, cx.uni()),
                    cx.ink(Tok::Fg, 1.0).add_modifier(Modifier::BOLD),
                );
                cx.hit(Rect::new(x0, y, x - x0, 1), format!("device:{}", d.id));
            }
            Crumb::Link(p, q) => {
                x += cx.dchip(buf, x, y, p, false);
                x += cx.put(buf, x, y, 3, cx.g(" ⇄ ", " - "), cx.ink(Tok::Fg, 0.8));
                x += cx.dchip(buf, x, y, q, false);
                if cx.size() >= Size::S {
                    x += cx.put(buf, x, y, 6, " link", cx.ink(Tok::Muted, 0.9));
                }
            }
            Crumb::Text(t) => {
                let room = limit.saturating_sub(x);
                x += cx.put(
                    buf,
                    x,
                    y,
                    room,
                    &fit(t, room as usize, cx.uni()),
                    cx.ink(Tok::Fg, 0.9),
                );
                if i + 1 < crumbs.len() {
                    cx.hit(Rect::new(x0, y, x - x0, 1), "back");
                }
            }
        }
    }
    if crumbs.is_empty() && cx.size() >= Size::S {
        cx.put(
            buf,
            x + 1,
            y,
            limit.saturating_sub(x + 1),
            cx.g("› network", "/ network"),
            cx.ink(Tok::Muted, 0.8),
        );
    }
}

/// Right side of the top bar; returns its width.
fn right_status(cx: &Cx, buf: &mut Buffer, y: u16) -> u16 {
    let s = cx.s;
    let a = cx.area;
    let up = s.devices.iter().filter(|d| cx.up(d)).count();
    let down = s.devices.len() - up;
    let mut parts: Vec<(String, Style, &str)> = Vec::new();
    // Parts are placed right to left; an alert outranks the transfer readout.
    if let Some(n) = s.notifications.iter().max_by_key(|n| n.kind as u8) {
        let tok = match n.kind {
            crate::ui::lab::NoticeKind::Err => Tok::Err,
            crate::ui::lab::NoticeKind::Warn => Tok::Warn,
            crate::ui::lab::NoticeKind::Ok => Tok::Ok,
            crate::ui::lab::NoticeKind::Info => Tok::Info,
        };
        // The ticker above the footer says what and how to fix it; here only the count.
        let text = format!("{} {}", cx.g("▲", "^"), s.notifications.len());
        parts.push((text, cx.st(tok), "notices"));
    }
    if let Some(t) = s.transfers.first().filter(|_| cx.size() >= Size::S) {
        let pct = t.done as f32 / t.total.max(1) as f32;
        let rate = t.rate_hist.last().copied().unwrap_or(0.0);
        let text = if cx.size() >= Size::M {
            format!(
                "{} {} {:.0}% {}/s",
                cx.g("⇣", "v"),
                t.path.rsplit('/').next().unwrap_or(""),
                pct * 100.0,
                fmt::bytes(rate as u64)
            )
        } else {
            format!("{} {:.0}%", cx.g("⇣", "v"), pct * 100.0)
        };
        parts.push((text, cx.st(Tok::Info), "transfers"));
    }
    let dot = if down == 0 {
        cx.st(Tok::Ok)
    } else {
        cx.st(Tok::Muted)
    };
    parts.push((
        format!("{} {up}/{}", cx.g("●", "o"), s.devices.len()),
        dot,
        "devices",
    ));
    let mut x = a.right().saturating_sub(1);
    let mut used = 0u16;
    if cx.size() >= Size::S {
        let w = cx.put_r(buf, x, y, " ? ", cx.key_style());
        cx.hit(Rect::new(x - w, y, w, 1), "help");
        x = x.saturating_sub(w + 2);
        used += w + 2;
    }
    for (t, st, act) in parts {
        let w = super::width(&t, cx.uni());
        if used + w + 3 > a.width / 2 {
            break;
        }
        cx.put_r(buf, x, y, &t, st);
        cx.hit(Rect::new(x - w, y, w, 1), act);
        x = x.saturating_sub(w + 2);
        used += w + 2;
    }
    used + 1
}

/// Footer chips, most important first. Phones keep the row above the footer free, so a
/// footer that does not fit wraps its tail into it instead of dropping verbs; a single
/// row keeps two-row thumb targets.
fn footer(cx: &Cx, buf: &mut Buffer, r: Rect, items: Keys) {
    let items: Keys = items
        .into_iter()
        .map(|(k, l, a)| {
            // Words where the symbol may be missing (ASCII, and phones, whose default
            // fonts lack ⏎ ⌫); `␣` is always `spc` (it renders as a bare underscore).
            let k = match k {
                "␣" => "spc",
                "⏎" if !cx.uni() || cx.touch() => "ret",
                "⌫" if cx.touch() => "del",
                "⌫" if !cx.uni() => "bksp",
                "↑↓" if !cx.uni() => "up/dn",
                other => other,
            };
            let l = if cx.size() == Size::Xs && l.len() > 7 {
                l.split(' ').next().unwrap_or(l)
            } else {
                l
            };
            (k, l, a)
        })
        .collect();
    let (x0, right) = (r.x + 1, r.right().saturating_sub(1));
    let sep = if cx.area.width < 50 { 1 } else { 2 };
    let mut fit_n = 0;
    let mut x = x0;
    for (k, l, _) in &items {
        let lw = super::width(l, cx.uni());
        let need = super::width(k, cx.uni()) + 2 + if lw > 0 { lw + 1 } else { 0 };
        if x + need > right {
            break;
        }
        x += need + sep;
        fit_n += 1;
    }
    if cx.touch() && fit_n < items.len() && r.y > cx.area.y + 1 {
        cx.chips_h(buf, x0, r.y, right, &items[..fit_n], 1);
        cx.chips_h(buf, x0, r.y - 1, right, &items[fit_n..], 1);
    } else {
        let th = if cx.touch() { 2 } else { 1 };
        cx.chips_h(buf, x0, r.y, right, &items, th);
    }
}

/// Hub footer. Device letters are printed on every node, so they get no chip; `⏎` says
/// what it does to the focused zone (a device opens, a session attaches).
fn hub_keys(cx: &Cx) -> Keys<'static> {
    let on_sess = cx.c.focus == Some(Pane::Sessions);
    // The footer follows the focused zone: a device opens and has links; a session
    // attaches, peeks and stops (`-` re-attaches the last detached one from anywhere).
    let mut v: Keys = if on_sess {
        vec![
            ("⏎", "attach", "attach"),
            ("␣", "peek", "peek"),
            ("⌫", "stop", "stop"),
            ("-", "last", "last"),
            ("tab", "network", "zone"),
        ]
    } else {
        vec![
            ("⏎", "open", "open"),
            ("e", "link", "links"),
            ("-", "last", "last"),
            ("tab", "sessions", "zone"),
        ]
    };
    if cx.touch() {
        // Phones tap nodes and rows directly; zone keys and `-` stay keyboard-only.
        v.retain(|(k, _, _)| !matches!(*k, "tab" | "-" | "e"));
    }
    v.extend([
        ("/", "find", "palette"),
        ("+", "invite", "invite"),
        ("K", "keys", "secrets"),
        ("?", "help", "help"),
    ]);
    v
}

// ------------------------------------------------------------------------------ hub

/// A zone header: the spaced label, then a faint rule up to `right`. Returns the x after
/// the label so callers can put a legend on the rule.
pub(super) fn zone(
    cx: &Cx,
    buf: &mut Buffer,
    x: u16,
    y: u16,
    right: u16,
    text: &str,
    focused: bool,
) -> u16 {
    let w = cx.label(buf, x, y, right.saturating_sub(x), text, focused);
    cx.rule(buf, x + w + 1, right, y);
    x + w
}

/// Session grid shape for a width: (columns, tile height).
pub(super) fn grid_shape(cx: &Cx, w: u16, h: u16) -> (u16, u16) {
    let (min_w, max_cols) = match cx.size() {
        Size::Xs => (w, 1),
        Size::S => (24, 3),
        Size::M => (34, 3),
        Size::L => (40, 4),
    };
    let cols = ((w + 2) / (min_w + 2)).clamp(1, max_cols);
    let th = if cx.size() == Size::L && h >= 50 {
        8
    } else if h >= 36 {
        7
    } else if h >= 28 {
        6
    } else {
        5
    };
    (cols, th)
}

/// Session thumbnails on one uniform grid (same size, one gutter), ranked by attention;
/// the next free cell becomes the "new session" ghost tile. Returns how many were shown.
#[allow(clippy::too_many_arguments)]
pub(super) fn thumbs_grid(
    cx: &Cx,
    buf: &mut Buffer,
    r: Rect,
    list: &[&LabSession],
    sel: (usize, bool),
    cols: u16,
    th: u16,
    ghost: Option<&LabDevice>,
) -> usize {
    if r.height < 3 || r.width < 12 {
        return 0;
    }
    let gap = 2u16;
    let rows = (r.height + 1) / (th + 1);
    let tw = (r.width + gap) / cols - gap;
    let cells = usize::from(rows * cols);
    let shown = list.len().min(cells);
    let n = cells.min(list.len() + 1);
    for i in 0..n {
        let (ci, ri) = (i as u16 % cols, i as u16 / cols);
        let tr = Rect::new(r.x + ci * (tw + gap), r.y + ri * (th + 1), tw, th);
        match list.get(i) {
            Some(x) => {
                let pick = (sel.0 % list.len() == i).then_some(sel.1);
                tiles::thumb(cx, buf, tr, x, pick);
            }
            None => new_tile(cx, buf, tr, ghost, false, true),
        }
        if !cx.has_surface() && ci + 1 < cols && i + 1 < n {
            cx.vsep(buf, tr.right(), tr.y, th);
        }
    }
    shown
}

// --------------------------------------------------------------------------- device

fn device(cx: &Cx, buf: &mut Buffer, r: Rect, d: &LabDevice) {
    let s = cx.s;
    let sz = cx.size();
    // The header's own rows (identity, then the cpu chart and gauges beside it).
    let head_h: u16 = match sz {
        Size::Xs => 3,
        _ => 4,
    };
    let head = Rect::new(r.x + 1, r.y + 1, r.width.saturating_sub(2), head_h);
    device_header(cx, buf, head, d);
    let right = r.right().saturating_sub(1);
    let main = Rect::new(
        r.x + 1,
        head.bottom() + 2,
        r.width.saturating_sub(2),
        r.bottom().saturating_sub(head.bottom() + 2),
    );
    let list: Vec<&LabSession> = s.sessions_on(&d.id).collect();
    let links_n = s.edges_of(&d.id).count() as u16;
    // The device's own neighbourhood (same schematic, one hop) under its sessions.
    // The fan is a star (links on both sides), so it needs half the rows of its links.
    let fan_want = if links_n == 0 {
        0
    } else {
        (links_n.div_ceil(2) * 4).max(5)
    };
    let sel = cx.c.selected % (list.len() + 1);
    let used = if sz == Size::Xs {
        device_sessions_rows(cx, buf, main, &list, d)
    } else {
        let cols = if sz == Size::S { 2 } else { 3 }.min(list.len() as u16 + 1);
        let gap = 2u16;
        let tw = (main.width + gap) / cols - gap;
        // Tiles are as tall as the tallest screen needs (ghost tile included).
        let need = list
            .iter()
            .map(|x| tiles::tail_rows(x) as u16)
            .max()
            .unwrap_or(5)
            + 2;
        let rows = (list.len() as u16 + 1).div_ceil(cols);
        let room = main
            .height
            .saturating_sub(if fan_want > 0 { fan_want + 3 } else { 0 });
        let th = ((room + 1) / rows).saturating_sub(1).clamp(5, need.max(7));
        // When everything fits one row, the ghost tile stays narrow and the sessions
        // share the rest (a single session gets its screen nearly 1:1).
        let one_row = rows == 1;
        let ghost_w = tw.min(32);
        let sw = if one_row && !list.is_empty() {
            (main.width + gap - ghost_w - gap) / list.len() as u16 - gap
        } else {
            tw
        };
        for i in 0..=list.len() {
            let (ci, ri) = (i as u16 % cols, i as u16 / cols);
            let x0 = main.x + ci * (sw + gap);
            let w = if i == list.len() && one_row {
                ghost_w
            } else {
                sw
            };
            let tr = Rect::new(x0, main.y + ri * (th + 1), w, th);
            if tr.bottom() > main.bottom() {
                break;
            }
            match list.get(i) {
                Some(x) => tiles::thumb(cx, buf, tr, x, (i == sel).then_some(true)),
                None => new_tile(cx, buf, tr, Some(d), list.is_empty(), false),
            }
            if !cx.has_surface() && ci + 1 < cols && i < list.len() {
                cx.vsep(buf, tr.right(), tr.y, th);
            }
        }
        rows * (th + 1)
    };
    zone(
        cx,
        buf,
        main.x,
        main.y.saturating_sub(1),
        right,
        "SESSIONS",
        true,
    );
    let fy = main.y + used + 1;
    if links_n > 0 && fy + 6 <= r.bottom() {
        // The footer says `e links`; the zone label is the tap target.
        zone(cx, buf, main.x, fy, right, "LINKS", false);
        cx.hit(Rect::new(main.x, fy, 12, 1), "links");
        // The fan, then (when rows are left) every link's rtt history, one row each.
        let left = r.bottom().saturating_sub(fy + 2);
        let edges: Vec<&LabEdge> = s.edges_of(&d.id).collect();
        let table = left >= fan_want + 2 + edges.len() as u16 && sz >= Size::M;
        let fan_h = if table { fan_want + 1 } else { left };
        let band = Rect::new(r.x, fy + 1, r.width, fan_h);
        orbit::draw_with(cx, buf, band, &d.id, 1, orbit::Sel::Root, false);
        if table {
            link_rows(
                cx,
                buf,
                Rect::new(main.x, band.bottom() + 1, main.width, left - fan_h - 1),
                d,
                &edges,
            );
        }
    }
}

/// One block per link of `d`, centred: the far end, its network, the policy as seen from
/// `d`, the rtt history as an area chart as tall as the rows allow (up to 2, a blank row
/// between blocks), and the latest rtt and loss on its baseline.
fn link_rows(cx: &Cx, buf: &mut Buffer, r: Rect, d: &LabDevice, edges: &[&LabEdge]) {
    // Left-aligned with the zone label and the tiles above it, full width.
    let w = r.width;
    let x0 = r.x;
    let n = edges.len().max(1) as u16;
    let ch = ((r.height + 1) / n).saturating_sub(1).clamp(1, 6);
    let step = if r.height >= n * (ch + 1) { ch + 1 } else { ch };
    for (i, e) in edges.iter().enumerate() {
        let top = r.y + i as u16 * step;
        if top + ch > r.bottom() {
            break;
        }
        let y = top + ch - 1;
        let other = if e.from == d.id { &e.to } else { &e.from };
        let Some(o) = cx.s.device(other) else {
            continue;
        };
        // The far end's letter as plain text: in a device, letters are slots and verbs.
        let mut x = x0;
        let ist = cx
            .ink(Tok::Id(cx.s.hue(&o.id)), 1.0)
            .add_modifier(Modifier::BOLD);
        cx.put(buf, x + 1, y, 1, &o.hotkey.to_string(), ist);
        x += 4;
        cx.put(
            buf,
            x,
            y,
            18,
            &fit(&o.name, 18, cx.uni()),
            cx.ink(Tok::Fg, 0.9),
        );
        x += 19;
        let (kind, tok) = edge_kind(cx, e);
        cx.put(buf, x, y, 20, &fit(&kind, 20, cx.uni()), cx.ink(tok, 0.9));
        x += 21;
        let pol = orbit::policy(cx, e, e.from == d.id, false);
        cx.put(buf, x, y, 11, &pol, cx.ink(Tok::Muted, 0.95));
        x += 12;
        let tail = match e.rtt_ms.filter(|_| e.up) {
            Some(ms) => format!("{}  loss {:.0}%", super::ms_text(ms), e.loss * 100.0),
            None => "no reply".to_string(),
        };
        let sw = (x0 + w).saturating_sub(x + 20);
        let tok = if e.up { tok } else { Tok::Err };
        tiles::rtt_chart(cx, buf, Rect::new(x, top, sw, ch), &e.rtt_hist, tok);
        cx.put_r(buf, x0 + w, y, &tail, cx.st(Tok::Muted));
        cx.hit(Rect::new(x0, top, w, ch), format!("edge:{}", e.id));
    }
}

fn device_header(cx: &Cx, buf: &mut Buffer, r: Rect, d: &LabDevice) {
    let (g, gst) = cx.status(d.status);
    let info = format!("{} · {}@{} · {}", d.os, d.account, d.host, kind_word(d));
    // The identity block takes the width it needs; metrics get the rest.
    let left_w = if cx.size() >= Size::M {
        (super::width(&info, cx.uni()) + 1).clamp(32, r.width * 11 / 20)
    } else {
        r.width
    };
    let mut x = r.x;
    x += cx.dchip(buf, x, r.y, d, false);
    x += 1;
    x += cx.put(
        buf,
        x,
        r.y,
        left_w.saturating_sub(x - r.x),
        &d.name,
        cx.ink(Tok::Fg, 1.0).add_modifier(Modifier::BOLD),
    );
    x += 2;
    x += cx.put(buf, x, r.y, 1, g, gst);
    x += 1;
    let ev = if d.id == cx.s.viewer {
        "this device".to_string()
    } else {
        d.evidence.clone()
    };
    let room = (r.x + left_w).saturating_sub(x);
    cx.put(
        buf,
        x,
        r.y,
        room,
        &fit(&ev, room as usize, cx.uni()),
        cx.st(Tok::Muted),
    );
    let mut y = r.y + 1;
    cx.put(
        buf,
        r.x,
        y,
        left_w,
        &fit(&info, left_w as usize, cx.uni()),
        cx.ink(Tok::Fg, 0.75),
    );
    y += 1;
    if y < r.bottom() {
        let mut mx = r.x;
        for (i, m) in
            cx.s.meshes
                .iter()
                .enumerate()
                .filter(|(_, m)| d.meshes.contains(&m.id))
        {
            let tok = if i == 0 { Tok::Mesh } else { Tok::Id(6) };
            mx += cx.put(
                buf,
                mx,
                y,
                left_w.saturating_sub(mx - r.x),
                &format!("{} {}  ", tiles::mesh_glyph(cx, &m.id), m.name),
                cx.st(tok),
            );
        }
        if let Some(j) = &d.joined_via {
            cx.put(
                buf,
                mx,
                y,
                left_w.saturating_sub(mx - r.x),
                &format!("joined via {j}"),
                cx.st(Tok::Muted),
            );
        } else if d.meshes.is_empty() {
            cx.put(
                buf,
                mx,
                y,
                left_w,
                "no mesh · direct ssh",
                cx.st(Tok::Muted),
            );
        }
    }
    // Metrics block.
    let Some(m) = d.metrics.as_ref().filter(|_| cx.up(d)) else {
        if r.height > 3 {
            cx.put(
                buf,
                r.x,
                r.y + 3,
                r.width,
                &format!("{} {}", cx.g("✕", "x"), d.evidence),
                cx.st(Tok::Err),
            );
        }
        return;
    };
    let (mx, my, mw, mh) = if cx.size() >= Size::M {
        let mx = r.x + left_w + 3;
        (mx, r.y, r.right().saturating_sub(mx), r.height)
    } else {
        (r.x, r.y + 3, r.width, r.height.saturating_sub(3))
    };
    if mh == 0 || mw < 20 {
        return;
    }
    if cx.size() >= Size::M {
        // cpu chart on the left of the block, gauges to its right.
        let gw = 24.min(mw / 2);
        let cw = mw.saturating_sub(gw + 3);
        let ch = mh.min(4);
        cx.put(
            buf,
            mx,
            my,
            12,
            &format!("cpu {:.0}%", m.cpu * 100.0),
            cx.heat(m.cpu).add_modifier(Modifier::BOLD),
        );
        let load = format!("load {:.1} · {} cores", m.load[0], m.cores);
        if super::width(&load, cx.uni()) + 10 < cw {
            cx.put_r(buf, mx + cw, my, &load, cx.st(Tok::Muted));
        }
        tiles::chart(
            cx,
            buf,
            Rect::new(mx, my + 1, cw, ch.saturating_sub(1).max(1)),
            &m.cpu_hist,
            None,
        );
        let gx = mx + cw + 3;
        let mut rows: Vec<(String, f32, Style)> = vec![(
            format!("mem {:.0}/{:.0}G", m.mem * m.mem_total_gb, m.mem_total_gb),
            m.mem,
            cx.st(Tok::Info),
        )];
        if let Some(t) = m.temp_c {
            let hv = ((t - 35.0) / 50.0).clamp(0.0, 1.0);
            rows.push((format!("temp {t:.0}{}", cx.g("°", "C")), hv, cx.heat(hv)));
        }
        if let Some(gp) = m.gpu {
            rows.push((format!("gpu {:.0}%", gp * 100.0), gp, cx.heat(gp)));
        }
        if let Some(b) = m.battery {
            rows.push((format!("batt {:.0}%", b * 100.0), b, cx.st(Tok::Ok)));
        }
        rows.push((
            format!("disk {:.0}%", m.disk * 100.0),
            m.disk,
            cx.heat(m.disk),
        ));
        for (i, (label, v, st)) in rows.iter().take(mh as usize).enumerate() {
            let y = my + i as u16;
            cx.put(buf, gx, y, 12, label, cx.ink(Tok::Fg, 0.8));
            tiles::gauge(cx, buf, gx + 13, y, gw.saturating_sub(13), *v, *st);
        }
    } else {
        let label = format!("cpu {:.0}%", m.cpu * 100.0);
        cx.put(buf, mx, my, 9, &label, cx.heat(m.cpu));
        let w = mw.saturating_sub(10 + 9);
        tiles::chart(cx, buf, Rect::new(mx + 9, my, w, 1), &m.cpu_hist, None);
        let rest = format!("mem {:.0}%", m.mem * 100.0);
        cx.put_r(buf, mx + mw, my, &rest, cx.st(Tok::Info));
    }
}

fn kind_word(d: &LabDevice) -> &'static str {
    use crate::ui::lab::DeviceKind::*;
    match d.kind {
        Desktop => "desktop",
        Laptop => "laptop",
        Robot => "robot",
        Server => "server",
        Phone => "phone",
    }
}

/// The dashed "new session" ghost tile with one key per provider. On the hub (`hub`) it
/// fills its grid cell (rows stay even) and its keys carry the device letter first
/// (`i c`), because hub letters are devices; in the device view it is content-sized.
/// Short tiles drop the title before any option, so every working key stays visible.
fn new_tile(cx: &Cx, buf: &mut Buffer, r: Rect, d: Option<&LabDevice>, picked: bool, hub: bool) {
    if r.width < 12 || r.height < 3 {
        return;
    }
    let opts = [
        ("c", "claude", Tok::Claude),
        ("x", "codex", Tok::Codex),
        ("n", "shell", Tok::Shell),
    ];
    let inner = r.height.saturating_sub(2);
    let stacked = inner >= 3 && d.is_some();
    let titled = !stacked || inner >= 5;
    let block_h = match (stacked, titled) {
        (true, true) => 2 + opts.len() as u16,
        (true, false) => opts.len() as u16,
        _ => 2,
    };
    // Content height, top-aligned with its neighbours' headers (never a tall empty box).
    let r = Rect::new(r.x, r.y, r.width, r.height.min(block_h + 4));
    cx.fill(
        buf,
        r,
        if picked {
            cx.st(Tok::AccentBg)
        } else {
            Style::default()
        },
    );
    let faint = if picked {
        cx.st(Tok::Accent)
    } else {
        cx.ink(Tok::Faint, 1.0)
    };
    let (h, v) = (cx.g("╌", "-"), cx.g("╎", ":"));
    let w = r.width.saturating_sub(2) as usize;
    let (tl, tr, bl, br) = if cx.uni() {
        ("╭", "╮", "╰", "╯")
    } else {
        ("+", "+", "+", "+")
    };
    cx.put(
        buf,
        r.x,
        r.y,
        r.width,
        &format!("{tl}{}{tr}", h.repeat(w)),
        faint,
    );
    cx.put(
        buf,
        r.x,
        r.bottom() - 1,
        r.width,
        &format!("{bl}{}{br}", h.repeat(w)),
        faint,
    );
    for y in r.y + 1..r.bottom().saturating_sub(1) {
        cx.put(buf, r.x, y, 1, v, faint);
        cx.put(buf, r.right() - 1, y, 1, v, faint);
    }
    let inner = r.height.saturating_sub(2);
    let mut y = r.y + 1 + inner.saturating_sub(block_h) / 2;
    let Some(d) = d else {
        let tw = r.right().saturating_sub(r.x + 3);
        cx.put(
            buf,
            r.x + 2,
            y,
            tw,
            &fit("new session", tw as usize, cx.uni()),
            cx.ink(Tok::Fg, 0.9).add_modifier(Modifier::BOLD),
        );
        cx.hit(r, "palette:new");
        return;
    };
    let x = r.x + 2;
    let tw = r.right().saturating_sub(x + 1);
    let title_st = cx.ink(Tok::Fg, 0.9).add_modifier(Modifier::BOLD);
    if titled {
        let mut tx = x;
        tx += cx.put(buf, tx, y, tw, "new on ", title_st);
        tx += cx.dchip(buf, tx, y, d, false) + 1;
        let room = r.right().saturating_sub(tx + 1);
        cx.put(
            buf,
            tx,
            y,
            room,
            &fit(&d.name, room as usize, cx.uni()),
            title_st,
        );
        y += if stacked { 2 } else { 1 };
    }
    let mut ox = x;
    for (k, label, tok) in opts {
        let key = if hub {
            format!(" {} {k} ", d.hotkey)
        } else {
            format!(" {k} ")
        };
        let kw = super::width(&key, cx.uni());
        if y >= r.bottom() - 1 || (!stacked && ox + kw + 8 > r.right()) {
            break;
        }
        let x0 = ox;
        ox += cx.put(buf, ox, y, kw, &key, cx.key_style());
        ox += 1;
        ox += cx.put(buf, ox, y, 8, label, cx.st(tok));
        cx.hit(
            Rect::new(x0, y, ox - x0, 1),
            format!("new:{}:{label}", d.id),
        );
        if stacked {
            y += 1;
            ox = x;
        } else {
            ox += 2;
        }
    }
}

/// Phone device view: two rows per session, then the provider buttons; returns rows used.
fn device_sessions_rows(
    cx: &Cx,
    buf: &mut Buffer,
    r: Rect,
    list: &[&LabSession],
    d: &LabDevice,
) -> u16 {
    let mut y = r.y;
    let sel = cx.c.selected % (list.len() + 1);
    for (i, x) in list.iter().enumerate() {
        if y + 1 >= r.bottom() {
            break;
        }
        tiles::session_row(
            cx,
            buf,
            Rect::new(r.x, y, r.width, 2),
            x,
            (i == sel).then_some(true),
        );
        y += 3;
    }
    if y < r.bottom() {
        let items = [
            ("c", "claude", "new-claude"),
            ("x", "codex", "new-codex"),
            ("n", "shell", "new-shell"),
        ];
        let x0 = r.x + 1 + cx.put(buf, r.x + 1, y, 4, "new ", cx.ink(Tok::Muted, 0.9));
        cx.chips(buf, x0, y, r.right(), &items);
        let _ = d;
        y += 1;
    }
    y - r.y
}

fn edge_kind(cx: &Cx, e: &LabEdge) -> (String, Tok) {
    match &e.kind {
        EdgeKind::Mesh(m) => {
            let i = cx.s.meshes.iter().position(|x| &x.id == m).unwrap_or(0);
            (
                format!("{} {m}", tiles::mesh_glyph(cx, m)),
                if i == 0 { Tok::Mesh } else { Tok::Id(6) },
            )
        }
        EdgeKind::Lan => ("lan".into(), Tok::Muted),
        EdgeKind::Direct => ("direct ssh".into(), Tok::Muted),
        EdgeKind::Jump(v) => (format!("{} via {v}", cx.g("↳", "->")), Tok::Muted),
    }
}

// ----------------------------------------------------------------------------- peek

fn peek(cx: &Cx, buf: &mut Buffer, r: Rect, x: &LabSession) {
    let sz = cx.size();
    let main = Rect::new(r.x + 1, r.y, r.width.saturating_sub(2), r.height);
    // Meta line: address, state, provider, folder, uptime, clients, activity trend.
    let (g, word, sst) = cx.sstate(x);
    let (p, ptok) = cx.prov(x);
    let mut mx = main.x;
    mx += tiles::addr(cx, buf, mx, main.y, x) + 1;
    mx += cx.put(buf, mx, main.y, 20, &format!("{g} {word}"), sst);
    mx += 2;
    mx += cx.put(buf, mx, main.y, 8, p, cx.st(ptok));
    mx += 2;
    let meta = format!(
        "{} · up {} · {} client{}",
        x.cwd,
        fmt::age(x.age_secs),
        x.clients,
        if x.clients == 1 { "" } else { "s" }
    );
    // Phones keep the identity (address, state, provider); the detail would truncate.
    let meta = if sz == Size::Xs { String::new() } else { meta };
    let spark_w = if sz >= Size::S { 12 } else { 0 };
    let room = main.right().saturating_sub(mx + spark_w + 2);
    let meta = fit(&meta, room as usize, cx.uni());
    mx += cx.put(buf, mx, main.y, room, &meta, cx.st(Tok::Muted));
    if spark_w > 0 {
        // The activity trend rides on the meta line, right after it.
        let v: Vec<Option<f32>> = x.activity.iter().map(|v| Some(*v)).collect();
        tiles::spark(
            cx,
            buf,
            mx + 2,
            main.y,
            spark_w,
            &v,
            cx.ink(Tok::Accent, 0.8),
        );
    }
    // The screen, 1:1, on a deeper surface as tall as its content. At M+ the other
    // sessions fill the rows below it as a filmstrip of live tiles (tap or `t a` to jump,
    // `tab` to step), so the screen never sits over a void.
    let lines = crate::ui::lab::ansi_lines(&x.screen, x.screen_cols);
    let used = lines
        .iter()
        .rposition(|l| l.spans.iter().any(|s| !s.content.trim().is_empty()))
        .map_or(1, |i| i + 1) as u16;
    let avail = main.height.saturating_sub(2);
    let strip_min = 9;
    let scr_h = if sz >= Size::M && avail >= used + 2 + strip_min {
        used + 2
    } else {
        avail
    };
    let scr = Rect::new(main.x, main.y + 2, main.width, scr_h);
    if cx.has_surface() {
        let base = if cx.dark() {
            (0x0c, 0x0e, 0x11)
        } else {
            (0xff, 0xff, 0xff)
        };
        cx.fill(buf, scr, cx.rgb_bg(base));
    }
    let inner = inset(scr, 2, 1);
    // Bottom-anchored like a live terminal: a short pane shows the newest lines.
    let skip = usize::from(used.saturating_sub(inner.height));
    for (i, l) in lines
        .iter()
        .skip(skip)
        .take(inner.height as usize)
        .enumerate()
    {
        let mut xx = inner.x;
        for sp in &l.spans {
            if xx >= inner.right() {
                break;
            }
            let st = tiles::degrade(cx, sp.style);
            xx += cx.put(
                buf,
                xx,
                inner.y + i as u16,
                inner.right() - xx,
                &sp.content,
                st,
            );
        }
    }
    cx.hit(scr, "attach");
    let strip = Rect::new(
        main.x,
        scr.bottom() + 1,
        main.width,
        main.bottom().saturating_sub(scr.bottom() + 2),
    );
    if strip.height < strip_min {
        return;
    }
    let others: Vec<_> = cx
        .by_attention()
        .into_iter()
        .filter(|o| o.id != x.id)
        .collect();
    let hint = format!(
        "{} other{}  tab next",
        others.len(),
        if others.len() == 1 { "" } else { "s" }
    );
    let hw = super::width(&hint, cx.uni());
    zone(
        cx,
        buf,
        strip.x,
        strip.y,
        strip.right().saturating_sub(hw + 1),
        "OTHERS",
        false,
    );
    cx.put_r(buf, strip.right(), strip.y, &hint, cx.st(Tok::Muted));
    let tiles_r = Rect::new(strip.x, strip.y + 1, strip.width, strip.height - 1);
    let gap = 2;
    let n = (others.len() as u16)
        .min((tiles_r.width + gap) / (28 + gap))
        .max(1);
    let tw = (tiles_r.width + gap) / n - gap;
    for (i, o) in others.iter().take(usize::from(n)).enumerate() {
        let tr = Rect::new(
            tiles_r.x + i as u16 * (tw + gap),
            tiles_r.y,
            tw,
            tiles_r.height,
        );
        tiles::thumb(cx, buf, tr, o, None);
        if !cx.has_surface() && i + 1 < usize::from(n) {
            cx.vsep(buf, tr.right(), tr.y, tr.height);
        }
    }
}

/// Attached (capture: `CX_LAB_FOCUS=attached`): the pane owns the whole terminal; cx
/// adds only tmux's status row, whose left chip names the session and whose `⏏ detach`
/// chip is the touch path for `Ctrl+\` (a double `Ctrl+\` sends one through to the pane,
/// for nvim's terminal escape and emacs). Detaching lands on the hub with this tile
/// selected, so `⏎` or `-` re-attaches.
/// An attached session owns the whole terminal under a one-row breadcrumb whose right end
/// is the tappable detach chip (`Ctrl+\\` does the same from the keyboard).
fn attached(cx: &Cx, buf: &mut Buffer, x: &LabSession) {
    let a = cx.area;
    let lines = crate::ui::lab::ansi_lines(&x.screen, x.screen_cols);
    let h = a.height.saturating_sub(1);
    let used = lines
        .iter()
        .rposition(|l| l.spans.iter().any(|s| !s.content.trim().is_empty()))
        .map_or(1, |i| i + 1) as u16;
    let skip = usize::from(used.saturating_sub(h));
    for (i, l) in lines.iter().skip(skip).take(usize::from(h)).enumerate() {
        let mut xx = a.x;
        for sp in &l.spans {
            if xx >= a.right() {
                break;
            }
            xx += cx.put(
                buf,
                xx,
                a.y + 1 + i as u16,
                a.right() - xx,
                &sp.content,
                tiles::degrade(cx, sp.style),
            );
        }
    }
    let y = a.y;
    let bar = if cx.has_surface() {
        cx.st(Tok::Surface2)
    } else {
        Style::default()
    };
    cx.fill(buf, Rect::new(a.x, y, a.width, 1), bar);
    let chip = if cx.uni() {
        " ⏏ ctrl+\\ detach "
    } else {
        " ctrl+\\ detach "
    };
    let cw = super::width(chip, cx.uni());
    let dx = a.right().saturating_sub(cw);
    let mut bx = a.x + 1;
    bx += cx.put(
        buf,
        bx,
        y,
        2,
        "cx",
        cx.st(Tok::Accent).add_modifier(Modifier::BOLD).patch(bar),
    );
    cx.hit(Rect::new(a.x, y, 3, 1), "hub");
    let sep = cx.g(" › ", " / ");
    bx += cx.put(buf, bx, y, 3, sep, cx.ink(Tok::Faint, 1.0).patch(bar));
    if let Some(d) = cx.s.device(&x.device) {
        let x0 = bx;
        bx += cx.dchip(buf, bx, y, d, false);
        if a.width >= 60 {
            bx += 1;
            bx += cx.put(buf, bx, y, 20, &d.name, cx.ink(Tok::Fg, 0.9).patch(bar));
        }
        cx.hit(Rect::new(x0, y, bx - x0, 1), format!("device:{}", d.id));
        bx += cx.put(buf, bx, y, 3, sep, cx.ink(Tok::Faint, 1.0).patch(bar));
    }
    let (g, word, sst) = cx.sstate(x);
    let state = if a.width >= 80 {
        format!("{g} {word}")
    } else {
        g.to_string()
    };
    let sw = super::width(&state, cx.uni());
    let room = dx.saturating_sub(bx + sw + 3);
    bx += cx.put(
        buf,
        bx,
        y,
        room,
        &fit(&x.title, room as usize, cx.uni()),
        cx.ink(Tok::Fg, 1.0).add_modifier(Modifier::BOLD).patch(bar),
    ) + 2;
    cx.put(buf, bx, y, sw, &state, sst.patch(bar));
    cx.put(buf, dx, y, cw, chip, cx.key_style());
    cx.hit(Rect::new(dx, y, cw, 1), "detach");
}

// ---------------------------------------------------------------------------- files

fn files(cx: &Cx, buf: &mut Buffer, r: Rect) {
    let sz = cx.size();
    let list_w = if sz >= Size::M {
        (r.width * 2 / 5).clamp(40, 64)
    } else {
        r.width.saturating_sub(2)
    };
    let list = Rect::new(r.x + 1, r.y + 1, list_w, r.height.saturating_sub(1));
    let f = &cx.s.files;
    let max = f.entries.iter().map(|e| e.size).max().unwrap_or(1).max(1);
    let sel = f.selected;
    cx.label(buf, list.x, r.y, list_w, "FILES", true);
    cx.put_r(
        buf,
        list.right(),
        r.y,
        &format!("{} items", f.entries.len()),
        cx.st(Tok::Muted),
    );
    let rows = list.height as usize;
    let first = sel.saturating_sub(rows.saturating_sub(3));
    for (i, e) in f.entries.iter().enumerate().skip(first).take(rows) {
        let y = list.y + (i - first) as u16;
        let picked = i == sel;
        let row = Rect::new(list.x, y, list.width, 1);
        if picked {
            cx.fill(buf, row, cx.st(Tok::AccentBg));
            cx.put(buf, list.x, y, 1, cx.g("▌", ">"), cx.st(Tok::Accent));
        }
        let mut x = list.x + 1;
        // Indent guides.
        for _ in 0..e.depth {
            x += cx.put(buf, x, y, 2, cx.g("│ ", "| "), cx.ink(Tok::Faint, 1.0));
        }
        let (icon, nst) = match e.kind {
            FileKind::Dir => (
                if e.expanded {
                    cx.g("▾ ", "- ")
                } else {
                    cx.g("▸ ", "+ ")
                },
                cx.st(Tok::Dir).add_modifier(Modifier::BOLD),
            ),
            FileKind::Symlink => (cx.g("→ ", "@ "), cx.st(Tok::Info)),
            FileKind::File => ("  ", cx.ink(Tok::Fg, 0.95)),
        };
        x += cx.put(buf, x, y, 2, icon, cx.ink(Tok::Muted, 0.9));
        let right_w: u16 = if list.width >= 44 { 20 } else { 8 };
        let room = list.right().saturating_sub(x + right_w + 2);
        let name = if e.kind == FileKind::Dir {
            format!("{}/", e.name)
        } else {
            e.name.clone()
        };
        x += cx.put(buf, x, y, room, &fit(&name, room as usize, cx.uni()), nst);
        if let Some(gc) = e.git {
            let tok = match gc {
                'M' => Tok::Warn,
                'A' => Tok::Ok,
                'D' => Tok::Err,
                _ => Tok::Muted,
            };
            cx.put(
                buf,
                x + 1,
                y,
                1,
                &gc.to_string(),
                cx.st(tok).add_modifier(Modifier::BOLD),
            );
        }
        let rx = list.right();
        let age = fmt::age(e.modified_secs);
        cx.put_r(buf, rx, y, &age, cx.st(Tok::Muted));
        if e.kind != FileKind::Dir && right_w >= 20 {
            let sz = fmt::bytes(e.size);
            cx.put_r(buf, rx.saturating_sub(5), y, &sz, cx.ink(Tok::Fg, 0.7));
            let bw = 6u16;
            let v = ((e.size as f32).ln() / (max as f32).ln()).clamp(0.0, 1.0);
            tiles::gauge(
                cx,
                buf,
                rx.saturating_sub(5 + 7 + bw),
                y,
                bw,
                v,
                cx.ink(Tok::Info, 0.8),
            );
        }
        cx.hit(row, format!("file:{}", e.name));
    }
    // Rows left under the tree: the selected entry's facts and where `s` can send it,
    // as the same device keycaps the schematic uses.
    let shown = f.entries.len().saturating_sub(first).min(rows) as u16;
    let dy = list.y + shown + 1;
    if let Some(e) = f.entries.get(sel).filter(|_| dy + 5 <= list.bottom()) {
        zone(cx, buf, list.x, dy, list.right(), "SELECTED", false);
        let path = format!(
            "{}/{}",
            f.cwd.trim_end_matches('/'),
            cx.s.files.preview_path
        );
        let git = match e.git {
            Some('M') => "modified since the last commit",
            Some('A') => "new, staged",
            Some('?') => "untracked",
            _ => "unchanged",
        };
        let facts = [
            path,
            format!(
                "{} {} modified {} ago",
                fmt::bytes(e.size),
                cx.g("·", "-"),
                fmt::age(e.modified_secs)
            ),
            format!("git: {git}"),
        ];
        for (i, t) in facts.iter().enumerate() {
            let w = list.width.saturating_sub(1);
            cx.put(
                buf,
                list.x + 1,
                dy + 2 + i as u16,
                w,
                &fit(t, w as usize, cx.uni()),
                cx.ink(Tok::Fg, 0.85),
            );
        }
        let sy = dy + 6;
        if sy < list.bottom() {
            // `s` is the key; the letters after it are what `s` then takes, drawn as
            // plain coloured text (here they are not keys on their own).
            let mut x = list.x + 1;
            x += cx.put(buf, x, sy, 3, " s ", cx.key_style()) + 1;
            x += cx.put(buf, x, sy, 9, "send to ", cx.st(Tok::Muted));
            for d in cx.s.devices.iter().filter(|d| d.id != f.device && cx.up(d)) {
                if x + 2 > list.right() {
                    break;
                }
                let st = cx
                    .ink(Tok::Id(cx.s.hue(&d.id)), 1.0)
                    .add_modifier(Modifier::BOLD);
                cx.put(buf, x, sy, 1, &d.hotkey.to_string(), st);
                cx.hit(Rect::new(x, sy, 1, 1), format!("send:{}", d.id));
                x += 2;
            }
        }
    }
    if sz >= Size::M {
        let pv = Rect::new(
            list.right() + 3,
            r.y,
            r.right().saturating_sub(list.right() + 4),
            r.height,
        );
        file_preview(cx, buf, pv, false);
    }
}

fn file_preview(cx: &Cx, buf: &mut Buffer, r: Rect, full: bool) {
    let Some(p) = cx.s.preview() else { return };
    let r = if full { inset(r, 1, 0) } else { r };
    let lang = match p.kind {
        PreviewKind::Markdown => "markdown",
        PreviewKind::Source(l) => l,
    };
    let lines: Vec<&str> = p.text.lines().collect();
    cx.label(buf, r.x, r.y, r.width, "PREVIEW", false);
    let meta = format!("{} · {} · {} lines", p.path, lang, lines.len());
    cx.put_r(
        buf,
        r.right(),
        r.y,
        &fit(&meta, r.width.saturating_sub(12) as usize, cx.uni()),
        cx.st(Tok::Muted),
    );
    // The surface is as tall as the file (a short file leaves no tinted void).
    let body = Rect::new(
        r.x,
        r.y + 1,
        r.width,
        r.height.saturating_sub(1).min(lines.len() as u16 + 1),
    );
    if cx.has_surface() {
        cx.fill(buf, body, cx.surface());
    }
    let gw = 5u16;
    let mut in_code = false;
    for (i, l) in lines.iter().take(body.height as usize).enumerate() {
        let y = body.y + i as u16;
        cx.put(
            buf,
            body.x,
            y,
            gw,
            &format!("{:>3}  ", i + 1),
            cx.ink(Tok::Faint, 1.0),
        );
        let x = body.x + gw;
        let room = body.right().saturating_sub(x + 1);
        match p.kind {
            PreviewKind::Markdown => {
                let t = l.trim_start();
                let st = if t.starts_with("```") {
                    in_code = !in_code;
                    cx.ink(Tok::Faint, 1.0)
                } else if in_code {
                    cx.st(Tok::Lan)
                } else if t.starts_with('#') {
                    cx.st(Tok::Accent).add_modifier(Modifier::BOLD)
                } else {
                    cx.ink(Tok::Fg, 0.95)
                };
                cx.put(buf, x, y, room, l, st);
            }
            PreviewKind::Source(_) => rust_line(cx, buf, x, y, room, l),
        }
        // A line cut at the edge says so in the last column.
        if super::width(l, cx.uni()) > room {
            cx.put(buf, x + room, y, 1, cx.g("›", "$"), cx.st(Tok::Accent));
        }
    }
}

/// Tiny Rust tinting: comments, strings, keywords, types.
fn rust_line(cx: &Cx, buf: &mut Buffer, x: u16, y: u16, room: u16, l: &str) {
    let t = l.trim_start();
    if t.starts_with("//") {
        cx.put(
            buf,
            x,
            y,
            room,
            l,
            cx.st(Tok::Muted).add_modifier(Modifier::ITALIC),
        );
        return;
    }
    const KW: [&str; 16] = [
        "use", "pub", "fn", "let", "mut", "const", "struct", "impl", "match", "if", "else",
        "return", "for", "in", "mod", "while",
    ];
    let mut xx = x;
    let mut word = String::new();
    let mut in_str = false;
    let flush = |w: &mut String, xx: &mut u16, buf: &mut Buffer| {
        if w.is_empty() {
            return;
        }
        let st = if KW.contains(&w.as_str()) {
            cx.st(Tok::Mesh)
        } else if w.chars().next().is_some_and(|c| c.is_ascii_uppercase()) {
            cx.st(Tok::Claude)
        } else if w.chars().all(|c| c.is_ascii_digit() || c == '_') {
            cx.st(Tok::Warn)
        } else {
            cx.ink(Tok::Fg, 0.95)
        };
        *xx += cx.put(buf, *xx, y, (x + room).saturating_sub(*xx), w, st);
        w.clear();
    };
    for ch in l.chars() {
        if in_str {
            xx += cx.put(
                buf,
                xx,
                y,
                (x + room).saturating_sub(xx),
                &ch.to_string(),
                cx.st(Tok::Ok),
            );
            if ch == '"' {
                in_str = false;
            }
            continue;
        }
        if ch.is_alphanumeric() || ch == '_' {
            word.push(ch);
            continue;
        }
        flush(&mut word, &mut xx, buf);
        if ch == '"' {
            in_str = true;
            xx += cx.put(
                buf,
                xx,
                y,
                (x + room).saturating_sub(xx),
                "\"",
                cx.st(Tok::Ok),
            );
        } else {
            xx += cx.put(
                buf,
                xx,
                y,
                (x + room).saturating_sub(xx),
                &ch.to_string(),
                cx.ink(Tok::Fg, 0.7),
            );
        }
    }
    flush(&mut word, &mut xx, buf);
}

// -------------------------------------------------------------------------- palette

/// Send-to picker over the files view: a bottom sheet holding the schematic seen from
/// the files' device, where pressing (or tapping) a device sends the file there; the
/// transfer then shows as the packet on that link.
fn send_to(cx: &Cx, buf: &mut Buffer, a: Rect) {
    cx.dim(buf, a);
    let h = (orbit::want_h(cx) + 3).min(a.height);
    let r = Rect::new(a.x, a.bottom().saturating_sub(h), a.width, h);
    panel(cx, buf, r);
    let f = &cx.s.files;
    let mut x = r.x + 2;
    let y = r.y + 1;
    x += cx.put(buf, x, y, 6, "send ", cx.ink(Tok::Fg, 0.9));
    let room = r.right().saturating_sub(x + 30);
    x += cx.put(
        buf,
        x,
        y,
        room,
        &fit(&f.preview_path, room as usize, cx.uni()),
        cx.ink(Tok::Fg, 1.0).add_modifier(Modifier::BOLD),
    );
    cx.put(buf, x, y, 4, " to", cx.ink(Tok::Fg, 0.9));
    if cx.size() >= Size::S {
        cx.put_r(
            buf,
            r.right() - 2,
            y,
            "press a device letter",
            cx.st(Tok::Muted),
        );
    }
    let band = Rect::new(r.x, y + 1, r.width, r.bottom().saturating_sub(y + 1));
    orbit::draw(cx, buf, band, &f.device, 3, orbit::Sel::Targets);
}

fn panel(cx: &Cx, buf: &mut Buffer, r: Rect) {
    if cx.has_surface() {
        cx.fill(buf, r, cx.st(Tok::Surface2));
        let edge = cx.ink(Tok::Accent, 0.9);
        for x in r.x..r.right() {
            cx.put(buf, x, r.y, 1, "▔", edge.patch(cx.st(Tok::Surface2)));
        }
    } else {
        cx.fill(buf, r, Style::default());
        let st = cx.st(Tok::Accent);
        let w = r.width.saturating_sub(2) as usize;
        let (tl, tr, bl, br, h, v) = if cx.uni() {
            ("┏", "┓", "┗", "┛", "━", "┃")
        } else {
            ("#", "#", "#", "#", "=", "#")
        };
        cx.put(
            buf,
            r.x,
            r.y,
            r.width,
            &format!("{tl}{}{tr}", h.repeat(w)),
            st,
        );
        cx.put(
            buf,
            r.x,
            r.bottom() - 1,
            r.width,
            &format!("{bl}{}{br}", h.repeat(w)),
            st,
        );
        for y in r.y + 1..r.bottom() - 1 {
            cx.put(buf, r.x, y, 1, v, st);
            cx.put(buf, r.right() - 1, y, 1, v, st);
        }
    }
}

fn palette(cx: &Cx, buf: &mut Buffer, a: Rect, q: &str) {
    let ql = q.to_lowercase();
    let mut rows: Vec<(String, Vec<(String, Style)>, String, String)> = Vec::new();
    for d in cx.s.devices.iter().filter(|d| d.name.contains(&ql)) {
        let (g, gst) = cx.status(d.status);
        rows.push((
            "device".into(),
            vec![
                (format!("{g} "), gst),
                (
                    d.name.clone(),
                    cx.ink(Tok::Fg, 1.0).add_modifier(Modifier::BOLD),
                ),
                (format!("  {}", cx.meta(d)), cx.st(Tok::Muted)),
            ],
            d.hotkey.to_string(),
            format!("device:{}", d.id),
        ));
        for x in cx.s.sessions_on(&d.id) {
            let (g, _, st) = cx.sstate(x);
            let slot = cx.slot(x).unwrap_or(' ');
            rows.push((
                "session".into(),
                vec![
                    (format!("{g} "), st),
                    (x.title.clone(), cx.ink(Tok::Fg, 0.95)),
                    (format!("  on {}", d.name), cx.st(Tok::Muted)),
                ],
                format!("{} {slot}", d.hotkey),
                format!("session:{}", x.id),
            ));
        }
        // Every row shows the complete key sequence from the hub.
        let mut verbs = vec![
            ("new claude session on ", format!("{} c", d.hotkey)),
            ("new codex session on ", format!("{} x", d.hotkey)),
            ("new shell on ", format!("{} n", d.hotkey)),
            ("files on ", format!("{} f", d.hotkey)),
        ];
        if d.id != cx.s.viewer {
            verbs.push(("edit link to ", format!("e {}", d.hotkey)));
        }
        for (v, k) in verbs {
            rows.push((
                "action".into(),
                vec![
                    (format!("{} ", cx.g("›", ">")), cx.st(Tok::Accent)),
                    (format!("{v}{}", d.name), cx.ink(Tok::Fg, 0.95)),
                ],
                k,
                format!("cmd:{v}{}", d.id),
            ));
        }
    }
    for (name, k) in [
        ("invite a device (join code)", "+"),
        ("keys and secrets", "K"),
    ] {
        if name.contains(&ql) || ql.is_empty() {
            rows.push((
                "action".into(),
                vec![
                    (format!("{} ", cx.g("›", ">")), cx.st(Tok::Accent)),
                    (name.into(), cx.ink(Tok::Fg, 0.95)),
                ],
                k.into(),
                format!("cmd:{name}"),
            ));
        }
    }
    // Sized to its results (query, a gap, rows and group labels, the key hint), at most
    // 60 % of the screen.
    let groups = {
        let mut g: Vec<&str> = rows.iter().map(|r| r.0.as_str()).collect();
        g.dedup();
        g.len() as u16
    };
    let want = rows.len() as u16 + groups + 5;
    let w = 64.min(a.width.saturating_sub(4));
    let h = want
        .min(a.height * 3 / 5)
        .max(8)
        .min(a.height.saturating_sub(2));
    let r = if cx.touch() {
        // Phones: a full sheet under the top bar on a cleared backdrop (a faded hub in
        // the margins would read as stray glyphs at this size).
        cx.fill(
            buf,
            Rect::new(a.x, a.y + 1, a.width, a.height - 1),
            Style::default(),
        );
        Rect::new(a.x, a.y + 1, a.width, a.height - 1)
    } else {
        Rect::new(
            a.x + (a.width - w) / 2,
            a.y + 1 + (a.height.saturating_sub(h + 2)) / 3,
            w,
            h,
        )
    };
    panel(cx, buf, r);
    let inner = inset(r, 2, 1);
    let y = inner.y;
    let mut x = inner.x;
    x += cx.put(
        buf,
        x,
        y,
        2,
        "/ ",
        cx.st(Tok::Accent).add_modifier(Modifier::BOLD),
    );
    x += cx.put(
        buf,
        x,
        y,
        inner.width.saturating_sub(4),
        q,
        cx.ink(Tok::Fg, 1.0).add_modifier(Modifier::BOLD),
    );
    cx.put(buf, x, y, 1, cx.g("▏", "_"), cx.st(Tok::Accent));
    // The last row teaches the palette's own keys.
    let hint_y = inner.bottom().saturating_sub(1);
    let hint = format!("{} go  {} move  esc close", cx.ret(), cx.g("↑↓", "up/dn"));
    cx.put(buf, inner.x, hint_y, inner.width, &hint, cx.st(Tok::Muted));
    let inner = Rect::new(
        inner.x,
        inner.y,
        inner.width,
        inner.height.saturating_sub(1),
    );
    let mut yy = y + 2;
    let mut last_group = String::new();
    let sel = cx.c.selected % rows.len().max(1);
    for (i, (group, spans, key, act)) in rows.iter().enumerate() {
        if yy >= inner.bottom() {
            break;
        }
        if *group != last_group {
            if yy + 1 >= inner.bottom() {
                break;
            }
            cx.put(
                buf,
                inner.x,
                yy,
                inner.width,
                &group.to_uppercase(),
                cx.ink(Tok::Muted, 0.8).add_modifier(Modifier::BOLD),
            );
            yy += 1;
            last_group = group.clone();
        }
        let row = Rect::new(inner.x.saturating_sub(1), yy, inner.width + 2, 1);
        if i == sel {
            cx.fill(buf, row, cx.st(Tok::AccentBg));
            cx.put(buf, row.x, yy, 1, cx.g("▌", ">"), cx.st(Tok::Accent));
        }
        let mut x = inner.x + 1;
        // The description is cut (with an ellipsis) before the key column.
        for (t, st) in spans {
            let room = inner.right().saturating_sub(x + key.len() as u16 + 3);
            if room == 0 {
                break;
            }
            x += cx.put(buf, x, yy, room, &fit(t, room as usize, cx.uni()), *st);
        }
        // The selected row's tint runs under its keys too.
        let kst = if i == sel {
            cx.st(Tok::AccentBg)
                .patch(cx.st(Tok::Accent))
                .add_modifier(Modifier::BOLD)
        } else {
            cx.key_style()
        };
        cx.put_r(buf, inner.right(), yy, &format!(" {key} "), kst);
        if i == sel && !cx.has_surface() {
            // Without tints the selection is one reversed run across the whole row.
            for xx in row.x..row.right() {
                buf[(xx, yy)]
                    .set_style(Style::reset().add_modifier(Modifier::REVERSED | Modifier::BOLD));
            }
        }
        cx.hit(row, act.clone());
        yy += 1;
    }
}

// -------------------------------------------------------------------------- secrets

fn secrets(cx: &Cx, buf: &mut Buffer, r: Rect) {
    // On tall screens the cards sit a quarter of the spare height down, not under the bar.
    let need = 4 + cx.s.secrets.len() as u16 * 5;
    let r = if r.height > need + 8 {
        let dy = (r.height - need) / 4;
        Rect::new(r.x, r.y + dy, r.width, r.height - dy)
    } else {
        r
    };
    let w = r.width.saturating_sub(4).min(96);
    let x0 = r.x + (r.width - w) / 2;
    let mut top = r.y + 1;
    cx.label(buf, x0, r.y, w, "SECRETS", true);
    let note = if w >= 60 {
        "stored in your keyring · shown only on reveal"
    } else {
        "shown only on reveal"
    };
    cx.put_r(buf, x0 + w, r.y, note, cx.st(Tok::Muted));
    let compact = cx.size() <= Size::S;
    let card_h = if compact { 3 } else { 4 };
    for (i, s) in cx.s.secrets.iter().enumerate() {
        if top + card_h > r.bottom() {
            break;
        }
        let card = Rect::new(x0, top, w, card_h);
        let y = top + u16::from(!compact);
        let picked = i == cx.c.selected % cx.s.secrets.len().max(1);
        let missing = s.masked.is_none();
        cx.fill(
            buf,
            card,
            if picked {
                cx.st(Tok::AccentBg)
            } else {
                cx.surface()
            },
        );
        if picked {
            for yy in card.y..card.bottom() {
                cx.put(buf, card.x, yy, 1, cx.g("▌", ">"), cx.st(Tok::Accent));
            }
        }
        let ix = card.x + 2;
        let (g, gst) = if missing {
            (cx.g("○", "-"), cx.st(Tok::Muted))
        } else {
            (cx.g("●", "o"), cx.st(Tok::Ok))
        };
        let right = card.right().saturating_sub(2);
        // Each card has a home-row key, like session slots.
        let mut x = ix;
        if let Some(k) = SLOTS.get(i) {
            x += cx.put(buf, x, y, 3, &format!(" {k} "), cx.key_style()) + 1;
        }
        x += cx.put(buf, x, y, 1, g, gst) + 1;
        let lw = if missing {
            right.saturating_sub(x + 12)
        } else {
            right.saturating_sub(x)
        }
        .min(w / 2);
        x += cx.put(
            buf,
            x,
            y,
            lw,
            &fit(&s.label, lw as usize, cx.uni()),
            cx.ink(Tok::Fg, 1.0).add_modifier(Modifier::BOLD),
        );
        if !compact {
            cx.put(buf, x + 2, y, w / 3, &s.key, cx.ink(Tok::Muted, 0.8));
        }
        let vx = ix + 4;
        if missing {
            let label = if s.key.starts_with("cloudflare") {
                "connect"
            } else {
                "add"
            };
            let t = format!(" {} {label} ", cx.ret());
            let st = cx
                .wash(Tok::Accent, 0.35)
                .patch(cx.st(Tok::Accent))
                .add_modifier(Modifier::BOLD);
            let st = if cx.has_surface() { st } else { cx.key_style() };
            let n = cx.put_r(buf, right, y, &t, st);
            cx.hit(
                Rect::new(right - n, y, n, 1),
                format!("secret-add:{}", s.key),
            );
            cx.put(
                buf,
                vx,
                y + 1,
                w.saturating_sub(6),
                "not set",
                cx.st(Tok::Muted),
            );
        } else {
            let v = s.masked.clone().unwrap_or_default();
            let age = s.updated_secs.map(fmt::age).unwrap_or_default();
            let src = format!("{} · {age} ago", s.source);
            let sw = super::width(&src, cx.uni());
            // Value and source share a row only when both fit with a clear gap.
            let vw = super::width(&v, cx.uni());
            if compact && vx + vw + 3 + sw > right {
                cx.put(
                    buf,
                    vx,
                    y + 1,
                    right.saturating_sub(vx),
                    &v,
                    cx.ink(Tok::Fg, 0.9),
                );
                if card_h >= 3 {
                    cx.put(
                        buf,
                        vx,
                        y + 2,
                        right.saturating_sub(vx),
                        &fit(&src, right.saturating_sub(vx) as usize, cx.uni()),
                        cx.st(Tok::Muted),
                    );
                }
            } else {
                cx.put(buf, vx, y + 1, 30, &v, cx.ink(Tok::Fg, 0.9));
                let sy = if compact { y + 1 } else { y };
                cx.put_r(buf, right, sy, &src, cx.st(Tok::Muted));
                if !s.used_by.is_empty() && card_h >= 3 {
                    let used = format!("used by {}", s.used_by.join(", "));
                    let ux = if compact { vx } else { ix + 36 };
                    let uy = if compact { y + 2 } else { y + 1 };
                    cx.put(
                        buf,
                        ux,
                        uy,
                        right.saturating_sub(ux),
                        &fit(&used, right.saturating_sub(ux) as usize, cx.uni()),
                        cx.ink(Tok::Muted, 0.85),
                    );
                }
            }
        }
        cx.hit(card, format!("secret:{}", s.key));
        top += card_h + 1;
    }
    // Cloudflare connection summary.
    if top + 3 < r.bottom() {
        let cf =
            cx.s.secrets
                .iter()
                .find(|s| s.key.starts_with("cloudflare"));
        let ok = cf.is_some_and(|s| s.masked.is_some());
        let text = if ok {
            "Cloudflare connected · mesh cloudflare-warp healthy · join codes enabled"
        } else {
            "Connect Cloudflare to create a mesh and hand out join codes"
        };
        cx.put(
            buf,
            x0 + 2,
            top + 1,
            w,
            text,
            if ok { cx.st(Tok::Ok) } else { cx.st(Tok::Warn) },
        );
    }
}

// ----------------------------------------------------------------------- onboarding

fn onboarding(cx: &Cx, buf: &mut Buffer, r: Rect) {
    let w = r.width.saturating_sub(4).min(72);
    let x0 = r.x + (r.width - w) / 2;
    let small = r.height < 24 || cx.size() == Size::Xs;
    let mut y = r.y
        + if small {
            1
        } else {
            2 + r.height.saturating_sub(34) / 4
        };
    cx.put(
        buf,
        x0,
        y,
        w,
        "Connect your machines",
        cx.ink(Tok::Fg, 1.0).add_modifier(Modifier::BOLD),
    );
    y += 1;
    let sub = "cx sees this device. Add a mesh to reach the others from anywhere.";
    // The only explanatory sentence on a first run: wrapped, never cut.
    for line in wrap(sub, w as usize).iter().take(3) {
        cx.put(buf, x0, y, w, line, cx.st(Tok::Muted));
        y += 1;
    }
    y += 1;
    // Illustration: this device with three ghost orbits.
    let ill_h: u16 = if small { 5 } else { 9 };
    let ill = Rect::new(x0, y, w, ill_h);
    if cx.uni() && cx.c.has_color() {
        let (pcx, pcy) = (
            (ill.x + ill.width / 2) as f32 * 2.0,
            (ill.y + ill.height / 2) as f32 * 4.0 + 2.0,
        );
        for ring in 1..=2 {
            let rx = ill.width as f32 * 0.42 * ring as f32 / 2.0 * 2.0;
            let ry = ill.height as f32 * 0.5 * ring as f32 / 2.0 * 4.0 - 2.0;
            let st = cx.ink(Tok::Muted, 0.35);
            let mut cells: std::collections::HashMap<(u16, u16), u8> = Default::default();
            for k in 0..360 {
                if k % 4 != 0 {
                    continue;
                }
                let a = k as f32 / 360.0 * std::f32::consts::TAU;
                let (px, py) = (
                    (pcx + rx * a.cos()).round() as i32,
                    (pcy + ry * a.sin()).round() as i32,
                );
                if px < 0 || py < 0 {
                    continue;
                }
                let (cxx, cyy) = ((px / 2) as u16, (py / 4) as u16);
                if cyy < ill.y || cyy >= ill.bottom() || cxx < ill.x || cxx >= ill.right() {
                    continue;
                }
                const BITS: [[u8; 4]; 2] = [[0x01, 0x02, 0x04, 0x40], [0x08, 0x10, 0x20, 0x80]];
                *cells.entry((cxx, cyy)).or_default() |= BITS[(px % 2) as usize][(py % 4) as usize];
            }
            for ((xx, yy), m) in cells {
                cx.put(
                    buf,
                    xx,
                    yy,
                    1,
                    &char::from_u32(0x2800 + u32::from(m))
                        .unwrap_or(' ')
                        .to_string(),
                    st,
                );
            }
        }
        for (dx, dy) in [
            (-0.3f32, -0.3f32),
            (0.32, 0.25),
            (0.18, -0.38),
            (-0.22, 0.35),
        ] {
            let x = (ill.x as f32 + ill.width as f32 * (0.5 + dx)) as u16;
            let y = (ill.y as f32 + ill.height as f32 * (0.5 + dy)) as u16;
            cx.put(buf, x, y, 1, "◌", cx.ink(Tok::Muted, 0.6));
        }
    }
    if let Some(d) = cx.s.device(&cx.s.viewer) {
        let mid = ill.y + ill.height / 2;
        let t = format!("   {}  ", d.name);
        let pw = t.len() as u16 + 3;
        let px = ill.x + (ill.width.saturating_sub(pw)) / 2;
        let tint = mix(cx.bg0(), cx.rgb(Tok::Accent), 0.2);
        cx.fill(buf, Rect::new(px, mid, pw, 1), cx.rgb_bg(tint));
        cx.dchip(buf, px + 1, mid, d, false);
        cx.put(
            buf,
            px + 5,
            mid,
            pw,
            &d.name,
            cx.ink(Tok::Fg, 1.0).add_modifier(Modifier::BOLD),
        );
    }
    y += ill_h + 1;
    // Primary action.
    let btn = format!("  {}  Connect Cloudflare  ", cx.ret());
    let bw = super::width(&btn, cx.uni());
    let bst = if cx.c.has_color() {
        Style::default()
            .bg(cx
                .c
                .color(Tok::Accent)
                .unwrap_or(ratatui::style::Color::Reset))
            .fg(ratatui::style::Color::Black)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD)
    };
    let bst = if cx.tc() {
        bst.fg(cx.col(cx.bg0()).unwrap_or(ratatui::style::Color::Black))
    } else {
        bst
    };
    cx.put(buf, x0, y, bw, &btn, bst);
    cx.hit(Rect::new(x0, y, bw, 1), "connect-cloudflare");
    if !small || cx.size() > Size::Xs {
        cx.put(
            buf,
            x0 + bw + 2,
            y,
            w.saturating_sub(bw + 2),
            "opens dash.cloudflare.com with a ready token",
            cx.st(Tok::Muted),
        );
    }
    y += 2;
    let steps = [
        (
            "1",
            "Create a token",
            "scopes: Zero Trust edit, Access edit",
        ),
        (
            "2",
            "Paste it here",
            "stored in your keyring; cx never prints it",
        ),
        (
            "3",
            "Invite devices",
            "a short code, a one-liner or a QR on the phone",
        ),
    ];
    for (n, t, d) in steps {
        if y + 1 >= r.bottom() {
            break;
        }
        // Step numbers are plain text, not keycaps: they are not keys.
        let mut x = x0 + 1;
        x += cx.put(buf, x, y, 2, &format!("{n}."), cx.st(Tok::Muted));
        x += 2;
        cx.put(
            buf,
            x,
            y,
            18,
            t,
            cx.ink(Tok::Fg, 1.0).add_modifier(Modifier::BOLD),
        );
        if cx.size() >= Size::S {
            cx.put(buf, x0 + 24, y, w.saturating_sub(24), d, cx.st(Tok::Muted));
        }
        y += if small { 1 } else { 2 };
    }
    // Secondary ways in, one per row, after a breathing row.
    y += 1;
    for (k, t, act) in [
        ("j", "I have a join code", "join"),
        ("esc", "plain SSH for now", "skip"),
    ] {
        if y >= r.bottom() {
            break;
        }
        let mut x = x0;
        x += cx.put(buf, x, y, 5, &format!(" {k} "), cx.key_style());
        x += 1;
        x += cx.put(buf, x, y, 24, t, cx.ink(Tok::Fg, 0.9));
        cx.hit(Rect::new(x0, y, x - x0, 1), act);
        y += 1;
    }
}

// ----------------------------------------------------------------------------- join

fn join(cx: &Cx, buf: &mut Buffer, r: Rect) {
    let Some(inv) = cx.s.invite.as_ref() else {
        cx.put(
            buf,
            r.x + 2,
            r.y + 1,
            r.width,
            "No invite pending. r creates one.",
            cx.st(Tok::Muted),
        );
        return;
    };
    let n = inv.qr.len() as u16;
    let qw = n + 4;
    let qh = (n + 4).div_ceil(2);
    let side_by_side = r.width >= qw + 44 && r.height >= qh;
    let (qx, qy, tx, ty, tw) = if side_by_side {
        let total = qw + 4 + 62.min(r.width - qw - 6);
        let x0 = r.x + (r.width.saturating_sub(total)) / 2;
        let qy = r.y + (r.height.saturating_sub(qh)) / 2;
        (x0, qy, x0 + qw + 4, qy + 1, total - qw - 4)
    } else {
        let x0 = r.x + (r.width.saturating_sub(qw)) / 2;
        (x0, r.y, r.x + 2, r.y + qh + 1, r.width.saturating_sub(4))
    };
    // QR in half blocks with a light quiet zone, always dark-on-light.
    let room_h = r.bottom().saturating_sub(qy);
    let show_qr = room_h >= qh && cx.uni() && r.height >= qh + 4 || side_by_side;
    if show_qr && cx.uni() {
        let (light, dark) = ((0xf4, 0xf4, 0xf2), (0x12, 0x15, 0x19));
        let module = |r_: i32, c: i32| -> bool {
            if r_ < 0 || c < 0 || r_ >= n as i32 || c >= n as i32 {
                return false;
            }
            inv.qr[r_ as usize][c as usize]
        };
        for row in 0..qh {
            for col in 0..qw {
                let (t, b) = (
                    module(row as i32 * 2 - 2, col as i32 - 2),
                    module(row as i32 * 2 - 1, col as i32 - 2),
                );
                let (f, bg) = (if t { dark } else { light }, if b { dark } else { light });
                let st = match cx.c.color {
                    crate::ui::lab::ColorLevel::None => Style::default(),
                    crate::ui::lab::ColorLevel::C16 => Style::default()
                        .fg(if t {
                            ratatui::style::Color::Black
                        } else {
                            ratatui::style::Color::White
                        })
                        .bg(if b {
                            ratatui::style::Color::Black
                        } else {
                            ratatui::style::Color::White
                        }),
                    _ => cx.rgb_fg(f).patch(cx.rgb_bg(bg)),
                };
                let sym = if cx.c.has_color() {
                    "▀"
                } else {
                    match (t, b) {
                        (true, true) => " ",
                        (true, false) => "▄",
                        (false, true) => "▀",
                        (false, false) => "█",
                    }
                };
                cx.put(buf, qx + col, qy + row, 1, sym, st);
            }
        }
        cx.hit(Rect::new(qx, qy, qw, qh), "qr");
    } else if !side_by_side {
        // No QR in ASCII; text carries everything.
    }
    let ty = if show_qr || side_by_side { ty } else { r.y + 1 };
    let mut y = ty;
    cx.put(
        buf,
        tx,
        y,
        tw,
        "Invite a device",
        cx.ink(Tok::Fg, 1.0).add_modifier(Modifier::BOLD),
    );
    y += 1;
    cx.put(
        buf,
        tx,
        y,
        tw,
        &format!("joins {} as a guest", inv.mesh),
        cx.st(Tok::Muted),
    );
    y += 2;
    // Big code as key caps.
    let mut x = tx;
    for ch in inv.code.chars() {
        if ch == '-' {
            x += cx.put(buf, x, y, 2, cx.g(" ·", " -"), cx.st(Tok::Muted)) + 1;
            continue;
        }
        let st = if cx.has_surface() {
            cx.st(Tok::Surface2)
                .patch(cx.st(Tok::Accent))
                .add_modifier(Modifier::BOLD)
        } else {
            cx.key_style()
        };
        x += cx.put(buf, x, y, 3, &format!(" {ch} "), st) + 1;
    }
    cx.hit(Rect::new(tx, y, x - tx, 1), "copy-code");
    y += 2;
    if y < r.bottom() {
        cx.put(
            buf,
            tx,
            y,
            tw,
            "or run on the new device",
            cx.st(Tok::Muted),
        );
        y += 1;
    }
    if y < r.bottom() {
        // The command wraps (up to three rows) so it can be read and retyped where
        // clipboard escapes do not reach (Termux without permission, tmux without
        // `set-clipboard on`); copying says so when it cannot confirm.
        let cw = tw.saturating_sub(2).max(1) as usize;
        let chars: Vec<char> = inv.oneliner.chars().collect();
        let mut parts: Vec<String> = chars.chunks(cw).map(|c| c.iter().collect()).collect();
        if parts.len() > 3 {
            parts.truncate(3);
            parts[2] = fit(&format!("{}{}", parts[2], "…"), cw, cx.uni());
        }
        let block = Rect::new(tx, y, tw, parts.len() as u16);
        if cx.has_surface() {
            cx.fill(buf, block, cx.surface());
        }
        for (i, part) in parts.iter().enumerate() {
            cx.put(buf, tx + 1, y + i as u16, cw as u16, part, cx.st(Tok::Lan));
        }
        cx.hit(block, "copy-cmd");
        y += block.height + 1;
    }
    if y < r.bottom() {
        let left = inv.expires_secs as f32 / 1800.0;
        let t = format!("expires in {}m", inv.expires_secs / 60);
        cx.put(buf, tx, y, 16, &t, cx.st(Tok::Muted));
        tiles::gauge(
            cx,
            buf,
            tx + 17,
            y,
            tw.saturating_sub(17).min(20),
            left,
            cx.st(Tok::Accent),
        );
    }
}

// ------------------------------------------------------------------------ edge edit

fn edge_editor(cx: &Cx, buf: &mut Buffer, r: Rect, id: &str) {
    // The id names the opener first (`i e t` opens `innovation-tranquility`), so the
    // opener's outbound direction is the first lane and takes the lowercase keys.
    let rev = id.split_once('-').map(|(a, b)| format!("{b}-{a}"));
    let (e, flipped) = match (cx.s.edge(id), rev.as_deref().and_then(|r| cx.s.edge(r))) {
        (Some(e), _) => (e, false),
        (None, Some(e)) => (e, true),
        _ => match cx.s.edges.first() {
            Some(e) => (e, false),
            None => return,
        },
    };
    let sheet_min: u16 = if cx.size() == Size::Xs { 9 } else { 11 }.min(r.height);
    // The schematic starts right under the crumb, as tall as its content; the sheet
    // takes the rest down to a row above the footer (extra rows go to the rtt chart).
    let bottom = r.bottom().saturating_sub(1);
    let room = bottom.saturating_sub(r.y + sheet_min);
    let band_h = if cx.size() == Size::Xs {
        room.min(7)
    } else {
        room.min(orbit::want_h(cx))
    };
    let band = Rect::new(r.x, r.y, r.width, band_h);
    let sheet_h = bottom.saturating_sub(band.bottom());
    if band.height >= 5 {
        orbit::draw(cx, buf, band, &cx.s.viewer, 3, orbit::Sel::Edge(&e.id));
    } else if let (Some(a), Some(b)) = (cx.s.device(&e.from), cx.s.device(&e.to)) {
        // No room for the map: the link itself, drawn heavy between its two keycaps.
        let tok = edge_kind(cx, e).1;
        let mid = format!(" {} ", orbit::policy(cx, e, true, false));
        let line = cx.g("━━━", "===");
        let w = 6 + 2 * super::width(line, cx.uni()) + super::width(&mid, cx.uni());
        let mut x = r.x + r.width.saturating_sub(w) / 2;
        let y = r.y + band.height.max(1) / 2;
        x += cx.dchip(buf, x, y, a, false);
        x += cx.put(buf, x, y, 3, line, cx.ink(tok, 1.0));
        x += cx.put(
            buf,
            x,
            y,
            20,
            &mid,
            cx.ink(tok, 1.0).add_modifier(Modifier::BOLD),
        );
        x += cx.put(buf, x, y, 3, line, cx.ink(tok, 1.0));
        cx.dchip(buf, x, y, b, false);
    }
    let sheet = Rect::new(r.x, bottom.saturating_sub(sheet_h), r.width, sheet_h);
    panel(cx, buf, sheet);
    let inner = inset(sheet, 2, 1);
    let (Some(a), Some(b)) = (cx.s.device(&e.from), cx.s.device(&e.to)) else {
        return;
    };
    let (a, b) = if flipped { (b, a) } else { (a, b) };
    let mut x = inner.x;
    let y = inner.y;
    x += cx.dchip(buf, x, y, a, false);
    x += 1;
    x += cx.put(
        buf,
        x,
        y,
        20,
        &a.name,
        cx.ink(Tok::Fg, 1.0).add_modifier(Modifier::BOLD),
    );
    x += cx.put(buf, x, y, 5, cx.g("  ⟷  ", " <-> "), cx.st(Tok::Muted));
    x += cx.dchip(buf, x, y, b, false);
    x += 1;
    x += cx.put(
        buf,
        x,
        y,
        20,
        &b.name,
        cx.ink(Tok::Fg, 1.0).add_modifier(Modifier::BOLD),
    );
    let (kind, tok) = edge_kind(cx, e);
    if cx.size() >= Size::S {
        let rtt = e
            .rtt_ms
            .map(super::ms_text)
            .unwrap_or_else(|| "down".into());
        let t = format!("{kind} · {rtt} · loss {:.0}%", e.loss * 100.0);
        let tw = super::width(&t, cx.uni());
        if x + tw + 4 < inner.right() {
            cx.put_r(buf, inner.right(), y, &t, cx.ink(tok, 0.95));
        }
    }
    let _ = x;
    // Two lanes, one per direction, each a segmented control.
    let (out, inb) = if flipped {
        (e.reverse, e.forward)
    } else {
        (e.forward, e.reverse)
    };
    let lanes = [(a, b, out), (b, a, inb)];
    let sel_lane = cx.c.selected % 2;
    for (i, (f, t, acc)) in lanes.iter().enumerate() {
        let ly = inner.y + 2 + i as u16 * 2;
        if ly >= inner.bottom() {
            break;
        }
        let mut lx = inner.x;
        let picked = i == sel_lane;
        if picked {
            cx.put(
                buf,
                lx.saturating_sub(1),
                ly,
                1,
                cx.g("▌", ">"),
                cx.st(Tok::Accent),
            );
        }
        lx += cx.put(
            buf,
            lx,
            ly,
            1,
            &f.hotkey.to_string(),
            cx.st(Tok::Id(cx.s.hue(&f.id))).add_modifier(Modifier::BOLD),
        );
        lx += cx.put(buf, lx, ly, 3, cx.g(" → ", " > "), cx.st(Tok::Muted));
        lx += cx.put(
            buf,
            lx,
            ly,
            1,
            &t.hotkey.to_string(),
            cx.st(Tok::Id(cx.s.hue(&t.id))).add_modifier(Modifier::BOLD),
        );
        lx += 2;
        let opts = [
            (Access::Key, "k", "key"),
            (Access::Password, "p", "password"),
            (Access::Denied, "o", "off"),
        ];
        for (val, k, label) in opts {
            let on = *acc == val;
            let k = if i == 1 {
                k.to_uppercase()
            } else {
                k.to_string()
            };
            let text = format!(" {k} {label} ");
            let st = if on {
                if cx.c.has_color() {
                    let tok = match val {
                        Access::Key => Tok::Ok,
                        Access::Password => Tok::Warn,
                        Access::Denied => Tok::Err,
                    };
                    cx.wash(tok, 0.3)
                        .patch(cx.st(tok))
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD)
                }
            } else {
                cx.ink(Tok::Fg, 0.6)
            };
            let st = if on && !cx.has_surface() && cx.c.has_color() {
                st.add_modifier(Modifier::REVERSED)
            } else {
                st
            };
            let n = cx.put(buf, lx, ly, inner.right().saturating_sub(lx), &text, st);
            cx.hit(
                Rect::new(lx, ly, n, 1),
                format!("policy:{}:{}:{label}", f.id, t.id),
            );
            lx += n + 1;
        }
        if cx.size() >= Size::M {
            let what = match acc {
                Access::Key => {
                    format!("{} opens sessions on {} with a managed key", f.name, t.name)
                }
                Access::Password => format!("{} asks for {}'s password each time", f.name, t.name),
                Access::Denied => format!("{} cannot reach {}", f.name, t.name),
            };
            cx.put(
                buf,
                lx + 2,
                ly,
                inner.right().saturating_sub(lx + 2),
                &what,
                cx.st(Tok::Muted),
            );
        }
    }
    // What a change does: policies apply on the key press (u undoes), so the line states
    // the live effect, wrapped rather than cut.
    let crossing: Vec<&LabSession> = cx
        .s
        .sessions
        .iter()
        .filter(|x| {
            x.device == e.to && e.from == cx.s.viewer || x.device == e.from && e.to == cx.s.viewer
        })
        .collect();
    let users = crossing.len();
    // The keys are in the footer; the line says only what a change does.
    let how = if cx.size() == Size::Xs {
        ""
    } else {
        "A key applies at once; u undoes. "
    };
    let msg = format!(
        "{how}{users} open session{} cross{} this link.",
        if users == 1 { "" } else { "s" },
        if users == 1 { "es" } else { "" },
    );
    for (i, line) in wrap(&msg, inner.width as usize).iter().take(2).enumerate() {
        let cy = inner.y + 6 + i as u16;
        if cy < inner.bottom() {
            cx.put(buf, inner.x, cy, inner.width, line, cx.ink(Tok::Info, 0.95));
        }
    }
    let ry = inner.y + 8;
    if ry < inner.bottom() && cx.size() >= Size::S {
        // The rtt history: an area chart as tall as the sheet allows (up to 4 rows),
        // scaled between its min and max so jitter shows.
        let ch = inner.bottom().saturating_sub(ry).clamp(1, 4);
        let label = format!("rtt {} samples", e.rtt_hist.len());
        cx.put(buf, inner.x, ry + ch - 1, 15, &label, cx.st(Tok::Muted));
        let cw = inner.width.saturating_sub(16);
        tiles::rtt_chart(
            cx,
            buf,
            Rect::new(inner.x + 16, ry, cw, ch),
            &e.rtt_hist,
            tok,
        );
        // The sessions a change would affect, as their live thumbnails.
        let ty = ry + ch + 1;
        let th = inner.bottom().saturating_sub(ty);
        if th >= 6 && !crossing.is_empty() {
            let need = crossing
                .iter()
                .map(|x| tiles::tail_rows(x) as u16 + 2)
                .max()
                .unwrap_or(6);
            let tw = ((inner.width + 2) / crossing.len().min(3) as u16)
                .saturating_sub(2)
                .min(56);
            let n = crossing.len().min(3) as u16;
            for (i, x) in crossing.iter().take(3).enumerate() {
                let tr = Rect::new(inner.x + i as u16 * (tw + 2), ty, tw, th.min(need));
                tiles::thumb(cx, buf, tr, x, None);
            }
            // Beside the tiles, the link's numbers (what the chart shows, in figures).
            let rx = inner.x + n * (tw + 2) + 2;
            if rx + 28 <= inner.right() {
                link_stats(cx, buf, Rect::new(rx, ty, inner.right() - rx, th), e);
            }
        }
    }
}

/// A link's figures in a two-column table: rtt now / min / max / jitter, loss, network.
fn link_stats(cx: &Cx, buf: &mut Buffer, r: Rect, e: &LabEdge) {
    let v: Vec<f32> = e.rtt_hist.iter().flatten().copied().collect();
    let ms = |x: f32| super::ms_text(x);
    let mut rows: Vec<(&str, String)> = Vec::new();
    rows.push(("now", e.rtt_ms.map_or("down".into(), ms)));
    if !v.is_empty() {
        let lo = v.iter().copied().fold(f32::MAX, f32::min);
        let hi = v.iter().copied().fold(f32::MIN, f32::max);
        let mean = v.iter().sum::<f32>() / v.len() as f32;
        let jit =
            v.windows(2).map(|w| (w[1] - w[0]).abs()).sum::<f32>() / (v.len().max(2) - 1) as f32;
        rows.push(("min / max", format!("{} / {}", ms(lo), ms(hi))));
        rows.push(("mean", ms(mean)));
        rows.push(("jitter", ms(jit)));
    }
    let lost = e.rtt_hist.iter().filter(|x| x.is_none()).count();
    rows.push((
        "loss",
        format!(
            "{:.0}% ({lost} of {} probes)",
            e.loss * 100.0,
            e.rtt_hist.len()
        ),
    ));
    let (kind, tok) = edge_kind(cx, e);
    rows.push(("network", kind));
    if let EdgeKind::Mesh(m) = &e.kind {
        if let Some(m) = cx.s.meshes.iter().find(|x| &x.id == m) {
            rows.push(("", m.detail.clone()));
        }
    }
    zone(cx, buf, r.x, r.y, r.right(), "LINK", false);
    for (i, (k, v)) in rows.iter().enumerate() {
        let y = r.y + 2 + i as u16;
        if y >= r.bottom() {
            break;
        }
        cx.put(buf, r.x + 1, y, 11, k, cx.st(Tok::Muted));
        let st = if *k == "network" {
            cx.ink(tok, 0.95)
        } else {
            cx.ink(Tok::Fg, 0.95)
        };
        let room = r.right().saturating_sub(r.x + 13);
        cx.put(buf, r.x + 13, y, room, &fit(v, room as usize, cx.uni()), st);
    }
}

/// Greedy word wrap.
fn wrap(s: &str, w: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    for word in s.split(' ') {
        if !cur.is_empty() && cur.chars().count() + 1 + word.chars().count() > w {
            out.push(std::mem::take(&mut cur));
        }
        if !cur.is_empty() {
            cur.push(' ');
        }
        cur.push_str(word);
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

// ----------------------------------------------------------------------------- help

/// Keyboard map: each key shows its role, per layer; device names sit in a legend so no
/// keycap is cut mid-word.
fn help(cx: &Cx, buf: &mut Buffer, r: Rect) {
    let letters = ["qwertyuiop", "asdfghjkl;", "zxcvbnm,./"];
    // The widest keycaps whose staggered rows still fit (rows shift by a third of a key).
    let kw: u16 = (6..=9u16)
        .rev()
        .find(|k| k * 10 + 9 + 2 * (k / 3) + 4 <= r.width)
        .unwrap_or(0);
    let devs: Vec<&LabDevice> = cx.s.devices.iter().collect();
    let globals: [(&str, &str); 11] = [
        ("⏎", "open / attach"),
        ("spc", "peek"),
        ("-", "last session"),
        ("tab", "zone"),
        ("/", "find"),
        ("+", "invite"),
        ("K", "keys"),
        ("⌫", "stop"),
        ("`", "hub"),
        ("esc", "back"),
        ("ctrl \\", "detach"),
    ];
    let key = |k: &'static str| -> &'static str {
        if cx.uni() && !cx.touch() {
            k
        } else {
            match k {
                "⏎" => "ret",
                "␣" => "spc",
                "⌫" => "bksp",
                o => o,
            }
        }
    };
    if kw == 0 {
        // Phone: a compact list in the order you use it.
        let mut y = r.y + 1;
        let items: Vec<(&str, &str)> = vec![
            ("letter", "open that device"),
            ("then a s d", "attach its session"),
            ("then c x n", "new claude/codex/shell"),
            ("then f / e", "files / links"),
            ("then del", "stop a session"),
            ("ret / spc", "open / peek"),
            ("/", "find anything"),
            ("+ / K", "invite / keys"),
            ("esc / `", "back / hub"),
            ("ctrl \\", "detach"),
        ];
        for (k, v) in items {
            if y >= r.bottom() {
                break;
            }
            let k = if cx.uni() {
                k.to_string()
            } else {
                super::asciify(k).replace('>', "ret")
            };
            cx.put(buf, r.x + 1, y, 12, &format!(" {k} "), cx.key_style());
            cx.put(
                buf,
                r.x + 14,
                y,
                r.width.saturating_sub(15),
                v,
                cx.ink(Tok::Fg, 0.9),
            );
            y += 1;
        }
        return;
    }
    let total_w = kw * 10 + 9;
    let x0 = r.x + (r.width.saturating_sub(total_w + 3)) / 2;
    let right = x0 + total_w + 3;
    let key_rows = |y0: u16, buf: &mut Buffer, f: &dyn Fn(char) -> (String, Style)| {
        for (ri, row) in letters.iter().enumerate() {
            let y = y0 + ri as u16 * 3;
            let off = ri as u16 * (kw / 3).max(1);
            for (ci, c) in row.chars().enumerate() {
                let x = x0 + off + ci as u16 * (kw + 1);
                let (label, st) = f(c);
                let kr = Rect::new(x, y, kw, 2);
                let has = !label.is_empty();
                let bst = if has {
                    st
                } else if cx.has_surface() {
                    cx.surface()
                } else {
                    Style::default()
                };
                if cx.has_surface() || has {
                    cx.fill(buf, kr, bst);
                }
                let kst = if has {
                    bst.add_modifier(Modifier::BOLD)
                } else {
                    cx.ink(Tok::Muted, 0.6).patch(bst)
                };
                cx.put(buf, x + 1, y, 1, &c.to_string(), kst);
                if has {
                    cx.put(buf, x + 1, y + 1, kw - 1, &label, bst);
                }
            }
        }
    };
    // Centred vertically: the hub keyboard, its legend, the device keyboard (or its
    // one-line legend when short) and the global keys.
    let span = right - x0;
    let colw_d = devs
        .iter()
        .map(|d| 6 + super::width(&d.name, cx.uni()))
        .max()
        .unwrap_or(10);
    let leg_rows = (devs.len() as u16).div_ceil((span / colw_d).max(1));
    let colw_g = globals
        .iter()
        .map(|(k, v)| super::width(key(k), cx.uni()) + 5 + super::width(v, cx.uni()))
        .max()
        .unwrap_or(12);
    let glob_rows = (globals.len() as u16).div_ceil((span / colw_g).max(1));
    let base_h = 2 + 9 + leg_rows + 1 + glob_rows;
    let full_dev = base_h + 11 <= r.height.saturating_sub(1);
    let total = base_h + if full_dev { 11 } else { 2 };
    let mut y = r.y + 1 + r.height.saturating_sub(total + 1) / 2;
    zone(cx, buf, x0, y, right, "HUB", true);
    y += 2;
    key_rows(
        y,
        buf,
        &|c: char| match devs.iter().find(|d| d.hotkey == c) {
            Some(d) => {
                let (g, _) = cx.status(d.status);
                // Narrow keycaps keep the status alone; the legend names the device.
                let ev = if kw >= 8 {
                    format!("{g} {}", cx.meta(d))
                } else {
                    g.to_string()
                };
                (
                    fit(&ev, usize::from(kw - 1), cx.uni()),
                    cx.chip_style(cx.s.hue(&d.id)),
                )
            }
            None => match c {
                '/' => ("find".into(), cx.key_style()),
                // `e` then a letter: the link from this device to that one.
                'e' => ("link".into(), cx.key_style()),
                _ => (String::new(), Style::default()),
            },
        },
    );
    y += 9;
    // Legend: the full device names, in keycap colours, on a fixed column grid.
    let colw = colw_d;
    let per = (span / colw).max(1);
    for (i, d) in devs.iter().enumerate() {
        let (ci, ri) = (i as u16 % per, i as u16 / per);
        let x = x0 + ci * (span / per);
        let w = cx.dchip(buf, x, y + ri, d, false) + 1;
        cx.put(buf, x + w, y + ri, colw - w, &d.name, cx.ink(Tok::Fg, 0.85));
    }
    y += (devs.len() as u16).div_ceil(per) + 1;
    if !full_dev {
        // Short screens: the device layer as one line of chips.
        let lx = cx.label(buf, x0, y, 14, "IN A DEVICE", false) + x0 + 2;
        cx.chips(
            buf,
            lx,
            y,
            right,
            &[
                ("a s d", "attach"),
                ("c x n", "new"),
                ("f", "files"),
                ("e", "link"),
                (key("⌫"), "stop"),
            ]
            .map(|(k, l)| (k, l, "help")),
        );
        y += 2;
    } else {
        zone(cx, buf, x0, y, right, "IN A DEVICE", false);
        y += 2;
        let ks = cx.key_style();
        let acc = if cx.has_surface() {
            cx.wash(Tok::Accent, 0.25).patch(cx.ink(Tok::Fg, 1.0))
        } else {
            ks
        };
        let tint = |t: Tok| {
            if cx.has_surface() {
                cx.wash(t, 0.25).patch(cx.ink(Tok::Fg, 1.0))
            } else {
                ks
            }
        };
        key_rows(y, buf, &|c: char| {
            if SLOTS.contains(&c) {
                return ("attach".into(), acc);
            }
            match c {
                'n' => ("+shell".into(), ks),
                'f' => ("files".into(), ks),
                'e' => ("link".into(), ks),
                'r' => ("retry".into(), ks),
                // Motion, as in files and every list (never a slot).
                'j' => ("down".into(), ks),
                'k' => ("up".into(), ks),
                'c' => ("+claude".into(), tint(Tok::Claude)),
                'x' => ("+codex".into(), tint(Tok::Codex)),
                '/' => ("find".into(), ks),
                _ => (String::new(), Style::default()),
            }
        });
        y += 9;
    }
    // Keys that mean the same everywhere, on the same kind of grid.
    let colw = colw_g;
    let per = (span / colw).max(1);
    for (i, (k, v)) in globals.iter().enumerate() {
        let (ci, ri) = (i as u16 % per, i as u16 / per);
        let (x, yy) = (x0 + ci * (span / per), y + ri);
        if yy >= r.bottom() {
            break;
        }
        let k = key(k);
        let w = cx.put(buf, x, yy, 10, &format!(" {k} "), cx.key_style()) + 1;
        cx.put(buf, x + w, yy, colw - w, v, cx.ink(Tok::Fg, 0.85));
    }
    let _ = (DeviceStatus::Online, centered, Access::Key);
}
