---
title: Sharing Services Between Plugins
description: Expose an API from one native Infrarust plugin and use it from another through the service registry.
outline: [2, 3]
---

# Sharing Services Between Plugins

A plugin can publish an API for other plugins through the service registry. A permissions plugin can expose a `Permissions` trait that other plugins call. An auth plugin can expose an "is this player logged in" check. The consumer depends on the trait, not on the plugin that implements it.

The registry is keyed by Rust type. It works for native plugins compiled into the same binary (the `StaticPluginLoader`), where a `TypeId` means the same thing on both sides. WASM plugins have no access to it.

## Define the API

Put the trait in a crate that both plugins depend on, so they agree on the type:

```rust
pub trait LoginState: Send + Sync {
    fn is_logged_in(&self, username: &str) -> bool;
}
```

The registry stores an `Arc<T>` where `T` can be a trait object (`dyn LoginState`) or a concrete type. Registering a trait object is the usual choice: consumers never see the implementation.

## Provide it

Call `provide` on `ctx.services()`. The generic methods come from `ServiceRegistryExt`, which the prelude brings in:

```rust
use infrarust_api::prelude::*;

struct AuthPlugin {
    sessions: Arc<Sessions>,
}

impl Plugin for AuthPlugin {
    fn metadata(&self) -> PluginMetadata {
        PluginMetadata::new("auth", "Auth", "1.0.0")
    }

    fn on_enable<'a>(&'a self, ctx: &'a dyn PluginContext) -> BoxFuture<'a, Result<(), PluginError>> {
        Box::pin(async move {
            ctx.services()
                .provide::<dyn LoginState>(self.sessions.clone())?;
            Ok(())
        })
    }
}
```

`provide` returns a `ServiceHandle`. Dropping the handle keeps the service registered; call `handle.withdraw()` to remove it earlier. When the plugin is disabled, every service it provided is removed.

`ServiceError` converts into `PluginError::InitFailed`, so `?` works inside `on_enable`.

Each type has one provider. The first plugin to provide a type keeps it, and a second `provide` for the same type fails with `ServiceError::AlreadyProvided { service, by }`, where `by` is the ID of the plugin that holds it.

## Use it

`get` returns `Option<Arc<T>>`:

```rust
if let Some(logins) = ctx.services().get::<dyn LoginState>() {
    if !logins.is_logged_in("Steve") {
        // ...
    }
}
```

`provider::<T>()` returns the ID of the plugin that currently provides `T`, if any.

A service can come and go while the proxy runs: its provider can be disabled, or enabled after the consumer. Don't cache the `Arc` for the lifetime of your plugin. Keep a registry handle and look the service up when you need it:

```rust
let services = ctx.services_handle();
ctx.event_bus().subscribe(EventPriority::NORMAL, move |event: &mut PostLoginEvent| {
    let logged_in = services
        .get::<dyn LoginState>()
        .is_some_and(|logins| logins.is_logged_in(&event.profile.username));
    // ...
});
```

A consumer that looks the service up in its own `on_enable` needs the provider to be enabled first. Declare the provider as a dependency: `.depends_on("auth")` if your plugin cannot work without it, `.optional_dependency("auth")` if it can. An optional dependency that is present still loads first.

## React to changes

The proxy posts two events when the registry changes:

| Event | When |
|-------|------|
| `ServiceProvidedEvent` | A plugin provided a service |
| `ServiceRemovedEvent` | A service was withdrawn, or its provider was disabled |

Both carry `service` (the type name, for logs), `provider` (the plugin ID) and an `is::<T>()` check:

```rust
ctx.event_bus().subscribe(EventPriority::NORMAL, |event: &mut ServiceRemovedEvent| {
    if event.is::<dyn LoginState>() {
        tracing::warn!("{} stopped providing logins", event.provider);
    }
});
```

These events are reserved: only the proxy fires them. See [Events](./events#plugin-events).

## Example: a permissions API

A permissions plugin that stores groups can expose them to other plugins:

```rust
pub trait Groups: Send + Sync {
    fn groups_of(&self, player: &uuid::Uuid) -> Vec<String>;
    fn add_to_group(&self, player: &uuid::Uuid, group: &str) -> Result<(), String>;
}

// in the permissions plugin
ctx.services().provide::<dyn Groups>(store.clone())?;

// in a chat plugin
let prefix = ctx
    .services()
    .get::<dyn Groups>()
    .and_then(|groups| groups.groups_of(&player.profile().uuid).into_iter().next());
```

Deciding whether a player holds a permission node is a different job, done by a [permission provider](./permissions). The service registry is for the plugin's own API on top of that.

## Reference

| Method | On | Returns |
|--------|----|---------|
| `provide::<T>(Arc<T>)` | `ServiceRegistryExt` | `Result<ServiceHandle, ServiceError>` |
| `get::<T>()` | `ServiceRegistryExt` | `Option<Arc<T>>` |
| `provider::<T>()` | `ServiceRegistryExt` | `Option<String>` |
| `withdraw()` | `ServiceHandle` | `bool`, `false` if it was already gone |
| `service()` | `ServiceHandle` | The type name |
| `services()` | `PluginContext` | `&dyn ServiceRegistry` |
| `services_handle()` | `PluginContext` | `Arc<dyn ServiceRegistry>` |
