use rusqlite::{Connection, params};

use super::EventStore;
use crate::store::{
    events::{Attempt, Error, Event, ProcessingStatus, diagnostics::*},
    rules::{Acceptance, Rule},
};

impl EventStore {
    pub(crate) fn diagnostics(&self, sequence: i64) -> Result<Diagnostics, Error> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        let (provider, event_id) = self.source(&transaction, sequence)?;
        let payload: String = transaction.query_row(
            "SELECT payload FROM events WHERE sequence = ?1",
            [sequence],
            |row| row.get(0),
        )?;
        let event: Event = decode(&payload)?;
        let attempts_total = transaction.query_row(
            "SELECT count(*) FROM event_attempts WHERE event_sequence = ?1",
            [sequence],
            |row| row.get::<_, u64>(0),
        )?;
        let mut statement = transaction.prepare(
            "SELECT id, status, replay, created_at, accepted_at FROM event_attempts
             WHERE event_sequence = ?1 ORDER BY id DESC LIMIT ?2",
        )?;
        let attempts = statement
            .query_map(params![sequence, ATTEMPT_LIMIT as i64], |row| {
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
        let mut diagnostics = Diagnostics {
            sequence,
            event_id,
            provider,
            summary: event.summary,
            counts: counts(&transaction, sequence)?,
            attempts_total,
            attempts_truncated: attempts_total > attempts.len() as u64,
            attempts: Vec::new(),
        };
        for attempt in attempts {
            let mut statement = transaction.prepare(
                "SELECT r.rule_snapshot, r.outcome, r.error, a.payload, r.warning
                 FROM rule_evaluations r LEFT JOIN rule_acceptances a
                 ON a.attempt_id = r.attempt_id AND a.rule_name = r.rule_name
                 WHERE r.attempt_id = ?1 ORDER BY r.rule_name",
            )?;
            let rows = statement
                .query_map([attempt.id], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, Option<String>>(4)?,
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            let mut rules = Vec::new();
            for (snapshot, outcome, error, acceptance, warning) in rows {
                let outcome = match outcome.as_deref() {
                    None => EvaluationOutcome::Pending,
                    Some("matched") => EvaluationOutcome::Matched,
                    Some("no_match") => EvaluationOutcome::NoMatch,
                    Some("failed") => EvaluationOutcome::Failed,
                    Some("deferred") => EvaluationOutcome::Deferred,
                    Some("throttled") => EvaluationOutcome::Throttled,
                    Some(value) => {
                        return Err(Error::Storage(format!(
                            "unknown evaluation outcome: {value}"
                        )));
                    }
                };
                rules.push(RuleDiagnostics {
                    rule: decode::<Rule>(&snapshot)?,
                    outcome,
                    error,
                    warning,
                    acceptance: acceptance
                        .as_deref()
                        .map(decode::<Acceptance>)
                        .transpose()?,
                    startup: None,
                    details_unavailable: None,
                });
            }
            diagnostics
                .attempts
                .push(AttemptDiagnostics { attempt, rules });
        }
        drop(statement);
        transaction.commit()?;
        for attempt in &mut diagnostics.attempts {
            for rule in &mut attempt.rules {
                self.startup_diagnostics(rule);
            }
        }
        Ok(diagnostics)
    }

    fn startup_diagnostics(&self, rule: &mut RuleDiagnostics) {
        let Some(acceptance) = &rule.acceptance else {
            if rule.outcome == EvaluationOutcome::Matched {
                rule.details_unavailable =
                    Some("The matched evaluation has no retained acceptance.".into());
            }
            return;
        };
        let Some(operation) = acceptance.operation_id.as_deref() else {
            rule.details_unavailable =
                Some("No startup operation is recorded for this acceptance.".into());
            return;
        };
        match super::super::startup::read(&self.config, &acceptance.instance) {
            Ok(Some(record))
                if record.origin_operation_id() == operation
                    && record.operation.template.as_deref()
                        == Some(&acceptance.rule.definition.template) =>
            {
                let mut budget = LOG_BYTE_LIMIT;
                let mut truncated = false;
                let warnings =
                    bounded_lines(&record.operation.warnings, &mut budget, &mut truncated);
                let progress =
                    bounded_lines(&record.operation.progress, &mut budget, &mut truncated);
                rule.startup = Some(StartupDiagnostics {
                    operation_id: record.operation.id,
                    state: record.operation.state,
                    error: record.operation.error,
                    warnings,
                    progress,
                    logs_truncated: truncated,
                });
            }
            Ok(_) => {
                rule.details_unavailable = Some(
                    "Startup logs are unavailable for this acceptance's recorded lineage.".into(),
                )
            }
            Err(error) => {
                rule.details_unavailable =
                    Some(format!("Cannot read retained startup logs: {error}"))
            }
        }
    }
}

pub(super) fn counts(connection: &Connection, sequence: i64) -> Result<Counts, Error> {
    let (errors, evaluation_warnings) = connection.query_row(
        "SELECT COALESCE(sum(r.outcome = 'failed' OR r.error IS NOT NULL), 0),
                COALESCE(sum(r.outcome = 'throttled' OR r.warning IS NOT NULL), 0)
         FROM rule_evaluations r JOIN event_attempts p ON p.id = r.attempt_id
         WHERE p.event_sequence = ?1",
        [sequence],
        |row| Ok((row.get::<_, u64>(0)?, row.get::<_, u64>(1)?)),
    )?;
    let (dispatch_errors, warnings) = connection.query_row(
        "SELECT COALESCE(sum(json_extract(a.payload, '$.status') != 'uncertain'
                AND (json_extract(a.payload, '$.status') = 'failed' OR json_extract(a.payload, '$.error') IS NOT NULL)), 0),
                COALESCE(sum(json_extract(a.payload, '$.status') = 'uncertain'), 0)
         FROM rule_acceptances a JOIN event_attempts p ON p.id = a.attempt_id
         WHERE p.event_sequence = ?1",
        [sequence], |row| Ok((row.get::<_, u64>(0)?, row.get::<_, u64>(1)?)),
    )?;
    Ok(Counts {
        errors: errors + dispatch_errors,
        warnings: warnings + evaluation_warnings,
    })
}

fn decode<T: serde::de::DeserializeOwned>(value: &str) -> Result<T, Error> {
    serde_json::from_str(value).map_err(|error| Error::Storage(error.to_string()))
}

fn bounded_lines(lines: &[String], budget: &mut usize, truncated: &mut bool) -> Vec<String> {
    let mut result = Vec::new();
    let mut remaining = *budget;
    for line in lines
        .iter()
        .flat_map(|entry| entry.lines())
        .rev()
        .take(LOG_LINE_LIMIT)
    {
        if remaining <= 1 {
            break;
        }
        let mut start = line.len().saturating_sub(remaining - 1);
        while !line.is_char_boundary(start) {
            start += 1;
        }
        *truncated |= start > 0;
        let tail = &line[start..];
        remaining -= tail.len() + 1;
        result.push(tail.to_owned());
    }
    *budget = remaining;
    *truncated |= result.len() < lines.iter().map(|entry| entry.lines().count()).sum();
    result.reverse();
    result
}
