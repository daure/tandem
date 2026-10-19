use super::{history_server::Server, *};

#[test]
fn detached_sessions_are_verified_when_workspace_observation_fails() {
    for v2 in [false, true] {
        for outcome in ["deleted", "present", "unavailable"] {
            let root = tempfile::tempdir().unwrap();
            let observer = observer(root.path());
            let server = Server::start();
            let directory = root.path().join("missing").display().to_string();
            {
                let mut data = server.data.lock().unwrap();
                data.v2 = v2;
                data.status_failure = true;
                data.history_failure = true;
                data.session_failure = outcome == "unavailable";
            }
            if outcome == "present" {
                server.session("ses_old", &directory, None);
            }
            let previous = Snapshot {
                sessions: vec![Session {
                    id: "ses_old".into(),
                    directory: directory.clone(),
                    server: server.url.clone(),
                    activity: Activity::Busy,
                    ..Default::default()
                }],
                ..Default::default()
            };
            let snapshot = tokio::runtime::Runtime::new()
                .unwrap()
                .block_on(observer.observe_changes(&[], previous, true))
                .unwrap();
            assert!(snapshot.missing_directory(&directory));
            assert!(snapshot.error.is_some());
            if outcome == "deleted" {
                assert!(snapshot.sessions.is_empty(), "v2={v2}");
                assert!(snapshot.observation.unfinished_sessions.is_empty());
                assert_eq!(
                    snapshot.observation.deleted_sessions,
                    BTreeSet::from([(server.url.clone(), "ses_old".into())])
                );
            } else {
                assert_eq!(snapshot.sessions.len(), 1, "v2={v2}: {outcome}");
                assert_eq!(snapshot.sessions[0].id, "ses_old");
                assert!(snapshot.sessions[0].stale);
                assert_eq!(snapshot.sessions[0].activity, Activity::Unknown);
                assert!(snapshot.sessions[0].panes.is_empty());
                assert!(snapshot.observation.unfinished_sessions.contains("ses_old"));
                assert!(snapshot.observation.deleted_sessions.is_empty());
            }
            assert!(server.data.lock().unwrap().deleted.is_empty());
        }
    }
}

#[test]
fn known_detached_history_outside_the_recent_window_is_verified_by_id() {
    for session_failure in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let observer = observer(root.path());
        let server = Server::start();
        {
            let mut data = server.data.lock().unwrap();
            data.v2 = true;
            data.session_failure = session_failure;
        }
        let directory = root.path().display().to_string();
        for index in 0..SESSION_DIRECTORY_WINDOW {
            server.session(&format!("ses_{index:03}"), &directory, None);
        }
        server.session("ses_old", &directory, None);
        let previous = Snapshot {
            sessions: vec![Session {
                id: "ses_old".into(),
                directory,
                server: server.url.clone(),
                activity: Activity::Busy,
                ..Default::default()
            }],
            ..Default::default()
        };
        let snapshot = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(observer.observe_changes(&[], previous, true))
            .unwrap();
        let session = snapshot
            .sessions
            .iter()
            .find(|session| session.id == "ses_old")
            .unwrap();
        assert_eq!(session.stale, session_failure);
        assert!(snapshot.observation.deleted_sessions.is_empty());
    }
}

#[test]
fn idle_missing_workspaces_allow_identity_checks_without_directory_reads() {
    for v2 in [false, true] {
        for present in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let observer = observer(root.path());
            let server = Server::start();
            {
                let mut data = server.data.lock().unwrap();
                data.v2 = v2;
                data.status_failure = true;
                data.history_failure = true;
            }
            let directory = root.path().join("missing").display().to_string();
            if present {
                server.session("ses_old", &directory, None);
            }
            let previous = Snapshot {
                sessions: vec![Session {
                    id: "ses_old".into(),
                    server: server.url.clone(),
                    directory,
                    activity: Activity::Idle,
                    ..Default::default()
                }],
                ..Default::default()
            };
            let snapshot = tokio::runtime::Runtime::new()
                .unwrap()
                .block_on(observer.observe_changes(&[], previous, true))
                .unwrap();
            assert_eq!(snapshot.error, None);
            assert_eq!(snapshot.sessions.len(), usize::from(present));
            assert_eq!(snapshot.observation.deleted_sessions.is_empty(), present);
        }
    }
}
