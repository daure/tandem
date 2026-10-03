use super::super::events::{Changes, Decoder, LOCAL, Signal};
use super::*;

#[test]
fn native_and_legacy_events_handle_fragmentation_keepalives_and_failure_frames() {
    let mut decoder = Decoder::default();
    assert!(
        !decoder
            .push(b": keepalive\r\n\r\ndata: {\"type\":\"session.ren")
            .unwrap()
    );
    assert!(!decoder.push(b"amed\",\"data\":{\"title\":\"").unwrap());
    assert!(!decoder.push("日本語".as_bytes()).unwrap());
    assert!(decoder.push(b"\"}}\r\n\r\n").unwrap());
    assert!(
        !decoder
            .push(b"data: {\"type\":\"session.message.text.delta\"}\n\n")
            .unwrap()
    );
    assert!(
        decoder
            .push(b"data: {\"payload\":{\"type\":\"session.status\"}}\n\n")
            .unwrap()
    );
    let encoded = json!({"type":"permission.asked"}).to_string();
    assert!(
        decoder
            .push(format!("data: {}\n\n", json!(encoded)).as_bytes())
            .unwrap()
    );
    assert!(
        decoder
            .push(b"event: effect/httpapi/stream/failure\n")
            .is_err()
    );
}

#[test]
fn atomic_receipt_changes_wake_local_observation_without_server_requests() {
    let root = tempfile::tempdir().unwrap();
    let observer = observer(root.path());
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let signal = Arc::new(Signal::default());
        let mut changes = Changes::new(&observer, Arc::clone(&signal)).unwrap();
        assert!(!changes.sync(&observer, &Snapshot::default()).await.unwrap());
        presence(&observer, "one.tmp", "ses_one", 7, "");
        fs::rename(
            observer.presence.join("one.tmp"),
            observer.presence.join("one.json"),
        )
        .unwrap();
        assert_eq!(
            tokio::time::timeout(std::time::Duration::from_secs(2), signal.wait())
                .await
                .unwrap(),
            LOCAL
        );
        assert!(changes.sync(&observer, &Snapshot::default()).await.unwrap());
        let snapshot = observer
            .observe_changes(&[], Snapshot::default(), false)
            .await
            .unwrap();
        assert_eq!(snapshot.sessions[0].id, "ses_one");
        assert_eq!(snapshot.sessions[0].panes[0].id, 7);
        assert!(!changes.sync(&observer, &snapshot).await.unwrap());
        fs::remove_file(observer.presence.join("one.json")).unwrap();
        assert_eq!(
            tokio::time::timeout(std::time::Duration::from_secs(2), signal.wait())
                .await
                .unwrap(),
            LOCAL
        );
        assert!(changes.sync(&observer, &snapshot).await.unwrap());
        let detached = observer
            .observe_changes(&[], snapshot, false)
            .await
            .unwrap();
        assert!(detached.sessions[0].panes.is_empty());
    });
}

#[test]
fn dormant_daemon_records_keep_saved_history_without_live_update_warnings() {
    let root = tempfile::tempdir().unwrap();
    let observer = observer(root.path());
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        // Reserve an unused port without listening so another test cannot claim it.
        let socket = tokio::net::TcpSocket::new_v4().unwrap();
        socket.bind("127.0.0.1:0".parse().unwrap()).unwrap();
        let port = socket.local_addr().unwrap().port();
        let server = format!("http://127.0.0.1:{port}");
        let daemon = observer.daemons.join("dormant");
        fs::create_dir_all(daemon.join("dirs")).unwrap();
        fs::write(daemon.join("port"), port.to_string()).unwrap();
        fs::write(daemon.join("dirs/work.dir"), "/work/review/repo").unwrap();
        let previous = Snapshot {
            sessions: vec![Session {
                id: "ses_saved".into(),
                directory: "/work/review/repo".into(),
                server,
                activity: Activity::Idle,
                ..Default::default()
            }],
            ..Default::default()
        };
        let signal = Arc::new(Signal::default());
        let mut changes = Changes::new(&observer, Arc::clone(&signal)).unwrap();
        changes.sync(&observer, &previous).await.unwrap();
        let snapshot = observer.observe(&[], previous).await.unwrap();
        assert_eq!(snapshot.error, None);
        assert!(snapshot.sessions[0].saved());
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(150), signal.wait())
                .await
                .is_err()
        );
        assert!(signal.errors().is_empty());
        assert!(daemon.join("port").exists());
    });
}

#[test]
fn abandoning_one_conversation_keeps_shared_directory_peers_observable() {
    let root = tempfile::tempdir().unwrap();
    let mut observer = observer(root.path());
    let server = Server::start();
    observer.excluded.sessions.insert("ses_idle".into());
    observer
        .excluded
        .directories
        .insert((server.url.clone(), "/work/review/repo".into()));
    let previous = Snapshot {
        sessions: vec![Session {
            id: "ses_busy".into(),
            directory: "/work/review/repo".into(),
            server: server.url.clone(),
            activity: Activity::Busy,
            ..Default::default()
        }],
        ..Default::default()
    };
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let snapshot = runtime.block_on(observer.observe(&[], previous)).unwrap();
    let peer = snapshot
        .sessions
        .iter()
        .find(|session| session.id == "ses_busy")
        .unwrap();
    assert_eq!(peer.activity, Activity::Busy);
    assert!(!peer.stale);
    assert!(
        !snapshot
            .sessions
            .iter()
            .any(|session| session.id == "ses_idle")
    );
}

#[test]
fn live_clients_and_unfinished_work_keep_disconnect_warnings_until_dormant() {
    for activity in [Activity::Busy, Activity::AwaitingAnswer, Activity::Unknown] {
        let root = tempfile::tempdir().unwrap();
        let observer = observer(root.path());
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let socket = tokio::net::TcpSocket::new_v4().unwrap();
            socket.bind("127.0.0.1:0".parse().unwrap()).unwrap();
            let server = format!("http://{}", socket.local_addr().unwrap());
            let previous = Snapshot {
                sessions: vec![Session {
                    id: "ses_work".into(),
                    directory: "/work/review/repo".into(),
                    server: server.clone(),
                    activity,
                    ..Default::default()
                }],
                ..Default::default()
            };
            let signal = Arc::new(Signal::default());
            let mut changes = Changes::new(&observer, Arc::clone(&signal)).unwrap();
            assert!(changes.sync(&observer, &previous).await.unwrap());
            tokio::time::timeout(std::time::Duration::from_secs(2), signal.wait())
                .await
                .unwrap();
            assert!(signal.errors().contains_key(&server));
            let uncertain = observer.observe(&[], previous).await.unwrap();
            assert!(uncertain.error.is_some());
            assert!(uncertain.sessions[0].stale);
            assert_eq!(uncertain.sessions[0].activity, Activity::Unknown);
            assert!(
                observer
                    .observe(&[], uncertain)
                    .await
                    .unwrap()
                    .error
                    .is_some()
            );

            let mut idle = Snapshot {
                sessions: vec![Session {
                    id: "ses_work".into(),
                    directory: "/work/review/repo".into(),
                    server: server.clone(),
                    activity: Activity::Idle,
                    ..Default::default()
                }],
                ..Default::default()
            };
            assert!(changes.sync(&observer, &idle).await.unwrap());
            assert!(signal.errors().is_empty());
            assert_eq!(
                observer.observe(&[], idle.clone()).await.unwrap().error,
                None
            );

            // A queued permission still needs observation even on an idle session.
            idle.sessions[0].approval_pending = Some(true);
            assert!(changes.sync(&observer, &idle).await.unwrap());
            tokio::time::timeout(std::time::Duration::from_secs(2), signal.wait())
                .await
                .unwrap();
            assert!(signal.errors().contains_key(&server));
            idle.sessions[0].approval_pending = None;
            changes.sync(&observer, &idle).await.unwrap();

            presence(&observer, "client.json", "", 7, &server);
            assert!(changes.sync(&observer, &idle).await.unwrap());
            tokio::time::timeout(std::time::Duration::from_secs(2), async {
                while !signal.errors().contains_key(&server) {
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
            })
            .await
            .unwrap();
            fs::remove_file(observer.presence.join("client.json")).unwrap();
            assert!(changes.sync(&observer, &idle).await.unwrap());
            assert!(signal.errors().is_empty());
        });
    }
}

#[test]
fn deleted_inactive_directories_do_not_require_observation_on_a_live_server() {
    let root = tempfile::tempdir().unwrap();
    let observer = observer(root.path());
    let server = Server::start();
    server.busy.store(false, Ordering::Relaxed);
    let workspace = root.path().join("workspace");
    fs::create_dir(&workspace).unwrap();
    let deleted = root.path().join("deleted");
    let deleted_directory = deleted.to_str().unwrap();
    let daemon = observer.daemons.join("station");
    fs::create_dir_all(daemon.join("dirs")).unwrap();
    fs::write(daemon.join("port"), server.url.rsplit(':').next().unwrap()).unwrap();
    fs::write(daemon.join("dirs/work.dir"), workspace.to_str().unwrap()).unwrap();
    fs::write(daemon.join("dirs/deleted.dir"), deleted_directory).unwrap();
    server
        .failed_question_directories
        .lock()
        .unwrap()
        .insert(deleted_directory.into());
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let previous = Snapshot {
            sessions: vec![Session {
                id: "ses_deleted".into(),
                directory: deleted_directory.into(),
                server: server.url.clone(),
                activity: Activity::Idle,
                ..Default::default()
            }],
            ..Default::default()
        };
        let snapshot = observer.observe(&[], previous.clone()).await.unwrap();
        assert_eq!(snapshot.error, None);
        let (_, servers) = observer.inventory_with_sessions(&previous.sessions);
        assert_eq!(
            servers[&server.url],
            BTreeSet::from([workspace.to_str().unwrap().into()])
        );
        assert!(server.requests.lock().unwrap().iter().all(|request| {
            let target = request.split_whitespace().nth(1).unwrap();
            let url = reqwest::Url::parse(&format!("http://localhost{target}")).unwrap();
            !url.query_pairs()
                .any(|(key, value)| key == "directory" && value == deleted_directory)
        }));
        assert!(daemon.join("dirs/deleted.dir").exists());

        for activity in [Activity::Busy, Activity::AwaitingAnswer, Activity::Unknown] {
            let mut unfinished = previous.clone();
            unfinished.sessions[0].activity = activity;
            let snapshot = observer.observe(&[], unfinished).await.unwrap();
            assert!(snapshot.error.is_some(), "{activity:?}");
            assert_eq!(
                snapshot
                    .observation
                    .unfinished_sessions
                    .contains("ses_deleted"),
                matches!(activity, Activity::Busy | Activity::AwaitingAnswer)
            );
            assert!(
                snapshot
                    .sessions
                    .iter()
                    .any(|session| session.id == "ses_deleted" && session.stale)
            );
        }
        let mut approval = previous.clone();
        approval.sessions[0].approval_pending = Some(true);
        assert!(
            observer
                .observe(&[], approval)
                .await
                .unwrap()
                .error
                .is_some()
        );

        presence_in(
            &observer,
            "client.json",
            "",
            7,
            &server.url,
            deleted_directory,
        );
        assert!(
            observer
                .observe(&[], previous)
                .await
                .unwrap()
                .error
                .is_some()
        );
    });
}

#[test]
fn retained_daemon_records_are_rediscovered_when_their_server_listens() {
    let root = tempfile::tempdir().unwrap();
    let observer = observer(root.path());
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let socket = tokio::net::TcpSocket::new_v4().unwrap();
        socket.bind("127.0.0.1:0".parse().unwrap()).unwrap();
        let port = socket.local_addr().unwrap().port();
        let server = format!("http://127.0.0.1:{port}");
        let daemon = observer.daemons.join("station");
        fs::create_dir_all(daemon.join("dirs")).unwrap();
        fs::write(daemon.join("port"), port.to_string()).unwrap();
        let directory = root.path().to_str().unwrap();
        fs::write(daemon.join("dirs/work.dir"), directory).unwrap();
        let signal = Arc::new(Signal::default());
        let mut changes = Changes::new(&observer, Arc::clone(&signal)).unwrap();
        assert!(!changes.sync(&observer, &Snapshot::default()).await.unwrap());
        assert!(!observer.inventory().1.contains_key(&server));
        let listener = socket.listen(8).unwrap();
        assert!(changes.sync(&observer, &Snapshot::default()).await.unwrap());
        assert_eq!(
            observer.inventory().1[&server],
            BTreeSet::from([directory.to_owned()])
        );
        drop(listener);
        assert!(changes.sync(&observer, &Snapshot::default()).await.unwrap());
        assert!(signal.errors().is_empty());
        assert!(!observer.inventory().1.contains_key(&server));
        assert!(!changes.sync(&observer, &Snapshot::default()).await.unwrap());
    });
}
