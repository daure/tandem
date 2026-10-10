use super::{Config, Kind, Record, read, runtime_db, write};
use crate::store::environments::SessionPing;

const RETENTION_NANOSECONDS: u64 = 3_600_000_000_000;

fn now() -> u64 {
    chrono::Utc::now()
        .timestamp_nanos_opt()
        .unwrap_or_default()
        .max(0) as u64
}

pub(in crate::environments) fn record_ping(
    config: &Config,
    instance: &str,
    session_id: &str,
) -> Result<(), String> {
    let mut record = read(config, instance)?;
    let now = now();
    record
        .pings
        .retain(|_, revision| now.abs_diff(*revision) < RETENTION_NANOSECONDS);
    let revision = record.pings.entry(session_id.to_owned()).or_default();
    *revision = now.max(revision.saturating_add(1));
    write(config, instance, &record)
}

pub(in crate::environments) fn session_pings(config: &Config) -> Result<Vec<SessionPing>, String> {
    let now = now();
    let mut pings = Vec::new();
    for (_, text) in runtime_db::list(config, Kind::Journal)? {
        let record: Record = serde_json::from_str(&text).map_err(|error| error.to_string())?;
        let Some(instance) = record.expected else {
            continue;
        };
        for (session_id, revision) in record.pings {
            if now.abs_diff(revision) < RETENTION_NANOSECONDS {
                pings.push(SessionPing {
                    instance: instance.name.clone(),
                    session_id,
                    revision,
                });
            }
        }
    }
    Ok(pings)
}
