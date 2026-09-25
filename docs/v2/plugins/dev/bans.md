---
title: Bans
description: The ban provider model. Every ban check and ban command goes through one active provider, the built-in one or a plugin's, so a ban plugin can own enforcement end to end.
outline: [2, 3]
---

# Bans

Infrarust does not hard-wire where bans live. Every ban check the proxy makes, and every ban issued from the console, the admin API or a plugin, goes through one active **ban provider**. The built-in provider keeps bans in `bans.json`. A plugin such as a LibertyBans port can register its own provider and take over: the proxy then asks that plugin whether a player may join, and the built-in list is not consulted at all.

```
 login (pre-auth / post-auth)          console, admin API, plugins
            │                                     │
            ▼                                     ▼
   ┌──────────────────────── BanService (facade) ────────────────────────┐
   │  check()        ban() / unban() / get() / list()                     │
   │                 fills in the source, fires BanIssued / BanRevoked,   │
   │                 kicks matching online players                        │
   └──────────────────────────────┬───────────────────────────────────────┘
                                  ▼
             active BanProvider: built-in, a plugin's, or none
```

Two traits split the work:

| Trait | Implemented by | Role |
|-------|----------------|------|
| `BanProvider` | the built-in store, or your plugin | Decides who is banned and stores bans. Not sealed. |
| `BanService` | the proxy only (sealed) | The facade every caller uses. It picks the active provider, attributes each ban to whoever issued it, posts ban events and kicks online players. |

Both live in `infrarust_api::services::ban_service` and are re-exported from the prelude.

## Choosing the provider

The operator picks the provider in `infrarust.toml`:

```toml
[ban]
provider = "builtin"   # the default: bans.json
# provider = "none"    # no ban checks at all
# provider = "libertybans"  # the plugin with this id provides bans
```

Only the plugin whose id matches `provider` becomes active. Any other plugin that registers a provider gets `BanProviderRejected::NotSelected` back, and the proxy logs a warning. See [Bans configuration](../../configuration/security/bans#choosing-a-provider) for the operator side.

### When the selected plugin is missing

If `provider` names a plugin that never registers a provider (it failed to load, is disabled, or the id is misspelled), the proxy logs an error at startup and **refuses logins** with "Your ban status cannot be checked right now" until the provider registers. The same happens while a registered provider's `check` returns an error, and after the provider plugin is disabled at runtime.

This is fail-closed, on purpose:

- The operator told the proxy that a specific plugin decides who is banned. Letting everyone in because that plugin is absent would silently void every ban it holds, and banned players would get in while the error scrolls past in the log.
- One rule covers every case: when the proxy cannot tell whether a player is banned, the player does not get in. A database outage inside a working provider and a provider that never loaded behave the same way.
- `provider = "none"` exists for operators who want no ban checks, so fail-open is always one config line away and never an accident.

Status pings are the exception. The server list check fails open: a ping from an address the proxy cannot check still gets its MOTD, so an outage does not make the server look offline.

## Writing a provider

Implement `BanProvider` and register it from `on_enable`. Registering needs the `ban-provider` capability, which compiled-in plugins hold by default.

```rust
use infrarust_api::prelude::*;

struct LibertyBans {
    store: Arc<Store>,
}

impl BanProvider for LibertyBans {
    fn check<'a>(
        &'a self,
        attempt: &'a LoginAttempt,
    ) -> BoxFuture<'a, Result<Option<BanVerdict>, ServiceError>> {
        Box::pin(async move {
            // Pre-auth attempts carry the IP and the name the client claims.
            // Wait for the authenticated profile before matching UUID bans.
            if attempt.stage != LoginStage::PostAuth {
                return self.store.address_ban(attempt.ip).await.map(|b| b.map(verdict));
            }
            let Some(uuid) = attempt.uuid else {
                return Ok(None);
            };
            self.store.applicable(uuid, attempt.ip).await.map(|b| b.map(verdict))
        })
    }

    fn ban(&self, request: BanRequest) -> BoxFuture<'_, Result<BanEntry, ServiceError>> {
        Box::pin(async move { self.store.insert(request).await })
    }

    fn unban(&self, request: UnbanRequest) -> BoxFuture<'_, Result<Option<BanEntry>, ServiceError>> {
        Box::pin(async move { self.store.remove(request).await })
    }

    fn get<'a>(
        &'a self,
        target: &'a BanTarget,
    ) -> BoxFuture<'a, Result<Option<BanEntry>, ServiceError>> {
        Box::pin(async move { self.store.find(target).await })
    }

    fn list(&self, query: BanQuery) -> BoxFuture<'_, Result<BanPage, ServiceError>> {
        Box::pin(async move { self.store.page(query.cursor, query.effective_limit()).await })
    }

    fn features(&self) -> BanFeatures {
        BanFeatures::new().ip_ranges(true).pagination(true)
    }
}

fn verdict(ban: StoredBan) -> BanVerdict {
    let message = Component::text(format!("Banned: {}\nAppeal at example.org", ban.reason));
    BanVerdict::new(ban.into_entry()).message(message)
}

// in on_enable
if let Err(rejected) = ctx.register_ban_provider(Arc::new(LibertyBans { store })) {
    tracing::warn!("not the active ban provider: {rejected}");
}
```

Every method returns a boxed future, so a provider can query a database, call a web service or wait on a lock without blocking the proxy. Return `ServiceError::Unavailable` when your backend is down; the proxy then refuses the login rather than guessing.

### What the proxy asks

`check` receives a `LoginAttempt`:

| Field | Meaning |
|-------|---------|
| `stage` | `Status` (server list ping), `PreAuth` (login start, before authentication) or `PostAuth` (the final profile is known) |
| `ip` | The player's real address: the PROXY protocol source when `receive_proxy_protocol` is on. IPv4-mapped IPv6 addresses are already turned into plain IPv4. |
| `username` | The name the client sent. `None` for `Status`. |
| `uuid` | At `PostAuth`, the profile UUID. At `PreAuth`, the UUID the client claimed, if any. |
| `uuid_verified` | `true` only when Mojang authenticated the profile (client-only mode). Offline and forwarded profiles are not verified. |
| `virtual_host` | The domain the client connected to |
| `server` | The server the login is routed to, when known |

The proxy checks at these points:

| Where | Stage | What a verdict does |
|-------|-------|---------------------|
| Server list ping, modern and legacy | `Status` | The connection is closed without an answer |
| Login pipeline, after the login start packet | `PreAuth` | The client is disconnected in the login state with the verdict's message |
| Legacy (pre-1.7) login | `PreAuth` | Legacy kick with the verdict's message |
| Client-only and offline modes, after authentication and `GameProfileRequestEvent` | `PostAuth` | Login-state disconnect with the verdict's message |
| Passthrough, zero-copy, server-only and legacy logins, after `GameProfileRequestEvent` | `PostAuth` | Login-state disconnect with the verdict's message |

A provider that only matches UUIDs can return `Ok(None)` for anything that is not `PostAuth`. A provider that ignores pings returns `Ok(None)` for `Status`.

The message you put in `BanVerdict::message` is what the player sees. `BanVerdict::new(entry)` builds the default text from the entry's reason and remaining time.

### Features

`features()` tells the facade what the provider supports:

| Feature | When `false` |
|---------|--------------|
| `ip_ranges` | The facade refuses `BanTarget::IpRange` bans with `ServiceError::OperationFailed` before calling you |
| `pagination` | Callers expect `list` to return everything in one page with no cursor |

### Pagination

`list` takes a `BanQuery` with an opaque `cursor` and a `limit` (`effective_limit()` clamps it to 1..=1000). Return the next cursor in `BanPage::next_cursor`, or `None` on the last page. A cursor must stay valid while bans are added and removed: the built-in provider uses its ever-growing ban ids, so a client walking the pages never sees an entry twice and never skips one that survives.

`BanService::list_all()` walks every page for callers that need the whole list.

## Using the ban service

Plugins get the facade from their context. The same service works whichever provider is active.

```rust
use std::time::Duration;
use infrarust_api::prelude::*;

let bans = ctx.ban_service();

let entry = bans
    .ban(
        BanRequest::new(BanTarget::Username("griefer".into()))
            .reason("Griefing")
            .duration(Duration::from_secs(3600)),
    )
    .await?;

bans.ban(BanRequest::new(BanTarget::IpRange("203.0.113.0/24".parse()?)).reason("Botnet"))
    .await?;

let active = bans.get(&BanTarget::Username("griefer".into())).await?;
let removed = bans.unban(UnbanRequest::new(BanTarget::Username("griefer".into()))).await?;
let first_page = bans.list(BanQuery::new().limit(50)).await?;
```

| Method | Returns |
|--------|---------|
| `check(&LoginAttempt)` | The verdict the active provider gives for this attempt |
| `ban(BanRequest)` | The stored `BanEntry`, with the id the provider gave it |
| `unban(UnbanRequest)` | The removed entry, or `None` if the target was not banned |
| `get(&BanTarget)` | The active ban on exactly this target |
| `list(BanQuery)` / `list_all()` | A page of active bans / every active ban |
| `features()` | The active provider's features |

With `provider = "none"`, `check` lets everyone in and the other methods return `ServiceError::Unavailable`.

### Requests

`BanRequest::new(target)` starts a permanent ban that kicks matching players. Builder methods set the rest:

| Method | Default | Effect |
|--------|---------|--------|
| `reason(text)` | none | Shown to the player and stored with the ban |
| `duration(d)` | permanent | The ban expires after `d` |
| `source(BanSource)` | the calling plugin | Who issued the ban |
| `kick(bool)` | `true` | Disconnect online players the target matches |
| `silent(bool)` | `false` | Passed to the provider and to the ban events, for plugins that announce bans |

`UnbanRequest::new(target)` takes `source` and `silent` the same way.

### Targets

| `BanTarget` | Matches |
|-------------|---------|
| `Ip(addr)` | One address. An IPv4 ban also matches the same address seen as `::ffff:a.b.c.d`. |
| `IpRange(net)` | Every address in a CIDR block, IPv4 or IPv6 (`ipnet::IpNet`, re-exported as `IpNet`). A range written as IPv4-mapped IPv6 is stored as its IPv4 range. |
| `Username(name)` | A player name, case-insensitive |
| `Uuid(uuid)` | A profile UUID |

`BanTarget::matches(&attempt)` applies these rules for you.

### Sources

Each ban records who issued it as a `BanSource`:

| Source | Set by |
|--------|--------|
| `Console` | The proxy console commands |
| `WebApi { actor }` | The admin API |
| `Plugin(id)` | Any plugin that does not set a source: the facade fills in the calling plugin's id |
| `Player { uuid, name }` | A plugin acting for a staff member, e.g. an in-game `/ban` command |
| `System` | The proxy itself, and any caller of the proxy-wide service that names no source |

A plugin can set any source, for example `BanSource::Player` when a moderator runs its command.

## Kicking online players

When a ban has `kick` set, the facade looks for online players the target matches: by real IP for `Ip` and `IpRange`, by name or UUID otherwise. For each one it asks the active provider's `check` with a `PostAuth` attempt for that player and disconnects them with the verdict's message, through the same path as `Player::disconnect`, so plugins see a normal `DisconnectEvent` with cause `kicked`. When the provider has no verdict for that player, the default message built from the new entry is used.

## Events

The facade posts two events through the event bus queue, in the order the operations happened:

| Event | Fields | Posted when |
|-------|--------|-------------|
| `BanIssuedEvent` | `entry`, `source`, `silent` | A provider stored a ban |
| `BanRevokedEvent` | `entry` (the removed ban), `source` (who removed it), `silent` | A provider removed a ban. Not posted when there was nothing to remove. |

`BanIssuedEvent` is posted before any player is kicked. Both events are proxy events: plugins can listen to them but not fire them. See the [Events reference](./events#ban-events).

## Plugging in a ban plugin

A plugin that wants to own bans, LibertyBans for example, does four things:

1. Implement `BanProvider` over its own storage, returning its own ids, reasons and messages.
2. Call `ctx.register_ban_provider(...)` in `on_enable` and handle `NotSelected` by staying passive.
3. Ask the operator to set `[ban] provider = "<plugin id>"`.
4. Issue bans from its own commands through `ctx.ban_service()`, with `BanSource::Player` for moderators, so the proxy kicks online players and posts `BanIssuedEvent`.

From then on the proxy console, the admin API and every other plugin read and write bans through that plugin. Nothing is copied into `bans.json`.

WASM plugins can use the ban service through the `ban-service` interface (see [WASM services](../wasm/services#bans)) but cannot provide bans yet.
