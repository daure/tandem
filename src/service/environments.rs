use std::{path::Path, process::Stdio, sync::Arc};

#[cfg(not(test))]
use std::process::Command;

use super::AppService;
use super::refresh::Refresh;
use crate::{
    environments::{Environments, Startup},
    store::environments::{
        EnvironmentSnapshot, Instructions, Operation, OperationState, RuntimeInventory, Template,
    },
};

pub(crate) enum CreateInstanceOutcome {
    Existing,
    Started(Box<Operation>),
}

#[derive(Debug, Default)]
pub(crate) struct InstanceBatch {
    pub operations: Vec<Operation>,
    pub errors: Vec<String>,
}

impl AppService {
    pub(crate) fn environment_snapshot(&self) -> EnvironmentSnapshot {
        let mut snapshot = self.environments.snapshot();
        snapshot.startup_averages_milliseconds = self.settings.startup_averages();
        for instance in &snapshot.instances {
            if let Some(startup) = snapshot.startup.get_mut(&instance.name) {
                startup.estimate_milliseconds = snapshot
                    .startup_averages_milliseconds
                    .get(&instance.template)
                    .copied();
            }
        }
        snapshot
    }
    pub(crate) fn environment_keys(&self) -> [tuicore::KeySpec; 11] {
        self.environments.config.keys
    }

    pub(crate) fn poll_environments(&self) {
        self.refresh.request(Refresh::Instances);
    }

    pub(crate) fn refresh_environments(&self) {
        self.refresh.request(Refresh::All);
    }

    pub(crate) fn manual_refresh(
        &self,
    ) -> Result<tokio::sync::oneshot::Receiver<Result<(), String>>, String> {
        self.refresh.request_completion()
    }

    pub(crate) fn open_gateway(&self, url: &str) -> Result<(), String> {
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            return Err("gateway URL must use HTTP or HTTPS".into());
        }
        self.open_system_target(url)
    }

    pub(crate) fn open_workspace(
        &self,
        workspace: &str,
        instance: &str,
    ) -> Result<tokio::task::JoinHandle<Result<(), String>>, String> {
        if !Path::new(workspace).is_absolute() {
            return Err("workspace path must be absolute".into());
        }
        let workspace = workspace.to_owned();
        let instance = instance.to_owned();
        let settings = Arc::clone(&self.settings);
        let environments = Arc::clone(&self.environments);
        #[cfg(test)]
        let state = Arc::clone(&self.state);
        Ok(self.runtime.spawn(async move {
            let result = async {
                let target = workspace.clone();
                let name = instance.clone();
                tokio::task::spawn_blocking(move || {
                    environments.prepare_workspace_open(&target, &name)
                })
                .await
                .map_err(|error| error.to_string())??;
                let command = settings.read_open_command().await?;
                #[cfg(test)]
                if command.trim().is_empty() {
                    state.opened_system_targets.lock().unwrap().push(workspace);
                    return Ok(());
                }
                run_workspace_command(&command, &workspace, &instance).await
            }
            .await;
            if let Err(error) = &result {
                crate::diagnostics::record_error(
                    "cannot open workspace",
                    &std::io::Error::other(error.clone()),
                );
            }
            result
        }))
    }

    pub(crate) async fn run_open_command(
        &self,
        name: String,
        confirmed: bool,
    ) -> Result<String, String> {
        if !confirmed {
            return Err(
                "confirmation_required: the open command executes with local host privileges"
                    .into(),
            );
        }
        let workspace = self.environments.workspace(&name)?;
        self.open_workspace(&workspace, &name)?
            .await
            .map_err(|_| "workspace opener task failed".to_owned())??;
        Ok(workspace)
    }

    fn open_system_target(&self, target: &str) -> Result<(), String> {
        #[cfg(test)]
        {
            self.state
                .opened_system_targets
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .push(target.into());
            Ok(())
        }
        #[cfg(not(test))]
        {
            Command::new("xdg-open")
                .arg(target)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .map(|_| ())
                .map_err(|error| format!("cannot open system target: {error}"))
        }
    }

    #[cfg(test)]
    pub(crate) fn opened_system_targets(&self) -> Vec<String> {
        self.state
            .opened_system_targets
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    pub(crate) fn operations(&self) -> Vec<Operation> {
        self.environments.operations()
    }

    pub(crate) fn get_operation(&self, id: &str) -> Result<Operation, String> {
        self.environments.operation(id)
    }

    pub(crate) fn submit_new_instance(
        &self,
        name: &str,
        template: String,
    ) -> Result<CreateInstanceOutcome, String> {
        if self
            .environments
            .snapshot()
            .instances
            .iter()
            .any(|instance| instance.name == name)
        {
            return Ok(CreateInstanceOutcome::Existing);
        }
        self.submit_operation("create_instance", name, Some(template), 600, true)
            .map(|operation| CreateInstanceOutcome::Started(Box::new(operation)))
    }

    pub(crate) fn delete_instance(&self, name: &str) -> Result<(), String> {
        let operation = self.submit_operation("delete_instance", name, None, 60, true)?;
        let operation = self.runtime.block_on(self.wait_operation(&operation.id))?;
        if operation.state != OperationState::Succeeded {
            return Err(operation
                .error
                .unwrap_or_else(|| "instance deletion failed".into()));
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn queue_instance_for_tests(&self, name: &str, template: &str) -> Operation {
        self.environments
            .begin("create_instance", name, Some(template.into()))
            .unwrap()
    }

    #[cfg(test)]
    pub(crate) fn complete_instance_for_tests(
        &self,
        id: &str,
        instance: crate::store::environments::Instance,
    ) {
        self.environments.complete_instance_for_tests(id, instance);
    }

    pub(crate) fn submit_operation(
        &self,
        action: &str,
        name: &str,
        template: Option<String>,
        timeout: u64,
        confirmed: bool,
    ) -> Result<Operation, String> {
        if ![
            "create_instance",
            "stop_instance",
            "delete_instance",
            "stop_template",
            "delete_template",
            "remove_template",
            "create_template",
        ]
        .contains(&action)
        {
            return Err("unknown operation".into());
        }
        if action != "create_template" && !confirmed {
            return Err(
                "confirmation_required: this provisions repositories with host Git, executes Docker operations or removes local data"
                    .into(),
            );
        }
        if !(5..=900).contains(&timeout) {
            return Err("timeout_seconds must be between 5 and 900".into());
        }
        let operation = self.environments.begin(action, name, template)?;
        Ok(self.schedule_operation(operation, timeout, Startup::default()))
    }

    pub(crate) fn submit_instance_batch(
        &self,
        action: &str,
        names: &[String],
        confirmed: bool,
    ) -> Result<InstanceBatch, String> {
        if !matches!(action, "stop_instance" | "delete_instance") {
            return Err("unsupported instance batch operation".into());
        }
        if !confirmed {
            return Err(
                "confirmation_required: this stops containers or permanently removes instance data"
                    .into(),
            );
        }
        let mut batch = InstanceBatch::default();
        let names = names.iter().collect::<std::collections::BTreeSet<_>>();
        for name in names {
            match self.submit_operation(action, name, None, 60, true) {
                Ok(operation) => batch.operations.push(operation),
                Err(error) => batch.errors.push(format!("{name}: {error}")),
            }
        }
        Ok(batch)
    }

    pub(crate) fn submit_restart(
        &self,
        name: &str,
        service: Option<String>,
        confirmed: bool,
    ) -> Result<Operation, String> {
        if !confirmed {
            return Err("confirmation_required: restarting containers interrupts services".into());
        }
        let operation = self.environments.begin_restart(name, service)?;
        Ok(self.schedule_operation(operation, 600, Startup::default()))
    }

    pub(crate) fn submit_service_state(
        &self,
        name: &str,
        service: String,
        running: bool,
        confirmed: bool,
    ) -> Result<Operation, String> {
        if !confirmed {
            return Err(
                "confirmation_required: starting or stopping a service executes Docker operations"
                    .into(),
            );
        }
        let operation = self
            .environments
            .begin_service_state(name, service, running)?;
        Ok(self.schedule_operation(
            operation,
            if running { 600 } else { 60 },
            Startup::default(),
        ))
    }

    pub(super) fn schedule_operation(
        &self,
        operation: Operation,
        timeout: u64,
        mut startup: Startup,
    ) -> Operation {
        let action = operation.action.as_str();
        let operation_id = operation.id.clone();
        let environments = Arc::clone(&self.environments);
        let worker_operation = operation.clone();
        let startup_template = (action == "create_instance")
            .then(|| worker_operation.template.clone())
            .flatten();
        let notifier = self.refresh.notifier.clone();
        let refresh = Refresh::for_operation(action);
        startup.branch_instances = action == "create_instance" && self.branch_instances();
        let settings = Arc::clone(&self.settings);
        self.runtime.spawn_blocking(move || {
            notifier.publish(refresh);
            environments.execute(worker_operation, timeout, startup);
            if let Some(template) = startup_template
                && let Ok(operation) = environments.operation(&operation_id)
                && operation.state == OperationState::Succeeded
                && let Err(error) =
                    settings.record_startup(template, operation.elapsed_milliseconds)
            {
                crate::diagnostics::record_error(
                    "cannot save startup timing",
                    &std::io::Error::other(error),
                );
            }
            // Failed lifecycle operations can still leave changed Docker/filesystem state.
            notifier.publish(refresh);
        });
        operation
    }

    pub(crate) async fn wait_operation(&self, id: &str) -> Result<Operation, String> {
        loop {
            let operation = self.get_operation(id)?;
            if operation.state != OperationState::Running {
                return Ok(operation);
            }
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
    }

    async fn environment_call<T: Send + 'static>(
        &self,
        call: impl FnOnce(&Environments) -> Result<T, String> + Send + 'static,
    ) -> Result<T, String> {
        let environments = Arc::clone(&self.environments);
        tokio::task::spawn_blocking(move || call(&environments))
            .await
            .map_err(|_| "environment worker failed")?
    }

    pub(crate) async fn list_templates(&self) -> Result<Vec<Template>, String> {
        self.environment_call(Environments::list_templates).await
    }
    pub(crate) async fn get_template(&self, name: String) -> Result<Template, String> {
        self.environment_call(move |environments| environments.get_template(&name))
            .await
    }
    pub(crate) async fn create_template(&self, name: String) -> Result<Template, String> {
        let notifier = self.refresh.notifier.clone();
        self.environment_call(move |environments| {
            let result = environments.create_template(&name);
            if result.is_ok() {
                notifier.publish(Refresh::Templates);
            }
            result
        })
        .await
    }
    pub(crate) async fn update_template_manifest(
        &self,
        name: String,
        manifest: crate::store::environments::Manifest,
        confirmed: bool,
    ) -> Result<Template, String> {
        if !confirmed {
            return Err("confirmation_required: this replaces the shared template manifest".into());
        }
        let notifier = self.refresh.notifier.clone();
        self.environment_call(move |environments| {
            let result = environments.update_template_manifest(&name, manifest);
            if result.is_ok() {
                notifier.publish(Refresh::Templates);
            }
            result
        })
        .await
    }
    pub(crate) async fn list_instances(&self) -> Result<RuntimeInventory, String> {
        self.environment_call(Environments::list_instances).await
    }
    pub(crate) async fn get_instructions(&self) -> Result<Instructions, String> {
        self.environment_call(Environments::instructions).await
    }
}

async fn run_workspace_command(
    command: &str,
    workspace: &str,
    instance: &str,
) -> Result<(), String> {
    let status = spawn_workspace_command(command, workspace, instance)?
        .wait()
        .await
        .map_err(|error| format!("cannot wait for workspace opener: {error}"))?;
    if !status.success() {
        return Err(format!("workspace opener exited with {status}"));
    }
    Ok(())
}

pub(super) fn spawn_workspace_command(
    command: &str,
    workspace: &str,
    instance: &str,
) -> Result<tokio::process::Child, String> {
    let mut process = if command.trim().is_empty() {
        let mut process = tokio::process::Command::new("xdg-open");
        process.arg(workspace);
        process
    } else {
        let mut process = tokio::process::Command::new("sh");
        process
            .arg("-c")
            .arg(command)
            .env("TANDEM_INSTANCE", instance)
            .env("TANDEM_WORKSPACE", workspace)
            .current_dir(workspace);
        process
    };
    process
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("cannot launch workspace opener: {error}"))
}
