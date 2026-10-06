use super::*;

#[test]
fn fresh_receipts_keep_attachments_during_zellij_query_failures_and_recover() {
    for failure in ["fail-sessions", "fail-panes", "invalid-panes"] {
        let root = tempfile::tempdir().unwrap();
        let server = Server::start();
        server.busy.store(false, Ordering::Relaxed);
        let observer = observer(root.path());
        presence(&observer, "session.json", "ses_idle", 7, &server.url);
        presence(&observer, "home.json", "", 8, &server.url);
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let roots = ["/work/review".into()];
        let mut snapshot = runtime
            .block_on(observer.observe(&roots, Snapshot::default()))
            .unwrap();
        let attached = snapshot
            .sessions
            .iter()
            .find(|session| session.id == "ses_idle")
            .unwrap()
            .panes
            .clone();
        let home = snapshot.clients[0].pane.clone();
        let panes_path = root.path().join("panes.json");
        let healthy_panes = fs::read_to_string(&panes_path).unwrap();
        if failure == "invalid-panes" {
            fs::write(&panes_path, "invalid JSON").unwrap();
        } else {
            fs::write(root.path().join(failure), "").unwrap();
        }

        for _ in 0..2 {
            snapshot = runtime
                .block_on(observer.observe(&roots, snapshot))
                .unwrap();
            assert!(snapshot.error.is_some(), "{failure}");
            let session = snapshot
                .sessions
                .iter()
                .find(|session| session.id == "ses_idle")
                .unwrap();
            assert_eq!(session.panes, attached, "{failure}");
            assert!(session.stale, "{failure}");
            assert_eq!(snapshot.clients.len(), 1, "{failure}");
            assert_eq!(snapshot.clients[0].pane, home, "{failure}");
            assert!(snapshot.clients[0].stale, "{failure}");
        }

        if failure == "invalid-panes" {
            fs::write(panes_path, healthy_panes).unwrap();
        } else {
            fs::remove_file(root.path().join(failure)).unwrap();
        }
        let recovered = runtime
            .block_on(observer.observe(&roots, snapshot))
            .unwrap();
        assert_eq!(recovered.error, None);
        let session = recovered
            .sessions
            .iter()
            .find(|session| session.id == "ses_idle")
            .unwrap();
        assert_eq!(session.panes, attached);
        assert!(!session.stale);
        assert_eq!(recovered.clients[0].pane, home);
        assert!(!recovered.clients[0].stale);
    }
}

#[test]
fn fresh_route_changes_move_cached_panes_during_inventory_failure() {
    let root = tempfile::tempdir().unwrap();
    let server = Server::start();
    server.busy.store(false, Ordering::Relaxed);
    let observer = observer(root.path());
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let roots = ["/work/review".into()];
    presence(&observer, "one.json", "ses_idle", 7, &server.url);
    let mut snapshot = runtime
        .block_on(observer.observe(&roots, Snapshot::default()))
        .unwrap();
    fs::write(root.path().join("fail-panes"), "").unwrap();

    for id in ["ses_busy", "", "ses_idle"] {
        presence(&observer, "one.json", id, 7, &server.url);
        snapshot = runtime
            .block_on(observer.observe(&roots, snapshot))
            .unwrap();
        let attached = snapshot
            .sessions
            .iter()
            .filter(|session| session.attached())
            .collect::<Vec<_>>();
        if id.is_empty() {
            assert!(attached.is_empty());
            assert_eq!(snapshot.clients.len(), 1);
            assert_eq!(snapshot.clients[0].pane.id, 7);
            assert!(snapshot.clients[0].stale);
        } else {
            assert!(snapshot.clients.is_empty());
            assert_eq!(attached.len(), 1);
            assert_eq!(attached[0].id, id);
            assert_eq!(attached[0].panes[0].id, 7);
            assert!(attached[0].stale);
        }
    }
}

#[test]
fn successful_inventory_removes_closed_attachments_despite_fresh_receipts() {
    for closure in ["empty-panes", "no-sessions"] {
        let root = tempfile::tempdir().unwrap();
        let server = Server::start();
        let observer = observer(root.path());
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let roots = ["/work/review".into()];
        presence(&observer, "one.json", "ses_idle", 7, &server.url);
        presence(&observer, "home.json", "", 8, &server.url);
        let snapshot = runtime
            .block_on(observer.observe(&roots, Snapshot::default()))
            .unwrap();
        if closure == "empty-panes" {
            fs::write(root.path().join("panes.json"), "[]").unwrap();
        } else {
            fs::write(root.path().join(closure), "").unwrap();
        }
        let snapshot = runtime
            .block_on(observer.observe(&roots, snapshot))
            .unwrap();
        assert_eq!(snapshot.error, None, "{closure}");
        assert!(snapshot.sessions.iter().all(|session| !session.attached()));
        assert!(snapshot.clients.is_empty());
    }
}

#[test]
fn expired_receipts_cannot_keep_cached_attachments_during_inventory_failure() {
    let root = tempfile::tempdir().unwrap();
    let server = Server::start();
    let observer = observer(root.path());
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let roots = ["/work/review".into()];
    presence(&observer, "one.json", "ses_idle", 7, &server.url);
    presence(&observer, "home.json", "", 8, &server.url);
    let snapshot = runtime
        .block_on(observer.observe(&roots, Snapshot::default()))
        .unwrap();
    for file in ["one.json", "home.json"] {
        let path = observer.presence.join(file);
        let mut receipt: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        receipt["observed_at"] = json!(0);
        fs::write(path, receipt.to_string()).unwrap();
    }
    fs::write(root.path().join("fail-panes"), "").unwrap();
    let snapshot = runtime
        .block_on(observer.observe(&roots, snapshot))
        .unwrap();
    assert!(snapshot.error.is_some());
    assert!(snapshot.sessions.iter().all(|session| !session.attached()));
    assert!(snapshot.clients.is_empty());
}

#[test]
fn local_client_closure_retires_its_sample_and_preserves_conversation_history() {
    for fresh_receipt in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let server = Server::start();
        server.busy.store(false, Ordering::Relaxed);
        let observer = observer(root.path());
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let roots = ["/work/review".into()];
        presence(&observer, "one.json", "ses_idle", 7, &server.url);
        let snapshot = runtime
            .block_on(observer.observe(&roots, Snapshot::default()))
            .unwrap();
        assert_eq!(snapshot.resources.len(), 1);
        let attached = snapshot
            .sessions
            .iter()
            .find(|session| session.id == "ses_idle")
            .unwrap();
        assert!(attached.attached());
        let last_question = attached.last_question.clone();
        assert!(last_question.is_some());
        server.requests.lock().unwrap().clear();
        fs::write(root.path().join("panes.json"), "[]").unwrap();
        if !fresh_receipt {
            fs::remove_file(observer.presence.join("one.json")).unwrap();
        }

        let snapshot = runtime
            .block_on(observer.observe_changes(&roots, snapshot, false))
            .unwrap();
        let saved = snapshot
            .sessions
            .iter()
            .find(|session| session.id == "ses_idle")
            .unwrap();
        assert!(saved.saved());
        assert_eq!(saved.last_question, last_question);
        assert!(snapshot.resources.is_empty());
        assert!(server.requests.lock().unwrap().is_empty());
    }
}
