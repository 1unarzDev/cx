//! Session navigation uses runtime identity, never a title or a shared folder.
use super::*;

pub(super) fn same_scope(a: Option<&ContainerScope>, b: Option<&ContainerScope>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => {
            a.engine == b.engine && a.id == b.id && a.started_at == b.started_at && a.user == b.user
        }
        _ => false,
    }
}
pub(super) fn runtime_key(s: &Session) -> String {
    if let Some(t) = &s.terminal {
        return serde_json::to_string(&(
            t,
            &s.boot_id,
            s.container
                .as_ref()
                .map(|c| (&c.engine, &c.id, &c.started_at, &c.user)),
        ))
        .unwrap_or_default();
    }
    serde_json::to_string(&(
        &s.id,
        &s.socket,
        s.pid,
        &s.started,
        &s.boot_id,
        s.container
            .as_ref()
            .map(|c| (&c.engine, &c.id, &c.started_at, &c.user)),
    ))
    .unwrap_or_default()
}
/// Sessions whose attach is identity-checked on the execution host attach
/// directly. Legacy rows without terminal identity keep the verification
/// round trip, because tmux reuses `$N` and names can be recreated.
pub(super) fn optimistic_attach(s: &Session) -> bool {
    s.terminal.is_some() && !(s.external && s.origin.is_some())
}
pub(super) fn device_matches(a: &Device, b: &Device) -> bool {
    a.id == b.id && a.target == b.target && a.account == b.account && a.host == b.host
}

impl App {
    pub(super) fn pin_matches(&self, pin: &store::SessionPin, d: usize, s: &Session) -> bool {
        device_matches(&pin.device, &self.devices[d]) && runtime_key(&pin.session) == runtime_key(s)
    }
    pub(super) fn pinned(&self, d: usize, s: &Session) -> bool {
        self.pins.iter().any(|p| self.pin_matches(p, d, s))
    }
    pub(super) fn observed(&self, d: usize, s: &Session) -> bool {
        {
            let key = runtime_key(s);
            self.work[d]
                .sessions
                .iter()
                .any(|current| std::ptr::eq(current, s) || runtime_key(current) == key)
        }
    }
    pub(super) fn session_status(&self, d: usize, s: &Session) -> &'static str {
        let w = &self.work[d];
        if w.error.is_some() {
            "? stale"
        } else if w.fetched == 0 || transport::now().saturating_sub(w.fetched) > 60 {
            "? cached"
        } else if !self.observed(d, s) {
            "not seen"
        } else if s.external {
            "+ ext"
        } else {
            "+ live"
        }
    }
    pub(super) fn toggle_pin(&mut self) {
        let Some((d, s)) = self.selected_session() else {
            return;
        };
        let remove = self.pinned(d, &s);
        let pin = store::SessionPin {
            device: self.devices[d].clone(),
            session: s.clone(),
            pinned_at: transport::now(),
        };
        match store::change_session_pin(&pin, remove, |p| self.pin_matches(p, d, &s)) {
            Ok(pins) => {
                self.pins = pins;
                self.selected = self
                    .session_rows()
                    .iter()
                    .position(|(i, row)| *i == d && runtime_key(row) == runtime_key(&s))
                    .unwrap_or(0);
                self.set_notice(
                    if remove {
                        "Session unpinned"
                    } else {
                        "Pinned · kept after this terminal ends"
                    }
                    .into(),
                );
            }
            Err(e) => self.set_notice_as(NoticeKind::Error, format!("Could not save pin: {e}")),
        }
    }
    pub(super) fn open_session(&mut self, d: usize, s: Session, watch: bool) {
        if s.external && s.origin.is_some() {
            if watch && outside_tmux(&s) {
                self.set_notice(
                    "This terminal is outside tmux; press Enter to move it into CX".into(),
                );
                return;
            } else if !watch {
                self.adopt_external_session(d, s);
                return;
            }
        }
        self.check_session(d, s, watch);
    }
    pub(super) fn check_session(&mut self, d: usize, s: Session, watch: bool) {
        if !self.observed(d, &s)
            && self.work[d].fetched > 0
            && self.work[d].error.is_none()
            && transport::now().saturating_sub(self.work[d].fetched) <= 60
        {
            self.dialog = Some(Dialog::PinnedEnded(d, s));
            self.dialog_selected = 0;
            return;
        }
        if self.attach_check.is_some() {
            return;
        }
        // Only a row the device confirmed in a successful refresh within the
        // last minute attaches directly. Errored, offline or startup-cache rows
        // keep the round trip, so an unreachable device fails inside cx with a
        // notice instead of suspending the viewer into a hanging ssh.
        if optimistic_attach(&s) && self.session_status(d, &s).starts_with('+') {
            // The checked attach re-verifies server, session and creation
            // identity atomically on the execution host, so a click-time
            // round trip adds latency without adding safety. A refusal
            // refreshes the list and reports it (runtime pending_attach).
            self.pending_attach = Some((d, s, watch));
            return;
        }
        self.generation += 1;
        if self.send_explicit(d, Operation::Sessions) {
            self.work[d].loading = true;
            self.attach_check = Some((d, s, watch, self.generation));
            self.set_notice("Checking the selected terminal…".into());
        } else {
            self.set_notice_as(
                NoticeKind::Warning,
                "Refresh queue busy · attachment was not started".into(),
            );
        }
    }
    pub(super) fn scoped_sessions(
        &self,
        d: usize,
        path: &str,
        scope: Option<&ContainerScope>,
        provider: &str,
    ) -> Vec<Session> {
        let mut seen = BTreeSet::new();
        self.work[d]
            .sessions
            .iter()
            .filter(|s| {
                same_scope(s.container.as_ref(), scope)
                    && s.directory == path
                    && (provider.is_empty() || s.provider == provider)
                    && seen.insert(runtime_key(s))
            })
            .cloned()
            .collect()
    }
    pub(super) fn sessions_here(&mut self) {
        let Some(b) = self.browser.as_ref() else {
            return;
        };
        let path = browser_entries(b)
            .get(b.selected)
            .filter(|e| e.kind == "directory")
            .map(|e| e.path.clone())
            .unwrap_or_else(|| b.path.clone());
        let d = b.device;
        let mut scope = b.container.clone();
        if let Some(c) = scope.as_mut() {
            c.folder = path.clone();
        }
        let rows = self.scoped_sessions(d, &path, scope.as_ref(), "");
        self.dialog = Some(Dialog::SessionPicker(d, path, scope, String::new(), rows));
        self.dialog_selected = 0;
        self.refresh_sessions_on(d);
    }
    pub(super) fn open_container_sessions(&mut self, d: usize, scope: ContainerScope) {
        let mut seen = BTreeSet::new();
        let rows = self.work[d]
            .sessions
            .iter()
            .filter(|s| {
                same_scope(s.container.as_ref(), Some(&scope)) && seen.insert(runtime_key(s))
            })
            .cloned()
            .collect();
        self.dialog = Some(Dialog::SessionPicker(
            d,
            String::new(),
            Some(scope),
            String::new(),
            rows,
        ));
        self.dialog_selected = 0;
        self.refresh_sessions_on(d);
    }
    pub(super) fn refresh_sessions_on(&mut self, d: usize) {
        if !self.work[d].loading {
            self.work[d].loading = self.send_explicit(d, Operation::Sessions);
        }
    }
    pub(super) fn reconcile_picker(&mut self, d: usize) {
        let Some(Dialog::SessionPicker(device, path, scope, provider, prior)) = self.dialog.clone()
        else {
            return;
        };
        if device != d {
            return;
        }
        let selected = prior.get(self.dialog_selected).cloned();
        let action_offset = self.dialog_selected.saturating_sub(prior.len());
        let mut rows = if let Some(scope) = scope.as_ref() {
            self.work[d]
                .sessions
                .iter()
                .filter(|s| {
                    same_scope(s.container.as_ref(), Some(scope))
                        && (path.is_empty() || s.directory == path)
                        && (provider.is_empty() || s.provider == provider)
                })
                .cloned()
                .collect::<Vec<_>>()
        } else {
            self.scoped_sessions(d, &path, None, &provider)
        };
        let mut seen = BTreeSet::new();
        rows.retain(|s| seen.insert(runtime_key(s)));
        self.dialog_selected = if let Some(selected) = selected {
            if let Some(index) = rows
                .iter()
                .position(|s| runtime_key(s) == runtime_key(&selected))
            {
                index
            } else {
                rows.push(selected);
                rows.len() - 1
            }
        } else {
            rows.len()
                + action_offset.min(usize::from(container_agents::has_history(
                    scope.as_ref(),
                    &provider,
                )))
        };
        self.dialog = Some(Dialog::SessionPicker(device, path, scope, provider, rows));
    }
    pub(super) fn folder_agents(&self, b: &Browser) -> BTreeMap<String, String> {
        let mut counts: BTreeMap<String, (usize, usize)> = BTreeMap::new();
        let mut seen = BTreeSet::new();
        for s in &self.work[b.device].sessions {
            if !same_scope(s.container.as_ref(), b.container.as_ref())
                || !seen.insert(runtime_key(s))
            {
                continue;
            }
            if s.provider == "claude" || s.provider == "codex" {
                let count = counts.entry(s.directory.clone()).or_default();
                if s.provider == "claude" {
                    count.0 += 1;
                } else {
                    count.1 += 1;
                }
            }
        }
        let w = &self.work[b.device];
        let stale =
            w.fetched == 0 || w.error.is_some() || transport::now().saturating_sub(w.fetched) > 60;
        counts
            .into_iter()
            .map(|(path, (claude, codex))| {
                let mut parts = Vec::new();
                if claude > 0 {
                    parts.push(format!("Claude {claude}"));
                }
                if codex > 0 {
                    parts.push(format!("Codex {codex}"));
                }
                (
                    path,
                    format!("{}{}", parts.join(" · "), if stale { " ?" } else { "" }),
                )
            })
            .collect()
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct NavigationCache {
    version: u32,
    devices: Vec<DeviceCache>,
}
#[derive(serde::Serialize, serde::Deserialize)]
struct DeviceCache {
    device: Device,
    sessions: Vec<Session>,
    browsers: Vec<Browser>,
}
impl NavigationCache {
    pub(super) fn snapshot(app: &App) -> Self {
        let devices = app
            .devices
            .iter()
            .enumerate()
            .map(|(d, device)| {
                let mut seen = BTreeSet::new();
                let browsers = app
                    .browser
                    .iter()
                    .chain(app.other_browser.iter())
                    .chain(app.browser_cache.values())
                    .filter(|b| {
                        b.device == d
                            && b.entries.len() <= 2048
                            && (!b.loading || !b.entries.is_empty())
                    })
                    .filter(|b| {
                        seen.insert((b.path.clone(), browser_scope_key(b.container.as_ref())))
                    })
                    .take(24)
                    .map(|b| {
                        // Retain only navigation, never preview contents, clipboard or selected mutation state.
                        let mut clean = Browser::new(d, b.path.clone());
                        clean.container = b.container.clone();
                        clean.display_path = b.display_path.clone();
                        clean.parent = b.parent.clone();
                        clean.entries = b.entries.clone();
                        clean.selected = b.selected;
                        clean.restore_selection =
                            browser_entries(b).get(b.selected).map(|e| e.path.clone());
                        clean
                    })
                    .collect();
                DeviceCache {
                    device: device.clone(),
                    sessions: app.work[d]
                        .sessions
                        .iter()
                        .take(512)
                        .cloned()
                        .map(|mut s| {
                            s.activity = None;
                            s.idle_seconds = None;
                            s.agent_tasks.clear();
                            s
                        })
                        .collect(),
                    browsers,
                }
            })
            .take(128)
            .collect();
        Self {
            version: 1,
            devices,
        }
    }
    pub(super) fn restore(self, app: &mut App) {
        if self.version != 1 || self.devices.len() > 128 {
            return;
        }
        for cached in self.devices {
            let Some(d) = app
                .devices
                .iter()
                .position(|d| device_matches(d, &cached.device))
            else {
                continue;
            };
            if cached.sessions.len() <= 512 {
                app.work[d].sessions = cached.sessions;
            }
            for mut b in cached.browsers.into_iter().take(24) {
                if b.entries.len() > 2048 {
                    continue;
                }
                b.device = d;
                b.marked.clear();
                b.loading = true;
                // Persisted identities are stale hints; mutations require a fresh listing.
                for entry in b.entries.iter_mut() {
                    entry.identity = None;
                }
                app.browser_cache.insert(
                    (d, b.path.clone(), browser_scope_key(b.container.as_ref())),
                    b,
                );
            }
        }
    }
}

/// Cache writes are coalesced and kept off the input/render thread.
pub(super) struct CacheWriter {
    tx: mpsc::SyncSender<NavigationCache>,
    worker: thread::JoinHandle<()>,
}
impl CacheWriter {
    pub(super) fn save(&self, snapshot: NavigationCache) {
        let _ = self.tx.try_send(snapshot);
    }
    pub(super) fn finish(self, snapshot: NavigationCache) {
        let _ = self.tx.send(snapshot);
        drop(self.tx);
        let _ = self.worker.join();
    }
}
pub(super) fn cache_writer() -> CacheWriter {
    let (tx, rx) = mpsc::sync_channel::<NavigationCache>(1);
    let worker = thread::spawn(move || {
        while let Ok(mut snapshot) = rx.recv() {
            while let Ok(newer) = rx.try_recv() {
                snapshot = newer;
            }
            let _ = store::write_state("ui-navigation.json", &snapshot, 2 * 1024 * 1024);
        }
    });
    CacheWriter { tx, worker }
}

pub(super) fn navigation_read(op: &Operation) -> bool {
    match op {
        Operation::List { .. }
        | Operation::ListPage { .. }
        | Operation::Preview { .. }
        | Operation::PreviewPage { .. } => true,
        Operation::ContainerFiles { operation, .. } => navigation_read(operation),
        _ => false,
    }
}

/// Replies already reject old navigation generations. Discard those reads before
/// they consume helper time, retaining mutation ordering and reply accounting.
pub(super) fn superseded_reads(tasks: &mut VecDeque<Task>, incoming: &Task) -> Vec<Task> {
    let preview = matches!(incoming.op, Operation::SessionPreview { .. });
    if !preview && !navigation_read(&incoming.op) {
        return Vec::new();
    }
    let mut discarded = Vec::new();
    let mut retained = VecDeque::new();
    while let Some(task) = tasks.pop_front() {
        let same_kind = if preview {
            matches!(task.op, Operation::SessionPreview { .. })
        } else {
            navigation_read(&task.op)
        };
        if task.generation < incoming.generation && same_kind {
            discarded.push(task);
        } else {
            retained.push_back(task);
        }
    }
    *tasks = retained;
    discarded
}
