//! session — CLI tool for live session control (setlist, navigation, playback).

use std::path::PathBuf;

use clap::Parser;
use eyre::Result;

#[derive(Parser)]
#[command(
    name = "session",
    about = "Live session control — setlist navigation and playback"
)]
struct Cli {
    /// Unix socket path for DAW connection (auto-discovers from /tmp if omitted)
    #[arg(long, global = true)]
    socket: Option<PathBuf>,

    /// Output as JSON
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: session_cli::SessionCommand,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "warn".into()),
        )
        .with_writer(std::io::stderr)
        .init();

    let cli = Cli::parse();
    session_cli::run(cli.socket, cli.command, cli.json).await
}
