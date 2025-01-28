//! InfraRust - Minecraft Proxy Server
//!
//! Command-line interface for the InfraRust proxy server.

use clap::Parser;
use env_logger::Env;
use log::{error, info};
use std::process;
use std::sync::Arc;

use infrarust::{core::config::provider::file::FileProvider, Infrarust};

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[arg(long, default_value = "config.yaml")]
    config_path: String,

    #[arg(long)]
    proxies_path: Option<String>,

    #[arg(long, default_value = "false")]
    watch: bool,
}

#[tokio::main]
async fn main() {
    env_logger::Builder::from_env(Env::default().default_filter_or("debug")).init();

    let args = Args::parse();

    let config = match FileProvider::try_load_config(Some(&args.config_path)) {
        Ok(mut config) => {
            if let Some(ref mut file_provider) = config.file_provider {
                if let Some(proxies_path) = args.proxies_path {
                    file_provider.proxies_path.push(proxies_path);
                }
                file_provider.watch = args.watch || file_provider.watch;
            }
            config
        }
        Err(e) => {
            error!("Failed to load configuration: {}", e);
            process::exit(1);
        }
    };

    info!("Starting Infrarust proxy...");

    let server = match Infrarust::new(config) {
        Ok(s) => Arc::new(s),
        Err(e) => {
            error!("Failed to create server: {}", e);
            process::exit(1);
        }
    };

    if let Err(e) = Arc::clone(&server).run().await {
        error!("Server error: {}", e);
        process::exit(1);
    }
}
