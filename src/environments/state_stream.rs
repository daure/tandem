use std::{collections::BTreeMap, sync::Arc};

use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};
use serde_json::{Value, json};

use super::{config::Config, database::Pool};
use crate::store::{
    opencode::{PromptedSession, Session},
    state_stream::{self, StateSnapshot},
};

mod edits;
mod identity;
mod live;
mod observation;

pub(crate) struct Store {
    pub(crate) config: Config,
    pool: Arc<Pool>,
    runtime_pool: Arc<Pool>,
}

pub(crate) struct Boundary {
    pub(crate) cursor: String,
    pub(crate) snapshot: StateSnapshot,
    pub(crate) events: Vec<Value>,
    pub(crate) reset: bool,
}

impl Store {
    pub(crate) fn new(config: &Config) -> Result<Self, String> {
        let path = config.home.join("state-stream.sqlite3");
        match super::config::private_file(&path, true) {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                use std::os::unix::fs::{MetadataExt, PermissionsExt};
                let metadata =
                    std::fs::symlink_metadata(&path).map_err(|error| error.to_string())?;
                if !metadata.is_file() || metadata.uid() != unsafe { libc::geteuid() } {
                    return Err(
                        "state database must be a regular file owned by the current user".into(),
                    );
                }
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
                    .map_err(|error| error.to_string())?;
            }
            Err(error) => return Err(error.to_string()),
        }
        let store = Self {
            config: config.clone(),
            pool: Arc::new(Pool::new(path)),
            runtime_pool: Arc::new(Pool::new(config.home.join("settings.sqlite3"))),
        };
        let connection = store.connection()?;
        let epoch = format!(
            "{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        );
        connection.execute(
            "INSERT OR IGNORE INTO state_stream_heads(namespace, epoch, snapshot) VALUES (?1, ?2, ?3)",
            params![config.namespace, epoch, serde_json::to_string(&StateSnapshot::empty()).map_err(|error| error.to_string())?],
        ).map_err(|error| error.to_string())?;
        Ok(store)
    }

    pub(super) fn connection(&self) -> Result<super::database::Lease, String> {
        self.pool.get(|connection| {
            connection.execute_batch("PRAGMA journal_mode = WAL;")?;
            connection.execute_batch(include_str!("../../migrations/0019_state_stream.sql"))
        })
    }

    pub(crate) fn begin_creation(
        &self,
        directory: &str,
        instance: &str,
        origin: Option<&str>,
    ) -> Result<String, String> {
        state_stream::validate_origin(origin)?;
        let id = format!(
            "{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        );
        self.prepare_identity_receipt(&id, directory)?;
        self.connection()?.execute(
            "INSERT INTO session_creation_requests(namespace, id, directory, instance, origin, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![self.config.namespace, id, directory, instance, origin, state_stream::now()],
        ).map_err(|error| error.to_string())?;
        Ok(id)
    }

    pub(crate) fn finish_creation(
        &self,
        request: &str,
        instance: &str,
        receipt: &PromptedSession,
    ) -> Result<(), String> {
        if receipt.creation_confirmed_absent {
            return self.cancel_creation(request);
        }
        let (Some(id), Some(server)) = (&receipt.session_id, &receipt.server) else {
            // Unknown creation remains quarantined; it must not acquire an inferred origin.
            return Ok(());
        };
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        let request_record: Option<(String, Option<String>)> = transaction.query_row(
            "SELECT directory, origin FROM session_creation_requests WHERE namespace = ?1 AND id = ?2",
            params![self.config.namespace, request], |row| Ok((row.get(0)?, row.get(1)?)),
        ).optional().map_err(|error| error.to_string())?;
        let Some((directory, origin)) = request_record else {
            return Ok(());
        };
        let session = json!({
            "session_id": id, "server": server, "instance": instance, "directory": directory,
            "title": "", "activity": "unknown", "attached": true, "panes": [receipt.pane],
            "stale": true, "availability": "unverified", "origins": origin.iter().collect::<Vec<_>>()
        });
        self.bind(&transaction, session, origin.as_deref())?;
        transaction.execute("UPDATE session_creation_requests SET server = ?3, session_id = ?4, completed = 1 WHERE namespace = ?1 AND id = ?2", params![self.config.namespace, request, server, id]).map_err(|error| error.to_string())?;
        transaction.commit().map_err(|error| error.to_string())?;
        self.remove_identity_receipt(request);
        Ok(())
    }

    pub(crate) fn associate_origin(
        &self,
        session: &Session,
        instance: &str,
        origin: Option<&str>,
    ) -> Result<(), String> {
        state_stream::validate_origin(origin)?;
        self.recover_identities()?;
        let Some(origin) = origin else {
            return Ok(());
        };
        let value = json!({
            "session_id": session.id, "server": session.server, "instance": instance,
            "directory": session.directory, "title": session.title, "activity": "unknown",
            "attached": session.attached(), "panes": session.panes,
            "stale": true, "availability": "unverified", "origins": [origin]
        });
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        let unresolved: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM session_creation_requests WHERE namespace = ?1 AND directory = ?2 AND session_id IS NULL)",
            params![self.config.namespace, session.directory], |row| row.get(0),
        ).map_err(|error| error.to_string())?;
        if unresolved
            && !self.snapshot(&transaction)?.sessions["sessions"]
                .as_array()
                .is_some_and(|sessions| {
                    sessions.iter().any(|known| {
                        state_stream::session_key(known) == state_stream::session_key(&value)
                    })
                })
        {
            return Err("Session creation is unresolved in this workspace; inspect the conversation before retrying".into());
        }
        self.bind(&transaction, value, Some(origin))?;
        transaction.commit().map_err(|error| error.to_string())
    }

    pub(super) fn bind(
        &self,
        transaction: &Transaction<'_>,
        mut session: Value,
        origin: Option<&str>,
    ) -> Result<(), String> {
        let edits = self.edits_in(transaction)?;
        let key = (
            session["server"].as_str().unwrap_or_default().to_owned(),
            session["session_id"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
        );
        if matches!(edits.get(&key), Some(None)) {
            return Ok(());
        }
        for key in [
            "model",
            "variant",
            "agent",
            "context_tokens",
            "context_limit",
        ] {
            if session.get(key).is_none() {
                session[key] = Value::Null;
            }
        }
        let mut snapshot = self.snapshot(transaction)?;
        let key = state_stream::session_key(&session);
        let sessions = snapshot.sessions["sessions"]
            .as_array_mut()
            .ok_or("invalid stream session state")?;
        let previous = sessions
            .iter()
            .position(|value| state_stream::session_key(value) == key);
        if let Some(index) = previous {
            let mut known = sessions[index].clone();
            if known["instance"].is_null() && !session["instance"].is_null() {
                known["instance"] = session["instance"].clone();
            }
            if known["panes"].as_array().is_some_and(Vec::is_empty)
                && session["panes"]
                    .as_array()
                    .is_some_and(|panes| !panes.is_empty())
            {
                known["panes"] = session["panes"].clone();
                known["attached"] = session["attached"].clone();
            }
            session = known;
        }
        if let Some(origin) = origin {
            transaction.execute(
                "INSERT OR IGNORE INTO session_origins(namespace, server, session_id, origin, directory, session) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![self.config.namespace, session["server"].as_str(), session["session_id"].as_str(), origin, session["directory"].as_str(), session.to_string()],
            ).map_err(|error| error.to_string())?;
        }
        session["origins"] = json!(
            self.origins_in(transaction)?
                .get(&key)
                .cloned()
                .unwrap_or_default()
        );
        match previous {
            Some(index) => sessions[index] = session,
            None => sessions.push(session),
        }
        self.commit_snapshot(transaction, &snapshot)?;
        Ok(())
    }

    pub(crate) fn origins(&self) -> Result<BTreeMap<String, Vec<String>>, String> {
        let connection = self.connection()?;
        self.origins_in(&connection)
    }

    fn origins_in(
        &self,
        connection: &rusqlite::Connection,
    ) -> Result<BTreeMap<String, Vec<String>>, String> {
        let mut statement = connection.prepare_cached("SELECT server, session_id, origin FROM session_origins WHERE namespace = ?1 ORDER BY rowid").map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([&self.config.namespace], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .map_err(|error| error.to_string())?;
        let mut origins = BTreeMap::<String, Vec<String>>::new();
        for row in rows {
            let (server, id, origin) = row.map_err(|error| error.to_string())?;
            origins
                .entry(serde_json::to_string(&(server, id)).unwrap())
                .or_default()
                .push(origin);
        }
        Ok(origins)
    }

    pub(crate) fn retained_sessions(&self) -> Result<Vec<Session>, String> {
        let connection = self.connection()?;
        let mut statement = connection
            .prepare_cached("SELECT DISTINCT session FROM session_origins WHERE namespace = ?1")
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([&self.config.namespace], |row| row.get::<_, String>(0))
            .map_err(|error| error.to_string())?;
        rows.map(|row| {
            let value: Value = serde_json::from_str(&row.map_err(|error| error.to_string())?)
                .map_err(|error| error.to_string())?;
            Ok(Session {
                id: value["session_id"].as_str().unwrap_or_default().into(),
                server: value["server"].as_str().unwrap_or_default().into(),
                directory: value["directory"].as_str().unwrap_or_default().into(),
                title: value["title"].as_str().unwrap_or_default().into(),
                stale: true,
                ..Default::default()
            })
        })
        .collect()
    }

    pub(crate) fn publish(&self, mut snapshot: StateSnapshot) -> Result<(), String> {
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        self.retire_renames_in(&transaction, &snapshot.sessions["sessions"])?;
        let edits = self.edits_in(&transaction)?;
        let previous = self.snapshot(&transaction)?;
        let origins = self.origins_in(&transaction)?;
        let mut statement = transaction.prepare_cached("SELECT directory FROM session_creation_requests WHERE namespace = ?1 AND session_id IS NULL").map_err(|error| error.to_string())?;
        let gates = statement
            .query_map([&self.config.namespace], |row| row.get::<_, String>(0))
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        drop(statement);
        if let Some(sessions) = snapshot.sessions["sessions"].as_array_mut() {
            sessions.retain_mut(|session| {
                let key = (
                    session["server"].as_str().unwrap_or_default().to_owned(),
                    session["session_id"]
                        .as_str()
                        .unwrap_or_default()
                        .to_owned(),
                );
                match edits.get(&key) {
                    Some(Some(title)) => session["title"] = json!(title),
                    Some(None) => return false,
                    None => {}
                }
                true
            });
            sessions.retain(|session| {
                !gates
                    .iter()
                    .any(|directory| session["directory"].as_str() == Some(directory))
                    || previous.sessions["sessions"]
                        .as_array()
                        .is_some_and(|known| {
                            known.iter().any(|value| {
                                state_stream::session_key(value)
                                    == state_stream::session_key(session)
                            })
                        })
            });
            for session in sessions.iter_mut() {
                let key = state_stream::session_key(session);
                session["origins"] = json!(origins.get(&key).cloned().unwrap_or_default());
                transaction.execute("UPDATE session_origins SET session = ?4 WHERE namespace = ?1 AND server = ?2 AND session_id = ?3 AND session <> ?4", params![self.config.namespace, session["server"].as_str(), session["session_id"].as_str(), session.to_string()]).map_err(|error| error.to_string())?;
                transaction.execute("UPDATE session_creation_requests SET observed = 1 WHERE namespace = ?1 AND server = ?2 AND session_id = ?3", params![self.config.namespace, session["server"].as_str(), session["session_id"].as_str()]).map_err(|error| error.to_string())?;
            }
            // A creation receipt can arrive before the observer sees its conversation.
            for known in previous.sessions["sessions"]
                .as_array()
                .into_iter()
                .flatten()
            {
                let provisional: bool = transaction.query_row("SELECT EXISTS(SELECT 1 FROM session_creation_requests WHERE namespace = ?1 AND server = ?2 AND session_id = ?3 AND observed = 0)", params![self.config.namespace, known["server"].as_str(), known["session_id"].as_str()], |row| row.get(0)).map_err(|error| error.to_string())?;
                let deleted = matches!(
                    edits.get(&(
                        known["server"].as_str().unwrap_or_default().to_owned(),
                        known["session_id"].as_str().unwrap_or_default().to_owned()
                    )),
                    Some(None)
                );
                if !deleted
                    && (provisional
                        || gates
                            .iter()
                            .any(|directory| known["directory"].as_str() == Some(directory)))
                    && !sessions.iter().any(|value| {
                        state_stream::session_key(value) == state_stream::session_key(known)
                    })
                {
                    let mut retained = known.clone();
                    retained["stale"] = json!(true);
                    retained["activity"] = json!("unknown");
                    retained["availability"] = json!("unverified");
                    retained["attached"] = json!(false);
                    retained["panes"] = json!([]);
                    sessions.push(retained);
                }
            }
        }
        transaction.execute("DELETE FROM session_creation_requests WHERE namespace = ?1 AND observed = 1 AND (completed = 1 OR created_at < ?2)", params![self.config.namespace, state_stream::now().saturating_sub(7 * 86400)]).map_err(|error| error.to_string())?;
        self.commit_snapshot(&transaction, &snapshot)?;
        transaction.commit().map_err(|error| error.to_string())
    }

    pub(super) fn snapshot(
        &self,
        connection: &rusqlite::Connection,
    ) -> Result<StateSnapshot, String> {
        let text: String = connection
            .query_row(
                "SELECT snapshot FROM state_stream_heads WHERE namespace = ?1",
                [&self.config.namespace],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        serde_json::from_str(&text).map_err(|error| error.to_string())
    }

    pub(super) fn commit_snapshot(
        &self,
        transaction: &Transaction<'_>,
        snapshot: &StateSnapshot,
    ) -> Result<(), String> {
        let previous = self.snapshot(transaction)?;
        let changes = snapshot.changes(&previous);
        if changes.is_empty() {
            return Ok(());
        }
        let encoded = serde_json::to_string(snapshot).map_err(|error| error.to_string())?;
        if encoded.len() > state_stream::MAX_FRAME_BYTES - 1024 {
            return Err("state snapshot exceeds 16 MiB".into());
        }
        let (epoch, mut sequence): (String, i64) = transaction
            .query_row(
                "SELECT epoch, sequence FROM state_stream_heads WHERE namespace = ?1",
                [&self.config.namespace],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(|error| error.to_string())?;
        let mut bytes = 0usize;
        for (kind, data) in changes {
            sequence += 1;
            let event =
                state_stream::envelope(kind, &format!("{epoch}:{sequence}"), data).to_string();
            if event.len() > state_stream::MAX_FRAME_BYTES {
                return Err("state event exceeds 16 MiB".into());
            }
            bytes += event.len();
            transaction.execute("INSERT INTO state_stream_events(namespace, sequence, payload) VALUES (?1, ?2, ?3)", params![self.config.namespace, sequence, event]).map_err(|error| error.to_string())?;
        }
        transaction.execute("UPDATE state_stream_heads SET sequence = ?2, snapshot = ?3, bytes = bytes + ?4 WHERE namespace = ?1", params![self.config.namespace, sequence, encoded, bytes]).map_err(|error| error.to_string())?;
        self.prune(transaction, sequence)
    }

    fn prune(&self, transaction: &Transaction<'_>, sequence: i64) -> Result<(), String> {
        let mut floor = sequence
            .saturating_sub(state_stream::MAX_REPLAY_EVENTS as i64)
            .max(0);
        let mut bytes: usize = transaction
            .query_row(
                "SELECT bytes FROM state_stream_heads WHERE namespace = ?1",
                [&self.config.namespace],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        let expired: usize = transaction.query_row("SELECT coalesce(sum(length(CAST(payload AS BLOB))), 0) FROM state_stream_events WHERE namespace = ?1 AND sequence <= ?2", params![self.config.namespace, floor], |row| row.get(0)).map_err(|error| error.to_string())?;
        bytes = bytes.saturating_sub(expired);
        if bytes > state_stream::MAX_REPLAY_BYTES {
            let mut statement = transaction.prepare_cached("SELECT sequence, length(CAST(payload AS BLOB)) FROM state_stream_events WHERE namespace = ?1 AND sequence > ?2 ORDER BY sequence").map_err(|error| error.to_string())?;
            let rows = statement
                .query_map(params![self.config.namespace, floor], |row| {
                    Ok((row.get::<_, i64>(0)?, row.get::<_, usize>(1)?))
                })
                .map_err(|error| error.to_string())?;
            for row in rows {
                let (id, size) = row.map_err(|error| error.to_string())?;
                bytes = bytes.saturating_sub(size);
                floor = id;
                if bytes <= state_stream::MAX_REPLAY_BYTES {
                    break;
                }
            }
        }
        transaction
            .execute(
                "DELETE FROM state_stream_events WHERE namespace = ?1 AND sequence <= ?2",
                params![self.config.namespace, floor],
            )
            .map_err(|error| error.to_string())?;
        transaction.execute("UPDATE state_stream_heads SET floor = max(floor, ?2), bytes = ?3 WHERE namespace = ?1", params![self.config.namespace, floor, bytes]).map_err(|error| error.to_string())?;
        Ok(())
    }

    pub(crate) fn boundary(&self, cursor: Option<&str>) -> Result<Boundary, String> {
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction()
            .map_err(|error| error.to_string())?;
        let (epoch, sequence, floor): (String, i64, i64) = transaction
            .query_row(
                "SELECT epoch, sequence, floor FROM state_stream_heads WHERE namespace = ?1",
                [&self.config.namespace],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .map_err(|error| error.to_string())?;
        let after = cursor
            .and_then(|cursor| cursor.rsplit_once(':'))
            .filter(|(value, _)| *value == epoch)
            .and_then(|(_, number)| number.parse::<i64>().ok())
            .filter(|number| *number >= floor && *number <= sequence);
        let mut events = Vec::new();
        let mut reset = after.is_none();
        let mut bytes = 0usize;
        if let Some(after) = after {
            let mut statement = transaction.prepare_cached("SELECT payload FROM state_stream_events WHERE namespace = ?1 AND sequence > ?2 ORDER BY sequence").map_err(|error| error.to_string())?;
            let rows = statement
                .query_map(params![self.config.namespace, after], |row| {
                    row.get::<_, String>(0)
                })
                .map_err(|error| error.to_string())?;
            for row in rows {
                let text = row.map_err(|error| error.to_string())?;
                bytes += text.len() + 1;
                if bytes > 2 * 1024 * 1024 {
                    events.clear();
                    reset = true;
                    break;
                }
                events.push(serde_json::from_str(&text).map_err(|error| error.to_string())?);
            }
        }
        let snapshot = if reset {
            self.snapshot(&transaction)?
        } else {
            StateSnapshot::default()
        };
        transaction.commit().map_err(|error| error.to_string())?;
        Ok(Boundary {
            cursor: format!("{epoch}:{sequence}"),
            snapshot,
            events,
            reset,
        })
    }

    pub(crate) fn inventory_revision(&self) -> Result<i64, String> {
        self.runtime_pool
            .get(|_| Ok(()))?
            .query_row(
                "SELECT revision FROM refresh_revisions WHERE scope = ?1",
                [format!("instances:{}", self.config.namespace)],
                |row| row.get(0),
            )
            .optional()
            .map(|value| value.unwrap_or(0))
            .map_err(|error| error.to_string())
    }

    pub(crate) fn observation_lease(
        &self,
        kind: &str,
    ) -> Result<Option<super::gateway::Lock>, String> {
        super::gateway::try_lock(
            &self.config,
            &format!("observation-{}-{kind}", self.config.namespace),
        )
    }

    pub(crate) fn save_observation(
        &self,
        kind: &str,
        snapshot: &impl serde::Serialize,
    ) -> Result<(), String> {
        let payload = serde_json::to_string(snapshot).map_err(|error| error.to_string())?;
        if payload.len() > state_stream::MAX_FRAME_BYTES {
            return Err("shared observation exceeds 16 MiB".into());
        }
        self.connection()?.execute("INSERT INTO state_observations(namespace, kind, payload) VALUES (?1, ?2, ?3) ON CONFLICT(namespace, kind) DO UPDATE SET payload = excluded.payload, revision = revision + 1 WHERE payload <> excluded.payload", params![self.config.namespace, kind, payload]).map_err(|error| error.to_string())?;
        Ok(())
    }

    pub(crate) fn observation<T: serde::de::DeserializeOwned>(
        &self,
        kind: &str,
        after: i64,
    ) -> Result<Option<(i64, T)>, String> {
        let row: Option<(i64, String)> = self.connection()?.query_row("SELECT revision, payload FROM state_observations WHERE namespace = ?1 AND kind = ?2 AND revision > ?3", params![self.config.namespace, kind, after], |row| Ok((row.get(0)?, row.get(1)?))).optional().map_err(|error| error.to_string())?;
        row.map(|(revision, payload)| {
            serde_json::from_str(&payload)
                .map(|value| (revision, value))
                .map_err(|error| error.to_string())
        })
        .transpose()
    }
}
