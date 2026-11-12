//! Per-viewer session activity history (from listings, in memory), the trend a
//! session's peek shows.
use super::*;

/// Per-viewer activity history for the sparkline.
#[derive(Default)]
pub(super) struct State {
    history: HashMap<(String, String), VecDeque<(u64, bool)>>,
}

const HISTORY_SECONDS: u64 = 60;
const HISTORY_KEYS: usize = 512;

impl State {
    /// Record one activity sample per observed session from a fresh listing.
    /// A session is active when it produced output since its previous sample
    /// (refreshes are not evenly spaced: RTT, busy devices and other views
    /// stretch the gap) or reports native work; unknown evidence records nothing.
    pub(super) fn record(&mut self, device: &Device, sessions: &[Session], now: u64) {
        let history = &mut self.history;
        for s in sessions {
            let samples = history
                .entry((device.id.clone(), workspaces::runtime_key(s)))
                .or_default();
            if samples.back().is_some_and(|(t, _)| *t == now) {
                samples.pop_back();
            }
            // No earlier sample: one bucket width, the span the bar covers.
            let gap = samples.back().map_or(15, |(t, _)| now.saturating_sub(*t));
            let active = match (s.idle_seconds, session_busy(s)) {
                (_, true) => true,
                (Some(idle), false) => idle <= gap.max(3),
                (None, false) => continue,
            };
            samples.push_back((now, active));
            while samples
                .front()
                .is_some_and(|(t, _)| now.saturating_sub(*t) > HISTORY_SECONDS)
            {
                samples.pop_front();
            }
        }
        history.retain(|_, samples| {
            samples
                .back()
                .is_some_and(|(t, _)| now.saturating_sub(*t) <= 2 * HISTORY_SECONDS)
        });
        if history.len() > HISTORY_KEYS {
            let mut oldest: Vec<_> = history
                .iter()
                .map(|(k, v)| (v.back().map(|(t, _)| *t).unwrap_or(0), k.clone()))
                .collect();
            oldest.sort();
            for (_, key) in oldest.into_iter().take(history.len() - HISTORY_KEYS) {
                history.remove(&key);
            }
        }
    }
    /// Activity of the last 60 s in four 15 s buckets (0..1), oldest first; empty
    /// without evidence (the view layer draws a gap for none).
    pub(super) fn levels(&self, device: &Device, s: &Session, now: u64) -> Vec<f32> {
        let Some(samples) = self
            .history
            .get(&(device.id.clone(), workspaces::runtime_key(s)))
        else {
            return Vec::new();
        };
        (0..4u64)
            .filter_map(|bucket| {
                let (start, end) = (
                    now.saturating_sub(HISTORY_SECONDS - bucket * 15),
                    now.saturating_sub(HISTORY_SECONDS - (bucket + 1) * 15),
                );
                let in_bucket: Vec<bool> = samples
                    .iter()
                    .filter(|(t, _)| *t > start && *t <= end.max(start + 1))
                    .map(|(_, a)| *a)
                    .collect();
                (!in_bucket.is_empty()).then(|| {
                    in_bucket.iter().filter(|a| **a).count() as f32 / in_bucket.len() as f32
                })
            })
            .collect()
    }
}
