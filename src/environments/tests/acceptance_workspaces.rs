use crate::environments::{config::Config, events::EventStore, rules::RuleStore};
use crate::store::{
    environments::EnvironmentSnapshot,
    events::{Batch, Deletion},
    opencode::{Activity, Pane, Session, Snapshot},
};

#[test]
fn acceptance_history_survives_inventory_removal_and_restart_and_obeys_event_ownership() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let service = crate::service::AppService::for_tests();
    let config = service.config_for_tests();
    runtime
        .block_on(service.set_clear_opencode_history(false).unwrap())
        .unwrap()
        .unwrap();
    runtime
        .block_on(service.create_template("blank".into()))
        .unwrap();
    let store = RuleStore::open(&config).unwrap();
    let events = EventStore::open(&config).unwrap();
    store
        .save(
            crate::store::rules::Definition {
                name: "inspect".into(),
                description: String::new(),
                script: "fn matches(event) { true }".into(),
                template: "blank".into(),
                model: "openai/test".into(),
                variant: None,
                initial_prompt: "Inspect {{event.summary}}".into(),
                enabled: true,
                start_instance: false,
                focus_pane: true,
                throttle_seconds: 0,
                trigger_at_end: false,
            },
            None,
            "main".into(),
        )
        .unwrap();
    let token = events.register_provider("sample").unwrap();
    events
        .ingest(
            &token,
            Batch {
                events: vec![crate::environments::events::tests::event("one")],
            },
        )
        .unwrap();
    store.evaluate().unwrap();
    let mut acceptance = store.snapshot().unwrap().acceptances.remove(0);
    let operation = service
        .submit_operation(
            "create_instance",
            &acceptance.instance,
            Some("blank".into()),
            60,
            true,
        )
        .unwrap();
    let completed = runtime
        .block_on(service.wait_operation(&operation.id))
        .unwrap();
    assert_eq!(
        completed.state,
        crate::store::environments::OperationState::Succeeded
    );
    acceptance.operation_id = Some(operation.id.clone());
    store.update(&acceptance, false).unwrap();
    let directory = config
        .workspaces
        .join(&acceptance.instance)
        .display()
        .to_string();
    let inventory = EnvironmentSnapshot {
        instances: vec![completed.instance.unwrap()],
        ..Default::default()
    };
    let observed = Snapshot {
        sessions: vec![
            Session {
                id: "ses_retained".into(),
                title: "Original task".into(),
                directory: directory.clone(),
                server: "http://127.0.0.1:12345".into(),
                activity: Activity::Busy,
                panes: vec![Pane {
                    session: "main".into(),
                    id: 2,
                    tab_id: 1,
                    tab_name: "Inspect".into(),
                }],
                ..Default::default()
            },
            Session {
                id: "ses_external".into(),
                directory: "/external".into(),
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    store.remember_workspaces(&inventory, &observed).unwrap();
    let mut unavailable = observed.clone();
    unavailable.sessions[0].server.clear();
    store.remember_workspaces(&inventory, &unavailable).unwrap();
    store
        .remember_workspaces(&EnvironmentSnapshot::default(), &Snapshot::default())
        .unwrap();
    let restarted = RuleStore::open(&config).unwrap();
    let history = restarted.snapshot().unwrap().workspaces;
    assert_eq!(history[&acceptance.id].directory, directory);
    assert_eq!(history[&acceptance.id].sessions.len(), 1);
    let session = &history[&acceptance.id].sessions[0];
    assert_eq!(session.id, "ses_retained");
    assert_eq!(session.server, "http://127.0.0.1:12345");
    assert_eq!(session.activity, Activity::Idle);
    assert!(session.panes.is_empty());
    assert_eq!(
        restarted.retained_sessions().unwrap(),
        vec![session.clone()]
    );
    let mut recent = observed.clone();
    let mut removed = recent.sessions[0].clone();
    removed.id = "ses_removed".into();
    recent.sessions.push(removed);
    restarted.remember_workspaces(&inventory, &recent).unwrap();
    restarted
        .update_opencode_history(
            &[
                (
                    ("http://127.0.0.1:12345".into(), "ses_retained".into()),
                    Some("Reviewed task".into()),
                ),
                (
                    ("http://127.0.0.1:12345".into(), "ses_removed".into()),
                    None,
                ),
            ]
            .into(),
        )
        .unwrap();
    let edited = RuleStore::open(&config).unwrap().snapshot().unwrap();
    assert_eq!(
        edited.workspaces[&acceptance.id].sessions[0].title,
        "Reviewed task"
    );
    assert_eq!(edited.workspaces[&acceptance.id].sessions.len(), 1);
    restarted.remember_workspaces(&inventory, &recent).unwrap();
    assert_eq!(
        restarted.snapshot().unwrap().workspaces[&acceptance.id]
            .sessions
            .len(),
        1
    );
    crate::environments::lifecycle::delete(
        &config,
        &acceptance.instance,
        std::sync::Arc::new(|_| {}),
        &|_, _| Ok(()),
    )
    .unwrap();
    let replacement = service
        .submit_operation(
            "create_instance",
            &acceptance.instance,
            Some("blank".into()),
            60,
            true,
        )
        .unwrap();
    let replacement = runtime
        .block_on(service.wait_operation(&replacement.id))
        .unwrap();
    let unrelated = Snapshot {
        sessions: vec![Session {
            id: "ses_unrelated".into(),
            directory: directory.clone(),
            ..Default::default()
        }],
        ..Default::default()
    };
    restarted
        .remember_workspaces(
            &EnvironmentSnapshot {
                instances: vec![replacement.instance.unwrap()],
                ..Default::default()
            },
            &unrelated,
        )
        .unwrap();
    assert_eq!(
        restarted.snapshot().unwrap().workspaces[&acceptance.id].sessions[0].id,
        "ses_retained"
    );
    assert_eq!(
        restarted.snapshot().unwrap().workspaces[&acceptance.id]
            .sessions
            .len(),
        1
    );
    let other = Config::at(config.home.clone(), "other".into(), 9876).unwrap();
    assert!(
        RuleStore::open(&other)
            .unwrap()
            .retained_sessions()
            .unwrap()
            .is_empty()
    );
    assert!(
        RuleStore::open(&other)
            .unwrap()
            .snapshot()
            .unwrap()
            .workspaces
            .is_empty()
    );
    events
        .delete(Deletion::Event(acceptance.event_sequence))
        .unwrap();
    assert!(restarted.snapshot().unwrap().workspaces.is_empty());
}
