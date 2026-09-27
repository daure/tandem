use super::*;

#[test]
fn new_sessions_attach_without_resuming_and_stack_in_the_observed_tab() {
    let root = tempfile::tempdir().unwrap();
    let workspace = root.path().join("space ' ; $(touch injected)");
    fs::create_dir(&workspace).unwrap();
    let directory = workspace.to_str().unwrap();
    let server = Server::start();
    let observer = observer(root.path());
    presence_in(
        &observer,
        "client.json",
        "ses_busy",
        7,
        &server.url,
        directory,
    );
    let pane = Pane {
        session: "main".into(),
        id: 7,
        tab_id: 4,
        tab_name: "review".into(),
    };
    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(observer.new_session(directory, "review", "other", Some(&pane), Some("Explain 'this'; $(touch injected)\nsecond line")))
        .unwrap();
    let calls = fs::read_to_string(root.path().join("calls")).unwrap();
    assert!(calls.contains(&format!("new-pane --stacked --name  --tab-id 4 --cwd {directory} -- env TANDEM_INITIAL_PROMPT=Explain 'this'; $(touch injected)\nsecond line opencode attach {} --dir {directory}\n", server.url)), "{calls}");
    assert!(!calls.contains("rename-pane"), "{calls}");
    assert!(
        calls.contains("--session other action switch-session main --pane-id terminal_99"),
        "{calls}"
    );
    assert!(!workspace.join("injected").exists());
    assert!(
        server
            .requests
            .lock()
            .unwrap()
            .iter()
            .all(|request| request.starts_with("GET /global/health "))
    );
}

#[test]
fn new_sessions_start_opencode_in_a_named_tab_without_a_known_server() {
    let root = tempfile::tempdir().unwrap();
    let observer = observer(root.path());
    let directory = root.path().to_str().unwrap();
    let created = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(observer.new_session(directory, "workspace", "main", None, None))
        .unwrap();
    assert_eq!(
        (created.session.as_str(), created.id, created.tab_id),
        ("main", 100, 9)
    );
    let calls = fs::read_to_string(root.path().join("calls")).unwrap();
    assert!(
        calls.contains(&format!(
            "new-tab --name workspace --cwd {directory} -- env TANDEM_INITIAL_PROMPT= opencode {directory}\n"
        )),
        "{calls}"
    );
    assert!(
        calls.contains("--session main action go-to-tab-by-id 9"),
        "{calls}"
    );
    assert!(
        calls.contains("--session main action hide-floating-panes --tab-id 9"),
        "{calls}"
    );
    assert!(
        calls.contains("rename-pane --pane-id terminal_100 \n"),
        "{calls}"
    );
    assert!(calls.contains("focus-pane-id terminal_100"), "{calls}");
}

#[test]
fn new_tab_returns_the_command_pane_with_an_application_owned_title_in_a_multi_pane_layout() {
    let root = tempfile::tempdir().unwrap();
    let observer = observer(root.path());
    let directory = root.path().to_str().unwrap();
    fs::write(root.path().join("panes.json"), json!([
        {"id":100,"is_plugin":false,"exited":false,"tab_id":9,"tab_name":"workspace","terminal_command":"sleep 60"},
        {"id":101,"is_plugin":false,"exited":false,"tab_id":9,"tab_name":"workspace","terminal_command":format!("env TANDEM_INITIAL_PROMPT= opencode {directory}")}
    ]).to_string()).unwrap();
    let pane = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(observer.new_session(directory, "workspace", "main", None, None))
        .unwrap();
    assert_eq!(pane.id, 101);
    let calls = fs::read_to_string(root.path().join("calls")).unwrap();
    assert!(
        calls.contains("rename-pane --pane-id terminal_101 \n"),
        "{calls}"
    );
    assert!(calls.contains("focus-pane-id terminal_101"), "{calls}");
}

#[test]
fn new_session_selects_the_destination_tab_and_focuses_its_exact_new_pane() {
    let root = tempfile::tempdir().unwrap();
    let observer = observer(root.path());
    let pane = Pane {
        session: "main".into(),
        id: 7,
        tab_id: 4,
        tab_name: "review".into(),
    };
    let created = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(observer.new_session(
            root.path().to_str().unwrap(),
            "workspace",
            "main",
            Some(&pane),
            None,
        ))
        .unwrap();
    assert_eq!(
        (created.session.as_str(), created.id, created.tab_id),
        ("main", 99, 4)
    );
    let calls = fs::read_to_string(root.path().join("calls")).unwrap();
    let created = calls
        .find("new-pane --stacked --name  --tab-id 4")
        .unwrap();
    let selected = calls.find("go-to-tab-by-id 4").expect(&calls);
    let visible = calls.find("hide-floating-panes --tab-id 4").expect(&calls);
    let focused = calls.find("focus-pane-id terminal_99").expect(&calls);
    assert!(
        created < selected && selected < visible && visible < focused,
        "{calls}"
    );
}

#[test]
fn zellij_accepts_only_the_requested_focus_and_visibility_noops() {
    let root = tempfile::tempdir().unwrap();
    let observer = observer(root.path());
    let runtime = tokio::runtime::Runtime::new().unwrap();
    for (action, target, status, error, succeeds) in [
        (
            "focus-pane-id",
            "terminal_7",
            2,
            "Pane Terminal(7) is already focused",
            true,
        ),
        (
            "focus-pane-id",
            "terminal_8",
            2,
            "Pane Terminal(7) is already focused",
            false,
        ),
        (
            "focus-pane-id",
            "terminal_7",
            2,
            "Pane with id Terminal(7) not found",
            false,
        ),
        (
            "new-pane",
            "terminal_7",
            2,
            "Pane Terminal(7) is already focused",
            false,
        ),
        ("hide-floating-panes", "", 2, "", true),
        ("show-floating-panes", "", 2, "", true),
        ("hide-floating-panes", "", 1, "Tab not found", false),
    ] {
        fs::write(
            &observer.zellij,
            format!("#!/bin/sh\nprintf '%s\\n' '{error}' >&2\nexit {status}\n"),
        )
        .unwrap();
        let result = runtime.block_on(zellij(
            &observer.zellij,
            &["action".into(), action.into(), target.into()],
        ));
        assert_eq!(result.is_ok(), succeeds, "{action}: {result:?}");
    }
}

#[test]
fn jumping_selects_the_target_pane_layer_and_accepts_an_already_focused_pane() {
    for floating in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let observer = observer(root.path());
        presence(
            &observer,
            "client.json",
            "ses_one",
            7,
            "http://127.0.0.1:4199",
        );
        fs::write(root.path().join("panes.json"), json!([
            {"id":7,"is_plugin":false,"exited":false,"tab_id":4,"tab_name":"Review","is_floating":floating}
        ]).to_string()).unwrap();
        fs::write(
            &observer.zellij,
            r#"#!/bin/sh
root=$(dirname "$0")
printf '%s\n' "$*" >> "$root/calls"
case "$*" in
  *list-panes*) cat "$root/panes.json" ;;
  *floating-panes*) exit 2 ;;
  *focus-pane-id*) printf '%s\n' 'Pane Terminal(7) is already focused' >&2; exit 2 ;;
esac
"#,
        )
        .unwrap();
        let pane = Pane {
            session: "main".into(),
            id: 7,
            tab_id: 99,
            tab_name: "stale".into(),
        };
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(observer.jump("ses_one", &pane, "main"))
            .unwrap();
        let calls = fs::read_to_string(root.path().join("calls")).unwrap();
        let visibility = if floating {
            "show-floating-panes"
        } else {
            "hide-floating-panes"
        };
        assert!(calls.contains(&format!("go-to-tab-by-id 4\n--session main action {visibility} --tab-id 4\n--session main action focus-pane-id terminal_7")), "{calls}");
    }
}

#[test]
fn new_sessions_reject_missing_workspaces_and_non_zellij_navigation() {
    let root = tempfile::tempdir().unwrap();
    let observer = observer(root.path());
    let runtime = tokio::runtime::Runtime::new().unwrap();
    assert!(
        runtime
            .block_on(observer.new_session(root.path().to_str().unwrap(), "workspace", "", None, None))
            .unwrap_err()
            .contains("inside Zellij")
    );
    assert!(
        runtime
            .block_on(observer.new_session(
                root.path().join("missing").to_str().unwrap(),
                "workspace",
                "main",
                None,
                None,
            ))
            .unwrap_err()
            .contains("unavailable")
    );
    assert!(!root.path().join("calls").exists());
}

#[test]
fn bulk_close_rechecks_the_panes_workspace_before_closing() {
    let root = tempfile::tempdir().unwrap();
    let observer = observer(root.path());
    presence_in(
        &observer,
        "client.json",
        "ses_one",
        7,
        "http://127.0.0.1:4199",
        "/work/moved",
    );
    let pane = Pane {
        session: "main".into(),
        id: 7,
        tab_id: 4,
        tab_name: "review".into(),
    };
    let runtime = tokio::runtime::Runtime::new().unwrap();
    assert!(
        runtime
            .block_on(observer.close_in_directory("/work/original", &pane))
            .unwrap_err()
            .contains("closed or changed")
    );
    assert!(!root.path().join("calls").exists());
    runtime
        .block_on(observer.close_in_directory("/work/moved", &pane))
        .unwrap();
    let calls = fs::read_to_string(root.path().join("calls")).unwrap();
    assert!(
        calls.contains("--session main action close-pane --pane-id terminal_7"),
        "{calls}"
    );
}

#[test]
fn zellij_failures_identify_the_action_and_preserve_stderr() {
    let root = tempfile::tempdir().unwrap();
    let observer = observer(root.path());
    fs::write(
        &observer.zellij,
        "#!/bin/sh\nprintf '%s\\n' 'error: target tab is unavailable' >&2\nexit 2\n",
    )
    .unwrap();
    let error = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(zellij(
            &observer.zellij,
            &[
                "--session".into(),
                "main".into(),
                "action".into(),
                "go-to-tab-by-id".into(),
                "4".into(),
            ],
        ))
        .unwrap_err();
    assert!(error.contains("go-to-tab-by-id"), "{error}");
    assert!(error.contains("exit status: 2"), "{error}");
    assert!(
        error.contains("error: target tab is unavailable"),
        "{error}"
    );
}

#[test]
fn zellij_drains_stderr_concurrently_and_bounds_diagnostics() {
    let root = tempfile::tempdir().unwrap();
    let observer = observer(root.path());
    let runtime = tokio::runtime::Runtime::new().unwrap();
    for (size, exit) in [(60000, 0), (4000, 2), (70000, 2)] {
        fs::write(
            &observer.zellij,
            format!("#!/bin/sh\nprintf '%0{size}d' 1 >&2\nprintf '[]'\nexit {exit}\n"),
        )
        .unwrap();
        let result = runtime.block_on(zellij(
            &observer.zellij,
            &["action".into(), "list-panes".into()],
        ));
        match size {
            60000 => assert_eq!(result.unwrap(), "[]"),
            4000 => {
                let error = result.unwrap_err();
                assert!(error.contains("list-panes failed (exit status: 2)"));
                assert!(error.ends_with("(see Tandem diagnostic log)"));
                assert!(error.len() < 2200);
            }
            _ => assert!(result.unwrap_err().contains("exceeds 65536 bytes")),
        }
    }
}
