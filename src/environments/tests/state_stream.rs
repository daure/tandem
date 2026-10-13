use std::{collections::BTreeMap, fs};

use serde_json::{Value, json};

use crate::{
    environments::{config::Config, runtime_db, state_stream::Store},
    store::{
        opencode::{Pane, PromptOutcome, PromptedSession, Session},
        state_stream::{StateSnapshot, validate_origin},
    },
};

fn fixture() -> (tempfile::TempDir, Config, Store) {
    let home = tempfile::Builder::new()
        .prefix("tandem-state-")
        .tempdir_in("/tmp/opencode")
        .unwrap();
    let config = Config::at(home.path().to_owned(), "state-test".into(), 9876).unwrap();
    runtime_db::prepare(&config).unwrap();
    let store = Store::new(&config).unwrap();
    (home, config, store)
}

fn snapshot() -> StateSnapshot {
    let mut snapshot = StateSnapshot::empty();
    snapshot.instances["instances"] = json!([{
        "name": "review", "description": "Review a Slack message", "workspace_only": true,
        "summary": {"status": "running", "label": "Ready", "severity": "info", "busy": false}
    }]);
    snapshot.sessions["sessions"] = json!([session()]);
    snapshot
}

fn session() -> Value {
    json!({"session_id": "ses_test", "server": "http://127.0.0.1:4199", "directory": "/work/review", "instance": "review", "title": "Review", "activity": "idle", "attached": false, "panes": [], "stale": false, "availability": "observed", "origins": []})
}

fn receipt() -> PromptedSession {
    PromptedSession {
        session_id: Some("ses_test".into()),
        server: Some("http://127.0.0.1:4199".into()),
        pane: Pane {
            session: "main".into(),
            id: 100,
            tab_id: 9,
            tab_name: "Review".into(),
        },
        prompt_outcome: PromptOutcome::Submitted,
        error: None,
        creation_confirmed_absent: false,
    }
}

#[test]
fn snapshots_and_replay_cover_all_entity_events_with_full_objects() {
    let (_home, _config, store) = fixture();
    let initial = store.boundary(None).unwrap();
    assert!(initial.reset);
    assert_eq!(initial.snapshot.instances["instances"], json!([]));
    let mut state = snapshot();
    store.publish(state.clone()).unwrap();
    let created = store.boundary(Some(&initial.cursor)).unwrap();
    assert!(!created.reset);
    assert_eq!(
        created
            .events
            .iter()
            .map(|event| event["type"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["instance.changed", "session.created"]
    );
    assert_eq!(created.events[0]["data"], state.instances["instances"][0]);
    assert_eq!(created.events[1]["data"], state.sessions["sessions"][0]);
    for event in &created.events {
        assert_eq!(event["version"], 1);
        assert!(event["at"].as_u64().is_some());
    }
    state.instances["instances"][0]["description"] = json!("A new description");
    state.sessions["sessions"][0]["title"] = json!("Renamed");
    state.sessions["sessions"][0]["activity"] = json!("busy");
    state.sessions["sessions"][0]["attached"] = json!(true);
    state.sessions["sessions"][0]["panes"] =
        json!([{"session":"main","id":101,"tab_id":10,"tab_name":"New tab"}]);
    store.publish(state.clone()).unwrap();
    let changed = store.boundary(Some(&created.cursor)).unwrap();
    assert_eq!(changed.events[0]["type"], "instance.changed");
    assert_eq!(changed.events[1]["type"], "session.changed");
    assert_eq!(changed.events[1]["data"], state.sessions["sessions"][0]);
    let connected = store.boundary(None).unwrap();
    assert_eq!(connected.cursor, changed.cursor);
    assert_eq!(connected.snapshot.sessions, state.sessions);
    store.publish(StateSnapshot::empty()).unwrap();
    let removed = store.boundary(Some(&changed.cursor)).unwrap();
    assert_eq!(removed.events[0]["type"], "instance.removed");
    assert_eq!(removed.events[1]["type"], "session.removed");
    assert_eq!(removed.events[1]["data"], state.sessions["sessions"][0]);
}

#[test]
fn origins_bind_before_publication_and_survive_restarts_and_later_prompts() {
    let (_home, config, store) = fixture();
    let initial = store.boundary(None).unwrap().cursor;
    let request = store
        .begin_creation("/work/review", "review", Some("slack://channel?message=1"))
        .unwrap();
    store.publish(snapshot()).unwrap();
    assert_eq!(
        store.boundary(None).unwrap().snapshot.sessions["sessions"],
        json!([])
    );
    store
        .finish_creation(&request, "review", &receipt())
        .unwrap();
    let created = store.boundary(Some(&initial)).unwrap();
    let event = created
        .events
        .iter()
        .find(|event| event["type"] == "session.created")
        .unwrap();
    assert_eq!(
        event["data"]["origins"],
        json!(["slack://channel?message=1"])
    );
    let target = Session {
        id: "ses_test".into(),
        server: "http://127.0.0.1:4199".into(),
        directory: "/work/review".into(),
        ..Default::default()
    };
    store
        .associate_origin(&target, "review", Some("slack://channel?message=2"))
        .unwrap();
    store
        .associate_origin(&target, "review", Some("slack://channel?message=2"))
        .unwrap();
    drop(store);
    let restarted = Store::new(&config).unwrap();
    restarted.publish(snapshot()).unwrap();
    assert_eq!(
        restarted.boundary(None).unwrap().snapshot.sessions["sessions"][0]["origins"],
        json!(["slack://channel?message=1", "slack://channel?message=2"])
    );
    assert_eq!(restarted.retained_sessions().unwrap().len(), 1);
    assert!(
        restarted
            .boundary(Some(&initial))
            .unwrap()
            .events
            .iter()
            .any(|event| event["type"] == "session.changed")
    );
}

#[test]
fn companion_identity_handoff_recovers_creation_without_claiming_prompt_delivery() {
    let (_home, config, store) = fixture();
    let request = store
        .begin_creation("/work/review", "review", Some("correlation"))
        .unwrap();
    fs::write(store.identity_receipt(&request), json!({"request_id": request, "directory": "/work/review", "session_id": "ses_test", "server": "http://127.0.0.1:4199"}).to_string()).unwrap();
    drop(store);
    let restarted = Store::new(&config).unwrap();
    restarted.recover_identities().unwrap();
    restarted.publish(snapshot()).unwrap();
    let mut uncertain = receipt();
    uncertain.session_id = None;
    uncertain.prompt_outcome = PromptOutcome::Uncertain;
    uncertain.error = Some("response lost".into());
    let recovered = restarted
        .complete_creation(&request, "review", uncertain)
        .unwrap();
    assert_eq!(recovered.session_id.as_deref(), Some("ses_test"));
    assert_eq!(recovered.prompt_outcome, PromptOutcome::Uncertain);
    assert_eq!(recovered.error.as_deref(), Some("response lost"));
    assert_eq!(
        restarted.boundary(None).unwrap().snapshot.sessions["sessions"][0]["origins"],
        json!(["correlation"])
    );
}

#[test]
fn followup_origins_recover_pending_identity_before_first_publication() {
    let (_home, config, store) = fixture();
    let cursor = store.boundary(None).unwrap().cursor;
    let request = store
        .begin_creation("/work/review", "review", Some("initial"))
        .unwrap();
    fs::write(store.identity_receipt(&request), json!({"request_id": request, "directory": "/work/review", "session_id": "ses_test", "server": "http://127.0.0.1:4199"}).to_string()).unwrap();
    drop(store);
    let restarted = Store::new(&config).unwrap();
    restarted
        .associate_origin(
            &Session {
                id: "ses_test".into(),
                server: "http://127.0.0.1:4199".into(),
                directory: "/work/review".into(),
                ..Default::default()
            },
            "review",
            Some("followup"),
        )
        .unwrap();
    let events = restarted.boundary(Some(&cursor)).unwrap().events;
    assert_eq!(events[0]["type"], "session.created");
    assert_eq!(events[0]["data"]["origins"], json!(["initial"]));
    assert_eq!(events[1]["type"], "session.changed");
    assert_eq!(events[1]["data"]["origins"], json!(["initial", "followup"]));
}

#[test]
fn followup_origins_cannot_bypass_unresolved_creation() {
    let (_home, _config, store) = fixture();
    store
        .begin_creation("/work/review", "review", Some("initial"))
        .unwrap();
    let error = store
        .associate_origin(
            &Session {
                id: "ses_test".into(),
                server: "http://127.0.0.1:4199".into(),
                directory: "/work/review".into(),
                ..Default::default()
            },
            "review",
            Some("followup"),
        )
        .unwrap_err();
    assert!(error.contains("creation is unresolved"));
    assert_eq!(
        store.boundary(None).unwrap().snapshot.sessions["sessions"],
        json!([])
    );
    assert!(store.origins().unwrap().is_empty());
}

#[test]
fn persistence_failure_preserves_known_identity_and_delivery_outcome() {
    let (_home, _config, store) = fixture();
    let request = store
        .begin_creation("/work/review", "review", Some("initial"))
        .unwrap();
    fs::remove_file(store.identity_receipt(&request)).unwrap();
    fs::create_dir(store.identity_receipt(&request)).unwrap();
    let outcome = store
        .complete_creation(&request, "review", receipt())
        .unwrap();
    assert_eq!(outcome.session_id.as_deref(), Some("ses_test"));
    assert_eq!(outcome.server.as_deref(), Some("http://127.0.0.1:4199"));
    assert_eq!(outcome.prompt_outcome, PromptOutcome::Submitted);
    assert!(
        outcome
            .error
            .unwrap()
            .contains("correlation persistence failed")
    );
}

#[test]
fn confirmed_deletion_removes_origins_and_blocks_stale_observation_revival() {
    let (_home, config, store) = fixture();
    let request = store
        .begin_creation("/work/review", "review", Some("correlation"))
        .unwrap();
    store
        .finish_creation(&request, "review", &receipt())
        .unwrap();
    store.publish(snapshot()).unwrap();
    store
        .apply_session_edits(&BTreeMap::from([(
            ("http://127.0.0.1:4199".into(), "ses_test".into()),
            None,
        )]))
        .unwrap();
    let delayed = Store::new(&config).unwrap();
    delayed
        .apply_session_edits(&BTreeMap::from([(
            ("http://127.0.0.1:4199".into(), "ses_test".into()),
            Some("Delayed rename".into()),
        )]))
        .unwrap();
    store.publish(snapshot()).unwrap();
    assert_eq!(
        store.boundary(None).unwrap().snapshot.sessions["sessions"],
        json!([])
    );
    assert!(store.origins().unwrap().is_empty());
    assert!(store.retained_sessions().unwrap().is_empty());
}

#[test]
fn unchanged_observations_and_timestamps_do_not_append_events() {
    let (_home, _config, store) = fixture();
    let mut state = snapshot();
    store.publish(state.clone()).unwrap();
    let head = store.boundary(None).unwrap().cursor;
    state.instances["observed_at_unix_seconds"] = json!(123);
    state.sessions["observed_at_unix_seconds"] = json!(123);
    store.publish(state).unwrap();
    let replay = store.boundary(Some(&head)).unwrap();
    assert_eq!(replay.cursor, head);
    assert!(replay.events.is_empty());
}

#[test]
fn successful_inventory_reconciles_instances_retained_during_dependency_failure() {
    use crate::store::environments::{Instance, RuntimeInventory};
    let (_home, config, store) = fixture();
    let mut state = StateSnapshot::empty();
    state.instances["instances"] = json!([Instance {
        name: "review".into(),
        ..Default::default()
    }]);
    store.publish(state).unwrap();
    drop(store);
    let restarted = Store::new(&config).unwrap();
    let stale = restarted
        .preserve_failed_inventory(RuntimeInventory {
            runtime_error: Some("Docker unavailable".into()),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(stale.instances.len(), 1);
    assert!(stale.instances[0].runtime.stale);
    let mut state = StateSnapshot::empty();
    state.instances = serde_json::to_value(stale).unwrap();
    restarted.publish(state).unwrap();
    let cursor = restarted.boundary(None).unwrap().cursor;
    let fresh = restarted
        .preserve_failed_inventory(RuntimeInventory::default())
        .unwrap();
    assert!(fresh.instances.is_empty());
    let mut state = StateSnapshot::empty();
    state.instances = serde_json::to_value(fresh).unwrap();
    restarted.publish(state).unwrap();
    assert!(
        restarted
            .boundary(Some(&cursor))
            .unwrap()
            .events
            .iter()
            .any(|event| event["type"] == "instance.removed")
    );
}

#[test]
fn replay_pruning_preserves_origins_and_resets_expired_or_invalid_cursors() {
    let (_home, config, store) = fixture();
    let initial = store.boundary(None).unwrap().cursor;
    let request = store
        .begin_creation("/work/review", "review", Some("correlation"))
        .unwrap();
    store
        .finish_creation(&request, "review", &receipt())
        .unwrap();
    let mut state = snapshot();
    for index in 0..4100 {
        state.sessions["sessions"][0]["title"] = json!(format!("Title {index}"));
        store.publish(state.clone()).unwrap();
    }
    assert!(store.boundary(Some(&initial)).unwrap().reset);
    assert!(store.boundary(Some("another-epoch:1")).unwrap().reset);
    assert!(store.boundary(Some("malformed")).unwrap().reset);
    let head = store.boundary(None).unwrap().cursor;
    let epoch = head.rsplit_once(':').unwrap().0;
    assert!(
        store
            .boundary(Some(&format!("{epoch}:9999999")))
            .unwrap()
            .reset
    );
    drop(store);
    let restarted = Store::new(&config).unwrap();
    assert!(!restarted.boundary(Some(&head)).unwrap().reset);
    assert_eq!(
        restarted.origins().unwrap().values().next().unwrap(),
        &["correlation"]
    );
}

#[test]
fn origins_preserve_opaque_unicode_and_json_framing() {
    assert!(validate_origin(Some(&"🛡".repeat(512))).is_ok());
    assert!(validate_origin(Some(&"🛡".repeat(513))).is_err());
    assert!(validate_origin(Some("")).is_ok());
    let (_home, _config, store) = fixture();
    let origin = "  opaque\n日本語  ";
    let request = store
        .begin_creation("/work/review", "review", Some(origin))
        .unwrap();
    store
        .finish_creation(&request, "review", &receipt())
        .unwrap();
    assert_eq!(
        store.boundary(None).unwrap().snapshot.sessions["sessions"][0]["origins"],
        json!([origin])
    );
}

#[test]
fn deleted_sessions_reject_delayed_identity_and_origin_bindings() {
    let (_home, _config, store) = fixture();
    let request = store
        .begin_creation("/work/review", "review", Some("creation"))
        .unwrap();
    let edits = BTreeMap::from([(("http://127.0.0.1:4199".into(), "ses_test".into()), None)]);
    store.apply_session_edits(&edits).unwrap();
    store
        .finish_creation(&request, "review", &receipt())
        .unwrap();
    let target = Session {
        id: "ses_test".into(),
        server: "http://127.0.0.1:4199".into(),
        directory: "/work/review".into(),
        ..Default::default()
    };
    store
        .associate_origin(&target, "review", Some("later prompt"))
        .unwrap();
    assert!(store.origins().unwrap().is_empty());
    assert_eq!(
        store.boundary(None).unwrap().snapshot.sessions["sessions"],
        json!([])
    );
}

#[test]
fn confirmed_non_creation_releases_a_directory_gate() {
    let (_home, _config, store) = fixture();
    let request = store
        .begin_creation("/work/review", "review", Some("creation"))
        .unwrap();
    let mut rejected = receipt();
    rejected.session_id = None;
    rejected.creation_confirmed_absent = true;
    rejected.prompt_outcome = PromptOutcome::NotSubmitted;
    store
        .finish_creation(&request, "review", &rejected)
        .unwrap();
    store.publish(snapshot()).unwrap();
    assert_eq!(
        store.boundary(None).unwrap().snapshot.sessions["sessions"][0]["session_id"],
        "ses_test"
    );
    assert!(store.origins().unwrap().is_empty());
}

#[test]
fn confirmed_renames_allow_later_native_title_changes() {
    let (_home, _config, store) = fixture();
    let mut state = snapshot();
    store.publish(state.clone()).unwrap();
    store
        .apply_session_edits(&BTreeMap::from([(
            ("http://127.0.0.1:4199".into(), "ses_test".into()),
            Some("Tandem title".into()),
        )]))
        .unwrap();
    store.publish(state.clone()).unwrap();
    assert_eq!(
        store.boundary(None).unwrap().snapshot.sessions["sessions"][0]["title"],
        "Tandem title"
    );
    state.sessions["sessions"][0]["title"] = json!("Tandem title");
    store.publish(state.clone()).unwrap();
    state.sessions["sessions"][0]["title"] = json!("Native title");
    store.publish(state).unwrap();
    assert_eq!(
        store.boundary(None).unwrap().snapshot.sessions["sessions"][0]["title"],
        "Native title"
    );
}

#[test]
fn uncertain_creation_preserves_missing_sessions_only_as_unverified_history() {
    let (_home, _config, store) = fixture();
    let mut state = snapshot();
    state.sessions["sessions"][0]["activity"] = json!("busy");
    state.sessions["sessions"][0]["attached"] = json!(true);
    state.sessions["sessions"][0]["panes"] = json!([{"id":100}]);
    store.publish(state).unwrap();
    store
        .begin_creation("/work/review", "review", Some("uncertain"))
        .unwrap();
    store.publish(StateSnapshot::empty()).unwrap();
    let retained = &store.boundary(None).unwrap().snapshot.sessions["sessions"][0];
    assert_eq!(retained["session_id"], "ses_test");
    assert_eq!(retained["stale"], true);
    assert_eq!(retained["activity"], "unknown");
    assert_eq!(retained["availability"], "unverified");
    assert_eq!(retained["attached"], false);
    assert_eq!(retained["panes"], json!([]));
}
