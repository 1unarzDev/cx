//! The hub map's animation clock. The event loop calls `track` once per pass with the
//! selection the next frame draws; a change starts one short transition (the
//! highlight glides, clusters bloom and fold, nodes fade) that the view draws part-way
//! from `motion`. The state itself changes at once: a key never waits for an
//! animation, and a key during one simply starts the next from where the eye is.
//! Nothing ticks when nothing moves: `animating` is false once settled, so an idle hub
//! draws no frame at all. No thread: the loop's own frame pacing (at most ~60 fps)
//! drives it.
//!
//! Motion is off with `CX_REDUCED_MOTION=1`, on phones, and where the terminal is
//! reached over a slow link (its startup query answered after more than 80 ms).
use super::views::Motion;
use std::time::{Duration, Instant};

/// How long a transition takes.
pub(super) const GLIDE: Duration = Duration::from_millis(160);
/// How long the attach expand and the detach shrink take.
pub(super) const EXPAND: Duration = Duration::from_millis(120);

/// A session's tile growing to the full screen before its attach (`grow`), or the
/// full screen shrinking back into its tile after a detach. Drawn by the view layer
/// (`views::mission::render_expand`) over the frame; a key skips it to its end.
#[derive(Clone, Debug)]
pub(super) struct Expand {
    pub d: usize,
    /// The session's runtime key (its view-model id).
    pub key: String,
    /// The tile it grows from; a shrink finds its tile on the frame it draws.
    pub from: Option<ratatui::layout::Rect>,
    pub grow: bool,
    pub start: Instant,
}

impl Expand {
    /// Progress 0..1 while it plays, None once done.
    pub(super) fn progress(&self, now: Instant) -> Option<f32> {
        let t = now.saturating_duration_since(self.start).as_secs_f32() / EXPAND.as_secs_f32();
        (t < 1.0).then_some(t)
    }
    /// Far enough that the eased rect is the full screen to the eye (99.7%): an attach
    /// waiting on it hands off here rather than a frame later.
    pub(super) fn landed(&self, now: Instant) -> bool {
        self.progress(now).is_none_or(|t| t >= 0.85)
    }
}
/// Above this terminal round trip, frames are better spent elsewhere.
const SLOW_LINK_MS: u32 = 80;

pub(super) struct Clock {
    /// The selection the last tracked frame drew (None: off the map).
    shown: Option<String>,
    from: Option<String>,
    start: Option<Instant>,
    reduced: bool,
}

/// Whether this terminal gets reduced motion (see the module comment).
pub(super) fn reduced(env: Option<&str>, phone: bool, rtt_ms: Option<u32>) -> bool {
    env.is_some_and(|v| !matches!(v.trim(), "" | "0"))
        || phone
        || rtt_ms.is_some_and(|ms| ms > SLOW_LINK_MS)
}

impl Default for Clock {
    fn default() -> Self {
        Self::new(false)
    }
}

impl Clock {
    pub(super) fn new(reduced: bool) -> Self {
        Self {
            shown: None,
            from: None,
            start: None,
            reduced,
        }
    }

    /// Motion is off for this terminal (the attach expand follows it too).
    pub(super) fn reduced(&self) -> bool {
        self.reduced
    }

    /// The terminal's own setting (environment, phone, measured link).
    pub(super) fn for_terminal() -> Self {
        let c = super::caps::caps();
        Self::new(reduced(
            std::env::var("CX_REDUCED_MOTION").ok().as_deref(),
            c.phone,
            super::caps::terminal_rtt_ms(),
        ))
    }

    /// The selection the next frame draws on the map (None off the map). A change
    /// starts a transition from the old one, unless motion is reduced or this is the
    /// map's first frame.
    pub(super) fn track(&mut self, sel: Option<&str>, now: Instant) {
        if self.shown.as_deref() == sel {
            return;
        }
        let was = std::mem::replace(&mut self.shown, sel.map(str::to_string));
        match (was, sel) {
            (Some(old), Some(_)) if !self.reduced => {
                self.from = Some(old);
                self.start = Some(now);
            }
            _ => {
                self.from = None;
                self.start = None;
            }
        }
    }

    /// Progress 0..=1 of the transition in flight, None when settled.
    fn progress(&self, now: Instant) -> Option<f32> {
        let start = self.start?;
        let t = now.saturating_duration_since(start).as_secs_f32() / GLIDE.as_secs_f32();
        (t < 1.0).then_some(t.max(0.0))
    }

    /// True while a transition is in flight (the loop keeps drawing until it settles).
    pub(super) fn animating(&self, now: Instant) -> bool {
        self.progress(now).is_some()
    }

    /// How long the loop may sleep before the next frame is due (None: settled).
    pub(super) fn wait(&self, now: Instant) -> Option<Duration> {
        self.animating(now)
            .then(|| super::runtime::FRAME_INTERVAL.min(GLIDE))
    }

    /// What the view draws now; `index` maps a device id to its view-model index.
    pub(super) fn motion(&self, now: Instant, index: impl Fn(&str) -> Option<usize>) -> Motion {
        match (self.progress(now), self.from.as_deref()) {
            (Some(t), Some(from)) => Motion {
                from: index(from).and_then(|i| u16::try_from(i).ok()),
                t: (t * 1000.0) as u16,
            },
            _ => Motion::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_change_animates_briefly_then_settles_to_nothing() {
        let t0 = Instant::now();
        let mut c = Clock::new(false);
        c.track(Some("0"), t0);
        assert!(!c.animating(t0), "the first frame does not animate");
        c.track(Some("1"), t0);
        assert!(c.animating(t0));
        let m = c.motion(t0 + GLIDE / 2, |id| id.parse().ok());
        assert_eq!(m.from, Some(0));
        assert!((400..=600).contains(&m.t), "{m:?}");
        let after = t0 + GLIDE + Duration::from_millis(1);
        assert!(!c.animating(after));
        assert_eq!(c.motion(after, |id| id.parse().ok()), Motion::default());
        assert_eq!(c.wait(after), None);
        // Same selection again: nothing starts.
        c.track(Some("1"), after);
        assert!(!c.animating(after));
    }

    #[test]
    fn reduced_motion_never_animates() {
        let t0 = Instant::now();
        let mut c = Clock::new(true);
        c.track(Some("0"), t0);
        c.track(Some("1"), t0);
        assert!(!c.animating(t0));
        assert!(reduced(Some("1"), false, None));
        assert!(!reduced(Some("0"), false, Some(20)));
        assert!(reduced(None, true, None));
        assert!(reduced(None, false, Some(120)));
        assert!(!reduced(None, false, Some(30)));
    }

    #[test]
    fn an_expand_plays_once_then_is_done() {
        let t0 = Instant::now();
        let e = Expand {
            d: 0,
            key: "k".into(),
            from: None,
            grow: true,
            start: t0,
        };
        assert_eq!(e.progress(t0), Some(0.0));
        let mid = e.progress(t0 + EXPAND / 2).unwrap();
        assert!((0.45..0.55).contains(&mid), "{mid}");
        assert_eq!(e.progress(t0 + EXPAND), None);
        assert!(!e.landed(t0 + EXPAND / 2) && e.landed(t0 + EXPAND * 9 / 10));
    }

    #[test]
    fn leaving_the_map_drops_the_transition() {
        let t0 = Instant::now();
        let mut c = Clock::new(false);
        c.track(Some("0"), t0);
        c.track(Some("1"), t0);
        c.track(None, t0);
        assert!(!c.animating(t0));
        c.track(Some("2"), t0);
        assert!(!c.animating(t0), "back on the map: no glide from before");
    }
}
