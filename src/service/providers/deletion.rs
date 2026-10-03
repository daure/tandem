use std::time::Instant;
use tokio::sync::oneshot;

use crate::{
    service::AppService,
    store::providers::{Action, ActionError},
};

impl AppService {
    pub(crate) fn delete_provider(
        &self,
        name: String,
        confirmed: bool,
    ) -> oneshot::Receiver<Result<String, String>> {
        let (sender, receiver) = oneshot::channel();
        if !confirmed {
            let _ = sender.send(Err("confirmation_required: permanently delete provider package, collector, checkpoints, credentials, streams, events, and event-created instances, workspaces and OpenCode sessions".into()));
            return receiver;
        }
        let integration = self.providers.clone();
        {
            let mut operations = integration
                .operations
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if operations
                .keys()
                .any(|key| key == &name || key.starts_with(&format!("{name}/")))
            {
                let _ = sender.send(Err(
                    "A provider operation is already in progress; retry when it finishes".into(),
                ));
                return receiver;
            }
            operations.insert(name.clone(), Action::Stop);
        }
        let service = self.clone();
        self.runtime.spawn_blocking(move || {
            let result = service.delete_provider_blocking(&name);
            if let Err(error) = &result {
                let failure = Err(ActionError::Failed(error.clone()));
                integration.complete_action(&name, Action::Stop, &failure);
            } else {
                let mut snapshot = integration
                    .snapshot
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                snapshot.providers.retain(|provider| provider.name != name);
                integration
                    .revision
                    .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
                integration
                    .operations
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .remove(&name);
            }
            service.poll_events();
            service.poll_rules();
            service.refresh_environments();
            let _ = sender.send(result);
        });
        receiver
    }

    fn delete_provider_blocking(&self, name: &str) -> Result<String, String> {
        let _lease = self
            .rules
            .store
            .lease()
            .map_err(|error| error.to_string())?
            .ok_or("rule worker is active; retry provider deletion when it finishes")?;
        let deletion = self.providers.manager.begin_deletion(name)?;
        deletion.remove_collector()?;
        let mut instances = std::collections::BTreeSet::new();
        for acceptance in &deletion.acceptances {
            let Some(operation) = &acceptance.operation_id else {
                continue;
            };
            if !instances.insert(acceptance.instance.clone()) {
                continue;
            }
            self.providers.manager.delete_created_instance(
                &acceptance.instance,
                operation,
                &|workspace, deadline: Instant| {
                    self.runtime
                        .handle()
                        .block_on(self.opencode.observer.close_workspace(workspace, deadline))?;
                    // History cleanup is part of this explicit purge, independent of creation preferences.
                    self.runtime.handle().block_on(
                        crate::environments::opencode::clear_history(workspace, deadline),
                    )?;
                    Ok(())
                },
            )?;
        }
        let source = deletion.source.clone();
        let events = deletion.finish()?;
        Ok(format!(
            "Deleted provider {name} ({source}), its package and private state, {events} events, and {} event-created instances",
            instances.len()
        ))
    }
}
