//! Text-input fields (filter, search, rename, commands, device edits): they own every printable key.
use super::*;

impl App {
    pub(super) fn input_key(&mut self, key: KeyEvent, mode: Input) {
        self.navigation.reset();
        let before_text = self.text.clone();
        match key.code {
            KeyCode::Esc => {
                // Cancelling a filter edit reverts the live filter to what it
                // was when the edit began (empty if there was none), so the
                // half-typed text is not left applied for the next `f`.
                if mode == Input::Filter {
                    if let Some(original) = self.filter_restore.take() {
                        self.text = original;
                        self.update_list_filter();
                    }
                }
                self.input = None;
                self.text.clear();
                self.rename_target = None;
                self.command_target = None;
                self.network_add_target = None;
                self.pending_add_via = None;
                self.pending_edit = None;
            }
            KeyCode::Backspace => {
                if matches!(mode, Input::Rename | Input::Command) {
                    if let Some((at, _)) = self.text[..self.rename_cursor].char_indices().last() {
                        self.text.replace_range(at..self.rename_cursor, "");
                        self.rename_cursor = at;
                    }
                } else {
                    self.text.pop();
                }
                self.palette_selected = 0;
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.text.clear();
                self.rename_cursor = 0;
            }
            KeyCode::Left if matches!(mode, Input::Rename | Input::Command) => {
                self.rename_cursor = self.text[..self.rename_cursor]
                    .char_indices()
                    .last()
                    .map(|(at, _)| at)
                    .unwrap_or(0);
            }
            KeyCode::Right if matches!(mode, Input::Rename | Input::Command) => {
                if let Some(c) = self.text[self.rename_cursor..].chars().next() {
                    self.rename_cursor += c.len_utf8();
                }
            }
            KeyCode::Home if matches!(mode, Input::Rename | Input::Command) => {
                self.rename_cursor = 0
            }
            KeyCode::End if matches!(mode, Input::Rename | Input::Command) => {
                self.rename_cursor = self.text.len()
            }
            KeyCode::Delete if matches!(mode, Input::Rename | Input::Command) => {
                if let Some(c) = self.text[self.rename_cursor..].chars().next() {
                    self.text
                        .replace_range(self.rename_cursor..self.rename_cursor + c.len_utf8(), "");
                }
            }
            KeyCode::Char(c)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                if self.text.len() + c.len_utf8() <= if mode == Input::Command { 8192 } else { 256 }
                {
                    if matches!(mode, Input::Rename | Input::Command) {
                        self.text.insert(self.rename_cursor, c);
                        self.rename_cursor += c.len_utf8();
                    } else {
                        self.text.push(c);
                    }
                }
                self.palette_selected = 0;
            }
            KeyCode::Down if mode == Input::PreviewSearch => self.next_preview_match(1),
            KeyCode::Up if mode == Input::PreviewSearch => self.next_preview_match(-1),
            KeyCode::Down if mode == Input::Search && self.view == View::Files => {
                self.next_file_match(1)
            }
            KeyCode::Up if mode == Input::Search && self.view == View::Files => {
                self.next_file_match(-1)
            }
            KeyCode::Down if mode == Input::Search => self.move_selection(1),
            KeyCode::Up if mode == Input::Search => self.move_selection(-1),
            KeyCode::Down if mode == Input::Filter && self.dialog.is_some() => {
                if let Some(dialog) = self.dialog.clone() {
                    self.dialog_key(key, dialog);
                }
            }
            KeyCode::Up if mode == Input::Filter && self.dialog.is_some() => {
                if let Some(dialog) = self.dialog.clone() {
                    self.dialog_key(key, dialog);
                }
            }
            KeyCode::Down if mode == Input::Filter => self.move_selection(1),
            KeyCode::Up if mode == Input::Filter => self.move_selection(-1),
            KeyCode::Tab | KeyCode::BackTab
                if matches!(mode, Input::Search | Input::PreviewSearch | Input::Filter) =>
            {
                self.input = None;
                self.cycle_focus(
                    key.code == KeyCode::BackTab || key.modifiers.contains(KeyModifiers::SHIFT),
                );
            }
            KeyCode::Down if mode == Input::Palette => {
                let n = self.palette_view().len();
                self.palette_selected = (self.palette_selected + 1).min(n.saturating_sub(1));
            }
            KeyCode::Up if mode == Input::Palette => {
                self.palette_selected = self.palette_selected.saturating_sub(1)
            }
            KeyCode::PageDown if mode == Input::Palette => {
                let n = self.palette_view().len();
                self.palette_selected = (self.palette_selected + 8).min(n.saturating_sub(1));
            }
            KeyCode::PageUp if mode == Input::Palette => {
                self.palette_selected = self.palette_selected.saturating_sub(8)
            }
            KeyCode::Tab | KeyCode::BackTab if mode == Input::Palette => self.palette_next_section(
                key.code == KeyCode::BackTab || key.modifiers.contains(KeyModifiers::SHIFT),
            ),
            KeyCode::Enter => match mode {
                Input::Palette => self.run_palette_selected(),
                Input::Command => {
                    if !self.text.trim().is_empty() {
                        if let Some((d, directory)) = self.command_target.take() {
                            self.pending_command_panel = Some((
                                d,
                                RunCommand {
                                    directory,
                                    command: std::mem::take(&mut self.text),
                                },
                            ));
                            self.input = None;
                        }
                    }
                }
                Input::Search => {
                    self.input = None;
                    self.focus = Focus::Workspace;
                    if self.view != View::Files {
                        self.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
                    }
                }
                Input::PreviewSearch => {
                    self.input = None;
                    self.focus = Focus::Workspace;
                }
                Input::Filter => {
                    // Enter commits the filter; nothing to cancel back to.
                    self.filter_restore = None;
                    self.input = None;
                }
                Input::Rename => {
                    if !self.text.is_empty()
                        && self.text != "."
                        && self.text != ".."
                        && !self.text.contains('/')
                        && !self.text.chars().any(char::is_control)
                    {
                        if let Some((d, entry)) = self.rename_target.take() {
                            if entry.rename_name.as_deref() == Some(self.text.as_str()) {
                                self.set_notice_as(NoticeKind::Warning, "Name unchanged".into());
                            } else {
                                self.queue_file_actions(vec![(
                                    d,
                                    Operation::Rename {
                                        path: entry.path,
                                        name: self.text.clone(),
                                        expected_identity: entry.identity,
                                    },
                                )]);
                            }
                        }
                        self.input = None;
                    } else {
                        self.set_notice(
                            "Enter one filename · existing files are never replaced".into(),
                        );
                    }
                }
                Input::Add => {
                    let target = if let Some((_, address)) = &self.network_add_target {
                        format!("{}@{}", self.text, address)
                    } else {
                        self.text.clone()
                    };
                    if transport::valid_target(&target) {
                        if let Some(device) = self.pending_edit.take() {
                            match store::update_target(&self.devices[device], &target) {
                                Ok(updated) => {
                                    self.replace_device_endpoint(device, updated);
                                    self.set_notice_as(
                                        NoticeKind::Success,
                                        "SSH address updated · refresh to verify the connection"
                                            .into(),
                                    );
                                }
                                Err(error) => self.set_notice_as(
                                    NoticeKind::Error,
                                    safe_text(&format!("SSH address unchanged: {error:#}")),
                                ),
                            }
                            self.input = None;
                            return;
                        }
                        self.pending_add_via = self
                            .network_add_target
                            .take()
                            .map(|(d, _)| self.devices[d].clone())
                            .or_else(|| self.pending_add_via.take());
                        self.pending_add = Some(target);
                        self.input = None;
                    } else {
                        self.set_notice_as(
                            NoticeKind::Warning,
                            "Use an SSH alias or user@host".into(),
                        );
                    }
                }
                Input::Mkdir => {
                    if !self.text.is_empty()
                        && self.text != "."
                        && self.text != ".."
                        && !self.text.contains('/')
                        && !self.text.contains('\0')
                    {
                        if let Some(b) = &self.browser {
                            let path = join_component(&b.path, &self.text);
                            self.send(b.device, Operation::Mkdir { path });
                        }
                        self.input = None;
                    } else {
                        self.set_notice_as(
                            NoticeKind::Warning,
                            "Enter a single directory name".into(),
                        );
                    }
                }
            },
            _ => {}
        }
        if mode == Input::Search && self.input == Some(Input::Search) && self.text != before_text {
            self.live_search();
        }
        if mode == Input::PreviewSearch
            && self.input == Some(Input::PreviewSearch)
            && self.text != before_text
        {
            self.update_preview_search();
        }
        if mode == Input::Filter && self.input == Some(Input::Filter) && self.text != before_text {
            self.update_list_filter();
        }
    }
}
