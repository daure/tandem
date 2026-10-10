use super::*;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[test]
fn tab_actions_use_the_exact_client_and_preserve_the_zellij_pane() {
    for (action, succeeds) in ["create", "focus", "close"]
        .into_iter()
        .flat_map(|action| [(action, true), (action, false)])
    {
        let focus = action == "focus";
        let close = action == "close";
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
            if focus || close {
                let mut background = value.clone();
                background["id"] = json!("ses_background");
                value["tabs"] = json!([background]);
            }
            fs::write(&receipt, value.to_string()).unwrap();
            let expected_body = if focus || close { json!({"sessionID":"ses_background"}) } else { json!({"directory":directory, "instructions":"Services won't start automatically"}) }.to_string();
            let closed_receipt = receipt.clone();
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
                let path = if focus { "/tabs/focus" } else if close { "/tabs/close" } else { "/tabs" };
                assert!(request.starts_with(&format!("POST {path} HTTP/1.1\r\n")), "{request}");
                assert!(request.contains("authorization: Bearer client-secret\r\n"), "{request}");
                let (status, body) = if succeeds {
                    ("200 OK", json!({"id":if focus || close {"ses_background"} else {"ses_new"}}))
                } else {
                    ("409 Conflict", json!({"error":"Enable OpenCode session tabs before creating a tab"}))
                };
                let body = body.to_string();
                if close && succeeds {
                    let mut value: serde_json::Value = serde_json::from_str(&fs::read_to_string(&closed_receipt).unwrap()).unwrap();
                    value["tabs"].as_array_mut().unwrap().retain(|tab| tab["id"] != "ses_background");
                    fs::write(closed_receipt, value.to_string()).unwrap();
                }
                stream.write_all(format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
            });
            let target = Pane { session: "main".into(), id: 7, tab_id: 99, tab_name: "stale".into() };
            let result = tokio::time::timeout(std::time::Duration::from_secs(5), async {
                if focus { observer.jump("ses_background", &target, "other").await.map(|_| None) }
                else if close { observer.close("ses_background", &target).await.map(|_| None) }
                else { observer.new_session_tab(directory, "other", Some(&target), Some("Services won't start automatically")).await }
            }).await.unwrap();
            controller.await.unwrap();
            let calls = fs::read_to_string(root.path().join("calls")).unwrap();
            assert!(!calls.contains("new-pane") && !calls.contains("new-tab"), "{calls}");
            assert!(!calls.contains("close-pane"), "{calls}");
            if succeeds {
                let pane = result.unwrap();
                if !focus && !close {
                    let location = pane.unwrap();
                    assert_eq!((location.pane.id, location.pane.tab_id), (7, 4));
                    assert_eq!(location.session_id.as_deref(), Some("ses_new"));
                }
                if close {
                    assert!(!calls.contains("switch-session"), "{calls}");
                } else {
                    assert!(calls.contains("--session other action switch-session main --pane-id terminal_7"), "{calls}");
                }
            } else {
                assert!(result.unwrap_err().contains("Enable OpenCode session tabs"));
                assert!(!calls.contains("switch-session"), "{calls}");
            }
        });
    }
}

#[test]
fn closing_the_last_native_session_closes_its_client_pane_once() {
    for (home, succeeds) in [false, true]
        .into_iter()
        .flat_map(|home| [false, true].map(|succeeds| (home, succeeds)))
    {
        let root = tempfile::tempdir().unwrap();
        let observer = observer(root.path());
        presence(
            &observer,
            "client.json",
            "ses_last",
            7,
            "http://127.0.0.1:1",
        );
        let receipt = observer.presence.join("client.json");
        let mut value: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&receipt).unwrap()).unwrap();
        value["tab_index"] = json!(0);
        if home {
            let tab = value.clone();
            value["id"] = json!("");
            value.as_object_mut().unwrap().remove("tab_index");
            value["tabs"] = json!([tab]);
        }
        let target = Pane {
            session: "main".into(),
            id: 7,
            tab_id: 4,
            tab_name: "Review".into(),
        };
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let result = runtime.block_on(async {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            value["tab_control"] = json!({"server":format!("http://{}", listener.local_addr().unwrap()), "token":"client-secret"});
            fs::write(&receipt, value.to_string()).unwrap();
            let calls = root.path().join("calls");
            let controller = tokio::spawn(async move {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut bytes = Vec::new();
                loop {
                    let mut chunk = [0; 4096];
                    let count = stream.read(&mut chunk).await.unwrap();
                    assert_ne!(count, 0);
                    bytes.extend_from_slice(&chunk[..count]);
                    if bytes.ends_with(b"{\"sessionID\":\"ses_last\"}") { break; }
                }
                let request = String::from_utf8(bytes).unwrap();
                assert!(request.starts_with("POST /tabs/close HTTP/1.1\r\n"), "{request}");
                assert!(!fs::read_to_string(calls).unwrap().contains("close-pane"));
                let (status, body) = if succeeds {
                    value["id"] = json!("");
                    value["tabs"] = json!([]);
                    fs::write(receipt, value.to_string()).unwrap();
                    ("200 OK", json!({"id":"ses_last"}))
                } else {
                    ("409 Conflict", json!({"error":"OpenCode refused tab closure"}))
                };
                let body = body.to_string();
                stream.write_all(format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
            });
            let result = tokio::time::timeout(Duration::from_secs(3), observer.close("ses_last", &target)).await.unwrap();
            controller.await.unwrap();
            result
        });
        let calls = fs::read_to_string(root.path().join("calls")).unwrap();
        if !succeeds {
            assert!(result.unwrap_err().contains("refused tab closure"));
            assert!(!calls.contains("close-pane"), "{calls}");
            continue;
        }
        result.unwrap();
        assert_eq!(calls.matches("close-pane").count(), 1, "{calls}");
        assert!(
            calls.contains("--session main action close-pane --pane-id terminal_7"),
            "{calls}"
        );
        assert!(
            !calls.contains("new-pane") && !calls.contains("new-tab"),
            "{calls}"
        );
    }
}

#[test]
fn clients_with_native_tabs_disabled_close_their_pane() {
    let root = tempfile::tempdir().unwrap();
    let observer = observer(root.path());
    presence(
        &observer,
        "client.json",
        "ses_active",
        7,
        "http://127.0.0.1:1",
    );
    let path = observer.presence.join("client.json");
    let mut value: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    value["tab_control"] = json!({"server":"http://127.0.0.1:1", "token":"client-secret"});
    fs::write(path, value.to_string()).unwrap();
    let target = Pane {
        session: "main".into(),
        id: 7,
        tab_id: 4,
        tab_name: "Review".into(),
    };
    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(observer.close("ses_active", &target))
        .unwrap();
    let calls = fs::read_to_string(root.path().join("calls")).unwrap();
    assert_eq!(calls.matches("close-pane").count(), 1, "{calls}");
}

#[test]
fn shared_session_tabs_require_tab_control_before_closing() {
    let root = tempfile::tempdir().unwrap();
    let observer = observer(root.path());
    presence(&observer, "client.json", "ses_one", 7, "http://127.0.0.1:1");
    let path = observer.presence.join("client.json");
    let mut value: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    let mut background = value.clone();
    background["id"] = json!("ses_background");
    value["tabs"] = json!([background]);
    fs::write(path, value.to_string()).unwrap();
    let target = Pane {
        session: "main".into(),
        id: 7,
        tab_id: 4,
        tab_name: "Review".into(),
    };
    let error = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(observer.close("ses_background", &target))
        .unwrap_err();
    assert!(error.contains("without companion tab control"), "{error}");
    assert!(!root.path().join("calls").exists());
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
                .block_on(observer.new_session_tab(directory, "main", destination, None))
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
            .block_on(observer.new_session_tab(directory, "main", Some(&pane), None))
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
