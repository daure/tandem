use crate::{
    environments::opencode::tests::history_server::Server,
    service::AppService,
    store::opencode::{Activity, Session, Snapshot},
};

#[test]
fn deleted_rule_session_links_are_retired_without_deleting_acceptance_history() {
    let mut service = AppService::for_tests();
    let config = service.config_for_tests();
    let observer = &mut std::sync::Arc::get_mut(&mut service.opencode)
        .unwrap()
        .observer;
    observer.presence = config.home.join("presence");
    observer.daemons = config.home.join("daemons");
    observer.zellij = config.home.join("zellij");
    let server = Server::start();
    server.data.lock().unwrap().v2 = true;
    let session = Session {
        id: "ses_old".into(),
        server: server.url.clone(),
        directory: config.workspaces.join("missing").display().to_string(),
        activity: Activity::Idle,
        ..Default::default()
    };
    let definition = serde_json::from_value(serde_json::json!({
        "name":"inspect", "script":"fn matches(event) { true }", "template":"blank",
        "model":"openai/test", "initial_prompt":"Inspect the event", "enabled":true
    }))
    .unwrap();
    service
        .rules
        .store
        .save(definition, None, "main".into())
        .unwrap();
    let events = crate::environments::events::EventStore::open(&config).unwrap();
    let token = events.register_provider("sample").unwrap();
    events
        .ingest(
            &token,
            crate::store::events::Batch {
                events: vec![crate::environments::events::tests::event("one")],
            },
        )
        .unwrap();
    service.rules.store.evaluate().unwrap();
    let acceptance = service
        .rules
        .store
        .snapshot()
        .unwrap()
        .acceptances
        .remove(0);
    let workspace = crate::store::rules::AcceptanceWorkspace {
        directory: session.directory.clone(),
        sessions: vec![session],
        ..Default::default()
    };
    let connection = rusqlite::Connection::open(config.home.join("settings.sqlite3")).unwrap();
    connection
        .execute(
            "INSERT INTO rule_acceptance_workspaces(acceptance_id, payload) VALUES (?1, ?2)",
            rusqlite::params![acceptance.id, serde_json::to_string(&workspace).unwrap()],
        )
        .unwrap();
    let listing = service
        .runtime
        .block_on(service.list_sessions(None, true))
        .unwrap();
    assert!(listing.sessions.is_empty());
    let history = service.rules.store.snapshot().unwrap();
    assert_eq!(history.acceptances.len(), 1);
    assert_eq!(history.acceptances[0].id, acceptance.id);
    assert!(history.workspaces[&acceptance.id].sessions.is_empty());
    assert!(service.rules.store.retained_sessions().unwrap().is_empty());
}

#[test]
fn listings_reconcile_retained_origins_without_a_workspace_or_client() {
    for v2 in [false, true] {
        for outcome in ["deleted", "present", "unavailable"] {
            let mut service = AppService::for_tests();
            let config = service.config_for_tests();
            let observer = &mut std::sync::Arc::get_mut(&mut service.opencode)
                .unwrap()
                .observer;
            observer.presence = config.home.join("presence");
            observer.daemons = config.home.join("daemons");
            observer.zellij = config.home.join("zellij");
            let server = Server::start();
            {
                let mut data = server.data.lock().unwrap();
                data.v2 = v2;
                data.status_failure = true;
                data.history_failure = true;
                data.session_failure = outcome == "unavailable";
            }
            let session = Session {
                id: "ses_old".into(),
                server: server.url.clone(),
                directory: config.workspaces.join("missing").display().to_string(),
                activity: Activity::Busy,
                ..Default::default()
            };
            if outcome == "present" {
                server.session(&session.id, &session.directory, None);
            }
            service
                .state_stream
                .associate_origin(&session, "missing", Some("slack://message/1"))
                .unwrap();
            let listing = service
                .runtime
                .block_on(service.list_sessions(None, true))
                .unwrap();
            assert!(listing.observation_error.is_some());
            if outcome == "deleted" {
                assert!(listing.sessions.is_empty(), "v2={v2}");
                assert!(service.state_stream.retained_sessions().unwrap().is_empty());
                service.set_opencode_snapshot_for_tests(Snapshot {
                    sessions: vec![session.clone()],
                    ..Default::default()
                });
                assert!(service.opencode_snapshot().sessions.is_empty());
                let restarted = AppService::from_config(config).unwrap();
                assert!(
                    restarted
                        .state_stream
                        .retained_sessions()
                        .unwrap()
                        .is_empty()
                );
            } else {
                assert_eq!(listing.sessions.len(), 1, "v2={v2}: {outcome}");
                assert_eq!(listing.sessions[0].session_id, "ses_old");
                assert_eq!(listing.sessions[0].activity, Activity::Unknown);
                assert!(listing.sessions[0].stale);
                assert_eq!(listing.sessions[0].origins, ["slack://message/1"]);
                assert_eq!(service.state_stream.retained_sessions().unwrap().len(), 1);
            }
            assert!(server.data.lock().unwrap().deleted.is_empty());
        }
    }
}
