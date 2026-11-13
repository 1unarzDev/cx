//! `ViewModel::from(&App)`: the App's real state mapped into the view layer's state
//! (`views::LabState`), plus the screen, focus and selection to draw. Pure: it reads the
//! App and never dispatches a request. Device ids in the view-model are device indexes
//! (`"0"`, `"1"`, ...), so tap actions resolve without parsing user-controlled ids.
use super::actions::PaletteTarget;
use super::views::{
    Access, ColorLevel, DeviceStatus, EdgeKind, FileKind, FileNode, LabCaps, LabDevice, LabEdge,
    LabFiles, LabInvite, LabNotice, LabPreview, LabSecret, LabSecretEntry, LabSession, LabState,
    LabTransfer, LabView, NoticeKind, PaletteRow, Pane, PreviewKind, Provider, SessionState, Theme,
    NEVER_SEEN,
};
use super::*;

pub(super) struct ViewModel {
    pub view: LabView,
    pub state: LabState,
    pub caps: LabCaps,
    /// Draw the palette over `view` (the palette is an overlay on every screen).
    pub palette: Option<String>,
}

/// The view-model id of device `i`.
pub(super) fn dev_id(i: usize) -> String {
    i.to_string()
}
/// The device index behind a view-model id.
pub(super) fn dev_index(id: &str) -> Option<usize> {
    id.parse().ok()
}

impl From<&App> for ViewModel {
    fn from(app: &App) -> Self {
        let mut caps = terminal_caps();
        caps.frame = app.transfer_frame as u32;
        let state = LabState {
            fixture: "app",
            viewer: dev_id(app.local_index()),
            devices: devices(app),
            meshes: Vec::new(),
            edges: edges(app),
            neighbors: Vec::new(),
            sessions: sessions(app),
            files: files(app),
            secrets: secrets(app),
            invite: invite(app),
            secret_entry: app.secret_entry.as_ref().map(|e| LabSecretEntry {
                name: safe_label(&e.name),
                naming: e.naming,
                typed: e.value.chars().count(),
            }),
            notifications: notices(app),
            transfers: transfers(app),
            palette: (app.input == Some(Input::Palette)).then(|| palette(app)),
        };
        let view = screen(app, &state, &mut caps);
        ViewModel {
            view,
            palette: (app.input == Some(Input::Palette)).then(|| app.text.clone()),
            state,
            caps,
        }
    }
}

/// The terminal's tier, theme and glyph set as the view layer names them.
pub(super) fn terminal_caps() -> LabCaps {
    let c = caps::caps();
    LabCaps {
        color: match c.color {
            caps::ColorDepth::TrueColor => ColorLevel::Truecolor,
            caps::ColorDepth::Ansi256 => ColorLevel::C256,
            caps::ColorDepth::Ansi16 => ColorLevel::C16,
            caps::ColorDepth::None => ColorLevel::None,
        },
        unicode: c.unicode,
        pills: c.pills,
        theme: match c.background {
            caps::Background::Dark => Theme::Dark,
            caps::Background::Light => Theme::Light,
        },
        ..LabCaps::default()
    }
}

/// Which screen to draw, with its focus and selection set on `caps`.
fn screen(app: &App, state: &LabState, caps: &mut LabCaps) -> LabView {
    if app.help {
        // The table for the screen it was opened over.
        caps.focus = match app.view {
            _ if app.edge.is_some() => Some(Pane::Custom("edge")),
            View::Work if app.device > 0 => Some(Pane::Custom("device")),
            View::Files => Some(Pane::Files),
            _ => None,
        };
        return LabView::Help;
    }
    if app.invite.is_some() {
        return LabView::JoinCode;
    }
    if let Some(e) = &app.edge {
        caps.selected = e.lane;
        return LabView::EdgeEditor(e.id.clone());
    }
    let session_id = app
        .selected_session()
        .map(|(_, s)| workspaces::runtime_key(&s));
    if app.peek {
        if let Some(id) = &session_id {
            caps.focus = Some(Pane::Custom("peek"));
            return LabView::SessionPeek(id.clone());
        }
    }
    match app.view {
        View::Work if app.device > 0 => {
            let d = app.device - 1;
            let id = dev_id(d);
            let list: Vec<&LabSession> = state.sessions_on(&id).collect();
            caps.selected = session_id
                .and_then(|id| list.iter().position(|x| x.id == id))
                .unwrap_or(list.len());
            if app.stop_armed.is_some() {
                caps.focus = Some(Pane::Custom("stop"));
            }
            // Its LINKS focused: `selected` is the link, in `edges_of` order.
            if let Some(n) = &app.device_link {
                caps.focus = Some(Pane::Graph);
                caps.selected = state
                    .edges_of(&id)
                    .position(|e| e.from == *n || e.to == *n)
                    .unwrap_or(0);
            }
            LabView::DeviceFocus(id)
        }
        View::Files => {
            let preview = app.browser.as_ref().is_some_and(|b| b.preview.is_some());
            if app.send_pick {
                caps.focus = Some(Pane::Dialog);
            }
            if preview {
                LabView::FilePreview
            } else {
                LabView::Files
            }
        }
        View::Secrets => {
            caps.selected = app.secret_selected;
            LabView::Secrets
        }
        _ if app.onboarding() => LabView::Onboarding,
        _ => {
            if app.hub_sessions {
                caps.focus = Some(Pane::Sessions);
                let order = super::views::mission::by_attention(state);
                caps.selected = session_id
                    .and_then(|id| order.iter().position(|x| *x == id))
                    .unwrap_or(0);
            } else {
                caps.focus = Some(Pane::Graph);
                let order = app.hub_order.borrow();
                let want = app
                    .hub_device
                    .clone()
                    .unwrap_or_else(|| dev_id(app.local_index()));
                caps.selected = order.iter().position(|id| *id == want).unwrap_or(0);
            }
            LabView::Hub
        }
    }
}

fn status(app: &App, d: usize) -> (DeviceStatus, String, u32) {
    let w = &app.work[d];
    let age = transport::now().saturating_sub(w.fetched);
    // Never listed: no sighting to age (an age from the epoch read as `20770d`).
    let secs = if w.fetched == 0 {
        NEVER_SEEN
    } else {
        u32::try_from(age).unwrap_or(u32::MAX - 1)
    };
    let listed = || {
        let n = w.sessions.len();
        format!(
            "{n} session{} listed {}s ago",
            if n == 1 { "" } else { "s" },
            age
        )
    };
    if let Some(e) = &w.error {
        return (DeviceStatus::Down, safe_label(e), secs);
    }
    if w.fetched == 0 {
        let word = if w.loading {
            "checking"
        } else {
            "not checked yet"
        };
        let status = if app.devices[d].target.is_none() {
            DeviceStatus::Local
        } else {
            DeviceStatus::Unknown
        };
        return (status, word.into(), NEVER_SEEN);
    }
    if app.devices[d].target.is_none() {
        return (DeviceStatus::Local, listed(), secs);
    }
    if age > 60 {
        return (DeviceStatus::Stale, listed(), secs);
    }
    (DeviceStatus::Online, listed(), secs)
}

fn devices(app: &App) -> Vec<LabDevice> {
    let letters = app.hotkeys();
    // Identity hues by a stable hash of the device id (spec 4.1); while there are hues
    // to spare, a collision takes the next free one in listing order.
    let mut taken = [false; super::theme::SWATCHES as usize];
    let spare = app.devices.len() <= taken.len();
    let mut hue = |id: &str| {
        let mut h = usize::from(super::theme::swatch_index(id));
        while spare && taken[h] {
            h = (h + 1) % taken.len();
        }
        taken[h] = true;
        h as u8
    };
    app.devices
        .iter()
        .enumerate()
        .map(|(i, d)| {
            let (status, mut evidence, observed_secs) = status(app, i);
            // A remote device's access posture is part of how it is reached.
            if d.target.is_some() && app.device_policy(i).access == store::AccessMode::Directed {
                evidence.push_str(" · single-direction");
            }
            LabDevice {
                id: dev_id(i),
                name: safe_label(&d.name),
                hotkey: letters.get(&d.id).copied().unwrap_or(' '),
                hue: hue(&d.id),
                os: String::new(),
                account: safe_label(&d.account),
                host: safe_label(&d.host),
                status,
                evidence,
                observed_secs,
                metrics: None,
                meshes: Vec::new(),
                joined_via: None,
                core: app.device_policy(i).access == store::AccessMode::Core,
            }
        })
        .collect()
}

fn access(a: crate::edges::Access) -> Access {
    match a {
        crate::edges::Access::Key => Access::Key,
        crate::edges::Access::Password | crate::edges::Access::Unmanaged => Access::Password,
        crate::edges::Access::Refused => Access::Denied,
    }
}

/// The stored edge graph (`edges::graph`, loaded by the controller), by device index. A
/// link this device reaches through an enrolled relay (its route's last hop) is a jump
/// through that relay. Meshes and rtt are not carried yet (STATUS.md).
fn edges(app: &App) -> Vec<LabEdge> {
    let local = app.local_index();
    let index = |id: &str| {
        if id == crate::edges::LOCAL {
            Some(local)
        } else {
            app.devices.iter().position(|d| d.id == id)
        }
    };
    let up = |a: usize, b: usize| {
        [a, b]
            .iter()
            .all(|&d| app.devices[d].target.is_none() || status(app, d).0 == DeviceStatus::Online)
    };
    let mut out: Vec<LabEdge> = app
        .edge_views
        .iter()
        .filter_map(|e| {
            let (a, b) = (index(&e.from)?, index(&e.to)?);
            let far = if a == local { b } else { a };
            let relay = [a, b]
                .contains(&local)
                .then(|| app.network_jump_routes.get(&far)?.as_ref()?.last())
                .flatten()
                .and_then(|hop| {
                    app.devices
                        .iter()
                        .position(|d| d.target.as_deref() == Some(hop.as_str()))
                });
            Some(LabEdge {
                id: format!("{a}-{b}"),
                from: dev_id(a),
                to: dev_id(b),
                kind: relay.map_or(EdgeKind::Direct, |j| EdgeKind::Jump(dev_id(j))),
                forward: access(e.forward.access),
                reverse: access(e.reverse.access),
                up: up(a, b),
                rtt_ms: None,
                rtt_hist: Vec::new(),
                loss: 0.0,
            })
        })
        .collect();
    // The link the editor has open, when nothing is recorded for it yet: unmanaged both
    // ways (a password), as its first change is planned from. The sheet always shows
    // the link its keys change.
    let open = app.edge.as_ref().and_then(|e| {
        let (a, b) = e.id.split_once('-')?;
        let (a, b): (usize, usize) = (a.parse().ok()?, b.parse().ok()?);
        (a.max(b) < app.devices.len()).then_some((a, b))
    });
    if let Some((a, b)) = open {
        let (f, t) = (dev_id(a), dev_id(b));
        if !out
            .iter()
            .any(|e| (e.from == f && e.to == t) || (e.from == t && e.to == f))
        {
            out.push(LabEdge {
                id: format!("{a}-{b}"),
                from: f,
                to: t,
                kind: EdgeKind::Direct,
                forward: Access::Password,
                reverse: Access::Password,
                up: up(a, b),
                rtt_ms: None,
                rtt_hist: Vec::new(),
                loss: 0.0,
            });
        }
    }
    out
}

fn session_state(app: &App, d: usize, s: &Session) -> SessionState {
    match app.session_status(d, s) {
        // A fresh listing without it: ended.
        "not seen" => return SessionState::Exited,
        // An unreachable device or an old listing: no evidence either way.
        status if !status.starts_with('+') => return SessionState::Unknown,
        _ => {}
    }
    // Activity from an old listing says nothing about now.
    if s.activity.is_some() && !session_activity_current(&app.work[d]) {
        return SessionState::Unknown;
    }
    match s.activity.as_deref() {
        Some("waiting") => SessionState::AwaitingInput,
        _ if session_busy(s) && s.provider != "shell" => SessionState::Working,
        _ if session_busy(s) => SessionState::Running,
        _ => SessionState::Idle,
    }
}

/// Every live session, in each device's listing order (its slots), with its last frame
/// from memory (the live preview's for the selected one).
fn sessions(app: &App) -> Vec<LabSession> {
    let live = app
        .selected_session()
        .map(|(d, s)| (d, workspaces::runtime_key(&s)));
    let mut out = Vec::new();
    for d in 0..app.work.len().min(app.devices.len()) {
        for s in app.slot_sessions(d) {
            let key = workspaces::runtime_key(s);
            let (screen, cols) = if live.as_ref() == Some(&(d, key.clone()))
                && !app.session_preview_lines().is_empty()
            {
                (
                    app.session_preview_lines().to_vec(),
                    app.session_preview_cols(),
                )
            } else {
                app.remembered_frame(d, s)
                    .map(|(lines, cols)| (lines.to_vec(), cols))
                    .unwrap_or_default()
            };
            // A session in a container says which (devcontainer, Docker, ...).
            let mut title = session_preview::label(s);
            if s.container.is_some() && s.provider == "shell" {
                title = format!("{title} · {}", menus::session_label(app, d, s));
            }
            out.push(LabSession {
                title,
                device: dev_id(d),
                provider: match s.provider.as_str() {
                    "claude" => Provider::Claude,
                    "codex" => Provider::Codex,
                    _ => Provider::Shell,
                },
                state: session_state(app, d, s),
                // An unknown folder is evidence, never a path.
                cwd: if s.directory.is_empty() {
                    "? folder".into()
                } else {
                    safe_label(&s.directory)
                },
                command: safe_label(&s.provider),
                last_activity_secs: s.idle_seconds.unwrap_or(0).min(u64::from(u32::MAX)) as u32,
                age_secs: 0,
                activity: app
                    .session_list
                    .levels(&app.devices[d], s, transport::now()),
                clients: 0,
                unread: false,
                screen,
                screen_cols: cols,
                id: key,
            });
        }
    }
    out
}

/// Rows on either side of the selection the Files view is given.
const WINDOW: usize = 256;

fn files(app: &App) -> LabFiles {
    let Some(b) = app.browser.as_ref() else {
        return LabFiles {
            device: dev_id(app.local_index()),
            cwd: String::new(),
            entries: Vec::new(),
            selected: 0,
            previews: Vec::new(),
            preview_path: String::new(),
            offset: 0,
            total: 0,
        };
    };
    let now = transport::now() as i64;
    let rows = super::browser_entries(b);
    // Only a window around the selection (more than any screen holds), so a
    // 20,000-entry folder costs the same per frame as a small one.
    let offset = b.selected.saturating_sub(WINDOW);
    let end = (b.selected + WINDOW).min(rows.len());
    let entries = (offset..end)
        .filter_map(|i| rows.get(i))
        .map(|e| FileNode {
            name: safe_label(&e.name),
            depth: 0,
            kind: match e.kind.as_str() {
                "directory" => FileKind::Dir,
                "symlink" => FileKind::Symlink,
                _ => FileKind::File,
            },
            size: e.size,
            modified_secs: e
                .modified
                .map_or(0, |m| (now - m).clamp(0, i64::from(u32::MAX)) as u32),
            expanded: false,
            git: None,
        })
        .collect();
    // The preview the screen shows: the open one, else the inline peek. Only its path, kind,
    // line count and first screenful are given; the real preview renderer draws its body.
    let shown = match (&b.preview, b.peek.as_deref()) {
        (Some(_), _) => Some(b),
        (None, Some(p)) if p.preview.is_some() => Some(p),
        _ => None,
    };
    // The selected entry's name (the design's facts and send line say it).
    let selected = rows
        .get(b.selected)
        .map(|e| safe_label(&e.name))
        .unwrap_or_default();
    LabFiles {
        device: dev_id(b.device),
        cwd: safe_label(&b.display_path),
        entries,
        selected: b.selected - offset,
        offset,
        total: rows.len(),
        previews: shown.map(lab_preview).into_iter().collect(),
        preview_path: if b.preview.is_some() {
            let path = b.preview_path.as_deref().unwrap_or("");
            safe_label(path.rsplit('/').next().unwrap_or(""))
        } else {
            selected
        },
    }
}

/// The view layer needs a preview's kind, line count and first screenful (the app draws
/// the real body over it), not a copy of a multi-megabyte text each frame.
fn lab_preview(b: &Browser) -> LabPreview {
    let text = b.preview.as_deref().unwrap_or("");
    let path = b.preview_path.as_deref().unwrap_or("");
    let kind = match b.preview_rich.as_ref().map(|p| p.kind.as_str()) {
        Some("markdown") => PreviewKind::Markdown,
        Some("code") => PreviewKind::Source("code"),
        Some("binary") => PreviewKind::Source("binary"),
        Some("image") => PreviewKind::Source("image"),
        Some("pdf") => PreviewKind::Source("pdf"),
        Some("archive") => PreviewKind::Source("archive"),
        Some("media") => PreviewKind::Source("media"),
        _ => PreviewKind::Source("text"),
    };
    LabPreview {
        path: safe_label(path.rsplit('/').next().unwrap_or("")),
        kind,
        text: text.lines().take(120).collect::<Vec<_>>().join("\n"),
        lines: source_lines(b),
        note: partial_cue(b),
    }
}
/// The hub's invite once its listener is up: code, one-liner and a QR code of the one-liner.
fn invite(app: &App) -> Option<LabInvite> {
    let (inv, _) = app.invite.as_ref()?.shown.as_ref()?;
    let qr = qrcode::QrCode::new(inv.one_liner_sh.as_bytes())
        .map(|q| {
            let w = q.width();
            let colors = q.to_colors();
            colors
                .chunks(w)
                .map(|row| row.iter().map(|c| *c == qrcode::Color::Dark).collect())
                .collect()
        })
        .unwrap_or_default();
    Some(LabInvite {
        code: safe_label(&inv.code),
        oneliner: safe_label(&inv.one_liner_sh),
        mesh: inv.backend.clone(),
        expires_secs: app.invite_secs_left().min(u64::from(u32::MAX)) as u32,
        also: inv.hosts.iter().skip(1).map(|h| safe_label(h)).collect(),
        qr,
    })
}

/// Secret names and metadata only (never values), as `cx secrets list` shows them.
fn secrets(app: &App) -> Vec<LabSecret> {
    let now = transport::now();
    let mut out: Vec<LabSecret> = app
        .secret_list
        .iter()
        .map(|m| LabSecret {
            key: safe_label(&m.name),
            label: safe_label(&m.name),
            masked: Some(format!(
                "{} ({})",
                if ascii() { "****" } else { "••••" },
                m.length
            )),
            source: m.backend.as_str().into(),
            updated_secs: Some(now.saturating_sub(m.updated).min(u64::from(u32::MAX)) as u32),
            used_by: Vec::new(),
        })
        .collect();
    let token = crate::secrets::CLOUDFLARE_TOKEN;
    if !out.iter().any(|s| s.key == token) {
        out.push(LabSecret {
            key: token.into(),
            label: "Cloudflare API token".into(),
            masked: None,
            source: String::new(),
            updated_secs: None,
            used_by: vec!["cx mesh".into()],
        });
    }
    out
}

/// The ticker: devices whose listing failed, each with its retry (transient notices are
/// toasts).
fn notices(app: &App) -> Vec<LabNotice> {
    let mut out = Vec::new();
    for (i, w) in app.work.iter().enumerate() {
        if let Some(e) = &w.error {
            out.push(LabNotice {
                kind: NoticeKind::Warn,
                device: Some(dev_id(i)),
                text: format!(
                    "{}: {}",
                    safe_label(&app.devices[i].name),
                    safe_label(reason(e))
                ),
                age_secs: 0,
                action: Some("retry".into()),
            });
        }
    }
    out
}

/// A failure's reason for the ticker: without the `account@host: ` prefixes (the
/// ticker names the device) or the advice after ` · ` (its key chip is the advice).
fn reason(e: &str) -> &str {
    let mut r = e.split(" · ").next().unwrap_or(e);
    while let Some((head, rest)) = r.split_once(": ") {
        if !head.contains('@') || head.contains(' ') {
            break;
        }
        r = rest;
    }
    r
}

fn transfers(app: &App) -> Vec<LabTransfer> {
    let by_name = |name: &str| {
        app.devices
            .iter()
            .position(|d| d.name == name || d.host == name)
    };
    app.job_rows()
        .iter()
        .filter(|(_, j)| {
            matches!(
                j["status"].as_str(),
                Some("running" | "queued" | "submitting")
            )
        })
        .filter_map(|(_, j)| {
            Some(LabTransfer {
                from: dev_id(by_name(j["source_host"].as_str()?)?),
                to: dev_id(by_name(j["destination_host"].as_str()?)?),
                path: transfer_name(j),
                done: j["bytes"].as_u64().unwrap_or(0),
                total: j["total"].as_u64().unwrap_or(0),
                rate_hist: Vec::new(),
            })
        })
        .collect()
}

/// The real palette (`palette_view`: ranked sessions, actions, views, devices, folders),
/// each row with the keys that reach it from the hub. A row's action is its index.
fn palette(app: &App) -> Vec<PaletteRow> {
    let letters = app.hotkeys();
    let letter = |d: usize| letters.get(&app.devices[d].id).copied();
    app.palette_view()
        .into_iter()
        .enumerate()
        .map(|(i, item)| {
            let (keys, device) = match &item.target {
                PaletteTarget::Session(d, key) => {
                    let slot = app
                        .slot_sessions(*d)
                        .iter()
                        .position(|s| workspaces::runtime_key(s) == *key)
                        .and_then(|n| super::views::mission::SLOTS.get(n));
                    let keys = match (letter(*d), slot) {
                        (Some(l), Some(s)) => format!("{l} {s}"),
                        _ => String::new(),
                    };
                    (keys, Some(dev_id(*d)))
                }
                PaletteTarget::Device(v) if *v > 0 => (
                    letter(v - 1).map(String::from).unwrap_or_default(),
                    Some(dev_id(v - 1)),
                ),
                // Only one- or two-key bindings read as keycaps.
                _ if item.key.chars().count() <= 2 => (item.key.clone(), None),
                _ => (String::new(), None),
            };
            PaletteRow {
                group: item.section.to_lowercase(),
                label: item.label,
                detail: item.detail,
                keys,
                action: format!("palette:{i}"),
                device,
            }
        })
        .collect()
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use ratatui::{backend::TestBackend, Terminal};

    fn device(id: &str, name: &str, remote: bool) -> Device {
        Device {
            id: id.into(),
            name: name.into(),
            target: remote.then(|| name.to_string()),
            account: "tester".into(),
            host: name.into(),
            status: "unknown".into(),
            observed_at: 0,
        }
    }
    pub(in crate::ui) fn hub_app() -> App {
        crate::store::isolate_test_state();
        let (tx, _) = mpsc::sync_channel(64);
        let mut a = App::new(
            vec![
                device("local", "tranquility", false),
                device("inno", "innovation", true),
                device("nas", "nas", true),
            ],
            tx,
        );
        for w in &mut a.work {
            w.fetched = transport::now();
        }
        a
    }
    pub(in crate::ui) fn text(a: &App, w: u16, h: u16) -> String {
        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
        t.draw(|f| rendering::render(f, a)).unwrap();
        let buf = t.backend().buffer().clone();
        (0..h)
            .map(|y| {
                (0..w)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn the_hub_is_the_start_view_with_every_device_on_the_map() {
        let a = hub_app();
        assert_eq!(a.view, View::Network);
        let t = text(&a, 120, 40);
        for name in ["tranquility", "innovation", "nas"] {
            assert!(t.contains(name), "{name} missing:\n{t}");
        }
        // (No `NETWORK lan` header strip over the map.)
        assert!(!t.contains("NETWORK"), "{t}");
    }
}
