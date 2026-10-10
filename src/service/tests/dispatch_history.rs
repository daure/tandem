use super::*;
use std::{fs, os::unix::fs::PermissionsExt};

use crate::environments::opencode::tests::history_server;

#[test]
fn headless_dispatch_retains_ownership_through_retry_purge_and_historical_reopening() {
    let mut service = AppService::for_tests();
    service
        .runtime
        .block_on(service.set_clear_opencode_history(false).unwrap())
        .unwrap()
        .unwrap();
    service
        .runtime
        .block_on(service.create_template("blank".into()))
        .unwrap();
    let config = service.environments.config.clone();
    let zellij = config.home.join("zellij");
    fs::write(&zellij, r#"#!/bin/sh
root=$(dirname "$0")
printf '%s\n' "$*" >> "$root/zellij-calls"
case "$*" in
  list-sessions*)
    case "$*" in
      *--short*) printf 'main\nnewest\n' ;;
      *) cat "$root/rule-sessions" ;;
    esac ;;
  *list-panes*) printf '[{"id":100,"is_plugin":false,"exited":false,"tab_id":9,"tab_name":"Review"}]' ;;
  *new-tab*) printf '9\n' ;;
  *new-pane*) printf 'terminal_100\n' ;;
esac
"#).unwrap();
    fs::set_permissions(&zellij, fs::Permissions::from_mode(0o700)).unwrap();
    fs::write(config.home.join("rule-sessions"), "main [Created 2s ago]\n").unwrap();
    let integration = Arc::get_mut(&mut service.opencode).unwrap();
    integration.observer.zellij = zellij;
    integration.current_zellij = "main".into();
    let presence = integration.observer.presence.clone();
    let seed = config.templates.join("blank/tandem-files");
    fs::create_dir(&seed).unwrap();
    std::os::unix::fs::symlink(&config.home, seed.join("unsafe")).unwrap();
    let token = service.register_provider_for_tests("sample");
    let mut rule = definition("inspect");
    rule.script = "fn matches(event) { true } fn instance_name(event) { \" PR 82 \" } fn instance_description(event) { \"slack://channel?team=T1&id=C2\" }".into();
    service
        .rules
        .store
        .save(rule, None, "expired".into())
        .unwrap();
    crate::environments::events::EventStore::open(&config)
        .unwrap()
        .ingest(
            &token,
            Batch {
                events: vec![crate::environments::events::tests::event("one")],
            },
        )
        .unwrap();
    service.rule_cycle().unwrap();
    let mut acceptance = service
        .rules
        .store
        .snapshot()
        .unwrap()
        .acceptances
        .remove(0);
    let origin = acceptance.operation_id.clone().unwrap();
    assert_eq!(acceptance.instance, "pr-82");
    assert_eq!(
        acceptance.instance_description(),
        "slack://channel?team=T1&id=C2"
    );
    let failed = service
        .runtime
        .block_on(service.wait_operation(&origin))
        .unwrap();
    assert_eq!(failed.state, OperationState::Failed);
    service.rule_cycle().unwrap();
    assert_eq!(
        service
            .rules
            .store
            .acceptance(acceptance.id)
            .unwrap()
            .status,
        DispatchStatus::Failed
    );
    fs::remove_file(seed.join("unsafe")).unwrap();
    acceptance = service
        .runtime
        .block_on(service.retry_acceptance(acceptance.id, true))
        .unwrap();
    service.advance_dispatch(&mut acceptance).unwrap();
    let retry = startup::read(&config, &acceptance.instance)
        .unwrap()
        .unwrap();
    assert_ne!(retry.operation.id, origin);
    assert_eq!(retry.origin_operation_id(), origin);
    assert_eq!(
        retry.description.as_deref(),
        Some("slack://channel?team=T1&id=C2")
    );
    assert_eq!(acceptance.operation_id.as_deref(), Some(origin.as_str()));
    let ready = service
        .runtime
        .block_on(service.wait_operation(&retry.operation.id))
        .unwrap();
    assert_eq!(ready.state, OperationState::Succeeded, "{:?}", ready.error);
    fs::write(config.home.join("rule-sessions"), "dead [Created 1s ago] (EXITED - attach to resurrect)\nnewest [Created 2s ago]\nmain [Created 3s ago]\n").unwrap();
    service.advance_dispatch(&mut acceptance).unwrap();
    assert_eq!(acceptance.status, DispatchStatus::Launching);
    assert_eq!(acceptance.pane.as_ref().unwrap().session, "newest");
    assert_eq!(acceptance.rule.zellij_session, "expired");
    let directory = config.workspaces.join(&acceptance.instance);
    let server = history_server::Server::start();
    server.session("ses_retained", directory.to_str().unwrap(), None);
    {
        let mut data = server.data.lock().unwrap();
        data.sessions.get_mut("ses_retained").unwrap()["title"] =
            serde_json::json!("Retained inspection");
        data.sessions.get_mut("ses_retained").unwrap()["time"] = serde_json::json!({"updated": 1});
    }
    fs::create_dir_all(&presence).unwrap();
    let receipt = presence.join("history.json");
    fs::write(&receipt, serde_json::json!({
        "pid": std::process::id(), "observed_at": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis(),
        "id": "ses_retained", "title": "Retained inspection", "activity": "idle", "server": server.url, "directory": directory,
        "zellij_session": "newest", "pane_id": 100,
    }).to_string()).unwrap();
    service.advance_dispatch(&mut acceptance).unwrap();
    assert_eq!(acceptance.status, DispatchStatus::Launched);
    assert!(service.opencode_snapshot().sessions.is_empty());
    let retained =
        service.rules.store.snapshot().unwrap().workspaces[&acceptance.id].sessions[0].clone();
    assert_eq!(retained.server, server.url);
    assert_eq!(retained.directory, directory.to_str().unwrap());
    assert!(retained.panes.is_empty());
    fs::remove_file(receipt).unwrap();
    let purge = service
        .submit_operation("delete_instance", &acceptance.instance, None, 60, true)
        .unwrap();
    let purged = service
        .runtime
        .block_on(service.wait_operation(&purge.id))
        .unwrap();
    assert_eq!(
        purged.state,
        OperationState::Succeeded,
        "{:?}",
        purged.error
    );
    assert!(!directory.exists());
    assert_eq!(
        service.rules.store.snapshot().unwrap().workspaces[&acceptance.id].sessions,
        vec![retained]
    );
    service
        .recreate_acceptance(acceptance.id, Some("ses_retained".into()), true)
        .blocking_recv()
        .unwrap()
        .unwrap();
    assert!(directory.exists());
    let recreated = startup::read(&config, &acceptance.instance)
        .unwrap()
        .unwrap();
    assert_eq!(
        recreated.description.as_deref(),
        Some("slack://channel?team=T1&id=C2")
    );
    let connection = rusqlite::Connection::open(config.home.join("settings.sqlite3")).unwrap();
    let names: i64 = connection
        .query_row("SELECT count(*) FROM rule_instance_names", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(names, 1);
    assert_eq!(
        service.rules.store.acceptance(acceptance.id).unwrap(),
        acceptance
    );
    assert_eq!(
        startup::read(&config, &acceptance.instance)
            .unwrap()
            .unwrap()
            .origin_operation_id(),
        origin
    );
    let calls = fs::read_to_string(config.home.join("zellij-calls")).unwrap();
    assert!(
        calls.contains(&format!(
            "opencode attach {} --dir {} --session ses_retained",
            server.url,
            directory.display()
        )),
        "{calls}"
    );
    assert!(server.data.lock().unwrap().deleted.is_empty());
}
