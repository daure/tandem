use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
    time::Duration,
};

use crate::{environments::opencode, store::opencode::Snapshot};

pub(super) fn apply_edits(
    snapshot: &mut Snapshot,
    edits: &BTreeMap<(String, String), Option<String>>,
) {
    snapshot.sessions.retain_mut(|session| {
        match edits.get(&(session.server.clone(), session.id.clone())) {
            Some(Some(title)) => session.title.clone_from(title),
            Some(None) => return false,
            None => {}
        }
        true
    });
}

pub(super) fn apply_forgotten(snapshot: &mut Snapshot, forgotten: &BTreeSet<String>) {
    snapshot
        .directories
        .retain(|directory| !forgotten.contains(directory));
    snapshot
        .sessions
        .retain(|session| !forgotten.contains(&session.directory));
    snapshot
        .clients
        .retain(|client| !forgotten.contains(&client.directory));
}

fn validate_external_directory(
    config: &crate::environments::config::Config,
    directory: &str,
) -> Result<(), String> {
    let path = std::path::Path::new(directory);
    if !path.is_absolute()
        || path
            .components()
            .any(|part| part == std::path::Component::ParentDir)
    {
        return Err("OpenCode folder must use an absolute path without parent traversal".into());
    }
    if path.starts_with(&config.workspaces) {
        return Err("Only external OpenCode folders can be cleared".into());
    }
    Ok(())
}

impl super::super::AppService {
    pub(crate) fn clear_opencode_folder(
        &self,
        directory: &str,
    ) -> Result<tokio::sync::oneshot::Receiver<Result<(), String>>, String> {
        if !self.opencode_enabled() {
            return Err("OpenCode integration is disabled".into());
        }
        validate_external_directory(&self.environments.config, directory)?;
        let snapshot = self.opencode_snapshot();
        if !snapshot
            .workspace_directories()
            .any(|path| path == directory)
        {
            return Err("OpenCode folder is unavailable; refresh and try again".into());
        }
        let instances = self.environments.snapshot().instances;
        if crate::store::opencode::workspace_owner(
            directory,
            instances
                .iter()
                .map(|instance| (instance.name.as_str(), instance.workspace.as_str())),
        )
        .is_some()
        {
            return Err("Only external OpenCode folders can be cleared".into());
        }
        if snapshot
            .clients
            .iter()
            .any(|client| client.directory == directory)
            || snapshot.sessions.iter().any(|session| {
                session.directory == directory
                    && (session.attached()
                        || session.activity != crate::store::opencode::Activity::Idle)
            })
        {
            return Err("Close active OpenCode sessions before clearing this folder".into());
        }
        let servers = snapshot
            .sessions
            .iter()
            .filter(|session| session.directory == directory)
            .map(|session| session.server.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let directory = directory.to_owned();
        let mut state = self
            .opencode
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if state.clearing_directories.contains(&directory) {
            return Err("OpenCode folder cleanup is already in progress".into());
        }
        let worker = opencode::cleanup::launch(
            &self.environments.config,
            &self.opencode.observer,
            &directory,
            servers,
        )?;
        state.clearing_directories.insert(directory.clone());
        let integration = Arc::downgrade(&self.opencode);
        let (sender, receiver) = tokio::sync::oneshot::channel();
        std::thread::spawn(move || {
            let outcome = worker.wait();
            if let Some(owner) = integration.upgrade() {
                let mut state = owner
                    .state
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                state.clearing_directories.remove(&directory);
                let edits = outcome.removed.into_iter().map(|key| (key, None)).collect();
                apply_edits(&mut state.snapshot, &edits);
                state.session_edits.extend(edits);
                if outcome.cleared {
                    state.forgotten_directories.insert(directory);
                    let forgotten = state.forgotten_directories.clone();
                    apply_forgotten(&mut state.snapshot, &forgotten);
                }
                if let Some(signal) = &state.changes {
                    signal.send(opencode::events::REMOTE);
                }
            }
            let _ = sender.send(outcome.error.map_or(Ok(()), Err));
        });
        Ok(receiver)
    }

    pub(crate) fn run_opencode_cleanup_worker(descriptor: i32) -> Result<(), String> {
        let mut output = opencode::cleanup::claim_output(descriptor)?;
        let result = (|| {
            let config = crate::environments::config::Config::from_env()?;
            crate::environments::runtime_db::require_ready(&config)?;
            let request = opencode::cleanup::request()?;
            validate_external_directory(&config, &request.directory)?;
            let service = Self::from_config(config).map_err(|error| error.to_string())?;
            if !service.opencode_enabled() {
                return Err("OpenCode integration is disabled".into());
            }
            let mut outcome = service.runtime.block_on(
                request
                    .observer()
                    .clear_directory(&request.directory, request.servers),
            );
            let edits: BTreeMap<_, _> = outcome
                .removed
                .iter()
                .cloned()
                .map(|key| (key, None))
                .collect();
            if !edits.is_empty()
                && let Err(error) = service.rules.update_opencode_history(&edits)
            {
                let history_error = format!(
                    "Removed OpenCode conversations could not be updated in retained history: {error}"
                );
                outcome.error = Some(outcome.error.map_or(history_error.clone(), |error| {
                    format!("{error}; {history_error}")
                }));
            }
            Ok(outcome)
        })();
        let outcome = result.unwrap_or_else(opencode::cleanup::Outcome::failed);
        if let Some(error) = &outcome.error {
            crate::diagnostics::record_error(
                "OpenCode folder cleanup failed",
                &std::io::Error::other(error.clone()),
            );
        }
        opencode::cleanup::write_outcome(&mut output, &outcome)
    }

    pub(crate) fn rename_opencode_session(
        &self,
        id: &str,
        title: String,
    ) -> Result<tokio::sync::oneshot::Receiver<Result<(), String>>, String> {
        let title = title.trim().to_owned();
        if title.is_empty() || title.len() > 1024 || title.chars().any(char::is_control) {
            return Err(
                "Use a nonempty title of at most 1024 bytes without control characters".into(),
            );
        }
        self.edit_opencode_session(id, Some(title))
    }

    pub(crate) fn delete_opencode_session(
        &self,
        id: &str,
    ) -> Result<tokio::sync::oneshot::Receiver<Result<(), String>>, String> {
        self.edit_opencode_session(id, None)
    }

    fn edit_opencode_session(
        &self,
        id: &str,
        title: Option<String>,
    ) -> Result<tokio::sync::oneshot::Receiver<Result<(), String>>, String> {
        if !self.opencode_enabled() {
            return Err("OpenCode integration is disabled".into());
        }
        let session = self
            .known_opencode_session(id)
            .ok_or("OpenCode conversation is unavailable; refresh and try again")?;
        let service = self.clone();
        self.opencode.spawn_navigation(
            &self.runtime,
            "OpenCode action is already in progress",
            async move {
                if !service.opencode_enabled() {
                    return Err("OpenCode integration is disabled".into());
                }
                let ids = tokio::time::timeout(Duration::from_secs(60), async {
                    if let Some(title) = &title {
                        opencode::rename(&session, title).await?;
                        Ok(vec![session.id.clone()])
                    } else {
                        service.opencode.observer.delete_session(&session).await
                    }
                })
                .await
                .map_err(|_| "OpenCode session action timed out; refresh before retrying")??;
                let edits: BTreeMap<_, _> = ids
                    .into_iter()
                    .map(|id| ((session.server.clone(), id), title.clone()))
                    .collect();
                {
                    let mut state = service
                        .opencode
                        .state
                        .lock()
                        .unwrap_or_else(|error| error.into_inner());
                    state.session_edits.extend(edits.clone());
                    apply_edits(&mut state.snapshot, &edits);
                    if let Some(signal) = &state.changes {
                        signal.send(opencode::events::REMOTE);
                    }
                }
                let rules = Arc::clone(&service.rules);
                tokio::task::spawn_blocking(move || rules.update_opencode_history(&edits))
                    .await
                    .map_err(|error| error.to_string())?
                    .map_err(|error| format!("OpenCode session changed, but retained history could not be updated: {error}"))
            },
        )
    }
}

#[cfg(test)]
#[path = "tests/sessions.rs"]
mod tests;
