use std::sync::Arc;

use super::AppService;
use crate::{
    environments::{InstanceScope, Startup},
    store::environments::{Operation, OperationState},
};

impl AppService {
    pub(crate) fn bind_instance_workspace(&self) -> Result<Arc<InstanceScope>, String> {
        self.environments.bind_instance_workspace().map(Arc::new)
    }

    pub(crate) async fn instance_self_action(
        &self,
        scope: Arc<InstanceScope>,
        start: bool,
    ) -> Result<Operation, String> {
        let service = self.clone();
        let operation = tokio::task::spawn_blocking(move || {
            let (instance, instance_lock) =
                service.environments.admit_instance_self(&scope, start)?;
            let operation = if start {
                service
                    .environments
                    .begin_instance(&instance.name, instance.template, None)?
            } else {
                service
                    .environments
                    .begin("stop_instance", &instance.name, None)?
            };
            Ok::<_, String>(service.schedule_operation(
                operation,
                if start { 600 } else { 60 },
                Startup {
                    instance_lock: Some(instance_lock),
                    ..Default::default()
                },
            ))
        })
        .await
        .map_err(|error| format!("instance MCP worker failed: {error}"))??;
        let operation = self.wait_operation(&operation.id).await?;
        if operation.state != OperationState::Succeeded {
            return Err(operation
                .error
                .unwrap_or_else(|| "instance lifecycle action failed".into()));
        }
        Ok(operation)
    }
}
