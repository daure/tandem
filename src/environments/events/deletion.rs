use rusqlite::{Transaction, TransactionBehavior, params};

use super::EventStore;
use crate::environments::{conclusion, gateway};
use crate::store::events::{Deletion, Error};

impl EventStore {
    pub(crate) fn event_lease(&self, sequence: i64) -> Result<Option<gateway::Lock>, Error> {
        gateway::try_lock(
            &self.config,
            &format!("event-{}-{sequence}", self.namespace),
        )
        .map_err(Error::Storage)
    }

    pub(crate) fn delete(&self, target: Deletion) -> Result<i64, Error> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute_batch(
            "CREATE TEMP TABLE event_deletion_targets(sequence INTEGER PRIMARY KEY);",
        )?;
        match target {
            Deletion::Acceptance(id) => {
                transaction.execute(
                    "INSERT INTO event_deletion_targets SELECT e.sequence FROM events e
                     JOIN event_attempts p ON p.event_sequence = e.sequence
                     JOIN rule_acceptances a ON a.attempt_id = p.id
                     WHERE e.namespace = ?1 AND a.id = ?2",
                    params![self.namespace, id],
                )?;
            }
            _ => {
                let sequence = match target {
                    Deletion::Event(sequence) => Some(sequence),
                    _ => None,
                };
                transaction.execute(
                    "INSERT INTO event_deletion_targets SELECT e.sequence FROM events e
                     WHERE e.namespace = ?1 AND (?2 IS NULL OR e.sequence = ?2)
                     AND (?3 = 0 OR NOT EXISTS (
                         SELECT 1 FROM event_attempts p JOIN rule_acceptances a ON a.attempt_id = p.id
                         WHERE p.event_sequence = e.sequence))",
                    params![self.namespace, sequence, target == Deletion::Ignored],
                )?;
            }
        }
        let count: i64 =
            transaction.query_row("SELECT count(*) FROM event_deletion_targets", [], |row| {
                row.get(0)
            })?;
        if count == 0 && matches!(target, Deletion::Event(_) | Deletion::Acceptance(_)) {
            return Err(Error::NotFound);
        }
        let acceptance = match target {
            Deletion::Acceptance(id) => Some(id),
            _ => None,
        };
        let _leases = if target == Deletion::Ignored {
            Vec::new()
        } else {
            self.deletion_leases(&transaction, acceptance)?
        };
        ensure_deletion_allowed(&transaction, acceptance)?;
        if let Some(id) = acceptance {
            transaction.execute(
                "DELETE FROM provider_notifications WHERE namespace = ?1
                 AND json_extract(payload, '$.dispatch_id') = ?2",
                params![self.namespace, id],
            )?;
            transaction.execute("DELETE FROM rule_acceptances WHERE id = ?1", [id])?;
        } else {
            transaction.execute(
                "DELETE FROM provider_notifications WHERE namespace = ?1
                 AND json_extract(payload, '$.sequence') IN (SELECT sequence FROM event_deletion_targets)",
                [&self.namespace],
            )?;
            for table in ["rule_evaluations", "rule_acceptances"] {
                transaction.execute(
                    &format!(
                        "DELETE FROM {table} WHERE attempt_id IN
                         (SELECT id FROM event_attempts WHERE event_sequence IN
                          (SELECT sequence FROM event_deletion_targets))"
                    ),
                    [],
                )?;
            }
            transaction.execute(
                "DELETE FROM event_attempts WHERE event_sequence IN (SELECT sequence FROM event_deletion_targets)",
                [],
            )?;
            transaction.execute(
                "DELETE FROM events WHERE sequence IN (SELECT sequence FROM event_deletion_targets)",
                [],
            )?;
        }
        transaction.commit()?;
        Ok(count)
    }

    fn deletion_leases(
        &self,
        transaction: &Transaction<'_>,
        acceptance: Option<i64>,
    ) -> Result<Vec<gateway::Lock>, Error> {
        let mut leases = Vec::new();
        let mut events =
            transaction.prepare("SELECT sequence FROM event_deletion_targets ORDER BY sequence")?;
        for sequence in events.query_map([], |row| row.get::<_, i64>(0))? {
            let sequence = sequence?;
            leases.push(self.event_lease(sequence)?.ok_or_else(|| {
                Error::Conflict(format!(
                    "event #{sequence} is busy; retry when its operation finishes"
                ))
            })?);
        }
        let mut acceptances = transaction.prepare(
            "SELECT a.id FROM rule_acceptances a JOIN event_attempts p ON p.id = a.attempt_id
             JOIN rule_acceptance_reports r ON r.acceptance_id = a.id
             WHERE p.event_sequence IN (SELECT sequence FROM event_deletion_targets)
             AND (?1 IS NULL OR a.id = ?1) AND r.cleanup_state IN ('pending', 'purging') ORDER BY a.id",
        )?;
        for id in acceptances.query_map([acceptance], |row| row.get::<_, i64>(0))? {
            let id = id?;
            leases.push(
                gateway::try_lock(&self.config, &conclusion::lease(id))
                    .map_err(Error::Storage)?
                    .ok_or_else(|| {
                        Error::Conflict(format!(
                            "acceptance #{id} conclusion is busy; retry when its cleanup finishes"
                        ))
                    })?,
            );
        }
        Ok(leases)
    }
}

fn ensure_deletion_allowed(
    transaction: &Transaction<'_>,
    acceptance: Option<i64>,
) -> Result<(), Error> {
    let deleting: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM events e JOIN provider_deletions d
         ON d.namespace = e.namespace AND d.source = e.provider
         WHERE e.sequence IN (SELECT sequence FROM event_deletion_targets))",
        [],
        |row| row.get(0),
    )?;
    if deleting {
        return Err(Error::Conflict(
            "events belong to an incomplete provider deletion; retry delete_provider".into(),
        ));
    }
    let active: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM rule_acceptances a JOIN event_attempts p ON p.id = a.attempt_id
         WHERE p.event_sequence IN (SELECT sequence FROM event_deletion_targets)
         AND (?1 IS NULL OR a.id = ?1)
         AND json_extract(a.payload, '$.status') IN ('provisioning', 'launching'))",
        [acceptance],
        |row| row.get(0),
    )?;
    if active {
        return Err(Error::Conflict(
            "events have active rule actions; try again when they finish".into(),
        ));
    }
    Ok(())
}
