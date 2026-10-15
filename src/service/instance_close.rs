use std::sync::Arc;

use super::AppService;
use crate::{
    environments::{InstanceScope, config::Config, instance_close},
    store::opencode::closing::{CloseInput, CloseReceipt},
};

impl AppService {
    pub(crate) async fn close_instance_sessions(
        &self,
        scope: Arc<InstanceScope>,
        caller: Option<String>,
        input: CloseInput,
    ) -> Result<CloseReceipt, String> {
        input.validate()?;
        let caller = caller
            .ok_or("close requires OpenCode session metadata; caller identity is unavailable")?;
        let service = self.clone();
        tokio::task::spawn_blocking(move || {
            let (instance, lock) = service.environments.admit_instance_conclusion(&scope)?;
            let observer = &service.opencode.observer;
            let plan = service
                .runtime
                .block_on(async {
                    tokio::time::timeout(
                        std::time::Duration::from_secs(15),
                        observer.plan_instance_close(&instance.workspace, &caller, &input),
                    )
                    .await
                })
                .map_err(|_| "instance closure admission timed out".to_owned())??;
            let mut child = instance_close::launch(
                &service.environments.config,
                observer,
                &instance.name,
                scope.identity(&service.environments.config)?,
                plan,
                lock,
            )?;
            std::thread::spawn(move || {
                if let Err(error) = child.wait() {
                    crate::diagnostics::record_error("cannot reap instance close worker", &error);
                }
            });
            Ok(CloseReceipt {
                instance: instance.name,
                sessions: input.sessions,
                zellij_tab: input.zellij_tab,
                state: "pending",
            })
        })
        .await
        .map_err(|error| format!("instance closure worker failed: {error}"))?
    }

    pub(crate) fn run_instance_close_worker(name: &str, descriptor: i32) -> Result<(), String> {
        let result = (|| {
            let config = Config::from_env()?;
            crate::environments::runtime_db::require_ready(&config)?;
            let (request, _lock) = instance_close::claim(&config, name, descriptor)?;
            let service = Self::from_config(config).map_err(|error| error.to_string())?;
            let scope = service
                .environments
                .bind_instance_directory(std::path::Path::new(
                    &service.environments.workspace(name)?,
                ))?;
            let instance = service.environments.verify_instance_scope(&scope)?;
            if scope.identity(&service.environments.config)? != request.identity {
                return Err("instance closure workspace was replaced".into());
            }
            let observer = request.observer();
            service
                .runtime
                .block_on(observer.execute_instance_close(&instance.workspace, request.plan))
        })();
        if let Err(error) = &result {
            crate::diagnostics::record_error(
                "instance closure failed",
                &std::io::Error::other(error.clone()),
            );
        }
        result
    }
}

#[cfg(test)]
#[path = "tests/instance_close.rs"]
mod tests;
