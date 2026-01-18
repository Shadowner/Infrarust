use std::sync::Arc;

use infrarust_config::provider::ProviderMessage;
use tokio::sync::{RwLock, mpsc::Sender};

use crate::{FilterRegistry, InfrarustConfig, cli::ShutdownController, server::manager::Manager};

use super::{
    actors::supervisor::ActorSupervisor, config::service::ConfigurationService,
    event::GatewayMessage,
};

use crate::server::gateway::Gateway;

pub struct SharedComponent {
    config: Arc<InfrarustConfig>,

    actor_supervisor: Arc<ActorSupervisor>,
    configuration_service: Arc<ConfigurationService>,
    filter_registry: Arc<FilterRegistry>,
    server_managers: Arc<Manager>,
    gateway: RwLock<Option<Arc<Gateway>>>,

    shutdown_controller: Arc<ShutdownController>,

    _gateway_sender: Sender<GatewayMessage>,
    provider_sender: Sender<ProviderMessage>,
}

impl std::fmt::Debug for SharedComponent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SharedComponent")
            .field("config", &self.config)
            .field("actor_supervisor", &self.actor_supervisor)
            .field("configuration_service", &self.configuration_service)
            .field("filter_registry", &self.filter_registry)
            .field("server_managers", &self.server_managers)
            .field("gateway", &"<Gateway>") // Skip to prevent circular reference
            .field("shutdown_controller", &self.shutdown_controller)
            .finish_non_exhaustive()
    }
}

#[allow(clippy::too_many_arguments)]
impl SharedComponent {
    pub fn new(
        config: Arc<InfrarustConfig>,
        actor_supervisor: Arc<ActorSupervisor>,
        configuration_service: Arc<ConfigurationService>,
        filter_registry: Arc<FilterRegistry>,
        shutdown_controller: Arc<ShutdownController>,
        gateway_sender: Sender<GatewayMessage>,
        provider_sender: Sender<ProviderMessage>,
        server_managers: Arc<Manager>,
    ) -> Self {
        Self {
            config,
            actor_supervisor,
            configuration_service,
            filter_registry,
            shutdown_controller,
            _gateway_sender: gateway_sender,
            provider_sender,
            server_managers,
            gateway: RwLock::new(None),
        }
    }

    pub(crate) fn config(&self) -> &InfrarustConfig {
        &self.config
    }

    pub(crate) fn actor_supervisor(&self) -> &ActorSupervisor {
        &self.actor_supervisor
    }

    pub(crate) fn actor_supervisor_arc(&self) -> Arc<ActorSupervisor> {
        Arc::clone(&self.actor_supervisor)
    }

    pub(crate) fn configuration_service(&self) -> &ConfigurationService {
        &self.configuration_service
    }

    pub(crate) fn configuration_service_arc(&self) -> Arc<ConfigurationService> {
        Arc::clone(&self.configuration_service)
    }

    pub(crate) fn filter_registry(&self) -> &FilterRegistry {
        &self.filter_registry
    }

    pub(crate) fn filter_registry_arc(&self) -> Arc<FilterRegistry> {
        Arc::clone(&self.filter_registry)
    }

    pub(crate) fn shutdown_controller(&self) -> &ShutdownController {
        &self.shutdown_controller
    }

    pub(crate) fn shutdown_controller_arc(&self) -> Arc<ShutdownController> {
        Arc::clone(&self.shutdown_controller)
    }

    pub(crate) fn _gateway_sender(&self) -> &Sender<GatewayMessage> {
        &self._gateway_sender
    }

    pub(crate) fn provider_sender(&self) -> &Sender<ProviderMessage> {
        &self.provider_sender
    }

    pub(crate) fn server_managers(&self) -> &Manager {
        &self.server_managers
    }

    pub async fn set_gateway(&self, gateway: Arc<Gateway>) {
        let mut lock = self.gateway.write().await;
        *lock = Some(gateway);
    }

    pub fn gateway(&self) -> Option<Arc<Gateway>> {
        self.gateway.try_read().ok()?.clone()
    }
}
