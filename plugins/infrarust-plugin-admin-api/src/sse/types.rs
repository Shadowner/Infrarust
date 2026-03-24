use serde::Deserialize;

/// Filters for the SSE event stream.
///
/// Usage: `GET /api/v1/events?types=player.join,player.leave,stats.tick&token=<key>`
#[derive(Debug, Deserialize)]
pub struct EventStreamFilter {
    /// Comma-separated event types to subscribe to.
    /// If absent, all event types are sent.
    pub types: Option<String>,
    /// API key for authentication (EventSource cannot send headers).
    pub token: Option<String>,
}

/// Filters for the SSE log stream.
///
/// Usage: `GET /api/v1/logs?level=warn&target=infrarust_core&token=<key>`
#[derive(Debug, Deserialize)]
pub struct LogStreamFilter {
    /// Minimum log level: "trace", "debug", "info", "warn", "error".
    /// Default: "info".
    pub level: Option<String>,
    /// Target prefix filter (e.g. "infrarust_core::proxy").
    pub target: Option<String>,
    /// API key for authentication.
    pub token: Option<String>,
}

/// Filters for the log history REST endpoint.
///
/// Usage: `GET /api/v1/logs/history?n=50&level=warn`
#[derive(Debug, Deserialize)]
pub struct LogHistoryFilter {
    /// Number of entries to return (default 100, max 1000).
    pub n: Option<usize>,
    /// Minimum log level.
    pub level: Option<String>,
    /// Target prefix filter.
    pub target: Option<String>,
}
