//! Task replies for listings, previews, transfers and file mutations.
use super::*;

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
                if let Some(b) = self.browser.as_mut().filter(|b| b.device == reply.device) {
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
                    let entries: Vec<Entry> = value
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
                                        identity: e
                                            .get("identity")
                                            .and_then(Value::as_str)
                                            .map(str::to_owned),
                                        hidden: e
                                            .get("hidden")
                                            .and_then(Value::as_bool)
                                            .unwrap_or_else(|| {
                                                e["name"]
                                                    .as_str()
                                                    .is_some_and(|s| s.starts_with('.'))
                                            }),
                                        rename_name: e
                                            .get("rename_name")
                                            .and_then(Value::as_str)
                                            .map(str::to_owned),
                                    })
                                })
                                .collect()
                        })
                        .unwrap_or_default();
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
                        b.entries = entries;
                    } else {
                        b.entries.extend(entries);
                    }
                    b.entries.sort_by(|a, b| {
                        (a.kind != "directory", a.name.as_str())
                            .cmp(&(b.kind != "directory", b.name.as_str()))
                    });
                    if let Some(path) = &b.restore_selection {
                        let visible = browser_entries(b);
                        if let Some(index) = visible.iter().position(|entry| &entry.path == path) {
                            b.selected = index;
                            if value["next_offset"].is_null() {
                                b.restore_selection = None;
                            }
                        } else {
                            b.selected = 0;
                            if value["next_offset"].is_null() {
                                b.restore_selection = None;
                            }
                        }
                    } else {
                        b.selected = b.selected.min(browser_entries(b).len().saturating_sub(1));
                    }
                    b.loading = false;
                    if value["next_offset"].is_null() {
                        let existing: BTreeSet<_> =
                            b.entries.iter().map(|e| e.path.clone()).collect();
                        b.marked.retain(|p| existing.contains(p));
                    }
                    if range_finished {
                        self.set_notice(
                            "Directory refreshed · range finished; selected files kept".into(),
                        );
                    }
                }
                if let Some(offset) = value["next_offset"].as_u64() {
                    if let Some(b) = &self.browser {
                        if b.entries.len() < 10000 {
                            self.send(
                                b.device,
                                Operation::ListPage {
                                    path: b.path.clone(),
                                    offset,
                                    limit: 1000,
                                },
                            );
                            if let Some(b) = &mut self.browser {
                                b.loading = true;
                            }
                            self.set_notice(
                                "Loading more directory entries · fuzzy search stays live".into(),
                            );
                        } else {
                            self.set_notice("10,000 entries loaded · remaining entries require a narrower directory".into());
                        }
                    }
                }
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
                let mut next = false;
                if let Some(b) = &mut self.browser {
                    if b.preview_path
                        .as_ref()
                        .is_some_and(|requested| requested != path)
                    {
                        return;
                    }
                    b.preview_path = Some(path.clone());
                    b.preview_pending_page = None;
                    if b.preview_requested_page == 0 {
                        b.preview_requested_page = page;
                    }
                    if b.preview_requested_page != page && b.preview_rich.is_some() {
                        next = true;
                    } else {
                        b.preview_requested_page = page;
                        b.preview_scroll.set(0);
                        b.preview_revision = b.preview_revision.wrapping_add(1);
                        let rich = reply
                            .preview
                            .unwrap_or_else(|| RichPreview::from_value(&value));
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
                        b.preview_find.matches = preview_matches(
                            &preview_display_lines(b, b.preview.as_deref().unwrap_or("")),
                            &b.preview_find.query,
                        );
                        b.preview_find.selected = b
                            .preview_find
                            .selected
                            .min(b.preview_find.matches.len().saturating_sub(1));
                        b.preview_find.reveal.set(!restoring);
                        if let Some(anchor) = b.preview_anchor.take() {
                            if let Some(scroll) = markdown_anchor_scroll(b, &anchor) {
                                b.preview_scroll.set(scroll);
                                b.preview_find.reveal.set(false);
                            } else {
                                self.set_notice_as(
                                    NoticeKind::Warning,
                                    "Linked file opened · heading not found".into(),
                                );
                            }
                        }
                    }
                }
                if next {
                    self.start_pdf_page_request();
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
}
