mod bans;
mod codec;
mod commands;
mod config;
mod event_bus;
mod limbo;
mod load_balancer;
mod log;
mod messaging;
mod permissions;
mod players;
mod plugin_registry;
mod providers;
mod proxy_info;
mod scheduler;
mod servers;
mod text;

#[cfg(test)]
mod tests;

use std::future::Future;
use std::sync::Arc;

use infrarust_api::error::ServiceError;
use infrarust_api::permissions::Capability;
use infrarust_api::player::Player;
use infrarust_api::plugin::PluginContext;
use infrarust_api::types::{Component, PlayerId};

use crate::bindings::infrarust::plugin::types as wt;
use crate::component;
use crate::deadline::HostCallLimit;
use crate::host_error::{
    HostResult, invalid_component, missing_capability, no_services, player_gone, service_error,
    timed_out,
};
use crate::store_state::PluginStoreState;

pub(crate) async fn await_service<T>(
    limit: HostCallLimit,
    fut: impl Future<Output = Result<T, ServiceError>> + Send,
) -> HostResult<T> {
    match limit.run(fut).await {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(e)) => Err(service_error(e)),
        Err(expired) => Err(timed_out(expired)),
    }
}

pub(crate) fn parse_text(component: &wt::Component) -> HostResult<Component> {
    component::from_wit(component).map_err(|e| invalid_component(&e))
}

impl PluginStoreState {
    pub(crate) fn service_call_limit(&self) -> HostCallLimit {
        self.host_call_limit(self.host_call_timeout())
    }

    pub(crate) fn lacks(&mut self, capability: Capability, call: &'static str) -> bool {
        if self.capabilities().has(capability) {
            return false;
        }
        self.report_denied(capability, call);
        true
    }

    pub(crate) fn check(&mut self, capability: Capability, call: &'static str) -> HostResult<()> {
        if self.lacks(capability, call) {
            Err(missing_capability(capability))
        } else {
            Ok(())
        }
    }

    pub(crate) fn services(&self) -> HostResult<Arc<dyn PluginContext>> {
        self.ctx().cloned().ok_or_else(no_services)
    }

    pub(crate) fn online_player(&self, id: u64) -> HostResult<Arc<dyn Player>> {
        self.services()?
            .player_registry()
            .get_player_by_id(PlayerId::new(id))
            .ok_or_else(|| player_gone(id))
    }
}
