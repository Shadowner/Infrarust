use std::future::Future;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::pin::Pin;

use infrarust_core::error::CoreError;
use infrarust_core::pipeline::Pipeline;
use infrarust_core::pipeline::context::ConnectionContext;
use infrarust_core::pipeline::middleware::{Middleware, MiddlewareResult};

// --- Test helpers ---

/// Creates a minimal ConnectionContext for testing (no real TCP stream).
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
        IpAddr::V4(Ipv4Addr::LOCALHOST),
        addr,
    )
}

/// A middleware that always continues and inserts a marker.
struct ContinueMiddleware;

impl Middleware for ContinueMiddleware {
    fn name(&self) -> &'static str {
        "continue"
    }

    fn process<'a>(
        &'a self,
        ctx: &'a mut ConnectionContext,
    ) -> Pin<Box<dyn Future<Output = Result<MiddlewareResult, CoreError>> + Send + 'a>> {
        Box::pin(async move {
            ctx.extensions.insert(42u32);
            Ok(MiddlewareResult::Continue)
        })
    }
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
    ) -> Pin<Box<dyn Future<Output = Result<MiddlewareResult, CoreError>> + Send + 'a>> {
        Box::pin(async move { Ok(MiddlewareResult::Reject("blocked".into())) })
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
    ) -> Pin<Box<dyn Future<Output = Result<MiddlewareResult, CoreError>> + Send + 'a>> {
        Box::pin(async move { Ok(MiddlewareResult::ShortCircuit) })
    }
}

/// A middleware that inserts a string marker, used to detect execution order.
struct MarkerMiddleware(&'static str);

impl Middleware for MarkerMiddleware {
    fn name(&self) -> &'static str {
        self.0
    }

    fn process<'a>(
        &'a self,
        ctx: &'a mut ConnectionContext,
    ) -> Pin<Box<dyn Future<Output = Result<MiddlewareResult, CoreError>> + Send + 'a>> {
        let name = self.0;
        Box::pin(async move {
            let existing = ctx
                .extensions
                .get::<Vec<String>>()
                .cloned()
                .unwrap_or_default();
            let mut order = existing;
            order.push(name.to_string());
            ctx.extensions.insert(order);
            Ok(MiddlewareResult::Continue)
        })
    }
}

// --- Tests ---

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
    pipeline.add(Box::new(ContinueMiddleware));

    let mut ctx = make_test_ctx().await;
    let result = pipeline.execute(&mut ctx).await.unwrap();
    assert!(matches!(result, MiddlewareResult::Reject(_)));
    // ContinueMiddleware should NOT have run
    assert!(ctx.extensions.get::<u32>().is_none());
}

#[tokio::test]
async fn test_middleware_shortcircuit_stops() {
    let mut pipeline = Pipeline::new();
    pipeline.add(Box::new(ShortCircuitMiddleware));
    pipeline.add(Box::new(ContinueMiddleware));

    let mut ctx = make_test_ctx().await;
    let result = pipeline.execute(&mut ctx).await.unwrap();
    assert!(matches!(result, MiddlewareResult::ShortCircuit));
    assert!(ctx.extensions.get::<u32>().is_none());
}

#[tokio::test]
async fn test_middleware_order_matters() {
    let mut pipeline = Pipeline::new();
    pipeline.add(Box::new(MarkerMiddleware("first")));
    pipeline.add(Box::new(MarkerMiddleware("second")));
    pipeline.add(Box::new(MarkerMiddleware("third")));

    let mut ctx = make_test_ctx().await;
    let result = pipeline.execute(&mut ctx).await.unwrap();
    assert!(matches!(result, MiddlewareResult::Continue));

    let order = ctx.extensions.get::<Vec<String>>().unwrap();
    assert_eq!(order, &["first", "second", "third"]);
}

#[tokio::test]
async fn test_extensions_accumulate() {
    let mut pipeline = Pipeline::new();
    pipeline.add(Box::new(ContinueMiddleware)); // inserts u32
    pipeline.add(Box::new(MarkerMiddleware("after"))); // inserts Vec<String>

    let mut ctx = make_test_ctx().await;
    pipeline.execute(&mut ctx).await.unwrap();

    assert_eq!(ctx.extensions.get::<u32>(), Some(&42));
    assert!(ctx.extensions.get::<Vec<String>>().is_some());
}
