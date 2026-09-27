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

impl ProxyMode {
    /// Returns `true` if the proxy parses packets beyond the handshake.
    pub const fn is_intercepted(&self) -> bool {
        matches!(self, Self::ClientOnly | Self::Offline | Self::Full)
    }

    /// Returns `true` if the proxy performs raw forwarding after the handshake.
    pub const fn is_forwarding(&self) -> bool {
        matches!(self, Self::Passthrough | Self::ZeroCopy | Self::ServerOnly)
    }

    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Passthrough => "passthrough",
            Self::ZeroCopy => "zero_copy",
            Self::ClientOnly => "client_only",
            Self::Offline => "offline",
            Self::ServerOnly => "server_only",
            Self::Full => "full",
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::ProxyMode;

    #[derive(serde::Deserialize)]
    struct Holder {
        mode: ProxyMode,
    }

    #[test]
    fn as_str_is_the_name_the_config_accepts() {
        for mode in [
            ProxyMode::Passthrough,
            ProxyMode::ZeroCopy,
            ProxyMode::ClientOnly,
            ProxyMode::Offline,
            ProxyMode::ServerOnly,
            ProxyMode::Full,
        ] {
            let parsed: Holder = toml::from_str(&format!("mode = \"{}\"", mode.as_str())).unwrap();
            assert_eq!(parsed.mode, mode);
        }
    }
}
