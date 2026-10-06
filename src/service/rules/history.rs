use super::{AppService, Integration, Ordering};
use crate::store::{
    environments::{EnvironmentSnapshot, Instance},
    events::Error,
    opencode::Snapshot,
};

impl Integration {
    pub(in crate::service) fn update_opencode_history(
        &self,
        edits: &std::collections::BTreeMap<(String, String), Option<String>>,
    ) -> Result<(), Error> {
        self.store.update_opencode_history(edits)?;
        let mut snapshot = self
            .snapshot
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        for workspace in snapshot.workspaces.values_mut() {
            workspace.edit_sessions(edits);
        }
        self.revision.fetch_add(1, Ordering::AcqRel);
        Ok(())
    }
}

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
