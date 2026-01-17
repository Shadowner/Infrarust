use std::sync::Arc;

use infrarust_config::provider::ProviderMessage;
use tokio::sync::mpsc::Sender;

use crate::{FilterRegistry, InfrarustConfig, cli::ShutdownController, server::manager::Manager};

use super::{
    actors::supervisor::ActorSupervisor, config::service::ConfigurationService,
    event::GatewayMessage,
};

#[derive(Debug)]
pub struct SharedComponent {
    config: Arc<InfrarustConfig>,

    actor_supervisor: Arc<ActorSupervisor>,
    configuration_service: Arc<ConfigurationService>,
    filter_registry: Arc<FilterRegistry>,
    server_managers: Arc<Manager>,

    shutdown_controller: Arc<ShutdownController>,

    _gateway_sender: Sender<GatewayMessage>,
    provider_sender: Sender<ProviderMessage>,
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

    pub(crate) fn server_managers_arc(&self) -> Arc<Manager> {
        Arc::clone(&self.server_managers)
    }
}
