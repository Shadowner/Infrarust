use std::future::Future;
use std::pin::Pin;

use infrarust_config::IpFilterConfig;

use crate::error::CoreError;
use crate::pipeline::context::ConnectionContext;
use crate::pipeline::middleware::{Middleware, MiddlewareResult};

/// Middleware that checks client IP against a global whitelist/blacklist.
pub struct IpFilterMiddleware {
    global_filter: Option<IpFilterConfig>,
}

impl IpFilterMiddleware {
    /// Creates a new IP filter middleware with an optional global filter config.
    pub const fn new(global_filter: Option<IpFilterConfig>) -> Self {
        Self { global_filter }
    }
}

impl Middleware for IpFilterMiddleware {
    fn name(&self) -> &'static str {
        "ip_filter"
    }

    fn process<'a>(
        &'a self,
        ctx: &'a mut ConnectionContext,
    ) -> Pin<Box<dyn Future<Output = Result<MiddlewareResult, CoreError>> + Send + 'a>> {
        Box::pin(async move {
            if let Some(ref filter) = self.global_filter
                && !filter.is_allowed(&ctx.client_ip)
            {
                tracing::debug!(ip = %ctx.client_ip, "ip blocked by global filter");
                return Ok(MiddlewareResult::Reject(format!(
                    "IP {} is not allowed",
                    ctx.client_ip
                )));
            }
            Ok(MiddlewareResult::Continue)
        })
    }
}
