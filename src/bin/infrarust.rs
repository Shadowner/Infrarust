//! InfraRust - Minecraft Proxy Server
//!
//! Command-line interface for the InfraRust proxy server.

use clap::Parser;
use env_logger::Env;
use log::{error, info, warn};
use std::process;
use std::sync::Arc;
use std::time::Duration;

use infrarust::{core::config::{provider::file::FileProvider, InfrarustConfig}, Infrarust};

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[arg(long, default_value = "config.yaml")]
    config_path: String,

    #[arg(long, default_value = "proxies")]
    proxies_path: String,
}

// fn load_config(provider: &FileProvider) -> Result<InfrarustConfig, Box<dyn std::error::Error>> {
//     let mut config = InfrarustConfig {
//         bind: Some("0.0.0.0:25565".to_string()),
//         server_configs: Vec::new(),
//         keepalive_timeout: Some(Duration::from_secs(30)),
//         ..Default::default()
//     };

//     match provider.load_config() {
//         Ok(loaded_config) => config = loaded_config,
//         Err(e) => {
//             warn!(
//                 "Failed to load main configuration file, using default configuration: {}",
//                 e
//             );
//         }
//     }

//     Ok(config)
// }

#[tokio::main]
async fn main() {
    env_logger::Builder::from_env(Env::default().default_filter_or("debug")).init();

    let args = Args::parse();

    // let provider = FileProvider::new(args.config_path, args.proxies_path, FileType::Yaml);

    // let config = match load_config(&provider) {
    //     Ok(config) => config,
    //     Err(e) => {
    //         error!("Failed to load configuration: {}", e);
    //         process::exit(1);
    //     }
    // };

    info!("Starting Infrarust proxy...");

    let mut config = InfrarustConfig::default();
    config.bind = Some("127.0.0.1:25565".to_string());

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
