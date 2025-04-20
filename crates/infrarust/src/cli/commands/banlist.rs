use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::cli::command::{Command, CommandFuture};
use crate::cli::format as fmt;
use crate::core::shared_component::SharedComponent;
use crate::security::BanSystemAdapter;
use crate::security::filter::FilterError;
use crate::with_filter_result;
use tracing::debug;

pub struct BanListCommand {
    shared: Arc<SharedComponent>,
}

impl BanListCommand {
    pub fn new(shared: Arc<SharedComponent>) -> Self {
        Self { shared }
    }

    async fn list_bans(&self) -> String {
        let registry = self.shared.filter_registry();
        let result = with_filter_result!(
            registry,
            "global_ban_system",
            BanSystemAdapter,
            async |filter: &BanSystemAdapter| {
                match filter.get_all_bans().await {
                    Ok(bans) => Ok(bans),
                    Err(e) => Err(e),
                }
            },
            Vec::new()
        );

        match result {
            Ok(bans) => {
                if bans.is_empty() {
                    return fmt::info("No active bans found.").to_string();
                }

                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();

                let mut result = format!(
                    "{}\n\n",
                    fmt::header(&format!("Active Bans ({})", bans.len()))
                );

                for (i, ban) in bans.iter().enumerate() {
                    result.push_str(&format!("{}. ", i + 1));

                    // Determine identifier to display
                    if let Some(ip) = ban.ip {
                        result.push_str(&format!(
                            "{}: {}",
                            fmt::label("IP"),
                            fmt::entity(&ip.to_string())
                        ));
                    } else if let Some(username) = &ban.username {
                        result.push_str(&format!(
                            "{}: {}",
                            fmt::label("Username"),
                            fmt::entity(username)
                        ));
                    } else if let Some(uuid) = &ban.uuid {
                        result.push_str(&format!("{}: {}", fmt::label("UUID"), fmt::entity(uuid)));
                    } else {
                        result.push_str(&fmt::warning("Unknown ban target"));
                    }

                    result.push('\n');
                    result.push_str(&format!("   {}: {}\n", fmt::label("Reason"), ban.reason));
                    result.push_str(&format!(
                        "   {}: {}\n",
                        fmt::label("Banned by"),
                        ban.banned_by
                    ));

                    // Format creation time
                    let created_time = format_time_ago(now, ban.created_at);
                    result.push_str(&format!("   {}: {}\n", fmt::label("Created"), created_time));

                    // Show expiration if applicable
                    if let Some(expires_at) = ban.expires_at {
                        if expires_at > now {
                            let remaining = format_duration(Duration::from_secs(expires_at - now));
                            result.push_str(&format!(
                                "   {}: {} ({})\n",
                                fmt::label("Expires"),
                                format_time_from_now(now, expires_at),
                                fmt::secondary(&format!("in {}", remaining))
                            ));
                        } else {
                            result.push_str(&format!(
                                "   {}: {}\n",
                                fmt::label("Expires"),
                                fmt::warning("Expired (will be removed on next cleanup)")
                            ));
                        }
                    } else {
                        result.push_str(&format!(
                            "   {}: {}\n",
                            fmt::label("Expires"),
                            fmt::entity("Never (permanent ban)")
                        ));
                    }

                    result.push('\n');
                }

                result
            }
            Err(FilterError::NotFound(_)) => {
                fmt::error("Ban filter is not registered. Enable ban filter in configuration.")
                    .to_string()
            }
            Err(e) => fmt::error(&format!("Error accessing ban filter: {}", e)).to_string(),
        }
    }
}

impl Command for BanListCommand {
    fn name(&self) -> &'static str {
        "bans"
    }

    fn description(&self) -> &'static str {
        "Lists all active bans"
    }

    fn execute(&self, _args: Vec<String>) -> CommandFuture {
        debug!("Executing banlist command");
        let shared = self.shared.clone();

        Box::pin(async move {
            let cmd = BanListCommand { shared };
            cmd.list_bans().await
        })
    }
}

fn format_time_ago(now: u64, past_time: u64) -> String {
    if past_time > now {
        return "In the future (time synchronization issue)".to_string();
    }

    let diff = now - past_time;
    format!("{} ago", format_duration(Duration::from_secs(diff)))
}

fn format_time_from_now(now: u64, future_time: u64) -> String {
    if future_time < now {
        return "In the past (already expired)".to_string();
    }

    let diff = future_time - now;
    format!("In {}", format_duration(Duration::from_secs(diff)))
}

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
