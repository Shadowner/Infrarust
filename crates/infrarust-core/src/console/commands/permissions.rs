use std::future::Future;
use std::pin::Pin;

use infrarust_api::player::Player;
use uuid::Uuid;

use crate::console::ConsoleServices;
use crate::console::commands::usage;
use crate::console::dispatcher::ConsoleCommand;
use crate::console::output::{
    Block, CommandCategory, CommandOutput, Failure, Hint, Line, Span, Table,
};

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
                return usage(self);
            };
            if let Some(refused) = delegated(services) {
                return refused;
            }

            if let Some(player) = services.connection_registry.find_by_username(username)
                && !player.is_online_mode()
                && !services
                    .permission_service
                    .builtin()
                    .trusts_offline_admins()
            {
                return Failure::new(format!("{username} is connected in offline mode"))
                    .with_hint(Hint::Note(
                        "only online-mode players can be admins unless [permissions] trust_offline_admins is set".to_string(),
                    ))
                    .into();
            }
            let uuid = match uuid_of(services, username).await {
                Ok(uuid) => uuid,
                Err(failure) => return failure.into(),
            };

            services.permission_service.add_admin(uuid);
            refresh(services, &uuid).await;

            CommandOutput::Success(format!(
                "Opped {username} ({uuid}) until restart; add it to [permissions].admins to keep it"
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
                return usage(self);
            };
            if let Some(refused) = delegated(services) {
                return refused;
            }

            let uuid = match uuid_of(services, username).await {
                Ok(uuid) => uuid,
                Err(failure) => return failure.into(),
            };

            if services.permission_service.remove_admin(&uuid) {
                refresh(services, &uuid).await;
                CommandOutput::Success(format!(
                    "De-opped {username} ({uuid}); remove it from [permissions].admins to keep it"
                ))
            } else {
                CommandOutput::error(format!("{username} ({uuid}) is not an admin"))
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
            if let Some(refused) = delegated(services) {
                return refused;
            }
            let admins = services.permission_service.admin_list();
            admins_output(&admins, |uuid| {
                services
                    .connection_registry
                    .get(uuid)
                    .map(|player| player.profile().username.clone())
            })
        })
    }
}

fn delegated(services: &ConsoleServices) -> Option<CommandOutput> {
    let selection = services.permission_service.selection();
    selection.plugin_id().map(|plugin| {
        Failure::new(format!("Permissions come from the '{plugin}' plugin"))
            .with_hint(Hint::Note("manage admins in that plugin".to_string()))
            .into()
    })
}

async fn refresh(services: &ConsoleServices, uuid: &Uuid) {
    if let Some(player) = services.connection_registry.find_by_uuid(uuid) {
        player.refresh_permissions().await;
    }
}

async fn uuid_of(services: &ConsoleServices, username: &str) -> Result<Uuid, Failure> {
    if let Some(player) = services.connection_registry.find_by_username(username) {
        return Ok(player.profile().uuid);
    }
    crate::permissions::resolve_username_to_uuid(username)
        .await
        .map_err(|error| Failure::new(format!("Failed to resolve '{username}': {error}")))
}

fn admins_output(admins: &[Uuid], online_name: impl Fn(&Uuid) -> Option<String>) -> CommandOutput {
    if admins.is_empty() {
        return CommandOutput::Note("No admins configured".to_string());
    }

    let mut table = Table::new(&["UUID", "Player"]);
    for uuid in admins {
        let player = online_name(uuid).map_or_else(|| Span::muted("offline"), Span::entity);
        table.row([Line::from(uuid.to_string()), player.into()]);
    }

    Block::new("Admins")
        .meta(format!("{} configured", admins.len()))
        .table(table)
        .into()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use crate::console::render::Renderer;

    fn plain(output: &CommandOutput) -> String {
        Renderer::new(false, None).render(output)
    }

    #[test]
    fn ops_names_the_admins_that_are_online() {
        let steve = Uuid::from_u128(0x5e7e);
        let alex = Uuid::from_u128(0xa1e);
        let output = admins_output(&[steve, alex], |uuid| {
            (*uuid == steve).then(|| "Steve".to_string())
        });
        assert_eq!(
            plain(&output),
            "# Admins - 2 configured\n\
             | UUID                                   PLAYER\n\
             | 00000000-0000-0000-0000-000000005e7e   Steve\n\
             | 00000000-0000-0000-0000-000000000a1e   offline"
        );
    }

    #[test]
    fn ops_without_admins_is_a_note() {
        assert_eq!(
            plain(&admins_output(&[], |_| None)),
            "- No admins configured"
        );
    }
}
