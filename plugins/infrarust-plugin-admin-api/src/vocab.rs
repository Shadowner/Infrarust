use std::fmt;
use std::str::FromStr;

use infrarust_api::services::ban_service::BanTarget;
use infrarust_api::services::config_service::ProxyMode;
use infrarust_api::services::server_manager::ServerState;

use crate::error::ApiError;

#[derive(Debug, thiserror::Error)]
#[error("unsupported {kind} variant {variant}")]
pub struct UnsupportedVariant {
    kind: &'static str,
    variant: String,
}

impl From<UnsupportedVariant> for ApiError {
    fn from(error: UnsupportedVariant) -> Self {
        tracing::error!(error = %error, "Value outside the API's vocabulary");
        ApiError::Internal(error.to_string())
    }
}

fn expected<T: fmt::Display>(names: &[T]) -> String {
    names
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

/// Declares a unit enum whose `ALL` and `as_str` are generated from the same
/// variant list, so neither can miss a variant.
macro_rules! named_enum {
    ($name:ident { $($variant:ident => $text:literal),+ $(,)? }) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum $name {
            $($variant),+
        }

        impl $name {
            pub const ALL: &'static [Self] = &[$(Self::$variant),+];

            pub const fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $text),+
                }
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_str())
            }
        }
    };
}

named_enum!(BanTargetKind {
    Ip => "ip",
    IpRange => "ip_range",
    Username => "username",
    Uuid => "uuid",
});

impl BanTargetKind {
    pub fn parse_target(self, value: &str) -> Result<BanTarget, ApiError> {
        match self {
            Self::Ip => value
                .parse()
                .map(BanTarget::Ip)
                .or_else(|_| value.parse().map(BanTarget::IpRange))
                .map_err(|_| ApiError::BadRequest(format!("Invalid IP address: {value}"))),
            Self::IpRange => value
                .parse()
                .map(BanTarget::IpRange)
                .map_err(|_| ApiError::BadRequest(format!("Invalid IP range: {value}"))),
            Self::Username => Ok(BanTarget::Username(value.to_string())),
            Self::Uuid => value
                .parse()
                .map(BanTarget::Uuid)
                .map_err(|_| ApiError::BadRequest(format!("Invalid UUID: {value}"))),
        }
    }
}

impl FromStr for BanTargetKind {
    type Err = ApiError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .iter()
            .copied()
            .find(|kind| kind.as_str() == s)
            .ok_or_else(|| {
                ApiError::BadRequest(format!(
                    "Invalid target type '{s}'. Expected: {}",
                    expected(Self::ALL)
                ))
            })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BanTargetParts {
    pub kind: BanTargetKind,
    pub value: String,
}

impl TryFrom<&BanTarget> for BanTargetParts {
    type Error = UnsupportedVariant;

    fn try_from(target: &BanTarget) -> Result<Self, Self::Error> {
        let (kind, value) = match target {
            BanTarget::Ip(ip) => (BanTargetKind::Ip, ip.to_string()),
            BanTarget::IpRange(net) => (BanTargetKind::IpRange, net.to_string()),
            BanTarget::Username(name) => (BanTargetKind::Username, name.clone()),
            BanTarget::Uuid(uuid) => (BanTargetKind::Uuid, uuid.to_string()),
            other => {
                return Err(UnsupportedVariant {
                    kind: "BanTarget",
                    variant: format!("{other:?}"),
                });
            }
        };
        Ok(Self { kind, value })
    }
}

pub fn parse_ban_target(target_type: &str, value: &str) -> Result<BanTarget, ApiError> {
    target_type.parse::<BanTargetKind>()?.parse_target(value)
}

named_enum!(ProxyModeName {
    Passthrough => "passthrough",
    ZeroCopy => "zero_copy",
    ClientOnly => "client_only",
    Offline => "offline",
    ServerOnly => "server_only",
});

impl ProxyModeName {
    const ZERO_COPY_ALIAS: &str = "zerocopy";
}

impl FromStr for ProxyModeName {
    type Err = ApiError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s == Self::ZERO_COPY_ALIAS {
            return Ok(Self::ZeroCopy);
        }
        Self::ALL
            .iter()
            .copied()
            .find(|mode| mode.as_str() == s)
            .ok_or_else(|| {
                ApiError::BadRequest(format!(
                    "Invalid proxy mode '{s}'. Expected: {}",
                    expected(Self::ALL)
                ))
            })
    }
}

impl TryFrom<ProxyMode> for ProxyModeName {
    type Error = UnsupportedVariant;

    fn try_from(mode: ProxyMode) -> Result<Self, Self::Error> {
        match mode {
            ProxyMode::Passthrough => Ok(Self::Passthrough),
            ProxyMode::ZeroCopy => Ok(Self::ZeroCopy),
            ProxyMode::ClientOnly => Ok(Self::ClientOnly),
            ProxyMode::Offline => Ok(Self::Offline),
            ProxyMode::ServerOnly => Ok(Self::ServerOnly),
            other => Err(UnsupportedVariant {
                kind: "ProxyMode",
                variant: format!("{other:?}"),
            }),
        }
    }
}

pub fn server_state_str(state: &ServerState) -> &'static str {
    match state {
        ServerState::Online => "online",
        ServerState::Offline => "offline",
        ServerState::Starting => "starting",
        ServerState::Stopping => "stopping",
        ServerState::Sleeping => "sleeping",
        ServerState::Crashed => "crashed",
        other => {
            tracing::warn!(?other, "Unknown ServerState variant");
            "unknown"
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn parse_ban_target_ip() {
        let target = parse_ban_target("ip", "192.168.1.1").unwrap();
        assert!(matches!(target, BanTarget::Ip(_)));
    }

    #[test]
    fn parse_ban_target_username() {
        let target = parse_ban_target("username", "Steve").unwrap();
        assert!(matches!(target, BanTarget::Username(ref s) if s == "Steve"));
    }

    #[test]
    fn parse_ban_target_uuid() {
        let target = parse_ban_target("uuid", "550e8400-e29b-41d4-a716-446655440000").unwrap();
        assert!(matches!(target, BanTarget::Uuid(_)));
    }

    #[test]
    fn an_unknown_target_type_is_a_bad_request() {
        let error = parse_ban_target("email", "test@test.com").unwrap_err();
        assert!(
            matches!(error, ApiError::BadRequest(ref m) if m.contains("ip, ip_range, username, uuid"))
        );
    }

    #[test]
    fn parse_ban_target_invalid_ip() {
        assert!(parse_ban_target("ip", "not-an-ip").is_err());
    }

    #[test]
    fn ban_target_kinds_round_trip_through_their_names() {
        for &kind in BanTargetKind::ALL {
            assert_eq!(kind.to_string().parse::<BanTargetKind>().unwrap(), kind);
        }
    }

    #[test]
    fn ban_target_parts_carry_the_kind_and_the_value() {
        let parts = BanTargetParts::try_from(&BanTarget::Username("Steve".into())).unwrap();
        assert_eq!(parts.kind, BanTargetKind::Username);
        assert_eq!(parts.value, "Steve");

        let range =
            BanTargetParts::try_from(&BanTarget::IpRange("10.0.0.0/8".parse().unwrap())).unwrap();
        assert_eq!(range.kind, BanTargetKind::IpRange);
        assert_eq!(range.value, "10.0.0.0/8");
    }

    #[test]
    fn proxy_mode_names_round_trip() {
        for &mode in ProxyModeName::ALL {
            assert_eq!(mode.to_string().parse::<ProxyModeName>().unwrap(), mode);
        }
        assert_eq!(ProxyModeName::Passthrough.as_str(), "passthrough");
        assert_eq!(ProxyModeName::ClientOnly.as_str(), "client_only");
        assert_eq!(
            "zerocopy".parse::<ProxyModeName>().unwrap(),
            ProxyModeName::ZeroCopy
        );
    }

    #[test]
    fn an_unknown_proxy_mode_is_a_bad_request() {
        let error = "turbo".parse::<ProxyModeName>().unwrap_err();
        assert!(matches!(error, ApiError::BadRequest(ref m) if m.contains("passthrough")));
    }

    #[test]
    fn proxy_mode_names_follow_the_service_modes() {
        assert_eq!(
            ProxyModeName::try_from(ProxyMode::ServerOnly).unwrap(),
            ProxyModeName::ServerOnly
        );
    }
}
