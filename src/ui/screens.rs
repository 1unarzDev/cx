//! The design's screens as a controller: the network hub (start view), a device's
//! sessions, peek, the edge policy editor, keys and the send-to picker. One grammar:
//! a device letter, then a verb or a session slot (docs/design/concept-mission.md 6).
//! Keys, taps and context-menu rows all end in the same methods here.
use super::viewmodel::{dev_id, dev_index};
use super::views::mission::SLOTS;
use super::*;

/// The edge editor: which link (`a-b`, the opener first) and which lane has the bar.
pub(super) struct EdgeEdit {
    pub id: String,
    pub lane: usize,
    /// Policies before each change, newest last (`u` restores one).
    pub undo: Vec<(usize, usize, crate::edges::Policy)>,
}

/// A second `Esc` within this long goes to the hub (for layouts where `` ` `` is dead).
const DOUBLE_ESC: Duration = Duration::from_millis(400);

impl App {
    /// The viewer's own device (the local one), else the first.
    pub(super) fn local_index(&self) -> usize {
        self.devices
            .iter()
            .position(|d| d.target.is_none())
            .unwrap_or(0)
    }
    /// Device letters, kept stable while devices stay listed (see `hotkeys`).
    pub(super) fn hotkeys(&self) -> std::cell::Ref<'_, BTreeMap<String, char>> {
        let stale = {
            let map = self.hotkeys.borrow();
            map.len() != self.devices.len() || self.devices.iter().any(|d| !map.contains_key(&d.id))
        };
        if stale {
            let list: Vec<(&str, &str)> = self
                .devices
                .iter()
                .map(|d| (d.id.as_str(), d.name.as_str()))
                .collect();
            let next = hotkeys::assign(&list, &self.hotkeys.borrow());
            *self.hotkeys.borrow_mut() = next;
        }
        self.hotkeys.borrow()
    }
    fn device_for_letter(&self, c: char) -> Option<usize> {
        let letters = self.hotkeys();
        self.devices
            .iter()
            .position(|d| letters.get(&d.id) == Some(&c))
    }
    /// The browser the Files screen shows (the destination while choosing one).
    pub(super) fn active_browser(&self) -> Option<&Browser> {
        self.browser.as_ref()
    }
    /// First run: only this device, no links yet, not dismissed.
    pub(super) fn onboarding(&self) -> bool {
        !self.onboarding_dismissed
            && self.view == View::Network
            && self.devices.iter().all(|d| d.target.is_none())
            && self.edge_views.is_empty()
    }
    /// Remember a session's last frame in memory (never on disk), for its tile.
    pub(super) fn remember_frame(&mut self, d: usize, s: &Session, lines: &[String], cols: u16) {
        if self.frames.len() >= 64 {
            self.frames.clear();
        }
        let tail = lines.len().saturating_sub(60);
        self.frames.insert(
            (self.devices[d].id.clone(), workspaces::runtime_key(s)),
            (lines[tail..].to_vec(), cols),
        );
    }

    // ------------------------------------------------------------------ navigation

    /// The hub, with the network zone and this device selected (`` ` ``, Esc Esc, `cx`).
    pub(super) fn go_hub(&mut self) {
        self.close_screens();
        self.input = None;
        self.help = false;
        if self.view != View::Network {
            self.view = View::Network;
            self.reload_edges();
            self.refresh_work();
        }
        self.device = 0;
        self.search.clear();
        self.focus = Focus::Workspace;
    }
    fn close_screens(&mut self) {
        self.peek = false;
        self.stop_armed = None;
        self.send_pick = false;
        self.edge = None;
        self.edge_pending = false;
    }
    /// A device's sessions (its letter from the hub, a node tap).
    pub(super) fn open_device(&mut self, d: usize) {
        if d >= self.devices.len() {
            return;
        }
        self.close_screens();
        self.view = View::Work;
        self.device = d + 1;
        self.search.clear();
        self.focus = Focus::Workspace;
        self.hub_device = Some(dev_id(d));
        // The first session (slot a) is selected, else the new-session tile.
        let first = self.work[d].sessions.first().map(workspaces::runtime_key);
        self.reconcile_session_selection(first.map(|k| (d, k)));
        if !self.work[d].loading {
            self.work[d].loading = self.send(d, Operation::Sessions);
        }
    }
    /// The edge graph from the state files (on refresh and after a policy change; never
    /// from render).
    pub(super) fn reload_edges(&mut self) {
        if let Ok(graph) = crate::edges::graph() {
            self.edge_views = graph;
        }
    }
    /// The hub's sessions in tile order (what needs you first).
    fn hub_sessions_order(&self) -> Vec<(usize, String)> {
        let vm = viewmodel::ViewModel::from(self);
        let order = views::mission::by_attention(&vm.state);
        order
            .into_iter()
            .filter_map(|id| {
                let x = vm.state.session(&id)?;
                Some((dev_index(&x.device)?, id))
            })
            .collect()
    }
    /// Select a session by identity across all devices (the hub's scope).
    fn select_session(&mut self, d: usize, key: String) {
        self.reconcile_session_selection(Some((d, key)));
    }
    /// Move within the focused zone (keys, wheel).
    pub(super) fn screen_move(&mut self, delta: isize) {
        match self.view {
            View::Network if self.hub_sessions => {
                let order = self.hub_sessions_order();
                if order.is_empty() {
                    return;
                }
                let current = self
                    .selected_session()
                    .map(|(d, s)| (d, workspaces::runtime_key(&s)));
                let at = current
                    .and_then(|c| order.iter().position(|o| *o == c))
                    .unwrap_or(0);
                let (d, key) = order[shift(at, delta, order.len())].clone();
                self.select_session(d, key);
            }
            View::Network => {
                let order = self.hub_order.borrow().clone();
                if order.is_empty() {
                    return;
                }
                let current = self
                    .hub_device
                    .clone()
                    .unwrap_or_else(|| dev_id(self.local_index()));
                let at = order.iter().position(|id| *id == current).unwrap_or(0);
                self.hub_device = Some(order[shift(at, delta, order.len())].clone());
            }
            View::Work if self.device > 0 => {
                // Slots, then the new-session tile.
                let d = self.device - 1;
                let n = self.work[d].sessions.len();
                let current = self
                    .selected_session()
                    .and_then(|(_, s)| {
                        let k = workspaces::runtime_key(&s);
                        self.work[d]
                            .sessions
                            .iter()
                            .position(|x| workspaces::runtime_key(x) == k)
                    })
                    .unwrap_or(n);
                let next = shift(current, delta, n + 1);
                match self.work[d].sessions.get(next).map(workspaces::runtime_key) {
                    Some(key) => self.reconcile_session_selection(Some((d, key))),
                    // The new-session tile.
                    None => self.selected = self.session_rows().len(),
                }
            }
            View::Secrets => {
                let n = viewmodel::ViewModel::from(&*self).state.secrets.len();
                self.secret_selected = shift(self.secret_selected, delta, n.max(1));
            }
            _ => {}
        }
    }

    // --------------------------------------------------------------------- verbs

    fn attach_selected(&mut self) {
        if let Some((d, s)) = self.selected_session() {
            self.peek = false;
            self.open_session(d, s, false);
        }
    }
    /// A session slot on the current device (`a s d g`).
    fn attach_slot(&mut self, d: usize, slot: usize) {
        if let Some(s) = self.work[d].sessions.get(slot).cloned() {
            self.reconcile_session_selection(Some((d, workspaces::runtime_key(&s))));
            self.peek = false;
            self.open_session(d, s, false);
        }
    }
    /// New claude / codex / shell in the device's home, attached at once.
    fn new_session(&mut self, d: usize, provider: &str) {
        if self.creating {
            return;
        }
        if let Some(e) = &self.work[d].error {
            let e = safe_text(e);
            self.set_notice_as(NoticeKind::Warning, format!("Not started: {e}"));
            return;
        }
        // A selected container session scopes new work to its container.
        if let Some(scope) = self
            .selected_session()
            .filter(|(sd, _)| *sd == d)
            .and_then(|(_, s)| s.container)
        {
            self.open_container_chooser(d, scope);
            return;
        }
        self.launch_provider = None;
        self.create_permission_session(d, "~".into(), provider.into(), false, false, false);
    }
    /// `-`: re-attach the last detached session, from any screen.
    pub(super) fn attach_last(&mut self) {
        let Some((d, key)) = self.last_session.clone() else {
            self.set_notice("No session detached yet".into());
            return;
        };
        let Some(s) = self
            .work
            .get(d)
            .and_then(|w| {
                w.sessions
                    .iter()
                    .find(|s| workspaces::runtime_key(s) == key)
            })
            .cloned()
        else {
            self.set_notice("The last session has ended".into());
            return;
        };
        self.open_session(d, s, false);
    }
    /// After a detach: the hub, the sessions zone, the tile you left selected, so `⏎`
    /// (or `-`) re-attaches.
    pub(super) fn returned_from(&mut self, d: usize, s: &Session) {
        let key = workspaces::runtime_key(s);
        self.last_session = Some((d, key.clone()));
        self.close_screens();
        self.view = View::Network;
        self.device = 0;
        self.search.clear();
        self.hub_sessions = true;
        self.select_session(d, key);
    }
    /// `⌫`: the selected session asks inline; a second `⌫` stops it.
    fn arm_stop(&mut self) {
        let Some((d, session)) = self.selected_session() else {
            return;
        };
        if self.stoppable(d, &session) {
            self.stop_armed = Some((d, session));
        }
    }
    /// Retry a device the ticker names (`s r`), or the current one.
    fn retry(&mut self, d: usize) {
        if d < self.devices.len() && !self.work[d].loading {
            self.providers.remove(&d);
            self.work[d].loading = self.send_explicit(d, Operation::Sessions);
            self.set_notice(format!("Checking {}…", identity(&self.devices[d])));
        }
    }
    pub(super) fn open_secrets(&mut self) {
        self.close_screens();
        self.view = View::Secrets;
        self.focus = Focus::Workspace;
        // Names and metadata only: the index, never a backend or a value.
        let list = crate::store::ensure()
            .map(|dir| {
                crate::secrets::Secrets::at(dir, None, crate::secrets::Mode::Auto)
                    .list()
                    .unwrap_or_default()
            })
            .unwrap_or_default();
        self.secret_list = list;
    }
    /// A card's slot key (or `⏎`): its editor; past the last card, a new secret.
    fn secret_card(&mut self, i: usize) {
        let vm = viewmodel::ViewModel::from(&*self);
        match vm.state.secrets.get(i) {
            Some(s) => {
                self.secret_selected = i;
                self.set_notice(format!(
                    "Set it from a shell: cx secrets set {} (the value is read from stdin)",
                    s.key
                ));
            }
            None => self.set_notice("Add a secret from a shell: cx secrets set NAME".into()),
        }
    }
    fn invite(&mut self) {
        self.set_notice("Invite a device from a shell: cx mesh invite".into());
    }

    // ---------------------------------------------------------------- edge editor

    fn edge_device_id(&self, d: usize) -> String {
        if self.devices[d].target.is_none() {
            crate::edges::LOCAL.into()
        } else {
            self.devices[d].id.clone()
        }
    }
    /// `e` then a letter: the link between `from` (this device or the hub's viewer) and
    /// that device.
    fn open_edge(&mut self, from: usize, to: usize) {
        if from == to {
            return;
        }
        self.reload_edges();
        self.edge_pending = false;
        self.edge = Some(EdgeEdit {
            id: format!("{from}-{to}"),
            lane: 0,
            undo: Vec::new(),
        });
    }
    fn edge_ends(&self) -> Option<(usize, usize)> {
        let e = self.edge.as_ref()?;
        let (a, b) = e.id.split_once('-')?;
        Some((a.parse().ok()?, b.parse().ok()?))
    }
    fn current_policy(&self, from: usize, to: usize) -> crate::edges::Policy {
        use crate::edges::{Access, Policy};
        let (f, t) = (self.edge_device_id(from), self.edge_device_id(to));
        let access = self.edge_views.iter().find_map(|e| {
            if e.from == f && e.to == t {
                Some(e.forward.access)
            } else if e.from == t && e.to == f {
                Some(e.reverse.access)
            } else {
                None
            }
        });
        match access {
            Some(Access::Key) => Policy::Key,
            Some(Access::Refused) => Policy::None,
            _ => Policy::Password,
        }
    }
    /// Apply one direction's policy at once (off the UI thread: enforcement uses SSH).
    fn set_policy(&mut self, from: usize, to: usize, policy: crate::edges::Policy, undoable: bool) {
        let before = self.current_policy(from, to);
        if undoable {
            if let Some(e) = self.edge.as_mut() {
                e.undo.push((from, to, before));
            }
        }
        let change = crate::edges::Change {
            from: self.edge_device_id(from),
            to: self.edge_device_id(to),
            policy,
        };
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let result = crate::edges::plan(change)
                .and_then(|plan| crate::edges::apply(&plan).map(|a| (plan, a)))
                .map(|(plan, applied)| crate::edges::render_applied(&plan, &applied));
            let _ = tx.send(result.map_err(|e| format!("{e:#}")));
        });
        self.edge_job = Some(rx);
        self.set_notice(format!(
            "Saving {} → {}: {}…",
            safe_label(&self.devices[from].name),
            safe_label(&self.devices[to].name),
            policy.label()
        ));
    }
    /// A finished policy change: reload the graph and say how it went.
    pub(super) fn poll_edge_job(&mut self) -> bool {
        let Some(rx) = &self.edge_job else {
            return false;
        };
        let Ok(result) = rx.try_recv() else {
            return false;
        };
        self.edge_job = None;
        self.reload_edges();
        match result {
            Ok(text) => {
                let first = text.lines().next().unwrap_or("Saved").to_string();
                self.set_notice_as(NoticeKind::Success, safe_text(&first));
            }
            Err(e) => self.set_notice_as(
                NoticeKind::Error,
                safe_text(&format!("Policy unchanged: {e}")),
            ),
        }
        true
    }
    fn edge_key(&mut self, key: KeyEvent) {
        use crate::edges::Policy;
        let Some((a, b)) = self.edge_ends() else {
            self.edge = None;
            return;
        };
        let KeyCode::Char(c) = key.code else {
            match key.code {
                KeyCode::Esc => self.edge = None,
                KeyCode::Down | KeyCode::Up => self.edge_lane(),
                _ => {}
            }
            return;
        };
        let pol = match c.to_ascii_lowercase() {
            'a' => Some(Policy::Key),
            'p' => Some(Policy::Password),
            'o' => Some(Policy::None),
            _ => None,
        };
        match (c, pol) {
            (_, Some(p)) if c.is_ascii_lowercase() => self.set_policy(a, b, p, true),
            (_, Some(p)) => self.set_policy(b, a, p, true),
            ('j' | 'k', _) => self.edge_lane(),
            ('u', _) => {
                if let Some((f, t, p)) = self.edge.as_mut().and_then(|e| e.undo.pop()) {
                    self.set_policy(f, t, p, false);
                } else {
                    self.set_notice("Nothing to undo".into());
                }
            }
            _ => {}
        }
    }
    fn edge_lane(&mut self) {
        if let Some(e) = self.edge.as_mut() {
            e.lane = 1 - e.lane.min(1);
        }
    }

    // ------------------------------------------------------------------ dispatch

    /// The design's keys, before the static keymap. Returns true when consumed.
    pub(super) fn screen_key(&mut self, key: KeyEvent) -> bool {
        if self.edge.is_some() {
            self.edge_key(key);
            return true;
        }
        let plain = key.modifiers.difference(KeyModifiers::SHIFT).is_empty();
        // `⌫` asked; a second `⌫` stops, every other key keeps the session.
        if let Some((d, session)) = self.stop_armed.take() {
            if key.code == KeyCode::Backspace {
                self.send_stop(d, session);
            }
            return true;
        }
        if self.send_pick {
            match key.code {
                KeyCode::Char(c) if plain => {
                    if let Some(d) = self.device_for_letter(c) {
                        self.send_pick = false;
                        self.send_to(d);
                    }
                }
                KeyCode::Esc => self.send_pick = false,
                _ => {}
            }
            return true;
        }
        if !plain {
            return false;
        }
        if key.code == KeyCode::Esc {
            let double = self.last_esc.is_some_and(|t| t.elapsed() < DOUBLE_ESC);
            self.last_esc = Some(Instant::now());
            if double {
                self.go_hub();
                return true;
            }
        }
        match key.code {
            KeyCode::Char('`') => {
                self.go_hub();
                return true;
            }
            KeyCode::Char('-') if self.view != View::Files => {
                self.attach_last();
                return true;
            }
            KeyCode::Char('+') => {
                self.invite();
                return true;
            }
            KeyCode::Char('K') => {
                self.open_secrets();
                return true;
            }
            _ => {}
        }
        if self.peek {
            return self.peek_key(key);
        }
        match self.view {
            // Onboarding answers its own keys; the hub's work beneath it.
            View::Network => (self.onboarding() && self.onboarding_key(key)) || self.hub_key(key),
            View::Work if self.device > 0 => self.device_key(key, self.device - 1),
            View::Secrets => self.secrets_key(key),
            View::Files => self.files_key(key),
            _ => false,
        }
    }
    /// `e` then a device letter (from `from`), or a device letter alone.
    fn letter(&mut self, c: char, from: usize) -> bool {
        if std::mem::take(&mut self.edge_pending) {
            if let Some(d) = self.device_for_letter(c) {
                self.open_edge(from, d);
            }
            return true;
        }
        if c == 'e' {
            self.edge_pending = true;
            return true;
        }
        false
    }
    fn hub_key(&mut self, key: KeyEvent) -> bool {
        let local = self.local_index();
        match key.code {
            KeyCode::Char(c) if self.letter(c, local) => true,
            // (`j k` and the vertical arrows are list motion: counts, `gg`, `G`.)
            KeyCode::Right => {
                self.screen_move(1);
                true
            }
            KeyCode::Left => {
                self.screen_move(-1);
                true
            }
            KeyCode::Char('j' | 'k' | 'G') => false,
            KeyCode::Char(c) if c.is_ascii_lowercase() => {
                if let Some(d) = self.device_for_letter(c) {
                    self.open_device(d);
                }
                true
            }
            KeyCode::Tab | KeyCode::BackTab => {
                self.toggle_zone();
                true
            }
            KeyCode::Enter => {
                self.primary_here();
                true
            }
            KeyCode::Char(' ') if self.hub_sessions => {
                self.peek = self.selected_session().is_some();
                true
            }
            KeyCode::Backspace if self.hub_sessions => {
                self.arm_stop();
                true
            }
            _ => false,
        }
    }
    fn device_key(&mut self, key: KeyEvent, d: usize) -> bool {
        match key.code {
            KeyCode::Char(c) if self.letter(c, d) => true,
            KeyCode::Char(c) if SLOTS.contains(&c) => {
                let slot = SLOTS.iter().position(|s| *s == c).unwrap_or(0);
                self.attach_slot(d, slot);
                true
            }
            KeyCode::Char('c') => {
                self.new_session(d, "claude");
                true
            }
            KeyCode::Char('x') => {
                self.new_session(d, "codex");
                true
            }
            KeyCode::Char('n') => {
                self.new_session(d, "shell");
                true
            }
            KeyCode::Char('f') => {
                self.open_files(d);
                true
            }
            KeyCode::Char('r') => {
                self.retry(d);
                true
            }
            KeyCode::Right | KeyCode::Tab => {
                self.screen_move(1);
                true
            }
            KeyCode::Left | KeyCode::BackTab => {
                self.screen_move(-1);
                true
            }
            KeyCode::Enter => {
                self.primary_here();
                true
            }
            KeyCode::Char(' ') => {
                self.peek = self.selected_session().is_some();
                true
            }
            KeyCode::Backspace => {
                self.arm_stop();
                true
            }
            KeyCode::Esc => {
                self.go_hub();
                true
            }
            _ => false,
        }
    }
    fn peek_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Enter => self.attach_selected(),
            KeyCode::Tab => self.peek_next(1),
            KeyCode::BackTab => self.peek_next(-1),
            KeyCode::Backspace => self.arm_stop(),
            KeyCode::Esc => self.peek = false,
            // Hub letters work in peek: a letter, then a slot (`t a`).
            KeyCode::Char(c) if self.device_for_letter(c).is_some() => {
                if let Some(d) = self.device_for_letter(c) {
                    self.open_device(d);
                }
            }
            _ => return false,
        }
        true
    }
    fn peek_next(&mut self, delta: isize) {
        let was = self.hub_sessions;
        self.hub_sessions = true;
        let view = self.view;
        self.view = View::Network;
        self.screen_move(delta);
        self.view = view;
        self.hub_sessions = was;
    }
    fn onboarding_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Enter => {
                self.set_notice("Connect Cloudflare from a shell: cx mesh setup".into());
                true
            }
            KeyCode::Esc => {
                // Plain SSH: the hub, with devices added by address (palette).
                self.onboarding_dismissed = true;
                true
            }
            _ => false,
        }
    }
    fn secrets_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Char(c) if SLOTS.contains(&c) => {
                let i = SLOTS.iter().position(|s| *s == c).unwrap_or(0);
                self.secret_card(i);
            }
            KeyCode::Enter => self.secret_card(self.secret_selected),
            KeyCode::Char('h') => {
                self.set_notice("Add a secret from a shell: cx secrets set NAME".into())
            }
            KeyCode::Esc => self.go_hub(),
            _ => return false,
        }
        true
    }
    /// Files: `s` raises the send-to picker (a device letter sends).
    fn files_key(&mut self, key: KeyEvent) -> bool {
        let list = self.browser.as_ref().is_some_and(|b| b.preview.is_none());
        match key.code {
            KeyCode::Char('s') if list && self.input.is_none() => {
                if self.chosen_entries().is_empty() {
                    return false;
                }
                self.send_pick = true;
                true
            }
            _ => false,
        }
    }
    /// A device's files: where its browser already is (the launch folder for this
    /// device), else the folder last seen there, else its home. Only the focused
    /// location changes; a transfer's other location stays.
    pub(super) fn open_files(&mut self, d: usize) {
        self.close_screens();
        self.device = d + 1;
        self.view = View::Files;
        self.select_file_device();
        self.generation += 1;
        self.refresh_browser();
    }
    /// Send the chosen files to device `d`: its destination browser opens, Enter there
    /// transfers (the existing destination flow).
    fn send_to(&mut self, d: usize) {
        self.execute(Action::Copy);
        self.chosen_device(d, ChooseDevice::Destination);
    }

    /// `⏎` (and its chips): open or attach what the screen has selected.
    fn primary_here(&mut self) {
        if self.input == Some(Input::Palette) {
            return self.run_palette_selected();
        }
        match self.view {
            _ if self.peek => self.attach_selected(),
            View::Network if self.hub_sessions => self.attach_selected(),
            View::Network => {
                let local = self.local_index();
                let d = self
                    .hub_device
                    .as_deref()
                    .and_then(dev_index)
                    .unwrap_or(local);
                self.open_device(d);
            }
            View::Work if self.device > 0 => {
                if self.selected_session().is_some() {
                    self.attach_selected();
                } else {
                    self.new_session(self.device - 1, "shell");
                }
            }
            _ => self.primary(),
        }
    }
    /// `esc` (and its chips): back one level.
    fn back_here(&mut self) {
        if self.input == Some(Input::Palette) {
            self.input = None;
            self.text.clear();
        } else if self.edge.is_some() {
            self.edge = None;
        } else if self.peek {
            self.peek = false;
        } else if self.send_pick {
            self.send_pick = false;
        } else if self.help {
            self.help = false;
        } else if matches!(self.view, View::Work | View::Secrets) {
            self.go_hub();
        } else {
            self.back();
        }
    }
    fn toggle_zone(&mut self) {
        self.hub_sessions = !self.hub_sessions;
        if self.hub_sessions && self.selected_session().is_none() {
            self.screen_move(0);
        }
    }
    /// Select what a view target shows (a right-click or long press, before its menu).
    pub(super) fn select_target(&mut self, action: &str) {
        let mut parts = action.splitn(3, ':');
        match (parts.next(), parts.next(), parts.next()) {
            (Some("device"), Some(d), _) => {
                if self.view == View::Network {
                    self.hub_sessions = false;
                    self.hub_device = Some(d.to_string());
                }
            }
            (Some("session"), Some(d), Some(key)) => {
                if let Some(d) = dev_index(d) {
                    if self.view == View::Network {
                        self.hub_sessions = true;
                    }
                    self.select_session(d, key.to_string());
                }
            }
            _ => {}
        }
    }
    /// A tap on a view target: the same method its key runs.
    pub(super) fn view_action(&mut self, action: &str) {
        let mut parts = action.splitn(3, ':');
        let (head, a, b) = (parts.next().unwrap_or(""), parts.next(), parts.next());
        let current = self
            .device
            .checked_sub(1)
            .filter(|_| self.view == View::Work);
        match (head, a, b) {
            ("device", Some(d), _) => {
                if self.send_pick {
                    self.send_pick = false;
                    if let Some(d) = dev_index(d) {
                        self.send_to(d);
                    }
                } else if let Some(d) = dev_index(d) {
                    self.open_device(d);
                }
            }
            ("send", Some(d), _) => {
                if let Some(d) = dev_index(d) {
                    self.send_pick = false;
                    self.send_to(d);
                }
            }
            ("session", Some(d), Some(key)) => {
                if let Some(d) = dev_index(d) {
                    self.select_session(d, key.to_string());
                    self.attach_selected();
                }
            }
            ("new", Some(d), Some(provider)) => {
                if let Some(d) = dev_index(d) {
                    self.new_session(d, provider);
                }
            }
            ("edge", Some(id), _) => {
                if let Some((x, y)) = id.split_once('-') {
                    if let (Ok(x), Ok(y)) = (x.parse(), y.parse()) {
                        self.open_edge(x, y);
                    }
                }
            }
            ("policy", Some(from), Some(rest)) => {
                let (to, value) = rest.split_once(':').unwrap_or((rest, ""));
                let policy = match value {
                    "key" => Some(crate::edges::Policy::Key),
                    "password" => Some(crate::edges::Policy::Password),
                    "off" => Some(crate::edges::Policy::None),
                    _ => None,
                };
                if let (Some(f), Some(t), Some(p)) = (dev_index(from), dev_index(to), policy) {
                    self.set_policy(f, t, p, true);
                }
            }
            ("retry", Some(d), _) => {
                if let Some(d) = dev_index(d) {
                    self.retry(d);
                }
            }
            ("palette", Some(i), _) => {
                if let Ok(i) = i.parse::<usize>() {
                    if self.input == Some(Input::Palette) && self.palette_selected == i {
                        self.run_palette_selected();
                    } else if self.input == Some(Input::Palette) {
                        self.palette_selected = i;
                    }
                } else {
                    self.open_palette(match i {
                        "sessions" | "new" | "devices" => "",
                        _ => "",
                    });
                }
            }
            ("secret" | "secret-add", Some(name), _) => {
                let vm = viewmodel::ViewModel::from(&*self);
                if let Some(i) = vm.state.secrets.iter().position(|s| s.key == name) {
                    self.secret_card(i);
                }
            }
            ("file", _, _) | ("mesh", _, _) | ("cmd", _, _) => {}
            // Footer chips and buttons: the verbs their keys run.
            ("back", None, _) => self.back_here(),
            ("hub", None, _) => self.go_hub(),
            ("last", None, _) => self.attach_last(),
            ("palette" | "find", None, _) => self.open_palette(""),
            ("invite" | "join", None, _) => self.invite(),
            ("secrets", None, _) => self.open_secrets(),
            ("help", None, _) => self.run_cmd(keymap::Cmd::Help),
            ("detach", None, _) => {}
            ("zone", None, _) => self.toggle_zone(),
            ("attach" | "open" | "go" | "connect", None, _) => self.primary_here(),
            ("peek" | "preview", None, _) => {
                if self.view == View::Files {
                    self.primary();
                } else {
                    self.peek = self.selected_session().is_some();
                }
            }
            ("stop", None, _) => self.arm_stop(),
            ("files", None, _) => {
                if let Some(d) = current {
                    self.open_files(d);
                }
            }
            ("links", None, _) => self.edge_pending = true,
            ("retry", None, _) => {
                if let Some(d) = current {
                    self.retry(d);
                }
            }
            ("send", None, _) => {
                if !self.chosen_entries().is_empty() {
                    self.send_pick = true;
                }
            }
            ("copy", None, _) => self.execute(Action::Copy),
            ("peek-next", None, _) => self.peek_next(1),
            ("undo", None, _) => {
                self.edge_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::NONE))
            }
            ("lane", None, _) => self.edge_lane(),
            ("skip", None, _) => self.onboarding_dismissed = true,
            (verb @ ("new-claude" | "new-codex" | "new-shell"), None, _) => {
                if let Some(d) = current {
                    self.new_session(d, verb.trim_start_matches("new-"));
                }
            }
            _ => {}
        }
    }
}
