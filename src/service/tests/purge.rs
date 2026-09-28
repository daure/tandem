use super::*;
use crate::environments::opencode::Observer;
use crate::store::environments::OperationState;
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

#[test]
fn purge_operations_close_clients_before_removing_workspaces() {
    for action in ["delete_instance", "delete_template", "batch"] {
        let mut service = AppService::for_tests();
        let home = service.environments.config.home.clone();
        let workspace = service.environments.config.workspaces.join("review");
        let observer = Observer {
            presence: home.join("presence"),
            daemons: home.join("daemons"),
            zellij: home.join("zellij"),
        };
        fs::create_dir(&observer.presence).unwrap();
        fs::write(
            &observer.zellij,
            format!(
                r#"#!/bin/sh
root=$(dirname "$0")
case "$*" in
  list-sessions*) printf 'main\n' ;;
  *list-panes*)
    if [ -f "$root/closed" ]; then printf '[]'; else
      printf '[{{"id":7,"is_plugin":false,"exited":false,"tab_id":4,"tab_name":"Review"}}]'
    fi ;;
  *close-pane*)
    test -d '{}' || exit 1
    touch "$root/closed" ;;
esac
"#,
                workspace.display()
            ),
        )
        .unwrap();
        fs::set_permissions(&observer.zellij, fs::Permissions::from_mode(0o700)).unwrap();
        Arc::get_mut(&mut service.opencode).unwrap().observer = observer.clone();
        service
            .runtime
            .block_on(service.set_clear_opencode_history(false).unwrap())
            .unwrap()
            .unwrap();
        service
            .runtime
            .block_on(service.create_template("local".into()))
            .unwrap();
        let creation = service
            .submit_operation("create_instance", "review", Some("local".into()), 60, true)
            .unwrap();
        let created = service
            .runtime
            .block_on(service.wait_operation(&creation.id))
            .unwrap();
        assert_eq!(
            created.state,
            OperationState::Succeeded,
            "{:?}",
            created.error
        );
        fs::write(observer.presence.join("client.json"), serde_json::json!({
            "pid": std::process::id(),
            "observed_at": SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64,
            "id": "ses_one", "title": "Review", "directory": workspace,
            "server": "", "activity": "idle", "zellij_session": "main", "pane_id": 7
        }).to_string()).unwrap();
        assert!(service.opencode_snapshot().sessions.is_empty());
        let operation = if action == "batch" {
            service
                .submit_instance_batch("delete_instance", &["review".into()], true)
                .unwrap()
                .operations
                .remove(0)
        } else {
            service
                .submit_operation(
                    action,
                    if action == "delete_template" {
                        "local"
                    } else {
                        "review"
                    },
                    None,
                    60,
                    true,
                )
                .unwrap()
        };
        let deleted = service
            .runtime
            .block_on(service.wait_operation(&operation.id))
            .unwrap();
        assert_eq!(
            deleted.state,
            OperationState::Succeeded,
            "{:?}",
            deleted.error
        );
        assert!(home.join("closed").exists(), "{action}");
        assert!(!workspace.exists(), "{action}");
    }
}
