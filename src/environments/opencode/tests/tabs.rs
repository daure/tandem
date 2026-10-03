use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[test]
fn tab_creation_and_navigation_use_the_exact_client_and_preserve_the_zellij_pane() {
    for (focus, succeeds) in [(false, true), (false, false), (true, true), (true, false)] {
        let root = tempfile::tempdir().unwrap();
        let observer = observer(root.path());
        let directory = root.path().to_str().unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            presence_in(&observer, "client.json", "ses_one", 7, "http://127.0.0.1:1", directory);
            let receipt = observer.presence.join("client.json");
            let mut value: serde_json::Value = serde_json::from_str(&fs::read_to_string(&receipt).unwrap()).unwrap();
            value["tab_control"] = json!({"server":format!("http://{}", listener.local_addr().unwrap()), "token":"client-secret"});
            if focus {
                let mut background = value.clone();
                background["id"] = json!("ses_background");
                value["tabs"] = json!([background]);
            }
            fs::write(receipt, value.to_string()).unwrap();
            let expected_body = if focus { json!({"sessionID":"ses_background"}) } else { json!({"directory":directory}) }.to_string();
            let controller = tokio::spawn(async move {
                let (mut stream, _) = tokio::time::timeout(std::time::Duration::from_secs(2), listener.accept()).await.expect("client control request").unwrap();
                let mut bytes = Vec::new();
                loop {
                    let mut chunk = [0; 4096];
                    let count = stream.read(&mut chunk).await.unwrap();
                    assert_ne!(count, 0);
                    bytes.extend_from_slice(&chunk[..count]);
                    if bytes.windows(4).any(|window| window == b"\r\n\r\n")
                        && bytes.ends_with(expected_body.as_bytes()) {
                        break;
                    }
                }
                let request = String::from_utf8(bytes).unwrap();
                let path = if focus { "/tabs/focus" } else { "/tabs" };
                assert!(request.starts_with(&format!("POST {path} HTTP/1.1\r\n")), "{request}");
                assert!(request.contains("authorization: Bearer client-secret\r\n"), "{request}");
                let (status, body) = if succeeds {
                    ("200 OK", json!({"id":if focus {"ses_background"} else {"ses_new"}}))
                } else {
                    ("409 Conflict", json!({"error":"Enable OpenCode session tabs before creating a tab"}))
                };
                let body = body.to_string();
                stream.write_all(format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
            });
            let target = Pane { session: "main".into(), id: 7, tab_id: 99, tab_name: "stale".into() };
            let result = tokio::time::timeout(std::time::Duration::from_secs(5), async {
                if focus { observer.jump("ses_background", &target, "other").await.map(|_| None) }
                else { observer.new_session_tab(directory, "other", Some(&target)).await }
            }).await.unwrap();
            controller.await.unwrap();
            let calls = fs::read_to_string(root.path().join("calls")).unwrap();
            assert!(!calls.contains("new-pane") && !calls.contains("new-tab"), "{calls}");
            if succeeds {
                let pane = result.unwrap();
                if !focus {
                    let pane = pane.unwrap();
                    assert_eq!((pane.id, pane.tab_id), (7, 4));
                }
                assert!(calls.contains("--session other action switch-session main --pane-id terminal_7"), "{calls}");
            } else {
                assert!(result.unwrap_err().contains("Enable OpenCode session tabs"));
                assert!(!calls.contains("switch-session"), "{calls}");
            }
        });
    }
}

#[test]
fn workspaces_without_a_tab_capable_client_use_client_launching() {
    let root = tempfile::tempdir().unwrap();
    let observer = observer(root.path());
    let directory = root.path().to_str().unwrap();
    let pane = Pane {
        session: "main".into(),
        id: 7,
        tab_id: 4,
        tab_name: "Review".into(),
    };
    let runtime = tokio::runtime::Runtime::new().unwrap();
    for destination in [None, Some(&pane)] {
        assert_eq!(
            runtime
                .block_on(observer.new_session_tab(directory, "main", destination))
                .unwrap(),
            None
        );
    }
    presence_in(
        &observer,
        "client.json",
        "",
        7,
        "http://127.0.0.1:1",
        directory,
    );
    assert_eq!(
        runtime
            .block_on(observer.new_session_tab(directory, "main", Some(&pane)))
            .unwrap(),
        None
    );
    assert!(!root.path().join("calls").exists());
}

#[test]
fn all_open_session_tabs_attach_to_one_pane_and_share_one_process_sample() {
    let root = tempfile::tempdir().unwrap();
    let observer = observer(root.path());
    let server = Server::start();
    presence(&observer, "client.json", "ses_busy", 7, &server.url);
    let file = observer.presence.join("client.json");
    let mut receipt: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&file).unwrap()).unwrap();
    let mut background = receipt.clone();
    background["id"] = json!("ses_idle");
    background["active"] = json!(false);
    background["tab_index"] = json!(0);
    receipt["tab_index"] = json!(1);
    receipt["tabs"] = json!([background]);
    fs::write(&file, receipt.to_string()).unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let snapshot = runtime
        .block_on(observer.observe(&[], Snapshot::default()))
        .unwrap();
    for id in ["ses_busy", "ses_idle"] {
        let session = snapshot
            .sessions
            .iter()
            .find(|session| session.id == id)
            .unwrap();
        assert_eq!(
            session.panes.iter().map(|pane| pane.id).collect::<Vec<_>>(),
            [7]
        );
        assert!(!session.saved());
        assert_eq!(
            session.tab_position.as_ref().unwrap().index,
            usize::from(id == "ses_busy")
        );
    }
    assert_eq!(snapshot.resources.len(), 1);
    assert_eq!(snapshot.resources[0].session_id, "ses_busy");
    receipt["tabs"] = json!([]);
    fs::write(file, receipt.to_string()).unwrap();
    let snapshot = runtime.block_on(observer.observe(&[], snapshot)).unwrap();
    let background = snapshot
        .sessions
        .iter()
        .find(|session| session.id == "ses_idle")
        .unwrap();
    assert!(background.saved());
}
