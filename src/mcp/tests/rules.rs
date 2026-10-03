use super::*;
use rmcp::handler::server::wrapper::Parameters;

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
            "name": "inspect", "script": "fn matches(event) { event.profile == \"message\" }",
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
        assert_eq!(preview["model"], "openai/test#fast");
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
