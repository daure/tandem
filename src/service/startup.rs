use std::{sync::Arc, time::Duration};

use super::AppService;
use crate::{
    environments::{Startup, config::Config, startup},
    store::environments::{Operation, OperationState},
};

impl AppService {
    pub(super) fn schedule_startup(
        &self,
        operation: Operation,
        timeout: u64,
        mut request: Startup,
    ) -> Operation {
        match startup::launch(&self.environments, &operation, timeout, &mut request) {
            Ok(mut child) => {
                std::thread::spawn(move || {
                    if let Err(error) = child.wait() {
                        crate::diagnostics::record_error("cannot reap startup worker", &error);
                    }
                });
            }
            Err(error) => {
                self.environments
                    .finish_operation(&operation.id, Err(error));
                return self
                    .environments
                    .operation(&operation.id)
                    .unwrap_or(operation);
            }
        }
        let Some(ready) = request.workspace_ready else {
            return operation;
        };
        let config = self.environments.config.clone();
        let id = operation.id.clone();
        let name = operation.name.clone();
        self.runtime.spawn(async move {
            loop {
                let read_config = config.clone();
                let name = name.clone();
                let observed = tokio::task::spawn_blocking(move || {
                    startup::read(&read_config, &name)?
                        .map(|record| record.observe(&read_config))
                        .transpose()
                })
                .await;
                match observed {
                    Ok(Ok(Some(record))) if record.operation.id == id => {
                        if record.workspace_ready {
                            let _ = ready.send(
                                config
                                    .workspaces
                                    .join(&record.operation.name)
                                    .display()
                                    .to_string(),
                            );
                            break;
                        }
                        if record.operation.state != OperationState::Running {
                            break;
                        }
                    }
                    _ => break,
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        });
        operation
    }

    pub(crate) fn run_startup_worker(
        name: &str,
        id: &str,
        instance_fd: i32,
        lease_fd: i32,
    ) -> Result<(), String> {
        let mut config = Config::from_env()?;
        config.operation_id = Some(id.into());
        let claimed = startup::claim(&config, name, id, instance_fd, lease_fd)?;
        let service = Self::from_config(config).map_err(|error| error.to_string())?;
        service.environments.adopt_startup(&claimed.record);
        let remaining = claimed.record.timeout.saturating_sub(
            (chrono::Utc::now().timestamp().max(0) as u64)
                .saturating_sub(claimed.record.started_at),
        );
        if remaining == 0 {
            service.environments.finish_operation(
                id,
                Err("Startup deadline reached before worker initialization".into()),
            );
        } else {
            service.schedule_operation(
                claimed.record.operation.clone(),
                remaining,
                Startup {
                    instance_lock: Some(claimed.instance_lock.borrowed()?),
                    branch_instances: claimed.record.branch_instances,
                    description: claimed.record.description.clone(),
                    writer: Some(Arc::clone(&claimed.writer)),
                    ..Default::default()
                },
            );
        }
        let operation = service.runtime.block_on(service.wait_operation(id))?;
        claimed.writer.finish(operation.clone())?;
        if let Some(error) = operation.error {
            crate::diagnostics::record_error(
                "instance startup failed",
                &std::io::Error::other(format!("{id}: {error}")),
            );
        }
        Ok(())
    }
}
