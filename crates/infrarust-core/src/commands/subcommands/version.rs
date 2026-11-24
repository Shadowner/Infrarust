use infrarust_api::branding::ProxyMessage;
use infrarust_api::command::CommandContext;
use infrarust_api::event::BoxFuture;
use infrarust_api::permissions::ADMIN_PERMISSION;
use infrarust_api::services::AvailableUpdate;
use infrarust_api::services::player_registry::PlayerRegistry;
use infrarust_api::types::Component;

use crate::commands::{CommandServices, SubcommandHandler};

pub(crate) struct VersionSubcommand;

impl SubcommandHandler for VersionSubcommand {
    fn name(&self) -> &str {
        "version"
    }

    fn description(&self) -> &str {
        "Show proxy version and status"
    }

    fn usage(&self) -> &str {
        "/ir version"
    }

    fn execute<'a>(
        &'a self,
        ctx: &'a CommandContext,
        _args: &'a [String],
        services: &'a CommandServices,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            let player = &ctx.source;

            let version = env!("CARGO_PKG_VERSION");
            let players = services.player_registry.online_count();
            let uptime = format_uptime(services.start_time.elapsed());

            player.send_message(ProxyMessage::info(&format!("Infrarust v{version}")));
            player.send_message(ProxyMessage::detail(&format!(
                "  Players online: {players}"
            )));
            player.send_message(ProxyMessage::detail(&format!("  Uptime: {uptime}")));

            if player.has_permission(ADMIN_PERMISSION)
                && let Some(update) = services
                    .updates
                    .as_ref()
                    .and_then(|updates| updates.status().available)
            {
                player.send_message(update_line(&update));
            }
        })
    }
}

fn update_line(update: &AvailableUpdate) -> Component {
    if update.security {
        Component::text(format!("  Security update available: {}", update.version)).color("red")
    } else {
        Component::text(format!("  Update available: {}", update.version)).color("yellow")
    }
}

fn format_uptime(duration: std::time::Duration) -> String {
    let secs = duration.as_secs();
    let days = secs / 86400;
    let hours = (secs % 86400) / 3600;
    let mins = (secs % 3600) / 60;

    if days > 0 {
        format!("{days}d {hours}h {mins}m")
    } else if hours > 0 {
        format!("{hours}h {mins}m")
    } else {
        format!("{mins}m")
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::Instant;

    use infrarust_api::services::{ProxyUpdates, UpdateStatus};
    use infrarust_api::test_util::{MockPlayer, player_source};
    use infrarust_config::PermissionsConfig;

    use super::*;
    use crate::permissions::PermissionService;
    use crate::player::registry::PlayerRegistryImpl;
    use crate::plugin::PluginRegistryImpl;
    use crate::registry::ConnectionRegistry;
    use crate::routing::DomainRouter;
    use crate::services::command_manager::CommandManagerImpl;
    use crate::services::config_service::ConfigServiceImpl;
    use crate::test_support::FakeUpdates;

    fn services(updates: Option<Arc<dyn ProxyUpdates>>) -> CommandServices {
        CommandServices {
            player_registry: Arc::new(PlayerRegistryImpl::new(Arc::new(ConnectionRegistry::new()))),
            config_service: Arc::new(ConfigServiceImpl::new(
                Arc::new(DomainRouter::new()),
                PathBuf::from("infrarust.toml"),
                Arc::new(toml::from_str("").unwrap()),
            )),
            server_manager: None,
            plugin_registry: Arc::new(PluginRegistryImpl::new()),
            command_manager: Arc::new(CommandManagerImpl::new()),
            permission_service: Arc::new(
                PermissionService::new_sync(&PermissionsConfig::default()),
            ),
            start_time: Instant::now(),
            updates,
        }
    }

    fn pending(security: bool) -> Option<Arc<dyn ProxyUpdates>> {
        let status = UpdateStatus {
            available: Some(FakeUpdates::release("2.0.0-beta.6", security)),
            ..FakeUpdates::idle("2.0.0-beta.5")
        };
        Some(Arc::new(FakeUpdates::new(status)))
    }

    async fn reply(player: MockPlayer, updates: Option<Arc<dyn ProxyUpdates>>) -> String {
        let player = player.into_arc();
        let ctx = CommandContext::new(player_source(&player), "ir", "version");
        VersionSubcommand
            .execute(&ctx, &[], &services(updates))
            .await;
        player.sent_text()
    }

    fn admin() -> MockPlayer {
        MockPlayer::new(1, "Admin").with_permission(ADMIN_PERMISSION)
    }

    fn player() -> MockPlayer {
        MockPlayer::new(2, "Steve").with_permission("infrarust.command.version")
    }

    #[tokio::test]
    async fn an_admin_sees_the_pending_update() {
        let text = reply(admin(), pending(false)).await;
        assert!(
            text.ends_with("\n  Update available: 2.0.0-beta.6"),
            "{text}"
        );
        let text = reply(admin(), pending(true)).await;
        assert!(
            text.ends_with("\n  Security update available: 2.0.0-beta.6"),
            "{text}"
        );
    }

    #[tokio::test]
    async fn a_player_who_is_not_admin_never_learns_of_a_pending_update() {
        for security in [false, true] {
            let text = reply(player(), pending(security)).await;
            assert!(text.starts_with("[Infrarust] Infrarust v"), "{text}");
            assert!(!text.contains("update"), "{text}");
            assert!(!text.contains("2.0.0-beta.6"), "{text}");
        }
    }

    #[tokio::test]
    async fn no_update_line_without_a_pending_update_or_a_service() {
        let idle: Option<Arc<dyn ProxyUpdates>> = Some(Arc::new(FakeUpdates::new(
            FakeUpdates::idle("2.0.0-beta.6"),
        )));
        for updates in [idle, None] {
            let text = reply(admin(), updates).await;
            assert!(text.contains("Uptime"), "{text}");
            assert!(!text.contains("pdate available"), "{text}");
        }
    }
}
