//! InfraRust - Minecraft Proxy Server
//!
//! Command-line interface for the InfraRust proxy server.

use clap::Parser;
use std::sync::Arc;
use std::{process, time::Duration};
use tokio::signal;
use tokio::sync::mpsc;
use tracing::{error, info, warn};

use infrarust::{
    cli::{CommandProcessor, ShutdownController, command::CommandMessage, commands},
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
    
    /// Disable interactive CLI mode (useful for Docker and non-TTY environments)
    #[arg(long, default_value = "false")]
    no_interactive: bool,
}

async fn wait_for_shutdown_signal(shutdown_controller: Arc<ShutdownController>) {
    #[cfg(unix)]
    {
        let mut term_signal = signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("Failed to create SIGTERM signal handler");
        
        tokio::select! {
            _ = signal::ctrl_c() => {
                info!("Received SIGINT (CTRL+C), goodbye :)");
                shutdown_controller.trigger_shutdown("SIGINT (CTRL+C)").await;
            }
            _ = term_signal.recv() => {
                info!("Received SIGTERM, goodbye :)");
                shutdown_controller.trigger_shutdown("SIGTERM").await;
            }
        }
    }
    
    #[cfg(windows)]
    {
        let mut ctrl_close = signal::windows::ctrl_close().expect("Failed to create CTRL_CLOSE handler");
        
        tokio::select! {
            _ = signal::ctrl_c() => {
                info!("Received CTRL+C, goodbye :)");
                shutdown_controller.trigger_shutdown("CTRL+C").await;
            }
            _ = ctrl_close.recv() => {
                info!("Received CTRL_CLOSE, goodbye :)");
                shutdown_controller.trigger_shutdown("CTRL_CLOSE").await;
            }
        }
    }
}

#[tokio::main]
async fn main() {
    let args = Args::parse();

    let shutdown_controller = ShutdownController::new();

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

    let server = match Infrarust::new(config, shutdown_controller.clone()) {
        Ok(s) => Arc::new(s),
        Err(e) => {
            error!("Failed to create server: {}", e);
            process::exit(1);
        }
    };

    // Get the actor supervisor and config service from the server
    let supervisor = server.get_supervisor();
    let config_service = server.get_config_service();
    
    let signal_task = {
        let shutdown = shutdown_controller.clone();
        tokio::spawn(async move {
            wait_for_shutdown_signal(shutdown).await;
        })
    };
    
    // Server task
    let server_task = {
        let server_clone = server.clone();
        let shutdown = shutdown_controller.clone();
        tokio::spawn(async move {
            if let Err(e) = server_clone.run().await {
                error!("Server error: {}", e);
                shutdown.trigger_shutdown("Server error").await;
            }
        })
    };
    
    if args.no_interactive {
        info!("Interactive mode disabled, not starting command processor");
        
        tokio::select! {
            _ = server_task => {
                info!("Server task completed");
            }
            _ = signal_task => {
                info!("Signal handler task completed");
            }
        }
    } else {
        let commands = commands::get_all_commands(Some(supervisor), Some(config_service));
        let (command_processor, mut command_rx) = CommandProcessor::new(
            commands,
            Some(shutdown_controller.clone())
        );
        
        command_processor.start_input_loop().await;
        
        let command_task = {
            let shutdown = shutdown_controller.clone();
            tokio::spawn(async move {
                while let Some(msg) = command_rx.recv().await {
                    match msg {
                        CommandMessage::Execute(cmd) => {
                            // Process command asynchronously
                            let result = command_processor.process_command(&cmd).await;
                            println!("{}", result);
                        }
                        CommandMessage::Shutdown => {
                            info!("Shutdown requested via command");
                            shutdown.trigger_shutdown("User command").await;
                            break;
                        }
                    }
                }
            })
        };

        tokio::select! {
            _ = server_task => {
                info!("Server task completed");
            }
            _ = signal_task => {
                info!("Signal handler task completed");
            }
            _ = command_task => {
                info!("Command task completed");
            }
        }
    }

    info!("Cleaning up and shutting down...");
    let shutdown_complete = server.shutdown().await;
    
    let timeout = Duration::from_secs(3);
    match tokio::time::timeout(timeout, async {
        let _ = shutdown_complete.await;
    }).await {
        Ok(_) => info!("All components shut down cleanly"),
        Err(_) => warn!("Shutdown timed out after {:?}, forcing exit", timeout),
    }
    
    tokio::time::sleep(Duration::from_millis(200)).await;
    
    info!("Shutdown complete, goodbye!");

    std::process::exit(0);
}
