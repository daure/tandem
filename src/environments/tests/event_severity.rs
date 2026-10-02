use super::*;
use crate::store::events::SeverityColor;

fn system_event(id: &str, data: serde_json::Value) -> Event {
    let mut value = serde_json::to_value(event(id)).unwrap();
    value["profile"] = json!("system_event");
    value["data"] = data;
    serde_json::from_value(value).unwrap()
}

#[test]
fn system_snapshots_preserve_source_labels_and_explicit_colors() {
    let (_home, config, store, token) = setup();
    let colors = [
        ("plain", SeverityColor::Plain),
        ("info", SeverityColor::Info),
        ("warning", SeverityColor::Warning),
        ("error", SeverityColor::Error),
        ("success", SeverityColor::Success),
    ];
    let mut expected = Vec::new();
    for (label, color) in colors {
        let severity = if color == SeverityColor::Error {
            " Healthy "
        } else {
            " Critical "
        };
        expected.push((
            system_event(
                label,
                json!({
                    "resource": "ci", "signal": "build", "severity": severity,
                    "description": "Build observation", "environment": " Production / EU ",
                    "severity_color": label
                }),
            ),
            color,
        ));
    }
    expected.push((
        system_event(
            "custom",
            json!({
                "resource": "ci", "signal": "build", "severity": " P1 / provider-defined ",
                "description": "Build observation", "environment": " Production / EU ",
                "severity_color": "warning"
            }),
        ),
        SeverityColor::Warning,
    ));
    store
        .ingest(
            &token,
            Batch {
                events: expected.iter().map(|(event, _)| event.clone()).collect(),
            },
        )
        .unwrap();
    let snapshot = EventStore::open(&config).unwrap().snapshot().unwrap();
    assert_eq!(snapshot.total, expected.len() as u64);
    for (event, color) in expected {
        let record = snapshot
            .records
            .iter()
            .find(|record| record.event.event_id == event.event_id)
            .unwrap();
        assert_eq!(record.provider, "sample");
        assert_eq!(record.event, event);
        let Payload::SystemEvent(system) = &record.event.payload else {
            panic!("expected system event")
        };
        assert_eq!(system.environment.as_deref(), Some(" Production / EU "));
        assert_eq!(system.severity_color, Some(color));
        assert_eq!(system.severity_color(), color);
        assert_eq!(record.attempts[0].status, ProcessingStatus::Pending);
    }
}

#[test]
fn severity_aliases_match_exactly_ignoring_case_and_surrounding_whitespace() {
    let (_home, _config, store, token) = setup();
    let groups: &[(SeverityColor, &[&str])] = &[
        (
            SeverityColor::Info,
            &[
                "info",
                "information",
                "informational",
                "notice",
                "debug",
                "trace",
            ],
        ),
        (SeverityColor::Warning, &["warn", "warning", "caution"]),
        (
            SeverityColor::Error,
            &[
                "err",
                "error",
                "critical",
                "crit",
                "fatal",
                "severe",
                "emergency",
                "emerg",
                "alert",
            ],
        ),
        (
            SeverityColor::Success,
            &[
                "success",
                "successful",
                "ok",
                "okay",
                "done",
                "complete",
                "completed",
                "passed",
                "resolved",
                "healthy",
            ],
        ),
        (
            SeverityColor::Plain,
            &[
                "cherry",
                "noterror",
                "successful-ish",
                "information technology",
            ],
        ),
    ];
    let mut expected = Vec::new();
    for &(color, aliases) in groups {
        for alias in aliases {
            for severity in [
                alias.to_string(),
                format!(" {} ", alias.to_ascii_uppercase()),
            ] {
                expected.push((
                    system_event(
                        &format!("alias-{}", expected.len()),
                        json!({
                            "resource": "ci", "signal": "build", "severity": severity,
                            "description": "Build observation"
                        }),
                    ),
                    color,
                ));
            }
        }
    }
    store
        .ingest(
            &token,
            Batch {
                events: expected.iter().map(|(event, _)| event.clone()).collect(),
            },
        )
        .unwrap();
    let snapshot = store.snapshot().unwrap();
    assert_eq!(snapshot.records.len(), expected.len());
    for (event, color) in expected {
        let record = snapshot
            .records
            .iter()
            .find(|record| record.event.event_id == event.event_id)
            .unwrap();
        assert_eq!(record.event, event);
        let Payload::SystemEvent(system) = &record.event.payload else {
            panic!("expected system event")
        };
        assert_eq!(system.severity_color(), color, "{}", system.severity);
        assert_eq!(record.attempts[0].status, ProcessingStatus::Pending);
    }
}

#[test]
fn system_event_colors_reject_unknown_or_non_string_values() {
    for color in [json!("critical"), json!("Info"), json!("cherry"), json!(1)] {
        let mut value = serde_json::to_value(event("invalid")).unwrap();
        value["profile"] = json!("system_event");
        value["data"] = json!({
            "resource": "ci", "signal": "build", "severity": "error",
            "description": "Build observation", "severity_color": color
        });
        assert!(serde_json::from_value::<Event>(value).is_err());
    }
}

#[test]
fn canonical_system_payloads_omit_missing_optionals_and_redeliver_after_restart() {
    const CANONICAL: &str = r#"{"schema_version":1,"event_id":"legacy","stream":"samples","type":"sample.system.observed","occurred_at":null,"subject":null,"summary":"Build observation","body":null,"url":null,"people":[],"attachments":[],"relations":[],"context":null,"profile":"system_event","data":{"resource":"ci","signal":"build","severity":"error","description":"Build observation"},"metadata":{"source":"fixture"}}"#;
    let (_home, config, store, token) = setup();
    let connection = store.connection().unwrap();
    for severity in ["info", "warning", "error", "critical"] {
        let canonical = CANONICAL
            .replace("\"legacy\"", &format!("\"{severity}\""))
            .replace(
                "\"severity\":\"error\"",
                &format!("\"severity\":\"{severity}\""),
            );
        let event: Event = serde_json::from_str(&canonical).unwrap();
        assert_eq!(serde_json::to_string(&event).unwrap(), canonical);
        connection.execute(
            "INSERT INTO events(namespace, provider, event_id, payload, received_at) VALUES (?1, 'sample', ?2, ?3, '2026-10-02T00:00:00Z')",
            params![config.namespace, severity, canonical],
        ).unwrap();
        let sequence = connection.last_insert_rowid();
        connection.execute(
            "INSERT INTO event_attempts(event_sequence, status, replay, created_at) VALUES (?1, 'pending', 0, '2026-10-02T00:00:00Z')",
            [sequence],
        ).unwrap();
        let reopened = EventStore::open(&config).unwrap();
        let receipt = reopened
            .ingest(
                &token,
                Batch {
                    events: vec![event],
                },
            )
            .unwrap();
        assert!(receipt.receipts[0].duplicate);
        assert_eq!(receipt.receipts[0].sequence, sequence);
    }
    let snapshot = store.snapshot().unwrap();
    assert_eq!(snapshot.total, 4);
    assert!(store.notifications(&token).unwrap().is_empty());
    for record in snapshot.records {
        let Payload::SystemEvent(system) = record.event.payload else {
            panic!("expected system event")
        };
        assert_eq!(system.severity, record.event.event_id);
        assert_eq!(system.environment, None);
        assert_eq!(system.severity_color, None);
        assert_eq!(record.attempts.len(), 1);
        assert_eq!(record.attempts[0].status, ProcessingStatus::Pending);
    }
}
