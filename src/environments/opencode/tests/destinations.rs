use super::*;

#[test]
fn rule_destinations_select_the_newest_running_session_and_reject_exited_targets() {
    let root = tempfile::tempdir().unwrap();
    let observer = observer(root.path());
    fs::write(
        &observer.zellij,
        r#"#!/bin/sh
root=$(dirname "$0")
[ "$*" = 'list-sessions --no-formatting --reverse' ] || exit 1
[ -z "${ZELLIJ_SESSION_NAME+x}" ] || exit 1
cat "$root/sessions"
"#,
    )
    .unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    for (sessions, expected) in [
        (
            "dead [Created 1s ago] (EXITED - attach to resurrect)\nnewest live [Created 2s ago] \nold [Created 3s ago] (current)\n",
            "newest live",
        ),
        ("old [Created 3s ago] (current)\n", "old"),
    ] {
        fs::write(root.path().join("sessions"), sessions).unwrap();
        assert_eq!(
            runtime
                .block_on(observer.resolve_rule_destination())
                .unwrap(),
            expected
        );
        runtime
            .block_on(observer.validate_rule_destination(expected))
            .unwrap();
        assert!(
            runtime
                .block_on(observer.validate_rule_destination("dead"))
                .unwrap_err()
                .contains("not live")
        );
    }
    for sessions in ["", "dead [Created 1s ago] (EXITED - attach to resurrect)\n"] {
        fs::write(root.path().join("sessions"), sessions).unwrap();
        assert!(
            runtime
                .block_on(observer.resolve_rule_destination())
                .unwrap_err()
                .contains("no live Zellij session")
        );
    }
    fs::write(root.path().join("sessions"), "broken inventory\n").unwrap();
    assert!(
        runtime
            .block_on(observer.resolve_rule_destination())
            .unwrap_err()
            .contains("Invalid Zellij session inventory")
    );
    fs::write(
        &observer.zellij,
        "#!/bin/sh\nprintf 'inventory unavailable' >&2\nexit 1\n",
    )
    .unwrap();
    assert!(
        runtime
            .block_on(observer.resolve_rule_destination())
            .unwrap_err()
            .contains("inventory unavailable")
    );
}

fn destination(session: &str) -> Pane {
    Pane {
        session: session.into(),
        id: 7,
        tab_id: 4,
        tab_name: "review".into(),
    }
}

fn launch_observer(root: &Path, sessions: &str) -> Observer {
    let observer = observer(root);
    fs::write(root.join("sessions"), sessions).unwrap();
    fs::write(
        &observer.zellij,
        r#"#!/bin/sh
root=$(dirname "$0")
printf '%s\n' "$*" >> "$root/calls"
case "$*" in
  list-sessions*) cat "$root/sessions" ;;
  *list-panes*) cat "$root/panes.json" ;;
  *new-tab*) printf '9\n' ;;
esac
"#,
    )
    .unwrap();
    observer
}

#[test]
fn manual_launches_keep_a_live_current_session_or_select_the_newest_live_session() {
    for (current, sessions, expected) in [
        (
            "main",
            "newest [Created 1s ago]\nmain [Created 2s ago]\n",
            "main",
        ),
        (
            "old",
            "newest [Created 1s ago]\nmain [Created 2s ago]\n",
            "newest",
        ),
        (
            "old",
            "old [Created 1s ago] (EXITED - attach to resurrect)\nnewest [Created 2s ago]\nmain [Created 3s ago]\n",
            "newest",
        ),
    ] {
        let root = tempfile::tempdir().unwrap();
        let observer = launch_observer(root.path(), sessions);
        let pane = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(observer.new_session(
                root.path().to_str().unwrap(),
                "workspace",
                current,
                None,
                None,
                None,
            ))
            .unwrap();
        assert_eq!(
            (pane.session.as_str(), pane.id, pane.tab_id),
            (expected, 100, 9)
        );
        let calls = fs::read_to_string(root.path().join("calls")).unwrap();
        assert_eq!(calls.matches("action new-tab ").count(), 1, "{calls}");
        assert!(
            calls.contains(&format!("--session {expected} action new-tab")),
            "{calls}"
        );
        assert!(
            calls.contains(&format!(
                "--session {expected} action focus-pane-id terminal_100"
            )),
            "{calls}"
        );
        if current != expected {
            assert!(
                !calls.contains(&format!("--session {current} action")),
                "{calls}"
            );
        }
    }
}

#[test]
fn manual_launches_require_a_valid_live_session_inventory_before_creation() {
    for (sessions, expected) in [
        ("", "no live Zellij session"),
        (
            "old [Created 1s ago] (EXITED - attach to resurrect)\n",
            "no live Zellij session",
        ),
        ("invalid inventory\n", "Invalid Zellij session inventory"),
    ] {
        let root = tempfile::tempdir().unwrap();
        let observer = launch_observer(root.path(), sessions);
        let error = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(observer.new_session(
                root.path().to_str().unwrap(),
                "workspace",
                "old",
                None,
                None,
                None,
            ))
            .unwrap_err();
        assert!(error.contains(expected), "{error}");
        let calls = fs::read_to_string(root.path().join("calls")).unwrap();
        assert!(!calls.contains("action new-tab "), "{calls}");
        assert!(!calls.contains("action new-pane "), "{calls}");
    }
}

#[test]
fn focusing_a_live_client_uses_a_live_source_session() {
    let root = tempfile::tempdir().unwrap();
    let observer = launch_observer(root.path(), "main [Created 1s ago]\n");
    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(observer.focus_pane("old", &destination("main"), false))
        .unwrap();
    let calls = fs::read_to_string(root.path().join("calls")).unwrap();
    assert!(
        calls.contains("--session main action focus-pane-id terminal_7"),
        "{calls}"
    );
    assert!(!calls.contains("--session old action"), "{calls}");
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
  'list-sessions --short --no-formatting') printf 'main\n{}' ;;
  list-sessions*) printf 'main [Created 1s ago]\n' ;;
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
    for failure in ["new-tab", "new-pane", "focus-pane-id"] {
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
  list-sessions*) printf 'main [Created 1s ago]\n' ;;
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
                "old",
                (failure != "new-tab").then_some(&destination("main")),
                None,
                None,
            ))
            .unwrap_err();
        assert!(error.contains("action failed"), "{error}");
        let calls = fs::read_to_string(root.path().join("calls")).unwrap();
        assert_eq!(
            calls
                .lines()
                .filter(|line| line.contains(if failure == "new-tab" {
                    "action new-tab "
                } else {
                    "action new-pane "
                }))
                .count(),
            1
        );
        if failure == "new-tab" {
            assert!(!calls.contains("action new-pane "), "{calls}");
        } else {
            assert!(!calls.contains("action new-tab "), "{calls}");
        }
    }
}
