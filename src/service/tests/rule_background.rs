use super::*;
use crate::{
    environments::opencode::tests::history_server::Server,
    store::opencode::{Launch, PromptOutcome, SessionPrompt},
};
use serde_json::json;

fn background_fixture(existing: bool, named: bool) -> (AppService, Server) {
    let mut rule = definition("inspect");
    rule.target_instance = Some("shared".into());
    rule.session_name = named.then(|| "reply-drafts".into());
    rule.open_client = false;
    rule.variant = Some("high".into());
    rule.follow_up_prompt = Some("Follow {{event.data.text}}".into());
    let mut service = fixture_with_rule(existing, rule);
    let config = service.config_for_tests();
    let observer = &mut Arc::get_mut(&mut service.opencode).unwrap().observer;
    observer.zellij = config.home.join("no-zellij");
    observer.daemons = config.home.join("background-daemons");
    let server = Server::start();
    server.data.lock().unwrap().v2 = true;
    let station = observer.daemons.join("station");
    fs::create_dir_all(station.join("dirs")).unwrap();
    fs::write(station.join("port"), server.url.rsplit(':').next().unwrap()).unwrap();
    let directory = if existing {
        config.workspaces.join("other/shared")
    } else {
        config.workspaces.join("blank/shared")
    };
    fs::write(station.join("dirs/work.dir"), directory.to_str().unwrap()).unwrap();
    server.session("ses_rule_author", directory.to_str().unwrap(), None);
    (service, server)
}

#[test]
fn clientless_rules_retain_exact_created_sessions_and_publish_detached_history() {
    let (service, server) = background_fixture(true, false);
    service.rule_cycle().unwrap();
    let snapshot = service.rules.store.snapshot().unwrap();
    assert_eq!(snapshot.acceptances.len(), 2);
    let mut ids = Vec::new();
    for row in &snapshot.acceptances {
        assert_eq!(row.status, DispatchStatus::Launched, "{:?}", row.error);
        assert_eq!(row.prompt_outcome, Some(PromptOutcome::Submitted));
        assert_eq!(row.session_server.as_deref(), Some(server.url.as_str()));
        assert_eq!(row.pane, None);
        let id = row.session_id.clone().unwrap();
        assert_ne!(id, "ses_rule_author");
        assert_eq!(
            snapshot.workspaces[&row.id]
                .sessions
                .iter()
                .map(|session| &session.id)
                .collect::<Vec<_>>(),
            [&id]
        );
        ids.push(id);
    }
    assert_ne!(ids[0], ids[1]);
    let boundary = service.state_stream.boundary(None).unwrap();
    let retained = boundary.snapshot.sessions["sessions"].as_array().unwrap();
    assert_eq!(retained.len(), 2);
    assert!(
        retained
            .iter()
            .all(|session| session["panes"] == json!([]) && session["attached"] == false)
    );
    let listing = service
        .runtime
        .block_on(service.list_sessions(Some("shared".into()), true))
        .unwrap();
    let value = serde_json::to_value(listing).unwrap();
    for id in ids {
        assert!(
            value["sessions"]
                .as_array()
                .unwrap()
                .iter()
                .any(|session| session["session_id"] == id && session["attached"] == false)
        );
    }
    let data = server.data.lock().unwrap();
    let creates: Vec<_> = data
        .background_calls
        .iter()
        .filter(|(path, _)| path == "/api/session")
        .collect();
    assert_eq!(creates.len(), 2);
    assert_eq!(
        creates[0].1["model"],
        json!({"providerID":"openai","id":"test","variant":"high"})
    );
    assert_eq!(launches(&service), 0);
    assert_eq!(
        fs::read_to_string(
            service
                .config_for_tests()
                .workspaces
                .join("other/shared/keep.txt")
        )
        .unwrap(),
        "user work"
    );
}

#[test]
fn clientless_rules_prepare_missing_targets_without_zellij() {
    let (service, _server) = background_fixture(false, false);
    service.rule_cycle().unwrap();
    let operation = startup::read(&service.config_for_tests(), "shared")
        .unwrap()
        .unwrap()
        .operation
        .id;
    assert_eq!(
        service
            .runtime
            .block_on(service.wait_operation(&operation))
            .unwrap()
            .state,
        OperationState::Succeeded
    );
    service.rule_cycle().unwrap();
    let rows = service.rules.store.snapshot().unwrap().acceptances;
    assert!(
        rows.iter()
            .all(|row| row.status == DispatchStatus::Launched),
        "{rows:?}"
    );
    assert_eq!(launches(&service), 0);
}

#[test]
fn clientless_named_rules_reuse_identity_and_apply_delivery_modes() {
    for mode in [
        crate::store::rules::DeliveryMode::Queue,
        crate::store::rules::DeliveryMode::Steer,
        crate::store::rules::DeliveryMode::Interrupt,
    ] {
        let (service, server) = background_fixture(true, true);
        for mut acceptance in service.rules.store.snapshot().unwrap().acceptances {
            acceptance.rule.definition.delivery_mode = mode;
            service.rules.store.update(&acceptance, false).unwrap();
        }
        service.rule_cycle().unwrap();
        let rows = service.rules.store.snapshot().unwrap().acceptances;
        assert!(
            rows.iter()
                .all(|row| row.status == DispatchStatus::Launched),
            "{rows:?}"
        );
        assert_eq!(rows[0].session_id, rows[1].session_id);
        let data = server.data.lock().unwrap();
        assert_eq!(
            data.background_calls
                .iter()
                .filter(|(path, _)| path == "/api/session")
                .count(),
            1
        );
        let prompts: Vec<_> = data
            .background_calls
            .iter()
            .filter(|(path, _)| path.ends_with("/prompt"))
            .collect();
        assert_eq!(prompts.len(), 2);
        assert_eq!(
            prompts[0].1["text"],
            rows[1].resolved_prompt.as_deref().unwrap()
        );
        assert_eq!(
            prompts[1].1["text"],
            rows[0].resolved_follow_up_prompt.as_deref().unwrap()
        );
        assert_eq!(
            prompts[1].1["delivery"],
            if mode == crate::store::rules::DeliveryMode::Queue {
                "queue"
            } else {
                "steer"
            }
        );
        assert_eq!(
            data.background_calls
                .iter()
                .filter(|(path, _)| path.ends_with("/interrupt?resume=false"))
                .count(),
            usize::from(mode == crate::store::rules::DeliveryMode::Interrupt)
        );
        assert_eq!(launches(&service), 0);
    }
}

#[test]
fn clientless_rule_uncertainty_preserves_identity_without_resending() {
    for phase in ["create", "prompt", "instructions", "identity"] {
        let (service, server) = background_fixture(true, false);
        {
            let mut data = server.data.lock().unwrap();
            data.create_disconnect = phase == "create";
            data.prompt_disconnect = phase == "prompt";
            data.instruction_failure = phase == "instructions";
            data.create_wrong_directory = phase == "identity";
        }
        service.rule_cycle().unwrap();
        let rows = service.rules.store.snapshot().unwrap().acceptances;
        for row in rows {
            assert_eq!(
                row.status,
                if phase == "instructions" {
                    DispatchStatus::Failed
                } else {
                    DispatchStatus::Uncertain
                },
                "{phase}: {row:?}"
            );
            assert_eq!(
                row.session_id.is_some(),
                matches!(phase, "prompt" | "instructions")
            );
        }
        let calls = server.data.lock().unwrap().background_calls.clone();
        service.rule_cycle().unwrap();
        assert_eq!(server.data.lock().unwrap().background_calls, calls);
        assert_eq!(
            calls
                .iter()
                .filter(|(path, _)| path.ends_with("/prompt"))
                .count(),
            if phase == "prompt" { 2 } else { 0 }
        );
    }
}

#[test]
fn fresh_instance_session_can_submit_without_a_client_and_bind_its_origin() {
    let (service, server) = background_fixture(true, false);
    let input = SessionPrompt {
        launch: Launch {
            prompt: Some("Draft the mention reply".into()),
            model: Some("openai/test#low".into()),
            ..Default::default()
        },
        agent: Some("reply-agent".into()),
    };
    let receipt = service
        .prompt_instance_session_options(
            "shared".into(),
            input,
            true,
            Some("reply-request".into()),
            false,
        )
        .unwrap()
        .blocking_recv()
        .unwrap()
        .unwrap();
    assert_eq!(
        receipt.prompt_outcome,
        PromptOutcome::Submitted,
        "{:?}",
        receipt.error
    );
    assert_eq!(receipt.pane, None);
    assert_eq!(receipt.server.as_deref(), Some(server.url.as_str()));
    let boundary = service.state_stream.boundary(None).unwrap();
    let published = boundary.snapshot.sessions["sessions"].as_array().unwrap();
    assert_eq!(published.len(), 1);
    assert_eq!(published[0]["session_id"], receipt.session_id.unwrap());
    assert_eq!(published[0]["origins"], json!(["reply-request"]));
    let data = server.data.lock().unwrap();
    assert_eq!(data.background_calls[0].1["agent"], "reply-agent");
    assert_eq!(
        data.background_calls[0].1["model"],
        json!({"providerID":"openai","id":"test","variant":"low"})
    );
    assert_eq!(launches(&service), 0);
}

#[test]
fn clientless_sessions_use_server_defaults_and_project_variant_only_model_selection() {
    for variant in [None, Some("balanced".to_owned())] {
        let (service, server) = background_fixture(true, false);
        let input = SessionPrompt {
            launch: Launch {
                prompt: Some("Draft a response".into()),
                variant: variant.clone(),
                ..Default::default()
            },
            agent: None,
        };
        let receipt = service
            .prompt_instance_session_options("shared".into(), input, true, None, false)
            .unwrap()
            .blocking_recv()
            .unwrap()
            .unwrap();
        assert_eq!(receipt.prompt_outcome, PromptOutcome::Submitted);
        let data = server.data.lock().unwrap();
        let mut expected = json!({"location":{"directory":service.config_for_tests().workspaces.join("other/shared")}});
        if let Some(variant) = variant {
            expected["model"] =
                json!({"providerID":"default-provider","id":"default-model","variant":variant});
        }
        assert_eq!(data.background_calls[0], ("/api/session".into(), expected));
    }
}

#[test]
fn replay_pins_client_opening_without_changing_rules_or_submission_retries() {
    let (service, _server) = background_fixture(true, false);
    let config = service.config_for_tests();
    let events = crate::environments::events::EventStore::open(&config).unwrap();
    let mut rule = service.rules.store.snapshot().unwrap().rules.remove(0);
    rule.definition.open_client = true;
    service
        .rules
        .store
        .save(rule.definition, Some(rule.revision), "main".into())
        .unwrap();
    let attempt = events
        .replay_options(1, "quiet-replay", Some(false))
        .unwrap();
    assert_eq!(
        events
            .replay_options(1, "quiet-replay", Some(true))
            .unwrap(),
        attempt
    );
    service.rules.store.evaluate().unwrap();
    let snapshot = service.rules.store.snapshot().unwrap();
    assert!(snapshot.rules[0].definition.open_client);
    let replayed = snapshot
        .acceptances
        .iter()
        .find(|row| row.attempt_id == attempt)
        .unwrap();
    assert!(!replayed.rule.definition.open_client);
    service.rule_cycle().unwrap();
    assert_eq!(
        service.rules.store.acceptance(replayed.id).unwrap().status,
        DispatchStatus::Launched
    );
}

#[test]
fn clientless_rule_activation_does_not_require_zellij() {
    let (service, _server) = background_fixture(true, false);
    let rule = service.rules.store.snapshot().unwrap().rules.remove(0);
    let saved = service
        .save_rule(rule.definition, Some(rule.revision), None, true)
        .blocking_recv()
        .unwrap()
        .unwrap();
    assert!(saved.definition.enabled);
    assert!(!saved.definition.open_client);
    assert_eq!(launches(&service), 0);
}

#[test]
fn clientless_dispatch_rejects_missing_and_ambiguous_servers_before_creation() {
    for ambiguous in [false, true] {
        let (service, server) = background_fixture(true, false);
        let second = Server::start();
        second.data.lock().unwrap().v2 = true;
        if ambiguous {
            let station = service.opencode.observer.daemons.join("second");
            fs::create_dir_all(station.join("dirs")).unwrap();
            fs::write(station.join("port"), second.url.rsplit(':').next().unwrap()).unwrap();
            fs::write(
                station.join("dirs/work.dir"),
                service
                    .config_for_tests()
                    .workspaces
                    .join("other/shared")
                    .to_str()
                    .unwrap(),
            )
            .unwrap();
        } else {
            fs::remove_file(service.opencode.observer.daemons.join("station/port")).unwrap();
        }
        service.rule_cycle().unwrap();
        let rows = service.rules.store.snapshot().unwrap().acceptances;
        assert!(
            rows.iter()
                .all(|row| row.status == DispatchStatus::Failed && row.launch_started_at.is_none())
        );
        assert!(server.data.lock().unwrap().background_calls.is_empty());
        assert!(second.data.lock().unwrap().background_calls.is_empty());
    }
}
