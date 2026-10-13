use rusqlite::params;
use serde_json::Value;

use super::Store;

pub(crate) struct LivePage {
    pub(crate) events: Vec<Value>,
    pub(crate) cursor: String,
}

impl Store {
    pub(crate) fn head(&self) -> Result<String, String> {
        self.connection()?
            .query_row(
                "SELECT epoch || ':' || sequence FROM state_stream_heads WHERE namespace = ?1",
                [&self.config.namespace],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())
    }

    pub(crate) fn live_page(&self, cursor: &str) -> Result<LivePage, String> {
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction()
            .map_err(|error| error.to_string())?;
        let (epoch, head, floor): (String, i64, i64) = transaction
            .query_row(
                "SELECT epoch, sequence, floor FROM state_stream_heads WHERE namespace = ?1",
                [&self.config.namespace],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .map_err(|error| error.to_string())?;
        let after = cursor
            .rsplit_once(':')
            .filter(|(value, _)| *value == epoch)
            .and_then(|(_, value)| value.parse::<i64>().ok())
            .filter(|value| *value >= floor && *value <= head)
            .ok_or("state subscriber cursor expired; reconnect for a snapshot")?;
        let mut events = Vec::new();
        let mut last = after;
        let mut bytes = 0;
        {
            let mut statement = transaction
                .prepare_cached("SELECT sequence, payload FROM state_stream_events WHERE namespace = ?1 AND sequence > ?2 ORDER BY sequence LIMIT 64")
                .map_err(|error| error.to_string())?;
            let rows = statement
                .query_map(params![self.config.namespace, after], |row| {
                    Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
                })
                .map_err(|error| error.to_string())?;
            for row in rows {
                let (sequence, text) = row.map_err(|error| error.to_string())?;
                if !events.is_empty() && bytes + text.len() + 1 > 2 * 1024 * 1024 {
                    break;
                }
                bytes += text.len() + 1;
                events.push(serde_json::from_str(&text).map_err(|error| error.to_string())?);
                last = sequence;
            }
        }
        transaction.commit().map_err(|error| error.to_string())?;
        Ok(LivePage {
            events,
            cursor: format!("{epoch}:{last}"),
        })
    }
}
