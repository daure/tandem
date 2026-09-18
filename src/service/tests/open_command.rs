use super::AppService;

fn register_workspace(service: &AppService, workspace: &std::path::Path) {
    let operation = service.queue_instance_for_tests("review", "website");
    service.complete_instance_for_tests(
        &operation.id,
        crate::store::environments::Instance {
            name: "review".into(),
            template: "website".into(),
            template_directory: service
                .environments
                .config
                .templates
                .join("website")
                .display()
                .to_string(),
            workspace: workspace.display().to_string(),
            project: "test-review".into(),
            ..Default::default()
        },
    );
}

#[test]
fn workspace_open_uses_persisted_command_and_passes_literal_path() {
    let writer = AppService::for_tests();
    let reader = AppService::from_config(writer.environments.config.clone()).unwrap();
    let workspace = writer
        .environments
        .config
        .workspaces
        .join("space ' ; $(touch injected)");
    std::fs::create_dir(&workspace).unwrap();
    register_workspace(&reader, &workspace);
    let command = "test -s AGENTS.md || exit 27; printf '%s' \"$TANDEM_WORKSPACE\" > received; printf '%s' \"$TANDEM_INSTANCE\" > instance; pwd > cwd";
    writer.runtime.block_on(async {
        assert_eq!(writer.get_open_command().await.unwrap(), "");
        writer
            .configure_open_command(command.into(), true)
            .await
            .unwrap();
        assert!(!workspace.join("received").exists());

        reader
            .open_workspace(workspace.to_str().unwrap(), "review")
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(workspace.join("received")).unwrap(),
            workspace.to_str().unwrap()
        );
        assert_eq!(
            std::fs::read_to_string(workspace.join("instance")).unwrap(),
            "review"
        );
        assert_eq!(
            std::fs::read_to_string(workspace.join("cwd"))
                .unwrap()
                .trim_end(),
            workspace.to_str().unwrap()
        );
        assert!(!workspace.join("injected").exists());
        assert!(reader.opened_system_targets().is_empty());
        assert_eq!(reader.get_open_command().await.unwrap(), command);

        for empty in ["", "  \n\t"] {
            writer
                .configure_open_command(empty.into(), true)
                .await
                .unwrap();
            reader
                .open_workspace(workspace.to_str().unwrap(), "review")
                .unwrap()
                .await
                .unwrap()
                .unwrap();
        }
        assert_eq!(
            reader.opened_system_targets(),
            [workspace.to_str().unwrap(); 2]
        );
    });
}

#[test]
fn open_command_failures_are_reported_without_falling_back_to_folder_opener() {
    let service = AppService::for_tests();
    let path = service.environments.config.workspaces.join("review");
    std::fs::create_dir(&path).unwrap();
    register_workspace(&service, &path);
    let workspace = path.to_str().unwrap();
    service.runtime.block_on(async {
        assert!(
            service
                .configure_open_command("touch unexpected".into(), false)
                .await
                .unwrap_err()
                .contains("confirmation_required")
        );
        assert!(
            service
                .configure_open_command("bad\0command".into(), true)
                .await
                .unwrap_err()
                .contains("NUL")
        );
        assert_eq!(service.get_open_command().await.unwrap(), "");
        assert!(service.open_workspace("relative/path", "review").is_err());
        service
            .configure_open_command("exit 23".into(), true)
            .await
            .unwrap();
        let error = service
            .open_workspace(workspace, "review")
            .unwrap()
            .await
            .unwrap()
            .unwrap_err();
        assert!(error.contains("23"), "{error}");
        assert!(service.opened_system_targets().is_empty());
        let missing = service.environments.config.workspaces.join("missing");
        assert!(
            service
                .open_workspace(missing.to_str().unwrap(), "review")
                .unwrap()
                .await
                .unwrap()
                .unwrap_err()
                .contains("workspace")
        );
    });
}

#[test]
fn saved_open_command_runs_for_a_named_workspace_after_approval() {
    let service = AppService::for_tests();
    let workspace = service.environments.config.workspaces.join("review");
    std::fs::create_dir(&workspace).unwrap();
    register_workspace(&service, &workspace);
    service.runtime.block_on(async {
        service
            .configure_open_command(
                "test -s AGENTS.md || exit 27; printf '%s' \"$TANDEM_INSTANCE\" > received".into(),
                true,
            )
            .await
            .unwrap();
        assert!(
            service
                .run_open_command("review".into(), false)
                .await
                .unwrap_err()
                .contains("confirmation_required")
        );
        assert!(!workspace.join("received").exists());
        assert_eq!(
            service
                .run_open_command("review".into(), true)
                .await
                .unwrap(),
            workspace.to_str().unwrap()
        );
    });
    assert_eq!(
        std::fs::read_to_string(workspace.join("received")).unwrap(),
        "review"
    );
}

#[test]
fn guidance_generation_failure_blocks_custom_and_system_openers() {
    let service = AppService::for_tests();
    let workspace = service.environments.config.workspaces.join("review");
    std::fs::create_dir(&workspace).unwrap();
    register_workspace(&service, &workspace);
    std::fs::write(
        service.environments.config.workspace_agents_template(),
        "{{unknown}}",
    )
    .unwrap();
    service.runtime.block_on(async {
        for command in ["touch unexpected", ""] {
            service
                .configure_open_command(command.into(), true)
                .await
                .unwrap();
            let error = service
                .run_open_command("review".into(), true)
                .await
                .unwrap_err();
            assert!(
                error.contains("unknown workspace AGENTS.md template placeholder"),
                "{error}"
            );
        }
    });
    assert!(!workspace.join("unexpected").exists());
    assert!(!workspace.join("AGENTS.md").exists());
    assert!(service.opened_system_targets().is_empty());
}

#[test]
fn open_command_survives_restart_and_write_failures_reach_the_caller() {
    let service = AppService::for_tests();
    service
        .runtime
        .block_on(service.configure_open_command("true".into(), true))
        .unwrap();
    let restarted = AppService::from_config(service.environments.config.clone()).unwrap();
    assert_eq!(restarted.open_command(), "true");
    let connection =
        rusqlite::Connection::open(service.environments.config.home.join("settings.sqlite3"))
            .unwrap();
    connection.execute_batch("CREATE TRIGGER reject_settings BEFORE INSERT ON app_settings BEGIN SELECT RAISE(FAIL, 'read only settings'); END;").unwrap();
    let error = service
        .runtime
        .block_on(service.configure_open_command("false".into(), true))
        .unwrap_err();
    assert!(error.contains("read only settings"));
    assert_eq!(service.open_command(), "true");
}
