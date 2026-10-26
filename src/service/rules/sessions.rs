use super::{Acceptance, AppService, DispatchStatus};
use crate::store::{
    environments::{EnvironmentSnapshot, Instance},
    opencode::{PromptOutcome, Session, Snapshot},
};

impl AppService {
    pub(super) fn launch_named_rule_session(
        &self,
        acceptance: &mut Acceptance,
        instance: &Instance,
        destination: &str,
        assigned: bool,
    ) -> Result<(), String> {
        let store = &self.rules.store;
        let name = acceptance
            .rule
            .definition
            .session_name
            .as_deref()
            .ok_or("session name unavailable")?;
        let identity = acceptance
            .target_workspace_identity
            .as_deref()
            .ok_or("workspace identity unavailable")?;
        let mut binding = store
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
                .block_on(self.opencode.observer.named_rule_session_exists(&session))?
            {
                store
                    .finish_named_session(saved, true)
                    .map_err(|error| error.to_string())?;
            } else if saved.ready {
                binding = None;
            } else {
                return Err("Named conversation creation is uncertain; inspect the client before using another session name".into());
            }
        }
        let existing = binding.as_ref().map(|saved| Session {
            id: saved.session_id.clone(),
            server: saved.server.clone(),
            directory: instance.workspace.clone(),
            ..Default::default()
        });
        let client =
            self.runtime
                .handle()
                .block_on(self.opencode.observer.prepare_named_rule_prompt(
                    &instance.workspace,
                    &instance.name,
                    destination,
                    existing.as_ref(),
                    acceptance.rule.definition.focus_pane,
                ))?;
        let creating = binding.is_none();
        let binding = match binding {
            Some(binding) => binding,
            None => store
                .reserve_named_session(&instance.name, name, identity, &client.server)
                .map_err(|error| error.to_string())?,
        };
        let session = Session {
            id: binding.session_id.clone(),
            server: binding.server.clone(),
            directory: instance.workspace.clone(),
            title: name.into(),
            ..Default::default()
        };
        acceptance.status = DispatchStatus::Launching;
        acceptance.launch_started_at = Some(chrono::Utc::now().to_rfc3339());
        acceptance.session_id = Some(session.id.clone());
        acceptance.session_server = Some(session.server.clone());
        store
            .update(acceptance, assigned)
            .map_err(|error| error.to_string())?;
        let receipt =
            self.runtime
                .handle()
                .block_on(self.opencode.observer.send_named_rule_prompt(
                    client,
                    destination,
                    &session,
                    acceptance,
                    instance.session_instructions(),
                    creating,
                ))?;
        let known = receipt.session_id.as_deref() == Some(session.id.as_str())
            && receipt.server.as_deref() == Some(session.server.as_str());
        if known || receipt.creation_confirmed_absent {
            store
                .finish_named_session(&binding, known)
                .map_err(|error| error.to_string())?;
        }
        acceptance.pane = receipt.pane;
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
        if known {
            store
                .remember_workspaces(
                    &EnvironmentSnapshot {
                        instances: vec![instance.clone()],
                        ..Default::default()
                    },
                    &Snapshot {
                        sessions: vec![session],
                        ..Default::default()
                    },
                )
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    }
}
