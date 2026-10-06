use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use tokio::sync::oneshot;

use super::AppService;
use crate::{
    environments::{config::Config, events::EventStore},
    store::events::{Batch, Deletion, Error, Ingestion, ProviderNotification, Snapshot},
};

pub(super) struct Integration {
    store: EventStore,
    snapshot: Mutex<Snapshot>,
    polling: AtomicBool,
    last_poll: Mutex<Instant>,
}

impl Integration {
    pub(super) fn new(config: &Config) -> Result<Self, Error> {
        Ok(Self {
            store: EventStore::open(config)?,
            snapshot: Mutex::new(Snapshot::default()),
            polling: AtomicBool::new(false),
            last_poll: Mutex::new(Instant::now() - Duration::from_secs(1)),
        })
    }
}

struct PollGuard(Arc<Integration>);

impl Drop for PollGuard {
    fn drop(&mut self) {
        self.0.polling.store(false, Ordering::Release);
    }
}

impl AppService {
    pub(crate) fn open_event_link(
        &self,
        event: &crate::store::events::Event,
    ) -> Result<bool, String> {
        let Some(url) = event
            .url
            .as_deref()
            .map(str::trim)
            .filter(|url| !url.is_empty())
        else {
            return Ok(false);
        };
        reqwest::Url::parse(url)
            .map_err(|_| "Event link must be a valid absolute URL".to_string())?;
        self.open_system_target(url)?;
        Ok(true)
    }

    pub(crate) fn event_snapshot(&self) -> Snapshot {
        self.events
            .snapshot
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    pub(crate) fn poll_events(&self) {
        let mut last_poll = self
            .events
            .last_poll
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if last_poll.elapsed() < Duration::from_millis(500)
            || self.events.polling.swap(true, Ordering::AcqRel)
        {
            return;
        }
        *last_poll = Instant::now();
        let events = Arc::clone(&self.events);
        self.runtime.spawn_blocking(move || {
            let _guard = PollGuard(Arc::clone(&events));
            let result = events.store.snapshot();
            let mut snapshot = events
                .snapshot
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            match result {
                Ok(current) => *snapshot = current,
                Err(error) => snapshot.error = Some(error.to_string()),
            }
        });
    }

    async fn event_request<T: Send + 'static>(
        &self,
        request: impl FnOnce(EventStore) -> Result<T, Error> + Send + 'static,
    ) -> Result<T, Error> {
        let store = self.events.store.clone();
        self.runtime
            .spawn_blocking(move || request(store))
            .await
            .map_err(|error| Error::Storage(format!("event worker stopped: {error}")))?
    }

    pub(crate) async fn authenticate_provider(&self, token: String) -> Result<(), Error> {
        self.event_request(move |store| store.authenticate(&token))
            .await
    }

    pub(crate) async fn provider_stream_controls(
        &self,
        token: String,
    ) -> Result<Vec<crate::store::providers::StreamControl>, Error> {
        self.event_request(move |store| store.stream_controls(&token))
            .await
    }

    pub(crate) async fn acknowledge_provider_stream(
        &self,
        token: String,
        control: crate::store::providers::StreamControl,
    ) -> Result<(), Error> {
        self.event_request(move |store| store.acknowledge_stream(&token, &control))
            .await
    }

    pub(crate) async fn ingest_events(
        &self,
        token: String,
        batch: Batch,
    ) -> Result<Ingestion, Error> {
        self.event_request(move |store| store.ingest(&token, batch))
            .await
    }

    pub(crate) async fn list_events(&self) -> Result<Snapshot, String> {
        self.event_request(|store| store.snapshot())
            .await
            .map_err(|error| error.to_string())
    }

    pub(crate) async fn replay_event_request(
        &self,
        sequence: i64,
        request_id: String,
        confirmed: bool,
    ) -> Result<i64, String> {
        if !confirmed {
            return Err("confirmation_required: replay evaluates current rules and can create fresh instances and model prompts".into());
        }
        self.event_request(move |store| store.replay(sequence, &request_id))
            .await
            .map_err(|error| error.to_string())
    }

    pub(crate) async fn provider_notifications(
        &self,
        token: String,
    ) -> Result<Vec<ProviderNotification>, Error> {
        self.event_request(move |store| store.notifications(&token))
            .await
    }

    pub(crate) async fn acknowledge_provider_notification(
        &self,
        token: String,
        id: i64,
    ) -> Result<(), Error> {
        self.event_request(move |store| store.acknowledge(&token, id))
            .await
    }

    pub(crate) fn replay_event(&self, sequence: i64) -> oneshot::Receiver<Result<i64, Error>> {
        let store = self.events.store.clone();
        let (sender, receiver) = oneshot::channel();
        let request = format!(
            "{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        );
        self.runtime.spawn_blocking(move || {
            let _ = sender.send(store.replay(sequence, &request));
        });
        receiver
    }

    pub(crate) fn delete_events(&self, target: Deletion) -> oneshot::Receiver<Result<i64, Error>> {
        let store = self.events.store.clone();
        let (sender, receiver) = oneshot::channel();
        self.runtime.spawn_blocking(move || {
            let _ = sender.send(store.delete(target));
        });
        receiver
    }

    pub(crate) fn setup_developer_providers(&self) -> Result<String, Error> {
        self.events
            .store
            .setup_developer_credentials()
            .map(|path| path.display().to_string())
    }

    #[cfg(test)]
    pub(crate) fn register_provider_for_tests(&self, name: &str) -> String {
        self.events.store.register_provider(name).unwrap()
    }
}

#[cfg(test)]
#[path = "tests/event_deletion.rs"]
mod tests;
