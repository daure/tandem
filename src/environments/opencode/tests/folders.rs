use super::*;

#[test]
fn observations_remember_a_workspace_after_its_empty_client_closes() {
    let root = tempfile::tempdir().unwrap();
    let observer = observer(root.path());
    presence_in(&observer, "empty.json", "", 7, "", "/work/empty");
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let attached = runtime
        .block_on(observer.observe(&[], Snapshot::default()))
        .unwrap();
    assert_eq!(attached.clients.len(), 1);
    assert_eq!(attached.directories, ["/work/empty"]);
    fs::remove_file(observer.presence.join("empty.json")).unwrap();
    fs::write(root.path().join("panes.json"), "[]").unwrap();
    let closed = runtime.block_on(observer.observe(&[], attached)).unwrap();
    assert!(closed.clients.is_empty());
    assert!(closed.sessions.is_empty());
    assert_eq!(closed.directories, ["/work/empty"]);
}

#[test]
fn clearing_a_directory_removes_its_complete_history_and_receipts_and_preserves_files() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    for native in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let observer = observer(root.path());
        let server = history_server::Server::start();
        server.data.lock().unwrap().v2 = native;
        let workspace = root.path().join("external");
        fs::create_dir(&workspace).unwrap();
        fs::write(workspace.join("keep.txt"), "workspace data").unwrap();
        let directory = workspace.to_str().unwrap();
        let other = root.path().join("external-other");
        fs::create_dir(&other).unwrap();
        server.session("ses_old", directory, None);
        server.session("ses_child", directory, Some("ses_old"));
        server.session("ses_other", other.to_str().unwrap(), None);
        let station = observer.daemons.join("station");
        fs::create_dir_all(station.join("dirs")).unwrap();
        fs::write(station.join("port"), server.url.rsplit(':').next().unwrap()).unwrap();
        fs::write(station.join("dirs/external.dir"), directory).unwrap();
        fs::write(station.join("dirs/other.dir"), other.to_str().unwrap()).unwrap();
        let outcome = runtime.block_on(observer.clear_directory(directory, Vec::new()));
        assert!(outcome.error.is_none(), "{:?}", outcome.error);
        let removed = outcome.removed;
        assert_eq!(
            removed,
            [
                (server.url.clone(), "ses_old".into()),
                (server.url.clone(), "ses_child".into())
            ]
        );
        assert_eq!(server.data.lock().unwrap().deleted, ["ses_old"]);
        assert_eq!(
            fs::read_to_string(workspace.join("keep.txt")).unwrap(),
            "workspace data"
        );
        assert!(!station.join("dirs/external.dir").exists());
        assert!(station.join("dirs/other.dir").exists());
        assert!(station.join("port").exists());
        assert_eq!(
            server
                .data
                .lock()
                .unwrap()
                .sessions
                .keys()
                .cloned()
                .collect::<Vec<_>>(),
            ["ses_other"]
        );
        let observed = runtime
            .block_on(observer.observe(&[], Snapshot::default()))
            .unwrap();
        assert!(!observed.directories.iter().any(|path| path == directory));
        presence_in(&observer, "reopened.json", "", 7, &server.url, directory);
        let reopened = runtime.block_on(observer.observe(&[], observed)).unwrap();
        assert!(reopened.directories.iter().any(|path| path == directory));
    }
}

#[test]
fn clearing_a_folder_preserves_receipts_and_history_when_work_is_pending() {
    let root = tempfile::tempdir().unwrap();
    let observer = observer(root.path());
    let server = history_server::Server::start();
    server.session("ses_old", "/work/review", None);
    {
        let mut data = server.data.lock().unwrap();
        data.v2 = true;
        data.queued = true;
    }
    let station = observer.daemons.join("station");
    fs::create_dir_all(station.join("dirs")).unwrap();
    fs::write(station.join("port"), server.url.rsplit(':').next().unwrap()).unwrap();
    fs::write(station.join("dirs/review.dir"), "/work/review").unwrap();
    assert!(
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(observer.clear_directory("/work/review", Vec::new()))
            .error
            .is_some()
    );
    assert!(station.join("dirs/review.dir").exists());
    assert!(server.data.lock().unwrap().deleted.is_empty());
}

#[test]
fn daemon_directory_receipts_expose_workspaces_with_no_conversations() {
    let root = tempfile::tempdir().unwrap();
    let server = Server::start();
    server.busy.store(false, Ordering::Relaxed);
    let observer = observer(root.path());
    let station = observer.daemons.join("station");
    let workspace = root.path().join("empty");
    fs::create_dir(&workspace).unwrap();
    fs::create_dir_all(station.join("dirs")).unwrap();
    fs::write(station.join("port"), server.url.rsplit(':').next().unwrap()).unwrap();
    fs::write(station.join("dirs/empty.dir"), workspace.to_str().unwrap()).unwrap();
    let snapshot = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(observer.observe(&[], Snapshot::default()))
        .unwrap();
    assert!(snapshot.sessions.is_empty());
    assert!(snapshot.clients.is_empty());
    assert_eq!(snapshot.directories, [workspace.to_str().unwrap()]);
}
