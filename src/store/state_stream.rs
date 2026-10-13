use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub(crate) const VERSION: u32 = 1;
pub(crate) const MAX_FRAME_BYTES: usize = 16 * 1024 * 1024;
pub(crate) const MAX_REPLAY_BYTES: usize = 32 * 1024 * 1024;
pub(crate) const MAX_REPLAY_EVENTS: usize = 4096;

pub(crate) fn validate_origin(origin: Option<&str>) -> Result<(), String> {
    if origin.is_some_and(|value| value.chars().count() > 512) {
        return Err("origin must contain at most 512 Unicode characters".into());
    }
    Ok(())
}

pub(crate) fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub(crate) struct StateSnapshot {
    pub instances: Value,
    pub sessions: Value,
}

impl StateSnapshot {
    pub(crate) fn empty() -> Self {
        Self {
            instances: json!({"instances": [], "activities": [], "observed_at_unix_seconds": 0, "runtime_error": null}),
            sessions: json!({"sessions": [], "observed_at_unix_seconds": 0, "history_window_per_directory": 21, "observation_error": null, "inventory_error": null}),
        }
    }

    pub(crate) fn changes(&self, previous: &Self) -> Vec<(&'static str, Value)> {
        let mut changes = Vec::new();
        compare(
            &mut changes,
            &previous.instances["instances"],
            &self.instances["instances"],
            false,
        );
        compare(
            &mut changes,
            &previous.sessions["sessions"],
            &self.sessions["sessions"],
            true,
        );
        let metadata = self.metadata();
        if metadata != previous.metadata() {
            changes.push(("state.changed", metadata));
        }
        changes
    }

    pub(crate) fn metadata(&self) -> Value {
        let mut instances = self.instances.clone();
        let mut sessions = self.sessions.clone();
        for (value, key) in [(&mut instances, "instances"), (&mut sessions, "sessions")] {
            if let Some(object) = value.as_object_mut() {
                object.remove(key);
                object.remove("observed_at_unix_seconds");
            }
        }
        json!({"instances": instances, "sessions": sessions})
    }
}

pub(crate) fn session_key(value: &Value) -> String {
    serde_json::to_string(&(&value["server"], &value["session_id"])).unwrap()
}

fn entities(value: &Value, sessions: bool) -> BTreeMap<String, Value> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .map(|value| {
            let key = if sessions {
                session_key(value)
            } else {
                value["name"]
                    .as_str()
                    .unwrap_or_default()
                    .to_ascii_lowercase()
            };
            (key, value.clone())
        })
        .collect()
}

fn compare(
    changes: &mut Vec<(&'static str, Value)>,
    previous: &Value,
    current: &Value,
    sessions: bool,
) {
    let previous = entities(previous, sessions);
    let current = entities(current, sessions);
    for (key, value) in &current {
        match previous.get(key) {
            None => changes.push((
                if sessions {
                    "session.created"
                } else {
                    "instance.changed"
                },
                value.clone(),
            )),
            Some(old) if old != value => changes.push((
                if sessions {
                    "session.changed"
                } else {
                    "instance.changed"
                },
                value.clone(),
            )),
            _ => {}
        }
    }
    for (key, value) in previous {
        if !current.contains_key(&key) {
            changes.push((
                if sessions {
                    "session.removed"
                } else {
                    "instance.removed"
                },
                value,
            ));
        }
    }
}

pub(crate) fn envelope(kind: &str, cursor: &str, data: Value) -> Value {
    json!({"type": kind, "version": VERSION, "at": now(), "cursor": cursor, "data": data})
}
