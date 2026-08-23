//! Load balancer service.

use crate::types::{ServerAddress, ServerId};

pub mod private {
    /// Sealed — only the proxy implements [`LoadBalancerService`](super::LoadBalancerService).
    pub trait Sealed {}
}

/// Selection state of one backend address.
///
/// Mirrors the proxy's internal state so plugins never link against the core.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum BackendState {
    /// In rotation.
    Healthy,
    /// Ejected, but its backoff elapsed so the next sweep may retry it.
    Probing,
    /// Ejected and still inside its backoff.
    Unhealthy,
    /// Taken out of rotation by an operator.
    Draining,
}

impl BackendState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Healthy => "healthy",
            Self::Probing => "probing",
            Self::Unhealthy => "unhealthy",
            Self::Draining => "draining",
        }
    }
}

/// Live view of one backend address of a server.
#[derive(Debug, Clone)]
pub struct BackendStatus {
    pub address: ServerAddress,
    /// Static weight from the configuration.
    pub weight: u32,
    /// Weight actually used for selection, after the slow-start ramp.
    pub effective_weight: u32,
    pub state: BackendState,
    pub active_connections: usize,
    /// How long the address has been healthy. `None` once it is past its
    /// slow-start ramp, or while it is not healthy at all.
    pub healthy_since_secs: Option<u64>,
    /// How many times the address has been ejected.
    pub ejections: u32,
    pub last_failure_secs_ago: Option<u64>,
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum LbError {
    #[error("unknown server '{0}'")]
    UnknownServer(ServerId),

    #[error("'{address}' is not a backend address of server '{server}'")]
    UnknownAddress {
        server: ServerId,
        address: ServerAddress,
    },
}

/// Inspection and maintenance of a server's backend addresses.
///
/// Obtained via [`PluginContext::load_balancer_service()`](crate::plugin::PluginContext::load_balancer_service).
pub trait LoadBalancerService: Send + Sync + private::Sealed {
    /// Name of the balancing strategy of a server, e.g. `least_conn`.
    fn strategy(&self, server: &ServerId) -> Option<String>;

    /// Every backend address of a server, in configuration order.
    fn backends(&self, server: &ServerId) -> Vec<BackendStatus>;

    /// Takes an address out of the selectable pool, or puts it back.
    ///
    /// Draining never closes established sessions, and never black-holes a
    /// server: if every address of a server is drained they all stay
    /// selectable.
    ///
    /// # Errors
    /// [`LbError`] if the server or the address is unknown.
    fn set_drained(
        &self,
        server: &ServerId,
        addr: &ServerAddress,
        drained: bool,
    ) -> Result<(), LbError>;

    /// Clears the failure history of an address: ejection count, failure
    /// streak and backoff. The address ramps back up through slow start.
    /// Its drain state is left untouched.
    ///
    /// # Errors
    /// [`LbError`] if the server or the address is unknown.
    fn reset_backend(&self, server: &ServerId, addr: &ServerAddress) -> Result<(), LbError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_state_strings() {
        assert_eq!(BackendState::Healthy.as_str(), "healthy");
        assert_eq!(BackendState::Draining.as_str(), "draining");
    }

    #[test]
    fn errors_name_what_was_missing() {
        let error = LbError::UnknownAddress {
            server: ServerId::new("lobby"),
            address: ServerAddress {
                host: "10.0.0.1".into(),
                port: 25565,
            },
        };
        assert_eq!(
            error.to_string(),
            "'10.0.0.1:25565' is not a backend address of server 'lobby'"
        );
    }
}
