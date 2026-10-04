use super::AppService;
use crate::store::{
    environments::{EnvironmentSnapshot, Instance},
    opencode::Snapshot,
};

impl AppService {
    pub(in crate::service) fn retain_instance_history(
        &self,
        instance: &Instance,
    ) -> Result<(), String> {
        let observed = match self
            .runtime
            .handle()
            .block_on(self.opencode.observer.observe(
                std::slice::from_ref(&instance.workspace),
                Default::default(),
            )) {
            Ok(observed) => observed,
            Err(error) => {
                crate::diagnostics::record_error(
                    "cannot refresh conversation metadata before purge",
                    &std::io::Error::other(error),
                );
                Snapshot::default()
            }
        };
        self.rules
            .store
            .remember_workspaces(
                &EnvironmentSnapshot {
                    instances: vec![instance.clone()],
                    ..Default::default()
                },
                &observed,
            )
            .map_err(|error| format!("cannot retain acceptance workspace before purge: {error}"))
    }
}
