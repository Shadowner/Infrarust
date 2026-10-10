use std::sync::Arc;
use std::time::Duration;

use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use infrarust_api::events::proxy::ServerStateChangeEvent;
use infrarust_api::types::ServerId;
use infrarust_config::{ForwardingMode as ConfigForwardingMode, ProxyConfig};
use infrarust_protocol::build_default_registry;
use infrarust_server_manager::ServerManagerService;
use infrarust_transport::BackendConnector;

use super::ProxyServer;
use crate::auth::mojang::MojangAuth;
use crate::ban::manager::BanManager;
use crate::error::CoreError;
use crate::event_bus::conversion::convert_server_state;
use crate::event_bus::{EventBusConfig, EventBusImpl};
use crate::forwarding::ForwardingMode;
use crate::forwarding::secret::load_or_generate_secret;
use crate::handler::InterceptedHandler;
use crate::handler::legacy::LegacyHandler;
use crate::handler::passthrough::PassthroughHandler;
use crate::middleware::backend_selection::BackendSelectionMiddleware;
use crate::middleware::ban_check::BanCheckMiddleware;
use crate::middleware::ban_ip_check::BanIpCheckMiddleware;
use crate::middleware::handshake_parser::HandshakeParserMiddleware;
use crate::middleware::ip_filter::IpFilterMiddleware;
use crate::middleware::login_start_parser::LoginStartParserMiddleware;
use crate::middleware::rate_limiter::RateLimiterMiddleware;
use crate::middleware::routing_middleware::DomainRouterMiddleware;
use crate::middleware::telemetry::TelemetryMiddleware;
use crate::pipeline::Pipeline;
use crate::player::registry::PlayerRegistryImpl;
use crate::provider::file::FileProvider;
use crate::provider::registry::ProviderRegistry;
use crate::routing::DomainRouter;
use crate::services::ProxyServices;
use crate::services::command_manager::CommandManagerImpl;
use crate::session::connection_registry::ConnectionRegistry;
use crate::status::{FaviconCache, StatusCache, StatusHandler, StatusRelayClient};

impl ProxyServer {
    #[allow(clippy::too_many_lines)]
    pub async fn new(
        config: ProxyConfig,
        config_path: std::path::PathBuf,
        shutdown: CancellationToken,
    ) -> Result<Self, CoreError> {
        let sessions = CancellationToken::new();
        let background = CancellationToken::new();

        let domain_router = Arc::new(DomainRouter::new());

        let packet_registry = Arc::new(build_default_registry());

        let event_bus = Arc::new(EventBusImpl::with_config(EventBusConfig::from(
            &config.events,
        )));
        event_bus.start_dispatcher();

        #[cfg(feature = "telemetry")]
        let proxy_metrics = Arc::new(crate::telemetry::ProxyMetrics::new());

        let backend_health = crate::loadbalancer::PassiveBackendHealth::new();
        #[cfg(feature = "telemetry")]
        let backend_health = backend_health.with_listener(Arc::new(
            crate::loadbalancer::HealthTransitionMetrics(Arc::clone(&proxy_metrics)),
        ));
        let backend_health = Arc::new(backend_health);
        backend_health.add_listener(Arc::new(crate::loadbalancer::HealthTransitionEvents::new(
            Arc::clone(&domain_router),
            Arc::clone(&event_bus),
        )));

        let backend_observer = Arc::new(crate::loadbalancer::BackendAttemptObserver::new(
            Arc::clone(&backend_health),
            #[cfg(feature = "telemetry")]
            Arc::clone(&proxy_metrics),
        ));
        let backend_connector = Arc::new(
            BackendConnector::new(config.connect_timeout, config.keepalive.clone())
                .with_max_attempts(config.connect_max_attempts)
                .with_observer(backend_observer as _),
        );
        let registry = Arc::new(ConnectionRegistry::new());
        let backend_load = Arc::new(crate::loadbalancer::BackendLoad::new());
        let pending_backends = Arc::new(crate::loadbalancer::PendingRegistry::new(
            Arc::clone(&backend_load) as _,
            crate::loadbalancer::reservation_ttl(
                config.connect_timeout,
                config.connect_max_attempts,
            ),
        ));

        let status_cache = Arc::new(StatusCache::new(config.status_cache.ttl));
        let favicon_cache =
            Arc::new(FaviconCache::load_from_configs(&[], config.default_motd.as_ref()).await?);

        let mut provider_registry = ProviderRegistry::new(
            Arc::clone(&domain_router),
            Arc::clone(&event_bus),
            Arc::clone(&status_cache),
            Arc::clone(&favicon_cache),
            background.clone(),
            config
                .forwarding
                .as_ref()
                .map(|forwarding| forwarding.mode.clone())
                .unwrap_or_default(),
        );

        provider_registry.add_provider(Box::new(FileProvider::new(config.servers_dir.clone())));

        #[cfg(feature = "docker")]
        if let Some(ref docker_config) = config.docker {
            provider_registry.add_provider(Box::new(crate::provider::docker::DockerProvider::new(
                docker_config,
            )));
        }

        #[cfg(not(feature = "docker"))]
        if config.docker.is_some() {
            tracing::warn!(
                "docker configuration found but docker feature is not enabled, ignoring"
            );
        }

        let (provider_handle, provider_event_sender) = provider_registry.start().await?;
        tokio::spawn(async move {
            if let Err(e) = provider_handle.await
                && e.is_panic()
            {
                tracing::error!(
                    error = %e,
                    "provider event loop panicked, configuration hot-reload is no longer active"
                );
            }
        });

        let managed_configs: Vec<(String, infrarust_config::ServerManagerConfig)> = domain_router
            .list_all()
            .iter()
            .filter_map(|(_pid, c)| {
                c.server_manager
                    .as_ref()
                    .map(|sm| (c.effective_id(), sm.clone()))
            })
            .collect();

        let server_manager = if managed_configs.is_empty() {
            None
        } else {
            let http_client = reqwest::Client::new();
            let service = ServerManagerService::new(&managed_configs, http_client);

            let bus = Arc::clone(&event_bus);
            service.add_on_state_change(Arc::new(move |server_id, old, new| {
                let api_old = convert_server_state(old);
                let api_new = convert_server_state(new);
                bus.post(ServerStateChangeEvent::new(
                    ServerId::new(server_id),
                    api_old,
                    api_new,
                ));
            }));

            tracing::info!(count = managed_configs.len(), "server manager initialized");
            Some(Arc::new(service))
        };

        let favicon_configs: Vec<(String, Arc<infrarust_config::ServerConfig>)> = domain_router
            .list_all()
            .into_iter()
            .map(|(_pid, cfg)| (cfg.effective_id(), cfg))
            .collect();
        if let Err(e) = favicon_cache
            .reload(&favicon_configs, config.default_motd.as_ref())
            .await
        {
            tracing::warn!(error = %e, "failed to load initial favicons");
        }

        let relay_client = StatusRelayClient::new(
            Arc::clone(&backend_connector),
            Arc::clone(&packet_registry),
            Duration::from_secs(5),
        );

        let status_handler = StatusHandler::new(
            relay_client,
            Arc::clone(&status_cache),
            Arc::clone(&favicon_cache),
            server_manager.as_ref().map(Arc::clone),
            Arc::clone(&packet_registry),
            config.default_motd.clone(),
            Arc::clone(&event_bus),
            Arc::clone(&backend_load) as _,
            Arc::clone(&backend_health) as _,
        );

        #[cfg(feature = "telemetry")]
        let status_handler = status_handler.with_metrics(Arc::clone(&proxy_metrics));

        let ban_manager = Arc::new(
            BanManager::from_config(&config.ban, Arc::clone(&registry), Arc::clone(&event_bus))
                .await?,
        );

        let player_registry = Arc::new(PlayerRegistryImpl::new(Arc::clone(&registry)));
        let command_manager = Arc::new(CommandManagerImpl::new());

        let codec_filter_registry =
            Arc::new(crate::filter::codec_registry::CodecFilterRegistryImpl::new());

        let limbo_handler_registry = Arc::new(crate::limbo::registry::LimboHandlerRegistry::new());

        let forwarding_mode = Arc::new(resolve_forwarding_mode(&config));
        let forwarding_secret = load_forwarding_secret(&config, &forwarding_mode);

        let permission_service =
            Arc::new(crate::permissions::PermissionService::new(&config.permissions).await);

        let services = ProxyServices {
            event_bus: Arc::clone(&event_bus),
            player_registry,
            command_manager,
            connection_registry: Arc::clone(&registry),
            backend_load: Arc::clone(&backend_load),
            pending_backends: Arc::clone(&pending_backends),
            packet_registry: Arc::clone(&packet_registry),
            server_manager: server_manager.clone(),
            ban_manager: Arc::clone(&ban_manager),
            config: Arc::new(config.clone()),
            config_path,
            domain_router: Arc::clone(&domain_router),
            backend_health: Arc::clone(&backend_health) as _,
            load_balancer_service: Arc::new(
                crate::services::load_balancer_service::LoadBalancerServiceImpl::new(
                    Arc::clone(&domain_router),
                    Arc::clone(&backend_health),
                    Arc::clone(&backend_load) as _,
                ),
            ),
            codec_filter_registry: Arc::clone(&codec_filter_registry),
            transport_filter_registry: Arc::new(
                crate::filter::transport_registry::TransportFilterRegistryImpl::new(),
            ),
            limbo_handler_registry,
            registry_codec_cache: Arc::new(crate::limbo::registry_cache::RegistryCodecCache::new(
                Arc::new(crate::registry_data::embedded::EmbeddedRegistryDataProvider),
            )),
            provider_event_sender,
            forwarding_mode,
            forwarding_secret,
            permission_service,
            plugin_messaging: Arc::new(crate::plugin_messaging::PluginMessaging::new(
                &config.plugin_messaging,
            )),
        };

        let mut common_pipeline = Pipeline::new();
        common_pipeline.add(Box::new(IpFilterMiddleware::new(config.ip_filter.clone())));
        common_pipeline.add(Box::new(HandshakeParserMiddleware::new()));
        common_pipeline.add(Box::new(BanIpCheckMiddleware::new(Arc::clone(
            &ban_manager,
        ))));
        common_pipeline.add(Box::new(RateLimiterMiddleware::new(&config.rate_limit)));
        common_pipeline.add(Box::new(DomainRouterMiddleware::new(Arc::clone(
            &domain_router,
        ))));

        let mut login_pipeline = Pipeline::new();
        login_pipeline.add(Box::new(LoginStartParserMiddleware::new()));
        login_pipeline.add(Box::new(BanCheckMiddleware::new(Arc::clone(&ban_manager))));
        login_pipeline.add(Box::new(TelemetryMiddleware));
        login_pipeline.add(Box::new(
            BackendSelectionMiddleware::new(
                Arc::clone(&backend_load) as _,
                Arc::clone(&backend_health) as _,
            )
            .with_pending(Arc::clone(&pending_backends)),
        ));

        let legacy_handler = LegacyHandler::new(
            services.clone(),
            Arc::clone(&backend_connector),
            sessions.clone(),
        );

        let passthrough_handler =
            PassthroughHandler::new(Arc::clone(&backend_connector), services.clone());
        #[cfg(feature = "telemetry")]
        let passthrough_handler = passthrough_handler.with_metrics(Arc::clone(&proxy_metrics));

        let auth = Arc::new(MojangAuth::with_session_url(
            config.auth.session_url.clone(),
        )?);

        let offline_handler = InterceptedHandler::offline(
            Arc::clone(&backend_connector),
            services.clone(),
            Some(Arc::clone(&auth)),
        );
        #[cfg(feature = "telemetry")]
        let offline_handler = offline_handler.with_metrics(Arc::clone(&proxy_metrics));

        let client_only_handler =
            InterceptedHandler::client_only(Arc::clone(&backend_connector), services.clone(), auth);
        #[cfg(feature = "telemetry")]
        let client_only_handler = client_only_handler.with_metrics(Arc::clone(&proxy_metrics));

        Ok(Self {
            common_pipeline,
            login_pipeline,
            status_handler,
            legacy_handler,
            passthrough_handler,
            offline_handler,
            client_only_handler,
            services,
            backend_health,
            unknown_domain_behavior: config.unknown_domain_behavior,
            shutdown,
            sessions,
            background,
            connections: TaskTracker::new(),
        })
    }
}

fn load_forwarding_secret(config: &ProxyConfig, mode: &ForwardingMode) -> Option<Arc<[u8]>> {
    match mode {
        ForwardingMode::Velocity { secret } => {
            return Some(secret.as_slice().into());
        }
        ForwardingMode::BungeeGuard { token } => {
            return Some(token.as_bytes().into());
        }
        _ => {}
    }

    let secret_path = config
        .forwarding
        .as_ref()
        .map(|f| f.secret_file.clone())
        .unwrap_or_else(|| std::path::PathBuf::from("forwarding.secret"));

    if secret_path.exists() {
        match load_or_generate_secret(&secret_path) {
            Ok(secret) => {
                tracing::info!(
                    path = %secret_path.display(),
                    "loaded forwarding secret for Velocity auto-detection"
                );
                Some(secret.into())
            }
            Err(e) => {
                tracing::debug!(
                    path = %secret_path.display(),
                    error = %e,
                    "could not load forwarding secret — Velocity auto-detection disabled"
                );
                None
            }
        }
    } else {
        None
    }
}

fn resolve_forwarding_mode(config: &ProxyConfig) -> ForwardingMode {
    let fwd_config = match &config.forwarding {
        Some(c) => c,
        None => return ForwardingMode::None,
    };

    match &fwd_config.mode {
        ConfigForwardingMode::None => ForwardingMode::None,
        ConfigForwardingMode::BungeeCord => {
            tracing::warn!(
                "BungeeCord legacy forwarding is configured globally. \
                 This mode is fundamentally insecure — anyone reaching the backend \
                 directly can impersonate any player. Consider migrating to Velocity \
                 modern forwarding."
            );
            ForwardingMode::BungeeCord
        }
        ConfigForwardingMode::BungeeGuard => {
            match load_or_generate_secret(&fwd_config.secret_file) {
                Ok(secret) => {
                    let token = String::from_utf8_lossy(&secret).to_string();
                    ForwardingMode::BungeeGuard { token }
                }
                Err(e) => {
                    tracing::error!(
                        path = %fwd_config.secret_file.display(),
                        error = %e,
                        "failed to load BungeeGuard secret, falling back to no forwarding"
                    );
                    ForwardingMode::None
                }
            }
        }
        ConfigForwardingMode::Velocity => match load_or_generate_secret(&fwd_config.secret_file) {
            Ok(secret) => ForwardingMode::Velocity { secret },
            Err(e) => {
                tracing::error!(
                    path = %fwd_config.secret_file.display(),
                    error = %e,
                    "failed to load Velocity secret, falling back to no forwarding"
                );
                ForwardingMode::None
            }
        },
    }
}
