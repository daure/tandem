use std::sync::Arc;

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
    pub(crate) fn environment_keys(&self) -> [char; 5] {
        self.environments.config.keys
    }

    pub(crate) fn poll_environments(&self) {
        if !self.environments.begin_refresh() {
            return;
        }
        let environments = Arc::clone(&self.environments);
        self.runtime.spawn_blocking(move || environments.refresh());
    }

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
        if !["create_instance", "stop_instance", "create_template"].contains(&action) {
            return Err("unknown operation".into());
        }
        if action != "create_template" && !confirmed {
            return Err(
                "confirmation_required: this executes or removes local Docker containers".into(),
            );
        }
        if !(5..=900).contains(&timeout) {
            return Err("timeout_seconds must be between 5 and 900".into());
        }
        let operation = self.environments.begin(action, name, template)?;
        let environments = Arc::clone(&self.environments);
        let worker_operation = operation.clone();
        self.runtime
            .spawn_blocking(move || environments.execute(worker_operation, timeout));
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
