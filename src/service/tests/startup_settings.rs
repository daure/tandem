use super::*;
use crate::service::refresh::Refresh;
use std::time::{Duration, Instant};

fn wait_for(mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !condition() {
        assert!(Instant::now() < deadline, "startup timing was not observed");
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn settings_refresh_loads_external_cold_and_hot_startup_history() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("settings.sqlite3");
    let observer = Settings::open(path.clone()).unwrap();
    let writer = Settings::open(path).unwrap();
    for kind in [StartupKind::Cold, StartupKind::Hot] {
        writer.record_startup("blank".into(), kind, 25).unwrap();
    }
    observer.refresh().unwrap();
    for kind in [StartupKind::Cold, StartupKind::Hot] {
        assert_eq!(
            observer.startup_averages(kind),
            [("blank".into(), 25)].into()
        );
    }
}

#[test]
fn startup_recording_returns_database_errors_without_caching_failed_samples() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("settings.sqlite3");
    let settings = Settings::open(path.clone()).unwrap();
    let connection = Connection::open(path).unwrap();
    connection
        .execute_batch(
            "CREATE TRIGGER reject_startup BEFORE INSERT ON instance_startups
             BEGIN SELECT RAISE(FAIL, 'startup write rejected'); END;",
        )
        .unwrap();
    let error = settings
        .record_startup("blank".into(), StartupKind::Cold, 25)
        .unwrap_err();
    assert!(error.contains("startup write rejected"), "{error}");
    assert!(settings.startup_averages(StartupKind::Cold).is_empty());
}

#[test]
fn detached_workspace_startups_update_an_open_observers_cold_and_hot_timings() {
    for start_instance in [false, true] {
        assert_workspace_timings(start_instance);
    }
}

fn assert_workspace_timings(start_instance: bool) {
    let writer = AppService::for_tests();
    writer
        .runtime
        .block_on(writer.set_opencode_enabled(false).unwrap())
        .unwrap()
        .unwrap();
    writer
        .runtime
        .block_on(writer.create_template("blank".into()))
        .unwrap();
    let observer = AppService::from_config(writer.environments.config.clone()).unwrap();
    observer.refresh.request(Refresh::Settings);

    for kind in [StartupKind::Cold, StartupKind::Hot] {
        let operation = writer
            .submit_instance_creation("scratch", "blank".into(), 60, true, start_instance, None)
            .unwrap();
        let operation = writer
            .runtime
            .block_on(writer.wait_operation(&operation.id))
            .unwrap();
        assert_eq!(
            operation.state,
            crate::store::environments::OperationState::Succeeded
        );
        assert!(operation.instance.unwrap().workspace_only);
        wait_for(|| {
            observer.settings.startup_averages(kind).get("blank")
                == Some(&operation.elapsed_milliseconds)
        });
    }
    wait_for(|| observer.environment_snapshot().instances.len() == 1);
    let snapshot = observer.environment_snapshot();
    assert!(
        snapshot
            .cold_startup_averages_milliseconds
            .contains_key("blank")
    );
    assert!(
        snapshot
            .hot_startup_averages_milliseconds
            .contains_key("blank")
    );
}

#[test]
fn startup_countdowns_follow_template_readiness_and_service_start_intent() {
    let service = AppService::for_tests();
    service.environments.create_template("blank").unwrap();
    service.environments.refresh_templates();
    for template in ["blank", "website"] {
        for kind in [StartupKind::Cold, StartupKind::Hot] {
            service
                .settings
                .record_startup(template.into(), kind, 12_000)
                .unwrap();
        }
        for start_instance in [false, true] {
            let name = format!("{template}-{start_instance}");
            let mut operation = service.queue_instance_for_tests(&name, template);
            for kind in [StartupKind::Cold, StartupKind::Hot] {
                if kind == StartupKind::Hot {
                    let snapshot = service.environment_snapshot();
                    let instance = snapshot
                        .instances
                        .iter()
                        .find(|instance| instance.name == name)
                        .unwrap()
                        .clone();
                    service.complete_instance_for_tests(&operation.id, instance);
                    operation = service.queue_instance_for_tests(&name, template);
                }
                service
                    .environments
                    .set_start_instance(&operation.id, start_instance);
                let snapshot = service.environment_snapshot();
                let startup = &snapshot.startup[&name];
                assert_eq!(startup.kind, kind);
                assert_eq!(
                    startup.estimate_milliseconds,
                    (start_instance || template == "blank").then_some(12_000),
                    "{template}, {start_instance}, {kind:?}"
                );
            }
        }
    }
}
