use super::AppService;
use crate::environments::Startup;
use crate::store::environments::{Instance, OperationState};

pub(crate) enum NewInstanceOutcome {
    Created(Instance),
    Existing(Instance),
}

impl AppService {
    pub(super) fn configure_startup_opencode(&self, startup: &mut Startup, name: &str) {
        let Some(launch) = startup.opencode.take() else {
            return;
        };
        let settings = std::sync::Arc::clone(&self.settings);
        let integration = std::sync::Arc::clone(&self.opencode);
        let writer = startup.writer.clone();
        let runtime = self.runtime.handle().clone();
        let name = name.to_owned();
        let environments = std::sync::Arc::clone(&self.environments);
        startup.before_repositories = Some(Box::new(move |workspace, deadline| {
            settings.refresh()?;
            if let Some(writer) = &writer {
                writer.progress("Launching OpenCode before repository preparation".into());
            }
            let result = runtime.block_on(async {
                let instructions =
                    super::opencode::instance_guidance(environments, name.clone()).await?;
                tokio::time::timeout(
                    deadline.saturating_duration_since(std::time::Instant::now()),
                    super::opencode::launch_workspace_opencode(
                        &settings,
                        &integration,
                        workspace,
                        &name,
                        &launch,
                        Some(instructions),
                    ),
                )
                .await
                .unwrap_or_else(|_| Err("OpenCode launch timed out".into()))
            });
            if let Some(writer) = writer {
                if let Err(error) = &result {
                    writer.progress(format!("OpenCode launch failed: {error}"));
                }
                writer.opencode_result(result);
            }
            Ok(())
        }));
    }

    pub(crate) fn new_instance(
        &self,
        name: &str,
        template: String,
        opencode: Option<crate::store::opencode::Launch>,
        description: Option<String>,
        start_instance: bool,
    ) -> Result<NewInstanceOutcome, String> {
        if let Some(launch) = &opencode {
            launch.validate()?;
        }
        let (existing, instance_lock) = self.environments.admit_new_instance(name, &template)?;
        if opencode.is_some() {
            self.validate_opencode_launch()?;
        }
        if let Some(instance) = existing {
            if let Some(launch) = opencode {
                let workspace = self.environments.workspace(name)?;
                self.runtime
                    .block_on(self.launch_instance_opencode(&workspace, name, &launch))?;
            }
            return Ok(NewInstanceOutcome::Existing(instance));
        }
        let (sender, ready) = tokio::sync::oneshot::channel();
        let open_requested = opencode.is_some();
        let operation = self
            .environments
            .begin_instance(name, template, description.as_deref())?;
        // Keep admission's lock through startup so another process cannot create between
        // the existence check and Compose execution.
        let operation = self.schedule_operation(
            operation,
            600,
            Startup {
                start_instance,
                instance_lock: Some(instance_lock),
                description: description.clone(),
                opencode,
                opencode_result: open_requested.then_some(sender),
                ..Default::default()
            },
        );
        self.runtime.block_on(async {
            let open = async {
                if !open_requested {
                    return Ok(());
                }
                ready
                    .await
                    .map_err(|_| "OpenCode launch result unavailable".to_owned())?
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
