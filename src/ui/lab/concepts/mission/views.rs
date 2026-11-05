//! Screens: hub, device, session peek, files, preview, palette, keys, onboarding, join,
//! link policy and help. Each is a plain function over the shared `Cx`.
use super::{centered, fmt, inset, map, mix, tiles, Cx, Size, SLOTS};
use crate::ui::lab::{
    caps::Pane, Access, DeviceStatus, EdgeKind, FileKind, LabDevice, LabEdge, LabView, PreviewKind,
    Tok,
};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
};

pub(super) fn draw(cx: &Cx, view: &LabView, buf: &mut Buffer) {
    let a = cx.area;
    let body = Rect::new(a.x, a.y + 1, a.width, a.height.saturating_sub(2));
    let foot = Rect::new(a.x, a.bottom() - 1, a.width, 1);
    match view {
        LabView::Hub => {
            top(cx, buf, &[]);
            hub(cx, buf, body);
            footer(cx, buf, foot, hub_keys(cx));
        }
        LabView::DeviceFocus(id) => {
            let d = cx.s.device(id).or(cx.s.devices.first());
            top(
                cx,
                buf,
                &d.map(|d| Crumb::Dev(d)).into_iter().collect::<Vec<_>>(),
            );
            if let Some(d) = d {
                device(cx, buf, body, d);
            }
            footer(
                cx,
                buf,
                foot,
                vec![
                    ("a s d", "open", "attach"),
                    ("c", "claude", "new-claude"),
                    ("x", "codex", "new-codex"),
                    ("n", "shell", "new-shell"),
                    ("f", "files", "files"),
                    ("esc", "hub", "back"),
                ],
            );
        }
        LabView::SessionPeek(id) => {
            let x = cx.s.session(id).or(cx.s.sessions.first());
            let d = x.and_then(|x| cx.s.device(&x.device));
            let mut crumbs: Vec<Crumb> = d.map(Crumb::Dev).into_iter().collect();
            if let Some(x) = x {
                crumbs.push(Crumb::Text(x.title.clone()));
            }
            top(cx, buf, &crumbs);
            if let Some(x) = x {
                peek(cx, buf, body, x);
            }
            footer(
                cx,
                buf,
                foot,
                vec![
                    ("⏎", "attach", "attach"),
                    ("i", "type here", "type"),
                    ("␣", "close", "back"),
                    ("ctrl \\", "detach", "detach"),
                ],
            );
        }
        LabView::Files | LabView::FilePreview => {
            let d = cx.s.device(&cx.s.files.device);
            let mut crumbs: Vec<Crumb> = d.map(Crumb::Dev).into_iter().collect();
            crumbs.push(Crumb::Text(cx.s.files.cwd.clone()));
            if matches!(view, LabView::FilePreview) {
                crumbs.push(Crumb::Text(cx.s.files.preview_path.clone()));
            }
            top(cx, buf, &crumbs);
            if matches!(view, LabView::Files) {
                files(cx, buf, body);
                footer(
                    cx,
                    buf,
                    foot,
                    vec![
                        ("⏎", "open", "open"),
                        ("␣", "preview", "preview"),
                        ("t", "send to", "send"),
                        ("y", "copy", "copy"),
                        ("esc", "back", "back"),
                    ],
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
                        ("t", "send to", "send"),
                        ("y", "copy path", "copy"),
                    ],
                );
            }
        }
        LabView::Palette(q) => {
            top(cx, buf, &[]);
            hub(cx, buf, body);
            footer(cx, buf, foot, hub_keys(cx));
            cx.dim(buf, a);
            palette(cx, buf, a, q);
        }
        LabView::Secrets => {
            top(cx, buf, &[Crumb::Text("keys".into())]);
            secrets(cx, buf, body);
            footer(
                cx,
                buf,
                foot,
                vec![
                    ("⏎", "edit", "edit"),
                    ("r", "reveal 10s", "reveal"),
                    ("c", "copy", "copy"),
                    ("esc", "hub", "back"),
                ],
            );
        }
        LabView::Onboarding => {
            top(cx, buf, &[Crumb::Text("welcome".into())]);
            onboarding(cx, buf, body);
            footer(
                cx,
                buf,
                foot,
                vec![
                    ("⏎", "connect", "connect"),
                    ("j", "have a code", "join"),
                    ("esc", "skip", "skip"),
                ],
            );
        }
        LabView::JoinCode => {
            top(cx, buf, &[Crumb::Text("invite".into())]);
            join(cx, buf, body);
            footer(
                cx,
                buf,
                foot,
                vec![
                    ("c", "copy code", "copy-code"),
                    ("y", "copy command", "copy-cmd"),
                    ("r", "new code", "renew"),
                    ("esc", "done", "back"),
                ],
            );
        }
        LabView::EdgeEditor(id) => {
            top(cx, buf, &[Crumb::Text("link".into())]);
            edge_editor(cx, buf, body, id);
            footer(
                cx,
                buf,
                foot,
                vec![
                    ("↑↓", "direction", "dir"),
                    ("k p o", "key pwd off", "policy"),
                    ("⏎", "apply", "apply"),
                    ("esc", "cancel", "back"),
                ],
            );
        }
        LabView::Help => {
            top(cx, buf, &[Crumb::Text("keys".into())]);
            help(cx, buf, body);
            footer(cx, buf, foot, vec![("esc", "close", "back")]);
        }
    }
}

enum Crumb<'a> {
    Dev(&'a LabDevice),
    Text(String),
}

/// Top bar: wordmark, breadcrumb, then global evidence on the right.
fn top(cx: &Cx, buf: &mut Buffer, crumbs: &[Crumb]) {
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
    let right_w = right_status(cx, buf, y);
    let limit = a.right().saturating_sub(right_w + 2);
    for c in crumbs {
        if x + 4 >= limit {
            break;
        }
        x += cx.put(buf, x, y, 3, cx.g(" › ", " > "), cx.ink(Tok::Faint, 1.0));
        match c {
            Crumb::Dev(d) => {
                let x0 = x;
                x += cx.dchip(buf, x, y, d, false);
                x += 1;
                x += cx.put(
                    buf,
                    x,
                    y,
                    limit.saturating_sub(x),
                    &d.name,
                    cx.ink(Tok::Fg, 1.0).add_modifier(Modifier::BOLD),
                );
                cx.hit(Rect::new(x0, y, x - x0, 1), format!("device:{}", d.id));
            }
            Crumb::Text(t) => {
                let room = limit.saturating_sub(x);
                x += cx.put(
                    buf,
                    x,
                    y,
                    room,
                    &fmt::fit(t, room as usize, cx.uni()),
                    cx.ink(Tok::Fg, 0.9),
                );
            }
        }
    }
    if crumbs.is_empty() && cx.size() >= Size::S {
        cx.put(
            buf,
            x + 2,
            y,
            limit.saturating_sub(x + 2),
            "mission control",
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
    if let Some(n) = s.notifications.iter().max_by_key(|n| n.kind as u8) {
        let tok = match n.kind {
            crate::ui::lab::NoticeKind::Err => Tok::Err,
            crate::ui::lab::NoticeKind::Warn => Tok::Warn,
            crate::ui::lab::NoticeKind::Ok => Tok::Ok,
            crate::ui::lab::NoticeKind::Info => Tok::Info,
        };
        let text = if cx.size() >= Size::L {
            format!("{} {}", cx.g("▲", "!"), fmt::fit(&n.text, 44, cx.uni()))
        } else {
            format!("{} {}", cx.g("▲", "!"), s.notifications.len())
        };
        parts.push((text, cx.st(tok), "notices"));
    }
    let dot = if down == 0 {
        cx.st(Tok::Ok)
    } else {
        cx.st(Tok::Muted)
    };
    parts.push((
        format!("{} {up}/{}", cx.g("●", "*"), s.devices.len()),
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

fn footer(cx: &Cx, buf: &mut Buffer, r: Rect, items: Vec<(&str, &str, &str)>) {
    let items: Vec<(&str, &str, &str)> = items
        .into_iter()
        .map(|(k, l, a)| {
            let k = if cx.uni() {
                k
            } else {
                match k {
                    "⏎" => "ret",
                    "␣" => "spc",
                    "↑↓" => "up/dn",
                    "←→" => "l/r",
                    other => other,
                }
            };
            let l = if cx.size() == Size::Xs && l.len() > 6 {
                l.split(' ').next().unwrap_or(l)
            } else {
                l
            };
            (k, l, a)
        })
        .collect();
    cx.chips(buf, r.x + 1, r.y, r.right().saturating_sub(1), &items);
}

fn hub_keys(cx: &Cx) -> Vec<(&'static str, &'static str, &'static str)> {
    let mut v = vec![
        ("a-z", "device", "device"),
        ("⏎", "open", "open"),
        ("tab", "zone", "zone"),
        ("/", "find", "palette"),
    ];
    if cx.size() >= Size::S {
        v.push(("␣", "peek", "peek"));
    }
    if cx.size() >= Size::M {
        v.push(("+", "invite", "invite"));
        v.push(("?", "keys", "help"));
    }
    v
}

// ------------------------------------------------------------------------------ hub

fn hub(cx: &Cx, buf: &mut Buffer, r: Rect) {
    let s = cx.s;
    let focus = cx.c.focus.unwrap_or(Pane::Graph);
    let sz = cx.size();
    let h = r.height;
    if sz == Size::Xs || h < 16 {
        return hub_compact(cx, buf, r, focus);
    }
    let tile_h: u16 = if sz == Size::L && h >= 40 { 5 } else { 4 };
    let map_h = (h * 42 / 100).max(7);
    let strip = Rect::new(r.x, r.y + map_h + 1, r.width, tile_h);
    let band = Rect::new(r.x, r.y + 1, r.width, map_h);
    let sess = Rect::new(
        r.x,
        strip.bottom() + 1,
        r.width,
        r.bottom().saturating_sub(strip.bottom() + 1),
    );

    // Network.
    let ids = map::order(cx, band);
    let sel_id = ids
        .get(cx.c.selected % ids.len().max(1))
        .cloned()
        .unwrap_or_default();
    let on_graph = focus == Pane::Graph;
    let lw = cx.label(buf, r.x + 1, r.y, 30, "NETWORK", on_graph);
    mesh_legend(cx, buf, r.x + lw + 4, r.y, r.right().saturating_sub(2));
    map::draw(cx, buf, band, map::Sel::Node(&sel_id, on_graph));

    // Devices strip.
    let on_rail = focus == Pane::Rail;
    cx.label(buf, r.x + 1, strip.y - 1, 30, "DEVICES", on_rail);
    let n = s.devices.len().max(1) as u16;
    let gap = 1;
    let inner = r.width.saturating_sub(2);
    let min_w = if sz == Size::L { 22 } else { 16 };
    let tw = ((inner + gap) / n).saturating_sub(gap).clamp(min_w, 30);
    let fit = ((inner + gap) / (tw + gap)).max(1) as usize;
    let sel_dev = if on_rail {
        cx.c.selected % s.devices.len().max(1)
    } else {
        s.devices.iter().position(|d| d.id == sel_id).unwrap_or(0)
    };
    let first = sel_dev.saturating_sub(fit.saturating_sub(1));
    let tw = if fit >= s.devices.len() {
        ((inner + gap) / n).saturating_sub(gap).clamp(min_w, 34)
    } else {
        tw
    };
    for (i, d) in s.devices.iter().enumerate().skip(first).take(fit) {
        let x = r.x + 1 + (i - first) as u16 * (tw + gap);
        let pick = if i == sel_dev { Some(on_rail) } else { None };
        let pick = if on_graph && d.id == sel_id {
            Some(false)
        } else {
            pick
        };
        tiles::device_tile(cx, buf, Rect::new(x, strip.y, tw, tile_h), d, pick);
    }
    if s.devices.len() > fit {
        let t = format!(
            "{}/{} {}",
            (first + fit).min(s.devices.len()),
            s.devices.len(),
            cx.g("›", ">")
        );
        cx.put_r(
            buf,
            r.right().saturating_sub(1),
            strip.y - 1,
            &t,
            cx.st(Tok::Muted),
        );
    }

    // Sessions.
    let on_sess = focus == Pane::Sessions;
    cx.label(
        buf,
        r.x + 1,
        sess.y.saturating_sub(1),
        30,
        "SESSIONS",
        on_sess,
    );
    let list = cx.by_attention();
    let grid = Rect::new(r.x + 1, sess.y, r.width.saturating_sub(2), sess.height);
    let shown = thumbs_grid(
        cx,
        buf,
        grid,
        &list,
        if on_sess { Some(cx.c.selected) } else { None },
        false,
    );
    let noun = if list.len() == 1 {
        "session"
    } else {
        "sessions"
    };
    let hint = if shown < list.len() {
        format!("{shown} of {} {noun}  {} more", list.len(), cx.g("›", ">"))
    } else {
        format!("{} {noun}", list.len())
    };
    let hy = sess.y.saturating_sub(1);
    let w = cx.put_r(
        buf,
        r.right().saturating_sub(1),
        hy,
        &hint,
        cx.st(Tok::Muted),
    );
    cx.hit(
        Rect::new(r.right().saturating_sub(1 + w), hy, w, 1),
        "palette:sessions",
    );
}

fn mesh_legend(cx: &Cx, buf: &mut Buffer, mut x: u16, y: u16, right: u16) {
    for (i, m) in cx.s.meshes.iter().enumerate() {
        let tok = if i == 0 { Tok::Mesh } else { Tok::Id(6) };
        let up = m
            .members
            .iter()
            .filter(|id| cx.s.device(id).is_some_and(|d| cx.up(d)))
            .count();
        let text = format!("{} {} {}/{}", cx.g("◆", "*"), m.name, up, m.members.len());
        let w = super::width(&text, cx.uni());
        if x + w + 2 > right {
            break;
        }
        let x0 = x;
        x += cx.put(buf, x, y, w, &text, cx.ink(tok, 0.95));
        if !m.healthy {
            x += cx.put(buf, x, y, 2, cx.g(" ▲", " !"), cx.st(Tok::Warn));
        }
        cx.hit(Rect::new(x0, y, x - x0, 1), format!("mesh:{}", m.id));
        x += 3;
    }
    // Legend for flows at wide sizes.
    if cx.size() >= Size::M {
        let lan = format!("{} lan", cx.g("⠒⠒", ".."));
        let w = super::width(&lan, cx.uni());
        if x + w + 16 < right {
            x += cx.put(buf, x, y, w, &lan, cx.ink(Tok::Lan, 0.9)) + 3;
            let t = if cx.uni() {
                "⠒⠶⣿ flow = allowed direction"
            } else {
                ".-= flow = allowed direction"
            };
            cx.put(
                buf,
                x,
                y,
                right.saturating_sub(x),
                t,
                cx.ink(Tok::Muted, 0.8),
            );
        }
    }
}

/// Phone / short hub: compact schematic, then one row per device.
fn hub_compact(cx: &Cx, buf: &mut Buffer, r: Rect, focus: Pane) {
    let s = cx.s;
    let map_h = (r.height * 2 / 5).clamp(5, 9);
    let band = Rect::new(r.x, r.y, r.width, map_h);
    let ids = map::order(cx, band);
    let sel_id = ids
        .get(cx.c.selected % ids.len().max(1))
        .cloned()
        .unwrap_or_default();
    let on_graph = focus == Pane::Graph;
    map::draw(cx, buf, band, map::Sel::Node(&sel_id, on_graph));
    let mut y = band.bottom();
    let on_rail = focus == Pane::Rail;
    let rows = r.bottom().saturating_sub(y + 1) as usize;
    cx.label(buf, r.x + 1, y, r.width - 2, "DEVICES", on_rail);
    let sess_n = s.sessions.len();
    cx.put_r(
        buf,
        r.right() - 1,
        y,
        &format!("{sess_n} sessions"),
        cx.st(Tok::Muted),
    );
    y += 1;
    let sel = if on_rail {
        cx.c.selected % s.devices.len().max(1)
    } else {
        s.devices.iter().position(|d| d.id == sel_id).unwrap_or(0)
    };
    let first = sel.saturating_sub(rows.saturating_sub(1));
    for (i, d) in s.devices.iter().enumerate().skip(first).take(rows) {
        let pick = if i == sel {
            Some(on_rail || (on_graph && d.id == sel_id) && false)
        } else {
            None
        };
        let pick = if i == sel && on_graph {
            Some(false)
        } else {
            pick
        };
        tiles::device_row(cx, buf, Rect::new(r.x, y, r.width, 1), d, pick);
        y += 1;
    }
    if s.devices.len() > rows + first {
        cx.put_r(
            buf,
            r.right() - 1,
            r.bottom() - 1,
            &format!("+{} more", s.devices.len() - rows - first),
            cx.st(Tok::Muted),
        );
    }
}

/// Grid of session thumbnails; `slot_keys` labels them with their slot key. Columns are
/// balanced so the last row is not a lone tile. Returns how many tiles were shown.
fn thumbs_grid(
    cx: &Cx,
    buf: &mut Buffer,
    r: Rect,
    list: &[&crate::ui::lab::LabSession],
    sel: Option<usize>,
    slot_keys: bool,
) -> usize {
    if list.is_empty() || r.height < 3 {
        if r.height > 0 {
            cx.put(
                buf,
                r.x + 1,
                r.y,
                r.width,
                "no sessions yet",
                cx.st(Tok::Muted),
            );
        }
        return 0;
    }
    let min_w = match cx.size() {
        Size::Xs => r.width,
        Size::S => 24,
        Size::M => 34,
        Size::L => 40,
    };
    let max_cols = ((r.width + 1) / (min_w + 1)).max(1) as usize;
    let max_rows = ((r.height + 1) / 7).max(1) as usize;
    let rows = list.len().div_ceil(max_cols).min(max_rows);
    let cols = list.len().div_ceil(rows).min(max_cols).max(1);
    let (cols, rows) = (cols as u16, rows as u16);
    let tw = (r.width + 1) / cols - 1;
    let th = ((r.height + 1) / rows).saturating_sub(1).min(tw / 2 + 4);
    let shown = (rows * cols) as usize;
    for (i, x) in list.iter().enumerate().take(shown) {
        let (ci, ri) = (i as u16 % cols, i as u16 / cols);
        let tr = Rect::new(r.x + ci * (tw + 1), r.y + ri * (th + 1), tw, th);
        let pick = sel.and_then(|s| (s % list.len() == i).then_some(true));
        tiles::thumb(cx, buf, tr, x, pick, slot_keys);
    }
    shown.min(list.len())
}

// --------------------------------------------------------------------------- device

fn device(cx: &Cx, buf: &mut Buffer, r: Rect, d: &LabDevice) {
    let s = cx.s;
    let sz = cx.size();
    let head_h: u16 = match sz {
        Size::Xs => 3,
        Size::S => 4,
        _ => 5,
    };
    let head = Rect::new(r.x + 1, r.y + 1, r.width.saturating_sub(2), head_h);
    device_header(cx, buf, head, d);
    let side_w = if sz >= Size::M {
        (r.width / 4).clamp(30, 44)
    } else {
        0
    };
    let main = Rect::new(
        r.x + 1,
        head.bottom() + 2,
        r.width
            .saturating_sub(2 + side_w + if side_w > 0 { 2 } else { 0 }),
        r.bottom().saturating_sub(head.bottom() + 2),
    );
    let list: Vec<&crate::ui::lab::LabSession> = s.sessions_on(&d.id).collect();
    cx.label(buf, main.x, main.y.saturating_sub(1), 30, "SESSIONS", true);
    let keys: String = list
        .iter()
        .filter_map(|x| cx.slot(x))
        .map(|c| c.to_string())
        .collect::<Vec<_>>()
        .join(" ");
    if !keys.is_empty() && sz >= Size::S {
        cx.put_r(
            buf,
            main.right(),
            main.y.saturating_sub(1),
            &format!("{keys}  attach"),
            cx.st(Tok::Muted),
        );
    }
    if sz == Size::Xs {
        device_sessions_rows(cx, buf, main, &list);
    } else {
        // Thumbnails plus a "new" tile.
        let n = list.len() as u16 + 1;
        let target = if sz == Size::S { 30 } else { 40 };
        let cols = ((main.width + 1) / (target + 1)).clamp(1, n.min(3));
        let tw = (main.width + 1) / cols - 1;
        let rows = n.div_ceil(cols);
        let th = ((main.height + 1) / rows.max(1))
            .saturating_sub(1)
            .clamp(4, tw / 2 + 2);
        let sel = cx.c.selected % n as usize;
        for i in 0..n {
            let tr = Rect::new(
                main.x + (i % cols) * (tw + 1),
                main.y + (i / cols) * (th + 1),
                tw,
                th,
            );
            if tr.bottom() > main.bottom() {
                break;
            }
            match list.get(i as usize) {
                Some(x) => tiles::thumb(
                    cx,
                    buf,
                    tr,
                    x,
                    (usize::from(i) == sel).then_some(true),
                    true,
                ),
                None => new_tile(cx, buf, tr, d, usize::from(i) == sel),
            }
        }
    }
    if side_w > 0 {
        let side = Rect::new(
            r.right().saturating_sub(side_w + 1),
            head.bottom() + 1,
            side_w,
            r.bottom().saturating_sub(head.bottom() + 1),
        );
        links(cx, buf, side, d);
    }
}

fn device_header(cx: &Cx, buf: &mut Buffer, r: Rect, d: &LabDevice) {
    let (g, gst) = cx.status(d.status);
    let mut x = r.x;
    x += cx.dchip(buf, x, r.y, d, false);
    x += 1;
    x += cx.put(
        buf,
        x,
        r.y,
        r.width / 2,
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
    cx.put(
        buf,
        x,
        r.y,
        r.right().saturating_sub(x),
        &ev,
        cx.st(Tok::Muted),
    );
    let info = format!("{} · {}@{} · {}", d.os, d.account, d.host, kind_word(d));
    let mut y = r.y + 1;
    let left_w = if cx.size() >= Size::M {
        46.min(r.width / 2)
    } else {
        r.width
    };
    cx.put(
        buf,
        r.x,
        y,
        left_w,
        &fmt::fit(&info, left_w as usize, cx.uni()),
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
                &format!("{} {}  ", cx.g("◆", "*"), m.name),
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
        let mx = r.x + left_w + 4;
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
        cx.put_r(
            buf,
            mx + cw,
            my,
            &format!("load {:.1} · {} cores", m.load[0], m.cores),
            cx.st(Tok::Muted),
        );
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
        let w = mw.saturating_sub(10 + 14);
        tiles::chart(cx, buf, Rect::new(mx + 9, my, w, 1), &m.cpu_hist, None);
        let rest = format!("mem {:.0}%", m.mem * 100.0);
        cx.put(buf, mx + 10 + w, my, 14, &rest, cx.st(Tok::Info));
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

/// Ghost tile for starting a session: a dashed outline with one key per provider.
fn new_tile(cx: &Cx, buf: &mut Buffer, r: Rect, d: &LabDevice, picked: bool) {
    let st = if picked {
        cx.st(Tok::AccentBg)
    } else {
        Style::default()
    };
    cx.fill(buf, r, st);
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
    let opts = [
        ("c", "claude", Tok::Claude),
        ("x", "codex", Tok::Codex),
        ("n", "shell", Tok::Shell),
    ];
    let inner = r.height.saturating_sub(2);
    let stacked = inner >= 5;
    let block_h = if stacked { 2 + opts.len() as u16 } else { 2 };
    let mut y = r.y + 1 + inner.saturating_sub(block_h) / 2;
    let x = r.x + 3;
    let title = format!("new on {}", d.name);
    cx.put(
        buf,
        x,
        y,
        r.right().saturating_sub(x + 2),
        &title,
        cx.ink(Tok::Fg, 0.9).add_modifier(Modifier::BOLD),
    );
    y += if stacked { 2 } else { 1 };
    let mut ox = x;
    for (k, label, tok) in opts {
        if y >= r.bottom() - 1 || (!stacked && ox + 10 > r.right()) {
            break;
        }
        let x0 = ox;
        ox += cx.put(buf, ox, y, 3, &format!(" {k} "), cx.key_style());
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
            ox += 3;
        }
    }
    cx.hit(r, format!("new:{}", d.id));
}

fn device_sessions_rows(cx: &Cx, buf: &mut Buffer, r: Rect, list: &[&crate::ui::lab::LabSession]) {
    let mut y = r.y;
    let sel = cx.c.selected % (list.len() + 1);
    for (i, x) in list.iter().enumerate() {
        if y + 1 >= r.bottom() {
            break;
        }
        let row = Rect::new(r.x, y, r.width, 2);
        let picked = i == sel;
        cx.fill(
            buf,
            row,
            if picked {
                cx.st(Tok::AccentBg)
            } else {
                cx.surface()
            },
        );
        let mut xx = r.x;
        if picked {
            cx.put(buf, xx, y, 1, cx.g("▌", ">"), cx.st(Tok::Accent));
            cx.put(buf, xx, y + 1, 1, cx.g("▌", ">"), cx.st(Tok::Accent));
        }
        xx += 1;
        let k = cx.slot(x).unwrap_or(' ');
        xx += cx.put(buf, xx, y, 3, &format!(" {k} "), cx.key_style());
        xx += 1;
        let (g, word, st) = cx.sstate(x);
        let room = r.right().saturating_sub(xx + 3);
        cx.put(
            buf,
            xx,
            y,
            room,
            &fmt::fit(&x.title, room as usize, cx.uni()),
            cx.ink(Tok::Fg, 1.0).add_modifier(Modifier::BOLD),
        );
        cx.put(buf, r.right() - 2, y, 1, g, st);
        let (p, ptok) = cx.prov(x);
        let mut cx2 = r.x + 5;
        cx2 += cx.put(buf, cx2, y + 1, 7, p, cx.st(ptok));
        cx2 += 1;
        let cap = if cx.size() == Size::Xs {
            word.to_string() + " · " + &tiles::caption(x)
        } else {
            tiles::caption(x)
        };
        let room = r.right().saturating_sub(cx2 + 1);
        cx.put(
            buf,
            cx2,
            y + 1,
            room,
            &fmt::fit(&cap, room as usize, cx.uni()),
            cx.st(Tok::Muted),
        );
        cx.hit(row, format!("session:{}:{}", x.device, x.id));
        y += 3;
    }
    if y < r.bottom() {
        let items = [
            ("c", "claude", "new-claude"),
            ("x", "codex", "new-codex"),
            ("n", "shell", "new-shell"),
        ];
        cx.chips(buf, r.x + 1, y, r.right(), &items);
    }
}

fn links(cx: &Cx, buf: &mut Buffer, r: Rect, d: &LabDevice) {
    cx.label(buf, r.x, r.y, r.width, "LINKS", false);
    cx.put_r(buf, r.right(), r.y, "e edit", cx.st(Tok::Muted));
    let mut y = r.y + 1;
    for e in cx.s.edges_of(&d.id) {
        if y + 2 > r.bottom() {
            break;
        }
        let peer_id = if e.from == d.id { &e.to } else { &e.from };
        let Some(p) = cx.s.device(peer_id) else {
            continue;
        };
        let mut x = r.x;
        x += cx.dchip(buf, x, y, p, false);
        x += 1;
        let pol = map::policy(cx, e, e.from == d.id);
        let pw = super::width(&pol, cx.uni());
        let room = r.right().saturating_sub(x + pw + 2);
        x += cx.put(
            buf,
            x,
            y,
            room,
            &fmt::fit(&p.name, room as usize, cx.uni()),
            cx.ink(Tok::Fg, if cx.up(p) { 1.0 } else { 0.6 }),
        );
        let _ = x;
        let pst = if e.forward == Access::Denied && e.reverse == Access::Denied {
            cx.st(Tok::Muted)
        } else {
            cx.ink(Tok::Fg, 0.9)
        };
        cx.put_r(buf, r.right(), y, &pol, pst);
        let (kind, tok) = edge_kind(cx, e);
        let rtt = e
            .rtt_ms
            .map(super::ms_text)
            .unwrap_or_else(|| "down".into());
        let rw = rtt.len() as u16;
        let mut x2 = r.x + 4;
        // The network name only when the rtt trend still gets a readable width.
        let kw = super::width(&kind, cx.uni());
        let label = if r.width >= 4 + kw + 1 + 10 + rw + 1 {
            kind.as_str()
        } else {
            kind.split(' ').next().unwrap_or("")
        };
        x2 += cx.put(buf, x2, y + 1, 18, label, cx.ink(tok, 0.9));
        x2 += 1;
        let sw = r.right().saturating_sub(x2 + rw + 1);
        tiles::spark(cx, buf, x2, y + 1, sw, &e.rtt_hist, cx.ink(tok, 0.7));
        cx.put_r(
            buf,
            r.right(),
            y + 1,
            &rtt,
            if e.up {
                cx.st(Tok::Muted)
            } else {
                cx.st(Tok::Err)
            },
        );
        cx.hit(Rect::new(r.x, y, r.width, 2), format!("edge:{}", e.id));
        y += 3;
    }
}

fn edge_kind(cx: &Cx, e: &LabEdge) -> (String, Tok) {
    match &e.kind {
        EdgeKind::Mesh(m) => {
            let i = cx.s.meshes.iter().position(|x| &x.id == m).unwrap_or(0);
            (
                format!("{} {m}", cx.g("◆", "*")),
                if i == 0 { Tok::Mesh } else { Tok::Id(6) },
            )
        }
        EdgeKind::Lan => ("lan".into(), Tok::Lan),
        EdgeKind::Direct => ("direct ssh".into(), Tok::Info),
        EdgeKind::Jump(v) => (format!("{} via {v}", cx.g("↳", "^")), Tok::Muted),
    }
}

// ----------------------------------------------------------------------------- peek

fn peek(cx: &Cx, buf: &mut Buffer, r: Rect, x: &crate::ui::lab::LabSession) {
    let sz = cx.size();
    let strip_w = if sz >= Size::M {
        (r.width / 5).clamp(24, 34)
    } else {
        0
    };
    let main = Rect::new(
        r.x + 1,
        r.y,
        r.width
            .saturating_sub(2 + strip_w + if strip_w > 0 { 2 } else { 0 }),
        r.height,
    );
    // Meta line.
    let (g, word, sst) = cx.sstate(x);
    let (p, ptok) = cx.prov(x);
    let mut mx = main.x;
    if let Some(k) = cx.slot(x) {
        mx += cx.put(buf, mx, main.y, 3, &format!(" {k} "), cx.key_style());
        mx += 1;
    }
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
    let room = main.right().saturating_sub(mx + 14);
    mx += cx.put(
        buf,
        mx,
        main.y,
        room,
        &fmt::fit(&meta, room as usize, cx.uni()),
        cx.st(Tok::Muted),
    );
    let _ = mx;
    if sz >= Size::S {
        let v: Vec<Option<f32>> = x.activity.iter().map(|v| Some(*v)).collect();
        tiles::spark(
            cx,
            buf,
            main.right().saturating_sub(12),
            main.y,
            12,
            &v,
            cx.ink(Tok::Accent, 0.8),
        );
    }
    // The screen, 1:1, on a deeper surface.
    let scr = Rect::new(
        main.x,
        main.y + 2,
        main.width,
        main.height.saturating_sub(2),
    );
    if cx.has_surface() {
        let base = if cx.dark() {
            (0x0c, 0x0e, 0x11)
        } else {
            (0xff, 0xff, 0xff)
        };
        cx.fill(buf, scr, cx.rgb_bg(base));
    }
    let inner = inset(scr, 2, 1);
    let lines = crate::ui::lab::ansi_lines(&x.screen, x.screen_cols);
    for (i, l) in lines.iter().take(inner.height as usize).enumerate() {
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
    // Filmstrip of the other sessions.
    if strip_w > 0 {
        let sx = r.right().saturating_sub(strip_w + 1);
        cx.label(buf, sx, r.y, strip_w, "OTHERS", false);
        let others: Vec<_> = cx
            .by_attention()
            .into_iter()
            .filter(|o| o.id != x.id)
            .collect();
        let th = 7u16;
        let mut y = r.y + 2;
        for o in others {
            if y + th > r.bottom() {
                break;
            }
            tiles::thumb(cx, buf, Rect::new(sx, y, strip_w, th), o, None, false);
            y += th + 1;
        }
    }
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
                    cx.g("▾ ", "v ")
                } else {
                    cx.g("▸ ", "> ")
                },
                cx.st(Tok::Dir).add_modifier(Modifier::BOLD),
            ),
            FileKind::Symlink => (cx.g("↪ ", "@ "), cx.st(Tok::Info)),
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
        x += cx.put(
            buf,
            x,
            y,
            room,
            &fmt::fit(&name, room as usize, cx.uni()),
            nst,
        );
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
        &fmt::fit(&meta, r.width.saturating_sub(12) as usize, cx.uni()),
        cx.st(Tok::Muted),
    );
    let body = Rect::new(r.x, r.y + 1, r.width, r.height.saturating_sub(1));
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
    let w = 64.min(a.width.saturating_sub(4));
    let h = 18.min(a.height.saturating_sub(2));
    let r = Rect::new(
        a.x + (a.width - w) / 2,
        a.y + 1 + (a.height.saturating_sub(h + 2)) / 3,
        w,
        h,
    );
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
                format!("{}{slot}", d.hotkey),
                format!("session:{}", x.id),
            ));
        }
        let verbs = [
            ("new claude session on ", "c"),
            ("files on ", "f"),
            ("edit link to ", "e"),
        ];
        for (v, k) in verbs {
            rows.push((
                "action".into(),
                vec![
                    (format!("{} ", cx.g("›", ">")), cx.st(Tok::Accent)),
                    (format!("{v}{}", d.name), cx.ink(Tok::Fg, 0.95)),
                ],
                format!("{} {k}", d.hotkey),
                format!("cmd:{v}{}", d.id),
            ));
        }
    }
    for (name, k) in [
        ("invite a device (join code)", "/ inv"),
        ("keys and secrets", "/ key"),
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
        for (t, st) in spans {
            let room = inner.right().saturating_sub(x + key.len() as u16 + 2);
            x += cx.put(buf, x, yy, room, t, *st);
        }
        cx.put_r(buf, inner.right(), yy, &format!(" {key} "), cx.key_style());
        cx.hit(row, act.clone());
        yy += 1;
    }
}

// -------------------------------------------------------------------------- secrets

fn secrets(cx: &Cx, buf: &mut Buffer, r: Rect) {
    let w = r.width.saturating_sub(4).min(96);
    let x0 = r.x + (r.width - w) / 2;
    let mut top = r.y + 1;
    cx.label(buf, x0, r.y, w, "KEYS", true);
    let note = if w >= 60 {
        "stored in your keyring · never shown in full"
    } else {
        "never shown in full"
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
            (cx.g("○", "o"), cx.st(Tok::Muted))
        } else {
            (cx.g("●", "*"), cx.st(Tok::Ok))
        };
        cx.put(buf, ix, y, 1, g, gst);
        let mut x = ix + 2;
        x += cx.put(
            buf,
            x,
            y,
            w / 2,
            &s.label,
            cx.ink(Tok::Fg, 1.0).add_modifier(Modifier::BOLD),
        );
        if !compact {
            cx.put(buf, x + 2, y, w / 3, &s.key, cx.ink(Tok::Muted, 0.8));
        }
        let right = card.right().saturating_sub(2);
        if missing {
            let label = if s.key.starts_with("cloudflare") {
                "connect"
            } else {
                "add"
            };
            let t = format!(" {} {label} ", cx.g("⏎", "ret"));
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
                ix + 2,
                y + 1,
                w.saturating_sub(6),
                "not set",
                cx.st(Tok::Muted),
            );
        } else {
            let v = s.masked.clone().unwrap_or_default();
            let vw = if compact {
                right.saturating_sub(ix + 2 + 16)
            } else {
                30
            };
            cx.put(buf, ix + 2, y + 1, vw, &v, cx.ink(Tok::Fg, 0.9));
            let age = s.updated_secs.map(fmt::age).unwrap_or_default();
            let src = format!("{} · {age} ago", s.source);
            if compact {
                cx.put_r(buf, right, y + 1, &src, cx.st(Tok::Muted));
            } else {
                cx.put_r(buf, right, y, &src, cx.st(Tok::Muted));
            }
        }
        if !s.used_by.is_empty() && card_h >= 3 {
            let used = format!("used by {}", s.used_by.join(", "));
            let ux = if compact { ix + 2 } else { ix + 34 };
            let uy = if compact { y + 2 } else { y + 1 };
            cx.put(
                buf,
                ux,
                uy,
                right.saturating_sub(ux),
                &fmt::fit(&used, right.saturating_sub(ux) as usize, cx.uni()),
                cx.ink(Tok::Muted, 0.85),
            );
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
    let mut y = r.y + if small { 1 } else { 2 };
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
    cx.put(
        buf,
        x0,
        y,
        w,
        &fmt::fit(sub, w as usize, cx.uni()),
        cx.st(Tok::Muted),
    );
    y += 2;
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
    let btn = format!("  {}  Connect Cloudflare  ", cx.g("⏎", "ret"));
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
        let mut x = x0;
        x += cx.put(buf, x, y, 3, &format!(" {n} "), cx.key_style());
        x += 1;
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
    if y < r.bottom() {
        let mut x = x0;
        x += cx.put(buf, x, y, 3, " j ", cx.key_style());
        x += 1;
        x += cx.put(buf, x, y, 22, "I have a join code", cx.ink(Tok::Fg, 0.9));
        cx.hit(Rect::new(x0, y, x - x0, 1), "join");
        if cx.size() >= Size::S {
            let x = x + 4;
            let xx = x + cx.put(buf, x, y, 5, " esc ", cx.key_style()) + 1;
            cx.put(buf, xx, y, 24, "plain SSH for now", cx.ink(Tok::Fg, 0.9));
        }
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
        let line = Rect::new(tx, y, tw, 1);
        if cx.has_surface() {
            cx.fill(buf, line, cx.surface());
        }
        let one = fmt::fit(&inv.oneliner, tw.saturating_sub(2) as usize, cx.uni());
        cx.put(buf, tx + 1, y, tw.saturating_sub(2), &one, cx.st(Tok::Lan));
        cx.hit(line, "copy-cmd");
        y += 2;
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
    let Some(e) = cx.s.edge(id).or(cx.s.edges.first()) else {
        return;
    };
    let sheet_h: u16 = if cx.size() == Size::Xs { 11 } else { 12 }.min(r.height);
    let band = Rect::new(r.x, r.y + 1, r.width, r.height.saturating_sub(sheet_h + 1));
    if band.height >= 5 {
        map::draw(cx, buf, band, map::Sel::Edge(&e.id));
    }
    let sheet = Rect::new(r.x, r.bottom().saturating_sub(sheet_h), r.width, sheet_h);
    panel(cx, buf, sheet);
    let inner = inset(sheet, 2, 1);
    let (Some(a), Some(b)) = (cx.s.device(&e.from), cx.s.device(&e.to)) else {
        return;
    };
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
    let lanes = [(a, b, e.forward), (b, a, e.reverse)];
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
    // Consequence line.
    let cy = inner.y + 6;
    if cy < inner.bottom() {
        let msg = if e.forward == Access::Key && e.reverse == Access::Key {
            format!(
                "Switching {} → {} off makes this link one-way; 0 sessions affected.",
                b.name, a.name
            )
        } else {
            "Changes apply on Enter; nothing reconnects until you open a session.".into()
        };
        let room = inner.width;
        cx.put(
            buf,
            inner.x,
            cy,
            room,
            &fmt::fit(&msg, room as usize, cx.uni()),
            cx.ink(Tok::Info, 0.95),
        );
    }
    let ry = inner.y + 8;
    if ry < inner.bottom() && cx.size() >= Size::S {
        let label = "rtt 32 samples";
        cx.put(buf, inner.x, ry, 15, label, cx.st(Tok::Muted));
        tiles::spark(
            cx,
            buf,
            inner.x + 16,
            ry,
            32.min(inner.width.saturating_sub(16)),
            &e.rtt_hist,
            cx.ink(tok, 0.9),
        );
    }
}

// ----------------------------------------------------------------------------- help

/// Keyboard map: each key shows its role, coloured by layer.
fn help(cx: &Cx, buf: &mut Buffer, r: Rect) {
    let rows = ["qwertyuiop", "asdfghjkl;", "zxcvbnm,./"];
    let kw: u16 = if r.width >= 120 {
        9
    } else if r.width >= 80 {
        7
    } else {
        0
    };
    let devs: Vec<&LabDevice> = cx.s.devices.iter().collect();
    let role = |c: char| -> (String, Style, String) {
        if let Some(d) = devs.iter().find(|d| d.hotkey == c) {
            let nm: String = d.name.chars().take(usize::from(kw.max(4)) - 1).collect();
            return (nm, cx.chip_style(cx.s.hue(&d.id)), "device".into());
        }
        let (label, tok) = match c {
            '/' => ("find", Tok::Accent),
            _ => ("", Tok::Faint),
        };
        (label.into(), cx.st(tok), String::new())
    };
    if kw == 0 {
        // Phone: a compact list.
        let mut y = r.y + 1;
        let items: Vec<(String, String)> = vec![
            ("letter".into(), "open that device".into()),
            ("then a s d".into(), "attach a session".into()),
            ("then c x n".into(), "new claude / codex / shell".into()),
            ("then f / e".into(), "files / links".into()),
            ("⏎ / ␣".into(), "open / peek".into()),
            ("tab".into(), "next zone".into()),
            ("/".into(), "find anything".into()),
            ("esc".into(), "back".into()),
            ("ctrl \\".into(), "detach from a session".into()),
        ];
        for (k, v) in items {
            if y >= r.bottom() {
                break;
            }
            cx.put(buf, r.x + 1, y, 12, &format!(" {k} "), cx.key_style());
            cx.put(
                buf,
                r.x + 14,
                y,
                r.width.saturating_sub(15),
                &v,
                cx.ink(Tok::Fg, 0.9),
            );
            y += 1;
        }
        return;
    }
    let total_w = kw * 10 + 9;
    let x0 = r.x + (r.width.saturating_sub(total_w + 3)) / 2;
    let mut y = r.y + 1;
    cx.label(buf, x0, y, total_w, "HUB LAYER", true);
    cx.put_r(
        buf,
        x0 + total_w + 3,
        y,
        "letters jump to devices",
        cx.st(Tok::Muted),
    );
    y += 2;
    let key_rows = |y0: u16, buf: &mut Buffer, f: &dyn Fn(char) -> (String, Style, String)| {
        for (ri, row) in rows.iter().enumerate() {
            let y = y0 + ri as u16 * 3;
            let off = ri as u16 * (kw / 3).max(1);
            for (ci, c) in row.chars().enumerate() {
                let x = x0 + off + ci as u16 * (kw + 1);
                let (label, st, _) = f(c);
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
                    cx.fill(
                        buf,
                        kr,
                        if cx.has_surface() || cx.c.has_color() {
                            bst
                        } else {
                            bst
                        },
                    );
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
    key_rows(y, buf, &role);
    y += 9;
    if y + 10 < r.bottom() {
        cx.label(buf, x0, y, total_w, "DEVICE LAYER", false);
        cx.put_r(
            buf,
            x0 + total_w + 3,
            y,
            "after a device letter",
            cx.st(Tok::Muted),
        );
        y += 2;
        let dev_role = |c: char| -> (String, Style, String) {
            let ks = cx.key_style();
            let slot = SLOTS.iter().position(|s| *s == c);
            let acc = if cx.has_surface() {
                cx.wash(Tok::Accent, 0.25).patch(cx.ink(Tok::Fg, 1.0))
            } else {
                ks
            };
            match (c, slot) {
                (_, Some(i)) => (format!("ses {}", i + 1), acc, String::new()),
                ('n', _) => ("+shell".into(), ks, String::new()),
                ('f', _) => ("files".into(), ks, String::new()),
                ('e', _) => ("links".into(), ks, String::new()),
                ('c', _) => (
                    "+claude".into(),
                    cx.wash(Tok::Claude, 0.25).patch(cx.ink(Tok::Fg, 1.0)),
                    String::new(),
                ),
                ('x', _) => (
                    "+codex".into(),
                    cx.wash(Tok::Codex, 0.25).patch(cx.ink(Tok::Fg, 1.0)),
                    String::new(),
                ),
                ('/', _) => ("find".into(), ks, String::new()),
                _ => (String::new(), Style::default(), String::new()),
            }
        };
        key_rows(y, buf, &dev_role);
        y += 9;
    }
    if y + 1 < r.bottom() {
        let mut x = x0;
        for (k, v) in [
            ("⏎", "open/attach"),
            ("␣", "peek"),
            ("tab", "zone"),
            ("esc", "back"),
            ("ctrl \\", "detach"),
        ] {
            let k = if cx.uni() {
                k
            } else {
                match k {
                    "⏎" => "ret",
                    "␣" => "spc",
                    o => o,
                }
            };
            x += cx.put(buf, x, y, 10, &format!(" {k} "), cx.key_style()) + 1;
            x += cx.put(buf, x, y, 14, v, cx.ink(Tok::Fg, 0.85)) + 3;
        }
    }
    let _ = DeviceStatus::Online;
    let _ = centered;
}
