use std::time::{Duration, SystemTime};

use infrarust_api::services::ban_service::BanTarget;
use infrarust_api::services::config_service::ProxyMode;

use crate::error::ApiError;

pub fn now_iso8601() -> String {
    let now = time::OffsetDateTime::now_utc();
    now.format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "unknown".to_string())
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
    let dt = time::OffsetDateTime::from(time);
    dt.format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "unknown".to_string())
}

pub fn ban_target_type_str(target: &BanTarget) -> &'static str {
    match target {
        BanTarget::Ip(_) => "ip",
        BanTarget::Username(_) => "username",
        BanTarget::Uuid(_) => "uuid",
        other => {
            tracing::warn!(?other, "Unknown BanTarget variant");
            "unknown"
        }
    }
}

pub fn parse_ban_target(target_type: &str, value: &str) -> Result<BanTarget, ApiError> {
    match target_type {
        "ip" => value
            .parse()
            .map(BanTarget::Ip)
            .map_err(|_| ApiError::BadRequest(format!("Invalid IP address: {value}"))),
        "username" => Ok(BanTarget::Username(value.to_string())),
        "uuid" => value
            .parse()
            .map(BanTarget::Uuid)
            .map_err(|_| ApiError::BadRequest(format!("Invalid UUID: {value}"))),
        _ => Err(ApiError::BadRequest(format!(
            "Invalid target type '{target_type}'. Expected: ip, username, uuid"
        ))),
    }
}

pub fn parse_proxy_mode(s: &str) -> Result<ProxyMode, ApiError> {
    match s {
        "passthrough" => Ok(ProxyMode::Passthrough),
        "zero_copy" | "zerocopy" => Ok(ProxyMode::ZeroCopy),
        "client_only" => Ok(ProxyMode::ClientOnly),
        "offline" => Ok(ProxyMode::Offline),
        "server_only" => Ok(ProxyMode::ServerOnly),
        _ => Err(ApiError::BadRequest(format!(
            "Invalid proxy mode '{s}'. Expected: passthrough, zero_copy, client_only, offline, server_only"
        ))),
    }
}

pub fn proxy_mode_str(mode: ProxyMode) -> &'static str {
    match mode {
        ProxyMode::Passthrough => "passthrough",
        ProxyMode::ZeroCopy => "zero_copy",
        ProxyMode::ClientOnly => "client_only",
        ProxyMode::Offline => "offline",
        ProxyMode::ServerOnly => "server_only",
        other => {
            tracing::warn!(?other, "Unknown ProxyMode variant");
            "unknown"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let target =
            parse_ban_target("uuid", "550e8400-e29b-41d4-a716-446655440000").unwrap();
        assert!(matches!(target, BanTarget::Uuid(_)));
    }

    #[test]
    fn parse_ban_target_invalid_type() {
        assert!(parse_ban_target("email", "test@test.com").is_err());
    }

    #[test]
    fn parse_ban_target_invalid_ip() {
        assert!(parse_ban_target("ip", "not-an-ip").is_err());
    }

    #[test]
    fn proxy_mode_str_values() {
        assert_eq!(proxy_mode_str(ProxyMode::Passthrough), "passthrough");
        assert_eq!(proxy_mode_str(ProxyMode::ClientOnly), "client_only");
    }
}
