use std::future::Future;
use std::pin::Pin;

use crate::error::ServerManagerError;
use crate::state::ServerState;

/// Provider that knows how to start, stop, and check the status of a server.
///
/// This trait is **non-sealed**: users can implement their own provider for
/// panels not natively supported.
///
/// Each provider is associated with ONE server. The `ServerManagerService`
/// manages the mapping from server ID to provider.
pub trait ServerProvider: Send + Sync {
    /// Starts the server.
    ///
    /// Non-blocking: returns as soon as the start request is sent.
    /// The actual "Online" detection is handled by `check_status()` or
    /// internal monitoring (e.g., `ready_pattern` for Local).
    fn start(&self) -> Pin<Box<dyn Future<Output = Result<(), ServerManagerError>> + Send + '_>>;

    /// Stops the server gracefully.
    fn stop(&self) -> Pin<Box<dyn Future<Output = Result<(), ServerManagerError>> + Send + '_>>;

    /// Checks the current server status.
    ///
    /// Called periodically by `ServerManagerService` to detect state transitions
    /// (Starting → Online, Online → Crashed, etc.).
    fn check_status(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<ProviderStatus, ServerManagerError>> + Send + '_>>;

    /// Name of the provider type (for logging).
    fn provider_type(&self) -> &'static str;
}

/// Status returned by a provider during a check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ProviderStatus {
    Running,
    Stopped,
    Starting,
    Stopping,
    Unknown,
}

impl From<ProviderStatus> for ServerState {
    fn from(status: ProviderStatus) -> Self {
        match status {
            ProviderStatus::Running => Self::Online,
            ProviderStatus::Stopped => Self::Sleeping,
            ProviderStatus::Starting => Self::Starting,
            ProviderStatus::Stopping => Self::Stopping,
            ProviderStatus::Unknown => Self::Unknown,
        }
    }
}
