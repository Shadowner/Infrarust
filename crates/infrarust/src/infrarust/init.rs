use std::{io, sync::Arc, time::Duration};

use infrarust_config::{
    InfrarustConfig,
    models::{
        logging::LogType,
        manager::{CraftyControllerManagerConfig, ManagerConfig, PterodactylManagerConfig},
    },
    provider::{docker::DockerProvider, file::FileProvider},
};
use infrarust_server_manager::{CraftyClient, LocalProvider, PterodactylClient};
use tracing::{Instrument, Span, debug, debug_span, error, info, warn};

use crate::{
    core::{
        actors::supervisor::ActorSupervisor,
        config::provider::ConfigProvider,
        config::service::ConfigurationService,
        shared_component::SharedComponent,
    },
    security::{
        filter::FilterRegistry, RateLimiter,
        ban_system_adapter::BanSystemAdapter,
    },
    server::{gateway::Gateway, manager::Manager},
    Infrarust,
    cli::ShutdownController,
};

impl Infrarust {
    pub fn new(
        config: InfrarustConfig,
        shutdown_controller: Arc<ShutdownController>,
    ) -> io::Result<Self> {
        let span = debug_span!("infrarust_init", log_type = LogType::Supervisor.as_str());
        let _enter = span.enter();
        let config = Arc::new(config);

        debug!(
            log_type = LogType::Supervisor.as_str(),
            "Initializing Infrarust server with config: {:?}", config
        );
        let config_service = Arc::new(ConfigurationService::new());

        let (gateway_sender, gateway_receiver) = tokio::sync::mpsc::channel(100);
        let (provider_sender, provider_receiver) = tokio::sync::mpsc::channel(100);

        // Initialize filter registry
        let filter_registry = Arc::new(FilterRegistry::new());

        let mut config_provider = ConfigProvider::new(
            config_service.clone(),
            provider_receiver,
            provider_sender.clone(),
        );

        let manager_config = config.managers_config.clone().unwrap_or(ManagerConfig {
            pterodactyl: None,
            crafty: None,
        });

        let pterodactyl_config = match manager_config.pterodactyl {
            Some(ref config) => config.clone(),
            None => {
                warn!(
                    log_type = LogType::Supervisor.as_str(),
                    "Pterodactyl manager configuration is missing and will be disabled"
                );
                PterodactylManagerConfig {
                    enabled: false,
                    api_key: String::new(),
                    base_url: String::new(),
                }
            }
        };

        let crafty_config = match manager_config.crafty {
            Some(ref config) => config.clone(),
            None => {
                warn!(
                    log_type = LogType::Supervisor.as_str(),
                    "Crafty Controller manager configuration is missing and will be disabled"
                );
                CraftyControllerManagerConfig {
                    enabled: false,
                    api_key: String::new(),
                    base_url: String::new(),
                }
            }
        };

        debug!(
            log_type = LogType::ServerManager.as_str(),
            "Pterodactyl manager configuration: enabled = {}, api_key = {}, base_url = {}",
            pterodactyl_config.enabled,
            pterodactyl_config.api_key,
            pterodactyl_config.base_url
        );

        debug!(
            log_type = LogType::ServerManager.as_str(),
            "Crafty Controller manager configuration: enabled = {}, api_key = {}, base_url = {}",
            crafty_config.enabled,
            crafty_config.api_key,
            crafty_config.base_url
        );

        let pterodactyl_provider =
            PterodactylClient::new(pterodactyl_config.api_key, pterodactyl_config.base_url);
        let local_provider = LocalProvider::new();
        let crafty_provider = CraftyClient::new(crafty_config.api_key, crafty_config.base_url);

        let managers = Arc::new(Manager::new(
            pterodactyl_provider,
            local_provider,
            crafty_provider,
        ));

        if ActorSupervisor::initialize_global(Some(managers.clone())).is_err() {
            error!(
                log_type = LogType::Supervisor.as_str(),
                "Failed to initialize ActorSupervisor"
            );
            return Err(io::Error::other("Failed to initialize ActorSupervisor"));
        }
        let supervisor = ActorSupervisor::global();

        let shared = Arc::new(SharedComponent::new(
            config,
            supervisor.clone(),
            config_service.clone(),
            filter_registry,
            shutdown_controller,
            gateway_sender,
            provider_sender,
            managers.clone(),
        ));

        // Inject dependencies that couldn't be set during construction due to circular references
        futures::executor::block_on(async {
            config_service.set_server_managers(managers.clone()).await;
            supervisor.set_configuration_service(config_service).await;
        });

        let server_gateway = Arc::new(Gateway::new(shared.clone()));
        if let Some(file_config) = shared.config().file_provider.clone() {
            let file_provider = FileProvider::new(
                file_config.proxies_path,
                file_config.file_type,
                file_config.watch,
                shared.provider_sender().clone(),
            );

            config_provider.register_provider(Box::new(file_provider));
        }

        if let Some(docker_config) = shared.config().docker_provider.clone() {
            let docker_provider = Box::new(DockerProvider::new(
                docker_config,
                shared.provider_sender().clone(),
            ));
            config_provider.register_provider(docker_provider);
            info!("Docker provider registered");
        }

        let provider_span = Span::current();
        tokio::spawn(async move {
            debug!(
                log_type = LogType::ConfigProvider.as_str(),
                "Starting ConfigProvider"
            );
            config_provider.run().instrument(provider_span).await;
        });

        let guard = server_gateway.clone();
        let gateway_span = Span::current();
        tokio::spawn(async move {
            debug!(log_type = LogType::Supervisor.as_str(), "Starting Gateway");
            guard
                .clone()
                .run(gateway_receiver)
                .instrument(gateway_span)
                .await;
        });
        let shared_clone = shared.clone();
        let registry_clone = shared_clone.filter_registry_arc();

        tokio::spawn(async move {
            let config_clone = shared_clone.config();
            if let Some(filter_config) = &config_clone.filters {
                if let Some(rate_config) = &filter_config.rate_limiter {
                    let rate_limiter = RateLimiter::new(
                        "global_rate_limiter",
                        rate_config.burst_size,
                        Duration::from_secs(rate_config.window_seconds),
                    );

                    if let Err(e) = registry_clone.register(rate_limiter).await {
                        debug!(
                            log_type = LogType::Filter.as_str(),
                            "Failed to register rate limiter: {}", e
                        );
                    }
                }

                if filter_config.ban.enabled {
                    if let Some(file_path) = &filter_config.ban.file_path {
                        match BanSystemAdapter::new("global_ban_system", file_path.clone()).await {
                            Ok(ban_filter) => {
                                if let Err(e) = registry_clone.register(ban_filter).await {
                                    debug!(
                                        log_type = LogType::BanSystem.as_str(),
                                        "Failed to register ban filter: {}", e
                                    );
                                }
                            }
                            Err(e) => {
                                error!(
                                    log_type = LogType::BanSystem.as_str(),
                                    "Failed to create ban system adapter: {}", e
                                );
                            }
                        }
                    } else {
                        warn!(
                            log_type = LogType::BanSystem.as_str(),
                            "Ban system enabled but no file path configured"
                        );
                    }
                }
            }
        });

        // Initialize system metrics collection if telemetry is enabled
        #[cfg(feature = "telemetry")]
        crate::telemetry::start_system_metrics_collection();

        Ok(Self {
            shared,
            gateway: server_gateway,
        })
    }
}
