//! Telemetry middleware — creates a root tracing span per login connection.
//!
//! NOT feature-gated. Uses only `tracing` — no `opentelemetry::*` imports.
//! When no `OTel` subscriber is installed, the span is a no-op (~2ns overhead).

use std::future::Future;
use std::pin::Pin;

use crate::error::CoreError;
use crate::pipeline::context::ConnectionContext;
use crate::pipeline::middleware::{Middleware, MiddlewareResult};
use crate::pipeline::types::{HandshakeData, LoginData, RoutingData};

/// Wrapper for storing a tracing span in `ConnectionContext` extensions.
///
/// Extracted in `server.rs` to `.instrument()` the handler call.
pub struct ConnectionSpan(pub tracing::Span);

/// Middleware that creates a root `"connection"` span with client/server metadata.
///
/// Placed in the login pipeline after `BanCheck` (so `LoginData` is available)
/// and before `ServerManager` (to include server startup time in the span).
///
/// **Reads**: `HandshakeData`, `RoutingData`, `LoginData` (all optional, degrades gracefully)
/// **Inserts**: `ConnectionSpan` (tracing span for the connection)
pub struct TelemetryMiddleware;

impl Middleware for TelemetryMiddleware {
    fn name(&self) -> &'static str {
        "telemetry"
    }

    fn process<'a>(
        &'a self,
        ctx: &'a mut ConnectionContext,
    ) -> Pin<Box<dyn Future<Output = Result<MiddlewareResult, CoreError>> + Send + 'a>> {
        Box::pin(async move {
            let client_ip = ctx.client_ip.to_string();

            let domain = ctx
                .extensions
                .get::<HandshakeData>()
                .map(|h| h.domain.clone())
                .unwrap_or_default();

            let server_id = ctx
                .extensions
                .get::<RoutingData>()
                .map(|r| r.config_id.clone())
                .unwrap_or_default();

            let proxy_mode = ctx
                .extensions
                .get::<RoutingData>()
                .map(|r| format!("{:?}", r.server_config.proxy_mode))
                .unwrap_or_default();

            let username = ctx
                .extensions
                .get::<LoginData>()
                .map(|l| l.username.clone())
                .unwrap_or_default();

            let protocol_version = ctx
                .extensions
                .get::<HandshakeData>()
                .map_or(0, |h| h.protocol_version.0);

            let span = tracing::info_span!(
                "connection",
                client.ip = %client_ip,
                server.domain = %domain,
                server.id = %server_id,
                proxy.mode = %proxy_mode,
                player.username = %username,
                protocol.version = protocol_version,
            );

            ctx.extensions.insert(ConnectionSpan(span));
            Ok(MiddlewareResult::Continue)
        })
    }
}
