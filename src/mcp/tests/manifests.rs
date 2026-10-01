use super::*;
use crate::{
    mcp::{NameInput, UpdateTemplateManifestInput},
    store::environments::Manifest,
};
use rmcp::ServerHandler;
use rmcp::handler::server::wrapper::Parameters;

#[test]
fn initialize_requires_both_guidance_fields_and_user_resolution_of_conflicts() {
    let server = McpServer::new(AppService::for_tests());
    let instructions = server.get_info().instructions.unwrap();
    assert!(instructions.contains("Call get_instructions before any other MCP calls."));
    assert!(instructions.contains("Read both core_guidance and markdown"));
    assert!(instructions.contains("ask the user before acting if they conflict"));
}

#[test]
fn instruction_payload_includes_current_schema_and_preserves_editable_guidance() {
    let service = AppService::for_tests();
    let server = McpServer::new(service);
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let instructions = server.get_instructions().await.unwrap().0;
        let baseline = include_str!("../../../agent-core-guidance.md");
        assert_eq!(instructions.core_guidance, baseline);
        assert!(baseline.contains(".local/cache"));
        assert!(baseline.contains("secrets in private external files"));
        assert_eq!(
            instructions.manifest_schema,
            serde_json::to_value(schemars::schema_for!(Manifest)).unwrap()
        );
        assert_eq!(instructions.manifest_schema["additionalProperties"], false);
        assert_eq!(
            instructions.manifest_schema["$defs"]["Repository"]["additionalProperties"],
            false
        );
        assert_eq!(
            instructions.manifest_schema["$defs"]["Route"]["properties"]["port"]["minimum"],
            1
        );
        assert_eq!(
            instructions.manifest_schema["$defs"]["Route"]["properties"]["strip_prefix"]["default"],
            true
        );
        for markdown in [
            "# Using Tandem\n\nPreserve Tandem-generated `.tandem-*` files for ownership checks and cleanup.\n",
            "# Custom guidance\r\n\r\nUse team conventions.  \r\n",
        ] {
            std::fs::write(&instructions.file, markdown).unwrap();
            let second = server.get_instructions().await.unwrap().0;
            assert_eq!(second.markdown, markdown);
            assert_eq!(second.core_guidance, baseline);
            assert_eq!(second.file, instructions.file);
            assert_eq!(second.manifest_schema, instructions.manifest_schema);
            assert_eq!(
                serde_json::to_value(&second).unwrap()["core_guidance"],
                baseline
            );
            assert_eq!(
                std::fs::read(&instructions.file).unwrap(),
                markdown.as_bytes()
            );
        }
    });
}

#[test]
fn manifest_update_tool_requires_approval_and_preserves_the_file_on_validation_failure() {
    let service = AppService::for_tests();
    let server = McpServer::new(service);
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        assert!(
            server
                .tool_router
                .map
                .contains_key("update_template_manifest")
        );
        let created = server
            .create_template(Parameters(NameInput {
                name: "website".into(),
            }))
            .await
            .unwrap()
            .0;
        let before = std::fs::read(&created.manifest_file).unwrap();
        for (confirmed, manifest) in [
            (false, json!({"description":"Unconfirmed"})),
            (
                true,
                json!({"repositories":[{"source":"/source","target":"../outside"}]}),
            ),
        ] {
            let input = serde_json::from_value(
                json!({"name":"website", "manifest":manifest, "confirmed":confirmed}),
            )
            .unwrap();
            assert!(
                server
                    .update_template_manifest(Parameters(input))
                    .await
                    .is_err()
            );
            assert_eq!(std::fs::read(&created.manifest_file).unwrap(), before);
        }
        for manifest in [
            json!({"typo":true}),
            json!({"repositories":[{"source":"/source","target":"app","typo":true}]}),
            json!({"routes":{"web":{"port":"80"}}}),
        ] {
            assert!(
                serde_json::from_value::<UpdateTemplateManifestInput>(
                    json!({"name":"website", "manifest":manifest,"confirmed":true})
                )
                .is_err()
            );
        }
        let updated = server
            .update_template_manifest(Parameters(UpdateTemplateManifestInput {
                name: "website".into(),
                manifest: serde_json::from_value(json!({"description":"Updated"})).unwrap(),
                confirmed: true,
            }))
            .await
            .unwrap()
            .0;
        assert_eq!(updated.manifest.description, "Updated");
        assert_eq!(
            server
                .get_template(Parameters(NameInput {
                    name: "website".into()
                }))
                .await
                .unwrap()
                .0,
            updated
        );
    });
}
