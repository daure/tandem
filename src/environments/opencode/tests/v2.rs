use super::{history_server::Server, *};

#[test]
fn native_observation_reads_closed_history_without_booting_locations() {
    let root = tempfile::tempdir().unwrap();
    let observer = observer(root.path());
    let server = Server::start();
    server.data.lock().unwrap().v2 = true;
    let station = observer.daemons.join("shared");
    fs::create_dir_all(station.join("dirs")).unwrap();
    fs::write(station.join("port"), server.url.rsplit(':').next().unwrap()).unwrap();
    let mut previous = Snapshot::default();
    for index in 0..22 {
        let directory = root.path().join(format!("workspace-{index}"));
        fs::create_dir(&directory).unwrap();
        let directory = directory.to_str().unwrap();
        fs::write(station.join(format!("dirs/{index}.dir")), directory).unwrap();
        let id = format!("ses_closed_{index}");
        server.session(&id, directory, None);
        previous.sessions.push(Session {
            id,
            directory: directory.into(),
            server: server.url.clone(),
            activity: Activity::Idle,
            approval_pending: Some(false),
            ..Default::default()
        });
    }
    let old = previous.sessions[0].clone();
    for index in 0..SESSION_DIRECTORY_WINDOW {
        server.session(&format!("ses_ancient_{index}"), &old.directory, None);
    }
    previous.sessions.push(Session {
        id: "ses_deleted".into(),
        directory: old.directory.clone(),
        server: server.url.clone(),
        activity: Activity::Idle,
        approval_pending: Some(false),
        ..Default::default()
    });
    let missing = root.path().join("removed").to_str().unwrap().to_owned();
    server.session("ses_missing", &missing, None);
    previous.sessions.push(Session {
        id: "ses_missing".into(),
        directory: missing,
        server: server.url.clone(),
        activity: Activity::Idle,
        approval_pending: Some(false),
        ..Default::default()
    });
    let runtime = tokio::runtime::Runtime::new().unwrap();
    for pass in 0..2 {
        server.data.lock().unwrap().requests.clear();
        let snapshot = runtime
            .block_on(observer.observe_changes(&[], previous, true))
            .unwrap();
        assert_eq!(snapshot.error, None);
        assert!(snapshot.sessions.iter().any(|session| session.id == old.id));
        assert!(
            !snapshot
                .sessions
                .iter()
                .any(|session| session.id == "ses_deleted")
        );
        assert!(
            snapshot
                .observation
                .deleted_sessions
                .contains(&(server.url.clone(), "ses_deleted".into()))
        );
        let data = server.data.lock().unwrap();
        assert_eq!(
            data.requests
                .iter()
                .filter(|path| *path == "/api/session/active")
                .count(),
            1
        );
        assert!(data.requests.contains(&format!("/api/session/{}", old.id)));
        if pass == 0 {
            assert!(data.requests.contains(&"/api/session/ses_missing".into()));
        }
        assert!(
            data.requests.iter().all(|path| {
                path == "/api/info"
                    || path == "/api/session/active"
                    || path.starts_with("/api/session?")
                    || (path.starts_with("/api/session/")
                        && !path.ends_with("/form")
                        && !path.ends_with("/permission"))
            }),
            "{:?}",
            data.requests
        );
        previous = snapshot;
    }
}

#[test]
fn native_observation_reads_forms_and_approvals_only_for_attached_or_running_sessions() {
    for attached in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let observer = observer(root.path());
        let server = Server::start();
        let directory = root.path().join("active");
        fs::create_dir(&directory).unwrap();
        let directory = directory.to_str().unwrap();
        server.session("ses_old", directory, None);
        server.session("ses_closed", directory, None);
        {
            let mut data = server.data.lock().unwrap();
            data.v2 = true;
            data.busy = !attached;
            data.awaiting_answer = true;
            data.approval = true;
        }
        let previous = Snapshot {
            sessions: vec![Session {
                id: "ses_old".into(),
                directory: directory.into(),
                server: server.url.clone(),
                activity: Activity::Idle,
                approval_pending: Some(false),
                ..Default::default()
            }],
            ..Default::default()
        };
        if attached {
            presence_in(
                &observer,
                "client.json",
                "ses_old",
                7,
                &server.url,
                directory,
            );
        }
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let snapshot = runtime
            .block_on(observer.observe_changes(&[], previous, true))
            .unwrap();
        assert_eq!(snapshot.error, None);
        let session = snapshot
            .sessions
            .iter()
            .find(|session| session.id == "ses_old")
            .unwrap();
        assert_eq!(session.activity, Activity::AwaitingAnswer);
        assert_eq!(session.approval_pending, Some(true));
        let data = server.data.lock().unwrap();
        assert_eq!(
            data.requests
                .iter()
                .filter(|path| path.ends_with("/form"))
                .collect::<Vec<_>>(),
            ["/api/session/ses_old/form"]
        );
        assert_eq!(
            data.requests
                .iter()
                .filter(|path| path.ends_with("/permission"))
                .collect::<Vec<_>>(),
            ["/api/session/ses_old/permission"]
        );
        assert!(!data.requests.iter().any(|path| path.contains("location")));
    }
}

#[test]
fn native_observation_preserves_uncertainty_when_activity_or_pending_requests_fail() {
    for failure in ["activity", "form", "permission", "identity"] {
        let root = tempfile::tempdir().unwrap();
        let observer = observer(root.path());
        let server = Server::start();
        let directory = root.path().to_str().unwrap();
        server.session("ses_old", directory, None);
        presence_in(
            &observer,
            "client.json",
            "ses_old",
            7,
            &server.url,
            directory,
        );
        {
            let mut data = server.data.lock().unwrap();
            data.v2 = true;
            data.busy = true;
            data.status_failure = failure == "activity";
            data.form_failure = failure == "form";
            data.permission_failure = failure == "permission";
            data.session_failure = failure == "identity";
        }
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let snapshot = runtime
            .block_on(observer.observe_changes(&[], Snapshot::default(), true))
            .unwrap();
        assert!(snapshot.error.is_some(), "{failure}");
        let session = snapshot
            .sessions
            .iter()
            .find(|session| session.id == "ses_old")
            .unwrap();
        if failure == "permission" {
            assert_eq!(session.activity, Activity::Busy);
            assert_eq!(session.approval_pending, None);
        } else {
            assert_eq!(session.activity, Activity::Unknown);
            assert!(session.stale);
        }
    }
}

#[test]
fn native_history_cleanup_checks_children_approvals_queued_work_and_no_content_deletion() {
    for guard in [
        "idle", "busy", "question", "approval", "queued", "child", "status",
    ] {
        let root = tempfile::tempdir().unwrap();
        let server = Server::start();
        server.data.lock().unwrap().v2 = true;
        let observer = observer(root.path());
        let state = observer.daemons.join("native");
        fs::create_dir_all(state.join("dirs")).unwrap();
        fs::write(state.join("port"), server.url.rsplit(':').next().unwrap()).unwrap();
        fs::write(state.join("dirs/work.dir"), "/work/review\n").unwrap();
        server.session("ses_old", "/work/review", None);
        server.session("ses_child", "/work/review", Some("ses_old"));
        server.session("ses_elsewhere", "/work/elsewhere", None);
        {
            let mut data = server.data.lock().unwrap();
            data.busy = guard == "busy";
            data.awaiting_answer = guard == "question";
            data.approval = guard == "approval";
            data.queued = guard == "queued";
            data.status_failure = guard == "status";
        }
        if guard == "child" {
            server.session("ses_foreign_child", "/work/elsewhere", Some("ses_old"));
        }
        let result = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(super::super::history::clear_with(&observer, "/work/review"));
        let data = server.data.lock().unwrap();
        if guard == "idle" {
            result.unwrap();
            assert_eq!(data.deleted, ["ses_old"]);
            assert_eq!(
                data.sessions.keys().map(String::as_str).collect::<Vec<_>>(),
                ["ses_elsewhere"]
            );
        } else {
            assert!(result.is_err(), "{guard}");
            assert!(data.deleted.is_empty(), "{guard}");
        }
    }
}

#[test]
fn native_inventory_follows_opaque_cursors_and_projects_complete_transcripts() {
    let server = Server::start();
    server.data.lock().unwrap().v2 = true;
    for index in 0..205 {
        server.session(&format!("ses_{index:04}"), "/work/review", None);
    }
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let client = transport::client().unwrap();
    let sessions: Vec<super::super::RemoteSession> = runtime
        .block_on(transport::get(
            &client,
            &server.url,
            "/experimental/session?directory=%2Fwork%2Freview&limit=10000",
        ))
        .unwrap();
    assert_eq!(sessions.len(), 205);
    assert!(
        sessions
            .iter()
            .all(|session| session.directory == "/work/review")
    );
    let session = Session {
        id: "ses_0000".into(),
        directory: "/work/review".into(),
        server: server.url.clone(),
        ..Default::default()
    };
    let transcript = runtime
        .block_on(super::super::conversation(&session))
        .unwrap();
    assert!(transcript.contains("Native question"));
    assert!(transcript.contains("Native answer"));
    assert!(transcript.contains("### Reasoning\n\nNative reasoning"));
    assert!(transcript.contains("Tool: shell (completed)"));
}
