use super::AppService;
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
        opencode: Option<Option<String>>,
        description: Option<String>,
    ) -> Result<NewInstanceOutcome, String> {
        let (existing, instance_lock) = self.environments.admit_new_instance(name, &template)?;
        if opencode.is_some() {
            self.validate_opencode_launch()?;
        }
        if let Some(instance) = existing {
            if let Some(initial_prompt) = opencode {
                let workspace = self.environments.workspace(name)?;
                self.runtime.block_on(self.launch_instance_opencode(
                    &workspace,
                    name,
                    initial_prompt.as_deref(),
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
                workspace_ready: opencode.is_some().then_some(sender),
                ..Default::default()
            },
        );
        self.runtime.block_on(async {
            let open = async {
                let Some(initial_prompt) = opencode else {
                    return Ok(());
                };
                // A failed preparation closes the channel without permitting OpenCode startup.
                let Ok(workspace) = ready.await else {
                    return Ok(());
                };
                self.launch_instance_opencode(&workspace, name, initial_prompt.as_deref())
                    .await
            };
            let (operation, opened) = tokio::join!(self.wait_operation(&operation.id), open);
            let operation = operation?;
            if operation.state != OperationState::Succeeded {
                let mut error = operation
                    .error
                    .unwrap_or_else(|| "instance startup failed".into());
                if let Err(open_error) = opened {
                    error.push_str(&format!("; OpenCode launch failed: {open_error}"));
                }
                return Err(error);
            }
            opened.map_err(|error| {
                format!("instance is ready, but OpenCode launch failed: {error}")
            })?;
            operation
                .instance
                .map(NewInstanceOutcome::Created)
                .ok_or_else(|| "startup completed without an instance".into())
        })
    }
}
