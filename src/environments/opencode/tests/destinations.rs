use super::*;

fn destination(session: &str) -> Pane {
    Pane {
        session: session.into(),
        id: 7,
        tab_id: 4,
        tab_name: "review".into(),
    }
}

#[test]
fn new_sessions_use_a_fresh_tab_when_the_cached_destination_is_not_live() {
    for state in ["missing", "exited", "plugin"] {
        let root = tempfile::tempdir().unwrap();
        let observer = observer(root.path());
        let mut panes = vec![json!({
            "id":100,"is_plugin":false,"exited":false,"tab_id":9,"tab_name":"workspace"
        })];
        if state != "missing" {
            panes.push(json!({
                "id":7,"is_plugin":state == "plugin","exited":state == "exited",
                "tab_id":4,"tab_name":"review"
            }));
        }
        fs::write(root.path().join("panes.json"), json!(panes).to_string()).unwrap();
        let created = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(observer.new_session(
                root.path().to_str().unwrap(),
                "workspace",
                "main",
                Some(&destination("main")),
                None,
                None,
            ))
            .unwrap();
        assert_eq!(
            (created.session.as_str(), created.id, created.tab_id),
            ("main", 100, 9)
        );
        let calls = fs::read_to_string(root.path().join("calls")).unwrap();
        assert_eq!(
            calls
                .lines()
                .filter(|line| line.contains("action new-tab "))
                .count(),
            1
        );
        assert!(!calls.contains("action new-pane "), "{state}: {calls}");
        assert!(calls.contains("focus-pane-id terminal_100"), "{calls}");
    }
}

#[test]
fn new_sessions_fall_back_from_closed_zellij_sessions_but_preserve_inventory_errors() {
    for session_alive in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let observer = observer(root.path());
        fs::write(
            &observer.zellij,
            format!(
                r#"#!/bin/sh
root=$(dirname "$0")
printf '%s\n' "$*" >> "$root/calls"
case "$*" in
  '--session old action list-panes --all --json') printf 'inventory unavailable\n' >&2; exit 1 ;;
  list-sessions*) printf 'main\n{}' ;;
  *list-panes*) cat "$root/panes.json" ;;
  *new-tab*) printf '9\n' ;;
esac
"#,
                if session_alive { "old\n" } else { "" }
            ),
        )
        .unwrap();
        let result = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(observer.new_session(
                root.path().to_str().unwrap(),
                "workspace",
                "main",
                Some(&destination("old")),
                None,
                None,
            ));
        let calls = fs::read_to_string(root.path().join("calls")).unwrap();
        if session_alive {
            assert!(result.unwrap_err().contains("inventory unavailable"));
            assert!(!calls.contains("action new-tab "), "{calls}");
        } else {
            let created = result.unwrap();
            assert_eq!((created.session.as_str(), created.id), ("main", 100));
            assert_eq!(
                calls
                    .lines()
                    .filter(|line| line.contains("action new-tab "))
                    .count(),
                1
            );
            assert!(calls.contains("--session main action new-tab"), "{calls}");
        }
        assert!(!calls.contains("action new-pane "), "{calls}");
    }
}

#[test]
fn launch_failures_do_not_create_a_second_client() {
    for failure in ["new-pane", "focus-pane-id"] {
        let root = tempfile::tempdir().unwrap();
        let observer = observer(root.path());
        fs::write(
            &observer.zellij,
            format!(
                r#"#!/bin/sh
root=$(dirname "$0")
printf '%s\n' "$*" >> "$root/calls"
case "$*" in
  *{failure}*) printf 'action failed\n' >&2; exit 1 ;;
  *list-panes*) cat "$root/panes.json" ;;
  *new-pane*) printf 'terminal_99\n' ;;
esac
"#
            ),
        )
        .unwrap();
        let error = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(observer.new_session(
                root.path().to_str().unwrap(),
                "workspace",
                "main",
                Some(&destination("main")),
                None,
                None,
            ))
            .unwrap_err();
        assert!(error.contains("action failed"), "{error}");
        let calls = fs::read_to_string(root.path().join("calls")).unwrap();
        assert_eq!(
            calls
                .lines()
                .filter(|line| line.contains("action new-pane "))
                .count(),
            1
        );
        assert!(!calls.contains("action new-tab "), "{calls}");
    }
}
