//! The hub: the network orbit on top, a legend row, the selected device's vitals strip,
//! one row of live session thumbnails (what needs you first) and the alert ticker that
//! pairs each problem with the keys that fix it. Phones get the indented device tree
//! (relays above the devices reached through them).
use super::{fit, nebula, orbit, tiles, views, Cx, Size};
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

    // Bottom up: ticker, the thumbnails, the vitals strip; a blank row on top; the
    // orbit takes the rest, and the rows it leaves empty go to the thumbnails.
    let tick_h = u16::from(!alerts(cx).is_empty());
    let vit_h: u16 = if sz >= Size::M { 2 } else { 1 };
    let (cols, th_min) = views::grid_shape(cx, r.width.saturating_sub(2), r.height);
    // Tiles take the rows the map leaves, up to a tall screen: a screen that has not
    // arrived yet keeps its size (no tile grows when its text lands).
    let th_max = match sz {
        Size::L => 22,
        Size::M => 16,
        _ => 12,
    };
    // With nothing running, the launch panel needs ten rows at most: the map keeps the rest.
    let need = if list.is_empty() { 10 } else { th_max };
    let fixed = tick_h + 1 + vit_h + 1 + 1 + 1; // ticker, sessions label, vitals, top row, gaps
                                                // (Plus the band's blank row either side, and the air kept where the screen is tall.)
    let pad = if sz >= Size::M && r.height >= 30 {
        4
    } else {
        2
    };
    let orbit_min = (orbit::want_h(cx) + pad).min(r.height.saturating_sub(fixed + th_min));
    // (`min` then `max`: the bounds cross on a short screen, and the floor wins.)
    let th = r
        .height
        .saturating_sub(fixed + orbit_min)
        .min(need.min(th_max).max(th_min))
        .max(th_min.min(r.height));
    let band_h = r.height.saturating_sub(fixed + th);
    // Past its natural height the map stops growing (its rings are capped): the rows go
    // to the thumbnails, up to what their screens hold.
    let give = band_h
        .saturating_sub(orbit::MAX_H)
        .min(need.saturating_sub(th));
    let (band_h, th) = (band_h - give, th + give);
    let band_th = th;
    // (The top row stays blank, as air under the top bar: no header strip. A device's
    // meshes show quietly in its vitals strip.)
    // (One column in: a label at the map's edge stays on the text's gutter.)
    let band = Rect::new(
        r.x + 1,
        r.y + 2,
        r.width.saturating_sub(2),
        band_h.saturating_sub(2),
    );
    let ids = nebula::order(cx, band);
    cx.order.borrow_mut().clone_from(&ids);
    let sel_id = if on_sess && !list.is_empty() {
        list[cx.c.selected % list.len()].device.clone()
    } else {
        ids.get(cx.c.selected % ids.len().max(1))
            .cloned()
            .unwrap_or_default()
    };
    // Rows the drawing leaves go to the thumbnails (a screen taller than its text is
    // still a screen), keeping the map's air: two blank rows either side
    // where the screen is tall, else the blank row the band already keeps. With room
    // for two, a second row of tiles shows the rest of the sessions and the ghost
    // (only when each row keeps ten rows, so a tile still shows enough of its screen).
    // (Nothing running: the map grows into the rows the panel leaves.)
    // (Its rows, and those the launch panel's ten leave: it needs three or so.)
    let tall = if list.is_empty() {
        band.height + th.saturating_sub(4)
    } else {
        0
    };
    let gh = nebula::height(cx, band, &cx.s.viewer, tall);
    let air = if sz >= Size::M && r.height >= 30 {
        2
    } else {
        0
    };
    let room = if list.is_empty() {
        th
    } else {
        th + band.height.saturating_sub(gh + air)
    };
    let (rows, th) = if list.len() >= usize::from(cols) && room > 2 * th_min.max(10) {
        (2, (room - 1) / 2)
    } else {
        (1, room)
    };
    // Two rows share the sessions evenly (the last row's tiles widen to fill it).
    let cols = if rows == 2 {
        cols.min((list.len() as u16).div_ceil(2))
    } else {
        cols
    };
    let mut grid_h = (rows * th + rows).saturating_sub(1);
    let mut y = (band.bottom() + 1 + band_th).saturating_sub(grid_h);
    // The map sits centred in the rows above the vitals strip.
    // (Counting the row of mesh wash above the map where rows are airy.)
    let lift = u16::from(sz >= Size::M);
    let mut above = ((y - r.y - 1).saturating_sub(gh + lift) / 2).max(1) + lift;
    if list.is_empty() {
        // Nothing running: the map near the top as in the reference, the strip and the
        // panel right under it, and the rows left over stay below, where tiles will go.
        // The rows the panel does not need go half above the map, half under the
        // panel: the group sits in the middle, not over a void. (Measured off screen.)
        above = (1 + lift).min(above);
        let bottom = y + grid_h;
        let ny = (r.y + above + gh + 2 + lift).min(y);
        let panel = cx.s.device(&sel_id).map_or(1, |d| {
            let n = cx.hits.borrow().len();
            let area = Rect::new(0, 0, r.width, grid_h);
            let h = views::launch_panel(cx, &mut Buffer::empty(area), area, d, true);
            cx.hits.borrow_mut().truncate(n);
            h
        });
        let free = bottom.saturating_sub(ny + panel + 1) / 2;
        above += free;
        y = ny + free;
        grid_h = bottom.saturating_sub(y);
    }
    // The only device and nothing running: this device at the middle of its quiet
    // orbits (the welcome page's picture), as tall as the panel's rows allow.
    let lone = list.is_empty() && cx.s.devices.len() == 1 && cx.uni();
    let grow = if lone {
        grid_h.saturating_sub(8).min(15u16.saturating_sub(gh + 2))
    } else {
        0
    };
    y += grow;
    grid_h -= grow;
    // (Not so low that the drawing runs into the vitals strip.)
    let above = above.min(y.saturating_sub(r.y + gh + 1)).max(1);
    // (A row more either side where there is air, for the mesh wash's margin. The
    // map draws in a band of at least three rows; a one-row drawing sits in its
    // middle row.)
    let band = if above >= 2 {
        Rect::new(band.x, r.y + above, band.width, gh + 2)
    } else {
        let pad = u16::from(gh == 1);
        Rect::new(band.x, r.y + 1 + above - pad, band.width, gh.max(3))
    };
    // (Never into the vitals strip's rows: a ring that does not fit is left out whole.)
    let band = Rect {
        height: band.height.min(y.saturating_sub(band.y)).max(1),
        ..band
    };
    match cx.s.device(&sel_id).filter(|_| grow > 0) {
        Some(d) => {
            let band = Rect {
                height: band.height + grow,
                ..band
            };
            nebula::welcome(cx, buf, band, d);
            cx.hit(band, format!("device:{}", d.id));
        }
        None => nebula::draw(cx, buf, band, &cx.s.viewer, &sel_id, on_graph, tall),
    }
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
    // Only when tiles are hidden: `/ +2 more` (when every session shows, no chip).
    let hidden = n.saturating_sub(usize::from(rows * cols));
    let hw = views::more_chip(cx, buf, right, y, hidden);
    let lx = views::zone(
        cx,
        buf,
        r.x + 1,
        y,
        right.saturating_sub(hw + 1),
        "SESSIONS",
        on_sess,
    );
    // (A sort order only means something with tiles to sort.)
    // (With none, the rule says so: the panel under it starts with what to do.)
    let t = if n > 0 {
        " needs you first "
    } else {
        " nothing running "
    };
    // Where the bare `c x n` start (their keys too, unless this device's launch row
    // under it shows them); it takes the label's room where both do not fit.
    let keys = n > 0 || sel_id != cx.s.viewer;
    let end = right.saturating_sub(hw + 1);
    let label = (sz >= Size::M || n == 0) && views::here_fits(cx, lx + 18, end, keys);
    if label || cx.s.device(&cx.s.viewer).is_none_or(|d| d.start == "~") {
        cx.put(buf, lx + 1, y, 17, t, cx.ink(Tok::Muted, 0.8));
    }
    views::here_hint(cx, buf, if label { lx + 18 } else { lx + 1 }, y, end, keys);
    y += 1;
    let grid = Rect::new(r.x + 1, y, r.width.saturating_sub(2), grid_h);
    let mut pick = if on_sess {
        (cx.c.selected, true)
    } else {
        (0, false)
    };
    // The selected session always has a tile (after a detach it may rank below the
    // row): it takes the last cell in place of the one there.
    let shown = usize::from(rows * cols);
    let mut list = list;
    if on_sess && n > shown && shown > 0 && cx.c.selected % n >= shown {
        list.swap(shown - 1, cx.c.selected % n);
        pick.0 = shown - 1;
    }
    let th = if list.is_empty() { grid_h } else { th };
    views::thumbs_grid(cx, buf, grid, &list, pick, cols, th, cx.s.device(&sel_id));
    if tick_h > 0 {
        ticker(
            cx,
            buf,
            Rect::new(r.x + 1, r.bottom() - 1, r.width.saturating_sub(2), 1),
        );
    }
}

/// The selected device: identity, route with the rtt of every hop, live vitals and the
/// keys that act on it from the hub (a letter, then a verb).
/// `ghost`: the session grid shows the `new on` tile, so the strip leaves new out.
fn vitals(cx: &Cx, buf: &mut Buffer, r: Rect, d: &LabDevice, ghost: bool) {
    cx.fill(buf, r, ratatui::style::Style::default());
    // No bar: column 0 is the focus bar only, and the strip is neither focused nor a
    // selection (it mirrors the map's).
    let right = r.right().saturating_sub(1);
    // (Text on the column every zone label and the ticker start on.)
    let y = r.y;
    let mut x = r.x + 1;
    let nst = cx.ink(Tok::Fg, 1.0).add_modifier(Modifier::BOLD);
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
    };
    x += cx.put(buf, x, y, right.saturating_sub(x), &via, vst) + 2;
    if cx.size() >= Size::M {
        let who = format!("{} {} {}@{}", d.os, cx.g("·", "-"), d.account, d.host);
        // (No os word when it is not known.)
        let who = who.trim_start_matches([' ', '·', '-']).to_string();
        x += cx.put(
            buf,
            x,
            y,
            right.saturating_sub(x + 30),
            &fit(&who, 44, cx.uni()),
            cx.ink(Tok::Muted, 0.7),
        ) + 2;
    }
    // Keys: the device's letter, then what it leads to, as two visible steps (the `c`
    // that follows a letter is claude, never the creation device).
    // Each chip's tap is its keys' (`i a` attaches slot a of this device, not the
    // hub's selection).
    let first = cx.s.sessions_on(&d.id).next();
    let dev = format!("@{}", d.hotkey);
    let act = format!("device:{}", d.id);
    let attach = first.map(|x| format!("session:{}:{}", d.id, x.id));
    let new = format!("new:{}:claude", d.id);
    let files = format!("files:{}", d.id);
    let mut items: Vec<(&str, &str, &str)> = vec![(&dev, "then", &act)];
    if let Some(attach) = &attach {
        items.push(("a", "attach", attach));
    }
    if up && !ghost {
        items.push(("c", "claude", &new));
    }
    // (The launch panel under it says `f files` with the rest, but for this device,
    // whose launch keys work without its letter.)
    if up && (!ghost || d.id == cx.s.viewer) {
        items.push(("f", "files", &files));
    }
    if items.len() == 1 {
        items.clear();
    }
    let ky = r.bottom() - 1;
    let kw: u16 = cx.chips_w(&items);
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
    let mut x = r.x + 1;
    let stop = right.saturating_sub(kw);
    let lab = cx.ink(Tok::Muted, 0.75);
    if let Some(m) = d.metrics.as_ref().filter(|_| up) {
        x += cx.put(buf, x, y, 4, "cpu ", lab);
        let v: Vec<Option<f32>> = m.cpu_hist.iter().map(|v| Some(*v)).collect();
        tiles::spark(cx, buf, x, y, 16, &v, cx.spark_heat(m.cpu));
        x += 17;
        x += cx.put(
            buf,
            x,
            y,
            5,
            &format!("{:.0}%", m.cpu * 100.0),
            cx.ink(Tok::Fg, 0.9),
        ) + 2;
        if x + 24 < stop {
            x += cx.put(buf, x, y, 4, "mem ", lab);
            tiles::gauge(cx, buf, x, y, 8, m.mem, cx.heat(m.mem));
            x += 9;
            x += cx.put(
                buf,
                x,
                y,
                14,
                &format!("{:.0}% of {:.0}G", m.mem * 100.0, m.mem_total_gb),
                cx.ink(Tok::Fg, 0.9),
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
                cx.heat(hv),
            ) + 2;
        }
        if x + 10 < stop {
            x += cx.put(buf, x, y, 12, &format!("up {}", m.uptime), lab) + 2;
        }
        if let Some(v) = d.via.as_ref() {
            let t = format!("via {v}");
            if x + super::width(&t, cx.uni()) + 2 <= stop {
                x += cx.put(buf, x, y, super::width(&t, cx.uni()), &t, lab) + 2;
            }
        }
    } else {
        x += cx.put(
            buf,
            x,
            y,
            stop.saturating_sub(x),
            &d.evidence,
            // Evidence without live figures: red only when the device is down.
            if up { lab } else { cx.ink(Tok::Err, 0.85) },
        ) + 2;
    }
    for m in &d.meshes {
        let Some(mesh) = cx.s.meshes.iter().find(|q| &q.id == m) else {
            continue;
        };
        let t = format!("{}{}", tiles::mesh_glyph(cx), mesh.name);
        let w = super::width(&t, cx.uni());
        if x + w + 2 > stop {
            break;
        }
        x += cx.put(buf, x, y, w, &t, cx.ink(orbit::mesh_tok(cx, m), 0.95)) + 2;
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
        // A session past the key pool has no letter pair: `/` finds it by name.
        let (keys, verb) = match cx.slot(x) {
            Some(s) => (format!("@{} {s}", d.hotkey), "attach"),
            None => ("/".to_string(), "find"),
        };
        v.push((
            cx.g("◆", "^"),
            text,
            Tok::Warn,
            keys,
            verb,
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
        let d = n.device.as_deref().and_then(|id| cx.s.device(id));
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
        let act = match (d, verb) {
            (Some(d), "retry") => format!("retry:{}", d.id),
            (Some(d), _) => format!("device:{}", d.id),
            _ => "notices".to_string(),
        };
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
            let more = format!("+{} more", n - i);
            let kw = cx.key_w("/");
            if i > 0 && x + kw + 1 + super::width(&more, cx.uni()) < right {
                // `/ +2 more`: the alerts that did not fit are a find away, by key or tap.
                let w = cx.keycap(buf, x, r.y, kw, "/", cx.key_style()) + 1;
                let w = w + cx.put(buf, x + w, r.y, 8, &more, cx.st(Tok::Muted));
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
    // A breathing row, the tree, a breathing row, the ticker. With nothing running,
    // three rows stay for SESSIONS and its launch row where the tree can spare them.
    let idle = cx.s.sessions.is_empty();
    let avail = r.height.saturating_sub(1 + tick_h + 1);
    let avail = if idle && avail >= n + 3 {
        avail - 3
    } else {
        avail
    };
    // Two-row device rows when the height allows (a thumb-sized target with vitals).
    let row_h = if avail >= 2 * n { 2 } else { 1 };
    let shown = (avail / row_h).min(n);
    let ids: Vec<&str> = rows.iter().map(|t| t.d.id.as_str()).collect();
    *cx.order.borrow_mut() = ids.iter().map(|s| s.to_string()).collect();
    let sel_id = ids
        .get(cx.c.selected % ids.len().max(1))
        .copied()
        .unwrap_or_default();
    let toks = nebula::letter_toks(cx);
    let mut y = r.y + 1;
    for t in rows.iter().take(usize::from(shown)) {
        let tok = toks
            .iter()
            .find(|(id, _)| *id == t.d.id)
            .map_or(Tok::Fg, |e| e.1);
        tree_row(
            cx,
            buf,
            Rect::new(r.x, y, r.width, row_h),
            t,
            tok,
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
    // (The rule alone where no tile fits: it says where `c x n` start.)
    let here = cx.s.device(&cx.s.viewer).is_some_and(|d| d.start != "~");
    if sy + 2 < ty || (sy < ty && (idle || here)) {
        let lx = views::zone(
            cx,
            buf,
            r.x + 1,
            y + 1,
            right,
            "SESSIONS",
            focus == Pane::Sessions,
        );
        let list = cx.by_attention();
        // Where the bare `c x n` start (their keys too, unless this device's launch row
        // under it shows them); it takes the label's room where both do not fit.
        let keys = !list.is_empty() || sel_id != cx.s.viewer;
        let label = list.is_empty() && views::here_fits(cx, lx + 18, right, keys);
        if label || (list.is_empty() && cx.s.device(&cx.s.viewer).is_none_or(|d| d.start == "~")) {
            let quiet = cx.ink(Tok::Muted, 0.8);
            cx.put(buf, lx + 1, y + 1, 17, " nothing running ", quiet);
        }
        views::here_hint(
            cx,
            buf,
            if label { lx + 18 } else { lx + 1 },
            y + 1,
            right,
            keys,
        );
        if list.is_empty() {
            if let Some(d) = cx.s.device(sel_id) {
                let pr = Rect::new(r.x, sy, r.width, ty.saturating_sub(sy + 1).max(1));
                views::launch_panel(cx, buf, pr, d, true);
            }
        }
        let sel = (focus == Pane::Sessions && !list.is_empty()).then(|| cx.c.selected % list.len());
        // With rows to spare, the selected session's (else the first's) last lines sit
        // under its row: the phone's live preview.
        let tail_at = sel.unwrap_or(0);
        for (i, x) in list.iter().enumerate() {
            if sy + 2 >= ty {
                break;
            }
            let pick = (sel == Some(i)).then_some(true);
            tiles::session_row(cx, buf, Rect::new(r.x, sy, r.width, 2), x, pick);
            sy += 2;
            let left = ty.saturating_sub(sy + 1);
            let rest = (list.len() - i - 1) as u16 * 3;
            if i == tail_at && left >= rest + 4 {
                let h = (left - rest - 1).min(12);
                tiles::screen_text(
                    cx,
                    buf,
                    Rect::new(r.x + 3, sy, r.width.saturating_sub(4), h),
                    x,
                    cx.c.has_color(),
                );
                sy += h;
            }
            sy += 1;
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
    cx.fill(buf, r, ratatui::style::Style::default());
    let mut x = r.x + 1;
    x += cx.put(buf, x, r.y, 2, g, cx.st(*tok)) + 1;
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
        cx.ink(Tok::Fg, 0.92),
    );
    let kx = r.right().saturating_sub(kw + 1);
    cx.chips(buf, kx, r.y, r.right(), &[(keys, verb, act)]);
    cx.hit(Rect::new(r.x, r.y, kx.saturating_sub(r.x), 1), act.clone());
}

/// One device in the phone tree: branch guides in the hue of the link to its parent
/// (dotted where only a password is allowed), its letter, name, and rtt or last
/// sighting; with two rows its cpu trend and temperature underneath. `tok` is the
/// letter's group hue, as on the map.
fn tree_row(cx: &Cx, buf: &mut Buffer, r: Rect, t: &orbit::TreeRow, tok: Tok, sel: bool) {
    let d = t.d;
    cx.fill(buf, r, ratatui::style::Style::default());
    if sel {
        cx.bar(buf, r.x, r.y, r.height, cx.st(Tok::Accent));
    }
    let right = r.right().saturating_sub(1);
    let mut x = r.x + 2;
    let gst = |tok: Tok| cx.ink(tok, 0.85);
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
            // (ASCII `+` is the invite key; `.` a password line.)
            (false, false) => cx.g("├─", "|-"),
            (true, false) => cx.g("└─", "`-"),
            (false, true) => cx.g("├┄", "|."),
            (true, true) => cx.g("└┄", "`."),
        };
        cx.put(buf, x, r.y, 2, g, gst(tok));
        if r.height > 1 && !t.last {
            cx.put(buf, x, r.y + 1, 1, cx.g("│", "|"), gst(Tok::Faint));
        }
        x += 2;
    }
    // The letter in its group's hue; the selected row gets the keycap.
    let st = cx.ink(tok, 1.0).add_modifier(Modifier::BOLD);
    if sel {
        cx.keycap(buf, x, r.y, 3, &d.hotkey.to_string(), st);
    } else {
        let k = format!(" {} ", d.hotkey);
        cx.put(buf, x, r.y, 3, &k, st);
    }
    x += 4;
    let up = cx.up(d);
    let nst = if sel {
        cx.st(Tok::Accent).add_modifier(Modifier::BOLD)
    } else if up {
        cx.ink(Tok::Fg, 1.0).add_modifier(Modifier::BOLD)
    } else {
        cx.st(Tok::Muted)
    };
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
    tx += cx.put(buf, tx, r.y, 1, g, sst) + 1;
    let mst = if up {
        cx.st(Tok::Muted)
    } else {
        cx.ink(Tok::Err, 0.85)
    };
    cx.put(buf, tx, r.y, super::width(&meta, cx.uni()), &meta, mst);
    if r.height > 1 {
        let ex = x;
        if let Some(m) = d.metrics.as_ref().filter(|_| up) {
            let v: Vec<Option<f32>> = m.cpu_hist.iter().map(|v| Some(*v)).collect();
            let sw = right.saturating_sub(ex + 12).min(12);
            // One neutral tone: only a hot device's figure takes a colour.
            tiles::spark(cx, buf, ex, r.y + 1, sw, &v, cx.ink(Tok::Muted, 0.6));
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
                cx.st(if m.cpu >= 0.85 { Tok::Warn } else { Tok::Muted }),
            );
        } else {
            cx.put(
                buf,
                ex,
                r.y + 1,
                right.saturating_sub(ex),
                &fit(&d.evidence, usize::from(right.saturating_sub(ex)), cx.uni()),
                cx.ink(Tok::Muted, 0.8),
            );
        }
    }
    cx.nav(r, format!("device:{}", d.id));
    cx.hit(r, format!("device:{}", d.id));
}
