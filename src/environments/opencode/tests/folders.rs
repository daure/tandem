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
