//! Production view layer: the Mission design (docs/design/concept-mission.md) drawn
//! from a pure view-model (`LabState`, filled by `ui::viewmodel` from the App, or by the
//! lab fixtures in tests) into a ratatui buffer. Nothing here touches App, IO, tmux or
//! the network; rendering never dispatches requests. The test-only lab harness
//! (`ui::lab`) captures the same code from fixtures.
#![allow(dead_code)]

pub mod caps;
pub mod mission;
pub mod state;

#[allow(unused_imports)]
pub use caps::{ColorLevel, LabCaps, Pane, Theme, Tok};
pub use state::*;

use ratatui::{buffer::Buffer, layout::Rect, text::Line};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LabView {
    /// Start view: the network hub.
    Hub,
    /// Hub zoomed onto one device (device id): its sessions, ready to start/attach.
    DeviceFocus(String),
    /// A session's live peek (session id).
    SessionPeek(String),
    Files,
    FilePreview,
    /// Command palette / jump-to-anything with the typed query.
    Palette(String),
    Secrets,
    /// First run / "Connect Cloudflare".
    Onboarding,
    /// Pending join invite: short code, one-liner, QR.
    JoinCode,
    /// Access policy editor for an edge (edge id "from-to").
    EdgeEditor(String),
    Help,
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

/// A tap/click target a concept exposes (dumped for the touch-target overlay).
#[derive(Clone, Debug, PartialEq)]
pub struct LabTarget {
    pub area: Rect,
    pub action: String,
}

pub trait Concept {
    fn name(&self) -> &str;
    /// One line for the contact sheet header.
    fn summary(&self) -> &str {
        ""
    }
    fn render(&self, view: LabView, state: &LabState, caps: &LabCaps, area: Rect, buf: &mut Buffer);
    /// Click/tap targets for the same frame. Optional; drives the `.targets.png` overlay.
    fn targets(
        &self,
        _view: &LabView,
        _state: &LabState,
        _caps: &LabCaps,
        _area: Rect,
    ) -> Vec<LabTarget> {
        Vec::new()
    }
}

/// Parse a SGR-colored screen snapshot (as in `LabSession::screen`) into lines, padding
/// each to `cols` with the running style, like the real preview does.
pub fn ansi_lines(screen: &[String], cols: u16) -> Vec<Line<'static>> {
    super::rendering::ansi_screen(screen, None, cols)
}

/// Shared tiny helpers for concepts.
pub mod fmt {
    pub fn age(secs: u32) -> String {
        match secs {
            0..=59 => format!("{secs}s"),
            60..=3599 => format!("{}m", secs / 60),
            3600..=86399 => format!("{}h", secs / 3600),
            _ => format!("{}d", secs / 86400),
        }
    }
    pub fn bytes(n: u64) -> String {
        const U: [&str; 5] = ["B", "K", "M", "G", "T"];
        let mut v = n as f64;
        let mut i = 0;
        while v >= 1024.0 && i < U.len() - 1 {
            v /= 1024.0;
            i += 1;
        }
        if i == 0 {
            format!("{n}B")
        } else if v < 10.0 {
            format!("{v:.1}{}", U[i])
        } else {
            format!("{v:.0}{}", U[i])
        }
    }
    /// Eighth-block sparkline (or 5-level ASCII) for values in 0..=1; None is a gap.
    pub fn spark(values: &[Option<f32>], unicode: bool) -> String {
        const U: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
        const A: [char; 5] = ['_', '.', '-', '=', '#'];
        values
            .iter()
            .map(|v| match v {
                None => {
                    if unicode {
                        '·'
                    } else {
                        ' '
                    }
                }
                Some(v) => {
                    let v = v.clamp(0.0, 1.0);
                    if unicode {
                        U[((v * 7.0).round() as usize).min(7)]
                    } else {
                        A[((v * 4.0).round() as usize).min(4)]
                    }
                }
            })
            .collect()
    }
    /// Truncate to `w` display columns with an ellipsis.
    pub fn fit(s: &str, w: usize, unicode: bool) -> String {
        let n = s.chars().count();
        if n <= w {
            return s.to_string();
        }
        if w == 0 {
            return String::new();
        }
        let ell = if unicode { "…" } else { "~" };
        let mut out: String = s.chars().take(w - 1).collect();
        out.push_str(ell);
        out
    }
}
