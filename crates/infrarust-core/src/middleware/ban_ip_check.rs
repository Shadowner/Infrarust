use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use infrarust_api::services::ban_service::LoginAttempt;

use crate::ban::BanManager;
use crate::error::CoreError;
use crate::pipeline::context::ConnectionContext;
use crate::pipeline::middleware::{Middleware, MiddlewareResult};
use crate::pipeline::types::{ConnectionIntent, HandshakeData, Refused};

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
    ) -> Pin<Box<dyn Future<Output = Result<MiddlewareResult, CoreError>> + Send + 'a>> {
        Box::pin(async move {
            let Some(handshake) = ctx.extensions.get::<HandshakeData>() else {
                return Ok(MiddlewareResult::Continue);
            };
            if handshake.intent != ConnectionIntent::Status {
                return Ok(MiddlewareResult::Continue);
            }

            let attempt =
                LoginAttempt::status(ctx.client_ip).virtual_host(handshake.domain.clone());
            match self.ban_manager.refuse(&attempt).await {
                Some(refusal) => {
                    ctx.extensions.insert(Refused(refusal.reason));
                    Ok(MiddlewareResult::ShortCircuit)
                }
                None => Ok(MiddlewareResult::Continue),
            }
        })
    }
}
