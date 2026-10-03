use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use tokio::sync::oneshot;

use super::AppService;
use crate::{
    environments::{config::Config, provider_sidecar, providers::Providers},
    store::providers::{Action, ActionError, ActionOutcome, Snapshot, Status},
};

mod deletion;

pub(super) struct Integration {
    manager: Providers,
    config: Config,
    snapshot: Mutex<Snapshot>,
    revision: AtomicU64,
    operations: Mutex<BTreeMap<String, Action>>,
    polling: AtomicBool,
    last_poll: Mutex<Instant>,
}

impl Integration {
    pub(super) fn new(config: &Config) -> Result<Self, String> {
        Ok(Self {
            manager: Providers::new(config)?,
            config: config.clone(),
            snapshot: Mutex::new(Snapshot::default()),
            revision: AtomicU64::new(0),
            operations: Mutex::new(BTreeMap::new()),
            polling: AtomicBool::new(false),
            last_poll: Mutex::new(Instant::now() - Duration::from_secs(3)),
        })
    }

    fn publish_observation(&self, revision: u64, observed: Result<Snapshot, String>) -> bool {
        let mut snapshot = self
            .snapshot
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if self.revision.load(Ordering::Acquire) != revision {
            return false;
        }
        match observed {
            Ok(current) => *snapshot = current,
            Err(error) => snapshot.error = Some(error),
        }
        true
    }

    fn complete_action(
        &self,
        name: &str,
        action: Action,
        result: &Result<ActionOutcome, ActionError>,
    ) {
        let mut snapshot = self
            .snapshot
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(provider) = snapshot
            .providers
            .iter_mut()
            .find(|provider| provider.name == name)
        {
            match result {
                Ok(outcome) => {
                    if let Some(runtime) = &outcome.runtime {
                        provider.status = runtime.status;
                        provider.container_id = Some(runtime.container_id.clone());
                        provider.error = None;
                        for stream in &mut provider.streams {
                            stream.status = if runtime.status == Status::Running {
                                if stream.controllable {
                                    stream.enabled = true;
                                    stream.error = None;
                                    Status::Starting
                                } else {
                                    Status::Unknown
                                }
                            } else {
                                runtime.status
                            };
                        }
                    }
                }
                Err(ActionError::Failed(error)) if action != Action::Logs => {
                    provider.status = Status::Unknown;
                    provider.error = Some(error.clone());
                }
                Err(_) => {}
            }
        }
        self.revision.fetch_add(1, Ordering::AcqRel);
        self.operations
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(name);
    }
}

struct PollGuard(Arc<Integration>);
impl Drop for PollGuard {
    fn drop(&mut self) {
        self.0.polling.store(false, Ordering::Release);
    }
}

impl AppService {
    pub(crate) fn provider_snapshot(&self) -> Snapshot {
        let snapshot = self
            .providers
            .snapshot
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let operations = self
            .providers
            .operations
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let mut snapshot = snapshot.clone();
        for provider in &mut snapshot.providers {
            provider.operation = operations.get(&provider.name).copied();
            for stream in &mut provider.streams {
                stream.operation = operations
                    .get(&format!("{}/{}", provider.name, stream.name))
                    .copied()
                    .or(provider.operation.filter(|_| stream.controllable));
            }
            if provider.error.is_none() {
                provider.error = snapshot.error.clone();
            }
        }
        snapshot
    }

    pub(crate) fn poll_providers(&self) {
        let mut last = self
            .providers
            .last_poll
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if last.elapsed() < Duration::from_secs(2)
            || self.providers.polling.swap(true, Ordering::AcqRel)
        {
            return;
        }
        *last = Instant::now();
        let integration = Arc::clone(&self.providers);
        self.runtime.spawn_blocking(move || {
            let _guard = PollGuard(integration.clone());
            let revision = integration.revision.load(Ordering::Acquire);
            let mut observed = integration.manager.snapshot();
            if let Ok(snapshot) = &mut observed
                && snapshot.providers.iter().any(|provider| {
                    matches!(
                        provider.status,
                        crate::store::providers::Status::Running
                            | crate::store::providers::Status::Paused
                    )
                })
                && let Err(error) = provider_sidecar::ensure(&integration.config)
            {
                snapshot.error = Some(format!("Provider sidecar: {error}"));
            }
            integration.publish_observation(revision, observed);
        });
    }

    pub(crate) fn provider_action(
        &self,
        name: String,
        action: Action,
        confirmed: bool,
    ) -> oneshot::Receiver<Result<String, ActionError>> {
        let (sender, receiver) = oneshot::channel();
        if action != Action::Logs && !confirmed {
            let _ = sender.send(Err(
                "confirmation_required: provider lifecycle uses trusted Docker privileges".into(),
            ));
            return receiver;
        }
        let integration = Arc::clone(&self.providers);
        {
            let mut operations = integration
                .operations
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if operations
                .keys()
                .any(|key| key == &name || key.starts_with(&format!("{name}/")))
            {
                let _ = sender.send(Err(ActionError::Unavailable(
                    "A provider operation is already in progress; wait for it to finish",
                )));
                return receiver;
            }
            operations.insert(name.clone(), action);
        }
        self.runtime.spawn_blocking(move || {
            let result = integration.manager.action(&name, action);
            integration.complete_action(&name, action, &result);
            let _ = sender.send(result.map(|outcome| outcome.message));
        });
        receiver
    }

    pub(crate) fn provider_stream_action(
        &self,
        name: String,
        stream: String,
        action: Action,
        confirmed: bool,
    ) -> oneshot::Receiver<Result<String, ActionError>> {
        let (sender, receiver) = oneshot::channel();
        if !confirmed {
            let _ = sender.send(Err(
                "confirmation_required: stream lifecycle controls trusted provider collection"
                    .into(),
            ));
            return receiver;
        }
        let integration = Arc::clone(&self.providers);
        let key = format!("{name}/{stream}");
        {
            let mut operations = integration
                .operations
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if operations
                .keys()
                .any(|key| key == &name || key.starts_with(&format!("{name}/")))
            {
                let _ = sender.send(Err(ActionError::Unavailable(
                    "A provider or stream operation is already in progress",
                )));
                return receiver;
            }
            operations.insert(key.clone(), action);
        }
        self.runtime.spawn_blocking(move || {
            let result = integration.manager.stream_action(&name, &stream, action);
            let mut snapshot = integration
                .snapshot
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if let Some(provider) = snapshot
                .providers
                .iter_mut()
                .find(|provider| provider.name == name)
            {
                match &result {
                    Ok(outcome) => {
                        provider.status = outcome.runtime.status;
                        provider.container_id = Some(outcome.runtime.container_id.clone());
                        provider.error = None;
                        provider.streams = outcome.streams.clone();
                    }
                    Err(ActionError::Failed(error)) => {
                        if let Some(row) =
                            provider.streams.iter_mut().find(|row| row.name == stream)
                        {
                            row.status = Status::Unknown;
                            row.error = Some(error.clone());
                        }
                    }
                    Err(_) => {}
                }
            }
            integration.revision.fetch_add(1, Ordering::AcqRel);
            integration
                .operations
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .remove(&key);
            drop(snapshot);
            let _ = sender.send(result.map(|outcome| outcome.message));
        });
        receiver
    }

    pub(crate) async fn list_providers(&self) -> Result<Snapshot, String> {
        let manager = self.providers.manager.clone();
        self.runtime
            .spawn_blocking(move || manager.snapshot())
            .await
            .map_err(|error| error.to_string())?
    }

    pub(crate) fn run_provider_sidecar_worker(
        &self,
        bind: std::net::SocketAddr,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let lease = provider_sidecar::Lease::acquire(&self.environments.config)?;
        let runtime = tokio::runtime::Runtime::new()?;
        let result = runtime.block_on(crate::events_http::run_owned(self.clone(), bind, lease));
        drop(runtime);
        result
    }
}

#[cfg(test)]
#[path = "tests/provider_ingestion.rs"]
mod tests;
