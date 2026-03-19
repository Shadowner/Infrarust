//! Middleware that ensures backend servers are started before connecting players.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use infrarust_server_manager::{ServerManagerError, ServerManagerService, ServerState};

use crate::error::CoreError;
use crate::pipeline::context::ConnectionContext;
use crate::pipeline::middleware::{Middleware, MiddlewareResult};
use crate::pipeline::types::RoutingData;

/// Middleware that intercepts login connections to servers with a `server_manager`
/// and triggers wake-up if the server is not online.
///
/// Placed after `BanCheckMiddleware` in the login pipeline.
///
/// **Requires**: `RoutingData` (from `DomainRouterMiddleware`)
pub struct ServerManagerMiddleware {
    server_manager: Arc<ServerManagerService>,
}

impl ServerManagerMiddleware {
    pub const fn new(server_manager: Arc<ServerManagerService>) -> Self {
        Self { server_manager }
    }
}

impl Middleware for ServerManagerMiddleware {
    fn name(&self) -> &'static str {
        "server_manager"
    }

    fn process<'a>(
        &'a self,
        ctx: &'a mut ConnectionContext,
    ) -> Pin<Box<dyn Future<Output = Result<MiddlewareResult, CoreError>> + Send + 'a>> {
        Box::pin(async move {
            let Some(routing) = ctx.extensions.get::<RoutingData>() else {
                return Ok(MiddlewareResult::Continue);
            };

            // No server_manager configured → pass through
            if routing.server_config.server_manager.is_none() {
                return Ok(MiddlewareResult::Continue);
            }

            let server_id = routing.server_config.effective_id();

            // Check if this server is managed
            let Some(state) = self.server_manager.get_state(&server_id) else {
                return Ok(MiddlewareResult::Continue);
            };

            match state {
                ServerState::Stopping => Ok(MiddlewareResult::Reject(
                    "Server is shutting down, please try again later.".into(),
                )),
                ServerState::Sleeping | ServerState::Crashed | ServerState::Starting => {
                    match self.server_manager.ensure_started(&server_id).await {
                        Ok(()) => Ok(MiddlewareResult::Continue),
                        Err(ServerManagerError::StartTimeout { .. }) => {
                            Ok(MiddlewareResult::Reject(
                                "Server failed to start in time. Please try again.".into(),
                            ))
                        }
                        Err(e) => {
                            tracing::error!(server = %server_id, "server manager error: {e}");
                            Ok(MiddlewareResult::Reject(
                                "Server is unavailable. Please try again later.".into(),
                            ))
                        }
                    }
                }
                _ => Ok(MiddlewareResult::Continue),
            }
        })
    }
}
