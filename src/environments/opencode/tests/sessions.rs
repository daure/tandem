use super::{history_server::Server, *};

fn session(server: &Server) -> Session {
    Session {
        id: "ses_old".into(),
        title: "Fixture".into(),
        directory: "/work/review".into(),
        server: server.url.clone(),
        activity: Activity::Idle,
        ..Default::default()
    }
}

#[test]
fn rename_saves_the_exact_title_on_native_and_compatible_servers() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    for v2 in [false, true] {
        let server = Server::start();
        server.data.lock().unwrap().v2 = v2;
        server.session("ses_old", "/work/review", None);
        let title = "Review \"API\" — café";
        runtime
            .block_on(super::super::rename(&session(&server), title))
            .unwrap();
        let data = server.data.lock().unwrap();
        assert_eq!(data.sessions["ses_old"]["title"], title);
        assert_eq!(data.renamed, [json!({"title":title})]);
    }
}

#[test]
fn rename_verifies_ownership_and_the_saved_title() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    for reason in ["directory", "missing", "unsaved"] {
        let server = Server::start();
        server.data.lock().unwrap().v2 = true;
        if reason != "missing" {
            server.session(
                "ses_old",
                if reason == "directory" {
                    "/work/other"
                } else {
                    "/work/review"
                },
                None,
            );
        }
        server.data.lock().unwrap().rename_lies = reason == "unsaved";
        let error = runtime
            .block_on(super::super::rename(&session(&server), "Updated"))
            .unwrap_err();
        assert!(
            error.contains(match reason {
                "directory" => "ownership changed",
                "missing" => "was deleted",
                _ => "did not save",
            }),
            "{error}"
        );
        assert_eq!(
            server.data.lock().unwrap().renamed.len(),
            usize::from(reason == "unsaved")
        );
    }
}

#[test]
fn delete_removes_only_the_selected_conversation_tree() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    for v2 in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let observer = observer(root.path());
        let server = Server::start();
        server.data.lock().unwrap().v2 = v2;
        server.session("ses_old", "/work/review", None);
        server.session("ses_child", "/work/review", Some("ses_old"));
        server.session("ses_grandchild", "/work/review", Some("ses_child"));
        server.session("ses_sibling", "/work/review", None);
        let removed = runtime
            .block_on(observer.delete_session(&session(&server)))
            .unwrap();
        assert_eq!(removed, ["ses_child", "ses_grandchild", "ses_old"]);
        let data = server.data.lock().unwrap();
        assert_eq!(data.deleted, ["ses_old"]);
        assert_eq!(
            data.sessions.keys().cloned().collect::<Vec<_>>(),
            ["ses_sibling"]
        );
    }
}

#[test]
fn delete_preserves_history_when_stopping_closing_or_ownership_cannot_be_verified() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    for reason in [
        "attached-child",
        "foreign-child",
        "directory",
        "status",
        "cycle",
        "failure",
        "unsaved",
        "interrupt-failure",
        "interrupt-lies",
    ] {
        let root = tempfile::tempdir().unwrap();
        let observer = observer(root.path());
        let server = Server::start();
        server.session(
            "ses_old",
            if reason == "directory" {
                "/work/other"
            } else {
                "/work/review"
            },
            None,
        );
        server.session(
            "ses_child",
            if reason == "foreign-child" {
                "/work/other"
            } else {
                "/work/review"
            },
            Some("ses_old"),
        );
        {
            let mut data = server.data.lock().unwrap();
            data.v2 = true;
            data.busy = reason == "interrupt-lies";
            data.interrupt_failure = reason == "interrupt-failure";
            data.interrupt_lies = reason == "interrupt-lies";
            data.status_failure = reason == "status";
            data.delete_failure = reason == "failure";
            data.delete_lies = reason == "unsaved";
        }
        if reason == "attached-child" {
            presence(
                &observer,
                "client.json",
                "ses_child",
                7,
                &format!("{}/", server.url),
            );
            fs::write(root.path().join("fail-panes"), "").unwrap();
        }
        if reason == "cycle" {
            server.session("ses_old", "/work/review", Some("ses_child"));
        }
        let result = runtime.block_on(observer.delete_session(&session(&server)));
        assert!(result.is_err(), "{reason}");
        let data = server.data.lock().unwrap();
        assert_eq!(
            data.deleted.len(),
            usize::from(reason == "unsaved"),
            "{reason}"
        );
        assert!(data.sessions.contains_key("ses_old"), "{reason}");
    }
}

#[test]
fn deletion_stops_work_and_closes_attached_children_before_removing_the_tree() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    for v2 in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let observer = observer(root.path());
        fs::write(
            &observer.zellij,
            r#"#!/bin/sh
root=$(dirname "$0")
printf '%s\n' "$*" >> "$root/calls"
case "$*" in
  list-sessions*) printf 'main\n' ;;
  *list-panes*)
    if [ -f "$root/closed" ]; then printf '[]'; else cat "$root/panes.json"; fi ;;
  *close-pane*) touch "$root/closed" ;;
esac
"#,
        )
        .unwrap();
        let server = Server::start();
        server.session("ses_old", "/work/review", None);
        server.session("ses_child", "/work/review", Some("ses_old"));
        server.session("ses_sibling", "/work/review", None);
        {
            let mut data = server.data.lock().unwrap();
            data.v2 = v2;
            data.busy = true;
            data.awaiting_answer = true;
            data.approval = true;
            data.queued = true;
        }
        presence_in(
            &observer,
            "client.json",
            "ses_child",
            7,
            &format!("{}/", server.url),
            "/work/review",
        );
        runtime
            .block_on(observer.delete_session(&session(&server)))
            .unwrap();
        assert!(root.path().join("closed").exists());
        let calls = fs::read_to_string(root.path().join("calls")).unwrap();
        assert_eq!(calls.matches("close-pane").count(), 1, "{calls}");
        let data = server.data.lock().unwrap();
        assert_eq!(data.interrupted, ["ses_child", "ses_old"]);
        assert_eq!(data.deleted, ["ses_old"]);
        assert_eq!(
            data.sessions.keys().cloned().collect::<Vec<_>>(),
            ["ses_sibling"]
        );
    }
}
