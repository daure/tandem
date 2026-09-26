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
        run(connection, scopes, receiver, |targets, manual| {
            assert!(!manual);
            observed.send(targets).unwrap();
            Ok(())
        })
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
        run(connection, scopes, receiver, |targets, manual| {
            assert!(!manual);
            if first {
                first = false;
                notifier.publish_result(Refresh::Templates).unwrap();
            }
            observed.send(targets).unwrap();
            Ok(())
        });
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
        run(connection, notifier.scopes, receiver, |targets, manual| {
            entered.send((targets, manual)).unwrap();
            released.recv_timeout(Duration::from_secs(3)).unwrap()
        });
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
        .block_on(writer.configure_open_command("editor .".into(), true))
        .unwrap();
    wait_for(|| observer.open_command() == "editor .");
    writer
        .runtime
        .block_on(writer.configure_close_command("close-editor".into(), true))
        .unwrap();
    wait_for(|| observer.close_command() == "close-editor");
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
