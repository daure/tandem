use super::*;

#[test]
fn provider_ownership_requires_namespace_name_project_and_collector_service() {
    let mut container = json!({"Config": {"Labels": {
        "io.tandem.provider-namespace": "test", "io.tandem.provider-name": "sample",
        "com.docker.compose.project": "test-provider-sample", "com.docker.compose.service": "collector"
    }}});
    validate_owner(&container, "test", "sample", "test-provider-sample").unwrap();
    for key in [
        "io.tandem.provider-namespace",
        "io.tandem.provider-name",
        "com.docker.compose.project",
        "com.docker.compose.service",
    ] {
        let previous = container["Config"]["Labels"][key].take();
        assert!(validate_owner(&container, "test", "sample", "test-provider-sample").is_err());
        container["Config"]["Labels"][key] = previous;
    }
}

#[test]
fn provider_discovery_exposes_invalid_packages_without_creating_runtime_resources() {
    let home = tempfile::tempdir().unwrap();
    let config = Config::at(home.path().into(), "providers-test".into(), 9876).unwrap();
    EventStore::open(&config).unwrap();
    let directory = config.home.join("templates/providers/broken");
    fs::create_dir_all(&directory).unwrap();
    fs::write(directory.join("provider.json"), "{}").unwrap();
    let manager = Providers::new(&config).unwrap();
    let result = manager.definition("broken").unwrap_err();
    assert!(result.contains("Dockerfile"));
    assert!(!config.home.join("provider-credentials").exists());
    assert!(!config.home.join("runtime").exists());
}

#[test]
fn provider_launches_preserve_identity_and_reject_reuse_by_another_package() {
    let home = tempfile::tempdir().unwrap();
    let config = Config::at(home.path().into(), "identity-test".into(), 9876).unwrap();
    EventStore::open(&config).unwrap();
    let manager = Providers::new(&config).unwrap();
    let mut provider = Provider {
        name: "package".into(),
        directory: "/templates/providers/package".into(),
        manifest: Some(Manifest {
            schema_version: 1,
            name: "source".into(),
            profile: "generic".into(),
            description: "Source".into(),
            protocol: "tandem-events-v1".into(),
            feedback: vec![],
        }),
        available: true,
        status: Status::NotStarted,
        container_id: None,
        error: None,
        operation: None,
    };
    manager.save_launch(&provider, "{}").unwrap();
    manager.save_launch(&provider, "{}").unwrap();
    provider.name = "other-package".into();
    assert!(
        manager
            .save_launch(&provider, "{}")
            .unwrap_err()
            .contains("already belongs")
    );
    provider.name = "package".into();
    provider.manifest.as_mut().unwrap().name = "different-source".into();
    assert!(
        manager
            .save_launch(&provider, "{}")
            .unwrap_err()
            .contains("identity is fixed")
    );
}

fn ingestion_setup() -> (tempfile::TempDir, Config, Providers, EventStore, String) {
    let home = tempfile::tempdir().unwrap();
    let config = Config::at(home.path().into(), "ingestion-test".into(), 9876).unwrap();
    let store = EventStore::open(&config).unwrap();
    let token = store.register_provider("source").unwrap();
    let manager = Providers::new(&config).unwrap();
    let provider = Provider {
        name: "package".into(),
        directory: "/templates/providers/package".into(),
        manifest: Some(
            serde_json::from_value(json!({
                "schema_version": 1, "name": "source", "profile": "message",
                "description": "Source", "protocol": "tandem-events-v1"
            }))
            .unwrap(),
        ),
        available: false,
        status: Status::Running,
        container_id: None,
        error: None,
        operation: None,
    };
    manager.save_launch(&provider, "{}").unwrap();
    (home, config, manager, store, token)
}

fn ingest_sample(store: &EventStore, token: &str, id: &str) -> crate::store::events::Ingestion {
    store
        .ingest(
            token,
            crate::store::events::Batch {
                events: vec![super::super::events::tests::event(id)],
            },
        )
        .unwrap()
}

#[test]
fn stop_and_pause_discard_events_while_the_lifecycle_command_is_pending() {
    for action in [Action::Stop, Action::Pause] {
        let (_home, config, manager, _store, token) = ingestion_setup();
        let sidecar = EventStore::open(&config).unwrap();
        assert_eq!(
            ingest_sample(&sidecar, &token, "retained").receipts.len(),
            1
        );
        let (entered, pending) = std::sync::mpsc::channel();
        let (release, finish) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            manager.run_action::<String>("package", action, || {
                entered.send(()).unwrap();
                finish.recv_timeout(Duration::from_secs(5)).unwrap();
                Ok("completed".into())
            })
        });
        pending.recv_timeout(Duration::from_secs(5)).unwrap();
        let result = ingest_sample(&sidecar, &token, "during-stop");
        assert!(result.receipts.is_empty());
        assert_eq!(result.discarded, ["during-stop"]);
        assert_eq!(sidecar.snapshot().unwrap().total, 1);
        assert_eq!(sidecar.notifications(&token).unwrap().len(), 1);
        release.send(()).unwrap();
        worker.join().unwrap().unwrap();
        assert_eq!(
            ingest_sample(&sidecar, &token, "after-stop").discarded,
            ["after-stop"]
        );
    }
}

#[test]
fn failed_stop_or_pause_keeps_ingestion_disabled_and_logs_preserve_the_gate() {
    for action in [Action::Stop, Action::Pause] {
        let (_home, _config, manager, store, token) = ingestion_setup();
        assert!(
            manager
                .run_action::<String>("package", action, || Err("Docker timed out".into()))
                .is_err()
        );
        assert_eq!(
            ingest_sample(&store, &token, "failed-stop").discarded,
            ["failed-stop"]
        );
        manager
            .run_action::<String>("package", Action::Logs, || Ok("logs".into()))
            .unwrap();
        assert_eq!(
            ingest_sample(&store, &token, "after-logs").discarded,
            ["after-logs"]
        );
    }
}

#[test]
fn activation_enables_ingestion_before_collector_work_and_restores_the_gate_on_failure() {
    for action in [Action::Start, Action::Resume, Action::Restart] {
        let (_home, _config, manager, store, token) = ingestion_setup();
        store.set_ingestion_enabled("source", false).unwrap();
        assert!(
            manager
                .run_action::<String>("package", action, || {
                    assert_eq!(
                        ingest_sample(&store, &token, "during-activation")
                            .receipts
                            .len(),
                        1
                    );
                    Err("Docker failed".into())
                })
                .is_err()
        );
        assert_eq!(
            ingest_sample(&store, &token, "failed-activation").discarded,
            ["failed-activation"]
        );
        manager
            .run_action::<String>("package", action, || Ok("completed".into()))
            .unwrap();
        assert_eq!(ingest_sample(&store, &token, "activated").receipts.len(), 1);
        assert!(matches!(
            manager.run_action::<String>("package", Action::Pause, || {
                Err(ActionError::Unavailable("runtime changed"))
            }),
            Err(ActionError::Unavailable(_))
        ));
        assert_eq!(
            ingest_sample(&store, &token, "unavailable-pause")
                .receipts
                .len(),
            1
        );
    }
}
