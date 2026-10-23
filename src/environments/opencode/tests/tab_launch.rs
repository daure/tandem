use super::*;

fn delayed_observer(root: &Path) -> Observer {
    let observer = observer(root);
    fs::write(
        &observer.zellij,
        r#"#!/bin/sh
root=$(dirname "$0")
printf '%s\n' "$*" >> "$root/calls"
case "$*" in
  list-sessions*) printf 'main [Created 1s ago]\n' ;;
  *new-tab*) printf '9\n' ;;
  *list-panes*)
    count=0
    [ ! -f "$root/count" ] || read -r count < "$root/count"
    count=$((count + 1))
    printf '%s\n' "$count" > "$root/count"
    if [ -f "$root/panes-$count.json" ]; then
      cat "$root/panes-$count.json"
    else
      cat "$root/panes.json"
    fi ;;
esac
"#,
    )
    .unwrap();
    observer
}

#[test]
fn new_tabs_wait_for_a_live_terminal_pane_in_the_created_tab() {
    let root = tempfile::tempdir().unwrap();
    let observer = delayed_observer(root.path());
    fs::write(root.path().join("panes-1.json"), "[]").unwrap();
    fs::write(
        root.path().join("panes-2.json"),
        json!([
            {"id":1,"is_plugin":false,"exited":false,"tab_id":8,"tab_name":"other"},
            {"id":2,"is_plugin":true,"exited":false,"tab_id":9,"tab_name":"workspace"},
            {"id":3,"is_plugin":false,"exited":true,"tab_id":9,"tab_name":"workspace"}
        ])
        .to_string(),
    )
    .unwrap();
    let pane = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(observer.launch_panel(
            root.path().to_str().unwrap(),
            "workspace",
            "main",
            None,
            &["opencode".into()],
            false,
        ))
        .unwrap();
    assert_eq!(
        (pane.session.as_str(), pane.id, pane.tab_id),
        ("main", 100, 9)
    );
    let calls = fs::read_to_string(root.path().join("calls")).unwrap();
    assert_eq!(calls.matches("action list-panes").count(), 3, "{calls}");
    assert_eq!(calls.matches("action new-tab").count(), 1, "{calls}");
    assert!(
        calls.contains("rename-pane --pane-id terminal_100"),
        "{calls}"
    );
    assert!(!calls.contains("focus-pane-id"), "{calls}");
}

#[test]
fn new_tabs_report_uncertain_identity_after_the_pane_wait_expires() {
    let root = tempfile::tempdir().unwrap();
    let observer = delayed_observer(root.path());
    fs::write(root.path().join("panes.json"), "[]").unwrap();
    let error = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(async {
            tokio::time::timeout(
                std::time::Duration::from_secs(5),
                observer.launch_panel(
                    root.path().to_str().unwrap(),
                    "workspace",
                    "main",
                    None,
                    &["opencode".into()],
                    true,
                ),
            )
            .await
            .expect("pane discovery must be bounded")
        })
        .unwrap_err();
    assert_eq!(error, "Cannot identify the new OpenCode pane");
    let calls = fs::read_to_string(root.path().join("calls")).unwrap();
    assert!(calls.matches("action list-panes").count() > 1, "{calls}");
    assert_eq!(calls.matches("action new-tab").count(), 1, "{calls}");
    assert!(!calls.contains("rename-pane"), "{calls}");
    assert!(!calls.contains("focus-pane-id"), "{calls}");
}

#[test]
fn new_tabs_preserve_inventory_errors_and_ambiguous_command_failures() {
    let root = tempfile::tempdir().unwrap();
    let observer = delayed_observer(root.path());
    let command = navigation::client_command(&["opencode".into()]).join(" ");
    let runtime = tokio::runtime::Runtime::new().unwrap();
    for (inventory, expected) in [
        ("invalid".into(), "Invalid Zellij pane inventory"),
        (
            json!([
                {"id":100,"is_plugin":false,"exited":false,"tab_id":9,"tab_name":"workspace","terminal_command":command},
                {"id":101,"is_plugin":false,"exited":false,"tab_id":9,"tab_name":"workspace","terminal_command":command}
            ]).to_string(),
            "Multiple panes match the new OpenCode command",
        ),
    ] {
        fs::write(root.path().join("panes.json"), inventory).unwrap();
        let error = runtime
            .block_on(observer.launch_panel(
                root.path().to_str().unwrap(),
                "workspace",
                "main",
                None,
                &["opencode".into()],
                true,
            ))
            .unwrap_err();
        assert_eq!(error, expected);
    }
    let calls = fs::read_to_string(root.path().join("calls")).unwrap();
    assert_eq!(calls.matches("action list-panes").count(), 2, "{calls}");
    assert!(!calls.contains("rename-pane"), "{calls}");
    assert!(!calls.contains("focus-pane-id"), "{calls}");
}
