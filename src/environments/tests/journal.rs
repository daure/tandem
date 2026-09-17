use super::*;
use crate::store::environments::{HealthState, ServiceRuntime, Severity, Status};

fn fixture() -> (tempfile::TempDir, Config, Instance) {
    let directory = tempfile::tempdir().unwrap();
    let config = Config::at(directory.path().into(), "status-test".into(), 9876).unwrap();
    let instance = Instance {
        name: "review".into(),
        template: "website".into(),
        template_directory: config.templates.join("website").display().to_string(),
        project: config.project("review"),
        services: vec![InstanceService {
            name: "web".into(),
            container_id: "abc".into(),
            started_at: Some("run-1".into()),
            runtime: ServiceRuntime {
                state: ContainerState::Running,
                health: HealthState::Healthy,
                replica: 1,
                ..Default::default()
            },
            ..Default::default()
        }],
        ..Default::default()
    };
    (directory, config, instance)
}

#[test]
fn persisted_stop_evidence_is_scoped_to_the_exact_run_and_exit() {
    let (_directory, config, mut app) = fixture();
    app.services[0].runtime.state = ContainerState::Exited;
    app.services[0].runtime.exit_code = Some(143);
    app.services[0].runtime.finished_at = Some("finished".into());
    let mut record = Record::default();
    record.stops.insert(
        "abc".into(),
        StopReceipt {
            started_at: Some("run-1".into()),
            finished_at: Some("finished".into()),
            exit_code: Some(143),
            requested_at: "requested".into(),
            confirmed: true,
        },
    );
    write(&config, "review", &record).unwrap();
    let reconnected =
        Config::at(config.home.clone(), config.namespace.clone(), config.port).unwrap();
    enrich(&reconnected, std::slice::from_mut(&mut app)).unwrap();
    assert!(app.services[0].runtime.requested_stop);
    assert_eq!(
        app.services[0].status_summary().detail_severity,
        Severity::Muted
    );
    app.services[0].started_at = Some("run-2".into());
    enrich(&reconnected, std::slice::from_mut(&mut app)).unwrap();
    assert!(!app.services[0].runtime.requested_stop);
    assert_eq!(
        app.services[0].status_summary().detail_severity,
        Severity::Warning
    );
}

#[test]
fn topology_preserves_missing_replica_slots_and_job_roles() {
    let (_directory, config, mut app) = fixture();
    let mut second = app.services[0].clone();
    second.container_id.clear();
    second.runtime.replica = 2;
    let _lock = gateway::lock(&config, "instance-review").unwrap();
    let guard = ActivityGuard::begin(&config, "review", "create_instance", None, 60).unwrap();
    topology(
        &config,
        "website",
        "review",
        vec![app.services[0].clone(), second],
    )
    .unwrap();
    let activities = enrich(&config, std::slice::from_mut(&mut app)).unwrap();
    assert!(activities[0].active());
    assert_eq!(app.services.len(), 2);
    assert_eq!(app.services[1].status_summary().status, Status::Waiting);
    guard.finish(Ok(())).unwrap();
    app.services.truncate(1);
    enrich(&config, std::slice::from_mut(&mut app)).unwrap();
    assert_eq!(app.services[1].status_summary().status, Status::Missing);
    assert_eq!(
        (app.status_summary().running, app.status_summary().expected),
        (1, 2)
    );
}

#[test]
fn lost_operation_lock_ends_activity_and_startup_suppression() {
    let (_directory, config, mut app) = fixture();
    let lock = gateway::lock(&config, "instance-review").unwrap();
    let guard = ActivityGuard::begin(&config, "review", "create_instance", None, 60).unwrap();
    enrich(&config, std::slice::from_mut(&mut app)).unwrap();
    assert!(app.suppress_resources());
    drop(lock);
    let activities = enrich(&config, std::slice::from_mut(&mut app)).unwrap();
    assert!(!activities[0].active());
    assert!(
        activities[0]
            .error
            .as_ref()
            .unwrap()
            .contains("interrupted")
    );
    assert!(!app.suppress_resources());
    drop(guard);
}

#[test]
fn failed_cleanup_remains_observable_without_containers() {
    let (_directory, config, app) = fixture();
    topology(&config, "website", "review", app.services).unwrap();
    let _lock = gateway::lock(&config, "instance-review").unwrap();
    let guard = ActivityGuard::begin(&config, "review", "delete_instance", None, 60).unwrap();
    assert!(guard.finish::<()>(Err("volume busy".into())).is_err());
    let activities = enrich(&config, &mut []).unwrap();
    assert_eq!(activities[0].template.as_deref(), Some("website"));
    assert_eq!(activities[0].error.as_deref(), Some("volume busy"));
}

#[test]
fn rendered_launch_topology_is_read_without_using_edited_template_configuration() {
    use crate::environments::compose;
    use serde_json::json;
    let (_directory, config, mut app) = fixture();
    let directory = config.templates.join("website");
    fs::create_dir_all(&directory).unwrap();
    let labels = json!({
        (compose::NAMESPACE): config.namespace,
        (compose::KIND): "instance", (compose::INSTANCE): app.name,
        (compose::TEMPLATE): app.template, (compose::DIRECTORY): app.template_directory,
        (compose::WORKSPACE): app.workspace, (compose::ROLE): "service"
    });
    let model = json!({"name": app.project, "services": {"web": {"labels": labels, "deploy": {"replicas": 2}}}});
    fs::write(
        directory.join(format!(".tandem-{}-review.compose.json", config.namespace)),
        serde_json::to_vec(&model).unwrap(),
    )
    .unwrap();
    fs::write(directory.join("compose.yaml"), "services: {}").unwrap();
    enrich(&config, std::slice::from_mut(&mut app)).unwrap();
    assert!(app.runtime.topology_known);
    assert_eq!(app.services.len(), 2);
    assert_eq!(app.services[1].runtime.replica, 2);
    assert_eq!(app.status_summary().status, Status::Degraded);
}
