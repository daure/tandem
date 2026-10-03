use super::*;
use crate::store::events::{Event, Payload};
use serde_json::json;

#[path = "event_severity.rs"]
mod event_severity;

pub(crate) fn event(id: &str) -> Event {
    serde_json::from_value(json!({
        "schema_version": 1, "event_id": id, "stream": "samples",
        "type": "sample.message.created", "summary": "A sample message",
        "profile": "message", "data": {"author": "Alex", "channel": "development", "text": "Please inspect this event", "thread": "thread-1"},
        "metadata": {"source": "fixture"}
    })).unwrap()
}

fn setup() -> (tempfile::TempDir, Config, EventStore, String) {
    let home = tempfile::tempdir().unwrap();
    let config = Config::at(home.path().into(), "events-test".into(), 9876).unwrap();
    let store = EventStore::open(&config).unwrap();
    let token = store.register_provider("sample").unwrap();
    (home, config, store, token)
}

#[test]
fn developer_credentials_are_private_namespace_scoped_files_outside_templates() {
    let (_home, mut config, store, _token) = setup();
    let path = store.setup_developer_credentials().unwrap();
    let token_path = path.parent().unwrap().join("dev-message.token");
    let token = fs::read_to_string(&token_path).unwrap();
    assert_eq!(token.len(), 64);
    assert!(!fs::read_to_string(&path).unwrap().contains(&token));
    assert!(!path.starts_with(&config.templates));
    assert_eq!(store.setup_developer_credentials().unwrap(), path);
    assert_eq!(fs::read_to_string(&token_path).unwrap(), token);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&token_path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(path.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
    }
    config.namespace = "other-namespace".into();
    let other = EventStore::open(&config).unwrap();
    assert_ne!(other.setup_developer_credentials().unwrap(), path);
    assert!(matches!(
        other.authenticate(&token),
        Err(Error::Unauthorized)
    ));
}

#[cfg(unix)]
#[test]
fn credential_setup_refuses_symlinked_directories_without_writing_tokens() {
    let (_home, config, store, _token) = setup();
    let outside = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(outside.path(), config.home.join("provider-credentials")).unwrap();
    assert!(matches!(
        store.setup_developer_credentials(),
        Err(Error::Storage(_))
    ));
    assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);
}

#[test]
fn batches_preserve_every_event_and_redeliveries_survive_restart() {
    let (_home, config, store, token) = setup();
    let batch = Batch {
        events: (0..10)
            .map(|index| event(&format!("event-{index}")))
            .collect(),
    };
    assert_eq!(store.ingest(&token, batch).unwrap().receipts.len(), 10);
    let store = EventStore::open(&config).unwrap();
    let receipt = store
        .ingest(
            &token,
            Batch {
                events: vec![event("event-0")],
            },
        )
        .unwrap();
    assert!(receipt.receipts[0].duplicate);
    let snapshot = store.snapshot().unwrap();
    assert_eq!(snapshot.total, 10);
    assert_eq!(snapshot.provider_totals.get("sample"), Some(&10));
    assert_eq!(snapshot.records.len(), 10);
    assert!(
        snapshot
            .records
            .iter()
            .all(|record| record.attempts.len() == 1
                && record.attempts[0].status == ProcessingStatus::Pending)
    );
    assert_eq!(store.notifications(&token).unwrap().len(), 10);
    assert_eq!(store.register_provider("sample").unwrap(), token);
}

#[test]
fn provider_totals_cover_retained_history_beyond_the_feed_and_keep_namespaces_separate() {
    let (_home, mut config, store, token) = setup();
    for batch in 0..3 {
        store
            .ingest(
                &token,
                Batch {
                    events: (0..100)
                        .map(|index| event(&format!("{batch}-{index}")))
                        .collect(),
                },
            )
            .unwrap();
    }
    let other = store.register_provider("other").unwrap();
    store
        .ingest(
            &other,
            Batch {
                events: vec![event("one")],
            },
        )
        .unwrap();
    config.namespace = "another-namespace".into();
    let foreign = EventStore::open(&config).unwrap();
    let foreign_token = foreign.register_provider("sample").unwrap();
    foreign
        .ingest(
            &foreign_token,
            Batch {
                events: vec![event("one")],
            },
        )
        .unwrap();
    let snapshot = store.snapshot().unwrap();
    assert_eq!(snapshot.records.len(), FEED_LIMIT);
    assert_eq!(snapshot.total, 301);
    assert_eq!(
        snapshot.provider_totals,
        [("sample".into(), 300), ("other".into(), 1)].into()
    );
    assert_eq!(
        foreign.snapshot().unwrap().provider_totals,
        [("sample".into(), 1)].into()
    );
}

#[test]
fn conflicting_or_invalid_events_roll_back_the_whole_batch() {
    let (_home, _config, store, token) = setup();
    store
        .ingest(
            &token,
            Batch {
                events: vec![event("known")],
            },
        )
        .unwrap();
    let mut conflicting = event("known");
    conflicting.summary = "Different content".into();
    assert!(matches!(
        store.ingest(
            &token,
            Batch {
                events: vec![event("new"), conflicting]
            }
        ),
        Err(Error::Conflict(_))
    ));
    let mut invalid = event("invalid");
    invalid.schema_version = 2;
    assert!(matches!(
        store.ingest(
            &token,
            Batch {
                events: vec![event("new"), invalid]
            }
        ),
        Err(Error::Invalid(_))
    ));
    assert_eq!(store.snapshot().unwrap().total, 1);
    assert_eq!(store.notifications(&token).unwrap().len(), 1);
}

#[test]
fn replay_preserves_accepted_history_and_retries_keep_the_same_attempt() {
    let (_home, config, store, token) = setup();
    let sequence = store
        .ingest(
            &token,
            Batch {
                events: vec![event("one")],
            },
        )
        .unwrap()
        .receipts[0]
        .sequence;
    let initial = store.snapshot().unwrap().records[0].attempts[0].id;
    assert!(matches!(
        store.replay(sequence, "premature"),
        Err(Error::Conflict(_))
    ));
    store
        .connection()
        .unwrap()
        .execute(
            "UPDATE event_attempts SET status = 'accepted', accepted_at = ?2 WHERE id = ?1",
            params![initial, now()],
        )
        .unwrap();
    let replay = store.replay(sequence, "replay-request").unwrap();
    assert_eq!(store.replay(sequence, "replay-request").unwrap(), replay);
    let store = EventStore::open(&config).unwrap();
    let snapshot = store.snapshot().unwrap();
    let attempts = &snapshot.records[0].attempts;
    assert_eq!(attempts.len(), 2);
    assert!(attempts[0].replay);
    assert_eq!(attempts[0].status, ProcessingStatus::Pending);
    assert_eq!(attempts[1].status, ProcessingStatus::Accepted);
    assert!(
        store
            .ingest(
                &token,
                Batch {
                    events: vec![event("one")]
                }
            )
            .unwrap()
            .receipts[0]
            .duplicate
    );
    assert_eq!(store.notifications(&token).unwrap().len(), 2);
}

#[test]
fn event_schema_upgrade_preserves_attempt_identities_timestamps_and_replay_receipts() {
    let home = tempfile::tempdir().unwrap();
    let config = Config::at(home.path().into(), "events-test".into(), 9876).unwrap();
    let connection = Connection::open(config.home.join("settings.sqlite3")).unwrap();
    connection
        .execute_batch(include_str!("../../../migrations/0006_events.sql"))
        .unwrap();
    let token = "a".repeat(64);
    connection
        .execute(
            "INSERT INTO event_providers(namespace, name, token) VALUES (?1, 'sample', ?2)",
            params![config.namespace, token],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO events(sequence, namespace, provider, event_id, payload, received_at)
         VALUES (7, ?1, 'sample', 'one', ?2, 'received')",
            params![
                config.namespace,
                serde_json::to_string(&event("one")).unwrap()
            ],
        )
        .unwrap();
    connection.execute_batch(
        "INSERT INTO event_attempts(id, event_sequence, status, replay, request_id, created_at, handled_at)
         VALUES (11, 7, 'handled', 0, NULL, 'created', 'processed'),
                (12, 7, 'pending', 1, 'existing-replay', 'replayed', NULL);
         UPDATE sqlite_sequence SET seq = 20 WHERE name = 'event_attempts';",
    ).unwrap();
    drop(connection);

    for _ in 0..2 {
        let store = EventStore::open(&config).unwrap();
        let record = store.snapshot().unwrap().records.remove(0);
        assert_eq!(record.sequence, 7);
        assert_eq!(record.event, event("one"));
        assert_eq!(record.attempts[0].id, 12);
        assert_eq!(record.attempts[0].status, ProcessingStatus::Pending);
        assert_eq!(record.attempts[0].accepted_at, None);
        assert_eq!(record.attempts[1].id, 11);
        assert_eq!(record.attempts[1].status, ProcessingStatus::Accepted);
        assert_eq!(record.attempts[1].created_at, "created");
        assert_eq!(record.attempts[1].accepted_at.as_deref(), Some("processed"));
        assert_eq!(store.replay(7, "existing-replay").unwrap(), 12);
        let serialized = serde_json::to_value(&record.attempts[1]).unwrap();
        assert_eq!(serialized["status"], "accepted");
        assert_eq!(serialized["accepted_at"], "processed");
    }

    let store = EventStore::open(&config).unwrap();
    assert!(
        store
            .ingest(
                &token,
                Batch {
                    events: vec![event("one")]
                }
            )
            .unwrap()
            .receipts[0]
            .duplicate
    );
    assert_eq!(store.snapshot().unwrap().records[0].attempts.len(), 2);
    assert!(
        matches!(store.replay(7, "pending-replay"), Err(Error::Conflict(message))
        if message == "the current attempt must be accepted before replaying")
    );
    store.connection().unwrap().execute(
        "UPDATE event_attempts SET status = 'accepted', accepted_at = 'processed' WHERE id = 12",
        [],
    ).unwrap();
    assert_eq!(store.replay(7, "next-replay").unwrap(), 21);
}

#[test]
fn credentials_scope_event_identity_and_feedback_to_one_provider_and_namespace() {
    let (_home, mut config, store, token) = setup();
    let other = store.register_provider("other").unwrap();
    store
        .ingest(
            &token,
            Batch {
                events: vec![event("one")],
            },
        )
        .unwrap();
    let notification = store.notifications(&token).unwrap()[0].notification_id;
    assert!(store.notifications(&other).unwrap().is_empty());
    assert!(matches!(
        store.acknowledge(&other, notification),
        Err(Error::NotFound)
    ));
    store.acknowledge(&token, notification).unwrap();
    store.acknowledge(&token, notification).unwrap();
    assert!(store.notifications(&token).unwrap().is_empty());
    store
        .ingest(
            &other,
            Batch {
                events: vec![event("one")],
            },
        )
        .unwrap();
    assert_eq!(store.snapshot().unwrap().total, 2);
    config.namespace = "different".into();
    let foreign = EventStore::open(&config).unwrap();
    assert!(matches!(
        foreign.ingest(
            &token,
            Batch {
                events: vec![event("one")]
            }
        ),
        Err(Error::Unauthorized)
    ));
    assert!(matches!(foreign.replay(1, "replay"), Err(Error::NotFound)));
    assert_eq!(foreign.snapshot().unwrap().total, 0);
}

#[test]
fn all_profiles_are_typed_and_event_content_is_bounded() {
    let (_home, _config, store, token) = setup();
    let mut sample = event("one");
    for (index, payload) in [
        json!({"profile": "message", "data": {"author": "Alex", "channel": "mail", "text": "Hello"}}),
        json!({"profile": "ticket", "data": {"key": "DEV-1", "title": "Build events", "status": "open"}}),
        json!({"profile": "system_event", "data": {"resource": "ci", "signal": "failed", "severity": "error", "description": "Build failed"}}),
        json!({"profile": "generic", "data": {"count": 1}}),
    ].into_iter().enumerate() {
        sample.event_id = format!("profile-{index}");
        sample.payload = serde_json::from_value(payload).unwrap();
        store.ingest(&token, Batch { events: vec![sample.clone()] }).unwrap();
    }
    for (index, environment) in [
        None,
        Some(String::new()),
        Some("  ".into()),
        Some("é".repeat(100)),
    ]
    .into_iter()
    .enumerate()
    {
        sample.event_id = format!("environment-{index}");
        sample.payload = serde_json::from_value(json!({
            "profile": "system_event",
            "data": {"resource": "ci", "signal": "failed", "severity": "é".repeat(40),
                     "description": "Build failed", "environment": environment}
        }))
        .unwrap();
        store
            .ingest(
                &token,
                Batch {
                    events: vec![sample.clone()],
                },
            )
            .unwrap();
    }
    let accepted = store.snapshot().unwrap().total;
    for (field, values, message) in [
        (
            "severity",
            [
                json!(""),
                json!("   "),
                json!("error\n"),
                json!("é".repeat(41)),
            ],
            "severity must be nonempty, at most 80 bytes, and contain no control characters",
        ),
        (
            "environment",
            [
                json!("é".repeat(101)),
                json!(" ".repeat(201)),
                json!("production\u{1b}"),
                json!("\t"),
            ],
            "environment must be at most 200 bytes and contain no control characters",
        ),
    ] {
        for value in values {
            let mut data = json!({"resource": "ci", "signal": "failed", "severity": "error", "description": "Build failed"});
            data[field] = value;
            sample.payload =
                serde_json::from_value(json!({"profile": "system_event", "data": data})).unwrap();
            assert!(matches!(
                store.ingest(&token, Batch { events: vec![sample.clone()] }),
                Err(Error::Invalid(error)) if error == message
            ));
        }
    }
    assert_eq!(store.snapshot().unwrap().total, accepted);
    assert_eq!(store.notifications(&token).unwrap().len() as u64, accepted);
    sample.payload = Payload::Generic(json!(null));
    assert!(matches!(sample.validate(), Err(Error::Invalid(_))));
    sample.payload = Payload::Generic(json!({"large": "a".repeat(65_536)}));
    assert!(matches!(sample.validate(), Err(Error::Invalid(_))));
    sample.event_id = "bad\u{1b}id".into();
    assert!(matches!(sample.validate(), Err(Error::Invalid(_))));
}

#[test]
fn disabled_ingestion_discards_batches_across_connections_and_preserves_feedback() {
    let (_home, mut config, store, token) = setup();
    store
        .ingest(
            &token,
            Batch {
                events: vec![event("retained")],
            },
        )
        .unwrap();
    let other = store.register_provider("other").unwrap();
    assert!(store.set_ingestion_enabled("sample", false).unwrap());
    let sidecar = EventStore::open(&config).unwrap();
    let discarded = sidecar
        .ingest(
            &token,
            Batch {
                events: vec![event("discarded-1"), event("discarded-2")],
            },
        )
        .unwrap();
    assert!(discarded.receipts.is_empty());
    assert_eq!(discarded.discarded, ["discarded-1", "discarded-2"]);
    assert_eq!(sidecar.snapshot().unwrap().total, 1);
    let feedback = sidecar.notifications(&token).unwrap();
    assert_eq!(feedback.len(), 1);
    sidecar
        .acknowledge(&token, feedback[0].notification_id)
        .unwrap();
    assert!(sidecar.notifications(&token).unwrap().is_empty());
    assert_eq!(
        sidecar
            .ingest(
                &other,
                Batch {
                    events: vec![event("other")]
                }
            )
            .unwrap()
            .receipts
            .len(),
        1
    );
    assert!(matches!(
        sidecar.ingest(
            "invalid",
            Batch {
                events: vec![event("unauthorized")]
            }
        ),
        Err(Error::Unauthorized)
    ));
    config.namespace = "another-namespace".into();
    let foreign = EventStore::open(&config).unwrap();
    let foreign_token = foreign.register_provider("sample").unwrap();
    assert_eq!(
        foreign
            .ingest(
                &foreign_token,
                Batch {
                    events: vec![event("foreign")]
                }
            )
            .unwrap()
            .receipts
            .len(),
        1
    );
    assert!(!store.set_ingestion_enabled("sample", true).unwrap());
    assert_eq!(
        sidecar
            .ingest(
                &token,
                Batch {
                    events: vec![event("resumed")]
                }
            )
            .unwrap()
            .receipts
            .len(),
        1
    );
    assert_eq!(sidecar.snapshot().unwrap().total, 3);
}
