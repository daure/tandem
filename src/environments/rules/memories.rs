use rusqlite::{
    OptionalExtension, Transaction, TransactionBehavior, params, params_from_iter, types::Value,
};

use super::RuleStore;
use crate::store::{
    events::Error,
    memories::{
        Memory, MemoryInput, MemoryMatch, MemoryMatches, MemorySearch, MemorySummary,
        canonical_url, parse_time,
    },
};

const SUMMARY: &str = "m.id, m.instance, m.acceptance_id, m.conclusion_id, m.created_at,
    m.title, m.summary, m.labels, m.links, m.workspace_identity";

impl RuleStore {
    pub(crate) fn remember(
        &self,
        instance: &str,
        identity: &str,
        acceptance: Option<i64>,
        input: MemoryInput,
    ) -> Result<MemorySummary, Error> {
        let input = input.normalized().map_err(Error::Invalid)?;
        let mut connection = self.events.connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let receipt =
            self.save_memory_in(&transaction, instance, identity, acceptance, None, &input)?;
        transaction.commit()?;
        Ok(receipt)
    }

    pub(super) fn save_memory_in(
        &self,
        transaction: &Transaction<'_>,
        instance: &str,
        identity: &str,
        acceptance: Option<i64>,
        conclusion: Option<i64>,
        input: &MemoryInput,
    ) -> Result<MemorySummary, Error> {
        let now = chrono::Utc::now();
        transaction.execute(
            "INSERT INTO memories(namespace, instance, workspace_identity, acceptance_id, conclusion_id,
             created_at, created_ms, title, summary, labels, links, markdown, fields)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![self.config.namespace, instance, identity, acceptance, conclusion,
                now.to_rfc3339(), now.timestamp_millis(), input.title, input.summary,
                json(&input.labels)?, json(&input.links)?, input.markdown, json(&input.fields)?],
        )?;
        Ok(MemorySummary {
            memory_id: transaction.last_insert_rowid(),
            instance: instance.into(),
            acceptance_id: acceptance,
            workspace_identity: identity.into(),
            conclusion_id: conclusion,
            created_at: now.to_rfc3339(),
            title: input.title.clone(),
            summary: input.summary.clone(),
            labels: input.labels.clone(),
            links: input.links.clone(),
        })
    }

    pub(crate) fn memory(&self, id: i64) -> Result<Memory, Error> {
        if id <= 0 {
            return Err(Error::Invalid("memory_id must be positive".into()));
        }
        self.events.connection()?.query_row(
            &format!("SELECT {SUMMARY}, m.markdown, m.fields FROM memories m WHERE m.namespace = ?1 AND m.id = ?2"),
            params![self.config.namespace, id], |row| Ok(Memory { details: read_summary(row)?, markdown: row.get(10)?, fields: read_json(row, 11)? }),
        ).optional()?.ok_or(Error::NotFound)
    }

    pub(crate) fn search_memories(&self, input: MemorySearch) -> Result<MemoryMatches, Error> {
        let query = input.validate().map_err(Error::Invalid)?;
        let (mut sql, mut parameters) = if let Some(query) = query {
            (
                format!(
                    "SELECT {SUMMARY}, substr(snippet(memory_search, -1, '[', ']', ' … ', 32), 1, 480)
                FROM memory_search JOIN memories m ON m.id = memory_search.rowid
                WHERE m.namespace = ? AND memory_search MATCH ?"
                ),
                vec![
                    Value::Text(self.config.namespace.clone()),
                    Value::Text(query),
                ],
            )
        } else {
            (
                format!(
                    "SELECT {SUMMARY}, substr(m.summary, 1, 240) FROM memories m WHERE m.namespace = ?"
                ),
                vec![Value::Text(self.config.namespace.clone())],
            )
        };
        for label in &input.labels {
            sql.push_str(" AND EXISTS (SELECT 1 FROM json_each(m.labels) WHERE value = ?)");
            parameters.push(Value::Text(label.trim().to_lowercase()));
        }
        for link in &input.links {
            sql.push_str(" AND EXISTS (SELECT 1 FROM json_each(m.links) WHERE json_extract(value, '$.url') = ?)");
            parameters.push(Value::Text(canonical_url(link).map_err(Error::Invalid)?));
        }
        if let Some(instance) = &input.instance {
            sql.push_str(" AND m.instance = ?");
            parameters.push(Value::Text(instance.clone()));
        }
        for (date, predicate) in [
            (&input.after, " AND m.created_ms >= ?"),
            (&input.before, " AND m.created_ms <= ?"),
        ] {
            if let Some(date) = date {
                sql.push_str(predicate);
                parameters.push(Value::Integer(parse_time(date).map_err(Error::Invalid)?));
            }
        }
        for (key, value) in &input.fields {
            sql.push_str(
                " AND EXISTS (SELECT 1 FROM json_each(m.fields) f WHERE f.key = ?
                AND f.type = json_type(?, '$') AND f.value IS json_extract(?, '$'))",
            );
            let mut value = value.clone();
            value.sort_all_objects();
            let encoded = json(&value)?;
            parameters.extend([
                Value::Text(key.clone()),
                Value::Text(encoded.clone()),
                Value::Text(encoded),
            ]);
        }
        if input.query.is_some() {
            sql.push_str(" ORDER BY bm25(memory_search, 10.0, 8.0, 5.0, 1.0, 1.0, 1.0), m.id DESC");
        } else {
            sql.push_str(" ORDER BY m.id DESC");
        }
        let limit = input.limit.unwrap_or(20) as usize;
        sql.push_str(" LIMIT ? OFFSET ?");
        parameters.extend([
            Value::Integer(limit as i64 + 1),
            Value::Integer(input.offset.unwrap_or(0).into()),
        ]);
        let connection = self.events.connection()?;
        let mut statement = connection.prepare(&sql)?;
        let mut memories = statement
            .query_map(params_from_iter(parameters), |row| {
                Ok(MemoryMatch {
                    details: read_summary(row)?,
                    snippet: row.get(10)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        let has_more = memories.len() > limit;
        memories.truncate(limit);
        Ok(MemoryMatches { memories, has_more })
    }
}

fn json(value: &impl serde::Serialize) -> Result<String, Error> {
    serde_json::to_string(value).map_err(|error| Error::Storage(error.to_string()))
}

fn read_json<T: serde::de::DeserializeOwned>(
    row: &rusqlite::Row<'_>,
    index: usize,
) -> rusqlite::Result<T> {
    let payload: String = row.get(index)?;
    serde_json::from_str(&payload).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            index,
            rusqlite::types::Type::Text,
            Box::new(error),
        )
    })
}

fn read_summary(row: &rusqlite::Row<'_>) -> rusqlite::Result<MemorySummary> {
    Ok(MemorySummary {
        memory_id: row.get(0)?,
        instance: row.get(1)?,
        acceptance_id: row.get(2)?,
        conclusion_id: row.get(3)?,
        created_at: row.get(4)?,
        title: row.get(5)?,
        summary: row.get(6)?,
        labels: read_json(row, 7)?,
        links: read_json(row, 8)?,
        workspace_identity: row.get(9)?,
    })
}

#[cfg(test)]
#[path = "../tests/memories.rs"]
mod tests;
