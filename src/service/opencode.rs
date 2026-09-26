use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use crate::{
    environments::opencode::{self, Observer},
    store::opencode::{Pane, Session, Snapshot},
};

pub(super) struct Integration {
    state: Mutex<State>,
    observer: Observer,
    current_zellij: String,
}

struct State {
    snapshot: Snapshot,
    task: Option<tokio::task::JoinHandle<()>>,
    navigation: Option<tokio::task::JoinHandle<()>>,
    conversation: Option<tokio::task::JoinHandle<()>>,
    next: Instant,
    generation: u64,
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
                task: None,
                navigation: None,
                conversation: None,
                next: Instant::now(),
                generation: 0,
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
        state.next = Instant::now();
    }
}

impl Drop for Integration {
    fn drop(&mut self) {
        self.reset();
    }
}

impl super::AppService {
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
            .opencode_snapshot()
            .sessions
            .into_iter()
            .find(|session| session.id == id)
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

    pub(crate) fn opencode_snapshot(&self) -> Snapshot {
        if !self.opencode_enabled() {
            return Snapshot::default();
        }
        self.opencode
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .snapshot
            .clone()
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
        if Instant::now() < state.next
            || state.task.as_ref().is_some_and(|task| !task.is_finished())
        {
            return;
        }
        let roots: Vec<_> = self
            .environments
            .snapshot()
            .instances
            .into_iter()
            .map(|instance| instance.workspace)
            .collect();
        state.next = Instant::now() + Duration::from_secs(2);
        let generation = state.generation;
        let previous = state.snapshot.clone();
        let observer = self.opencode.observer.clone();
        let integration = Arc::downgrade(&self.opencode);
        let settings = Arc::clone(&self.settings);
        state.task = Some(self.runtime.spawn(async move {
            let result =
                tokio::time::timeout(Duration::from_secs(15), observer.observe(&roots, previous))
                    .await
                    .unwrap_or_else(|_| Err("OpenCode observation timed out".into()));
            let Some(integration) = integration.upgrade() else {
                return;
            };
            let mut state = integration
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if state.generation != generation || !settings.opencode_enabled() {
                return;
            }
            match result {
                Ok(snapshot) => state.snapshot = snapshot,
                Err(error) => {
                    for session in &mut state.snapshot.sessions {
                        session.stale = true;
                    }
                    for client in &mut state.snapshot.clients {
                        client.stale = true;
                    }
                    state.snapshot.error = Some(error);
                }
            }
        }));
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
        let session: Session = snapshot
            .sessions
            .iter()
            .find(|session| session.id == id)
            .cloned()
            .ok_or("OpenCode conversation is unavailable; refresh and try again")?;
        if let Some(pane) = &pane
            && !session.panes.contains(pane)
        {
            return Err("OpenCode pane is unavailable".into());
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
        if pane.is_none() && instance.is_none() {
            return Err("OpenCode workspace is not owned by an instance".into());
        }
        let destination = instance.as_deref().and_then(|instance| {
            snapshot
                .sessions
                .iter()
                .filter(|other| owner(&other.directory) == Some(instance))
                .flat_map(|other| other.panes.iter())
                .min_by_key(|pane| pane.session != current)
                .cloned()
        });
        let (sender, receiver) = tokio::sync::oneshot::channel();
        let mut state = self
            .opencode
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if state
            .navigation
            .as_ref()
            .is_some_and(|task| !task.is_finished())
        {
            return Err("OpenCode navigation is already in progress".into());
        }
        state.navigation = Some(self.runtime.spawn(async move {
            let result = if !settings.opencode_enabled() {
                Err("OpenCode integration is disabled".into())
            } else if let Some(pane) = pane {
                observer.jump(&session.id, &pane, &current).await
            } else {
                observer
                    .attach(
                        &session,
                        instance
                            .as_deref()
                            .expect("attach requires an owning instance"),
                        &current,
                        destination.as_ref(),
                    )
                    .await
            };
            let _ = sender.send(result);
        }));
        Ok(receiver)
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
        let (sender, receiver) = tokio::sync::oneshot::channel();
        let mut state = self
            .opencode
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if state
            .navigation
            .as_ref()
            .is_some_and(|task| !task.is_finished())
        {
            return Err("OpenCode navigation is already in progress".into());
        }
        state.navigation = Some(self.runtime.spawn(async move {
            let result = if settings.opencode_enabled() {
                observer.jump_pane(&pane, &current).await
            } else {
                Err("OpenCode integration is disabled".into())
            };
            let _ = sender.send(result);
        }));
        Ok(receiver)
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
        let (sender, receiver) = tokio::sync::oneshot::channel();
        let mut state = self
            .opencode
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if state
            .navigation
            .as_ref()
            .is_some_and(|task| !task.is_finished())
        {
            return Err("OpenCode action is already in progress".into());
        }
        state.navigation = Some(self.runtime.spawn(async move {
            let result = if settings.opencode_enabled() {
                if let Some(session_id) = session_id {
                    observer.close(&session_id, &pane).await
                } else {
                    observer.close_pane(&pane).await
                }
            } else {
                Err("OpenCode integration is disabled".into())
            };
            let _ = sender.send(result);
        }));
        Ok(receiver)
    }

    pub(crate) fn close_instance_opencode(
        &self,
        name: &str,
    ) -> Result<CloseOpencodeOutcome, String> {
        if !self.opencode_enabled() {
            return Err("OpenCode integration is disabled".into());
        }
        let instances = self.environments.snapshot().instances;
        if !instances.iter().any(|instance| instance.name == name) {
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
        let snapshot = self.opencode_snapshot();
        let panes = snapshot
            .sessions
            .iter()
            .filter(|session| owner(&session.directory) == Some(name))
            .flat_map(|session| session.panes.iter().cloned())
            .chain(
                snapshot
                    .clients
                    .iter()
                    .filter(|client| owner(&client.directory) == Some(name))
                    .map(|client| client.pane.clone()),
            )
            .collect::<Vec<_>>();
        let panes = panes.into_iter().fold(Vec::new(), |mut unique, pane| {
            if !unique.contains(&pane) {
                unique.push(pane);
            }
            unique
        });
        let targets = panes.clone();
        let settings = Arc::clone(&self.settings);
        let observer = self.opencode.observer.clone();
        let (sender, receiver) = tokio::sync::oneshot::channel();
        let mut state = self
            .opencode
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if state
            .navigation
            .as_ref()
            .is_some_and(|task| !task.is_finished())
        {
            return Err("OpenCode action is already in progress".into());
        }
        state.navigation = Some(self.runtime.spawn(async move {
            let result = if settings.opencode_enabled() {
                let mut errors = Vec::new();
                for pane in panes {
                    if let Err(error) = observer.close_pane(&pane).await {
                        errors.push(format!("{} / pane {}: {error}", pane.session, pane.id));
                    }
                }
                if errors.is_empty() {
                    Ok(())
                } else {
                    Err(errors.join("\n"))
                }
            } else {
                Err("OpenCode integration is disabled".into())
            };
            let _ = sender.send(result);
        }));
        Ok(CloseOpencodeOutcome {
            reply: receiver,
            panes: targets,
        })
    }

    pub(crate) fn setup_opencode(&self) -> Result<String, String> {
        opencode::install(&self.environments.config.home)
    }

    #[cfg(test)]
    pub(crate) fn set_opencode_snapshot_for_tests(&self, snapshot: Snapshot) {
        self.opencode.state.lock().unwrap().snapshot = snapshot;
    }
}
