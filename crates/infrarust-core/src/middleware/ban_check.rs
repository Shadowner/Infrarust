//! Middleware that checks if a connecting player is banned.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::ban::BanManager;
use crate::error::CoreError;
use crate::pipeline::context::ConnectionContext;
use crate::pipeline::middleware::{Middleware, MiddlewareResult};
use crate::pipeline::types::LoginData;

/// Middleware that rejects banned players during the login pipeline.
///
/// Checks the player's IP and username against the ban storage.
/// Placed after `LoginStartParserMiddleware` in the login pipeline.
pub struct BanCheckMiddleware {
    ban_manager: Arc<BanManager>,
}

impl BanCheckMiddleware {
    /// Creates a new ban check middleware.
    pub fn new(ban_manager: Arc<BanManager>) -> Self {
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
            let ip = ctx.client_ip;

            // Username from LoginData (inserted by LoginStartParser)
            let username = match ctx.extensions.get::<LoginData>() {
                Some(data) => data.username.as_str(),
                None => {
                    tracing::warn!("ban_check: LoginData not found in extensions, skipping check");
                    return Ok(MiddlewareResult::Continue);
                }
            };

            // No UUID check in Phase 2B (will be added in Phase 3 post-auth)
            let uuid: Option<&uuid::Uuid> = None;

            match self.ban_manager.check_player(&ip, username, uuid).await? {
                Some(ban_entry) => {
                    let message = ban_entry.kick_message();
                    tracing::info!(
                        ip = %ip,
                        username = %username,
                        ban_type = ban_entry.target.display_type(),
                        "connection rejected: player is banned"
                    );
                    Ok(MiddlewareResult::Reject(message))
                }
                None => Ok(MiddlewareResult::Continue),
            }
        })
    }
}
