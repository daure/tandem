use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::store::{
    environments::validate_instance_name,
    opencode::{Activity, listing::SessionListing},
};

impl super::super::AppService {
    pub(crate) async fn list_sessions(
        &self,
        instance: Option<String>,
        include_closed: bool,
    ) -> Result<SessionListing, String> {
        if !self.opencode_enabled() {
            return Err("OpenCode integration is disabled".into());
        }
        if let Some(name) = &instance {
            validate_instance_name(name)?;
        }
        let inventory = self.list_instances().await?;
        if let Some(name) = &instance
            && !inventory.instances.iter().any(|item| item.name == *name)
        {
            return Err(inventory.runtime_error.unwrap_or_else(|| {
                format!("instance {name} is unavailable; refresh and try again")
            }));
        }
        let mut previous = self.opencode_snapshot();
        let store = self.rules.store.clone();
        let retained = self
            .runtime
            .spawn_blocking(move || store.retained_sessions().map_err(|error| error.to_string()))
            .await
            .map_err(|error| error.to_string())??;
        for session in retained {
            if !previous
                .sessions
                .iter()
                .any(|known| known.id == session.id && known.server == session.server)
            {
                previous.sessions.push(session);
            }
        }
        // Persisted links and process-local caches need fresh evidence on every listing.
        for session in &mut previous.sessions {
            session.stale = true;
        }
        let roots = inventory
            .instances
            .iter()
            .map(|item| item.workspace.clone())
            .collect::<Vec<_>>();
        let observer = self.opencode.observer.clone();
        let fallback = previous.clone();
        let observed = self
            .runtime
            .spawn(async move {
                tokio::time::timeout(
                    Duration::from_secs(10),
                    observer.observe_changes(&roots, previous, true),
                )
                .await
                .map_err(|_| "OpenCode session observation timed out".to_owned())?
            })
            .await
            .map_err(|error| error.to_string())?;
        let mut snapshot = match observed {
            Ok(snapshot) => snapshot,
            Err(error) => {
                let mut snapshot = fallback;
                snapshot.error = Some(error);
                for session in &mut snapshot.sessions {
                    session.panes.clear();
                    session.activity = Activity::Unknown;
                }
                snapshot
            }
        };
        if !self.opencode_enabled() {
            return Err("OpenCode integration is disabled".into());
        }
        let state = self
            .opencode
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        super::sessions::apply_edits(&mut snapshot, &state.session_edits);
        super::sessions::apply_forgotten(&mut snapshot, &state.forgotten_directories);
        super::sessions::apply_forgotten(&mut snapshot, &state.clearing_directories);
        let observation_error = snapshot.error.clone();
        Ok(SessionListing {
            sessions: crate::store::opencode::listing::project(
                snapshot,
                &inventory.instances,
                instance.as_deref(),
                include_closed,
            ),
            observed_at_unix_seconds: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            history_window_per_directory: crate::environments::opencode::SESSION_DIRECTORY_WINDOW,
            observation_error,
            inventory_error: inventory.runtime_error,
        })
    }
}
