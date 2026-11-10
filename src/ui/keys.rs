//! Key and mouse dispatch for the workspace: focus movement, panel keys and preview mouse.
use super::*;

impl App {
    /// Route a key to the open command panel: Esc closes it, Ctrl+O expands it
    /// full-screen, everything else is delivered to the running command.
    pub(super) fn command_panel_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if key.code == KeyCode::Esc {
            if let Some(panel) = self.command_panel.take() {
                self.send(panel.device, Operation::PaneClose { id: panel.id });
            }
            return;
        }
        if ctrl && key.code == KeyCode::Char('o') {
            if let Some(panel) = self.command_panel.as_ref() {
                if panel.alive {
                    self.pending_panel_expand = Some((panel.device, command_panel_session(panel)));
                }
            }
            return;
        }
        let alive = self.command_panel.as_ref().is_some_and(|p| p.alive);
        if !alive {
            // A finished command only accepts closing or expanding to scroll.
            return;
        }
        if let Some(bytes) = encode_key(key) {
            let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
            let (id, device) = self
                .command_panel
                .as_ref()
                .map(|p| (p.id.clone(), p.device))
                .unwrap();
            self.send(device, Operation::PaneInput { id, hex });
        }
    }
    pub(super) fn key(&mut self, key: KeyEvent) {
        if key.kind == KeyEventKind::Release {
            return;
        }
        if key.code != KeyCode::Enter {
            self.pending_adoption = None;
        }
        if self.pane_focused() {
            self.session_pane_key(key);
            return;
        }
        // A command panel captures input (including Ctrl+C) while it is open.
        if self.command_panel.is_some() {
            self.command_panel_key(key);
            return;
        }
        if self.attach_check.take().is_some() || self.pending_attach.take().is_some() {
            self.pending_attach = None;
            self.set_notice("Attachment cancelled".into());
            if key.code == KeyCode::Esc {
                return;
            }
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.request_quit();
            return;
        }
        self.press = None;
        if self.context_menu.is_some() {
            self.context_menu_key(key);
            return;
        }
        if self.help {
            if let Some(motion) = self.navigation.read(key, 8, false) {
                navigation::Scroll {
                    offset: &mut self.help_scroll,
                    max: 80,
                }
                .apply(motion);
                return;
            }
            if matches!(key.code, KeyCode::Esc | KeyCode::F(1) | KeyCode::Char('?')) {
                self.help = false;
            }
            return;
        }
        if let Some(dialog) = self.dialog.clone().filter(|_| self.input.is_none()) {
            if matches!(key.code, KeyCode::Char('?') | KeyCode::F(1)) {
                self.help = true;
                return;
            }
            self.dialog_key(key, dialog);
            return;
        }
        // Input fields own every printable key, including navigation shortcuts.
        if let Some(mode) = self.input {
            self.input_key(key, mode);
            return;
        }
        // `g` then a view letter goes there; `gg` stays "first row".
        if self.navigation.goto_pending()
            && key.modifiers.difference(KeyModifiers::SHIFT).is_empty()
        {
            if let KeyCode::Char(c) = key.code {
                if let Some(cmd) = keymap::chord(c) {
                    self.navigation.reset();
                    self.run_cmd(cmd);
                    return;
                }
            }
        }
        match keymap::resolve(self, &key) {
            Some(cmd) if cmd != keymap::Cmd::Motion => {
                self.run_cmd(cmd);
                self.navigation.reset();
            }
            _ => {
                if let Some(motion) =
                    self.navigation
                        .read(key, if self.pdf_preview_active() { 1 } else { 10 }, false)
                {
                    panels::active(self).apply(motion);
                }
            }
        }
    }
    /// Run a keymap command. Keys, footer chips, tabs and context-menu rows
    /// all arrive here, so a tap and its key share one implementation.
    pub(super) fn run_cmd(&mut self, cmd: keymap::Cmd) {
        use keymap::Cmd;
        // Any command other than a motion acts on what the user sees as
        // scoped, so a pending Files device switch lands first.
        if cmd != Cmd::Motion && self.view == View::Files && self.device_refresh_due.is_some() {
            self.device_refresh_due = None;
            self.settle_file_device();
        }
        match cmd {
            Cmd::Motion => {}
            Cmd::Goto(view) => self.goto_view(view),
            Cmd::PrevView => self.cycle_view(-1),
            Cmd::NextView => self.cycle_view(1),
            Cmd::NextPanel => self.cycle_focus(false),
            Cmd::PrevPanel => self.cycle_focus(true),
            Cmd::FocusDir(dx, dy) => self.directional_focus(i32::from(dx), i32::from(dy)),
            Cmd::Palette => self.open_palette(""),
            Cmd::RunCommand => self.execute(Action::Command),
            Cmd::Help => {
                self.help = true;
                self.help_scroll = 0;
            }
            Cmd::Quit => self.request_quit(),
            Cmd::Primary => self.primary(),
            Cmd::Back => self.back(),
            Cmd::Search => {
                if self.focus == Focus::Devices {
                    self.begin_list_filter();
                    return;
                }
                self.finish_visual();
                self.input = Some(Input::Search);
                self.focus = Focus::Workspace;
                self.text = if self.view == View::Files {
                    self.browser
                        .as_ref()
                        .map(|b| b.search.clone())
                        .unwrap_or_default()
                } else {
                    self.search.clone()
                };
            }
            Cmd::Filter => self.begin_list_filter(),
            Cmd::New => self.session_shortcut(),
            Cmd::Jobs => self.execute(Action::Jobs),
            Cmd::AddDevice => self.execute(Action::Add),
            Cmd::Watch => self.execute(Action::Observe),
            Cmd::Pin => self.toggle_pin(),
            Cmd::Stop => self.stop_selected_session(),
            Cmd::AgentTasks => {
                if let Some((device, session)) = self.selected_session() {
                    self.dialog =
                        Some(Dialog::AgentTasks(self.devices[device].id.clone(), session));
                    self.dialog_selected = 0;
                    self.dialog_scroll = 0;
                    self.dialog_detail_focus = false;
                }
            }
            Cmd::Refresh => {
                // Explicit: the one place a device refresh clears provider caches.
                self.device_refresh_due = None;
                self.refresh();
            }
            Cmd::ToggleAccess => self.toggle_access(),
            Cmd::RemoveDevice => {
                if self.device > 0 {
                    self.dialog_selected = 0;
                    self.dialog = Some(Dialog::RemoveDevice(self.device - 1));
                }
            }
            Cmd::EditDevice => {
                if self.device > 0 {
                    self.dialog_selected = 0;
                    self.dialog = Some(Dialog::EditDevice(self.device - 1));
                }
            }
            Cmd::Left => {
                if self.view == View::Containers && self.focus == Focus::Workspace {
                    self.container_tree_motion(false);
                } else if self.view == View::Network && self.focus == Focus::Workspace {
                    self.network_tree_motion(false);
                } else if self.view == View::Files && self.focus == Focus::Workspace {
                    self.parent_directory();
                } else {
                    self.focus = Focus::Devices;
                }
            }
            Cmd::Right => {
                if self.view == View::Containers && self.focus == Focus::Workspace {
                    self.container_tree_motion(true);
                } else if self.view == View::Network && self.focus == Focus::Workspace {
                    self.network_tree_motion(true);
                } else if self.view == View::Files && self.focus == Focus::Workspace {
                    if let Some(b) = &self.browser {
                        if let Some(e) = self.visible_entries().get(b.selected).cloned() {
                            if e.kind == "directory" {
                                self.open_browser(b.device, e.path);
                            }
                        }
                    }
                } else {
                    self.focus = Focus::Workspace;
                }
            }
            Cmd::SessionsHere => self.sessions_here(),
            Cmd::NextMatch | Cmd::PrevMatch => {
                let step = if cmd == Cmd::NextMatch { 1 } else { -1 };
                let count = self.navigation.take_count().unwrap_or(1);
                let preview = self.browser.as_ref().is_some_and(|b| b.preview.is_some());
                if !preview {
                    self.finish_visual();
                }
                for _ in 0..count {
                    if preview {
                        self.next_preview_match(step);
                    } else {
                        self.next_file_match(step);
                    }
                }
            }
            Cmd::ClearClipboard => {
                self.clipboard = None;
                self.set_notice("Clipboard cleared · files unchanged".into());
            }
            Cmd::ClearMarks => {
                self.finish_visual();
                if let Some(b) = &mut self.browser {
                    b.marked.clear();
                }
            }
            Cmd::PreviewLinks => self.show_preview_links(),
            Cmd::PreviewSearch => {
                if self.text_preview_active() {
                    self.input = Some(Input::PreviewSearch);
                    self.text = self.browser.as_ref().unwrap().preview_find.query.clone();
                } else {
                    self.set_notice(
                        if self
                            .browser
                            .as_ref()
                            .is_some_and(|b| b.preview_pending_page.is_some())
                        {
                            "Preview is still loading"
                        } else {
                            "Search is available in text previews"
                        }
                        .into(),
                    );
                }
            }
            Cmd::DetailDown => {
                self.network_detail_scroll = self.network_detail_scroll.saturating_add(1).min(256)
            }
            Cmd::DetailUp => {
                self.network_detail_scroll = self.network_detail_scroll.saturating_sub(1)
            }
            Cmd::Do(action) => {
                if self.action_enabled(action) {
                    self.execute(action);
                }
            }
        }
    }
    /// Go to a top-level view without a device-chooser step (P13): the
    /// device rail's scope applies.
    pub(super) fn goto_view(&mut self, view: View) {
        self.finish_visual();
        match view {
            View::Work | View::Containers => {
                if self.view != view {
                    self.view = view;
                    if view == View::Work {
                        self.refresh_work();
                    } else {
                        self.refresh_containers();
                    }
                }
            }
            View::Files if self.view == View::Files => {}
            View::Files => {
                if self.browser.is_some() {
                    self.view = View::Files;
                    self.generation += 1;
                    self.refresh_browser();
                } else if self.view == View::Work && self.selected_session().is_some() {
                    self.execute(Action::Files);
                } else {
                    self.select_file_device();
                }
            }
            View::Network => {
                if self.view != View::Network {
                    self.view = View::Network;
                    self.refresh();
                }
            }
            View::Secrets => self.view = View::Secrets,
        }
        if self.dialog.is_none() {
            self.focus = Focus::Workspace;
        }
    }
    pub(super) fn cycle_view(&mut self, step: isize) {
        let index = keymap::tab_index(self.view) as isize;
        let tabs = keymap::TABS.len() as isize;
        let next = (index + step).rem_euclid(tabs) as usize;
        self.goto_view(keymap::TABS[next].0);
    }
    /// Enter: the primary action of the focused panel.
    pub(super) fn primary(&mut self) {
        if self.focus == Focus::Actions {
            let actions = sidebar_actions(self);
            if let Some((action, _)) = actions.get(self.side_selected) {
                self.execute(*action)
            };
        } else if self.focus == Focus::Devices {
            // Scope is already applied by moving; Enter lands in the view.
            if self.view == View::Files {
                self.select_file_device();
            }
            self.focus = Focus::Workspace;
        } else {
            match self.view {
                View::Secrets => {}
                View::Containers => self.open_container_actions(),
                View::Work => {
                    if let Some((d, s)) = self.selected_session() {
                        if !self.focus_session_pane() {
                            self.open_session(d, s, false);
                        }
                    } else if empty_work_can_create(self) {
                        self.session_shortcut();
                    }
                }
                View::Files => {
                    if self.destination_active
                        && self.clipboard.is_some()
                        && self.browser.as_ref().is_some_and(|b| b.preview.is_none())
                    {
                        if self.browser.as_ref().is_some_and(|b| !b.loading) {
                            self.submit_transfer();
                        }
                    } else if let Some(b) = &self.browser {
                        if let Some(e) = self.visible_entries().get(b.selected).cloned() {
                            if e.kind == "directory" {
                                self.open_browser(b.device, e.path);
                            } else {
                                self.open_preview(b.device, e.path);
                            }
                        }
                    }
                }
                View::Network => self.open_neighbor(),
            }
        }
    }
    /// Esc: back / clear / focus up one level.
    pub(super) fn back(&mut self) {
        if self.view == View::Files && self.focus == Focus::Workspace {
            if let Some(b) = self.browser.as_mut().filter(|b| b.preview.is_some()) {
                if !b.preview_find.query.is_empty() {
                    b.preview_find = PreviewFind::default();
                    return;
                }
                if !b.preview_history.is_empty() {
                    let device = b.device;
                    let (path, scroll, find) = b.preview_history.pop().unwrap();
                    self.open_preview(device, path);
                    self.browser.as_mut().unwrap().preview_restore = Some((scroll, find));
                    return;
                }
            }
        }
        if self.view == View::Files {
            if let Some(b) = &mut self.browser {
                b.preview_rich = None;
                b.preview_find = PreviewFind::default();
                b.preview_pending_page = None;
                b.preview_path = None;
                if b.preview.take().is_none() {
                    if b.visual_anchor.take().is_some() {
                        b.visual_base.clear();
                    } else if !b.marked.is_empty() {
                        b.marked.clear();
                    } else if !b.filter.is_empty() {
                        b.filter.clear();
                        b.selected = 0;
                    } else if !b.search.is_empty() {
                        b.search.clear();
                    } else {
                        self.view = View::Work;
                        self.generation += 1;
                        b.loading = false;
                    }
                } else {
                    self.generation += 1;
                }
            }
        } else if !self.search.is_empty() {
            self.search.clear();
        } else {
            self.focus = Focus::Devices;
        }
    }
    pub(super) fn stop_selected_session(&mut self) {
        let Some((device, session)) = self.selected_session() else {
            return;
        };
        if !self.observed(device, &session) {
            self.set_notice("This pinned runtime has ended; no process was stopped".into());
            return;
        }
        if session.external || !["shell", "codex", "claude"].contains(&session.provider.as_str()) {
            self.set_notice_as(
                NoticeKind::Warning,
                "Only CX-managed sessions can be stopped here".into(),
            );
        } else if self.devices[device].target.is_some()
            && !self.providers.get(&device).is_some_and(|(caps, checked)| {
                transport::now().saturating_sub(*checked) < 60
                    && caps.iter().any(|c| {
                        c == if session.provider == "shell" {
                            "stop-session-v1"
                        } else {
                            "stop-agent-session-v1"
                        }
                    })
            })
        {
            self.check_providers(device);
            self.set_notice("Checking session-stop support · press d again when ready".into());
        } else {
            self.dialog_selected = 0;
            self.dialog_scroll = 0;
            self.dialog_detail_focus = false;
            self.dialog = Some(Dialog::StopShell(device, session));
        }
    }
    /// Run the debounced device-scoped refresh once the selection settled.
    /// Returns true when it ran (the frame changes).
    pub(super) fn settle_device_refresh(&mut self, now: Instant) -> bool {
        if self.device_refresh_due.is_none_or(|due| now < due) {
            return false;
        }
        self.device_refresh_due = None;
        match self.view {
            View::Work => {
                for d in 0..self.devices.len() {
                    if self.background_checks_enabled(d)
                        && (self.device == 0 || self.device == d + 1)
                        && !self.work[d].loading
                    {
                        self.work[d].loading = self.send(d, Operation::Sessions);
                    }
                }
            }
            View::Network => {
                let selection = self.network_selection();
                self.refresh();
                self.restore_network_selection(selection);
            }
            View::Containers => self.refresh_containers(),
            View::Files => self.settle_file_device(),
            View::Secrets => {}
        }
        true
    }
    /// Switch the Files browser to the scoped device without moving focus.
    pub(super) fn settle_file_device(&mut self) {
        let focus = self.focus;
        self.select_file_device();
        self.focus = focus;
    }
    pub(super) fn switch_pane(&mut self) {
        std::mem::swap(&mut self.browser, &mut self.other_browser);
        if let Some(browser) = &self.browser {
            self.device = browser.device + 1;
        }
        self.destination_active = !self.destination_active;
        self.generation += 1;
        self.refresh_browser();
    }
    pub(super) fn directional_focus(&mut self, dx: i32, dy: i32) {
        let panels = self.panels.borrow().clone();
        let Some((_, _, current)) = panels.iter().find(|(focus, dest, _)| {
            *focus == self.focus && (*focus != Focus::Workspace || *dest == self.destination_active)
        }) else {
            return;
        };
        let center = |r: &Rect| {
            (
                r.x as i32 * 2 + r.width as i32,
                r.y as i32 * 2 + r.height as i32,
            )
        };
        let (cx, cy) = center(current);
        let selected = panels
            .iter()
            .filter_map(|(focus, dest, r)| {
                let (x, y) = center(r);
                let primary = (x - cx) * dx + (y - cy) * dy;
                let cross = if dx != 0 {
                    (y - cy).abs()
                } else {
                    (x - cx).abs()
                };
                (primary > 0).then_some((primary + cross * 2, *focus, *dest))
            })
            .min_by_key(|(score, _, _)| *score);
        if let Some((_, focus, dest)) = selected {
            if focus == Focus::Workspace
                && self.other_browser.is_some()
                && dest != self.destination_active
            {
                self.switch_pane();
            }
            self.focus = focus;
        }
    }
    pub(super) fn cycle_focus(&mut self, reverse: bool) {
        if self.view == View::Files && self.other_browser.is_some() {
            if self.focus == Focus::Workspace && self.destination_active == reverse {
                self.switch_pane();
                return;
            }
            if (self.focus == Focus::Actions && !reverse)
                || (self.focus == Focus::Devices && reverse)
            {
                if self.destination_active != reverse {
                    self.switch_pane();
                }
            }
        }
        let step = |focus| match (focus, reverse) {
            (Focus::Workspace, false) | (Focus::Actions, true) => Focus::Devices,
            (Focus::Devices, false) | (Focus::Workspace, true) => Focus::Actions,
            (Focus::Actions, false) | (Focus::Devices, true) => Focus::Workspace,
        };
        // Skip panels the current layout does not draw (the phone layout has
        // no Actions panel). Without a rendered layout every panel counts.
        let drawn: Vec<Focus> = self.panels.borrow().iter().map(|(f, _, _)| *f).collect();
        let mut next = step(self.focus);
        if !drawn.is_empty() && !drawn.contains(&next) {
            next = step(next);
        }
        self.focus = next;
    }
    pub(super) fn mouse(&mut self, mouse: MouseEvent, area: Rect) -> bool {
        if self.scroll_session_preview(mouse) {
            return true;
        }
        if self.pane_focused() {
            return true;
        }
        // Hover never changes state (P16), and pure motion never redraws.
        if mouse.kind == MouseEventKind::Moved {
            return false;
        }
        if self.context_menu.is_some()
            && matches!(
                mouse.kind,
                MouseEventKind::ScrollUp | MouseEventKind::ScrollDown
            )
        {
            return true;
        }
        if matches!(
            mouse.kind,
            MouseEventKind::Down(_) | MouseEventKind::Up(_) | MouseEventKind::Drag(_)
        ) {
            if let Some(handled) = self.pointer(mouse, Instant::now()) {
                return handled;
            }
            if !matches!(mouse.kind, MouseEventKind::Down(_)) {
                return false;
            }
        }
        if matches!(
            mouse.kind,
            MouseEventKind::Down(_) | MouseEventKind::ScrollUp | MouseEventKind::ScrollDown
        ) {
            self.attach_check = None;
            self.pending_attach = None;
        }
        let delta = match mouse.kind {
            MouseEventKind::ScrollDown => 1,
            MouseEventKind::ScrollUp => -1,
            MouseEventKind::Down(event::MouseButton::Left)
                if self.view == View::Files
                    && !self.help
                    && self.dialog.is_none()
                    && self.input.is_none() =>
            {
                // Rows, breadcrumb segments and the inline preview are tap targets.
                if mouse.modifiers == KeyModifiers::NONE && self.file_click(mouse.column, mouse.row)
                {
                    return true;
                }
                let target = self
                    .panels
                    .borrow()
                    .iter()
                    .rev()
                    .find(|(focus, _, rect)| {
                        *focus == Focus::Workspace
                            && rect
                                .contains(ratatui::layout::Position::new(mouse.column, mouse.row))
                    })
                    .map(|(_, destination, _)| *destination);
                let Some(destination) = target else {
                    return false;
                };
                // Click-to-focus on a preview never opens or selects files.
                let browser =
                    if self.other_browser.is_some() && destination != self.destination_active {
                        self.other_browser.as_ref()
                    } else {
                        self.browser.as_ref()
                    };
                if !browser.is_some_and(|b| b.preview.is_some()) {
                    return false;
                }
                if self.other_browser.is_some() && destination != self.destination_active {
                    self.switch_pane();
                }
                self.focus = Focus::Workspace;
                self.preview_mouse(mouse, area);
                return true;
            }
            _ => return self.preview_mouse(mouse, area),
        };
        if mouse.modifiers != KeyModifiers::NONE {
            return false;
        }
        // Wheel motion reviews confirmation details; it cannot select the
        // destructive/exit choice and leave a subsequent Enter armed.
        if matches!(
            self.dialog,
            Some(
                Dialog::Delete(..)
                    | Dialog::StopShell(..)
                    | Dialog::PendingExit(..)
                    | Dialog::ContainerConfirm(..)
                    | Dialog::DevcontainerUp(..)
            )
        ) {
            self.dialog_scroll = if delta > 0 {
                self.dialog_scroll.saturating_add(1).min(4096)
            } else {
                self.dialog_scroll.saturating_sub(1)
            };
            return true;
        }
        // Inline file search/filter keep the browser visible. Wheel motion uses
        // adjacent rows/lines, not the keyboard's next-match navigation. Preserve
        // the query without re-running live_search and snapping to a match.
        let inline_file_input = self.view == View::Files
            && matches!(
                self.input,
                Some(Input::Search | Input::Filter | Input::PreviewSearch)
            );
        // Other editors/modals own navigation; a wheel never acts behind them.
        if self.help || self.dialog.is_some() || (self.input.is_some() && !inline_file_input) {
            self.key(KeyEvent::new(
                if delta > 0 {
                    KeyCode::Down
                } else {
                    KeyCode::Up
                },
                KeyModifiers::NONE,
            ));
            return true;
        }
        if area.width < 36 || area.height < 10 {
            return false;
        }
        let target = self
            .panels
            .borrow()
            .iter()
            .rev()
            .find(|(_, _, rect)| {
                rect.contains(ratatui::layout::Position::new(mouse.column, mouse.row))
            })
            .map(|(focus, destination, _)| (*focus, *destination));
        let Some((focus, destination)) = target else {
            return false;
        };
        if focus == Focus::Workspace
            && self.other_browser.is_some()
            && destination != self.destination_active
        {
            self.switch_pane();
        }
        self.focus = focus;
        if focus == Focus::Workspace && self.preview_mouse(mouse, area) {
            return true;
        }
        self.move_selection(delta);
        true
    }
    pub(super) fn preview_mouse(&mut self, mouse: MouseEvent, area: Rect) -> bool {
        if self.help || self.dialog.is_some() || self.input.is_some() {
            return false;
        }
        if self.markdown_preview_active() {
            if matches!(mouse.kind, MouseEventKind::Down(event::MouseButton::Left)) {
                let tag = self.browser.as_ref().and_then(|b| {
                    b.preview_link_cells
                        .borrow()
                        .iter()
                        .find(|(x, y, _)| *x == mouse.column && *y == mouse.row)
                        .map(|(_, _, tag)| *tag)
                });
                if let Some(tag) = tag {
                    let links = self
                        .browser
                        .as_ref()
                        .and_then(|b| b.preview.as_ref())
                        .map(|s| markdown_preview_links(s))
                        .unwrap_or_default();
                    let matches: Vec<_> = links
                        .iter()
                        .enumerate()
                        .filter(|(_, l)| markdown_link_tag(&l.target) == tag)
                        .collect();
                    if matches.is_empty() {
                        return false;
                    }
                    let target = matches.first().map(|(_, l)| l.target.as_str());
                    let unique = target.is_some()
                        && matches
                            .iter()
                            .all(|(_, l)| Some(l.target.as_str()) == target);
                    if unique
                        && mouse
                            .modifiers
                            .intersects(KeyModifiers::SHIFT | KeyModifiers::CONTROL)
                    {
                        let link = matches[0].1.clone();
                        self.open_markdown_link(&link);
                    } else {
                        self.show_preview_links();
                        if unique {
                            self.dialog_selected = matches[0].0;
                        }
                    }
                    return true;
                }
            }
            return false;
        }
        if !self.pdf_preview_active() {
            return false;
        }
        if !native_preview_area(self, area)
            .is_some_and(|r| r.contains(ratatui::layout::Position::new(mouse.column, mouse.row)))
        {
            return false;
        }
        match mouse.kind {
            MouseEventKind::ScrollDown => self.move_selection(1),
            MouseEventKind::ScrollUp => self.move_selection(-1),
            _ => return false,
        }
        true
    }
    pub(super) fn move_selection(&mut self, delta: isize) {
        panels::active(self).move_by(delta);
    }
}
