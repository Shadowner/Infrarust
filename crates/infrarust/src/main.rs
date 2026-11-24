#[cfg(feature = "mimalloc")]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use std::ffi::OsString;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use anyhow::Context;
use clap::{Parser, Subcommand};
use tokio_util::sync::CancellationToken;
use tracing_appender::non_blocking::{NonBlockingBuilder, WorkerGuard};
use tracing_subscriber::EnvFilter;

use infrarust_config::ProxyConfig;
use infrarust_core::binary::{BuildInfo, ConfigCheck, ExeSnapshot, RunningBinary};
use infrarust_core::hot_restart::{BinaryPreflight, Inheritance};
use infrarust_core::runtime::{
    ProxyExit, ProxyRuntime, ProxyRuntimeBuilder, RestartHandover, proxy_info_from_config,
};
use infrarust_core::terminal::Banner;
use infrarust_core::terminal::formatter::InfrarustFormatter;
use infrarust_plugin_admin_api::log_layer::{BroadcastLogLayer, LogBroadcast};

use crate::signals::{HubSlot, OsSignals, Watch};

mod hot_restart;
mod migrate;
mod plugins;
mod signals;
mod update_cli;
mod wizard;

/// Infrarust - A Minecraft reverse proxy
#[derive(Parser)]
#[command(name = "infrarust", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    /// Path to the proxy configuration file
    #[arg(short, long, default_value = "infrarust.toml")]
    config: PathBuf,

    /// Override the bind address (e.g. "0.0.0.0:25577")
    #[arg(short, long)]
    bind: Option<std::net::SocketAddr>,

    /// Log level filter (overridden by `RUST_LOG` env var)
    #[arg(short, long, default_value = "info")]
    log_level: String,

    /// Override the plugins directory path
    #[arg(long)]
    plugins_dir: Option<PathBuf>,

    /// Override the servers directory path
    #[arg(long)]
    servers_dir: Option<PathBuf>,

    #[arg(long = "handover-probe", hide = true)]
    handover_probe: bool,
}

#[derive(Subcommand)]
enum Command {
    /// Migrate V1 proxy configs (YAML) to V2 server configs (TOML)
    Migrate {
        input: PathBuf,
        #[arg(short, long, default_value_os_t = infrarust_config::defaults::servers_dir())]
        output: PathBuf,
        #[arg(long)]
        config: Option<PathBuf>,
    },
    #[command(
        name = "check-config",
        about = "Validate the proxy configuration without starting the proxy"
    )]
    CheckConfig {
        #[arg(long, help = "Print the result as one JSON line")]
        json: bool,
    },
    #[command(about = "Check for, install or roll back Infrarust releases")]
    Update {
        #[command(subcommand)]
        action: update_cli::UpdateAction,
    },
}

const BUILD: BuildInfo = BuildInfo {
    version: env!("CARGO_PKG_VERSION"),
    target: env!("INFRARUST_TARGET"),
    official: matches!(env!("INFRARUST_OFFICIAL").as_bytes(), b"1"),
    docker: matches!(env!("INFRARUST_DOCKER").as_bytes(), b"1"),
};

const RUNTIME_SHUTDOWN: Duration = Duration::from_secs(3);

fn main() -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) => return hot_restart::refuse_command_line(error),
    };
    if cli.handover_probe {
        return print_probe();
    }
    infrarust_core::terminal::color::init();
    let exe = ExeSnapshot::capture();

    match &cli.command {
        Some(Command::CheckConfig { json }) => return check_config(&cli, *json),
        Some(Command::Update { action }) => return update_cli::run(&cli, action, exe),
        Some(Command::Migrate {
            input,
            output,
            config,
        }) => return migrate::run(input, output, config.as_deref()),
        None => {}
    }

    match hot_restart::inherit() {
        Ok(inheritance) => serve(cli, exe, inheritance),
        Err(code) => code,
    }
}

fn serve(
    cli: Cli,
    exe: std::io::Result<ExeSnapshot>,
    inheritance: Option<Box<dyn Inheritance>>,
) -> ExitCode {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    let inherited_listener = match infrarust_transport::activated_listener() {
        Ok(listener) => listener,
        Err(e) => return hot_restart::give_up(inheritance, &e.to_string()),
    };

    let (config, config_warnings) = match prepare_config(&cli, inheritance.is_some()) {
        Ok(Some(config)) => config,
        Ok(None) => return ExitCode::SUCCESS,
        Err(e) => return hot_restart::give_up(inheritance, &format!("{e:#}")),
    };

    let log_broadcast = config
        .web
        .as_ref()
        .is_some_and(|w| w.enable_api)
        .then(|| LogBroadcast::new(512, 1000));
    let tracing_guard = init_tracing(
        &cli.log_level,
        &config,
        log_broadcast.as_ref().map(LogBroadcast::layer),
    );

    startup_banner(&config).print();

    for warning in config_warnings {
        tracing::warn!("{warning}");
    }

    tracing::info!(
        bind = %config.bind,
        servers_dir = %config.servers_dir.display(),
        "starting infrarust v{}",
        env!("CARGO_PKG_VERSION"),
    );

    let runtime = match build_runtime(&config) {
        Ok(runtime) => runtime,
        Err(e) => {
            let reason = format!("failed to build tokio runtime: {e}");
            tracing::error!("{reason}");
            return hot_restart::give_up(inheritance, &reason);
        }
    };

    let binary = match exe {
        Ok(exe) => Some(RunningBinary::new(BUILD, exe, args)),
        Err(e) => {
            tracing::warn!(error = %e, "cannot locate the running executable; updates and hot restarts are unavailable");
            None
        }
    };
    let successor = hot_restart::successor(binary.as_ref());

    let exit = runtime.block_on(run(Launch {
        hot_restart: successor.is_some(),
        config,
        config_path: cli.config,
        log_broadcast,
        inherited_listener,
        binary,
        inheritance,
    }));
    match exit {
        Ok(Exit::Stopped) => ExitCode::SUCCESS,
        Ok(Exit::Restart(handover)) => {
            runtime.shutdown_timeout(RUNTIME_SHUTDOWN);
            drop(tracing_guard);
            hot_restart::hand_over(successor, *handover)
        }
        Err(e) => {
            tracing::error!("{e:#}");
            ExitCode::FAILURE
        }
    }
}

fn build_runtime(config: &ProxyConfig) -> std::io::Result<tokio::runtime::Runtime> {
    let mut builder = tokio::runtime::Builder::new_multi_thread();
    if config.worker_threads > 0 {
        builder.worker_threads(config.worker_threads);
    }
    builder.enable_all().thread_name("infrarust-worker").build()
}

fn prepare_config(
    cli: &Cli,
    inherited: bool,
) -> anyhow::Result<Option<(ProxyConfig, Vec<String>)>> {
    let wizard = !inherited
        && !cli.config.exists()
        && cli.config == Path::new("infrarust.toml")
        && std::io::stdout().is_terminal();
    if !wizard {
        return load_config(cli).map(Some);
    }
    match wizard::run(&cli.config)? {
        wizard::WizardOutcome::Config(config) => finalize_config(cli, *config).map(Some),
        wizard::WizardOutcome::ExitClean => Ok(None),
    }
}

static LOG_WORKER: Mutex<Option<WorkerGuard>> = Mutex::new(None);

fn flush_logs() {
    let worker = LOG_WORKER
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .take();
    drop(worker);
}

struct LogFlush;

impl Drop for LogFlush {
    fn drop(&mut self) {
        flush_logs();
    }
}

struct TracingGuard {
    #[cfg(feature = "telemetry")]
    _otel: Option<infrarust_core::telemetry::OtelGuard>,
    _logs: LogFlush,
}

fn init_tracing(
    log_level: &str,
    #[cfg_attr(not(feature = "telemetry"), allow(unused_variables))] config: &ProxyConfig,
    log_layer: Option<BroadcastLogLayer>,
) -> TracingGuard {
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(log_level));

    let (stdout, worker) = NonBlockingBuilder::default()
        .lossy(false)
        .thread_name("infrarust-log")
        .finish(std::io::stdout());
    *LOG_WORKER.lock().unwrap_or_else(PoisonError::into_inner) = Some(worker);

    let registry = tracing_subscriber::registry().with(filter).with(
        tracing_subscriber::fmt::layer()
            .event_format(InfrarustFormatter::new())
            .with_writer(stdout),
    );

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
        _logs: LogFlush,
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

#[allow(clippy::print_stderr)]
fn print_probe() -> ExitCode {
    match serde_json::to_string(&BUILD.probe()) {
        Ok(line) => {
            println!("{line}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

#[allow(clippy::print_stderr)]
fn check_config(cli: &Cli, json: bool) -> ExitCode {
    let check = match load_config(cli) {
        Ok((_, warnings)) => ConfigCheck {
            ok: true,
            config: cli.config.display().to_string(),
            warnings,
            error: None,
        },
        Err(e) => ConfigCheck {
            ok: false,
            config: cli.config.display().to_string(),
            warnings: Vec::new(),
            error: Some(format!("{e:#}")),
        },
    };
    if json {
        match serde_json::to_string(&check) {
            Ok(line) => println!("{line}"),
            Err(e) => {
                eprintln!("error: {e}");
                return ExitCode::from(2);
            }
        }
    } else {
        for warning in &check.warnings {
            eprintln!("warning: {warning}");
        }
        match &check.error {
            None => println!("configuration is valid: {}", check.config),
            Some(error) => eprintln!("error: {error}"),
        }
    }
    if check.ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
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

struct Launch {
    config: ProxyConfig,
    config_path: PathBuf,
    log_broadcast: Option<LogBroadcast>,
    inherited_listener: Option<std::net::TcpListener>,
    binary: Option<Arc<RunningBinary>>,
    inheritance: Option<Box<dyn Inheritance>>,
    hot_restart: bool,
}

enum Exit {
    Stopped,
    Restart(Box<RestartHandover>),
}

async fn run(launch: Launch) -> anyhow::Result<Exit> {
    let Launch {
        config,
        config_path,
        log_broadcast,
        inherited_listener,
        binary,
        inheritance,
        hot_restart,
    } = launch;
    let shutdown = CancellationToken::new();
    let hub = HubSlot::default();

    tokio::spawn(signals::watch_signals(
        OsSignals::install(),
        Watch {
            shutdown: shutdown.clone(),
            hub: Arc::clone(&hub),
        },
        |code| {
            flush_logs();
            std::process::exit(code)
        },
    ));

    let web_config = config.web.clone();
    let builder = match runtime_builder(config, config_path, log_broadcast, &shutdown) {
        Ok(builder) => builder,
        Err(error) => return Err(hot_restart::abandon(inheritance, error)),
    };
    let mut builder = builder
        .inherited_listener(inherited_listener)
        .binary(binary);
    if hot_restart {
        builder = builder.hot_restart(Arc::new(BinaryPreflight));
    }
    if let Some(inheritance) = inheritance {
        builder = builder.restored(inheritance);
    }

    let running = builder
        .start()
        .await
        .context("failed to initialize proxy server")?;
    let _ = hub.set(running.restart_hub());

    let console_services = Arc::new(running.console_services());

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

    let exit = running.run_to_exit().await;
    console_handle.abort();
    match exit.context("proxy server error")? {
        ProxyExit::Stopped => {
            tracing::info!("infrarust stopped");
            Ok(Exit::Stopped)
        }
        ProxyExit::Restart(handover) => {
            tracing::info!(
                sessions = handover.package.state.sessions.len(),
                target_binary = %handover.plan.target.display(),
                "infrarust hands its sessions over to the next image"
            );
            Ok(Exit::Restart(handover))
        }
    }
}

fn runtime_builder(
    config: ProxyConfig,
    config_path: PathBuf,
    log_broadcast: Option<LogBroadcast>,
    shutdown: &CancellationToken,
) -> anyhow::Result<ProxyRuntimeBuilder> {
    let web_config = config.web.clone();

    #[cfg(feature = "wasm")]
    let wasm_engine = infrarust_loader_wasm::build_engine(&config)?;
    #[cfg(feature = "wasm")]
    let wasm_config = infrarust_loader_wasm::WasmLoaderConfig::from_proxy_config(&config);

    let static_loader = plugins::build_static_loader(web_config.as_ref(), log_broadcast)?;
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

    Ok(builder)
}

fn startup_banner(config: &ProxyConfig) -> Banner {
    let workers = match config.worker_threads {
        0 => "auto".to_string(),
        count => count.to_string(),
    };
    let mut banner = Banner::new()
        .row("listen", config.bind)
        .row("servers", config.servers_dir.display())
        .row("plugins", config.plugins_dir.display())
        .row("workers", workers);
    if let Some(web) = config.web.as_ref().filter(|web| web.enable_api) {
        banner = banner.row("web", &web.bind);
    }
    if config.docker.is_some() {
        banner = banner.row("docker", "on");
    }
    banner.row(
        "platform",
        format!("{}/{}", std::env::consts::OS, std::env::consts::ARCH),
    )
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    pub(crate) fn parse(args: &[&str]) -> Cli {
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

    fn update_action(args: &[&str]) -> update_cli::UpdateAction {
        match parse(args).command {
            Some(Command::Update { action }) => action,
            _ => panic!("{args:?} is not an update command"),
        }
    }

    #[test]
    fn the_command_line_definition_is_consistent() {
        <Cli as clap::CommandFactory>::command().debug_assert();
    }

    #[test]
    fn update_check_parses_its_channel_and_json_flags() {
        use update_cli::{ChannelArg, UpdateAction};

        assert_eq!(
            update_action(&["update", "check"]),
            UpdateAction::Check {
                channel: None,
                json: false
            }
        );
        for (value, channel) in [
            ("auto", ChannelArg::Auto),
            ("stable", ChannelArg::Stable),
            ("prerelease", ChannelArg::Prerelease),
        ] {
            assert_eq!(
                update_action(&["update", "check", "--channel", value, "--json"]),
                UpdateAction::Check {
                    channel: Some(channel),
                    json: true
                }
            );
        }
    }

    #[test]
    fn update_check_rejects_an_unknown_channel() {
        for args in [
            ["infrarust", "update", "check", "--channel", "nightly"],
            ["infrarust", "update", "check", "--channel", "Stable"],
        ] {
            let error = Cli::try_parse_from(args).err().unwrap();
            assert_eq!(
                error.kind(),
                clap::error::ErrorKind::InvalidValue,
                "{args:?}"
            );
        }
    }

    #[test]
    fn update_install_parses_its_version_and_confirmation() {
        use update_cli::UpdateAction;

        assert_eq!(
            update_action(&["update", "install"]),
            UpdateAction::Install {
                version: None,
                yes: false
            }
        );
        assert_eq!(
            update_action(&["update", "install", "--version", "2.0.0-beta.6", "--yes"]),
            UpdateAction::Install {
                version: Some("2.0.0-beta.6".to_string()),
                yes: true
            }
        );
        assert_eq!(
            update_action(&["update", "install", "-y"]),
            UpdateAction::Install {
                version: None,
                yes: true
            }
        );
    }

    #[test]
    fn update_rollback_parses_its_confirmation() {
        use update_cli::UpdateAction;

        assert_eq!(
            update_action(&["update", "rollback"]),
            UpdateAction::Rollback { yes: false }
        );
        assert_eq!(
            update_action(&["update", "rollback", "--yes"]),
            UpdateAction::Rollback { yes: true }
        );
        assert_eq!(
            update_action(&["update", "rollback", "-y"]),
            UpdateAction::Rollback { yes: true }
        );
    }

    #[test]
    fn update_needs_an_action() {
        for args in [
            &["infrarust", "update"][..],
            &["infrarust", "update", "apply"][..],
            &["infrarust", "update", "check", "--yes"][..],
        ] {
            assert!(Cli::try_parse_from(args).is_err(), "{args:?}");
        }
    }

    #[test]
    fn global_options_before_update_are_kept() {
        let cli = parse(&["-c", "/etc/infrarust.toml", "update", "check"]);
        assert_eq!(cli.config, Path::new("/etc/infrarust.toml"));
        assert!(matches!(cli.command, Some(Command::Update { .. })));
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
