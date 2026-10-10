use chrono::{DateTime, Utc};
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};

use super::{RuleStore, decode, encode, names};
use crate::store::{
    events::{Error, Event},
    rules::{self, Acceptance, DispatchStatus, Rule},
};

impl RuleStore {
    pub(crate) fn evaluate(&self) -> Result<(), Error> {
        self.evaluate_at(Utc::now())
    }

    pub(super) fn evaluate_at(&self, now: DateTime<Utc>) -> Result<(), Error> {
        let connection = self.events.connection()?;
        // Future deferred work must not occupy the bounded pending batch.
        for deferred in [true, false] {
            let mut statement = connection.prepare(
                "SELECT r.attempt_id, r.rule_name FROM rule_evaluations r
                 JOIN event_attempts p ON p.id = r.attempt_id JOIN events e ON e.sequence = p.event_sequence
                 WHERE e.namespace = ?1 AND
                    ((?3 = 1 AND r.outcome = 'deferred' AND r.deadline_ms <= ?2)
                     OR (?3 = 0 AND r.outcome IS NULL))
                 ORDER BY r.attempt_id, r.rule_name LIMIT 32",
            )?;
            let pending = statement
                .query_map(
                    params![self.config.namespace, now.timestamp_millis(), deferred],
                    |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
                )?
                .collect::<Result<Vec<_>, _>>()?;
            for (attempt, name) in pending {
                self.evaluate_one(attempt, &name, now)?;
            }
        }
        Ok(())
    }

    fn evaluate_one(&self, attempt: i64, name: &str, now: DateTime<Utc>) -> Result<(), Error> {
        let mut connection = self.events.connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let pending = transaction.query_row(
            "SELECT r.rule_snapshot, e.sequence, e.provider, e.received_at, e.payload, p.replay, r.outcome
             FROM rule_evaluations r JOIN event_attempts p ON p.id = r.attempt_id
             JOIN events e ON e.sequence = p.event_sequence
             WHERE e.namespace = ?1 AND r.attempt_id = ?2 AND r.rule_name = ?3
               AND (r.outcome IS NULL OR (r.outcome = 'deferred' AND r.deadline_ms <= ?4))",
            params![self.config.namespace, attempt, name, now.timestamp_millis()],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?, row.get::<_, String>(2)?,
                      row.get::<_, String>(3)?, row.get::<_, String>(4)?, row.get::<_, bool>(5)?,
                      row.get::<_, Option<String>>(6)?)),
        ).optional()?;
        let Some((snapshot, sequence, provider, received_at, payload, replay, previous_outcome)) =
            pending
        else {
            return Ok(());
        };
        let rule: Rule = decode(&snapshot)?;
        let event: Event = decode(&payload)?;
        let input = rules::event_input(&event, &provider, sequence, &received_at);
        let deferred = previous_outcome.as_deref() == Some("deferred");
        let matched = if deferred {
            Ok(true)
        } else {
            rules::matches(&rule.definition, &input)
        };
        let (outcome, error) = match matched {
            Ok(true) => {
                let deadline = if !deferred && !replay && rule.definition.throttle_seconds > 0 {
                    let until: Option<i64> = transaction.query_row(
                        "SELECT until_ms FROM rule_throttles WHERE namespace = ?1 AND rule_name = ?2",
                        params![self.config.namespace, name], |row| row.get(0),
                    ).optional()?;
                    if let Some(until) = until.filter(|until| *until > now.timestamp_millis()) {
                        let warning =
                            format!("Rule acceptance throttled until {until} ms since Unix epoch.");
                        transaction.execute(
                            "UPDATE rule_evaluations SET outcome = 'throttled', warning = ?3 WHERE attempt_id = ?1 AND rule_name = ?2",
                            params![attempt, name, warning],
                        )?;
                        transaction.commit()?;
                        return Ok(());
                    }
                    Some(
                        now.timestamp_millis()
                            .checked_add(i64::from(rule.definition.throttle_seconds) * 1000)
                            .ok_or_else(|| {
                                Error::Storage("rule throttle deadline exhausted".into())
                            })?,
                    )
                } else {
                    None
                };
                if let Some(deadline) = deadline.filter(|_| rule.definition.trigger_at_end) {
                    self.save_throttle(&transaction, name, deadline)?;
                    transaction.execute(
                        "UPDATE rule_evaluations SET outcome = 'deferred', deadline_ms = ?3 WHERE attempt_id = ?1 AND rule_name = ?2",
                        params![attempt, name, deadline],
                    )?;
                    transaction.commit()?;
                    return Ok(());
                }
                transaction.execute("INSERT INTO rule_acceptances(attempt_id, rule_name, payload) VALUES (?1, ?2, '')", params![attempt, name])?;
                let id = transaction.last_insert_rowid();
                let overrides = rules::resolve_instance_hooks(&rule.definition, &input)
                    .map_err(Error::Invalid)?;
                let base = overrides
                    .custom_name
                    .unwrap_or_else(|| rules::instance_name(name, sequence, id));
                let Some(instance) = names::reserve_name(&transaction, &self.config, &base, id)?
                else {
                    transaction.execute("DELETE FROM rule_acceptances WHERE id = ?1", [id])?;
                    transaction.execute("UPDATE rule_evaluations SET outcome = 'failed', error = 'instance name candidates exhausted (1024)' WHERE attempt_id = ?1 AND rule_name = ?2", params![attempt, name])?;
                    transaction.commit()?;
                    return Ok(());
                };
                let description = overrides
                    .custom_description
                    .unwrap_or_else(|| rules::default_instance_description(name, &event.summary));
                let prompt = rules::render_prompt(&rule.definition.initial_prompt, &input);
                let timestamp = now.to_rfc3339();
                let acceptance = Acceptance {
                    id,
                    event_sequence: sequence,
                    event_summary: event.summary,
                    attempt_id: attempt,
                    rule_name: name.into(),
                    rule_revision: rule.revision,
                    accepted_at: timestamp.clone(),
                    instance,
                    session_id: None,
                    pane: None,
                    operation_id: None,
                    launch_started_at: None,
                    status: if prompt.is_ok() {
                        DispatchStatus::Queued
                    } else {
                        DispatchStatus::Failed
                    },
                    error: prompt.as_ref().err().cloned(),
                    rule,
                    resolved_prompt: prompt.ok(),
                    resolved_description: Some(description),
                };
                transaction.execute(
                    "UPDATE rule_acceptances SET payload = ?2 WHERE id = ?1",
                    params![id, encode(&acceptance)?],
                )?;
                transaction.execute("UPDATE event_attempts SET status = 'accepted', accepted_at = COALESCE(accepted_at, ?2) WHERE id = ?1", params![attempt, timestamp])?;
                if let Some(deadline) = deadline {
                    self.save_throttle(&transaction, name, deadline)?;
                }
                ("matched", None)
            }
            Ok(false) => ("no_match", None),
            Err(error) => ("failed", Some(error)),
        };
        transaction.execute("UPDATE rule_evaluations SET outcome = ?3, error = ?4 WHERE attempt_id = ?1 AND rule_name = ?2", params![attempt, name, outcome, error])?;
        transaction.commit()?;
        Ok(())
    }

    fn save_throttle(
        &self,
        transaction: &Transaction<'_>,
        name: &str,
        deadline: i64,
    ) -> Result<(), Error> {
        transaction.execute(
            "INSERT INTO rule_throttles(namespace, rule_name, until_ms) VALUES (?1, ?2, ?3)
             ON CONFLICT(namespace, rule_name) DO UPDATE SET until_ms = excluded.until_ms",
            params![self.config.namespace, name, deadline],
        )?;
        Ok(())
    }
}
