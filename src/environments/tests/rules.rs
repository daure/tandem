use super::*;
use crate::store::events::{Batch, Deletion};

fn definition(name: &str, script: &str) -> Definition {
    Definition {
        name: name.into(),
        description: "Fixture".into(),
        script: script.into(),
        template: "blank".into(),
        model: "openai/test".into(),
        variant: None,
        initial_prompt: "Inspect {{event.data.text}}".into(),
        enabled: true,
        start_instance: true,
        focus_pane: true,
        throttle_seconds: 0,
        trigger_at_end: false,
    }
}

#[test]
fn dispatch_work_is_oldest_first_namespace_scoped_and_excludes_terminal_or_reported_history() {
    let home = tempfile::tempdir().unwrap();
    let config = Config::at(home.path().into(), "dispatch-query".into(), 9876).unwrap();
    let store = RuleStore::open(&config).unwrap();
    let events = EventStore::open(&config).unwrap();
    let token = events.register_provider("sample").unwrap();
    let rule = definition("inspect", "fn matches(event) { true }");
    store.save(rule.clone(), None, "main".into()).unwrap();
    let statuses = [
        DispatchStatus::Queued,
        DispatchStatus::Provisioning,
        DispatchStatus::Launching,
        DispatchStatus::Launched,
        DispatchStatus::Failed,
        DispatchStatus::Uncertain,
        DispatchStatus::Queued,
        DispatchStatus::Provisioning,
        DispatchStatus::Launching,
    ];
    events
        .ingest(
            &token,
            Batch {
                events: (0..statuses.len())
                    .map(|id| super::super::events::tests::event(&id.to_string()))
                    .collect(),
            },
        )
        .unwrap();
    store.evaluate().unwrap();
    let mut history = store.snapshot().unwrap().acceptances;
    history.reverse();
    for (index, (acceptance, status)) in history.iter_mut().zip(statuses).enumerate() {
        acceptance.status = status;
        store.update(acceptance, false).unwrap();
        if index >= 6 {
            store
                .save_report(
                    acceptance.id,
                    &crate::store::rules::reports::ReportInput {
                        title: "Completed inspection".into(),
                        summary: "Dispatch belongs to conclusion cleanup".into(),
                        markdown: "Inspection completed.".into(),
                    },
                )
                .unwrap();
        }
    }

    let other = Config::at(home.path().into(), "other".into(), 9876).unwrap();
    let foreign = RuleStore::open(&other).unwrap();
    let revision = foreign.snapshot().unwrap().rules[0].revision;
    foreign.save(rule, Some(revision), "main".into()).unwrap();
    let foreign_events = EventStore::open(&other).unwrap();
    let foreign_token = foreign_events.register_provider("sample").unwrap();
    foreign_events
        .ingest(
            &foreign_token,
            Batch {
                events: vec![super::super::events::tests::event("foreign")],
            },
        )
        .unwrap();
    foreign.evaluate().unwrap();
    assert_eq!(foreign.dispatch_acceptances().unwrap().len(), 1);
    assert_eq!(store.dispatch_acceptances().unwrap(), history[..3]);

    history[0].status = DispatchStatus::Failed;
    store.update(&history[0], false).unwrap();
    assert_eq!(store.dispatch_acceptances().unwrap(), history[1..3]);
    history[4].status = DispatchStatus::Queued;
    store.update(&history[4], false).unwrap();
    let restarted = RuleStore::open(&config).unwrap();
    assert_eq!(
        restarted.dispatch_acceptances().unwrap(),
        vec![history[1].clone(), history[2].clone(), history[4].clone()]
    );
    let snapshot = restarted.snapshot().unwrap();
    history.reverse();
    assert_eq!(snapshot.acceptances, history);
    assert_eq!(snapshot.reports.len(), 3);
}

#[test]
fn custom_assignments_survive_pending_work_replay_deletion_and_restart() {
    let home = tempfile::tempdir().unwrap();
    let config = Config::at(home.path().into(), "custom-names".into(), 9876).unwrap();
    let store = RuleStore::open(&config).unwrap();
    let events = EventStore::open(&config).unwrap();
    let token = events.register_provider("sample").unwrap();
    let rule = definition(
        "inspect",
        "fn matches(event) { true } fn instance_name(event) { \" PR / 82 \" } fn instance_description(event) { \" slack://channel?team=T1&id=C2 \" }",
    );
    store.save(rule.clone(), None, "main".into()).unwrap();
    let sequence = events
        .ingest(
            &token,
            Batch {
                events: vec![super::super::events::tests::event("one")],
            },
        )
        .unwrap()
        .receipts[0]
        .sequence;
    let record = store.event(sequence).unwrap();
    let input = rules::event_input(
        &record.event,
        &record.provider,
        sequence,
        &record.received_at,
    );
    let connection = events.connection().unwrap();
    let before: i64 = connection
        .query_row("PRAGMA data_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(
        store
            .preview_instance_identity(&rule, &input, sequence, &record.event.summary)
            .unwrap(),
        ("pr-82".into(), "slack://channel?team=T1&id=C2".into())
    );
    let after: i64 = connection
        .query_row("PRAGMA data_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(after, before);
    assert!(store.snapshot().unwrap().acceptances.is_empty());
    store.evaluate().unwrap();
    let first = store.snapshot().unwrap().acceptances.remove(0);
    assert_eq!(first.instance, "pr-82");
    assert_eq!(
        first.instance_description(),
        "slack://channel?team=T1&id=C2"
    );
    let mut changed = first.clone();
    changed.resolved_description = Some("changed".into());
    assert!(matches!(
        store.update(&changed, false),
        Err(Error::Conflict(_))
    ));
    assert_eq!(
        RuleStore::open(&config)
            .unwrap()
            .acceptance(first.id)
            .unwrap(),
        first
    );
    events.replay(sequence, "again").unwrap();
    store.evaluate().unwrap();
    assert_eq!(store.snapshot().unwrap().acceptances[0].instance, "pr-82-2");
    events.delete(Deletion::Event(sequence)).unwrap();
    let store = RuleStore::open(&config).unwrap();
    events
        .ingest(
            &token,
            Batch {
                events: vec![super::super::events::tests::event("two")],
            },
        )
        .unwrap();
    store.evaluate().unwrap();
    assert_eq!(store.snapshot().unwrap().acceptances[0].instance, "pr-82-3");
}

#[test]
fn preview_uses_durable_next_id_without_writing_or_reserving() {
    let home = tempfile::tempdir().unwrap();
    let config = Config::at(home.path().into(), "preview".into(), 9876).unwrap();
    let store = RuleStore::open(&config).unwrap();
    let events = EventStore::open(&config).unwrap();
    let connection = events.connection().unwrap();
    connection
        .execute(
            "INSERT INTO sqlite_sequence(name, seq) VALUES ('rule_acceptances', 99)",
            [],
        )
        .unwrap();
    let before: i64 = connection
        .query_row("PRAGMA data_version", [], |row| row.get(0))
        .unwrap();
    let rule = definition("inspect", "fn matches(event) { true }");
    let summary = "x".repeat(1200);
    let (name, description) = store
        .preview_instance_identity(&rule, &serde_json::json!({}), 42, &summary)
        .unwrap();
    assert_eq!(name, "inspect-42-a100");
    assert_eq!(description, format!("inspect: {summary}"));
    let after: i64 = connection
        .query_row("PRAGMA data_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(before, after);
    connection
        .execute(
            "UPDATE sqlite_sequence SET seq = ?1 WHERE name = 'rule_acceptances'",
            [i64::MAX],
        )
        .unwrap();
    assert!(
        store
            .preview_instance_identity(&rule, &serde_json::json!({}), 42, "summary")
            .is_err()
    );
    let count: i64 = connection
        .query_row("SELECT count(*) FROM rule_instance_names", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn exhaustion_and_filesystem_errors_leave_no_orphan_acceptance_or_reservation() {
    let home = tempfile::tempdir().unwrap();
    let config = Config::at(home.path().into(), "exhausted".into(), 9876).unwrap();
    let store = RuleStore::open(&config).unwrap();
    let events = EventStore::open(&config).unwrap();
    let token = events.register_provider("sample").unwrap();
    store
        .save(
            definition(
                "inspect",
                "fn matches(event) { true } fn instance_name(event) { \"full\" }",
            ),
            None,
            "main".into(),
        )
        .unwrap();
    events
        .ingest(
            &token,
            Batch {
                events: vec![super::super::events::tests::event("one")],
            },
        )
        .unwrap();
    let connection = events.connection().unwrap();
    for number in 1..=1024 {
        let name = if number == 1 {
            "full".into()
        } else {
            format!("full-{number}")
        };
        connection
            .execute(
                "INSERT INTO rule_instance_names VALUES (?1, 'foreign', NULL)",
                [name],
            )
            .unwrap();
    }
    store.evaluate().unwrap();
    let snapshot = store.snapshot().unwrap();
    assert!(snapshot.acceptances.is_empty());
    assert_eq!(
        snapshot.evaluation_errors[0].error,
        "instance name candidates exhausted (1024)"
    );
    assert_eq!(events.snapshot().unwrap().accepted_attempts, Some(0));
    connection
        .execute("DELETE FROM rule_instance_names", [])
        .unwrap();
    events
        .ingest(
            &token,
            Batch {
                events: vec![super::super::events::tests::event("two")],
            },
        )
        .unwrap();
    std::fs::remove_dir(&config.workspaces).unwrap();
    assert!(store.evaluate().is_err());
    assert!(store.snapshot().unwrap().acceptances.is_empty());
    std::fs::create_dir(&config.workspaces).unwrap();
    store.evaluate().unwrap();
    assert_eq!(store.snapshot().unwrap().acceptances[0].instance, "full");
    let count: i64 = connection
        .query_row("SELECT count(*) FROM rule_instance_names", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn legacy_descriptions_and_cross_namespace_acceptance_backfill_remain_frozen() {
    let home = tempfile::tempdir().unwrap();
    let config = Config::at(home.path().into(), "legacy-names".into(), 9876).unwrap();
    let store = RuleStore::open(&config).unwrap();
    let events = EventStore::open(&config).unwrap();
    let token = events.register_provider("sample").unwrap();
    store
        .save(
            definition("inspect", "fn matches(event) { true }"),
            None,
            "main".into(),
        )
        .unwrap();
    events
        .ingest(
            &token,
            Batch {
                events: vec![super::super::events::tests::event("one")],
            },
        )
        .unwrap();
    store.evaluate().unwrap();
    let acceptance = store.snapshot().unwrap().acceptances.remove(0);
    let connection = events.connection().unwrap();
    connection.execute_batch("UPDATE rule_acceptances SET payload = json_remove(payload, '$.resolved_description'); DELETE FROM rule_instance_names; DELETE FROM rule_instance_name_backfill;").unwrap();
    let mut legacy = store.acceptance(acceptance.id).unwrap();
    assert_eq!(legacy.resolved_description, None);
    assert_eq!(
        legacy.instance_description(),
        rules::default_instance_description(&legacy.rule_name, &legacy.event_summary)
    );
    legacy.resolved_description = Some(legacy.instance_description());
    assert!(store.update(&legacy, false).is_err());
    let other = Config::at(home.path().into(), "other".into(), 9876).unwrap();
    RuleStore::open(&other).unwrap();
    let namespace: String = connection
        .query_row(
            "SELECT namespace FROM rule_instance_names WHERE name_key = ?1",
            [&acceptance.instance],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(namespace, config.namespace);
    connection
        .execute_batch("DELETE FROM rule_acceptances;")
        .unwrap();
    RuleStore::open(&other).unwrap();
    assert_eq!(
        names::find_available_name(&connection, &other, &acceptance.instance).unwrap(),
        Some(format!("{}-2", acceptance.instance))
    );
}

#[test]
fn definition_upgrade_preserves_pinned_work_history_and_provider_metadata() {
    let home = tempfile::tempdir().unwrap();
    let config = Config::at(home.path().into(), "rule-upgrade".into(), 9876).unwrap();
    let store = RuleStore::open(&config).unwrap();
    let events = EventStore::open(&config).unwrap();
    let token = events.register_provider("sample").unwrap();
    let rule = store
        .save(
            definition(
                "inspect",
                "fn matches(event) { event.metadata.source == \"fixture\" }",
            ),
            None,
            "main".into(),
        )
        .unwrap();
    let event = super::super::events::tests::event("retained");
    let sequence = events
        .ingest(
            &token,
            Batch {
                events: vec![event.clone()],
            },
        )
        .unwrap()
        .receipts[0]
        .sequence;
    store.evaluate().unwrap();
    let mut acceptance = store.snapshot().unwrap().acceptances.remove(0);
    acceptance.status = DispatchStatus::Launched;
    acceptance.session_id = Some("retained-session".into());
    store.update(&acceptance, true).unwrap();
    let replay = events.replay(sequence, "pending-upgrade").unwrap();
    let history = events.snapshot().unwrap();
    let snapshot = store.snapshot().unwrap();
    events.connection().unwrap().execute_batch(
        "UPDATE event_rules SET definition = json_set(json_remove(definition, '$.start_instance', '$.focus_pane'), '$.metadata', json('{\"owner\":\"fixture\"}'));
         UPDATE rule_evaluations SET rule_snapshot = json_set(json_remove(rule_snapshot, '$.definition.start_instance', '$.definition.focus_pane'), '$.definition.metadata', json('{\"owner\":\"fixture\"}'));
         UPDATE rule_acceptances SET payload = json_set(json_remove(payload, '$.rule.definition.start_instance', '$.rule.definition.focus_pane'), '$.rule.definition.metadata', json('{\"owner\":\"fixture\"}'));"
    ).unwrap();

    events
        .connection()
        .unwrap()
        .execute_batch("ALTER TABLE event_rules DROP COLUMN catalog_present;")
        .unwrap();
    let migrated = RuleStore::open(&config).unwrap();
    assert_eq!(migrated.snapshot().unwrap(), snapshot);
    assert_eq!(events.snapshot().unwrap(), history);
    assert_eq!(events.snapshot().unwrap().records[0].event, event);
    migrated.evaluate().unwrap();
    let current = migrated.snapshot().unwrap();
    assert_eq!(current.rules, vec![rule]);
    assert_eq!(current.acceptances.len(), 2);
    assert!(current.acceptances.contains(&acceptance));
    assert_eq!(current.acceptances[0].attempt_id, replay);
    assert_eq!(
        RuleStore::open(&config).unwrap().snapshot().unwrap(),
        current
    );
}

#[test]
fn event_deletion_preserves_rules_and_rate_limits_and_protects_active_dispatches() {
    let home = tempfile::tempdir().unwrap();
    let config = Config::at(home.path().into(), "deletion-test".into(), 9876).unwrap();
    let store = RuleStore::open(&config).unwrap();
    let events = EventStore::open(&config).unwrap();
    let token = events.register_provider("sample").unwrap();
    let rule = store
        .save(
            definition("inspect", "fn matches(event) { true }"),
            None,
            "main".into(),
        )
        .unwrap();
    let sequence = events
        .ingest(
            &token,
            Batch {
                events: vec![super::super::events::tests::event("one")],
            },
        )
        .unwrap()
        .receipts[0]
        .sequence;
    store.evaluate().unwrap();
    let mut acceptance = store.snapshot().unwrap().acceptances.remove(0);
    acceptance.status = DispatchStatus::Launched;
    store.update(&acceptance, true).unwrap();
    for _ in 0..10 {
        assert!(store.admit_dispatch(acceptance.id).unwrap());
    }

    let connection = events.connection().unwrap();
    connection.execute_batch(
        "ALTER TABLE rule_dispatch_starts RENAME TO scoped_dispatch_starts;
         DROP INDEX rule_dispatch_rate;
         CREATE TABLE rule_dispatch_starts (
             id INTEGER PRIMARY KEY AUTOINCREMENT,
             acceptance_id INTEGER NOT NULL REFERENCES rule_acceptances(id),
             started_at TEXT NOT NULL
         );
         INSERT INTO rule_dispatch_starts SELECT id, acceptance_id, started_at FROM scoped_dispatch_starts;
         DROP TABLE scoped_dispatch_starts;
         CREATE INDEX rule_dispatch_rate ON rule_dispatch_starts(started_at);"
    ).unwrap();
    let store = RuleStore::open(&config).unwrap();
    assert_eq!(store.snapshot().unwrap().acceptances, vec![acceptance]);
    assert_eq!(events.delete(Deletion::Event(sequence)).unwrap(), 1);
    let current = store.snapshot().unwrap();
    assert_eq!(current.rules, vec![rule]);
    assert!(current.acceptances.is_empty());
    assert!(current.evaluation_errors.is_empty());
    assert!(events.notifications(&token).unwrap().is_empty());
    assert!(events.snapshot().unwrap().records.is_empty());
    let count: i64 = connection
        .query_row(
            "SELECT count(*) FROM rule_dispatch_starts WHERE namespace = ?1",
            [&config.namespace],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 10);

    events
        .ingest(
            &token,
            Batch {
                events: vec![super::super::events::tests::event("two")],
            },
        )
        .unwrap();
    store.evaluate().unwrap();
    let mut acceptance = store.snapshot().unwrap().acceptances.remove(0);
    assert!(!store.admit_dispatch(acceptance.id).unwrap());
    for status in [DispatchStatus::Provisioning, DispatchStatus::Launching] {
        acceptance.status = status;
        store.update(&acceptance, false).unwrap();
        let before = events.snapshot().unwrap();
        assert!(matches!(
            events.delete(Deletion::All),
            Err(Error::Conflict(_))
        ));
        assert_eq!(events.snapshot().unwrap(), before);
        assert_eq!(events.notifications(&token).unwrap().len(), 1);
    }
    acceptance.status = DispatchStatus::Failed;
    store.update(&acceptance, false).unwrap();
    assert_eq!(events.delete(Deletion::All).unwrap(), 1);
    assert!(store.snapshot().unwrap().acceptances.is_empty());
    assert_eq!(
        RuleStore::open(&config)
            .unwrap()
            .snapshot()
            .unwrap()
            .rules
            .len(),
        1
    );
}

#[test]
fn all_matching_rules_accept_independently_and_replay_keeps_failed_and_successful_history() {
    let home = tempfile::tempdir().unwrap();
    let config = Config::at(home.path().into(), "rule-tests".into(), 9876).unwrap();
    let store = RuleStore::open(&config).unwrap();
    let events = EventStore::open(&config).unwrap();
    let token = events.register_provider("sample").unwrap();
    for (name, script) in [
        (
            "first",
            "fn matches(event) { event.profile == \"message\" }",
        ),
        (
            "second",
            "fn matches(event) { event.metadata.source == \"fixture\" }",
        ),
        ("error", "fn matches(event) { event.missing.value == 1 }"),
        ("false", "fn matches(event) { false }"),
    ] {
        store
            .save(definition(name, script), None, "terminal".into())
            .unwrap();
    }
    let mut disabled = definition("disabled", "fn matches(event) { true }");
    disabled.enabled = false;
    store.save(disabled, None, "terminal".into()).unwrap();
    let event = super::super::events::tests::event("one");
    let sequence = events
        .ingest(
            &token,
            Batch {
                events: vec![event.clone()],
            },
        )
        .unwrap()
        .receipts[0]
        .sequence;
    store.evaluate().unwrap();
    let snapshot = store.snapshot().unwrap();
    assert_eq!(snapshot.acceptances.len(), 2);
    assert_eq!(snapshot.evaluation_errors.len(), 1);
    assert_ne!(
        snapshot.acceptances[0].instance,
        snapshot.acceptances[1].instance
    );
    for acceptance in &snapshot.acceptances {
        assert_eq!(
            acceptance.instance,
            format!("{}-{sequence}-a{}", acceptance.rule_name, acceptance.id)
        );
    }
    let mut successful = snapshot.acceptances[0].clone();
    successful.status = DispatchStatus::Launched;
    successful.session_id = Some("session-1".into());
    successful.operation_id = Some("operation-1".into());
    store.update(&successful, true).unwrap();
    store.update(&successful, true).unwrap();
    let mut failed = snapshot.acceptances[1].clone();
    failed.status = DispatchStatus::Failed;
    failed.error = Some("provisioning failed".into());
    store.update(&failed, false).unwrap();
    assert!(
        events
            .ingest(
                &token,
                Batch {
                    events: vec![event]
                }
            )
            .unwrap()
            .receipts[0]
            .duplicate
    );
    store.evaluate().unwrap();
    let snapshot = store.snapshot().unwrap();
    assert_eq!(snapshot.acceptances.len(), 2);
    assert_eq!(snapshot.acceptances[0].status, DispatchStatus::Launched);
    assert_eq!(snapshot.acceptances[1].status, DispatchStatus::Failed);
    let assigned: Vec<_> = events
        .notifications(&token)
        .unwrap()
        .into_iter()
        .filter(|notification| matches!(notification.kind, NotificationKind::Assigned))
        .collect();
    assert_eq!(assigned.len(), 1);
    assert_eq!(assigned[0].dispatch_id, Some(successful.id));
    assert_eq!(events.snapshot().unwrap().provider_handovers["sample"], 1);
    assert_eq!(events.snapshot().unwrap().accepted_attempts, Some(1));
    let attempt = events.replay(sequence, "intentional-replay").unwrap();
    assert_eq!(
        events.replay(sequence, "intentional-replay").unwrap(),
        attempt
    );
    store.evaluate().unwrap();
    let store = RuleStore::open(&config).unwrap();
    let snapshot = store.snapshot().unwrap();
    assert_eq!(snapshot.acceptances.len(), 4);
    for acceptance in &snapshot.acceptances {
        let expected = format!("{}-{sequence}-a{}", acceptance.rule_name, acceptance.id);
        assert_eq!(acceptance.instance, expected);
    }
    assert_eq!(
        snapshot
            .acceptances
            .iter()
            .filter(|row| row.attempt_id == attempt)
            .count(),
        2
    );
    assert_eq!(
        snapshot
            .acceptances
            .iter()
            .filter(|row| row.status == DispatchStatus::Launched)
            .count(),
        1
    );
    assert_eq!(store.event(sequence).unwrap().acceptances.len(), 4);
    assert_eq!(events.snapshot().unwrap().accepted_attempts, Some(2));
}

#[test]
fn truncated_rule_names_keep_distinct_instances_and_stable_assignments() {
    let home = tempfile::tempdir().unwrap();
    let config = Config::at(home.path().into(), "rule-names".into(), 9876).unwrap();
    let store = RuleStore::open(&config).unwrap();
    let events = EventStore::open(&config).unwrap();
    let token = events.register_provider("sample").unwrap();
    let prefix = "a".repeat(39);
    for tail in ["b", "c"] {
        store
            .save(
                definition(&format!("{prefix}{tail}"), "fn matches(event) { true }"),
                None,
                "main".into(),
            )
            .unwrap();
    }
    let sequence = events
        .ingest(
            &token,
            Batch {
                events: vec![super::super::events::tests::event("one")],
            },
        )
        .unwrap()
        .receipts[0]
        .sequence;
    store.evaluate().unwrap();
    let accepted = store.snapshot().unwrap().acceptances;
    assert_eq!(
        accepted[1].instance,
        format!("{}-{sequence}-a{}", "a".repeat(35), accepted[1].id)
    );
    assert_eq!(
        accepted[0].instance,
        format!("{}-{sequence}-a{}", "a".repeat(35), accepted[0].id)
    );
    for acceptance in &accepted {
        assert_eq!(acceptance.instance.len(), 40);
        crate::store::environments::validate_instance_name(&acceptance.instance).unwrap();
    }
    store.evaluate().unwrap();
    assert_eq!(
        RuleStore::open(&config)
            .unwrap()
            .snapshot()
            .unwrap()
            .acceptances,
        accepted
    );
}

#[test]
fn replay_of_unaccepted_events_uses_current_enabled_rules_once_per_attempt() {
    let home = tempfile::tempdir().unwrap();
    let config = Config::at(home.path().into(), "rule-replay".into(), 9876).unwrap();
    let store = RuleStore::open(&config).unwrap();
    let events = EventStore::open(&config).unwrap();
    let token = events.register_provider("sample").unwrap();
    let sequence = events
        .ingest(
            &token,
            Batch {
                events: vec![super::super::events::tests::event("before")],
            },
        )
        .unwrap()
        .receipts[0]
        .sequence;
    store.evaluate().unwrap();
    assert!(store.snapshot().unwrap().acceptances.is_empty());
    let empty_attempt = events.replay(sequence, "without-rules").unwrap();
    store.evaluate().unwrap();
    assert!(store.snapshot().unwrap().acceptances.is_empty());

    let mut draft = definition("match", "fn matches(event) { true }");
    draft.enabled = false;
    let disabled = store.save(draft, None, "terminal".into()).unwrap();
    let mut enabled = disabled.definition;
    enabled.enabled = true;
    let rule = store
        .save(enabled, Some(disabled.revision), "terminal".into())
        .unwrap();
    let first = events.replay(sequence, "first-match").unwrap();
    assert_eq!(events.replay(sequence, "first-match").unwrap(), first);
    let second = events.replay(sequence, "second-match").unwrap();
    assert_ne!(first, second);
    store.evaluate().unwrap();
    store.evaluate().unwrap();
    let record = store.event(sequence).unwrap();
    assert_eq!(record.attempts.len(), 4);
    assert_eq!(record.acceptances.len(), 2);
    assert_ne!(
        record.acceptances[0].instance,
        record.acceptances[1].instance
    );
    for attempt in [first, second] {
        let acceptance = record
            .acceptances
            .iter()
            .find(|acceptance| acceptance.attempt_id == attempt)
            .unwrap();
        assert_eq!(acceptance.rule_name, "match");
        assert_eq!(acceptance.rule_revision, rule.revision);
    }
    assert!(
        record
            .acceptances
            .iter()
            .all(|acceptance| acceptance.attempt_id != empty_attempt)
    );
    assert_eq!(
        events
            .notifications(&token)
            .unwrap()
            .iter()
            .filter(|notification| matches!(notification.kind, NotificationKind::Replayed))
            .count(),
        3
    );
}

#[test]
fn receipt_pins_rule_revisions_and_enablement_is_prospective() {
    let home = tempfile::tempdir().unwrap();
    let config = Config::at(home.path().into(), "rule-tests".into(), 9876).unwrap();
    let store = RuleStore::open(&config).unwrap();
    let events = EventStore::open(&config).unwrap();
    let token = events.register_provider("sample").unwrap();
    events
        .ingest(
            &token,
            Batch {
                events: vec![super::super::events::tests::event("before")],
            },
        )
        .unwrap();
    let rule = store
        .save(
            definition("match", "fn matches(event) { true }"),
            None,
            "terminal".into(),
        )
        .unwrap();
    events
        .ingest(
            &token,
            Batch {
                events: vec![super::super::events::tests::event("after")],
            },
        )
        .unwrap();
    let mut changed = rule.definition.clone();
    changed.initial_prompt = "An edited prompt".into();
    changed.focus_pane = false;
    changed.enabled = false;
    store
        .save(changed.clone(), Some(rule.revision), "terminal".into())
        .unwrap();
    assert!(matches!(
        store.save(changed, Some(rule.revision), "terminal".into()),
        Err(Error::Conflict(_))
    ));
    store.evaluate().unwrap();
    let snapshot = store.snapshot().unwrap();
    assert_eq!(snapshot.acceptances.len(), 1);
    assert_eq!(snapshot.acceptances[0].rule_revision, 1);
    assert!(snapshot.acceptances[0].rule.definition.focus_pane);
    assert!(!snapshot.rules[0].definition.focus_pane);
    assert_eq!(
        snapshot.acceptances[0].resolved_prompt.as_deref(),
        Some("Inspect Please inspect this event")
    );
    let first = store.lease().unwrap().unwrap();
    assert!(RuleStore::open(&config).unwrap().lease().unwrap().is_none());
    drop(first);
    assert!(store.lease().unwrap().is_some());
}

#[test]
fn dispatch_rate_counts_each_retry_and_keeps_namespace_budgets_independent() {
    let home = tempfile::tempdir().unwrap();
    for namespace in ["first", "second"] {
        let config = Config::at(home.path().into(), namespace.into(), 9876).unwrap();
        let store = RuleStore::open(&config).unwrap();
        let revision = store
            .snapshot()
            .unwrap()
            .rules
            .first()
            .map(|rule| rule.revision);
        store
            .save(
                definition("match", "fn matches(event) { true }"),
                revision,
                "main".into(),
            )
            .unwrap();
        let events = EventStore::open(&config).unwrap();
        let token = events.register_provider("sample").unwrap();
        events
            .ingest(
                &token,
                Batch {
                    events: vec![super::super::events::tests::event("one")],
                },
            )
            .unwrap();
        store.evaluate().unwrap();
        let id = store.snapshot().unwrap().acceptances[0].id;
        for _ in 0..10 {
            assert!(store.admit_dispatch(id).unwrap());
        }
        assert!(!store.admit_dispatch(id).unwrap());
    }
}
