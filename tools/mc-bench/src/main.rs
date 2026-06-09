//! mc-bench — end-to-end Minecraft Play-state load generator + mock backend.
//!
//! Measures how long a packet takes to traverse Infrarust in intercepted mode
//! under sustained concurrent load, and the proxy-added latency delta versus
//! talking to a backend directly. Targets protocol versions < 1.20.2 (764) so
//! that login transitions straight to Play with no config phase.

mod backend;
mod load;
mod proto;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "mc-bench", about, version)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run a minimal mock Minecraft backend that echoes benchmark pings.
    ServeBackend(backend::BackendArgs),
    /// Run the open-loop Play-state load generator.
    Load(load::LoadArgs),
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    match Cli::parse().command {
        Command::ServeBackend(args) => backend::run(args).await,
        Command::Load(args) => load::run(args).await,
    }
}
