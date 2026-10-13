use std::{sync::Arc, time::Duration};

use crate::{
    service::AppService,
    store::{
        environments::RuntimeInventory,
        opencode::listing::SessionListing,
        state_stream::{self, StateSnapshot},
    },
};

impl AppService {
    pub(crate) fn state_socket_path(&self) -> Result<std::path::PathBuf, String> {
        crate::state_socket::socket_path(&self.state_stream.config)
    }

    pub(crate) async fn serve_state(&self) -> Result<(), String> {
        let socket = crate::state_socket::Listener::bind(&self.state_stream.config)?;
        let (stop, stopped) = tokio::sync::watch::channel(false);
        let service = self.clone();
        let coordinator = self
            .runtime
            .spawn(async move { service.observe_state(stopped).await });
        let result = crate::state_socket::serve(self.state_stream.clone(), socket).await;
        let _ = stop.send(true);
        let _ = coordinator.await;
        result
    }

    pub(crate) async fn watch_state(&self, since: Option<String>) -> Result<(), String> {
        crate::state_socket::watch(&self.state_stream.config, since).await
    }

    async fn observe_state(&self, mut stop: tokio::sync::watch::Receiver<bool>) {
        let mut ticker = tokio::time::interval(Duration::from_secs(1));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut revision = -1;
        let mut inventory_due = tokio::time::Instant::now();
        let mut enabled = self.opencode_enabled();
        loop {
            let observation_changed = tokio::select! {
                _ = ticker.tick() => false,
                _ = self.opencode.published.notified() => true,
                _ = stop.changed() => return,
            };
            let store = self.state_stream.clone();
            let settings = self.settings.clone();
            let read = self
                .runtime
                .spawn_blocking(move || {
                    settings.refresh()?;
                    store.recover_identities()?;
                    store.inventory_revision()
                })
                .await;
            let changed = match read {
                Ok(Ok(value)) => {
                    let changed = revision != value;
                    revision = value;
                    changed
                }
                Ok(Err(error)) => {
                    record("state observation failed", error);
                    false
                }
                Err(error) => {
                    record("state observation failed", error.to_string());
                    false
                }
            };
            let refresh = changed || tokio::time::Instant::now() >= inventory_due;
            if refresh {
                let environments = self.environments.clone();
                if let Err(error) = self
                    .runtime
                    .spawn_blocking(move || environments.refresh_instances())
                    .await
                {
                    record("state inventory worker failed", error.to_string());
                }
                inventory_due = tokio::time::Instant::now() + Duration::from_secs(10);
            }
            self.poll_opencode();
            let integration_changed = enabled != self.opencode_enabled();
            enabled = self.opencode_enabled();
            if !refresh && !observation_changed && !integration_changed {
                continue;
            }
            let inventory = self.environments.snapshot();
            let instances = RuntimeInventory {
                instances: inventory.instances,
                activities: inventory.activities,
                observed_at_unix_seconds: inventory
                    .observed_at_unix_seconds
                    .unwrap_or_else(state_stream::now),
                runtime_error: inventory.runtime_error,
            };
            let store = self.state_stream.clone();
            let instances = match self
                .runtime
                .spawn_blocking(move || store.preserve_failed_inventory(instances))
                .await
            {
                Ok(Ok(instances)) => instances,
                Ok(Err(error)) => {
                    record("state inventory recovery failed", error);
                    continue;
                }
                Err(error) => {
                    record("state inventory recovery worker failed", error.to_string());
                    continue;
                }
            };
            let observed = self.opencode_snapshot();
            let observation_error = observed.error.clone();
            let sessions = SessionListing {
                sessions: crate::store::opencode::listing::project(
                    observed,
                    &instances.instances,
                    None,
                    true,
                ),
                observed_at_unix_seconds: state_stream::now(),
                history_window_per_directory:
                    crate::environments::opencode::SESSION_DIRECTORY_WINDOW,
                observation_error,
                inventory_error: instances.runtime_error.clone(),
            };
            let mut snapshot = match (
                serde_json::to_value(instances),
                serde_json::to_value(sessions),
            ) {
                (Ok(instances), Ok(sessions)) => StateSnapshot {
                    instances,
                    sessions,
                },
                _ => {
                    record(
                        "state projection failed",
                        "state serialization failed".into(),
                    );
                    continue;
                }
            };
            if self.opencode_loading() {
                let store = self.state_stream.clone();
                match self
                    .runtime
                    .spawn_blocking(move || store.boundary(None))
                    .await
                {
                    Ok(Ok(previous)) => {
                        snapshot.sessions = previous.snapshot.sessions;
                        if let Some(sessions) = snapshot.sessions["sessions"].as_array_mut() {
                            for session in sessions {
                                session["stale"] = serde_json::json!(true);
                                session["activity"] = serde_json::json!("unknown");
                            }
                        }
                        snapshot.sessions["observation_error"] =
                            serde_json::json!("OpenCode baseline pending");
                    }
                    _ => continue,
                }
            }
            let store = Arc::clone(&self.state_stream);
            match self
                .runtime
                .spawn_blocking(move || store.publish(snapshot))
                .await
            {
                Ok(Ok(())) => {}
                Ok(Err(error)) => record("state publication failed", error),
                Err(error) => record("state publication worker failed", error.to_string()),
            }
        }
    }
}

fn record(context: &str, error: String) {
    crate::diagnostics::record_error(context, &std::io::Error::other(error));
}
