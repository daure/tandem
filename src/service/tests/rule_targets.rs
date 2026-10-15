use super::*;
use crate::store::{
    environments::EnvironmentSnapshot,
    opencode::{Session, Snapshot as OpenCodeSnapshot},
};
use std::{fs, os::unix::fs::PermissionsExt};

fn fixture(existing: bool) -> AppService {
    fixture_with_session(existing, None)
}

fn fixture_with_session(existing: bool, session: Option<&str>) -> AppService {
    let mut service = AppService::for_tests();
    service
        .runtime
        .block_on(service.set_clear_opencode_history(false).unwrap())
        .unwrap()
        .unwrap();
    for template in ["blank", "other"] {
        service
            .runtime
            .block_on(service.create_template(template.into()))
            .unwrap();
    }
    let config = service.environments.config.clone();
    let zellij = config.home.join("target-zellij");
    fs::write(&zellij, r#"#!/bin/sh
root=$(dirname "$0")
printf '%s\n' "$*" >> "$root/target-calls"
case "$*" in
  list-sessions*)
    case "$*" in
      *--short*) printf 'main\n' ;;
      *) printf 'main [Created 1s ago]\n' ;;
    esac ;;
  *list-panes*) printf '[{"id":100,"is_plugin":false,"exited":false,"tab_id":9,"tab_name":"Review"}]' ;;
  *new-tab*) printf '9\n' ;;
esac
"#).unwrap();
    fs::set_permissions(&zellij, fs::Permissions::from_mode(0o700)).unwrap();
    Arc::get_mut(&mut service.opencode).unwrap().observer.zellij = zellij;
    Arc::get_mut(&mut service.opencode)
        .unwrap()
        .observer
        .presence = config.home.join("target-presence");
    if existing {
        let operation = service
            .submit_operation("create_instance", "shared", Some("other".into()), 60, true)
            .unwrap();
        let ready = service
            .runtime
            .block_on(service.wait_operation(&operation.id))
            .unwrap();
        assert_eq!(ready.state, OperationState::Succeeded, "{:?}", ready.error);
        fs::write(config.workspaces.join("other/shared/keep.txt"), "user work").unwrap();
    }
    let mut rule = definition("inspect");
    rule.target_instance = Some("shared".into());
    rule.session_name = session.map(str::to_owned);
    rule.script = "fn matches(event) { true } fn instance_name(event) { \"ignored\" }".into();
    service.rules.store.save(rule, None, "main".into()).unwrap();
    let events = crate::environments::events::EventStore::open(&config).unwrap();
    let token = events.register_provider("sample").unwrap();
    events
        .ingest(
            &token,
            Batch {
                events: vec![
                    crate::environments::events::tests::event("one"),
                    crate::environments::events::tests::event("two"),
                ],
            },
        )
        .unwrap();
    service.rules.store.evaluate().unwrap();
    service
}

#[path = "rule_sessions.rs"]
mod sessions;

fn launches(service: &AppService) -> usize {
    fs::read_to_string(service.environments.config.home.join("target-calls"))
        .unwrap_or_default()
        .lines()
        .filter(|line| line.contains("new-tab"))
        .count()
}

#[test]
fn named_targets_reuse_other_templates_and_retain_only_their_conversation() {
    let service = fixture(true);
    let config = &service.environments.config;
    let acceptance = service
        .rules
        .store
        .snapshot()
        .unwrap()
        .acceptances
        .remove(0);
    let original = startup::read(config, "shared").unwrap().unwrap();
    let lock = crate::environments::gateway::lock(config, "instance-shared").unwrap();
    let mut available = 4;
    service
        .advance_retained_acceptance(&acceptance, &mut available)
        .unwrap();
    assert_eq!(available, 4);
    assert_eq!(launches(&service), 0);
    assert_eq!(
        service
            .rules
            .store
            .acceptance(acceptance.id)
            .unwrap()
            .status,
        DispatchStatus::Queued
    );
    drop(lock);
    service
        .advance_retained_acceptance(&acceptance, &mut available)
        .unwrap();
    let mut launched = service.rules.store.acceptance(acceptance.id).unwrap();
    assert_eq!(launched.status, DispatchStatus::Launching);
    assert_eq!(launched.instance, "shared");
    assert!(launched.operation_id.is_none());
    assert!(launched.target_workspace_identity.is_some());
    assert_eq!(
        startup::read(config, "shared")
            .unwrap()
            .unwrap()
            .operation
            .id,
        original.operation.id
    );
    assert_eq!(
        fs::read_to_string(config.workspaces.join("other/shared/keep.txt")).unwrap(),
        "user work"
    );
    let instance = service
        .environments
        .recorded_instance("shared")
        .unwrap()
        .unwrap();
    assert_eq!(instance.template, "other");
    assert!(
        service
            .rules
            .store
            .instance_acceptance(&instance)
            .unwrap()
            .is_none()
    );
    let unrelated = Session {
        id: "ses_unrelated".into(),
        directory: instance.workspace.clone(),
        ..Default::default()
    };
    let own = Session {
        id: "ses_own".into(),
        directory: instance.workspace.clone(),
        panes: vec![launched.pane.clone().unwrap()],
        ..Default::default()
    };
    service
        .rules
        .store
        .remember_workspaces(
            &EnvironmentSnapshot {
                instances: vec![instance.clone()],
                ..Default::default()
            },
            &OpenCodeSnapshot {
                sessions: vec![unrelated, own],
                ..Default::default()
            },
        )
        .unwrap();
    let history = RuleStore::open(config)
        .unwrap()
        .snapshot()
        .unwrap()
        .workspaces;
    assert_eq!(history[&launched.id].sessions.len(), 1);
    assert_eq!(history[&launched.id].sessions[0].id, "ses_own");
    let server = crate::environments::opencode::tests::history_server::Server::start();
    server.session("ses_own", &instance.workspace, None);
    let presence = &service.opencode.observer.presence;
    fs::create_dir_all(presence).unwrap();
    fs::write(presence.join("target.json"), serde_json::json!({
        "pid": std::process::id(),
        "observed_at": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis(),
        "id": "ses_own", "title": "Review", "activity": "idle", "server": server.url,
        "directory": instance.workspace, "zellij_session": "main", "pane_id": 100,
    }).to_string()).unwrap();
    service
        .advance_retained_acceptance(&launched, &mut available)
        .unwrap();
    launched = service.rules.store.acceptance(launched.id).unwrap();
    assert_eq!(launched.status, DispatchStatus::Launched);
    assert_eq!(launched.session_id.as_deref(), Some("ses_own"));
    assert!(
        crate::environments::rules::acceptance_workspace_matches(config, &instance, &launched)
            .unwrap()
    );
    fs::rename(
        config.workspaces.join("other/shared"),
        config.workspaces.join("other/previous"),
    )
    .unwrap();
    fs::create_dir(config.workspaces.join("other/shared")).unwrap();
    assert!(
        !crate::environments::rules::acceptance_workspace_matches(config, &instance, &launched)
            .unwrap()
    );
    launched.status = DispatchStatus::Launching;
    launched.pane = None;
    service.rules.store.update(&launched, false).unwrap();
    service
        .advance_retained_acceptance(&launched, &mut available)
        .unwrap();
    assert_eq!(
        service.rules.store.acceptance(launched.id).unwrap().status,
        DispatchStatus::Uncertain
    );
    assert_eq!(launches(&service), 1);
}

#[test]
fn missing_named_targets_create_once_and_later_matches_reuse_the_exact_name() {
    let service = fixture(false);
    let config = &service.environments.config;
    service.rule_cycle().unwrap();
    let first = startup::read(config, "shared").unwrap().unwrap();
    let ready = service
        .runtime
        .block_on(service.wait_operation(&first.operation.id))
        .unwrap();
    assert_eq!(ready.state, OperationState::Succeeded, "{:?}", ready.error);
    service.rule_cycle().unwrap();
    let acceptances = service.rules.store.snapshot().unwrap().acceptances;
    assert_eq!(acceptances.len(), 2);
    for acceptance in &acceptances {
        assert_eq!(acceptance.instance, "shared");
        assert_eq!(
            acceptance.status,
            DispatchStatus::Launching,
            "{:?}",
            acceptance.error
        );
        assert!(acceptance.target_workspace_identity.is_some());
    }
    assert_eq!(
        acceptances
            .iter()
            .filter(|row| row.operation_id.is_some())
            .count(),
        1
    );
    assert_eq!(
        startup::read(config, "shared")
            .unwrap()
            .unwrap()
            .operation
            .id,
        first.operation.id
    );
    assert_eq!(
        service
            .environments
            .recorded_instance("shared")
            .unwrap()
            .unwrap()
            .template,
        "blank"
    );
    assert_eq!(launches(&service), 2);
    assert!(!config.workspaces.join("blank/shared-2").exists());
}

#[test]
fn named_target_creation_retries_wait_for_successful_preparation_before_launch() {
    let service = fixture(false);
    let config = &service.environments.config;
    let mut acceptance = service
        .rules
        .store
        .snapshot()
        .unwrap()
        .acceptances
        .remove(0);
    let mut available = 4;
    service
        .advance_retained_acceptance(&acceptance, &mut available)
        .unwrap();
    acceptance = service.rules.store.acceptance(acceptance.id).unwrap();
    let origin = acceptance.operation_id.clone().unwrap();
    let ready = service
        .runtime
        .block_on(service.wait_operation(&origin))
        .unwrap();
    assert_eq!(ready.state, OperationState::Succeeded);
    let mut startup = startup::read(config, "shared").unwrap().unwrap();
    startup.operation.state = OperationState::Failed;
    startup.operation.error = Some("service readiness failed".into());
    let connection = rusqlite::Connection::open(config.home.join("settings.sqlite3")).unwrap();
    connection.execute(
        "UPDATE runtime_records SET payload=?1 WHERE namespace=?2 AND name='shared' AND kind='startup'",
        rusqlite::params![serde_json::to_string(&startup).unwrap(), config.namespace],
    ).unwrap();
    service
        .advance_retained_acceptance(&acceptance, &mut available)
        .unwrap();
    let failed = service.rules.store.acceptance(acceptance.id).unwrap();
    assert_eq!(failed.status, DispatchStatus::Failed);
    assert_eq!(failed.error.as_deref(), Some("service readiness failed"));
    let queued = service
        .runtime
        .block_on(service.retry_acceptance(failed.id, true))
        .unwrap();
    service
        .advance_retained_acceptance(&queued, &mut available)
        .unwrap();
    let retried = service.rules.store.acceptance(failed.id).unwrap();
    assert_eq!(retried.status, DispatchStatus::Provisioning);
    assert_eq!(retried.operation_id.as_deref(), Some(origin.as_str()));
    assert_eq!(launches(&service), 0);
    let retry = startup::read(config, "shared").unwrap().unwrap();
    assert_ne!(retry.operation.id, origin);
    assert_eq!(retry.origin_operation_id(), origin);
    let ready = service
        .runtime
        .block_on(service.wait_operation(&retry.operation.id))
        .unwrap();
    assert_eq!(ready.state, OperationState::Succeeded);
    service
        .advance_retained_acceptance(&retried, &mut available)
        .unwrap();
    assert_eq!(
        service.rules.store.acceptance(failed.id).unwrap().status,
        DispatchStatus::Launching
    );
    assert_eq!(launches(&service), 1);
}

#[test]
fn named_targets_reject_unowned_paths_and_foreign_namespace_records() {
    for foreign in [false, true] {
        let service = fixture(false);
        let config = &service.environments.config;
        if foreign {
            let connection =
                rusqlite::Connection::open(config.home.join("settings.sqlite3")).unwrap();
            connection
                .execute(
                    "INSERT INTO runtime_records VALUES ('foreign', 'shared', 'ownership', '{}')",
                    [],
                )
                .unwrap();
        } else {
            fs::create_dir(config.workspaces.join("shared")).unwrap();
            fs::write(config.workspaces.join("shared/keep.txt"), "user work").unwrap();
        }
        service.rule_cycle().unwrap();
        for acceptance in service.rules.store.snapshot().unwrap().acceptances {
            assert_eq!(acceptance.instance, "shared");
            assert_eq!(acceptance.status, DispatchStatus::Failed);
            assert!(acceptance.launch_started_at.is_none());
        }
        assert_eq!(launches(&service), 0);
        assert!(startup::read(config, "shared").unwrap().is_none());
        if !foreign {
            assert_eq!(
                fs::read_to_string(config.workspaces.join("shared/keep.txt")).unwrap(),
                "user work"
            );
        }
    }
}
