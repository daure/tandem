use rmcp::model::ClientJsonRpcMessage;
use serde_json::json;

use crate::service::AppService;

use super::{McpServer, StdioHandshakeBuffer};

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
        let error = server
            .create_instance(Parameters(super::CreateInstanceInput {
                template: "website".into(),
                name: "review".into(),
                confirmed: false,
                wait: true,
                timeout_seconds: 60,
            }))
            .await
            .err()
            .expect("unconfirmed creation must fail");
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
