use std::collections::HashSet;
use std::convert::Infallible;
use std::time::Duration;

use axum::extract::{Query, State};
use axum::response::Json;
use axum::response::sse::{Event, KeepAlive, Sse};
use std::sync::Arc;
use tokio_stream::Stream;

use crate::error::ApiError;
use crate::log_layer::LogEntry;
use crate::response::{ApiResponse, ok};
use crate::state::ApiState;

use super::auth::verify_sse_auth;
use super::types::{EventStreamFilter, LogHistoryFilter, LogStreamFilter};

/// SSE endpoint for real-time proxy events.
///
/// `GET /api/v1/events?token=<key>&types=player.join,stats.tick`
pub async fn event_stream(
    State(state): State<Arc<ApiState>>,
    Query(filter): Query<EventStreamFilter>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, ApiError> {
    verify_sse_auth(&state, &filter.token)?;

    let type_filter: Option<HashSet<String>> = filter
        .types
        .map(|t| t.split(',').map(|s| s.trim().to_lowercase()).collect());

    let mut receiver = state.event_tx.subscribe();

    let stream = async_stream::stream! {
        loop {
            match receiver.recv().await {
                Ok(event) => {
                    if let Some(ref allowed) = type_filter {
                        if !allowed.contains(event.event_type()) {
                            continue;
                        }
                    }

                    match serde_json::to_string(&event) {
                        Ok(json) => {
                            yield Ok(Event::default()
                                .event(event.event_type())
                                .data(json));
                        }
                        Err(e) => {
                            tracing::warn!(error = %e, "Failed to serialize SSE event");
                        }
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    let data = serde_json::json!({"missed": n}).to_string();
                    yield Ok(Event::default()
                        .event("lagged")
                        .data(data));
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    };

    Ok(Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keep-alive"),
    ))
}

/// SSE endpoint for real-time log streaming.
///
/// `GET /api/v1/logs?token=<key>&level=warn&target=infrarust_core`
pub async fn log_stream(
    State(state): State<Arc<ApiState>>,
    Query(filter): Query<LogStreamFilter>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, ApiError> {
    verify_sse_auth(&state, &filter.token)?;

    let log_tx = state.log_tx.as_ref().ok_or_else(|| {
        ApiError::ServiceUnavailable(
            "Log streaming is not available (BroadcastLogLayer not installed)".into(),
        )
    })?;

    let min_level = parse_level(&filter.level);
    let target_prefix = filter.target.clone();

    let mut receiver = log_tx.subscribe();

    let stream = async_stream::stream! {
        loop {
            match receiver.recv().await {
                Ok(entry) => {
                    if !level_matches(&entry.level, min_level) {
                        continue;
                    }
                    if let Some(ref prefix) = target_prefix {
                        if !entry.target.starts_with(prefix.as_str()) {
                            continue;
                        }
                    }

                    match serde_json::to_string(&entry) {
                        Ok(json) => {
                            yield Ok(Event::default()
                                .event("log")
                                .data(json));
                        }
                        Err(e) => {
                            tracing::warn!(error = %e, "Failed to serialize log entry");
                        }
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    let data = serde_json::json!({"missed": n}).to_string();
                    yield Ok(Event::default()
                        .event("lagged")
                        .data(data));
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    };

    Ok(Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keep-alive"),
    ))
}

/// REST endpoint for log history from the ring buffer.
///
/// `GET /api/v1/logs/history?n=50&level=warn&target=infrarust_core`
pub async fn log_history(
    State(state): State<Arc<ApiState>>,
    Query(filter): Query<LogHistoryFilter>,
) -> Result<Json<ApiResponse<Vec<LogEntry>>>, ApiError> {
    let history = state
        .log_history
        .as_ref()
        .ok_or_else(|| ApiError::ServiceUnavailable("Log history is not available".into()))?;

    let min_level = parse_level(&filter.level);
    let n = filter.n.unwrap_or(100).min(1000);

    let history_guard = history
        .lock()
        .map_err(|_| ApiError::Internal("Log history lock poisoned".into()))?;

    let entries: Vec<LogEntry> = history_guard
        .iter()
        .rev()
        .filter(|e| level_matches(&e.level, min_level))
        .filter(|e| {
            filter
                .target
                .as_ref()
                .map(|t| e.target.starts_with(t.as_str()))
                .unwrap_or(true)
        })
        .take(n)
        .cloned()
        .collect();

    Ok(ok(entries))
}

/// Maps a level string to a numeric value for comparison.
fn level_to_num(level: &str) -> u8 {
    match level.to_uppercase().as_str() {
        "TRACE" => 0,
        "DEBUG" => 1,
        "INFO" => 2,
        "WARN" => 3,
        "ERROR" => 4,
        _ => 2, // default to INFO
    }
}

/// Parses the filter level string, defaulting to INFO.
fn parse_level(level: &Option<String>) -> u8 {
    level.as_deref().map(level_to_num).unwrap_or(2) // INFO
}

/// Returns `true` if the entry's level is >= the minimum level.
fn level_matches(entry_level: &str, min_level: u8) -> bool {
    level_to_num(entry_level) >= min_level
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn level_ordering() {
        assert_eq!(level_to_num("TRACE"), 0);
        assert_eq!(level_to_num("DEBUG"), 1);
        assert_eq!(level_to_num("INFO"), 2);
        assert_eq!(level_to_num("WARN"), 3);
        assert_eq!(level_to_num("ERROR"), 4);
    }

    #[test]
    fn level_matches_filters_correctly() {
        // min=WARN (3), only WARN and ERROR pass
        assert!(!level_matches("INFO", 3));
        assert!(level_matches("WARN", 3));
        assert!(level_matches("ERROR", 3));
    }

    #[test]
    fn parse_level_defaults_to_info() {
        assert_eq!(parse_level(&None), 2);
        assert_eq!(parse_level(&Some("warn".into())), 3);
    }
}
