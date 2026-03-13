pub mod context;
pub mod middleware;
pub mod types;

use crate::error::CoreError;
use context::ConnectionContext;
use middleware::{Middleware, MiddlewareResult};

/// A sequential pipeline of middlewares executed on each connection.
///
/// Middlewares run in insertion order. The pipeline stops at the first
/// `ShortCircuit` or `Reject` result.
pub struct Pipeline {
    middlewares: Vec<Box<dyn Middleware>>,
}

impl Pipeline {
    /// Creates an empty pipeline.
    pub fn new() -> Self {
        Self {
            middlewares: Vec::new(),
        }
    }

    /// Appends a middleware to the end of the pipeline.
    pub fn add(&mut self, middleware: Box<dyn Middleware>) {
        self.middlewares.push(middleware);
    }

    /// Executes all middlewares sequentially on the given context.
    ///
    /// Returns `Continue` if all middlewares passed, or the first
    /// `ShortCircuit`/`Reject` encountered.
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
                MiddlewareResult::ShortCircuit => {
                    tracing::debug!(middleware = mw.name(), "pipeline short-circuited");
                    return Ok(MiddlewareResult::ShortCircuit);
                }
                MiddlewareResult::Reject(ref reason) => {
                    tracing::debug!(middleware = mw.name(), reason, "pipeline rejected");
                    return Ok(result);
                }
            }
        }
        Ok(MiddlewareResult::Continue)
    }

    /// Returns the number of middlewares in the pipeline.
    pub fn len(&self) -> usize {
        self.middlewares.len()
    }

    /// Returns `true` if the pipeline has no middlewares.
    pub fn is_empty(&self) -> bool {
        self.middlewares.is_empty()
    }
}

impl Default for Pipeline {
    fn default() -> Self {
        Self::new()
    }
}
