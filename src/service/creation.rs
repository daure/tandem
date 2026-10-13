use super::AppService;
use crate::environments::Startup;
use crate::store::environments::{Instance, OperationState};

pub(crate) struct NewInstanceRequest {
    pub template: String,
    pub opencode: Option<crate::store::opencode::Launch>,
    pub description: Option<String>,
    pub start_instance: bool,
    pub origin: Option<String>,
    pub session_receipt_requested: bool,
}

pub(crate) enum NewInstanceOutcome {
    Created(Instance, Option<crate::store::opencode::PromptedSession>),
    Existing(Instance, Option<crate::store::opencode::PromptedSession>),
    Failed {
        error: String,
        session: Option<crate::store::opencode::PromptedSession>,
    },
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
        let store = self.state_stream.clone();
        let origin = startup.session_origin.clone();
        let receipt_requested = startup.session_receipt_requested || origin.is_some();
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
                    async {
                        if !receipt_requested {
                            return super::opencode::actions::launch_workspace_opencode(
                                &settings,
                                &integration,
                                workspace,
                                &name,
                                &launch,
                                Some(instructions),
                            )
                            .await
                            .map(|()| None);
                        }
                        super::opencode::actions::validate_launch(&settings, &integration)?;
                        super::opencode::prompted::open_correlated_session(
                            &integration,
                            workspace,
                            &name,
                            &launch,
                            instructions,
                            &store,
                            origin.as_deref(),
                        )
                        .await
                        .map(Some)
                    },
                )
                .await
                .unwrap_or_else(|_| Err("OpenCode launch timed out".into()))
            });
            if let Some(writer) = writer {
                if let Err(error) = &result {
                    writer.progress(format!("OpenCode launch failed: {error}"));
                }
                let result = result.and_then(|receipt| {
                    let Some(receipt) = receipt else {
                        return Ok(());
                    };
                    let error = receipt.error.clone();
                    let accepted = launch.prompt.is_none()
                        && receipt.session_id.is_some()
                        && receipt.error.is_none()
                        || receipt.prompt_outcome.accepted();
                    writer.opencode_session(receipt)?;
                    if accepted {
                        Ok(())
                    } else {
                        Err(error.unwrap_or_else(|| {
                            "OpenCode session creation or input was not confirmed".into()
                        }))
                    }
                });
                writer.opencode_result(result);
            }
            Ok(())
        }));
    }

    pub(crate) fn new_instance(
        &self,
        name: &str,
        request: NewInstanceRequest,
    ) -> Result<NewInstanceOutcome, String> {
        let NewInstanceRequest {
            template,
            opencode,
            description,
            start_instance,
            origin,
            session_receipt_requested,
        } = request;
        crate::store::state_stream::validate_origin(origin.as_deref())?;
        let receipt_requested = session_receipt_requested || origin.is_some();
        if let Some(launch) = &opencode {
            launch.validate()?;
        }
        let (existing, instance_lock) = self.environments.admit_new_instance(name, &template)?;
        if opencode.is_some() {
            self.validate_opencode_launch()?;
        }
        if let Some(instance) = existing {
            let receipt = if let Some(launch) = opencode {
                let workspace = self.environments.workspace(name)?;
                if receipt_requested {
                    let instructions =
                        self.runtime.block_on(super::opencode::instance_guidance(
                            self.environments.clone(),
                            name.to_owned(),
                        ))?;
                    self.validate_opencode_launch()?;
                    Some(self.runtime.block_on(
                        super::opencode::prompted::open_correlated_session(
                            &self.opencode,
                            &workspace,
                            name,
                            &launch,
                            instructions,
                            &self.state_stream,
                            origin.as_deref(),
                        ),
                    )?)
                } else {
                    self.runtime
                        .block_on(super::opencode::actions::launch_instance_opencode(
                            self.environments.clone(),
                            &self.settings,
                            &self.opencode,
                            &workspace,
                            name,
                            &launch,
                        ))?;
                    None
                }
            } else {
                None
            };
            return Ok(NewInstanceOutcome::Existing(instance, receipt));
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
                session_origin: origin,
                session_receipt_requested: receipt_requested,
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
            let config = self.environments.config.clone();
            let target = name.to_owned();
            let receipt = self
                .runtime
                .spawn_blocking(move || {
                    crate::environments::startup::read(&config, &target)
                        .map(|record| record.and_then(|record| record.opencode_session))
                })
                .await
                .map_err(|error| error.to_string())??;
            let operation = match operation {
                Ok(operation) => operation,
                Err(error) => {
                    return Ok(NewInstanceOutcome::Failed {
                        error,
                        session: receipt,
                    });
                }
            };
            if operation.state != OperationState::Succeeded {
                let mut error = operation
                    .error
                    .unwrap_or_else(|| "instance startup failed".into());
                if let Err(open_error) = opened {
                    error.push_str(&format!("; OpenCode launch failed: {open_error}"));
                }
                return Ok(NewInstanceOutcome::Failed {
                    error,
                    session: receipt,
                });
            }
            if let Err(error) = opened {
                return Ok(NewInstanceOutcome::Failed {
                    error: format!("instance is ready, but OpenCode launch failed: {error}"),
                    session: receipt,
                });
            }
            let instance = operation
                .instance
                .ok_or_else(|| "startup completed without an instance".to_owned())?;
            Ok(NewInstanceOutcome::Created(instance, receipt))
        })
    }
}
