//! File browser: navigation/selection state, transfer submission and drawer labels.
//! The Files screen is drawn by `views/mission/views.rs` from `viewmodel.rs`.
use super::*;

/// Visible (hidden/filter-applied) rows; cached in the browser's index, so this is
/// cheap to call per frame or per key.
pub(super) fn browser_entries(b: &Browser) -> VisibleRows {
    b.index
        .borrow_mut()
        .visible(&b.entries, b.show_hidden, &b.filter)
}
/// Search matches over the visible rows (one entry per row), cached per query.
pub(super) fn browser_matches(b: &Browser) -> file_index::RowMatches {
    b.index
        .borrow_mut()
        .search(&b.entries, b.show_hidden, &b.filter, &b.search)
}
/// How long a folder or file stays highlighted before its listing/preview is fetched.
const PREFETCH_DELAY: Duration = Duration::from_millis(150);

pub(super) fn human_size(size: u64) -> String {
    if size < 1024 {
        format!("{size} B")
    } else if size < 1024 * 1024 {
        format!("{:.1} KiB", size as f64 / 1024.0)
    } else if size < 1024 * 1024 * 1024 {
        format!("{:.1} MiB", size as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.1} GiB", size as f64 / (1024.0 * 1024.0 * 1024.0))
    }
}
pub(super) fn transfer_spinner(frame: usize, plain: bool) -> &'static str {
    let frames: &[&str] = if plain {
        &["|", "/", "-", "\\"]
    } else {
        &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]
    };
    frames[frame % frames.len()]
}
pub(super) fn transfer_name(job: &Value) -> String {
    let path = job["source_display"]
        .as_str()
        .or(job["source_path"].as_str())
        .unwrap_or("file");
    safe_label(path.rsplit('/').find(|s| !s.is_empty()).unwrap_or(path))
}
pub(super) fn transfer_status(job: &Value) -> String {
    let state = safe_label(job["status"].as_str().unwrap_or("unknown"));
    let skipped = job["skipped"].as_u64().unwrap_or(0);
    if state == "complete" && skipped > 0 {
        format!("complete · {skipped} skipped")
    } else {
        state
    }
}
pub(super) fn transfer_destination(job: &Value) -> String {
    let path = job["actual_destinations"]
        .as_array()
        .and_then(|a| a.first())
        .and_then(Value::as_str)
        .or(job["destination_display"].as_str())
        .or(job["destination_path"].as_str())
        .unwrap_or("");
    match crate::files::decode_path(path) {
        Ok(p) => safe_label(&crate::files::display(&p.to_string_lossy())),
        Err(_) => safe_label(path),
    }
}

impl App {
    pub(super) fn visible_entries(&self) -> VisibleRows {
        self.browser
            .as_ref()
            .map(browser_entries)
            .unwrap_or_else(VisibleRows::empty)
    }
    /// Debounced background work for the Files view, run every loop iteration:
    /// prefetch a folder highlighted for 150 ms, load the inline peek of a highlighted
    /// file, and keep a streaming preview fed. Returns true when a redraw is needed.
    pub(super) fn files_tick(&mut self, now: Instant) -> bool {
        if self.view != View::Files {
            return false;
        }
        self.pump_preview_stream();
        let dual = self.other_browser.is_some();
        let Some(b) = self.browser.as_mut() else {
            return false;
        };
        if b.preview.is_some() {
            return false;
        }
        let Some(entry) = browser_entries(b).get(b.selected).cloned() else {
            b.settle = None;
            return false;
        };
        match &b.settle {
            Some((path, _, _)) if *path == entry.path => {}
            _ => {
                b.settle = Some((entry.path.clone(), now, false));
                return false;
            }
        }
        let (_, since, done) = b.settle.as_ref().unwrap();
        if *done || now.duration_since(*since) < PREFETCH_DELAY {
            return false;
        }
        b.settle.as_mut().unwrap().2 = true;
        let device = b.device;
        let peek = b.peek_shown.get() && !dual;
        let mut dirty = false;
        if entry.kind == "directory" {
            let key = (
                device,
                entry.path.clone(),
                browser_scope_key(b.container.as_ref()),
            );
            let cached = self.browser_cache.get(&key).map(|c| c.entries.clone());
            let b = self.browser.as_mut().unwrap();
            let current = b.peek.as_ref().is_some_and(|p| {
                p.path == entry.path && p.preview.is_none() && !(p.loading && cached.is_some())
            });
            if peek && !current {
                let mut child = Browser::new(device, entry.path.clone());
                child.show_hidden = b.show_hidden;
                // Uncached: "loading" until the prefetch reply fills it (never "Empty folder").
                child.loading = cached.is_none();
                if let Some(entries) = &cached {
                    child.entries = entries.clone();
                }
                b.peek = Some(Box::new(child));
                dirty = true;
            }
            if cached.is_none() && !b.prefetch.contains(&entry.path) {
                let path = entry.path.clone();
                let sent = !b.loading
                    && b.prefetch.len() < 4
                    && self.send(device, Operation::List { path: path.clone() });
                if let Some(b) = self.browser.as_mut() {
                    if sent {
                        b.prefetch.insert(path);
                    } else {
                        // Listing or prefetch slots busy: ask again after another debounce.
                        b.settle = Some((path, now, false));
                    }
                }
            }
        } else if peek && entry.kind == "file" {
            let wanted = b
                .peek
                .as_ref()
                .is_none_or(|p| p.preview_path.as_ref() != Some(&entry.path));
            if wanted {
                let mut child = b
                    .peek
                    .take()
                    .filter(|p| p.preview.is_some() && p.path != entry.path)
                    .unwrap_or_else(|| Box::new(Browser::new(device, b.path.clone())));
                child.path = b.path.clone();
                child.entries = vec![entry.clone()].into();
                child.preview_path = Some(entry.path.clone());
                child.preview_pending_page = Some(1);
                child.peek_value = None;
                b.peek = Some(child);
                let generation = self.generation;
                if self.send(
                    device,
                    Operation::Preview {
                        path: entry.path.clone(),
                    },
                ) {
                    if let Some(b) = self.browser.as_mut() {
                        if b.peek_inflight.len() >= 16 {
                            b.peek_inflight.remove(0);
                        }
                        b.peek_inflight.push((entry.path.clone(), generation));
                    }
                } else if let Some(p) = self.browser.as_mut().and_then(|b| b.peek.as_mut()) {
                    p.preview_pending_page = None;
                }
                dirty = true;
            }
        }
        dirty
    }
    /// Time until `files_tick` has debounced work to do.
    pub(super) fn files_due(&self, now: Instant) -> Option<Duration> {
        if self.view != View::Files {
            return None;
        }
        let b = self.browser.as_ref()?;
        let (_, since, done) = b.settle.as_ref()?;
        (!done).then(|| PREFETCH_DELAY.saturating_sub(now.duration_since(*since)))
    }
    pub(super) fn chosen_entries(&self) -> Vec<Entry> {
        let Some(b) = &self.browser else {
            return vec![];
        };
        if b.marked.is_empty() {
            browser_entries(b)
                .get(b.selected)
                .cloned()
                .into_iter()
                .collect()
        } else {
            // Marks are path identities, independent of hidden/filter state.
            b.entries
                .iter()
                .filter(|e| b.marked.contains(&e.path))
                .cloned()
                .collect()
        }
    }
    pub(super) fn queue_file_actions(&mut self, actions: Vec<(usize, Operation)>) {
        if !self.file_busy && self.file_queue.is_empty() {
            self.file_errors.clear();
        }
        for (_, op) in &actions {
            if let Operation::TransferRetry { key } = op {
                self.watched_jobs.insert(key.clone());
            }
        }
        let actions = actions
            .into_iter()
            .map(|(d, op)| (d, self.scoped_file_operation(d, op)))
            .collect::<Vec<_>>();
        self.file_queue.extend(actions);
        self.start_next_file_action();
    }
    pub(super) fn start_next_file_action(&mut self) {
        if !self.file_busy {
            if let Some((d, op)) = self.file_queue.front().cloned() {
                if self.send_explicit(d, op) {
                    self.file_queue.pop_front();
                    self.file_busy = true;
                } else {
                    self.set_notice_as(
                        NoticeKind::Warning,
                        "Request queue busy · file action remains pending".into(),
                    );
                }
            }
        }
    }
    pub(super) fn finish_visual(&mut self) {
        if let Some(b) = &mut self.browser {
            b.visual_anchor = None;
            b.visual_base.clear();
        }
    }
    pub(super) fn live_search(&mut self) {
        if self.view == View::Files {
            if let Some(b) = &mut self.browser {
                b.search = self.text.clone();
                b.restore_selection = None;
                if !b.search.is_empty() {
                    let matches = browser_matches(b);
                    if let Some(index) = matches
                        .iter()
                        .map(|(i, _)| *i)
                        .find(|i| *i >= b.selected)
                        .or_else(|| matches.first().map(|(i, _)| *i))
                    {
                        b.selected = index;
                    }
                }
            }
        } else {
            self.search = self.text.clone();
            self.selected = 0;
            self.container_selected = 0;
        }
    }
    pub(super) fn next_file_match(&mut self, direction: isize) {
        if let Some(b) = &mut self.browser {
            let matches = browser_matches(b);
            let index = if direction > 0 {
                matches
                    .iter()
                    .find(|(i, _)| *i > b.selected)
                    .or_else(|| matches.first())
            } else {
                matches
                    .iter()
                    .rev()
                    .find(|(i, _)| *i < b.selected)
                    .or_else(|| matches.last())
            };
            if let Some((index, _)) = index {
                b.selected = *index;
                b.restore_selection = None;
            }
        }
    }
    pub(super) fn open_browser(&mut self, device: usize, path: String) {
        let scope = self
            .browser
            .as_ref()
            .filter(|b| self.view == View::Files && b.device == device)
            .and_then(|b| b.container.clone());
        self.open_scoped_browser(device, path, scope);
    }
    pub(super) fn open_host_browser(&mut self, device: usize, path: String) {
        self.open_scoped_browser(device, path, None);
    }
    pub(super) fn open_container_browser(&mut self, device: usize, scope: ContainerScope) {
        self.other_browser = None;
        self.destination_active = false;
        let path = self
            .container_locations
            .get(&(device, scope.id.clone()))
            .cloned()
            .unwrap_or_else(|| scope.folder.clone());
        self.open_scoped_browser(device, path, Some(scope));
    }
    pub(super) fn open_scoped_browser(
        &mut self,
        device: usize,
        path: String,
        scope: Option<ContainerScope>,
    ) {
        self.device = device + 1;
        let show_hidden = self
            .browser
            .as_ref()
            .or(self.other_browser.as_ref())
            .is_some_and(|b| b.show_hidden);
        self.finish_visual();
        self.generation += 1;
        // The settled selection already asked for this folder: its reply is the listing.
        let prefetched = self
            .browser
            .as_ref()
            .is_some_and(|b| b.device == device && scope.is_none() && b.prefetch.contains(&path));
        if let Some(mut old) = self.browser.take() {
            old.preview = None;
            old.preview_rich = None;
            old.preview_history.clear();
            old.preview_restore = None;
            old.preview_anchor = None;
            old.preview_link_cells.borrow_mut().clear();
            if let Some(scope) = &old.container {
                self.container_locations
                    .insert((old.device, scope.id.clone()), old.path.clone());
            } else {
                self.file_locations.insert(old.device, old.path.clone());
            }
            if self.browser_cache.len() >= 64 {
                if let Some(key) = self.browser_cache.keys().next().cloned() {
                    self.browser_cache.remove(&key);
                }
            }
            self.browser_cache.insert(
                (
                    old.device,
                    old.path.clone(),
                    browser_scope_key(old.container.as_ref()),
                ),
                old,
            );
        }
        self.browser = Some(
            self.browser_cache
                .remove(&(device, path.clone(), browser_scope_key(scope.as_ref())))
                .unwrap_or_else(|| Browser::new(device, path)),
        );
        if let Some(b) = &mut self.browser {
            b.show_hidden = show_hidden;
            b.container = scope;
        }
        self.view = View::Files;
        self.focus = Focus::Workspace;
        match self.browser.as_mut().filter(|_| prefetched) {
            Some(b) => {
                b.loading = true;
                b.prefetch.insert(b.path.clone());
            }
            None => self.refresh_browser(),
        }
        self.check_providers(device);
    }
    pub(super) fn select_file_device(&mut self) {
        // Files always has one execution host; All devices returns to the local location.
        let device = self
            .device
            .checked_sub(1)
            .or_else(|| self.devices.iter().position(|d| d.target.is_none()));
        if let Some(device) = device {
            if self
                .browser
                .as_ref()
                .is_none_or(|b| b.device != device || b.container.is_some())
            {
                let path = self
                    .file_locations
                    .get(&device)
                    .cloned()
                    .unwrap_or_else(|| "~".into());
                self.open_host_browser(device, path);
            }
        }
    }
    pub(super) fn refresh_browser(&mut self) {
        self.generation += 1;
        if let Some(b) = self.browser.as_mut() {
            if b.restore_selection.is_none() {
                b.restore_selection = browser_entries(b)
                    .get(b.selected)
                    .map(|entry| entry.path.clone());
            }
            b.loading = true;
            b.preview = None;
            b.preview_rich = None;
            b.preview_history.clear();
            b.preview_restore = None;
            b.preview_anchor = None;
            b.preview_link_cells.borrow_mut().clear();
            let device = b.device;
            let path = b.path.clone();
            if !self.send(device, Operation::List { path }) {
                if let Some(b) = &mut self.browser {
                    b.loading = false;
                }
                self.set_notice_as(
                    NoticeKind::Warning,
                    "Refresh queue busy · retry shortly".into(),
                );
            }
        }
    }
    pub(super) fn conflict_policy(&self) -> &'static str {
        ["skip", "overwrite", "rename"][self.conflict]
    }
    pub(super) fn active_transfer_keys(&self) -> BTreeSet<String> {
        let mut keys = self.pending_transfers.clone();
        for (_, job) in self.job_rows() {
            if matches!(
                job["status"].as_str(),
                Some("queued" | "running" | "submitting")
            ) {
                if let Some(key) = job["key"].as_str() {
                    keys.insert(key.into());
                }
            }
        }
        keys
    }
    pub(super) fn submit_transfer(&mut self) {
        let (Some(clip), Some(b)) = (self.clipboard.clone(), self.browser.as_ref()) else {
            return;
        };
        let Some(local) = self.devices.iter().position(|d| d.target.is_none()) else {
            return;
        };
        let destination = self.devices[b.device].clone();
        let destination_path = b.path.clone();
        let destination_container = b.container.clone();
        let active = self.active_transfer_keys();
        if self.submitted.iter().any(|(key, spec)| {
            active.contains(key)
                && spec.source.id == self.devices[clip.device].id
                && spec.destination.id == destination.id
                && spec.source_container == clip.container
                && spec.destination_container == destination_container
                && spec.destination_path == destination_path
                && clip.entries.iter().any(|e| e.path == spec.source_path)
        }) {
            self.set_notice_as(
                NoticeKind::Warning,
                "Transfer already pending · T shows progress".into(),
            );
            return;
        }
        let mut actions = Vec::new();
        for entry in &clip.entries {
            let spec = crate::model::TransferSpec {
                source_container: clip.container.clone(),
                destination_container: destination_container.clone(),
                source: self.devices[clip.device].clone(),
                source_path: entry.path.clone(),
                destination: destination.clone(),
                destination_path: destination_path.clone(),
                conflict: self.conflict_policy().into(),
                key: unique_key(),
                cut: clip.cut,
                source_identity: if clip.cut {
                    entry.identity.clone()
                } else {
                    None
                },
            };
            if self.submitted.len() >= 256 {
                if let Some(key) = self.submitted.keys().next().cloned() {
                    self.submitted.remove(&key);
                }
            }
            self.pending_transfers.insert(spec.key.clone());
            self.submitted.insert(spec.key.clone(), spec.clone());
            if clip.cut {
                self.submitted_clipboards
                    .insert(spec.key.clone(), clip.id.clone());
            }
            actions.push((local, spec.operation()));
        }
        self.set_notice(format!(
            "{} {} items · {} → {} · existing: {}",
            if clip.cut { "Moving" } else { "Copying" },
            clip.entries.len(),
            file_endpoint_label(&self.devices[clip.device], &clip.container),
            file_endpoint_label(&destination, &destination_container),
            self.conflict_policy()
        ));
        self.queue_file_actions(actions);
        self.transfer_drawer = true;
    }
    pub(super) fn job_rows(&self) -> Vec<(usize, Value)> {
        let mut unique = HashMap::new();
        for (d, value) in &self.jobs {
            if let Some(jobs) = value["jobs"].as_array() {
                for job in jobs {
                    let key = (
                        job["key"].as_str().unwrap_or("").to_string(),
                        job["source_host"].as_str().unwrap_or("").to_string(),
                        job["destination_host"].as_str().unwrap_or("").to_string(),
                    );
                    let row = unique.entry(key).or_insert((*d, job.clone()));
                    // Prefer the viewer's durable forwarding reference, which knows job ownership.
                    if (self.devices[*d].target.is_none() && self.devices[row.0].target.is_some())
                        || (self.devices[*d].target.is_none()
                            == self.devices[row.0].target.is_none()
                            && *d < row.0)
                    {
                        *row = (*d, job.clone());
                    }
                }
            }
        }
        let mut rows = unique
            .into_values()
            .filter(|(_, job)| {
                fuzzy_score(
                    &self.job_filter,
                    &format!(
                        "{} {} {} {} {} {}",
                        transfer_name(job),
                        transfer_status(job),
                        job["source_host"].as_str().unwrap_or(""),
                        job["destination_host"].as_str().unwrap_or(""),
                        job["source_display"]
                            .as_str()
                            .or(job["source_path"].as_str())
                            .unwrap_or(""),
                        transfer_destination(job)
                    ),
                )
                .is_some()
            })
            .collect::<Vec<_>>();
        rows.sort_by(|(a, x), (b, y)| {
            y["updated"]
                .as_u64()
                .cmp(&x["updated"].as_u64())
                .then_with(|| a.cmp(b))
                .then_with(|| x["key"].as_str().cmp(&y["key"].as_str()))
        });
        rows
    }
    pub(super) fn parent_directory(&mut self) {
        if let Some(b) = &self.browser {
            if let Some(path) = b.parent.clone() {
                self.open_browser(b.device, path);
            }
        }
    }
}
