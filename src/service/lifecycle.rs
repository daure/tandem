use super::AppService;
use crate::{
    environments::Startup,
    store::environments::{Operation, OperationState},
};

impl AppService {
    pub(crate) fn start_instance(&self, name: &str) -> Result<Operation, String> {
        let (instance, instance_lock) = self.environments.admit_instance_start(name)?;
        let operation = self
            .environments
            .begin_instance(name, instance.template, None)?;
        let operation = self.schedule_operation(
            operation,
            600,
            Startup {
                instance_lock: Some(instance_lock),
                ..Default::default()
            },
        );
        self.complete_instance_operation(operation)
    }

    pub(crate) fn stop_instance(&self, name: &str) -> Result<Operation, String> {
        let operation = self.submit_operation("stop_instance", name, None, 60, true)?;
        self.complete_instance_operation(operation)
    }

    pub(crate) fn restart_instance(&self, name: &str) -> Result<Operation, String> {
        let operation = self.submit_restart(name, None, true)?;
        self.complete_instance_operation(operation)
    }

    fn complete_instance_operation(&self, operation: Operation) -> Result<Operation, String> {
        let operation = self.runtime.block_on(self.wait_operation(&operation.id))?;
        if operation.state != OperationState::Succeeded {
            let mut error = operation
                .error
                .unwrap_or_else(|| format!("{} failed for {}", operation.action, operation.name));
            for warning in operation.warnings {
                error.push_str(&format!("\nWarning: {warning}"));
            }
            return Err(error);
        }
        Ok(operation)
    }
}
