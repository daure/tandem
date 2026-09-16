mod app;
pub mod cli;
pub mod diagnostics;
mod environments;
mod mcp;
mod service;
mod store;

use std::{net::SocketAddr, sync::mpsc, thread};

use service::AppService;

pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    let service = AppService::initialize()?;
    tuicore::TreeApp::new(app::root(service))
        .initial_focus(app::initial_focus())
        .on_message(|app, message, ctx| app.handle_message(message, ctx))
        .run()?;
    Ok(())
}

pub fn run_mcp() -> Result<(), Box<dyn std::error::Error>> {
    let service = AppService::initialize()?;
    let runtime = tokio::runtime::Runtime::new()?;
    let result = runtime.block_on(mcp::run_stdio(service.clone()));
    drop(runtime);
    result
}

pub fn run_http(bind: SocketAddr) -> Result<(), Box<dyn std::error::Error>> {
    let service = AppService::initialize()?;
    let runtime = tokio::runtime::Runtime::new()?;
    let result = runtime.block_on(mcp::run_http(service.clone(), bind));
    drop(runtime);
    result
}

pub fn run_dev(bind: SocketAddr) -> Result<(), Box<dyn std::error::Error>> {
    let service = AppService::initialize()?;
    let (startup_tx, startup_rx) = mpsc::channel();
    let http_service = service.clone();
    thread::Builder::new()
        .name("tandem-mcp-http".into())
        .spawn(move || match tokio::runtime::Runtime::new() {
            Ok(runtime) => {
                if let Err(error) =
                    runtime.block_on(mcp::run_http_with_startup(http_service, bind, startup_tx))
                {
                    diagnostics::record_error("development HTTP MCP stopped", error.as_ref());
                    eprintln!("HTTP MCP stopped: {error}");
                }
            }
            Err(error) => {
                let _ = startup_tx.send(Err(error.to_string()));
                diagnostics::record_error("could not start development HTTP MCP", &error);
            }
        })?;
    startup_rx
        .recv()
        .map_err(|_| "HTTP MCP startup thread exited without status")?
        .map_err(|error| format!("HTTP MCP startup failed: {error}"))?;
    tuicore::TreeApp::new(app::root(service))
        .initial_focus(app::initial_focus())
        .on_message(|app, message, ctx| app.handle_message(message, ctx))
        .run()?;
    Ok(())
}
