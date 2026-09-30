use std::sync::Arc;

#[cfg(not(test))]
use std::process::{Command, Stdio};

use super::AppService;
use super::refresh::Refresh;
use crate::{
    environments::{Environments, Startup},
    store::environments::{
        EnvironmentSnapshot, Instructions, Operation, OperationState, RuntimeInventory,
        StartupKind, Template,
    },
};

pub(crate) enum CreateInstanceOutcome {
    Existing,
    Started {
        operation: Box<Operation>,
        opencode: Option<tokio::sync::oneshot::Receiver<Result<(), String>>>,
    },
}

#[derive(Debug, Default)]
pub(crate) struct InstanceBatch {
    pub operations: Vec<Operation>,
    pub errors: Vec<String>,
}

impl AppService {
    pub(crate) fn environment_snapshot(&self) -> EnvironmentSnapshot {
        let mut snapshot = self.environments.snapshot();
        snapshot.cold_startup_averages_milliseconds =
            self.settings.startup_averages(StartupKind::Cold);
        snapshot.hot_startup_averages_milliseconds =
            self.settings.startup_averages(StartupKind::Hot);
        for instance in &snapshot.instances {
            if let Some(startup) = snapshot.startup.get_mut(&instance.name) {
                startup.estimate_milliseconds = match startup.kind {
                    StartupKind::Cold => snapshot
                        .cold_startup_averages_milliseconds
                        .get(&instance.template),
                    StartupKind::Hot => snapshot
                        .hot_startup_averages_milliseconds
                        .get(&instance.template),
                }
                .copied();
            }
        }
        snapshot
    }
    pub(crate) fn environment_keys(&self) -> [tuicore::KeySpec; 10] {
        self.environments.config.keys
    }

    pub(crate) fn poll_environments(&self) {
        self.refresh.request(Refresh::Instances);
    }

    pub(crate) fn update_instance_description(
        &self,
        name: String,
        description: String,
    ) -> tokio::sync::oneshot::Receiver<Result<(), String>> {
        let environments = Arc::clone(&self.environments);
        let (sender, receiver) = tokio::sync::oneshot::channel();
        self.runtime.spawn_blocking(move || {
            let _ = sender.send(environments.update_instance_description(&name, description));
        });
        receiver
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
        description: String,
        opencode: Option<Option<String>>,
    ) -> Result<CreateInstanceOutcome, String> {
        if let Some(instance) = self
            .environments
            .snapshot()
            .instances
            .iter()
            .find(|instance| instance.name == name)
        {
            if !instance.workspace_only
                || instance.runtime.workspace_ready
                || instance
                    .runtime
                    .activity
                    .as_ref()
                    .is_some_and(|activity| activity.active())
            {
                return Ok(CreateInstanceOutcome::Existing);
            }
            if instance.template != template {
                return Err("instance name belongs to another template".into());
            }
        }
        if opencode.is_some() {
            self.validate_opencode_launch()?;
        }
        let operation = self
            .environments
            .begin_instance(name, template, Some(&description))?;
        let (sender, ready) = tokio::sync::oneshot::channel();
        let open_requested = opencode.is_some();
        let operation = self.schedule_operation(
            operation,
            600,
            Startup {
                description: Some(description),
                opencode,
                opencode_result: open_requested.then_some(sender),
                ..Default::default()
            },
        );
        Ok(CreateInstanceOutcome::Started {
            operation: Box::new(operation),
            opencode: open_requested.then_some(ready),
        })
    }

    pub(crate) fn delete_instance(&self, name: &str) -> Result<Vec<String>, String> {
        let operation = self.environments.begin("delete_instance", name, None)?;
        let operation = self.schedule_operation(operation, 60, Startup::default());
        let operation = self.runtime.block_on(self.wait_operation(&operation.id))?;
        if operation.state != OperationState::Succeeded {
            let mut error = operation
                .error
                .unwrap_or_else(|| "instance deletion failed".into());
            for warning in operation.warnings {
                error.push_str(&format!("\nWarning: {warning}"));
            }
            return Err(error);
        }
        Ok(operation.warnings)
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
        if action == "create_instance" && startup.writer.is_none() {
            startup.branch_instances = self.branch_instances();
            return self.schedule_startup(operation, timeout, startup);
        }
        let operation_id = operation.id.clone();
        let environments = Arc::clone(&self.environments);
        let worker_operation = operation.clone();
        let startup_timing = (action == "create_instance")
            .then(|| {
                worker_operation
                    .template
                    .clone()
                    .zip(self.environments.startup_kind(&worker_operation.id))
            })
            .flatten();
        let notifier = self.refresh.notifier.clone();
        let refresh = Refresh::for_operation(action);
        if action == "create_instance" {
            let settings = Arc::clone(&self.settings);
            let runtime = self.runtime.handle().clone();
            startup.before_creation = Some(Box::new(move |workspace, deadline| {
                settings.refresh()?;
                if settings.opencode_enabled() && settings.clear_opencode_history() {
                    runtime.block_on(crate::environments::opencode::clear_history(
                        workspace, deadline,
                    ))?;
                }
                Ok(())
            }));
            self.configure_startup_opencode(&mut startup, &operation.name);
        }
        let settings = Arc::clone(&self.settings);
        let observer = self.opencode.observer.clone();
        let runtime = self.runtime.handle().clone();
        self.runtime.spawn_blocking(move || {
            notifier.publish(refresh);
            environments.execute(
                worker_operation,
                timeout,
                startup,
                &|workspace, deadline| {
                    settings.refresh()?;
                    if settings.opencode_enabled() {
                        runtime.block_on(observer.close_workspace(workspace, deadline))?;
                    }
                    Ok(())
                },
            );
            if let Some((template, kind)) = startup_timing
                && let Ok(operation) = environments.operation(&operation_id)
                && operation.state == OperationState::Succeeded
            {
                match settings.record_startup(template, kind, operation.elapsed_milliseconds) {
                    Ok(()) => notifier.publish(Refresh::Settings),
                    Err(error) => crate::diagnostics::record_error(
                        "cannot save startup timing",
                        &std::io::Error::other(error),
                    ),
                }
            }
            // Failed lifecycle operations can still leave changed Docker/filesystem state.
            notifier.publish(refresh);
        });
        operation
    }

    pub(crate) async fn wait_operation(&self, id: &str) -> Result<Operation, String> {
        loop {
            let operation = self.get_operation_fresh(id.to_owned()).await?;
            if operation.state != OperationState::Running {
                return Ok(operation);
            }
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
    }

    pub(crate) async fn get_operation_fresh(&self, id: String) -> Result<Operation, String> {
        self.environment_call(move |environments| {
            environments.refresh_startups()?;
            environments.operation(&id)
        })
        .await
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
    pub(crate) async fn inspect_instance(
        &self,
        name: String,
    ) -> Result<crate::store::environments::InstanceInspection, String> {
        self.environment_call(move |environments| environments.inspect_instance(&name))
            .await
    }
    pub(crate) async fn get_instructions(&self) -> Result<Instructions, String> {
        self.environment_call(Environments::instructions).await
    }
}
