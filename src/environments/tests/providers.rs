use super::*;

#[cfg(unix)]
#[test]
fn provider_snapshots_batch_listing_and_isolate_collector_failures() {
    const FIXTURE: &str = "TANDEM_PROVIDER_SNAPSHOT_FIXTURE";
    if let Some(directory) = std::env::var_os(FIXTURE) {
        check_provider_snapshots(std::path::Path::new(&directory));
        return;
    }

    use std::os::unix::fs::PermissionsExt;

    let directory = tempfile::tempdir().unwrap();
    let docker = directory.path().join("docker");
    fs::write(
        &docker,
        r#"#!/bin/sh
set -eu
root=${0%/*}
printf '%s\n' "$*" >> "$root/calls"
case "$1" in
    ps)
        if [ -f "$root/list-error" ]; then
            cat "$root/list-error" >&2
            exit 1
        fi
        cat "$root/listing"
        ;;
    inspect)
        [ "$#" -eq 2 ]
        if [ -f "$root/$2-error" ]; then
            cat "$root/$2-error" >&2
            exit 1
        fi
        cat "$root/$2.json"
        ;;
    *) exit 1 ;;
esac
"#,
    )
    .unwrap();
    fs::set_permissions(&docker, fs::Permissions::from_mode(0o755)).unwrap();
    let path = std::env::join_paths(std::iter::once(directory.path().to_path_buf()).chain(
        std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()),
    ))
    .unwrap();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "environments::providers::tests::provider_snapshots_batch_listing_and_isolate_collector_failures",
            "--nocapture",
        ])
        .env(FIXTURE, directory.path())
        .env("PATH", path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[cfg(unix)]
fn check_provider_snapshots(directory: &std::path::Path) {
    let config = Config::at(directory.join("home"), "snapshot-test".into(), 9876).unwrap();
    EventStore::open(&config).unwrap();
    let manager = Providers::new(&config).unwrap();
    assert!(manager.snapshot().unwrap().providers.is_empty());
    assert!(!directory.join("calls").exists());

    for name in ["alpha", "beta"] {
        let package = config.home.join("templates/providers").join(name);
        fs::create_dir_all(&package).unwrap();
        fs::write(package.join("Dockerfile"), "FROM scratch\n").unwrap();
        fs::write(
            package.join("provider.json"),
            json!({
                "schema_version": 2, "name": name, "description": "Fixture",
                "protocol": "tandem-events-v1",
                "streams": [{"name": "updates", "profile": "generic"}]
            })
            .to_string(),
        )
        .unwrap();
    }
    let collector = |id: &str, name: &str, running: bool| {
        json!([{
            "Id": id, "State": {"Running": running, "Paused": false},
            "Config": {"Labels": {
                "io.tandem.provider-namespace": config.namespace,
                "io.tandem.provider-name": name,
                "com.docker.compose.project": manager.project(name),
                "com.docker.compose.service": "collector"
            }}
        }])
    };
    fs::write(
        directory.join("alpha.json"),
        collector("alpha", "alpha", true).to_string(),
    )
    .unwrap();
    fs::write(
        directory.join("beta.json"),
        collector("beta", "beta", false).to_string(),
    )
    .unwrap();
    let listing = "alpha\tsnapshot-test-provider-alpha\nbeta\tsnapshot-test-provider-beta\nunrelated\tother-provider-alpha\nprefix\tsnapshot-test-provider-alpha-extra\nempty-label\t\n";
    fs::write(directory.join("listing"), listing).unwrap();
    let snapshot = |inspected: &[&str]| {
        let snapshot = manager.snapshot().unwrap();
        let calls = fs::read_to_string(directory.join("calls")).unwrap();
        fs::remove_file(directory.join("calls")).unwrap();
        let mut expected = vec![
            "ps --all --filter label=com.docker.compose.project --format {{.ID}}\t{{.Label \"com.docker.compose.project\"}}".to_owned(),
        ];
        expected.extend(inspected.iter().map(|id| format!("inspect {id}")));
        assert_eq!(calls.lines().collect::<Vec<_>>(), expected);
        assert_eq!(snapshot.providers.len(), 2);
        for provider in &snapshot.providers {
            assert_eq!(provider.streams.len(), 1);
            assert_eq!(provider.streams[0].name, "updates");
        }
        snapshot
    };

    let observed = snapshot(&["alpha", "beta"]);
    assert_eq!(observed.error, None);
    assert_eq!(observed.providers[0].status, Status::Running);
    assert_eq!(observed.providers[1].status, Status::Stopped);
    assert_eq!(observed.providers[1].container_id.as_deref(), Some("beta"));

    fs::write(
        directory.join("listing"),
        "alpha-new\tsnapshot-test-provider-alpha\n",
    )
    .unwrap();
    fs::write(
        directory.join("alpha-new.json"),
        collector("alpha-new", "alpha", false).to_string(),
    )
    .unwrap();
    let observed = snapshot(&["alpha-new"]);
    assert_eq!(observed.providers[0].status, Status::Stopped);
    assert_eq!(
        observed.providers[0].container_id.as_deref(),
        Some("alpha-new")
    );
    assert_eq!(observed.providers[1].status, Status::NotStarted);
    assert_eq!(observed.providers[1].container_id, None);

    fs::write(directory.join("listing"), listing).unwrap();
    fs::write(directory.join("alpha-error"), "inspection failed").unwrap();
    let observed = snapshot(&["alpha", "beta"]);
    assert_eq!(observed.providers[0].status, Status::Unknown);
    assert!(
        observed.providers[0]
            .error
            .as_deref()
            .unwrap()
            .contains("inspection failed")
    );
    assert_eq!(observed.providers[1].status, Status::Stopped);
    assert_eq!(observed.providers[1].error, None);
    fs::remove_file(directory.join("alpha-error")).unwrap();

    for namespace in [None, Some("foreign")] {
        let mut foreign = collector("alpha", "alpha", true);
        let labels = foreign[0]["Config"]["Labels"].as_object_mut().unwrap();
        match namespace {
            Some(namespace) => {
                labels.insert("io.tandem.provider-namespace".into(), json!(namespace));
            }
            None => {
                labels.remove("io.tandem.provider-namespace");
            }
        }
        fs::write(directory.join("alpha.json"), foreign.to_string()).unwrap();
        let observed = snapshot(&["alpha", "beta"]);
        assert_eq!(observed.providers[0].status, Status::Unknown);
        assert_eq!(observed.providers[0].container_id, None);
        assert_eq!(
            observed.providers[0].error.as_deref(),
            Some("provider container has unverifiable ownership")
        );
        assert_eq!(observed.providers[1].status, Status::Stopped);
        assert_eq!(observed.providers[1].error, None);
    }

    fs::write(directory.join("list-error"), "listing failed").unwrap();
    let observed = snapshot(&[]);
    assert!(
        observed
            .error
            .as_deref()
            .unwrap()
            .contains("listing failed")
    );
    for provider in observed.providers {
        assert_eq!(provider.status, Status::Unknown);
        assert_eq!(provider.container_id, None);
        assert!(
            provider
                .error
                .as_deref()
                .unwrap()
                .contains("listing failed")
        );
    }
}

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
            schema_version: 2,
            name: "source".into(),
            description: "Source".into(),
            protocol: "tandem-events-v1".into(),
            feedback: vec![],
            streams: vec![crate::store::providers::StreamDeclaration {
                name: "releases".into(),
                profile: "generic".into(),
            }],
            stream_control: true,
        }),
        available: true,
        status: Status::NotStarted,
        container_id: None,
        error: None,
        operation: None,
        streams: vec![],
    };
    let legacy = json!({
        "schema_version": 1, "name": "source", "profile": "generic",
        "description": "Source", "protocol": "tandem-events-v1",
        "streams": ["releases"], "stream_control": true
    });
    manager
        .database()
        .unwrap()
        .execute(
            "INSERT INTO provider_launches(namespace, name, directory, manifest, compose) VALUES (?1, 'package', ?2, ?3, '{}')",
            params![config.namespace, provider.directory, legacy.to_string()],
        )
        .unwrap();
    let store = EventStore::open(&config).unwrap();
    let token = store.register_provider("source").unwrap();
    manager.prepare_stream_controls("package", None).unwrap();
    assert_eq!(store.stream_controls(&token).unwrap()[0].stream, "releases");
    manager.save_launch(&provider, "{}").unwrap();
    let saved: String = manager
        .database()
        .unwrap()
        .query_row(
            "SELECT manifest FROM provider_launches WHERE name = 'package'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Manifest>(&saved).unwrap(),
        *provider.manifest.as_ref().unwrap()
    );
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
                "schema_version": 2, "name": "source",
                "description": "Source", "protocol": "tandem-events-v1"
            }))
            .unwrap(),
        ),
        available: false,
        status: Status::Running,
        container_id: None,
        error: None,
        operation: None,
        streams: vec![],
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
fn stop_discards_events_while_the_lifecycle_command_is_pending() {
    let (_home, config, manager, _store, token) = ingestion_setup();
    let sidecar = EventStore::open(&config).unwrap();
    assert_eq!(
        ingest_sample(&sidecar, &token, "retained").receipts.len(),
        1
    );
    let (entered, pending) = std::sync::mpsc::channel();
    let (release, finish) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        manager.run_action::<String>("package", Action::Stop, || {
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

#[test]
fn failed_stop_keeps_ingestion_disabled_and_logs_preserve_the_gate() {
    let (_home, _config, manager, store, token) = ingestion_setup();
    assert!(
        manager
            .run_action::<String>("package", Action::Stop, || Err("Docker timed out".into()))
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

#[test]
fn start_enables_ingestion_before_collector_work_and_restores_the_gate_on_failure() {
    let (_home, _config, manager, store, token) = ingestion_setup();
    store.set_ingestion_enabled("source", false).unwrap();
    assert!(
        manager
            .run_action::<String>("package", Action::Start, || {
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
        .run_action::<String>("package", Action::Start, || Ok("completed".into()))
        .unwrap();
    assert_eq!(ingest_sample(&store, &token, "activated").receipts.len(), 1);
    assert!(matches!(
        manager.run_action::<String>("package", Action::Stop, || {
            Err(ActionError::Unavailable("runtime changed"))
        }),
        Err(ActionError::Unavailable(_))
    ));
    assert_eq!(
        ingest_sample(&store, &token, "unavailable-stop")
            .receipts
            .len(),
        1
    );
}
