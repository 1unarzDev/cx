//! Production view layer: the Mission design (docs/design/concept-mission.md) drawn
//! from a pure view-model (`LabState`, filled by `ui::viewmodel` from the App, or by the
//! lab fixtures in tests) into a ratatui buffer. Nothing here touches App, IO, tmux or
//! the network; rendering never dispatches requests. The test-only lab harness
//! (`ui::lab`) captures the same code from fixtures.

pub mod caps;
pub mod mission;
pub mod state;

pub use caps::{ColorLevel, LabCaps, Motion, Pane, Theme, Tok};
pub use state::*;

use ratatui::{layout::Rect, text::Line};

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
    /// Command palette / jump-to-anything with the typed query (lab captures; the app
    /// draws the palette as an overlay on any view).
    #[cfg(test)]
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

/// A tap/click target a concept exposes (dumped for the touch-target overlay).
#[derive(Clone, Debug, PartialEq)]
pub struct LabTarget {
    pub area: Rect,
    pub action: String,
}

/// Parse a SGR-colored screen snapshot (as in `LabSession::screen`) into lines, padding
/// each to `cols` with the running style, like the real preview does.
pub fn ansi_lines(screen: &[String], cols: u16) -> Vec<Line<'static>> {
    super::rendering::ansi_screen(screen, None, cols)
}

/// `LabDevice::observed_secs` of a device never seen (no sighting to age).
pub const NEVER_SEEN: u32 = u32::MAX;

/// Shared tiny helpers for concepts.
pub mod fmt {
    pub fn age(secs: u32) -> String {
        match secs {
            super::NEVER_SEEN => "never".into(),
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
}
