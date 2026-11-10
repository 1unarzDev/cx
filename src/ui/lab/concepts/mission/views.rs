//! Screens: hub, device, session peek, files, preview, palette, keys, onboarding, join,
//! link policy and help. Each is a plain function over the shared `Cx`.
use super::{centered, fit, fmt, inset, map, mix, tiles, Cx, Size, SLOTS};
use crate::ui::lab::{
    caps::Pane, Access, DeviceStatus, EdgeKind, FileKind, LabDevice, LabEdge, LabSession, LabView,
    PreviewKind, SessionState, Tok,
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
    let spare = u16::from(cx.size() == Size::Xs);
    let body = Rect::new(a.x, a.y + 1, a.width, a.height.saturating_sub(2 + spare));
    let foot = Rect::new(a.x, a.bottom() - 1, a.width, 1);
    match view {
        LabView::Hub => {
            top(cx, buf, &[], true);
            hub(cx, buf, body);
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
            let slots: String = SLOTS[..n.clamp(1, 3)]
                .iter()
                .map(|c| c.to_string())
                .collect::<Vec<_>>()
                .join(" ");
            let mut items: Keys = Vec::new();
            if cx.size() == Size::Xs {
                if n > 0 {
                    items.push(("⏎", "attach", "attach"));
                }
                items.extend([
                    ("f", "files", "files"),
                    ("e", "links", "links"),
                    ("esc", "", "back"),
                ]);
            } else {
                if n > 0 {
                    items.extend([(slots.as_str(), "attach", "attach"), ("␣", "peek", "peek")]);
                }
                items.extend([("f", "files", "files"), ("e", "links", "links")]);
                if n > 0 {
                    items.push(("⌫", "stop", "stop"));
                }
                items.extend([("/", "find", "palette"), ("esc", "hub", "back")]);
            }
            footer(cx, buf, foot, items);
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
                vec![
                    ("⏎", "attach", "attach"),
                    ("␣", "close", "back"),
                    ("⌫", "stop", "stop"),
                    ("/", "find", "palette"),
                    ("esc", "hub", "hub"),
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
            top(cx, buf, &crumbs, true);
            if matches!(view, LabView::Files) {
                files(cx, buf, body);
                footer(
                    cx,
                    buf,
                    foot,
                    vec![
                        ("⏎", "open", "open"),
                        ("/", "filter", "filter"),
                        ("t", "send to", "send"),
                        ("y", "copy", "copy"),
                        ("␣", "preview", "preview"),
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
                        ("`", "hub", "hub"),
                    ],
                );
            }
        }
        LabView::Palette(q) => {
            top(cx, buf, &[], true);
            hub(cx, buf, body);
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
                    ("⏎", "edit", "edit"),
                    ("y", "copy", "copy"),
                    ("r", "reveal 10s", "reveal"),
                    ("esc", "hub", "back"),
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
                ("k p o", "key pwd off", "policy"),
                ("u", "undo", "undo"),
                ("esc", "done", "back"),
            ];
            if cx.size() > Size::Xs {
                items.insert(0, ("↑↓", "direction", "dir"));
            }
            footer(cx, buf, foot, items);
        }
        LabView::Help => {
            top(cx, buf, &[Crumb::Text("keys".into())], true);
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
        x += cx.put(buf, x, y, 3, cx.g(" › ", " > "), cx.ink(Tok::Faint, 1.0));
        let x0 = x;
        match c {
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
            format!("{} {}", cx.g("▲", "!"), fit(&n.text, 44, cx.uni()))
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

fn footer(cx: &Cx, buf: &mut Buffer, r: Rect, items: Keys) {
    let items: Keys = items
        .into_iter()
        .map(|(k, l, a)| {
            let k = if cx.uni() {
                k
            } else {
                match k {
                    "⏎" => "ret",
                    "␣" => "spc",
                    "⌫" => "bksp",
                    "↑↓" => "up/dn",
                    "←→" => "l/r",
                    other => other,
                }
            };
            let l = if l == "key pwd off" && cx.size() == Size::Xs {
                "set"
            } else if cx.size() == Size::Xs && l.len() > 7 {
                l.split(' ').next().unwrap_or(l)
            } else {
                l
            };
            (k, l, a)
        })
        .collect();
    let th = if cx.size() == Size::Xs { 2 } else { 1 };
    cx.chips_h(buf, r.x + 1, r.y, r.right().saturating_sub(1), &items, th);
}

fn hub_keys(cx: &Cx) -> Keys<'static> {
    match cx.size() {
        // Phones tap nodes and rows directly; the footer carries what has no other target.
        Size::Xs => vec![
            ("/", "find", "palette"),
            ("+", "add", "invite"),
            ("K", "keys", "secrets"),
            ("?", "help", "help"),
        ],
        Size::S => vec![
            ("a-z", "device", "device"),
            ("⏎", "open", "open"),
            ("tab", "zone", "zone"),
            ("/", "find", "palette"),
            ("+", "invite", "invite"),
            ("K", "keys", "secrets"),
        ],
        _ => vec![
            ("a-z", "device", "device"),
            ("⏎", "open", "open"),
            ("␣", "peek", "peek"),
            ("tab", "zone", "zone"),
            ("/", "find", "palette"),
            ("+", "invite", "invite"),
            ("K", "keys", "secrets"),
            ("?", "help", "help"),
        ],
    }
}

// ------------------------------------------------------------------------------ hub

/// A zone header: the spaced label, then a faint rule up to `right`. Returns the x after
/// the label so callers can put a legend on the rule.
fn zone(cx: &Cx, buf: &mut Buffer, x: u16, y: u16, right: u16, text: &str, focused: bool) -> u16 {
    let w = cx.label(buf, x, y, right.saturating_sub(x), text, focused);
    cx.rule(buf, x + w + 1, right, y);
    x + w
}

fn hub(cx: &Cx, buf: &mut Buffer, r: Rect) {
    let s = cx.s;
    let focus = cx.c.focus.unwrap_or(Pane::Graph);
    let sz = cx.size();
    let h = r.height;
    if sz == Size::Xs || h < 16 {
        return hub_compact(cx, buf, r, focus);
    }
    // A blank row before each zone header when there is height for it.
    let air = u16::from(h >= 30);
    let tile_h: u16 = if sz == Size::L && h >= 50 {
        5
    } else if h >= 30 {
        4
    } else {
        3
    };
    let map_h = map::want_h(cx).min(h * 2 / 5).max(7);
    let band = Rect::new(r.x, r.y + 1 + air, r.width, map_h);
    let dev_y = band.bottom() + air;
    let strip = Rect::new(r.x + 1, dev_y + 1, r.width.saturating_sub(2), tile_h);
    let ses_y = strip.bottom() + air;
    let grid = Rect::new(
        r.x + 1,
        ses_y + 1,
        r.width.saturating_sub(2),
        r.bottom().saturating_sub(ses_y + 1),
    );
    let right = r.right().saturating_sub(1);

    // Network.
    let ids = map::order(cx, band);
    let sel_id = ids
        .get(cx.c.selected % ids.len().max(1))
        .cloned()
        .unwrap_or_default();
    let on_graph = focus == Pane::Graph;
    let lx = cx.label(buf, r.x + 1, r.y, 30, "NETWORK", on_graph);
    let lx = mesh_legend(cx, buf, r.x + 1 + lx + 3, r.y, right);
    cx.rule(buf, lx + 1, right, r.y);
    map::draw(
        cx,
        buf,
        band,
        &s.viewer,
        3,
        map::Sel::Node(&sel_id, on_graph),
    );

    // Devices strip.
    let on_rail = focus == Pane::Rail;
    let n = s.devices.len().max(1) as u16;
    let gap: u16 = if cx.has_surface() { 1 } else { 2 };
    let min_w = if sz == Size::L { 22 } else { 16 };
    let fit_n = ((strip.width + gap) / (min_w + gap)).clamp(1, n) as usize;
    let tw = ((strip.width + gap) / fit_n as u16)
        .saturating_sub(gap)
        .min(34);
    let sel_dev = if on_rail {
        cx.c.selected % s.devices.len().max(1)
    } else {
        s.devices.iter().position(|d| d.id == sel_id).unwrap_or(0)
    };
    let first = sel_dev.saturating_sub(fit_n.saturating_sub(1));
    let more = if s.devices.len() > fit_n {
        format!(
            "{}/{} {}",
            (first + fit_n).min(s.devices.len()),
            s.devices.len(),
            cx.g("›", ">")
        )
    } else {
        String::new()
    };
    let mw = super::width(&more, cx.uni());
    zone(
        cx,
        buf,
        r.x + 1,
        dev_y,
        right.saturating_sub(mw + 1),
        "DEVICES",
        on_rail,
    );
    if mw > 0 {
        let w = cx.put_r(buf, right, dev_y, &more, cx.st(Tok::Muted));
        cx.hit(Rect::new(right - w, dev_y, w, 1), "palette:devices");
    }
    for (i, d) in s.devices.iter().enumerate().skip(first).take(fit_n) {
        let x = strip.x + (i - first) as u16 * (tw + gap);
        let pick = if on_graph && d.id == sel_id {
            Some(false)
        } else if on_rail && i == sel_dev {
            Some(true)
        } else {
            None
        };
        tiles::device_tile(cx, buf, Rect::new(x, strip.y, tw, tile_h), d, pick);
        if !cx.has_surface() && i + 1 < first + fit_n {
            cx.vsep(buf, x + tw, strip.y, tile_h);
        }
    }

    // Sessions.
    let on_sess = focus == Pane::Sessions;
    let list = cx.by_attention();
    let noun = if list.len() == 1 {
        "session"
    } else {
        "sessions"
    };
    let shown = thumbs_grid(cx, buf, grid, &list, on_sess.then_some(cx.c.selected));
    let hint = if shown < list.len() {
        format!("{shown} of {} {noun} {}", list.len(), cx.g("›", ">"))
    } else {
        format!("{} {noun}", list.len())
    };
    let hw = super::width(&hint, cx.uni());
    zone(
        cx,
        buf,
        r.x + 1,
        ses_y,
        right.saturating_sub(hw + 1),
        "SESSIONS",
        on_sess,
    );
    cx.put_r(buf, right, ses_y, &hint, cx.st(Tok::Muted));
    cx.hit(Rect::new(right - hw, ses_y, hw, 1), "palette:sessions");
}

/// Mesh and link-colour legend; returns the x after the last item.
fn mesh_legend(cx: &Cx, buf: &mut Buffer, mut x: u16, y: u16, right: u16) -> u16 {
    let mut end = x;
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
        end = x;
        x += 3;
    }
    // The other link colours, at wide sizes.
    if cx.size() >= Size::M {
        for (name, tok) in [("lan", Tok::Lan), ("ssh", Tok::Info)] {
            let t = format!("{} {name}", cx.g("──", "--"));
            let w = super::width(&t, cx.uni());
            if x + w + 2 > right {
                break;
            }
            x += cx.put(buf, x, y, w, &t, cx.ink(tok, 0.9));
            end = x;
            x += 3;
        }
    }
    end
}

/// Phone / short hub: compact schematic, one row per device, then what needs you.
fn hub_compact(cx: &Cx, buf: &mut Buffer, r: Rect, focus: Pane) {
    let s = cx.s;
    let attn: Vec<&LabSession> = cx
        .by_attention()
        .into_iter()
        .filter(|x| x.state == SessionState::AwaitingInput)
        .take(2)
        .collect();
    let att_h = attn.len() as u16;
    let map_h = (r.height * 2 / 5).clamp(5, 9);
    let band = Rect::new(r.x, r.y, r.width, map_h);
    let ids = map::order(cx, band);
    let sel_id = ids
        .get(cx.c.selected % ids.len().max(1))
        .cloned()
        .unwrap_or_default();
    let on_graph = focus == Pane::Graph;
    map::draw(
        cx,
        buf,
        band,
        &s.viewer,
        3,
        map::Sel::Node(&sel_id, on_graph),
    );
    let mut y = band.bottom();
    let on_rail = focus == Pane::Rail;
    let right = r.right().saturating_sub(1);
    let sess = format!("{} sessions", s.sessions.len());
    let sw = super::width(&sess, cx.uni());
    zone(
        cx,
        buf,
        r.x + 1,
        y,
        right.saturating_sub(sw + 1),
        "DEVICES",
        on_rail,
    );
    cx.put_r(buf, right, y, &sess, cx.st(Tok::Muted));
    cx.hit(Rect::new(right - sw, y, sw, 1), "palette:sessions");
    y += 1;
    let rows = r.bottom().saturating_sub(y + att_h) as usize;
    let sel = if on_rail {
        cx.c.selected % s.devices.len().max(1)
    } else {
        s.devices.iter().position(|d| d.id == sel_id).unwrap_or(0)
    };
    let first = sel.saturating_sub(rows.saturating_sub(1));
    for (i, d) in s.devices.iter().enumerate().skip(first).take(rows) {
        let pick = (i == sel && (on_rail || on_graph)).then_some(on_rail);
        tiles::device_row(cx, buf, Rect::new(r.x, y, r.width, 1), d, pick);
        y += 1;
    }
    if s.devices.len() > rows + first && rows > 0 {
        let t = format!("+{} more", s.devices.len() - rows - first);
        cx.put_r(buf, right, y - 1, &t, cx.st(Tok::Muted));
    }
    // What needs you stays on screen even on a phone.
    let mut y = r.bottom().saturating_sub(att_h);
    for x in attn {
        let row = Rect::new(r.x, y, r.width, 1);
        cx.fill(buf, row, cx.wash(Tok::Warn, 0.12));
        let (g, word, st) = cx.sstate(x);
        let mut xx = r.x + 1;
        xx += cx.put(buf, xx, y, 1, g, st) + 1;
        xx += tiles::addr(cx, buf, xx, y, x) + 1;
        let room = right.saturating_sub(xx + super::width(word, cx.uni()) + 1);
        cx.put(
            buf,
            xx,
            y,
            room,
            &fit(&x.title, room as usize, cx.uni()),
            cx.ink(Tok::Fg, 1.0),
        );
        cx.put_r(buf, right, y, word, st);
        cx.hit(row, format!("session:{}:{}", x.device, x.id));
        y += 1;
    }
}

/// Session thumbnails on one uniform grid (same widths, one gutter), ranked by attention;
/// a free cell becomes the "new session" ghost tile. Returns how many tiles were shown.
fn thumbs_grid(
    cx: &Cx,
    buf: &mut Buffer,
    r: Rect,
    list: &[&LabSession],
    sel: Option<usize>,
) -> usize {
    if r.height < 3 || r.width < 12 {
        return 0;
    }
    let (min_w, max_cols) = match cx.size() {
        Size::Xs => (r.width, 1),
        Size::S => (24, 3),
        Size::M => (34, 3),
        Size::L => (40, 4),
    };
    let gap = 2u16;
    let cols = ((r.width + gap) / (min_w + gap)).clamp(1, max_cols);
    let min_h = if cx.size() == Size::S { 5 } else { 6 };
    let fit_rows = ((r.height + 1) / (min_h + 1)).max(1);
    let rows = (list.len() as u16 + 1).div_ceil(cols).clamp(1, fit_rows);
    let tw = (r.width + gap) / cols - gap;
    let th = ((r.height + 1) / rows).saturating_sub(1).min(tw / 2 + 4);
    let cells = usize::from(rows * cols);
    let shown = list.len().min(cells);
    for i in 0..cells.min(list.len() + 1) {
        let (ci, ri) = (i as u16 % cols, i as u16 / cols);
        let tr = Rect::new(r.x + ci * (tw + gap), r.y + ri * (th + 1), tw, th);
        match list.get(i) {
            Some(x) => {
                let pick = sel.and_then(|s| (s % list.len() == i).then_some(true));
                tiles::thumb(cx, buf, tr, x, pick);
            }
            None => new_tile(cx, buf, tr, None, false),
        }
        if !cx.has_surface() && ci + 1 < cols && i + 1 < cells.min(list.len() + 1) {
            cx.vsep(buf, tr.right(), tr.y, th);
        }
    }
    shown
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
    let fan_want = if links_n == 0 {
        0
    } else {
        (links_n * 3 + 1).max(5)
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
            .map(|x| x.screen.len() as u16)
            .max()
            .unwrap_or(5)
            + 2;
        let rows = (list.len() as u16 + 1).div_ceil(cols);
        let room = main
            .height
            .saturating_sub(if fan_want > 0 { fan_want + 2 } else { 0 });
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
                None => new_tile(cx, buf, tr, Some(d), list.is_empty()),
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
    if links_n > 0 && fy + 5 <= r.bottom() {
        let hint = format!("{} edit", "e");
        let hw = super::width(&hint, cx.uni()) + 2;
        zone(
            cx,
            buf,
            main.x,
            fy,
            right.saturating_sub(hw + 1),
            "LINKS",
            false,
        );
        let n = cx.put_r(buf, right, fy, &format!(" {hint} "), cx.key_style());
        cx.hit(Rect::new(right - n, fy, n, 1), "links");
        let band = Rect::new(
            r.x,
            fy + 1,
            r.width,
            r.bottom().saturating_sub(fy + 1).min(fan_want + 2),
        );
        map::draw(cx, buf, band, &d.id, 1, map::Sel::Root);
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

/// Ghost tile for starting a session: a dashed outline with one key per provider. On the
/// hub (no device) it teaches the grammar instead: a device letter, then the provider.
fn new_tile(cx: &Cx, buf: &mut Buffer, r: Rect, d: Option<&LabDevice>, picked: bool) {
    if r.width < 12 || r.height < 3 {
        return;
    }
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
    let opts = [
        ("c", "claude", Tok::Claude),
        ("x", "codex", Tok::Codex),
        ("n", "shell", Tok::Shell),
    ];
    let inner = r.height.saturating_sub(2);
    let stacked = inner >= 6 && d.is_some();
    let block_h = if stacked { 2 + opts.len() as u16 } else { 2 };
    let mut y = r.y + 1 + inner.saturating_sub(block_h) / 2;
    let x = r.x + 3;
    let title = match d {
        Some(d) => format!("new on {}", d.name),
        None => "new session".into(),
    };
    let tw = r.right().saturating_sub(x + 2);
    cx.put(
        buf,
        x,
        y,
        tw,
        &fit(&title, tw as usize, cx.uni()),
        cx.ink(Tok::Fg, 0.9).add_modifier(Modifier::BOLD),
    );
    y += if stacked { 2 } else { 1 };
    if d.is_none() {
        if y < r.bottom() - 1 {
            cx.put(
                buf,
                x,
                y,
                tw,
                &fit("device letter, then c x n", tw as usize, cx.uni()),
                cx.st(Tok::Muted),
            );
        }
        cx.hit(r, "palette:new");
        return;
    }
    let id = d.map(|d| d.id.as_str()).unwrap_or_default();
    let mut ox = x;
    for (k, label, tok) in opts {
        if y >= r.bottom() - 1 || (!stacked && ox + 10 > r.right()) {
            break;
        }
        let x0 = ox;
        ox += cx.put(buf, ox, y, 3, &format!(" {k} "), cx.key_style());
        ox += 1;
        ox += cx.put(buf, ox, y, 8, label, cx.st(tok));
        cx.hit(Rect::new(x0, y, ox - x0, 1), format!("new:{id}:{label}"));
        if stacked {
            y += 1;
            ox = x;
        } else {
            ox += 3;
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
        if picked {
            cx.put(buf, r.x, y, 1, cx.g("▌", ">"), cx.st(Tok::Accent));
            cx.put(buf, r.x, y + 1, 1, cx.g("▌", ">"), cx.st(Tok::Accent));
        }
        let mut xx = r.x + 1;
        xx += tiles::addr(cx, buf, xx, y, x) + 1;
        let (g, word, st) = cx.sstate(x);
        let room = r.right().saturating_sub(xx + 3);
        cx.put(
            buf,
            xx,
            y,
            room,
            &fit(&x.title, room as usize, cx.uni()),
            cx.ink(Tok::Fg, 1.0).add_modifier(Modifier::BOLD),
        );
        cx.put(buf, r.right() - 2, y, 1, g, st);
        let (p, ptok) = cx.prov(x);
        let mut cx2 = r.x + 2;
        cx2 += cx.put(buf, cx2, y + 1, 7, p, cx.st(ptok));
        cx2 += 1;
        let cap = format!("{word} · {}", tiles::caption(x));
        let room = r.right().saturating_sub(cx2 + 1);
        cx.put(
            buf,
            cx2,
            y + 1,
            room,
            &fit(&cap, room as usize, cx.uni()),
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

fn peek(cx: &Cx, buf: &mut Buffer, r: Rect, x: &LabSession) {
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
    let spark_w = if sz >= Size::S { 12 } else { 0 };
    let room = main.right().saturating_sub(mx + spark_w + 2);
    cx.put(
        buf,
        mx,
        main.y,
        room,
        &fit(&meta, room as usize, cx.uni()),
        cx.st(Tok::Muted),
    );
    if spark_w > 0 {
        let v: Vec<Option<f32>> = x.activity.iter().map(|v| Some(*v)).collect();
        let sx = main.right().saturating_sub(spark_w);
        tiles::spark(cx, buf, sx, main.y, spark_w, &v, cx.ink(Tok::Accent, 0.8));
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
    // Filmstrip of the other sessions, each with its address keys; it starts level with
    // the screen, its label on the blank row above.
    if strip_w > 0 {
        let sx = r.right().saturating_sub(strip_w + 1);
        zone(cx, buf, sx, r.y + 1, sx + strip_w, "OTHERS", false);
        let others: Vec<_> = cx
            .by_attention()
            .into_iter()
            .filter(|o| o.id != x.id)
            .collect();
        let th = 7u16;
        let mut y = scr.y;
        for o in others {
            if y + th > r.bottom() {
                break;
            }
            tiles::thumb(cx, buf, Rect::new(sx, y, strip_w, th), o, None);
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
                format!("{} {slot}", d.hotkey),
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
        // The selected row's tint runs under its keys too.
        let kst = if i == sel {
            cx.st(Tok::AccentBg)
                .patch(cx.st(Tok::Accent))
                .add_modifier(Modifier::BOLD)
        } else {
            cx.key_style()
        };
        cx.put_r(buf, inner.right(), yy, &format!(" {key} "), kst);
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
    cx.put(
        buf,
        x0,
        y,
        w,
        &fit(sub, w as usize, cx.uni()),
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
    // Secondary ways in, one per row.
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
        let line = Rect::new(tx, y, tw, 1);
        if cx.has_surface() {
            cx.fill(buf, line, cx.surface());
        }
        let one = fit(&inv.oneliner, tw.saturating_sub(2) as usize, cx.uni());
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
    // The sheet ends a row above the footer; the schematic keeps the rest.
    let bottom = r.bottom().saturating_sub(1);
    let band = Rect::new(
        r.x,
        r.y + 1,
        r.width,
        bottom.saturating_sub(r.y + 2 + sheet_h),
    );
    if band.height >= 5 {
        map::draw(cx, buf, band, &cx.s.viewer, 3, map::Sel::Edge(&e.id));
    }
    let sheet = Rect::new(r.x, bottom.saturating_sub(sheet_h), r.width, sheet_h);
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
    // What a change does: policies apply on the key press (u undoes), so the line states
    // the live effect, wrapped rather than cut.
    let users = cx
        .s
        .sessions
        .iter()
        .filter(|x| {
            x.device == e.to && e.from == cx.s.viewer || x.device == e.from && e.to == cx.s.viewer
        })
        .count();
    let msg = format!(
        "k p o apply at once; u undoes. {users} open session{} cross{} this link.",
        if users == 1 { "" } else { "s" },
        if users == 1 { "es" } else { "" },
    );
    for (i, line) in wrap(&msg, inner.width as usize).iter().take(2).enumerate() {
        let cy = inner.y + 6 + i as u16;
        if cy < inner.bottom() {
            cx.put(buf, inner.x, cy, inner.width, line, cx.ink(Tok::Info, 0.95));
        }
    }
    let ry = inner.y + 9;
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
    let kw: u16 = if r.width >= 120 {
        9
    } else if r.width >= 80 {
        7
    } else {
        0
    };
    let devs: Vec<&LabDevice> = cx.s.devices.iter().collect();
    let globals: [(&str, &str); 10] = [
        ("⏎", "open / attach"),
        ("␣", "peek"),
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
        if cx.uni() {
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
            ("then ⌫", "stop a session"),
            ("⏎ / ␣", "open / peek"),
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
    let mut y = r.y + 1;
    zone(cx, buf, x0, y, right, "HUB", true);
    y += 2;
    key_rows(
        y,
        buf,
        &|c: char| match devs.iter().find(|d| d.hotkey == c) {
            Some(d) => {
                let (g, _) = cx.status(d.status);
                let ev = format!("{g} {}", cx.meta(d));
                (
                    fit(&ev, usize::from(kw - 1), cx.uni()),
                    cx.chip_style(cx.s.hue(&d.id)),
                )
            }
            None => match c {
                '/' => ("find".into(), cx.key_style()),
                _ => (String::new(), Style::default()),
            },
        },
    );
    y += 9;
    // Legend: the full device names, in keycap colours.
    let mut x = x0;
    for d in &devs {
        let w = 3 + 1 + super::width(&d.name, cx.uni());
        if x + w > right {
            y += 1;
            x = x0;
        }
        x += cx.dchip(buf, x, y, d, false) + 1;
        x += cx.put(buf, x, y, w, &d.name, cx.ink(Tok::Fg, 0.85)) + 3;
    }
    y += 2;
    if y + 10 < r.bottom() {
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
            const NTH: [&str; 8] = ["1st", "2nd", "3rd", "4th", "5th", "6th", "7th", "8th"];
            if let Some(i) = SLOTS.iter().position(|s| *s == c) {
                return (format!("{} ses", NTH[i]), acc);
            }
            match c {
                'n' => ("+shell".into(), ks),
                'f' => ("files".into(), ks),
                'e' => ("links".into(), ks),
                'c' => ("+claude".into(), tint(Tok::Claude)),
                'x' => ("+codex".into(), tint(Tok::Codex)),
                '/' => ("find".into(), ks),
                _ => (String::new(), Style::default()),
            }
        });
        y += 9;
    }
    // Keys that mean the same everywhere.
    let mut x = x0;
    for (k, v) in globals {
        let k = key(k);
        let w = super::width(k, cx.uni()) + 2 + 1 + super::width(v, cx.uni());
        if x + w > right {
            y += 1;
            x = x0;
        }
        if y >= r.bottom() {
            break;
        }
        x += cx.put(buf, x, y, 10, &format!(" {k} "), cx.key_style()) + 1;
        x += cx.put(buf, x, y, 14, v, cx.ink(Tok::Fg, 0.85)) + 3;
    }
    let _ = (DeviceStatus::Online, centered, Access::Key);
}
