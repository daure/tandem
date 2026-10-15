use std::sync::Arc;

use super::AppService;
use crate::{
    environments::{InstanceScope, Startup, conclusion, config::Config},
    store::{
        environments::{
            InstanceInstructions, InstancePing, Operation, OperationState, RepositoryUpdates,
        },
        rules::reports::{CleanupState, Conclusion, ConclusionReceipt, Report, ReportSummary},
    },
};

impl AppService {
    pub(crate) async fn ping_instance(
        &self,
        scope: Arc<InstanceScope>,
        session_id: Option<String>,
    ) -> Result<InstancePing, String> {
        let service = self.clone();
        tokio::task::spawn_blocking(move || {
            let (instance, _lock) = service.environments.admit_instance_conclusion(&scope)?;
            service.refresh_instance_ping_settings()?;
            let enabled = service.instance_ping_enabled();
            if enabled {
                if let Some(session_id) = session_id.filter(|id| {
                    !id.is_empty()
                        && id.len() < 128
                        && id
                            .bytes()
                            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
                }) {
                    service
                        .environments
                        .record_instance_ping(&instance.name, &session_id)?;
                }
                service.play_selected_sound(service.instance_ping_sound_choice());
            }
            Ok(InstancePing {
                instance: instance.name,
                sound_scheduled: enabled,
            })
        })
        .await
        .map_err(|error| format!("instance ping worker failed: {error}"))?
    }

    pub(crate) async fn update_instance_repositories(
        &self,
        scope: Arc<InstanceScope>,
    ) -> Result<RepositoryUpdates, String> {
        let environments = self.environments.clone();
        tokio::task::spawn_blocking(move || environments.update_instance_repositories(&scope))
            .await
            .map_err(|error| format!("repository update worker failed: {error}"))?
    }

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

    pub(crate) async fn purge_instance(
        &self,
        scope: Arc<InstanceScope>,
    ) -> Result<ConclusionReceipt, String> {
        let service = self.clone();
        tokio::task::spawn_blocking(move || {
            let rules = service
                .rules
                .store
                .lease()
                .map_err(|error| error.to_string())?
                .ok_or("rule dispatch is busy; retry purge after it finishes")?;
            let (instance, instance_lock) =
                service.environments.admit_instance_conclusion(&scope)?;
            let acceptance = service
                .rules
                .store
                .instance_acceptance(&instance)
                .map_err(|error| error.to_string())?;
            let id = acceptance.as_ref().map_or(0, |acceptance| acceptance.id);
            let completion = conclusion::reserve(&service.environments.config, id, &instance.name)?;
            let identity = scope.identity(&service.environments.config)?;
            let report = service
                .rules
                .store
                .workspace_conclusion(&instance.name, &identity)
                .map_err(|error| error.to_string())?
                .and_then(|record| record.report);
            let receipt = service
                .rules
                .store
                .save_conclusion(
                    &instance.name,
                    &identity,
                    acceptance.as_ref().map(|acceptance| acceptance.id),
                    report.as_ref(),
                )
                .map_err(|error| error.to_string())?;
            match conclusion::launch(
                &service.environments.config,
                id,
                receipt.conclusion_id,
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
                    Ok(receipt)
                }
                Err(error) => {
                    service
                        .rules
                        .store
                        .set_conclusion_cleanup(
                            receipt.conclusion_id,
                            CleanupState::Failed,
                            Some(&error),
                        )
                        .map_err(|error| error.to_string())?;
                    Err(format!(
                        "Conclusion {} saved; cleanup could not start: {error}",
                        receipt.conclusion_id
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

    pub(crate) async fn get_conclusion(&self, id: i64) -> Result<Conclusion, String> {
        if id <= 0 {
            return Err("conclusion_id must be positive".into());
        }
        let rules = self.rules.clone();
        tokio::task::spawn_blocking(move || {
            rules
                .store
                .conclusion(id)
                .map_err(|error| error.to_string())
        })
        .await
        .map_err(|error| format!("conclusion read worker failed: {error}"))?
    }

    pub(crate) async fn search_conclusions(
        &self,
        strings: Vec<String>,
    ) -> Result<Vec<ConclusionReceipt>, String> {
        let rules = self.rules.clone();
        tokio::task::spawn_blocking(move || {
            rules
                .store
                .search_conclusions(strings)
                .map_err(|error| error.to_string())
        })
        .await
        .map_err(|error| format!("conclusion search worker failed: {error}"))?
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
        conclusion_id: i64,
        name: &str,
        descriptors: [i32; 3],
    ) -> Result<(), String> {
        let config = Config::from_env()?;
        crate::environments::runtime_db::require_ready(&config)?;
        let claimed = conclusion::claim(&config, id, name, descriptors)?;
        let service = Self::from_config(config).map_err(|error| error.to_string())?;
        let record = service
            .rules
            .store
            .conclusion(conclusion_id)
            .map_err(|error| error.to_string())?;
        if record.details.instance != name
            || record.details.acceptance_id.unwrap_or(0) != id
            || record.details.cleanup_state != CleanupState::Pending
        {
            return Err("conclusion request does not belong to this worker".into());
        }
        let result = service.purge_concluded_instance(id, name, &record, &claimed);
        let state = if result.is_ok() {
            CleanupState::Purged
        } else {
            CleanupState::Failed
        };
        service
            .rules
            .store
            .set_conclusion_cleanup(
                conclusion_id,
                state,
                result.as_ref().err().map(String::as_str),
            )
            .map_err(|error| error.to_string())?;
        result
    }

    fn purge_concluded_instance(
        &self,
        id: i64,
        name: &str,
        record: &Conclusion,
        claimed: &conclusion::Claimed,
    ) -> Result<(), String> {
        if id != 0 && record.report.is_some() {
            let report = self
                .rules
                .store
                .event_report(id)
                .map_err(|error| error.to_string())?;
            if report.details.cleanup_state != CleanupState::Pending {
                return Err("conclusion request is not available for this worker".into());
            }
        }
        let scope = self
            .environments
            .bind_instance_directory(&self.environments.config.workspaces.join(name))?;
        let instance = self.environments.verify_instance_scope(&scope)?;
        if scope.identity(&self.environments.config)? != record.workspace_identity {
            return Err("conclusion workspace was replaced".into());
        }
        let acceptance = self
            .rules
            .store
            .instance_acceptance(&instance)
            .map_err(|error| error.to_string())?;
        if acceptance.as_ref().map_or(0, |acceptance| acceptance.id) != id {
            return Err("conclusion acceptance does not own this instance".into());
        }
        self.rules
            .store
            .set_conclusion_cleanup(record.details.conclusion_id, CleanupState::Purging, None)
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
