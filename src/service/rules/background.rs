use super::{Acceptance, AppService, DispatchStatus};
use crate::store::{
    environments::{EnvironmentSnapshot, Instance},
    opencode::{PromptOutcome, Session, SessionPrompt, Snapshot},
    rules::DeliveryMode,
};

impl AppService {
    pub(super) fn launch_background_rule_session(
        &self,
        acceptance: &mut Acceptance,
        instance: &Instance,
        assigned: bool,
    ) -> Result<(), String> {
        let store = &self.rules.store;
        let observer = &self.opencode.observer;
        let mut binding = None;
        if let Some(name) = &acceptance.rule.definition.session_name {
            let identity = acceptance
                .target_workspace_identity
                .as_deref()
                .ok_or("workspace identity unavailable")?;
            binding = store
                .named_session(&instance.name, name, identity)
                .map_err(|error| error.to_string())?;
            if let Some(saved) = &binding {
                let session = Session {
                    id: saved.session_id.clone(),
                    server: saved.server.clone(),
                    directory: instance.workspace.clone(),
                    ..Default::default()
                };
                if self
                    .runtime
                    .handle()
                    .block_on(observer.named_rule_session_exists(&session))?
                {
                    store
                        .finish_named_session(saved, true)
                        .map_err(|error| error.to_string())?;
                } else if saved.ready {
                    binding = None;
                } else {
                    return Err("Named conversation creation is uncertain; inspect server history before replaying".into());
                }
            }
        }
        let creating = binding.is_none();
        let server = match &binding {
            Some(saved) => saved.server.clone(),
            None => self
                .runtime
                .handle()
                .block_on(observer.background_server(&instance.workspace))?,
        };
        if creating && let Some(name) = &acceptance.rule.definition.session_name {
            binding = Some(
                store
                    .reserve_named_session(
                        &instance.name,
                        name,
                        acceptance
                            .target_workspace_identity
                            .as_deref()
                            .ok_or("workspace identity unavailable")?,
                        &server,
                    )
                    .map_err(|error| error.to_string())?,
            );
        }
        let session = Session {
            id: binding
                .as_ref()
                .map(|saved| saved.session_id.clone())
                .unwrap_or_default(),
            server,
            directory: instance.workspace.clone(),
            title: acceptance
                .rule
                .definition
                .session_name
                .clone()
                .unwrap_or_default(),
            ..Default::default()
        };
        acceptance.status = DispatchStatus::Launching;
        acceptance.launch_started_at = Some(chrono::Utc::now().to_rfc3339());
        acceptance.session_id = (!session.id.is_empty()).then_some(session.id.clone());
        acceptance.session_server = Some(session.server.clone());
        store
            .update(acceptance, assigned)
            .map_err(|error| error.to_string())?;
        let rule = acceptance.launch_definition();
        let input = SessionPrompt {
            launch: crate::store::opencode::Launch {
                prompt: Some(acceptance.session_prompt(creating)?.into()),
                ..rule.session_launch()
            },
            agent: None,
        };
        let receipt = if creating {
            self.runtime
                .handle()
                .block_on(crate::service::opencode::background::create_session(
                    observer,
                    &self.state_stream,
                    &session,
                    crate::service::opencode::background::BackgroundPrompt {
                        input: &input,
                        instructions: instance.session_instructions(),
                        delivery: if rule.session_name.is_some() {
                            rule.delivery_mode
                        } else {
                            DeliveryMode::Steer
                        },
                    },
                    &instance.name,
                    None,
                ))?
        } else {
            self.runtime
                .handle()
                .block_on(observer.submit_background_prompt(
                    &session,
                    &input,
                    instance.session_instructions(),
                    rule.delivery_mode,
                    true,
                ))
        };
        if let Some(binding) = &binding
            && receipt.session_id.is_some()
        {
            store
                .finish_named_session(binding, true)
                .map_err(|error| error.to_string())?;
        }
        acceptance.session_id = receipt.session_id.clone();
        acceptance.session_server = receipt.server.clone();
        acceptance.pane = None;
        acceptance.prompt_outcome = Some(receipt.prompt_outcome);
        acceptance.error = receipt.error;
        acceptance.status = match receipt.prompt_outcome {
            PromptOutcome::Submitted | PromptOutcome::Queued => DispatchStatus::Launched,
            PromptOutcome::NotSubmitted => DispatchStatus::Failed,
            PromptOutcome::Uncertain => DispatchStatus::Uncertain,
        };
        store
            .update(acceptance, false)
            .map_err(|error| error.to_string())?;
        if let Some(id) = receipt.session_id {
            store
                .remember_workspaces(
                    &EnvironmentSnapshot {
                        instances: vec![instance.clone()],
                        ..Default::default()
                    },
                    &Snapshot {
                        sessions: vec![Session { id, ..session }],
                        ..Default::default()
                    },
                )
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    }
}
