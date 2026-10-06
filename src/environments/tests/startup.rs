use super::*;
use std::fs;

#[test]
#[ignore = "entry point for detached test workers"]
fn worker_entry() {
    let Ok(name) = std::env::var("TANDEM_TEST_WORKER_NAME") else {
        return;
    };
    crate::service::AppService::run_startup_worker(
        &name,
        &std::env::var("TANDEM_TEST_WORKER_ID").unwrap(),
        std::env::var("TANDEM_TEST_WORKER_LOCK")
            .unwrap()
            .parse()
            .unwrap(),
        std::env::var("TANDEM_TEST_WORKER_LEASE")
            .unwrap()
            .parse()
            .unwrap(),
    )
    .unwrap();
}

fn fixture() -> (tempfile::TempDir, Config, Record) {
    let directory = tempfile::tempdir().unwrap();
    let config = Config::at(directory.path().into(), "startup-test".into(), 9876).unwrap();
    let environments = Environments::new(config.clone());
    let operation = environments
        .begin("create_instance", "Review", Some("website".into()))
        .unwrap();
    let record = Record {
        operation,
        origin_operation_id: None,
        description: Some("Review workspace".into()),
        branch_instances: false,
        start_instance: true,
        preserve_opencode_history: false,
        kind: StartupKind::Cold,
        started_at: journal::now(),
        timeout: 60,
        owner_pid: 42,
        workspace_ready: false,
        opencode_requested: false,
        opencode_result: None,
        services: Vec::new(),
    };
    (directory, config, record)
}

#[test]
fn reconnect_exposes_startup_before_containers_exist() {
    let (_directory, config, record) = fixture();
    let _instance = gateway::lock(&config, "instance-Review").unwrap();
    let _lease = gateway::lock(&config, &lease(&record.operation.id)).unwrap();
    write(&config, &record).unwrap();
    let reconnected = Environments::new(config.clone());
    reconnected.refresh_startups().unwrap();
    assert_eq!(
        reconnected.operation(&record.operation.id).unwrap().state,
        OperationState::Running
    );
    let mut instances = Vec::new();
    let mut activities = Vec::new();
    enrich(&config, &mut instances, &mut activities).unwrap();
    assert_eq!(instances.len(), 1);
    assert!(instances[0].pending);
    assert_eq!(instances[0].name, "Review");
    assert_eq!(instances[0].description, "Review workspace");
    assert_eq!(activities[0].owner_pid, 42);
    assert!(activities[0].active());
    assert!(journal::enrich(&config, &mut instances).unwrap().is_empty());
}

#[test]
fn startup_snapshots_preserve_service_start_intent_across_reconnection() {
    let (_directory, config, mut record) = fixture();
    let _lease = gateway::lock(&config, &lease(&record.operation.id)).unwrap();
    for start_instance in [false, true] {
        record.start_instance = start_instance;
        write(&config, &record).unwrap();
        let reconnected = Environments::new(config.clone());
        reconnected.refresh_startups().unwrap();
        assert_eq!(
            reconnected.snapshot().startup["Review"].prepare_only,
            !start_instance
        );
        let worker = Environments::new(config.clone());
        worker.adopt_startup(&record);
        assert_eq!(
            worker.snapshot().startup["Review"].prepare_only,
            !start_instance
        );
    }
}

#[test]
fn startup_admission_rejects_a_conflicting_origin_without_replacing_ownership() {
    let (_directory, config, record) = fixture();
    write(&config, &record).unwrap();
    let environments = Environments::new(config.clone());
    let operation = environments
        .begin("create_instance", "Review", Some("website".into()))
        .unwrap();
    let mut startup = Startup {
        origin_operation_id: Some("1-1".into()),
        ..Default::default()
    };
    let error = launch(&environments, &operation, 60, &mut startup).unwrap_err();
    assert_eq!(error, "startup belongs to another instance lineage");
    assert_eq!(
        read(&config, "Review").unwrap().unwrap().operation.id,
        record.operation.id
    );
}

#[test]
fn worker_failure_survives_a_later_successful_action() {
    let (_directory, config, record) = fixture();
    write(&config, &record).unwrap();
    let _later_lock = gateway::lock(&config, "instance-Review").unwrap();
    journal::ActivityGuard::begin(&config, "Review", "start_service", Some("api"), 60)
        .unwrap()
        .finish(Ok(()))
        .unwrap();
    let mut instances = Vec::new();
    let mut activities = Vec::new();
    enrich(&config, &mut instances, &mut activities).unwrap();
    assert!(!instances[0].pending);
    assert!(
        instances[0]
            .runtime
            .issue
            .as_ref()
            .unwrap()
            .contains("interrupted")
    );
    assert_eq!(activities[0].id, record.operation.id);
    assert!(activities[0].finished);
    let reconnected = Environments::new(config);
    reconnected.refresh_startups().unwrap();
    assert_eq!(
        reconnected.operation(&record.operation.id).unwrap().state,
        OperationState::Failed
    );
}

#[test]
fn completed_startup_retains_its_outcome_and_bounded_progress() {
    let (_directory, config, record) = fixture();
    write(&config, &record).unwrap();
    let writer = Writer {
        config: config.clone(),
        record: Mutex::new(record.clone()),
    };
    for index in 0..40 {
        writer.progress(format!("Step {index}"));
    }
    let progress = read(&config, "Review").unwrap().unwrap().operation.progress;
    assert_eq!(progress.len(), 30);
    assert_eq!(progress[0], "Step 10");
    let mut operation = record.operation.clone();
    operation.state = OperationState::Succeeded;
    writer.opencode_result(Err("client launch unavailable".into()));
    writer.finish(operation.clone()).unwrap();
    assert_eq!(
        read(&config, "Review").unwrap().unwrap().opencode_result,
        Some(Err("client launch unavailable".into()))
    );
    assert_eq!(
        record.observe(&config).unwrap().operation.state,
        OperationState::Succeeded
    );
    let reconnected = Environments::new(config.clone());
    reconnected.refresh_startups().unwrap();
    assert_eq!(
        reconnected.operation(&operation.id).unwrap().state,
        OperationState::Succeeded
    );
    forget(&config, "Review").unwrap();
    assert!(records(&config).unwrap().is_empty());
}

#[test]
fn successful_completion_persists_after_a_progress_write_error() {
    let (_directory, config, record) = fixture();
    write(&config, &record).unwrap();
    let writer = Writer {
        config: config.clone(),
        record: Mutex::new(record.clone()),
    };
    let connection = rusqlite::Connection::open(config.home.join("settings.sqlite3")).unwrap();
    connection.execute_batch("CREATE TRIGGER reject_progress BEFORE UPDATE ON runtime_records WHEN NEW.kind = 'startup' BEGIN SELECT RAISE(ABORT, 'progress write unavailable'); END;").unwrap();
    writer.workspace_ready();
    connection
        .execute_batch("DROP TRIGGER reject_progress;")
        .unwrap();
    let mut operation = record.operation;
    operation.state = OperationState::Succeeded;
    writer.finish(operation).unwrap();
    let completed = read(&config, "Review").unwrap().unwrap();
    assert_eq!(completed.operation.state, OperationState::Succeeded);
    assert!(completed.operation.error.is_none());
    assert!(completed.workspace_ready);
}

#[test]
fn completion_write_failures_preserve_the_last_durable_startup_record() {
    let (_directory, config, record) = fixture();
    write(&config, &record).unwrap();
    let writer = Writer {
        config: config.clone(),
        record: Mutex::new(record.clone()),
    };
    let connection = rusqlite::Connection::open(config.home.join("settings.sqlite3")).unwrap();
    connection.execute_batch("CREATE TRIGGER reject_completion BEFORE UPDATE ON runtime_records WHEN NEW.kind = 'startup' BEGIN SELECT RAISE(ABORT, 'completion write unavailable'); END;").unwrap();
    let mut operation = record.operation;
    operation.state = OperationState::Succeeded;
    assert!(
        writer
            .finish(operation)
            .unwrap_err()
            .contains("completion write unavailable")
    );
    assert_eq!(
        read(&config, "Review").unwrap().unwrap().operation.state,
        OperationState::Running
    );
}

#[test]
fn purge_clears_an_unclaimed_cold_startup_and_preserves_unverified_data() {
    for existing_workspace in [false, true] {
        let (_directory, config, mut record) = fixture();
        record.owner_pid = 0;
        record.operation.state = OperationState::Failed;
        record.operation.error =
            Some("cannot launch startup worker: No such file or directory (os error 2)".into());
        write(&config, &record).unwrap();
        let workspace = config.workspaces.join("Review");
        if existing_workspace {
            fs::create_dir(&workspace).unwrap();
            fs::write(workspace.join("data"), "keep").unwrap();
        }
        let mut instances = Vec::new();
        let mut activities = Vec::new();
        enrich(&config, &mut instances, &mut activities).unwrap();
        assert_eq!(instances.len(), 1);

        let held = gateway::lock(&config, "instance-Review").unwrap();
        assert!(
            super::super::lifecycle::delete(
                &config,
                "Review",
                std::sync::Arc::new(|_| {}),
                &|_, _| { panic!("busy startup must not close clients") }
            )
            .unwrap_err()
            .contains("busy")
        );
        assert!(read(&config, "Review").unwrap().is_some());
        drop(held);
        super::super::lifecycle::delete(&config, "Review", std::sync::Arc::new(|_| {}), &|_, _| {
            panic!("unclaimed startup must not close clients")
        })
        .unwrap();

        let reconnected = Environments::new(config.clone());
        reconnected.refresh_startups().unwrap();
        assert!(reconnected.operation(&record.operation.id).is_err());
        instances.clear();
        activities.clear();
        enrich(&config, &mut instances, &mut activities).unwrap();
        assert!(instances.is_empty());
        assert!(activities.is_empty());
        assert!(journal::enrich(&config, &mut instances).unwrap().is_empty());
        if existing_workspace {
            assert_eq!(fs::read_to_string(workspace.join("data")).unwrap(), "keep");
        } else {
            assert!(!workspace.exists());
        }
    }
}

#[test]
fn metadata_only_purge_requires_a_failed_unclaimed_cold_startup_without_preparation() {
    let (_directory, config, mut record) = fixture();
    let eligible =
        |config: &Config| super::super::cleanup::unclaimed_startup(config, "Review").unwrap();
    assert!(!eligible(&config));
    let _lease = gateway::lock(&config, &lease(&record.operation.id)).unwrap();
    record.owner_pid = 0;
    write(&config, &record).unwrap();
    assert!(!eligible(&config));
    record.operation.state = OperationState::Succeeded;
    write(&config, &record).unwrap();
    assert!(!eligible(&config));
    record.operation.state = OperationState::Failed;
    record.owner_pid = 42;
    write(&config, &record).unwrap();
    assert!(!eligible(&config));
    record.owner_pid = 0;
    record.kind = StartupKind::Hot;
    write(&config, &record).unwrap();
    assert!(!eligible(&config));
    record.kind = StartupKind::Cold;
    write(&config, &record).unwrap();
    assert!(eligible(&config));
    let template = super::super::templates::create(&config, "website").unwrap();
    journal::prepare(&config, &template, "Review", None).unwrap();
    assert!(!eligible(&config));
    assert!(read(&config, "Review").unwrap().is_some());
}
