use std::{
    error::Error,
    net::SocketAddr,
    process::{Command, Stdio},
};

use clap::{Parser, Subcommand};

mod inspection;

#[derive(Parser)]
#[command(
    name = "tandem",
    version,
    about = "Tandem development environments, TUI and MCP server"
)]
pub struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    #[command(hide = true)]
    StartupWorker {
        name: String,
        id: String,
        instance_fd: i32,
        lease_fd: i32,
    },
    #[command(about = "Install the OpenCode TUI companion and print its tui.json plugin entry")]
    OpencodeSetup,
    #[command(about = "List instances with runtime status and workspace paths")]
    ListInstances {
        #[arg(long, help = "Print structured JSON")]
        json: bool,
    },
    #[command(about = "Show an instance's runtime details and retained startup result")]
    InspectInstance {
        name: String,
        #[arg(long, help = "Print structured JSON")]
        json: bool,
    },
    #[command(about = "List available templates, including invalid templates")]
    ListTemplates {
        #[arg(long, help = "Print structured JSON")]
        json: bool,
    },
    #[command(about = "Show template configuration, source files, and configured gateway URL")]
    InspectTemplate {
        name: String,
        #[arg(long, help = "Print structured JSON")]
        json: bool,
    },
    #[command(about = "Create an instance if absent; leave existing instances unchanged")]
    NewInstance {
        name: String,
        #[arg(short = 't', long, value_name = "TEMPLATE")]
        template: String,
        #[arg(
            short = 'o',
            long,
            num_args = 0..=1,
            value_name = "INITIAL_PROMPT",
            help = "Open a fresh OpenCode session in Zellij before repository cloning, optionally submitting an initial prompt through the Tandem companion"
        )]
        opencode: Option<Option<String>>,
        #[arg(short = 'd', long)]
        description: Option<String>,
    },
    #[command(
        about = "Start an existing instance by reapplying its trusted template; wait for readiness"
    )]
    StartInstance { name: String },
    #[command(about = "Stop an instance while preserving its workspace, volumes, and networks")]
    StopInstance { name: String },
    #[command(
        about = "Restart existing instance containers, skip setup jobs, and wait for readiness"
    )]
    RestartInstance { name: String },
    #[command(
        about = "Permanently delete an instance, its workspace, volumes, and networks",
        disable_help_flag = true
    )]
    DeleteInstance {
        name: String,
        #[arg(
            short = 'h',
            long,
            help = "Launch deletion in a detached process and return immediately"
        )]
        headless: bool,
    },
    #[command(about = "Run protocol-only MCP server over stdin/stdout")]
    Mcp,
    #[cfg(debug_assertions)]
    #[command(about = "Run the TUI and loopback HTTP MCP for development")]
    Dev {
        #[arg(long, default_value = "127.0.0.1:7348", value_parser = parse_loopback)]
        bind: SocketAddr,
    },
    #[command(about = "Run loopback HTTP MCP in the foreground")]
    Serve {
        #[arg(long, default_value = "127.0.0.1:7345", value_parser = parse_loopback)]
        bind: SocketAddr,
    },
}

fn parse_loopback(value: &str) -> Result<SocketAddr, String> {
    let address = value
        .parse::<SocketAddr>()
        .map_err(|error| error.to_string())?;
    if address.ip().is_loopback() {
        Ok(address)
    } else {
        Err("address must be loopback".into())
    }
}

pub fn run() -> Result<(), Box<dyn Error>> {
    let cli = Cli::try_parse_from(std::env::args_os()).unwrap_or_else(|error| error.exit());
    match cli.command {
        Some(Commands::StartupWorker {
            name,
            id,
            instance_fd,
            lease_fd,
        }) => crate::service::AppService::run_startup_worker(&name, &id, instance_fd, lease_fd)
            .map_err(Into::into),
        Some(Commands::OpencodeSetup) => {
            let service = crate::service::AppService::initialize()?;
            println!("{}", service.setup_opencode()?);
            Ok(())
        }
        Some(Commands::ListInstances { json }) => {
            let service = crate::service::AppService::initialize()?;
            inspection::list_instances(&service, json)
        }
        Some(Commands::InspectInstance { name, json }) => {
            let service = crate::service::AppService::initialize()?;
            inspection::inspect_instance(&service, name, json)
        }
        Some(Commands::ListTemplates { json }) => {
            let service = crate::service::AppService::initialize()?;
            inspection::list_templates(&service, json)
        }
        Some(Commands::InspectTemplate { name, json }) => {
            let service = crate::service::AppService::initialize()?;
            inspection::inspect_template(&service, name, json)
        }
        None => {
            tuicore::init();
            crate::run()
        }
        Some(Commands::Mcp) => crate::run_mcp(),
        #[cfg(debug_assertions)]
        Some(Commands::Dev { bind }) => {
            tuicore::init();
            crate::run_dev(bind)
        }
        Some(Commands::Serve { bind }) => crate::run_http(bind),
        Some(Commands::NewInstance {
            name,
            template,
            opencode,
            description,
        }) => {
            let service = crate::service::AppService::initialize()?;
            eprintln!("Checking {name} for template {template}...");
            let (instance, status) =
                match service.new_instance(&name, template, opencode, description)? {
                    crate::service::NewInstanceOutcome::Created(instance) => (instance, "ready"),
                    crate::service::NewInstanceOutcome::Existing(instance) => {
                        (instance, "already exists; left unchanged")
                    }
                };
            println!(
                "Instance {} {status}\nWorkspace: {}",
                instance.name, instance.workspace
            );
            for service in instance.services {
                if let Some(url) = service.url {
                    println!("{}: {url}", service.name);
                }
            }
            Ok(())
        }
        Some(Commands::StartInstance { name }) => {
            let service = crate::service::AppService::initialize()?;
            eprintln!("Starting {name} from its template...");
            print_instance_operation(service.start_instance(&name)?, "ready");
            Ok(())
        }
        Some(Commands::StopInstance { name }) => {
            let service = crate::service::AppService::initialize()?;
            eprintln!("Stopping {name}; preserving its data...");
            print_instance_operation(service.stop_instance(&name)?, "stopped");
            Ok(())
        }
        Some(Commands::RestartInstance { name }) => {
            let service = crate::service::AppService::initialize()?;
            eprintln!("Restarting {name}'s existing containers...");
            print_instance_operation(service.restart_instance(&name)?, "restarted");
            Ok(())
        }
        Some(Commands::DeleteInstance { name, headless }) if headless => {
            spawn_headless_delete(&name)?;
            println!("Instance {name} deletion started");
            Ok(())
        }
        Some(Commands::DeleteInstance { name, .. }) => {
            let service = crate::service::AppService::initialize()?;
            eprintln!("Deleting {name} and its data...");
            for warning in service.delete_instance(&name)? {
                eprintln!("Warning: {warning}");
            }
            println!("Instance {name} deleted");
            Ok(())
        }
    }
}

fn print_instance_operation(operation: crate::store::environments::Operation, status: &str) {
    for warning in operation.warnings {
        eprintln!("Warning: {warning}");
    }
    println!("Instance {} {status}", operation.name);
    if let Some(instance) = operation.instance {
        println!("Workspace: {}", instance.workspace);
        for service in instance.services {
            if let Some(url) = service.url {
                println!("{}: {url}", service.name);
            }
        }
    }
}

fn spawn_headless_delete(name: &str) -> Result<(), String> {
    let executable = std::env::current_exe()
        .map_err(|error| format!("cannot locate Tandem executable: {error}"))?;
    let mut command = Command::new(executable);
    command.args(["delete-instance", name]);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("cannot start headless deletion: {error}"))
}

#[cfg(test)]
#[path = "cli/tests/mod.rs"]
mod tests;
