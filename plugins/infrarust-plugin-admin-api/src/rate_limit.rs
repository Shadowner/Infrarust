use std::sync::Arc;
use std::sync::Mutex;
use std::time::Instant;

use axum::extract::State;
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::state::ApiState;

pub struct RateLimiter {
    inner: Mutex<RateLimiterInner>,
    max_requests: u64,
}

struct RateLimiterInner {
    count: u64,
    window_start: Instant,
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

    pub fn check(&self) -> bool {
        let mut inner = match self.inner.lock() {
            Ok(guard) => guard,
            Err(poisoned) => {
                tracing::warn!("Rate limiter lock poisoned, allowing request");
                poisoned.into_inner()
            }
        };

        let now = Instant::now();

        if now.duration_since(inner.window_start).as_secs() >= 60 {
            inner.window_start = now;
            inner.count = 1;
            return true;
        }

        if inner.count < self.max_requests {
            inner.count += 1;
            true
        } else {
            false
        }
    }
}

pub async fn rate_limit_middleware(
    State(state): State<Arc<ApiState>>,
    request: axum::extract::Request,
    next: Next,
) -> Response {
    if !state.rate_limiter.check() {
        return (
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
    }

    next.run(request).await
}
