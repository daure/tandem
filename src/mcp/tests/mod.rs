use rmcp::model::ClientJsonRpcMessage;
use serde_json::json;

use crate::service::AppService;

use super::{McpServer, StdioHandshakeBuffer};

mod manifests;
mod providers;
mod rules;

#[test]
fn session_listing_mcp_validates_filters_and_requires_enabled_observation() {
    use rmcp::handler::server::wrapper::Parameters;
    let service = AppService::for_tests();
    let server = McpServer::new(service.clone());
    let input: super::ListSessionsInput = serde_json::from_value(json!({})).unwrap();
    assert!(input.instance.is_none());
    assert!(!input.include_closed);
    let history: super::ListSessionsInput =
        serde_json::from_value(json!({"include_closed":true})).unwrap();
    assert!(history.include_closed);
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let error = server
            .list_sessions(Parameters(super::ListSessionsInput {
                instance: Some("../outside".into()),
                include_closed: false,
            }))
            .await
            .err()
            .unwrap();
        assert!(error.contains("name must be"), "{error}");
        service
            .set_opencode_enabled(false)
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        let error = server.list_sessions(Parameters(input)).await.err().unwrap();
        assert!(error.contains("integration is disabled"), "{error}");
    });
    assert!(server.tool_router.map.contains_key("list_sessions"));
}

#[test]
fn prompted_sessions_require_task_approval_and_valid_literal_input() {
    use rmcp::handler::server::wrapper::Parameters;
    let server = McpServer::new(AppService::for_tests());
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        for (confirmed, prompt, expected) in [
            (false, "Inspect this", "confirmation_required"),
            (true, " \n\t", "prompt must contain"),
        ] {
            let error = server
                .new_instance_session(Parameters(super::NewInstanceSessionInput {
                    name: "review".into(),
                    initial_prompt: prompt.into(),
                    model: None,
                    variant: None,
                    agent: None,
                    confirmed,
                }))
                .await
                .err()
                .unwrap();
            assert!(error.contains(expected), "{error}");
            let input: super::PromptSessionInput = serde_json::from_value(json!({
                "session_id":"ses_existing", "prompt":prompt, "confirmed":confirmed,
            }))
            .unwrap();
            assert!(matches!(
                input.when_busy,
                crate::store::opencode::WhenBusy::Queue
            ));
            let error = server
                .prompt_session(Parameters(input))
                .await
                .err()
                .unwrap();
            assert!(error.contains(expected), "{error}");
        }
        assert!(server.service.operations().is_empty());
    });
}

#[test]
fn stdio_handshake_defers_early_requests_until_initialized() {
    let initialize: ClientJsonRpcMessage = serde_json::from_value(json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {}
    }))
    .unwrap();
    let early_request: ClientJsonRpcMessage = serde_json::from_value(json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {}
    }))
    .unwrap();
    let initialized: ClientJsonRpcMessage = serde_json::from_value(json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized",
        "params": {}
    }))
    .unwrap();
    let mut buffer = StdioHandshakeBuffer::default();

    assert!(buffer.receive(initialize).is_some());
    assert!(buffer.receive(early_request).is_none());
    assert!(buffer.receive(initialized).is_some());
    assert!(matches!(
        buffer.take_deferred(),
        Some(ClientJsonRpcMessage::Request(_))
    ));
}

#[test]
fn tools_omit_output_schemas_for_opencode_compatibility() {
    let server = McpServer::new(AppService::for_tests());

    assert!(
        server
            .tool_router
            .map
            .values()
            .all(|route| route.attr.output_schema.is_none())
    );
}

#[test]
fn template_tools_return_object_payloads_and_mutations_require_confirmation() {
    use rmcp::handler::server::wrapper::Parameters;
    let service = AppService::for_tests();
    let server = McpServer::new(service);
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let instructions = server.get_instructions().await.unwrap().0;
        assert!(instructions.markdown.contains("Tandem"));
        let created = server
            .create_template(Parameters(super::NameInput {
                name: "website".into(),
            }))
            .await
            .unwrap()
            .0;
        let templates = server.list_templates().await.unwrap().0;
        let json = serde_json::to_value(templates).unwrap();
        assert_eq!(json["templates"][0]["directory"], created.directory);
        assert!(created.workspace_only());
        assert_eq!(created.manifest_source.as_deref(), Some("{}\n"));
        assert!(created.guidance_source.is_none());
        let error = server
            .create_instance(Parameters(super::CreateInstanceInput {
                template: "website".into(),
                name: "review".into(),
                opencode: false,
                initial_prompt: None,
                model: None,
                variant: None,
                confirmed: false,
                wait: true,
                start_instance: true,
                timeout_seconds: 60,
            }))
            .await
            .err()
            .expect("unconfirmed creation must fail");
        assert!(error.contains("confirmation_required"));
        let error = server
            .delete_instance(Parameters(super::DeleteInstanceInput {
                name: "review".into(),
                confirmed: false,
            }))
            .await
            .err()
            .expect("unconfirmed deletion must fail");
        assert!(error.contains("confirmation_required"));
        std::fs::write(instructions.file, "# Edited agent guidance").unwrap();
        assert_eq!(
            server.get_instructions().await.unwrap().0.markdown,
            "# Edited agent guidance"
        );
    });
}

#[test]
fn http_transport_rejects_foreign_hosts_and_origins_before_dispatching_tools() {
    use std::{sync::mpsc, time::Duration};

    let service = AppService::for_tests();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let reservation = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = reservation.local_addr().unwrap();
        drop(reservation);
        let (sender, receiver) = mpsc::channel();
        let server_service = service.clone();
        let server = tokio::spawn(async move {
            super::run_http_with_startup(server_service, address, sender)
                .await
                .unwrap();
        });
        tokio::task::spawn_blocking(move || receiver.recv_timeout(Duration::from_secs(5)))
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        for (host, origin, expected) in [
            (address.to_string(), None, 200),
            (
                format!("localhost:{}", address.port()),
                Some(format!("http://localhost:{}", address.port())),
                200,
            ),
            (format!("attacker.example:{}", address.port()), None, 403),
            (
                address.to_string(),
                Some("https://attacker.example".into()),
                403,
            ),
            (address.to_string(), Some("null".into()), 403),
            (address.to_string(), Some("http://127.0.0.1:1".into()), 403),
        ] {
            let mut request = client
                .post(format!("http://{address}/mcp"))
                .header("Host", host)
                .header("Content-Type", "application/json")
                .header("Accept", "application/json, text/event-stream")
                .body(
                    json!({"jsonrpc":"2.0","id":1,"method":"tools/call",
                    "params":{"name":"get_status","arguments":{}}})
                    .to_string(),
                );
            if let Some(origin) = origin {
                request = request.header("Origin", origin);
            }
            let response = request.send().await.unwrap();
            assert_eq!(response.status().as_u16(), expected);
            if expected == 200 {
                assert!(response.text().await.unwrap().contains("Tandem"));
            }
        }
        server.abort();
        let _ = server.await;
    });
}
