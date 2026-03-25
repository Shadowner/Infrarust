use std::future::Future;
use std::net::IpAddr;
use std::num::NonZeroU32;
use std::pin::Pin;
use std::sync::Arc;

use governor::clock::DefaultClock;
use governor::state::keyed::DashMapStateStore;
use governor::{Quota, RateLimiter};

use infrarust_config::RateLimitConfig;

use crate::error::CoreError;
use crate::pipeline::context::ConnectionContext;
use crate::pipeline::middleware::{Middleware, MiddlewareResult};
use crate::pipeline::types::{ConnectionIntent, HandshakeData};

type KeyedLimiter = RateLimiter<IpAddr, DashMapStateStore<IpAddr>, DefaultClock>;

/// Middleware that rate-limits connections per IP, with separate limits
/// for status pings and login attempts.
pub struct RateLimiterMiddleware {
    login_limiter: Arc<KeyedLimiter>,
    status_limiter: Arc<KeyedLimiter>,
}

impl RateLimiterMiddleware {
    pub fn new(config: &RateLimitConfig) -> Self {
        let login_limiter = Self::build_limiter(config.max_connections, config.window);
        let status_limiter = Self::build_limiter(config.status_max, config.status_window);

        Self {
            login_limiter: Arc::new(login_limiter),
            status_limiter: Arc::new(status_limiter),
        }
    }

    #[allow(clippy::expect_used)] // max.max(1) guarantees NonZero, and period is always valid
    fn build_limiter(max: u32, window: std::time::Duration) -> KeyedLimiter {
        let max = NonZeroU32::new(max.max(1)).expect("rate limit max must be > 0");
        let quota = Quota::with_period(window / max.get())
            .expect("valid quota period")
            .allow_burst(max);
        RateLimiter::dashmap(quota)
    }
}

impl Middleware for RateLimiterMiddleware {
    fn name(&self) -> &'static str {
        "rate_limiter"
    }

    fn process<'a>(
        &'a self,
        ctx: &'a mut ConnectionContext,
    ) -> Pin<Box<dyn Future<Output = Result<MiddlewareResult, CoreError>> + Send + 'a>> {
        Box::pin(async move {
            let Some(handshake) = ctx.extensions.get::<HandshakeData>() else {
                return Ok(MiddlewareResult::Continue); // No handshake yet, skip
            };

            let limiter = match handshake.intent {
                ConnectionIntent::Status => &self.status_limiter,
                ConnectionIntent::Login | ConnectionIntent::Transfer => &self.login_limiter,
            };

            if limiter.check_key(&ctx.client_ip).is_ok() {
                Ok(MiddlewareResult::Continue)
            } else {
                tracing::debug!(
                    ip = %ctx.client_ip,
                    intent = ?handshake.intent,
                    "rate limit exceeded"
                );
                Ok(MiddlewareResult::Reject("Rate limit exceeded".into()))
            }
        })
    }
}
