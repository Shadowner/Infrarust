//! Ban commands: ban, ban-ip, unban, unban-ip, banlist, baninfo.

use std::future::Future;
use std::pin::Pin;

use comfy_table::Cell;
use infrarust_api::services::ban_service::{
    BanEntry, BanRequest, BanService, BanSource, BanTarget, IpNet, UnbanRequest,
};

use crate::console::ConsoleServices;
use crate::console::commands::{args, table, usage};
use crate::console::dispatcher::ConsoleCommand;
use crate::console::output::{CommandCategory, CommandOutput, OutputLine};
use crate::console::parser::{format_duration_short, parse_ban_target};

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
            let Some(name) = args.first() else {
                return usage(self);
            };

            let target = BanTarget::Username(name.to_string());
            issue_ban(services, target, name, &args[1..], String::new()).await
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
        "Ban an IP address or range"
    }

    fn usage(&self) -> &str {
        "ban-ip <ip|cidr> [duration] [reason...]"
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
            let Some(ip_str) = args.first() else {
                return usage(self);
            };

            let Some(target) = parse_address_target(ip_str) else {
                return CommandOutput::error(format!("Invalid IP address or range: '{ip_str}'"));
            };
            issue_ban(
                services,
                target,
                &format!("IP {ip_str}"),
                &args[1..],
                format!(" from IP {ip_str}"),
            )
            .await
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
            let Some(name) = args.first() else {
                return usage(self);
            };

            let target = BanTarget::Username(name.to_string());
            match services.ban_manager.revoke(console_unban(target)).await {
                Ok(Some(_)) => {
                    tracing::info!(target: "console", player = name, "Player unbanned from console");
                    CommandOutput::Success(format!("Unbanned {name}"))
                }
                Ok(None) => CommandOutput::error(format!("Player '{name}' is not banned")),
                Err(e) => CommandOutput::error(format!("Failed to unban {name}: {e}")),
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
        "Unban an IP address or range"
    }

    fn usage(&self) -> &str {
        "unban-ip <ip|cidr>"
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
            let Some(ip_str) = args.first() else {
                return usage(self);
            };

            let Some(target) = parse_address_target(ip_str) else {
                return CommandOutput::error(format!("Invalid IP address or range: '{ip_str}'"));
            };
            let ip = ip_str;

            match services.ban_manager.revoke(console_unban(target)).await {
                Ok(Some(_)) => {
                    tracing::info!(target: "console", ip = %ip, "IP unbanned from console");
                    CommandOutput::Success(format!("Unbanned IP {ip}"))
                }
                Ok(None) => CommandOutput::error(format!("IP {ip} is not banned")),
                Err(e) => CommandOutput::error(format!("Failed to unban IP {ip}: {e}")),
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
            let bans = match BanService::list_all(services.ban_manager.as_ref()).await {
                Ok(bans) => bans,
                Err(e) => return CommandOutput::error(format!("Failed to fetch bans: {e}")),
            };

            let active: Vec<&BanEntry> = bans.iter().filter(|b| !b.is_expired()).collect();

            if active.is_empty() {
                return CommandOutput::Success("No active bans".to_string());
            }

            let mut table = table(&["ID", "Target", "Type", "Reason", "Source", "Remaining"]);
            for ban in &active {
                table.row([
                    Cell::new(&ban.id),
                    Cell::new(format_ban_target(&ban.target)),
                    Cell::new(ban.target.display_type()),
                    Cell::new(ban.reason.as_deref().unwrap_or("-")),
                    Cell::new(&ban.source),
                    Cell::new(remaining_of(ban)),
                ]);
            }
            table.finish("active ban")
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
            let Some(arg) = args.first() else {
                return usage(self);
            };

            let target = parse_ban_target(arg);

            match services.ban_manager.get(&target).await {
                Ok(Some(ban)) => {
                    let remaining = remaining_of(&ban);
                    CommandOutput::Lines(vec![
                        OutputLine::Info(format!("  ID: {}", ban.id)),
                        OutputLine::Info(format!("  Target: {}", format_ban_target(&ban.target))),
                        OutputLine::Info(format!("  Type: {}", ban.target.display_type())),
                        OutputLine::Info(format!(
                            "  Reason: {}",
                            ban.reason.as_deref().unwrap_or("No reason specified")
                        )),
                        OutputLine::Info(format!("  Source: {}", ban.source)),
                        OutputLine::Info(format!("  Remaining: {remaining}")),
                        OutputLine::Info(format!("  Permanent: {}", ban.is_permanent())),
                    ])
                }
                Ok(None) => CommandOutput::Success(format!("{arg} is not banned")),
                Err(e) => CommandOutput::error(format!("Failed to check ban: {e}")),
            }
        })
    }
}

async fn issue_ban(
    services: &ConsoleServices,
    target: BanTarget,
    label: &str,
    tail: &[&str],
    kicked_scope: String,
) -> CommandOutput {
    let (duration, reason) = args::duration_and_reason(tail);
    let issued = match services
        .ban_manager
        .issue(console_ban(target, reason.clone(), duration))
        .await
    {
        Ok(issued) => issued,
        Err(e) => return CommandOutput::error(format!("Failed to ban {label}: {e}")),
    };

    let duration_str = duration
        .map(format_duration_short)
        .unwrap_or_else(|| "permanently".to_string());
    let reason_str = reason.as_deref().unwrap_or("No reason specified");

    tracing::info!(
        target: "console",
        banned = label,
        duration = %duration_str,
        reason = %reason_str,
        "ban issued from console"
    );

    let mut lines = vec![OutputLine::Success(format!(
        "Banned {label} {duration_str} (reason: {reason_str})"
    ))];
    if issued.kicked > 0 {
        lines.push(OutputLine::Success(format!(
            "Kicked {} player(s){kicked_scope}",
            issued.kicked
        )));
    }
    CommandOutput::Lines(lines)
}

fn remaining_of(ban: &BanEntry) -> String {
    if ban.is_permanent() {
        return "permanent".to_string();
    }
    ban.remaining()
        .map(format_duration_short)
        .unwrap_or_else(|| "expired".to_string())
}

fn format_ban_target(target: &BanTarget) -> String {
    match target {
        BanTarget::Ip(ip) => ip.to_string(),
        BanTarget::IpRange(net) => net.to_string(),
        BanTarget::Username(name) => name.clone(),
        BanTarget::Uuid(uuid) => uuid.to_string(),
        _ => "unknown".to_string(),
    }
}

fn parse_address_target(arg: &str) -> Option<BanTarget> {
    if let Ok(ip) = arg.parse::<std::net::IpAddr>() {
        return Some(BanTarget::Ip(ip));
    }
    arg.parse::<IpNet>().ok().map(BanTarget::IpRange)
}

fn console_ban(
    target: BanTarget,
    reason: Option<String>,
    duration: Option<std::time::Duration>,
) -> BanRequest {
    let mut request = BanRequest::new(target).source(BanSource::Console);
    request.reason = reason;
    request.duration = duration;
    request
}

fn console_unban(target: BanTarget) -> UnbanRequest {
    UnbanRequest::new(target).source(BanSource::Console)
}
