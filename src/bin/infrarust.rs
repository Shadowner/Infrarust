//! InfraRust - Minecraft Proxy Server
//!
//! Command-line interface for the InfraRust proxy server.

use clap::Parser;
use std::sync::Arc;
use std::{process, time::Duration};
use tracing::{error, info, warn};

use infrarust::{
    core::config::provider::file::FileProvider,
    telemetry::{
        self, exporter::resource, init_meter_provider, start_system_metrics_collection,
    },
    Infrarust,
};

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
            println!("Failed to load configuration: {}", e);
            process::exit(1);
        }
    };

    let _logging_guard = telemetry::tracing::init_logging(&config.logging);
    
    let mut _meter_guard: Option<telemetry::MeterProviderGuard> = None;
    let mut _tracer_guard: Option<telemetry::tracing::TracerProviderGuard> = None;
    
    if config.telemetry.enabled {
        if config.telemetry.enable_tracing {
            _tracer_guard = telemetry::tracing::init_opentelemetry_tracing(
                resource(), 
                &config.telemetry
            );
        }
        
        if config.telemetry.enable_metrics {
            if config.telemetry.export_url.clone().is_none() {
                warn!("Metrics enabled but no export URL provided");
            } else {
                start_system_metrics_collection();
                _meter_guard = Some(init_meter_provider(
                    resource(),
                    config.telemetry.export_url.clone().unwrap(),
                    Duration::from_secs(config.telemetry.export_interval_seconds),
                ));
            }
        }
    }

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
