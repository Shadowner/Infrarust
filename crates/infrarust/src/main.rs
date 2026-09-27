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
use infrarust_plugin_admin_api::log_layer::{BroadcastLogLayer, LogBroadcast};

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
        #[arg(short, long, default_value_os_t = infrarust_config::defaults::servers_dir())]
        output: std::path::PathBuf,
        #[arg(long)]
        config: Option<std::path::PathBuf>,
    },
}

#[allow(clippy::print_stderr)] // eprintln used before tracing is initialized
fn main() -> ExitCode {
    let cli = Cli::parse();
    infrarust_core::terminal::color::init();

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

    let (config, config_warnings) = if !cli.config.exists()
        && cli.config == Path::new("infrarust.toml")
        && std::io::stdout().is_terminal()
    {
        match wizard::run(&cli.config).and_then(|outcome| match outcome {
            wizard::WizardOutcome::Config(c) => finalize_config(&cli, *c).map(Some),
            wizard::WizardOutcome::ExitClean => Ok(None),
        }) {
            Ok(Some(c)) => c,
            Ok(None) => return ExitCode::SUCCESS,
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

    let log_broadcast = config
        .web
        .as_ref()
        .is_some_and(|w| w.enable_api)
        .then(|| LogBroadcast::new(512, 1000));
    let _tracing_guard = init_tracing(
        &cli.log_level,
        &config,
        log_broadcast.as_ref().map(LogBroadcast::layer),
    );

    infrarust_core::telemetry::formatter::print_banner();

    for warning in config_warnings {
        tracing::warn!("{warning}");
    }

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

    match runtime.block_on(run(config, cli.config, log_broadcast)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            tracing::error!("{e:#}");
            ExitCode::FAILURE
        }
    }
}

struct TracingGuard {
    #[cfg(feature = "telemetry")]
    _otel: Option<infrarust_core::telemetry::OtelGuard>,
}

fn init_tracing(
    log_level: &str,
    #[cfg_attr(not(feature = "telemetry"), allow(unused_variables))] config: &ProxyConfig,
    log_layer: Option<BroadcastLogLayer>,
) -> TracingGuard {
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(log_level));

    let registry = tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer().event_format(InfrarustFormatter::new()));

    #[cfg(feature = "telemetry")]
    let otel = enabled_telemetry(config).map(infrarust_core::telemetry::init_telemetry);
    #[cfg(feature = "telemetry")]
    let registry = registry.with(otel.as_ref().is_some_and(Result::is_ok).then(|| {
        tracing_opentelemetry::layer().with_tracer(opentelemetry::global::tracer("infrarust"))
    }));

    registry.with(log_layer).init();

    TracingGuard {
        #[cfg(feature = "telemetry")]
        _otel: match otel {
            Some(Ok(guard)) => Some(guard),
            Some(Err(e)) => {
                tracing::warn!(
                    "failed to initialize OpenTelemetry: {e}, continuing without telemetry"
                );
                None
            }
            None => None,
        },
    }
}

#[cfg(feature = "telemetry")]
fn enabled_telemetry(config: &ProxyConfig) -> Option<&infrarust_config::TelemetryConfig> {
    config.telemetry.as_ref().filter(|tc| tc.enabled)
}

fn load_config(cli: &Cli) -> anyhow::Result<(ProxyConfig, Vec<String>)> {
    let content = std::fs::read_to_string(&cli.config)
        .with_context(|| format!("cannot read config file: {}", cli.config.display()))?;

    let config: ProxyConfig = toml::from_str(&content)
        .with_context(|| format!("invalid TOML in {}", cli.config.display()))?;

    finalize_config(cli, config)
}

fn apply_cli_overrides(cli: &Cli, config: &mut ProxyConfig) {
    if let Some(bind) = cli.bind {
        config.bind = bind;
    }
    if let Some(ref plugins_dir) = cli.plugins_dir {
        config.plugins_dir = plugins_dir.clone();
    }
    if let Some(ref servers_dir) = cli.servers_dir {
        config.servers_dir = servers_dir.clone();
    }
}

fn finalize_config(
    cli: &Cli,
    mut config: ProxyConfig,
) -> anyhow::Result<(ProxyConfig, Vec<String>)> {
    apply_cli_overrides(cli, &mut config);
    let warnings = infrarust_config::validate_proxy_config(&config)
        .context("configuration validation failed")?;
    Ok((config, warnings))
}

async fn run(
    config: ProxyConfig,
    config_path: std::path::PathBuf,
    log_broadcast: Option<LogBroadcast>,
) -> anyhow::Result<()> {
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

    let static_loader = plugins::build_static_loader(web_config.as_mut(), log_broadcast)?;
    let static_ids = static_loader.registered_ids();
    let proxy_info = proxy_info_from_config(&config, env!("CARGO_PKG_VERSION"));

    let builder = ProxyRuntime::builder(config, config_path)
        .shutdown_token(shutdown.clone())
        .proxy_info(proxy_info)
        .trusted_plugins(static_ids)
        .loader(Box::new(static_loader));

    #[cfg(feature = "wasm")]
    let builder = builder.loader(Box::new(
        infrarust_loader_wasm::WasmPluginLoader::new(wasm_engine, wasm_config)
            .context("failed to start the wasm plugin loader")?,
    ));

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
        let mut sigterm = match signal::unix::signal(signal::unix::SignalKind::terminate()) {
            Ok(sigterm) => sigterm,
            Err(e) => {
                tracing::error!(error = %e, "failed to install SIGTERM handler; only Ctrl-C will stop the proxy");
                ctrl_c.await.ok();
                return;
            }
        };
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    fn parse(args: &[&str]) -> Cli {
        Cli::parse_from(std::iter::once("infrarust").chain(args.iter().copied()))
    }

    fn wizard_like_config(servers_dir: &Path) -> ProxyConfig {
        let toml_str = format!(
            "bind = \"0.0.0.0:25565\"\nservers_dir = {:?}\n\n[web]\nenable_api = true\nbind = \"127.0.0.1:8080\"\n",
            servers_dir.display().to_string()
        );
        toml::from_str(&toml_str).unwrap()
    }

    #[test]
    fn cli_overrides_replace_only_the_flags_given() {
        let tmp = tempfile::tempdir().unwrap();
        let mut config = wizard_like_config(tmp.path());
        let plugins_dir = config.plugins_dir.clone();

        apply_cli_overrides(&parse(&["--bind", "127.0.0.1:25577"]), &mut config);
        assert_eq!(config.bind, "127.0.0.1:25577".parse().unwrap());
        assert_eq!(config.servers_dir, tmp.path());
        assert_eq!(config.plugins_dir, plugins_dir);

        apply_cli_overrides(
            &parse(&["--servers-dir", "/srv/mc", "--plugins-dir", "/srv/plugins"]),
            &mut config,
        );
        assert_eq!(config.bind, "127.0.0.1:25577".parse().unwrap());
        assert_eq!(config.servers_dir, Path::new("/srv/mc"));
        assert_eq!(config.plugins_dir, Path::new("/srv/plugins"));
    }

    #[cfg(feature = "telemetry")]
    #[test]
    fn telemetry_is_only_enabled_when_the_section_says_so() {
        let tmp = tempfile::tempdir().unwrap();
        let config = wizard_like_config(tmp.path());
        assert!(enabled_telemetry(&config).is_none());

        let mut config = config;
        config.telemetry = Some(infrarust_config::TelemetryConfig::default());
        assert!(enabled_telemetry(&config).is_none());

        config.telemetry = Some(infrarust_config::TelemetryConfig {
            enabled: true,
            ..infrarust_config::TelemetryConfig::default()
        });
        assert!(enabled_telemetry(&config).is_some_and(|tc| tc.enabled));
    }

    #[test]
    fn a_bind_override_is_validated_against_the_generated_web_bind() {
        let tmp = tempfile::tempdir().unwrap();
        let config = wizard_like_config(tmp.path());

        let err = finalize_config(&parse(&["--bind", "0.0.0.0:8080"]), config.clone()).unwrap_err();
        assert!(format!("{err:#}").contains("collides"), "{err:#}");

        let (ok, warnings) = finalize_config(&parse(&["--bind", "0.0.0.0:25566"]), config).unwrap();
        assert_eq!(ok.bind, "0.0.0.0:25566".parse().unwrap());
        assert!(warnings.is_empty(), "{warnings:?}");
    }
}
