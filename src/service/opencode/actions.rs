use std::{path::Path, sync::Arc, time::Duration};

use crate::{
    service::AppService,
    store::opencode::{Pane, Snapshot, workspace_owner},
};

pub(super) fn directory_name(directory: &str) -> String {
    Path::new(directory)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(directory)
        .to_owned()
}

impl AppService {
    pub(in crate::service) fn validate_opencode_launch(&self) -> Result<(), String> {
        if !self.opencode_enabled() {
            return Err("OpenCode integration is disabled".into());
        }
        if self.opencode.current_zellij.is_empty() {
            return Err("Run Tandem inside Zellij to create an OpenCode session".into());
        }
        Ok(())
    }

    pub(in crate::service) async fn launch_instance_opencode(
        &self,
        workspace: &str,
        name: &str,
        initial_prompt: Option<&str>,
    ) -> Result<(), String> {
        self.validate_opencode_launch()?;
        let environments = Arc::clone(&self.environments);
        let target = workspace.to_owned();
        let instance = name.to_owned();
        tokio::task::spawn_blocking(move || {
            environments.prepare_workspace_open(&target, &instance)
        })
        .await
        .map_err(|error| error.to_string())??;
        // A short-lived CLI has no TUI observation cache yet.
        let snapshot = tokio::time::timeout(
            Duration::from_secs(15),
            self.opencode
                .observer
                .observe(&[workspace.to_owned()], Snapshot::default()),
        )
        .await
        .map_err(|_| "OpenCode observation timed out")??;
        let current = &self.opencode.current_zellij;
        let destination = session_destination(&snapshot, current, None, |directory| {
            Path::new(directory).starts_with(workspace)
        })?;
        self.validate_opencode_launch()?;
        self.opencode
            .observer
            .new_session(
                workspace,
                name,
                current,
                destination.as_ref(),
                initial_prompt,
            )
            .await
            .map(|_| ())
    }

    pub(crate) fn new_opencode_session(
        &self,
        directory: &str,
        preferred: Option<Pane>,
    ) -> Result<tokio::sync::oneshot::Receiver<Result<Pane, String>>, String> {
        if !self.opencode_enabled() {
            return Err("OpenCode integration is disabled".into());
        }
        let snapshot = self.opencode_snapshot();
        let instances = self.environments.snapshot().instances;
        let owner = |directory: &str| {
            workspace_owner(
                directory,
                instances
                    .iter()
                    .map(|instance| (instance.name.as_str(), instance.workspace.as_str())),
            )
        };
        let instance = owner(directory);
        let owned = instances
            .iter()
            .find(|candidate| Some(candidate.name.as_str()) == instance);
        if owned.is_none_or(|instance| instance.workspace != directory)
            && !snapshot
                .workspace_directories()
                .any(|known| known == directory)
        {
            return Err("OpenCode workspace is unavailable; refresh and try again".into());
        }
        let current = self.opencode.current_zellij.clone();
        let matches = |other: &str| match instance {
            Some(name) => owner(other) == Some(name),
            None => other == directory,
        };
        let destination = session_destination(&snapshot, &current, preferred, matches)?;
        let name = instance
            .map(str::to_owned)
            .unwrap_or_else(|| directory_name(directory));
        let workspace = owned.map(|instance| instance.workspace.clone());
        let directory = directory.to_owned();
        let environments = Arc::clone(&self.environments);
        let observer = self.opencode.observer.clone();
        let settings = Arc::clone(&self.settings);
        let (sender, receiver) = tokio::sync::oneshot::channel();
        let mut state = self
            .opencode
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if state
            .navigation
            .as_ref()
            .is_some_and(|task| !task.is_finished())
        {
            return Err("OpenCode action is already in progress".into());
        }
        state.navigation = Some(self.runtime.spawn(async move {
            let result = async {
                if !settings.opencode_enabled() {
                    return Err("OpenCode integration is disabled".into());
                }
                if let Some(workspace) = workspace {
                    let instance = name.clone();
                    tokio::task::spawn_blocking(move || {
                        environments.prepare_workspace_open(&workspace, &instance)
                    })
                    .await
                    .map_err(|error| error.to_string())??;
                }
                observer
                    .new_session(&directory, &name, &current, destination.as_ref(), None)
                    .await
            }
            .await;
            let _ = sender.send(result);
        }));
        Ok(receiver)
    }
}

fn session_destination(
    snapshot: &Snapshot,
    current: &str,
    preferred: Option<Pane>,
    matches: impl Fn(&str) -> bool,
) -> Result<Option<Pane>, String> {
    let panes = snapshot
        .sessions
        .iter()
        .filter(|session| matches(&session.directory))
        .flat_map(|session| &session.panes)
        .chain(
            snapshot
                .clients
                .iter()
                .filter(|client| matches(&client.directory))
                .map(|client| &client.pane),
        )
        .collect::<Vec<_>>();
    if preferred
        .as_ref()
        .is_some_and(|pane| !panes.contains(&pane))
    {
        return Err("OpenCode pane is unavailable; refresh and try again".into());
    }
    Ok(preferred.or_else(|| {
        panes
            .into_iter()
            .min_by_key(|pane| pane.session != current)
            .cloned()
    }))
}
