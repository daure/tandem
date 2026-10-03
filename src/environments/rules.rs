use std::fs::{File, OpenOptions, TryLockError};

use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};

use super::{config::Config, events::EventStore};
use crate::store::{
    events::{Error, Event, NotificationKind, ProviderNotification},
    rules::{self, Acceptance, Definition, DispatchStatus, Evaluation, Rule, Snapshot},
};

pub(super) mod catalog;

#[derive(Clone)]
pub(crate) struct RuleStore {
    events: EventStore,
    config: Config,
}

pub(super) fn queue_attempt(
    transaction: &Transaction<'_>,
    attempt: i64,
    sequence: i64,
) -> Result<(), Error> {
    let namespace: String = transaction.query_row(
        "SELECT namespace FROM events WHERE sequence = ?1",
        [sequence],
        |row| row.get(0),
    )?;
    let mut statement = transaction.prepare("SELECT name, revision, definition, zellij_session FROM event_rules WHERE namespace = ?1 AND enabled = 1 AND catalog_present = 1 ORDER BY name")?;
    let rules = statement
        .query_map([namespace], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    for (name, revision, definition, zellij_session) in rules {
        let rule = Rule {
            definition: decode(&definition)?,
            revision,
            zellij_session,
        };
        transaction.execute("INSERT INTO rule_evaluations(attempt_id, rule_name, rule_revision, rule_snapshot) VALUES (?1, ?2, ?3, ?4)", params![attempt, name, revision, encode(&rule)?])?;
    }
    Ok(())
}

pub(super) fn acceptances(
    connection: &Connection,
    sequence: Option<i64>,
    namespace: &str,
) -> Result<Vec<Acceptance>, Error> {
    let mut statement = connection.prepare(
        "SELECT a.payload FROM rule_acceptances a
         JOIN event_attempts p ON p.id = a.attempt_id JOIN events e ON e.sequence = p.event_sequence
         WHERE e.namespace = ?1 AND (?2 IS NULL OR e.sequence = ?2) ORDER BY a.id DESC",
    )?;
    statement
        .query_map(params![namespace, sequence], |row| row.get::<_, String>(0))?
        .map(|text| decode(&text?))
        .collect()
}

fn encode(value: &impl serde::Serialize) -> Result<String, Error> {
    serde_json::to_string(value).map_err(|error| Error::Storage(error.to_string()))
}

fn decode<T: serde::de::DeserializeOwned>(text: &str) -> Result<T, Error> {
    serde_json::from_str(text).map_err(|error| Error::Storage(error.to_string()))
}

fn assign_instance(
    transaction: &Transaction<'_>,
    rule: &str,
    sequence: i64,
) -> Result<String, Error> {
    let mut statement = transaction.prepare(
        "SELECT EXISTS(SELECT 1 FROM rule_acceptances WHERE json_extract(NULLIF(payload, ''), '$.instance') = ?1)",
    )?;
    for ordinal in 1..=u32::MAX {
        let name = rules::instance_name(rule, sequence, ordinal);
        let assigned: bool = statement.query_row([&name], |row| row.get(0))?;
        if !assigned {
            return Ok(name);
        }
    }
    Err(Error::Conflict("rule instance names are exhausted".into()))
}

impl RuleStore {
    pub(crate) fn provider_deleting(&self, sequence: i64) -> Result<bool, Error> {
        Ok(self.events.connection()?.query_row(
            "SELECT EXISTS(SELECT 1 FROM events e JOIN provider_deletions d ON d.namespace=e.namespace AND d.source=e.provider WHERE e.namespace=?1 AND e.sequence=?2)",
            params![self.config.namespace, sequence], |row| row.get(0),
        )?)
    }

    pub(crate) fn open(config: &Config) -> Result<Self, Error> {
        Ok(Self {
            events: EventStore::open(config)?,
            config: config.clone(),
        })
    }

    pub(crate) fn lease(&self) -> Result<Option<File>, Error> {
        let mut options = OpenOptions::new();
        options.create(true).truncate(false).read(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
        }
        let file = options
            .open(
                self.config
                    .home
                    .join("locks")
                    .join(format!("rules-{}", self.config.namespace)),
            )
            .map_err(|error| Error::Storage(error.to_string()))?;
        match file.try_lock() {
            Ok(()) => Ok(Some(file)),
            Err(TryLockError::WouldBlock) => Ok(None),
            Err(error) => Err(Error::Storage(error.to_string())),
        }
    }

    pub(crate) fn snapshot(&self) -> Result<Snapshot, Error> {
        let _lock = catalog::lock(&self.config)?;
        let mut connection = self.events.connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        catalog::synchronize(&self.config, &transaction)?;
        let mut statement = transaction.prepare("SELECT definition, revision, zellij_session FROM event_rules WHERE namespace = ?1 AND catalog_present = 1 ORDER BY name")?;
        let definitions = statement
            .query_map([&self.config.namespace], |row| {
                Ok((row.get::<_, String>(0)?, row.get(1)?, row.get(2)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        drop(statement);
        let rules = definitions
            .into_iter()
            .map(|(text, revision, zellij_session)| {
                Ok(Rule {
                    definition: decode(&text)?,
                    revision,
                    zellij_session,
                })
            })
            .collect::<Result<_, Error>>()?;
        let mut statement = transaction.prepare("SELECT r.attempt_id, r.rule_name, r.rule_revision, r.error FROM rule_evaluations r JOIN event_attempts p ON p.id = r.attempt_id JOIN events e ON e.sequence = p.event_sequence WHERE e.namespace = ?1 AND r.error IS NOT NULL ORDER BY r.attempt_id DESC LIMIT 100")?;
        let evaluation_errors = statement
            .query_map([&self.config.namespace], |row| {
                Ok(Evaluation {
                    attempt_id: row.get(0)?,
                    rule_name: row.get(1)?,
                    rule_revision: row.get(2)?,
                    error: row.get(3)?,
                })
            })?
            .collect::<Result<_, _>>()?;
        let snapshot = Snapshot {
            rules,
            acceptances: acceptances(&transaction, None, &self.config.namespace)?,
            evaluation_errors,
            error: None,
        };
        drop(statement);
        transaction.commit()?;
        Ok(snapshot)
    }

    pub(crate) fn save(
        &self,
        definition: Definition,
        expected_revision: Option<i64>,
        zellij_session: String,
    ) -> Result<Rule, Error> {
        rules::validate(&definition).map_err(Error::Invalid)?;
        let _lock = catalog::lock(&self.config)?;
        let mut connection = self.events.connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        catalog::synchronize(&self.config, &transaction)?;
        let previous: Option<(i64, bool)> = transaction
            .query_row(
                "SELECT revision, catalog_present FROM event_rules WHERE namespace = ?1 AND name = ?2",
                params![self.config.namespace, definition.name],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let visible_revision = previous
            .filter(|(_, present)| *present)
            .map(|(revision, _)| revision);
        if visible_revision != expected_revision {
            return Err(Error::Conflict(
                "rule revision changed; reread it before saving".into(),
            ));
        }
        let count: i64 = transaction.query_row(
            "SELECT count(*) FROM event_rules WHERE namespace = ?1 AND catalog_present = 1",
            [&self.config.namespace],
            |row| row.get(0),
        )?;
        if visible_revision.is_none() && count >= 100 {
            return Err(Error::Invalid("at most 100 rules per catalog".into()));
        }
        let revision = previous.map(|(revision, _)| revision).unwrap_or_default() + 1;
        catalog::save(&self.config, &definition)?;
        transaction.execute("INSERT INTO event_rules(namespace, name, revision, definition, zellij_session, enabled, catalog_present) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1) ON CONFLICT(namespace, name) DO UPDATE SET revision = excluded.revision, definition = excluded.definition, zellij_session = excluded.zellij_session, enabled = excluded.enabled, catalog_present = 1", params![self.config.namespace, definition.name, revision, encode(&definition)?, zellij_session, definition.enabled])?;
        transaction.commit()?;
        Ok(Rule {
            definition,
            revision,
            zellij_session,
        })
    }

    pub(crate) fn evaluate(&self) -> Result<(), Error> {
        let connection = self.events.connection()?;
        let mut statement = connection.prepare("SELECT r.attempt_id, r.rule_name FROM rule_evaluations r JOIN event_attempts p ON p.id = r.attempt_id JOIN events e ON e.sequence = p.event_sequence WHERE e.namespace = ?1 AND r.outcome IS NULL ORDER BY r.attempt_id, r.rule_name LIMIT 32")?;
        let pending = statement
            .query_map([&self.config.namespace], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        for (attempt, name) in pending {
            self.evaluate_one(attempt, &name)?;
        }
        Ok(())
    }

    fn evaluate_one(&self, attempt: i64, name: &str) -> Result<(), Error> {
        let mut connection = self.events.connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let pending: Option<(String, i64, String, String, String)> = transaction.query_row(
            "SELECT r.rule_snapshot, e.sequence, e.provider, e.received_at, e.payload FROM rule_evaluations r JOIN event_attempts p ON p.id = r.attempt_id JOIN events e ON e.sequence = p.event_sequence WHERE e.namespace = ?1 AND r.attempt_id = ?2 AND r.rule_name = ?3 AND r.outcome IS NULL",
            params![self.config.namespace, attempt, name], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?))).optional()?;
        let Some((snapshot, sequence, provider, received_at, payload)) = pending else {
            return Ok(());
        };
        let rule: Rule = decode(&snapshot)?;
        let event: Event = decode(&payload)?;
        let input = rules::event_input(&event, &provider, sequence, &received_at);
        let (outcome, error) = match rules::matches(&rule.definition, &input) {
            Ok(true) => {
                transaction.execute("INSERT INTO rule_acceptances(attempt_id, rule_name, payload) VALUES (?1, ?2, '')", params![attempt, name])?;
                let id = transaction.last_insert_rowid();
                let instance = assign_instance(&transaction, name, sequence)?;
                let prompt = rules::render_prompt(&rule.definition.initial_prompt, &input);
                let timestamp = chrono::Utc::now().to_rfc3339();
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
                };
                transaction.execute(
                    "UPDATE rule_acceptances SET payload = ?2 WHERE id = ?1",
                    params![id, encode(&acceptance)?],
                )?;
                transaction.execute("UPDATE event_attempts SET status = 'accepted', accepted_at = COALESCE(accepted_at, ?2) WHERE id = ?1", params![attempt, timestamp])?;
                ("matched", None)
            }
            Ok(false) => ("no_match", None),
            Err(error) => ("failed", Some(error)),
        };
        transaction.execute("UPDATE rule_evaluations SET outcome = ?3, error = ?4 WHERE attempt_id = ?1 AND rule_name = ?2", params![attempt, name, outcome, error])?;
        transaction.commit()?;
        Ok(())
    }

    pub(crate) fn update(&self, acceptance: &Acceptance, assigned: bool) -> Result<(), Error> {
        let mut connection = self.events.connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let previous: String = transaction.query_row("SELECT a.payload FROM rule_acceptances a JOIN event_attempts p ON p.id = a.attempt_id JOIN events e ON e.sequence = p.event_sequence WHERE e.namespace = ?1 AND a.id = ?2", params![self.config.namespace, acceptance.id], |row| row.get(0)).optional()?.ok_or(Error::NotFound)?;
        let previous: Acceptance = decode(&previous)?;
        if previous.instance != acceptance.instance
            || previous.attempt_id != acceptance.attempt_id
            || previous.rule_name != acceptance.rule_name
        {
            return Err(Error::Conflict("acceptance identity cannot change".into()));
        }
        transaction.execute(
            "UPDATE rule_acceptances SET payload = ?2 WHERE id = ?1",
            params![acceptance.id, encode(acceptance)?],
        )?;
        if assigned && previous.operation_id.is_none() {
            let (provider, event_id): (String, String) = transaction.query_row(
                "SELECT provider, event_id FROM events WHERE sequence = ?1 AND namespace = ?2",
                params![acceptance.event_sequence, self.config.namespace],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?;
            transaction.execute("INSERT INTO provider_notifications(namespace, provider, payload) VALUES (?1, ?2, '')", params![self.config.namespace, provider])?;
            let id = transaction.last_insert_rowid();
            let notification = ProviderNotification {
                notification_id: id,
                event_id,
                sequence: acceptance.event_sequence,
                attempt_id: acceptance.attempt_id,
                kind: NotificationKind::Assigned,
                created_at: chrono::Utc::now().to_rfc3339(),
                dispatch_id: Some(acceptance.id),
                rule_name: Some(acceptance.rule_name.clone()),
                rule_revision: Some(acceptance.rule_revision),
                instance: Some(acceptance.instance.clone()),
            };
            transaction.execute(
                "UPDATE provider_notifications SET payload = ?2 WHERE id = ?1",
                params![id, encode(&notification)?],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub(crate) fn event(&self, sequence: i64) -> Result<crate::store::events::Record, Error> {
        self.events.record(sequence)
    }

    pub(crate) fn admit_dispatch(&self, id: i64) -> Result<bool, Error> {
        let mut connection = self.events.connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let recent: i64 = transaction.query_row(
            "SELECT count(*) FROM rule_dispatch_starts WHERE namespace = ?1 AND started_at >= ?2",
            params![
                self.config.namespace,
                (chrono::Utc::now() - chrono::Duration::minutes(1)).to_rfc3339()
            ],
            |row| row.get(0),
        )?;
        if recent >= 10 {
            return Ok(false);
        }
        transaction.execute(
            "INSERT INTO rule_dispatch_starts(namespace, acceptance_id, started_at) VALUES (?1, ?2, ?3)",
            params![self.config.namespace, id, chrono::Utc::now().to_rfc3339()],
        )?;
        transaction.commit()?;
        Ok(true)
    }
}

#[cfg(test)]
#[path = "tests/rules.rs"]
mod tests;
