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
pub(super) const DOUBLE_ESC: Duration = Duration::from_millis(400);

/// The keys screen's masked entry. The value is never drawn, logged or copied out of this
/// buffer except into the store, and is wiped when the entry is dropped.
pub(super) struct SecretEntry {
    pub name: String,
    /// Typing the new secret's name (plain text) before its value.
    pub naming: bool,
    pub value: zeroize::Zeroizing<String>,
}

impl SecretEntry {
    /// The value's buffer is allocated once at its limit, so typing never reallocates
    /// and leaves no unwiped copy behind.
    pub fn new(name: String, naming: bool) -> SecretEntry {
        SecretEntry {
            name,
            naming,
            value: zeroize::Zeroizing::new(String::with_capacity(SECRET_INPUT_LIMIT)),
        }
    }
    /// Typed or pasted text: the name while naming, else the value (control characters
    /// dropped, bounded).
    fn push(&mut self, text: &str) {
        let field: &mut String = if self.naming {
            &mut self.name
        } else {
            &mut self.value
        };
        for c in text.chars().filter(|c| !c.is_control()) {
            if field.len() + c.len_utf8() > SECRET_INPUT_LIMIT {
                break;
            }
            field.push(c);
        }
    }
}

/// The hub's invite (`+`): the ssh-overlay one-shot listener on its own thread. Dropping
/// this closes it (Esc, `R`, quitting) within a poll, so no listener outlives the screen.
pub(super) struct InviteRun {
    pub rx: mpsc::Receiver<std::result::Result<InviteEvent, String>>,
    pub cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// The invite once its listener is up, and when it expires.
    pub shown: Option<(crate::mesh::Invite, Instant)>,
    /// Seconds left when last drawn (the countdown redraws once a second).
    pub drawn_secs: std::cell::Cell<u64>,
}
pub(super) enum InviteEvent {
    Ready(crate::mesh::Invite),
    Joined(serde_json::Value),
}
impl Drop for InviteRun {
    fn drop(&mut self) {
        self.cancel
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }
}
/// How long a hub invite stays open (its expiry bar's full length).
pub(super) const INVITE_TTL: Duration = crate::mesh::ssh_overlay::DEFAULT_TTL;
/// The longest secret the entry takes (the store's own limit is larger).
const SECRET_INPUT_LIMIT: usize = 16 * 1024;

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
    /// A device's sessions in slot order (`a s d g`): creation order, so a slot keeps
    /// its session while others come and go.
    pub(super) fn slot_sessions(&self, d: usize) -> Vec<&Session> {
        let mut v: Vec<&Session> = self.work[d].sessions.iter().collect();
        v.sort_by_key(|s| {
            let created = s
                .terminal
                .as_ref()
                .and_then(|t| t.created.parse::<u64>().ok());
            (created.unwrap_or(u64::MAX), s.started.clone(), s.id.clone())
        });
        v
    }
    pub(super) fn device_for_letter(&self, c: char) -> Option<usize> {
        let letters = self.hotkeys();
        self.devices
            .iter()
            .position(|d| letters.get(&d.id) == Some(&c))
    }
    /// First run: only this device, no links and no sessions yet, not dismissed.
    pub(super) fn onboarding(&self) -> bool {
        !self.onboarding_dismissed
            && self.view == View::Network
            && self.devices.iter().all(|d| d.target.is_none())
            && self.edge_views.is_empty()
            && self.work.iter().all(|w| w.sessions.is_empty())
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
        self.close_invite();
    }
    /// Close the invite, if one is open: its listener stops within a poll (its thread,
    /// in `invite_thread`, is waited on by the next invite's). True when one was open.
    fn close_invite(&mut self) -> bool {
        self.invite.take().is_some()
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
        let first = self
            .slot_sessions(d)
            .first()
            .map(|s| workspaces::runtime_key(s));
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
    /// An arrow (or `h j k l`) in the hub's map or tiles, or in a device's tiles: the
    /// nearest item the last frame drew that way, else along the zone's order (left and
    /// right), else into the other zone (down from the map, up from the tiles).
    /// A count repeats it (`3j`).
    fn arrow(&mut self, dx: i32, dy: i32) {
        for _ in 0..self.navigation.take_count().unwrap_or(1).min(64) {
            self.arrow_once(dx, dy);
        }
    }
    fn arrow_once(&mut self, dx: i32, dy: i32) {
        match self.spatial(dx, dy) {
            Some(true) => return,
            // Not drawn (no frame yet, or scrolled off): along the order.
            None => return self.screen_move((dx + dy) as isize),
            Some(false) => {}
        }
        let other = self.view == View::Network && (dy > 0) != self.hub_sessions && dy != 0;
        if other {
            self.toggle_zone();
        } else if dx != 0 || self.view != View::Network {
            self.screen_move((dx + dy) as isize);
        }
    }
    /// Move to the nearest drawn item in direction (`dx`, `dy`) of the selection's own:
    /// whether one lay that way, None when the selection was not drawn.
    fn spatial(&mut self, dx: i32, dy: i32) -> Option<bool> {
        let graph = self.view == View::Network && !self.hub_sessions;
        let current = if graph {
            let d = self.hub_device.clone();
            format!("device:{}", d.unwrap_or_else(|| dev_id(self.local_index())))
        } else {
            let (d, s) = self.selected_session()?;
            format!("session:{d}:{}", workspaces::runtime_key(&s))
        };
        let kind = if graph { "device:" } else { "session:" };
        let nav = self.nav.borrow().clone();
        let from = nav.iter().find(|(_, a)| *a == current).map(|(r, _)| *r)?;
        // Centres in half cells; an item counts when it lies past the selection's edge
        // that way, scored by distance with the sideways offset weighed double.
        let centre = |r: Rect| {
            (
                2 * i32::from(r.x) + i32::from(r.width),
                2 * i32::from(r.y) + i32::from(r.height),
            )
        };
        let (fx, fy) = centre(from);
        let best = nav
            .iter()
            .filter(|(r, a)| a.starts_with(kind) && *a != current && *r != from)
            .filter_map(|(r, a)| {
                let (x, y) = centre(*r);
                let (along, side) = if dx != 0 {
                    let past = if dx > 0 {
                        r.x >= from.right()
                    } else {
                        r.right() <= from.x
                    };
                    (past.then_some((x - fx).abs())?, (y - fy).abs())
                } else {
                    let past = if dy > 0 {
                        r.y >= from.bottom()
                    } else {
                        r.bottom() <= from.y
                    };
                    (past.then_some((y - fy).abs())?, (x - fx).abs())
                };
                Some((along + 2 * side, a.clone()))
            })
            .min_by_key(|(score, _)| *score)
            .map(|(_, a)| a);
        let Some(to) = best else {
            return Some(false);
        };
        let mut parts = to.splitn(3, ':').skip(1);
        match (parts.next(), parts.next()) {
            (Some(id), None) => self.hub_device = Some(id.to_string()),
            (Some(d), Some(key)) => self.select_session(dev_index(d)?, key.to_string()),
            _ => return None,
        }
        Some(true)
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
                let slots: Vec<String> = self
                    .slot_sessions(d)
                    .iter()
                    .map(|s| workspaces::runtime_key(s))
                    .collect();
                let n = slots.len();
                let current = self
                    .selected_session()
                    .and_then(|(_, s)| {
                        let k = workspaces::runtime_key(&s);
                        slots.iter().position(|x| *x == k)
                    })
                    .unwrap_or(n);
                let next = shift(current, delta, n + 1);
                match slots.get(next).cloned() {
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
        if let Some(s) = self.slot_sessions(d).get(slot).map(|s| (*s).clone()) {
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
        if let Some((d, s)) = self.last_detached() {
            self.open_session(d, s, false);
        }
    }
    /// The last detached session, if it still runs; otherwise a notice says why.
    pub(super) fn last_detached(&mut self) -> Option<(usize, Session)> {
        let Some((d, key)) = self.last_session.clone() else {
            self.set_notice("No session detached yet".into());
            return None;
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
            return None;
        };
        Some((d, s))
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
                crate::secrets::Secrets::at(dir, None)
                    .list()
                    .unwrap_or_default()
            })
            .unwrap_or_default();
        self.secret_list = list;
    }
    /// A card's slot key (or `⏎`): its masked entry (set, or rotate a set one); past the
    /// last card, a new secret (its name first).
    fn secret_card(&mut self, i: usize) {
        let name = self.secret_list.get(i).map(|m| m.name.clone()).or_else(|| {
            // The missing cards the view adds after the stored ones.
            let vm = viewmodel::ViewModel::from(&*self);
            vm.state.secrets.get(i).map(|s| s.key.clone())
        });
        match name {
            Some(name) => {
                self.secret_selected = i;
                self.secret_entry = Some(SecretEntry::new(name, false));
            }
            None => self.new_secret(),
        }
    }
    fn new_secret(&mut self) {
        self.secret_entry = Some(SecretEntry::new(String::new(), true));
    }
    /// Keys while the masked entry is open: it owns them all (Ctrl+C still quits).
    fn secret_entry_key(&mut self, key: KeyEvent) {
        let Some(e) = self.secret_entry.as_mut() else {
            return;
        };
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Esc => self.secret_entry = None,
            KeyCode::Char('u') if ctrl => {
                e.value.clear();
                if e.naming {
                    e.name.clear();
                }
            }
            KeyCode::Char(c) if !ctrl => e.push(&c.to_string()),
            KeyCode::Backspace if e.naming => {
                e.name.pop();
            }
            KeyCode::Backspace => {
                e.value.pop();
            }
            KeyCode::Enter if e.naming => match crate::secrets::validate_name(&e.name) {
                Ok(()) => e.naming = false,
                Err(err) => self.set_notice_as(NoticeKind::Error, safe_text(&format!("{err:#}"))),
            },
            KeyCode::Enter if !e.value.is_empty() => self.save_secret(),
            _ => {}
        }
    }
    /// A paste into the entry (bracketed, so a trailing newline never submits it).
    pub(super) fn secret_paste(&mut self, text: &str) {
        if let Some(e) = self.secret_entry.as_mut() {
            e.push(text.trim_end_matches(['\r', '\n']));
        }
    }
    pub(super) fn secret_entry_open(&self) -> bool {
        self.secret_entry.is_some()
    }
    /// Store the entered value off the UI thread (the keyring is a subprocess).
    fn save_secret(&mut self) {
        let Some(SecretEntry { name, value, .. }) = self.secret_entry.take() else {
            return;
        };
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let result = crate::secrets::Secrets::open()
                .and_then(|s| s.set(&name, &value))
                .map(|m| format!("Saved {} ({})", m.name, m.backend.as_str()));
            drop(value);
            let _ = tx.send(result.map_err(|e| format!("Not saved: {e:#}")));
        });
        self.secret_job = Some(rx);
        self.set_notice("Saving…".into());
    }
    pub(super) fn poll_secret_job(&mut self) -> bool {
        let Some(Ok(result)) = self.secret_job.as_ref().map(|rx| rx.try_recv()) else {
            return false;
        };
        self.secret_job = None;
        match result {
            Ok(text) => {
                if self.view == View::Secrets {
                    self.open_secrets();
                }
                self.set_notice_as(NoticeKind::Success, safe_text(&text));
            }
            Err(e) => self.set_notice_as(NoticeKind::Error, safe_text(&e)),
        }
        true
    }

    /// `+`: open a single-use ssh-overlay invite on this host's mesh/LAN addresses; its
    /// listener runs on its own thread. A running one is closed first (`R`: a new code
    /// revokes one that may already be shared).
    fn invite(&mut self) {
        self.close_screens();
        // The old listener (also one Esc closed) ends within a poll; the new one waits for it
        // off the UI thread, so it finds the port free.
        self.invite = None;
        let previous = self.invite_thread.take();
        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let opts = crate::mesh::ssh_overlay::InviteOptions {
            cancel: Some(cancel.clone()),
            ..Default::default()
        };
        let (tx, rx) = mpsc::channel();
        let thread = thread::spawn(move || {
            if let Some(previous) = previous {
                let _ = previous.join();
            }
            let ready = tx.clone();
            let result = crate::mesh::ssh_overlay::invite(&opts, |inv| {
                let _ = ready.send(Ok(InviteEvent::Ready(inv.clone())));
                Ok(())
            });
            let _ = tx.send(
                result
                    .map(InviteEvent::Joined)
                    .map_err(|e| format!("{e:#}")),
            );
        });
        self.invite = Some(InviteRun {
            rx,
            cancel,
            shown: None,
            drawn_secs: Default::default(),
        });
        self.invite_thread = Some(thread);
    }
    /// The invite's news: up (show it), joined or closed (say so); and its countdown.
    pub(super) fn poll_invite(&mut self) -> bool {
        let Some(run) = self.invite.as_mut() else {
            return false;
        };
        let mut dirty = false;
        while let Ok(event) = run.rx.try_recv() {
            match event {
                Ok(InviteEvent::Ready(inv)) => {
                    run.shown = Some((inv, Instant::now() + INVITE_TTL));
                    dirty = true;
                }
                Ok(InviteEvent::Joined(v)) => {
                    self.close_invite();
                    let name = v["name"].as_str().unwrap_or("a device");
                    self.set_notice_as(
                        NoticeKind::Success,
                        safe_text(&format!("{name} joined the mesh")),
                    );
                    return true;
                }
                Err(e) => {
                    self.close_invite();
                    self.set_notice_as(NoticeKind::Error, safe_text(&format!("Invite: {e}")));
                    return true;
                }
            }
        }
        let left = self.invite_secs_left();
        let run = self.invite.as_ref().unwrap();
        dirty | (run.drawn_secs.replace(left) != left)
    }
    pub(super) fn invite_secs_left(&self) -> u64 {
        self.invite
            .as_ref()
            .and_then(|r| r.shown.as_ref())
            .map_or(0, |(_, until)| {
                until.saturating_duration_since(Instant::now()).as_secs()
            })
    }
    /// The invite's keys; `?` and `/` are the keymap's (help, find).
    fn invite_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Esc => {
                self.close_invite();
                self.set_notice("Invite closed".into());
            }
            KeyCode::Char('R') => self.invite(),
            KeyCode::Char('y') => self.copy_invite(false),
            KeyCode::Char('Y') => self.copy_invite(true),
            KeyCode::Char('?' | '/') => return false,
            _ => {}
        }
        true
    }
    /// `y` (a tap on the code): the code; `Y` (a tap on the command): the whole `cx join` line.
    fn copy_invite(&mut self, command: bool) {
        let Some((inv, _)) = self.invite.as_ref().and_then(|r| r.shown.as_ref()) else {
            return;
        };
        let (what, text) = if command {
            ("the command", inv.one_liner_sh.clone())
        } else {
            ("the code", inv.code.clone())
        };
        self.copy_text(what, text);
    }
    /// `text` to the system clipboard through the terminal (OSC 52, written by the event
    /// loop). No terminal confirms it, so the notice hedges (spec 6).
    pub(super) fn copy_text(&mut self, what: &str, text: String) {
        self.copy_out = Some(text);
        self.set_notice(format!(
            "Copied {what}, if your terminal allows clipboard access"
        ));
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
    pub(super) fn open_edge(&mut self, from: usize, to: usize) {
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
        // One change at a time: the next one is planned from the graph this one reloads.
        if self.edge_job.is_some() {
            self.set_notice("Still saving the last change…".into());
            return;
        }
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
    /// The link's keys (plain keys only; chords are the keymap's); `?` and `/` are the
    /// keymap's too (help, find).
    fn edge_key(&mut self, key: KeyEvent) -> bool {
        use crate::edges::Policy;
        let Some((a, b)) = self.edge_ends() else {
            self.edge = None;
            return true;
        };
        let KeyCode::Char(c) = key.code else {
            match key.code {
                KeyCode::Esc => self.edge = None,
                KeyCode::Down | KeyCode::Up => self.edge_lane(),
                _ => {}
            }
            return true;
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
                if self.edge_job.is_some() {
                    self.set_notice("Still saving the last change…".into());
                } else if let Some((f, t, p)) = self.edge.as_mut().and_then(|e| e.undo.pop()) {
                    self.set_policy(f, t, p, false);
                } else {
                    self.set_notice("Nothing to undo".into());
                }
            }
            ('?' | '/', _) => return false,
            _ => {}
        }
        true
    }
    fn edge_lane(&mut self) {
        if let Some(e) = self.edge.as_mut() {
            e.lane = 1 - e.lane.min(1);
        }
    }

    // ------------------------------------------------------------------ dispatch

    /// The design's keys, before the static keymap. Returns true when consumed.
    pub(super) fn screen_key(&mut self, key: KeyEvent) -> bool {
        if self.secret_entry.is_some() {
            self.secret_entry_key(key);
            return true;
        }
        let plain = key.modifiers.difference(KeyModifiers::SHIFT).is_empty();
        // A pending `e` takes the next key only when it is a device letter: the link to
        // it; any other key drops the `e` and does what it always does.
        if std::mem::take(&mut self.edge_pending) && plain {
            if let Some(d) = match key.code {
                KeyCode::Char(c) => self.device_for_letter(c),
                _ => None,
            } {
                let from = match self.view {
                    View::Work if self.device > 0 => self.device - 1,
                    _ => self.local_index(),
                };
                self.open_edge(from, d);
                return true;
            }
        }
        // `⌫` asked; a second `⌫` stops, a slot key (in a device) asks for that
        // session instead (`⌫ s ⌫`), every other key keeps the session.
        if let Some((d, session)) = self.stop_armed.take() {
            match key.code {
                KeyCode::Backspace => self.send_stop(d, session),
                KeyCode::Char(c) if plain && self.view == View::Work && SLOTS.contains(&c) => {
                    let slot = SLOTS.iter().position(|s| *s == c).unwrap_or(0);
                    if let Some(k) = self
                        .slot_sessions(d)
                        .get(slot)
                        .map(|s| workspaces::runtime_key(s))
                    {
                        self.reconcile_session_selection(Some((d, k)));
                        self.arm_stop();
                    }
                }
                _ => {}
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
        // `` ` `` `-` `+` `K` are the keymap's Global rows (BINDINGS), resolved after this,
        // also over the link editor and the invite, which own the rest of the plain keys.
        if matches!(key.code, KeyCode::Char('`' | '-' | 'K'))
            || (key.code == KeyCode::Char('+') && self.invite.is_none())
        {
            return false;
        }
        if self.edge.is_some() {
            return self.edge_key(key);
        }
        if self.invite.is_some() {
            return self.invite_key(key);
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
    fn hub_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            // `e` then a device letter: that link (see `screen_key`).
            KeyCode::Char('e') => {
                self.edge_pending = true;
                true
            }
            // The arrows and `h j k l` move on screen, in the map between nodes and in
            // the tiles between tiles (a device's letter when one has it: `gg` and `G`
            // stay first and last).
            KeyCode::Char(c @ ('h' | 'j' | 'k' | 'l')) if self.device_for_letter(c).is_none() => {
                let (dx, dy) = vim_arrow(c);
                self.arrow(dx, dy);
                true
            }
            KeyCode::Left | KeyCode::Right | KeyCode::Up | KeyCode::Down => {
                let (dx, dy) = key_arrow(key.code);
                self.arrow(dx, dy);
                true
            }
            KeyCode::Char('G') => false,
            KeyCode::Char('g') if self.device_for_letter('g').is_none() => false,
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
            KeyCode::Char('e') => {
                self.edge_pending = true;
                true
            }
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
            // Tiles, then the new-session row: Tab in order, arrows and `h j k l` on screen.
            KeyCode::Tab => {
                self.screen_move(1);
                true
            }
            KeyCode::BackTab => {
                self.screen_move(-1);
                true
            }
            KeyCode::Char(c @ ('h' | 'j' | 'k' | 'l')) => {
                let (dx, dy) = vim_arrow(c);
                self.arrow(dx, dy);
                true
            }
            KeyCode::Left | KeyCode::Right | KeyCode::Up | KeyCode::Down => {
                let (dx, dy) = key_arrow(key.code);
                self.arrow(dx, dy);
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
                self.connect_cloudflare();
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
    /// Onboarding's `⏎ Connect Cloudflare`: the token's masked entry, so the flow is
    /// `⏎`, paste, `⏎`.
    fn connect_cloudflare(&mut self) {
        self.open_secrets();
        let vm = viewmodel::ViewModel::from(&*self);
        let token = crate::secrets::CLOUDFLARE_TOKEN;
        if let Some(i) = vm.state.secrets.iter().position(|s| s.key == token) {
            self.secret_card(i);
        }
    }
    fn secrets_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Char(c) if SLOTS.contains(&c) => {
                let i = SLOTS.iter().position(|s| *s == c).unwrap_or(0);
                self.secret_card(i);
            }
            KeyCode::Enter => self.secret_card(self.secret_selected),
            KeyCode::Char('h') => self.new_secret(),
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
    /// Files' `y`: the previewed file's path, else the selected entry's.
    fn copy_path(&mut self) {
        let Some(b) = self.browser.as_ref() else {
            return;
        };
        let path = match (&b.preview, &b.preview_path) {
            (Some(_), Some(p)) => Some(p.clone()),
            _ => browser_entries(b).get(b.selected).map(|e| e.path.clone()),
        };
        // (A path that is not UTF-8 travels encoded: its text, as the screen shows it.)
        if let Some(path) = path {
            let text =
                crate::files::decode_path(&path).map_or(path, |p| p.to_string_lossy().into_owned());
            self.copy_text("the path", text);
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
        if self.secret_entry.is_some() {
            return self.secret_entry_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        }
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
        self.edge_pending = false;
        if self.close_invite() {
            self.set_notice("Invite closed".into());
        } else if self.secret_entry.take().is_some() {
        } else if self.input == Some(Input::Palette) {
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
            (Some("session" | "tile"), Some(d), Some(key)) => {
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
    /// A tap on a view target: the same method its key runs. False for an action no
    /// screen knows (every target a frame registers is known: see the tests).
    pub(super) fn view_action(&mut self, action: &str) -> bool {
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
            // A tile: a tap focuses it; a tap on the focused tile attaches (as `⏎`).
            ("tile", Some(d), Some(key)) => {
                if let Some(d) = dev_index(d) {
                    let focused = (self.view != View::Network || self.hub_sessions)
                        && self
                            .selected_session()
                            .is_some_and(|(sd, s)| sd == d && workspaces::runtime_key(&s) == key);
                    if self.view == View::Network {
                        self.hub_sessions = true;
                    }
                    if focused {
                        self.attach_selected();
                    } else {
                        self.select_session(d, key.to_string());
                    }
                }
            }
            ("new", Some(d), Some(provider)) => {
                if let Some(d) = dev_index(d) {
                    self.new_session(d, provider);
                }
            }
            ("files", Some(d), _) => {
                if let Some(d) = dev_index(d) {
                    self.open_files(d);
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
                    self.open_palette("");
                }
            }
            ("secret" | "secret-add", Some(name), _) => {
                let vm = viewmodel::ViewModel::from(&*self);
                if let Some(i) = vm.state.secrets.iter().position(|s| s.key == name) {
                    self.secret_card(i);
                }
            }
            // Footer chips and buttons: the verbs their keys run.
            ("back", None, _) => self.back_here(),
            ("hub", None, _) => self.go_hub(),
            // The top bar's device count: the map; its transfer readout: `T`.
            ("devices", None, _) => {
                self.go_hub();
                self.hub_sessions = false;
            }
            ("transfers", None, _) => self.run_cmd(keymap::Cmd::Jobs),
            ("last", None, _) => self.attach_last(),
            // Files' `/ find` chip finds as its key does (in the folder or the preview).
            ("palette" | "find", None, _) if self.view == View::Files => {
                let preview = self.browser.as_ref().is_some_and(|b| b.preview.is_some());
                self.run_cmd(if preview {
                    keymap::Cmd::PreviewSearch
                } else {
                    keymap::Cmd::Search
                });
            }
            // The ticker's `+N` and the top bar's count: `/` finds the devices they name.
            ("palette" | "find" | "notices", None, _) => self.open_palette(""),
            ("invite" | "join" | "new-code", None, _) => self.invite(),
            ("secret-new", None, _) => self.new_secret(),
            ("secrets", None, _) => self.open_secrets(),
            ("help", None, _) => self.run_cmd(keymap::Cmd::Help),
            ("zone", None, _) => self.toggle_zone(),
            ("connect" | "connect-cloudflare", None, _) => self.connect_cloudflare(),
            ("copy-code", None, _) => self.copy_invite(false),
            ("copy-cmd", None, _) => self.copy_invite(true),
            ("copy-path", None, _) => self.copy_path(),
            ("select", None, _) => self.execute(Action::Select),
            ("edit", None, _) => self.secret_card(self.secret_selected),
            ("attach" | "open" | "go", None, _) => self.primary_here(),
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
                self.edge_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::NONE));
            }
            ("lane", None, _) => self.edge_lane(),
            ("skip", None, _) => self.onboarding_dismissed = true,
            (verb @ ("new-claude" | "new-codex" | "new-shell"), None, _) => {
                if let Some(d) = current {
                    self.new_session(d, verb.trim_start_matches("new-"));
                }
            }
            _ => return false,
        }
        true
    }
}

/// `h j k l` as a direction.
fn vim_arrow(c: char) -> (i32, i32) {
    match c {
        'h' => (-1, 0),
        'l' => (1, 0),
        'k' => (0, -1),
        _ => (0, 1),
    }
}
/// An arrow key as a direction.
fn key_arrow(code: KeyCode) -> (i32, i32) {
    match code {
        KeyCode::Left => (-1, 0),
        KeyCode::Right => (1, 0),
        KeyCode::Up => (0, -1),
        _ => (0, 1),
    }
}
