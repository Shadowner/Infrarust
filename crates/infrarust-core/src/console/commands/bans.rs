use std::future::Future;
use std::pin::Pin;
use std::time::{Duration, SystemTime};

use infrarust_api::services::ban_service::{
    BanEntry, BanRequest, BanService, BanSource, BanTarget, IpNet, UnbanRequest,
};

use crate::console::ConsoleServices;
use crate::console::commands::{args, usage};
use crate::console::dispatcher::ConsoleCommand;
use crate::console::output::{
    Block, CommandCategory, CommandOutput, Fields, Line, OutputLine, Span, Table,
};
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

            bans_output(&bans, SystemTime::now())
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
                Ok(Some(ban)) => ban_block(&ban, SystemTime::now()).into(),
                Ok(None) => CommandOutput::Note(format!("{arg} is not banned")),
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

    tracing::info!(
        target: "console",
        banned = label,
        duration = %duration.map_or_else(|| "permanently".to_string(), format_duration_short),
        reason = %reason.as_deref().unwrap_or("No reason specified"),
        "ban issued from console"
    );

    let banned = ban_summary(label, duration, reason.as_deref());
    if issued.kicked == 0 {
        return CommandOutput::Success(banned);
    }
    CommandOutput::Lines(vec![
        OutputLine::Success(banned),
        OutputLine::Success(format!("Kicked {} player(s){kicked_scope}", issued.kicked)),
    ])
}

fn ban_summary(label: &str, duration: Option<Duration>, reason: Option<&str>) -> String {
    let span = duration.map_or_else(
        || "permanently".to_string(),
        |duration| format!("for {}", format_duration_short(duration)),
    );
    match reason {
        Some(reason) => format!("Banned {label} {span} ({reason})"),
        None => format!("Banned {label} {span}"),
    }
}

fn bans_output(bans: &[BanEntry], now: SystemTime) -> CommandOutput {
    let active: Vec<&BanEntry> = bans
        .iter()
        .filter(|ban| ban.expires_at.is_none_or(|expires| expires > now))
        .collect();
    if active.is_empty() {
        return CommandOutput::Note("No active bans".to_string());
    }

    let mut table = Table::new(&["ID", "Target", "Type", "Remaining", "Source", "Reason"]);
    for ban in &active {
        table.row([
            Line::from(ban.id.as_str()),
            Span::entity(format_ban_target(&ban.target)).into(),
            Line::from(ban.target.display_type()),
            remaining_cell(ban, now).into(),
            Line::from(ban.source.to_string()),
            reason_cell(ban).into(),
        ]);
    }

    Block::new("Bans")
        .meta(format!("{} active", active.len()))
        .table(table)
        .into()
}

fn ban_block(ban: &BanEntry, now: SystemTime) -> Block {
    let mut fields = Fields::new()
        .field("id", ban.id.as_str())
        .field("reason", reason_cell(ban))
        .field("source", ban.source.to_string())
        .field("remaining", remaining_cell(ban, now))
        .field("issued", timestamp(ban.created_at));
    if let Some(expires) = ban.expires_at {
        fields.push("expires", timestamp(expires));
    }
    Block::new(format_ban_target(&ban.target))
        .meta(format!("{} ban", ban.target.display_type()))
        .fields(fields)
}

fn remaining_cell(ban: &BanEntry, now: SystemTime) -> Span {
    match ban.expires_at {
        None => Span::muted("permanent"),
        Some(expires) => expires.duration_since(now).map_or_else(
            |_| Span::muted("expired"),
            |left| Span::warn(format_duration_short(left)),
        ),
    }
}

fn reason_cell(ban: &BanEntry) -> Span {
    ban.reason
        .as_deref()
        .map_or_else(|| Span::muted("-"), Span::plain)
}

fn timestamp(time: SystemTime) -> String {
    humantime::format_rfc3339_seconds(time).to_string()
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
    duration: Option<Duration>,
) -> BanRequest {
    let mut request = BanRequest::new(target).source(BanSource::Console);
    request.reason = reason;
    request.duration = duration;
    request
}

fn console_unban(target: BanTarget) -> UnbanRequest {
    UnbanRequest::new(target).source(BanSource::Console)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use crate::console::render::Renderer;

    fn plain(output: &CommandOutput) -> String {
        Renderer::new(false, None).render(output)
    }

    fn at(secs: u64) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(secs)
    }

    const NOW: u64 = 1_790_000_000;

    fn bans() -> Vec<BanEntry> {
        let mut griefer = BanEntry::new(
            "b1",
            BanTarget::Username("Griefer".into()),
            BanSource::Console,
        )
        .reason("griefing spawn");
        griefer.created_at = at(NOW - 600);
        griefer.expires_at = Some(at(NOW + 3600));
        let mut bot = BanEntry::new(
            "b2",
            BanTarget::Ip("203.0.113.7".parse().unwrap()),
            BanSource::Plugin("guard".into()),
        );
        bot.created_at = at(NOW - 60);
        let mut lapsed = BanEntry::new("b3", BanTarget::Username("Old".into()), BanSource::System);
        lapsed.expires_at = Some(at(NOW - 1));
        vec![griefer, bot, lapsed]
    }

    #[test]
    fn ban_summary_names_the_span_and_the_reason() {
        assert_eq!(
            ban_summary("Notch", Some(Duration::from_secs(3600)), Some("spam")),
            "Banned Notch for 1h (spam)"
        );
        assert_eq!(
            ban_summary("IP 10.0.0.1", None, None),
            "Banned IP 10.0.0.1 permanently"
        );
    }

    #[test]
    fn banlist_lists_active_bans_with_the_reason_last() {
        assert_eq!(
            plain(&bans_output(&bans(), at(NOW))),
            "# Bans - 2 active\n\
             | ID   TARGET        TYPE       REMAINING   SOURCE         REASON\n\
             | b1   Griefer       username   1h          console        griefing spawn\n\
             | b2   203.0.113.7   IP         permanent   plugin:guard   -"
        );
    }

    #[test]
    fn banlist_without_active_bans_is_a_note() {
        let lapsed = bans().split_off(2);
        assert_eq!(plain(&bans_output(&lapsed, at(NOW))), "- No active bans");
    }

    #[test]
    fn remaining_tones_temporary_bans_as_warnings() {
        let bans = bans();
        assert_eq!(remaining_cell(&bans[0], at(NOW)), Span::warn("1h"));
        assert_eq!(remaining_cell(&bans[1], at(NOW)), Span::muted("permanent"));
        assert_eq!(remaining_cell(&bans[2], at(NOW)), Span::muted("expired"));
    }

    #[test]
    fn baninfo_shows_the_ban_as_fields() {
        let bans = bans();
        assert_eq!(
            plain(&ban_block(&bans[0], at(NOW)).into()),
            "# Griefer - username ban\n\
             | id         b1\n\
             | reason     griefing spawn\n\
             | source     console\n\
             | remaining  1h\n\
             | issued     2026-09-21T14:03:20Z\n\
             | expires    2026-09-21T15:13:20Z"
        );
        assert_eq!(
            plain(&ban_block(&bans[1], at(NOW)).into()),
            "# 203.0.113.7 - IP ban\n\
             | id         b2\n\
             | reason     -\n\
             | source     plugin:guard\n\
             | remaining  permanent\n\
             | issued     2026-09-21T14:12:20Z"
        );
    }
}
