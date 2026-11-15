use crate::error::CoreError;
use crate::pipeline::context::ConnectionContext;
use infrarust_api::event::BoxFuture;

#[derive(Debug)]
#[non_exhaustive]
pub enum MiddlewareResult {
    Continue,
    ShortCircuit,
    Reject(String),
    Kick(infrarust_api::types::Component),
}

pub trait Middleware: Send + Sync {
    fn name(&self) -> &'static str;

    fn process<'a>(
        &'a self,
        ctx: &'a mut ConnectionContext,
    ) -> BoxFuture<'a, Result<MiddlewareResult, CoreError>>;
}
