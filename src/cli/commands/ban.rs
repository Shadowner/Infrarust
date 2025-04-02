use std::net::IpAddr;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use crate::Infrarust;
use crate::cli::command::{Command, CommandFuture};
use crate::cli::format as fmt;
use crate::security::BanEntry;
use crate::security::filter::FilterError;
use tracing::debug;

pub struct BanCommand {
    infrarust: Arc<Infrarust>,
}

impl BanCommand {
    pub fn new(infrarust: Arc<Infrarust>) -> Self {
        Self { infrarust }
    }

    async fn ban_player(&self, args: Vec<String>) -> String {
        if args.is_empty() {
            return fmt::error(
                "Usage: ban [--ip/-ip <address> | --username/-u <username> | --uuid/-id <uuid>] [--reason <reason>] [--duration <duration>]",
            )
            .to_string();
        }

        let mut ip = None;
        let mut username = None;
        let mut uuid = None;
        let mut reason = "Banned by administrator".to_string();
        let mut duration = None;

        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "--ip" | "-ip" => {
                    if i + 1 < args.len() {
                        match IpAddr::from_str(&args[i + 1]) {
                            Ok(addr) => ip = Some(addr),
                            Err(_) => {
                                return fmt::error(&format!("Invalid IP address: {}", args[i + 1]))
                                    .to_string();
                            }
                        }
                        i += 2;
                    } else {
                        return fmt::error("Missing IP address after --ip/-ip").to_string();
                    }
                }
                "--username" | "-u" => {
                    if i + 1 < args.len() {
                        username = Some(args[i + 1].clone());
                        i += 2;
                    } else {
                        return fmt::error("Missing username after --username/-u").to_string();
                    }
                }
                "--uuid" | "-id" => {
                    if i + 1 < args.len() {
                        uuid = Some(args[i + 1].clone());
                        i += 2;
                    } else {
                        return fmt::error("Missing UUID after --uuid/-id").to_string();
                    }
                }
                "--reason" => {
                    if i + 1 < args.len() {
                        reason = args[i + 1].clone();
                        i += 2;
                    } else {
                        return fmt::error("Missing reason after --reason").to_string();
                    }
                }
                "--duration" => {
                    if i + 1 < args.len() {
                        match parse_duration(&args[i + 1]) {
                            Ok(dur) => duration = Some(dur),
                            Err(e) => {
                                return fmt::error(&format!("Invalid duration: {}", e)).to_string();
                            }
                        }
                        i += 2;
                    } else {
                        return fmt::error("Missing duration after --duration").to_string();
                    }
                }
                _ => {
                    return fmt::error(&format!("Unknown option: {}", args[i])).to_string();
                }
            }
        }

        if ip.is_none() && username.is_none() && uuid.is_none() {
            return fmt::error("At least one identifier (IP, username, or UUID) is required")
                .to_string();
        }

        let ban_entry = BanEntry::new(
            ip,
            uuid.clone(),
            username.clone(),
            reason.clone(),
            duration,
            "console".to_string(),
        );

        match self.infrarust.add_ban(ban_entry).await {
            Ok(_) => {
                let mut result = String::new();
                result.push_str(&fmt::success("Ban applied successfully:").to_string());
                result.push('\n');

                if let Some(ip) = ip {
                    result.push_str(&format!(
                        "  {}: {}\n",
                        fmt::label("IP"),
                        fmt::entity(&ip.to_string())
                    ));
                }

                if let Some(username) = &username {
                    result.push_str(&format!(
                        "  {}: {}\n",
                        fmt::label("Username"),
                        fmt::entity(username)
                    ));
                }

                if let Some(uuid) = &uuid {
                    result.push_str(&format!(
                        "  {}: {}\n",
                        fmt::label("UUID"),
                        fmt::entity(uuid)
                    ));
                }

                result.push_str(&format!("  {}: {}\n", fmt::label("Reason"), reason));

                if let Some(duration) = duration {
                    result.push_str(&format!(
                        "  {}: {}\n",
                        fmt::label("Duration"),
                        format_duration(duration)
                    ));
                } else {
                    result.push_str(&format!("  {}: {}\n", fmt::label("Duration"), "Permanent"));
                }

                result
            }
            Err(e) => match e {
                FilterError::NotFound(_) => {
                    fmt::error("Ban filter is not registered. Enable ban filter in configuration.")
                        .to_string()
                }
                _ => fmt::error(&format!("Failed to apply ban: {}", e)).to_string(),
            },
        }
    }
}

impl Command for BanCommand {
    fn name(&self) -> &'static str {
        "ban"
    }

    fn description(&self) -> &'static str {
        "Bans a player by IP, username, or UUID. Use --ip/-ip, --username/-u, or --uuid/-id flags."
    }

    fn execute(&self, args: Vec<String>) -> CommandFuture {
        debug!("Executing ban command with args: {:?}", args);
        let infrarust = self.infrarust.clone();

        Box::pin(async move {
            let ban_cmd = BanCommand { infrarust };
            ban_cmd.ban_player(args).await
        })
    }
}

// Helper function to parse duration from string like "1d", "2h", "30m", etc.
fn parse_duration(duration_str: &str) -> Result<Duration, String> {
    let duration_str = duration_str.trim().to_lowercase();
    if duration_str.is_empty() {
        return Err("Empty duration string".to_string());
    }
    let mut numeric_part = String::new();
    let mut unit_part = String::new();

    for c in duration_str.chars() {
        if c.is_ascii_digit() {
            numeric_part.push(c);
        } else {
            unit_part.push(c);
        }
    }

    if numeric_part.is_empty() {
        return Err("No numeric value in duration".to_string());
    }

    let value: u64 = numeric_part
        .parse()
        .map_err(|_| "Invalid numeric value".to_string())?;

    match unit_part.as_str() {
        "s" => Ok(Duration::from_secs(value)),
        "m" => Ok(Duration::from_secs(value * 60)),
        "h" => Ok(Duration::from_secs(value * 3600)),
        "d" => Ok(Duration::from_secs(value * 86400)),
        "w" => Ok(Duration::from_secs(value * 604800)),
        "mo" => Ok(Duration::from_secs(value * 2592000)), // approximate month
        "y" => Ok(Duration::from_secs(value * 31536000)), // approximate year
        _ => Err(format!("Unknown time unit: {}", unit_part)),
    }
}

// Helper function to format duration for display
fn format_duration(duration: Duration) -> String {
    let secs = duration.as_secs();

    if secs < 60 {
        return format!("{} seconds", secs);
    }

    let minutes = secs / 60;
    if minutes < 60 {
        return format!("{} minutes", minutes);
    }

    let hours = minutes / 60;
    if hours < 24 {
        return format!("{} hours", hours);
    }

    let days = hours / 24;
    if days < 7 {
        return format!("{} days", days);
    }

    let weeks = days / 7;
    if weeks < 4 {
        return format!("{} weeks", weeks);
    }

    let months = days / 30; // approximate
    if months < 12 {
        return format!("{} months", months);
    }

    let years = days / 365; // approximate
    format!("{} years", years)
}
