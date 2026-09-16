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
    store::environments::{Instance, Instructions, Operation, Template},
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

#[derive(Debug, Serialize, JsonSchema)]
struct TemplateList {
    templates: Vec<Template>,
}

#[derive(Debug, Serialize, JsonSchema)]
struct InstanceList {
    instances: Vec<Instance>,
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
    /// Instance name; 1–40 lowercase letters/digits/hyphens; gateway is reserved.
    name: String,
    /// Set only after approval to execute this trusted Compose template with local Docker privileges.
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
    /// Approval to remove containers and private networks; workspace and volumes are kept.
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
        description = "Read editable agent guidance and the absolute instructions, template and workspace paths. Call this before using Tandem."
    )]
    async fn get_instructions(&self) -> Result<Json<Instructions>, String> {
        self.service.get_instructions().await.map(Json)
    }

    #[tool(
        description = "List template directories, Compose paths and routing metadata. Invalid templates include errors. Does not require Docker."
    )]
    async fn list_templates(&self) -> Result<Json<TemplateList>, String> {
        self.service
            .list_templates()
            .await
            .map(|templates| Json(TemplateList { templates }))
    }

    #[tool(
        description = "Get absolute editable template directory, Compose file/source and manifest. Relative scripts and config belong in this directory."
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
        description = "Discover instances and per-service health/URLs from Docker labels. Running without a probe is up, not healthy."
    )]
    async fn list_instances(&self) -> Result<Json<InstanceList>, String> {
        self.service
            .list_instances()
            .await
            .map(|instances| Json(InstanceList { instances }))
    }

    #[tool(
        description = "Run a trusted template as an instance. Requires confirmed=true after user approval. Starts the shared loopback gateway; waits for content readiness by default. With wait=false, poll get_operation in this process."
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
        description = "Remove an instance's containers and private networks, preserving workspace, volumes and gateway. Requires confirmed=true. Returns a background operation."
    )]
    async fn stop_instance(
        &self,
        Parameters(input): Parameters<StopInstanceInput>,
    ) -> Result<Json<Operation>, String> {
        self.service
            .submit_operation("stop_instance", &input.name, None, 60, input.confirmed)
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
