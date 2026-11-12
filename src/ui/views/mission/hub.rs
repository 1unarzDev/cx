//! The hub: the network orbit on top, a legend row, the selected device's vitals strip,
//! one row of live session thumbnails (what needs you first) and the alert ticker that
//! pairs each problem with the keys that fix it. Phones get the indented device tree
//! (relays above the devices reached through them).
use super::{fit, orbit, tiles, views, Cx, Size};
use crate::ui::views::{caps::Pane, Access, LabDevice, NoticeKind, SessionState, Tok};
use ratatui::{buffer::Buffer, layout::Rect, style::Modifier};

pub(super) fn hub(cx: &Cx, buf: &mut Buffer, r: Rect) {
    if cx.touch() || r.height < 16 {
        return compact(cx, buf, r);
    }
    let sz = cx.size();
    let focus = cx.c.focus.unwrap_or(Pane::Graph);
    let (on_graph, on_sess) = (focus == Pane::Graph, focus == Pane::Sessions);
    let list = cx.by_attention();
    let right = r.right().saturating_sub(1);

    // Bottom up: ticker, one row of thumbnails, the vitals strip; the NETWORK label
    // (with the mesh legend on its rule) on top; the orbit takes the rest, and the
    // rows it leaves empty go to the thumbnails.
    let tick_h = u16::from(!alerts(cx).is_empty());
    let vit_h: u16 = if sz >= Size::M { 2 } else { 1 };
    let (cols, th_min) = views::grid_shape(cx, r.width.saturating_sub(2), r.height);
    let need = list
        .iter()
        .take(usize::from(cols))
        .map(|x| tiles::tail_rows(x) as u16 + 2)
        .max()
        .unwrap_or(th_min);
    let th_max = match sz {
        Size::L => 12,
        Size::M => 9,
        _ => 6,
    };
    let fixed = tick_h + 1 + vit_h + 1 + 1 + 1; // ticker, sessions label, vitals, network label, gaps
    let orbit_min = orbit::want_h(cx).min(r.height.saturating_sub(fixed + th_min));
    let th = r
        .height
        .saturating_sub(fixed + orbit_min)
        .clamp(th_min.min(r.height), need.clamp(th_min, th_max));
    let band_h = r.height.saturating_sub(fixed + th);
    // Past its natural height the map stops growing (its rings are capped): the rows go
    // to the thumbnails, up to what their screens hold.
    let give = band_h
        .saturating_sub(orbit::MAX_H)
        .min(need.saturating_sub(th));
    let (band_h, th) = (band_h - give, th + give);
    let band_th = th;
    let lx = cx.label(buf, r.x + 1, r.y, 16, "NETWORK", on_graph) + r.x + 1;
    let lend = legend(cx, buf, lx + 2, r.y, right);
    // A short stub of rule after the legend reads as debris: only a real run.
    if right.saturating_sub(lend) >= 12 {
        cx.rule(buf, lend + 1, right, r.y);
    }
    // (A blank row either side: the map never touches the legend or the vitals strip.)
    let band = Rect::new(r.x, r.y + 2, r.width, band_h.saturating_sub(2));
    let ids = orbit::order(cx, band);
    let sel_id = if on_sess && !list.is_empty() {
        list[cx.c.selected % list.len()].device.clone()
    } else {
        ids.get(cx.c.selected % ids.len().max(1))
            .cloned()
            .unwrap_or_default()
    };
    // Rows the drawing leaves empty go to the thumbnails (up to what their screens
    // hold), keeping a blank row either side of the map; with room for two, a second
    // row of tiles shows the rest of the sessions and the ghost.
    let gh = orbit::drawn_h(cx, band);
    let room = th + band.height.saturating_sub(gh);
    // (Only when each row keeps ten rows: a tile must still show enough of its screen
    // to be told apart.)
    let (rows, th) = if list.len() >= usize::from(cols) && room > 2 * th_min.max(10) {
        (2, ((room - 1) / 2).min(need))
    } else {
        (1, room.min(need))
    };
    let grid_h = rows * th + rows - 1;
    let mut y = band.bottom() + 1 + band_th - grid_h;
    // The map sits centred in the rows above the vitals strip.
    let above = ((y - r.y - 1).saturating_sub(gh) / 2).max(1);
    let band = Rect::new(band.x, r.y + 1 + above, band.width, gh);
    orbit::draw(
        cx,
        buf,
        band,
        &cx.s.viewer,
        3,
        orbit::Sel::Node(&sel_id, on_graph),
    );
    if let Some(d) = cx.s.device(&sel_id) {
        vitals(
            cx,
            buf,
            Rect::new(r.x + 1, y, r.width.saturating_sub(2), vit_h),
            d,
            list.len() < usize::from(rows * cols),
        );
    }
    y += vit_h + 1;

    // Sessions: one row, ranked by attention; the full grid lives in each device.
    let n = list.len();
    // `›` (a crumb, "goes on") in Unicode; ASCII keeps the words (`>` is selection).
    // Only when tiles are hidden: `+2 more ›` (when every session shows, no chip).
    let hidden = n.saturating_sub(usize::from(rows * cols));
    let hint = if hidden > 0 {
        format!("+{hidden} more{}", cx.g(" ›", ""))
    } else {
        String::new()
    };
    let hw = super::width(&hint, cx.uni());
    let lx = views::zone(
        cx,
        buf,
        r.x + 1,
        y,
        right.saturating_sub(hw + 1),
        "SESSIONS",
        on_sess,
    );
    if sz >= Size::M {
        let t = " needs you first ";
        cx.put(buf, lx + 1, y, 17, t, cx.ink(Tok::Muted, 0.8));
    }
    if hidden > 0 {
        cx.put_r(buf, right, y, &hint, cx.st(Tok::Muted));
        cx.hit(Rect::new(right - hw, y, hw, 1), "palette:sessions");
    }
    y += 1;
    let grid = Rect::new(r.x + 1, y, r.width.saturating_sub(2), grid_h);
    let pick = if on_sess {
        (cx.c.selected, true)
    } else {
        (0, false)
    };
    views::thumbs_grid(cx, buf, grid, &list, pick, cols, th, cx.s.device(&sel_id));
    if tick_h > 0 {
        ticker(
            cx,
            buf,
            Rect::new(r.x + 1, r.bottom() - 1, r.width.saturating_sub(2), 1),
        );
    }
}

/// One quiet line beside the NETWORK label: the meshes in their hue (members up) and
/// lan's teal. The line grammar lives in help (`?`).
fn legend(cx: &Cx, buf: &mut Buffer, mut x: u16, y: u16, right: u16) -> u16 {
    let sep = if cx.size() >= Size::M { 3 } else { 2 };
    let mut items: Vec<(String, Tok, Option<&crate::ui::views::LabMesh>)> =
        cx.s.meshes
            .iter()
            .map(|m| {
                let up = m
                    .members
                    .iter()
                    .filter(|id| cx.s.device(id).is_some_and(|d| cx.up(d)))
                    .count();
                let text = if cx.size() >= Size::M {
                    format!("{} {up}/{}", m.name, m.members.len())
                } else {
                    m.name.clone()
                };
                (text, orbit::mesh_tok(cx, &m.id), Some(m))
            })
            .collect();
    items.push(("lan".into(), Tok::Lan, None));
    for (t, tok, mesh) in &items {
        let alert = mesh.is_some_and(|m| !m.healthy);
        let w = super::width(t, cx.uni()) + if alert { 2 } else { 0 };
        if x + w + 1 > right {
            break;
        }
        let tw = cx.put(buf, x, y, w, t, cx.ink(*tok, 0.85));
        if alert {
            cx.put(buf, x + tw, y, 2, cx.g(" ▲", " ^"), cx.st(Tok::Warn));
        }
        if let Some(m) = mesh {
            cx.hit(Rect::new(x, y, w, 1), format!("mesh:{}", m.id));
        }
        x += w + sep;
    }
    x
}

/// The selected device: identity, route with the rtt of every hop, live vitals and the
/// keys that act on it from the hub (a letter, then a verb).
/// `ghost`: the session grid shows the `new on` tile, so the strip leaves new out.
fn vitals(cx: &Cx, buf: &mut Buffer, r: Rect, d: &LabDevice, ghost: bool) {
    let bg = if cx.has_surface() {
        cx.surface()
    } else {
        ratatui::style::Style::default()
    };
    cx.fill(buf, r, bg);
    // No bar: column 0 is the focus bar only, and the strip is neither focused nor a
    // selection (it mirrors the map's).
    let right = r.right().saturating_sub(1);
    let y = r.y;
    let mut x = r.x + 2;
    let nst = cx.ink(Tok::Fg, 1.0).add_modifier(Modifier::BOLD).patch(bg);
    x += orbit::name(cx, buf, x, y, d, 20, nst, true) + 2;
    let path = orbit::route(cx, &d.id);
    let via = if d.id == cx.s.viewer {
        "this device".to_string()
    } else if path.is_empty() {
        format!("no route {} {}", cx.g("·", "-"), cx.meta(d))
    } else {
        let mut at = cx.s.viewer.as_str();
        let mut hops = vec![cx.s.device(at).map_or('?', |v| v.hotkey).to_string()];
        let mut ms = Vec::new();
        for e in &path {
            at = if e.from == at { &e.to } else { &e.from };
            hops.push(cx.s.device(at).map_or('?', |v| v.hotkey).to_string());
            ms.push(e.rtt_ms.map_or("?".into(), |v| {
                super::ms_text(v).trim_end_matches("ms").to_string()
            }));
        }
        format!("{}  {} ms", hops.join(cx.g(" → ", " -> ")), ms.join(" + "))
    };
    let up = cx.up(d);
    let vst = if up {
        cx.st(Tok::Muted)
    } else {
        cx.ink(Tok::Err, 0.9)
    }
    .patch(bg);
    x += cx.put(buf, x, y, right.saturating_sub(x), &via, vst) + 2;
    if cx.size() >= Size::M {
        let who = format!("{} {} {}@{}", d.os, cx.g("·", "-"), d.account, d.host);
        x += cx.put(
            buf,
            x,
            y,
            right.saturating_sub(x + 30),
            &fit(&who, 44, cx.uni()),
            cx.ink(Tok::Muted, 0.7).patch(bg),
        ) + 2;
    }
    // Keys: what this device's letter leads to.
    let k = d.hotkey;
    let n = cx.s.sessions_on(&d.id).count();
    let (attach, new, files, link) = (
        format!("@{k} a"),
        format!("@{k} c"),
        format!("@{k} f"),
        format!("e @{k}"),
    );
    let mut items: Vec<(&str, &str, &str)> = Vec::new();
    if n > 0 {
        items.push((&attach, "attach", "attach"));
    }
    if up && !ghost {
        items.push((&new, "claude", "new"));
    }
    if up {
        items.push((&files, "files", "files"));
    }
    if d.id != cx.s.viewer {
        items.push((&link, "link", "links"));
    }
    let ky = r.bottom() - 1;
    let kw: u16 = items
        .iter()
        .map(|(k, l, _)| cx.key_w(k) + 1 + super::width(l, cx.uni()) + 2)
        .sum();
    let mut kx = r.right();
    if r.height >= 2 || x + kw <= right {
        kx = if r.height >= 2 {
            right.saturating_sub(kw) + 2
        } else {
            x
        };
        cx.chips(buf, kx, ky, right + 1, &items);
    }
    if r.height < 2 {
        // The strip opens the device; its key chips keep their own taps.
        cx.hit(
            Rect::new(r.x, r.y, kx.saturating_sub(r.x + 1), 1),
            format!("device:{}", d.id),
        );
        return;
    }
    // Second row: live vitals.
    let y = r.y + 1;
    let mut x = r.x + 2;
    let stop = right.saturating_sub(kw);
    let lab = cx.ink(Tok::Muted, 0.75).patch(bg);
    if let Some(m) = d.metrics.as_ref().filter(|_| up) {
        x += cx.put(buf, x, y, 4, "cpu ", lab);
        let v: Vec<Option<f32>> = m.cpu_hist.iter().map(|v| Some(*v)).collect();
        tiles::spark(cx, buf, x, y, 16, &v, cx.spark_heat(m.cpu).patch(bg));
        x += 17;
        x += cx.put(
            buf,
            x,
            y,
            5,
            &format!("{:.0}%", m.cpu * 100.0),
            cx.ink(Tok::Fg, 0.9).patch(bg),
        ) + 2;
        if x + 24 < stop {
            x += cx.put(buf, x, y, 4, "mem ", lab);
            tiles::gauge(cx, buf, x, y, 8, m.mem, cx.heat(m.mem).patch(bg));
            x += 9;
            x += cx.put(
                buf,
                x,
                y,
                14,
                &format!("{:.0}% of {:.0}G", m.mem * 100.0, m.mem_total_gb),
                cx.ink(Tok::Fg, 0.9).patch(bg),
            ) + 2;
        }
        if let Some(t) = m.temp_c.filter(|_| x + 5 < stop) {
            let hv = ((t - 35.0) / 50.0).clamp(0.0, 1.0);
            x += cx.put(
                buf,
                x,
                y,
                4,
                &format!("{t:.0}{}", cx.g("°", "C")),
                cx.heat(hv).patch(bg),
            ) + 2;
        }
        if x + 10 < stop {
            x += cx.put(buf, x, y, 12, &format!("up {}", m.uptime), lab) + 2;
        }
    } else {
        x += cx.put(
            buf,
            x,
            y,
            stop.saturating_sub(x),
            &d.evidence,
            cx.ink(Tok::Err, 0.85).patch(bg),
        ) + 2;
    }
    for m in &d.meshes {
        let Some(mesh) = cx.s.meshes.iter().find(|q| &q.id == m) else {
            continue;
        };
        let t = format!("{} {}", tiles::mesh_glyph(cx), mesh.name);
        let w = super::width(&t, cx.uni());
        if x + w + 2 > stop {
            break;
        }
        x += cx.put(
            buf,
            x,
            y,
            w,
            &t,
            cx.ink(orbit::mesh_tok(cx, m), 0.95).patch(bg),
        ) + 2;
    }
    cx.hit(
        Rect::new(r.x, r.y, stop.saturating_sub(r.x), r.height),
        format!("device:{}", d.id),
    );
}

/// What needs attention, each with the keys that fix it from the hub: (glyph, text,
/// tok, keys, verb, action).
type Alert = (&'static str, String, Tok, String, &'static str, String);

fn alerts(cx: &Cx) -> Vec<Alert> {
    let mut v: Vec<Alert> = Vec::new();
    for x in
        cx.s.sessions
            .iter()
            .filter(|x| x.state == SessionState::AwaitingInput)
    {
        let Some(d) = cx.s.device(&x.device) else {
            continue;
        };
        let text = if d.id == cx.s.viewer {
            format!("{} needs you", x.title)
        } else {
            format!("{} needs you on {}", x.title, d.name)
        };
        v.push((
            cx.g("◆", "!"),
            text,
            Tok::Warn,
            format!("@{} {}", d.hotkey, cx.slot(x).unwrap_or('a')),
            "attach",
            format!("session:{}:{}", d.id, x.id),
        ));
    }
    let mut notes: Vec<_> =
        cx.s.notifications
            .iter()
            .filter(|n| matches!(n.kind, NoticeKind::Warn | NoticeKind::Err))
            .collect();
    notes.sort_by_key(|n| std::cmp::Reverse(n.kind as u8));
    for n in notes {
        let d = cx.s.devices.iter().find(|d| n.text.starts_with(&d.name));
        let tok = if n.kind == NoticeKind::Err {
            Tok::Err
        } else {
            Tok::Warn
        };
        let head = n.text.replace(" for ", " ");
        let (keys, verb) = match (d, n.action.as_deref()) {
            (Some(d), Some("retry")) => (format!("@{} r", d.hotkey), "retry"),
            (Some(d), _) => (format!("@{}", d.hotkey), "inspect"),
            _ => ("/".to_string(), "find"),
        };
        let act = d.map_or("notices".to_string(), |d| format!("retry:{}", d.id));
        v.push((cx.g("▲", "^"), head, tok, keys, verb, act));
    }
    v
}

/// The alert ticker: each problem in its colour, then its fix as a key chip.
fn ticker(cx: &Cx, buf: &mut Buffer, r: Rect) {
    // The first-ranked session is the first tile right above: its alert is not repeated.
    let top = cx
        .by_attention()
        .first()
        .map(|x| format!("session:{}:{}", x.device, x.id));
    let items: Vec<Alert> = alerts(cx)
        .into_iter()
        .filter(|a| Some(&a.5) != top.as_ref())
        .collect();
    let mut x = r.x + 1;
    let right = r.right();
    let n = items.len();
    for (i, (g, text, tok, keys, verb, act)) in items.into_iter().enumerate() {
        let kw = cx.key_w(&keys) + 1 + super::width(verb, cx.uni());
        let room = right.saturating_sub(x + kw + 4);
        // The first item keeps its words; later ones only when they fit whole-ish.
        let tw = super::width(&text, cx.uni()).min(room);
        if tw < 12 || (i > 0 && tw < super::width(&text, cx.uni()).min(28)) {
            if i > 0 && x + 6 < right {
                // A chip, so the alerts that did not fit are one tap away.
                let more = format!(" +{} ", n - i);
                let w = cx.put(buf, x, r.y, 5, &more, cx.key_style());
                cx.hit(Rect::new(x, r.y, w, 1), "notices");
            }
            break;
        }
        let x0 = x;
        x += cx.put(buf, x, r.y, 2, g, cx.st(tok)) + 1;
        x += cx.put(
            buf,
            x,
            r.y,
            tw,
            &fit(&text, usize::from(tw), cx.uni()),
            cx.ink(Tok::Fg, 0.92),
        ) + 2;
        cx.hit(Rect::new(x0, r.y, x - x0, 1), act.clone());
        x = cx.chips(buf, x, r.y, right, &[(&keys, verb, &act)]) + 2;
    }
}

/// Phone hub: every device as an indented tree row (devices reached through a relay
/// sit under it; a graph of keycaps at this size only crowded), then the ticker.
fn compact(cx: &Cx, buf: &mut Buffer, r: Rect) {
    let focus = cx.c.focus.unwrap_or(Pane::Graph);
    let right = r.right().saturating_sub(1);
    let rows = orbit::tree_rows(cx, &cx.s.viewer);
    let n = rows.len() as u16;
    let al = alerts(cx);
    let tick_h = (al.len() as u16).min(2).min(r.height / 8);
    // The label, a breathing row, the tree, a breathing row, the ticker.
    let avail = r.height.saturating_sub(2 + tick_h + 1);
    // Two-row device rows when the height allows (a thumb-sized target with vitals).
    let row_h = if avail >= 2 * n { 2 } else { 1 };
    let shown = (avail / row_h).min(n);
    let lw = cx.label(buf, r.x + 1, r.y, 16, "NETWORK", focus != Pane::Sessions);
    legend(cx, buf, r.x + 2 + lw + 1, r.y, right);
    let ids: Vec<&str> = rows.iter().map(|t| t.d.id.as_str()).collect();
    let sel_id = ids[cx.c.selected % ids.len().max(1)];
    let mut y = r.y + 2;
    for t in rows.iter().take(usize::from(shown)) {
        tree_row(
            cx,
            buf,
            Rect::new(r.x, y, r.width, row_h),
            t,
            t.d.id == sel_id,
        );
        y += row_h;
    }
    if shown < n {
        let t = format!("+{} more", n - shown);
        cx.put_r(buf, right, y.saturating_sub(1), &t, cx.st(Tok::Muted));
    }
    let ty = r.bottom().saturating_sub(tick_h);
    // Rows the tree leaves go to the sessions that need you first (two-row thumb
    // targets), keeping a breathing row above the ticker.
    let mut sy = y + 2;
    if sy + 2 < ty {
        views::zone(
            cx,
            buf,
            r.x + 1,
            y + 1,
            right,
            "SESSIONS",
            focus == Pane::Sessions,
        );
        let list = cx.by_attention();
        for (i, x) in list.iter().enumerate() {
            if sy + 2 >= ty {
                break;
            }
            let pick = (focus == Pane::Sessions && i == cx.c.selected % list.len()).then_some(true);
            tiles::session_row(cx, buf, Rect::new(r.x, sy, r.width, 2), x, pick);
            sy += 3;
        }
    }
    let shown = &al[..usize::from(tick_h).min(al.len())];
    // One key column for every row, so the fix keys line up.
    let kw = shown
        .iter()
        .map(|a| cx.key_w(&a.3) + 1 + super::width(a.4, cx.uni()))
        .max()
        .unwrap_or(0);
    for (i, a) in shown.iter().enumerate() {
        phone_alert(cx, buf, Rect::new(r.x, ty + i as u16, r.width, 1), a, kw);
    }
}

fn phone_alert(cx: &Cx, buf: &mut Buffer, r: Rect, a: &Alert, kw: u16) {
    let (g, text, tok, keys, verb, act) = a;
    let bg = cx.wash(*tok, 0.12);
    cx.fill(buf, r, bg);
    let mut x = r.x + 1;
    x += cx.put(buf, x, r.y, 2, g, cx.st(*tok).patch(bg)) + 1;
    let room = r.right().saturating_sub(x + kw + 2);
    // `◆` already says "needs you": those words go before the title is cut.
    let text = if super::width(text, cx.uni()) > room {
        text.replacen(" needs you", "", 1)
    } else {
        text.clone()
    };
    cx.put(
        buf,
        x,
        r.y,
        room,
        &fit(&text, usize::from(room), cx.uni()),
        cx.ink(Tok::Fg, 0.92).patch(bg),
    );
    let kx = r.right().saturating_sub(kw + 1);
    cx.chips(buf, kx, r.y, r.right(), &[(keys, verb, act)]);
    cx.hit(Rect::new(r.x, r.y, kx - r.x, 1), act.clone());
}

/// One device in the phone tree: branch guides in the hue of the link to its parent
/// (dotted where only a password is allowed), its letter, name, and rtt or last
/// sighting; with two rows its cpu trend and temperature underneath.
fn tree_row(cx: &Cx, buf: &mut Buffer, r: Rect, t: &orbit::TreeRow, sel: bool) {
    let d = t.d;
    let bg = if sel && cx.has_surface() {
        cx.st(Tok::AccentBg)
    } else {
        ratatui::style::Style::default()
    };
    cx.fill(buf, r, bg);
    if sel {
        cx.bar(buf, r.x, r.y, r.height, cx.st(Tok::Accent));
    }
    let right = r.right().saturating_sub(1);
    let mut x = r.x + 2;
    let gst = |tok: Tok| cx.ink(tok, 0.85).patch(bg);
    for &more in &t.guides {
        cx.put(
            buf,
            x,
            r.y,
            1,
            if more { cx.g("│", "|") } else { " " },
            gst(Tok::Faint),
        );
        if r.height > 1 && more {
            cx.put(buf, x, r.y + 1, 1, cx.g("│", "|"), gst(Tok::Faint));
        }
        x += 2;
    }
    if let Some(e) = t.edge {
        // Down: a faint guide (the device's `✕` is its one red mark).
        let tok = if e.up {
            orbit::edge_tok(cx, e)
        } else {
            Tok::Faint
        };
        let pass = e.up && e.forward != Access::Key && e.reverse != Access::Key;
        let g = match (t.last, pass) {
            (false, false) => cx.g("├─", "+-"),
            (true, false) => cx.g("└─", "`-"),
            (false, true) => cx.g("├┄", "+-"),
            (true, true) => cx.g("└┄", "`-"),
        };
        cx.put(buf, x, r.y, 2, g, gst(tok));
        if r.height > 1 && !t.last {
            cx.put(buf, x, r.y + 1, 1, cx.g("│", "|"), gst(Tok::Faint));
        }
        x += 2;
    }
    // The letter in its hue; the selected row gets the keycap.
    if sel {
        cx.dchip(buf, x, r.y, d, false);
    } else {
        let k = format!(" {} ", d.hotkey);
        let st = cx
            .ink(Tok::Id(cx.s.hue(&d.id)), 1.0)
            .add_modifier(Modifier::BOLD);
        cx.put(buf, x, r.y, 3, &k, st.patch(bg));
    }
    x += 4;
    let up = cx.up(d);
    let nst = if sel {
        cx.st(Tok::Accent).add_modifier(Modifier::BOLD)
    } else if up {
        cx.ink(Tok::Fg, 1.0).add_modifier(Modifier::BOLD)
    } else {
        cx.st(Tok::Muted)
    }
    .patch(bg);
    let meta = cx.meta(d);
    let (g, sst) = cx.status(d.status);
    let tail = super::width(&meta, cx.uni()) + 3;
    let room = right.saturating_sub(x + tail);
    cx.put(
        buf,
        x,
        r.y,
        room,
        &fit(&d.name, usize::from(room), cx.uni()),
        nst,
    );
    let mut tx = right.saturating_sub(tail) + 1;
    tx += cx.put(buf, tx, r.y, 1, g, sst.patch(bg)) + 1;
    let mst = if up {
        cx.st(Tok::Muted)
    } else {
        cx.ink(Tok::Err, 0.85)
    };
    cx.put(buf, tx, r.y, 8, &meta, mst.patch(bg));
    if r.height > 1 {
        let ex = x;
        if let Some(m) = d.metrics.as_ref().filter(|_| up) {
            let v: Vec<Option<f32>> = m.cpu_hist.iter().map(|v| Some(*v)).collect();
            let sw = right.saturating_sub(ex + 12).min(12);
            // One neutral tone: only a hot device's figure takes a colour.
            tiles::spark(
                cx,
                buf,
                ex,
                r.y + 1,
                sw,
                &v,
                cx.ink(Tok::Muted, 0.6).patch(bg),
            );
            let t = match m.temp_c {
                Some(t) => format!("{:.0}% {t:.0}{}", m.cpu * 100.0, cx.g("°", "C")),
                None => format!("{:.0}%", m.cpu * 100.0),
            };
            cx.put(
                buf,
                ex + sw + 1,
                r.y + 1,
                10,
                &t,
                cx.st(if m.cpu >= 0.85 { Tok::Warn } else { Tok::Muted })
                    .patch(bg),
            );
        } else {
            cx.put(
                buf,
                ex,
                r.y + 1,
                right.saturating_sub(ex),
                &fit(&d.evidence, usize::from(right.saturating_sub(ex)), cx.uni()),
                cx.ink(Tok::Muted, 0.8).patch(bg),
            );
        }
    }
    cx.hit(r, format!("device:{}", d.id));
}
