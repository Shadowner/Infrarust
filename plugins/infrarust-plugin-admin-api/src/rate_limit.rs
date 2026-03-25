use std::sync::Arc;
use std::sync::Mutex;
use std::time::Instant;

use axum::extract::State;
use axum::http::{HeaderValue, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::state::ApiState;

const WINDOW_SECS: u64 = 60;

pub struct RateLimiter {
    inner: Mutex<RateLimiterInner>,
    max_requests: u64,
}

struct RateLimiterInner {
    count: u64,
    window_start: Instant,
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
            inner: Mutex::new(RateLimiterInner {
                count: 0,
                window_start: Instant::now(),
            }),
            max_requests: max_requests_per_minute,
        }
    }

    pub fn check(&self) -> RateLimitInfo {
        let mut inner = match self.inner.lock() {
            Ok(guard) => guard,
            Err(poisoned) => {
                tracing::warn!("Rate limiter lock poisoned, allowing request");
                poisoned.into_inner()
            }
        };

        let now = Instant::now();
        let elapsed = now.duration_since(inner.window_start).as_secs();

        if elapsed >= WINDOW_SECS {
            inner.window_start = now;
            inner.count = 1;
            return RateLimitInfo {
                allowed: true,
                limit: self.max_requests,
                remaining: self.max_requests.saturating_sub(1),
                reset_in: WINDOW_SECS,
            };
        }

        let reset_in = WINDOW_SECS.saturating_sub(elapsed);

        if inner.count < self.max_requests {
            inner.count += 1;
            RateLimitInfo {
                allowed: true,
                limit: self.max_requests,
                remaining: self.max_requests.saturating_sub(inner.count),
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
    let info = state.rate_limiter.check();

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
