use rusqlite::{OptionalExtension, TransactionBehavior, params};

use super::{RuleStore, decode};
use crate::store::{
    environments::Instance,
    events::Error,
    rules::{
        Acceptance,
        reports::{CleanupState, Report, ReportInput, ReportSummary},
    },
};

const REPORT_SELECT: &str = "SELECT a.payload, r.title, r.summary, r.markdown, r.reported_at,
    r.cleanup_state, r.cleanup_error FROM rule_acceptance_reports r
    JOIN rule_acceptances a ON a.id = r.acceptance_id
    JOIN event_attempts p ON p.id = a.attempt_id JOIN events e ON e.sequence = p.event_sequence
    WHERE e.namespace = ?1";

impl RuleStore {
    pub(super) fn report_summaries(
        &self,
        connection: &rusqlite::Connection,
    ) -> Result<std::collections::BTreeMap<i64, ReportSummary>, Error> {
        let select = REPORT_SELECT.replace("r.markdown", "''");
        let mut statement = connection.prepare(&select)?;
        let mut summaries = std::collections::BTreeMap::new();
        for report in statement.query_map([&self.config.namespace], read_report)? {
            let mut details = report?.details;
            self.observe_report_cleanup(&mut details)?;
            summaries.insert(details.acceptance_id, details);
        }
        Ok(summaries)
    }

    pub(crate) fn reported_acceptances(&self) -> Result<std::collections::BTreeSet<i64>, Error> {
        let connection = self.events.connection()?;
        connection
            .prepare(
                "SELECT r.acceptance_id FROM rule_acceptance_reports r
                 JOIN rule_acceptances a ON a.id=r.acceptance_id
                 JOIN event_attempts p ON p.id=a.attempt_id JOIN events e ON e.sequence=p.event_sequence
                 WHERE e.namespace=?1",
            )?
            .query_map([&self.config.namespace], |row| row.get(0))?
            .collect::<Result<_, _>>()
            .map_err(Error::from)
    }

    pub(crate) fn instance_acceptance(
        &self,
        instance: &Instance,
    ) -> Result<Option<Acceptance>, Error> {
        let connection = self.events.connection()?;
        let mut statement = connection.prepare(
            "SELECT a.payload FROM rule_acceptances a
             JOIN event_attempts p ON p.id = a.attempt_id JOIN events e ON e.sequence = p.event_sequence
             WHERE e.namespace = ?1 AND json_extract(a.payload, '$.instance') = ?2",
        )?;
        let rows = statement
            .query_map(params![self.config.namespace, instance.name], |row| {
                row.get::<_, String>(0)
            })?
            .map(|row| decode::<Acceptance>(&row?))
            .collect::<Result<Vec<_>, Error>>()?;
        if rows.is_empty() {
            return Ok(None);
        }
        if rows.len() != 1 || rows[0].rule.definition.template != instance.template {
            return Err(Error::Conflict(
                "workspace must belong to exactly one retained event acceptance".into(),
            ));
        }
        if !super::super::startup::belongs_to(
            &self.config,
            &instance.name,
            &instance.template,
            rows[0].operation_id.as_deref(),
        )
        .map_err(Error::Storage)?
        {
            return Err(Error::Conflict(
                "workspace belongs to another acceptance lineage".into(),
            ));
        }
        if self.provider_deleting(rows[0].event_sequence)? {
            return Err(Error::Conflict(
                "acceptance belongs to an incomplete provider deletion".into(),
            ));
        }
        Ok(rows.into_iter().next())
    }

    pub(crate) fn save_report(&self, id: i64, input: &ReportInput) -> Result<Report, Error> {
        input.validate().map_err(Error::Invalid)?;
        let mut connection = self.events.connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing = transaction
            .query_row(
                &format!("{REPORT_SELECT} AND a.id = ?2"),
                params![self.config.namespace, id],
                read_report,
            )
            .optional()?;
        if let Some(existing) = existing {
            if existing.details.title != input.title
                || existing.details.summary != input.summary
                || existing.markdown != input.markdown
            {
                return Err(Error::Conflict(
                    "acceptance already has a different report; saved reports are immutable".into(),
                ));
            }
            if existing.details.cleanup_state == CleanupState::Purged {
                return Err(Error::Conflict(
                    "acceptance instance is already concluded".into(),
                ));
            }
        }
        let owns: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM rule_acceptances a JOIN event_attempts p ON p.id=a.attempt_id
             JOIN events e ON e.sequence=p.event_sequence WHERE e.namespace=?1 AND a.id=?2)",
            params![self.config.namespace, id], |row| row.get(0),
        )?;
        if !owns {
            return Err(Error::NotFound);
        }
        transaction.execute(
            "INSERT INTO rule_acceptance_reports(acceptance_id, title, summary, markdown, reported_at, cleanup_state)
             VALUES (?1, ?2, ?3, ?4, ?5, 'pending') ON CONFLICT(acceptance_id) DO UPDATE SET cleanup_state='pending', cleanup_error=NULL",
            params![id, input.title, input.summary, input.markdown, chrono::Utc::now().to_rfc3339()],
        )?;
        let report = transaction.query_row(
            &format!("{REPORT_SELECT} AND a.id = ?2"),
            params![self.config.namespace, id],
            read_report,
        )?;
        transaction.commit()?;
        Ok(report)
    }

    pub(crate) fn set_report_cleanup(
        &self,
        id: i64,
        state: CleanupState,
        error: Option<&str>,
    ) -> Result<(), Error> {
        let state = match state {
            CleanupState::Pending => "pending",
            CleanupState::Purging => "purging",
            CleanupState::Purged => "purged",
            CleanupState::Failed => "failed",
        };
        let changed = self.events.connection()?.execute(
            "UPDATE rule_acceptance_reports SET cleanup_state=?3, cleanup_error=?4 WHERE acceptance_id=?2 AND acceptance_id IN
             (SELECT a.id FROM rule_acceptances a JOIN event_attempts p ON p.id=a.attempt_id
              JOIN events e ON e.sequence=p.event_sequence WHERE e.namespace=?1)",
            params![self.config.namespace, id, state, error],
        )?;
        if changed != 1 {
            return Err(Error::NotFound);
        }
        Ok(())
    }

    pub(crate) fn event_report(&self, id: i64) -> Result<Report, Error> {
        let mut report = self
            .events
            .connection()?
            .query_row(
                &format!("{REPORT_SELECT} AND a.id = ?2"),
                params![self.config.namespace, id],
                read_report,
            )
            .optional()?
            .ok_or(Error::NotFound)?;
        self.observe_report_cleanup(&mut report.details)?;
        Ok(report)
    }

    pub(crate) fn search_reports(&self, strings: Vec<String>) -> Result<Vec<ReportSummary>, Error> {
        let terms = crate::store::rules::reports::search_terms(strings).map_err(Error::Invalid)?;
        let connection = self.events.connection()?;
        let mut statement = connection.prepare(&format!("{REPORT_SELECT} ORDER BY a.id DESC"))?;
        let mut matches = Vec::new();
        for report in statement.query_map([&self.config.namespace], read_report)? {
            let report = report?;
            let fields = [
                &report.details.title,
                &report.details.summary,
                &report.markdown,
            ]
            .map(|text| text.to_lowercase());
            if terms
                .iter()
                .any(|term| fields.iter().any(|field| field.contains(term)))
            {
                let mut details = report.details;
                self.observe_report_cleanup(&mut details)?;
                matches.push(details);
            }
        }
        Ok(matches)
    }

    fn observe_report_cleanup(&self, details: &mut ReportSummary) -> Result<(), Error> {
        if matches!(
            details.cleanup_state,
            CleanupState::Pending | CleanupState::Purging
        ) && !super::super::gateway::is_locked(
            &self.config,
            &super::super::conclusion::lease(details.acceptance_id),
        )
        .map_err(Error::Storage)?
        {
            let latest = self.events.connection()?.query_row(
                &format!("{REPORT_SELECT} AND a.id = ?2"),
                params![self.config.namespace, details.acceptance_id],
                read_report,
            )?;
            if matches!(
                latest.details.cleanup_state,
                CleanupState::Purged | CleanupState::Failed
            ) {
                *details = latest.details;
                return Ok(());
            }
            details.cleanup_state = CleanupState::Failed;
            details.cleanup_error = Some("Conclusion cleanup interrupted; report preserved. Inspect the instance before retrying.".into());
        }
        Ok(())
    }
}

fn read_report(row: &rusqlite::Row<'_>) -> rusqlite::Result<Report> {
    let payload: String = row.get(0)?;
    let acceptance: Acceptance = serde_json::from_str(&payload).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(error))
    })?;
    let state: String = row.get(5)?;
    let cleanup_state = match state.as_str() {
        "pending" => CleanupState::Pending,
        "purging" => CleanupState::Purging,
        "purged" => CleanupState::Purged,
        "failed" => CleanupState::Failed,
        _ => return Err(rusqlite::Error::InvalidQuery),
    };
    Ok(Report {
        details: ReportSummary {
            acceptance_id: acceptance.id,
            event_sequence: acceptance.event_sequence,
            rule_name: acceptance.rule_name,
            title: row.get(1)?,
            summary: row.get(2)?,
            reported_at: row.get(4)?,
            cleanup_state,
            cleanup_error: row.get(6)?,
        },
        markdown: row.get(3)?,
    })
}

#[cfg(test)]
#[path = "../tests/reports.rs"]
mod tests;
