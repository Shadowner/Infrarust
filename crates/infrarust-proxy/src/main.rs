#[cfg(target_env = "musl")]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use clap::Parser;
use tokio_util::sync::CancellationToken;
use tracing_subscriber::EnvFilter;

use infrarust_api::events::proxy::{ProxyInitializeEvent, ProxyShutdownEvent};
use infrarust_config::ProxyConfig;
use infrarust_core::plugin::manager::{PluginManager, PluginServices};
use infrarust_core::server::ProxyServer;
use infrarust_core::services::ban_bridge::BanServiceBridge;
use infrarust_core::services::config_service::ConfigServiceImpl;
use infrarust_core::services::scheduler::SchedulerImpl;
use infrarust_core::services::server_manager_bridge::{NoopServerManager, ServerManagerBridge};

mod plugins;

/// Infrarust — A Minecraft reverse proxy
#[derive(Parser)]
#[command(name = "infrarust", version, about)]
struct Cli {
    /// Path to the proxy configuration file
    #[arg(short, long, default_value = "infrarust.toml")]
    config: std::path::PathBuf,

    /// Override the bind address (e.g. "0.0.0.0:25577")
    #[arg(short, long)]
    bind: Option<std::net::SocketAddr>,

    /// Log level filter (overridden by `RUST_LOG` env var)
    #[arg(short, long, default_value = "info")]
    log_level: String,
}

#[allow(clippy::print_stderr)] // eprintln used before tracing is initialized
fn main() -> ExitCode {
    let cli = Cli::parse();

    // Load config FIRST (before subscriber, to get telemetry config)
    let config = match load_config(&cli) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e:#}");
            return ExitCode::FAILURE;
        }
    };

    // Init tracing subscriber — RUST_LOG takes priority over --log-level
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(&cli.log_level));

    #[cfg(feature = "telemetry")]
    let _otel_guard = {
        use tracing_subscriber::layer::SubscriberExt;
        use tracing_subscriber::util::SubscriberInitExt;

        if let Some(ref tc) = config.telemetry {
            if tc.enabled {
                match infrarust_core::telemetry::init_telemetry(tc) {
                    Ok(guard) => {
                        let tracer = opentelemetry::global::tracer("infrarust");
                        let otel_layer = tracing_opentelemetry::layer().with_tracer(tracer);

                        tracing_subscriber::registry()
                            .with(filter)
                            .with(tracing_subscriber::fmt::layer().with_target(true))
                            .with(otel_layer)
                            .init();
                        Some(guard)
                    }
                    Err(e) => {
                        // Fall back to fmt-only subscriber
                        tracing_subscriber::fmt()
                            .with_env_filter(filter)
                            .with_target(true)
                            .init();
                        tracing::warn!(
                            "failed to initialize OpenTelemetry: {e}, continuing without telemetry"
                        );
                        None
                    }
                }
            } else {
                tracing_subscriber::fmt()
                    .with_env_filter(filter)
                    .with_target(true)
                    .init();
                None
            }
        } else {
            tracing_subscriber::fmt()
                .with_env_filter(filter)
                .with_target(true)
                .init();
            None
        }
    };

    #[cfg(not(feature = "telemetry"))]
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(true)
        .init();

    tracing::info!(
        bind = %config.bind,
        servers_dir = %config.servers_dir.display(),
        "starting infrarust v{}",
        env!("CARGO_PKG_VERSION"),
    );

    // Build tokio runtime with configurable worker threads
    let mut builder = tokio::runtime::Builder::new_multi_thread();
    if config.worker_threads > 0 {
        builder.worker_threads(config.worker_threads);
    }
    let runtime = match builder.enable_all().thread_name("infrarust-worker").build() {
        Ok(rt) => rt,
        Err(e) => {
            tracing::error!("failed to build tokio runtime: {e}");
            return ExitCode::FAILURE;
        }
    };

    match runtime.block_on(run(config)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            tracing::error!("{e:#}");
            ExitCode::FAILURE
        }
    }
}

fn load_config(cli: &Cli) -> anyhow::Result<ProxyConfig> {
    let content = std::fs::read_to_string(&cli.config)
        .with_context(|| format!("cannot read config file: {}", cli.config.display()))?;

    let mut config: ProxyConfig = toml::from_str(&content)
        .with_context(|| format!("invalid TOML in {}", cli.config.display()))?;

    // CLI --bind overrides config
    if let Some(bind) = cli.bind {
        config.bind = bind;
    }

    infrarust_config::validate_proxy_config(&config).context("configuration validation failed")?;

    Ok(config)
}

async fn run(config: ProxyConfig) -> anyhow::Result<()> {
    let shutdown = CancellationToken::new();

    // Signal handler in background
    let shutdown_signal = shutdown.clone();
    tokio::spawn(async move {
        signal_handler().await;
        tracing::info!("shutdown signal received");
        shutdown_signal.cancel();
    });

    // Build and run the proxy server
    let mut server = ProxyServer::new(config, shutdown.clone())
        .await
        .context("failed to initialize proxy server")?;

    // Collect and load plugins
    let static_plugins = plugins::collect_static_plugins();
    let mut plugin_manager = PluginManager::new(static_plugins)
        .context("failed to resolve plugin dependencies")?;

    let services = server.services();
    let server_manager: Arc<dyn infrarust_api::services::server_manager::ServerManager> =
        match &services.server_manager {
            Some(sm) => Arc::new(ServerManagerBridge::new(Arc::clone(sm))),
            None => Arc::new(NoopServerManager),
        };

    let transport_filter_registry = Arc::new(
        infrarust_core::filter::transport_registry::TransportFilterRegistryImpl::new(),
    );

    let plugin_services = PluginServices {
        event_bus: Arc::clone(&services.event_bus) as Arc<dyn infrarust_api::event::bus::EventBus>,
        player_registry: Arc::clone(&services.player_registry)
            as Arc<dyn infrarust_api::services::player_registry::PlayerRegistry>,
        server_manager,
        ban_service: Arc::new(BanServiceBridge::new(Arc::clone(&services.ban_manager))),
        command_manager: Arc::clone(&services.command_manager)
            as Arc<dyn infrarust_api::command::CommandManager>,
        scheduler: Arc::new(SchedulerImpl::new()),
        config_service: Arc::new(ConfigServiceImpl::new(Arc::clone(&services.domain_router))),
        codec_filter_registry: Arc::clone(&services.codec_filter_registry),
        transport_filter_registry: Arc::clone(&transport_filter_registry),
        plugins_dir: PathBuf::from("plugins"),
    };

    let errors = plugin_manager.enable_all(&plugin_services).await;
    if !errors.is_empty() {
        tracing::warn!(count = errors.len(), "Some plugins failed to enable");
    }

    // Collect limbo handlers registered by plugins and populate the registry
    for handler in plugin_manager.collect_limbo_handlers() {
        services
            .limbo_handler_registry
            .register(std::sync::Arc::from(handler));
    }

    // Rebuild transport filter chain now that plugins may have registered filters
    server.rebuild_transport_filter_chain(&transport_filter_registry);

    tracing::info!("infrarust is ready, accepting connections");

    let server = Arc::new(server);

    server.event_bus().fire(ProxyInitializeEvent).await;

    Arc::clone(&server)
        .run()
        .await
        .context("proxy server error")?;

    plugin_manager.disable_all().await;

    server.event_bus().fire(ProxyShutdownEvent).await;

    // Post-shutdown: drain active connections with a timeout
    let remaining = server.registry().count();
    if remaining > 0 {
        tracing::info!(remaining, "waiting for active connections to drain");

        let _ = tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                let count = server.registry().count();
                if count == 0 {
                    tracing::info!("all connections drained");
                    break;
                }
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
        })
        .await
        .inspect_err(|_| {
            tracing::warn!(
                remaining = server.registry().count(),
                "drain timeout, forcing shutdown"
            );
        });
    }

    tracing::info!("infrarust stopped");
    Ok(())
}

async fn signal_handler() {
    use tokio::signal;

    let ctrl_c = signal::ctrl_c();

    #[cfg(unix)]
    {
        #[allow(clippy::expect_used)]
        // Fatal: if we can't install the signal handler, there's no recovery
        let mut sigterm = signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler");
        tokio::select! {
            biased;
            _ = sigterm.recv() => {}
            _ = ctrl_c => {}
        }
    }

    #[cfg(not(unix))]
    {
        ctrl_c.await.ok();
    }
}
