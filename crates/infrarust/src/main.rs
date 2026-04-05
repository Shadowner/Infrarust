#[cfg(target_env = "musl")]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use std::io::IsTerminal;
use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use clap::{Parser, Subcommand};
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
use infrarust_core::telemetry::formatter::InfrarustFormatter;

mod migrate;
mod plugins;
mod wizard;

/// Infrarust — A Minecraft reverse proxy
#[derive(Parser)]
#[command(name = "infrarust", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    /// Path to the proxy configuration file
    #[arg(short, long, default_value = "infrarust.toml")]
    config: std::path::PathBuf,

    /// Override the bind address (e.g. "0.0.0.0:25577")
    #[arg(short, long)]
    bind: Option<std::net::SocketAddr>,

    /// Log level filter (overridden by `RUST_LOG` env var)
    #[arg(short, long, default_value = "info")]
    log_level: String,

    /// Override the plugins directory path
    #[arg(long)]
    plugins_dir: Option<std::path::PathBuf>,
}

#[derive(Subcommand)]
enum Command {
    /// Migrate V1 proxy configs (YAML) to V2 server configs (TOML)
    Migrate {
        input: std::path::PathBuf,
        #[arg(short, long, default_value = "./servers")]
        output: std::path::PathBuf,
        #[arg(long)]
        config: Option<std::path::PathBuf>,
    },
}

#[allow(clippy::print_stderr)] // eprintln used before tracing is initialized
fn main() -> ExitCode {
    let cli = Cli::parse();

    if let Some(Command::Migrate {
        input,
        output,
        config,
    }) = &cli.command
    {
        return migrate::run(input, output, config.as_deref());
    }

    let config = if !cli.config.exists()
        && cli.config == Path::new("infrarust.toml")
        && std::io::stdout().is_terminal()
    {
        match wizard::run(&cli.config) {
            Ok(wizard::WizardOutcome::Config(c)) => {
                let mut c = *c;
                // CLI overrides (load_config does this for the normal path)
                if let Some(bind) = cli.bind {
                    c.bind = bind;
                }
                if let Some(ref plugins_dir) = cli.plugins_dir {
                    c.plugins_dir = plugins_dir.clone();
                }
                c
            }
            Ok(wizard::WizardOutcome::ExitClean) => return ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("error: {e:#}");
                return ExitCode::FAILURE;
            }
        }
    } else {
        match load_config(&cli) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("error: {e:#}");
                return ExitCode::FAILURE;
            }
        }
    };

    // Init tracing subscriber — RUST_LOG takes priority over --log-level
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(&cli.log_level));

    let formatter = InfrarustFormatter::new();

    let log_layer = if config.web.is_some() {
        use infrarust_plugin_admin_api::log_layer::{BroadcastLogLayer, LogBroadcast};
        let lb = LogBroadcast::new(512, 1000);
        let layer = BroadcastLogLayer::new(lb.tx.clone(), lb.history.clone(), 1000);
        let _ = LogBroadcast::install(lb);
        Some(layer)
    } else {
        None
    };

    {
        use tracing_subscriber::layer::SubscriberExt;
        use tracing_subscriber::util::SubscriberInitExt;

        #[cfg(feature = "telemetry")]
        let _otel_guard = {
            if let Some(ref tc) = config.telemetry {
                if tc.enabled {
                    match infrarust_core::telemetry::init_telemetry(tc) {
                        Ok(guard) => {
                            let tracer = opentelemetry::global::tracer("infrarust");
                            let otel_layer = tracing_opentelemetry::layer().with_tracer(tracer);

                            tracing_subscriber::registry()
                                .with(filter)
                                .with(tracing_subscriber::fmt::layer().event_format(formatter))
                                .with(otel_layer)
                                .with(log_layer)
                                .init();
                            Some(guard)
                        }
                        Err(e) => {
                            tracing_subscriber::registry()
                                .with(filter)
                                .with(tracing_subscriber::fmt::layer().event_format(formatter))
                                .with(log_layer)
                                .init();
                            tracing::warn!(
                                "failed to initialize OpenTelemetry: {e}, continuing without telemetry"
                            );
                            None
                        }
                    }
                } else {
                    tracing_subscriber::registry()
                        .with(filter)
                        .with(tracing_subscriber::fmt::layer().event_format(formatter))
                        .with(log_layer)
                        .init();
                    None
                }
            } else {
                tracing_subscriber::registry()
                    .with(filter)
                    .with(tracing_subscriber::fmt::layer().event_format(formatter))
                    .with(log_layer)
                    .init();
                None
            }
        };

        #[cfg(not(feature = "telemetry"))]
        tracing_subscriber::registry()
            .with(filter)
            .with(tracing_subscriber::fmt::layer().event_format(formatter))
            .with(log_layer)
            .init();
    }

    infrarust_core::telemetry::formatter::print_banner();

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

    // CLI overrides
    if let Some(bind) = cli.bind {
        config.bind = bind;
    }
    if let Some(ref plugins_dir) = cli.plugins_dir {
        config.plugins_dir = plugins_dir.clone();
    }

    infrarust_config::validate_proxy_config(&config).context("configuration validation failed")?;

    Ok(config)
}

fn build_proxy_info(config: &ProxyConfig) -> infrarust_api::services::proxy_info::ProxyInfo {
    use infrarust_api::services::proxy_info::{
        KeepaliveInfo, ProxyInfo, RateLimitInfo, StatusCacheInfo, UnknownDomainBehavior,
    };

    ProxyInfo {
        version: env!("CARGO_PKG_VERSION").to_string(),
        bind: config.bind,
        max_connections: config.max_connections,
        connect_timeout: config.connect_timeout,
        receive_proxy_protocol: config.receive_proxy_protocol,
        worker_threads: config.worker_threads,
        so_reuseport: config.so_reuseport,
        rate_limit: RateLimitInfo {
            max_connections: config.rate_limit.max_connections,
            window: config.rate_limit.window,
            status_max: config.rate_limit.status_max,
            status_window: config.rate_limit.status_window,
        },
        status_cache: StatusCacheInfo {
            ttl: config.status_cache.ttl,
            max_entries: config.status_cache.max_entries,
        },
        keepalive: KeepaliveInfo {
            time: config.keepalive.time,
            interval: config.keepalive.interval,
            retries: config.keepalive.retries,
        },
        telemetry_enabled: config.telemetry.as_ref().is_some_and(|t| t.enabled),
        docker_enabled: config.docker.is_some(),
        web_api_enabled: config.web.as_ref().is_some_and(|w| w.enable_api),
        web_ui_enabled: config.web.as_ref().is_some_and(|w| w.enable_webui),
        unknown_domain_behavior: match config.unknown_domain_behavior {
            infrarust_config::UnknownDomainBehavior::DefaultMotd => {
                UnknownDomainBehavior::DefaultMotd
            }
            infrarust_config::UnknownDomainBehavior::Drop => UnknownDomainBehavior::Drop,
        },
    }
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

    let mut web_config = config.web.clone();
    let plugins_dir = config.plugins_dir.clone();
    let proxy_info = build_proxy_info(&config);

    // Build and run the proxy server
    let mut server = ProxyServer::new(config, shutdown.clone())
        .await
        .context("failed to initialize proxy server")?;

    let static_loader = plugins::build_static_loader(web_config.as_mut())?;
    let loaders: Vec<Box<dyn infrarust_core::plugin::PluginLoader>> = vec![Box::new(static_loader)];

    let mut plugin_manager = PluginManager::new(loaders);

    let services = server.services();

    plugin_manager
        .discover_all(&plugins_dir)
        .await
        .context("failed to discover plugins")?;

    let server_manager: Arc<dyn infrarust_api::services::server_manager::ServerManager> =
        match &services.server_manager {
            Some(sm) => Arc::new(ServerManagerBridge::new(Arc::clone(sm))),
            None => Arc::new(NoopServerManager),
        };

    let transport_filter_registry =
        Arc::new(infrarust_core::filter::transport_registry::TransportFilterRegistryImpl::new());

    let plugin_registry = Arc::new(infrarust_core::plugin::PluginRegistryImpl::new());

    let start_time = std::time::Instant::now();

    infrarust_core::commands::register_builtin_commands(
        &services.command_manager,
        services,
        Arc::clone(&plugin_registry)
            as Arc<dyn infrarust_api::services::plugin_registry::PluginRegistry>,
        start_time,
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
        plugin_registry: Arc::clone(&plugin_registry)
            as Arc<dyn infrarust_api::services::plugin_registry::PluginRegistry>,
        codec_filter_registry: Arc::clone(&services.codec_filter_registry),
        transport_filter_registry: Arc::clone(&transport_filter_registry),
        domain_router: Arc::clone(&services.domain_router),
        proxy_shutdown: shutdown.clone(),
        proxy_info,
        plugins_dir,
    };

    let context_factory = infrarust_core::plugin::PluginContextFactoryImpl::new(
        plugin_services,
        std::collections::HashMap::new(),
    );

    let errors = plugin_manager.load_and_enable_all(&context_factory).await;
    if !errors.is_empty() {
        tracing::warn!(count = errors.len(), "Some plugins failed to enable");
    }

    // Refresh the plugin registry snapshot so all plugins are visible via the API
    plugin_registry.update_from(&plugin_manager.list_plugins(), &|id| {
        plugin_manager.plugin_state(id).cloned()
    });

    // Collect limbo handlers registered by plugins and populate the registry
    for handler in plugin_manager.collect_limbo_handlers() {
        services
            .limbo_handler_registry
            .register(std::sync::Arc::from(handler));
    }

    let plugin_providers = plugin_manager.collect_config_providers();
    if !plugin_providers.is_empty() {
        tracing::info!(
            count = plugin_providers.len(),
            "activating plugin config providers"
        );
        let results = infrarust_core::provider::plugin_adapter::activate_plugin_providers(
            plugin_providers,
            services.provider_event_sender.clone(),
            &services.domain_router,
            shutdown.clone(),
        )
        .await;
        plugin_manager.store_provider_cleanup(results);
    }

    // Clone Arcs for console before releasing the immutable borrow on `server`
    let console_player_registry = Arc::clone(&services.player_registry);
    let console_connection_registry = Arc::clone(&services.connection_registry);
    let console_ban_manager = Arc::clone(&services.ban_manager);
    let console_server_manager = services.server_manager.clone();
    let console_domain_router = Arc::clone(&services.domain_router);
    let console_permission_service = Arc::clone(&services.permission_service);

    // Rebuild transport filter chain now that plugins may have registered filters
    server.rebuild_transport_filter_chain(&transport_filter_registry);

    // Wrap PluginManager in Arc<RwLock> for shared read access from console
    let plugin_manager = Arc::new(tokio::sync::RwLock::new(plugin_manager));

    // Start interactive console
    let console_services = Arc::new(infrarust_core::console::ConsoleServices::new(
        console_player_registry,
        console_connection_registry,
        console_ban_manager,
        console_server_manager,
        Arc::new(ConfigServiceImpl::new(console_domain_router)),
        Arc::clone(&plugin_manager),
        console_permission_service,
        shutdown.clone(),
        start_time,
    ));

    let console_task = infrarust_core::console::ConsoleTask::new(console_services);
    let console_handle = tokio::spawn(console_task.run());

    tracing::info!("infrarust is ready, accepting connections");

    if let Some(web) = &web_config {
        let label = if web.enable_webui {
            "Web dashboard"
        } else {
            "API"
        };
        tracing::info!("{label} accessible at: http://{}", web.bind);
    }

    let server = Arc::new(server);

    server.event_bus().fire(ProxyInitializeEvent).await;

    Arc::clone(&server)
        .run()
        .await
        .context("proxy server error")?;

    console_handle.abort();

    plugin_manager.write().await.shutdown().await;

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
