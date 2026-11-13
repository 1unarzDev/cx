//! Render lab: the production view layer (`ui::views`) captured from fixtures and
//! dumped (cells + styles) for `tests/lab_png.py`. Test-only; see docs/design/lab.md.
#![allow(dead_code)]

mod capture;
pub mod dump;

pub use super::views::*;

use ratatui::{buffer::Buffer, layout::Rect};

/// Render one frame exactly as the app does, with its tap targets (tests and capture).
pub fn render_frame(
    view: &LabView,
    state: &LabState,
    caps: &LabCaps,
    w: u16,
    h: u16,
) -> (Buffer, Vec<LabTarget>) {
    let area = Rect::new(0, 0, w, h);
    let mut buf = Buffer::empty(area);
    let drawn = mission::render(view, state, caps, area, &mut buf);
    (buf, drawn.targets)
}

impl LabView {
    /// The default capture set for a fixture.
    pub fn defaults(state: &LabState) -> Vec<LabView> {
        let device = state
            .devices
            .iter()
            .find(|d| d.id == "innovation")
            .or(state.devices.get(1))
            .map(|d| d.id.clone())
            .unwrap_or_default();
        let session = state
            .sessions
            .first()
            .map(|s| s.id.clone())
            .unwrap_or_default();
        let edge = state
            .edges
            .first()
            .map(|e| e.id.clone())
            .unwrap_or_default();
        vec![
            LabView::Hub,
            LabView::DeviceFocus(device),
            LabView::SessionPeek(session),
            LabView::Files,
            LabView::FilePreview,
            LabView::Palette("inno".into()),
            LabView::Secrets,
            LabView::Onboarding,
            LabView::JoinCode,
            LabView::EdgeEditor(edge),
            LabView::Help,
        ]
    }

    /// Parse `hub`, `device:<id>`, `session:<id>`, `files`, `file-preview`,
    /// `palette:<query>`, `secrets`, `onboarding`, `join`, `edge:<from-to>`, `help`.
    /// `device`, `session` and `edge` without an id take the fixture's default.
    pub fn parse(spec: &str, state: &LabState) -> Option<LabView> {
        let (head, arg) = match spec.split_once(':') {
            Some((h, a)) => (h, Some(a.to_string())),
            None => (spec, None),
        };
        let defaults = Self::defaults(state);
        let pick = |f: fn(&LabView) -> bool| defaults.iter().find(|v| f(v)).cloned();
        Some(match head {
            "hub" => LabView::Hub,
            "device" => match arg {
                Some(a) => LabView::DeviceFocus(a),
                None => pick(|v| matches!(v, LabView::DeviceFocus(_)))?,
            },
            "session" => match arg {
                Some(a) => LabView::SessionPeek(a),
                None => pick(|v| matches!(v, LabView::SessionPeek(_)))?,
            },
            "files" => LabView::Files,
            "file-preview" | "preview" => LabView::FilePreview,
            "palette" => LabView::Palette(arg.unwrap_or_default()),
            "secrets" => LabView::Secrets,
            "onboarding" => LabView::Onboarding,
            "join" | "join-code" => LabView::JoinCode,
            "edge" => match arg {
                Some(a) => LabView::EdgeEditor(a),
                None => pick(|v| matches!(v, LabView::EdgeEditor(_)))?,
            },
            "help" => LabView::Help,
            _ => return None,
        })
    }

    /// File-name slug.
    pub fn slug(&self) -> String {
        let clean = |s: &str| {
            s.chars()
                .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
                .collect::<String>()
        };
        match self {
            LabView::Hub => "hub".into(),
            LabView::DeviceFocus(d) => format!("device-{}", clean(d)),
            LabView::SessionPeek(s) => format!("session-{}", clean(s)),
            LabView::Files => "files".into(),
            LabView::FilePreview => "file-preview".into(),
            LabView::Palette(q) if q.is_empty() => "palette".into(),
            LabView::Palette(q) => format!("palette-{}", clean(q)),
            LabView::Secrets => "secrets".into(),
            LabView::Onboarding => "onboarding".into(),
            LabView::JoinCode => "join".into(),
            LabView::EdgeEditor(e) => format!("edge-{}", clean(e)),
            LabView::Help => "help".into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn view_specs_round_trip() {
        let s = state::typical();
        for v in LabView::defaults(&s) {
            let slug = v.slug();
            assert!(!slug.is_empty());
        }
        assert_eq!(
            LabView::parse("device:nas", &s),
            Some(LabView::DeviceFocus("nas".into()))
        );
        assert_eq!(
            LabView::parse("edge", &s),
            Some(LabView::EdgeEditor("tranquility-innovation".into()))
        );
        assert!(LabView::parse("nope", &s).is_none());
    }

    /// The design renders every default view, size, variant and fixture
    /// without panicking (sizes include degenerate ones).
    #[test]
    fn all_concepts_render_everywhere() {
        let sizes = [(1, 1), (10, 4), (40, 20), (200, 60)];
        for fx in FIXTURES {
            let s = LabState::fixture(fx).unwrap();
            for v in LabView::defaults(&s) {
                for variant in ["light-256", "ascii"] {
                    let caps = LabCaps::from_variant(variant).unwrap();
                    for (w, h) in sizes {
                        let (buf, _) = render_frame(&v, &s, &caps, w, h);
                        if w == 40 {
                            let _ = dump::dump(&buf, &caps);
                        }
                    }
                }
            }
        }
    }
}
#[cfg(test)]
mod map_tests {
    use super::*;
    /// A network whose drawing is one row (the viewer and its direct peers) still
    /// draws: the map's band was one row, under the three it needs.
    #[test]
    fn a_one_row_network_still_draws_its_map() {
        let mut s = state::small();
        s.edges
            .retain(|e| e.from == "tranquility" && e.to == "innovation");
        let (buf, _) = render_frame(&LabView::Hub, &s, &LabCaps::default(), 120, 40);
        let row = (2..20)
            .map(|y| {
                (0..120)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .find(|r| r.contains("tranquility"));
        let row = row.expect("map row");
        assert!(row.contains("innovation") && row.contains("nas"), "{row}");
    }
}
#[cfg(test)]
mod review_tests {
    //! v0.2.1 review: what the design's frames say and which targets they register.
    use super::*;
    fn text(buf: &Buffer) -> String {
        let a = buf.area;
        (0..a.height)
            .map(|y| {
                (0..a.width)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
    fn draw(v: LabView, s: &LabState, w: u16, h: u16) -> (String, Vec<LabTarget>) {
        let (buf, targets) = render_frame(&v, s, &LabCaps::default(), w, h);
        (text(&buf), targets)
    }

    /// A waiting session past the four slots has no `letter slot` address: the ticker
    /// never prints slot `a` (another session's) for it.
    #[test]
    fn the_ticker_never_prints_another_sessions_slot() {
        let mut s = state::typical();
        let on: Vec<LabSession> = s.sessions_on("innovation").cloned().collect();
        for i in on.len()..5 {
            let mut x = on[0].clone();
            x.id = format!("extra{i}");
            x.title = format!("extra {i}");
            x.state = SessionState::Idle;
            s.sessions.push(x);
        }
        let fifth = s
            .sessions
            .iter_mut()
            .rfind(|x| x.device == "innovation")
            .unwrap();
        fifth.title = "FIFTH".into();
        fifth.state = SessionState::AwaitingInput;
        // (Ranked after the other waiting session, so the ticker names it.)
        fifth.last_activity_secs = 99_999;
        let (t, targets) = draw(LabView::Hub, &s, 200, 60);
        let row = t
            .lines()
            .find(|l| l.contains("FIFTH needs you"))
            .expect("ticker row");
        assert!(row.contains("find") && !row.contains("attach"), "{row}");
        assert!(targets
            .iter()
            .any(|t| t.action.starts_with("session:innovation:extra")));
    }

    /// Transparent terminals: cx's own chrome paints no cell background in any tier
    /// (session screens keep their programs' own; here they are empty, and the
    /// invite's QR code keeps its modules).
    #[test]
    fn chrome_paints_no_background() {
        let mut s = state::typical();
        for x in &mut s.sessions {
            x.screen.clear();
        }
        for variant in ["dark-truecolor", "light-truecolor", "dark-16", "ascii"] {
            let base = LabCaps::from_variant(variant).unwrap();
            for v in LabView::defaults(&s)
                .into_iter()
                .filter(|v| *v != LabView::JoinCode)
            {
                for focus in [None, Some(Pane::Sessions)] {
                    let caps = LabCaps { focus, ..base };
                    for (w, h) in [(40, 20), (120, 40), (190, 50)] {
                        let (buf, _) = render_frame(&v, &s, &caps, w, h);
                        let painted = buf.content.iter().position(|c| {
                            c.bg != ratatui::style::Color::Reset
                                || c.modifier.contains(ratatui::style::Modifier::REVERSED)
                        });
                        assert!(
                            painted.is_none(),
                            "{} {variant} {w}x{h}: cell {painted:?} has a background",
                            v.slug()
                        );
                    }
                }
            }
        }
    }

    /// A device that was never seen says so, not an age counted from the epoch
    /// (`✕ 20770d`).
    #[test]
    fn a_never_seen_device_shows_no_age() {
        let mut s = state::typical();
        let d = s.devices.iter_mut().find(|d| d.id == "nas").unwrap();
        d.status = DeviceStatus::Down;
        d.observed_secs = NEVER_SEEN;
        let (t, _) = draw(LabView::Hub, &s, 40, 20);
        let row = t.lines().find(|l| l.contains("nas")).expect("nas row");
        assert!(row.contains("never seen") && !row.contains("d "), "{row}");
        assert_eq!(fmt::age(NEVER_SEEN), "never");
    }

    /// The ticker acts on the device its notice is about, not on a device whose name
    /// starts the text (`lab` and `lab-arm`).
    #[test]
    fn a_notice_retries_its_own_device() {
        let mut s = state::typical();
        s.devices[1].name = "lab".into();
        s.devices[2].name = "lab-arm".into();
        let (near, far) = (s.devices[1].id.clone(), s.devices[2].id.clone());
        s.notifications = vec![LabNotice {
            kind: NoticeKind::Warn,
            device: Some(far.clone()),
            text: "lab-arm: timeout".into(),
            age_secs: 0,
            action: Some("retry".into()),
        }];
        let (_, targets) = draw(LabView::Hub, &s, 200, 60);
        let acts: Vec<&str> = targets.iter().map(|t| t.action.as_str()).collect();
        assert!(acts.contains(&format!("retry:{far}").as_str()), "{acts:?}");
        assert!(
            !acts.contains(&format!("retry:{near}").as_str()),
            "{acts:?}"
        );
    }

    /// The link editor draws the link it was opened on (either order), never the first
    /// stored one; the crumb names the opener first, as the sheet does.
    #[test]
    fn the_link_editor_never_draws_another_link() {
        let s = state::typical();
        let (t, _) = draw(
            LabView::EdgeEditor("creation-innovation".into()),
            &s,
            200,
            60,
        );
        let crumb = t.lines().next().unwrap();
        assert!(crumb.contains(" c  ⇄  i "), "{crumb}");
        let (t, _) = draw(LabView::EdgeEditor("innovation-nas".into()), &s, 200, 60);
        assert!(!t.contains("tranquility"), "{t}");
    }

    /// A device's session tile is as tall as its screen's text: the `new` row follows
    /// it, with no band of blank rows inside the tile.
    #[test]
    fn a_session_tile_is_no_taller_than_its_text() {
        let s = state::typical();
        let (t, _) = draw(LabView::DeviceFocus("innovation".into()), &s, 200, 60);
        let rows: Vec<&str> = t.lines().collect();
        let text_end = rows.iter().position(|r| r.contains("Refactoring")).unwrap();
        let ghost = rows.iter().position(|r| r.contains(" new ")).unwrap();
        assert!(ghost - text_end <= 3, "{t}");
    }

    /// First run: the viewer alone, no braille rings or beads (rejected 2026-11-10),
    /// so nothing touches its label at any size.
    #[test]
    fn onboarding_draws_no_braille_orbit() {
        let s = state::typical();
        for (w, h) in [(40, 20), (120, 40), (200, 60)] {
            let (t, _) = draw(LabView::Onboarding, &s, w, h);
            assert!(
                !t.chars()
                    .any(|c| ('\u{2800}'..='\u{28ff}').contains(&c) || c == '◌'),
                "{w}x{h}:\n{t}"
            );
            assert!(t.contains("tranquility"));
        }
    }
}
#[cfg(test)]
mod size_fuzz {
    //! Every production view, for realistic fixtures, at every terminal size from 20x5
    //! to 250x80 (half terminals, phones, tall and narrow): drawing never panics.
    use super::*;

    /// The fixture with only its first `n` devices (and what still refers to them).
    fn first(n: usize) -> LabState {
        let mut s = state::typical();
        s.devices.truncate(n);
        let ids: Vec<String> = s.devices.iter().map(|d| d.id.clone()).collect();
        let has = |id: &String| ids.contains(id);
        s.edges.retain(|e| has(&e.from) && has(&e.to));
        s.sessions.retain(|x| has(&x.device));
        for m in &mut s.meshes {
            m.members.retain(|id| has(id));
        }
        s.notifications
            .retain(|n| n.device.as_ref().is_none_or(|d| has(d)));
        s
    }

    pub(crate) fn fixtures() -> Vec<(String, LabState)> {
        let mut v: Vec<(String, LabState)> =
            (0..=7).map(|n| (format!("{n}-devices"), first(n))).collect();
        let mut down = state::typical();
        for (i, d) in down.devices.iter_mut().enumerate().skip(1) {
            d.status = if i % 2 == 0 {
                DeviceStatus::Down
            } else {
                DeviceStatus::Unknown
            };
            d.metrics = None;
            // (Never seen: no sighting at all.)
            d.observed_secs = u32::MAX;
        }
        for e in &mut down.edges {
            e.up = false;
            e.rtt_ms = None;
        }
        v.push(("unreachable".into(), down));
        let mut flat = state::typical();
        flat.meshes.clear();
        for d in &mut flat.devices {
            d.meshes.clear();
        }
        for e in &mut flat.edges {
            if matches!(e.kind, EdgeKind::Mesh(_)) {
                e.kind = EdgeKind::Direct;
            }
        }
        v.push(("no-meshes".into(), flat));
        let mut long = state::typical();
        for d in &mut long.devices {
            d.name = format!("{}-with-a-very-long-descriptive-hostname-0123456789", d.name);
            d.account = "a-very-long-account-name-for-this-machine".into();
        }
        for x in &mut long.sessions {
            x.title = format!("{} and a title that goes on and on past any tile", x.title);
            x.cwd = format!("~/{}", "deeply/nested/folder/".repeat(8));
        }
        v.push(("long-names".into(), long));
        v.push(("many-sessions".into(), state::large()));
        let mut blank = state::typical();
        for x in &mut blank.sessions {
            x.screen.clear();
        }
        v.push(("empty-previews".into(), blank));
        v
    }

    fn views(s: &LabState) -> Vec<(LabView, Option<Pane>)> {
        let mut v: Vec<(LabView, Option<Pane>)> =
            LabView::defaults(s).into_iter().map(|v| (v, None)).collect();
        let session = s.sessions.first().map(|x| x.id.clone()).unwrap_or_default();
        let device = s.devices.last().map(|d| d.id.clone()).unwrap_or_default();
        v.extend([
            (LabView::Hub, Some(Pane::Sessions)),
            (LabView::Hub, Some(Pane::Graph)),
            (LabView::DeviceFocus(device.clone()), Some(Pane::Sessions)),
            (LabView::DeviceFocus(device), Some(Pane::Custom("stop"))),
            (LabView::SessionPeek(session), Some(Pane::Custom("peek"))),
            (LabView::Files, Some(Pane::Dialog)),
        ]);
        v
    }

    fn sizes() -> Vec<(u16, u16)> {
        let ws = (20..=140).chain((150..=250).step_by(10));
        ws.flat_map(|w| (5..=60).chain([70, 80]).map(move |h| (w, h)))
            .collect()
    }

    thread_local! {
        static WHERE: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
    }

    /// Every view, fixture and tier over `sizes`; each panic site with its first case
    /// and count.
    fn sweep(sizes: &[(u16, u16)]) -> Vec<String> {
        let caps = [LabCaps::default(), LabCaps::from_variant("ascii").unwrap()];
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(|info| {
            let at = info.location().map(|l| l.to_string()).unwrap_or_default();
            let what = info
                .payload()
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| info.payload().downcast_ref::<&str>().map(|s| s.to_string()))
                .unwrap_or_default();
            WHERE.with(|w| *w.borrow_mut() = format!("{at} {what}"));
        }));
        let cases = fixtures();
        let fails = std::sync::Mutex::new(std::collections::BTreeMap::<String, (usize, String)>::new());
        std::thread::scope(|sc| {
            for chunk in cases.chunks(cases.len().div_ceil(4)) {
                let (fails, caps) = (&fails, &caps);
                sc.spawn(move || {
                    for (name, s) in chunk {
                        for (v, focus) in views(s) {
                            for (ci, c) in caps.iter().enumerate() {
                                for &(w, h) in sizes {
                                    let c = LabCaps {
                                        focus,
                                        selected: usize::from(w) % 5,
                                        ..*c
                                    };
                                    let drew = std::panic::catch_unwind(
                                        std::panic::AssertUnwindSafe(|| {
                                            let area = Rect::new(0, 0, w, h);
                                            let mut buf = Buffer::empty(area);
                                            mission::render(&v, s, &c, area, &mut buf);
                                            mission::render_palette(s, &c, area, &mut buf, "in");
                                        }),
                                    );
                                    if drew.is_err() {
                                        let at = WHERE.with(|w| w.borrow().clone());
                                        let mut f = fails.lock().unwrap();
                                        let e = f.entry(at).or_insert((0, String::new()));
                                        e.0 += 1;
                                        if e.1.is_empty() {
                                            e.1 = format!(
                                                "{name} {} {focus:?} {w}x{h} ascii={}",
                                                v.slug(),
                                                ci == 1
                                            );
                                        }
                                    }
                                }
                            }
                        }
                    }
                });
            }
        });
        std::panic::set_hook(prev);
        fails
            .into_inner()
            .unwrap()
            .into_iter()
            .map(|(at, (n, first))| format!("{at}: {n}x, first {first}"))
            .collect()
    }

    fn check(fails: Vec<String>) {
        assert!(
            fails.is_empty(),
            "{} panic sites: {:#?}",
            fails.len(),
            fails
        );
    }

    /// The suite's share: every view and fixture at the widths and heights where the
    /// layouts change (phone, touch, size classes, half terminals).
    #[test]
    fn every_view_renders_across_sizes() {
        let ws = [20, 21, 40, 59, 60, 69, 70, 80, 95, 99, 100, 120, 159, 160, 190, 250];
        let hs = [5, 6, 7, 15, 16, 20, 28, 36, 48, 50];
        let sizes: Vec<(u16, u16)> = ws.iter().flat_map(|&w| hs.map(|h| (w, h))).collect();
        check(sweep(&sizes));
    }

    /// The whole matrix: 20x5 to 250x80, every width to 140 and every height to 60.
    /// Run it with overflow checks in an optimised build, e.g.
    /// `CARGO_PROFILE_RELEASE_OVERFLOW_CHECKS=true cargo test --release -- --ignored size_fuzz`.
    #[test]
    #[ignore]
    fn every_view_renders_at_every_size() {
        check(sweep(&sizes()));
    }
}
