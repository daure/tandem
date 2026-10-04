use crate::{
    environments::{Startup, startup},
    store::{
        environments::{Operation, OperationState},
        opencode::Session,
        rules::DispatchStatus,
    },
};

use super::AppService;

impl AppService {
    pub(crate) fn recreate_acceptance(
        &self,
        id: i64,
        conversation: Option<String>,
        confirmed: bool,
    ) -> tokio::sync::oneshot::Receiver<Result<(), String>> {
        let service = self.clone();
        let (sender, receiver) = tokio::sync::oneshot::channel();
        self.runtime.spawn(async move {
            let worker = service.clone();
            let result = tokio::task::spawn_blocking(move || {
                worker.begin_acceptance_recreation(id, conversation, confirmed)
            })
            .await
            .map_err(|error| error.to_string())
            .and_then(|result| result);
            let result = match result {
                Err(error) => {
                    let _ = sender.send(Err(error));
                    return;
                }
                Ok((operation, session)) => match service.wait_operation(&operation.id).await {
                    Ok(operation) if operation.state == OperationState::Succeeded => {
                        if let Some(session) = session {
                            service
                                .reopen_recreated_conversation(session, operation.name)
                                .await
                        } else {
                            Ok(())
                        }
                    }
                    Ok(operation) => Err(operation
                        .error
                        .unwrap_or_else(|| "Instance recreation failed".into())),
                    Err(error) => Err(error),
                },
            };
            service.refresh_environments();
            service.poll_rules();
            let _ = sender.send(result);
        });
        receiver
    }

    fn begin_acceptance_recreation(
        &self,
        id: i64,
        conversation: Option<String>,
        confirmed: bool,
    ) -> Result<(Operation, Option<Session>), String> {
        if !confirmed {
            return Err("confirmation_required: recreation creates a fresh workspace without restoring deleted files or runtime data".into());
        }
        if conversation.is_some() && !self.opencode_enabled() {
            return Err("OpenCode integration is disabled".into());
        }
        let store = &self.rules.store;
        let _lease = store
            .lease()
            .map_err(|error| error.to_string())?
            .ok_or("Rule work is in progress; retry recreation after it finishes")?;
        let requested = store.acceptance(id).map_err(|error| error.to_string())?;
        let _event = store
            .event_lease(requested.event_sequence)
            .map_err(|error| error.to_string())?
            .ok_or("Event work is in progress; retry recreation after it finishes")?;
        let snapshot = store.snapshot().map_err(|error| error.to_string())?;
        let acceptance = snapshot
            .acceptances
            .iter()
            .find(|row| row.id == id)
            .ok_or("Acceptance is unavailable")?;
        if store
            .provider_deleting(acceptance.event_sequence)
            .map_err(|error| error.to_string())?
        {
            return Err("The originating provider is being deleted".into());
        }
        let report = snapshot.reports.get(&id);
        if report.is_some_and(|report| {
            matches!(
                report.cleanup_state,
                crate::store::rules::reports::CleanupState::Pending
                    | crate::store::rules::reports::CleanupState::Purging
            )
        }) || (report.is_none()
            && matches!(
                acceptance.status,
                DispatchStatus::Queued | DispatchStatus::Provisioning | DispatchStatus::Launching
            ))
        {
            return Err("Dispatch is still active; wait before recreating its instance".into());
        }
        let directory = self
            .environments
            .config
            .workspaces
            .join(&acceptance.instance)
            .display()
            .to_string();
        if snapshot
            .workspaces
            .get(&id)
            .is_some_and(|workspace| workspace.directory != directory)
        {
            return Err(
                "The recorded workspace differs from the configured instance directory".into(),
            );
        }
        let session = conversation
            .as_ref()
            .map(|id| {
                snapshot
                    .workspaces
                    .get(&acceptance.id)
                    .and_then(|workspace| {
                        workspace.sessions.iter().find(|session| &session.id == id)
                    })
                    .cloned()
                    .ok_or("The retained conversation is unavailable".to_owned())
            })
            .transpose()?;
        let (existing, lock) = self
            .environments
            .admit_new_instance(&acceptance.instance, &acceptance.rule.definition.template)?;
        if existing.is_some()
            || startup::read(&self.environments.config, &acceptance.instance)?.is_some()
        {
            return Err(
                "The instance name is already in use; navigate to it or inspect its startup".into(),
            );
        }
        let description = format!("{}: {}", acceptance.rule_name, acceptance.event_summary);
        let origin = acceptance
            .operation_id
            .clone()
            .ok_or("Acceptance has no verifiable instance lineage; inspect before recreating")?;
        let operation = self.environments.begin_instance(
            &acceptance.instance,
            acceptance.rule.definition.template.clone(),
            Some(&description),
        )?;
        let operation = self.schedule_operation(
            operation,
            600,
            Startup {
                instance_lock: Some(lock),
                origin_operation_id: Some(origin),
                preserve_opencode_history: true,
                start_instance: acceptance.rule.definition.start_instance,
                description: Some(description),
                ..Default::default()
            },
        );
        Ok((operation, session))
    }

    async fn reopen_recreated_conversation(
        &self,
        session: Session,
        name: String,
    ) -> Result<(), String> {
        let settings = self.settings.clone();
        let observer = self.opencode.observer.clone();
        let current = self.opencode.current_zellij.clone();
        let reply = self.opencode.spawn_navigation(
            &self.runtime,
            "OpenCode navigation is already in progress",
            async move {
                if !settings.opencode_enabled() {
                    return Err("Instance recreated, but OpenCode integration is disabled".into());
                }
                observer.attach(&session, &name, &current, None).await
            },
        )?;
        reply.await.map_err(|error| error.to_string())?
    }
}
