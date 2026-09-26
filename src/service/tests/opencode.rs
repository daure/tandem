use super::AppService;

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
