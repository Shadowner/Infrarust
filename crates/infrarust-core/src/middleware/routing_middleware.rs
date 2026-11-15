use std::sync::Arc;

use infrarust_api::event::BoxFuture;
use infrarust_api::events::handshake::RejectReason;

use crate::error::CoreError;
use crate::pipeline::context::ConnectionContext;
use crate::pipeline::middleware::{Middleware, MiddlewareResult};
use crate::pipeline::types::{LegacyHello, RoutingData};
use crate::routing::DomainRouter;

/// Middleware that resolves the target server from the handshake domain.
///
/// Uses the `DomainRouter` for lock-free concurrent domain resolution
/// with incremental add/update/remove support.
///
/// **Requires**: `HandshakeData` (from `HandshakeParserMiddleware`)
/// **Inserts**: `RoutingData` (server config + config ID)
pub struct DomainRouterMiddleware {
    domain_router: Arc<DomainRouter>,
}

impl DomainRouterMiddleware {
    pub const fn new(domain_router: Arc<DomainRouter>) -> Self {
        Self { domain_router }
    }
}

impl Middleware for DomainRouterMiddleware {
    fn name(&self) -> &'static str {
        "domain_router"
    }

    fn process<'a>(
        &'a self,
        ctx: &'a mut ConnectionContext,
    ) -> BoxFuture<'a, Result<MiddlewareResult, CoreError>> {
        Box::pin(async move {
            let handshake = ctx.require_handshake()?;

            let domain = &handshake.domain;

            let hostless = ctx
                .legacy_hello
                .as_ref()
                .is_some_and(LegacyHello::is_hostless);
            let route = (!hostless)
                .then(|| self.domain_router.resolve_route(domain))
                .flatten();

            let Some((_provider_id, server_config, load_balancer)) = route else {
                tracing::debug!(domain, "no server found for domain");
                return Ok(MiddlewareResult::Reject {
                    reason: RejectReason::UnknownDomain,
                    message: format!("Unknown server: {domain}"),
                });
            };
            let config_id = server_config.effective_id();

            // Check per-server IP filter
            if let Some(ref ip_filter) = server_config.ip_filter
                && !ip_filter.is_allowed(&ctx.client_ip())
            {
                tracing::debug!(
                    ip = %ctx.client_ip(),
                    server = %config_id,
                    "ip blocked by server filter"
                );
                return Ok(MiddlewareResult::Reject {
                    reason: RejectReason::IpFilter,
                    message: format!("IP {} is not allowed on this server", ctx.client_ip()),
                });
            }

            tracing::debug!(domain, config_id, "domain routed");

            ctx.routing = Some(RoutingData {
                server_config,
                config_id,
                load_balancer,
            });

            Ok(MiddlewareResult::Continue)
        })
    }
}
