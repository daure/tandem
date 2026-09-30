use super::*;
use crate::{service::AppService, store::environments::Manifest};

fn fixture() -> (tempfile::TempDir, Connection, RefreshNotifier) {
    let directory = tempfile::tempdir().unwrap();
    let notifier = RefreshNotifier {
        database: directory.path().join("settings.sqlite3"),
        scopes: [
            "templates".into(),
            "instances:test".into(),
            "settings".into(),
        ],
    };
    let connection = Connection::open(&notifier.database).unwrap();
    connection.execute_batch(SCHEMA).unwrap();
    (directory, connection, notifier)
}

#[test]
fn external_changes_refresh_only_their_domain_without_an_inventory_poll() {
    let (_directory, connection, notifier) = fixture();
    let (requests, receiver) = mpsc::channel();
    let (observed, results) = mpsc::channel();
    let scopes = notifier.scopes.clone();
    let worker = thread::spawn(move || {
        run(
            connection,
            scopes,
            receiver,
            || {},
            |targets, manual| {
                assert!(!manual);
                observed.send(targets).unwrap();
                Ok(())
            },
        )
    });
    requests.send(Refresh::All.into()).unwrap();
    assert_eq!(
        results.recv_timeout(Duration::from_secs(3)).unwrap(),
        [true; 3]
    );
    for change in [Refresh::Templates, Refresh::Instances, Refresh::Settings] {
        notifier.publish_result(change).unwrap();
        assert_eq!(
            results.recv_timeout(Duration::from_secs(3)).unwrap(),
            change.targets()
        );
    }
    drop(requests);
    worker.join().unwrap();
}

#[test]
fn changes_during_a_refresh_are_picked_up_on_the_next_check() {
    let (_directory, connection, notifier) = fixture();
    let (requests, receiver) = mpsc::channel();
    let (observed, results) = mpsc::channel();
    let scopes = notifier.scopes.clone();
    let worker = thread::spawn(move || {
        let mut first = true;
        run(
            connection,
            scopes,
            receiver,
            || {},
            |targets, manual| {
                assert!(!manual);
                if first {
                    first = false;
                    notifier.publish_result(Refresh::Templates).unwrap();
                }
                observed.send(targets).unwrap();
                Ok(())
            },
        );
    });
    requests.send(Refresh::All.into()).unwrap();
    assert_eq!(
        results.recv_timeout(Duration::from_secs(3)).unwrap(),
        [true; 3]
    );
    assert_eq!(
        results.recv_timeout(Duration::from_secs(3)).unwrap(),
        [true, false, false]
    );
    drop(requests);
    worker.join().unwrap();
}

#[test]
fn manual_completion_waits_for_its_own_refresh_and_returns_errors_to_coalesced_callers() {
    let (_directory, connection, notifier) = fixture();
    let (requests, receiver) = mpsc::channel();
    let (entered, entries) = mpsc::channel();
    let (release, released) = mpsc::channel();
    let worker = thread::spawn(move || {
        run(
            connection,
            notifier.scopes,
            receiver,
            || {},
            |targets, manual| {
                entered.send((targets, manual)).unwrap();
                released.recv_timeout(Duration::from_secs(3)).unwrap()
            },
        );
    });
    requests.send(Refresh::Instances.into()).unwrap();
    assert_eq!(
        entries.recv_timeout(Duration::from_secs(3)).unwrap(),
        ([false, true, false], false)
    );
    let mut completions = Vec::new();
    for _ in 0..2 {
        let (sender, mut completion) = oneshot::channel();
        requests
            .send(RefreshRequest {
                refresh: Refresh::All,
                completion: Some(sender),
            })
            .unwrap();
        assert_eq!(
            completion.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        );
        completions.push(completion);
    }
    release.send(Ok(())).unwrap();
    assert_eq!(
        entries.recv_timeout(Duration::from_secs(3)).unwrap(),
        ([true; 3], true)
    );
    for completion in &mut completions {
        assert_eq!(
            completion.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        );
    }
    release.send(Err("Docker unavailable".into())).unwrap();
    for completion in completions {
        assert_eq!(
            completion.blocking_recv().unwrap(),
            Err("Docker unavailable".into())
        );
    }
    drop(requests);
    worker.join().unwrap();
}

#[test]
fn runtime_invalidations_are_namespaced_and_concurrent_writers_preserve_changes() {
    let (_directory, connection, notifier) = fixture();
    thread::scope(|scope| {
        for _ in 0..4 {
            let notifier = &notifier;
            scope.spawn(move || notifier.publish_result(Refresh::Instances).unwrap());
        }
    });
    assert_eq!(revisions(&connection, &notifier.scopes).unwrap(), [0, 4, 0]);
    let mut other = notifier.clone();
    other.scopes[1] = "instances:other".into();
    assert_eq!(revisions(&connection, &other.scopes).unwrap(), [0; 3]);
    other.publish_result(Refresh::Inventory).unwrap();
    assert_eq!(revisions(&connection, &notifier.scopes).unwrap(), [1, 4, 0]);
    assert_eq!(revisions(&connection, &other.scopes).unwrap(), [1, 1, 0]);
}

fn wait_for(mut condition: impl FnMut() -> bool) {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !condition() {
        assert!(
            std::time::Instant::now() < deadline,
            "observer did not receive the change"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn service_mutations_update_an_independent_observer_and_rejected_manifests_do_not_invalidate() {
    let writer = AppService::for_tests();
    let observer = AppService::from_config(writer.environments.config.clone()).unwrap();
    observer.refresh.request(Refresh::Templates);
    writer
        .runtime
        .block_on(writer.create_template("website".into()))
        .unwrap();
    wait_for(|| observer.environment_snapshot().templates.len() == 1);

    let manifest = Manifest {
        description: "Changed by MCP".into(),
        ..Default::default()
    };
    writer
        .runtime
        .block_on(writer.update_template_manifest("website".into(), manifest, true))
        .unwrap();
    wait_for(|| {
        observer.environment_snapshot().templates[0]
            .manifest
            .description
            == "Changed by MCP"
    });

    let connection = Connection::open(&writer.refresh.notifier.database).unwrap();
    let before = revisions(&connection, &writer.refresh.notifier.scopes).unwrap();
    assert!(
        writer
            .runtime
            .block_on(writer.update_template_manifest("website".into(), Manifest::default(), false))
            .is_err()
    );
    assert_eq!(
        revisions(&connection, &writer.refresh.notifier.scopes).unwrap(),
        before
    );

    writer
        .runtime
        .block_on(writer.set_completion_fade_seconds("45".into()).unwrap())
        .unwrap()
        .unwrap();
    wait_for(|| observer.completion_fade_seconds() == 45);
}

#[test]
fn manual_refresh_rereads_external_template_edits() {
    let service = AppService::for_tests();
    let template = service
        .runtime
        .block_on(service.create_template("website".into()))
        .unwrap();
    service.environments.refresh_templates();
    let original = service.environment_snapshot().templates[0]
        .compose_source
        .clone();
    std::fs::write(
        std::path::Path::new(&template.directory).join("compose.yaml"),
        "services: {}\n",
    )
    .unwrap();
    assert_eq!(
        service.environment_snapshot().templates[0].compose_source,
        original
    );
    service.refresh.request(Refresh::Templates);
    wait_for(|| service.environment_snapshot().templates[0].compose_source == "services: {}\n");
}

#[test]
fn template_observer_tracks_disk_edits_replacements_and_optional_file_removal() {
    let service = AppService::for_tests();
    let template = service
        .runtime
        .block_on(service.create_template("website".into()))
        .unwrap();
    service.refresh.request(Refresh::Templates);
    wait_for(|| service.environment_snapshot().templates.len() == 1);
    let directory = std::path::Path::new(&template.directory);
    std::fs::write(directory.join("compose.yaml"), "services: {}\n").unwrap();
    std::fs::write(directory.join("tandem-agents.md"), "# Disk guidance\n").unwrap();
    let replacement = directory.join("manifest.tmp");
    std::fs::write(&replacement, "{\"description\":\"Disk manifest\"}\n").unwrap();
    std::fs::rename(replacement, directory.join("tandem.json")).unwrap();
    wait_for(|| {
        let snapshot = service.environment_snapshot();
        let template = &snapshot.templates[0];
        template.compose_source == "services: {}\n"
            && template.guidance_source.as_deref() == Some("# Disk guidance\n")
            && template.manifest.description == "Disk manifest"
    });
    std::fs::write(directory.join("tandem.json"), "{invalid").unwrap();
    wait_for(|| {
        let snapshot = service.environment_snapshot();
        snapshot.templates[0].manifest_source.as_deref() == Some("{invalid")
            && snapshot.templates[0].error.is_some()
    });
    std::fs::remove_file(directory.join("compose.yaml")).unwrap();
    std::fs::remove_file(directory.join("tandem-agents.md")).unwrap();
    std::fs::write(directory.join("tandem.json"), "{}\n").unwrap();
    wait_for(|| {
        let snapshot = service.environment_snapshot();
        let template = &snapshot.templates[0];
        template.compose_file.is_empty()
            && template.guidance_source.is_none()
            && template.error.is_none()
    });
}

#[test]
fn template_observer_tracks_seed_file_edits_nested_entries_and_folder_removal() {
    let service = AppService::for_tests();
    let template = service
        .runtime
        .block_on(service.create_template("website".into()))
        .unwrap();
    service.refresh.request(Refresh::Templates);
    wait_for(|| service.environment_snapshot().templates.len() == 1);
    let files = std::path::Path::new(&template.directory).join("tandem-files");
    std::fs::create_dir_all(files.join("nested")).unwrap();
    std::fs::write(files.join("nested/file.txt"), "first").unwrap();
    wait_for(|| service.environment_snapshot().templates[0].files.len() == 2);
    let initial = service.environment_snapshot().templates[0].files.clone();
    std::fs::write(files.join("nested/file.txt"), "updated content").unwrap();
    wait_for(|| service.environment_snapshot().templates[0].files != initial);
    let updated = service.environment_snapshot().templates[0].files.clone();
    assert_eq!(updated[1].size_bytes, 15);
    std::fs::write(files.join("nested/file.txt"), "changed content").unwrap();
    let stamp = std::time::UNIX_EPOCH + Duration::from_secs(1_000_000);
    std::fs::File::options()
        .write(true)
        .open(files.join("nested/file.txt"))
        .unwrap()
        .set_modified(stamp)
        .unwrap();
    wait_for(|| service.environment_snapshot().templates[0].files != updated);
    std::fs::rename(
        files.join("nested/file.txt"),
        files.join("nested/renamed.txt"),
    )
    .unwrap();
    wait_for(|| service.environment_snapshot().templates[0].files[1].path == "nested/renamed.txt");
    std::fs::remove_file(files.join("nested/renamed.txt")).unwrap();
    wait_for(|| service.environment_snapshot().templates[0].files.len() == 1);
    std::fs::remove_dir_all(files).unwrap();
    wait_for(|| {
        let snapshot = service.environment_snapshot();
        snapshot.templates[0].files_directory.is_none() && snapshot.templates[0].files.is_empty()
    });
}
