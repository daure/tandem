use rmcp::handler::server::wrapper::Parameters;

use super::*;
use crate::mcp::SetCloseCommandInput;

#[test]
fn close_settings_require_approval_and_remain_separate_from_open_settings() {
    let service = AppService::for_tests();
    let server = McpServer::new(service.clone());
    let runtime = tokio::runtime::Runtime::new().unwrap();
    assert!(server.tool_router.map.contains_key("get_close_command"));
    assert!(server.tool_router.map.contains_key("set_close_command"));
    runtime.block_on(async {
        assert_eq!(server.get_close_command().await.unwrap().0.command, "");
        let unconfirmed = serde_json::from_value(json!({"command": "exit 23"})).unwrap();
        assert!(
            server
                .set_close_command(Parameters(unconfirmed))
                .await
                .err()
                .unwrap()
                .contains("confirmation_required")
        );
        assert!(
            service
                .configure_close_command("bad\0command".into(), true)
                .await
                .unwrap_err()
                .contains("NUL")
        );
        service
            .configure_open_command("editor .".into(), true)
            .await
            .unwrap();
        let saved = server
            .set_close_command(Parameters(SetCloseCommandInput {
                command: "exit 23".into(),
                confirmed: true,
            }))
            .await
            .unwrap()
            .0;
        assert_eq!(
            serde_json::to_value(saved).unwrap(),
            json!({"command": "exit 23"})
        );
        assert_eq!(service.close_command(), "exit 23");
        assert_eq!(service.open_command(), "editor .");
        assert_eq!(
            server.get_close_command().await.unwrap().0.command,
            "exit 23"
        );
        service
            .configure_close_command("".into(), true)
            .await
            .unwrap();
        assert_eq!(server.get_close_command().await.unwrap().0.command, "");
    });
}
