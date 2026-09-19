use std::{
    error::Error,
    ffi::OsString,
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
    #[command(about = "Create an instance if absent; leave existing instances unchanged")]
    NewInstance {
        name: String,
        #[arg(short = 't', long, value_name = "TEMPLATE")]
        template: String,
        #[arg(
            long,
            num_args = 0..=1,
            value_name = "EXTRA",
            help = "Run the saved open command when workspace repositories are on disk, optionally setting TANDEM_EXTRA (alias: -oc)"
        )]
        open_command: Option<Option<String>>,
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
        #[arg(
            long,
            help = "Run the saved close command before removing the workspace (alias: -cc)"
        )]
        close_command: bool,
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
    let cli = Cli::try_parse_from(normalize_arguments(std::env::args_os()))
        .unwrap_or_else(|error| error.exit());
    match cli.command {
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
            open_command,
        }) => {
            let service = crate::service::AppService::initialize()?;
            eprintln!("Checking {name} for template {template}...");
            let (instance, status) = match service.new_instance(&name, template, open_command)? {
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
        Some(Commands::DeleteInstance {
            name,
            headless,
            close_command,
        }) if headless => {
            spawn_headless_delete(&name, close_command)?;
            println!("Instance {name} deletion started");
            Ok(())
        }
        Some(Commands::DeleteInstance {
            name,
            close_command,
            ..
        }) => {
            let service = crate::service::AppService::initialize()?;
            eprintln!("Deleting {name} and its data...");
            for warning in service.delete_instance(&name, close_command)? {
                eprintln!("Warning: {warning}");
            }
            println!("Instance {name} deleted");
            Ok(())
        }
    }
}

fn spawn_headless_delete(name: &str, close_command: bool) -> Result<(), String> {
    let executable = std::env::current_exe()
        .map_err(|error| format!("cannot locate Tandem executable: {error}"))?;
    let mut command = Command::new(executable);
    command.args(["delete-instance", name]);
    if close_command {
        command.arg("--close-command");
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("cannot start headless deletion: {error}"))
}

fn normalize_arguments(arguments: impl IntoIterator<Item = OsString>) -> Vec<OsString> {
    let mut arguments: Vec<_> = arguments.into_iter().collect();
    let alias = match arguments.get(1).and_then(|argument| argument.to_str()) {
        Some("new-instance") => ("-oc", "--open-command"),
        Some("delete-instance") => ("-cc", "--close-command"),
        _ => return arguments,
    };
    {
        // Clap short options are single characters; preserve values and the `--` boundary.
        let mut template_value = false;
        for argument in arguments.iter_mut().skip(2) {
            if template_value {
                template_value = false;
            } else if argument == "--" {
                break;
            } else if alias.0 == "-oc" && (argument == "-t" || argument == "--template") {
                template_value = true;
            } else if argument == alias.0 {
                *argument = alias.1.into();
            }
        }
    }
    arguments
}

#[cfg(test)]
#[path = "cli/tests/mod.rs"]
mod tests;
