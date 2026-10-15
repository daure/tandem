use rusqlite::{OptionalExtension, params};

use super::RuleStore;
use crate::store::events::Error;

#[cfg(test)]
#[path = "../tests/rule_sessions.rs"]
mod tests;

pub(crate) struct NamedSession {
    pub session_id: String,
    pub server: String,
    pub ready: bool,
}

impl RuleStore {
    pub(crate) fn named_session(
        &self,
        instance: &str,
        name: &str,
        identity: &str,
    ) -> Result<Option<NamedSession>, Error> {
        Ok(self
            .events
            .connection()?
            .query_row(
                "SELECT session_id, server, ready FROM rule_named_sessions
             WHERE namespace=?1 AND instance=lower(?2) AND name=?3 AND workspace_identity=?4",
                params![self.config.namespace, instance, name, identity],
                |row| {
                    Ok(NamedSession {
                        session_id: row.get(0)?,
                        server: row.get(1)?,
                        ready: row.get(2)?,
                    })
                },
            )
            .optional()?)
    }

    // The caller holds the instance lock through reservation, creation and confirmation.
    pub(crate) fn reserve_named_session(
        &self,
        instance: &str,
        name: &str,
        identity: &str,
        server: &str,
    ) -> Result<NamedSession, Error> {
        let session_id = self.events.connection()?.query_row(
            "INSERT INTO rule_named_sessions(namespace, instance, name, workspace_identity, session_id, server)
             VALUES (?1, lower(?2), ?3, ?4, 'ses_' || lower(hex(randomblob(16))), ?5)
             ON CONFLICT(namespace, instance, name) DO UPDATE SET
             workspace_identity=excluded.workspace_identity, session_id=excluded.session_id,
             server=excluded.server, ready=0 RETURNING session_id",
            params![self.config.namespace, instance, name, identity, server],
            |row| row.get(0),
        )?;
        Ok(NamedSession {
            session_id,
            server: server.into(),
            ready: false,
        })
    }

    pub(crate) fn finish_named_session(
        &self,
        session: &NamedSession,
        created: bool,
    ) -> Result<(), Error> {
        let sql = if created {
            "UPDATE rule_named_sessions SET ready=1 WHERE namespace=?1 AND session_id=?2 AND server=?3"
        } else {
            "DELETE FROM rule_named_sessions WHERE namespace=?1 AND session_id=?2 AND server=?3"
        };
        self.events.connection()?.execute(
            sql,
            params![self.config.namespace, session.session_id, session.server],
        )?;
        Ok(())
    }
}
