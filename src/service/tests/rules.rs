use super::*;
use crate::store::events::{Batch, Deletion};

#[path = "dispatch_history.rs"]
mod history;

impl AppService {
    pub(crate) fn set_rule_session_for_tests(&mut self, current: &str) {
        use std::os::unix::fs::PermissionsExt;
        let home = &self.environments.config.home;
        let program = home.join("rule-zellij");
        std::fs::write(
            &program,
            "#!/bin/sh\nroot=$(dirname \"$0\")\ncase \"$*\" in\n  list-sessions*) cat \"$root/rule-sessions\";;\n  *) exit 1;;\nesac\n",
        )
        .unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::write(home.join("rule-sessions"), format!("main\n{current}\n")).unwrap();
        let integration = Arc::get_mut(&mut self.opencode).unwrap();
        integration.observer.zellij = program;
        integration.current_zellij = current.into();
    }
}

fn definition(name: &str) -> Definition {
    Definition {
        name: name.into(),
        description: "Fixture rule".into(),
        script: "fn matches(event) { true }".into(),
        template: "blank".into(),
        model: "openai/test".into(),
        variant: None,
        initial_prompt: "Inspect {{event.data.text}}".into(),
        enabled: true,
        start_instance: true,
    }
}

#[test]
fn dispatch_skips_busy_events_without_blocking_unrelated_work() {
    let service = AppService::for_tests();
    service
        .runtime
        .block_on(service.set_opencode_enabled(false).unwrap())
        .unwrap()
        .unwrap();
    service
        .rules
        .store
        .save(definition("inspect"), None, "main".into())
        .unwrap();
    let token = service.register_provider_for_tests("sample");
    service
        .runtime
        .block_on(service.ingest_events(
            token,
            Batch {
                events: vec![
                    crate::environments::events::tests::event("first"),
                    crate::environments::events::tests::event("second"),
                ],
            },
        ))
        .unwrap();
    service.rules.store.evaluate().unwrap();
    let snapshot = service.rules.store.snapshot().unwrap();
    let busy = &snapshot.acceptances[1];
    let _event = service
        .rules
        .store
        .event_lease(busy.event_sequence)
        .unwrap()
        .unwrap();
    service.rule_cycle().unwrap();
    assert_eq!(
        service.rules.store.acceptance(busy.id).unwrap().status,
        DispatchStatus::Queued
    );
    assert_eq!(
        service
            .rules
            .store
            .acceptance(snapshot.acceptances[0].id)
            .unwrap()
            .status,
        DispatchStatus::Failed
    );
    assert!(service.operations().is_empty());
}

#[test]
fn stale_dispatch_snapshots_do_not_admit_or_launch_deleted_acceptances() {
    let service = AppService::for_tests();
    service
        .rules
        .store
        .save(definition("inspect"), None, "main".into())
        .unwrap();
    let token = service.register_provider_for_tests("sample");
    service
        .runtime
        .block_on(service.ingest_events(
            token,
            Batch {
                events: vec![crate::environments::events::tests::event("one")],
            },
        ))
        .unwrap();
    service.rules.store.evaluate().unwrap();
    let requested = service
        .rules
        .store
        .snapshot()
        .unwrap()
        .acceptances
        .remove(0);
    service
        .delete_events(Deletion::Event(requested.event_sequence))
        .blocking_recv()
        .unwrap()
        .unwrap();
    let mut available = 4;
    service
        .advance_retained_acceptance(&requested, &mut available)
        .unwrap();
    assert_eq!(available, 4);
    assert!(service.operations().is_empty());
    assert!(
        service
            .rules
            .store
            .snapshot()
            .unwrap()
            .acceptances
            .is_empty()
    );
    let connection =
        rusqlite::Connection::open(service.environments.config.home.join("settings.sqlite3"))
            .unwrap();
    let starts: i64 = connection
        .query_row("SELECT count(*) FROM rule_dispatch_starts", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(starts, 0);
}

#[test]
fn acceptance_recreation_requires_confirmation_and_preserves_the_original_dispatch() {
    let service = AppService::for_tests();
    service
        .runtime
        .block_on(service.create_template("blank".into()))
        .unwrap();
    let mut rule = definition("inspect");
    rule.start_instance = false;
    service.rules.store.save(rule, None, "main".into()).unwrap();
    let token = service.register_provider_for_tests("sample");
    crate::environments::events::EventStore::open(&service.environments.config)
        .unwrap()
        .ingest(
            &token,
            Batch {
                events: vec![crate::environments::events::tests::event("one")],
            },
        )
        .unwrap();
    service.rules.store.evaluate().unwrap();
    let mut acceptance = service
        .rules
        .store
        .snapshot()
        .unwrap()
        .acceptances
        .remove(0);
    let error = service
        .recreate_acceptance(acceptance.id, None, false)
        .blocking_recv()
        .unwrap()
        .unwrap_err();
    assert!(error.contains("confirmation_required"), "{error}");
    let error = service
        .recreate_acceptance(acceptance.id, None, true)
        .blocking_recv()
        .unwrap()
        .unwrap_err();
    assert!(error.contains("Dispatch is still active"), "{error}");
    assert!(service.operations().is_empty());
    acceptance.status = DispatchStatus::Launched;
    acceptance.operation_id = Some("123-456-1".into());
    acceptance.session_id = Some("ses_original".into());
    service.rules.store.update(&acceptance, false).unwrap();
    service
        .recreate_acceptance(acceptance.id, None, true)
        .blocking_recv()
        .unwrap()
        .unwrap();
    let record = startup::read(&service.environments.config, &acceptance.instance)
        .unwrap()
        .unwrap();
    assert!(record.preserve_opencode_history);
    assert_eq!(
        Some(record.origin_operation_id()),
        acceptance.operation_id.as_deref()
    );
    assert!(!record.opencode_requested);
    assert!(!record.start_instance);
    assert_eq!(record.operation.state, OperationState::Succeeded);
    assert!(
        service
            .environments
            .config
            .workspaces
            .join(&acceptance.instance)
            .is_dir()
    );
    assert_eq!(
        service.rules.store.snapshot().unwrap().acceptances,
        vec![acceptance.clone()]
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let error = service
            .recreate_acceptance(acceptance.id, None, true)
            .blocking_recv()
            .unwrap()
            .unwrap_err();
        if error.contains("Rule work is in progress") {
            assert!(Instant::now() < deadline, "{error}");
            std::thread::sleep(Duration::from_millis(20));
            continue;
        }
        assert!(error.contains("already in use"), "{error}");
        break;
    }
    crate::environments::providers::Providers::new(&service.environments.config)
        .unwrap()
        .delete_created_instance(
            &acceptance.instance,
            acceptance.operation_id.as_deref().unwrap(),
            &|_, _| Ok(()),
        )
        .unwrap();
    assert!(
        !service
            .environments
            .config
            .workspaces
            .join(&acceptance.instance)
            .exists()
    );
}

#[test]
fn rule_authorization_and_preview_keep_external_actions_explicit() {
    let mut service = AppService::for_tests();
    service.set_rule_session_for_tests("main");
    let template = service.environments.config.templates.join("blank");
    std::fs::create_dir_all(&template).unwrap();
    std::fs::write(template.join("tandem.json"), "{}").unwrap();
    let token = service.register_provider_for_tests("sample");
    let receipt = service
        .runtime
        .block_on(service.ingest_events(
            token,
            Batch {
                events: vec![crate::environments::events::tests::event("one")],
            },
        ))
        .unwrap();
    let mut rule = definition("match");
    rule.variant = Some("high".into());
    let error = service
        .runtime
        .block_on(service.save_rule(rule.clone(), None, Some("main".into()), false))
        .unwrap()
        .unwrap_err();
    assert!(error.contains("confirmation_required"));
    let preview = service
        .runtime
        .block_on(service.preview_rule(rule.clone(), receipt.receipts[0].sequence))
        .unwrap();
    assert!(preview.matched);
    assert_eq!(preview.model, "openai/test");
    assert_eq!(preview.variant.as_deref(), Some("high"));
    assert_eq!(
        preview.resolved_prompt.as_deref(),
        Some("Inspect Please inspect this event")
    );
    assert!(service.operations().is_empty());
    assert!(
        service
            .runtime
            .block_on(service.list_rules())
            .unwrap()
            .rules
            .is_empty()
    );
    let saved = service
        .runtime
        .block_on(service.save_rule(rule, None, Some("main".into()), true))
        .unwrap()
        .unwrap();
    assert_eq!(saved.zellij_session, "main");
    assert_eq!(service.rule_snapshot().rules, vec![saved.clone()]);
    let mut disabled = saved.definition;
    disabled.enabled = false;
    let disabled = service
        .runtime
        .block_on(service.save_rule(disabled, Some(saved.revision), Some("main".into()), false))
        .unwrap()
        .unwrap();
    assert!(!disabled.definition.enabled);
    assert_eq!(disabled.revision, 2);
    assert_eq!(service.rule_snapshot().rules, vec![disabled]);
}

#[test]
fn incomplete_provider_deletion_blocks_queued_dispatch_across_service_restarts() {
    let service = AppService::for_tests();
    let config = &service.environments.config;
    service
        .rules
        .store
        .save(definition("matching"), None, "main".into())
        .unwrap();
    let token = service.register_provider_for_tests("sample");
    service
        .runtime
        .block_on(service.ingest_events(
            token,
            Batch {
                events: vec![crate::environments::events::tests::event("one")],
            },
        ))
        .unwrap();
    service.rules.store.evaluate().unwrap();
    let queued = service.rules.store.snapshot().unwrap().acceptances;
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].status, DispatchStatus::Queued);
    crate::environments::events::EventStore::open(config)
        .unwrap()
        .begin_provider_deletion("package", "sample")
        .unwrap();
    let events = crate::environments::events::EventStore::open(config).unwrap();
    assert!(
        events
            .delete(crate::store::events::Deletion::Event(
                queued[0].event_sequence
            ))
            .unwrap_err()
            .to_string()
            .contains("incomplete provider deletion")
    );
    assert!(
        events
            .delete(crate::store::events::Deletion::All)
            .unwrap_err()
            .to_string()
            .contains("incomplete provider deletion")
    );
    assert!(
        events
            .set_ingestion_enabled("sample", true)
            .unwrap_err()
            .to_string()
            .contains("deletion is incomplete")
    );
    service.rule_cycle().unwrap();
    let restarted = AppService::from_config(config.clone()).unwrap();
    restarted.rule_cycle().unwrap();
    assert_eq!(
        restarted.rules.store.snapshot().unwrap().acceptances,
        queued
    );
    assert!(service.operations().is_empty());
    assert!(restarted.operations().is_empty());
}

#[test]
fn restart_surfaces_interrupted_launches_without_repeating_successful_siblings() {
    let service = AppService::for_tests();
    let config = &service.environments.config;
    let store = &service.rules.store;
    for name in ["success", "interrupted", "preparation"] {
        store.save(definition(name), None, "main".into()).unwrap();
    }
    let token = service.register_provider_for_tests("sample");
    service
        .runtime
        .block_on(service.ingest_events(
            token,
            Batch {
                events: vec![crate::environments::events::tests::event("one")],
            },
        ))
        .unwrap();
    store.evaluate().unwrap();
    let mut successful = None;
    for mut acceptance in store.snapshot().unwrap().acceptances {
        match acceptance.rule_name.as_str() {
            "success" => {
                acceptance.status = DispatchStatus::Launched;
                acceptance.session_id = Some("ses_success".into());
                successful = Some(acceptance.clone());
            }
            "interrupted" => {
                acceptance.status = DispatchStatus::Launching;
                acceptance.launch_started_at = Some(chrono::Utc::now().to_rfc3339());
            }
            "preparation" => {
                acceptance.status = DispatchStatus::Provisioning;
                acceptance.operation_id = Some("1-1".into());
            }
            _ => unreachable!(),
        }
        store.update(&acceptance, false).unwrap();
    }
    let recovered = AppService::from_config(config.clone()).unwrap();
    recovered.rule_cycle().unwrap();
    recovered.rule_cycle().unwrap();
    let snapshot = recovered.rules.store.snapshot().unwrap();
    assert_eq!(snapshot.acceptances.len(), 3);
    assert_eq!(
        snapshot
            .acceptances
            .iter()
            .find(|row| row.rule_name == "success"),
        successful.as_ref()
    );
    let uncertain = snapshot
        .acceptances
        .iter()
        .find(|row| row.rule_name == "interrupted")
        .unwrap();
    assert_eq!(uncertain.status, DispatchStatus::Uncertain);
    assert!(
        uncertain
            .error
            .as_ref()
            .unwrap()
            .contains("uncertain outcome")
    );
    let failed = snapshot
        .acceptances
        .iter()
        .find(|row| row.rule_name == "preparation")
        .unwrap();
    assert_eq!(failed.status, DispatchStatus::Failed);
    assert!(
        recovered
            .runtime
            .block_on(recovered.retry_acceptance(uncertain.id, true))
            .unwrap_err()
            .contains("uncertain")
    );
    assert!(recovered.operations().is_empty());
}

#[test]
fn reported_acceptances_preserve_dispatch_history_and_require_cleanup_retry() {
    let service = AppService::for_tests();
    service
        .rules
        .store
        .save(definition("inspect"), None, "main".into())
        .unwrap();
    let token = service.register_provider_for_tests("sample");
    service
        .runtime
        .block_on(service.ingest_events(
            token,
            Batch {
                events: vec![crate::environments::events::tests::event("one")],
            },
        ))
        .unwrap();
    service.rules.store.evaluate().unwrap();
    let mut acceptance = service
        .rules
        .store
        .snapshot()
        .unwrap()
        .acceptances
        .remove(0);
    service
        .rules
        .store
        .save_report(
            acceptance.id,
            &crate::store::rules::reports::ReportInput {
                title: "Investigation ended".into(),
                summary: "Saved evidence".into(),
                markdown: "# Evidence\nThe work ended before dispatch reconciliation.\n".into(),
            },
        )
        .unwrap();
    service.rule_cycle().unwrap();
    assert_eq!(
        service.rules.store.snapshot().unwrap().acceptances,
        vec![acceptance.clone()]
    );
    assert!(service.operations().is_empty());
    acceptance.status = DispatchStatus::Failed;
    service.rules.store.update(&acceptance, false).unwrap();
    let error = service
        .runtime
        .block_on(service.retry_acceptance(acceptance.id, true))
        .unwrap_err();
    assert!(error.contains("conclusion report"), "{error}");
    assert_eq!(
        service.rules.store.snapshot().unwrap().acceptances,
        vec![acceptance]
    );
}

#[test]
fn saved_rule_states_survive_delayed_observations_and_out_of_order_completions() {
    let mut service = AppService::for_tests();
    service.set_rule_session_for_tests("main");
    service
        .runtime
        .block_on(service.create_template("blank".into()))
        .unwrap();
    let mut target = definition("target");
    target.enabled = false;
    let mut saved = service
        .save_rule(target, None, Some("main".into()), false)
        .blocking_recv()
        .unwrap()
        .unwrap();
    let mut sibling = definition("sibling");
    sibling.enabled = false;
    let sibling = service
        .save_rule(sibling, None, Some("main".into()), false)
        .blocking_recv()
        .unwrap()
        .unwrap();
    for enabled in [true, false] {
        let stale = service.rule_snapshot();
        let revision = service.rules.revision.load(Ordering::Acquire);
        let previous = saved.clone();
        let mut target = saved.definition.clone();
        target.enabled = enabled;
        saved = service
            .save_rule(target, Some(saved.revision), Some("main".into()), true)
            .blocking_recv()
            .unwrap()
            .unwrap();
        let expected = vec![sibling.clone(), saved.clone()];
        assert_eq!(service.rule_snapshot().rules, expected);
        assert!(
            !service
                .rules
                .publish_observation(revision, Ok(stale), Ok(()))
        );
        service.rules.publish_saved(&previous);
        assert_eq!(service.rule_snapshot().rules, expected);
        let revision = service.rules.revision.load(Ordering::Acquire);
        assert!(service.rules.publish_observation(
            revision,
            service.rules.store.snapshot(),
            Ok(())
        ));
        assert_eq!(service.rule_snapshot().rules, expected);
    }
}

#[test]
fn closed_rule_destinations_require_reactivation_before_provisioning() {
    let mut service = AppService::for_tests();
    service.set_rule_session_for_tests("today");
    service
        .runtime
        .block_on(service.create_template("blank".into()))
        .unwrap();
    let saved = service
        .save_rule(definition("target"), None, None, true)
        .blocking_recv()
        .unwrap()
        .unwrap();
    assert_eq!(saved.zellij_session, "today");
    let token = service.register_provider_for_tests("sample");
    service
        .runtime
        .block_on(service.ingest_events(
            token,
            Batch {
                events: vec![crate::environments::events::tests::event("one")],
            },
        ))
        .unwrap();
    std::fs::write(
        service.environments.config.home.join("rule-sessions"),
        "tomorrow\n",
    )
    .unwrap();
    service.rule_cycle().unwrap();
    let snapshot = service.rules.store.snapshot().unwrap();
    let acceptance = &snapshot.acceptances[0];
    assert_eq!(acceptance.status, DispatchStatus::Failed);
    assert!(acceptance.error.as_ref().unwrap().contains("reactivate"));
    assert!(acceptance.operation_id.is_none());
    assert!(acceptance.launch_started_at.is_none());
    assert!(
        !service
            .environments
            .config
            .workspaces
            .join(&acceptance.instance)
            .exists()
    );
    assert!(service.operations().is_empty());
    let error = service
        .save_rule(saved.definition, Some(saved.revision), None, true)
        .blocking_recv()
        .unwrap()
        .unwrap_err();
    assert!(error.contains("reactivate"), "{error}");
}

#[test]
fn developer_setup_preserves_custom_rules_and_previews_independent_scratch_tasks() {
    let service = AppService::for_tests();
    let template = service.environments.config.templates.join("guidance-only");
    std::fs::create_dir_all(&template).unwrap();
    std::fs::write(template.join("tandem.json"), "{}").unwrap();
    let created = service
        .setup_developer_rules("openai/test#fast", false)
        .unwrap();
    assert_eq!(created.len(), 6);
    assert!(created.iter().all(|rule| !rule.definition.enabled));
    assert!(service.operations().is_empty());
    assert_ne!(
        created[0].definition.initial_prompt,
        created[1].definition.initial_prompt
    );
    let mut custom = created[0].definition.clone();
    custom.model = "custom/model".into();
    custom.start_instance = false;
    custom.description = "Sample Slack messages".into();
    custom.script = custom.script.replace(
        "stream_sequence % 3 == 0",
        "stream_sequence % 3 == 0\n        && sample(event.event_id, 0.8)",
    );
    let saved = service
        .save_rule(custom, Some(1), Some("chosen".into()), false)
        .blocking_recv()
        .unwrap()
        .unwrap();
    let repeated = service.setup_developer_rules("other/model", false).unwrap();
    assert_eq!(repeated[0], saved);
    assert_eq!(repeated[1..], created[1..]);
    let refreshed = service.setup_developer_rules("other/model", true).unwrap();
    let mut expected = saved.clone();
    expected.definition.script = created[0].definition.script.clone();
    expected.definition.description = created[0].definition.description.clone();
    expected.revision += 1;
    assert_eq!(refreshed[0], expected);
    assert_eq!(refreshed[1..], created[1..]);
    assert_eq!(
        service.setup_developer_rules("other/model", true).unwrap(),
        refreshed
    );
    let mut event = crate::environments::events::tests::event("fixture");
    event.attachments.push(crate::store::events::Attachment {
        name: "example.txt".into(),
        url: "https://example.invalid/example.txt".into(),
        media_type: Some("text/plain".into()),
    });
    event.metadata.insert("fixture".into(), true.into());
    let message = serde_json::json!({
        "author": "Maya Patel", "channel": "#pull-requests", "thread": "gateway-timeout",
        "text": "Please review gateway PR #87."
    });
    for (rule, (name, provider, stream, profile, data, file)) in created.iter().zip([
        ("slack-pr-request", "slack", "messages", "message", message.clone(), "pr-review-brief.md"),
        ("slack-support-query", "slack", "messages", "message", message.clone(), "support-checklist.md"),
        ("slack-message-reaction", "slack", "reactions", "message", message, "reaction-summary.md"),
        ("jira-ticket-triage", "jira", "backlog", "ticket", serde_json::json!({
            "key": "PLAT-142", "title": "Fix gateway timeouts", "status": "To Do", "assignee": "Owen Brooks"
        }), "ticket-triage.md"),
        ("datadog-gateway-issue", "datadog", "production-gateway-issue", "system_event", serde_json::json!({
            "resource": "production/gateway", "signal": "monitor.alert", "severity": "critical",
            "description": "Gateway latency exceeds threshold."
        }), "gateway-incident-brief.md"),
        ("github-release-notes", "github", "releases", "generic", serde_json::json!({
            "repository": "northstar/gateway", "tag": "v2.8.1", "release_notes": "Bounded upstream retries."
        }), "release-summary.md"),
    ]) {
        assert_eq!(rule.definition.name, name);
        let mut input = rules::event_input(&event, provider, 1, "now");
        input["stream"] = stream.into();
        input["profile"] = profile.into();
        input["data"] = data;
        for sequence in 1..=9 {
            input["metadata"]["stream_sequence"] = sequence.into();
            assert_eq!(rules::matches(&rule.definition, &input).unwrap(), sequence % 3 == 0, "{name}: {input}");
        }
        let prompt = rules::render_prompt(&rule.definition.initial_prompt, &input).unwrap();
        assert!(prompt.contains(&event.event_id));
        assert!(prompt.contains(&format!("Write {file}")));
        assert!(prompt.contains(&format!("Stream: {stream}")));
        assert!(prompt.contains("- example.txt (https://example.invalid/example.txt)"), "{prompt}");
        assert!(prompt.contains(&format!("Event JSON:\n{}", input)), "{prompt}");
        if profile == "message" {
            assert!(
                prompt.contains("Author: Maya Patel\nChannel: #pull-requests\nThread: gateway-timeout"),
                "{prompt}"
            );
            let mut without_optional = input.clone();
            without_optional["data"]
                .as_object_mut()
                .unwrap()
                .remove("thread");
            without_optional["attachments"] = serde_json::json!([]);
            let prompt =
                rules::render_prompt(&rule.definition.initial_prompt, &without_optional).unwrap();
            assert!(!prompt.contains("Thread:"), "{prompt}");
            assert!(
                prompt.contains("Attachments (do not fetch):\nNone."),
                "{prompt}"
            );
        }
        for (field, value) in [("provider", "other"), ("stream", "other")] {
            let mut skipped = input.clone();
            skipped[field] = value.into();
            assert!(!rules::matches(&rule.definition, &skipped).unwrap());
        }
        input["metadata"]["fixture"] = false.into();
        assert!(!rules::matches(&rule.definition, &input).unwrap());
    }
}
