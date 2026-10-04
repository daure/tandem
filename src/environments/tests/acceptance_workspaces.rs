use crate::environments::{config::Config, events::EventStore, rules::RuleStore};
use crate::store::{
    environments::{EnvironmentSnapshot, Instance},
    events::Batch,
    opencode::{Activity, Pane, Session, Snapshot},
};

#[test]
fn acceptance_history_survives_inventory_removal_and_restart_and_obeys_event_ownership() {
    let home = tempfile::tempdir().unwrap();
    let config = Config::at(home.path().into(), "history-test".into(), 9876).unwrap();
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
                initial_prompt: "Inspect {{event.summary}}".into(),
                enabled: true,
                start_instance: false,
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
    let acceptance = store.snapshot().unwrap().acceptances.remove(0);
    let directory = config
        .workspaces
        .join(&acceptance.instance)
        .display()
        .to_string();
    let inventory = EnvironmentSnapshot {
        instances: vec![Instance {
            name: acceptance.instance.clone(),
            template: "blank".into(),
            workspace: directory.clone(),
            ..Default::default()
        }],
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
    store
        .remember_workspaces(&EnvironmentSnapshot::default(), &Snapshot::default())
        .unwrap();
    let restarted = RuleStore::open(&config).unwrap();
    let history = restarted.snapshot().unwrap().workspaces;
    assert_eq!(history[&acceptance.id].directory, directory);
    assert_eq!(history[&acceptance.id].sessions.len(), 1);
    let session = &history[&acceptance.id].sessions[0];
    assert_eq!(session.id, "ses_retained");
    assert_eq!(session.activity, Activity::Idle);
    assert!(session.panes.is_empty());
    let other = Config::at(home.path().into(), "other".into(), 9876).unwrap();
    assert!(
        RuleStore::open(&other)
            .unwrap()
            .snapshot()
            .unwrap()
            .workspaces
            .is_empty()
    );
    events.delete(Some(acceptance.event_sequence)).unwrap();
    assert!(restarted.snapshot().unwrap().workspaces.is_empty());
}
