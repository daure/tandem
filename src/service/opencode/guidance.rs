use std::sync::Arc;

use crate::{
    environments::{Environments, startup},
    store::environments::{Instance, OperationState},
};

pub(in crate::service) async fn instance_guidance(
    environments: Arc<Environments>,
    name: String,
) -> Result<&'static str, String> {
    tokio::task::spawn_blocking(move || {
        let instance = environments.session_instance(&name)?;
        if instance.workspace_only {
            return Ok(instance.session_instructions());
        }
        if let Some(record) = startup::read(&environments.config, &name)? {
            let record = record.observe(&environments.config)?;
            if record.operation.state == OperationState::Running {
                return Ok(Instance::startup_instructions(record.start_instance));
            }
        }
        Ok(instance.session_instructions())
    })
    .await
    .map_err(|error| error.to_string())?
}
