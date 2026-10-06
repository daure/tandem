use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use crate::{
    environments::opencode::{self, Observer},
    store::opencode::{CloseScope, Pane, Session, Snapshot, retention::Retention},
};

mod actions;
mod guidance;
mod observation;
mod sessions;

pub(super) use actions::launch_workspace_opencode;
pub(super) use guidance::instance_guidance;

pub(super) struct Integration {
    state: Mutex<State>,
    pub(super) observer: Observer,
    pub(super) current_zellij: String,
}

struct State {
    // Quarantined evidence supports bounded verification independently of visible rows.
    snapshot: Snapshot,
    initial_observation_complete: bool,
    retention: Retention,
    task: Option<tokio::task::JoinHandle<()>>,
    navigation: Option<tokio::task::JoinHandle<()>>,
    conversation: Option<tokio::task::JoinHandle<()>>,
    next: Instant,
    changes: Option<Arc<opencode::events::Signal>>,
    roots: Vec<String>,
    generation: u64,
    session_edits: std::collections::BTreeMap<(String, String), Option<String>>,
    forgotten_directories: std::collections::BTreeSet<String>,
    clearing_directories: std::collections::BTreeSet<String>,
}

#[derive(Debug)]
pub(crate) struct CloseOpencodeOutcome {
    pub reply: tokio::sync::oneshot::Receiver<Result<(), String>>,
    pub panes: Vec<Pane>,
}

impl Integration {
    pub(super) fn new() -> Self {
        Self {
            observer: Observer::from_env(),
            current_zellij: std::env::var("ZELLIJ_SESSION_NAME").unwrap_or_default(),
            state: Mutex::new(State {
                snapshot: Snapshot::default(),
                initial_observation_complete: false,
                retention: Retention::default(),
                task: None,
                navigation: None,
                conversation: None,
                next: Instant::now(),
                changes: None,
                roots: Vec::new(),
                generation: 0,
                session_edits: Default::default(),
                forgotten_directories: Default::default(),
                clearing_directories: Default::default(),
            }),
        }
    }

    pub(super) fn reset(&self) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(task) = state.task.take() {
            task.abort();
        }
        if let Some(task) = state.navigation.take() {
            task.abort();
        }
        if let Some(task) = state.conversation.take() {
            task.abort();
        }
        state.generation += 1;
        state.snapshot = Snapshot::default();
        state.initial_observation_complete = false;
        state.retention = Retention::default();
        state.next = Instant::now();
        state.changes = None;
        state.roots.clear();
        state.session_edits.clear();
        state.forgotten_directories.clear();
    }
}

impl Drop for Integration {
    fn drop(&mut self) {
        self.reset();
    }
}

impl super::AppService {
    pub(crate) fn known_opencode_session(&self, id: &str) -> Option<Session> {
        self.opencode_snapshot()
            .sessions
            .into_iter()
            .find(|session| session.id == id)
            .or_else(|| {
                self.rule_snapshot()
                    .workspaces
                    .into_values()
                    .flat_map(|workspace| workspace.sessions)
                    .find(|session| session.id == id)
            })
    }
    pub(crate) fn cancel_opencode_conversation(&self) {
        if let Some(task) = self
            .opencode
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .conversation
            .take()
        {
            task.abort();
        }
    }

    pub(crate) fn opencode_conversation(
        &self,
        id: &str,
    ) -> Result<tokio::sync::oneshot::Receiver<Result<String, String>>, String> {
        if !self.opencode_enabled() {
            return Err("OpenCode integration is disabled".into());
        }
        let session = self
            .known_opencode_session(id)
            .ok_or("OpenCode conversation is unavailable; refresh and try again")?;
        let settings = Arc::clone(&self.settings);
        let (mut sender, receiver) = tokio::sync::oneshot::channel();
        let mut state = self
            .opencode
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(task) = state.conversation.take() {
            task.abort();
        }
        state.conversation = Some(self.runtime.spawn(async move {
            let result = tokio::select! {
                _ = sender.closed() => return,
                result = opencode::conversation(&session) => result,
            };
            let result = if settings.opencode_enabled() {
                result
            } else {
                Err("OpenCode integration is disabled".into())
            };
            let _ = sender.send(result);
        }));
        Ok(receiver)
    }

    pub(crate) fn opencode_loading(&self) -> bool {
        self.opencode_enabled()
            && !self
                .opencode
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .initial_observation_complete
    }

    pub(crate) fn opencode_snapshot(&self) -> Snapshot {
        if !self.opencode_enabled() {
            return Snapshot::default();
        }
        let state = self
            .opencode
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let mut snapshot = state.retention.visible_snapshot(&state.snapshot);
        sessions::apply_edits(&mut snapshot, &state.session_edits);
        sessions::apply_forgotten(&mut snapshot, &state.forgotten_directories);
        sessions::apply_forgotten(&mut snapshot, &state.clearing_directories);
        snapshot
    }

    pub(crate) fn poll_opencode(&self) {
        if !self.opencode_enabled() {
            self.opencode.reset();
            return;
        }
        let mut state = self
            .opencode
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let mut roots: Vec<_> = self
            .environments
            .snapshot()
            .instances
            .into_iter()
            .map(|instance| instance.workspace)
            .collect();
        roots.sort();
        if roots != state.roots {
            state.roots = roots;
            if let Some(signal) = &state.changes {
                signal.send(opencode::events::REMOTE);
            }
        }
        if Instant::now() < state.next
            || state.task.as_ref().is_some_and(|task| !task.is_finished())
        {
            return;
        }
        // Retry only a failed worker startup; healthy workers wait on external events.
        state.next = Instant::now() + Duration::from_secs(5);
        let generation = state.generation;
        let observer = self.opencode.observer.clone();
        let integration = Arc::downgrade(&self.opencode);
        let settings = Arc::clone(&self.settings);
        let signal = Arc::new(opencode::events::Signal::default());
        state.changes = Some(Arc::clone(&signal));
        state.task = Some(self.runtime.spawn(observation::run(
            observer,
            integration,
            settings,
            generation,
            signal,
        )));
    }

    pub(crate) fn open_opencode(
        &self,
        id: &str,
        pane: Option<Pane>,
    ) -> Result<tokio::sync::oneshot::Receiver<Result<(), String>>, String> {
        if !self.opencode_enabled() {
            return Err("OpenCode integration is disabled".into());
        }
        let snapshot = self.opencode_snapshot();
        let session: Session = self
            .known_opencode_session(id)
            .ok_or("OpenCode conversation is unavailable; refresh and try again")?;
        if let Some(pane) = &pane
            && !session.panes.contains(pane)
        {
            return Err("OpenCode pane is unavailable".into());
        }
        if pane.is_none() && snapshot.missing_directory(&session.directory) {
            return Err(
                "The workspace folder is missing; only an existing OpenCode client can be opened"
                    .into(),
            );
        }
        let settings = Arc::clone(&self.settings);
        let current = self.opencode.current_zellij.clone();
        let observer = self.opencode.observer.clone();
        let instances = self.environments.snapshot().instances;
        let owner = |directory: &str| {
            crate::store::opencode::workspace_owner(
                directory,
                instances
                    .iter()
                    .map(|instance| (instance.name.as_str(), instance.workspace.as_str())),
            )
        };
        let instance = owner(&session.directory).map(str::to_owned);
        let destination = snapshot
            .sessions
            .iter()
            .filter(|other| match instance.as_deref() {
                Some(instance) => owner(&other.directory) == Some(instance),
                None => other.directory == session.directory,
            })
            .flat_map(|other| other.panes.iter())
            .min_by_key(|pane| pane.session != current)
            .cloned();
        let name = instance.unwrap_or_else(|| actions::directory_name(&session.directory));
        self.opencode.spawn_navigation(
            &self.runtime,
            "OpenCode navigation is already in progress",
            async move {
                if !settings.opencode_enabled() {
                    Err("OpenCode integration is disabled".into())
                } else if let Some(pane) = pane {
                    observer.jump(&session.id, &pane, &current).await
                } else {
                    observer
                        .attach(&session, &name, &current, destination.as_ref())
                        .await
                }
            },
        )
    }

    pub(crate) fn open_opencode_client(
        &self,
        pane: Pane,
    ) -> Result<tokio::sync::oneshot::Receiver<Result<(), String>>, String> {
        if !self.opencode_enabled() {
            return Err("OpenCode integration is disabled".into());
        }
        if !self
            .opencode_snapshot()
            .clients
            .iter()
            .any(|client| client.pane == pane)
        {
            return Err("OpenCode pane is unavailable".into());
        }
        let settings = Arc::clone(&self.settings);
        let current = self.opencode.current_zellij.clone();
        let observer = self.opencode.observer.clone();
        self.opencode.spawn_navigation(
            &self.runtime,
            "OpenCode navigation is already in progress",
            async move {
                if settings.opencode_enabled() {
                    observer.jump_pane(&pane, &current).await
                } else {
                    Err("OpenCode integration is disabled".into())
                }
            },
        )
    }

    pub(crate) fn close_opencode(
        &self,
        id: &str,
        pane: Pane,
    ) -> Result<tokio::sync::oneshot::Receiver<Result<(), String>>, String> {
        if !self.opencode_enabled() {
            return Err("OpenCode integration is disabled".into());
        }
        let session = self
            .opencode_snapshot()
            .sessions
            .into_iter()
            .find(|session| session.id == id)
            .ok_or("OpenCode conversation is unavailable; refresh and try again")?;
        if !session.panes.contains(&pane) {
            return Err("OpenCode pane is unavailable".into());
        }
        self.close_observed_opencode_pane(Some(session.id), pane)
    }

    pub(crate) fn close_opencode_client(
        &self,
        pane: Pane,
    ) -> Result<tokio::sync::oneshot::Receiver<Result<(), String>>, String> {
        if !self.opencode_enabled() {
            return Err("OpenCode integration is disabled".into());
        }
        if !self
            .opencode_snapshot()
            .clients
            .iter()
            .any(|client| client.pane == pane)
        {
            return Err("OpenCode pane is unavailable".into());
        }
        self.close_observed_opencode_pane(None, pane)
    }

    fn close_observed_opencode_pane(
        &self,
        session_id: Option<String>,
        pane: Pane,
    ) -> Result<tokio::sync::oneshot::Receiver<Result<(), String>>, String> {
        let settings = Arc::clone(&self.settings);
        let observer = self.opencode.observer.clone();
        self.opencode.spawn_navigation(
            &self.runtime,
            "OpenCode action is already in progress",
            async move {
                if settings.opencode_enabled() {
                    if let Some(session_id) = session_id {
                        observer.close(&session_id, &pane).await
                    } else {
                        observer.close_pane(&pane).await
                    }
                } else {
                    Err("OpenCode integration is disabled".into())
                }
            },
        )
    }

    pub(crate) fn close_opencode_scope(
        &self,
        scope: &CloseScope,
    ) -> Result<CloseOpencodeOutcome, String> {
        if !self.opencode_enabled() {
            return Err("OpenCode integration is disabled".into());
        }
        let instances = self.environments.snapshot().instances;
        if let CloseScope::Instance(name) = scope
            && !instances.iter().any(|instance| instance.name == *name)
        {
            return Err("Instance is unavailable; refresh and try again".into());
        }
        let owner = |directory: &str| {
            crate::store::opencode::workspace_owner(
                directory,
                instances
                    .iter()
                    .map(|instance| (instance.name.as_str(), instance.workspace.as_str())),
            )
        };
        let included = |directory: &str| match scope {
            CloseScope::Instance(name) => owner(directory) == Some(name.as_str()),
            CloseScope::Directory(path) => directory == path && owner(directory).is_none(),
            CloseScope::ExternalWorkspaces => owner(directory).is_none(),
        };
        let snapshot = self.opencode_snapshot();
        let panes = snapshot
            .sessions
            .iter()
            .filter(|session| included(&session.directory))
            .flat_map(|session| {
                session
                    .panes
                    .iter()
                    .map(|pane| (session.directory.clone(), pane.clone()))
            })
            .chain(
                snapshot
                    .clients
                    .iter()
                    .filter(|client| included(&client.directory))
                    .map(|client| (client.directory.clone(), client.pane.clone())),
            )
            .collect::<Vec<_>>();
        let panes =
            panes
                .into_iter()
                .fold(Vec::new(), |mut unique: Vec<(String, Pane)>, target| {
                    if !unique
                        .iter()
                        .any(|(_, pane)| pane.session == target.1.session && pane.id == target.1.id)
                    {
                        unique.push(target);
                    }
                    unique
                });
        let targets = panes.iter().map(|(_, pane)| pane.clone()).collect();
        let settings = Arc::clone(&self.settings);
        let observer = self.opencode.observer.clone();
        let receiver = self.opencode.spawn_navigation(
            &self.runtime,
            "OpenCode action is already in progress",
            async move {
                if settings.opencode_enabled() {
                    observer
                        .close_panes(panes, Instant::now() + Duration::from_secs(60))
                        .await
                } else {
                    Err("OpenCode integration is disabled".into())
                }
            },
        )?;
        Ok(CloseOpencodeOutcome {
            reply: receiver,
            panes: targets,
        })
    }

    pub(crate) fn setup_opencode(&self) -> Result<String, String> {
        opencode::install(&self.environments.config.home)
    }

    #[cfg(test)]
    pub(crate) fn reset_opencode_for_tests(&self) {
        self.opencode.reset();
    }

    #[cfg(test)]
    pub(crate) fn set_opencode_snapshot_for_tests(&self, snapshot: Snapshot) {
        let mut state = self.opencode.state.lock().unwrap();
        state.snapshot = snapshot;
        state.initial_observation_complete = true;
    }
}

#[cfg(test)]
#[path = "tests/opencode_completion.rs"]
mod tests;
