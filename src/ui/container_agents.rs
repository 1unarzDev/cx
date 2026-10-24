//! Container profiles belong to an exact execution scope, never the host's PATH.
use super::*;

#[derive(Default)]
pub(super) struct Profiles {
    providers: Vec<String>,
    fetched: u64,
    error: Option<String>,
}
fn key(d: usize, scope: &ContainerScope) -> (usize, String) {
    (d, serde_json::to_string(scope).unwrap_or_default())
}
pub(super) fn has_history(scope: Option<&ContainerScope>, provider: &str) -> bool {
    scope.is_some() && matches!(provider, "claude" | "codex")
}
impl App {
    fn container_agents_supported(&self, d: usize) -> bool {
        self.devices[d].target.is_none()
            || self.providers.get(&d).is_some_and(|(caps, t)| {
                transport::now().saturating_sub(*t) < 60
                    && caps.iter().any(|s| s == "container-agents-v1")
            })
    }
    pub(super) fn container_session_choices(
        &self,
        d: usize,
        scope: &ContainerScope,
    ) -> Vec<&'static menus::SessionChoice> {
        let evidence = self
            .container_profiles
            .get(&key(d, scope))
            .filter(|p| p.error.is_none() && transport::now().saturating_sub(p.fetched) < 60);
        menus::CONTAINER
            .iter()
            .filter(|choice| {
                choice.provider == "shell"
                    || (self.container_agents_supported(d)
                        && evidence
                            .is_some_and(|p| p.providers.iter().any(|s| s == choice.provider)))
            })
            .collect()
    }
    pub(super) fn container_profile_detail(&self, d: usize, scope: &ContainerScope) -> String {
        let k = key(d, scope);
        let status = if self.container_profile_loading.contains(&k) {
            "Checking agents in this container…"
        } else if !self.container_agents_supported(d) {
            "Shell ready · check/update device CX for agents"
        } else if let Some(p) = self.container_profiles.get(&k) {
            if p.error.is_some() {
                "Agent check failed · reopen to retry"
            } else if transport::now().saturating_sub(p.fetched) >= 60 {
                "Agent evidence expired · reopen to check"
            } else {
                "Installed here · sign-in not checked"
            }
        } else {
            "Agent state unknown · Shell available"
        };
        format!(
            "{} · user {}\n{}\n{}",
            safe_label(&scope.name),
            safe_label(&scope.user),
            safe_label(&scope.folder),
            status
        )
    }
    pub(super) fn open_container_chooser(&mut self, d: usize, scope: ContainerScope) {
        self.dialog = Some(Dialog::ContainerProvider(d, scope.clone()));
        self.dialog_selected = 0;
        self.check_container_profiles(d, &scope);
    }
    pub(super) fn check_container_profiles(&mut self, d: usize, scope: &ContainerScope) {
        if !self.container_agents_supported(d) {
            if !self.provider_loading.contains(&d)
                && self
                    .providers
                    .get(&d)
                    .is_none_or(|(_, t)| transport::now().saturating_sub(*t) >= 60)
                && self.send_explicit(d, Operation::Info)
            {
                self.provider_loading.insert(d);
            }
            return;
        }
        let k = key(d, scope);
        if self.container_profile_loading.contains(&k)
            || self.container_profiles.get(&k).is_some_and(|p| {
                p.error.is_none() && transport::now().saturating_sub(p.fetched) < 60
            })
        {
            return;
        }
        if self.send_explicit(
            d,
            Operation::ContainerProviders {
                scope: scope.clone(),
            },
        ) {
            self.container_profile_loading.insert(k);
        }
    }
    pub(super) fn apply_container_profiles(
        &mut self,
        d: usize,
        scope: &ContainerScope,
        result: Result<Value>,
    ) {
        let previous = match &self.dialog {
            Some(Dialog::ContainerProvider(device, current))
                if *device == d && current == scope =>
            {
                self.container_session_choices(d, scope)
                    .get(self.dialog_selected)
                    .map(|c| c.provider)
            }
            _ => None,
        };
        let k = key(d, scope);
        self.container_profile_loading.remove(&k);
        let result = result.and_then(|v| {
            anyhow::ensure!(
                v["observed_at"].as_u64().is_some_and(|t| t > 0),
                "container observation time unavailable"
            );
            let observed: ContainerScope = serde_json::from_value(v["scope"].clone())?;
            anyhow::ensure!(&observed == scope, "container profile scope changed");
            let providers: Vec<String> = serde_json::from_value(v["providers"].clone())?;
            anyhow::ensure!(
                providers.len() <= 3
                    && providers
                        .iter()
                        .all(|p| ["shell", "claude", "codex"].contains(&p.as_str())),
                "invalid container profiles"
            );
            Ok(providers)
        });
        // Bound metadata even when many workspaces are explored in one viewer.
        if self.container_profiles.len() >= 128 && !self.container_profiles.contains_key(&k) {
            self.container_profiles.clear();
        }
        self.container_profiles.insert(
            k,
            match result {
                Ok(providers) => Profiles {
                    providers,
                    fetched: transport::now(),
                    error: None,
                },
                Err(e) => Profiles {
                    providers: vec![],
                    fetched: transport::now(),
                    error: Some(e.to_string()),
                },
            },
        );
        if matches!(&self.dialog, Some(Dialog::ContainerProvider(device, current)) if *device == d && current == scope)
        {
            let choices = self.container_session_choices(d, scope);
            self.dialog_selected = previous
                .and_then(|p| choices.iter().position(|c| c.provider == p))
                .unwrap_or(choices.len());
        }
    }
    pub(super) fn choose_container_profile(
        &mut self,
        d: usize,
        scope: ContainerScope,
        provider: String,
    ) {
        if !self
            .container_session_choices(d, &scope)
            .iter()
            .any(|c| c.provider == provider)
        {
            return;
        }
        let rows = self.scoped_sessions(d, &scope.folder, Some(&scope), &provider);
        if provider == "shell" && rows.is_empty() {
            self.dialog = None;
            self.start_container_session(d, scope, provider, false);
        } else {
            self.dialog = Some(Dialog::SessionPicker(
                d,
                scope.folder.clone(),
                Some(scope),
                provider,
                rows,
            ));
            self.dialog_selected = 0;
            self.refresh_sessions_on(d);
        }
    }
    pub(super) fn container_permissions(
        &mut self,
        d: usize,
        scope: ContainerScope,
        provider: String,
        resume: bool,
    ) {
        if !self
            .container_session_choices(d, &scope)
            .iter()
            .any(|c| c.provider == provider)
        {
            self.check_container_profiles(d, &scope);
            self.set_notice_as(
                NoticeKind::Warning,
                "Container agent availability changed · reopen the session chooser".into(),
            );
            return;
        }
        self.dialog = Some(Dialog::ContainerPermissions(d, scope, provider, resume));
        self.dialog_selected = 0;
    }
    pub(super) fn launch_container_agent(
        &mut self,
        d: usize,
        scope: ContainerScope,
        provider: String,
        yolo: bool,
        resume: bool,
    ) {
        if self.creating {
            return;
        }
        if !self
            .container_session_choices(d, &scope)
            .iter()
            .any(|c| c.provider == provider)
        {
            self.check_container_profiles(d, &scope);
            self.set_notice_as(
                NoticeKind::Warning,
                "Container agent availability changed · reopen the session chooser".into(),
            );
            return;
        }
        let request = CreateSession {
            key: unique_key(),
            directory: scope.folder.clone(),
            provider,
            name: String::new(),
            resume,
        };
        self.creating = self.send_explicit(
            d,
            Operation::ContainerAgentCreate {
                scope,
                request,
                yolo,
            },
        );
        if self.creating {
            self.dialog = None;
            self.set_notice(
                if resume {
                    "Opening the provider's conversation picker in this container…"
                } else {
                    "Starting a new agent in this container…"
                }
                .into(),
            );
        } else {
            self.set_notice_as(
                NoticeKind::Warning,
                "Request queue busy · no agent started".into(),
            );
        }
    }
}
