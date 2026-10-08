//! Bounded keyboard motions shared by panels. Actions and text inputs stay with callers.
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Clone, Copy)]
pub(super) enum Motion {
    Pending,
    Move(isize),
    Row(usize),
}

#[derive(Default)]
pub(super) struct State {
    count: Option<usize>,
    pending_g: bool,
}
impl State {
    pub(super) fn reset(&mut self) {
        self.count = None;
        self.pending_g = false;
    }
    pub(super) fn take_count(&mut self) -> Option<usize> {
        self.pending_g = false;
        self.count.take()
    }
    #[cfg(test)]
    pub(super) fn is_idle(&self) -> bool {
        self.count.is_none() && !self.pending_g
    }
    pub(super) fn read(&mut self, key: KeyEvent, page: usize, horizontal: bool) -> Option<Motion> {
        if key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            self.count = None;
            self.pending_g = false;
            return None;
        }
        if let KeyCode::Char(c @ '0'..='9') = key.code {
            if c != '0' || self.count.is_some() {
                self.count = Some(
                    self.count
                        .unwrap_or(0)
                        .saturating_mul(10)
                        .saturating_add((c as u8 - b'0') as usize)
                        .min(10000),
                );
                self.pending_g = false;
                return Some(Motion::Pending);
            }
        }
        if key.code == KeyCode::Char('g') {
            if self.pending_g {
                self.pending_g = false;
                return Some(Motion::Row(
                    self.count.take().unwrap_or(1).saturating_sub(1),
                ));
            }
            self.pending_g = true;
            return Some(Motion::Pending);
        }
        self.pending_g = false;
        let count = self.count.take();
        let delta = match key.code {
            KeyCode::Char('G') => {
                return Some(
                    count.map_or(Motion::Move(100_000), |n| Motion::Row(n.saturating_sub(1))),
                )
            }
            KeyCode::Home => return Some(Motion::Row(0)),
            KeyCode::End => return Some(Motion::Move(100_000)),
            KeyCode::Up | KeyCode::Char('k') => -1,
            KeyCode::Down | KeyCode::Char('j') => 1,
            KeyCode::Left | KeyCode::Char('h') if horizontal => -1,
            KeyCode::Right | KeyCode::Char('l') if horizontal => 1,
            KeyCode::PageUp => -(page as isize),
            KeyCode::PageDown => page as isize,
            _ => return None,
        };
        Some(Motion::Move(delta * count.unwrap_or(1) as isize))
    }
}

/// Adapters own selection effects. The default row jump preserves existing panel refreshes.
pub(super) trait Target {
    fn move_by(&mut self, delta: isize);
    fn apply(&mut self, motion: Motion) {
        match motion {
            Motion::Pending => (),
            Motion::Move(delta) => self.move_by(delta),
            Motion::Row(row) => {
                self.move_by(-100_000);
                self.move_by(row as isize);
            }
        }
    }
}

pub(super) struct List<'a> {
    pub selected: &'a mut usize,
    pub length: usize,
}
impl Target for List<'_> {
    fn apply(&mut self, motion: Motion) {
        match motion {
            Motion::Pending => (),
            Motion::Move(delta) => self.move_by(delta),
            Motion::Row(row) => *self.selected = row.min(self.length.saturating_sub(1)),
        }
    }
    fn move_by(&mut self, delta: isize) {
        *self.selected = self
            .selected
            .saturating_add_signed(delta)
            .min(self.length.saturating_sub(1));
    }
}

pub(super) struct Scroll<'a> {
    pub offset: &'a mut u16,
    pub max: u16,
}
impl Target for Scroll<'_> {
    fn move_by(&mut self, delta: isize) {
        *self.offset =
            ((*self.offset).min(self.max) as isize + delta).clamp(0, self.max as isize) as u16;
    }
    fn apply(&mut self, motion: Motion) {
        match motion {
            Motion::Pending => (),
            Motion::Move(delta) => self.move_by(delta),
            Motion::Row(row) => *self.offset = row.min(usize::from(self.max)) as u16,
        }
    }
}
