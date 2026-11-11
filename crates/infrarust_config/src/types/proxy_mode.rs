//! Proxy mode definitions.

use serde::{Deserialize, Serialize};

/// Supported proxy modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ProxyMode {
    /// Raw forwarding via `tokio::io::copy_bidirectional`.
    #[default]
    Passthrough,
    /// Raw forwarding via `splice(2)` on Linux.
    ZeroCopy,
    /// Mojang auth on the proxy side, backend in `online_mode=false`.
    ClientOnly,
    /// No authentication, transparent relay.
    Offline,
    /// Authentication handled by the backend.
    ServerOnly,
    /// Encryption on both sides (new in V2).
    Full,
}

/// Who checks that a player owns the name and UUID they log in with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IdentityCheck {
    Proxy,
    Backend,
    Nobody,
}

impl ProxyMode {
    /// Returns `true` if the proxy parses packets beyond the handshake.
    pub const fn is_intercepted(&self) -> bool {
        matches!(self, Self::ClientOnly | Self::Offline | Self::Full)
    }

    /// Returns `true` if the proxy performs raw forwarding after the handshake.
    pub const fn is_forwarding(&self) -> bool {
        matches!(self, Self::Passthrough | Self::ZeroCopy | Self::ServerOnly)
    }

    pub const fn requires_verified_player(&self) -> bool {
        match self {
            Self::ClientOnly | Self::Full => true,
            Self::Passthrough | Self::ZeroCopy | Self::Offline | Self::ServerOnly => false,
        }
    }

    pub const fn identity_check(&self) -> IdentityCheck {
        match self {
            Self::ClientOnly | Self::Full => IdentityCheck::Proxy,
            Self::Passthrough | Self::ZeroCopy | Self::ServerOnly => IdentityCheck::Backend,
            Self::Offline => IdentityCheck::Nobody,
        }
    }
}
