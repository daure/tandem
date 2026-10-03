use rusqlite::{TransactionBehavior, params};

use super::EventStore;
use crate::store::{
    events::Error,
    rules::{Acceptance, DispatchStatus},
};

impl EventStore {
    pub(super) fn ensure_provider_not_deleting(
        &self,
        connection: &rusqlite::Connection,
        source: &str,
    ) -> Result<(), Error> {
        let deleting: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM provider_deletions WHERE namespace = ?1 AND source = ?2)",
            params![self.namespace, source],
            |row| row.get(0),
        )?;
        if deleting {
            return Err(Error::Conflict(
                "provider deletion is incomplete; retry delete_provider".into(),
            ));
        }
        Ok(())
    }

    pub(crate) fn begin_provider_deletion(&self, package: &str, source: &str) -> Result<(), Error> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute("INSERT INTO provider_deletions(namespace,name,source) VALUES (?1,?2,?3) ON CONFLICT(namespace,name) DO NOTHING", params![self.namespace, package, source])?;
        transaction.execute("INSERT INTO provider_ingestion(namespace,name,enabled) VALUES (?1,?2,0) ON CONFLICT(namespace,name) DO UPDATE SET enabled=0", params![self.namespace, source])?;
        transaction.commit()?;
        Ok(())
    }

    pub(crate) fn provider_deletion_acceptances(
        &self,
        source: &str,
    ) -> Result<Vec<Acceptance>, Error> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT a.payload FROM rule_acceptances a
             JOIN event_attempts p ON p.id = a.attempt_id JOIN events e ON e.sequence = p.event_sequence
             WHERE e.namespace = ?1 AND e.provider = ?2 ORDER BY a.id",
        )?;
        let rows = statement
            .query_map(params![self.namespace, source], |row| {
                row.get::<_, String>(0)
            })?
            .map(|text| {
                serde_json::from_str::<Acceptance>(&text?)
                    .map_err(|error| Error::Storage(error.to_string()))
            })
            .collect::<Result<Vec<_>, _>>()?;
        if rows.iter().any(|row| {
            matches!(
                row.status,
                DispatchStatus::Provisioning | DispatchStatus::Launching
            )
        }) {
            return Err(Error::Conflict(
                "provider has active rule actions; retry when they finish".into(),
            ));
        }
        Ok(rows)
    }

    pub(crate) fn delete_provider(&self, package: &str, source: &str) -> Result<i64, Error> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute(
            "DELETE FROM provider_notifications WHERE namespace = ?1 AND provider = ?2",
            params![self.namespace, source],
        )?;
        for table in ["rule_evaluations", "rule_acceptances"] {
            transaction.execute(&format!(
                "DELETE FROM {table} WHERE attempt_id IN
                 (SELECT p.id FROM event_attempts p JOIN events e ON e.sequence = p.event_sequence WHERE e.namespace = ?1 AND e.provider = ?2)"
            ), params![self.namespace, source])?;
        }
        transaction.execute("DELETE FROM event_attempts WHERE event_sequence IN (SELECT sequence FROM events WHERE namespace = ?1 AND provider = ?2)", params![self.namespace, source])?;
        let count = transaction.execute(
            "DELETE FROM events WHERE namespace = ?1 AND provider = ?2",
            params![self.namespace, source],
        )?;
        for (table, key) in [
            ("provider_streams", "provider"),
            ("provider_ingestion", "name"),
            ("event_providers", "name"),
        ] {
            transaction.execute(
                &format!("DELETE FROM {table} WHERE namespace = ?1 AND {key} = ?2"),
                params![self.namespace, source],
            )?;
        }
        transaction.execute(
            "DELETE FROM provider_launches WHERE namespace = ?1 AND name = ?2",
            params![self.namespace, package],
        )?;
        transaction.execute(
            "DELETE FROM provider_deletions WHERE namespace = ?1 AND name = ?2",
            params![self.namespace, package],
        )?;
        transaction.commit()?;
        Ok(count as i64)
    }
}
