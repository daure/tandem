use super::{history_server::Server, *};

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
