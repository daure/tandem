use super::*;

fn dynamic_rule(service: &AppService) -> Definition {
    let mut rule = service
        .rules
        .store
        .snapshot()
        .unwrap()
        .rules
        .remove(0)
        .definition;
    rule.template = "{{event.metadata.template}}".into();
    rule.target_instance = Some("{{event.metadata.target}}".into());
    rule.model = "{{event.metadata.model}}".into();
    rule.variant = Some("{{#if event.metadata.variant}}{{event.metadata.variant}}{{/if}}".into());
    rule.initial_prompt = "Start {{event.event_id}}: {{event.data.text}}".into();
    rule.follow_up_prompt = Some("Update {{event.event_id}}: {{event.data.text}}".into());
    rule
}

fn input(id: &str, model: &str, variant: &str) -> crate::store::events::Event {
    let mut event = crate::environments::events::tests::event(id);
    event.metadata =
        json!({"template":"blank", "target":"shared", "model":model, "variant":variant})
            .as_object()
            .unwrap()
            .clone();
    event
}

#[test]
fn event_settings_and_follow_up_prompts_are_pinned_and_delivered_through_named_session_dispatch() {
    for existing in [true, false] {
        let service = fixture_with_session(existing, Some("triage"));
        let server = Server::start(&service);
        let events =
            crate::environments::events::EventStore::open(&service.config_for_tests()).unwrap();
        events.delete(crate::store::events::Deletion::All).unwrap();
        let rule = dynamic_rule(&service);
        let revision = service.rules.store.snapshot().unwrap().rules[0].revision;
        service
            .rules
            .store
            .save(rule.clone(), Some(revision), "main".into())
            .unwrap();
        let token = events.register_provider("sample").unwrap();
        let receipt = events
            .ingest(
                &token,
                Batch {
                    events: vec![
                        input("first", "openai/test", "high"),
                        input("second", "openai/small", ""),
                    ],
                },
            )
            .unwrap();
        let preview = service
            .runtime
            .block_on(service.preview_rule(rule.clone(), receipt.receipts[0].sequence))
            .unwrap();
        assert_eq!(preview.template, "blank");
        assert_eq!(preview.target_instance.as_deref(), Some("shared"));
        assert_eq!(preview.model, "openai/test");
        assert_eq!(preview.variant.as_deref(), Some("high"));
        assert_eq!(
            preview.resolved_follow_up_prompt.as_deref(),
            Some("Update first: Please inspect this event")
        );
        let mut changed = rule.clone();
        changed.initial_prompt = "Changed initial prompt".into();
        changed.follow_up_prompt = Some("Changed follow-up prompt".into());
        changed.template = "other".into();
        changed.model = "openai/changed".into();
        let revision = service.rules.store.snapshot().unwrap().rules[0].revision;
        service
            .rules
            .store
            .save(changed, Some(revision), "main".into())
            .unwrap();
        service.rule_cycle().unwrap();
        if !existing {
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
        }
        let mut rows = service.rules.store.snapshot().unwrap().acceptances;
        rows.sort_by_key(|row| row.id);
        assert_eq!(rows.len(), 2);
        assert!(
            rows.iter()
                .all(|row| row.status == DispatchStatus::Launched),
            "{rows:?}"
        );
        assert_eq!(rows[0].rule.definition, rule);
        assert_eq!(rows[0].template(), "blank");
        assert_eq!(rows[0].session_id, rows[1].session_id);
        assert_eq!(rows[1].resolved_settings.as_ref().unwrap().variant, None);
        {
            let data = server.data.lock().unwrap();
            assert_eq!(data.calls.len(), 2);
            assert_eq!(data.calls[0].1["model"], "openai/test#high");
            assert_eq!(data.calls[1].1["model"], "openai/small");
            assert_eq!(
                data.calls[0].1["initial_prompt"],
                "Start first: Please inspect this event"
            );
            assert_eq!(
                data.calls[1].1["initial_prompt"],
                "Update second: Please inspect this event"
            );
        }
        let template = service
            .environments
            .session_instance("shared")
            .unwrap()
            .template;
        assert_eq!(template, if existing { "other" } else { "blank" });
        let restarted = restarted(&service);
        assert_eq!(
            restarted.rules.store.acceptance(rows[0].id).unwrap(),
            rows[0]
        );
        let revision = service.rules.store.snapshot().unwrap().rules[0].revision;
        service
            .rules
            .store
            .save(rule, Some(revision), "main".into())
            .unwrap();
        server.data.lock().unwrap().sessions.clear();
        events
            .ingest(
                &token,
                Batch {
                    events: vec![input("third", "openai/test", "low")],
                },
            )
            .unwrap();
        restarted.rule_cycle().unwrap();
        let data = server.data.lock().unwrap();
        assert_eq!(data.calls.len(), 3);
        assert_eq!(data.calls[2].0, "/sessions");
        assert_eq!(
            data.calls[2].1["initial_prompt"],
            "Start third: Please inspect this event"
        );
    }
}

#[test]
fn dynamic_sources_save_without_a_catalog_lookup_and_bad_event_values_report_field_errors() {
    let service = fixture_with_session(true, Some("triage"));
    let events =
        crate::environments::events::EventStore::open(&service.config_for_tests()).unwrap();
    events.delete(crate::store::events::Deletion::All).unwrap();
    let mut rule = dynamic_rule(&service);
    rule.enabled = false;
    let revision = service.rules.store.snapshot().unwrap().rules[0].revision;
    let saved = service
        .save_rule(rule.clone(), Some(revision), Some("main".into()), false)
        .blocking_recv()
        .unwrap()
        .unwrap();
    assert_eq!(saved.definition, rule);
    rule.enabled = true;
    service
        .rules
        .store
        .save(rule.clone(), Some(saved.revision), "main".into())
        .unwrap();
    let token = events.register_provider("sample").unwrap();
    let mut invalid_target = input("bad-target", "openai/test", "high");
    invalid_target.metadata["target"] = json!("../escape");
    let mut missing_template = input("missing-template", "openai/test", "high");
    missing_template.metadata.remove("template");
    let receipt = events
        .ingest(
            &token,
            Batch {
                events: vec![
                    invalid_target,
                    missing_template,
                    input("bad-model", "not-a-selector", "high"),
                    input("bad-variant", "openai/test", "not a variant"),
                    input("conflicting-variant", "openai/test#low", "high"),
                ],
            },
        )
        .unwrap();
    let fields = [
        "target instance",
        "handler template",
        "session model",
        "thinking variant",
        "conflicts",
    ];
    for (receipt, field) in receipt.receipts.iter().zip(fields) {
        let error = service
            .runtime
            .block_on(service.preview_rule(rule.clone(), receipt.sequence))
            .unwrap_err();
        assert!(error.contains(field), "{error}");
    }
    service.rules.store.evaluate().unwrap();
    let snapshot = service.rules.store.snapshot().unwrap();
    assert!(snapshot.acceptances.is_empty());
    assert_eq!(snapshot.evaluation_errors.len(), fields.len());
    for field in fields {
        assert!(
            snapshot
                .evaluation_errors
                .iter()
                .any(|row| row.error.contains(field)),
            "{field}"
        );
    }
    assert_eq!(launches(&service), 0);
}

#[test]
fn follow_up_render_failure_retains_the_acceptance_without_delivery_or_retry() {
    let service = fixture_with_session(true, Some("triage"));
    let server = Server::start(&service);
    let events =
        crate::environments::events::EventStore::open(&service.config_for_tests()).unwrap();
    events.delete(crate::store::events::Deletion::All).unwrap();
    let mut rule = dynamic_rule(&service);
    rule.follow_up_prompt = Some("Update {{event.metadata.missing}}".into());
    let revision = service.rules.store.snapshot().unwrap().rules[0].revision;
    service
        .rules
        .store
        .save(rule.clone(), Some(revision), "main".into())
        .unwrap();
    let token = events.register_provider("sample").unwrap();
    let receipt = events
        .ingest(
            &token,
            Batch {
                events: vec![input("missing", "openai/test", "high")],
            },
        )
        .unwrap();
    assert!(
        service
            .runtime
            .block_on(service.preview_rule(rule, receipt.receipts[0].sequence))
            .is_err()
    );
    service.rule_cycle().unwrap();
    let acceptance = service
        .rules
        .store
        .snapshot()
        .unwrap()
        .acceptances
        .remove(0);
    assert_eq!(acceptance.status, DispatchStatus::Failed);
    assert!(
        acceptance
            .error
            .as_deref()
            .unwrap()
            .contains("follow-up prompt: missing prompt field")
    );
    assert_eq!(
        acceptance.resolved_prompt.as_deref(),
        Some("Start missing: Please inspect this event")
    );
    assert!(acceptance.resolved_follow_up_prompt.is_none());
    assert!(
        service
            .runtime
            .block_on(service.retry_acceptance(acceptance.id, true))
            .is_err()
    );
    service.rule_cycle().unwrap();
    assert!(server.data.lock().unwrap().calls.is_empty());
}

#[test]
fn fresh_conversations_use_the_resolved_model_and_initial_prompt() {
    for targeted in [false, true] {
        let service = fixture_with_session(targeted, None);
        let events =
            crate::environments::events::EventStore::open(&service.config_for_tests()).unwrap();
        events.delete(crate::store::events::Deletion::All).unwrap();
        let mut rule = dynamic_rule(&service);
        if !targeted {
            rule.target_instance = None;
        }
        let revision = service.rules.store.snapshot().unwrap().rules[0].revision;
        service
            .rules
            .store
            .save(rule, Some(revision), "main".into())
            .unwrap();
        let token = events.register_provider("sample").unwrap();
        events
            .ingest(
                &token,
                Batch {
                    events: vec![input("fresh", "openai/selected", "high")],
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
        if !targeted {
            let operation = acceptance.operation_id.as_deref().unwrap();
            assert_eq!(
                service
                    .runtime
                    .block_on(service.wait_operation(operation))
                    .unwrap()
                    .state,
                OperationState::Succeeded
            );
            service.rule_cycle().unwrap();
            acceptance = service.rules.store.acceptance(acceptance.id).unwrap();
        }
        assert_eq!(
            acceptance.status,
            DispatchStatus::Launching,
            "{acceptance:?}"
        );
        let instance = service
            .environments
            .session_instance(&acceptance.instance)
            .unwrap();
        assert_eq!(instance.template, if targeted { "other" } else { "blank" });
        assert!(
            crate::environments::rules::acceptance_workspace_matches(
                &service.config_for_tests(),
                &instance,
                &acceptance
            )
            .unwrap()
        );
        let calls =
            fs::read_to_string(service.config_for_tests().home.join("target-calls")).unwrap();
        assert!(
            calls.contains("TANDEM_SESSION_MODEL=openai/selected#high"),
            "{calls}"
        );
        assert!(
            calls.contains("--model openai/selected --variant high"),
            "{calls}"
        );
        assert!(
            calls.contains("Start fresh: Please inspect this event"),
            "{calls}"
        );
        assert!(!calls.contains("Update fresh:"), "{calls}");
        assert_eq!(launches(&service), 1);
    }
}
