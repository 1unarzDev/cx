//! Scoped list filters retain the original device/job identities and text-input ownership.
use super::{identity, App, Dialog, Focus, Input, View};
use score as fuzzy_score;
impl App {
    pub(super) fn device_rows(&self) -> Vec<usize> {
        std::iter::once(0)
            .chain(
                (0..self.devices.len())
                    .filter(|d| self.device_matches(*d))
                    .map(|d| d + 1),
            )
            .collect()
    }
    pub(super) fn device_matches(&self, d: usize) -> bool {
        let device = &self.devices[d];
        fuzzy_score(
            &self.device_filter,
            &format!("{} {} {}", device.name, identity(device), device.host),
        )
        .is_some()
    }
    pub(super) fn begin_list_filter(&mut self) {
        self.input = Some(Input::Filter);
        self.text = if matches!(self.dialog, Some(Dialog::Jobs)) {
            self.job_filter.clone()
        } else if self.focus == Focus::Devices || matches!(self.dialog, Some(Dialog::Device(_))) {
            self.device_filter.clone()
        } else {
            self.search.clone()
        };
    }
    pub(super) fn update_list_filter(&mut self) {
        if matches!(self.dialog, Some(Dialog::Jobs)) {
            self.job_filter = self.text.clone();
            self.dialog_selected = 0;
            self.dialog_scroll = 0;
        } else if self.focus == Focus::Devices || matches!(self.dialog, Some(Dialog::Device(_))) {
            self.device_filter = self.text.clone();
            self.dialog_selected = 0;
            if !self.device_rows().contains(&self.device) {
                self.device = 0;
            }
        } else if self.view == View::Files {
            if let Some(b) = &mut self.browser {
                b.filter = self.text.clone();
                b.selected = 0;
                b.restore_selection = None;
            }
        } else {
            self.search = self.text.clone();
            self.selected = 0;
            self.network_selected = 0;
            self.container_selected = 0;
        }
    }
}
/// Subsequence matcher with word-start and contiguous bonuses; empty queries retain order.
pub(super) fn score(query: &str, text: &str) -> Option<i64> {
    let hay: Vec<char> = text.to_lowercase().chars().collect();
    let needle: Vec<char> = query
        .to_lowercase()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    if needle.is_empty() {
        return Some(0);
    }
    let mut at = 0;
    let mut prior = None;
    let mut score = 0;
    for c in needle {
        let pos = (at..hay.len()).find(|&i| hay[i] == c)?;
        score += 10;
        if pos == 0 || !hay[pos - 1].is_alphanumeric() {
            score += 12;
        }
        if prior == Some(pos.saturating_sub(1)) {
            score += 16;
        }
        score -= if let Some(p) = prior {
            (pos - p - 1) as i64
        } else {
            pos.min(30) as i64
        };
        prior = Some(pos);
        at = pos + 1;
    }
    Some(score)
}

/// The footer uses the same scope as filtering, including file preview/search ownership.
pub(super) fn visible_query(app: &App) -> &str {
    if matches!(app.dialog, Some(Dialog::Jobs)) {
        app.job_filter.as_str()
    } else if app.focus == Focus::Devices || matches!(app.dialog, Some(Dialog::Device(_))) {
        app.device_filter.as_str()
    } else if app.browser.as_ref().is_some_and(|b| b.preview.is_some()) && app.view == View::Files {
        app.browser.as_ref().unwrap().preview_find.query.as_str()
    } else if app.input == Some(Input::Filter) && app.view == View::Files {
        app.browser
            .as_ref()
            .map(|b| b.filter.as_str())
            .unwrap_or("")
    } else if app.view == View::Files {
        app.browser
            .as_ref()
            .map(|b| b.search.as_str())
            .unwrap_or("")
    } else {
        app.search.as_str()
    }
}
