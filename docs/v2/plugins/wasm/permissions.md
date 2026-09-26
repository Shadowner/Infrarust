---
title: Permissions
description: Give players permission snapshots from a WASM plugin, answer permissions-setup for one player, become the proxy's permission provider, and change a player's permissions while they are online.
outline: [2, 3]
---

# Permissions

A WASM plugin can decide what players are allowed to do, the way a LuckPerms-like native plugin does. It does so with **permission snapshots**: a list of node rules the host turns into the player's permission checker. There are three ways to hand one over:

| What you want | How | Needs |
|---------------|-----|-------|
| Replace the permissions of one player at login | Answer `PermissionsSetupEvent` with `provide(snapshot)` | `event-bus` (baseline) |
| Answer for every player and the console | Register a permission provider with `ctx.provide_permissions(...)` | `permission-provider`, and `[permissions] provider = "<your plugin id>"` |
| Change a player's permissions while they are online | `Permissions::set_snapshot(player, snapshot)` and `Permissions::release(player)` | `permission-provider` |

How the proxy resolves a node once a player has a checker, and how node defaults work, is the same for every plugin: see [Permissions](../dev/permissions) in the native guide.

## Snapshots

```rust
use infrarust_plugin_sdk::prelude::*;

let moderator = PermissionSnapshot::new()
    .grant("infrarust.command.kick")
    .grant("warps.*")
    .deny("warps.admin");
let operator = PermissionSnapshot::admin();
```

A snapshot answers a node like the native `PermissionMap`:

- The node is looked up exactly, then as `a.b.*`, `a.*` and finally `*`, one segment at a time. The first rule found wins, so the most specific rule beats a broader one.
- A node no rule covers is `Undefined`, and the proxy falls back to the node's registered default.
- `admin` answers `True` for every node, like an admin of the built-in provider or the console.

Node names are trimmed and lowercased on both sides of the boundary. A snapshot holds at most 65,536 rules.

| Rules | `warps.use` | `warps.admin` | `warps` | `other` |
|-------|-------------|---------------|---------|---------|
| `warps.* = true`, `warps.admin = false` | true | false | undefined | undefined |
| `* = true`, `warps.* = false` | false | false | true | true |

The host keeps each snapshot it installs for a player. That copy is what later updates change, and it lives outside the plugin's instance, so it survives a trap.

## One player at login

`PermissionsSetupEvent` fires after the active provider built the player's checker. `provide` replaces that checker for this player only:

```rust
ctx.on::<PermissionsSetupEvent>(EventPriority::Normal, |event| {
    if event.player.username == "Notch" {
        event.provide(PermissionSnapshot::admin());
    }
})?;
```

`use_default()` resets the result to the provider's checker, `result()` reads what earlier listeners chose. A custom checker a native plugin installed reaches the guest as `PermissionsSetupResult::Custom` with the snapshot it describes; a native checker that cannot describe itself shows as an empty snapshot. Returning the event untouched keeps it either way.

A player whose permissions come from this event keeps them for the whole session: a later `refresh_permissions` does not rebuild them from the provider. Change them with `set_snapshot`.

## Being the permission provider

The operator names the provider in `infrarust.toml` and grants the capability:

```toml
[permissions]
provider = "perms"

[plugins.perms]
permissions = ["permission-provider"]
```

The plugin implements `PermissionProvider` and registers it in `on_enable`:

```rust
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use infrarust_plugin_sdk::prelude::*;

#[derive(Clone, Default)]
struct Groups(Rc<RefCell<HashMap<String, PermissionSnapshot>>>);

impl Groups {
    fn of(&self, username: &str) -> PermissionSnapshot {
        self.0.borrow().get(username).cloned().unwrap_or_default()
    }
}

impl PermissionProvider for Groups {
    fn snapshot_for(&self, subject: &PermissionSubject) -> PermissionSnapshot {
        match subject.profile() {
            Some(profile) => self.of(&profile.username),
            None => PermissionSnapshot::admin(),
        }
    }
}

#[derive(Default)]
struct Perms;

#[plugin(id = "perms")]
impl Plugin for Perms {
    fn on_enable(&self, ctx: &Context) -> Result<(), PluginError> {
        ctx.provide_permissions(Groups::default())?;
        Ok(())
    }
}
```

The proxy then asks `snapshot_for` for every player that logs in, every player it refreshes, and the console. A `PermissionSubject::Player` carries the player id, profile, online mode, virtual host and address.

| Registration answer | Why |
|---------------------|-----|
| `Ok(())` | The plugin is the provider. The host refreshes every online player. |
| `ErrorKind::Conflict` | `[permissions] provider` names another plugin or `builtin`. |
| `ErrorKind::PermissionDenied` | The plugin lacks `permission-provider`. |

### When the plugin cannot answer

The host waits for `snapshot_for` at most the event budget (`[events] handler_timeout`). A trap, a missed deadline, a quarantined plugin or an oversized snapshot leaves the subject with an empty snapshot: every node falls back to its default. The host logs a warning naming the plugin and the subject. The player still holds a snapshot from this plugin, so `set_snapshot` can fix it once the plugin answers again.

## Changing a player while they are online

```rust
let promoted = PermissionSnapshot::new().grant("warps.*");
Permissions::set_snapshot(player.id(), &promoted)?;
```

`set_snapshot` replaces the snapshot the host holds for the player, and the proxy resends that player's command tree, so a command they gained or lost appears or disappears at once. The player must hold a snapshot from this plugin, either from the provider or from `PermissionsSetupEvent`:

| Answer | Why |
|--------|-----|
| `Ok(())` | Replaced; the command tree is refreshed. |
| `ErrorKind::NotFound` | The player holds no snapshot from this plugin, for example they joined before it registered as provider and got another plugin's checker. |
| `ErrorKind::PlayerGone` | The player is offline. |
| `ErrorKind::InvalidArgument` | More than 65,536 rules. |
| `ErrorKind::PermissionDenied` | The plugin lacks `permission-provider`. |

`Permissions::release(player)` gives the player up: their snapshot is cleared, so every node falls back to its default, and a later `set_snapshot` answers `NotFound`. Releasing a player the plugin holds nothing for succeeds. The host forgets a snapshot on its own when the player disconnects.

### Refreshes that start inside the plugin

The host never calls back into an instance that is still running. When a refresh starts from inside the provider's own call, the one `set_snapshot` triggers or a `Player::refresh_permissions` from a command handler, the host answers from the snapshot it holds instead of asking `snapshot_for`. To change what a player has, call `set_snapshot`.

## Recovery

A trap does not unregister the provider. While the plugin is quarantined, players who log in get the node defaults; online players keep the snapshot they had. The recovered instance runs `on_enable` again, and its `provide_permissions` succeeds without a second registration and without refreshing anyone. It starts from empty memory, so if it keeps state outside its data directory it can push fresh snapshots:

```rust
if let Some(EnableReason::Recovered(_)) = ctx.enable_reason() {
    for player in Players::list() {
        let _ = Permissions::set_snapshot(player.id(), &groups.of(&player.profile.username));
    }
}
```

Every player the provider answered for holds a snapshot from it, so these calls only answer `NotFound` for a player whose checker another plugin replaced at login.

## The contract

```wit
interface permissions {
    record permission-rule { node: string, value: bool }
    record permission-snapshot { rules: list<permission-rule>, admin: bool }
    variant permission-subject { player(player-subject), console }

    set-snapshot: func(player: player-id, snapshot: permission-snapshot) -> result<_, host-error>;
    release: func(player: player-id) -> result<_, host-error>;
}

interface providers {
    register-permission-provider: func() -> result<_, host-error>;
}
```

The guest exports `permission-snapshot-for: func(subject: permission-subject) -> permission-snapshot`, and `permissions-setup-result` has a `custom(permission-snapshot)` case. The full definitions are in the [WIT API reference](./api-reference#host-services).

## See also

- [Permissions](../dev/permissions): nodes, defaults, the native provider model.
- [Bans](./bans): the other provider a WASM plugin can be.
- [Capabilities](./capabilities): `permission-provider`.
