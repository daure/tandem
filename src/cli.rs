use std::{
    error::Error,
    net::SocketAddr,
    process::{Command, Stdio},
};

use clap::{Parser, Subcommand};

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
    #[command(about = "Install the OpenCode TUI companion and print its tui.json plugin entry")]
    OpencodeSetup,
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
            help = "Open a fresh OpenCode session in Zellij when the workspace is ready, optionally submitting an initial prompt through the Tandem companion"
        )]
        opencode: Option<Option<String>>,
        #[arg(short = 'd', long)]
        description: Option<String>,
    },
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
        Some(Commands::OpencodeSetup) => {
            let service = crate::service::AppService::initialize()?;
            println!("{}", service.setup_opencode()?);
            Ok(())
        }
        None => {
            tuicore::init();
            crate::run()
        }
        Some(Commands::Mcp) => crate::run_mcp(),
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
