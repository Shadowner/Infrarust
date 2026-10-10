use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use infrarust_api::services::ban_service::LoginAttempt;
use infrarust_api::types::ServerId;

use crate::ban::BanManager;
use crate::error::CoreError;
use crate::pipeline::context::ConnectionContext;
use crate::pipeline::middleware::{Middleware, MiddlewareResult};
use crate::pipeline::types::{HandshakeData, LoginData, Refused, RoutingData};

pub struct BanCheckMiddleware {
    ban_manager: Arc<BanManager>,
}

impl BanCheckMiddleware {
    pub const fn new(ban_manager: Arc<BanManager>) -> Self {
        Self { ban_manager }
    }
}

impl Middleware for BanCheckMiddleware {
    fn name(&self) -> &'static str {
        "ban_check"
    }

    fn process<'a>(
        &'a self,
        ctx: &'a mut ConnectionContext,
    ) -> Pin<Box<dyn Future<Output = Result<MiddlewareResult, CoreError>> + Send + 'a>> {
        Box::pin(async move {
            let Some(login) = ctx.extensions.get::<LoginData>() else {
                tracing::warn!("ban_check: LoginData not found in extensions, skipping check");
                return Ok(MiddlewareResult::Continue);
            };

            let mut attempt = LoginAttempt::pre_auth(ctx.client_ip, login.username.clone())
                .claimed_uuid(login.player_uuid);
            if let Some(handshake) = ctx.extensions.get::<HandshakeData>() {
                attempt = attempt.virtual_host(handshake.domain.clone());
            }
            if let Some(routing) = ctx.extensions.get::<RoutingData>() {
                attempt = attempt.server(ServerId::new(routing.config_id.clone()));
            }

            Ok(match self.ban_manager.refuse(&attempt).await {
                Some(refusal) => {
                    ctx.extensions.insert(Refused(refusal.reason));
                    MiddlewareResult::Kick(refusal.message)
                }
                None => MiddlewareResult::Continue,
            })
        })
    }
}
