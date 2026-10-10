---
title: Permissions
description: Give players permission snapshots from a WASM plugin, answer permissions-setup for one player, become the proxy's permission provider, change a player's permissions while they are online, and register the plugin's own permission nodes.
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

Any plugin, provider or not, can also register the nodes it checks, each with a default, and read every registered node. That needs no capability: see [Permission nodes](#permission-nodes).

## Snapshots

```rust
use infrarust_plugin_sdk::prelude::*;

let moderator = PermissionSnapshot::new()
    .grant("infrarust.command.kick")
    .grant("warps.*")
    .deny("warps.admin");
let operator = PermissionSnapshot::admin();
```

Once a snapshot is installed, the host answers a node from it like the native `PermissionMap`:

- The node is looked up exactly, then as `a.b.*`, `a.*` and finally `*`, one segment at a time. The first rule found wins, so the most specific rule beats a broader one.
- A node no rule covers is `Undefined`, and the proxy falls back to the node's registered default.
- `admin` answers `True` for every node, like an admin of the built-in provider or the console.

Node names are trimmed and lowercased on both sides of the boundary. A snapshot holds at most 65,536 rules.

Inside the guest, a snapshot is a list of rules, not a checker: `get(node)` returns the rule stored for that exact node, or `None`. It resolves no wildcard and ignores `admin`; `is_admin()` reads that flag. Only the host resolves nodes.

| Rules | `warps.use` | `warps.admin` | `warps` | `other` |
|-------|-------------|---------------|---------|---------|
| `warps.* = true`, `warps.admin = false` | true | false | undefined | undefined |
| `* = true`, `warps.* = false` | false | false | true | true |

The host keeps each snapshot it installs for a player. That copy is what later updates change, and it lives outside the plugin's instance, so it survives a trap.

## One player at login

`PermissionsSetupEvent` fires after the active provider built the player's checker. `provide` replaces that checker for this player only:

```rust
ctx.on::<PermissionsSetupEvent>(EventPriority::NORMAL, |event| {
    if event.player.username == "Notch" {
        event.provide(PermissionSnapshot::admin());
    }
})?;
```

`use_default()` resets the result to the provider's checker, `result()` reads what earlier listeners chose. A custom checker a native plugin installed reaches the guest as `PermissionsSetupResult::Custom` with the snapshot it describes; a native checker that cannot describe itself shows as an empty snapshot. Returning the event untouched keeps it either way.

`PermissionsSetupEvent` is an [access event](./events#a-listener-that-does-not-answer): when the plugin's listener gives no answer (it runs past the event deadline, traps, finds its queue full, or the plugin is recovering or quarantined), the player gets an empty checker, so every node falls back to its default, as when the permission provider fails. `set_snapshot` does not reach that checker: the player keeps it until they log in again.

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

`snapshot_for` has the deadline of an event, `[events] handler_timeout` minus a margin (a fifth, at most 250 ms). A call still running at the deadline is cut off, counted as a [fault](./fault-model) and the instance is replaced, so only that subject goes without an answer; the questions queued behind it reach the fresh instance. A trap, a cut-off call, a full queue, a quarantined plugin or an oversized snapshot leaves a player with an empty snapshot: every node falls back to its default. The host logs a warning naming the plugin and the subject. The player still holds a snapshot from this plugin, so `set_snapshot` can fix it once the plugin answers again.

The console is the exception. When the provider cannot answer for the console, the console keeps every permission, as it does when no provider is registered, and the warning says so. The console belongs to the proxy's operator, and the proxy asks the provider again for each console line that reaches a proxy or plugin command, so the operator keeps those commands while the permission plugin is down and gets the provider's answer again once it recovers. Players get no such fallback.

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

A trap does not unregister the provider. While the plugin is quarantined, players who log in get the node defaults and the console keeps every permission; online players keep the snapshot they had. The recovered instance runs `on_enable` again, and its `provide_permissions` succeeds without a second registration and without refreshing anyone. It starts from empty memory. A plugin that keeps its groups somewhere that outlives the instance, such as a file in its data directory, can load them again and push fresh snapshots. Here `Groups::load` stands for that reading:

```rust
fn on_enable(&self, ctx: &Context) -> Result<(), PluginError> {
    let groups = Groups::load();
    ctx.provide_permissions(groups.clone())?;
    if let Some(EnableReason::Recovered(_)) = ctx.enable_reason() {
        for player in Players::list() {
            let _ = Permissions::set_snapshot(player.id(), &groups.of(&player.player.username));
        }
    }
    Ok(())
}
```

A recovered instance that pushed the empty groups of `Groups::default()` would strip every online player of their permissions, so push only what was loaded.

Every player the provider answered for holds a snapshot from it, so these calls only answer `NotFound` for a player whose checker another plugin replaced at login.

## Permission nodes

Register the nodes your plugin checks, each with the default a player gets when the provider has no answer for it, as a native plugin does. Here the plugin id is `warps`:

```rust
fn on_enable(&self, ctx: &Context) -> Result<(), PluginError> {
    ctx.register_permission_node(
        PermissionNode::new("warps.use", PermissionDefault::True).description("Use /warp"),
    )?;
    ctx.register_permission_node(
        PermissionNode::new("warps.admin", PermissionDefault::Admin)
            .description("Create and delete warps"),
    )?;
    Ok(())
}
```

`PermissionDefault::True` grants the node, `False` denies it, and `Admin` grants it to a subject that holds `infrarust.admin`. The full resolution order is in [How a node is resolved](../dev/permissions#how-a-node-is-resolved). Node names are trimmed and lowercased.

`ctx.permission_node(name)` returns one registered node and the id of the plugin that owns it, or `None` when nobody registered it. `ctx.permission_nodes()` returns every registered node, sorted by name. Both see the proxy's nodes, such as `infrarust.admin`, whose `plugin_id` is `None`, and the nodes of every other plugin, native or WASM. A permission plugin can use them for an editor or for tab completion:

```rust
for info in ctx.permission_nodes() {
    let owner = info.plugin_id.as_deref().unwrap_or("the proxy");
    info!("{} from {owner}: {:?}", info.node.name, info.node.default);
}
```

### Only your own namespace

A WASM plugin registers only nodes whose name starts with its plugin id and a dot. The `warps` plugin may register `warps.use` and `warps.admin.delete`, but not `fly`, `essentials.fly` or `warpsplus.use`. Plugin ids never contain a dot, so two plugins' namespaces never overlap. Any other name is refused with `ErrorKind::InvalidArgument`, and the message names the prefix the plugin may use.

The rule exists because a default reaches every player the provider has no answer for. A plugin may check a node it never registers, or registers only later in its `on_enable`. If any plugin could register that node, a WASM plugin could register it first with `PermissionDefault::True` and so grant it to every player. WASM plugins are not trusted, so they set defaults only under their own name. Native plugins are trusted and have no such rule.

The native rules still apply inside the namespace:

| Answer | Why |
|--------|-----|
| `Ok(())` | The node is registered, or updated when the plugin already owns it. |
| `ErrorKind::InvalidArgument` | The name is outside the plugin's namespace, or it is not a valid node: an empty segment (`warps..use`), whitespace, or a wildcard (`warps.*`). |
| `ErrorKind::Conflict` | A native plugin registered the name first, or the name starts with `infrarust.`, which belongs to the proxy. |
| `ErrorKind::LimitExceeded` | The plugin holds its quota of nodes. |

The host logs a refused name as a warning naming the plugin and the node, at most five lines a minute for each plugin instance. A quota refusal is logged like any other, naming the quota.

### Quota and lifetime

A plugin holds at most `[wasm.quotas] permission_nodes` nodes, 256 by default. Registering a node the plugin already owns replaces its description and default and takes no more room, so `on_enable` can register the same nodes each time it runs. A node another plugin owns does not count against yours.

Nodes live on the host. They are kept across a [recovery](./fault-model) and keep counting: the recovered instance's `on_enable` registers them again without taking more of the quota, and a node it does not register again keeps what the old instance set. There is no call to unregister a node. The host removes all of a plugin's nodes when the plugin is disabled or unloaded, so their defaults stop applying.

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

interface permission-nodes {
    enum permission-default { %false, %true, admin }
    record permission-node { name: string, description: string, %default: permission-default }
    record permission-node-info { node: permission-node, plugin-id: option<string> }

    register: func(node: permission-node) -> result<_, host-error>;
    get: func(name: string) -> option<permission-node-info>;
    %list: func() -> list<permission-node-info>;
}
```

The guest exports `permission-snapshot-for: func(subject: permission-subject) -> permission-snapshot`, and `permissions-setup-result` has a `custom(permission-snapshot)` case. The full definitions are in the [WIT API reference](./api-reference#host-services).

## See also

- [Permissions](../dev/permissions): nodes, defaults, the native provider model.
- [Bans](./bans): the other provider a WASM plugin can be.
- [Capabilities](./capabilities): `permission-provider`, and the [registration quotas](./capabilities#registration-quotas).
