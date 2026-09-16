use std::{error::Error, net::SocketAddr};

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "tandem", version, about = "Tandem TUI and MCP server")]
pub struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
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
    match Cli::parse().command {
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
    }
}
