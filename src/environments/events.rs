use std::{fs, io::Write, path::PathBuf, time::Duration};

use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};

use super::config::{Config, private_file};
use crate::store::events::{
    Attempt, Batch, Error, FEED_LIMIT, Ingestion, MAX_BATCH, NotificationKind, ProcessingStatus,
    ProviderNotification, Receipt, Record, Snapshot,
};

mod provider_deletion;
mod streams;

#[derive(Clone)]
pub(crate) struct EventStore {
    path: PathBuf,
    namespace: String,
    config: Config,
}

impl EventStore {
    pub(crate) fn open(config: &Config) -> Result<Self, Error> {
        let store = Self {
            path: config.home.join("settings.sqlite3"),
            namespace: config.namespace.clone(),
            config: config.clone(),
        };
        let mut connection = store.connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute_batch(include_str!("../../migrations/0006_events.sql"))?;
        transaction.execute_batch(include_str!("../../migrations/0008_provider_ingestion.sql"))?;
        transaction.execute_batch(include_str!("../../migrations/0013_provider_streams.sql"))?;
        transaction.execute_batch(include_str!("../../migrations/0014_provider_deletions.sql"))?;
        let stream_declarations: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info('provider_streams') WHERE name = 'active')",
            [],
            |row| row.get(0),
        )?;
        if !stream_declarations {
            transaction.execute_batch(
                "ALTER TABLE provider_streams ADD COLUMN active INTEGER NOT NULL DEFAULT 1;",
            )?;
        }
        let stream_start_times: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info('provider_streams') WHERE name = 'requested_at')",
            [],
            |row| row.get(0),
        )?;
        if !stream_start_times {
            transaction
                .execute_batch("ALTER TABLE provider_streams ADD COLUMN requested_at INTEGER;")?;
        }
        let legacy: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info('event_attempts') WHERE name = 'handled_at')",
            [],
            |row| row.get(0),
        )?;
        if legacy {
            transaction
                .execute_batch(include_str!("../../migrations/0009_event_acceptance.sql"))?;
        }
        transaction.execute_batch(include_str!("../../migrations/0010_rules.sql"))?;
        transaction.execute_batch(include_str!("../../migrations/0011_rule_definitions.sql"))?;
        transaction.execute_batch(include_str!(
            "../../migrations/0015_acceptance_workspaces.sql"
        ))?;
        let scoped_dispatch_history: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info('rule_dispatch_starts') WHERE name = 'namespace')",
            [],
            |row| row.get(0),
        )?;
        if !scoped_dispatch_history {
            transaction
                .execute_batch(include_str!("../../migrations/0012_dispatch_history.sql"))?;
        }
        transaction.commit()?;
        Ok(store)
    }

    pub(super) fn connection(&self) -> Result<Connection, Error> {
        match private_file(&self.path, true) {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                if !fs::symlink_metadata(&self.path)
                    .map_err(|error| Error::Storage(error.to_string()))?
                    .file_type()
                    .is_file()
                {
                    return Err(Error::Storage(
                        "event database must be a regular file".into(),
                    ));
                }
            }
            Err(error) => return Err(Error::Storage(error.to_string())),
        }
        let connection = Connection::open(&self.path)?;
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.execute_batch("PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL;")?;
        Ok(connection)
    }

    pub(crate) fn register_provider(&self, name: &str) -> Result<String, Error> {
        crate::store::environments::validate_name(name).map_err(Error::Invalid)?;
        let mut connection = self.connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute(
            "INSERT INTO event_providers(namespace, name, token) VALUES (?1, ?2, lower(hex(randomblob(32))))
             ON CONFLICT(namespace, name) DO NOTHING",
            params![self.namespace, name],
        )?;
        let token = transaction.query_row(
            "SELECT token FROM event_providers WHERE namespace = ?1 AND name = ?2",
            params![self.namespace, name],
            |row| row.get(0),
        )?;
        transaction.commit()?;
        Ok(token)
    }

    fn credentials_directory(&self) -> Result<PathBuf, Error> {
        let root = self
            .path
            .parent()
            .ok_or_else(|| Error::Storage("event database has no parent".into()))?
            .join("provider-credentials");
        let directory = root.join(&self.namespace);
        for path in [&root, &directory] {
            fs::create_dir_all(path).map_err(|error| Error::Storage(error.to_string()))?;
            if !fs::symlink_metadata(path)
                .map_err(|error| Error::Storage(error.to_string()))?
                .file_type()
                .is_dir()
                || fs::canonicalize(path).map_err(|error| Error::Storage(error.to_string()))?
                    != *path
            {
                return Err(Error::Storage(
                    "provider credentials require a real private directory".into(),
                ));
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(path, fs::Permissions::from_mode(0o700))
                    .map_err(|error| Error::Storage(error.to_string()))?;
            }
        }
        let source = directory.to_string_lossy();
        if source.contains(['\'', '\n', '\r']) {
            return Err(Error::Invalid(
                "provider credentials path cannot contain quotes or line breaks".into(),
            ));
        }
        Ok(directory)
    }

    pub(crate) fn setup_developer_credentials(&self) -> Result<PathBuf, Error> {
        let directory = self.credentials_directory()?;
        for name in ["slack", "jira", "datadog", "github"] {
            self.provider_credential_file(name)?;
        }
        let path = directory.join("providers.env");
        write_private(
            &path,
            &format!(
                "TANDEM_PROVIDER_CREDENTIALS_DIR='{}'\n",
                directory.display()
            ),
        )?;
        Ok(path)
    }

    pub(crate) fn provider_credential_file(&self, name: &str) -> Result<PathBuf, Error> {
        let token = self.register_provider(name)?;
        let path = self.credentials_directory()?.join(format!("{name}.token"));
        write_private(&path, &token)?;
        Ok(path)
    }

    fn provider(&self, connection: &Connection, token: &str) -> Result<String, Error> {
        if token.len() != 64 || !token.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(Error::Unauthorized);
        }
        connection
            .query_row(
                "SELECT name FROM event_providers WHERE namespace = ?1 AND token = ?2",
                params![self.namespace, token],
                |row| row.get(0),
            )
            .optional()?
            .ok_or(Error::Unauthorized)
    }

    pub(crate) fn authenticate(&self, token: &str) -> Result<(), Error> {
        self.provider(&self.connection()?, token).map(|_| ())
    }

    pub(crate) fn set_ingestion_enabled(&self, name: &str, enabled: bool) -> Result<bool, Error> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let previous = self.ingestion_enabled(&transaction, name)?;
        if enabled {
            self.ensure_provider_not_deleting(&transaction, name)?;
        }
        transaction.execute(
            "INSERT INTO provider_ingestion(namespace, name, enabled) VALUES (?1, ?2, ?3)
             ON CONFLICT(namespace, name) DO UPDATE SET enabled = excluded.enabled",
            params![self.namespace, name, enabled],
        )?;
        transaction.commit()?;
        Ok(previous)
    }

    fn ingestion_enabled(&self, connection: &Connection, name: &str) -> Result<bool, Error> {
        Ok(connection
            .query_row(
                "SELECT enabled FROM provider_ingestion WHERE namespace = ?1 AND name = ?2",
                params![self.namespace, name],
                |row| row.get(0),
            )
            .optional()?
            .unwrap_or(true))
    }

    pub(crate) fn ingest(&self, token: &str, batch: Batch) -> Result<Ingestion, Error> {
        let _lock = super::rules::catalog::lock(&self.config)?;
        let mut connection = self.connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let provider = self.provider(&transaction, token)?;
        if batch.events.is_empty() || batch.events.len() > MAX_BATCH {
            return Err(Error::Invalid("batch must contain 1–100 events".into()));
        }
        if !self.ingestion_enabled(&transaction, &provider)? {
            for event in &batch.events {
                event.validate()?;
            }
            return Ok(Ingestion {
                receipts: Vec::new(),
                discarded: batch
                    .events
                    .into_iter()
                    .map(|event| event.event_id)
                    .collect(),
            });
        }
        super::rules::catalog::synchronize(&self.config, &transaction)?;
        let mut receipts = Vec::with_capacity(batch.events.len());
        let mut discarded = Vec::new();
        for event in batch.events {
            event.validate()?;
            let enabled = transaction.query_row(
                "SELECT enabled FROM provider_streams WHERE namespace = ?1 AND provider = ?2 AND stream = ?3 AND active = 1",
                params![self.namespace, provider, event.stream], |row| row.get::<_, bool>(0),
            ).optional()?.unwrap_or(true);
            if !enabled {
                discarded.push(event.event_id);
                continue;
            }
            let payload =
                serde_json::to_string(&event).map_err(|error| Error::Invalid(error.to_string()))?;
            let existing: Option<(i64, String)> = transaction.query_row(
                "SELECT sequence, payload FROM events WHERE namespace = ?1 AND provider = ?2 AND event_id = ?3",
                params![self.namespace, provider, event.event_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            ).optional()?;
            if let Some((sequence, previous)) = existing {
                if previous != payload {
                    return Err(Error::Conflict(
                        "event ID already exists with different content".into(),
                    ));
                }
                receipts.push(Receipt {
                    event_id: event.event_id,
                    sequence,
                    duplicate: true,
                });
                continue;
            }
            let timestamp = now();
            transaction.execute(
                "INSERT INTO events(namespace, provider, event_id, payload, received_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![self.namespace, provider, event.event_id, payload, timestamp],
            )?;
            let sequence = transaction.last_insert_rowid();
            let attempt = add_attempt(&transaction, sequence, false, None, &timestamp)?;
            self.notify(
                &transaction,
                &provider,
                &event.event_id,
                sequence,
                attempt,
                NotificationKind::Received,
            )?;
            receipts.push(Receipt {
                event_id: event.event_id,
                sequence,
                duplicate: false,
            });
        }
        transaction.commit()?;
        Ok(Ingestion {
            receipts,
            discarded,
        })
    }

    pub(crate) fn snapshot(&self) -> Result<Snapshot, Error> {
        self.snapshot_for(None)
    }

    pub(crate) fn record(&self, sequence: i64) -> Result<Record, Error> {
        self.snapshot_for(Some(sequence))?
            .records
            .pop()
            .ok_or(Error::NotFound)
    }

    fn snapshot_for(&self, sequence: Option<i64>) -> Result<Snapshot, Error> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        let total = transaction.query_row(
            "SELECT count(*) FROM events WHERE namespace = ?1",
            [&self.namespace],
            |row| row.get(0),
        )?;
        let provider_totals = transaction
            .prepare(
                "SELECT provider, count(*) FROM events WHERE namespace = ?1 GROUP BY provider",
            )?
            .query_map([&self.namespace], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<Result<_, _>>()?;
        let accepted_attempts = transaction.query_row(
            "SELECT count(*) FROM event_attempts p JOIN events e ON e.sequence = p.event_sequence
             WHERE e.namespace = ?1 AND p.status = 'accepted'",
            [&self.namespace],
            |row| row.get::<_, u64>(0),
        )?;
        let provider_handovers = transaction
            .prepare(
                "SELECT e.provider, count(DISTINCT e.sequence) FROM events e
                 JOIN event_attempts p ON p.event_sequence = e.sequence
                 JOIN rule_acceptances a ON a.attempt_id = p.id
                 WHERE e.namespace = ?1 AND json_extract(a.payload, '$.operation_id') IS NOT NULL
                 GROUP BY e.provider",
            )?
            .query_map([&self.namespace], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<Result<_, _>>()?;
        let mut statement = transaction.prepare(
            "SELECT sequence, provider, received_at, payload FROM events
             WHERE namespace = ?1 AND (?3 IS NULL OR sequence = ?3) ORDER BY sequence DESC LIMIT ?2",
        )?;
        let mut records = Vec::new();
        for result in statement.query_map(
            params![self.namespace, FEED_LIMIT as i64, sequence],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            },
        )? {
            let (sequence, provider, received_at, payload) = result?;
            let mut attempts = transaction.prepare(
                "SELECT id, status, replay, created_at, accepted_at FROM event_attempts
                 WHERE event_sequence = ?1 ORDER BY id DESC LIMIT 50",
            )?;
            let attempts = attempts
                .query_map([sequence], |row| {
                    Ok(Attempt {
                        id: row.get(0)?,
                        status: if row.get::<_, String>(1)? == "accepted" {
                            ProcessingStatus::Accepted
                        } else {
                            ProcessingStatus::Pending
                        },
                        replay: row.get(2)?,
                        created_at: row.get(3)?,
                        accepted_at: row.get(4)?,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            records.push(Record {
                sequence,
                provider,
                received_at,
                event: serde_json::from_str(&payload)
                    .map_err(|error| Error::Storage(error.to_string()))?,
                attempts,
                acceptances: super::rules::acceptances(
                    &transaction,
                    Some(sequence),
                    &self.namespace,
                )?,
            });
        }
        Ok(Snapshot {
            records,
            total,
            accepted_attempts: Some(accepted_attempts),
            provider_totals,
            provider_handovers,
            error: None,
        })
    }

    pub(crate) fn replay(&self, sequence: i64, request_id: &str) -> Result<i64, Error> {
        if request_id.is_empty() || request_id.len() > 200 {
            return Err(Error::Invalid(
                "replay request ID must contain 1–200 bytes".into(),
            ));
        }
        let _lock = super::rules::catalog::lock(&self.config)?;
        let mut connection = self.connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (provider, event_id) = self.source(&transaction, sequence)?;
        let existing = transaction
            .query_row(
                "SELECT id FROM event_attempts WHERE event_sequence = ?1 AND request_id = ?2",
                params![sequence, request_id],
                |row| row.get::<_, i64>(0),
            )
            .optional()?;
        if let Some(id) = existing {
            return Ok(id);
        }
        super::rules::catalog::synchronize(&self.config, &transaction)?;
        let attempt = add_attempt(&transaction, sequence, true, Some(request_id), &now())?;
        self.notify(
            &transaction,
            &provider,
            &event_id,
            sequence,
            attempt,
            NotificationKind::Replayed,
        )?;
        transaction.commit()?;
        Ok(attempt)
    }

    pub(crate) fn delete(&self, sequence: Option<i64>) -> Result<i64, Error> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(sequence) = sequence {
            self.source(&transaction, sequence)?;
        }
        let deleting: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM events e JOIN provider_deletions d
             ON d.namespace = e.namespace AND d.source = e.provider
             WHERE e.namespace = ?1 AND (?2 IS NULL OR e.sequence = ?2))",
            params![self.namespace, sequence],
            |row| row.get(0),
        )?;
        if deleting {
            return Err(Error::Conflict(
                "events belong to an incomplete provider deletion; retry delete_provider".into(),
            ));
        }
        let active: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM rule_acceptances a
             JOIN event_attempts p ON p.id = a.attempt_id JOIN events e ON e.sequence = p.event_sequence
             WHERE e.namespace = ?1 AND (?2 IS NULL OR e.sequence = ?2)
             AND json_extract(a.payload, '$.status') IN ('provisioning', 'launching'))",
            params![self.namespace, sequence],
            |row| row.get(0),
        )?;
        if active {
            return Err(Error::Conflict(
                "events have active rule actions; try again when they finish".into(),
            ));
        }
        transaction.execute(
            "DELETE FROM provider_notifications WHERE namespace = ?1
             AND json_extract(payload, '$.sequence') IN
             (SELECT sequence FROM events WHERE namespace = ?1 AND (?2 IS NULL OR sequence = ?2))",
            params![self.namespace, sequence],
        )?;
        for table in ["rule_evaluations", "rule_acceptances"] {
            transaction.execute(
                &format!(
                    "DELETE FROM {table} WHERE attempt_id IN
                     (SELECT p.id FROM event_attempts p JOIN events e ON e.sequence = p.event_sequence
                      WHERE e.namespace = ?1 AND (?2 IS NULL OR e.sequence = ?2))"
                ),
                params![self.namespace, sequence],
            )?;
        }
        transaction.execute(
            "DELETE FROM event_attempts WHERE event_sequence IN
             (SELECT sequence FROM events WHERE namespace = ?1 AND (?2 IS NULL OR sequence = ?2))",
            params![self.namespace, sequence],
        )?;
        let count = transaction.execute(
            "DELETE FROM events WHERE namespace = ?1 AND (?2 IS NULL OR sequence = ?2)",
            params![self.namespace, sequence],
        )?;
        transaction.commit()?;
        Ok(count as i64)
    }

    fn source(&self, connection: &Connection, sequence: i64) -> Result<(String, String), Error> {
        connection
            .query_row(
                "SELECT provider, event_id FROM events WHERE namespace = ?1 AND sequence = ?2",
                params![self.namespace, sequence],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?
            .ok_or(Error::NotFound)
    }

    fn notify(
        &self,
        transaction: &Transaction<'_>,
        provider: &str,
        event_id: &str,
        sequence: i64,
        attempt_id: i64,
        kind: NotificationKind,
    ) -> Result<(), Error> {
        transaction.execute(
            "INSERT INTO provider_notifications(namespace, provider, payload) VALUES (?1, ?2, '')",
            params![self.namespace, provider],
        )?;
        let id = transaction.last_insert_rowid();
        let notification = ProviderNotification {
            notification_id: id,
            event_id: event_id.into(),
            sequence,
            attempt_id,
            kind,
            created_at: now(),
            dispatch_id: None,
            rule_name: None,
            rule_revision: None,
            instance: None,
        };
        transaction.execute(
            "UPDATE provider_notifications SET payload = ?2 WHERE id = ?1",
            params![
                id,
                serde_json::to_string(&notification)
                    .map_err(|error| Error::Storage(error.to_string()))?
            ],
        )?;
        Ok(())
    }

    pub(crate) fn notifications(&self, token: &str) -> Result<Vec<ProviderNotification>, Error> {
        let connection = self.connection()?;
        let provider = self.provider(&connection, token)?;
        let mut statement = connection.prepare(
            "SELECT payload FROM provider_notifications WHERE namespace = ?1 AND provider = ?2
             AND acknowledged = 0 ORDER BY id LIMIT 100",
        )?;
        statement
            .query_map(params![self.namespace, provider], |row| {
                row.get::<_, String>(0)
            })?
            .map(|payload| {
                serde_json::from_str(&payload?).map_err(|error| Error::Storage(error.to_string()))
            })
            .collect()
    }

    pub(crate) fn acknowledge(&self, token: &str, notification: i64) -> Result<(), Error> {
        let connection = self.connection()?;
        let provider = self.provider(&connection, token)?;
        if connection.execute(
            "UPDATE provider_notifications SET acknowledged = 1 WHERE namespace = ?1 AND provider = ?2 AND id = ?3",
            params![self.namespace, provider, notification],
        )? == 0 { return Err(Error::NotFound); }
        Ok(())
    }
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn write_private(path: &std::path::Path, contents: &str) -> Result<(), Error> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if !metadata.file_type().is_file() => {
            return Err(Error::Storage(
                "provider credential must be a regular file".into(),
            ));
        }
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
            return Err(Error::Storage(error.to_string()));
        }
        _ => {}
    }
    let parent = path
        .parent()
        .ok_or_else(|| Error::Storage("credential path has no parent".into()))?;
    let mut file = tempfile::NamedTempFile::new_in(parent)
        .map_err(|error| Error::Storage(error.to_string()))?;
    file.write_all(contents.as_bytes())
        .and_then(|()| file.as_file().sync_all())
        .map_err(|error| Error::Storage(error.to_string()))?;
    file.persist(path)
        .map_err(|error| Error::Storage(error.to_string()))?;
    Ok(())
}

impl From<rusqlite::Error> for Error {
    fn from(error: rusqlite::Error) -> Self {
        Self::Storage(error.to_string())
    }
}

fn add_attempt(
    transaction: &Transaction<'_>,
    sequence: i64,
    replay: bool,
    request_id: Option<&str>,
    timestamp: &str,
) -> Result<i64, Error> {
    transaction.execute(
        "INSERT INTO event_attempts(event_sequence, status, replay, request_id, created_at)
         VALUES (?1, 'pending', ?2, ?3, ?4)",
        params![sequence, replay, request_id, timestamp],
    )?;
    let attempt = transaction.last_insert_rowid();
    super::rules::queue_attempt(transaction, attempt, sequence)?;
    Ok(attempt)
}

#[cfg(test)]
#[path = "tests/events.rs"]
pub(crate) mod tests;
