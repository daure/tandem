use std::sync::Arc;

use super::AppService;
use crate::{
    environments::{InstanceScope, Startup, conclusion, config::Config},
    store::{
        environments::{InstanceInstructions, Operation, OperationState},
        rules::reports::{CleanupState, Report, ReportInput, ReportSummary},
    },
};

impl AppService {
    pub(crate) async fn get_instance_instructions(
        &self,
        scope: Arc<InstanceScope>,
    ) -> Result<InstanceInstructions, String> {
        let environments = self.environments.clone();
        tokio::task::spawn_blocking(move || environments.instance_instructions(&scope))
            .await
            .map_err(|error| format!("instance guidance worker failed: {error}"))?
    }

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

    pub(crate) async fn conclude_instance(
        &self,
        scope: Arc<InstanceScope>,
        input: ReportInput,
    ) -> Result<ReportSummary, String> {
        input.validate()?;
        let service = self.clone();
        tokio::task::spawn_blocking(move || {
            let rules = service
                .rules
                .store
                .lease()
                .map_err(|error| error.to_string())?
                .ok_or("rule dispatch is busy; retry conclusion after it finishes")?;
            let (instance, instance_lock) =
                service.environments.admit_instance_conclusion(&scope)?;
            let acceptance = service
                .rules
                .store
                .instance_acceptance(&instance)
                .map_err(|error| error.to_string())?;
            let completion = conclusion::reserve(&service.environments.config, acceptance.id)?;
            let report = service
                .rules
                .store
                .save_report(acceptance.id, &input)
                .map_err(|error| error.to_string())?;
            match conclusion::launch(
                &service.environments.config,
                acceptance.id,
                &instance.name,
                instance_lock,
                rules,
                completion,
            ) {
                Ok(mut child) => {
                    std::thread::spawn(move || {
                        if let Err(error) = child.wait() {
                            crate::diagnostics::record_error(
                                "cannot reap conclusion worker",
                                &error,
                            );
                        }
                    });
                    Ok(report.details)
                }
                Err(error) => {
                    service
                        .rules
                        .store
                        .set_report_cleanup(acceptance.id, CleanupState::Failed, Some(&error))
                        .map_err(|error| error.to_string())?;
                    Err(format!(
                        "Report saved for acceptance {}; cleanup could not start: {error}",
                        acceptance.id
                    ))
                }
            }
        })
        .await
        .map_err(|error| format!("conclusion worker failed: {error}"))?
    }

    pub(crate) async fn search_event_reports(
        &self,
        strings: Vec<String>,
    ) -> Result<Vec<ReportSummary>, String> {
        let rules = self.rules.clone();
        tokio::task::spawn_blocking(move || {
            rules
                .store
                .search_reports(strings)
                .map_err(|error| error.to_string())
        })
        .await
        .map_err(|error| format!("report search worker failed: {error}"))?
    }

    pub(crate) async fn get_event_report(&self, acceptance_id: i64) -> Result<Report, String> {
        if acceptance_id <= 0 {
            return Err("acceptance_id must be positive".into());
        }
        let rules = self.rules.clone();
        tokio::task::spawn_blocking(move || {
            rules
                .store
                .event_report(acceptance_id)
                .map_err(|error| error.to_string())
        })
        .await
        .map_err(|error| format!("report worker failed: {error}"))?
    }

    pub(crate) fn read_event_report(
        &self,
        id: i64,
    ) -> tokio::sync::oneshot::Receiver<Result<Report, String>> {
        let service = self.clone();
        let (sender, receiver) = tokio::sync::oneshot::channel();
        self.runtime.spawn(async move {
            let _ = sender.send(service.get_event_report(id).await);
        });
        receiver
    }

    pub(crate) fn run_conclusion_worker(
        id: i64,
        name: &str,
        descriptors: [i32; 3],
    ) -> Result<(), String> {
        let config = Config::from_env()?;
        crate::environments::runtime_db::require_ready(&config)?;
        let claimed = conclusion::claim(&config, id, name, descriptors)?;
        let service = Self::from_config(config).map_err(|error| error.to_string())?;
        let result = service.purge_concluded_instance(id, name, &claimed);
        service
            .rules
            .store
            .set_report_cleanup(
                id,
                if result.is_ok() {
                    CleanupState::Purged
                } else {
                    CleanupState::Failed
                },
                result.as_ref().err().map(String::as_str),
            )
            .map_err(|error| error.to_string())?;
        result
    }

    fn purge_concluded_instance(
        &self,
        id: i64,
        name: &str,
        claimed: &conclusion::Claimed,
    ) -> Result<(), String> {
        let report = self
            .rules
            .store
            .event_report(id)
            .map_err(|error| error.to_string())?;
        if report.details.cleanup_state != CleanupState::Pending {
            return Err("conclusion request is not available for this worker".into());
        }
        let scope = self
            .environments
            .bind_instance_directory(&self.environments.config.workspaces.join(name))?;
        let instance = self.environments.verify_instance_scope(&scope)?;
        let acceptance = self
            .rules
            .store
            .instance_acceptance(&instance)
            .map_err(|error| error.to_string())?;
        if acceptance.id != id {
            return Err("conclusion acceptance does not own this instance".into());
        }
        self.rules
            .store
            .set_report_cleanup(id, CleanupState::Purging, None)
            .map_err(|error| error.to_string())?;
        self.retain_instance_history(&instance)?;
        let operation = self.environments.begin("delete_instance", name, None)?;
        self.environments.execute(
            operation.clone(),
            60,
            Startup {
                instance_lock: Some(claimed.instance_lock.borrowed()?),
                ..Default::default()
            },
            &|workspace, deadline| {
                self.runtime
                    .block_on(self.opencode.observer.close_workspace(workspace, deadline))
            },
        );
        let operation = self.environments.operation(&operation.id)?;
        if operation.state != OperationState::Succeeded {
            return Err(operation
                .error
                .unwrap_or_else(|| "conclusion purge failed".into()));
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "tests/instance_mcp.rs"]
mod tests;
