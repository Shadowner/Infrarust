---
title: Permissions
description: The permission provider model. One active provider answers permission nodes for every player and the console, plugins register the nodes they use with a default, and a player's permissions can be refreshed while they are online.
outline: [2, 3]
---

# Permissions

Every permission question the proxy asks is about a **permission node**: a dotted name such as `demo.use` or `infrarust.command.kick`. Can this player run `/demo`? Should `/ir kick` appear in their command tree? Does a plugin let them skip a queue? All of these are `has_permission(node)`.

One active **permission provider** answers those questions. The built-in provider reads `[permissions]` from `infrarust.toml`. A plugin (a LuckPerms port, a database-backed group system) can register its own provider and take over, and the proxy then asks that plugin about every player and the console.

```
 login / refresh_permissions()           console command
            │                                   │
            ▼                                   ▼
   active PermissionProvider ── create_checker(subject) ──► PermissionChecker
   (built-in or a plugin's)                                   value(node) -> Tristate
                                                                     │
                     PermissionsSetupEvent may replace it ◄──────────┘
                                                                     │
   has_permission(node): value, then the node's registered default, then false
```

| Type | Implemented by | Role |
|------|----------------|------|
| `PermissionProvider` | the built-in provider, or your plugin | Builds a checker for a player or the console |
| `PermissionChecker` | whoever built it | Answers `True`, `False` or `Undefined` for a node |
| `PermissionNode` | registered by plugins and the proxy | A node's name, description and default |
| `PermissionSubject` | the proxy | Who the checker is for: a player (with its connection) or the console |

All of them live in `infrarust_api::permissions` and are re-exported from the prelude, along with `Tristate`, `PermissionDefault`, `PermissionMap` and `ADMIN_PERMISSION`.

## How a node is resolved

`Player::has_permission(node)` and `CommandSource::has_permission(node)` resolve a node in this order:

1. The subject's checker: `checker.value(node)`. `True` and `False` are final.
2. If the checker says `Undefined`, the node's registered default:
   - `PermissionDefault::True`: granted.
   - `PermissionDefault::False`: denied.
   - `PermissionDefault::Admin`: granted when the same checker answers `True` for `infrarust.admin`.
3. If the node was never registered, denied.

Node names are case-insensitive. The proxy trims and lowercases a node before it asks a checker, so a provider only ever sees lowercase names.

`PermissionChecker::has_permission` on a checker you hold directly only tests `value(node) == Tristate::True`. It knows nothing about registered defaults. Ask the player or the command source when you want the proxy's answer.

### Wildcards

The resolver does not expand wildcards; that is the provider's job. The built-in provider, and the `PermissionMap` type any provider can reuse, follow one rule:

- A node is looked up exactly first, then as `a.b.*`, `a.*` and finally `*`, walking up one segment at a time. The first entry found wins, so the most specific entry beats a broader one.
- `a.b.*` covers `a.b.c` and `a.b.c.d`, but not `a.b` itself, and not `a.bc`.

| Entries | `demo.use` | `demo.kick` | `demo` | `other.node` |
|---------|-----------|-------------|--------|--------------|
| `demo.* = true` | true | true | undefined | undefined |
| `* = true`, `demo.* = false`, `demo.use = true` | true | false | true | true |

A plugin provider is free to implement a different matching rule. Document it for your users if you do.

## Registering nodes

Register the nodes your plugin checks, with the default a player gets when the provider has no opinion:

```rust
use infrarust_api::prelude::*;

ctx.register_permission_node(
    PermissionNode::new("demo.use", PermissionDefault::True).description("Run /demo"),
)?;
ctx.register_permission_node(
    PermissionNode::new("demo.reload", PermissionDefault::Admin).description("Reload the demo config"),
)?;
```

| Default | A player without an answer from the provider |
|---------|---------------------------------------------|
| `PermissionDefault::True` | has the node |
| `PermissionDefault::False` | does not have it |
| `PermissionDefault::Admin` | has it when they hold `infrarust.admin` |

Registration fails with a `PermissionNodeError`:

| Error | When |
|-------|------|
| `InvalidName` | The name is empty, has an empty segment (`a..b`), whitespace, or is a wildcard (`*`, `a.*`) |
| `Reserved` | The name starts with `infrarust.`, which belongs to the proxy |
| `OwnedBy { plugin, .. }` | Another plugin registered it first |

Registering a node your plugin already owns replaces its default and description. When the plugin is disabled, its nodes are removed with its commands and listeners.

You do not have to register a node to check it. An unregistered node has no default, so only a provider that answers `True` grants it. Registering it gives your users a sensible default and lets a provider list it: `ctx.permission_nodes()` returns every registered node with the plugin that owns it, which a permission plugin can use for its editor or tab completion.

### Nodes the proxy registers

| Node | Default |
|------|---------|
| `infrarust.admin` | `False`. Held by admins of the built-in provider |
| `infrarust.command.<name>` for each `/ir` subcommand | `True` when `<name>` (or `*`) is in `[permissions] player_commands` and the subcommand may be opened to players, `Admin` otherwise |

The subcommands `kick`, `send`, `broadcast`, `reload`, `plugin` and `plugins` are never opened by `player_commands`, but a plugin provider may grant their nodes to anyone, for example `infrarust.command.kick` to a moderator group.

## Choosing the provider

The operator picks the provider in `infrarust.toml`:

```toml
[permissions]
provider = "builtin"      # the default: admins and player_commands below
# provider = "luckperms"  # the plugin with this id answers
```

A plugin registers from `on_enable`:

```rust
ctx.register_permission_provider(Arc::new(MyProvider::new(store)))?;
```

See [Permissions configuration](../../configuration/security/permissions) for the operator side.

Registering needs the `permission-provider` capability, which compiled-in plugins hold by default. Only the plugin whose id matches `provider` is accepted; any other gets `PermissionProviderRejected::NotSelected { selected }` and the proxy logs a warning. Without the capability the call returns `PermissionProviderRejected::MissingCapability`. A WASM plugin can be the provider too, with permission snapshots: see [WASM permissions](../wasm/permissions).

When a provider registers while players are online, and when the provider plugin is disabled, the proxy refreshes every online player so they move to the new answers.

### When the selected plugin is missing

If `provider` names a plugin that never registers (it failed to load, is disabled, or the id is misspelled), the proxy logs an error at startup and every player gets a checker that answers `Undefined` for every node. Only the registered defaults apply, nobody holds `infrarust.admin`, and nodes with an `Admin` default are denied.

This fails safe rather than closed: permissions only ever grant, so a missing provider grants the least it can while players can still join. The operator does not lose the proxy either, because the console keeps every permission when the selected provider is missing, and the console's own commands (`op`, `kick`, `ban`, `stop`) never go through permissions.

## Writing a provider

A provider builds one checker per subject. The proxy calls `create_checker`:

- at login, for every player in every proxy mode, legacy clients included, before `PermissionsSetupEvent` and `LoginEvent`;
- when something calls `refresh_permissions()` on the player;
- for the console, each time the console runs a command.

`value` is synchronous and called often: on every command, every completion and every command tree rebuild. Load what you need in `create_checker`, which is async, and answer from memory in `value`.

```rust
use std::collections::HashMap;
use infrarust_api::prelude::*;

struct GroupProvider {
    store: Arc<GroupStore>,
}

impl PermissionProvider for GroupProvider {
    fn create_checker<'a>(
        &'a self,
        subject: &'a PermissionSubject,
    ) -> BoxFuture<'a, Arc<dyn PermissionChecker>> {
        Box::pin(async move {
            let Some(profile) = subject.profile() else {
                return Arc::new(PermissionMap::new().with("*", true)) as Arc<dyn PermissionChecker>;
            };
            let mut nodes = PermissionMap::new();
            for (node, value) in self.store.nodes_for(profile.uuid).await {
                nodes.set(&node, value);
            }
            Arc::new(nodes) as Arc<dyn PermissionChecker>
        })
    }
}
```

`PermissionSubject` tells you who the checker is for:

| Accessor | Player | Console |
|----------|--------|---------|
| `player_id()` | `Some(id)` | `None` |
| `profile()` | `Some(&GameProfile)` after `GameProfileRequestEvent` rewrites | `None` |
| `is_online_mode()` | whether the proxy authenticated the player with Mojang | `false` |
| `virtual_host()` | the domain the client connected to | `None` |
| `remote_addr()` | the client address, from the PROXY protocol header when there is one | `None` |
| `is_console()` | `false` | `true` |

The enum is `#[non_exhaustive]`; keep a fallback arm when you match on it.

Offline-mode and passthrough players report `is_online_mode() == false`. Their UUID comes from the name they claimed, so decide whether your provider trusts it before granting anything sensitive.

### Refreshing a player

When a player's groups change, ask the proxy to rebuild their checker:

```rust
if let Some(player) = ctx.player_registry().get_player_by_uuid(&uuid) {
    player.refresh_permissions().await;
}
```

`refresh_permissions()` calls your provider again, swaps the player's checker, and sends that player, and only that player, a rebuilt command tree, so commands they gained or lost appear or disappear right away. Until you refresh, the player keeps the checker built at login. The console's `op` and `deop` commands refresh the player they change.

## Overriding one player

`PermissionsSetupEvent` still works as a per-player override on top of the provider. It fires after the provider built the player's checker. A listener that sets `PermissionsSetupResult::Custom(checker)` replaces that checker for this player only; `UseDefault` keeps the provider's.

```rust
ctx.event_bus().subscribe(EventPriority::NORMAL, |event: &mut PermissionsSetupEvent| {
    if event.profile().username == "Notch" {
        event.set_result(PermissionsSetupResult::Custom(Arc::new(
            PermissionMap::new().with("*", true),
        )));
    }
});
```

A custom checker stays in place for the whole session. `refresh_permissions()` on that player does not call the provider; it only resends the command tree, so a custom checker whose answers change can push them with a refresh.

To answer for every player, register a provider instead of listening to this event.

## Admins

There are no permission levels. A player is an admin when they hold `infrarust.admin`:

```rust
if player.has_permission(ADMIN_PERMISSION) {
    // full access
}
```

For WASM plugins, `players.has-permission` (`Player::has_permission` in the SDK) resolves a node the same way, `infrarust.admin` included.

## The console

The console is a subject too. The built-in provider gives it every node. With a plugin provider, the console's checker comes from `create_checker(&PermissionSubject::Console)`, and the console's commands are resolved like a player's, registered defaults included. A plugin command the console may not run answers `The console may not run '<name>'.`

A handler sees the console as `CommandSource::Console(checker)`, and `ctx.source.has_permission(node)` goes through that checker.

## Reloading

`[permissions]` is read once at startup. Changing `provider`, `admins`, `player_commands` or `trust_offline_admins` needs a restart. At runtime, use the console's `op` and `deop`, or your provider's own storage and `refresh_permissions()`.

## Moving from permission levels

| Before | Now |
|--------|-----|
| `player.permission_level() == PermissionLevel::Admin` | `player.has_permission(ADMIN_PERMISSION)` |
| `impl PermissionChecker { fn permission_level(); fn has_permission() }` | `impl PermissionChecker { fn value(&self, node) -> Tristate }` |
| `CommandSource::Console` | `CommandSource::Console(checker)`, or `CommandSource::console(checker)` |
| A checker from `PermissionsSetupEvent` for every player | A `PermissionProvider` selected by `[permissions] provider` |
