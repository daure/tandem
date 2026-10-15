use super::*;
use crate::store::opencode::closing::{CloseInput, Sessions};

fn fixture(root: &Path) -> (Observer, String) {
    let observer = observer(root);
    let workspace = root.join("workspace");
    fs::create_dir(&workspace).unwrap();
    fs::create_dir(root.join("outside")).unwrap();
    let directory = workspace.to_str().unwrap();
    presence_in(&observer, "caller.json", "ses_caller", 7, "", directory);
    presence_in(&observer, "peer.json", "ses_peer", 8, "", directory);
    presence_in(
        &observer,
        "outside.json",
        "ses_outside",
        100,
        "",
        root.join("outside").to_str().unwrap(),
    );
    for id in [7, 8, 9, 100] {
        fs::write(root.join(format!("pane-{id}.json")), json!({
            "id": id, "is_plugin":false, "exited":false, "tab_id": if id == 100 {9} else {4},
            "tab_name":"Same name", "pane_cwd": if id == 100 {root.join("outside")} else {workspace.clone()}
        }).to_string()).unwrap();
    }
    fs::write(
        &observer.zellij,
        r#"#!/bin/sh
root=$(dirname "$0")
printf '%s\n' "$*" >> "$root/calls"
case "$*" in
  list-sessions*) printf 'main\n';;
  *list-panes*)
    printf '['; separator=''
    for id in 7 8 9 100; do
      if [ ! -f "$root/closed-$id" ]; then
        printf '%s' "$separator"; cat "$root/pane-$id.json"; separator=','
      fi
    done
    printf ']';;
  *close-pane*)
    for argument do last=$argument; done
    touch "$root/closed-${last#terminal_}";;
  *close-tab*) touch "$root/closed-7" "$root/closed-8" "$root/closed-9";;
esac
"#,
    )
    .unwrap();
    (observer, directory.into())
}

#[test]
fn session_and_terminal_closure_are_independent_and_preserve_other_instances() {
    for (sessions, tab) in [
        (Sessions::Current, false),
        (Sessions::All, false),
        (Sessions::None, true),
        (Sessions::Current, true),
        (Sessions::All, true),
    ] {
        let root = tempfile::tempdir().unwrap();
        let (observer, workspace) = fixture(root.path());
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let input = CloseInput {
                sessions,
                zellij_tab: tab,
            };
            let plan = observer
                .plan_instance_close(&workspace, "ses_caller", &input)
                .await
                .unwrap();
            observer
                .execute_instance_close(&workspace, plan)
                .await
                .unwrap();
        });
        assert!(root.path().join("closed-7").exists());
        assert_eq!(
            root.path().join("closed-8").exists(),
            tab || sessions == Sessions::All
        );
        assert_eq!(root.path().join("closed-9").exists(), tab);
        assert!(!root.path().join("closed-100").exists());
        assert!(Path::new(&workspace).is_dir());
        let calls = fs::read_to_string(root.path().join("calls")).unwrap();
        assert_eq!(calls.contains("close-tab --tab-id 4"), tab, "{calls}");
    }
}

#[test]
fn terminal_closure_rejects_unrelated_unknown_and_retargeted_panes_before_mutation() {
    for change_after_admission in [false, true] {
        for cwd in [Some("outside"), None] {
            let root = tempfile::tempdir().unwrap();
            let (observer, workspace) = fixture(root.path());
            let runtime = tokio::runtime::Runtime::new().unwrap();
            let input = CloseInput {
                sessions: Sessions::All,
                zellij_tab: true,
            };
            let plan = change_after_admission.then(|| {
                runtime
                    .block_on(observer.plan_instance_close(&workspace, "ses_caller", &input))
                    .unwrap()
            });
            let path = root.path().join("pane-9.json");
            let mut pane: serde_json::Value =
                serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
            pane["pane_cwd"] = cwd
                .map(|directory| json!(root.path().join(directory)))
                .unwrap_or(serde_json::Value::Null);
            fs::write(path, pane.to_string()).unwrap();
            let error = runtime
                .block_on(async {
                    if let Some(plan) = plan {
                        observer.execute_instance_close(&workspace, plan).await
                    } else {
                        observer
                            .plan_instance_close(&workspace, "ses_caller", &input)
                            .await
                            .map(|_| ())
                    }
                })
                .unwrap_err();
            assert!(error.contains("unrelated or unverified pane 9"), "{error}");
            assert!(
                !fs::read_to_string(root.path().join("calls"))
                    .unwrap()
                    .contains("close-")
            );
        }
    }
}

#[test]
fn closure_requires_current_unambiguous_caller_and_fresh_workspace_evidence() {
    for mutation in ["missing", "ambiguous", "changed", "outside", "stale"] {
        let root = tempfile::tempdir().unwrap();
        let (observer, workspace) = fixture(root.path());
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let input = CloseInput {
            sessions: Sessions::None,
            zellij_tab: true,
        };
        let plan = runtime
            .block_on(observer.plan_instance_close(&workspace, "ses_caller", &input))
            .unwrap();
        let receipt = observer.presence.join("caller.json");
        let mut value: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&receipt).unwrap()).unwrap();
        match mutation {
            "missing" => fs::remove_file(&receipt).unwrap(),
            "ambiguous" => {
                presence_in(&observer, "duplicate.json", "ses_caller", 8, "", &workspace)
            }
            "changed" => {
                value["id"] = json!("ses_replacement");
                fs::write(&receipt, value.to_string()).unwrap();
            }
            "outside" => {
                value["directory"] = json!(root.path().join("outside"));
                fs::write(&receipt, value.to_string()).unwrap();
            }
            "stale" => {
                value["observed_at"] = json!(0);
                fs::write(&receipt, value.to_string()).unwrap();
            }
            _ => unreachable!(),
        }
        if mutation == "ambiguous" {
            assert!(
                runtime
                    .block_on(observer.plan_instance_close(&workspace, "ses_caller", &input))
                    .err()
                    .unwrap()
                    .contains("ambiguous")
            );
        } else {
            assert!(
                runtime
                    .block_on(observer.execute_instance_close(&workspace, plan))
                    .is_err(),
                "{mutation}"
            );
        }
        assert!(
            !fs::read_to_string(root.path().join("calls"))
                .unwrap()
                .contains("close-")
        );
    }
}

#[test]
fn all_selection_uses_owned_native_tabs_without_sweeping_the_client_home_screen() {
    let root = tempfile::tempdir().unwrap();
    let (observer, workspace) = fixture(root.path());
    let receipt = observer.presence.join("caller.json");
    let mut value: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&receipt).unwrap()).unwrap();
    let mut caller = value.clone();
    caller["tab_index"] = json!(0);
    let mut outside = caller.clone();
    outside["id"] = json!("ses_external");
    outside["directory"] = json!(root.path().join("outside"));
    value["id"] = json!("");
    value["tabs"] = json!([caller, outside]);
    fs::write(&receipt, value.to_string()).unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let plan = runtime
        .block_on(observer.plan_instance_close(
            &workspace,
            "ses_caller",
            &CloseInput {
                sessions: Sessions::All,
                zellij_tab: false,
            },
        ))
        .unwrap();
    let ids: Vec<_> = serde_json::to_value(plan).unwrap()["attachments"]
        .as_array()
        .unwrap()
        .iter()
        .map(|target| target["id"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(ids.len(), 2);
    assert!(ids.contains(&"ses_caller".to_owned()));
    assert!(ids.contains(&"ses_peer".to_owned()));
}

#[test]
fn native_session_closure_precedes_terminal_cleanup_and_persists_tab_removal() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let root = tempfile::tempdir().unwrap();
    let (observer, workspace) = fixture(root.path());
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let receipt = observer.presence.join("caller.json");
        let mut value: serde_json::Value = serde_json::from_str(&fs::read_to_string(&receipt).unwrap()).unwrap();
        value["tab_index"] = json!(0);
        value["tab_control"] = json!({"server": format!("http://{}", listener.local_addr().unwrap()), "token": "secret"});
        fs::write(&receipt, value.to_string()).unwrap();
        let controller = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            loop {
                let mut buffer = [0; 4096];
                let count = stream.read(&mut buffer).await.unwrap();
                assert_ne!(count, 0);
                bytes.extend_from_slice(&buffer[..count]);
                if bytes.ends_with(b"{\"sessionID\":\"ses_caller\"}") { break; }
            }
            assert!(bytes.starts_with(b"POST /tabs/close HTTP/1.1\r\n"));
            value["id"] = json!("");
            value["tab_index"] = serde_json::Value::Null;
            fs::write(receipt, value.to_string()).unwrap();
            let body = json!({"id":"ses_caller"}).to_string();
            stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
        });
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            let plan = observer.plan_instance_close(&workspace, "ses_caller", &CloseInput { sessions: Sessions::Current, zellij_tab: true }).await.unwrap();
            observer.execute_instance_close(&workspace, plan).await.unwrap();
            controller.await.unwrap();
        }).await.unwrap();
    });
    for id in [7, 8, 9] {
        assert!(root.path().join(format!("closed-{id}")).exists());
    }
    assert!(!root.path().join("closed-100").exists());
}

#[test]
fn a_home_screen_opening_an_unrelated_tab_blocks_bulk_closure_before_mutation() {
    let root = tempfile::tempdir().unwrap();
    let (observer, workspace) = fixture(root.path());
    presence_in(&observer, "peer.json", "", 8, "", &workspace);
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let plan = runtime
        .block_on(observer.plan_instance_close(
            &workspace,
            "ses_caller",
            &CloseInput {
                sessions: Sessions::All,
                zellij_tab: false,
            },
        ))
        .unwrap();
    let receipt = observer.presence.join("peer.json");
    let mut value: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&receipt).unwrap()).unwrap();
    let mut outside = value.clone();
    outside["id"] = json!("ses_new");
    outside["directory"] = json!(root.path().join("outside"));
    value["tabs"] = json!([outside]);
    fs::write(receipt, value.to_string()).unwrap();
    assert!(
        runtime
            .block_on(observer.execute_instance_close(&workspace, plan))
            .is_err()
    );
    assert!(!root.path().join("closed-7").exists());
    assert!(!root.path().join("closed-8").exists());
}
