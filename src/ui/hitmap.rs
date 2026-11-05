//! Pointer targets. During render every clickable region (tab, row, chip,
//! scrollbar, palette/menu row) registers `Rect -> Hit`; mouse down/up resolve
//! through the map instead of re-deriving layout math. Taps and keys share
//! `keymap::Cmd`, so a target always does what its key does.
use super::keymap::{self, Cmd};
use super::navigation::Target;
use super::{
    accent, ascii, muted, navigation, selected_style, App, Block, BorderType, Borders, Clear,
    Focus, Frame, Input, KeyCode, KeyEvent, Line, Modifier, MouseEvent, MouseEventKind, Paragraph,
    Rect, Span, Style, View,
};
use crossterm::event::MouseButton;
use std::cell::RefCell;
use std::time::{Duration, Instant};

/// Holding a press this long (without moving off the target) opens the
/// context menu, for touch terminals that have no right button.
pub(super) const LONG_PRESS: Duration = Duration::from_millis(450);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum ListId {
    Sessions,
    Devices,
    Network,
    Files,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Hit {
    Tab(View),
    /// Footer chip, header button or any other command target.
    Cmd(Cmd),
    /// A list row: its list and the row's selection index.
    Row(ListId, usize),
    /// A list scrollbar track: click or drag jumps proportionally.
    Scrollbar(ListId, usize),
    PaletteRow(usize),
    MenuRow(usize),
}

#[derive(Default)]
pub(super) struct HitMap {
    hits: Vec<(Rect, Hit)>,
}
impl HitMap {
    /// The topmost (last registered) target under the point.
    pub(super) fn at(&self, x: u16, y: u16) -> Option<(Rect, Hit)> {
        self.hits
            .iter()
            .rev()
            .find(|(r, _)| r.contains(ratatui::layout::Position::new(x, y)))
            .copied()
    }
    #[cfg(test)]
    pub(super) fn all(&self) -> Vec<(Rect, Hit)> {
        self.hits.clone()
    }
}

thread_local! {
    static MAP: RefCell<HitMap> = RefCell::new(HitMap::default());
}
/// Start a frame: forget the previous frame's targets.
pub(super) fn begin() {
    MAP.with(|m| m.borrow_mut().hits.clear());
}
pub(super) fn register(rect: Rect, hit: Hit) {
    if rect.width > 0 && rect.height > 0 {
        MAP.with(|m| m.borrow_mut().hits.push((rect, hit)));
    }
}
/// An overlay owns the pointer: nothing drawn beneath it stays clickable.
pub(super) fn modal() {
    begin();
}
pub(super) fn with<T>(f: impl FnOnce(&HitMap) -> T) -> T {
    MAP.with(|m| f(&m.borrow()))
}
/// Register one row target per visible list row.
pub(super) fn rows(list: ListId, area: Rect, offset: usize, len: usize) {
    for (line, index) in (offset..len).enumerate() {
        let y = area.y.saturating_add(line as u16);
        if y >= area.bottom() {
            break;
        }
        register(Rect::new(area.x, y, area.width, 1), Hit::Row(list, index));
    }
}
/// Draw a one-column scrollbar on the right border when the list overflows,
/// and register it as a drag target.
pub(super) fn scrollbar(
    frame: &mut Frame<'_>,
    list: ListId,
    track: Rect,
    offset: usize,
    len: usize,
    height: usize,
    targets: usize,
) {
    if len <= height || track.height < 2 || height == 0 {
        return;
    }
    let mut state = ratatui::widgets::ScrollbarState::new(len.saturating_sub(height))
        .position(offset)
        .viewport_content_length(height);
    let bar =
        ratatui::widgets::Scrollbar::new(ratatui::widgets::ScrollbarOrientation::VerticalRight)
            .begin_symbol(None)
            .end_symbol(None)
            .track_symbol(Some(if ascii() { "|" } else { "│" }))
            .thumb_symbol(if ascii() { "#" } else { "┃" })
            .track_style(muted())
            .thumb_style(accent());
    frame.render_stateful_widget(bar, track, &mut state);
    let column = Rect::new(track.right().saturating_sub(1), track.y, 1, track.height);
    register(column, Hit::Scrollbar(list, targets));
}

/// A pointer press in progress.
#[derive(Clone, Copy)]
pub(super) struct Press {
    hit: Hit,
    rect: Rect,
    at: Instant,
    /// The row was already selected before this press (a tap then means Enter).
    was_selected: bool,
    /// Long-press already opened the menu; the release does nothing.
    fired: bool,
}

pub(super) struct ContextMenu {
    pub items: Vec<(&'static str, &'static str, Cmd)>,
    pub selected: usize,
    pub anchor: (u16, u16),
}

impl App {
    fn row_selected(&self, list: ListId, index: usize) -> bool {
        match list {
            ListId::Sessions => self.focus == Focus::Workspace && self.selected == index,
            ListId::Devices => self.focus == Focus::Devices && self.device == index,
            ListId::Network => self.focus == Focus::Workspace && self.network_selected == index,
            ListId::Files => {
                self.focus == Focus::Workspace
                    && self.browser.as_ref().is_some_and(|b| b.selected == index)
            }
        }
    }
    /// Select a row exactly as keyboard motion would (same panel adapters).
    fn select_row(&mut self, list: ListId, index: usize) {
        match list {
            ListId::Sessions => {
                if self.selected != index {
                    self.attach_check = None;
                    self.pending_attach = None;
                    self.pending_adoption = None;
                }
                self.focus = Focus::Workspace;
                let current = self.selected as isize;
                super::panels::active(self).move_by(index as isize - current);
            }
            ListId::Devices => {
                self.focus = Focus::Devices;
                let rows = self.device_rows();
                let current = rows.iter().position(|d| *d == self.device).unwrap_or(0) as isize;
                if let Some(target) = rows.iter().position(|d| *d == index) {
                    super::panels::active(self).move_by(target as isize - current);
                }
            }
            ListId::Network => {
                self.focus = Focus::Workspace;
                let current = self.network_selected as isize;
                super::panels::active(self).move_by(index as isize - current);
            }
            ListId::Files => {
                self.focus = Focus::Workspace;
                let current = self.browser.as_ref().map_or(0, |b| b.selected) as isize;
                super::panels::active(self).move_by(index as isize - current);
            }
        }
    }
    fn scroll_to(&mut self, list: ListId, len: usize, rect: Rect, y: u16) {
        let span = rect.height.saturating_sub(1).max(1) as usize;
        let at = (y.saturating_sub(rect.y) as usize).min(span);
        let index = (at * len.saturating_sub(1) + span / 2) / span;
        let index = match list {
            ListId::Devices => {
                let rows = self.device_rows();
                rows.get(index.min(rows.len().saturating_sub(1)))
                    .copied()
                    .unwrap_or(0)
            }
            _ => index,
        };
        self.select_row(list, index);
    }
    fn open_context_menu(&mut self, anchor: (u16, u16)) {
        let items = keymap::context_menu(self);
        if !items.is_empty() {
            self.context_menu = Some(ContextMenu {
                items,
                selected: 0,
                anchor,
            });
        }
    }
    /// Resolve a button event through the hit map. `None` means no target
    /// claimed it, so the caller keeps its existing routing (wheel, preview
    /// focus, markdown links).
    pub(super) fn pointer(&mut self, mouse: MouseEvent, now: Instant) -> Option<bool> {
        let (x, y) = (mouse.column, mouse.row);
        let hit = with(|m| m.at(x, y));
        // An open context menu owns the pointer.
        if self.context_menu.is_some() {
            if matches!(mouse.kind, MouseEventKind::Down(_)) {
                match hit {
                    Some((_, Hit::MenuRow(i))) => {
                        let cmd = self
                            .context_menu
                            .as_ref()
                            .and_then(|m| m.items.get(i))
                            .map(|i| i.2);
                        self.context_menu = None;
                        if let Some(cmd) = cmd {
                            self.run_cmd(cmd);
                        }
                    }
                    _ => self.context_menu = None,
                }
            }
            return Some(true);
        }
        if self.help || self.dialog.is_some() {
            return None;
        }
        if self.input == Some(Input::Palette) {
            if let MouseEventKind::Down(MouseButton::Left) = mouse.kind {
                match hit {
                    Some((_, Hit::PaletteRow(i))) => {
                        if self.palette_selected == i {
                            self.run_palette_selected();
                        } else {
                            self.palette_selected = i;
                        }
                    }
                    // Tapping outside the palette closes it.
                    _ => {
                        self.input = None;
                        self.text.clear();
                    }
                }
            }
            return Some(true);
        }
        if self.input.is_some() {
            return None;
        }
        match mouse.kind {
            MouseEventKind::Down(button @ (MouseButton::Left | MouseButton::Right)) => {
                let (rect, hit) = hit?;
                if let Hit::Row(list, index) = hit {
                    let was_selected = self.row_selected(list, index);
                    self.select_row(list, index);
                    if button == MouseButton::Right {
                        self.press = None;
                        self.open_context_menu((x, y));
                        return Some(true);
                    }
                    self.press = Some(Press {
                        hit,
                        rect,
                        at: now,
                        was_selected,
                        fired: false,
                    });
                    return Some(true);
                }
                if button == MouseButton::Right {
                    return Some(false);
                }
                if let Hit::Scrollbar(list, len) = hit {
                    self.scroll_to(list, len, rect, y);
                }
                self.press = Some(Press {
                    hit,
                    rect,
                    at: now,
                    was_selected: false,
                    fired: false,
                });
                Some(true)
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                let press = self.press?;
                if let Hit::Scrollbar(list, len) = press.hit {
                    self.scroll_to(list, len, press.rect, y);
                    return Some(true);
                }
                if !press.rect.contains(ratatui::layout::Position::new(x, y)) {
                    // Sliding off a target cancels the tap and the long press.
                    self.press = None;
                }
                Some(false)
            }
            MouseEventKind::Up(MouseButton::Left) => {
                let press = self.press.take()?;
                if press.fired || !press.rect.contains(ratatui::layout::Position::new(x, y)) {
                    return Some(false);
                }
                match press.hit {
                    Hit::Row(..) if press.was_selected => self.run_cmd(Cmd::Primary),
                    Hit::Tab(view) => self.goto_view(view),
                    Hit::Cmd(cmd) => self.run_cmd(cmd),
                    _ => return Some(false),
                }
                Some(true)
            }
            _ => None,
        }
    }
    /// When the pending press becomes a long press. The event loop waits no
    /// longer than this so the menu opens while the finger is still down.
    pub(super) fn press_deadline(&self) -> Option<Instant> {
        self.press
            .filter(|p| !p.fired && matches!(p.hit, Hit::Row(..)))
            .map(|p| p.at + LONG_PRESS)
    }
    /// Fire a due long press. Returns true when the screen changed.
    pub(super) fn tick_press(&mut self, now: Instant) -> bool {
        let Some(deadline) = self.press_deadline() else {
            return false;
        };
        if now < deadline {
            return false;
        }
        let press = self.press.as_mut().unwrap();
        press.fired = true;
        let anchor = (press.rect.x.saturating_add(2), press.rect.y);
        self.open_context_menu(anchor);
        true
    }
    pub(super) fn context_menu_key(&mut self, key: KeyEvent) {
        let Some(menu) = self.context_menu.as_mut() else {
            return;
        };
        let len = menu.items.len();
        match key.code {
            KeyCode::Esc => self.context_menu = None,
            KeyCode::Enter => {
                let cmd = menu.items.get(menu.selected).map(|i| i.2);
                self.context_menu = None;
                if let Some(cmd) = cmd {
                    self.run_cmd(cmd);
                }
            }
            _ => {
                if let Some(motion) = self.navigation.read(key, 5, false) {
                    let menu = self.context_menu.as_mut().unwrap();
                    navigation::List {
                        selected: &mut menu.selected,
                        length: len,
                    }
                    .apply(motion);
                }
            }
        }
    }
}

/// Draw the context menu as one box next to its anchor; it owns the pointer.
pub(super) fn render_context_menu(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let Some(menu) = &app.context_menu else {
        return;
    };
    modal();
    let label_width = menu
        .items
        .iter()
        .map(|(key, help, _)| key.chars().count() + help.chars().count() + 5)
        .max()
        .unwrap_or(10);
    let width = (label_width as u16 + 2)
        .min(area.width.saturating_sub(2))
        .max(12);
    let height = (menu.items.len() as u16 + 2).min(area.height.saturating_sub(2));
    let x = menu.anchor.0.min(area.right().saturating_sub(width));
    let y = if menu.anchor.1 + height < area.bottom() {
        menu.anchor.1 + 1
    } else {
        menu.anchor.1.saturating_sub(height)
    };
    let rect = Rect::new(x, y, width, height);
    frame.render_widget(Clear, rect);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(if ascii() {
            BorderType::Plain
        } else {
            BorderType::Rounded
        })
        .border_style(accent())
        .title(Span::styled(
            " Actions ",
            accent().add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    let key_width = menu
        .items
        .iter()
        .map(|(key, _, _)| key.chars().count())
        .max()
        .unwrap_or(1);
    let lines: Vec<Line> = menu
        .items
        .iter()
        .enumerate()
        .take(inner.height as usize)
        .map(|(i, (key, help, _))| {
            let style = if i == menu.selected {
                selected_style()
            } else {
                Style::default()
            };
            Line::from(vec![
                Span::styled(
                    if i == menu.selected {
                        if ascii() {
                            "> "
                        } else {
                            "▌ "
                        }
                    } else {
                        "  "
                    },
                    accent(),
                ),
                Span::styled(
                    {
                        let width = (inner.width as usize).saturating_sub(key_width + 3);
                        format!("{:<width$}", super::fit_label(help, width))
                    },
                    style,
                ),
                Span::raw(" "),
                Span::styled(format!("{key:>key_width$}"), muted()),
            ])
        })
        .collect();
    for i in 0..lines.len() {
        register(
            Rect::new(inner.x, inner.y + i as u16, inner.width, 1),
            Hit::MenuRow(i),
        );
    }
    frame.render_widget(Paragraph::new(lines), inner);
}
