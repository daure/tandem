use std::collections::BTreeMap;

use rusqlite::{OptionalExtension, TransactionBehavior, params};
use serde_json::json;

use super::Store;

impl Store {
    pub(crate) fn session_edits(
        &self,
    ) -> Result<BTreeMap<(String, String), Option<String>>, String> {
        let connection = self.connection()?;
        self.edits_in(&connection)
    }

    pub(super) fn edits_in(
        &self,
        connection: &rusqlite::Connection,
    ) -> Result<BTreeMap<(String, String), Option<String>>, String> {
        let mut statement = connection
            .prepare_cached(
                "SELECT server, session_id, title FROM state_session_edits WHERE namespace = ?1",
            )
            .map_err(|error| error.to_string())?;
        statement
            .query_map([&self.config.namespace], |row| {
                Ok(((row.get(0)?, row.get(1)?), row.get(2)?))
            })
            .map_err(|error| error.to_string())?
            .collect::<Result<_, _>>()
            .map_err(|error| error.to_string())
    }

    pub(crate) fn retire_renames(&self, sessions: &serde_json::Value) -> Result<(), String> {
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        self.retire_renames_in(&transaction, sessions)?;
        transaction.commit().map_err(|error| error.to_string())
    }

    pub(super) fn retire_renames_in(
        &self,
        connection: &rusqlite::Connection,
        sessions: &serde_json::Value,
    ) -> Result<(), String> {
        for session in sessions
            .as_array()
            .into_iter()
            .flatten()
            .filter(|session| session["stale"] == false)
        {
            connection.execute("DELETE FROM state_session_edits WHERE namespace = ?1 AND server = ?2 AND session_id = ?3 AND title = ?4 AND title IS NOT NULL", params![self.config.namespace, session["server"].as_str(), session["session_id"].as_str(), session["title"].as_str()]).map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    pub(crate) fn apply_session_edits(
        &self,
        edits: &BTreeMap<(String, String), Option<String>>,
    ) -> Result<(), String> {
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        let mut snapshot = self.snapshot(&transaction)?;
        for ((server, id), title) in edits {
            transaction.execute("INSERT INTO state_session_edits(namespace, server, session_id, title) VALUES (?1, ?2, ?3, ?4) ON CONFLICT(namespace, server, session_id) DO UPDATE SET title = excluded.title WHERE state_session_edits.title IS NOT NULL", params![self.config.namespace, server, id, title]).map_err(|error| error.to_string())?;
            if title.is_none() {
                transaction.execute("DELETE FROM session_origins WHERE namespace = ?1 AND server = ?2 AND session_id = ?3", params![self.config.namespace, server, id]).map_err(|error| error.to_string())?;
                transaction.execute("DELETE FROM session_creation_requests WHERE namespace = ?1 AND server = ?2 AND session_id = ?3", params![self.config.namespace, server, id]).map_err(|error| error.to_string())?;
            }
        }
        let effective = self.edits_in(&transaction)?;
        let observation: Option<String> = transaction
            .query_row(
                "SELECT payload FROM state_observations WHERE namespace = ?1 AND kind = 'opencode'",
                [&self.config.namespace],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        if let Some(payload) = observation {
            let mut observed: crate::store::opencode::Snapshot =
                serde_json::from_str(&payload).map_err(|error| error.to_string())?;
            observed.apply_session_edits(&effective);
            self.save_observation_in(&transaction, "opencode", &observed)?;
        }
        if let Some(sessions) = snapshot.sessions["sessions"].as_array_mut() {
            sessions.retain_mut(|session| {
                let key = (
                    session["server"].as_str().unwrap_or_default().to_owned(),
                    session["session_id"]
                        .as_str()
                        .unwrap_or_default()
                        .to_owned(),
                );
                match effective.get(&key) {
                    Some(Some(title)) => session["title"] = json!(title),
                    Some(None) => return false,
                    None => {}
                }
                true
            });
        }
        self.commit_snapshot(&transaction, &snapshot)?;
        transaction.commit().map_err(|error| error.to_string())
    }
}
