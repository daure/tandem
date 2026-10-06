use super::*;
use crate::{
    environments::rules::RuleStore,
    store::{
        opencode,
        rules::{Definition, DispatchStatus},
    },
};

fn rule(name: &str) -> Definition {
    Definition {
        name: name.into(),
        description: String::new(),
        script: "fn matches(event) { event.event_id == \"accepted\" }".into(),
        template: "blank".into(),
        model: "openai/test".into(),
        variant: None,
        initial_prompt: "Inspect {{event.summary}}".into(),
        enabled: true,
        start_instance: false,
        focus_pane: true,
    }
}

#[test]
fn ignored_deletion_covers_retained_history_and_preserves_every_event_with_an_acceptance() {
    let (_home, mut config, events, token) = setup();
    let rules = RuleStore::open(&config).unwrap();
    rules.save(rule("inspect"), None, "main".into()).unwrap();
    let accepted = events
        .ingest(
            &token,
            Batch {
                events: vec![event("accepted")],
            },
        )
        .unwrap()
        .receipts[0]
        .sequence;
    rules.evaluate().unwrap();
    let mut acceptance = rules.snapshot().unwrap().acceptances.remove(0);
    acceptance.status = DispatchStatus::Provisioning;
    rules.update(&acceptance, false).unwrap();
    events.replay(accepted, "pending-replay").unwrap();
    for batch in 0..3 {
        events
            .ingest(
                &token,
                Batch {
                    events: (0..100)
                        .map(|index| event(&format!("ignored-{batch}-{index}")))
                        .collect(),
                },
            )
            .unwrap();
    }
    assert_eq!(events.snapshot().unwrap().total, 301);
    assert!(
        !events
            .snapshot()
            .unwrap()
            .records
            .iter()
            .any(|row| row.sequence == accepted)
    );
    config.namespace = "other".into();
    let other = EventStore::open(&config).unwrap();
    let other_token = other.register_provider("sample").unwrap();
    other
        .ingest(
            &other_token,
            Batch {
                events: vec![event("ignored")],
            },
        )
        .unwrap();

    assert_eq!(events.delete(Deletion::Ignored).unwrap(), 300);
    let snapshot = events.snapshot().unwrap();
    assert_eq!(snapshot.total, 1);
    assert_eq!(snapshot.records[0].sequence, accepted);
    assert_eq!(snapshot.records[0].acceptances, vec![acceptance]);
    assert_eq!(snapshot.records[0].attempts.len(), 2);
    assert_eq!(
        snapshot.records[0].attempts[0].status,
        ProcessingStatus::Pending
    );
    assert_eq!(events.notifications(&token).unwrap().len(), 2);
    assert_eq!(other.snapshot().unwrap().total, 1);
    assert_eq!(events.delete(Deletion::Ignored).unwrap(), 0);
    assert_eq!(rules.snapshot().unwrap().rules.len(), 1);
    let evaluations: i64 = events
        .connection()
        .unwrap()
        .query_row("SELECT count(*) FROM rule_evaluations", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(evaluations, 2);
}

#[test]
fn acceptance_deletion_preserves_its_event_siblings_resources_and_completed_evaluations() {
    let (_home, config, events, token) = setup();
    let rules = RuleStore::open(&config).unwrap();
    for name in ["first", "second"] {
        rules.save(rule(name), None, "main".into()).unwrap();
    }
    let sequence = events
        .ingest(
            &token,
            Batch {
                events: vec![event("accepted")],
            },
        )
        .unwrap()
        .receipts[0]
        .sequence;
    rules.evaluate().unwrap();
    let mut acceptances = rules.snapshot().unwrap().acceptances;
    for acceptance in &mut acceptances {
        acceptance.status = DispatchStatus::Launched;
        acceptance.operation_id = Some(format!("operation-{}", acceptance.id));
        rules.update(acceptance, true).unwrap();
    }
    let mut selected = acceptances.remove(0);
    let sibling = acceptances.remove(0);
    let directory = config.workspaces.join(&selected.instance);
    fs::create_dir_all(&directory).unwrap();
    let workspace = crate::store::rules::AcceptanceWorkspace {
        directory: directory.display().to_string(),
        sessions: vec![opencode::Session {
            id: "ses_retained".into(),
            directory: directory.display().to_string(),
            ..Default::default()
        }],
        ..Default::default()
    };
    events
        .connection()
        .unwrap()
        .execute(
            "INSERT INTO rule_acceptance_workspaces(acceptance_id, payload) VALUES (?1, ?2)",
            params![selected.id, serde_json::to_string(&workspace).unwrap()],
        )
        .unwrap();
    assert!(rules.admit_dispatch(selected.id).unwrap());
    let before = events.snapshot().unwrap();
    for status in [DispatchStatus::Provisioning, DispatchStatus::Launching] {
        selected.status = status;
        rules.update(&selected, false).unwrap();
        let current = events.snapshot().unwrap();
        assert!(matches!(
            events.delete(Deletion::Acceptance(selected.id)),
            Err(Error::Conflict(_))
        ));
        assert_eq!(events.snapshot().unwrap(), current);
        assert_eq!(events.notifications(&token).unwrap().len(), 3);
    }
    selected.status = DispatchStatus::Launched;
    rules.update(&selected, false).unwrap();
    let other_config = Config::at(config.home.clone(), "other".into(), 9876).unwrap();
    let other = EventStore::open(&other_config).unwrap();
    assert!(matches!(
        other.delete(Deletion::Acceptance(selected.id)),
        Err(Error::NotFound)
    ));

    assert_eq!(events.delete(Deletion::Acceptance(selected.id)).unwrap(), 1);
    assert!(matches!(
        events.delete(Deletion::Acceptance(selected.id)),
        Err(Error::NotFound)
    ));
    let record = events.record(sequence).unwrap();
    assert_eq!(record.acceptances, vec![sibling.clone()]);
    assert_eq!(record.attempts, before.records[0].attempts);
    assert_eq!(record.event, before.records[0].event);
    assert_eq!(events.snapshot().unwrap().total, 1);
    assert!(directory.is_dir());
    let notifications = events.notifications(&token).unwrap();
    assert_eq!(notifications.len(), 2);
    assert!(
        notifications
            .iter()
            .all(|notification| notification.dispatch_id != Some(selected.id))
    );
    assert!(
        notifications
            .iter()
            .any(|notification| notification.dispatch_id == Some(sibling.id))
    );
    let restarted = RuleStore::open(&config).unwrap();
    restarted.evaluate().unwrap();
    let snapshot = restarted.snapshot().unwrap();
    assert_eq!(snapshot.acceptances, vec![sibling]);
    assert!(snapshot.workspaces.is_empty());
    assert_eq!(snapshot.rules.len(), 2);
    let admissions: i64 = events
        .connection()
        .unwrap()
        .query_row("SELECT count(*) FROM rule_dispatch_starts", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(admissions, 1);
    events.authenticate(&token).unwrap();
}

#[test]
fn history_deletion_preserves_records_owned_by_an_incomplete_provider_deletion() {
    let (_home, config, events, token) = setup();
    let rules = RuleStore::open(&config).unwrap();
    rules.save(rule("inspect"), None, "main".into()).unwrap();
    events
        .ingest(
            &token,
            Batch {
                events: vec![event("accepted"), event("ignored")],
            },
        )
        .unwrap();
    rules.evaluate().unwrap();
    let acceptance = rules.snapshot().unwrap().acceptances.remove(0);
    events.begin_provider_deletion("package", "sample").unwrap();
    let before = events.snapshot().unwrap();
    for target in [Deletion::Ignored, Deletion::Acceptance(acceptance.id)] {
        assert!(
            events
                .delete(target)
                .unwrap_err()
                .to_string()
                .contains("incomplete provider deletion")
        );
        assert_eq!(events.snapshot().unwrap(), before);
        assert_eq!(events.notifications(&token).unwrap().len(), 2);
    }
}

#[test]
fn ignored_event_deletion_racing_evaluation_preserves_matches_and_cancels_deleted_work() {
    use std::sync::{Arc, Barrier};

    let (_home, config, events, token) = setup();
    let rules = RuleStore::open(&config).unwrap();
    let mut definition = rule("inspect");
    definition.script = "fn matches(event) { true }".into();
    rules.save(definition, None, "main".into()).unwrap();
    for index in 0..20 {
        let before = rules.snapshot().unwrap().acceptances.len();
        let sequence = events
            .ingest(
                &token,
                Batch {
                    events: vec![event(&format!("candidate-{index}"))],
                },
            )
            .unwrap()
            .receipts[0]
            .sequence;
        let barrier = Arc::new(Barrier::new(2));
        let worker_barrier = barrier.clone();
        let worker = rules.clone();
        let evaluation = std::thread::spawn(move || {
            worker_barrier.wait();
            worker.evaluate().unwrap();
        });
        barrier.wait();
        let deleted = events.delete(Deletion::Ignored).unwrap();
        evaluation.join().unwrap();
        assert!(matches!(deleted, 0 | 1));
        if deleted == 0 {
            assert_eq!(events.record(sequence).unwrap().acceptances.len(), 1);
        } else {
            assert!(matches!(events.record(sequence), Err(Error::NotFound)));
        }
        rules.evaluate().unwrap();
        assert_eq!(
            rules.snapshot().unwrap().acceptances.len(),
            before + usize::from(deleted == 0)
        );
    }
}
