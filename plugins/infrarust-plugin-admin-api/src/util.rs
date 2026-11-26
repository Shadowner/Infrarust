use std::time::{Duration, SystemTime};

use infrarust_api::error::ServiceError;
use infrarust_api::services::ban_service::BanService;

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

pub fn whole_seconds(duration: Duration) -> u64 {
    duration
        .as_secs()
        .saturating_add(u64::from(duration.subsec_nanos() > 0))
}

pub fn format_system_time(time: SystemTime) -> String {
    time::OffsetDateTime::from(time)
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "unknown".to_string())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn a_started_second_counts_as_a_whole_one() {
        assert_eq!(whole_seconds(Duration::ZERO), 0);
        assert_eq!(whole_seconds(Duration::from_secs(5)), 5);
        assert_eq!(whole_seconds(Duration::from_millis(4200)), 5);
        assert_eq!(whole_seconds(Duration::MAX), u64::MAX);
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
}
