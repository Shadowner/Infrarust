use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::error::CoreError;
use crate::pipeline::context::ConnectionContext;
use crate::pipeline::middleware::{Middleware, MiddlewareResult};
use crate::pipeline::types::{HandshakeData, RoutingData};
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
    ) -> Pin<Box<dyn Future<Output = Result<MiddlewareResult, CoreError>> + Send + 'a>> {
        Box::pin(async move {
            let handshake = ctx.require_extension::<HandshakeData>("HandshakeData")?;

            let domain = &handshake.domain;

            // Resolve domain to server config
            let Some((_provider_id, server_config)) = self.domain_router.resolve(domain) else {
                tracing::debug!(domain, "no server found for domain");
                return Ok(MiddlewareResult::Reject(format!(
                    "Unknown server: {domain}"
                )));
            };
            let config_id = server_config.effective_id();

            // Check per-server IP filter
            if let Some(ref ip_filter) = server_config.ip_filter
                && !ip_filter.is_allowed(&ctx.client_ip)
            {
                tracing::debug!(
                    ip = %ctx.client_ip,
                    server = %config_id,
                    "ip blocked by server filter"
                );
                return Ok(MiddlewareResult::Reject(format!(
                    "IP {} is not allowed on this server",
                    ctx.client_ip
                )));
            }

            tracing::debug!(domain, config_id, "domain routed");

            ctx.extensions.insert(RoutingData {
                server_config,
                config_id,
            });

            Ok(MiddlewareResult::Continue)
        })
    }
}
