use crate::error::CoreError;
use crate::pipeline::context::ConnectionContext;
use infrarust_api::event::BoxFuture;
use infrarust_api::events::handshake::RejectReason;
use infrarust_api::handoff::Handoff;
use infrarust_api::types::Component;

#[derive(Debug)]
#[non_exhaustive]
pub enum MiddlewareResult {
    Continue,
    Drop(RejectReason),
    Reject {
        reason: RejectReason,
        message: String,
    },
    Kick {
        reason: RejectReason,
        message: Component,
    },
    Claimed(Handoff),
}

pub trait Middleware: Send + Sync {
    fn name(&self) -> &'static str;

    fn process<'a>(
        &'a self,
        ctx: &'a mut ConnectionContext,
    ) -> BoxFuture<'a, Result<MiddlewareResult, CoreError>>;
}
