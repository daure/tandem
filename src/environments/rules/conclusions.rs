use rusqlite::{OptionalExtension, TransactionBehavior, params};

use super::{RuleStore, decode, encode};
use crate::store::{
    events::Error,
    rules::reports::{CleanupState, Conclusion, ConclusionReceipt, ReportInput},
};

const SELECT: &str = "SELECT id, instance, workspace_identity, acceptance_id, report, admitted_at,
    cleanup_state, cleanup_error,
    (SELECT m.id FROM memories m WHERE m.conclusion_id = instance_conclusions.id) AS memory_id
    FROM instance_conclusions WHERE namespace = ?1";

impl RuleStore {
    pub(crate) fn save_conclusion(
        &self,
        instance: &str,
        identity: &str,
        acceptance: Option<i64>,
        report: Option<&ReportInput>,
    ) -> Result<ConclusionReceipt, Error> {
        let normalized = report
            .cloned()
            .map(ReportInput::normalized)
            .transpose()
            .map_err(Error::Invalid)?;
        let report = normalized.as_ref();
        let mut connection = self.events.connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing = transaction
            .query_row(
                &format!("{SELECT} AND instance = ?2 AND workspace_identity = ?3"),
                params![self.config.namespace, instance, identity],
                read,
            )
            .optional()?;
        if let Some(id) = acceptance {
            if let Some(report) = report {
                self.save_report_in(&transaction, id, report)?;
            } else {
                let saved: bool = transaction.query_row(
                    "SELECT EXISTS(SELECT 1 FROM rule_acceptance_reports WHERE acceptance_id = ?1)",
                    [id],
                    |row| row.get(0),
                )?;
                if saved {
                    return Err(Error::Conflict(
                        "acceptance has a saved report; retry with identical contents".into(),
                    ));
                }
            }
        }
        if let Some(existing) = existing {
            if existing.report.as_ref() != report || existing.details.acceptance_id != acceptance {
                return Err(Error::Conflict("conclusion already has different contents; retry with the identical report or omission".into()));
            }
            if existing.details.cleanup_state == CleanupState::Purged {
                return Err(Error::Conflict("instance is already concluded".into()));
            }
            transaction.execute("UPDATE instance_conclusions SET cleanup_state = 'pending', cleanup_error = NULL WHERE id = ?1", [existing.details.conclusion_id])?;
            transaction.commit()?;
            return Ok(ConclusionReceipt {
                cleanup_state: CleanupState::Pending,
                cleanup_error: None,
                ..existing.details
            });
        }
        let payload = report.map(encode).transpose()?;
        transaction.execute(
            "INSERT INTO instance_conclusions(namespace, instance, workspace_identity, acceptance_id, report, admitted_at, cleanup_state)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'pending')",
            params![self.config.namespace, instance, identity, acceptance, payload, chrono::Utc::now().to_rfc3339()],
        )?;
        let conclusion_id = transaction.last_insert_rowid();
        if let Some(report) = report {
            self.save_memory_in(
                &transaction,
                instance,
                identity,
                acceptance,
                Some(conclusion_id),
                report,
            )?;
        }
        let receipt = transaction
            .query_row(
                &format!("{SELECT} AND id = ?2"),
                params![self.config.namespace, conclusion_id],
                read,
            )?
            .details;
        transaction.commit()?;
        Ok(receipt)
    }

    pub(crate) fn conclusion(&self, id: i64) -> Result<Conclusion, Error> {
        let mut result = self
            .events
            .connection()?
            .query_row(
                &format!("{SELECT} AND id = ?2"),
                params![self.config.namespace, id],
                read,
            )
            .optional()?
            .ok_or(Error::NotFound)?;
        self.observe_conclusion(&mut result.details)?;
        Ok(result)
    }

    pub(crate) fn workspace_conclusion(
        &self,
        instance: &str,
        identity: &str,
    ) -> Result<Option<Conclusion>, Error> {
        self.events
            .connection()?
            .query_row(
                &format!("{SELECT} AND instance = ?2 AND workspace_identity = ?3"),
                params![self.config.namespace, instance, identity],
                read,
            )
            .optional()
            .map_err(Error::from)
    }

    pub(crate) fn set_conclusion_cleanup(
        &self,
        id: i64,
        state: CleanupState,
        error: Option<&str>,
    ) -> Result<(), Error> {
        let state = encode(&state)?;
        let mut connection = self.events.connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let changed = transaction.execute(
            "UPDATE instance_conclusions SET cleanup_state = ?3, cleanup_error = ?4 WHERE namespace = ?1 AND id = ?2",
            params![self.config.namespace, id, state.trim_matches('"'), error],
        )?;
        if changed != 1 {
            return Err(Error::NotFound);
        }
        transaction.execute("UPDATE rule_acceptance_reports SET cleanup_state = ?3, cleanup_error = ?4 WHERE acceptance_id IN
            (SELECT acceptance_id FROM instance_conclusions WHERE namespace = ?1 AND id = ?2 AND report IS NOT NULL)",
            params![self.config.namespace, id, state.trim_matches('"'), error])?;
        transaction.commit()?;
        Ok(())
    }

    pub(crate) fn search_conclusions(
        &self,
        strings: Vec<String>,
    ) -> Result<Vec<ConclusionReceipt>, Error> {
        let terms = crate::store::rules::reports::search_terms(strings).map_err(Error::Invalid)?;
        let connection = self.events.connection()?;
        let mut statement =
            connection.prepare(&format!("{SELECT} AND report IS NOT NULL ORDER BY id DESC"))?;
        let mut results = Vec::new();
        for result in statement.query_map([&self.config.namespace], read)? {
            let mut result = result?;
            if let Some(report) = result.report {
                let fields = [&report.title, &report.summary, &report.markdown]
                    .map(|field| field.to_lowercase());
                if terms
                    .iter()
                    .any(|term| fields.iter().any(|field| field.contains(term)))
                {
                    self.observe_conclusion(&mut result.details)?;
                    results.push(result.details);
                }
            }
        }
        Ok(results)
    }

    fn observe_conclusion(&self, details: &mut ConclusionReceipt) -> Result<(), Error> {
        if matches!(
            details.cleanup_state,
            CleanupState::Pending | CleanupState::Purging
        ) && !super::super::gateway::is_locked(
            &self.config,
            &super::super::conclusion::worker_lease(
                details.acceptance_id.unwrap_or(0),
                &details.instance,
            ),
        )
        .map_err(Error::Storage)?
        {
            let latest = self
                .events
                .connection()?
                .query_row(
                    &format!("{SELECT} AND id = ?2"),
                    params![self.config.namespace, details.conclusion_id],
                    read,
                )?
                .details;
            if matches!(
                latest.cleanup_state,
                CleanupState::Purged | CleanupState::Failed
            ) {
                *details = latest;
            } else {
                details.cleanup_state = CleanupState::Failed;
                details.cleanup_error = Some(
                    "Conclusion cleanup interrupted; inspect the instance before retrying.".into(),
                );
            }
        }
        Ok(())
    }
}

fn read(row: &rusqlite::Row<'_>) -> rusqlite::Result<Conclusion> {
    let payload: Option<String> = row.get(4)?;
    let report: Option<ReportInput> = payload
        .map(|payload| decode(&payload))
        .transpose()
        .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
    let state: String = row.get(6)?;
    let state = serde_json::from_value(serde_json::Value::String(state))
        .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
    Ok(Conclusion {
        details: ConclusionReceipt {
            conclusion_id: row.get(0)?,
            memory_id: row.get(8)?,
            instance: row.get(1)?,
            acceptance_id: row.get(3)?,
            title: report.as_ref().map(|report| report.title.clone()),
            summary: report.as_ref().map(|report| report.summary.clone()),
            admitted_at: row.get(5)?,
            cleanup_state: state,
            cleanup_error: row.get(7)?,
        },
        report,
        workspace_identity: row.get(2)?,
    })
}

#[cfg(test)]
#[path = "../tests/conclusions.rs"]
mod tests;
