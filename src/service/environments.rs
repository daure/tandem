use std::{path::Path, process::Stdio, sync::Arc};

#[cfg(not(test))]
use std::process::Command;

use super::AppService;
use crate::{
    environments::Environments,
    store::environments::{
        EnvironmentSnapshot, Instance, Instructions, Operation, OperationState, Template,
    },
};

impl AppService {
    pub(crate) fn environment_snapshot(&self) -> EnvironmentSnapshot {
        self.environments.snapshot()
    }
    pub(crate) fn environment_keys(&self) -> [char; 7] {
        self.environments.config.keys
    }

    pub(crate) fn poll_environments(&self) {
        self.refresh_open_command();
        if !self.environments.begin_refresh() {
            return;
        }
        let environments = Arc::clone(&self.environments);
        self.runtime.spawn_blocking(move || {
            environments.refresh();
            if let Some(request) = environments.begin_resource_sample() {
                environments.sample_resources(request);
            }
        });
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
        #[cfg(test)]
        let state = Arc::clone(&self.state);
        Ok(self.runtime.spawn(async move {
            let result = async {
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

    #[cfg(test)]
    pub(crate) fn operations(&self) -> Vec<Operation> {
        self.environments.operations()
    }

    pub(crate) fn get_operation(&self, id: &str) -> Result<Operation, String> {
        self.environments.operation(id)
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
                "confirmation_required: this executes Docker operations or removes local data"
                    .into(),
            );
        }
        if !(5..=900).contains(&timeout) {
            return Err("timeout_seconds must be between 5 and 900".into());
        }
        let operation = self.environments.begin(action, name, template)?;
        let environments = Arc::clone(&self.environments);
        let worker_operation = operation.clone();
        let branch_instances = action == "create_instance" && self.branch_instances();
        self.runtime.spawn_blocking(move || {
            environments.execute(worker_operation, timeout, branch_instances)
        });
        Ok(operation)
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
        self.environment_call(move |environments| environments.create_template(&name))
            .await
    }
    pub(crate) async fn list_instances(&self) -> Result<Vec<Instance>, String> {
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
    let status = process
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .map_err(|error| format!("cannot launch workspace opener: {error}"))?;
    if !status.success() {
        return Err(format!("workspace opener exited with {status}"));
    }
    Ok(())
}
