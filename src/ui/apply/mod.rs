//! Task reply dispatch: shared bookkeeping, then per-operation-group handlers.
mod containers;
pub(super) mod files;
mod network;
mod sessions;
use super::*;

impl App {
    pub(super) fn apply(&mut self, mut reply: Reply) {
        // A highlight follow-up is not a request of its own.
        let highlight = reply
            .preview
            .as_ref()
            .is_some_and(|p| p.kind == "highlight");
        if !highlight {
            self.pending_requests
                .set(self.pending_requests.get().saturating_sub(1));
        }
        let file_action = matches!(
            match &reply.op {
                Operation::ContainerFileAction { operation, .. } => operation.as_ref(),
                other => other,
            },
            Operation::Remove { .. }
                | Operation::Rename { .. }
                | Operation::Transfer(_)
                | Operation::ScopedTransfer(_)
                | Operation::TransferRetry { .. }
        );
        if file_action {
            self.file_busy = false;
            self.start_next_file_action();
        }
        if let Some(id) = &reply.device_id {
            let Some(device) = self.devices.iter().position(|d| &d.id == id) else {
                return; // Device removed while this request was in flight.
            };
            if reply
                .execution_target
                .as_ref()
                .is_some_and(|target| target != &self.devices[device].target)
            {
                return; // Reply belongs to an SSH address that has since changed.
            }
            reply.device = device;
        }
        if reply.device >= self.devices.len() || reply.device >= self.work.len() {
            return;
        }
        if highlight {
            self.apply_highlight_reply(reply);
            return;
        }
        if let Operation::SessionPaneInput {
            pane_id, session, ..
        } = &reply.op
        {
            if reply.result.is_err()
                && self
                    .session_preview
                    .focused
                    .as_ref()
                    .is_some_and(|(device, focused, pane)| {
                        device.id == self.devices[reply.device].id
                            && pane == pane_id
                            && focused.terminal == session.terminal
                    })
            {
                self.session_preview.focused = None;
                self.set_notice("Terminal input failed; focus released".into());
            }
            return;
        }
        if matches!(reply.op, Operation::SessionPreview { .. }) {
            self.apply_session_preview(reply);
            return;
        }
        if matches!(reply.op, Operation::Metrics) {
            self.vitals_replied(reply.device, reply.result);
            return;
        }
        if reply.generation < self.device_generation_floor {
            return;
        }
        if let Operation::ContainerProviders { scope } = &reply.op {
            self.apply_container_profiles(reply.device, scope, reply.result);
            return;
        }
        if let Operation::ContainerFiles { scope, operation } = &reply.op {
            if self
                .browser
                .as_ref()
                .is_none_or(|b| b.device != reply.device || b.container.as_ref() != Some(scope))
                || reply.generation != self.generation
            {
                return;
            }
            reply.op = *operation.clone();
        }
        if let Operation::ContainerFileAction { operation, .. } = &reply.op {
            // Mutation replies release the serialized queue even after navigating away.
            reply.op = *operation.clone();
        }
        // Folder prefetch, the inline peek and streamed preview chunks never touch the
        // visible listing or raise error notices of their own.
        if self.apply_files_side_reply(&mut reply) {
            return;
        }
        let failed_attachment_check = matches!(reply.op, Operation::Sessions)
            && reply.result.is_err()
            && self
                .attach_check
                .as_ref()
                .is_some_and(|(d, _, _, generation)| {
                    *d == reply.device && *generation == reply.generation
                });
        if failed_attachment_check {
            self.attach_check = None;
            self.set_notice_as(
                NoticeKind::Warning,
                "Could not verify the selected terminal · no attachment started".into(),
            );
        }
        let connection_failure = reply
            .result
            .as_ref()
            .err()
            .is_some_and(transport::is_connection_failure);
        let passive_connection_check = matches!(
            reply.op,
            Operation::Info
                | Operation::Sessions
                | Operation::Network
                | Operation::NetworkCandidates
                | Operation::Containers
                | Operation::ProbeCandidate { .. }
        );
        let known_down = self
            .devices
            .get(reply.device)
            .is_some_and(|_| !self.background_checks_enabled(reply.device));
        let notify_error = if reply.result.is_ok() {
            if reply.generation == self.generation {
                self.unavailable_notified
                    .remove(&self.devices[reply.device].id);
            }
            true
        } else if connection_failure && (passive_connection_check || known_down) {
            // The device's status mark is the passive reachability signal.
            false
        } else if connection_failure {
            (reply.generation == self.generation || file_action)
                && self
                    .unavailable_notified
                    .insert(self.devices[reply.device].id.clone())
        } else {
            true
        };
        // External-terminal discovery is a best-effort UI extra; a failing or
        // unreachable device must not raise a notice for it. A failed listing is the
        // hub ticker's (with its retry key), so it raises no toast either.
        let notify_error =
            notify_error && !matches!(reply.op, Operation::ExternalSessions | Operation::Sessions);
        let listing_offset = match &reply.op {
            Operation::ListPage { offset, .. } => *offset,
            _ => 0,
        };
        if matches!(reply.op, Operation::Info) {
            self.provider_loading.remove(&reply.device);
            if self.peer_inflight.remove(&reply.device) {
                self.peer_evidence
                    .insert(reply.device, (transport::now(), reply.result.is_ok()));
                self.pump_network_refresh();
            }
        }
        if matches!(reply.op, Operation::Network) {
            self.network_inflight.remove(&reply.device);
            self.network_loading = self
                .network_device()
                .is_some_and(|d| self.network_inflight.contains(&d));
        }
        if matches!(reply.op, Operation::NetworkCandidates) {
            self.network_candidates_inflight.remove(&reply.device);
        }
        if matches!(reply.op, Operation::Network | Operation::NetworkCandidates) {
            self.pump_network_refresh();
        }
        if let Operation::ProbeCandidate { address, interface } = &reply.op {
            self.neighbor_probes
                .remove(&(reply.device, address.clone(), interface.clone()));
            let evidence = match &reply.result {
                Ok(v) => v.clone(),
                Err(_) => {
                    serde_json::json!({"observed_at":transport::now(), "ssh":{"state":"unknown"}})
                }
            };
            self.neighbor_evidence
                .insert((reply.device, address.clone(), interface.clone()), evidence);
            self.pump_network_refresh();
        }
        let is_sessions = matches!(reply.op, Operation::Sessions);
        if is_sessions {
            self.work[reply.device].loading = false;
        }
        if matches!(
            reply.op,
            Operation::Create(_)
                | Operation::CreateYolo(_)
                | Operation::ContainerCreate { .. }
                | Operation::ContainerAgentCreate { .. }
                | Operation::AdoptSession { .. }
        ) {
            self.creating = false;
        }
        if matches!(
            reply.op,
            Operation::Containers
                | Operation::DevcontainerUp { .. }
                | Operation::DevcontainerRebuild { .. }
                | Operation::ContainerLifecycle { .. }
                | Operation::ContainerAccess { .. }
        ) {
            self.apply_container_reply(&reply, connection_failure, notify_error);
            return;
        }
        let value = match std::mem::replace(&mut reply.result, Ok(Value::Null)) {
            Ok(v) => v,
            Err(e) => {
                if let Operation::Transfer(spec) | Operation::ScopedTransfer(spec) = &reply.op {
                    self.pending_transfers.remove(&spec.key);
                }
                if let Operation::ProbeCandidate { address, interface } = &reply.op {
                    if let Some(candidates) = self
                        .network
                        .get_mut(&reply.device)
                        .and_then(|v| v["candidates"].as_array_mut())
                    {
                        if let Some(candidate) = candidates.iter_mut().find(|c| {
                            c["address"].as_str() == Some(address.as_str())
                                && c["interface"].as_str() == interface.as_deref()
                        }) {
                            candidate["ssh"]["state"] = serde_json::json!("unknown");
                        }
                    }
                }
                if matches!(reply.op, Operation::ProbeCandidate { .. }) {
                    return;
                }
                let message =
                    safe_text(&format!("{}: {e:#}", identity(&self.devices[reply.device])));
                if matches!(reply.op, Operation::Network | Operation::NetworkCandidates) {
                    if let Some(v) = self.network.get_mut(&reply.device) {
                        v["_authenticated_at"] = Value::Null;
                        v["candidates_observed_at"] = Value::Null;
                        v["internet"]["state"] = serde_json::json!("unknown");
                        v["internet"]["stale"] = serde_json::json!(true);
                        if let Some(candidates) = v["candidates"].as_array_mut() {
                            for candidate in candidates {
                                candidate["ssh"]["state"] = serde_json::json!("unknown");
                            }
                        }
                    }
                }
                if matches!(reply.op, Operation::Info) {
                    self.providers.remove(&reply.device);
                    if self
                        .pending_adoption
                        .as_ref()
                        .is_some_and(|(d, _, _)| d.id == self.devices[reply.device].id)
                    {
                        self.pending_adoption = None;
                        self.set_notice_as(
                            NoticeKind::Error,
                            format!(
                                "Terminal availability check failed: {}",
                                safe_text(&e.to_string())
                            ),
                        );
                    }
                }
                if is_sessions {
                    // Failed contact is not evidence that a cached terminal is attachable.
                    let selected = self
                        .selected_session()
                        .map(|(d, s)| (d, workspaces::runtime_key(&s)));
                    for session in &mut self.work[reply.device].sessions {
                        session.activity = None;
                        session.agent_tasks.clear();
                    }
                    self.work[reply.device].fetched = 0;
                    self.work[reply.device].error = Some(message.clone());
                    self.reconcile_session_selection(selected);
                }
                if reply.generation == self.generation || file_action {
                    if file_action {
                        if self.file_errors.len() < 8 {
                            self.file_errors.push(message.clone());
                        }
                        if notify_error {
                            self.set_notice_as(
                                NoticeKind::Error,
                                format!(
                                    "{} file actions failed · {}",
                                    self.file_errors.len(),
                                    message
                                ),
                            );
                        }
                    } else if notify_error && !failed_attachment_check {
                        self.set_notice_as(NoticeKind::Error, message.clone());
                    }
                }
                if reply.generation == self.generation
                    && matches!(
                        reply.op,
                        Operation::Preview { .. } | Operation::PreviewPage { .. }
                    )
                {
                    if let Some(b) = &mut self.browser {
                        b.preview_pending_page = None;
                        b.preview_requested_page = b.preview_rich.as_ref().map_or(1, |p| p.page);
                        if b.preview_rich.is_none() {
                            b.preview = Some(message.clone());
                        }
                    }
                }
                if reply.generation == self.generation {
                    if let Some(b) = &mut self.browser {
                        b.loading = false;
                    }
                    self.network_loading = false;
                    if matches!(reply.op, Operation::Network) {
                        if let Some(v) = self.network.get_mut(&reply.device) {
                            v["internet"]["state"] = serde_json::json!("unknown");
                            v["internet"]["stale"] = serde_json::json!(true);
                        }
                    }
                }
                return;
            }
        };
        if let Operation::Transfer(spec) | Operation::ScopedTransfer(spec) = &reply.op {
            if spec.cut
                && matches!(
                    value["status"].as_str(),
                    Some("queued" | "running" | "complete" | "submitting")
                )
            {
                let clipboard_id = self.submitted_clipboards.remove(&spec.key);
                if let Some(c) = self
                    .clipboard
                    .as_mut()
                    .filter(|c| Some(&c.id) == clipboard_id.as_ref())
                {
                    c.entries.retain(|e| e.path != spec.source_path);
                    if c.entries.is_empty() {
                        self.clipboard = None;
                    }
                }
            }
        }
        // Session caches remain useful across view changes; navigation responses do not.
        if matches!(
            reply.op,
            Operation::Info
                | Operation::Sessions
                | Operation::ExternalSessions
                | Operation::AdoptSession { .. }
                | Operation::Create(_)
                | Operation::CreateYolo(_)
                | Operation::ContainerCreate { .. }
                | Operation::ContainerAgentCreate { .. }
                | Operation::CommandPanel { .. }
                | Operation::CapturePane { .. }
                | Operation::PaneInput { .. }
                | Operation::PaneClose { .. }
                | Operation::StopSession { .. }
                | Operation::StopAgentSession { .. }
        ) {
            self.apply_session_reply(reply, value);
        } else if matches!(
            reply.op,
            Operation::Network | Operation::NetworkCandidates | Operation::ProbeCandidate { .. }
        ) {
            self.apply_network_reply(reply, value);
        } else if matches!(
            reply.op,
            Operation::Jobs
                | Operation::TransferJobs
                | Operation::TransferCancel { .. }
                | Operation::Transfer(_)
                | Operation::ScopedTransfer(_)
                | Operation::TransferRetry { .. }
                | Operation::Copy { .. }
                | Operation::List { .. }
                | Operation::ListPage { .. }
                | Operation::Preview { .. }
                | Operation::PreviewPage { .. }
                | Operation::Mkdir { .. }
                | Operation::Rename { .. }
                | Operation::Remove { .. }
        ) {
            self.apply_file_reply(reply, value, listing_offset);
        }
    }
}
