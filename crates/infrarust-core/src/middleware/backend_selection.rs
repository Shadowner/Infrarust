//! Middleware that orders a server's backend addresses for the proxy handler.

use std::sync::Arc;

use infrarust_api::event::BoxFuture;

use crate::error::CoreError;
use crate::loadbalancer::{
    AddressConnectionCount, BackendHealthView, PendingRegistry, select_backend_addresses,
};
use crate::pipeline::context::ConnectionContext;
use crate::pipeline::middleware::{Middleware, MiddlewareResult};
use crate::pipeline::types::BackendTargets;

pub struct BackendSelectionMiddleware {
    registry: Arc<dyn AddressConnectionCount>,
    health: Arc<dyn BackendHealthView>,
    pending: Option<Arc<PendingRegistry>>,
}

impl BackendSelectionMiddleware {
    pub fn new(
        registry: Arc<dyn AddressConnectionCount>,
        health: Arc<dyn BackendHealthView>,
    ) -> Self {
        Self {
            registry,
            health,
            pending: None,
        }
    }

    #[must_use]
    pub fn with_pending(mut self, pending: Arc<PendingRegistry>) -> Self {
        self.registry = Arc::clone(&pending) as _;
        self.pending = Some(pending);
        self
    }
}

impl Middleware for BackendSelectionMiddleware {
    fn name(&self) -> &'static str {
        "backend_selection"
    }

    fn process<'a>(
        &'a self,
        ctx: &'a mut ConnectionContext,
    ) -> BoxFuture<'a, Result<MiddlewareResult, CoreError>> {
        Box::pin(async move {
            let Some(routing) = ctx.routing.as_ref() else {
                return Ok(MiddlewareResult::Continue); // status ping, etc.
            };

            // Selection through the strategy carried by the resolved config.
            let targets = BackendTargets {
                addresses: select_backend_addresses(
                    &routing.server_config,
                    routing.load_balancer.as_ref(),
                    self.registry.as_ref(),
                    self.health.as_ref(),
                ),
            };

            tracing::trace!(
                server = %routing.config_id,
                strategy = routing.load_balancer.name(),
                order = ?targets.addresses,
                "backend selection"
            );

            if let (Some(pending), Some(picked)) = (&self.pending, targets.addresses.first()) {
                ctx.pending_ticket = Some(pending.reserve(picked));
            }
            ctx.backend_targets = Some(targets);
            Ok(MiddlewareResult::Continue)
        })
    }
}
