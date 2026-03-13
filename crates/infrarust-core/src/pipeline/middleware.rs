use std::future::Future;
use std::pin::Pin;

use crate::error::CoreError;
use crate::pipeline::context::ConnectionContext;

/// Result of a middleware execution.
#[derive(Debug)]
#[non_exhaustive]
pub enum MiddlewareResult {
    /// Continue to the next middleware in the pipeline.
    Continue,
    /// Short-circuit the pipeline (e.g. legacy detection handled inline).
    ShortCircuit,
    /// Reject the connection with a reason message (sent as kick packet).
    Reject(String),
}

/// A composable middleware in the connection processing pipeline.
///
/// Uses `Pin<Box<dyn Future>>` instead of `async-trait` for dyn-compatibility,
/// allowing `Vec<Box<dyn Middleware>>` for runtime-composable pipelines.
pub trait Middleware: Send + Sync {
    /// Human-readable name for logging and debugging.
    fn name(&self) -> &'static str;

    /// Process the connection context. Implementations should use
    /// `Box::pin(async move { ... })` to return the future.
    fn process<'a>(
        &'a self,
        ctx: &'a mut ConnectionContext,
    ) -> Pin<Box<dyn Future<Output = Result<MiddlewareResult, CoreError>> + Send + 'a>>;
}
