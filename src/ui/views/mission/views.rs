//! Screens: hub, device, session peek, files, preview, palette, keys, onboarding, join,
//! link policy and help. Each is a plain function over the shared `Cx`.
use super::{fit, fmt, inset, orbit, tiles, Cx, Size, SLOTS};
use crate::ui::views::{
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
            // (The first slot alone on phones, so the chip reads the same at every size.)
            let k = if cx.touch() { 1 } else { n.clamp(1, 3) };
            let slots: String = std::iter::once(cx.ret().to_string())
                .chain(SLOTS[..k].iter().map(|c| c.to_string()))
                .collect::<Vec<_>>()
                .join(" ");
            let mut items: Keys = Vec::new();
            // A device that is down or answering late offers its retry verb.
            let retry = d.is_some_and(|d| {
                matches!(
                    d.status,
                    DeviceStatus::Down | DeviceStatus::Stale | DeviceStatus::Unknown
                )
            });
            if cx.touch() {
                // The primary verb first, then the way back; tiles also attach on a tap.
                if n > 0 {
                    items.push((slots.as_str(), "attach", "attach"));
                }
                if retry {
                    items.push(("r", "retry", "retry"));
                }
                items.extend([
                    ("esc", "back", "back"),
                    ("f", "files", "files"),
                    ("e", "link", "links"),
                    ("`", "hub", "hub"),
                ]);
                // The destructive verb goes last, never first on a row.
                if n > 0 {
                    items.push(("⌫", "stop", "stop"));
                }
            } else {
                // The way back and to the hub stay in view at 80 columns; peek (a tap
                // on the tile peeks too) and find give way first.
                if n > 0 {
                    items.push((slots.as_str(), "attach", "attach"));
                    if cx.size() >= Size::M {
                        items.push(("␣", "peek", "peek"));
                    }
                }
                if retry {
                    items.push(("r", "retry", "retry"));
                }
                items.extend([("f", "files", "files"), ("e", "link", "links")]);
                if n > 0 {
                    items.push(("⌫", "stop", "stop"));
                }
                items.extend([
                    ("esc", "back", "back"),
                    ("`", "hub", "hub"),
                    ("-", "last", "last"),
                    ("/", "find", "palette"),
                ]);
            }
            // `⌫` asks inline (`CX_LAB_FOCUS=stop`): a second `⌫` stops, anything else
            // keeps the session. Never `y`, which answers the agent's own approvals.
            if cx.c.focus == Some(Pane::Custom("stop")) && n > 0 {
                items = vec![("⌫", "stop it", "stop"), ("esc", "keep", "back")];
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
                        ("`", "hub", "hub"),
                        ("⌫", "stop", "stop"),
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
                    } else if !cx.touch() {
                        // 80 columns: the motion keys first, as at M; the preview is `spc`.
                        vec![
                            ("⏎", "open", "open"),
                            ("j k", "move", "move"),
                            ("l h", "fold", "fold"),
                            ("s", "send", "send"),
                            ("␣", "preview", "preview"),
                            ("/", "find", "palette"),
                            ("esc", "back", "back"),
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
            top(cx, buf, &[Crumb::Text("secrets".into())], true);
            secrets(cx, buf, body);
            let keys = match &cx.s.secret_entry {
                Some(e) => vec![
                    (cx.ret(), if e.naming { "next" } else { "save" }, "go"),
                    ("esc", "cancel", "back"),
                ],
                None => vec![
                    ("a s d g", "set", "edit"),
                    ("h", "new", "secret-new"),
                    ("esc", "back", "back"),
                ],
            };
            footer(cx, buf, foot, keys);
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
                    ("+", "join code", "join"),
                    ("esc", "plain ssh", "skip"),
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
                    ("esc", "done", "back"),
                    ("y", "copy", "copy-code"),
                    // Shifted: a new code revokes one that may already be shared.
                    ("R", "new code", "new-code"),
                ],
            );
        }
        LabView::EdgeEditor(id) => {
            let e = cx.s.edge(id).or(cx.s.edges.first());
            let ends = e.and_then(|e| Some((cx.s.device(&e.from)?, cx.s.device(&e.to)?)));
            let crumbs: Vec<Crumb> = ends.map(|(a, b)| Crumb::Link(a, b)).into_iter().collect();
            top(cx, buf, &crumbs, true);
            edge_editor(cx, buf, body, id);
            // The lanes above name their keys; the footer keeps the rest.
            let mut items: Keys = vec![("u", "undo", "undo"), ("esc", "done", "back")];
            if cx.size() >= Size::M {
                items.insert(0, ("j k", "lane", "lane"));
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
                x += cx.put(buf, x, y, 4, cx.g(" ⇄ ", " <> "), cx.ink(Tok::Fg, 0.8));
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
            crate::ui::views::NoticeKind::Err => Tok::Err,
            crate::ui::views::NoticeKind::Warn => Tok::Warn,
            crate::ui::views::NoticeKind::Ok => Tok::Ok,
            crate::ui::views::NoticeKind::Info => Tok::Info,
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
                cx.g("⇣", "dl"),
                t.path.rsplit('/').next().unwrap_or(""),
                pct * 100.0,
                fmt::bytes(rate as u64)
            )
        } else {
            format!("{} {:.0}%", cx.g("⇣", "dl"), pct * 100.0)
        };
        parts.push((text, cx.st(Tok::Info), "transfers"));
    }
    let dot = if down == 0 {
        cx.st(Tok::Ok)
    } else {
        cx.st(Tok::Muted)
    };
    parts.push((
        if cx.uni() {
            format!("● {up}/{}", s.devices.len())
        } else {
            format!("{up}/{} up", s.devices.len())
        },
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
                "⏎" => cx.ret(),
                "⌫" => "bksp",
                "↑↓" if !cx.uni() => "up/dn",
                other => other,
            };
            // Labels stay whole (a phone footer wraps to a second row instead), so a
            // chip never loses the word that gives it meaning (`esc plain ssh`).
            (k, l, a)
        })
        .collect();
    let (x0, right) = (r.x + 1, r.right().saturating_sub(1));
    // Two columns between chips at every width, so a label never butts against the
    // next keycap.
    let sep = 2;
    // How many of `items` fit one row.
    let fits = |items: &[(&str, &str, &str)]| {
        let mut x = x0;
        let mut n = 0;
        for (k, l, _) in items {
            let lw = super::width(l, cx.uni());
            let need = super::width(k, cx.uni()) + 2 + if lw > 0 { lw + 1 } else { 0 };
            if x + need > right {
                break;
            }
            x += need + sep;
            n += 1;
        }
        n
    };
    let fit_n = fits(&items);
    if cx.touch() && fit_n < items.len() && r.y > cx.area.y + 1 {
        // Read top to bottom in priority order: the primary verb leads the upper row.
        // The rows are balanced (no lone chip below) when the rest still fits.
        let half = items.len().div_ceil(2).min(fit_n);
        let fit_n = if fits(&items[half..]) == items.len() - half {
            half
        } else {
            fit_n
        };
        cx.chips_h(buf, x0, r.y - 1, right, &items[..fit_n], 1);
        cx.chips_h(buf, x0, r.y, right, &items[fit_n..], 1);
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
        // Phones tap nodes and rows directly; zone keys stay keyboard-only. `-` stays:
        // it is the cheapest re-attach.
        v.retain(|(k, _, _)| !matches!(*k, "tab" | "e"));
    }
    v.extend([
        ("/", "find", "palette"),
        ("+", "invite", "invite"),
        ("K", "secrets", "secrets"),
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
    // One row keeps a ghost cell for a new session; with two, the last row's tiles
    // widen to fill it instead (the vitals strip above already offers `new`).
    let n = if rows > 1 {
        shown
    } else {
        cells.min(list.len() + 1)
    };
    for i in 0..n {
        let (ci, ri) = (i as u16 % cols, i as u16 / cols);
        let in_row = (n - usize::from(ri * cols)).min(usize::from(cols)) as u16;
        let tw = if in_row < cols {
            (r.width + gap) / in_row - gap
        } else {
            tw
        };
        let tr = Rect::new(r.x + ci * (tw + gap), r.y + ri * (th + 1), tw, th);
        match list.get(i) {
            Some(x) => {
                let pick = (sel.0 % list.len() == i).then_some(sel.1);
                tiles::thumb(cx, buf, tr, x, pick);
            }
            None => new_tile(cx, buf, tr, ghost, false, true),
        }
        if !cx.has_surface() && ci + 1 < in_row && i + 1 < n {
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
    } else if cx.touch() {
        // Phones list one row per link.
        links_n
    } else {
        (links_n.div_ceil(2) * 3).max(5)
    };
    let sel = cx.c.selected % (list.len() + 1);
    // The ghost is one row under the tiles (`new c claude x codex n shell`), so the
    // sessions take the full width and no half-empty card floats beside them.
    let used = if sz == Size::Xs {
        // The links keep a row each plus their zone rule; the rest goes to the tiles.
        let reserve = if links_n > 0 { links_n + 1 } else { 0 };
        device_sessions_rows(cx, buf, main, &list, reserve)
    } else {
        let tiles_n = (list.len() as u16).max(1);
        let cols = if sz == Size::S { 2 } else { 3 }.min(tiles_n);
        let gap = 2u16;
        let sw = (main.width + gap) / cols - gap;
        let rows = tiles_n.div_ceil(cols);
        let room = main
            .height
            .saturating_sub(if fan_want > 0 { fan_want + 4 } else { 0 })
            .saturating_sub(2);
        // The tiles are as tall as their screens' text (a taller tile only adds blank
        // rows); the rows they leave go to the LINKS star below.
        let text_h = list
            .iter()
            .map(|x| tiles::used_lines(x) as u16 + 2)
            .max()
            .unwrap_or(5);
        let mut th = ((room + 1) / rows.max(1))
            .saturating_sub(1)
            .min(text_h)
            .max(5);
        // The rows the star does not draw go back to the tiles (a screen taller than
        // its text is still a screen), so no blank band opens around LINKS.
        if fan_want > 0 {
            let left = main.height.saturating_sub(rows * (th + 1) + 2 + 4);
            let band = Rect::new(r.x, r.y, r.width, left);
            let star = orbit::height(cx, band, &d.id, 1, orbit::Sel::Root);
            th += left.saturating_sub(star) / rows.max(1);
        }
        let mut bottom = main.y;
        for i in 0..tiles_n as usize {
            let (ci, ri) = (i as u16 % cols, i as u16 / cols);
            let x0 = main.x + ci * (sw + gap);
            let tr = Rect::new(x0, main.y + ri * (th + 1), sw, th);
            if tr.bottom() > main.bottom() {
                break;
            }
            bottom = tr.bottom() + 1;
            match list.get(i) {
                Some(x) => {
                    tiles::thumb(cx, buf, tr, x, (i == sel).then_some(true));
                    if i == sel && cx.c.focus == Some(Pane::Custom("stop")) {
                        stop_confirm(cx, buf, tr, x);
                    }
                }
                None => new_tile(cx, buf, tr, Some(d), list.is_empty(), false),
            }
            if !cx.has_surface() && ci + 1 < cols && i + 1 < tiles_n as usize {
                cx.vsep(buf, tr.right(), tr.y, th);
            }
        }
        if !list.is_empty() && bottom < main.bottom() {
            new_row(cx, buf, main.x, bottom, main.right());
            bottom += 2;
        }
        bottom - main.y
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
    let fy = main.y + used;
    let edges: Vec<&LabEdge> = s.edges_of(&d.id).collect();
    if cx.touch() && links_n > 0 && fy + 2 <= r.bottom() {
        // Phones: one plain row per link (a star of bare letters would read as keys).
        zone(cx, buf, main.x, fy, right, "LINKS", false);
        cx.hit(Rect::new(main.x, fy, 12, 1), "links");
        let rows = Rect::new(
            main.x,
            fy + 1,
            main.width,
            r.bottom().saturating_sub(fy + 1),
        );
        link_list(cx, buf, rows, d, &edges);
    } else if links_n > 0 && fy + 7 <= r.bottom() {
        // The footer says `e links`; the zone label is the tap target.
        zone(cx, buf, main.x, fy, right, "LINKS", false);
        cx.hit(Rect::new(main.x, fy, 12, 1), "links");
        // The star alone: each link's numbers are one `e` away (the edge editor). It
        // sits centred in the rows left under the label, at the hub's spacing.
        let band = Rect::new(r.x, fy + 2, r.width, r.bottom().saturating_sub(fy + 3));
        orbit::draw_with(cx, buf, band, &d.id, 1, orbit::Sel::Root, false);
    }
}

/// Phone links: the far end's name, its policy seen from `d` and the rtt (or why not),
/// on fixed columns sized to the longest name; the loss goes first when rows are short.
fn link_list(cx: &Cx, buf: &mut Buffer, r: Rect, d: &LabDevice, edges: &[&LabEdge]) {
    let other = |e: &LabEdge| {
        let id = if e.from == d.id { &e.to } else { &e.from };
        cx.s.device(id)
    };
    let name_w = edges
        .iter()
        .filter_map(|e| other(e))
        .map(|o| super::width(&o.name, cx.uni()))
        .max()
        .unwrap_or(0);
    // A link closed both ways already says `off` in its policy column: its tail is just
    // the last value seen, so the names keep their room.
    let tail = |e: &LabEdge, loss: bool| {
        let t = link_tail(cx, e, loss);
        match t.split_once("last ") {
            Some((_, ms)) => format!("last {ms}"),
            None => t,
        }
    };
    let tail_w = |loss: bool| {
        edges
            .iter()
            .map(|e| super::width(&tail(e, loss), cx.uni()))
            .max()
            .unwrap_or(0)
    };
    // name, policy (up to 7), then the evidence right-aligned.
    let loss = name_w + 1 + 7 + 2 + tail_w(true) < r.width;
    let room = r.width.saturating_sub(tail_w(loss) + 11).min(name_w);
    for (i, e) in edges.iter().enumerate() {
        let y = r.y + i as u16;
        if y >= r.bottom() {
            break;
        }
        let Some(o) = other(e) else {
            continue;
        };
        let pol = orbit::policy(cx, e, e.from == d.id, false);
        let x = r.x + 1;
        cx.put(
            buf,
            x,
            y,
            room,
            &fit(&o.name, room as usize, cx.uni()),
            cx.ink(Tok::Fg, 0.9),
        );
        cx.put(buf, x + room + 1, y, 7, &pol, cx.st(Tok::Muted));
        cx.put_r(buf, r.right(), y, &tail(e, loss), cx.st(Tok::Muted));
        cx.hit(Rect::new(r.x, y, r.width, 1), format!("edge:{}", e.id));
    }
}

/// A link's evidence: its rtt and loss, why there is none (closed, no reply), or the
/// last value seen.
fn link_tail(cx: &Cx, e: &LabEdge, loss: bool) -> String {
    let last = e.rtt_hist.iter().rev().flatten().next().copied();
    let closed = e.forward == Access::Denied && e.reverse == Access::Denied;
    match (e.up, closed, e.rtt_ms) {
        (true, true, _) => match last {
            Some(ms) => format!("closed {} last {}", cx.g("·", "-"), super::ms_text(ms)),
            None => "closed".into(),
        },
        (true, false, Some(ms)) if loss => {
            format!("{}  loss {:.0}%", super::ms_text(ms), e.loss * 100.0)
        }
        (true, false, Some(ms)) => super::ms_text(ms),
        (true, false, None) => "not measured".into(),
        _ => "no reply".into(),
    }
}

fn device_header(cx: &Cx, buf: &mut Buffer, r: Rect, d: &LabDevice) {
    let (g, gst) = cx.status(d.status);
    // (No os word when it is not known.)
    let info = format!("{} · {}@{} · {}", d.os, d.account, d.host, kind_word(d));
    let info = info.trim_start_matches(" · ").to_string();
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
                &format!("{}{}  ", tiles::mesh_glyph(cx), m.name),
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
        // No live figures: a down device says why; an up one says how it is known.
        if r.height > 3 && !cx.up(d) {
            cx.put(
                buf,
                r.x,
                r.y + 3,
                r.width,
                &format!("{} {}", cx.g("✕", "!"), d.evidence),
                cx.st(Tok::Err),
            );
        } else if r.height > 3 && d.id == cx.s.viewer {
            cx.put(buf, r.x, r.y + 3, r.width, &d.evidence, cx.st(Tok::Muted));
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
        // The chart stops at 72 columns (a wider one only stretches its bars); the
        // block then hugs the right edge.
        let cw = mw.saturating_sub(gw + 3).min(72);
        let mx = mx + mw.saturating_sub(cw + gw + 3);
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
        // A short trend, not a run across the row.
        let w = mw.saturating_sub(9 + 3 + 8).min(24);
        let v: Vec<Option<f32>> = m.cpu_hist.iter().map(|v| Some(*v)).collect();
        tiles::spark(cx, buf, mx + 9, my, w, &v, cx.spark_heat(m.cpu));
        let rest = format!("mem {:.0}%", m.mem * 100.0);
        cx.put(buf, mx + 9 + w + 3, my, 10, &rest, cx.st(Tok::Info));
    }
}

fn kind_word(d: &LabDevice) -> &'static str {
    use crate::ui::views::DeviceKind::*;
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
/// (`i c`), because hub letters are devices.
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
    // The cell's full height, so the grid keeps one rhythm; the content sits centred.
    cx.fill(
        buf,
        r,
        if picked {
            cx.sel_bg()
        } else {
            Style::default()
        },
    );
    let faint = if picked {
        cx.st(Tok::Accent)
    } else {
        cx.ink(Tok::Faint, 1.0)
    };
    let (h, v) = (cx.g("╌", "-"), cx.g("╎", " "));
    let w = r.width.saturating_sub(2) as usize;
    let (tl, tr, bl, br) = if cx.uni() {
        ("╭", "╮", "╰", "╯")
    } else {
        ("-", "-", "-", "-")
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
        // On the hub the option is an address: the device letter in its hue, then the verb.
        let key = format!(" {k} ");
        let kw = 3 + if hub { 3 } else { 0 };
        if y >= r.bottom() - 1 || (!stacked && ox + kw + 8 > r.right()) {
            break;
        }
        let x0 = ox;
        if hub {
            ox += cx.dchip(buf, ox, y, d, false);
        }
        ox += cx.put(buf, ox, y, 3, &key, cx.key_style());
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
    reserve: u16,
) -> u16 {
    let mut y = r.y;
    let sel = cx.c.selected % (list.len() + 1);
    // Rows the two-row entries, the ghost row and the links leave go to the selected
    // session as a thumbnail (up to its screen tail), so a prompt is recognisable.
    let spare = r.height.saturating_sub(list.len() as u16 * 3 + 1 + reserve);
    for (i, x) in list.iter().enumerate() {
        if y + 1 >= r.bottom() {
            break;
        }
        let h = if i == sel && spare >= 2 {
            (2 + spare).min(tiles::tail_rows(x) as u16 + 2)
        } else {
            2
        };
        let row = Rect::new(r.x, y, r.width, h);
        if h > 2 {
            tiles::thumb(cx, buf, row, x, Some(true));
        } else {
            tiles::session_row(cx, buf, row, x, (i == sel).then_some(true));
        }
        if i == sel && cx.c.focus == Some(Pane::Custom("stop")) {
            stop_confirm(cx, buf, row, x);
        }
        y += h + 1;
    }
    if y < r.bottom() {
        new_row(cx, buf, r.x, y, r.right());
        // A blank row keeps the links' zone off the buttons, unless it would cost a link.
        y += 1 + u16::from(r.bottom().saturating_sub(y + 1) > reserve);
    }
    y - r.y
}

/// The inline stop confirm over a tile's last row: `stop refactor transport?  ⌫ stop
/// any other key keeps it`, warn-tinted. The default is no.
fn stop_confirm(cx: &Cx, buf: &mut Buffer, tr: Rect, x: &LabSession) {
    let y = tr.bottom().saturating_sub(1);
    let row = Rect::new(tr.x + 1, y, tr.width.saturating_sub(1), 1);
    let bg = cx.wash(Tok::Err, 0.22);
    cx.fill(buf, row, bg);
    let del = "bksp";
    let keys = format!(" {del} ");
    let tail = "again to stop, any key keeps it";
    let kw = super::width(&keys, cx.uni());
    let mut xx = row.x + 1;
    let q = format!("stop {}? ", x.title);
    let room = row
        .width
        .saturating_sub(kw + 3 + super::width(tail, cx.uni()).min(16));
    xx += cx.put(
        buf,
        xx,
        y,
        room,
        &fit(&q, room as usize, cx.uni()),
        cx.st(Tok::Err).add_modifier(Modifier::BOLD).patch(bg),
    );
    let k0 = xx;
    xx += cx.put(buf, xx, y, kw, &keys, cx.key_style()) + 1;
    cx.hit(Rect::new(k0, y, kw, 1), "stop");
    cx.put(
        buf,
        xx,
        y,
        row.right().saturating_sub(xx + 1),
        &fit(tail, row.right().saturating_sub(xx + 1) as usize, cx.uni()),
        cx.ink(Tok::Fg, 0.85).patch(bg),
    );
}

/// The ghost tile's one-row form: `new  c claude  x codex  n shell`.
fn new_row(cx: &Cx, buf: &mut Buffer, x: u16, y: u16, right: u16) {
    let items = [
        ("c", "claude", "new-claude"),
        ("x", "codex", "new-codex"),
        ("n", "shell", "new-shell"),
    ];
    let x0 = x + 1 + cx.put(buf, x + 1, y, 4, "new ", cx.ink(Tok::Muted, 0.9));
    cx.chips(buf, x0, y, right, &items);
}

fn edge_kind(cx: &Cx, e: &LabEdge) -> (String, Tok) {
    match &e.kind {
        EdgeKind::Mesh(m) => {
            let i = cx.s.meshes.iter().position(|x| &x.id == m).unwrap_or(0);
            (
                format!("{}{m}", tiles::mesh_glyph(cx)),
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
    // (Uptime and clients only when known: an unobserved session says nothing.)
    let meta = if x.age_secs == 0 && x.clients == 0 {
        x.cwd.clone()
    } else {
        format!(
            "{} · up {} · {} client{}",
            x.cwd,
            fmt::age(x.age_secs),
            x.clients,
            if x.clients == 1 { "" } else { "s" }
        )
    };
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
    let lines = crate::ui::views::ansi_lines(&x.screen, x.screen_cols);
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
    // The screen itself: the app scrolls it and types into it on a tap.
    cx.hit(inner, "peek-screen");
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
    let lines = crate::ui::views::ansi_lines(&x.screen, x.screen_cols);
    let h = a.height.saturating_sub(1);
    let used = lines
        .iter()
        .rposition(|l| l.spans.iter().any(|s| !s.content.trim().is_empty()))
        .map_or(1, |i| i + 1) as u16;
    // The pane is as wide as the screen: text reflows at word boundaries as it would
    // in a pane that size, and a drawn box (an agent's composer) is redrawn at the
    // pane's width, its right edge kept.
    let w = usize::from(a.width.max(1));
    let mut rows: Vec<Vec<(char, Style)>> = Vec::new();
    for l in lines.iter().take(usize::from(used)) {
        let mut cells: Vec<(char, Style)> = Vec::new();
        for sp in &l.spans {
            let st = tiles::degrade(cx, sp.style);
            cells.extend(sp.content.chars().map(|c| (c, st)));
        }
        // Trailing padding wraps into blank rows; a tinted run (a diff row) keeps it
        // while it fits.
        let long = cells.len() > w;
        while cells
            .last()
            .is_some_and(|(c, st)| *c == ' ' && (long || st.bg.is_none()))
        {
            cells.pop();
        }
        let boxed = cells.len() > w
            && cells.first().is_some_and(|c| "╭╰│".contains(c.0))
            && cells.last().is_some_and(|c| "╮╯│".contains(c.0));
        if boxed {
            let last = cells[cells.len() - 1];
            cells.truncate(w - 1);
            cells.push(last);
        }
        // Continuation rows hang at the line's content column (after its indent and a
        // `⎿` / `+` / `●` marker), as the agent wraps them, the indent keeping the
        // first row's tint so a diff gutter stays put.
        let indent = cells
            .iter()
            .position(|(c, _)| !(c.is_whitespace() || "⎿●+-*>✻•".contains(*c)))
            .unwrap_or(0)
            .min(w / 2);
        // ASCII after the box is redrawn, so the box keeps the pane's width.
        if !cx.uni() {
            cells = cells
                .into_iter()
                .flat_map(|(c, st)| {
                    let a = super::asciify(&c.to_string());
                    a.chars().map(move |c| (c, st)).collect::<Vec<_>>()
                })
                .collect();
            if boxed {
                cells.truncate(w);
            }
        }
        let pad: Vec<(char, Style)> = cells[..indent.min(cells.len())]
            .iter()
            .map(|&(_, st)| (' ', st))
            .collect();
        while cells.len() > w {
            // Break at the last space in the row (a word longer than the row is cut).
            let at = cells[..=w]
                .iter()
                .rposition(|c| c.0 == ' ')
                .filter(|&i| i > indent + (w - indent) / 3)
                .unwrap_or(w);
            let rest = cells.split_off(at);
            rows.push(cells);
            cells = pad
                .iter()
                .copied()
                .chain(rest.into_iter().skip_while(|c| c.0 == ' '))
                .collect();
        }
        rows.push(cells);
    }
    let skip = rows.len().saturating_sub(usize::from(h));
    for (i, row) in rows.iter().skip(skip).enumerate() {
        for (j, (c, st)) in row.iter().enumerate() {
            cx.put(
                buf,
                a.x + j as u16,
                a.y + 1 + i as u16,
                1,
                &c.to_string(),
                *st,
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
    // On phones (touch, under 70 columns) `⏏` is dropped (phone fonts draw it as a
    // wide emoji) and the chord shortens, so the title keeps its room.
    let chip = if cx.touch() {
        " ctrl+\\ "
    } else if cx.uni() {
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
    // `cx › innovation › i a refactor transport`: the session's address sits before
    // its title exactly as on its tile (one address everywhere); phones drop the name.
    if let Some(d) = cx.s.device(&x.device).filter(|_| a.width >= 60) {
        let x0 = bx;
        bx += cx.put(buf, bx, y, 20, &d.name, cx.ink(Tok::Fg, 0.9).patch(bar));
        cx.hit(Rect::new(x0, y, bx - x0, 1), format!("device:{}", d.id));
        bx += cx.put(buf, bx, y, 3, sep, cx.ink(Tok::Faint, 1.0).patch(bar));
    }
    bx += tiles::addr(cx, buf, bx, y, x) + 1;
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
    bx += cx.put(buf, bx, y, sw, &state, sst.patch(bar));
    // Wide screens keep one live fact on the bar: how long it has run, who else is in.
    if a.width >= 120 && (x.age_secs > 0 || x.clients > 0) {
        let fact = format!(
            "up {} {} {} client{}",
            fmt::age(x.age_secs),
            cx.g("·", "-"),
            x.clients,
            if x.clients == 1 { "" } else { "s" }
        );
        let fw = super::width(&fact, cx.uni());
        if bx + fw + 4 < dx {
            cx.put_r(buf, dx - 2, y, &fact, cx.st(Tok::Muted).patch(bar));
        }
    }
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
    // Phones keep the spacer row above their wrapped footer.
    let list_h = r.height.saturating_sub(1 + u16::from(cx.touch()));
    let list = Rect::new(r.x + 1, r.y + 1, list_w, list_h);
    let f = &cx.s.files;
    let max = f.entries.iter().map(|e| e.size).max().unwrap_or(1).max(1);
    let sel = f.selected;
    cx.label(buf, list.x, r.y, list_w, "FILES", true);
    cx.put_r(
        buf,
        list.right(),
        r.y,
        &format!("{} items", f.total.max(f.offset + f.entries.len())),
        cx.st(Tok::Muted),
    );
    let mut rows = list.height as usize;
    let first = sel.saturating_sub(rows.saturating_sub(3));
    // A cut tree says so on its last row (`+1 more ↓`), never silently.
    // (A window of a long listing: the rows after it count as more.)
    let total = f.total.max(f.offset + f.entries.len()) - f.offset;
    let cut = total.saturating_sub(first) > rows;
    if cut {
        rows -= 1;
        let more = total - first - rows;
        let t = format!("+{more} more{}", cx.g(" ↓", ""));
        cx.put(
            buf,
            list.x + 1,
            list.y + rows as u16,
            list.width,
            &t,
            cx.st(Tok::Muted),
        );
    }
    for (i, e) in f.entries.iter().enumerate().skip(first).take(rows) {
        let y = list.y + (i - first) as u16;
        let picked = i == sel;
        let row = Rect::new(list.x, y, list.width, 1);
        if picked {
            cx.fill(buf, row, cx.sel_bg());
            cx.put(buf, list.x, y, 1, cx.g("▌", ">"), cx.st(Tok::Accent));
        }
        let mut x = list.x + 1;
        // Indent guides.
        for _ in 0..e.depth {
            // ASCII: indentation alone (`:` is the unfocused selection bar).
            x += cx.put(buf, x, y, 2, cx.g("│ ", "  "), cx.ink(Tok::Faint, 1.0));
        }
        let (icon, nst) = match e.kind {
            FileKind::Dir => (
                if e.expanded {
                    cx.g("▾ ", "v ")
                } else {
                    cx.g("▸ ", "  ")
                },
                cx.st(Tok::Dir).add_modifier(Modifier::BOLD),
            ),
            // (ASCII `*` is a live session.)
            FileKind::Symlink => (cx.g("→ ", "& "), cx.st(Tok::Info)),
            FileKind::File => ("  ", cx.ink(Tok::Fg, 0.95)),
        };
        let nst = if picked {
            cx.st(Tok::Accent).add_modifier(Modifier::BOLD)
        } else {
            nst
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
                // Untracked is `U` (`?` means unknown and help).
                &if gc == '?' { 'U' } else { gc }.to_string(),
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
    let left = list.bottom().saturating_sub(dy);
    if let Some(e) = f.entries.get(sel).filter(|_| left >= 2) {
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
        let sep = cx.g("·", "-");
        let size_age = format!(
            "{} {sep} modified {} ago",
            fmt::bytes(e.size),
            fmt::age(e.modified_secs)
        );
        let w = list.width.saturating_sub(1);
        // Five rows: a SELECTED zone with one fact per row; fewer: the path, then the
        // facts and the send targets on one row.
        let (facts, sy) = if left >= 5 {
            zone(cx, buf, list.x, dy, list.right(), "SELECTED", false);
            (vec![path, size_age, format!("git: {git}")], dy + 6)
        } else {
            (vec![path], dy + 1)
        };
        let fy = if left >= 5 { dy + 2 } else { dy };
        for (i, t) in facts.iter().enumerate() {
            cx.put(
                buf,
                list.x + 1,
                fy + i as u16,
                w,
                &fit(t, w as usize, cx.uni()),
                cx.ink(Tok::Fg, 0.85),
            );
        }
        // Reachable send targets (closed links skipped), with their route's rtt.
        let targets: Vec<(&LabDevice, Vec<&LabEdge>)> =
            cx.s.devices
                .iter()
                .filter(|d| d.id != f.device && cx.up(d))
                .map(|d| (d, orbit::route_from(cx, &f.device, &d.id)))
                .filter(|(_, r)| !r.is_empty())
                .collect();
        if left >= 7 + targets.len() as u16 {
            // Room for one target per row: `s send to`, then each with its rtt and relay.
            let mut y = sy;
            let mut x = list.x + 1;
            x += cx.put(buf, x, y, 3, " s ", cx.key_style()) + 1;
            cx.put(buf, x, y, 9, "send to", cx.st(Tok::Muted));
            y += 1;
            for (d, route) in &targets {
                let st = cx
                    .ink(
                        Tok::Id(cx.s.hue(&d.id)),
                        if route.len() == 1 { 1.0 } else { 0.7 },
                    )
                    .add_modifier(Modifier::BOLD);
                let x = list.x + 3;
                cx.put(buf, x, y, 1, &d.hotkey.to_string(), st);
                cx.put(
                    buf,
                    x + 2,
                    y,
                    18,
                    &fit(&d.name, 18, cx.uni()),
                    cx.ink(Tok::Fg, 0.85),
                );
                // (No figure when a hop's rtt is not measured.)
                let ms: Option<f32> = route.iter().map(|e| e.rtt_ms).sum();
                let via = if route.len() > 1 {
                    let hop = if route[0].from == f.device {
                        &route[0].to
                    } else {
                        &route[0].from
                    };
                    cx.s.device(hop)
                        .map(|h| format!("via {}  ", h.name))
                        .unwrap_or_default()
                } else {
                    String::new()
                };
                cx.put_r(
                    buf,
                    list.right(),
                    y,
                    &format!("{via}{}", ms.map(super::ms_text).unwrap_or_default()),
                    cx.st(Tok::Muted),
                );
                cx.hit(
                    Rect::new(list.x, y, list.width, 1),
                    format!("send:{}", d.id),
                );
                y += 1;
            }
        } else if sy < list.bottom() {
            let mut x = list.x + 1;
            if left < 5 {
                let t = format!(
                    "{} {sep} {} ago  ",
                    fmt::bytes(e.size),
                    fmt::age(e.modified_secs)
                );
                x += cx.put(buf, x, sy, w, &t, cx.st(Tok::Muted));
            }
            // `s` is the key; the letters after it are what `s` then takes, drawn as
            // plain coloured text (here they are not keys on their own).
            x += cx.put(buf, x, sy, 3, " s ", cx.key_style()) + 1;
            x += cx.put(buf, x, sy, 9, "send to ", cx.st(Tok::Muted));
            // Only devices a transfer can reach from here (closed links are skipped);
            // one reached through a relay is drawn in its hue but not bold.
            for d in cx.s.devices.iter().filter(|d| d.id != f.device && cx.up(d)) {
                let hops = orbit::route_from(cx, &f.device, &d.id).len();
                if hops == 0 {
                    continue;
                }
                if x + 2 > list.right() {
                    break;
                }
                let st = cx.ink(Tok::Id(cx.s.hue(&d.id)), if hops == 1 { 1.0 } else { 0.7 });
                let st = if hops == 1 {
                    st.add_modifier(Modifier::BOLD)
                } else {
                    st
                };
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
    cx.label(buf, r.x, r.y, r.width, "PREVIEW", false);
    let mut meta = format!("{} · {} · {} lines", p.path, lang, p.lines);
    if let Some(note) = &p.note {
        meta = format!("{meta} · {note}");
    }
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
        r.height
            .saturating_sub(1)
            .min(p.lines.min(u16::MAX as usize - 1) as u16 + 1),
    );
    // The preview surface: a tap target, and where the app draws its real preview.
    cx.hit(body, "preview-body");
    if cx.has_surface() {
        cx.fill(buf, body, cx.surface());
    }
    let gw = 5u16;
    let mut in_code = false;
    for (i, l) in p.text.lines().take(body.height as usize).enumerate() {
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
    // As tall as its map (up to two thirds of the screen, all but the crumb and footer
    // on a phone): a title row, a blank row,
    // the map and a blank row inside its edges.
    let f = &cx.s.files;
    let fit_h = if cx.touch() {
        a.height.saturating_sub(7)
    } else {
        (a.height * 2 / 3).saturating_sub(4)
    };
    let gh = orbit::height(
        cx,
        Rect::new(a.x, a.y, a.width, fit_h),
        &f.device,
        3,
        orbit::Sel::Targets,
    );
    let h = (gh + 5).min(a.height);
    let r = Rect::new(a.x, a.bottom().saturating_sub(h), a.width, h);
    cx.clear_hits();
    cx.hit(Rect::new(a.x, a.y, a.width, r.y - a.y), "back");
    panel(cx, buf, r);
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
    let band = Rect::new(r.x, y + 2, r.width, gh);
    orbit::draw(cx, buf, band, &f.device, 3, orbit::Sel::Targets);
}

fn panel(cx: &Cx, buf: &mut Buffer, r: Rect) {
    if cx.has_surface() {
        cx.fill(buf, r, cx.st(Tok::Surface2));
    } else {
        cx.fill(buf, r, Style::default());
        let st = cx.st(Tok::Accent);
        let w = r.width.saturating_sub(2) as usize;
        let (tl, tr, bl, br, h, v) = if cx.uni() {
            ("╭", "╮", "╰", "╯", "─", "│")
        } else {
            // ASCII sides are blank: `|` is only the focus bar's continuation.
            ("-", "-", "-", "-", "-", " ")
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

pub(super) fn palette(cx: &Cx, buf: &mut Buffer, a: Rect, q: &str) {
    let ql = q.to_lowercase();
    // (group, description spans, the keys from the hub, action, the row's device).
    type Row<'r> = (
        String,
        Vec<(String, Style)>,
        String,
        String,
        Option<&'r LabDevice>,
    );
    let mut rows: Vec<Row> = Vec::new();
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
            Some(d),
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
                Some(d),
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
                    ("  ".into(), Style::default()),
                    (format!("{v}{}", d.name), cx.ink(Tok::Fg, 0.95)),
                ],
                k,
                format!("cmd:{v}{}", d.id),
                Some(d),
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
                    ("  ".into(), Style::default()),
                    (name.into(), cx.ink(Tok::Fg, 0.95)),
                ],
                k.into(),
                format!("cmd:{name}"),
                None,
            ));
        }
    }
    // Rows the app supplies (ranked by it) replace the fixture's.
    if let Some(given) = &cx.s.palette {
        rows = given
            .iter()
            .map(|p| {
                let mut spans = vec![
                    ("  ".into(), Style::default()),
                    (p.label.clone(), cx.ink(Tok::Fg, 0.95)),
                ];
                if !p.detail.is_empty() {
                    spans.push((format!("  {}", p.detail), cx.st(Tok::Muted)));
                }
                let dev = p.device.as_deref().and_then(|id| cx.s.device(id));
                (
                    p.group.clone(),
                    spans,
                    p.keys.clone(),
                    p.action.clone(),
                    dev,
                )
            })
            .collect();
    }
    // Sized to its results (query, a gap, rows and group labels, the key hint), at most
    // 60 % of the screen, giving way up to 75 % rather than cutting results.
    let groups = {
        let mut g: Vec<&str> = rows.iter().map(|r| r.0.as_str()).collect();
        g.dedup();
        g.len() as u16
    };
    // (A breathing row between sections when every row still fits.)
    let want = rows.len() as u16 + groups + 6 + groups.saturating_sub(1);
    let w = 64.min(a.width.saturating_sub(4));
    let h = want
        .min(a.height * 3 / 4)
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
        // The sheet ends a row above the screen's last, where its chips sit as on
        // every phone footer (never inside the sheet's border).
        Rect::new(a.x, a.y + 1, a.width, a.height.saturating_sub(2))
    } else {
        Rect::new(
            a.x + (a.width - w) / 2,
            a.y + 1 + (a.height.saturating_sub(h + 2)) / 3,
            w,
            h,
        )
    };
    // Only the sheet is live; a tap beside it closes the palette.
    cx.clear_hits();
    let below = if cx.touch() {
        0
    } else {
        a.bottom().saturating_sub(r.bottom())
    };
    for band in [
        Rect::new(a.x, a.y, a.width, r.y.saturating_sub(a.y)),
        Rect::new(a.x, r.bottom(), a.width, below),
    ] {
        cx.hit(band, "back");
    }
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
    // A query longer than the field shows its end, where the typing is.
    let room = usize::from(inner.width.saturating_sub(4));
    let n = q.chars().count();
    let shown: String = if n > room && room > 1 {
        std::iter::once('…')
            .chain(q.chars().skip(n + 1 - room))
            .collect()
    } else {
        q.to_string()
    };
    x += cx.put(
        buf,
        x,
        y,
        inner.width.saturating_sub(4),
        &shown,
        cx.ink(Tok::Fg, 1.0).add_modifier(Modifier::BOLD),
    );
    cx.put(buf, x, y, 1, cx.g("▏", "_"), cx.st(Tok::Accent));
    // The last row teaches the palette's own keys.
    // (Phones pin it to the screen's last row, like every phone footer.)
    let hint_y = if cx.touch() {
        a.bottom().saturating_sub(1)
    } else {
        inner.bottom().saturating_sub(1)
    };
    let mv = if cx.uni() { "↑↓" } else { "up/dn" };
    // (Phones tap a row, so their row keeps `esc close` in place of the arrows.)
    let chips: &[(&str, &str, &str)] = if cx.touch() {
        &[(cx.ret(), "go", "go"), ("esc", "close", "back")]
    } else {
        &[
            (cx.ret(), "go", "go"),
            (mv, "move", "move"),
            ("esc", "close", "back"),
        ]
    };
    cx.chips(buf, inner.x, hint_y, inner.right(), chips);
    // One breathing row between the last result and the hint chips.
    let inner = Rect::new(
        inner.x,
        inner.y,
        inner.width,
        inner.height.saturating_sub(2),
    );
    let mut yy = y + 2;
    let airy = inner.height + 2 >= want - 2;
    let mut last_group = String::new();
    let sel = cx.c.selected % rows.len().max(1);
    for (i, (group, spans, key, act, dev)) in rows.iter().enumerate() {
        let head = u16::from(*group != last_group);
        let rest = rows.len() - i;
        // Rows cut by the height cap say so on the last line, never silently.
        if yy + head >= inner.bottom() || (yy + head + 1 == inner.bottom() && rest > 1) {
            if yy < inner.bottom() {
                let t = format!("+{rest} more{}", cx.g(" ↓", ""));
                cx.put(buf, inner.x + 1, yy, inner.width, &t, cx.st(Tok::Muted));
            }
            break;
        }
        if head > 0 && i > 0 && airy {
            yy += 1;
        }
        if head > 0 {
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
            cx.fill(buf, row, cx.sel_bg());
            cx.put(buf, row.x, yy, 1, cx.g("▌", ">"), cx.st(Tok::Accent));
        }
        // The keys are keycaps, as everywhere (the row's device letter in its hue), so
        // the palette teaches the shortcuts by sight.
        let caps: Vec<&str> = key.split(' ').filter(|k| !k.is_empty()).collect();
        let kw = caps.len() as u16 * 3;
        let mut x = inner.x + 1;
        // The description is cut (with an ellipsis) before the key column.
        for (j, (t, st)) in spans.iter().enumerate() {
            let room = inner.right().saturating_sub(x + kw + 2);
            if room == 0 {
                break;
            }
            // The selected row's name is bold accent (the bar and tint do the rest).
            let st = if i == sel && j == 1 {
                st.patch(cx.st(Tok::Accent)).add_modifier(Modifier::BOLD)
            } else {
                *st
            };
            x += cx.put(buf, x, yy, room, &fit(t, room as usize, cx.uni()), st);
        }
        let mut kx = inner.right().saturating_sub(kw);
        for k in caps {
            kx += match dev {
                Some(d) if k.len() == 1 && k.starts_with(d.hotkey) => {
                    cx.dchip(buf, kx, yy, d, false)
                }
                _ => cx.put(buf, kx, yy, 3, &format!(" {k} "), cx.key_style()),
            };
        }
        cx.hit(row, act.clone());
        yy += 1;
    }
}

// -------------------------------------------------------------------------- secrets

fn secrets(cx: &Cx, buf: &mut Buffer, r: Rect) {
    // The zone label and cards sit on the same column as every other screen's zones;
    // one breathing row keeps the label's bar apart from the selected card's.
    let w = r.width.saturating_sub(2);
    let x0 = r.x + 1;
    let mut top = r.y + 2;
    cx.label(buf, x0, r.y, w, "SECRETS", true);
    let note = if w >= 60 {
        "keyring or 0600 file · shown only on reveal"
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
        cx.fill(buf, card, if picked { cx.sel_bg() } else { cx.surface() });
        if picked {
            cx.bar(buf, card.x, card.y, card.height, cx.st(Tok::Accent));
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
        // A missing card's button names the key that acts on it: `⏎` on the selected
        // card, its slot key on the others (`g add`).
        let button = missing.then(|| {
            let label = if s.key.starts_with("cloudflare") {
                "connect"
            } else {
                "add"
            };
            let key = match SLOTS.get(i) {
                Some(k) if !picked => k.to_string(),
                _ => cx.ret().to_string(),
            };
            format!(" {key} {label} ")
        });
        // The name is cut against the room really left (the button only on its row).
        let bw = button.as_ref().map_or(0, |t| super::width(t, cx.uni()) + 1);
        let lw = right.saturating_sub(x + bw);
        let lw = if compact { lw } else { lw.min(w / 2) };
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
        if let Some(t) = button {
            // A solid accent button with a background-coloured label (legible on a
            // tinted card in both themes).
            let st = cx
                .rgb_bg(cx.rgb(Tok::Accent))
                .patch(cx.rgb_fg(cx.bg0()))
                .add_modifier(Modifier::BOLD);
            // 16 colours: a cyan cell with black text, like the root capsule, so the one
            // call to action never reads as a plain gray keycap.
            let st = match cx.c.color {
                _ if cx.has_surface() => st,
                crate::ui::views::ColorLevel::C16 => Style::default()
                    .bg(ratatui::style::Color::Cyan)
                    .fg(ratatui::style::Color::Black)
                    .add_modifier(Modifier::BOLD),
                _ => cx.key_style(),
            };
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
        // The card opens its editor; a missing one's button (recorded above) keeps its
        // own tap, so the card's target stops short of it.
        let card_hit = if missing {
            Rect::new(card.x, card.y, card.width.saturating_sub(14), card.height)
        } else {
            card
        };
        cx.hit(card_hit, format!("secret:{}", s.key));
        top += card_h + 1;
    }
    // The open entry, where the `new secret` row goes (the screen's last rows when the
    // cards fill it): the name as typed, the value only as a count of dots.
    if let Some(e) = &cx.s.secret_entry {
        let y = top.min(r.bottom().saturating_sub(2));
        let rect = Rect::new(x0, y, w, r.bottom().saturating_sub(y).min(2));
        cx.fill(buf, rect, cx.sel_bg());
        cx.bar(buf, rect.x, rect.y, rect.height, cx.st(Tok::Accent));
        let (label, shown) = if e.naming {
            ("name", e.name.clone())
        } else {
            let dot = cx.g("•", "*");
            (e.name.as_str(), dot.repeat(e.typed.min(24)))
        };
        let mut x = x0 + 2;
        x += cx.put(buf, x, y, w / 2, label, cx.st(Tok::Muted)) + 2;
        x += cx.put(
            buf,
            x,
            y,
            w.saturating_sub(x - x0),
            &shown,
            cx.ink(Tok::Fg, 1.0),
        );
        cx.put(buf, x, y, 1, cx.g("▏", "_"), cx.st(Tok::Accent));
        if rect.height > 1 {
            let hint = match (e.naming, e.typed) {
                (true, _) => "the secret's name, then its value".to_string(),
                (false, 0) => "type or paste the value; it is never shown".into(),
                (false, n) => format!("{n} characters · ctrl+u clears"),
            };
            cx.put(
                buf,
                x0 + 2,
                y + 1,
                w.saturating_sub(2),
                &hint,
                cx.st(Tok::Muted),
            );
        }
        return;
    }
    // Any other secret takes the next free slot key.
    let n = cx.s.secrets.len();
    if let (Some(k), true) = (SLOTS.get(n), top < r.bottom()) {
        let x = x0 + 2;
        let kw = cx.put(buf, x, top, 3, &format!(" {k} "), cx.key_style());
        let t = cx.put(
            buf,
            x + kw + 1,
            top,
            w.saturating_sub(kw + 3),
            "new secret",
            cx.ink(Tok::Fg, 0.9),
        );
        cx.hit(Rect::new(x0, top, kw + t + 3, 1), "secret-new");
        top += 1;
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
            &fit_segs(text, usize::from(w.saturating_sub(2)), cx.uni()),
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
        // The hub's root: `◉ name` in bold, no background.
        let pw = super::width(&d.name, cx.uni()) + 2;
        let px = ill.x + (ill.width.saturating_sub(pw)) / 2;
        cx.put(
            buf,
            px,
            mid,
            1,
            cx.status(d.status).0,
            cx.status(d.status).1,
        );
        let nst = cx.ink(Tok::Fg, 1.0).add_modifier(Modifier::BOLD);
        cx.put(buf, px + 2, mid, pw - 2, &d.name, nst);
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
    // The button's note sits beside it when it fits whole, else on a muted row under
    // it (never cut).
    let note = "opens dash.cloudflare.com with a ready token";
    let nw = super::width(note, cx.uni());
    if bw + 2 + nw <= w {
        cx.put(buf, x0 + bw + 2, y, nw, note, cx.st(Tok::Muted));
    } else if !small && nw <= w {
        y += 1;
        cx.put(buf, x0, y, nw, note, cx.st(Tok::Muted));
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
            "kept in your keyring or a 0600 file; cx never prints it",
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
        // The note in a side column when it fits whole there; otherwise (below about
        // 90 columns) wrapped on muted rows under its step. Never cut.
        let dw = super::width(d, cx.uni());
        if dw + 24 <= w {
            cx.put(buf, x0 + 24, y, dw, d, cx.st(Tok::Muted));
        } else if !small {
            for line in wrap(d, usize::from(w.saturating_sub(x - x0))) {
                y += 1;
                cx.put(buf, x, y, w, &line, cx.st(Tok::Muted));
            }
        }
        y += if small { 1 } else { 2 };
    }
    // Secondary ways in, one per row, after a breathing row.
    y += 1;
    for (k, t, act) in [
        ("+", "I have a join code", "join"),
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
            "Opening an invite…",
            cx.st(Tok::Muted),
        );
        return;
    };
    let n = inv.qr.len() as u16;
    // A four-module quiet zone (the standard) wherever it fits beside or above the
    // text, two only when the alternative is no QR.
    let q: u16 = if r.height >= (n + 8).div_ceil(2) + 4 && r.width >= n + 8 {
        4
    } else {
        2
    };
    let qw = n + 2 * q;
    let qh = (n + 2 * q).div_ceil(2);
    let side_by_side = cx.uni() && r.width >= qw + 44 && r.height >= qh;
    let (qx, qy, tx, ty, tw) = if side_by_side {
        // The QR and the text column centre as one group, the column as wide as its
        // longest line (the one-liner).
        let col = (inv.oneliner.chars().count() as u16 + 2).clamp(40, 62);
        let total = qw + 4 + col.min(r.width - qw - 6);
        let x0 = r.x + (r.width.saturating_sub(total)) / 2;
        let qy = r.y + (r.height.saturating_sub(qh)) / 2;
        (x0, qy, x0 + qw + 4, qy + 1, total - qw - 4)
    } else {
        // Stacked, or no QR: the text column centres on its own, as wide as the
        // one-liner so the command stays unbroken where it can.
        let x0 = r.x + (r.width.saturating_sub(qw)) / 2;
        let col = (inv.oneliner.chars().count() as u16 + 2)
            .max(40)
            .min(r.width.saturating_sub(4));
        let tx = r.x + (r.width - col) / 2;
        (x0, r.y, tx, r.y + qh + 1, col)
    };
    // QR in half blocks with a light quiet zone, always dark-on-light.
    let room_h = r.bottom().saturating_sub(qy);
    let show_qr = n > 0 && (room_h >= qh && cx.uni() && r.height >= qh + 4 || side_by_side);
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
                    module(row as i32 * 2 - q as i32, col as i32 - q as i32),
                    module(row as i32 * 2 + 1 - q as i32, col as i32 - q as i32),
                );
                let (f, bg) = (if t { dark } else { light }, if b { dark } else { light });
                // 16 colours on a light theme: bright white is often a gray there, so the
                // light modules are the terminal's own background and the dark ones black
                // ink drawn with the NO_COLOR glyphs.
                let light16 = cx.c.color == crate::ui::views::ColorLevel::C16 && !cx.dark();
                let st = match cx.c.color {
                    _ if light16 => Style::default().fg(ratatui::style::Color::Black),
                    crate::ui::views::ColorLevel::None => Style::default(),
                    crate::ui::views::ColorLevel::C16 => Style::default()
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
                let sym = if cx.c.has_color() && !light16 {
                    "▀"
                } else {
                    // NO_COLOR draws light modules in the default ink; light16 draws the
                    // dark ones in black.
                    match (t != light16, b != light16) {
                        (true, true) => " ",
                        (true, false) => "▄",
                        (false, true) => "▀",
                        (false, false) => "█",
                    }
                };
                cx.put(buf, qx + col, qy + row, 1, sym, st);
            }
        }
        cx.hit(Rect::new(qx, qy, qw, qh), "copy-cmd");
    }
    let ty = if show_qr { ty } else { r.y + 1 };
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
        &format!("joins over {}, single use", inv.mesh),
        cx.st(Tok::Muted),
    );
    if !show_qr && cx.uni() && y + 1 < r.bottom() {
        y += 1;
        cx.put(buf, tx, y, tw, "QR on a wider screen", cx.st(Tok::Faint));
    }
    y += 2;
    // At M and up with room, the code is drawn in three-row half-block letters on a
    // five-row tint: the page's hero, readable across a room.
    // A half-block pixel is square (half a cell tall, one cell wide).
    let gw: u16 = inv.code.chars().map(|c| if c == '-' { 2 } else { 4 }).sum();
    let big =
        cx.size() >= Size::M && cx.uni() && gw + 3 <= tw && r.bottom().saturating_sub(y) >= 16;
    if big {
        let st = cx.ink(Tok::Accent, 1.0);
        let st = if cx.has_surface() {
            cx.st(Tok::Surface2).patch(st)
        } else {
            st
        };
        let tint = Rect::new(tx, y, gw + 3, 5);
        if cx.has_surface() {
            cx.fill(buf, tint, st);
        }
        big_text(cx, buf, tx + 2, y + 1, &inv.code, st);
        cx.hit(tint, "copy-code");
        y += 6;
    }
    // Otherwise the code as mono text on one tinted surface (not keycaps: its letters
    // are not cx keys), letters spaced so it reads aloud in pairs and triples.
    let spaced: String = inv
        .code
        .chars()
        .map(|c| {
            if c == '-' {
                " ".to_string()
            } else {
                c.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    let text = format!("  {spaced}  ");
    let cw = super::width(&text, cx.uni());
    let st = cx.ink(Tok::Accent, 1.0).add_modifier(Modifier::BOLD);
    let st = if cx.has_surface() {
        cx.st(Tok::Surface2).patch(st)
    } else {
        st
    };
    // With room, the surface is three rows tall, so the code is the page's largest mark.
    let tall = cx.has_surface() && r.height >= 24;
    if !big {
        if tall {
            cx.fill(buf, Rect::new(tx, y, cw, 3), st);
            y += 1;
        }
        let x = tx + cx.put(buf, tx, y, cw, &text, st);
        cx.hit(
            Rect::new(tx, y - u16::from(tall), x - tx, 1 + 2 * u16::from(tall)),
            "copy-code",
        );
        y += if tall { 3 } else { 2 };
    }
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
        // `set-clipboard on`); `y`/`Y` copying says it cannot confirm.
        let cw = tw.saturating_sub(2).max(1) as usize;
        let mut parts = break_cmd(&inv.oneliner, cw);
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
        y += block.height;
        // A device that cannot reach the one-liner's address (off-site, on the tailnet) may reach these.
        if !inv.also.is_empty() && y < r.bottom() {
            let also = format!("also listening on {}", inv.also.join(" "));
            cx.put(buf, tx, y, tw, &also, cx.st(Tok::Muted));
            y += 1;
        }
        y += 1;
    }
    if y < r.bottom() {
        let left = inv.expires_secs as f32 / crate::ui::screens::INVITE_TTL.as_secs_f32();
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

/// Cut a command into rows of at most `w`, breaking after a space or `/` where one
/// falls in the row (never inside the code), else hard.
fn break_cmd(s: &str, w: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest: Vec<char> = s.chars().collect();
    while rest.len() > w {
        let cut = (w / 2..w)
            .rev()
            .find(|&i| rest[i - 1] == ' ' || rest[i - 1] == '/')
            .unwrap_or(w);
        out.push(rest.drain(..cut).collect());
    }
    out.push(rest.into_iter().collect());
    out
}

/// A 3x5 pixel font for join codes (digits and capitals), one `u16` per glyph, rows top
/// to bottom, three bits per row (left bit first).
fn glyph_bits(c: char) -> u16 {
    const G: [&str; 36] = [
        "111101101101111",
        "010110010010111",
        "111001111100111",
        "111001011001111",
        "101101111001001",
        "111100111001111",
        "111100111101111",
        "111001001010010",
        "111101111101111",
        "111101111001111",
        "010101111101101",
        "110101110101110",
        "011100100100011",
        "110101101101110",
        "111100110100111",
        "111100110100100",
        "011100101101011",
        "101101111101101",
        "111010010010111",
        "001001001101010",
        "101110100110101",
        "100100100100111",
        "101111111101101",
        "110101101101101",
        "010101101101010",
        "110101110100100",
        "111101101111011",
        "110101110101101",
        "011100010001110",
        "111010010010010",
        "101101101101111",
        "101101101101010",
        "101101111111101",
        "101101010101101",
        "101101010010010",
        "111001010100111",
    ];
    let i = match c.to_ascii_uppercase() {
        d @ '0'..='9' => d as usize - '0' as usize,
        l @ 'A'..='Z' => 10 + l as usize - 'A' as usize,
        _ => return 0,
    };
    u16::from_str_radix(G[i], 2).unwrap_or(0)
}

/// Draw `code` three rows tall in half blocks: 3 columns per letter, one blank between
/// letters, two at a `-`.
fn big_text(cx: &Cx, buf: &mut Buffer, x: u16, y: u16, code: &str, st: Style) {
    let mut x = x;
    for c in code.chars() {
        if c == '-' {
            x += 2;
            continue;
        }
        let bits = glyph_bits(c);
        // Pixel rows -1..5 (a blank row on top centres five rows in three cells).
        let px = |row: i32, col: u16| row >= 0 && bits >> (14 - row * 3 - i32::from(col)) & 1 == 1;
        for row in 0..3u16 {
            for col in 0..3u16 {
                let (t, b) = (px(row as i32 * 2 - 1, col), px(row as i32 * 2, col));
                let g = match (t, b) {
                    (true, true) => "█",
                    (true, false) => "▀",
                    (false, true) => "▄",
                    (false, false) => " ",
                };
                cx.put(buf, x + col, y + row, 1, g, st);
            }
        }
        x += 4;
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
    // Map and sheet are one block: the map at the hub's spacing, two blank rows, then
    // the sheet (at most 110 wide, the screen's two-column gutter either side); the block
    // sits in the middle of the rows between the crumb and the footer, a row clear of
    // each.
    // Larger screens draw the rtt history as a taller chart and, at the largest, the
    // first session that crosses the link (what a change would cut).
    let crossing: Vec<&LabSession> = cx
        .s
        .sessions
        .iter()
        .filter(|x| {
            x.device == e.to && e.from == cx.s.viewer || x.device == e.from && e.to == cx.s.viewer
        })
        .collect();
    let users = crossing.len();
    let mut rtt_h: u16 = match cx.size() {
        Size::L => 5,
        Size::M => 3,
        _ => 1,
    };
    let tile_h: u16 = if cx.size() == Size::L && users > 0 {
        9
    } else {
        0
    };
    let mut sheet_h: u16 = if cx.size() == Size::Xs {
        9
    } else {
        10 + rtt_h + tile_h
    }
    .min(r.height);
    let col = r.width.saturating_sub(4).min(110);
    let cx0 = r.x + (r.width - col) / 2;
    let avail = r.height.saturating_sub(sheet_h + 4);
    let gh = if cx.touch() || avail < 3 {
        avail.min(3)
    } else {
        orbit::drawn_h(
            cx,
            Rect::new(r.x, r.y, r.width, avail),
            orbit::Sel::Edge(&e.id),
        )
    };
    let mut spare = avail.saturating_sub(gh);
    // Rows the map leaves go to the rtt chart (up to twice its height, a row of
    // air kept either side) before any air
    // opens around the block.
    if rtt_h > 1 {
        let more = spare.saturating_sub(2).min(rtt_h);
        rtt_h += more;
        sheet_h += more;
        spare -= more;
    }
    let band = Rect::new(r.x, r.y + 1 + spare / 2, r.width, gh);
    let sheet_y = band.bottom() + 2;
    if !cx.touch() && band.height >= 1 && avail >= 3 {
        // (A row more either side where there is air, for the mesh wash's margin.)
        let band = if spare >= 2 {
            Rect::new(band.x, band.y - 1, band.width, band.height + 2)
        } else {
            band
        };
        orbit::draw(cx, buf, band, &cx.s.viewer, 3, orbit::Sel::Edge(&e.id));
    } else if let (Some(a), Some(b)) = (cx.s.device(&e.from), cx.s.device(&e.to)) {
        // Phones draw no map (it only crowded at that size): the link alone, its two
        // keycaps around the policy, in the accent like the map's edited link.
        let mid = format!(" {} ", orbit::policy(cx, e, true, false));
        let line = cx.g("───", "---");
        let w = 6 + 2 * super::width(line, cx.uni()) + super::width(&mid, cx.uni());
        let mut x = r.x + r.width.saturating_sub(w) / 2;
        let y = band.y + band.height.max(1) / 2;
        let ink = cx.st(Tok::Accent);
        x += cx.dchip(buf, x, y, a, false);
        x += cx.put(buf, x, y, 3, line, ink);
        x += cx.put(buf, x, y, 20, &mid, ink.add_modifier(Modifier::BOLD));
        x += cx.put(buf, x, y, 3, line, ink);
        cx.dchip(buf, x, y, b, false);
    }
    let sheet = Rect::new(cx0, sheet_y, col, sheet_h);
    panel(cx, buf, sheet);
    let inner = inset(sheet, 2, 1);
    let (Some(a), Some(b)) = (cx.s.device(&e.from), cx.s.device(&e.to)) else {
        return;
    };
    let (a, b) = if flipped { (b, a) } else { (a, b) };
    let arrow = cx.g("→", "->");
    let dirs = (
        format!("{}{arrow}{}", a.hotkey, b.hotkey),
        format!("{}{arrow}{}", b.hotkey, a.hotkey),
    );
    let mut x = inner.x;
    let y = inner.y;
    x += cx.dchip(buf, x, y, a, false);
    x += 1;
    let bold = cx.ink(Tok::Fg, 1.0).add_modifier(Modifier::BOLD);
    x += cx.put(buf, x, y, 20, &a.name, bold);
    x += cx.put(buf, x, y, 6, cx.g("  ⇄  ", "  <>  "), cx.st(Tok::Muted));
    x += cx.dchip(buf, x, y, b, false);
    x += 1;
    x += cx.put(buf, x, y, 20, &b.name, bold);
    let (kind, tok) = edge_kind(cx, e);
    if cx.size() >= Size::S {
        // (A link nobody measured says so; loss only from samples.)
        let rtt = match (e.rtt_ms, e.up) {
            (Some(v), _) => super::ms_text(v),
            (None, true) => "rtt not measured".into(),
            (None, false) => "down".into(),
        };
        let sep = cx.g(" · ", " - ");
        let loss = if e.rtt_hist.is_empty() {
            String::new()
        } else {
            format!("{sep}loss {:.0}%", e.loss * 100.0)
        };
        let t = format!("{kind}{sep}{rtt}{loss}");
        let tw = super::width(&t, cx.uni());
        if x + tw + 4 < inner.right() {
            cx.put_r(buf, inner.right(), y, &t, cx.ink(tok, 0.95));
        }
    }
    // Two lanes, one per direction, each a segmented control: `a p o` for the opener's
    // way out, `A P O` for the way back (`j k` move between them, as in every list).
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
        // ASCII keeps a blank cell after the `>` bar (`> t->i`, never `>t->i`).
        let mut lx = inner.x + u16::from(!cx.uni());
        let picked = i == sel_lane;
        if picked {
            cx.put(
                buf,
                inner.x.saturating_sub(1),
                ly,
                1,
                cx.g("▌", ">"),
                cx.st(Tok::Accent),
            );
        }
        let dst = if picked {
            cx.st(Tok::Accent).add_modifier(Modifier::BOLD)
        } else {
            cx.ink(Tok::Fg, 0.8)
        };
        lx += cx.put(buf, lx, ly, 5, if i == 0 { &dirs.0 } else { &dirs.1 }, dst);
        lx += 2;
        // Phones say `pass`, so all three segments keep their shape on one row.
        let opts = [
            (Access::Key, "a", "key"),
            (
                Access::Password,
                "p",
                if inner.width < 44 { "pass" } else { "password" },
            ),
            (Access::Denied, "o", "off"),
        ];
        for (val, k, label) in opts {
            let on = *acc == val;
            let k = if i == 1 {
                k.to_uppercase()
            } else {
                k.to_string()
            };
            // Every segment has one shape: keycap, a blank cell, the word.
            let text = format!(" {k}  {label} ");
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
            if !on && n >= 3 {
                // An unselected segment's key still reads as pressable: its keycap.
                cx.paint(buf, Rect::new(lx, ly, 3, 1), cx.key_style());
            }
            cx.hit(
                Rect::new(lx, ly, n, 1),
                format!(
                    "policy:{}:{}:{}",
                    f.id,
                    t.id,
                    if k.eq_ignore_ascii_case("p") {
                        "password"
                    } else {
                        label
                    }
                ),
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
    // One row, so the rtt row keeps its blank row above: where the whole sentence does
    // not fit, only the count (the footer says `u undo`).
    let count = format!(
        "{users} open session{} cross{} this link.",
        if users == 1 { "" } else { "s" },
        if users == 1 { "es" } else { "" },
    );
    let msg = format!("Each press applies at once; u undoes. {count}");
    let msg = if super::width(&msg, cx.uni()) <= inner.width {
        msg
    } else {
        count
    };
    cx.put(
        buf,
        inner.x,
        inner.y + 6,
        inner.width,
        &msg,
        cx.st(Tok::Muted),
    );
    let ry = inner.y + 8;
    if ry < inner.bottom() && cx.size() >= Size::S {
        // The rtt history and its figures as quiet text after it, on its baseline.
        let base = (ry + rtt_h - 1).min(inner.bottom().saturating_sub(1));
        let ch = base + 1 - ry;
        cx.put(buf, inner.x, base, 4, "rtt", cx.st(Tok::Muted));
        // At most half the row, so the figures after it keep their words.
        let sw = (inner.width.saturating_sub(5) / 2).min(40);
        let v: Vec<f32> = e.rtt_hist.iter().flatten().copied().collect();
        if ch > 1 {
            let hi = v.iter().copied().fold(1.0f32, f32::max);
            let vals: Vec<f32> = e
                .rtt_hist
                .iter()
                .map(|x| x.map_or(0.0, |x| x / hi))
                .collect();
            tiles::chart_in(
                cx,
                buf,
                Rect::new(inner.x + 5, ry, sw, ch),
                &vals,
                Some(cx.ink(Tok::Muted, 0.6)),
            );
        } else {
            tiles::spark(
                cx,
                buf,
                inner.x + 5,
                ry,
                sw,
                &e.rtt_hist,
                cx.ink(Tok::Muted, 0.6),
            );
        }
        let mut t = if e.rtt_hist.is_empty() {
            "no samples yet".to_string()
        } else {
            format!("{} samples", e.rtt_hist.len())
        };
        if !v.is_empty() {
            let ms = super::ms_text;
            let lo = v.iter().copied().fold(f32::MAX, f32::min);
            let hi = v.iter().copied().fold(f32::MIN, f32::max);
            let mean = v.iter().sum::<f32>() / v.len() as f32;
            let jit = v.windows(2).map(|w| (w[1] - w[0]).abs()).sum::<f32>()
                / (v.len().max(2) - 1) as f32;
            t = format!(
                "{t} · min {} · mean {} · max {} · jitter {}",
                ms(lo),
                ms(mean),
                ms(hi),
                ms(jit)
            );
        }
        let tx = inner.x + 7 + sw;
        let room = inner.right().saturating_sub(tx) as usize;
        let t = fit_segs(&t, room, cx.uni());
        let t = if cx.uni() {
            t
        } else {
            t.replace(" · ", " - ")
        };
        cx.put(buf, tx, base, room as u16, &t, cx.st(Tok::Muted));
        if tile_h > 0 && base + tile_h < inner.bottom() + 1 {
            let tr = Rect::new(inner.x, base + 2, inner.width, tile_h - 1);
            tiles::thumb_with(cx, buf, tr, crossing[0], None, false);
        }
    }
}

/// Cut a `a · b · c` line at a segment boundary so no word is clipped mid-way (one long
/// segment falls back to an ellipsis).
fn fit_segs(s: &str, w: usize, unicode: bool) -> String {
    let mut out = String::new();
    for seg in s.split(" · ") {
        let next = if out.is_empty() {
            seg.to_string()
        } else {
            format!("{out} · {seg}")
        };
        if next.chars().count() > w {
            break;
        }
        out = next;
    }
    if out.is_empty() {
        fit(s, w, unicode)
    } else {
        out
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

/// Help for the view it was opened from (`CX_LAB_FOCUS=device|edge|files`, else the
/// hub): the keys that matter there, grouped by intent, one row of keys that work
/// everywhere and, where links are drawn, the line grammar. On the hub the two steps
/// are said apart: the device row shows real names with their letter underlined, as on
/// the map, and the `then` keys come after it, so `c` the device and `c` claude never
/// read as one key. A calm list, a third of the way down, at every size.
fn help(cx: &Cx, buf: &mut Buffer, r: Rect) {
    let key = |k: &'static str| -> &'static str {
        match k {
            "⏎" => cx.ret(),
            "⌫" => "bksp",
            o => o,
        }
    };
    let wide = r.width >= 60;
    let ctx = match cx.c.focus {
        Some(Pane::Custom("device")) => "IN A DEVICE",
        Some(Pane::Custom("edge")) => "ON A LINK",
        Some(Pane::Files) => "IN FILES",
        _ => "ON THE HUB",
    };
    let hub = ctx == "ON THE HUB";
    type Group = (&'static str, Vec<(String, &'static str)>);
    let k = |s: &str| s.to_string();
    let mut groups: Vec<Group> = match ctx {
        "IN A DEVICE" => vec![
            (
                "session",
                vec![
                    (
                        k("a s d g"),
                        if wide {
                            "attach the 1st / 2nd / 3rd / 4th session"
                        } else {
                            "attach session 1-4"
                        },
                    ),
                    (k(key("⏎")), "attach the selection"),
                    (k("spc"), "peek"),
                    (k(key("⌫")), "stop (press twice)"),
                ],
            ),
            (
                "new",
                vec![(
                    k("c x n"),
                    if wide {
                        "claude / codex / shell, attached"
                    } else {
                        "claude/codex/shell"
                    },
                )],
            ),
            (
                "device",
                vec![
                    (k("f"), "files"),
                    (
                        k("e"),
                        if wide {
                            "then a letter: edit that link"
                        } else {
                            "then a letter: link"
                        },
                    ),
                    (k("r"), "retry"),
                ],
            ),
            (
                "move",
                vec![
                    (k("j k"), "step"),
                    (k("esc"), "back"),
                    (k("-"), "last session"),
                ],
            ),
        ],
        "ON A LINK" => vec![
            ("way out", vec![(k("a p o"), "key / password / off")]),
            ("way back", vec![(k("A P O"), "key / password / off")]),
            (
                "move",
                vec![(k("j k"), "lane"), (k("u"), "undo"), (k("esc"), "done")],
            ),
        ],
        "IN FILES" => vec![
            (
                "move",
                vec![
                    (k("j k"), "step"),
                    (k("l h"), "unfold / fold"),
                    (k(key("⏎")), "open"),
                ],
            ),
            (
                "file",
                vec![
                    (k("spc"), "preview"),
                    (
                        k("s"),
                        if wide {
                            "then a letter: send to that device"
                        } else {
                            "then a letter: send"
                        },
                    ),
                    (k("y"), "copy the path"),
                ],
            ),
            ("back", vec![(k("esc"), "up a level")]),
        ],
        _ => vec![
            ("device", vec![(String::new(), "its letter opens it")]),
            // The second step, indented under the first: these keys follow a letter.
            (
                "  then",
                vec![
                    (
                        k("a s d g"),
                        if wide {
                            "attach its 1st / 2nd / 3rd / 4th session"
                        } else {
                            "attach session 1-4"
                        },
                    ),
                    (
                        k("c x n"),
                        if wide {
                            "new claude / codex / shell there"
                        } else {
                            "new claude/codex/shell"
                        },
                    ),
                    (k("f"), "its files"),
                ],
            ),
            (
                "move",
                vec![
                    (k("j k"), "step"),
                    (k("tab"), "next zone"),
                    (k(key("⏎")), "open"),
                    (k("-"), "last session"),
                ],
            ),
            (
                "act",
                vec![
                    (k("spc"), "peek"),
                    (
                        k("e"),
                        if wide {
                            "then a letter: edit that link"
                        } else {
                            "then a letter: link"
                        },
                    ),
                    (k(key("⌫")), "stop a session"),
                ],
            ),
        ],
    };
    // Keys that work everywhere, one per row like every other block (`esc` is on the
    // footer, where it closes this).
    groups.push((
        "global",
        vec![
            (k("/"), "find"),
            (k("`"), "hub"),
            (k("+"), "invite"),
            (k("K"), "secrets"),
            (k("ctrl+\\"), "detach"),
        ],
    ));
    // The map's marks, where a map is drawn: strokes, then the device states (and, in
    // ASCII, the cursor a selection takes there).
    let mut marks: Vec<(&str, &str)> = vec![
        (cx.g("──", "--"), "key"),
        (cx.g("╌╌", ".."), "password"),
        (cx.g("◀─", "<-"), "one way"),
        (cx.g("─╴", "- "), "closed"),
    ];
    if ctx == "ON A LINK" {
        marks.push((cx.g("──", "##"), "this link"));
    }
    if cx.uni() {
        marks.push(("●", "online"));
    }
    marks.extend([(cx.g("○", "o"), "late"), (cx.g("✕", "!"), "down")]);
    if !cx.uni() {
        marks.push(("[ ]", "selected"));
    }
    // (Wide enough for three device names and what their letter does.)
    let w = r
        .width
        .saturating_sub(if wide { 4 } else { 2 })
        .min(if cx.size() >= Size::M { 78 } else { 68 });
    let x0 = r.x + (r.width - w) / 2;
    let right = x0 + w;
    let ix = x0 + u16::from(wide);
    // The intent column as wide as its longest word and a gap.
    let iw = groups
        .iter()
        .map(|(n, _)| super::width(n, cx.uni()))
        .chain([5])
        .max()
        .unwrap_or(6)
        + if wide { 3 } else { 1 };
    let kx = ix + iw;
    // One key column for every block, marks included, so every row keeps two columns.
    let kw = groups
        .iter()
        .flat_map(|(_, ks)| ks.iter().map(|(k, _)| k.as_str()))
        .chain(marks.iter().map(|(k, _)| *k))
        .map(|k| super::width(k, cx.uni()))
        .max()
        .unwrap_or(3);
    let dx = kx + kw + if wide { 2 } else { 1 };
    // The marks as a grid whose every cell has the rows' key column.
    let tw = marks
        .iter()
        .map(|(_, t)| super::width(t, cx.uni()))
        .max()
        .unwrap_or(0);
    let cw = (dx - kx) + tw + 3;
    let per_row = ((right + 3).saturating_sub(kx) / cw).max(1);
    let mark_rows = (marks.len() as u16).div_ceil(per_row);
    let body0 = groups.iter().map(|(_, k)| k.len() as u16).sum::<u16>();
    // The marks where a map is drawn, when the screen holds them with their air.
    let strokes = !cx.touch()
        && matches!(ctx, "ON THE HUB" | "ON A LINK" | "IN A DEVICE")
        && body0 + mark_rows + groups.len() as u16 + 3 <= r.height;
    let body = body0 + if strokes { mark_rows } else { 0 };
    let blocks = groups.len() as u16 + u16::from(strokes);
    // A blank row between blocks, except between a step and its `then`; the label and
    // the air under it. Short screens give up the air between blocks, then the label.
    let gaps = blocks - 1 - u16::from(hub);
    let air = body + gaps + 2 <= r.height;
    let label = body + 1 + if air { gaps } else { 0 } <= r.height;
    let under = label && air && body + gaps + 3 <= r.height;
    let total = body + if air { gaps } else { 0 } + u16::from(label) + u16::from(under);
    // A third of the way down the rows between the crumb and the footer.
    let spare = r.height.saturating_sub(total);
    let mut y = r.y + spare / 3;
    if label {
        zone(cx, buf, x0, y, right, ctx, true);
        y += 1 + u16::from(under);
    }
    let intent = |buf: &mut Buffer, y: u16, t: &str| {
        cx.put(buf, ix, y, iw, t, cx.ink(Tok::Muted, 0.8));
    };
    let bold = cx.ink(Tok::Fg, 1.0).add_modifier(Modifier::BOLD);
    let what = cx.ink(Tok::Fg, 0.85);
    for (gi, (name, ks)) in groups.iter().enumerate() {
        if gi > 0 && air && !(hub && name.trim() == "then") {
            y += 1;
        }
        for (i, (k, v)) in ks.iter().enumerate() {
            if y >= r.bottom() {
                return;
            }
            if i == 0 {
                intent(buf, y, name);
            }
            if k.is_empty() {
                // The first step: devices by name, their letter underlined as on the map.
                let desc = super::width(v, cx.uni()) + 3;
                let room = right.saturating_sub(kx);
                let mut x = kx;
                let mut shown = 0;
                for d in &cx.s.devices {
                    let nw = super::width(&d.name, cx.uni()) + if cx.uni() { 0 } else { 2 };
                    let left = cx.s.devices.len() - shown - 1;
                    let tail = if left == 0 {
                        0
                    } else if cx.uni() {
                        3
                    } else {
                        8
                    };
                    // Wide, the names leave room for what the letter does.
                    let keep = if wide { desc } else { 0 };
                    if x + nw + tail + keep > kx + room && shown > 0 {
                        break;
                    }
                    x += orbit::name(cx, buf, x, y, d, 16, what, true) + 2;
                    shown += 1;
                }
                if shown < cx.s.devices.len() {
                    let more = if cx.uni() {
                        "…".to_string()
                    } else {
                        format!("{} more", cx.s.devices.len() - shown)
                    };
                    x += cx.put(buf, x, y, 8, &more, cx.st(Tok::Muted)) + 2;
                }
                if x + desc <= right {
                    cx.put(buf, x + 1, y, desc, v, cx.st(Tok::Muted));
                }
                y += 1;
                continue;
            }
            // Keys one by one, a space between them.
            let mut x = kx;
            for part in k.split(' ') {
                x += cx.put(buf, x, y, kw, part, bold) + 1;
            }
            let room = right.saturating_sub(dx);
            cx.put(buf, dx, y, room, &fit(v, usize::from(room), cx.uni()), what);
            if name.trim() == "global" {
                let act = match k.as_str() {
                    "/" => "palette",
                    "`" => "hub",
                    "+" => "invite",
                    "K" => "secrets",
                    _ => "detach",
                };
                cx.hit(
                    Rect::new(kx, y, dx - kx + super::width(v, cx.uni()), 1),
                    act,
                );
            }
            y += 1;
        }
    }
    if strokes {
        if air {
            y += 1;
        }
        for (i, (k, t)) in marks.iter().enumerate() {
            let (col, row) = (i as u16 % per_row, i as u16 / per_row);
            let yy = y + row;
            if yy >= r.bottom() {
                break;
            }
            if i == 0 {
                intent(buf, yy, "marks");
            }
            let x = kx + col * cw;
            cx.put(buf, x, yy, dx - kx, k, cx.ink(Tok::Muted, 0.75));
            cx.put(buf, x + dx - kx, yy, tw, t, cx.ink(Tok::Muted, 0.95));
        }
    }
}
