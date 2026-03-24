---
title: Getting Started with Plugin Development
description: Build your first Infrarust plugin from scratch — project setup, the Plugin trait, event listeners, and commands.
outline: [2, 3]
---

# Getting Started with Plugin Development

This tutorial walks you through creating a plugin that logs player connections and adds a `/greet` command. By the end you'll have a working plugin compiled into the proxy binary.

## Prerequisites

- Rust toolchain (edition 2024)
- The Infrarust source code checked out locally
- Familiarity with `async`/`await` in Rust

## Project setup

Infrarust plugins are regular Rust crates that depend on `infrarust-api`. Create a new crate inside the `plugins/` directory:

```bash
mkdir plugins/infrarust-plugin-greet
cd plugins/infrarust-plugin-greet
cargo init --lib
```

Replace the generated `Cargo.toml` with:

```toml
[package]
name = "infrarust-plugin-greet"
version = "0.1.0"
edition = "2024"

[dependencies]
infrarust-api = { path = "../../crates/infrarust-api" }
tracing = "0.1"
```

`infrarust-api` provides the `Plugin` trait, events, commands, and every type you need. `tracing` is the logging framework used across Infrarust.

## The Plugin trait

Every plugin implements `Plugin` from the prelude. The trait has three parts:

- `metadata()` — returns your plugin's id, name, and version.
- `on_enable()` — called at startup with a `PluginContext`. Register your listeners, commands, and handlers here.
- `on_disable()` — called at shutdown. Optional; the default implementation does nothing. All listeners and commands you registered are automatically cleaned up.

Open `src/lib.rs` and replace the contents:

```rust
use infrarust_api::prelude::*;

pub struct GreetPlugin;

impl Plugin for GreetPlugin {
    fn metadata(&self) -> PluginMetadata {
        PluginMetadata::new("greet", "Greet Plugin", "0.1.0") // [!code focus]
            .author("YourName")
            .description("Logs connections and adds a /greet command")
    }

    fn on_enable<'a>(
        &'a self,
        ctx: &'a dyn PluginContext,
    ) -> BoxFuture<'a, Result<(), PluginError>> {
        Box::pin(async move {
            // We'll fill this in next
            tracing::info!("[GreetPlugin] Enabled");
            Ok(())
        })
    }
}
```

The `metadata()` method uses a builder pattern. The `id` must be unique, `snake_case`, and is used internally for dependency resolution and resource tracking. `PluginMetadata::new` takes three required fields (id, name, version), and you chain `.author()`, `.description()`, `.depends_on()`, or `.optional_dependency()` as needed.

::: warning
`on_enable` and `on_disable` return `BoxFuture` because they're async but the trait isn't `async_trait`. Wrap your async block in `Box::pin(async move { ... })`.
:::

## Listening to events

The `PluginContext` gives you access to an event bus. Subscribe to any event type with a priority and a closure.

Add this inside your `on_enable` async block, before the `Ok(())`:

```rust
ctx.event_bus()
    .subscribe(EventPriority::NORMAL, |event: &mut PostLoginEvent| {
        tracing::info!("[GreetPlugin] {} joined the proxy!", event.profile.username);
    });

ctx.event_bus()
    .subscribe(EventPriority::NORMAL, |event: &mut DisconnectEvent| {
        tracing::info!("[GreetPlugin] {} left the proxy", event.username);
    });
```

The event type in the closure signature determines which events you receive. The proxy fires `PostLoginEvent` after a player authenticates and `DisconnectEvent` when they leave.

### Event priorities

Priorities control the order listeners run when multiple plugins listen to the same event:

| Priority | Value | Use case |
|----------|-------|----------|
| `FIRST`  | 0     | Runs before all others. Use for access control or authentication checks. |
| `EARLY`  | 64    | Before normal processing. |
| `NORMAL` | 128   | Default. Most plugins should use this. |
| `LATE`   | 192   | After normal processing. |
| `LAST`   | 255   | Runs after all others. Use for logging or analytics. |

You can also use `EventPriority::custom(u8)` for arbitrary values.

### Modifying events

Some events implement `ResultedEvent`, which means you can change the outcome. For example, `ChatMessageEvent` lets you block a message:

```rust
ctx.event_bus()
    .subscribe(EventPriority::NORMAL, |event: &mut ChatMessageEvent| {
        if event.message.contains("spam") {
            event.deny(Component::text("Message blocked"));
        }
    });
```

## Registering a command

Commands are registered through `ctx.command_manager()`. You provide a name, optional aliases, a description, and a handler struct that implements `CommandHandler`.

Add the command registration inside `on_enable`, after the event subscriptions:

```rust
ctx.command_manager().register(
    "greet",
    &["hi", "hey"],
    "Sends a greeting to the player",
    Box::new(GreetCommand),
);
```

Then define the handler struct outside the `impl Plugin` block:

```rust
struct GreetCommand;

impl CommandHandler for GreetCommand {
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
                    Component::text("Hello from Infrarust! ") // [!code focus]
                        .color("gold")
                        .bold()
                        .append(Component::text("Welcome to the proxy.").color("gray")),
                );
            }
        })
    }
}
```

`CommandContext` carries the `player_id` (if a player ran it, `None` for console) and parsed `args`. The `PlayerRegistry` lets you look up the player and send messages, switch servers, or disconnect them.

## Adding on_disable

`on_disable` is optional. The proxy automatically unsubscribes your event listeners, unregisters commands, and cancels scheduled tasks. Override it only if your plugin holds external resources (file handles, database connections) that need explicit cleanup:

```rust
fn on_disable(&self) -> BoxFuture<'_, Result<(), PluginError>> {
    Box::pin(async {
        tracing::info!("[GreetPlugin] Disabled");
        Ok(())
    })
}
```

## Wiring the plugin into the proxy

Infrarust compiles plugins statically via Cargo features. You need to make three changes.

### 1. Add the dependency to infrarust-proxy

In `crates/infrarust-proxy/Cargo.toml`, add your plugin as an optional dependency and create a feature flag:

```toml{3,7}
[features]
default = []
plugin-greet = ["dep:infrarust-plugin-greet"]

[dependencies]
# ... existing dependencies ...
infrarust-plugin-greet = { path = "../../plugins/infrarust-plugin-greet", optional = true }
```

### 2. Register the plugin in the static loader

In `crates/infrarust-proxy/src/plugins.rs`, add a registration block:

```rust
#[cfg(feature = "plugin-greet")]
{
    use infrarust_api::plugin::Plugin;
    let greet = infrarust_plugin_greet::GreetPlugin;
    loader.register(greet.metadata(), || {
        Box::new(infrarust_plugin_greet::GreetPlugin)
    });
}
```

### 3. Build with the feature enabled

```bash
cargo build --release --features "plugin-greet"
```

## Complete source

Here's the full `src/lib.rs`:

```rust
use infrarust_api::prelude::*;

pub struct GreetPlugin;

impl Plugin for GreetPlugin {
    fn metadata(&self) -> PluginMetadata {
        PluginMetadata::new("greet", "Greet Plugin", "0.1.0")
            .author("YourName")
            .description("Logs connections and adds a /greet command")
    }

    fn on_enable<'a>(
        &'a self,
        ctx: &'a dyn PluginContext,
    ) -> BoxFuture<'a, Result<(), PluginError>> {
        Box::pin(async move {
            ctx.event_bus()
                .subscribe(EventPriority::NORMAL, |event: &mut PostLoginEvent| {
                    tracing::info!("[GreetPlugin] {} joined!", event.profile.username);
                });

            ctx.event_bus()
                .subscribe(EventPriority::NORMAL, |event: &mut DisconnectEvent| {
                    tracing::info!("[GreetPlugin] {} left", event.username);
                });

            ctx.command_manager().register(
                "greet",
                &["hi", "hey"],
                "Sends a greeting to the player",
                Box::new(GreetCommand),
            );

            tracing::info!("[GreetPlugin] Enabled");
            Ok(())
        })
    }

    fn on_disable(&self) -> BoxFuture<'_, Result<(), PluginError>> {
        Box::pin(async {
            tracing::info!("[GreetPlugin] Disabled");
            Ok(())
        })
    }
}

struct GreetCommand;

impl CommandHandler for GreetCommand {
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

## Next steps

- [Plugin Overview](../index.md) — Full list of what plugins can do.
- Study the built-in [hello plugin](https://github.com/Shadowner/Infrarust/tree/main/plugins/infrarust-plugin-hello) for examples of limbo handlers and scheduled tasks.
- Check the `infrarust_api::prelude` module for the full list of available types, events, and services.
