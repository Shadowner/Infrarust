use std::fmt;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

use infrarust_api::error::ServiceError;
use infrarust_api::services::ban_service::{BanService, BanTarget};
use infrarust_api::services::config_service::ProxyMode;
use infrarust_api::services::server_manager::ServerState;
use infrarust_api::types::ServerAddress;
use tokio::io::AsyncWriteExt;

use crate::error::ApiError;

pub fn now_iso8601() -> String {
    format_system_time(SystemTime::now())
}

pub fn get_memory_rss() -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        std::fs::read_to_string("/proc/self/status")
            .ok()
            .and_then(|status| {
                status
                    .lines()
                    .find(|line| line.starts_with("VmRSS:"))
                    .and_then(|line| {
                        line.split_whitespace()
                            .nth(1)
                            .and_then(|kb| kb.parse::<u64>().ok())
                            .map(|kb| kb * 1024)
                    })
            })
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

pub async fn active_ban_count(ban_service: &dyn BanService) -> Result<usize, ServiceError> {
    let bans = ban_service.list_all().await?;
    Ok(bans.iter().filter(|ban| !ban.is_expired()).count())
}

pub fn get_active_features() -> Vec<String> {
    vec!["plugin-admin-api".into()]
}

pub fn format_duration(d: Duration) -> String {
    let total_secs = d.as_secs();
    let days = total_secs / 86400;
    let hours = (total_secs % 86400) / 3600;
    let minutes = (total_secs % 3600) / 60;
    let seconds = total_secs % 60;

    let mut parts = Vec::new();
    if days > 0 {
        parts.push(format!("{days}d"));
    }
    if hours > 0 {
        parts.push(format!("{hours}h"));
    }
    if minutes > 0 {
        parts.push(format!("{minutes}m"));
    }
    if seconds > 0 || parts.is_empty() {
        parts.push(format!("{seconds}s"));
    }
    parts.join(" ")
}

pub fn format_system_time(time: SystemTime) -> String {
    time::OffsetDateTime::from(time)
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "unknown".to_string())
}

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

/// Renders an address so that [`parse_address`] can read it back, which
/// `ServerAddress`'s own `host:port` form cannot do for IPv6.
pub fn format_address(addr: &ServerAddress) -> String {
    if addr.host.contains(':') {
        format!("[{}]:{}", addr.host, addr.port)
    } else {
        format!("{}:{}", addr.host, addr.port)
    }
}

pub fn parse_address(raw: &str) -> Result<ServerAddress, ApiError> {
    let raw = raw.trim();
    if let Ok(socket) = raw.parse::<SocketAddr>() {
        return Ok(ServerAddress {
            host: socket.ip().to_string(),
            port: socket.port(),
        });
    }

    let invalid = || ApiError::BadRequest(format!("Invalid backend address: {raw}"));
    let (host, port) = raw.rsplit_once(':').ok_or_else(invalid)?;
    let host = host.trim_start_matches('[').trim_end_matches(']');
    if host.is_empty() {
        return Err(invalid());
    }
    Ok(ServerAddress {
        host: host.to_string(),
        port: port.parse().map_err(|_| invalid())?,
    })
}

#[derive(Debug, thiserror::Error)]
#[error("{path}: {source}")]
pub struct WriteError {
    pub path: PathBuf,
    pub source: std::io::Error,
}

pub async fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), WriteError> {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);

    let mut name = path.as_os_str().to_os_string();
    name.push(format!(".{}.tmp", SEQUENCE.fetch_add(1, Ordering::Relaxed)));
    let tmp = PathBuf::from(name);

    let written = match write_synced(&tmp, bytes).await {
        Ok(()) => tokio::fs::rename(&tmp, path)
            .await
            .map_err(|source| WriteError {
                path: path.to_path_buf(),
                source,
            }),
        Err(source) => Err(WriteError {
            path: tmp.clone(),
            source,
        }),
    };
    if written.is_err() {
        let _ = tokio::fs::remove_file(&tmp).await;
    }
    written
}

async fn write_synced(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut file = tokio::fs::File::create(path).await?;
    file.write_all(bytes).await?;
    file.sync_all().await
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn address_round_trips() {
        for raw in ["10.0.0.1:25565", "[::1]:25565", "mc.example.com:25566"] {
            assert_eq!(format_address(&parse_address(raw).unwrap()), raw);
        }
    }

    #[test]
    fn address_rejects_garbage() {
        assert!(parse_address("no-port").is_err());
        assert!(parse_address(":25565").is_err());
        assert!(parse_address("host:not-a-port").is_err());
    }

    #[test]
    fn format_duration_zero() {
        assert_eq!(format_duration(Duration::from_secs(0)), "0s");
    }

    #[test]
    fn format_duration_seconds_only() {
        assert_eq!(format_duration(Duration::from_secs(45)), "45s");
    }

    #[test]
    fn format_duration_minutes_and_seconds() {
        assert_eq!(format_duration(Duration::from_secs(125)), "2m 5s");
    }

    #[test]
    fn format_duration_hours_minutes_seconds() {
        assert_eq!(format_duration(Duration::from_secs(3735)), "1h 2m 15s");
    }

    #[test]
    fn format_duration_days() {
        assert_eq!(format_duration(Duration::from_secs(90061)), "1d 1h 1m 1s");
    }

    #[test]
    fn format_duration_exact_hour() {
        assert_eq!(format_duration(Duration::from_secs(3600)), "1h");
    }

    #[test]
    fn format_system_time_produces_rfc3339() {
        let time = SystemTime::UNIX_EPOCH + Duration::from_secs(1711288200);
        let result = format_system_time(time);
        assert!(result.contains('T'));
        assert!(result.ends_with('Z'));
    }

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
