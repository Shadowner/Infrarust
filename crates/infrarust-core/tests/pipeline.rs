#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex};

use infrarust_api::event::BoxFuture;
use infrarust_api::events::handshake::RejectReason;
use infrarust_core::error::CoreError;
use infrarust_core::pipeline::Pipeline;
use infrarust_core::pipeline::context::ConnectionContext;
use infrarust_core::pipeline::middleware::{Middleware, MiddlewareResult};

/// Creates a minimal `ConnectionContext` for testing (no real TCP stream).
///
/// Uses a connected loopback TCP pair so the context has a valid stream.
async fn make_test_ctx() -> ConnectionContext {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let client = tokio::net::TcpStream::connect(addr).await.unwrap();
    let (_server, _) = listener.accept().await.unwrap();

    // Build context manually since we don't have AcceptedConnection
    ConnectionContext::new_for_test(
        client,
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 12345),
        addr,
    )
}

/// A middleware that always rejects.
struct RejectMiddleware;

impl Middleware for RejectMiddleware {
    fn name(&self) -> &'static str {
        "reject"
    }

    fn process<'a>(
        &'a self,
        _ctx: &'a mut ConnectionContext,
    ) -> BoxFuture<'a, Result<MiddlewareResult, CoreError>> {
        Box::pin(async move {
            Ok(MiddlewareResult::Reject {
                reason: RejectReason::IpFilter,
                message: "blocked".into(),
            })
        })
    }
}

/// A middleware that always short-circuits.
struct ShortCircuitMiddleware;

impl Middleware for ShortCircuitMiddleware {
    fn name(&self) -> &'static str {
        "shortcircuit"
    }

    fn process<'a>(
        &'a self,
        _ctx: &'a mut ConnectionContext,
    ) -> BoxFuture<'a, Result<MiddlewareResult, CoreError>> {
        Box::pin(async move { Ok(MiddlewareResult::Drop(RejectReason::IpBanned)) })
    }
}

type Ran = Arc<Mutex<Vec<&'static str>>>;

struct MarkerMiddleware(&'static str, Ran);

impl Middleware for MarkerMiddleware {
    fn name(&self) -> &'static str {
        self.0
    }

    fn process<'a>(
        &'a self,
        _ctx: &'a mut ConnectionContext,
    ) -> BoxFuture<'a, Result<MiddlewareResult, CoreError>> {
        Box::pin(async move {
            self.1.lock().unwrap().push(self.0);
            Ok(MiddlewareResult::Continue)
        })
    }
}

#[tokio::test]
async fn test_empty_pipeline_continues() {
    let pipeline = Pipeline::new();
    let mut ctx = make_test_ctx().await;
    let result = pipeline.execute(&mut ctx).await.unwrap();
    assert!(matches!(result, MiddlewareResult::Continue));
}

#[tokio::test]
async fn test_middleware_reject_stops() {
    let mut pipeline = Pipeline::new();
    pipeline.add(Box::new(RejectMiddleware));
    let ran = Ran::default();
    pipeline.add(Box::new(MarkerMiddleware("after", Arc::clone(&ran))));

    let mut ctx = make_test_ctx().await;
    let result = pipeline.execute(&mut ctx).await.unwrap();
    assert!(matches!(result, MiddlewareResult::Reject { .. }));
    assert!(ran.lock().unwrap().is_empty());
}

#[tokio::test]
async fn test_middleware_shortcircuit_stops() {
    let mut pipeline = Pipeline::new();
    pipeline.add(Box::new(ShortCircuitMiddleware));
    let ran = Ran::default();
    pipeline.add(Box::new(MarkerMiddleware("after", Arc::clone(&ran))));

    let mut ctx = make_test_ctx().await;
    let result = pipeline.execute(&mut ctx).await.unwrap();
    assert!(matches!(result, MiddlewareResult::Drop(_)));
    assert!(ran.lock().unwrap().is_empty());
}

#[tokio::test]
async fn test_middleware_order_matters() {
    let mut pipeline = Pipeline::new();
    let ran = Ran::default();
    for name in ["first", "second", "third"] {
        pipeline.add(Box::new(MarkerMiddleware(name, Arc::clone(&ran))));
    }

    let mut ctx = make_test_ctx().await;
    let result = pipeline.execute(&mut ctx).await.unwrap();
    assert!(matches!(result, MiddlewareResult::Continue));

    assert_eq!(*ran.lock().unwrap(), ["first", "second", "third"]);
}
