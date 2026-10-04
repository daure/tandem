use std::sync::Arc;

use rmcp::{
    Json, ServerHandler, ServiceExt,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{ServerCapabilities, ServerInfo},
    tool, tool_handler, tool_router,
};
use schemars::JsonSchema;
use serde::Deserialize;

use crate::{environments::InstanceScope, service::AppService, store::environments::Operation};

#[derive(Clone)]
pub(super) struct InstanceMcpServer {
    service: AppService,
    scope: Arc<InstanceScope>,
    tool_router: ToolRouter<Self>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SelfInput {}

impl InstanceMcpServer {
    fn new(service: AppService, scope: Arc<InstanceScope>) -> Self {
        let mut tool_router = Self::tool_router();
        for route in tool_router.map.values_mut() {
            route.attr.output_schema = None;
        }
        Self {
            service,
            scope,
            tool_router,
        }
    }
}

#[tool_router]
impl InstanceMcpServer {
    #[tool(
        description = "Start this workspace's instance by applying its trusted template, building services and waiting for readiness. Setup jobs may rerun. Preserves workspace edits and data. Use when the assigned work needs services. Returns only after completion; workspace-only instances prepare without Docker."
    )]
    async fn start_self(
        &self,
        Parameters(_): Parameters<SelfInput>,
    ) -> Result<Json<Operation>, String> {
        self.service
            .instance_self_action(Arc::clone(&self.scope), true)
            .await
            .map(Json)
    }

    #[tool(
        description = "Stop this workspace's instance containers to release service resources when they are no longer needed. Preserves workspace, containers, volumes, networks and conversation; leaves OpenCode and shared services running. Returns only after verified completion. Workspace-only instances are preserved without work; stopping does not certify task success."
    )]
    async fn stop_self(
        &self,
        Parameters(_): Parameters<SelfInput>,
    ) -> Result<Json<Operation>, String> {
        self.service
            .instance_self_action(Arc::clone(&self.scope), false)
            .await
            .map(Json)
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for InstanceMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some("This connection controls only the Tandem instance owning its startup workspace. Use start_self when assigned work needs services and stop_self when services are no longer needed. Both actions preserve data and leave the conversation open. Coordinate with other sessions sharing the workspace. A successful stop is not task completion. These tools accept no target name and provide no deletion capability.".into()),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            ..Default::default()
        }
    }
}

pub(crate) async fn run_stdio(
    service: AppService,
    scope: Arc<InstanceScope>,
) -> Result<(), Box<dyn std::error::Error>> {
    InstanceMcpServer::new(service, scope)
        .serve(super::TolerantStdioTransport::new())
        .await?
        .waiting()
        .await?;
    Ok(())
}
