use super::*;
use crate::store::providers::Action;
use rmcp::handler::server::wrapper::Parameters;

#[test]
fn provider_lifecycle_tools_require_approval_before_starting_runtime_resources() {
    let service = AppService::for_tests();
    let server = McpServer::new(service.clone());
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        assert!(
            server
                .list_providers()
                .await
                .unwrap()
                .0
                .providers
                .is_empty()
        );
        for action in [
            Action::Start,
            Action::Stop,
            Action::Pause,
            Action::Resume,
            Action::Restart,
        ] {
            let error = server
                .provider_action(Parameters(super::super::ProviderActionInput {
                    name: "message".into(),
                    action,
                    confirmed: false,
                }))
                .await
                .err()
                .unwrap();
            assert!(error.contains("confirmation_required"));
        }
        assert!(service.provider_snapshot().providers.is_empty());
    });
}
