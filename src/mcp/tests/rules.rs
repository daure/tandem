use super::*;
use rmcp::handler::server::wrapper::Parameters;

#[test]
fn rule_tools_expose_instance_identity_hooks_in_descriptions_and_script_schemas() {
    let server = McpServer::new(AppService::for_tests());
    for name in ["save_rule", "preview_rule"] {
        let tool = &server.tool_router.map[name].attr;
        let description = tool.description.as_deref().unwrap();
        let schema = serde_json::to_string(&tool.input_schema).unwrap();
        for hook in ["instance_name(event)", "instance_description(event)"] {
            assert!(
                description.contains(hook),
                "{name} description omits {hook}"
            );
            assert!(schema.contains(hook), "{name} input schema omits {hook}");
        }
    }
}

#[test]
fn rule_tools_round_trip_definitions_preview_retained_events_and_enforce_authorization() {
    let service = AppService::for_tests();
    let server = McpServer::new(service.clone());
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        server
            .create_template(Parameters(super::super::NameInput {
                name: "blank".into(),
            }))
            .await
            .unwrap();
        let token = service.register_provider_for_tests("sample");
        let receipt = service
            .ingest_events(
                token,
                crate::store::events::Batch {
                    events: vec![crate::environments::events::tests::event("one")],
                },
            )
            .await
            .unwrap();
        let sequence = receipt.receipts[0].sequence;
        let definition: crate::store::rules::Definition = serde_json::from_value(json!({
            "name": "inspect", "script": "fn matches(event) { event.profile == \"message\" } fn instance_name(event) { \" PR 82! \" } fn instance_description(event) { \"slack://channel?team=T1&id=C2\" }",
            "template": "blank", "model": "openai/test#fast", "enabled": false,
            "initial_prompt": "Inspect {{event.data.text}}"
        }))
        .unwrap();
        let saved = server
            .save_rule(Parameters(super::super::SaveRuleInput {
                definition: definition.clone(),
                expected_revision: None,
                zellij_session: Some("main".into()),
                confirmed: false,
            }))
            .await
            .unwrap()
            .0;
        let fetched = server
            .get_rule(Parameters(super::super::NameInput {
                name: "inspect".into(),
            }))
            .await
            .unwrap()
            .0;
        assert_eq!(fetched["rule"]["revision"], 1);
        assert_eq!(
            fetched["rule"]["definition"],
            serde_json::to_value(&definition).unwrap()
        );
        assert_eq!(fetched["acceptances"], json!([]));
        let preview = server
            .preview_rule(Parameters(super::super::PreviewRuleInput {
                definition: definition.clone(),
                event_sequence: sequence,
            }))
            .await
            .unwrap()
            .0;
        assert_eq!(preview["matched"], true);
        assert_eq!(preview["instance_name"], "pr-82");
        assert_eq!(
            preview["instance_description"],
            "slack://channel?team=T1&id=C2"
        );
        assert_eq!(preview["name_is_advisory"], true);
        assert_eq!(preview["model"], "openai/test#fast");
        let mut targeted = definition.clone();
        targeted.template = "{{#if event.metadata.source}}blank{{/if}}".into();
        targeted.target_instance = Some("shared-{{event.metadata.source}}".into());
        targeted.model = "openai/{{event.metadata.source}}".into();
        targeted.variant = Some("{{#if event.metadata.source}}high{{/if}}".into());
        targeted.follow_up_prompt = Some("Continue with {{event.data.text}}".into());
        targeted.session_name = Some("triage".into());
        targeted.delivery_mode = crate::store::rules::DeliveryMode::Steer;
        let targeted = server
            .save_rule(Parameters(super::super::SaveRuleInput {
                definition: targeted,
                expected_revision: Some(saved.revision),
                zellij_session: Some("main".into()),
                confirmed: false,
            }))
            .await
            .unwrap()
            .0;
        assert_eq!(
            targeted.definition.target_instance.as_deref(),
            Some("shared-{{event.metadata.source}}")
        );
        let target_preview = server
            .preview_rule(Parameters(super::super::PreviewRuleInput {
                definition: targeted.definition,
                event_sequence: sequence,
            }))
            .await
            .unwrap()
            .0;
        assert_eq!(target_preview["template"], "blank");
        assert_eq!(target_preview["target_instance"], "shared-fixture");
        assert_eq!(target_preview["model"], "openai/fixture");
        assert_eq!(target_preview["variant"], "high");
        assert_eq!(target_preview["resolved_follow_up_prompt"], "Continue with Please inspect this event");
        assert_eq!(target_preview["session_name"], "triage");
        assert_eq!(target_preview["delivery_mode"], "steer");
        assert_eq!(target_preview["instance_name"], "shared-fixture");
        assert_eq!(target_preview["name_is_advisory"], false);
        assert_eq!(
            preview["resolved_prompt"],
            "Inspect Please inspect this event"
        );
        let event = server
            .get_event(Parameters(super::super::EventInput { sequence }))
            .await
            .unwrap()
            .0;
        assert_eq!(event["sequence"], sequence);
        assert_eq!(event["event"]["metadata"]["source"], "fixture");
        assert_eq!(event["acceptances"], json!([]));
        let before = server
            .get_event(Parameters(super::super::EventInput { sequence }))
            .await
            .unwrap()
            .0;
        let diagnostics = server
            .get_event_diagnostics(Parameters(super::super::EventInput { sequence }))
            .await
            .unwrap()
            .0;
        assert_eq!(diagnostics["sequence"], sequence);
        assert_eq!(diagnostics["attempts_total"], 1);
        assert_eq!(diagnostics["attempts_truncated"], false);
        assert_eq!(diagnostics["attempts"][0]["rules"], json!([]));
        assert_eq!(
            server
                .get_event(Parameters(super::super::EventInput { sequence }))
                .await
                .unwrap()
                .0,
            before
        );
        assert!(
            server
                .get_event_diagnostics(Parameters(super::super::EventInput {
                    sequence: sequence + 1
                }))
                .await
                .is_err()
        );
        let mut enabled = definition;
        enabled.enabled = true;
        let error = server
            .save_rule(Parameters(super::super::SaveRuleInput {
                definition: enabled,
                expected_revision: Some(saved.revision),
                zellij_session: Some("main".into()),
                confirmed: false,
            }))
            .await
            .err()
            .expect("enabled revisions require approval");
        assert!(error.contains("confirmation_required"));
        let error = server
            .replay_event(Parameters(super::super::ReplayEventInput {
                sequence,
                request_id: "replay".into(),
                confirmed: false,
            }))
            .await
            .err()
            .expect("replay requires approval");
        assert!(error.contains("confirmation_required"));
        assert!(service.operations().is_empty());
        assert_eq!(
            server.list_rules().await.unwrap().0["rules"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
    });
}
