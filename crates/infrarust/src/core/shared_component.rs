use std::sync::Arc;

use tokio::sync::mpsc::Sender;

use crate::{FilterRegistry, InfrarustConfig, cli::ShutdownController};

use super::{
    actors::supervisor::ActorSupervisor,
    config::service::ConfigurationService,
    event::{GatewayMessage, ProviderMessage},
};

#[derive(Debug)]
pub struct SharedComponent {
    config: Arc<InfrarustConfig>,

    actor_supervisor: Arc<ActorSupervisor>,
    configuration_service: Arc<ConfigurationService>,
    filter_registry: Arc<FilterRegistry>,

    shutdown_controller: Arc<ShutdownController>,

    _gateway_sender: Sender<GatewayMessage>,
    provider_sender: Sender<ProviderMessage>,
}

impl SharedComponent {
    pub fn new(
        config: Arc<InfrarustConfig>,
        actor_supervisor: Arc<ActorSupervisor>,
        configuration_service: Arc<ConfigurationService>,
        filter_registry: Arc<FilterRegistry>,
        shutdown_controller: Arc<ShutdownController>,
        gateway_sender: Sender<GatewayMessage>,
        provider_sender: Sender<ProviderMessage>,
    ) -> Self {
        Self {
            config,
            actor_supervisor,
            configuration_service,
            filter_registry,
            shutdown_controller,
            _gateway_sender: gateway_sender,
            provider_sender,
        }
    }

    pub(crate) fn config(&self) -> &InfrarustConfig {
        &self.config
    }

    pub(crate) fn actor_supervisor(&self) -> Arc<ActorSupervisor> {
        self.actor_supervisor.clone()
    }

    pub(crate) fn configuration_service(&self) -> Arc<ConfigurationService> {
        self.configuration_service.clone()
    }

    pub(crate) fn filter_registry(&self) -> Arc<FilterRegistry> {
        self.filter_registry.clone()
    }

    pub(crate) fn shutdown_controller(&self) -> Arc<ShutdownController> {
        self.shutdown_controller.clone()
    }

    pub(crate) fn _gateway_sender(&self) -> &Sender<GatewayMessage> {
        &self._gateway_sender
    }

    pub(crate) fn provider_sender(&self) -> &Sender<ProviderMessage> {
        &self.provider_sender
    }
}
