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

    /// With rounded caps (kitty, ghostty, wezterm, `CX_NERD_FONT=1`) key chips are pills:
    /// the only cells with a background, and each between its two caps. Phones, 16
    /// colours and ASCII still paint nothing.
    #[test]
    fn pills_are_the_only_background_and_always_capped() {
        use crate::ui::views::mission::{PILL_L, PILL_R};
        let mut s = state::typical();
        // (A session's own screen may paint backgrounds: not chrome.)
        for x in &mut s.sessions {
            x.screen.clear();
        }
        for variant in [
            "dark-truecolor",
            "light-truecolor",
            "dark-256",
            "dark-16",
            "ascii",
        ] {
            let base = LabCaps {
                pills: true,
                ..LabCaps::from_variant(variant).unwrap()
            };
            let mut pills = 0;
            for v in LabView::defaults(&s)
                .into_iter()
                .filter(|v| *v != LabView::JoinCode)
            {
                for (w, h) in [(40, 20), (120, 40), (190, 50)] {
                    let (buf, _) = render_frame(&v, &s, &base, w, h);
                    for y in 0..h {
                        let mut open = false;
                        for x in 0..w {
                            let c = &buf[(x, y)];
                            match c.symbol() {
                                PILL_L => {
                                    open = true;
                                    pills += 1;
                                }
                                PILL_R => open = false,
                                _ => assert!(
                                    open || c.bg == ratatui::style::Color::Reset,
                                    "{} {variant} {w}x{h}: ({x},{y}) painted outside a pill",
                                    v.slug()
                                ),
                            }
                            assert!(
                                !open || (w >= 70 && base.unicode && variant != "dark-16"),
                                "{} {variant} {w}x{h}: a pill at ({x},{y})",
                                v.slug()
                            );
                        }
                        assert!(!open, "{} {variant} {w}x{h}: row {y} unclosed", v.slug());
                    }
                }
            }
            let want = matches!(variant, "dark-truecolor" | "light-truecolor" | "dark-256");
            assert_eq!(pills > 0, want, "{variant}: {pills} pills");
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

    /// A device's session tile is a large screen (v0.2.2: fewer, bigger previews): it
    /// takes the rows the links leave, shows its whole text, and the `new` row follows.
    #[test]
    fn a_device_session_tile_is_a_large_screen() {
        let s = state::typical();
        let (t, _) = draw(LabView::DeviceFocus("innovation".into()), &s, 200, 60);
        let rows: Vec<&str> = t.lines().collect();
        let title = rows
            .iter()
            .position(|r| r.contains("refactor transport"))
            .unwrap();
        let text_end = rows.iter().position(|r| r.contains("Refactoring")).unwrap();
        let ghost = rows.iter().position(|r| r.contains(" new ")).unwrap();
        assert!(
            title < text_end && text_end < ghost && ghost - title >= 16,
            "{t}"
        );
    }

    /// First run: the viewer alone in the middle of its quiet dotted orbits (round-67,
    /// asked back 2026-11-12), and no dot within two cells of its name on its row.
    #[test]
    fn onboarding_draws_quiet_orbits_clear_of_the_name() {
        let s = state::typical();
        for (w, h) in [(40, 20), (80, 24), (120, 40), (200, 60)] {
            let (t, _) = draw(LabView::Onboarding, &s, w, h);
            let dots = t
                .chars()
                .filter(|c| ('\u{2801}'..='\u{28ff}').contains(c))
                .count();
            assert!(dots > 8, "{w}x{h}: no orbit\n{t}");
            let row = t.lines().find(|l| l.contains("◉ tranquility")).unwrap();
            let chars: Vec<char> = row.chars().collect();
            let at = row.chars().position(|c| c == '◉').unwrap();
            let end = at + "◉ tranquility".chars().count();
            let near = &chars[at.saturating_sub(2)..(end + 2).min(chars.len())];
            assert!(
                !near.iter().any(|c| ('\u{2800}'..='\u{28ff}').contains(c)),
                "{w}x{h}: a dot touches the name: {row}"
            );
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
            .retain(|n| n.device.as_ref().is_none_or(has));
        s
    }

    pub(crate) fn fixtures() -> Vec<(String, LabState)> {
        let mut v: Vec<(String, LabState)> = (0..=7)
            .map(|n| (format!("{n}-devices"), first(n)))
            .collect();
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
            d.name = format!(
                "{}-with-a-very-long-descriptive-hostname-0123456789",
                d.name
            );
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
        // The hub's nebula: a mid-size and a full network (both fold clusters).
        for n in [10, 25] {
            v.push((format!("nebula-{n}"), state::nebula(n)));
        }
        v
    }

    fn views(s: &LabState) -> Vec<(LabView, Option<Pane>)> {
        let mut v: Vec<(LabView, Option<Pane>)> = LabView::defaults(s)
            .into_iter()
            .map(|v| (v, None))
            .collect();
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
        let fails =
            std::sync::Mutex::new(std::collections::BTreeMap::<String, (usize, String)>::new());
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
                                            mission::render_palette(s, &c, area, &mut buf, "in", 2);
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
        let ws = [
            20, 21, 40, 59, 60, 69, 70, 80, 95, 99, 100, 120, 159, 160, 190, 250,
        ];
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
#[cfg(test)]
mod nebula_tests {
    //! The hub's nebula: mirror-drawn halves, folding clusters, and arrow moves that
    //! reverse, reach every device and never go against their key.
    use super::*;
    use ratatui::style::{Color, Modifier};
    use std::collections::{BTreeSet, VecDeque};

    const SIZES: [(u16, u16); 4] = [(80, 24), (95, 50), (120, 40), (190, 50)];

    fn fixtures() -> Vec<LabState> {
        let mut v: Vec<LabState> = [1, 3, 5, 7, 10, 15, 25]
            .iter()
            .map(|n| LabState::fixture(&format!("nebula-{n}")).unwrap())
            .collect();
        v.extend(FIXTURES.iter().map(|f| LabState::fixture(f).unwrap()));
        v
    }

    fn map_caps(selected: usize) -> LabCaps {
        LabCaps {
            focus: Some(Pane::Graph),
            selected,
            ..LabCaps::default()
        }
    }

    fn drawn(s: &LabState, sel: usize, w: u16, h: u16) -> (Buffer, mission::Drawn) {
        let area = Rect::new(0, 0, w, h);
        let mut buf = Buffer::empty(area);
        let d = mission::render(&LabView::Hub, s, &map_caps(sel), area, &mut buf);
        (buf, d)
    }

    fn centre(r: Rect) -> (f32, f32) {
        (
            f32::from(r.x) + f32::from(r.width) / 2.0,
            f32::from(r.y) + f32::from(r.height) / 2.0,
        )
    }

    /// Braille dots mirrored left to right.
    fn flip(c: char) -> char {
        let b = c as u32 - 0x2800;
        let pairs = [(0x01, 0x08), (0x02, 0x10), (0x04, 0x20), (0x40, 0x80)];
        let mut o = 0;
        for (l, r) in pairs {
            if b & l != 0 {
                o |= r;
            }
            if b & r != 0 {
                o |= l;
            }
        }
        char::from_u32(0x2800 + o).unwrap()
    }

    /// A network that is its own mirror draws as one: every braille dot of its links,
    /// rings and halo has its mirror twin, and every node sits where its twin's
    /// mirror is.
    #[test]
    fn a_symmetric_network_draws_as_a_mirror_image() {
        let s = LabState::fixture("mirror").unwrap();
        for (w, h) in [(120, 40), (190, 50), (95, 50), (160, 44)] {
            let (buf, d) = drawn(&s, 0, w, h);
            let rects: Vec<(Rect, String)> = d
                .nav
                .iter()
                .filter(|t| t.action.starts_with("device:"))
                .map(|t| (t.area, t.action.clone()))
                .collect();
            let root = rects
                .iter()
                .find(|(_, a)| *a == format!("device:{}", s.viewer))
                .unwrap()
                .0;
            let a = 2 * i32::from(root.x) + i32::from(root.width);
            let rows = rects
                .iter()
                .map(|(r, _)| r.y)
                .min()
                .unwrap()
                .saturating_sub(2)
                ..rects.iter().map(|(r, _)| r.bottom()).max().unwrap() + 2;
            let mut dots = 0;
            for y in rows.clone() {
                for x in 0..w {
                    let c = buf[(x, y)].symbol().chars().next().unwrap_or(' ');
                    if !('\u{2801}'..='\u{28ff}').contains(&c) {
                        continue;
                    }
                    dots += 1;
                    let mx = a - 1 - i32::from(x);
                    assert!(
                        mx >= 0 && mx < i32::from(w),
                        "{w}x{h}: ({x},{y}) has no twin"
                    );
                    let twin = buf[(mx as u16, y)].symbol().chars().next().unwrap_or(' ');
                    assert_eq!(
                        twin,
                        flip(c),
                        "{w}x{h}: ({x},{y}) {c} against ({mx},{y}) {twin}"
                    );
                }
            }
            assert!(dots > 20, "{w}x{h}: {dots} dots");
            for (r, act) in &rects {
                let m = Rect::new(
                    (a - i32::from(r.x) - i32::from(r.width)) as u16,
                    r.y,
                    r.width,
                    r.height,
                );
                assert!(
                    rects.iter().any(|(q, _)| *q == m),
                    "{w}x{h}: {act} at {r:?} has no mirror twin at {m:?}"
                );
            }
        }
    }

    /// The folded clusters' letter rows of a frame: each row's cells, inner end
    /// first, found under every cluster mark (`◍`, `*` in ASCII).
    fn letter_rows(buf: &Buffer) -> Vec<Vec<(u16, u16)>> {
        let (w, h) = (buf.area.width, buf.area.height);
        let mut rows = Vec::new();
        for y in 0..h - 1 {
            for x in 0..w {
                // (A mark with its `+N` count beside it: not a `*` in session text.)
                let line: String = (x.saturating_sub(30)..(x + 30).min(w))
                    .map(|i| buf[(i, y)].symbol())
                    .collect();
                let counted = line
                    .split('+')
                    .skip(1)
                    .any(|t| t.starts_with(|c: char| c.is_ascii_digit()));
                if !["◍", "*"].contains(&buf[(x, y)].symbol()) || !counted {
                    continue;
                }
                let right = x + 2 < w
                    && buf[(x + 2, y)]
                        .symbol()
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '-');
                let (mut cx, mut gap, mut row) = (i32::from(x), 0, Vec::new());
                let step = if right { 1 } else { -1 };
                while cx >= 0 && cx < i32::from(w) {
                    let sym = buf[(cx as u16, y + 1)].symbol();
                    if sym == " " {
                        gap += 1;
                        if !row.is_empty() && gap > 1 {
                            // A double space with letters after it: a missing tooth.
                            let n = cx + step;
                            let more = n >= 0
                                && n < i32::from(w)
                                && buf[(n as u16, y + 1)]
                                    .symbol()
                                    .chars()
                                    .all(|c| c.is_ascii_lowercase());
                            assert!(!more, "a gap in the letter row at ({cx},{})", y + 1);
                            break;
                        }
                        if row.is_empty() && gap > 3 {
                            break;
                        }
                    } else if sym
                        .chars()
                        .all(|c| c.is_ascii_lowercase() || "·…:~".contains(c))
                        && sym.chars().count() == 1
                    {
                        gap = 0;
                        row.push((cx as u16, y + 1));
                    } else {
                        break;
                    }
                    cx += step;
                }
                if row.len() >= 3 {
                    rows.push(row);
                }
            }
        }
        rows
    }

    /// A folded cluster's member letters read cleanly in every tier: an even pitch (one
    /// space between cells, never a gap), up members in the cluster's one tone (a drift
    /// of a few percent at most in truecolor, one colour otherwise), then at most one
    /// dim `·` and the down and never-seen members in the theme's neutral dim ink (the
    /// same ink in every cluster, so no hue), never a ✕ or ! marker in the row.
    #[test]
    fn folded_member_letters_share_one_tone() {
        let tiers = [
            (ColorLevel::Truecolor, Theme::Dark, true),
            (ColorLevel::Truecolor, Theme::Light, true),
            (ColorLevel::C16, Theme::Dark, true),
            (ColorLevel::None, Theme::Dark, false),
        ];
        let mut seen = 0;
        for f in ["large", "nebula-25", "nebula-15"] {
            let s = LabState::fixture(f).unwrap();
            for (w, h) in [(120, 40), (190, 50), (95, 50)] {
                for (color, theme, unicode) in tiers {
                    let caps = LabCaps {
                        color,
                        theme,
                        unicode,
                        ..map_caps(0)
                    };
                    let area = Rect::new(0, 0, w, h);
                    let mut buf = Buffer::empty(area);
                    mission::render(&LabView::Hub, &s, &caps, area, &mut buf);
                    let rows = letter_rows(&buf);
                    let mut downs = BTreeSet::new();
                    for row in &rows {
                        let at = format!("{f} {w}x{h} {color:?} {theme:?}: row at {:?}", row[0]);
                        let mut xs: Vec<u16> = row.iter().map(|c| c.0).collect();
                        xs.sort();
                        assert!(
                            xs.windows(2).all(|p| p[1] - p[0] == 2),
                            "{at}: uneven pitch"
                        );
                        let syms: Vec<&str> = row.iter().map(|&c| buf[c].symbol()).collect();
                        assert!(
                            syms.iter().all(|t| !["✕", "×", "!", " "].contains(t)),
                            "{at}: {syms:?}"
                        );
                        let sep = ["·", ":"];
                        assert!(syms.iter().filter(|t| sep.contains(t)).count() <= 1, "{at}");
                        let split = syms.iter().position(|t| sep.contains(t));
                        let letter = |t: &&str| t.chars().all(|c| c.is_ascii_lowercase());
                        let ups: Vec<_> = row[..split.unwrap_or(row.len())]
                            .iter()
                            .filter(|&&c| letter(&buf[c].symbol()))
                            .map(|&c| (buf[c].fg, buf[c].modifier))
                            .collect();
                        let dn: BTreeSet<String> = split
                            .map_or(&row[..0], |i| &row[i + 1..])
                            .iter()
                            .filter(|&&c| letter(&buf[c].symbol()))
                            .map(|&c| format!("{:?}{:?}", buf[c].fg, buf[c].modifier))
                            .collect();
                        assert!(dn.len() <= 1, "{at}: down letters in {} inks", dn.len());
                        downs.extend(dn);
                        let Some(&(f0, m0)) = ups.first() else {
                            continue;
                        };
                        seen += 1;
                        for &(fg, m) in &ups {
                            match (f0, fg) {
                                (Color::Rgb(r0, g0, b0), Color::Rgb(r, g, b)) => {
                                    let d = [(r0, r), (g0, g), (b0, b)]
                                        .iter()
                                        .map(|&(a, b)| (i32::from(a) - i32::from(b)).abs())
                                        .max()
                                        .unwrap();
                                    assert!(d <= 24, "{at}: letters drift {d} apart");
                                }
                                _ => assert_eq!((f0, m0), (fg, m), "{at}: letters in two inks"),
                            }
                        }
                    }
                    assert!(
                        downs.len() <= 1,
                        "{f} {w}x{h} {color:?} {theme:?}: down ink varies by cluster {downs:?}"
                    );
                }
            }
        }
        assert!(seen > 20, "only {seen} folded rows seen");
    }

    /// Pointing at a member letter names that device on the cluster's title line, and
    /// the letter turns the accent, bold and underlined.
    #[test]
    fn a_pointed_at_letter_names_its_device() {
        let s = LabState::fixture("nebula-25").unwrap();
        let (buf, _) = drawn(&s, 0, 120, 40);
        let rows = letter_rows(&buf);
        assert!(!rows.is_empty());
        for row in rows {
            let &(x, y) = row
                .iter()
                .find(|&&c| buf[c].symbol().chars().all(|c| c.is_ascii_lowercase()))
                .unwrap();
            let k = buf[(x, y)].symbol().chars().next().unwrap();
            let d = s.devices.iter().find(|d| d.hotkey == k).unwrap();
            let caps = LabCaps {
                hover: Some((x, y)),
                ..map_caps(0)
            };
            let area = Rect::new(0, 0, 120, 40);
            let mut hb = Buffer::empty(area);
            mission::render(&LabView::Hub, &s, &caps, area, &mut hb);
            let title: String = (0..120).map(|i| hb[(i, y - 1)].symbol()).collect();
            let stem: String = d.name.chars().take(6).collect();
            assert!(title.contains(&stem), "hover {k}: {title:?} lacks {stem}");
            assert!(hb[(x, y)].modifier.contains(Modifier::UNDERLINED));
            assert_ne!(
                hb[(x, y)].fg,
                buf[(x, y)].fg,
                "hover {k}: the letter keeps its ink"
            );
        }
    }

    /// In every frame: a move never goes against its key; going back and forth settles
    /// (back from where a move lands leads to a node that moves there again, the very
    /// node it came from when the two are paired); every node drawn is reached from
    /// the root.
    #[test]
    fn moves_reverse_reach_everything_and_never_invert() {
        for s in fixtures() {
            let order = mission::by_attention(&s).len(); // (warm the fixture)
            let _ = order;
            for (w, h) in SIZES {
                let n = s.devices.len();
                for sel in 0..n {
                    let (_, d) = drawn(&s, sel, w, h);
                    if d.moves.is_empty() {
                        continue;
                    }
                    let at = |id: &str| {
                        d.nav
                            .iter()
                            .find(|t| t.action == id)
                            .map(|t| centre(t.area))
                            .unwrap()
                    };
                    let mv = |id: &str, k: usize| {
                        d.moves
                            .iter()
                            .find(|(x, _)| x == id)
                            .and_then(|(_, m)| m[k].clone())
                    };
                    let dirs = [(-1.0, 0.0), (1.0, 0.0), (0.0, -1.0), (0.0, 1.0)];
                    for (id, m) in &d.moves {
                        for (k, to) in m.iter().enumerate() {
                            let Some(to) = to else { continue };
                            let (p, q) = (at(id), at(to));
                            let along = (q.0 - p.0) * dirs[k].0 + (q.1 - p.1) * dirs[k].1;
                            assert!(
                                along > 0.0,
                                "{} {w}x{h} sel {sel}: {id} key {k} goes back to {to}",
                                s.fixture
                            );
                            let back = mv(to, k ^ 1).unwrap_or_else(|| {
                                panic!("{} {w}x{h}: no way back from {to}", s.fixture)
                            });
                            assert_eq!(
                                mv(&back, k).as_deref(),
                                Some(to.as_str()),
                                "{} {w}x{h} sel {sel}: {id} -{k}-> {to} -back-> {back}",
                                s.fixture
                            );
                        }
                    }
                    let root = format!("device:{}", s.viewer);
                    let mut seen = BTreeSet::from([root.clone()]);
                    let mut q = VecDeque::from([root]);
                    while let Some(x) = q.pop_front() {
                        for k in 0..4 {
                            if let Some(y) = mv(&x, k) {
                                if seen.insert(y.clone()) {
                                    q.push_back(y);
                                }
                            }
                        }
                    }
                    for (id, _) in &d.moves {
                        assert!(
                            seen.contains(id),
                            "{} {w}x{h} sel {sel}: {id} unreachable",
                            s.fixture
                        );
                    }
                }
            }
        }
    }

    /// Walking the arrows from this device reaches every device, folded ones too (a
    /// folded cluster's node opens it on a member, whose cluster then blooms): nothing
    /// is skipped silently.
    #[test]
    fn the_arrows_reach_every_device() {
        for s in fixtures() {
            for (w, h) in SIZES {
                let (_, d0) = drawn(&s, 0, w, h);
                if d0.moves.is_empty() {
                    continue;
                }
                let order = d0.order.clone();
                let idx = |id: &str| order.iter().position(|x| x == id).unwrap();
                let mut seen = BTreeSet::from([s.viewer.clone()]);
                let mut q = VecDeque::from([s.viewer.clone()]);
                while let Some(x) = q.pop_front() {
                    let (_, d) = drawn(&s, idx(&x), w, h);
                    let me = format!("device:{x}");
                    let m = d.moves.iter().find(|(id, _)| *id == me);
                    let m = m.unwrap_or_else(|| panic!("{} {w}x{h}: {x} not drawn", s.fixture));
                    for y in m.1.iter().flatten() {
                        let y = y.trim_start_matches("device:").to_string();
                        if seen.insert(y.clone()) {
                            q.push_back(y);
                        }
                    }
                }
                let all: BTreeSet<String> = s.devices.iter().map(|d| d.id.clone()).collect();
                let missed: Vec<&String> = all.difference(&seen).collect();
                assert!(
                    missed.is_empty(),
                    "{} {w}x{h}: never reached {missed:?} from {:?}",
                    s.fixture,
                    d0.moves
                );
            }
        }
    }

    /// A big network folds every cluster but the selection's into one node with its
    /// count; selecting into a cluster blooms it (its devices drawn by name).
    #[test]
    fn clusters_fold_with_a_count_and_bloom_on_focus() {
        let s = LabState::fixture("nebula-25").unwrap();
        let (buf, d) = drawn(&s, 0, 120, 40);
        let t = dump::text(&buf);
        assert!(
            t.lines()
                .any(|l| l.contains("+") && l.contains("cloudflare-warp")),
            "{t}"
        );
        let order = d.order.clone();
        let inno = order.iter().position(|x| x == "innovation").unwrap();
        let (buf, _) = drawn(&s, inno, 120, 40);
        let t = dump::text(&buf);
        assert!(t.contains("innovation"), "{t}");
        // Every letter stays a key: a folded cluster lists its members' letters.
        let (buf, _) = drawn(&s, 0, 190, 50);
        let t = dump::text(&buf);
        for d in s.devices.iter().skip(1) {
            assert!(
                t.contains(&d.name)
                    || t.contains(&format!(" {} ", d.hotkey))
                    || t.contains(&format!("{} ", d.hotkey)),
                "{} hidden:\n{t}",
                d.name
            );
        }
    }

    /// Mid-glide frames draw (and never panic) at every fixture and size, and a settled
    /// frame draws the same as no motion at all.
    #[test]
    fn transitions_draw_and_settle() {
        for s in fixtures() {
            for (w, h) in SIZES {
                let n = s.devices.len().min(6);
                for sel in 0..n {
                    for from in 0..n {
                        for t in [0, 300, 700] {
                            let caps = LabCaps {
                                motion: crate::ui::views::Motion {
                                    from: Some(from as u16),
                                    t,
                                },
                                ..map_caps(sel)
                            };
                            let _ = render_frame(&LabView::Hub, &s, &caps, w, h);
                        }
                    }
                    let settled = LabCaps {
                        motion: crate::ui::views::Motion {
                            from: Some(0),
                            t: 1000,
                        },
                        ..map_caps(sel)
                    };
                    let (a, _) = render_frame(&LabView::Hub, &s, &settled, w, h);
                    let (b, _) = render_frame(&LabView::Hub, &s, &map_caps(sel), w, h);
                    assert_eq!(a, b, "{} {w}x{h} sel {sel}", s.fixture);
                }
            }
        }
    }
}
