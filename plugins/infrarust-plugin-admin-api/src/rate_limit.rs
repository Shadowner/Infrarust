use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Instant;

use axum::extract::{ConnectInfo, State};
use axum::http::{HeaderValue, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::state::ApiState;

const WINDOW_SECS: u64 = 60;

/// Fixed-window rate limiter keyed by client IP.
///
/// The `None` key is a shared fallback for requests without a known peer
/// address (e.g. tests driving the router without `ConnectInfo`).
pub struct RateLimiter {
    clients: Mutex<HashMap<Option<IpAddr>, Window>>,
    max_requests: u64,
}

struct Window {
    count: u64,
    start: Instant,
}

pub struct RateLimitInfo {
    pub allowed: bool,
    pub limit: u64,
    pub remaining: u64,
    pub reset_in: u64,
}

impl RateLimiter {
    pub fn new(max_requests_per_minute: u64) -> Self {
        Self {
            clients: Mutex::new(HashMap::new()),
            max_requests: max_requests_per_minute,
        }
    }

    pub fn check(&self, client: Option<IpAddr>) -> RateLimitInfo {
        let mut clients = match self.clients.lock() {
            Ok(guard) => guard,
            Err(poisoned) => {
                tracing::warn!("Rate limiter lock poisoned, allowing request");
                poisoned.into_inner()
            }
        };

        let now = Instant::now();
        clients.retain(|_, w| now.duration_since(w.start).as_secs() < WINDOW_SECS);

        let window = clients.entry(client).or_insert(Window {
            count: 0,
            start: now,
        });
        let reset_in = WINDOW_SECS.saturating_sub(now.duration_since(window.start).as_secs());

        if window.count < self.max_requests {
            window.count += 1;
            RateLimitInfo {
                allowed: true,
                limit: self.max_requests,
                remaining: self.max_requests.saturating_sub(window.count),
                reset_in,
            }
        } else {
            RateLimitInfo {
                allowed: false,
                limit: self.max_requests,
                remaining: 0,
                reset_in,
            }
        }
    }
}

fn header_value(n: u64) -> HeaderValue {
    HeaderValue::from_str(&n.to_string()).unwrap()
}

pub async fn rate_limit_middleware(
    State(state): State<Arc<ApiState>>,
    request: axum::extract::Request,
    next: Next,
) -> Response {
    let client_ip = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|info| info.0.ip());

    let info = state.rate_limiter.check(client_ip);

    if !info.allowed {
        let mut response = (
            StatusCode::TOO_MANY_REQUESTS,
            serde_json::json!({
                "error": {
                    "code": "RATE_LIMITED",
                    "message": "Too many requests. Please try again later."
                }
            })
            .to_string(),
        )
            .into_response();

        let headers = response.headers_mut();
        headers.insert("X-RateLimit-Limit", header_value(info.limit));
        headers.insert("X-RateLimit-Remaining", header_value(0));
        headers.insert("X-RateLimit-Reset", header_value(info.reset_in));
        headers.insert("Retry-After", header_value(info.reset_in));
        return response;
    }

    let mut response = next.run(request).await;

    let headers = response.headers_mut();
    headers.insert("X-RateLimit-Limit", header_value(info.limit));
    headers.insert("X-RateLimit-Remaining", header_value(info.remaining));
    headers.insert("X-RateLimit-Reset", header_value(info.reset_in));

    response
}

#[cfg(test)]
mod tests {
    use std::net::Ipv4Addr;

    use super::*;

    #[test]
    fn per_client_budgets_are_independent() {
        let limiter = RateLimiter::new(2);
        let a = Some(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)));
        let b = Some(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 2)));

        assert!(limiter.check(a).allowed);
        assert!(limiter.check(a).allowed);
        assert!(!limiter.check(a).allowed);
        assert!(limiter.check(b).allowed);
    }

    #[test]
    fn unknown_clients_share_the_fallback_bucket() {
        let limiter = RateLimiter::new(1);
        assert!(limiter.check(None).allowed);
        assert!(!limiter.check(None).allowed);
    }
}
