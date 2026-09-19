use std::{collections::VecDeque, net::SocketAddr};

use rmcp::{
    Json, RoleServer, ServerHandler, ServiceExt,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{ClientJsonRpcMessage, ServerCapabilities, ServerInfo},
    tool, tool_handler, tool_router,
    transport::{Transport, async_rw::AsyncRwTransport},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    service::{AppService, ServiceStatus},
    store::environments::{Instructions, Manifest, Operation, RuntimeInventory, Template},
};

mod http;

#[derive(Clone)]
struct McpServer {
    service: AppService,
    tool_router: ToolRouter<Self>,
}

impl McpServer {
    fn new(service: AppService) -> Self {
        let mut tool_router = Self::tool_router();
        for route in tool_router.map.values_mut() {
            route.attr.output_schema = None;
        }
        Self {
            service,
            tool_router,
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
struct NameInput {
    /// 1–40 lowercase letters, digits or hyphens; start with a letter/digit; gateway is reserved.
    name: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct UpdateTemplateManifestInput {
    /// Existing template name from list_templates.
    name: String,
    /// Complete replacement for tandem.json, using the manifest_schema returned by get_instructions. Omitted optional fields use defaults.
    manifest: Manifest,
    /// Approval to replace shared template configuration; this does not start containers or provision repositories.
    #[serde(default)]
    confirmed: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct SetOpenCommandInput {
    /// Trusted host shell command. Use "$TANDEM_INSTANCE" for the instance name and "$TANDEM_WORKSPACE" for its path; empty restores the folder opener.
    command: String,
    /// Approval to configure host command execution when a user opens an instance.
    #[serde(default)]
    confirmed: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct SetCloseCommandInput {
    /// Trusted host shell command, run before workspace deletion. Quote "$TANDEM_INSTANCE" and "$TANDEM_WORKSPACE"; empty disables it.
    command: String,
    /// Approval for automatic host command execution during instance deletion, including purges and template deletion.
    #[serde(default)]
    confirmed: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct RunOpenCommandInput {
    /// Existing instance name; its workspace must be a real directory beneath Tandem's workspace root.
    name: String,
    /// Approval to execute the saved host command in this instance workspace.
    #[serde(default)]
    confirmed: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
struct OpenCommandSetting {
    command: String,
}

#[derive(Debug, Serialize, JsonSchema)]
struct OpenCommandRun {
    workspace: String,
}

#[derive(Debug, Serialize, JsonSchema)]
struct TemplateList {
    templates: Vec<Template>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct OperationInput {
    /// ID returned by a mutation in this MCP process; operation history is in memory.
    id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct CreateInstanceInput {
    /// Name of an editable template directory from list_templates.
    template: String,
    /// Instance name; 1–40 letters, digits or hyphens; starts with a letter/digit; gateway is reserved.
    name: String,
    /// Set only after approval for host Git repository provisioning and this trusted Compose template's local Docker privileges.
    #[serde(default)]
    confirmed: bool,
    /// Wait for content readiness (default true); false returns an operation to poll.
    #[serde(default = "default_wait")]
    wait: bool,
    /// Startup/readiness budget, 5–900 seconds; defaults to 600.
    #[serde(default = "default_timeout")]
    timeout_seconds: u64,
}

fn default_wait() -> bool {
    true
}
fn default_timeout() -> u64 {
    600
}

#[derive(Debug, Deserialize, JsonSchema)]
struct StopInstanceInput {
    /// Existing instance name; the gateway is not an instance.
    name: String,
    /// Approval to stop containers while keeping the instance data.
    #[serde(default)]
    confirmed: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct RestartInstanceInput {
    /// Existing instance name; the gateway is not an instance.
    name: String,
    /// Approval to interrupt services by restarting their existing containers.
    #[serde(default)]
    confirmed: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct RestartServiceInput {
    /// Existing instance containing the service.
    name: String,
    /// Exact Compose service name from list_instances; one-shot jobs cannot be restarted.
    service: String,
    /// Approval to interrupt this service by restarting its existing containers.
    #[serde(default)]
    confirmed: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ServiceStateInput {
    /// Existing instance containing the service; the shared gateway is excluded.
    name: String,
    /// Exact Compose service name from list_instances; one-shot jobs are excluded.
    service: String,
    /// Approval to start or stop this service's existing containers.
    #[serde(default)]
    confirmed: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct DeleteInstanceInput {
    /// Existing instance name; the gateway is not an instance.
    name: String,
    /// Approval to remove this instance's containers, networks, volumes, workspace, and rendered Compose file.
    #[serde(default)]
    confirmed: bool,
}

#[derive(Default)]
struct StdioHandshakeBuffer {
    phase: StdioHandshakePhase,
    deferred: VecDeque<ClientJsonRpcMessage>,
}

#[derive(Default, PartialEq, Eq)]
enum StdioHandshakePhase {
    #[default]
    AwaitInitialize,
    AwaitInitialized,
    Active,
}

impl StdioHandshakeBuffer {
    fn receive(&mut self, message: ClientJsonRpcMessage) -> Option<ClientJsonRpcMessage> {
        match self.phase {
            StdioHandshakePhase::AwaitInitialize => {
                self.phase = StdioHandshakePhase::AwaitInitialized;
                Some(message)
            }
            StdioHandshakePhase::AwaitInitialized => {
                if matches!(message, ClientJsonRpcMessage::Request(_)) {
                    self.deferred.push_back(message);
                    None
                } else {
                    self.phase = StdioHandshakePhase::Active;
                    Some(message)
                }
            }
            StdioHandshakePhase::Active => Some(message),
        }
    }

    fn take_deferred(&mut self) -> Option<ClientJsonRpcMessage> {
        (self.phase == StdioHandshakePhase::Active)
            .then(|| self.deferred.pop_front())
            .flatten()
    }
}

struct TolerantStdioTransport {
    inner: AsyncRwTransport<RoleServer, tokio::io::Stdin, tokio::io::Stdout>,
    handshake: StdioHandshakeBuffer,
}

impl TolerantStdioTransport {
    fn new() -> Self {
        Self {
            inner: AsyncRwTransport::new_server(tokio::io::stdin(), tokio::io::stdout()),
            handshake: StdioHandshakeBuffer::default(),
        }
    }
}

impl Transport<RoleServer> for TolerantStdioTransport {
    type Error = std::io::Error;

    fn send(
        &mut self,
        message: rmcp::service::TxJsonRpcMessage<RoleServer>,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'static {
        self.inner.send(message)
    }

    async fn receive(&mut self) -> Option<ClientJsonRpcMessage> {
        if let Some(message) = self.handshake.take_deferred() {
            return Some(message);
        }
        loop {
            let message = self.inner.receive().await?;
            if let Some(message) = self.handshake.receive(message) {
                return Some(message);
            }
        }
    }

    async fn close(&mut self) -> Result<(), Self::Error> {
        self.inner.close().await
    }
}

#[tool_router]
impl McpServer {
    #[tool(
        description = "Read the persisted workspace close command. Empty disables the deletion hook."
    )]
    async fn get_close_command(&self) -> Result<Json<OpenCommandSetting>, String> {
        self.service
            .get_close_command()
            .await
            .map(|command| Json(OpenCommandSetting { command }))
    }

    #[tool(
        description = "Save a trusted close command after user approval (confirmed=true). Saving does not execute it. Instance deletion, purges and template deletion run it through sh -c in each existing workspace before removing that folder, with TANDEM_INSTANCE and TANDEM_WORKSPACE set. Empty disables it. Failures or a ten-second timeout produce operation warnings and deletion continues. Stop/restart do not run it."
    )]
    async fn set_close_command(
        &self,
        Parameters(input): Parameters<SetCloseCommandInput>,
    ) -> Result<Json<OpenCommandSetting>, String> {
        let command = self
            .service
            .configure_close_command(input.command, input.confirmed)
            .await?;
        Ok(Json(OpenCommandSetting { command }))
    }

    #[tool(
        description = "Read the persisted workspace open command. Empty means the system folder opener."
    )]
    async fn get_open_command(&self) -> Result<Json<OpenCommandSetting>, String> {
        self.service
            .get_open_command()
            .await
            .map(|command| Json(OpenCommandSetting { command }))
    }

    #[tool(
        description = "Save a trusted workspace open command after user approval (confirmed=true). Saving does not execute it. On instance opening, runs via sh -c in the workspace with TANDEM_INSTANCE and TANDEM_WORKSPACE set; quote the variables. Empty restores the folder opener."
    )]
    async fn set_open_command(
        &self,
        Parameters(input): Parameters<SetOpenCommandInput>,
    ) -> Result<Json<OpenCommandSetting>, String> {
        let command = self
            .service
            .configure_open_command(input.command, input.confirmed)
            .await?;
        Ok(Json(OpenCommandSetting { command }))
    }

    #[tool(
        description = "Run the saved workspace open command for a named instance. Requires confirmed=true after user approval; runs through sh -c in the workspace with TANDEM_INSTANCE and TANDEM_WORKSPACE set. An empty saved command uses the system folder opener."
    )]
    async fn run_open_command(
        &self,
        Parameters(input): Parameters<RunOpenCommandInput>,
    ) -> Result<Json<OpenCommandRun>, String> {
        self.service
            .run_open_command(input.name, input.confirmed)
            .await
            .map(|workspace| Json(OpenCommandRun { workspace }))
    }

    #[tool(
        description = "Read editable agent guidance, absolute instructions/template/workspace paths, and the generated tandem.json manifest_schema. Call this before using Tandem."
    )]
    async fn get_instructions(&self) -> Result<Json<Instructions>, String> {
        self.service.get_instructions().await.map(Json)
    }

    #[tool(
        description = "List development templates with optional Compose paths and manifest metadata. Invalid templates include errors. Does not require Docker."
    )]
    async fn list_templates(&self) -> Result<Json<TemplateList>, String> {
        self.service
            .list_templates()
            .await
            .map(|templates| Json(TemplateList { templates }))
    }

    #[tool(
        description = "Get the editable template directory, Compose file/source when present, optional manifest and tandem-agents.md guidance. Workspace-only templates have empty Compose fields; guidance-only templates need no manifest. Relative scripts and config belong in this directory."
    )]
    async fn get_template(
        &self,
        Parameters(input): Parameters<NameInput>,
    ) -> Result<Json<Template>, String> {
        self.service.get_template(input.name).await.map(Json)
    }

    #[tool(
        description = "Create a dedicated editable template folder with a working static-web Compose starter and routing manifest; refuses existing names. Does not start containers."
    )]
    async fn create_template(
        &self,
        Parameters(input): Parameters<NameInput>,
    ) -> Result<Json<Template>, String> {
        self.service.create_template(input.name).await.map(Json)
    }

    #[tool(
        description = "Validate and atomically replace a template's tandem.json. Requires confirmed=true after approval to change shared configuration. Rejects invalid manifests without changing the file; runs no Docker or Git commands. Compose service references are checked at instance startup."
    )]
    async fn update_template_manifest(
        &self,
        Parameters(input): Parameters<UpdateTemplateManifestInput>,
    ) -> Result<Json<Template>, String> {
        self.service
            .update_template_manifest(input.name, input.manifest, input.confirmed)
            .await
            .map(Json)
    }

    #[tool(
        description = "List container-backed and workspace-only instances with provisioning outcomes, runtime evidence, summaries and retained activities. A runtime_error marks partial workspace-only inventory when Docker is unavailable. Running without a probe is not healthy."
    )]
    async fn list_instances(&self) -> Result<Json<RuntimeInventory>, String> {
        self.service.list_instances().await.map(Json)
    }

    #[tool(
        description = "Prepare an instance from a trusted template. Requires confirmed=true after user approval. Workspace-only templates prepare guidance and any declared Git repositories without Docker; guidance-only templates require no Git. Service templates start Compose and the shared gateway and verify configured readiness. With wait=false, poll get_operation in this process."
    )]
    async fn create_instance(
        &self,
        Parameters(input): Parameters<CreateInstanceInput>,
    ) -> Result<Json<Operation>, String> {
        let operation = self.service.submit_operation(
            "create_instance",
            &input.name,
            Some(input.template),
            input.timeout_seconds,
            input.confirmed,
        )?;
        if input.wait {
            self.service.wait_operation(&operation.id).await.map(Json)
        } else {
            Ok(Json(operation))
        }
    }

    #[tool(
        description = "Read bounded progress and outcome for an operation in this MCP process. After reconnecting, inspect list_instances instead."
    )]
    async fn get_operation(
        &self,
        Parameters(input): Parameters<OperationInput>,
    ) -> Result<Json<Operation>, String> {
        self.service.get_operation(&input.id).map(Json)
    }

    #[tool(
        description = "Stop an instance's containers while retaining its data for a later restart. Workspace-only instances are preserved without work. Requires confirmed=true. Returns a background operation."
    )]
    async fn stop_instance(
        &self,
        Parameters(input): Parameters<StopInstanceInput>,
    ) -> Result<Json<Operation>, String> {
        self.service
            .submit_operation("stop_instance", &input.name, None, 60, input.confirmed)
            .map(Json)
    }

    #[tool(
        description = "Start only the named service's existing containers. Preserves data and configuration; leaves dependencies and the gateway untouched. One-shot jobs are refused. Requires confirmed=true. Returns a background operation with a ten-minute readiness budget for running state, configured healthchecks and gateway content assertions. Routed services require current template readiness configuration."
    )]
    async fn start_service(
        &self,
        Parameters(input): Parameters<ServiceStateInput>,
    ) -> Result<Json<Operation>, String> {
        self.service
            .submit_service_state(&input.name, input.service, true, input.confirmed)
            .map(Json)
    }

    #[tool(
        description = "Stop only the named service's existing containers, preserving data and configuration. Leaves dependencies and the gateway untouched; one-shot jobs are refused. Requires confirmed=true. Returns a background operation with a one-minute budget."
    )]
    async fn stop_service(
        &self,
        Parameters(input): Parameters<ServiceStateInput>,
    ) -> Result<Json<Operation>, String> {
        self.service
            .submit_service_state(&input.name, input.service, false, input.confirmed)
            .map(Json)
    }

    #[tool(
        description = "Restart an instance's existing long-running containers, including stopped ones. Preserves data and configuration, skips one-shot jobs and the shared gateway. Requires confirmed=true. Returns a background operation with a ten-minute budget; success requires targeted containers to run and pass configured healthchecks and gateway content assertions. Routed services require current template readiness configuration; services without checks are verified only as running."
    )]
    async fn restart_instance(
        &self,
        Parameters(input): Parameters<RestartInstanceInput>,
    ) -> Result<Json<Operation>, String> {
        self.service
            .submit_restart(&input.name, None, input.confirmed)
            .map(Json)
    }

    #[tool(
        description = "Restart only the named service's existing containers within an instance. Leaves other services and dependencies untouched; preserves data and configuration. One-shot jobs are refused. Requires confirmed=true. Returns a background operation with a ten-minute budget; success requires targeted containers to run and pass configured healthchecks and gateway content assertions. Routed services require current template readiness configuration; services without checks are verified only as running."
    )]
    async fn restart_service(
        &self,
        Parameters(input): Parameters<RestartServiceInput>,
    ) -> Result<Json<Operation>, String> {
        self.service
            .submit_restart(&input.name, Some(input.service), input.confirmed)
            .map(Json)
    }

    #[tool(
        description = "Permanently remove an instance's containers, networks, volumes, workspace, and rendered Compose file. Requires confirmed=true. Returns a background operation."
    )]
    async fn delete_instance(
        &self,
        Parameters(input): Parameters<DeleteInstanceInput>,
    ) -> Result<Json<Operation>, String> {
        self.service
            .submit_operation("delete_instance", &input.name, None, 60, input.confirmed)
            .map(Json)
    }

    #[tool(description = "Return Tandem's shared service status.")]
    async fn get_status(&self) -> Json<ServiceStatus> {
        Json(self.service.status())
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for McpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some("Call get_instructions before any other MCP calls.".into()),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            ..Default::default()
        }
    }
}

pub(crate) async fn run_stdio(service: AppService) -> Result<(), Box<dyn std::error::Error>> {
    McpServer::new(service)
        .serve(TolerantStdioTransport::new())
        .await?
        .waiting()
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests;

pub(crate) async fn run_http(
    service: AppService,
    address: SocketAddr,
) -> Result<(), Box<dyn std::error::Error>> {
    run_http_inner(service, address, None).await
}

pub(crate) async fn run_http_with_startup(
    service: AppService,
    address: SocketAddr,
    startup: std::sync::mpsc::Sender<Result<(), String>>,
) -> Result<(), Box<dyn std::error::Error>> {
    run_http_inner(service, address, Some(startup)).await
}

async fn run_http_inner(
    service: AppService,
    address: SocketAddr,
    startup: Option<std::sync::mpsc::Sender<Result<(), String>>>,
) -> Result<(), Box<dyn std::error::Error>> {
    if !address.ip().is_loopback() {
        if let Some(startup) = startup {
            let _ = startup.send(Err("MCP HTTP bind must be loopback".into()));
        }
        return Err("MCP HTTP bind must be loopback".into());
    }
    let listener = match tokio::net::TcpListener::bind(address).await {
        Ok(listener) => listener,
        Err(error) => {
            if let Some(startup) = startup {
                let _ = startup.send(Err(error.to_string()));
            }
            return Err(Box::new(error));
        }
    };
    let router = http::router(service, listener.local_addr()?.port());
    if let Some(startup) = startup {
        let _ = startup.send(Ok(()));
    }
    axum::serve(listener, router)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}
