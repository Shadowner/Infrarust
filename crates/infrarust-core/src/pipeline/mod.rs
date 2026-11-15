pub mod admission;
pub mod context;
pub mod middleware;
pub mod preauth_frame;
pub mod types;

use crate::error::CoreError;
use context::ConnectionContext;
use middleware::{Middleware, MiddlewareResult};

pub struct Pipeline {
    middlewares: Vec<Box<dyn Middleware>>,
}

impl Pipeline {
    pub fn new() -> Self {
        Self {
            middlewares: Vec::new(),
        }
    }

    pub fn add(&mut self, middleware: Box<dyn Middleware>) {
        self.middlewares.push(middleware);
    }

    pub async fn execute(
        &self,
        ctx: &mut ConnectionContext,
    ) -> Result<MiddlewareResult, CoreError> {
        for mw in &self.middlewares {
            let result = mw.process(ctx).await?;
            match result {
                MiddlewareResult::Continue => {
                    tracing::trace!(middleware = mw.name(), "middleware passed");
                }
                MiddlewareResult::Drop(_) | MiddlewareResult::Claimed(_) => {
                    tracing::debug!(middleware = mw.name(), "pipeline short-circuited");
                    return Ok(result);
                }
                MiddlewareResult::Reject { ref message, .. } => {
                    tracing::debug!(
                        middleware = mw.name(),
                        reason = message,
                        "pipeline rejected"
                    );
                    return Ok(result);
                }
                MiddlewareResult::Kick { ref message, .. } => {
                    tracing::debug!(middleware = mw.name(), reason = %message, "pipeline kicked");
                    return Ok(result);
                }
            }
        }
        Ok(MiddlewareResult::Continue)
    }
}

impl Default for Pipeline {
    fn default() -> Self {
        Self::new()
    }
}
