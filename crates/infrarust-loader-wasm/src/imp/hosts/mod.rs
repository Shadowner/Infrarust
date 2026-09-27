macro_rules! gate {
    ($interface:literal, $function:literal) => {
        const { $crate::hosts::Gate::new($interface, $function) }
    };
}

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

pub(crate) use log::enabled_level;

use std::fmt;
use std::future::Future;
use std::sync::Arc;

use infrarust_api::error::ServiceError;
use infrarust_api::permissions::Capability;
use infrarust_api::player::Player;
use infrarust_api::plugin::PluginContext;
use infrarust_api::types::{Component, PlayerId};
use infrarust_plugin_common::capability::gates::required;

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

#[derive(Debug, Clone, Copy)]
pub(crate) struct Gate {
    interface: &'static str,
    function: &'static str,
    required: &'static [Capability],
}

impl Gate {
    pub(crate) const fn new(interface: &'static str, function: &'static str) -> Self {
        Self {
            interface,
            function,
            required: required(interface, function),
        }
    }
}

impl PluginStoreState {
    pub(crate) fn service_call_limit(&self) -> HostCallLimit {
        self.host_call_limit(self.host_call_timeout())
    }

    pub(crate) fn check(&mut self, gate: Gate) -> HostResult<()> {
        self.check_each(
            gate.required,
            format_args!("{}.{}", gate.interface, gate.function),
        )
    }

    pub(crate) fn check_each(
        &mut self,
        capabilities: &[Capability],
        call: fmt::Arguments<'_>,
    ) -> HostResult<()> {
        let Some(missing) = capabilities
            .iter()
            .copied()
            .find(|capability| !self.capabilities().has(*capability))
        else {
            return Ok(());
        };
        self.report_denied(missing, call);
        Err(missing_capability(missing))
    }

    pub(crate) fn lacks(&mut self, gate: Gate) -> bool {
        self.check(gate).is_err()
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
