use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use arc_swap::ArcSwap;

use infrarust_config::{DomainIndex, ServerConfig};

use crate::error::CoreError;
use crate::pipeline::context::ConnectionContext;
use crate::pipeline::middleware::{Middleware, MiddlewareResult};
use crate::pipeline::types::{HandshakeData, RoutingData};

/// Middleware that resolves the target server from the handshake domain.
///
/// Uses `ArcSwap` for lock-free hot-reloadable domain index lookups.
pub struct DomainRouterMiddleware {
    domain_index: Arc<ArcSwap<DomainIndex>>,
    configs: Arc<ArcSwap<HashMap<String, Arc<ServerConfig>>>>,
}

impl DomainRouterMiddleware {
    /// Creates a new domain router with shared hot-reloadable state.
    pub const fn new(
        domain_index: Arc<ArcSwap<DomainIndex>>,
        configs: Arc<ArcSwap<HashMap<String, Arc<ServerConfig>>>>,
    ) -> Self {
        Self {
            domain_index,
            configs,
        }
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

            // Resolve domain to config id
            let index = self.domain_index.load();
            let Some(id) = index.resolve(domain) else {
                tracing::debug!(domain, "no server found for domain");
                return Ok(MiddlewareResult::Reject(format!(
                    "Unknown server: {domain}"
                )));
            };
            let config_id = id.to_string();

            // Look up full config
            let configs = self.configs.load();
            let Some(cfg) = configs.get(&config_id) else {
                tracing::warn!(config_id, "config id resolved but config not found");
                return Ok(MiddlewareResult::Reject(format!(
                    "Unknown server: {domain}"
                )));
            };
            let server_config = Arc::clone(cfg);

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
