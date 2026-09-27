use super::*;

fn starting_observer(root: &Path, server: &Server) -> Observer {
    let observer = observer(root);
    let station = observer.daemons.join("station");
    fs::create_dir_all(station.join("dirs")).unwrap();
    fs::write(station.join("port"), server.url.rsplit(':').next().unwrap()).unwrap();
    fs::write(station.join("dirs/work.dir"), "/work/review\n").unwrap();
    fs::write(root.join("panes.json"), json!([
        {"id":7,"is_plugin":false,"exited":false,"tab_id":4,"tab_name":"Review","pane_command":"opencode attach","pane_cwd":"/work/review/repo"}
    ]).to_string()).unwrap();
    observer
}

#[test]
fn a_starting_pane_keeps_history_saved_and_reconciles_with_its_companion() {
    let root = tempfile::tempdir().unwrap();
    let server = Server::start();
    server.busy.store(false, Ordering::Relaxed);
    let observer = starting_observer(root.path(), &server);
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let roots = ["/work/review".into()];
    let starting = runtime
        .block_on(observer.observe(&roots, Snapshot::default()))
        .unwrap();

    assert_eq!(starting.sessions.len(), 4);
    assert!(starting.sessions.iter().all(Session::saved));
    assert_eq!(starting.error, None);
    assert_eq!(starting.clients.len(), 1);
    assert_eq!(starting.clients[0].title, "OpenCode");
    assert_eq!(starting.clients[0].pane.id, 7);
    assert!(!starting.clients[0].stale);
    assert!(
        runtime
            .block_on(observer.close_pane(&starting.clients[0].pane))
            .is_err()
    );
    assert!(
        runtime
            .block_on(observer.jump_pane(&starting.clients[0].pane, "main"))
            .is_err()
    );

    let since = starting.clients[0].awaiting_presence_since;
    assert!(since.is_some());
    let starting = runtime
        .block_on(observer.observe(&roots, starting))
        .unwrap();
    assert_eq!(starting.clients.len(), 1);
    assert_eq!(starting.clients[0].awaiting_presence_since, since);
    assert!(!starting.clients[0].stale);

    presence(&observer, "one.json", "", 7, &server.url);
    let home = runtime
        .block_on(observer.observe(&roots, starting))
        .unwrap();
    assert_eq!(home.error, None);
    assert_eq!(home.clients.len(), 1);
    assert!(!home.clients[0].stale);
    assert_eq!(home.clients[0].awaiting_presence_since, None);
    assert!(home.sessions.iter().all(Session::saved));

    presence(&observer, "one.json", "ses_idle", 7, &server.url);
    let conversation = runtime.block_on(observer.observe(&roots, home)).unwrap();
    assert_eq!(conversation.error, None);
    assert!(conversation.clients.is_empty());
    let attached = conversation
        .sessions
        .iter()
        .filter(|session| session.attached())
        .collect::<Vec<_>>();
    assert_eq!(attached.len(), 1);
    assert_eq!(attached[0].id, "ses_idle");
    assert!(!attached[0].stale);
}

#[test]
fn saved_history_is_published_without_message_enrichment() {
    let root = tempfile::tempdir().unwrap();
    let server = Server::start();
    server.busy.store(false, Ordering::Relaxed);
    let observer = starting_observer(root.path(), &server);
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let roots = ["/work/review".into()];

    let discovered = runtime
        .block_on(observer.observe(&roots, Snapshot::default()))
        .unwrap();

    assert_eq!(discovered.sessions.len(), 4);
    assert!(discovered.sessions.iter().all(Session::saved));
    assert!(
        discovered
            .sessions
            .iter()
            .all(|session| !session.question_observed)
    );
    assert!(
        server
            .requests
            .lock()
            .unwrap()
            .iter()
            .all(|request| !request.contains("/message?"))
    );

    let refreshed = runtime
        .block_on(observer.observe(&roots, discovered))
        .unwrap();

    assert!(
        refreshed
            .sessions
            .iter()
            .all(|session| !session.question_observed)
    );
    assert!(
        server
            .requests
            .lock()
            .unwrap()
            .iter()
            .all(|request| !request.contains("/message?"))
    );
}

#[test]
fn missing_companion_warnings_stay_on_the_pane_and_clear_when_it_closes() {
    let root = tempfile::tempdir().unwrap();
    let server = Server::start();
    server.busy.store(false, Ordering::Relaxed);
    let observer = starting_observer(root.path(), &server);
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let roots = ["/work/review".into()];
    let mut starting = runtime
        .block_on(observer.observe(&roots, Snapshot::default()))
        .unwrap();
    starting.clients[0].awaiting_presence_since = Some(0);
    let expired = runtime
        .block_on(observer.observe(&roots, starting))
        .unwrap();
    assert!(expired.error.as_ref().unwrap().contains("companion"));
    assert_eq!(expired.clients.len(), 1);
    assert!(expired.clients[0].stale);
    assert!(expired.sessions.iter().all(Session::saved));

    fs::write(root.path().join("panes.json"), "[]").unwrap();
    let closed = runtime.block_on(observer.observe(&roots, expired)).unwrap();
    assert_eq!(closed.error, None);
    assert!(closed.clients.is_empty());
    assert!(closed.sessions.iter().all(Session::saved));
}

#[test]
fn known_clients_losing_their_companion_do_not_receive_startup_grace() {
    let root = tempfile::tempdir().unwrap();
    let server = Server::start();
    server.busy.store(false, Ordering::Relaxed);
    let observer = starting_observer(root.path(), &server);
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let roots = ["/work/review".into()];
    for id in ["", "ses_idle"] {
        presence(&observer, "one.json", id, 7, &server.url);
        let attached = runtime
            .block_on(observer.observe(&roots, Snapshot::default()))
            .unwrap();
        fs::remove_file(observer.presence.join("one.json")).unwrap();
        let missing = runtime
            .block_on(observer.observe(&roots, attached))
            .unwrap();
        assert_eq!(missing.clients.len(), 1);
        assert!(missing.clients[0].stale);
        assert!(missing.error.as_ref().unwrap().contains("companion"));
        assert!(missing.sessions.iter().all(Session::saved));
    }
}

#[test]
fn a_starting_pane_in_an_owned_workspace_is_visible_before_daemon_discovery() {
    let root = tempfile::tempdir().unwrap();
    let server = Server::start();
    let observer = starting_observer(root.path(), &server);
    fs::remove_dir_all(&observer.daemons).unwrap();
    let snapshot = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(observer.observe(&["/work/review".into()], Snapshot::default()))
        .unwrap();
    assert_eq!(snapshot.error, None);
    assert!(snapshot.sessions.is_empty());
    assert_eq!(snapshot.clients.len(), 1);
    assert!(!snapshot.clients[0].stale);
}

#[test]
fn a_route_change_during_server_queries_publishes_only_the_current_pane_identity() {
    let root = tempfile::tempdir().unwrap();
    let server = Server::start();
    let observer = starting_observer(root.path(), &server);
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let roots = ["/work/review".into()];
    for (before, after) in [("", "ses_busy"), ("ses_busy", "")] {
        presence(&observer, "one.json", after, 7, &server.url);
        let path = observer.presence.join("one.json");
        let next_receipt = fs::read_to_string(&path).unwrap();
        presence(&observer, "one.json", before, 7, &server.url);
        *server.receipt_update.lock().unwrap() = Some((path, next_receipt));

        let snapshot = runtime
            .block_on(observer.observe(&roots, Snapshot::default()))
            .unwrap();
        assert_eq!(snapshot.error, None);
        assert_eq!(snapshot.clients.len(), usize::from(after.is_empty()));
        let attached = snapshot
            .sessions
            .iter()
            .filter(|session| session.attached())
            .collect::<Vec<_>>();
        assert_eq!(attached.len(), usize::from(!after.is_empty()));
        if !after.is_empty() {
            assert_eq!(attached[0].id, after);
        }
        assert_eq!(snapshot.resources.len(), 1);
        assert_eq!(snapshot.resources[0].session_id, after);
    }
}
