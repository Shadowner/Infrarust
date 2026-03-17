//! Middleware that rejects banned IPs early in the common pipeline.
//!
//! Unlike `BanCheckMiddleware` (login pipeline, checks IP + username),
//! this middleware runs before intent branching and blocks banned IPs
//! from even receiving the MOTD.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::ban::BanManager;
use crate::error::CoreError;
use crate::pipeline::context::ConnectionContext;
use crate::pipeline::middleware::{Middleware, MiddlewareResult};

/// Middleware that rejects banned IPs in the common pipeline.
///
/// Placed after `IpFilterMiddleware` in the common pipeline.
/// Blocks the connection before the handshake intent is even evaluated,
/// so banned IPs cannot receive the MOTD or status ping.
pub struct BanIpCheckMiddleware {
    ban_manager: Arc<BanManager>,
}

impl BanIpCheckMiddleware {
    /// Creates a new ban IP check middleware.
    pub const fn new(ban_manager: Arc<BanManager>) -> Self {
        Self { ban_manager }
    }
}

impl Middleware for BanIpCheckMiddleware {
    fn name(&self) -> &'static str {
        "ban_ip_check"
    }

    fn process<'a>(
        &'a self,
        ctx: &'a mut ConnectionContext,
    ) -> Pin<Box<dyn Future<Output = Result<MiddlewareResult, CoreError>> + Send + 'a>> {
        Box::pin(async move {
            let ip = ctx.client_ip;

            if self.ban_manager.is_ip_banned(&ip).await?.is_some() {
                tracing::info!(
                    ip = %ip,
                    "connection dropped: IP is banned"
                );
                // ShortCircuit, not Reject: at this stage the client hasn't
                // sent a handshake yet, so we can't send a proper disconnect
                // packet. Just close the connection silently.
                Ok(MiddlewareResult::ShortCircuit)
            } else {
                Ok(MiddlewareResult::Continue)
            }
        })
    }
}
