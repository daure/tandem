use super::{AppService, environments::spawn_workspace_command};
use crate::environments::Startup;
use crate::store::environments::{Instance, OperationState};

pub(crate) enum NewInstanceOutcome {
    Created(Instance),
    Existing(Instance),
}

impl AppService {
    pub(crate) fn new_instance(
        &self,
        name: &str,
        template: String,
        open_command: Option<Option<String>>,
        description: Option<String>,
    ) -> Result<NewInstanceOutcome, String> {
        let (existing, instance_lock) = self.environments.admit_new_instance(name, &template)?;
        if let Some(instance) = existing {
            if open_command.is_some() {
                let workspace = self.environments.workspace(name)?;
                self.runtime.block_on(self.launch_workspace_opener(
                    &workspace,
                    name,
                    open_command.flatten().as_deref(),
                    description.as_deref().unwrap_or(&instance.description),
                ))?;
            }
            return Ok(NewInstanceOutcome::Existing(instance));
        }
        let (sender, ready) = tokio::sync::oneshot::channel();
        let operation = self
            .environments
            .begin_instance(name, template, description.as_deref())?;
        // Keep admission's lock through startup so another process cannot create between
        // the existence check and Compose execution.
        let operation = self.schedule_operation(
            operation,
            600,
            Startup {
                instance_lock: Some(instance_lock),
                description: description.clone(),
                workspace_ready: open_command.is_some().then_some(sender),
                ..Default::default()
            },
        );
        self.runtime.block_on(async {
            let open = async {
                if open_command.is_none() {
                    return Ok(());
                }
                // A failed preparation closes the channel without permitting the opener.
                let Ok(workspace) = ready.await else {
                    return Ok(());
                };
                self.launch_workspace_opener(
                    &workspace,
                    name,
                    open_command.flatten().as_deref(),
                    description.as_deref().unwrap_or_default(),
                )
                .await
            };
            let (operation, opened) = tokio::join!(self.wait_operation(&operation.id), open);
            let operation = operation?;
            if operation.state != OperationState::Succeeded {
                let mut error = operation
                    .error
                    .unwrap_or_else(|| "instance startup failed".into());
                if let Err(open_error) = opened {
                    error.push_str(&format!("; workspace opener failed: {open_error}"));
                }
                return Err(error);
            }
            opened.map_err(|error| {
                format!("instance is ready, but workspace opener failed: {error}")
            })?;
            operation
                .instance
                .map(NewInstanceOutcome::Created)
                .ok_or_else(|| "startup completed without an instance".into())
        })
    }

    async fn launch_workspace_opener(
        &self,
        workspace: &str,
        name: &str,
        open_param: Option<&str>,
        description: &str,
    ) -> Result<(), String> {
        let environments = std::sync::Arc::clone(&self.environments);
        let target = workspace.to_owned();
        let instance = name.to_owned();
        tokio::task::spawn_blocking(move || {
            environments.prepare_workspace_open(&target, &instance)
        })
        .await
        .map_err(|error| error.to_string())??;
        let command = self.settings.read_open_command().await?;
        let mut child =
            spawn_workspace_command(&command, workspace, name, open_param, Some(description))?;
        // Editors may stay running after startup. Reap while Tandem is alive without
        // making instance readiness depend on the editor's lifetime.
        self.runtime.spawn(async move {
            let error = match child.wait().await {
                Ok(status) if status.success() => return,
                Ok(status) => format!("workspace opener exited with {status}"),
                Err(error) => format!("cannot wait for workspace opener: {error}"),
            };
            crate::diagnostics::record_error(
                "workspace opener failed",
                &std::io::Error::other(error),
            );
        });
        Ok(())
    }
}
