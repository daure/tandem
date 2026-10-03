use std::time::{Duration, Instant};

use rusqlite::params;

use super::Providers;
use crate::{
    environments::{events::EventStore, gateway, provider_sidecar},
    store::providers::{
        Action, ActionError, Manifest, Status, StreamActionOutcome, validate_stream,
    },
};

impl Providers {
    fn saved_manifest(&self, name: &str) -> Result<Manifest, String> {
        let source: String = self
            .database()?
            .query_row(
                "SELECT manifest FROM provider_launches WHERE namespace = ?1 AND name = ?2",
                params![self.config.namespace, name],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        serde_json::from_str(&source).map_err(|error| error.to_string())
    }

    pub(super) fn prepare_stream_controls(
        &self,
        name: &str,
        only_stream: Option<&str>,
    ) -> Result<(), String> {
        let manifest = self.saved_manifest(name)?;
        EventStore::open(&self.config)
            .and_then(|store| match only_stream {
                Some(stream) => store.prepare_streams_for(&manifest, Some(stream)),
                None => store.prepare_streams(&manifest),
            })
            .map_err(|error| error.to_string())
    }

    pub(super) fn invalidate_stream_controls(&self, name: &str) -> Result<(), String> {
        let manifest = self.saved_manifest(name)?;
        EventStore::open(&self.config)
            .and_then(|store| store.invalidate_streams(&manifest.name))
            .map_err(|error| error.to_string())
    }

    pub(crate) fn stream_action(
        &self,
        name: &str,
        stream: &str,
        action: Action,
    ) -> Result<StreamActionOutcome, ActionError> {
        crate::store::environments::validate_name(name)?;
        validate_stream(stream)?;
        let _package = gateway::shared_lock(&self.config, &format!("provider-package-{name}"))?;
        if action == Action::Logs {
            return Err(ActionError::Unavailable(
                "Logs belong to the provider collector",
            ));
        }
        let _lock = gateway::lock(
            &self.config,
            &format!("provider-{}-{name}", self.config.namespace),
        )?;
        self.ensure_not_deleting(name)?;
        let manifest = match self.saved_manifest(name) {
            Ok(manifest) => manifest,
            Err(_) => self
                .definition(name)?
                .manifest
                .ok_or("provider manifest missing")?,
        };
        if !manifest.stream_control || !manifest.streams.iter().any(|name| name == stream) {
            return Err(ActionError::Unavailable(
                "Provider does not support controls for this stream",
            ));
        }
        let container = self.container(name)?;
        let status = super::container_status(container.as_ref());
        if status != Status::Running && action == Action::Stop {
            return Err(ActionError::Unavailable(
                "Stream is already stopped or its provider is paused",
            ));
        }
        let store = EventStore::open(&self.config).map_err(|error| error.to_string())?;
        if status != Status::Running {
            let previous = store
                .set_ingestion_enabled(&manifest.name, true)
                .map_err(|error| error.to_string())?;
            if let Err(error) = self.start_collector(name, container.as_ref(), Some(stream)) {
                store
                    .set_ingestion_enabled(&manifest.name, previous)
                    .map_err(|error| error.to_string())?;
                return Err(error.into());
            }
        } else {
            provider_sidecar::ensure(&self.config)?;
        }
        let collector = self
            .container(name)?
            .ok_or("collector missing after start")?;
        let id = collector["Id"].as_str().ok_or("collector ID missing")?;
        let revision = if status == Status::Running {
            store.request_stream(&manifest.name, stream, action == Action::Start)
        } else {
            store.stream_revision(&manifest.name, stream)
        }
        .map_err(|error| error.to_string())?;
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if store
                .stream_applied(&manifest.name, stream, revision)
                .map_err(|error| error.to_string())?
            {
                let observed = self.inspect_containers(name, &[id])?;
                if super::container_status(Some(&observed)) != Status::Running {
                    return Err("Provider stopped before stream control completed".into());
                }
                let streams = store
                    .provider_streams(&manifest, Status::Running)
                    .map_err(|error| error.to_string())?;
                let runtime = if action == Action::Stop
                    && !streams.iter().any(|row| row.controllable && row.enabled)
                {
                    store
                        .set_ingestion_enabled(&manifest.name, false)
                        .map_err(|error| error.to_string())?;
                    self.execute(name, Action::Stop)?
                        .runtime
                        .ok_or("collector stop verification missing")?
                } else {
                    super::observed_outcome(String::new(), &observed)?
                        .runtime
                        .ok_or("collector verification missing")?
                };
                let streams = store
                    .provider_streams(&manifest, runtime.status)
                    .map_err(|error| error.to_string())?;
                return Ok(StreamActionOutcome {
                    message: format!("Stream {name}/{stream}: {action:?} (live-only)"),
                    runtime,
                    streams,
                });
            }
            if Instant::now() >= deadline {
                let error = "Collector did not acknowledge stream control within 10 seconds; collection state is unverified";
                store
                    .fail_stream(&manifest.name, stream, revision, error)
                    .map_err(|error| error.to_string())?;
                return Err(error.into());
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}
