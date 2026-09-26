use super::AppService;
use crate::store::opencode::{Client, Pane, Session, Snapshot};

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
            .close_instance_opencode("review")
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
    reader.settings.refresh_commands().unwrap();
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
    assert!(
        service
            .open_opencode("ses_external_detached", None)
            .unwrap_err()
            .contains("not owned")
    );
    let closing = service
        .close_opencode("ses_external_attached", pane)
        .unwrap();
    let _ = service.runtime.block_on(closing).unwrap();
    let navigation = service.open_opencode_client(client_pane.clone()).unwrap();
    let _ = service.runtime.block_on(navigation).unwrap();
    assert!(service.close_opencode_client(client_pane).is_ok());
}
