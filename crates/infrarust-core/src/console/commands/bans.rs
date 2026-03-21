//! Ban commands: ban, ban-ip, unban, unban-ip, banlist, baninfo.

use std::future::Future;
use std::pin::Pin;

use comfy_table::Cell;
use infrarust_api::services::ban_service::{BanEntry, BanTarget};
use infrarust_api::services::player_registry::PlayerRegistry;
use infrarust_api::types::Component;

use crate::console::dispatcher::ConsoleCommand;
use crate::console::output::{CommandCategory, CommandOutput, OutputLine};
use crate::console::parser::{format_duration_short, parse_ban_target, parse_duration_arg};
use crate::console::ConsoleServices;

pub struct BanCommand;

impl ConsoleCommand for BanCommand {
    fn name(&self) -> &str {
        "ban"
    }

    fn description(&self) -> &str {
        "Ban a player by username"
    }

    fn usage(&self) -> &str {
        "ban <player> [duration] [reason...]"
    }

    fn category(&self) -> CommandCategory {
        CommandCategory::Bans
    }

    fn execute<'a>(
        &'a self,
        args: &'a [&'a str],
        services: &'a ConsoleServices,
    ) -> Pin<Box<dyn Future<Output = CommandOutput> + Send + 'a>> {
        Box::pin(async move {
            let name = match args.first() {
                Some(n) => *n,
                None => {
                    return CommandOutput::Error(
                        "Usage: ban <player> [duration] [reason...]".to_string(),
                    )
                }
            };

            let (duration, reason_start) = if args.len() > 1 {
                match parse_duration_arg(args[1]) {
                    Ok(d) => (d, 2),
                    Err(_) => (None, 1),
                }
            } else {
                (None, args.len())
            };

            let reason = if reason_start < args.len() {
                Some(args[reason_start..].join(" "))
            } else {
                None
            };

            let target = BanTarget::Username(name.to_string());
            if let Err(e) = services
                .ban_manager
                .ban(target, reason.clone(), duration, "console".to_string())
                .await
            {
                return CommandOutput::Error(format!("Failed to ban {name}: {e}"));
            }

            let duration_str = duration
                .map(|d| format_duration_short(d))
                .unwrap_or_else(|| "permanently".to_string());
            let reason_str = reason
                .as_deref()
                .unwrap_or("No reason specified");

            tracing::info!(
                target: "console",
                player = name,
                duration = %duration_str,
                reason = %reason_str,
                "Player banned from console"
            );

            let mut lines = vec![OutputLine::Success(format!(
                "Banned {name} {duration_str} (reason: {reason_str})"
            ))];

            if let Some(player) = services.player_registry.get_player(name) {
                let server = player
                    .current_server()
                    .map(|s| s.as_str().to_string())
                    .unwrap_or_default();
                player
                    .disconnect(Component::text(format!("Banned: {reason_str}")))
                    .await;
                lines.push(OutputLine::Success(format!(
                    "Player kicked from {server}"
                )));
            }

            CommandOutput::Lines(lines)
        })
    }
}

pub struct BanIpCommand;

impl ConsoleCommand for BanIpCommand {
    fn name(&self) -> &str {
        "ban-ip"
    }

    fn aliases(&self) -> &[&str] {
        &["banip"]
    }

    fn description(&self) -> &str {
        "Ban an IP address"
    }

    fn usage(&self) -> &str {
        "ban-ip <ip> [duration] [reason...]"
    }

    fn category(&self) -> CommandCategory {
        CommandCategory::Bans
    }

    fn execute<'a>(
        &'a self,
        args: &'a [&'a str],
        services: &'a ConsoleServices,
    ) -> Pin<Box<dyn Future<Output = CommandOutput> + Send + 'a>> {
        Box::pin(async move {
            let ip_str = match args.first() {
                Some(ip) => *ip,
                None => {
                    return CommandOutput::Error(
                        "Usage: ban-ip <ip> [duration] [reason...]".to_string(),
                    )
                }
            };

            let ip: std::net::IpAddr = match ip_str.parse() {
                Ok(ip) => ip,
                Err(_) => return CommandOutput::Error(format!("Invalid IP address: '{ip_str}'")),
            };

            let (duration, reason_start) = if args.len() > 1 {
                match parse_duration_arg(args[1]) {
                    Ok(d) => (d, 2),
                    Err(_) => (None, 1),
                }
            } else {
                (None, args.len())
            };

            let reason = if reason_start < args.len() {
                Some(args[reason_start..].join(" "))
            } else {
                None
            };

            let target = BanTarget::Ip(ip);
            if let Err(e) = services
                .ban_manager
                .ban(target, reason.clone(), duration, "console".to_string())
                .await
            {
                return CommandOutput::Error(format!("Failed to ban IP {ip}: {e}"));
            }

            let duration_str = duration
                .map(|d| format_duration_short(d))
                .unwrap_or_else(|| "permanently".to_string());
            let reason_str = reason
                .as_deref()
                .unwrap_or("No reason specified");

            tracing::info!(
                target: "console",
                ip = %ip,
                duration = %duration_str,
                reason = %reason_str,
                "IP banned from console"
            );

            let sessions = services.connection_registry.find_by_ip(&ip);
            let kicked = sessions.len();
            for session in sessions {
                session.shutdown_token().cancel();
            }

            let mut lines = vec![OutputLine::Success(format!(
                "Banned IP {ip} {duration_str} (reason: {reason_str})"
            ))];

            if kicked > 0 {
                lines.push(OutputLine::Success(format!(
                    "Kicked {kicked} player(s) from IP {ip}"
                )));
            }

            CommandOutput::Lines(lines)
        })
    }
}

pub struct UnbanCommand;

impl ConsoleCommand for UnbanCommand {
    fn name(&self) -> &str {
        "unban"
    }

    fn aliases(&self) -> &[&str] {
        &["pardon"]
    }

    fn description(&self) -> &str {
        "Unban a player"
    }

    fn usage(&self) -> &str {
        "unban <player>"
    }

    fn category(&self) -> CommandCategory {
        CommandCategory::Bans
    }

    fn execute<'a>(
        &'a self,
        args: &'a [&'a str],
        services: &'a ConsoleServices,
    ) -> Pin<Box<dyn Future<Output = CommandOutput> + Send + 'a>> {
        Box::pin(async move {
            let name = match args.first() {
                Some(n) => *n,
                None => return CommandOutput::Error("Usage: unban <player>".to_string()),
            };

            let target = BanTarget::Username(name.to_string());
            match services.ban_manager.unban(&target).await {
                Ok(true) => {
                    tracing::info!(target: "console", player = name, "Player unbanned from console");
                    CommandOutput::Success(format!("Unbanned {name}"))
                }
                Ok(false) => CommandOutput::Error(format!("Player '{name}' is not banned")),
                Err(e) => CommandOutput::Error(format!("Failed to unban {name}: {e}")),
            }
        })
    }
}

pub struct UnbanIpCommand;

impl ConsoleCommand for UnbanIpCommand {
    fn name(&self) -> &str {
        "unban-ip"
    }

    fn aliases(&self) -> &[&str] {
        &["unbanip", "pardonip"]
    }

    fn description(&self) -> &str {
        "Unban an IP address"
    }

    fn usage(&self) -> &str {
        "unban-ip <ip>"
    }

    fn category(&self) -> CommandCategory {
        CommandCategory::Bans
    }

    fn execute<'a>(
        &'a self,
        args: &'a [&'a str],
        services: &'a ConsoleServices,
    ) -> Pin<Box<dyn Future<Output = CommandOutput> + Send + 'a>> {
        Box::pin(async move {
            let ip_str = match args.first() {
                Some(ip) => *ip,
                None => return CommandOutput::Error("Usage: unban-ip <ip>".to_string()),
            };

            let ip: std::net::IpAddr = match ip_str.parse() {
                Ok(ip) => ip,
                Err(_) => return CommandOutput::Error(format!("Invalid IP address: '{ip_str}'")),
            };

            let target = BanTarget::Ip(ip);
            match services.ban_manager.unban(&target).await {
                Ok(true) => {
                    tracing::info!(target: "console", ip = %ip, "IP unbanned from console");
                    CommandOutput::Success(format!("Unbanned IP {ip}"))
                }
                Ok(false) => CommandOutput::Error(format!("IP {ip} is not banned")),
                Err(e) => CommandOutput::Error(format!("Failed to unban IP {ip}: {e}")),
            }
        })
    }
}

pub struct BanListCommand;

impl ConsoleCommand for BanListCommand {
    fn name(&self) -> &str {
        "banlist"
    }

    fn aliases(&self) -> &[&str] {
        &["bans"]
    }

    fn description(&self) -> &str {
        "List all active bans"
    }

    fn usage(&self) -> &str {
        "banlist"
    }

    fn category(&self) -> CommandCategory {
        CommandCategory::Bans
    }

    fn execute<'a>(
        &'a self,
        _args: &'a [&'a str],
        services: &'a ConsoleServices,
    ) -> Pin<Box<dyn Future<Output = CommandOutput> + Send + 'a>> {
        Box::pin(async move {
            let bans = match services.ban_manager.get_all_bans().await {
                Ok(bans) => bans,
                Err(e) => return CommandOutput::Error(format!("Failed to fetch bans: {e}")),
            };

            let active: Vec<&BanEntry> = bans.iter().filter(|b| !b.is_expired()).collect();

            if active.is_empty() {
                return CommandOutput::Success("No active bans".to_string());
            }

            let renderer = crate::console::output::OutputRenderer::new();
            let mut table = renderer.create_table();
            table.set_header(vec!["Target", "Type", "Reason", "Source", "Remaining"]);

            for ban in &active {
                let remaining = if ban.is_permanent() {
                    "permanent".to_string()
                } else {
                    ban.remaining()
                        .map(|d| format_duration_short(d))
                        .unwrap_or_else(|| "expired".to_string())
                };

                table.add_row(vec![
                    Cell::new(format_ban_target(&ban.target)),
                    Cell::new(ban.target.display_type()),
                    Cell::new(
                        ban.reason
                            .as_deref()
                            .unwrap_or("-"),
                    ),
                    Cell::new(&ban.source),
                    Cell::new(remaining),
                ]);
            }

            CommandOutput::Table {
                table,
                footer: Some(format!(" {} active ban(s)", active.len())),
            }
        })
    }
}

pub struct BanInfoCommand;

impl ConsoleCommand for BanInfoCommand {
    fn name(&self) -> &str {
        "baninfo"
    }

    fn description(&self) -> &str {
        "Show details of a ban"
    }

    fn usage(&self) -> &str {
        "baninfo <player|ip|uuid>"
    }

    fn category(&self) -> CommandCategory {
        CommandCategory::Bans
    }

    fn execute<'a>(
        &'a self,
        args: &'a [&'a str],
        services: &'a ConsoleServices,
    ) -> Pin<Box<dyn Future<Output = CommandOutput> + Send + 'a>> {
        Box::pin(async move {
            let arg = match args.first() {
                Some(a) => *a,
                None => {
                    return CommandOutput::Error(
                        "Usage: baninfo <player|ip|uuid>".to_string(),
                    )
                }
            };

            let target = parse_ban_target(arg);

            match services.ban_manager.is_banned(&target).await {
                Ok(Some(ban)) => {
                    let remaining = if ban.is_permanent() {
                        "permanent".to_string()
                    } else {
                        ban.remaining()
                            .map(|d| format_duration_short(d))
                            .unwrap_or_else(|| "expired".to_string())
                    };

                    CommandOutput::Lines(vec![
                        OutputLine::Info(format!("  Target: {}", format_ban_target(&ban.target))),
                        OutputLine::Info(format!("  Type: {}", ban.target.display_type())),
                        OutputLine::Info(format!(
                            "  Reason: {}",
                            ban.reason.as_deref().unwrap_or("No reason specified")
                        )),
                        OutputLine::Info(format!("  Source: {}", ban.source)),
                        OutputLine::Info(format!("  Remaining: {remaining}")),
                        OutputLine::Info(format!(
                            "  Permanent: {}",
                            ban.is_permanent()
                        )),
                    ])
                }
                Ok(None) => CommandOutput::Success(format!("{arg} is not banned")),
                Err(e) => CommandOutput::Error(format!("Failed to check ban: {e}")),
            }
        })
    }
}

fn format_ban_target(target: &BanTarget) -> String {
    match target {
        BanTarget::Ip(ip) => ip.to_string(),
        BanTarget::Username(name) => name.clone(),
        BanTarget::Uuid(uuid) => uuid.to_string(),
        _ => "unknown".to_string(),
    }
}
