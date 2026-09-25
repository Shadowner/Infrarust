#[cfg(target_env = "musl")]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use std::io::IsTerminal;
use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;

use anyhow::Context;
use clap::{Parser, Subcommand};
use tokio_util::sync::CancellationToken;
use tracing_subscriber::EnvFilter;

use infrarust_config::ProxyConfig;
use infrarust_core::runtime::{ProxyRuntime, proxy_info_from_config};
use infrarust_core::services::config_service::ConfigServiceImpl;
use infrarust_core::telemetry::formatter::InfrarustFormatter;

mod migrate;
mod plugins;
mod wizard;

/// Infrarust - A Minecraft reverse proxy
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

    /// Override the servers directory path
    #[arg(long)]
    servers_dir: Option<std::path::PathBuf>,
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

    if let Err(e) = infrarust_transport::init_socket_activation() {
        eprintln!("error: {e}");
        return ExitCode::FAILURE;
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
                if let Some(ref servers_dir) = cli.servers_dir {
                    c.servers_dir = servers_dir.clone();
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

    // Init tracing subscriber. RUST_LOG takes priority over --log-level
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(&cli.log_level));

    let formatter = InfrarustFormatter::new();

    let log_layer = if config.web.as_ref().is_some_and(|w| w.enable_api) {
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

    match runtime.block_on(run(config, cli.config)) {
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
    if let Some(ref servers_dir) = cli.servers_dir {
        config.servers_dir = servers_dir.clone();
    }

    infrarust_config::validate_proxy_config(&config).context("configuration validation failed")?;

    Ok(config)
}

async fn run(config: ProxyConfig, config_path: std::path::PathBuf) -> anyhow::Result<()> {
    let shutdown = CancellationToken::new();

    // Signal handler in background
    let shutdown_signal = shutdown.clone();
    tokio::spawn(async move {
        signal_handler().await;
        tracing::info!("shutdown signal received");
        shutdown_signal.cancel();
    });

    let mut web_config = config.web.clone();

    #[cfg(feature = "wasm")]
    let wasm_engine = infrarust_loader_wasm::build_engine(&config)?;
    #[cfg(feature = "wasm")]
    let wasm_config = infrarust_loader_wasm::WasmLoaderConfig::from_proxy_config(&config);

    let static_loader = plugins::build_static_loader(web_config.as_mut())?;
    let static_ids = static_loader.registered_ids();
    let proxy_info = proxy_info_from_config(&config, env!("CARGO_PKG_VERSION"));

    let builder = ProxyRuntime::builder(config, config_path)
        .shutdown_token(shutdown.clone())
        .proxy_info(proxy_info)
        .trusted_plugins(static_ids)
        .loader(Box::new(static_loader));

    #[cfg(feature = "wasm")]
    let builder = builder.loader(Box::new(infrarust_loader_wasm::WasmPluginLoader::new(
        wasm_engine,
        wasm_config,
    )));

    let running = builder
        .start()
        .await
        .context("failed to initialize proxy server")?;

    let services = running.services();
    let console_services = Arc::new(infrarust_core::console::ConsoleServices::new(
        Arc::clone(&services.player_registry),
        Arc::clone(&services.connection_registry),
        Arc::clone(&services.ban_manager),
        services.server_manager.clone(),
        Arc::new(ConfigServiceImpl::new(
            Arc::clone(&services.domain_router),
            services.config_path.clone(),
            Arc::clone(&services.config),
        )),
        Arc::clone(running.plugin_manager()),
        Arc::clone(&services.permission_service),
        Arc::clone(&services.command_manager),
        shutdown.clone(),
        running.start_time(),
    ));

    let console_task = infrarust_core::console::ConsoleTask::new(console_services);
    let console_handle = tokio::spawn(console_task.run());

    tracing::info!("infrarust is ready, accepting connections");

    if let Some(web) = web_config.as_ref().filter(|w| w.enable_api) {
        let label = if web.webui_enabled() {
            "Web dashboard"
        } else {
            "API"
        };
        tracing::info!("{label} accessible at: http://{}", web.bind);
    }

    let result = running.wait().await;
    console_handle.abort();
    result.context("proxy server error")?;

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
