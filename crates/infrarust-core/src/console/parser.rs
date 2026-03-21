use std::net::IpAddr;
use std::time::Duration;

use infrarust_api::services::ban_service::BanTarget;

pub struct ParsedLine<'a> {
    pub command: &'a str,
    pub args: Vec<&'a str>,
}

pub fn parse_line(line: &str) -> Option<ParsedLine<'_>> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }

    let mut parts = trimmed.split_whitespace();
    let command = parts.next()?;
    let args: Vec<&str> = parts.collect();

    Some(ParsedLine { command, args })
}

pub fn parse_duration_arg(arg: &str) -> Result<Option<Duration>, String> {
    let lower = arg.to_lowercase();
    if lower == "permanent" || lower == "perm" {
        return Ok(None);
    }

    humantime::parse_duration(arg)
        .map(Some)
        .map_err(|e| format!("Invalid duration '{arg}': {e}"))
}

pub fn parse_ban_target(arg: &str) -> BanTarget {
    if let Ok(ip) = arg.parse::<IpAddr>() {
        return BanTarget::Ip(ip);
    }

    if let Ok(uuid) = uuid::Uuid::parse_str(arg) {
        return BanTarget::Uuid(uuid);
    }

    BanTarget::Username(arg.to_string())
}

pub fn format_duration_short(d: Duration) -> String {
    let total_secs = d.as_secs();

    if total_secs == 0 {
        return "0s".to_string();
    }

    let days = total_secs / 86400;
    let hours = (total_secs % 86400) / 3600;
    let minutes = (total_secs % 3600) / 60;
    let seconds = total_secs % 60;

    let mut parts = Vec::with_capacity(2);

    if days > 0 {
        parts.push(format!("{days}d"));
    }
    if hours > 0 {
        parts.push(format!("{hours}h"));
    }
    if minutes > 0 {
        parts.push(format!("{minutes}m"));
    }
    if seconds > 0 && parts.len() < 2 {
        parts.push(format!("{seconds}s"));
    }

    parts.truncate(2);
    parts.join(" ")
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn test_parse_line_empty() {
        assert!(parse_line("").is_none());
    }

    #[test]
    fn test_parse_line_whitespace_only() {
        assert!(parse_line("   ").is_none());
    }

    #[test]
    fn test_parse_line_single_command() {
        let parsed = parse_line("help").unwrap();
        assert_eq!(parsed.command, "help");
        assert!(parsed.args.is_empty());
    }

    #[test]
    fn test_parse_line_command_with_args() {
        let parsed = parse_line("ban player1 30m griefing").unwrap();
        assert_eq!(parsed.command, "ban");
        assert_eq!(parsed.args, vec!["player1", "30m", "griefing"]);
    }

    #[test]
    fn test_parse_line_leading_trailing_whitespace() {
        let parsed = parse_line("  kick  Steve  ").unwrap();
        assert_eq!(parsed.command, "kick");
        assert_eq!(parsed.args, vec!["Steve"]);
    }

    #[test]
    fn test_parse_line_multiple_spaces_between_args() {
        let parsed = parse_line("msg  player   hello   world").unwrap();
        assert_eq!(parsed.command, "msg");
        assert_eq!(parsed.args, vec!["player", "hello", "world"]);
    }

    #[test]
    fn test_parse_duration_permanent() {
        assert_eq!(parse_duration_arg("permanent").unwrap(), None);
        assert_eq!(parse_duration_arg("perm").unwrap(), None);
        assert_eq!(parse_duration_arg("PERMANENT").unwrap(), None);
        assert_eq!(parse_duration_arg("Perm").unwrap(), None);
    }

    #[test]
    fn test_parse_duration_minutes() {
        let d = parse_duration_arg("30m").unwrap().unwrap();
        assert_eq!(d, Duration::from_secs(1800));
    }

    #[test]
    fn test_parse_duration_days() {
        let d = parse_duration_arg("7d").unwrap().unwrap();
        assert_eq!(d, Duration::from_secs(7 * 86400));
    }

    #[test]
    fn test_parse_duration_combined() {
        let d = parse_duration_arg("2h30m").unwrap().unwrap();
        assert_eq!(d, Duration::from_secs(2 * 3600 + 30 * 60));
    }

    #[test]
    fn test_parse_duration_invalid() {
        assert!(parse_duration_arg("invalid!!!").is_err());
    }

    #[test]
    fn test_parse_ban_target_ipv4() {
        let target = parse_ban_target("1.2.3.4");
        assert!(matches!(target, BanTarget::Ip(ip) if ip.to_string() == "1.2.3.4"));
    }

    #[test]
    fn test_parse_ban_target_ipv6() {
        let target = parse_ban_target("::1");
        assert!(matches!(target, BanTarget::Ip(_)));
    }

    #[test]
    fn test_parse_ban_target_uuid() {
        let target = parse_ban_target("550e8400-e29b-41d4-a716-446655440000");
        assert!(matches!(target, BanTarget::Uuid(_)));
    }

    #[test]
    fn test_parse_ban_target_username() {
        let target = parse_ban_target("Notch");
        assert!(matches!(target, BanTarget::Username(ref name) if name == "Notch"));
    }

    #[test]
    fn test_format_duration_zero() {
        assert_eq!(format_duration_short(Duration::from_secs(0)), "0s");
    }

    #[test]
    fn test_format_duration_seconds_only() {
        assert_eq!(format_duration_short(Duration::from_secs(45)), "45s");
    }

    #[test]
    fn test_format_duration_minutes_and_seconds() {
        assert_eq!(format_duration_short(Duration::from_secs(125)), "2m 5s");
    }

    #[test]
    fn test_format_duration_hours_and_minutes() {
        assert_eq!(
            format_duration_short(Duration::from_secs(2 * 3600 + 14 * 60)),
            "2h 14m"
        );
    }

    #[test]
    fn test_format_duration_days_and_hours() {
        assert_eq!(
            format_duration_short(Duration::from_secs(3 * 86400 + 5 * 3600)),
            "3d 5h"
        );
    }

    #[test]
    fn test_format_duration_truncates_to_two_units() {
        let d = Duration::from_secs(86400 + 2 * 3600 + 30 * 60 + 15);
        assert_eq!(format_duration_short(d), "1d 2h");
    }
}
