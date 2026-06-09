//! Middleware that orders a server's backend addresses for the proxy handler.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use smallvec::SmallVec;

use infrarust_config::ServerAddress;

use crate::error::CoreError;
use crate::loadbalancer::{AddressConnectionCount, BackendCandidate, BackendHealthView};
use crate::pipeline::context::ConnectionContext;
use crate::pipeline::middleware::{Middleware, MiddlewareResult};
use crate::pipeline::types::RoutingData;

#[derive(Debug, Clone)]
pub struct BackendTargets {
    pub addresses: SmallVec<[ServerAddress; 4]>,
}

impl BackendTargets {
    pub fn single(addr: ServerAddress) -> Self {
        Self {
            addresses: smallvec::smallvec![addr],
        }
    }

    pub fn from_ordered(ordered: &[&BackendCandidate]) -> Self {
        Self {
            addresses: ordered.iter().map(|c| c.address.clone()).collect(),
        }
    }
}

pub struct BackendSelectionMiddleware {
    registry: Arc<dyn AddressConnectionCount>,
    health: Arc<dyn BackendHealthView>,
}

impl BackendSelectionMiddleware {
    pub fn new(
        registry: Arc<dyn AddressConnectionCount>,
        health: Arc<dyn BackendHealthView>,
    ) -> Self {
        Self { registry, health }
    }
}

impl Middleware for BackendSelectionMiddleware {
    fn name(&self) -> &'static str {
        "backend_selection"
    }

    fn process<'a>(
        &'a self,
        ctx: &'a mut ConnectionContext,
    ) -> Pin<Box<dyn Future<Output = Result<MiddlewareResult, CoreError>> + Send + 'a>> {
        Box::pin(async move {
            let Some(routing) = ctx.extensions.get::<RoutingData>() else {
                return Ok(MiddlewareResult::Continue); // status ping, etc.
            };
            let server = &routing.server_config;

            // Shortcut: a single address -> nothing to balance.
            if let [only] = server.addresses.as_slice() {
                let targets = BackendTargets::single(only.address.clone());
                ctx.extensions.insert(targets);
                return Ok(MiddlewareResult::Continue);
            }

            // Snapshot: config (weights) x connection registry x health view.
            let candidates: Vec<BackendCandidate> = server
                .addresses
                .iter()
                .map(|wa| {
                    let (healthy, healthy_since) = self.health.snapshot(&wa.address);
                    BackendCandidate {
                        address: wa.address.clone(),
                        weight: wa.weight,
                        active_connections: self
                            .registry
                            .active_connections_for_address(&wa.address),
                        healthy,
                        healthy_since,
                    }
                })
                .collect();

            // Selection through the strategy carried by the resolved config.
            let ordered = routing.load_balancer.order(&candidates);
            let targets = BackendTargets::from_ordered(&ordered);

            tracing::trace!(
                server = %routing.config_id,
                strategy = routing.load_balancer.name(),
                order = ?targets.addresses,
                "backend selection"
            );

            ctx.extensions.insert(targets);
            Ok(MiddlewareResult::Continue)
        })
    }
}
