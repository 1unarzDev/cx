//! Task replies for container discovery, devcontainer builds and lifecycle actions.
use super::*;

impl App {
    pub(in crate::ui) fn apply_container_reply(
        &mut self,
        reply: &Reply,
        connection_failure: bool,
        notify_error: bool,
    ) {
        if matches!(reply.op, Operation::Containers) {
            self.containers_loading.remove(&reply.device);
            match &reply.result {
                Ok(v) => {
                    match serde_json::from_value::<Vec<crate::containers::Container>>(
                        v["containers"].clone(),
                    ) {
                        Ok(items) => {
                            if v["complete"] == true && v["truncated"] != true {
                                let selected = self
                                    .selected_session()
                                    .map(|(d, s)| (d, workspaces::runtime_key(&s)));
                                self.work[reply.device].sessions.retain(|session| {
                                    session.container.as_ref().is_none_or(|scope| {
                                        items.iter().any(|c| {
                                            c.id == scope.id
                                                && c.engine == scope.engine
                                                && c.state == "running"
                                                && c.started_at == scope.started_at
                                        })
                                    })
                                });
                                self.reconcile_session_selection(selected);
                            }
                            self.containers.insert(reply.device, items);
                            self.container_errors.remove(&reply.device);
                        }
                        Err(_) => {
                            self.container_errors.insert(
                                reply.device,
                                "Invalid container discovery response".into(),
                            );
                        }
                    }
                }
                Err(e) => {
                    self.container_errors.insert(reply.device, format!("{e:#}"));
                    self.containers.remove(&reply.device);
                    if connection_failure && notify_error {
                        self.set_notice_as(
                            NoticeKind::Error,
                            safe_text(&format!("{}: {e:#}", identity(&self.devices[reply.device]))),
                        );
                    }
                }
            }
            if let Some(Dialog::DestinationScope(d, choices)) = &self.dialog {
                if *d == reply.device && choices.is_empty() && reply.result.is_ok() {
                    // The initial scan fills an empty picker; once populated, choices
                    // stay pinned so refresh cannot silently select a different container.
                    self.dialog = Some(Dialog::DestinationScope(
                        *d,
                        self.destination_containers(*d),
                    ));
                }
            }
            self.container_selected = self
                .container_selected
                .min(self.container_rows().len().saturating_sub(1));
            return;
        }
        if matches!(
            reply.op,
            Operation::DevcontainerUp { .. } | Operation::DevcontainerRebuild { .. }
        ) {
            let rebuild = if let Operation::DevcontainerRebuild { scope } = &reply.op {
                self.rebuilding.remove(&(reply.device, scope.id.clone()));
                true
            } else {
                false
            };
            self.send(reply.device, Operation::Containers);
            self.containers_loading.insert(reply.device);
            match &reply.result {
                Ok(_) => {
                    if !rebuild {
                        self.device = reply.device + 1;
                        self.open_containers();
                    }
                    self.set_notice_as(
                        NoticeKind::Success,
                        "Workspace ready · choose its terminal or files".into(),
                    );
                }
                Err(e) if notify_error => self.set_notice_as(
                    NoticeKind::Error,
                    format!("{e:#} · refresh checks workspace state"),
                ),
                Err(_) => (),
            }
            return;
        }
        if matches!(
            reply.op,
            Operation::ContainerLifecycle { .. } | Operation::ContainerAccess { .. }
        ) {
            if reply.result.is_ok() || notify_error {
                self.set_notice_as(
                    if reply.result.is_err() {
                        NoticeKind::Error
                    } else {
                        NoticeKind::Success
                    },
                    match &reply.result {
                        Ok(_) => {
                            "Container action complete · network configuration preserved".into()
                        }
                        Err(e) => format!("{e:#}"),
                    },
                );
            }
            self.refresh_containers();
            return;
        }
    }
}
