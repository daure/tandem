use super::AppService;
use crate::store::opencode::{Client, CloseScope, Pane, Session, Snapshot};

#[test]
fn history_cleanup_defaults_on_and_persists_independently_of_integration() {
    let service = AppService::for_tests();
    assert!(service.clear_opencode_history());
    service
        .runtime
        .block_on(service.set_clear_opencode_history(false).unwrap())
        .unwrap()
        .unwrap();
    assert!(!service.clear_opencode_history());
    assert!(service.opencode_enabled());
    let reader = AppService::from_config(service.environments.config.clone()).unwrap();
    assert!(!reader.clear_opencode_history());
    service
        .runtime
        .block_on(service.set_clear_opencode_history(true).unwrap())
        .unwrap()
        .unwrap();
    reader.settings.refresh().unwrap();
    assert!(reader.clear_opencode_history());
}

#[test]
fn failed_history_setting_writes_preserve_the_saved_value() {
    let service = AppService::for_tests();
    let connection =
        rusqlite::Connection::open(service.environments.config.home.join("settings.sqlite3"))
            .unwrap();
    connection.execute_batch("CREATE TRIGGER reject_history BEFORE INSERT ON app_settings BEGIN SELECT RAISE(FAIL, 'read only settings'); END;").unwrap();
    let error = service
        .runtime
        .block_on(service.set_clear_opencode_history(false).unwrap())
        .unwrap()
        .unwrap_err();
    assert!(error.contains("read only settings"));
    assert!(service.clear_opencode_history());
}

#[test]
fn integration_defaults_on_persists_and_rejects_actions_when_disabled() {
    let service = AppService::for_tests();
    assert!(service.opencode_enabled());
    service
        .runtime
        .block_on(service.set_opencode_enabled(false).unwrap())
        .unwrap()
        .unwrap();
    assert!(!service.opencode_enabled());
    assert!(
        service
            .opencode_conversation("ses_one")
            .unwrap_err()
            .contains("disabled")
    );
    assert_eq!(service.opencode_snapshot(), Default::default());
    assert!(
        service
            .open_opencode("ses_one", None)
            .unwrap_err()
            .contains("disabled")
    );
    assert!(
        service
            .close_opencode(
                "ses_one",
                crate::store::opencode::Pane {
                    session: "main".into(),
                    id: 7,
                    tab_id: 4,
                    tab_name: "review".into(),
                },
            )
            .unwrap_err()
            .contains("disabled")
    );
    assert!(
        service
            .close_opencode_scope(&CloseScope::Instance("review".into()))
            .unwrap_err()
            .contains("disabled")
    );
    assert!(
        service
            .new_opencode_session("/work/ledger", None)
            .unwrap_err()
            .contains("disabled")
    );
    assert!(
        service
            .rename_opencode_session("ses_one", "Updated".into())
            .unwrap_err()
            .contains("disabled")
    );
    assert!(
        service
            .delete_opencode_session("ses_one")
            .unwrap_err()
            .contains("disabled")
    );
    let reader = AppService::from_config(service.environments.config.clone()).unwrap();
    assert!(!reader.opencode_enabled());
    service
        .runtime
        .block_on(service.set_opencode_enabled(true).unwrap())
        .unwrap()
        .unwrap();
    reader.settings.refresh().unwrap();
    assert!(reader.opencode_enabled());
}

#[test]
fn rejected_integration_setting_preserves_the_last_persisted_value() {
    let service = AppService::for_tests();
    let connection =
        rusqlite::Connection::open(service.environments.config.home.join("settings.sqlite3"))
            .unwrap();
    connection.execute_batch("CREATE TRIGGER reject_settings BEFORE INSERT ON app_settings BEGIN SELECT RAISE(FAIL, 'read only settings'); END;").unwrap();
    let error = service
        .runtime
        .block_on(service.set_opencode_enabled(false).unwrap())
        .unwrap()
        .unwrap_err();
    assert!(error.contains("read only settings"));
    assert!(service.opencode_enabled());
}

#[test]
fn external_sessions_allow_reading_navigation_and_closing_observed_panes() {
    let service = AppService::for_tests();
    let pane = Pane {
        session: "main".into(),
        id: 7,
        tab_id: 4,
        tab_name: "ledger".into(),
    };
    let client_pane = Pane {
        id: 8,
        ..pane.clone()
    };
    service.set_opencode_snapshot_for_tests(Snapshot {
        sessions: vec![
            Session {
                id: "ses_external_attached".into(),
                directory: "/work/ledger".into(),
                panes: vec![pane.clone()],
                ..Default::default()
            },
            Session {
                id: "ses_external_detached".into(),
                directory: "/work/ledger".into(),
                ..Default::default()
            },
        ],
        clients: vec![Client {
            title: "OpenCode".into(),
            directory: "/work/ledger".into(),
            server: "http://127.0.0.1:4199".into(),
            pane: client_pane.clone(),
            stale: false,
            awaiting_presence_since: None,
        }],
        ..Default::default()
    });

    assert!(
        service
            .opencode_conversation("ses_external_attached")
            .is_ok()
    );
    service.cancel_opencode_conversation();
    let navigation = service
        .open_opencode("ses_external_attached", Some(pane.clone()))
        .unwrap();
    let _ = service.runtime.block_on(navigation).unwrap();
    let navigation = service
        .open_opencode("ses_external_detached", None)
        .unwrap();
    let _ = service.runtime.block_on(navigation).unwrap();
    let closing = service
        .close_opencode("ses_external_attached", pane)
        .unwrap();
    let _ = service.runtime.block_on(closing).unwrap();
    let navigation = service.open_opencode_client(client_pane.clone()).unwrap();
    let _ = service.runtime.block_on(navigation).unwrap();
    assert!(service.close_opencode_client(client_pane).is_ok());
}

#[test]
fn bulk_close_scopes_include_all_observed_panes_and_respect_workspace_boundaries() {
    let service = AppService::for_tests();
    let operation = service.queue_instance_for_tests("review", "website");
    service.complete_instance_for_tests(
        &operation.id,
        crate::store::environments::Instance {
            name: "review".into(),
            template: "website".into(),
            workspace: "/work/owned".into(),
            ..Default::default()
        },
    );
    let pane = |id| Pane {
        session: "tandem-test-nonexistent".into(),
        id,
        tab_id: 1,
        tab_name: "test".into(),
    };
    let mut sessions = (0..25)
        .map(|id| Session {
            id: format!("ses_{id}"),
            directory: "/work/external".into(),
            panes: vec![pane(id)],
            ..Default::default()
        })
        .collect::<Vec<_>>();
    for (id, directory) in [
        (25, "/work/external/sub"),
        (26, "/work/owned/repo"),
        (27, "/work/owned-other"),
    ] {
        sessions.push(Session {
            id: format!("ses_{id}"),
            directory: directory.into(),
            panes: vec![pane(id)],
            ..Default::default()
        });
    }
    service.set_opencode_snapshot_for_tests(Snapshot {
        sessions,
        clients: vec![
            Client {
                title: "OpenCode".into(),
                directory: "/work/external".into(),
                server: String::new(),
                pane: pane(0),
                stale: false,
                awaiting_presence_since: None,
            },
            Client {
                title: "OpenCode".into(),
                directory: "/work/external".into(),
                server: String::new(),
                pane: pane(28),
                stale: false,
                awaiting_presence_since: None,
            },
        ],
        ..Default::default()
    });
    for (scope, expected) in [
        (
            CloseScope::Directory("/work/external".into()),
            (0..25).chain([28]).collect::<Vec<_>>(),
        ),
        (CloseScope::Instance("review".into()), vec![26]),
        (
            CloseScope::ExternalWorkspaces,
            (0..26).chain([27, 28]).collect(),
        ),
    ] {
        let outcome = service.close_opencode_scope(&scope).unwrap();
        assert_eq!(
            outcome.panes.iter().map(|pane| pane.id).collect::<Vec<_>>(),
            expected
        );
        let _ = service.runtime.block_on(outcome.reply).unwrap();
    }
}

#[test]
fn new_sessions_require_a_known_workspace_and_a_pane_from_that_workspace() {
    let service = AppService::for_tests();
    assert!(
        service
            .new_opencode_session("/work/unknown", None)
            .unwrap_err()
            .contains("workspace is unavailable")
    );
    let pane = Pane {
        session: "tandem-test-nonexistent".into(),
        id: 7,
        tab_id: 1,
        tab_name: "test".into(),
    };
    service.set_opencode_snapshot_for_tests(Snapshot {
        sessions: vec![
            Session {
                id: "ses_one".into(),
                directory: "/work/one".into(),
                ..Default::default()
            },
            Session {
                id: "ses_two".into(),
                directory: "/work/two".into(),
                panes: vec![pane.clone()],
                ..Default::default()
            },
        ],
        ..Default::default()
    });
    assert!(
        service
            .new_opencode_session("/work/one", Some(pane))
            .unwrap_err()
            .contains("pane is unavailable")
    );
}
