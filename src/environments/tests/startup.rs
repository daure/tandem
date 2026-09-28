use super::*;

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
        description: Some("Review workspace".into()),
        branch_instances: false,
        kind: StartupKind::Cold,
        started_at: journal::now(),
        timeout: 60,
        owner_pid: 42,
        workspace_ready: false,
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
    let writer = Writer {
        config: config.clone(),
        record: Mutex::new(record.clone()),
        error: Mutex::new(None),
    };
    for index in 0..40 {
        writer.progress(format!("Step {index}"));
    }
    let progress = read(&config, "Review").unwrap().unwrap().operation.progress;
    assert_eq!(progress.len(), 30);
    assert_eq!(progress[0], "Step 10");
    let mut operation = record.operation.clone();
    operation.state = OperationState::Succeeded;
    writer.finish(operation.clone()).unwrap();
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
