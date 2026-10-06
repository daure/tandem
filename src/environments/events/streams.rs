use std::collections::BTreeMap;

use rusqlite::{OptionalExtension, TransactionBehavior, params};

use super::EventStore;
use crate::store::{
    events::Error,
    providers::{Manifest, Status, Stream, StreamControl, validate_stream},
};

impl EventStore {
    pub(crate) fn prepare_streams(&self, manifest: &Manifest) -> Result<(), Error> {
        self.prepare_streams_for(manifest, None)
    }

    pub(crate) fn prepare_streams_for(
        &self,
        manifest: &Manifest,
        only_stream: Option<&str>,
    ) -> Result<(), Error> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        transaction.execute(
            "UPDATE provider_streams SET active = 0, revision = revision + 1, applied_revision = NULL, applied_at = NULL
             WHERE namespace = ?1 AND provider = ?2",
            params![self.namespace, manifest.name],
        )?;
        if !manifest.stream_control {
            transaction.commit()?;
            return Ok(());
        }
        for stream in &manifest.streams {
            transaction.execute(
                "INSERT INTO provider_streams(namespace, provider, stream, enabled, requested_at)
                 VALUES (?1, ?2, ?3, ?4, unixepoch())
                 ON CONFLICT(namespace, provider, stream) DO UPDATE SET
                 active = 1, enabled = excluded.enabled, revision = revision + 1,
                 applied_revision = NULL, applied_at = NULL, requested_at = unixepoch(), error = NULL",
                params![self.namespace, manifest.name, stream.name, only_stream.is_none_or(|name| name == stream.name)],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub(crate) fn invalidate_streams(&self, provider: &str) -> Result<(), Error> {
        self.connection()?.execute(
            "UPDATE provider_streams SET revision = revision + 1, applied_revision = NULL, applied_at = NULL, requested_at = NULL
             WHERE namespace = ?1 AND provider = ?2",
            params![self.namespace, provider],
        )?;
        Ok(())
    }

    pub(crate) fn collector_started(&self, provider: &str) -> Result<(), Error> {
        self.connection()?.execute(
            "UPDATE provider_streams SET requested_at = unixepoch()
             WHERE namespace = ?1 AND provider = ?2 AND active = 1 AND applied_revision IS NULL",
            params![self.namespace, provider],
        )?;
        Ok(())
    }

    pub(crate) fn stream_controls(&self, token: &str) -> Result<Vec<StreamControl>, Error> {
        let connection = self.connection()?;
        let provider = self.provider(&connection, token)?;
        Ok(connection.prepare(
            "SELECT stream, enabled, revision FROM provider_streams WHERE namespace = ?1 AND provider = ?2 AND active = 1 ORDER BY stream",
        )?.query_map(params![self.namespace, provider], |row| Ok(StreamControl {
            stream: row.get(0)?, enabled: row.get(1)?, revision: row.get(2)?,
        }))?.collect::<Result<_, _>>()?)
    }

    pub(crate) fn acknowledge_stream(
        &self,
        token: &str,
        control: &StreamControl,
    ) -> Result<(), Error> {
        validate_stream(&control.stream).map_err(Error::Invalid)?;
        let mut connection = self.connection()?;
        // Acquire the writer before reading credentials so WAL snapshot upgrades cannot bypass the busy timeout.
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let provider = self.provider(&transaction, token)?;
        let updated = transaction.execute(
            "UPDATE provider_streams SET applied_revision = revision, applied_at = unixepoch(), error = NULL
             WHERE namespace = ?1 AND provider = ?2 AND stream = ?3 AND revision = ?4 AND enabled = ?5 AND active = 1",
            params![self.namespace, provider, control.stream, control.revision, control.enabled],
        )?;
        if updated != 1 {
            return Err(Error::Conflict(
                "stream control was superseded or is unavailable".into(),
            ));
        }
        transaction.commit()?;
        Ok(())
    }

    pub(crate) fn request_stream(
        &self,
        provider: &str,
        stream: &str,
        enabled: bool,
    ) -> Result<i64, Error> {
        validate_stream(stream).map_err(Error::Invalid)?;
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        let revision = transaction.query_row(
            "UPDATE provider_streams SET enabled = ?4, revision = revision + 1,
             applied_revision = NULL, applied_at = NULL, requested_at = unixepoch(), error = NULL
             WHERE namespace = ?1 AND provider = ?2 AND stream = ?3 AND active = 1 RETURNING revision",
            params![self.namespace, provider, stream, enabled], |row| row.get(0),
        ).optional()?.ok_or(Error::NotFound)?;
        transaction.commit()?;
        Ok(revision)
    }

    pub(crate) fn stream_applied(
        &self,
        provider: &str,
        stream: &str,
        revision: i64,
    ) -> Result<bool, Error> {
        Ok(self.connection()?.query_row(
            "SELECT revision = ?4 AND applied_revision = ?4 AND applied_at >= unixepoch() - 5
             FROM provider_streams WHERE namespace = ?1 AND provider = ?2 AND stream = ?3 AND active = 1",
            params![self.namespace, provider, stream, revision], |row| row.get::<_, Option<bool>>(0),
        ).optional()?.flatten().unwrap_or(false))
    }

    pub(crate) fn stream_revision(&self, provider: &str, stream: &str) -> Result<i64, Error> {
        self.connection()?.query_row(
            "SELECT revision FROM provider_streams WHERE namespace = ?1 AND provider = ?2 AND stream = ?3 AND active = 1",
            params![self.namespace, provider, stream], |row| row.get(0),
        ).optional()?.ok_or(Error::NotFound)
    }

    pub(crate) fn fail_stream(
        &self,
        provider: &str,
        stream: &str,
        revision: i64,
        error: &str,
    ) -> Result<(), Error> {
        self.connection()?.execute(
            "UPDATE provider_streams SET error = ?5 WHERE namespace = ?1 AND provider = ?2 AND stream = ?3
             AND revision = ?4 AND applied_revision IS NULL",
            params![self.namespace, provider, stream, revision, error],
        )?;
        Ok(())
    }

    pub(crate) fn provider_streams(
        &self,
        manifest: &Manifest,
        runtime: Status,
    ) -> Result<Vec<Stream>, Error> {
        let connection = self.connection()?;
        let mut streams = BTreeMap::new();
        for declaration in &manifest.streams {
            streams.insert(
                declaration.name.clone(),
                Stream {
                    name: declaration.name.clone(),
                    profile: Some(declaration.profile.clone()),
                    controllable: manifest.stream_control,
                    enabled: true,
                    status: Status::Unknown,
                    operation: None,
                    error: None,
                    total: 0,
                    handovers: 0,
                },
            );
        }
        let mut statement = connection.prepare(
            "SELECT json_extract(e.payload, '$.stream'), count(DISTINCT e.sequence),
              count(DISTINCT CASE WHEN json_extract(a.payload, '$.operation_id') IS NOT NULL THEN e.sequence END),
              CASE WHEN count(DISTINCT json_extract(e.payload, '$.profile')) = 1
                   THEN min(json_extract(e.payload, '$.profile')) END
             FROM events e LEFT JOIN event_attempts p ON p.event_sequence = e.sequence
             LEFT JOIN rule_acceptances a ON a.attempt_id = p.id
             WHERE e.namespace = ?1 AND e.provider = ?2 GROUP BY json_extract(e.payload, '$.stream')",
        )?;
        for result in statement.query_map(params![self.namespace, manifest.name], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, u64>(1)?,
                row.get::<_, u64>(2)?,
                row.get::<_, Option<String>>(3)?,
            ))
        })? {
            let (name, total, handovers, profile) = result?;
            let stream = streams.entry(name.clone()).or_insert(Stream {
                name,
                profile,
                controllable: false,
                enabled: true,
                status: Status::Unknown,
                operation: None,
                error: None,
                total: 0,
                handovers: 0,
            });
            stream.total = total;
            stream.handovers = handovers;
        }
        let mut statement = connection.prepare(
            "SELECT stream, enabled, applied_revision = revision AND applied_at >= unixepoch() - 5, error,
             applied_revision IS NULL AND requested_at >= unixepoch() - 10
             FROM provider_streams WHERE namespace = ?1 AND provider = ?2 AND active = 1",
        )?;
        for result in statement.query_map(params![self.namespace, manifest.name], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, bool>(1)?,
                row.get::<_, Option<bool>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<bool>>(4)?,
            ))
        })? {
            let (name, enabled, applied, error, starting) = result?;
            if let Some(stream) = streams.get_mut(&name) {
                stream.enabled = enabled;
                stream.error = error;
                if applied == Some(true) && stream.controllable {
                    stream.status = if enabled {
                        Status::Running
                    } else {
                        Status::Stopped
                    };
                } else if enabled && starting == Some(true) && stream.error.is_none() {
                    stream.status = Status::Starting;
                }
            }
        }
        for stream in streams.values_mut() {
            if runtime != Status::Running {
                stream.status = runtime;
            }
        }
        Ok(streams.into_values().collect())
    }
}
