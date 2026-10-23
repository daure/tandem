use std::sync::Arc;

use crate::{
    service::AppService,
    store::opencode::{PromptedSession, SessionPrompt, WhenBusy, workspace_owner},
};

pub(in crate::service) async fn open_correlated_session(
    integration: &super::Integration,
    workspace: &str,
    name: &str,
    launch: &crate::store::opencode::Launch,
    instructions: &str,
    store: &Arc<crate::environments::state_stream::Store>,
    origin: Option<&str>,
) -> Result<PromptedSession, String> {
    let instructions = launch.session_instructions(instructions);
    let input = SessionPrompt {
        launch: launch.clone(),
        agent: None,
    };
    let directory = workspace.to_owned();
    let origin = origin.map(str::to_owned);
    let adapter = store.clone();
    let instance = name.to_owned();
    let request = tokio::task::spawn_blocking(move || {
        adapter.begin_creation(&directory, &instance, origin.as_deref())
    })
    .await
    .map_err(|error| error.to_string())??;
    let identity = store.identity_receipt(&request);
    let receipt = match integration
        .observer
        .prompt_new_session(
            workspace,
            name,
            &integration.current_zellij,
            &input,
            &instructions,
            identity.to_str(),
        )
        .await
    {
        Ok(receipt) => receipt,
        Err(error) => {
            let adapter = store.clone();
            let pending = request.clone();
            tokio::task::spawn_blocking(move || adapter.cancel_creation(&pending))
                .await
                .map_err(|error| error.to_string())??;
            return Err(error);
        }
    };
    let adapter = store.clone();
    let instance = name.to_owned();
    tokio::task::spawn_blocking(move || adapter.complete_creation(&request, &instance, receipt))
        .await
        .map_err(|error| error.to_string())?
}

impl AppService {
    pub(crate) fn prompt_instance_session(
        &self,
        name: String,
        input: SessionPrompt,
        confirmed: bool,
    ) -> Result<tokio::sync::oneshot::Receiver<Result<PromptedSession, String>>, String> {
        self.prompt_instance_session_with_origin(name, input, confirmed, None)
    }

    pub(crate) fn prompt_instance_session_with_origin(
        &self,
        name: String,
        input: SessionPrompt,
        confirmed: bool,
        origin: Option<String>,
    ) -> Result<tokio::sync::oneshot::Receiver<Result<PromptedSession, String>>, String> {
        crate::store::state_stream::validate_origin(origin.as_deref())?;
        if !confirmed {
            return Err("confirmation_required: approve the initial prompt and model costs".into());
        }
        input.validate()?;
        self.validate_opencode_launch()?;
        let environments = Arc::clone(&self.environments);
        let settings = Arc::clone(&self.settings);
        let observer = self.opencode.observer.clone();
        let current = self.opencode.current_zellij.clone();
        let store = self.state_stream.clone();
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
                let directory = instance.workspace.clone();
                let instance_name = name.clone();
                let request_store = store.clone();
                let request = tokio::task::spawn_blocking(move || {
                    request_store.begin_creation(&directory, &instance_name, origin.as_deref())
                })
                .await
                .map_err(|error| error.to_string())??;
                let result = match observer
                    .prompt_new_session(
                        &instance.workspace,
                        &name,
                        &current,
                        &input,
                        instructions,
                        store.identity_receipt(&request).to_str(),
                    )
                    .await
                {
                    Ok(result) => result,
                    Err(error) => {
                        let adapter = store.clone();
                        let pending = request.clone();
                        tokio::task::spawn_blocking(move || adapter.cancel_creation(&pending))
                            .await
                            .map_err(|error| error.to_string())??;
                        return Err(error);
                    }
                };
                tokio::task::spawn_blocking(move || {
                    store.complete_creation(&request, &name, result)
                })
                .await
                .map_err(|error| error.to_string())?
            },
        )
    }

    pub(crate) fn prompt_instance_session_with_origin_wait(
        &self,
        name: String,
        input: SessionPrompt,
        origin: Option<String>,
    ) -> Result<PromptedSession, String> {
        let reply = self.prompt_instance_session_with_origin(name, input, true, origin)?;
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
        self.prompt_session_with_origin(session_id, input, when_busy, confirmed, None)
    }

    pub(crate) fn prompt_session_with_origin(
        &self,
        session_id: String,
        input: SessionPrompt,
        when_busy: WhenBusy,
        confirmed: bool,
        origin: Option<String>,
    ) -> Result<tokio::sync::oneshot::Receiver<Result<PromptedSession, String>>, String> {
        crate::store::state_stream::validate_origin(origin.as_deref())?;
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
        let store = self.state_stream.clone();
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
                let target = session.clone();
                let name = instance.name.clone();
                tokio::task::spawn_blocking(move || {
                    store.associate_origin(&target, &name, origin.as_deref())
                })
                .await
                .map_err(|error| error.to_string())??;
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

    pub(crate) fn prompt_session_with_origin_wait(
        &self,
        session_id: String,
        input: SessionPrompt,
        when_busy: WhenBusy,
        origin: Option<String>,
    ) -> Result<PromptedSession, String> {
        let reply = self.prompt_session_with_origin(session_id, input, when_busy, true, origin)?;
        self.runtime.block_on(reply).map_err(|_| {
            "OpenCode worker stopped; inspect the conversation before retrying".to_owned()
        })?
    }
}
