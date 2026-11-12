//! Task replies for listings, previews, transfers and file mutations.
use super::*;

/// Listing pages the UI asks for once the helper reports its sorted `order`
/// (older helpers reject limits above 1000).
const ORDERED_PAGE_LIMIT: u32 = 4000;
/// Entries kept per directory; helpers stop paging at offset 100000.
pub(in crate::ui) const LISTING_CAP: usize = 100_000;
/// The helper's sort order: directories first, then display name.
const HELPER_ORDER: &str = "directories-first-name";

/// Unix seconds from the helper's `modified`, else from the identity it always sends.
fn entry_modified(e: &Value) -> Option<i64> {
    if let Some(seconds) = e.get("modified").and_then(Value::as_i64) {
        return Some(seconds);
    }
    let identity = e.get("identity")?.as_str()?;
    let at = identity.find("\"modified\":")? + "\"modified\":".len();
    let digits: String = identity[at..]
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    let nanos: u128 = digits.parse().ok()?;
    i64::try_from(nanos / 1_000_000_000).ok()
}
pub(in crate::ui) fn parse_entries(value: &Value) -> Vec<Entry> {
    value
        .get("entries")
        .and_then(Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .filter_map(|e| {
                    Some(Entry {
                        name: e.get("name")?.as_str()?.into(),
                        path: e.get("path")?.as_str()?.into(),
                        kind: e.get("kind")?.as_str()?.into(),
                        size: e.get("size").and_then(Value::as_u64).unwrap_or(0),
                        identity: e.get("identity").and_then(Value::as_str).map(str::to_owned),
                        hidden: e.get("hidden").and_then(Value::as_bool).unwrap_or_else(|| {
                            e["name"].as_str().is_some_and(|s| s.starts_with('.'))
                        }),
                        rename_name: e
                            .get("rename_name")
                            .and_then(Value::as_str)
                            .map(str::to_owned),
                        modified: entry_modified(e),
                        items: e.get("items").and_then(Value::as_u64),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}
fn apply_location(b: &mut Browser, value: &Value) {
    b.path = value
        .get("path")
        .and_then(Value::as_str)
        .unwrap_or(&b.path)
        .to_owned();
    b.display_path = value
        .get("display_path")
        .and_then(Value::as_str)
        .unwrap_or(&b.path)
        .to_owned();
    b.parent = value
        .get("parent")
        .and_then(Value::as_str)
        .map(str::to_owned);
}

pub(in crate::ui) enum PreviewApplied {
    Stale,
    NextPage,
    Shown { missing_anchor: bool },
}
/// Install a Preview/PreviewPage reply into `b` (the browser or its inline peek).
pub(in crate::ui) fn apply_preview(
    b: &mut Browser,
    path: &str,
    page: u32,
    preview: Option<RichPreview>,
    value: &Value,
) -> PreviewApplied {
    // Only a preview that was asked for (open_preview / peek set preview_path) applies.
    if b.preview_path.as_deref() != Some(path) {
        return PreviewApplied::Stale;
    }
    b.preview_path = Some(path.to_owned());
    b.preview_pending_page = None;
    if b.preview_requested_page == 0 {
        b.preview_requested_page = page;
    }
    if b.preview_requested_page != page && b.preview_rich.is_some() {
        return PreviewApplied::NextPage;
    }
    b.preview_requested_page = page;
    b.preview_scroll.set(0);
    b.preview_revision = b.preview_revision.wrapping_add(1);
    let rich = preview.unwrap_or_else(|| RichPreview::from_value(value));
    let malformed = value["image"].is_object() && rich.raster.is_none();
    b.preview = Some(if malformed {
        "Image preview unavailable · invalid bitmap response".into()
    } else {
        safe_text(value["text"].as_str().unwrap_or("Preview unavailable"))
    });
    b.preview_rich = Some(rich);
    let restored = b.preview_restore.take();
    let restoring = restored.is_some();
    if let Some((scroll, find)) = restored {
        b.preview_scroll.set(scroll);
        b.preview_find = find;
    }
    b.preview_find.matches = doc_matches(b, &b.preview_find.query.clone());
    b.preview_find.selected = b
        .preview_find
        .selected
        .min(b.preview_find.matches.len().saturating_sub(1));
    b.preview_find.reveal.set(!restoring);
    let mut missing_anchor = false;
    if let Some(anchor) = b.preview_anchor.take() {
        if let Some(scroll) = markdown_anchor_scroll(b, &anchor) {
            b.preview_scroll.set(scroll);
            b.preview_find.reveal.set(false);
        } else {
            missing_anchor = true;
        }
    }
    PreviewApplied::Shown { missing_anchor }
}

impl App {
    pub(in crate::ui) fn apply_file_reply(
        &mut self,
        reply: Reply,
        value: Value,
        listing_offset: u64,
    ) {
        match reply.op {
            Operation::Jobs | Operation::TransferJobs => {
                if let Some(rows) = value["jobs"].as_array() {
                    for job in rows {
                        if let Some(key) = job["key"].as_str() {
                            self.pending_transfers.remove(key);
                        }
                    }
                }
                let newly_finished = value["jobs"]
                    .as_array()
                    .map(|rows| {
                        rows.iter()
                            .filter(|j| {
                                j["key"].as_str().is_some_and(|k| {
                                    self.submitted.contains_key(k) || self.watched_jobs.contains(k)
                                }) && matches!(
                                    j["status"].as_str(),
                                    Some("complete" | "failed" | "cancelled")
                                ) && !self
                                    .jobs
                                    .get(&reply.device)
                                    .and_then(|v| v["jobs"].as_array())
                                    .is_some_and(|old| {
                                        old.iter().any(|x| {
                                            x["key"] == j["key"] && x["status"] == j["status"]
                                        })
                                    })
                            })
                            .cloned()
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                let refresh_files = self.view == View::Files
                    && self.browser.as_ref().is_some_and(|b| {
                        let host = identity(&self.devices[b.device]);
                        value["jobs"].as_array().is_some_and(|rows| {
                            rows.iter().any(|j| {
                                let completed = j["status"] == "complete";
                                let affects_location = j["destination_host"].as_str()
                                    == Some(host.as_str())
                                    || (j["operation"] == "move"
                                        && j["source_host"].as_str() == Some(host.as_str()));
                                let already_known = self
                                    .jobs
                                    .get(&reply.device)
                                    .and_then(|v| v["jobs"].as_array())
                                    .is_some_and(|old| {
                                        old.iter().any(|x| {
                                            x["key"] == j["key"] && x["status"] == "complete"
                                        })
                                    });
                                completed && affects_location && !already_known
                            })
                        })
                    });
                let selected = if matches!(self.dialog, Some(Dialog::Jobs)) {
                    self.job_rows()
                        .get(self.dialog_selected)
                        .map(|(owner, job)| (*owner, job["key"].clone()))
                } else {
                    None
                };
                self.jobs.insert(reply.device, value);
                if let Some(job) = newly_finished
                    .last()
                    .filter(|j| self.file_errors.is_empty() || j["status"] == "failed")
                {
                    self.set_notice_as(
                        match job["status"].as_str() {
                            Some("complete") => NoticeKind::Success,
                            Some("failed") => NoticeKind::Error,
                            Some("cancelled") => NoticeKind::Warning,
                            _ => NoticeKind::Info,
                        },
                        format!(
                            "{} {} · {}{}",
                            if job["operation"] == "move" {
                                "Move"
                            } else {
                                "Copy"
                            },
                            transfer_status(job),
                            transfer_name(job),
                            job["error"]
                                .as_str()
                                .map(|e| format!(" · {}", safe_label(e)))
                                .unwrap_or_default()
                        ),
                    );
                }
                if let Some((owner, key)) = selected {
                    self.dialog_selected = self
                        .job_rows()
                        .iter()
                        .position(|(d, job)| *d == owner && job["key"] == key)
                        .unwrap_or(0);
                }
                if refresh_files {
                    self.browser_cache.clear();
                    self.refresh_browser();
                }
            }
            Operation::TransferCancel { .. } => {
                self.send(reply.device, Operation::TransferJobs);
                self.set_notice("Cancellation requested · waiting for worker".into());
            }
            Operation::Transfer(_)
            | Operation::ScopedTransfer(_)
            | Operation::TransferRetry { .. } => {
                if self.file_errors.is_empty() {
                    self.set_notice(format!(
                        "Transfer {} · {}",
                        safe_label(value["status"].as_str().unwrap_or("queued")),
                        safe_label(value["route"].as_str().unwrap_or("worker host"))
                    ));
                } else {
                    self.set_notice_as(
                        NoticeKind::Error,
                        format!(
                            "{} file actions failed · {}",
                            self.file_errors.len(),
                            self.file_errors.last().unwrap()
                        ),
                    );
                }
                self.send(reply.device, Operation::TransferJobs);
            }
            Operation::Copy { .. } => {
                self.set_notice(format!(
                    "Copy {} on {}",
                    value
                        .get("status")
                        .and_then(Value::as_str)
                        .unwrap_or("status unknown"),
                    identity(&self.devices[reply.device])
                ));
                self.send(reply.device, Operation::Jobs);
            }
            Operation::List { .. } | Operation::ListPage { .. }
                if reply.generation == self.generation =>
            {
                self.apply_listing(reply.device, &value, listing_offset)
            }
            Operation::Preview { ref path } | Operation::PreviewPage { ref path, .. }
                if reply.generation == self.generation
                    && self
                        .browser
                        .as_ref()
                        .is_some_and(|b| b.device == reply.device) =>
            {
                let page = match reply.op {
                    Operation::PreviewPage { page, .. } => page,
                    _ => 1,
                };
                let path = path.clone();
                let Some(b) = self.browser.as_mut() else {
                    return;
                };
                match apply_preview(b, &path, page, reply.preview, &value) {
                    PreviewApplied::Stale => {}
                    PreviewApplied::NextPage => self.start_pdf_page_request(),
                    PreviewApplied::Shown { missing_anchor } => {
                        if missing_anchor {
                            self.set_notice_as(
                                NoticeKind::Warning,
                                "Linked file opened · heading not found".into(),
                            );
                        }
                        self.start_preview_stream(&value);
                    }
                }
            }
            Operation::Mkdir { .. } if reply.generation == self.generation => {
                self.refresh_browser()
            }
            Operation::Rename { path, .. } | Operation::Remove { path, .. } => {
                if self.file_errors.is_empty() && self.file_queue.is_empty() && !self.file_busy {
                    self.set_notice(format!(
                        "File action complete · {}",
                        identity(&self.devices[reply.device])
                    ));
                }
                self.browser_cache.retain(|(d, _, _), _| *d != reply.device);
                for b in [&mut self.browser, &mut self.other_browser]
                    .into_iter()
                    .flatten()
                {
                    if b.device == reply.device {
                        b.marked.remove(&path);
                        b.entries.retain(|e| e.path != path);
                        b.visual_anchor = None;
                    }
                }
                if self
                    .browser
                    .as_ref()
                    .is_some_and(|b| b.device == reply.device)
                {
                    self.refresh_browser();
                }
            }
            _ => {}
        }
    }
    /// Merge one listing page into the current browser. Pages in the helper's order are
    /// appended without re-sorting; derived caches extend incrementally.
    fn apply_listing(&mut self, device: usize, value: &Value, listing_offset: u64) {
        let ordered = value["order"].as_str() == Some(HELPER_ORDER);
        if let Some(b) = self.browser.as_mut().filter(|b| b.device == device) {
            apply_location(b, value);
            let entries = parse_entries(value);
            b.total = value["total"].as_u64();
            let range_finished = b.visual_anchor.take().is_some();
            if range_finished {
                b.visual_base.clear();
            }
            if b.restore_selection.is_none() && listing_offset > 0 {
                b.restore_selection = browser_entries(b)
                    .get(b.selected)
                    .map(|entry| entry.path.clone());
            }
            if listing_offset == 0 {
                b.entries = entries.into();
            } else {
                b.entries.append(entries);
            }
            if !ordered {
                b.entries.sort_by(|a, b| {
                    (a.kind != "directory", a.name.as_str())
                        .cmp(&(b.kind != "directory", b.name.as_str()))
                });
            }
            let last_page = value["next_offset"].is_null();
            if let Some(path) = &b.restore_selection {
                b.selected = browser_entries(b).position_of(path).unwrap_or(0);
                if last_page {
                    b.restore_selection = None;
                }
            } else {
                b.selected = b.selected.min(browser_entries(b).len().saturating_sub(1));
            }
            b.loading = false;
            if last_page && !b.marked.is_empty() {
                let existing: std::collections::HashSet<_> =
                    b.entries.iter().map(|e| e.path.as_str()).collect();
                let marked = std::mem::take(&mut b.marked);
                b.marked = marked
                    .into_iter()
                    .filter(|p| existing.contains(p.as_str()))
                    .collect();
            }
            if range_finished {
                self.set_notice("Directory refreshed · range finished; selected files kept".into());
            }
        }
        if let Some(offset) = value["next_offset"].as_u64() {
            if let Some(b) = &self.browser {
                if b.entries.len() < LISTING_CAP && offset <= 100_000 {
                    self.send(
                        b.device,
                        Operation::ListPage {
                            path: b.path.clone(),
                            offset,
                            limit: if ordered { ORDERED_PAGE_LIMIT } else { 1000 },
                        },
                    );
                    if let Some(b) = &mut self.browser {
                        b.loading = true;
                    }
                } else {
                    self.set_notice(format!(
                        "{} entries loaded · remaining entries require a narrower directory",
                        b.entries.len()
                    ));
                }
            }
        }
    }
    /// Replies that belong to the Files view's background work rather than the visible
    /// listing: folder prefetch, the inline peek and streamed preview chunks. Returns
    /// true when the reply was consumed (including silently dropped failures).
    pub(in crate::ui) fn apply_files_side_reply(&mut self, reply: &mut Reply) -> bool {
        match &reply.op {
            Operation::List { path } => {
                let Some(b) = self
                    .browser
                    .as_mut()
                    .filter(|b| b.device == reply.device && b.prefetch.contains(path))
                else {
                    return false;
                };
                let path = path.clone();
                b.prefetch.remove(&path);
                if b.path == path {
                    // Opened while its prefetch was in flight: the prefetch is the listing
                    // (read again if it failed or was dropped as superseded).
                    match &reply.result {
                        Ok(value) if !value.is_null() && b.loading => {
                            let value = value.clone();
                            self.apply_listing(reply.device, &value, 0);
                        }
                        _ if b.loading => self.refresh_browser(),
                        _ => {}
                    }
                    return true;
                }
                if reply.generation != self.generation {
                    // Outdated or cancelled (`Ok(Null)` from a superseded read): a peek
                    // still waiting on this folder asks again after the debounce.
                    if b.peek.as_ref().is_some_and(|p| p.path == path && p.loading) {
                        if let Some(settle) = b.settle.as_mut().filter(|s| s.0 == path) {
                            settle.2 = false;
                        }
                    }
                    return true;
                }
                let key = (
                    b.device,
                    path.clone(),
                    browser_scope_key(b.container.as_ref()),
                );
                let container = b.container.clone();
                let show_hidden = b.show_hidden;
                if b.peek.as_ref().is_some_and(|p| p.path == path && p.loading) {
                    match &reply.result {
                        Ok(value) => {
                            let peek = b.peek.as_mut().unwrap();
                            peek.entries = parse_entries(value).into();
                            peek.loading = false;
                        }
                        // The peek falls back to the plain folder card.
                        Err(_) => b.peek = None,
                    }
                }
                if let Ok(value) = &reply.result {
                    if !self.browser_cache.contains_key(&key) {
                        let mut cached = Browser::new(reply.device, path);
                        apply_location(&mut cached, value);
                        cached.container = container;
                        cached.show_hidden = show_hidden;
                        cached.entries = parse_entries(value).into();
                        cached.total = value["total"].as_u64();
                        if self.browser_cache.len() >= 64 {
                            if let Some(old) = self.browser_cache.keys().next().cloned() {
                                self.browser_cache.remove(&old);
                            }
                        }
                        self.browser_cache.insert(key, cached);
                    }
                }
                true
            }
            Operation::Preview { path } => {
                let generation = reply.generation;
                let Some(b) = self.browser.as_mut().filter(|b| b.device == reply.device) else {
                    return false;
                };
                let Some(slot) = b
                    .peek_inflight
                    .iter()
                    .position(|(p, g)| p == path && *g == generation)
                else {
                    return false;
                };
                b.peek_inflight.remove(slot);
                let path = path.clone();
                // The runtime answers a read it cancelled (superseded by a newer navigation
                // read) with `Ok(Null)`. That is a cancellation, not content: re-arm the peek
                // so the settle tick asks again instead of showing "Preview unavailable".
                if matches!(reply.result, Ok(Value::Null)) {
                    if let Some(peek) = b
                        .peek
                        .as_mut()
                        .filter(|p| p.preview_path.as_ref() == Some(&path))
                    {
                        peek.preview_path = None;
                        peek.peek_value = None;
                    }
                    if let Some(settle) = b.settle.as_mut().filter(|s| s.0 == path) {
                        settle.2 = false;
                    }
                    self.open_waiting_preview(reply.device, path);
                    return true;
                }
                // A peek reply only ever feeds the peek; an outdated one is dropped.
                let Some(peek) = b
                    .peek
                    .as_mut()
                    .filter(|p| p.preview_path.as_ref() == Some(&path))
                else {
                    return true;
                };
                match std::mem::replace(&mut reply.result, Ok(Value::Null)) {
                    Ok(value) => {
                        apply_preview(peek, &path, 1, reply.preview.take(), &value);
                        peek.peek_value = Some(value);
                    }
                    Err(e) => {
                        peek.preview_pending_page = None;
                        peek.preview_rich = None;
                        peek.preview = Some(safe_text(&format!("Preview unavailable · {e:#}")));
                        peek.preview_revision = peek.preview_revision.wrapping_add(1);
                    }
                }
                self.open_waiting_preview(reply.device, path);
                true
            }
            Operation::ReadChunk { path, offset, .. } => {
                let (path, offset) = (path.clone(), *offset);
                let result = std::mem::replace(&mut reply.result, Ok(Value::Null));
                self.apply_preview_chunk(reply.device, &path, offset, result, reply.preview.take());
                true
            }
            _ => false,
        }
    }
}
