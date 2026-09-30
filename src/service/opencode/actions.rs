use std::{future::Future, path::Path, sync::Arc, time::Duration};

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

impl super::Integration {
    pub(super) fn spawn_navigation<T: Send + 'static>(
        self: &Arc<Self>,
        runtime: &tokio::runtime::Runtime,
        busy_message: &str,
        action: impl Future<Output = Result<T, String>> + Send + 'static,
    ) -> Result<tokio::sync::oneshot::Receiver<Result<T, String>>, String> {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if state
            .navigation
            .as_ref()
            .is_some_and(|task| !task.is_finished())
        {
            return Err(busy_message.into());
        }
        let generation = state.generation;
        let integration = Arc::downgrade(self);
        let (sender, receiver) = tokio::sync::oneshot::channel();
        state.navigation = Some(runtime.spawn(async move {
            let result = action.await;
            let Some(integration) = integration.upgrade() else {
                return;
            };
            {
                let mut state = integration
                    .state
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                if state.generation != generation {
                    return;
                }
                // The reply can immediately wake a caller that submits another action.
                state.navigation.take();
            }
            let _ = sender.send(result);
        }));
        Ok(receiver)
    }
}

impl AppService {
    pub(in crate::service) fn validate_opencode_launch(&self) -> Result<(), String> {
        validate_launch(&self.settings, &self.opencode)
    }

    pub(in crate::service) async fn launch_instance_opencode(
        &self,
        workspace: &str,
        name: &str,
        initial_prompt: Option<&str>,
    ) -> Result<(), String> {
        launch_instance_opencode(
            Arc::clone(&self.environments),
            &self.settings,
            &self.opencode,
            workspace,
            name,
            initial_prompt,
        )
        .await
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
        self.opencode.spawn_navigation(
            &self.runtime,
            "OpenCode action is already in progress",
            async move {
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
            },
        )
    }
}

fn validate_launch(
    settings: &crate::service::settings::Settings,
    integration: &super::Integration,
) -> Result<(), String> {
    if !settings.opencode_enabled() {
        return Err("OpenCode integration is disabled".into());
    }
    if integration.current_zellij.is_empty() {
        return Err("Run Tandem inside Zellij to create an OpenCode session".into());
    }
    Ok(())
}

pub(in crate::service) async fn launch_instance_opencode(
    environments: Arc<crate::environments::Environments>,
    settings: &crate::service::settings::Settings,
    integration: &super::Integration,
    workspace: &str,
    name: &str,
    initial_prompt: Option<&str>,
) -> Result<(), String> {
    validate_launch(settings, integration)?;
    let target = workspace.to_owned();
    let instance = name.to_owned();
    tokio::task::spawn_blocking(move || environments.prepare_workspace_open(&target, &instance))
        .await
        .map_err(|error| error.to_string())??;
    launch_workspace_opencode(settings, integration, workspace, name, initial_prompt).await
}

pub(in crate::service) async fn launch_workspace_opencode(
    settings: &crate::service::settings::Settings,
    integration: &super::Integration,
    workspace: &str,
    name: &str,
    initial_prompt: Option<&str>,
) -> Result<(), String> {
    validate_launch(settings, integration)?;
    // A short-lived CLI has no TUI observation cache yet.
    let snapshot = tokio::time::timeout(
        Duration::from_secs(15),
        integration
            .observer
            .observe(&[workspace.to_owned()], Snapshot::default()),
    )
    .await
    .map_err(|_| "OpenCode observation timed out")??;
    let current = &integration.current_zellij;
    let destination = session_destination(&snapshot, current, None, |directory| {
        Path::new(directory).starts_with(workspace)
    })?;
    validate_launch(settings, integration)?;
    integration
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
