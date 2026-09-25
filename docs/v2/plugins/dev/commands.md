---
title: Commands
description: Register commands that players and the console can run, with aliases, namespaced names, permission nodes, and tab completion.
outline: [2, 3]
---

# Commands

Plugins register commands through the `CommandManager`. When a player types `/yourcommand`, or an operator types `yourcommand` at the proxy console, the proxy runs your handler. The command never reaches the backend server.

## Registering a command

Describe the command with a `CommandSpec` and pass it with a handler to `register()`, usually from `on_enable`:

```rust
use infrarust_api::prelude::*;

let spec = CommandSpec::new("hello")
    .aliases(["hi", "hey"])
    .description("Says hello to the player")
    .usage("/hello");

match ctx.command_manager().register(spec, Box::new(HelloCommand)) {
    Ok(registration) if !registration.rejected_aliases.is_empty() => {
        tracing::warn!("aliases skipped: {:?}", registration.rejected_aliases);
    }
    Ok(_) => {}
    Err(e) => tracing::warn!("/hello was not registered: {e}"),
}
```

`CommandSpec` is a builder. Every method takes `self` and returns it:

| Method | Effect |
|--------|--------|
| `CommandSpec::new(name)` | The primary name |
| `alias(name)` | Add one alias |
| `aliases(names)` | Add several aliases |
| `description(text)` | Help text, shown by `/ir plugin <id>` and the console `help` |
| `usage(text)` | Usage line, for your own help output |
| `permission(node)` | Permission node the sender needs, see [Permission nodes](#permission-nodes) |
| `hidden(true)` | Keep the command out of the client command tree; it still runs |

Names and aliases are case-insensitive and stored in lowercase. A name must not be empty and must not contain whitespace or `:`, or start with `/`; otherwise `register` returns `CommandError::InvalidName`.

### Names, aliases, and conflicts

The proxy keeps one table for every plugin, so names are shared. These rules decide who gets a name:

- Every plugin command is also registered as `<plugin_id>:<name>`, for example `auth:changepassword`. That form always reaches your command, whatever other plugins register.
- The built-in names and aliases (`infrarust`, `ir`) are reserved. Registering one of them returns `CommandError::Reserved`.
- A bare name goes to the first plugin that registers it. A later plugin that asks for the same name gets `CommandError::OwnedBy { name, plugin }` and nothing is registered for it.
- An alias that is reserved, already taken, or invalid is skipped. The command is still registered with its other labels.
- Registering a name your plugin already owns replaces your earlier command.

`register` returns a `CommandRegistration` on success:

| Field | Type | Description |
|-------|------|-------------|
| `name` | `String` | The primary name, lowercased |
| `namespaced` | `String` | The `<plugin_id>:<name>` form |
| `aliases` | `Vec<String>` | Aliases that were accepted |
| `rejected_aliases` | `Vec<String>` | Aliases that were skipped |

`CommandError` has four variants:

| Variant | Returned by | Meaning |
|---------|-------------|---------|
| `Reserved(name)` | `register` | The name belongs to a built-in command |
| `OwnedBy { name, plugin }` | `register` | Another plugin registered that name first |
| `InvalidName(name)` | `register` | The name is empty, contains whitespace or `:`, or starts with `/` |
| `NotOwned(name)` | `unregister` | No command of this plugin answers to that name |

### Unregistering

```rust
if let Err(e) = ctx.command_manager().unregister("hello") {
    tracing::debug!("nothing to remove: {e}");
}
```

`unregister` accepts the name, an alias, or the namespaced name, and removes the whole command with all its labels. It only removes commands your plugin registered: for any other name it returns `CommandError::NotOwned` and changes nothing.

When your plugin is disabled, the proxy unregisters every command it registered. A registration that failed was never recorded, so it cannot remove anything at cleanup.

### Registering later

`ctx.command_manager()` borrows from the context, which you only hold during `on_enable`. To register or unregister commands afterwards, from a scheduled task or an event listener, keep the owned handle:

```rust
let commands = ctx.command_manager_handle();
ctx.scheduler().delay(
    std::time::Duration::from_secs(60),
    Box::new(move || {
        let _ = commands.register(CommandSpec::new("event"), Box::new(EventCommand));
    }),
);
```

The handle is bound to your plugin, so the same ownership rules apply and cleanup still covers it. Players who are already connected see the new command straight away, see [The client command tree](#the-client-command-tree).

`list()` returns a `CommandInfo` for every registered command, built-ins and other plugins included. `CommandInfo::plugin_id` is `None` for built-ins, and `namespaced()` returns the `<plugin_id>:<name>` form.

## CommandHandler

Implement `CommandHandler` on a struct. `execute` receives a `CommandContext` and returns a `BoxFuture`; wrap your async block with `Box::pin(async move { ... })`.

```rust
use infrarust_api::prelude::*;

struct HelloCommand;

impl CommandHandler for HelloCommand {
    fn execute<'a>(&'a self, ctx: CommandContext) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            ctx.source.send_message(
                Component::text("Hello from Infrarust! ")
                    .color("gold")
                    .bold()
                    .append(Component::text("Welcome to the proxy.").color("gray")),
            );
        })
    }
}
```

A player's command runs inside that player's session loop, so keep `execute` short and `tokio::spawn` anything slow.

## CommandSource

`ctx.source` says who ran the command:

```rust
pub enum CommandSource {
    Player(Arc<dyn Player>),
    Console,
}
```

The enum is `#[non_exhaustive]`, so add a wildcard arm when you match on it. Most handlers only need its methods:

| Method | Returns | Description |
|--------|---------|-------------|
| `name()` | `&str` | The player's username, or `"Console"` |
| `send_message(component)` | `()` | Chat message to the player; for the console, an `info` log line under the `infrarust::console` target |
| `has_permission(node)` | `bool` | The player's permission checker; the console holds every permission |
| `player()` | `Option<&Arc<dyn Player>>` | The player, or `None` for the console |
| `player_id()` | `Option<PlayerId>` | The player's id |
| `is_console()` | `bool` | Whether the console ran the command |

## CommandContext

| Field | Type | Description |
|-------|------|-------------|
| `source` | `CommandSource` | Who ran the command |
| `label` | `String` | The label as typed: the name, an alias, or the namespaced form |
| `args` | `Vec<String>` | Arguments split by whitespace |
| `raw_args` | `String` | Everything after the label, as typed |
| `raw` | `String` | The whole command without the leading `/` |

If a player types `/cp oldpass newpass`, your handler receives `label = "cp"`, `args = ["oldpass", "newpass"]`, `raw_args = "oldpass newpass"`, and `raw = "cp oldpass newpass"`.

`CommandContext` is `#[non_exhaustive]`. In tests, build one with `CommandContext::new(source, label, raw_args)` or `CommandContext::parse(source, "cp oldpass newpass")`.

## Parsing arguments

Check `args.len()` before indexing, and reply with the usage when the sender gave too few. A command that only makes sense for a player can refuse the console:

```rust
impl CommandHandler for ChangePasswordCommand {
    fn execute<'a>(&'a self, ctx: CommandContext) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            let Some(player) = ctx.source.player() else {
                ctx.source
                    .send_message(Component::error("Only players can use this command."));
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
            // validate and update the password
        })
    }
}
```

## Tab completion

Override `suggest` to answer tab completion. The default returns no suggestions.

```rust
impl CommandHandler for WarpCommand {
    fn execute<'a>(&'a self, ctx: CommandContext) -> BoxFuture<'a, ()> {
        Box::pin(async move { /* ... */ })
    }

    fn suggest<'a>(&'a self, ctx: SuggestContext) -> BoxFuture<'a, Vec<Suggestion>> {
        Box::pin(async move {
            let prefix = ctx.partial().to_lowercase();
            ["spawn", "arena", "shop"]
                .into_iter()
                .filter(|warp| warp.starts_with(&prefix))
                .map(|warp| {
                    Suggestion::new(warp).with_tooltip(Component::text(format!("Warp to {warp}")))
                })
                .collect()
        })
    }
}
```

`SuggestContext` has the same `source`, `label`, and `raw_args` as `CommandContext`, and `args` holds the arguments typed so far. When the input ends with a space the last element is an empty string, so you can tell that a new argument has started. `partial()` returns that last element. Each `Suggestion` carries its text and an optional tooltip `Component`, which the client shows on hover.

The proxy answers a completion request for a proxy command itself and never forwards it to the backend. If the player lacks the command's permission node, `suggest` is not called and the answer is empty.

## Permission nodes

Give a command a node with `CommandSpec::permission`:

```rust
let spec = CommandSpec::new("forcelogin").permission("auth.forcelogin");
```

Before the handler runs, the proxy calls `ctx.source.has_permission(node)`. When that returns `false`, the sender gets `[Infrarust] You don't have permission to use this command.`, the handler does not run, and the command is not forwarded to the backend. The command is also left out of that player's command tree and completions. The console passes every check.

With the config-based checker, `infrarust.admin` is held by admins, `infrarust.command.<name>` by admins and by everyone when `<name>` is listed in `player_commands`, and any other node by admins only. See the [Permissions configuration](../../configuration/security/permissions.md) page for how operators set up admins.

For finer decisions, check inside the handler:

```rust
if !ctx.source.has_permission("infrarust.admin") {
    ctx.source.send_message(Component::error("No permission."));
    return;
}
```

You can also read a player's level directly:

```rust
use infrarust_api::permissions::PermissionLevel;

if let Some(player) = ctx.source.player() {
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

This is how you'd integrate LuckPerms, a database, or any external permission backend. Permission nodes on commands go through the same checker.

## The client command tree

Clients from 1.13 on build their command suggestions from a command tree the backend sends. When `announce_proxy_commands` is on (the default), the proxy rewrites that tree for each player:

- It adds every proxy command the player may run: the name, each alias, and the `<plugin_id>:<name>` form, each taking a free-form argument that asks the proxy for completions.
- It leaves out commands marked `hidden(true)` and commands whose permission node the player lacks.
- It removes a backend command with the same name as a proxy command label, because the proxy would intercept it anyway. The player sees each root command once.

The proxy keeps the last tree each backend sent. When any command is registered or unregistered, every connected player in an intercepted mode gets a rebuilt tree right away, without waiting for the backend to send a new one.

## Sharing state with a handler

Commands often need plugin state. Store it in an `Arc` field on the handler struct:

```rust
use std::sync::Arc;

pub struct ChangePasswordCommand {
    pub handler: Arc<AuthHandler>,
}

impl CommandHandler for ChangePasswordCommand {
    fn execute<'a>(&'a self, ctx: CommandContext) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            let config = self.handler.config();
            let storage = self.handler.storage();
            // use the shared state
        })
    }
}
```

A handler that looks up other players keeps a registry handle, taken with `ctx.player_registry_handle()` at registration time.

## Organizing commands

For plugins with several commands, put each handler in its own module and register them from one function:

```rust
pub mod changepassword;
pub mod forcelogin;

pub fn register_commands(ctx: &dyn PluginContext, handler: Arc<AuthHandler>) {
    let commands = ctx.command_manager();
    let specs: Vec<(CommandSpec, Box<dyn CommandHandler>)> = vec![
        (
            CommandSpec::new("changepassword")
                .aliases(["changepw", "cp"])
                .description("Change your auth password"),
            Box::new(changepassword::ChangePasswordCommand {
                handler: Arc::clone(&handler),
            }),
        ),
        (
            CommandSpec::new("forcelogin").description("Force-authenticate a player in auth limbo"),
            Box::new(forcelogin::ForceLoginCommand {
                handler: Arc::clone(&handler),
            }),
        ),
    ];
    for (spec, command) in specs {
        let name = spec.name.clone();
        if let Err(e) = commands.register(spec, command) {
            tracing::warn!("/{name} was not registered: {e}");
        }
    }
}
```

Then call `register_commands(ctx, handler)` from your `on_enable`.

## Dispatch flow

Players and the console go through the same dispatch:

1. A player's command arrives as a command packet (1.19 and later) or as a chat message starting with `/`. At the console, a line that is not a console command is tried as a proxy command; a leading `/` is optional.
2. The proxy splits off the first word as the label and looks it up case-insensitively among built-in names, plugin names, aliases, and `<plugin_id>:<name>` forms.
3. If nothing matches, a player's command goes to the backend unchanged, or to the limbo handler's `on_command` while the player is in limbo. The console prints `Unknown command`.
4. If the command has a permission node the sender lacks, the sender gets the denial message and dispatch stops there.
5. Otherwise the proxy builds a `CommandContext` and awaits `handler.execute()`. The command is not forwarded.

The console lists plugin commands at the end of `help`. Console commands keep their names, so when a plugin command shares a name with one of them, run it at the console with its `<plugin_id>:<name>` form.
