---
title: Commands
description: Register custom commands that players can run from chat, with argument parsing and tab completion.
outline: [2, 3]
---

# Commands

Plugins register commands through the `CommandManager`. When a player types `/yourcommand` in chat, the proxy intercepts it, parses the arguments, and calls your handler. The message is never forwarded to the backend server.

## Registering a command

Call `command_manager().register()` during `on_enable`:

```rust
ctx.command_manager().register(
    "hello",                       // command name
    &["hi", "hey"],                // aliases
    "Says hello to the player",    // description
    Box::new(HelloCommand),        // handler
);
```

Command names and aliases are case-insensitive. The proxy converts them to lowercase internally.

To remove a command at runtime:

```rust
ctx.command_manager().unregister("hello");
```

Unregistering also cleans up all aliases for that command.

## CommandHandler

Implement the `CommandHandler` trait on a struct. The `execute` method receives a `CommandContext` and a reference to the `PlayerRegistry`.

```rust
use infrarust_api::prelude::*;

struct HelloCommand;

impl CommandHandler for HelloCommand {
    fn execute<'a>(
        &'a self,
        ctx: CommandContext,
        player_registry: &'a dyn PlayerRegistry,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            if let Some(id) = ctx.player_id
                && let Some(player) = player_registry.get_player_by_id(id)
            {
                let _ = player.send_message(
                    Component::text("Hello from Infrarust! ")
                        .color("gold")
                        .bold()
                        .append(Component::text("Welcome to the proxy.").color("gray")),
                );
            }
        })
    }
}
```

The return type is `BoxFuture<'a, ()>`. Wrap your async block with `Box::pin(async move { ... })`.

## CommandContext

Every handler receives a `CommandContext`:

| Field | Type | Description |
|-------|------|-------------|
| `player_id` | `Option<PlayerId>` | The player who ran the command. `None` for console commands |
| `args` | `Vec<String>` | Arguments split by whitespace |
| `raw` | `String` | The full command string as typed |

If a player types `/changepassword oldpass newpass`, your handler receives:

- `args[0]` = `"oldpass"`
- `args[1]` = `"newpass"`
- `raw` = `"changepassword oldpass newpass"`

## Parsing arguments

Arguments arrive as a `Vec<String>`. Check `args.len()` before accessing by index, and send a usage message if the player provides too few:

```rust
impl CommandHandler for ChangePasswordCommand {
    fn execute<'a>(
        &'a self,
        ctx: CommandContext,
        player_registry: &'a dyn PlayerRegistry,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            let Some(player_id) = ctx.player_id else { return };
            let Some(player) = player_registry.get_player_by_id(player_id) else {
                return;
            };

            if ctx.args.len() < 2 {
                let _ = player.send_message(
                    Component::text("Usage: /changepassword <old> <new>").color("red"),
                );
                return;
            }

            let old_password = &ctx.args[0];
            let new_password = &ctx.args[1];

            // ... validate and update password
        })
    }
}
```

## Tab completion

Override `tab_complete` to provide suggestions when a player presses Tab. The default implementation returns no suggestions.

```rust
impl CommandHandler for MyCommand {
    fn execute<'a>(
        &'a self,
        ctx: CommandContext,
        player_registry: &'a dyn PlayerRegistry,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move { /* ... */ })
    }

    fn tab_complete(&self, partial_args: &[&str]) -> Vec<String> {
        match partial_args.len() {
            0 | 1 => vec!["survival".into(), "creative".into(), "lobby".into()],
            _ => vec![],
        }
    }
}
```

`partial_args` contains what the player has typed so far, split by whitespace.

## Sharing state with a handler

Commands often need access to plugin state. Store shared data in an `Arc` field on the handler struct:

```rust
use std::sync::Arc;

pub struct ChangePasswordCommand {
    pub handler: Arc<AuthHandler>,
}

impl CommandHandler for ChangePasswordCommand {
    fn execute<'a>(
        &'a self,
        ctx: CommandContext,
        player_registry: &'a dyn PlayerRegistry,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            let config = self.handler.config();
            let storage = self.handler.storage();
            // ... use shared state
        })
    }
}
```

Register it with `Arc::clone`:

```rust
ctx.command_manager().register(
    "changepassword",
    &["changepw", "cp"],
    "Change your auth password",
    Box::new(ChangePasswordCommand {
        handler: Arc::clone(&handler),
    }),
);
```

## Permission checks

The proxy has a two-level permission system (Player / Admin) configured in `[permissions]` in `infrarust.toml`. See the [Permissions configuration](../../configuration/security/permissions.md) page for how operators set up admins and open commands to players.

In your handler, check admin status with `player.has_permission("infrarust.admin")`:

```rust
fn is_admin(player_id: PlayerId, player_registry: &dyn PlayerRegistry) -> bool {
    player_registry
        .get_player_by_id(player_id)
        .is_some_and(|p| p.has_permission("infrarust.admin"))
}
```

Use it to gate admin-only commands:

```rust
let Some(player_id) = ctx.player_id else { return };
if !is_admin(player_id, player_registry) {
    if let Some(player) = player_registry.get_player_by_id(player_id) {
        let _ = player.send_message(Component::error("No permission."));
    }
    return;
}
```

You can also read the player's permission level directly:

```rust
use infrarust_api::permissions::PermissionLevel;

if let Some(player) = player_registry.get_player_by_id(player_id) {
    match player.permission_level() {
        PermissionLevel::Admin => { /* full access */ }
        PermissionLevel::Player => { /* restricted */ }
    }
}
```

### Custom permission checker

Plugins can replace the built-in config-based checker by listening to `PermissionsSetupEvent`. This fires after authentication, before the player session is constructed. If no listener provides a custom checker, the proxy uses its config-based default.

```rust
use infrarust_api::events::lifecycle::{PermissionsSetupEvent, PermissionsSetupResult};
use infrarust_api::permissions::PermissionChecker;

ctx.event_bus().subscribe(EventPriority::NORMAL, |event: &mut PermissionsSetupEvent| {
    let checker = MyDatabaseChecker::new(event.profile.uuid);
    event.set_result(PermissionsSetupResult::Custom(Arc::new(checker)));
});
```

Your checker must implement the `PermissionChecker` trait:

```rust
pub trait PermissionChecker: Send + Sync {
    fn permission_level(&self) -> PermissionLevel;
    fn has_permission(&self, permission: &str) -> bool;
}
```

This is how you'd integrate LuckPerms, a database, or any external permission backend.

## Organizing commands

For plugins with multiple commands, put each handler in its own module and register them from a single function:

```rust
pub mod changepassword;
pub mod forcelogin;
pub mod unregister;

pub fn register_commands(ctx: &dyn PluginContext, handler: Arc<AuthHandler>) {
    ctx.command_manager().register(
        "changepassword",
        &["changepw", "cp"],
        "Change your auth password",
        Box::new(changepassword::ChangePasswordCommand {
            handler: Arc::clone(&handler),
        }),
    );

    ctx.command_manager().register(
        "forcelogin",
        &[],
        "Force-authenticate a player in auth limbo",
        Box::new(forcelogin::ForceLoginCommand {
            handler: Arc::clone(&handler),
        }),
    );
}
```

Then call `register_commands(ctx, handler)` from your `on_enable`.

## Dispatch flow

When a player sends a chat message starting with `/`:

1. The proxy strips the leading `/` and splits the input by whitespace.
2. The first token is the command name, converted to lowercase.
3. If the name matches an alias, it resolves to the canonical command name.
4. If a handler is registered for that name, the proxy builds a `CommandContext` and calls `handler.execute()`.
5. The handler runs asynchronously. The message is not forwarded to the backend.
6. If no handler matches, the message passes through to the backend server as normal.
