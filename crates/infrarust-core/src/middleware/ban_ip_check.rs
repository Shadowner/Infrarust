use std::sync::Arc;

use infrarust_api::event::BoxFuture;
use infrarust_api::services::ban_service::LoginAttempt;

use crate::ban::BanManager;
use crate::error::CoreError;
use crate::pipeline::admission;
use crate::pipeline::context::ConnectionContext;
use crate::pipeline::middleware::{Middleware, MiddlewareResult};
use crate::pipeline::types::ConnectionIntent;

pub struct BanIpCheckMiddleware {
    ban_manager: Arc<BanManager>,
}

impl BanIpCheckMiddleware {
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
    ) -> BoxFuture<'a, Result<MiddlewareResult, CoreError>> {
        Box::pin(async move {
            let Some(handshake) = ctx.handshake.as_ref() else {
                return Ok(MiddlewareResult::Continue);
            };
            if handshake.intent != ConnectionIntent::Status {
                return Ok(MiddlewareResult::Continue);
            }

            let mut attempt = LoginAttempt::status(ctx.client_ip());
            if let Some(host) = admission::virtual_host(ctx) {
                attempt = attempt.virtual_host(host);
            }
            match self.ban_manager.refuse(&attempt).await {
                Some(refusal) => Ok(MiddlewareResult::Drop(refusal.reason)),
                None => Ok(MiddlewareResult::Continue),
            }
        })
    }
}
