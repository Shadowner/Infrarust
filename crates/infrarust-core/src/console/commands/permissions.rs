//! Console commands for managing admin permissions: `op`, `deop`, `ops`.

use std::future::Future;
use std::pin::Pin;

use comfy_table::Cell;
use infrarust_api::player::Player;

use crate::console::ConsoleServices;
use crate::console::dispatcher::ConsoleCommand;
use crate::console::output::{CommandCategory, CommandOutput};

pub struct OpCommand;

impl ConsoleCommand for OpCommand {
    fn name(&self) -> &str {
        "op"
    }

    fn description(&self) -> &str {
        "Grant admin to a player"
    }

    fn usage(&self) -> &str {
        "op <username>"
    }

    fn category(&self) -> CommandCategory {
        CommandCategory::Players
    }

    fn execute<'a>(
        &'a self,
        args: &'a [&'a str],
        services: &'a ConsoleServices,
    ) -> Pin<Box<dyn Future<Output = CommandOutput> + Send + 'a>> {
        Box::pin(async move {
            let Some(username) = args.first() else {
                return CommandOutput::Error("Usage: op <username>".to_string());
            };

            let uuid = if let Some(player) = services.connection_registry.find_by_username(username)
            {
                if !player.is_online_mode() {
                    return CommandOutput::Error(format!(
                        "Player '{}' is connected in offline mode — only online-mode players can be admin.",
                        username
                    ));
                }
                player.profile().uuid
            } else {
                match crate::permissions::resolve_username_to_uuid(username).await {
                    Ok(uuid) => uuid,
                    Err(e) => {
                        return CommandOutput::Error(format!(
                            "Failed to resolve '{}': {}",
                            username, e
                        ));
                    }
                }
            };

            services.permission_service.add_admin(uuid);

            CommandOutput::Success(format!(
                "Opped {} (UUID: {}). Change is effective until restart — add UUID to [permissions].admins in infrarust.toml to persist.",
                username, uuid
            ))
        })
    }
}

pub struct DeopCommand;

impl ConsoleCommand for DeopCommand {
    fn name(&self) -> &str {
        "deop"
    }

    fn description(&self) -> &str {
        "Revoke admin from a player"
    }

    fn usage(&self) -> &str {
        "deop <username>"
    }

    fn category(&self) -> CommandCategory {
        CommandCategory::Players
    }

    fn execute<'a>(
        &'a self,
        args: &'a [&'a str],
        services: &'a ConsoleServices,
    ) -> Pin<Box<dyn Future<Output = CommandOutput> + Send + 'a>> {
        Box::pin(async move {
            let Some(username) = args.first() else {
                return CommandOutput::Error("Usage: deop <username>".to_string());
            };

            let uuid = if let Some(player) = services.connection_registry.find_by_username(username)
            {
                player.profile().uuid
            } else {
                match crate::permissions::resolve_username_to_uuid(username).await {
                    Ok(uuid) => uuid,
                    Err(e) => {
                        return CommandOutput::Error(format!(
                            "Failed to resolve '{}': {}",
                            username, e
                        ));
                    }
                }
            };

            if services.permission_service.remove_admin(&uuid) {
                CommandOutput::Success(format!(
                    "De-opped {} (UUID: {}). Remove UUID from [permissions].admins in infrarust.toml to persist.",
                    username, uuid
                ))
            } else {
                CommandOutput::Error(format!("'{}' (UUID: {}) was not an admin.", username, uuid))
            }
        })
    }
}

pub struct OpListCommand;

impl ConsoleCommand for OpListCommand {
    fn name(&self) -> &str {
        "ops"
    }

    fn aliases(&self) -> &[&str] {
        &["oplist"]
    }

    fn description(&self) -> &str {
        "List current admins"
    }

    fn usage(&self) -> &str {
        "ops"
    }

    fn category(&self) -> CommandCategory {
        CommandCategory::Players
    }

    fn execute<'a>(
        &'a self,
        _args: &'a [&'a str],
        services: &'a ConsoleServices,
    ) -> Pin<Box<dyn Future<Output = CommandOutput> + Send + 'a>> {
        Box::pin(async move {
            let admins = services.permission_service.admin_list();

            if admins.is_empty() {
                return CommandOutput::Success("No admins configured.".to_string());
            }

            let renderer = crate::console::output::OutputRenderer::new();
            let mut table = renderer.create_table();
            table.set_header(vec!["UUID", "Username (if online)"]);

            for uuid in &admins {
                let online_name = services
                    .connection_registry
                    .get(uuid)
                    .map(|p| p.profile().username.clone())
                    .unwrap_or_else(|| "-".to_string());
                table.add_row(vec![Cell::new(uuid.to_string()), Cell::new(online_name)]);
            }

            CommandOutput::Table {
                table,
                footer: Some(format!(" {} admin(s)", admins.len())),
            }
        })
    }
}
