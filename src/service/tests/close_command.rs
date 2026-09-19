use super::AppService;

#[test]
fn close_command_survives_restart_and_failed_saves_preserve_the_previous_value() {
    let service = AppService::for_tests();
    service
        .runtime
        .block_on(service.configure_close_command("exit 23".into(), true))
        .unwrap();
    let restarted = AppService::from_config(service.environments.config.clone()).unwrap();
    assert_eq!(restarted.close_command(), "exit 23");
    service
        .runtime
        .block_on(service.configure_close_command("".into(), true))
        .unwrap();
    assert_eq!(
        restarted
            .runtime
            .block_on(restarted.get_close_command())
            .unwrap(),
        ""
    );
    let connection =
        rusqlite::Connection::open(service.environments.config.home.join("settings.sqlite3"))
            .unwrap();
    connection.execute_batch("CREATE TRIGGER reject_settings BEFORE INSERT ON app_settings BEGIN SELECT RAISE(FAIL, 'read only settings'); END;").unwrap();
    assert!(
        service
            .runtime
            .block_on(service.configure_close_command("true".into(), true))
            .unwrap_err()
            .contains("read only settings")
    );
    assert_eq!(service.close_command(), "");
}
