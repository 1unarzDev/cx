//! Container panel state: discovery refresh, tree rows, actions and container sessions.
use super::*;

impl App {
    // Network browser owns candidate selection and captures scope before actions.
    pub(super) fn open_containers(&mut self) {
        self.view = View::Containers;
        self.focus = Focus::Workspace;
        self.search.clear();
        self.container_selected = 0;
        self.refresh_containers();
    }
    pub(super) fn refresh_containers(&mut self) {
        for d in 0..self.devices.len() {
            if self.background_checks_enabled(d)
                && !self.containers_loading.contains(&d)
                && self.send(d, Operation::Containers)
            {
                self.containers_loading.insert(d);
            }
        }
    }
    pub(super) fn container_rows(&self) -> Vec<(usize, Option<crate::containers::Container>)> {
        let mut rows = vec![];
        for d in 0..self.devices.len() {
            if self.device > 0 && self.device != d + 1 {
                continue;
            }
            rows.push((d, None));
            if self.container_collapsed.contains(&d) {
                continue;
            }
            if let Some(items) = self.containers.get(&d) {
                for c in items {
                    if fuzzy_score(
                        &self.search,
                        &format!(
                            "{} {} {} {}",
                            c.name,
                            c.image,
                            c.state,
                            if c.devcontainer {
                                "devcontainer"
                            } else {
                                "docker"
                            }
                        ),
                    )
                    .is_some()
                    {
                        rows.push((d, Some(c.clone())));
                    }
                }
            }
        }
        rows
    }
    pub(super) fn selected_container(&self) -> Option<(usize, crate::containers::Container)> {
        self.container_rows()
            .get(self.container_selected)
            .and_then(|(d, c)| c.clone().map(|c| (*d, c)))
    }
    pub(super) fn open_container_actions(&mut self) {
        if let Some((d, c)) = self.selected_container() {
            self.dialog = Some(Dialog::ContainerActions(d, c));
            self.dialog_selected = 0;
        } else if let Some((d, _)) = self.container_rows().get(self.container_selected) {
            let d = *d;
            if !self.container_collapsed.remove(&d) {
                self.container_collapsed.insert(d);
            }
        }
    }
    pub(super) fn container_tree_motion(&mut self, expand: bool) {
        if let Some((d, c)) = self.container_rows().get(self.container_selected) {
            let d = *d;
            if expand {
                self.container_collapsed.remove(&d);
                if c.is_some() {
                    self.open_container_actions();
                }
            } else {
                self.container_collapsed.insert(d);
                self.container_selected = self
                    .container_rows()
                    .iter()
                    .position(|(i, c)| *i == d && c.is_none())
                    .unwrap_or(0);
            }
        }
    }
    pub(super) fn start_container_session(
        &mut self,
        d: usize,
        scope: ContainerScope,
        provider: String,
        yolo: bool,
    ) {
        let request = CreateSession {
            key: unique_key(),
            directory: scope.folder.clone(),
            provider,
            name: String::new(),
            resume: false,
            resume_pick: false,
        };
        self.creating = self.send(
            d,
            Operation::ContainerCreate {
                scope,
                request,
                yolo,
            },
        );
        self.set_notice("Starting container session · existing networks are preserved".into());
    }
    pub(super) fn destination_containers(
        &self,
        device: usize,
    ) -> Vec<crate::containers::Container> {
        let mut containers = self.containers.get(&device).cloned().unwrap_or_default();
        containers.retain(|c| c.state == "running" && (c.devcontainer || c.allowed));
        containers.sort_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
        containers
    }
}
