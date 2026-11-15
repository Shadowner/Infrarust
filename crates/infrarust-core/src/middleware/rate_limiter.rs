use std::future::Future;
use std::num::NonZeroU32;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use governor::clock::DefaultClock;
use governor::state::keyed::DashMapStateStore;
use governor::{Quota, RateLimiter};
use tokio_util::sync::CancellationToken;

use infrarust_api::events::handshake::RejectReason;
use infrarust_config::RateLimitConfig;
use infrarust_transport::IpKey;

use crate::error::CoreError;
use crate::pipeline::context::ConnectionContext;
use crate::pipeline::middleware::{Middleware, MiddlewareResult};
use crate::pipeline::types::{ConnectionIntent, HandshakeData, Refused};

type KeyedLimiter = RateLimiter<IpKey, DashMapStateStore<IpKey>, DefaultClock>;

const PURGE_INTERVAL: Duration = Duration::from_secs(60);

/// Middleware that rate-limits connections per IP, with separate limits
/// for status pings and login attempts.
#[derive(Clone)]
pub struct RateLimiterMiddleware {
    enabled: bool,
    login_limiter: Arc<KeyedLimiter>,
    status_limiter: Arc<KeyedLimiter>,
    warned: Arc<AtomicBool>,
}

impl RateLimiterMiddleware {
    pub fn new(config: &RateLimitConfig) -> Self {
        let login_limiter = Self::build_limiter(config.max_connections, config.window);
        let status_limiter = Self::build_limiter(config.status_max, config.status_window);

        Self {
            enabled: config.enabled,
            login_limiter: Arc::new(login_limiter),
            status_limiter: Arc::new(status_limiter),
            warned: Arc::new(AtomicBool::new(false)),
        }
    }

    fn build_limiter(max: u32, window: Duration) -> KeyedLimiter {
        let max = NonZeroU32::new(max).unwrap_or(NonZeroU32::MIN);
        let quota = Quota::with_period(window / max.get())
            .unwrap_or_else(|| Quota::per_second(max))
            .allow_burst(max);
        RateLimiter::dashmap(quota)
    }

    /// Forgets the keys whose state is back to fresh.
    pub fn purge(&self) {
        for limiter in [&self.login_limiter, &self.status_limiter] {
            limiter.retain_recent();
            limiter.shrink_to_fit();
        }
    }

    /// Number of keys held across the login and status limiters.
    pub fn tracked_keys(&self) -> usize {
        self.login_limiter.len() + self.status_limiter.len()
    }

    /// Purges periodically until `shutdown` is cancelled. A disabled limiter
    /// tracks nothing and starts no task.
    pub fn start_purge_task(
        &self,
        shutdown: CancellationToken,
    ) -> Option<tokio::task::JoinHandle<()>> {
        self.enabled.then(|| {
            let limiter = self.clone();
            tokio::spawn(async move {
                let mut ticks = tokio::time::interval(PURGE_INTERVAL);
                ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                loop {
                    tokio::select! {
                        biased;
                        () = shutdown.cancelled() => break,
                        _ = ticks.tick() => limiter.purge(),
                    }
                }
            })
        })
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
            if !self.enabled {
                return Ok(MiddlewareResult::Continue);
            }

            let Some(handshake) = ctx.extensions.get::<HandshakeData>() else {
                return Ok(MiddlewareResult::Continue); // No handshake yet, skip
            };

            let limiter = match handshake.intent {
                ConnectionIntent::Status => &self.status_limiter,
                ConnectionIntent::Login | ConnectionIntent::Transfer => &self.login_limiter,
            };

            if limiter.check_key(&IpKey::from(ctx.client_ip)).is_ok() {
                Ok(MiddlewareResult::Continue)
            } else {
                tracing::debug!(
                    ip = %ctx.client_ip,
                    intent = ?handshake.intent,
                    "rate limit exceeded"
                );
                if !self.warned.swap(true, Ordering::Relaxed) {
                    tracing::warn!(
                        key = %IpKey::from(ctx.client_ip),
                        "refusing connections over [rate_limit]; clients behind playit, frp, \
                         ngrok, Docker userland proxy or NAT share one address: send the real \
                         address with receive_proxy_protocol and trusted_proxies, raise \
                         max_connections and status_max, or set enabled = false. This warning \
                         is logged once"
                    );
                }
                ctx.extensions.insert(Refused(RejectReason::RateLimit));
                Ok(MiddlewareResult::Reject("Rate limit exceeded".into()))
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enabled_field_mirrors_config() {
        let disabled = RateLimitConfig {
            enabled: false,
            ..Default::default()
        };
        assert!(!RateLimiterMiddleware::new(&disabled).enabled);

        let enabled = RateLimitConfig {
            enabled: true,
            ..Default::default()
        };
        assert!(RateLimiterMiddleware::new(&enabled).enabled);
    }

    #[test]
    fn default_config_is_enabled() {
        assert!(RateLimiterMiddleware::new(&RateLimitConfig::default()).enabled);
    }
}
