use std::sync::Arc;

use crate::{
    service::AppService,
    store::opencode::{PromptedSession, SessionPrompt, WhenBusy, workspace_owner},
};

impl AppService {
    pub(crate) fn prompt_instance_session(
        &self,
        name: String,
        input: SessionPrompt,
        confirmed: bool,
    ) -> Result<tokio::sync::oneshot::Receiver<Result<PromptedSession, String>>, String> {
        if !confirmed {
            return Err("confirmation_required: approve the initial prompt and model costs".into());
        }
        input.validate()?;
        self.validate_opencode_launch()?;
        let environments = Arc::clone(&self.environments);
        let settings = Arc::clone(&self.settings);
        let observer = self.opencode.observer.clone();
        let current = self.opencode.current_zellij.clone();
        self.opencode.spawn_navigation(
            &self.runtime,
            "OpenCode action is already in progress",
            async move {
                let target = name.clone();
                let adapter = Arc::clone(&environments);
                let (instance, _lock) =
                    tokio::task::spawn_blocking(move || adapter.admit_session_open(&target))
                        .await
                        .map_err(|error| error.to_string())??;
                let instructions = super::instance_guidance(environments, name.clone()).await?;
                if !settings.opencode_enabled() {
                    return Err("OpenCode integration is disabled".into());
                }
                observer
                    .prompt_new_session(&instance.workspace, &name, &current, &input, instructions)
                    .await
            },
        )
    }

    pub(crate) fn prompt_instance_session_wait(
        &self,
        name: String,
        input: SessionPrompt,
    ) -> Result<PromptedSession, String> {
        let reply = self.prompt_instance_session(name, input, true)?;
        self.runtime
            .block_on(reply)
            .map_err(|_| "OpenCode worker stopped; check the client before retrying".to_owned())?
    }

    pub(crate) fn prompt_session(
        &self,
        session_id: String,
        input: SessionPrompt,
        when_busy: WhenBusy,
        confirmed: bool,
    ) -> Result<tokio::sync::oneshot::Receiver<Result<PromptedSession, String>>, String> {
        if !confirmed {
            return Err(
                "confirmation_required: approve task input, model costs and any interruption"
                    .into(),
            );
        }
        input.validate()?;
        self.validate_opencode_launch()?;
        let environments = Arc::clone(&self.environments);
        let settings = Arc::clone(&self.settings);
        let observer = self.opencode.observer.clone();
        let current = self.opencode.current_zellij.clone();
        let mut known = self.opencode_snapshot().sessions;
        known.extend(
            self.rule_snapshot()
                .workspaces
                .into_values()
                .flat_map(|workspace| workspace.sessions),
        );
        self.opencode.spawn_navigation(
            &self.runtime,
            "OpenCode action is already in progress",
            async move {
                let session = observer.resolve_prompt_session(&session_id, known).await?;
                let adapter = Arc::clone(&environments);
                let directory = session.directory.clone();
                let (instance, _lock) = tokio::task::spawn_blocking(move || {
                    let instances = adapter.list_instances()?.instances;
                    let name = workspace_owner(
                        &directory,
                        instances
                            .iter()
                            .map(|instance| (instance.name.as_str(), instance.workspace.as_str())),
                    )
                    .ok_or("OpenCode conversation is outside an owned Tandem instance")?;
                    let (instance, lock) = adapter.admit_session_open(name)?;
                    let root = std::fs::canonicalize(&instance.workspace)
                        .map_err(|error| error.to_string())?;
                    let target =
                        std::fs::canonicalize(&directory).map_err(|error| error.to_string())?;
                    if !target.starts_with(root) {
                        return Err("OpenCode conversation workspace ownership changed".into());
                    }
                    Ok::<_, String>((instance, lock))
                })
                .await
                .map_err(|error| error.to_string())??;
                let instructions =
                    super::instance_guidance(environments, instance.name.clone()).await?;
                if !settings.opencode_enabled() {
                    return Err("OpenCode integration is disabled".into());
                }
                observer
                    .prompt_existing_session(
                        &session,
                        &instance.name,
                        &current,
                        &input,
                        when_busy,
                        instructions,
                    )
                    .await
            },
        )
    }

    pub(crate) fn prompt_session_wait(
        &self,
        session_id: String,
        input: SessionPrompt,
        when_busy: WhenBusy,
    ) -> Result<PromptedSession, String> {
        let reply = self.prompt_session(session_id, input, when_busy, true)?;
        self.runtime.block_on(reply).map_err(|_| {
            "OpenCode worker stopped; inspect the conversation before retrying".to_owned()
        })?
    }
}
