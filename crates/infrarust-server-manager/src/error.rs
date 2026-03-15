use crate::state::ServerState;

/// Errors that can occur during server management operations.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ServerManagerError {
    #[error("server {server_id} not found")]
    ServerNotFound { server_id: String },

    #[error("server {server_id} is in state {state:?}, cannot {action}")]
    InvalidState {
        server_id: String,
        state: ServerState,
        action: String,
    },

    #[error("server {server_id} failed to start within {timeout:?}")]
    StartTimeout {
        server_id: String,
        timeout: std::time::Duration,
    },

    #[error("provider error for {server_id}: {message}")]
    Provider { server_id: String, message: String },

    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),

    #[error("process error: {0}")]
    Process(std::io::Error),

    #[error("API returned unexpected response: {0}")]
    ApiResponse(String),

    #[error("server {server_id} process exited with code {exit_code:?}")]
    ProcessExited {
        server_id: String,
        exit_code: Option<i32>,
    },

    #[error("shutdown in progress")]
    Shutdown,
}
