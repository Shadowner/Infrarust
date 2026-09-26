---
title: Events Reference
description: Complete reference of all proxy events, their payloads, results, and usage examples.
outline: [2, 3]
---

# Events Reference

Infrarust fires events at key points in a player's lifecycle, from initial connection through disconnect. Your plugin subscribes to these events through the `EventBus`, and for resulted events, you can modify the outcome.

## Player lifecycle

The events a player goes through depend on the proxy mode of the server they join. Before any of them, every connection goes through the same [handshake](#before-the-login).

### Before the login

```
connection accepted
  → transport filters ── Reject, panic, timeout ──▶ ConnectionRejectedEvent (plugin)
  → IP filter ──────────────────────── refused ──▶ ConnectionRejectedEvent (ip_filter)
  → handshake read
  → IP ban, for a server list ping ─── banned ───▶ ConnectionRejectedEvent (ip_banned)
  → rate limit ─────────────────────── exceeded ─▶ ConnectionRejectedEvent (rate_limit)
  → domain routing ─────────────────── unknown ──▶ ConnectionRejectedEvent (unknown_domain)
                   ─── server's IP filter ──▶ ConnectionRejectedEvent (ip_filter)
  → ConnectionHandshakeEvent ── Deny, DropSilently ──▶ ConnectionRejectedEvent (plugin)
  → server list ping: ProxyPingEvent
  → login: name and IP ban ─────────── banned ───▶ ConnectionRejectedEvent (banned, ip_banned)
           PreLoginEvent, and the flows below
```

A connection that reaches `ConnectionHandshakeEvent` passed the proxy's own checks. A connection the proxy refuses, before or after that event, posts exactly one [`ConnectionRejectedEvent`](#connectionrejectedevent) and never becomes a player. Refusals from the login on (`PreLoginEvent` or `LoginEvent` denied, the UUID ban after authentication) do not post it: their own events report them.

A server list ping to an unknown domain is answered with `[default_motd]` when [`unknown_domain_behavior`](../../configuration/global#unknown-domain-behavior) is `default_motd`, so it goes through `ConnectionHandshakeEvent` with no server. A login to an unknown domain, or any connection to one with `drop`, is refused.

Clients older than 1.7 go through the IP filter, then their own path: a legacy ping checks IP bans, then fires `ConnectionHandshakeEvent`; a legacy login is refused for an unknown domain, then fires `ConnectionHandshakeEvent`, then checks name and IP bans. The rate limit does not apply to them.

### `offline` and `client_only`

The proxy runs the login itself, so every step can be observed and most can be refused:

```
PreLoginEvent ─────────────── Denied ──▶ disconnected during login
  → authentication (client_only: encryption and session server)
                            ─ failed ──▶ OnlineAuthFailed, disconnected
  → GameProfileRequestEvent    the profile can be rewritten
  → ban check (IP, name, final UUID) ─ banned ──▶ disconnected during login
  → PermissionsSetupEvent
  → LoginEvent ──────────────── Denied ──▶ disconnected during login
  → client_only: LoginSuccess sent with the final profile
  → player registered
  → PostLoginEvent
  → PlayerChooseInitialServerEvent
  → ServerPreConnectEvent (cause: initial)
  → limbo gate, when the server or a listener asks for one (LimboEnterEvent → LimboExitEvent)
  → server wake, for a managed server ── unavailable ──▶ KickedFromServerEvent
  → backend login ────────────── refused ──▶ no ServerConnectedEvent
  → ServerConnectedEvent       the backend accepted the login
  → ServerPostConnectEvent     the server's JoinGame reached the client
  → play (ChatMessageEvent, CommandExecuteEvent, raw packets,
          switches: ServerPreConnectEvent → ServerConnectedEvent → ServerPostConnectEvent)
  → DisconnectEvent
```

### Passthrough, `zero_copy` and `server_only`

The backend runs the login and the proxy only forwards bytes. Until it forwards them, the proxy holds the client's handshake and login start and fires the same events as the modes above, in the same order:

```
PreLoginEvent ─────────────── Denied ──▶ disconnected during login
  → GameProfileRequestEvent    online_mode is false
  → ban check (IP, name, final UUID) ─ banned ──▶ disconnected during login
  → PermissionsSetupEvent      online_mode is false
  → LoginEvent ──────────────── Denied ──▶ disconnected during login
  → player registered
  → PostLoginEvent
  → PlayerChooseInitialServerEvent
  → ServerPreConnectEvent (cause: initial) ── Denied ──▶ disconnected during login
  → server wake, for a managed server ── unavailable ──▶ KickedFromServerEvent
  → backend connection ─── unreachable ──▶ KickedFromServerEvent
  → ServerConnectedEvent       the login packets were sent to the backend
  → forwarding
  → DisconnectEvent
```

`full` is not implemented yet and falls back to passthrough, so it follows this flow too. Nothing reaches the backend before `ServerConnectedEvent`: a player refused by any of these events never opens a backend connection. What differs from `offline` and `client_only`:

- `PreLoginEvent`: `Denied` is honored. `ForceOffline` and `ForceOnline` are ignored and logged at debug level, since the backend runs its own login.
- `GameProfileRequestEvent` fires with `online_mode: false` and the offline profile (see [`[auth] offline_uuid`](../../configuration/global#authentication)). The profile left in the event is the player's identity on the proxy: the UUID ban check, the player registry and every later event use it, and BungeeCord or BungeeGuard forwarding sends its UUID and properties (skin textures, for example) to the backend. The backend still runs its own login with the name the client sent, and the client receives the backend's `LoginSuccess`, so a changed name or UUID only exists on the proxy unless forwarding carries it.
- `PermissionsSetupEvent` and `LoginEvent` fire with `online_mode: false`.
- The player is not active: `is_active()` is `false`, and `send_message`, `send_title`, `send_action_bar`, `send_packet`, `send_plugin_message`, `send_plugin_message_to_backend` and `switch_server` return `PlayerError::NotActive`. No plugin message or client state event fires, and `client_brand()`, `settings()` and `ping()` return `None`. `disconnect` works: until the proxy has connected to the backend, the client gets the reason in a login disconnect; after that, the proxy closes the connection without a message.
- `PlayerChooseInitialServerEvent`: `Redirect` is honored. `ServerPreConnectEvent`: `Allowed`, `ConnectTo` and `Denied` are honored.
- `SendToLimbo`, from either event, disconnects the player with "Limbo is not available on this server" and logs a warning: limbo needs the proxy to run the login, which only `offline` and `client_only` do. The `DisconnectEvent` cause is `Kicked` with that reason.
- A redirect (`Redirect`, `ConnectTo`, or `RedirectTo` from `KickedFromServerEvent`) must target a server in a forwarding mode. Forwarding the login to an `offline` or `client_only` server would skip the login the proxy runs for it, so the player is disconnected with "This server cannot be joined from here" and a warning is logged. An unknown server disconnects the player with "Unknown server".
- `KickedFromServerEvent` fires only when the backend cannot be reached or the [server manager](#server-wake) cannot start it (`cause` is `Unreachable`), or the connection drops while the login packets are sent (`ConnectionLost`). `during_connect` is `true` and `previous_server` is `None`. `reason` is `None`, except for a server the server manager could not start, where it is the proxy's message. Nothing reached the client yet, so `RedirectTo` works: it goes through `ServerPreConnectEvent` with the cause `KickRedirect`, and after three redirects in a row that failed, the next one is handled as `DisconnectPlayer { reason: None }`. The default result is `DisconnectPlayer { reason: None }`, which shows the event's `reason`, or the server's `disconnect_message` when it is `None`. `Notify` has no server to keep the player on and disconnects with its message. `SendToLimbo` is handled as `DisconnectPlayer { reason: None }` and logs a warning. When the player ends up disconnected, the `DisconnectEvent` cause is `Error` for an unreachable server and `BackendClosed` for a lost connection.

The proxy never reads the backend's packets in these modes, which changes three things:

- `ServerConnectedEvent` fires once the TCP connection to the backend is open and the login packets were forwarded, not when the backend accepts the login. A backend that then refuses the player still got a `ServerConnectedEvent`.
- `ServerPostConnectEvent` never fires, because the proxy does not see the `JoinGame` packet. `current_server()` is set right after `ServerConnectedEvent`.
- A backend that refuses the login, kicks the player or closes the connection ends the session with the `DisconnectEvent` cause `BackendClosed { reason: None }`. No `KickedFromServerEvent` fires, and the client gets whatever the backend sent.

### Clients older than 1.7

Clients that speak the pre-1.7 protocol are always forwarded, since the proxy cannot run their login. A legacy login (the `0x02` handshake) goes through the same events as the forwarding modes above, with these differences:

- `PreLoginEvent` carries the name from the legacy handshake, and its host as `server_domain`. `protocol_version` is the legacy protocol number (78 for 1.6.4). These numbers overlap the modern ones (47 is both 1.4.2 and 1.8).
- Name and IP bans are checked before `PreLoginEvent`, as the login pipeline does for newer clients, and the final UUID after `GameProfileRequestEvent`.
- Disconnects use the legacy kick packet, with the reason as legacy text: `Component::to_legacy('§')`.
- The legacy handshake is forwarded as the client sent it: domain rewrite and player info forwarding do not apply, so the profile from `GameProfileRequestEvent` never reaches the backend.

Legacy server list pings fire `ProxyPingEvent` with `legacy: true`, see [ProxyPingEvent](#proxypingevent).

### Guarantees

- Every event up to and including `PostLoginEvent` is awaited: the login waits for all listeners before it moves on.
- A login that ends before `PostLoginEvent` (denied, banned, failed authentication, disconnected by `player.disconnect` during an earlier event, or cut short by a proxy shutdown) never creates a player, so no `DisconnectEvent` follows.
- Once `PostLoginEvent` has fired, `DisconnectEvent` fires exactly once for that player, whatever ends the session: the client leaving, a kick, a denied or failed initial connection, the backend closing, a proxy shutdown or an error.
- During `PostLoginEvent` the player is already in the player registry (`get_player_by_id`, `get_player` and `get_player_by_uuid` find it) and `current_server()` is `None`, because the player has not been routed yet.
- A `player.disconnect(reason)` made during `PostLoginEvent` disconnects the client in the state it is in (the login phase for `offline` and the forwarding modes, before any server is chosen) with that reason, and the player's `DisconnectEvent` follows. In `offline` and `client_only`, messages, titles and action bars sent during `PostLoginEvent` wait and reach the client once it has joined the game.
- When a player logs in with a UUID that is already online, the proxy disconnects the first session with "You logged in from another location" and waits for its `DisconnectEvent` before the new session's `PostLoginEvent`, for at most `[events] disconnect_deadline`.
- `DisconnectEvent` is awaited, and the player leaves the registry once its listeners are done. The whole dispatch is bounded by [`disconnect_deadline`](../../configuration/global#plugin-event-handlers) (15 seconds by default): listeners still running then are cancelled, and the player is removed anyway.

### Server connections

These hold for `offline` and `client_only`. Passthrough modes differ as described above.

- `PlayerChooseInitialServerEvent`, `ServerPreConnectEvent`, `ServerConnectedEvent` and `ServerPostConnectEvent` are awaited in the player's session, after `PostLoginEvent` and before `DisconnectEvent`. For one connection attempt they fire in that order.
- `ServerPreConnectEvent` fires before the proxy opens any connection to the target, exactly once per attempt: the initial connection, every switch, a limbo handler sending the player to another server, a kicked player being redirected. A limbo gate on the chosen server does not fire it again when it lets the player through.
- `ServerConnectedEvent` fires once the target backend accepted the login. It never fires for a backend that refused the login or never answered. A switch can still fail after it, for example when the backend closes during the configuration phase: no `ServerPostConnectEvent` follows and a [`KickedFromServerEvent`](#kickedfromserverevent) fires.
- `ServerPostConnectEvent` fires once the server's `JoinGame` packet reached the client. From then on `current_server()` returns that server.
- `previous_server` is the server the player was on when the attempt started: `None` for the initial connection, including after an initial limbo gate, and `Some(a)` for a switch from `a`.
- `current_server()` stays `None` until the first `ServerPostConnectEvent`, also while a limbo gate holds the player before their first server. The player already counts toward that server in `PlayerRegistry::online_count_on` and `get_players_on_server`, in the status player count and in the server manager's idle detection.
- A switch to the server the player is already on does nothing and fires no event.
- `DisconnectEvent::last_server` is the last server the player joined, `None` if they never got a `ServerPostConnectEvent`.

### Server wake

A server with a [`[server_manager]`](../../guide/server-management) section is started when the proxy is about to connect a player to it, once `ServerPreConnectEvent` has picked it: the initial connection, a switch, a limbo exit or a kick redirect, in every proxy mode. This holds for the forwarding modes and for clients older than 1.7 too.

- Nothing starts a server for a player refused before (`PreLoginEvent`, `LoginEvent`, a ban, a `ServerPreConnectEvent` denial) or sent to another server by `PlayerChooseInitialServerEvent`, `ServerPreConnectEvent` or `KickedFromServerEvent`.
- A limbo gate on the server runs before the wake. The [server wake plugin](../builtin/server-wake) is such a gate: it starts the server itself and releases the player once it is online, so the proxy finds it online.
- While the server starts, the proxy holds the connection and forwards nothing: the client stays on its loading screen for the initial connection, and on its current server for a switch. A client gives up after about 30 seconds without packets, so hold players in limbo, with the server wake plugin for example, when a server starts slower than that.
- A server that is stopping, does not start within its `start_timeout`, or whose provider fails, is a failed connection: [`KickedFromServerEvent`](#kickedfromserverevent) fires with `cause` `Unreachable`, `during_connect` `true` and `reason` set to "Server is shutting down, please try again later.", "Server failed to start in time. Please try again." or "Server is unavailable. Please try again later.". A listener can redirect the player or send them to limbo, and the default result shows that message. No `ConnectionRejectedEvent` is posted, since the player exists by then.
- Once a start succeeds, the server's addresses begin their [slow start](../../configuration/load-balancing#slow-start) ramp before the proxy picks the one to connect to.

### Proxy shutdown

When the proxy stops (a signal, the `stop` console command, or a plugin cancelling `proxy_shutdown()`), it goes through these steps in order:

1. It stops accepting connections.
2. It ends every connection. In `offline` and `client_only`, a player is disconnected with "Proxy is shutting down", in whatever phase it is in (login, configuration or play). In passthrough modes, a player whose traffic is already forwarded only sees the connection close, because the proxy does not write into the forwarded stream. Each player's `DisconnectEvent` fires with the cause `Shutdown` while every plugin is still enabled and subscribed. A login still in progress gets the same message and ends without a `PostLoginEvent`, so it gets no `DisconnectEvent` either. Server list pings and connections that have not finished their handshake are closed.
3. It waits for every connection to finish, for at most 30 seconds. Each `DisconnectEvent` is still bounded by `[events] disconnect_deadline`, so a listener that hangs holds the shutdown only that long.
4. It fires `ProxyShutdownEvent` and waits for its listeners. By then no player is online, unless the 30 seconds ran out.
5. It delivers the queued events that were posted before this point (`ServerStateChangeEvent`, `BackendHealthEvent`, `ConfigReloadEvent`, `ConnectionRejectedEvent`).
6. It disables the plugins, in reverse load order: `on_disable` runs, then the plugin's listeners, commands, scheduled tasks and config providers are removed.
7. It stops its background work: the file and Docker providers and their watchers, active health probes, the expired ban purge and server manager monitoring.

A plugin can save per-player state in its `DisconnectEvent` listener and global state in `ProxyShutdownEvent` or `on_disable`. The services from the plugin context, such as the player registry or the ban service, still work in `on_disable`.

## Subscribing to events

Get the event bus from your `PluginContext` in `on_enable`:

```rust
use infrarust_api::prelude::*;

ctx.event_bus().subscribe::<PostLoginEvent, _>(
    EventPriority::NORMAL,
    |event| {
        tracing::info!("Player joined: {}", event.profile.username);
    },
);
```

For async work, use `subscribe_async`:

```rust
ctx.event_bus().subscribe_async::<DisconnectEvent, _>(
    EventPriority::NORMAL,
    |event| Box::pin(async move {
        save_player_data(event.player_id()).await;
    }),
);
```

Both methods return a `ListenerHandle` you can pass to `event_bus().unsubscribe(handle)` to remove the listener. `unsubscribe` returns `true` when it removed something. A plugin can only remove its own listeners: passing a handle that another plugin registered does nothing, returns `false`, and logs a warning.

## When a listener fails

A listener that panics is skipped: the proxy logs the panic with your plugin ID and passes the event on to the next listener. Anything the listener changed on the event before panicking is kept, so set the result last if a later step might fail.

When the failing listener was handling an event that another plugin fired (see [custom events](#custom-events)), the log line and the handler diagnostic also carry `fired_by`, the ID of the plugin that fired it. For events the proxy fires, `fired_by` is `infrarust`.

Async listeners also have a time limit, `handler_timeout` in the [`[events]`](../../configuration/global#plugin-event-handlers) section (10 seconds by default, `packet_handler_timeout` for raw packet listeners). A listener still running at the deadline is cancelled and the event continues without it. A listener that takes longer than `slow_handler_threshold` (1 second by default) is logged as slow. Synchronous listeners can't be interrupted, so keep blocking work out of them.

## Priority

Listeners run in priority order. Lower values run first.

| Constant | Value | When to use |
|----------|-------|-------------|
| `FIRST`  | 0     | Security checks, logging that must see the original event |
| `EARLY`  | 64    | Validation before normal processing |
| `NORMAL` | 128   | Default. Most plugins use this |
| `LATE`   | 192   | React to decisions made by earlier listeners |
| `LAST`   | 255   | Final overrides, monitoring |

Each listener sees modifications made by previous listeners. Use `EventPriority::custom(u8)` for fine-grained control.

## Resulted vs informational events

Some events implement `ResultedEvent`. These have a result that controls what the proxy does next. Call `event.set_result()` to change the outcome, or use shortcut methods like `event.deny()`.

Informational events (like `PostLoginEvent`) have no result. You can read their fields and act on the player they carry, but you cannot change what the proxy does next through them.

## Delivery

Every event goes through the same dispatch: listeners run one after another in priority order, with the panic isolation and timeouts described above. What differs is when that dispatch happens relative to the code that fired the event.

| Delivery | Events | What it means |
|----------|--------|---------------|
| Inline, awaited | `ConnectionHandshakeEvent`, `PreLoginEvent`, `OnlineAuthFailed`, `GameProfileRequestEvent`, `PermissionsSetupEvent`, `LoginEvent`, `PostLoginEvent`, `PlayerChooseInitialServerEvent`, `ServerPreConnectEvent`, `ServerConnectedEvent`, `ServerPostConnectEvent`, `KickedFromServerEvent`, `LimboEnterEvent`, `LimboExitEvent`, `ChatMessageEvent`, `CommandExecuteEvent`, `PluginMessageEvent`, `PreTransferEvent`, `ProxyPingEvent`, `ProxyInitializeEvent`, `ProxyShutdownEvent`, `DisconnectEvent`, custom events | The proxy (or the plugin that fired it) waits for every listener before it continues, so listeners can change the outcome. `DisconnectEvent` is also bounded as a whole by `[events] disconnect_deadline`. |
| Queued, in order | `ServerStateChangeEvent`, `BackendHealthEvent`, `ConfigReloadEvent`, `BanIssuedEvent`, `BanRevokedEvent`, `ConnectionRejectedEvent`, `PluginEnabledEvent`, `PluginDisabledEvent`, `ServiceProvidedEvent`, `ServiceRemovedEvent`, `PlayerClientBrandEvent`, `PlayerSettingsChangedEvent`, `PlayerChannelRegisterEvent`, `PlayerResourcePackStatusEvent` | The proxy posts these to a single queue. One dispatcher delivers them in the order they were posted, one event at a time. |

Because the queue delivers one event at a time, a slow listener on a queued event delays the queued events behind it, up to `handler_timeout` per listener. A listener that panics does not stop the queue: the next event is still delivered.

`ConnectionHandshakeEvent` and `ConnectionRejectedEvent` fire for every connection, including floods of bots. The proxy builds and dispatches them only while at least one listener is subscribed, so they cost nothing when no plugin uses them.

## Handshake events

These fire before a player exists: there is no `Player` yet, only an address and a handshake. Anti-bot and anti-VPN plugins use them.

### ConnectionHandshakeEvent

Fired once per connection, right after the proxy routed its domain and before anything else happens: before `ProxyPingEvent` for a server list ping, before the login start is read and `PreLoginEvent` for a login. It fires for connections that passed the IP filter, the rate limit and, for a ping, the IP ban check, see [before the login](#before-the-login). Awaited.

**Type:** Resulted

| Field | Type | Description |
|-------|------|-------------|
| `remote_addr` | `SocketAddr` | The client's address. Behind a load balancer that sends the PROXY protocol ([`receive_proxy_protocol`](../../configuration/global#proxy-protocol)), the address from the header |
| `virtual_host` | `Option<String>` | The domain from the handshake, lowercased, without the Forge marker or trailing dot. `None` when a legacy ping sends no host |
| `raw_host` | `String` | The host as the client sent it, with its case, Forge marker (`\0FML3\0`) and trailing dot. Empty when a legacy ping sends no host |
| `port` | `u16` | The port from the handshake, 0 when a legacy ping sends none |
| `protocol_version` | `ProtocolVersion` | The client's protocol version. For a client older than 1.7, the legacy protocol number, or 0 for a ping format that carries none |
| `intent` | `HandshakeIntent` | `Status` (server list ping), `Login` or `Transfer` (a 1.20.5+ client sent by a `Transfer` packet, which the proxy closes after the event: transfers are not supported yet). `as_str()` gives `status`, `login`, `transfer` |
| `legacy` | `bool` | `true` for a client older than 1.7 |
| `server` | `Option<ServerId>` | The server the domain routes to. `None` for a ping to an unknown domain |

**Results** (`ConnectionHandshakeResult`, `#[non_exhaustive]`, the shortcut method in parentheses):

| Variant | Description |
|---------|-------------|
| `Allow` (default, `allow()`) | Let the connection go on |
| `Deny { reason }` (`deny(reason)`) | Refuse the connection. A login is disconnected in the login phase with `reason`, or "You are not allowed to connect to this server" when it is `None`. A client older than 1.7 gets a legacy kick. A server list ping gets no answer: the proxy closes the connection |
| `DropSilently` (`drop_silently()`) | Close the connection without sending anything |

A refused connection posts a `ConnectionRejectedEvent` with the reason `Plugin`, carrying the ID of the plugin whose listener last changed the result.

```rust
use infrarust_api::events::handshake::{ConnectionHandshakeEvent, HandshakeIntent};

ctx.event_bus().subscribe::<ConnectionHandshakeEvent, _>(
    EventPriority::FIRST,
    |event| {
        if is_known_vpn(event.remote_addr.ip()) {
            match event.intent {
                HandshakeIntent::Status => event.drop_silently(),
                _ => event.deny(Component::error("VPNs are not allowed.")),
            }
        }
    },
);
```

### ConnectionRejectedEvent

Posted when the proxy refuses a connection before a player exists, exactly once per refused connection. Informational and queued: the proxy refuses the client without waiting for listeners, and listeners see the refusals in the order they happened. See [before the login](#before-the-login) for where each refusal happens.

| Field | Type | Description |
|-------|------|-------------|
| `remote_addr` | `SocketAddr` | The client's address, from the PROXY protocol header when there is one |
| `virtual_host` | `Option<String>` | The domain from the handshake, lowercased. `None` when the connection was refused before its handshake was read |
| `reason` | `RejectReason` | Why the proxy refused it |

**`RejectReason`** (`#[non_exhaustive]`, `as_str()` gives the name in parentheses):

| Variant | Description |
|---------|-------------|
| `IpFilter` (`ip_filter`) | The global [`ip_filter`](../../configuration/global#ip-filter) or the server's own filter refused the address |
| `RateLimit` (`rate_limit`) | The address went over [`rate_limit`](../../configuration/global#rate-limiting) |
| `UnknownDomain` (`unknown_domain`) | No server has the domain: a login, or any connection when `unknown_domain_behavior` is `drop` |
| `IpBanned` (`ip_banned`) | A ban on the address or a range that contains it, for a server list ping or a login |
| `Banned` (`banned`) | A ban on the name (or the UUID the client claimed) before authentication, or a ban check that failed and refused the login |
| `ServerUnavailable` (`server_unavailable`) | Not posted any more. The server manager starts a server once the player exists, and a server it cannot start is reported by `KickedFromServerEvent`, see [server wake](#server-wake). The variant remains so that existing matches compile |
| `Plugin { plugin_id }` (`plugin`) | A `ConnectionHandshakeEvent` listener denied or dropped the connection, or a [transport filter](./architecture#layer-1-transportfilter) rejected it, panicked on it or ran past `[events] transport_filter_timeout`. `plugin_id` is the plugin that set the result or registered the filter, `None` when the proxy cannot tell or the filter is the proxy's own |

A connection a transport filter refuses has no `virtual_host`: the filters run before the handshake is read. The listener limit ([`max_connections`](../../configuration/global#connection-limits)) refuses nothing: while it is reached, the proxy waits before it accepts the next connection, so the kernel holds it in the backlog.

```rust
use infrarust_api::events::handshake::{ConnectionRejectedEvent, RejectReason};

ctx.event_bus().subscribe::<ConnectionRejectedEvent, _>(
    EventPriority::NORMAL,
    |event| {
        if event.reason == RejectReason::RateLimit {
            tracing::warn!("{} is flooding the proxy", event.remote_addr.ip());
        }
    },
);
```

WASM plugins (contract 0.3.0) receive `connection-handshake` and `connection-rejected` with the same fields and results. See [WASM events](../wasm/events).

## Lifecycle events

### PreLoginEvent

Fired before authentication, when a player initiates a connection. This is your first chance to accept, deny, or change the auth mode for a player.

**Type:** Resulted

| Field | Type | Description |
|-------|------|-------------|
| `profile` | `GameProfile` | The player's profile. In `client_only` the UUID is nil until authentication; offline profiles follow [`[auth] offline_uuid`](../../configuration/global#authentication) |
| `remote_addr` | `SocketAddr` | The client's address. Behind a load balancer that sends the PROXY protocol ([`receive_proxy_protocol`](../../configuration/global#proxy-protocol)), the address from the header |
| `protocol_version` | `ProtocolVersion` | Protocol version reported by the client |
| `server_domain` | `String` | The domain from the handshake packet |

**Results** (`PreLoginResult`):

| Variant | Description |
|---------|-------------|
| `Allowed` (default) | Proceed with normal authentication |
| `Denied { reason }` | Kick the player with a message |
| `ForceOffline` | Skip Mojang auth for this player. Ignored in the forwarding modes |
| `ForceOnline` | Force Mojang auth even if the server is in offline mode. Ignored in the forwarding modes |

```rust
ctx.event_bus().subscribe::<PreLoginEvent, _>(
    EventPriority::NORMAL,
    |event| {
        // Ban check
        if is_banned(&event.profile.uuid) {
            event.deny(Component::error("You are banned."));
        }
    },
);
```

### OnlineAuthFailed

Fired when online-mode authentication fails, for example a cracked client that cannot complete the encryption handshake. This covers both forced online auth (`ForceOnline` returned from `PreLoginEvent` while the server is in offline mode) and the default online auth used by `client_only` mode. A plugin can listen for this to remember the username and return `ForceOffline` on the player's next connection attempt. Informational only, and awaited: the client is disconnected once the listeners are done.

| Field | Type | Description |
|-------|------|-------------|
| `username` | `String` | The username that failed online authentication |

`OnlineAuthFailed` is not re-exported from the prelude. Import it from its module: `use infrarust_api::events::lifecycle::OnlineAuthFailed;`.

### GameProfileRequestEvent

Fired right after authentication, before the proxy checks bans against the player's UUID and before the player exists. Change `profile` to give the player another UUID, name or properties, for example skin textures in offline mode. The profile left in the event when the last listener returns is the one the proxy uses from then on: for the UUID ban check, in the player registry and every later event, in the `LoginSuccess` the client receives, and in what forwarding sends to the backend.

**Type:** Informational, with a mutable `profile`

| Field | Type | Description |
|-------|------|-------------|
| `profile` | `GameProfile` | The profile the player will have. Change it to rewrite the player's identity |
| `online_mode` | `bool` | Whether the session server verified the player |
| `remote_addr` | `SocketAddr` | The client's address |
| `virtual_host` | `Option<String>` | The domain from the handshake |
| `protocol_version` | `ProtocolVersion` | The client's protocol version |

`original()` returns the profile as authentication produced it, and `is_modified()` tells whether a listener changed it.

```rust
use infrarust_api::events::lifecycle::GameProfileRequestEvent;

ctx.event_bus().subscribe::<GameProfileRequestEvent, _>(
    EventPriority::NORMAL,
    |event| {
        if !event.online_mode {
            event.profile.properties = skin_for(&event.profile.username);
        }
    },
);
```

In `offline` mode the backend normally completes the login with the client. When a listener changes the profile, the proxy completes the login itself instead, as it does for Velocity forwarding, so that the client and the backend both see the new profile.

In the forwarding modes the backend runs its own login, so the new profile only reaches it through BungeeCord or BungeeGuard forwarding, see [passthrough](#passthrough-zero-copy-and-server-only).

### PermissionsSetupEvent

Fired after the ban check, before `LoginEvent`, once the active permission provider has built the player's checker. A listener can replace that checker for this player only. To answer for every player (LuckPerms, a database), register a permission provider instead, see [Permissions](./permissions). If no listener provides a custom checker, the player keeps the provider's.

The player is built but not registered yet. The checker a listener sets applies to the player for the whole session, so `has_permission` already answers with it in `LoginEvent` and `PostLoginEvent`, and `refresh_permissions()` keeps it.

**Type:** Resulted

| Field | Type | Description |
|-------|------|-------------|
| `player` | `Arc<dyn Player>` | The player being logged in |
| `online_mode` | `bool` | Whether the player authenticated via Mojang |

`player_id()` and `profile()` are shortcuts for `player.id()` and `player.profile()`.

**Results** (`PermissionsSetupResult`):

| Variant | Description |
|---------|-------------|
| `UseDefault` (default) | Keep the checker built by the active permission provider |
| `Custom(Arc<dyn PermissionChecker>)` | Use a plugin-provided checker for this player |

```rust
use infrarust_api::events::lifecycle::{PermissionsSetupEvent, PermissionsSetupResult};

ctx.event_bus().subscribe::<PermissionsSetupEvent, _>(
    EventPriority::NORMAL,
    |event| {
        event.set_result(PermissionsSetupResult::Custom(
            Arc::new(MyPermissionChecker::new()),
        ));
    },
);
```

Like `OnlineAuthFailed`, this type is reached through `infrarust_api::events::lifecycle`, not the prelude glob.

### LoginEvent

Fired after `PermissionsSetupEvent`, just before the player is registered. It is the last point where a login can be refused: a denied player is disconnected during the login phase with the reason, is never registered, and fires neither `PostLoginEvent` nor `DisconnectEvent`. In `client_only` mode the client has not received `LoginSuccess` yet, and in the forwarding modes nothing was sent to the backend yet.

**Type:** Resulted

| Field | Type | Description |
|-------|------|-------------|
| `player` | `Arc<dyn Player>` | The player being logged in |
| `online_mode` | `bool` | Whether the player authenticated via Mojang |

`player_id()` and `profile()` are shortcuts for `player.id()` and `player.profile()`.

**Results** (`LoginResult`):

| Variant | Description |
|---------|-------------|
| `Allowed` (default) | Let the player in |
| `Denied { reason }` | Disconnect the player during login with this reason |

```rust
ctx.event_bus().subscribe::<LoginEvent, _>(
    EventPriority::NORMAL,
    |event| {
        if !is_whitelisted(&event.profile().uuid) {
            event.deny(Component::error("You are not whitelisted."));
        }
    },
);
```

### PostLoginEvent

Fired once the player is registered, before the proxy picks their first server. Informational and awaited: the proxy routes the player only after every listener has returned. See the [guarantees](#guarantees) for what a listener can rely on here.

| Field | Type | Description |
|-------|------|-------------|
| `player` | `Arc<dyn Player>` | The player, already in the player registry |
| `profile` | `GameProfile` | The player's final profile |
| `protocol_version` | `ProtocolVersion` | The player's protocol version |

`player_id()` is a shortcut for `player.id()`.

```rust
ctx.event_bus().subscribe::<PostLoginEvent, _>(
    EventPriority::NORMAL,
    |event| {
        tracing::info!("{} logged in ({:?})", event.profile.username, event.player_id());
        let _ = event.player.send_message(Component::text("Welcome!"));
    },
);
```

### DisconnectEvent

Fired exactly once for every player that got a `PostLoginEvent`, when their session ends. The proxy waits for all listeners, up to `[events] disconnect_deadline`, then removes the player from the registry, so async cleanup can still look the player up.

| Field | Type | Description |
|-------|------|-------------|
| `player` | `Arc<dyn Player>` | The disconnecting player |
| `last_server` | `Option<ServerId>` | The server they were on when the session ended, `None` if they never reached one |
| `cause` | `DisconnectCause` | Why the session ended |

`player_id()` and `username()` are shortcuts for `player.id()` and `player.profile().username`.

**`DisconnectCause`** (`as_str()` gives the name in parentheses, `reason()` the message when there is one):

| Variant | Description |
|---------|-------------|
| `ClientQuit` (`client_quit`) | The client closed the connection |
| `Kicked { reason }` (`kicked`) | The proxy ended the session: `Player::disconnect`, a denied connection, a duplicate login. `reason` is what the client was shown |
| `BackendClosed { reason }` (`backend_closed`) | The backend kicked the player or closed the connection and the player was not moved elsewhere. `reason` is what the client was shown, see [KickedFromServerEvent](#kickedfromserverevent). Always `None` in the forwarding modes, where the proxy does not read the backend's packets |
| `Shutdown` (`shutdown`) | The proxy is shutting down. The client was shown "Proxy is shutting down" (`offline` and `client_only`). See [Proxy shutdown](#proxy-shutdown) |
| `Error` (`error`) | The session failed, for example an I/O error or an initial backend that could not be reached |

```rust
ctx.event_bus().subscribe_async::<DisconnectEvent, _>(
    EventPriority::NORMAL,
    |event| Box::pin(async move {
        tracing::info!("{} disconnected ({})", event.username(), event.cause.as_str());
    }),
);
```

### WASM plugins

WASM plugins (contract 0.3.0) receive every event above in the same order, with the native fields and results, `GameProfileRequestEvent`, `LoginEvent` and the disconnect cause included. `PermissionsSetupEvent` can only reset to the default checker from WASM. See [WASM events](../wasm/events).

## Connection events

### PlayerChooseInitialServerEvent

Fired after `PostLoginEvent`, before `ServerPreConnectEvent`. Allows you to override which server a player connects to first. Useful for lobby systems, load balancing, or queue plugins.

**Type:** Resulted

| Field | Type | Description |
|-------|------|-------------|
| `player` | `Arc<dyn Player>` | The connecting player. `current_server()` is `None` |
| `initial_server` | `ServerId` | The server chosen by the domain router |

`player_id()` and `profile()` are shortcuts for `player.id()` and `player.profile()`.

**Results** (`PlayerChooseInitialServerResult`):

| Variant | Description |
|---------|-------------|
| `Allowed` (default) | Use the domain router's choice |
| `Redirect(ServerId)` | Send to a different server |
| `SendToLimbo { limbo_handlers }` | Route through limbo handlers |

```rust
ctx.event_bus().subscribe::<PlayerChooseInitialServerEvent, _>(
    EventPriority::NORMAL,
    |event| {
        if is_first_join(&event.profile().uuid) {
            event.set_result(PlayerChooseInitialServerResult::Redirect(
                ServerId::new("lobby"),
            ));
        }
    },
);
```

### ServerPreConnectEvent

Fired before the proxy opens a connection to a backend server, once per connection attempt. See [server connections](#server-connections) for when it fires.

**Type:** Resulted

| Field | Type | Description |
|-------|------|-------------|
| `player` | `Arc<dyn Player>` | The player |
| `server` | `ServerId` | The server the proxy is about to connect to |
| `previous_server` | `Option<ServerId>` | The server the player is on, `None` before their first server |
| `cause` | `ConnectCause` | Why the connection is attempted |

`player_id()` and `profile()` are shortcuts for `player.id()` and `player.profile()`.

**`ConnectCause`** (`#[non_exhaustive]`, `as_str()` gives the name in parentheses):

| Variant | Description |
|---------|-------------|
| `Initial` (`initial`) | The player's first server after login, also when an initial limbo gate held them first |
| `Switch` (`switch`) | `Player::switch_server`, `Player::connect`, a command or a plugin moves the player to another server |
| `LimboExit` (`limbo_exit`) | A limbo handler sends the player to another server than the one it held them for, or back to a server after a kick sent them to limbo |
| `KickRedirect` (`kick_redirect`) | A `KickedFromServerEvent` listener redirected the kicked player |
| `PluginMessage` (`plugin_message`) | A backend asked for it on the [BungeeCord channel](./messaging#the-bungeecord-channel) (`Connect`, `ConnectOther`) |

**Results** (`ServerPreConnectResult`):

| Variant | Description |
|---------|-------------|
| `Allowed` (default) | Connect to `server` |
| `ConnectTo(ServerId)` | Redirect to a different server |
| `SendToLimbo { limbo_handlers }` | Route through limbo handlers |
| `Denied { reason }` | Block the connection |

Virtual backends are planned but not wired into the proxy, so no result routes to one. The `VirtualBackend` result was removed because the proxy never acted on it.

```rust
ctx.event_bus().subscribe::<ServerPreConnectEvent, _>(
    EventPriority::NORMAL,
    |event| {
        if is_server_full(&event.server) {
            event.redirect_to(ServerId::new("fallback"));
        }
    },
);
```

### ServerConnectedEvent

Fired once a backend server accepted the player's login. Informational and awaited. The player has not joined the server yet: `current_server()` still returns the server they were on, or `None` before their first server.

| Field | Type | Description |
|-------|------|-------------|
| `player` | `Arc<dyn Player>` | The player |
| `server` | `ServerId` | The server that accepted the login |
| `previous_server` | `Option<ServerId>` | The server the player is on, `None` before their first server |

`player_id()` is a shortcut for `player.id()`.

### ServerPostConnectEvent

Fired once the player joined a server: its `JoinGame` packet reached the client and `current_server()` returns `server`. Informational and awaited. Not fired in passthrough modes.

| Field | Type | Description |
|-------|------|-------------|
| `player` | `Arc<dyn Player>` | The player |
| `server` | `ServerId` | The server the player joined |
| `previous_server` | `Option<ServerId>` | The server the player was on, `None` for their first server |

`player_id()` is a shortcut for `player.id()`. `switched_from()` returns the server the player left when this join moved them from another server, and `None` for their first server or a return to the same server.

There is no separate switch event: a switch is a `ServerPostConnectEvent` whose `switched_from()` is set.

```rust
ctx.event_bus().subscribe::<ServerPostConnectEvent, _>(
    EventPriority::NORMAL,
    |event| {
        if let Some(from) = event.switched_from() {
            tracing::info!("{} moved from {from} to {}", event.player.profile().username, event.server);
        }
    },
);
```

### KickedFromServerEvent

Fired when a backend server drops a player or cannot take them, before anything of it reaches the client. In the forwarding modes it only fires for a backend that cannot be reached, see [passthrough](#passthrough-zero-copy-and-server-only). In `offline` and `client_only` it fires for:

- a disconnect packet from the server the player is playing on, or that server closing the connection;
- a connection that fails before the player joined the server: the initial connection, a switch, a kick redirect or a limbo exit. The server cannot be reached, refuses the login, sends a disconnect during the configuration phase or before its `JoinGame`, or closes the connection.

It does not fire when a `ServerPreConnectEvent` listener denies a connection, or for `Player::disconnect`.

**Type:** Resulted. The proxy picks the default result for the situation, see [defaults](#kick-defaults).

| Field | Type | Description |
|-------|------|-------------|
| `player` | `Arc<dyn Player>` | The player |
| `server` | `ServerId` | The server that dropped the player, or that the player could not join |
| `reason` | `Option<Component>` | The reason the server sent, parsed from its disconnect packet, or the proxy's message when the server manager could not start the server. `None` when there is neither |
| `cause` | `KickCause` | How the server dropped the player |
| `during_connect` | `bool` | `true` when the player had not joined `server` yet |
| `previous_server` | `Option<ServerId>` | When `during_connect`, the player's `current_server()`: the server they are on, or the one that kicked them before a redirect, `None` before their first server. Otherwise the server they were on before `server` |

`player_id()` and `profile()` are shortcuts for `player.id()` and `player.profile()`.

**`KickCause`** (`#[non_exhaustive]`, `as_str()` gives the name in parentheses):

| Variant | Description |
|---------|-------------|
| `Unreachable { error }` (`unreachable`) | The proxy could not connect, the login or configuration phase did not finish in time, or the server manager could not start the server. `error` describes the failure |
| `LoginRefused` (`login_refused`) | The server answered the login with a disconnect |
| `ConfigDisconnect` (`config_disconnect`) | The server sent a disconnect during the configuration phase (1.20.2+) |
| `PlayDisconnect` (`play_disconnect`) | The server sent a disconnect in the play phase |
| `ConnectionLost` (`connection_lost`) | The server closed the connection without a disconnect packet |

**Results** (`KickedFromServerResult`, `#[non_exhaustive]`, the shortcut method in parentheses):

| Variant | Description |
|---------|-------------|
| `DisconnectPlayer { reason }` (`disconnect(reason)`) | Disconnect the player. With `reason: None` the client gets the server's disconnect packet byte for byte, re-encoded only when the client is in another phase than the server was. When the server sent none, the client gets the server's `disconnect_message` |
| `RedirectTo(ServerId)` (`redirect_to`) | Connect the player to another server |
| `SendToLimbo { limbo_handlers }` (`send_to_limbo`) | Send the player to limbo. An empty list uses the `limbo_handlers` of `server` |
| `Notify { message }` (`notify`) | Keep the player on the server they are on and send them `message` in chat. When they have no server to stay on, disconnect them with `message` |

#### Kick defaults

| Situation | Default result |
|-----------|----------------|
| The player was playing on `server` (`during_connect` is `false`) | `DisconnectPlayer { reason: None }`: the client sees the server's disconnect as the server sent it |
| A switch failed and the player can stay on the server they are on | `Notify` with the server's reason, or its `disconnect_message` when it sent none |
| A connection failed and the player has no server to stay on: the initial connection, a limbo exit, a redirect after a kick in the play phase | `SendToLimbo` with the `limbo_handlers` of `server` when it has some, otherwise `DisconnectPlayer { reason: None }` |

During a switch the player stays on their server until the new one sends its `JoinGame`, with one exception on 1.20.2+: the client leaves the old server's world when the new server starts its configuration phase. A disconnect sent as the first configuration packet still leaves the player where they were. A later one, or a failure after the configuration phase, leaves them with no server to stay on.

A `RedirectTo` goes through the [connection events](#server-connections): `ServerPreConnectEvent` fires with the cause `KickRedirect` and `previous_server` set to the player's current server, which is the server that kicked them after a kick in the play phase. If the redirect fails as well, a new `KickedFromServerEvent` fires for the redirect target. After three redirects in a row that failed, a fourth `RedirectTo` is handled as `DisconnectPlayer { reason: None }`.

When the player ends up disconnected, the `DisconnectEvent` cause is `BackendClosed` with the reason the client was shown: the server's parsed reason, or your `reason` or `message`. It is `None` when the server sent no reason and the client got the `disconnect_message`. A server that could not be reached gives the cause `Error`. A limbo handler reached through `SendToLimbo` gets `LimboEntryContext::KickedFromServer` with the server's reason, or its `disconnect_message`.

```rust
ctx.event_bus().subscribe::<KickedFromServerEvent, _>(
    EventPriority::NORMAL,
    |event| {
        if !event.during_connect && event.server != ServerId::new("lobby") {
            event.redirect_to(ServerId::new("lobby"));
        }
    },
);
```

### LimboEnterEvent

Fired when a player enters a chain of [limbo handlers](./architecture#layer-4-limbohandler): an initial gate (the server's `limbo_handlers`, or `SendToLimbo` from `PlayerChooseInitialServerEvent` or `ServerPreConnectEvent`), a kick sent to limbo by `KickedFromServerEvent`, a switch sent to limbo by `ServerPreConnectEvent`, or a limbo handler moving the player to another chain with `SendToLimbo`. Awaited in the player's session, once the client has left the login and configuration phases, before the proxy sends the limbo world and before the first handler's `on_player_enter`.

| Field | Type | Description |
|-------|------|-------------|
| `player` | `Arc<dyn Player>` | The player |
| `handlers` | `Vec<String>` | The names of the handlers the player goes through, in order |
| `context` | `LimboEntryContext` | Why the player entered: `InitialConnection { target_server }`, `KickedFromServer { server, reason }` or `PluginRedirect { from_server }` |

`player_id()` is a shortcut for `player.id()`.

### LimboExitEvent

Fired when the player leaves the handler chain, whatever ends it. Awaited in the player's session, before the proxy acts on the outcome: the `ServerPreConnectEvent` of the next server, the next `LimboEnterEvent`, or the player's `DisconnectEvent` follow it.

| Field | Type | Description |
|-------|------|-------------|
| `player` | `Arc<dyn Player>` | The player |
| `reason` | `LimboExitReason` | How the chain ended |
| `next_server` | `Option<ServerId>` | The server the proxy connects the player to now, `None` when the player is not sent to a server |

**`LimboExitReason`** (`#[non_exhaustive]`, `as_str()` gives the name in parentheses):

| Variant | Description |
|---------|-------------|
| `Released` (`released`) | Every handler accepted the player. `next_server` is the server the chain held them for: the initial server, the server that kicked them, or the server they were on when a switch sent them to limbo |
| `Redirected` (`redirected`) | A handler or `Player::switch_server` sent the player to `next_server` |
| `SentToLimbo { handlers }` (`sent_to_limbo`) | A handler moved the player to another chain. A `LimboEnterEvent` follows when those handlers exist |
| `Kicked { reason }` (`kicked`) | A handler denied the player or `Player::disconnect` was called. The client was shown `reason` |
| `Disconnected` (`disconnected`) | The client left |
| `TimedOut` (`timed_out`) | The client stopped answering keep-alives |
| `Shutdown` (`shutdown`) | The proxy is shutting down |

For one stay in limbo the order is `LimboEnterEvent`, then `LimboExitEvent`, then what the outcome leads to. After an initial gate that released the player, `ServerConnectedEvent` follows directly: the gate's `ServerPreConnectEvent` fired before `LimboEnterEvent`.

WASM plugins (contract 0.3.0) receive `limbo-enter` and `limbo-exit` with the same fields.

### WASM connection events

WASM plugins (contract 0.3.0) receive these events with the native fields and results: `server-pre-connect` carries `previous-server` and `cause`, `server-connected` waits for the backend to accept the login, and `server-post-connect` fires for every connection, not only for switches.

`kicked-from-server` carries the optional reason as a component, `cause`, `during-connect` and `previous-server`, and every result above. To show the server's own disconnect, leave the result as it is.

## Chat and command events

The proxy reads chat and commands in the `offline` and `client_only` modes, and in limbo. In passthrough, `zero_copy` and `server_only` it only forwards bytes, so these events do not fire.

### ChatMessageEvent

Fired when a player sends a chat message, before the proxy forwards it. On a server it fires for every chat packet the client sends. In limbo it fires before the limbo handler's `on_chat`, and the handler sees the message only if the result lets it through.

**Type:** Resulted

| Field | Type | Description |
|-------|------|-------------|
| `player` | `Arc<dyn Player>` | The sender. `player_id()` and `profile()` read from it |
| `message` | `String` | The message text |
| `signed` | `bool` | The client signed the message (1.19 and later, with a chat session) |
| `server` | `Option<ServerId>` | The server the message is going to. `None` in limbo |

**Results** (`ChatMessageResult`):

| Variant | Shortcut | Description |
|---------|----------|-------------|
| `Allow` (default) | `allow()` | Forward the message exactly as the client sent it |
| `Deny { reason: Some(..) }` | `deny(reason)` | Drop the message and show `reason` to the sender as a system message |
| `Deny { reason: None }` | `deny_silently()` | Drop the message without telling anyone |
| `Modify { message }` | `modify(message)` | Send `message` instead. In limbo, `on_chat` receives `message` |

```rust
ctx.event_bus().subscribe::<ChatMessageEvent, _>(
    EventPriority::NORMAL,
    |event| {
        if contains_banned_word(&event.message) {
            event.deny(Component::error("That word is not allowed."));
        } else if event.message.starts_with("!shout ") {
            let text = event.message.trim_start_matches("!shout ").to_uppercase();
            event.modify(text);
        }
    },
);
```

A vanilla server disconnects a player whose message is longer than 256 characters, and a modified message is sent as it is. Keep a rewritten message within that limit.

### CommandExecuteEvent

Fired when a player runs a command on a server, before the proxy looks for a proxy command with that name. It fires for every command: the proxy's own, the backend's, and signed commands.

It does not fire in limbo. Commands typed in limbo belong to the limbo handler: the proxy runs its own commands there and passes the others to `on_command`, so no plugin sees an auth limbo's `/login <password>`.

**Type:** Resulted

| Field | Type | Description |
|-------|------|-------------|
| `player` | `Arc<dyn Player>` | The player. `player_id()` and `profile()` read from it |
| `command` | `String` | The command line without the leading `/`. `label()` returns its first word |
| `signed` | `bool` | The client signed at least one argument (1.19 and later, with a chat session) |
| `server` | `Option<ServerId>` | The server the player is on |

**Results** (`CommandExecuteResult`):

| Variant | Shortcut | Description |
|---------|----------|-------------|
| `Allow` (default) | `allow()` | Run the proxy command with this name, or forward the command unchanged when the proxy has none |
| `Deny { reason: Some(..) }` | `deny(reason)` | Drop the command and show `reason` to the player |
| `Deny { reason: None }` | `deny_silently()` | Drop the command without telling anyone |
| `Modify { command }` | `modify(command)` | Run `command` instead: the proxy command with its name, or the backend's, sent unsigned |
| `ForwardToBackend` | `forward_to_backend()` | Forward the command unchanged, even when the proxy has a command with this name |

```rust
ctx.event_bus().subscribe::<CommandExecuteEvent, _>(
    EventPriority::NORMAL,
    |event| {
        match event.label() {
            "op" | "deop" => event.deny(Component::error("Run this from the console.")),
            "spawn" => event.modify("warp spawn"),
            _ => {}
        }
    },
);
```

The proxy checks a proxy command's permission after the event, when it runs the command.

### Signed chat and acknowledgements

From 1.19 on, clients sign chat messages and command arguments, and from 1.19.1 on every chat packet also tells the server which signed messages the client has seen. Velocity passes the client's chat session on to the backend, so it cannot change or drop a signed message without breaking the backend's checks, and disconnects the player when a plugin tries.

Infrarust drops the client's chat session (the Player Session packet on 1.19.3 and later, the key in Login Start before that) instead of sending it to the backend. The backend has no key for the player and accepts messages without a signature, the way a vanilla server in offline mode does. That lets the proxy deny and modify signed chat and signed commands, as long as it keeps the backend's count of seen messages right:

- A dropped packet that acknowledged messages is replaced with a Message Acknowledgment carrying the same count, so the backend's window of seen messages stays in step with the client's. Without it, the backend disconnects the player with a chat validation error on a later message.
- A modified packet keeps the timestamp, the salt and the acknowledgements of the original and loses the signature.
- A proxy command consumed by the proxy is acknowledged the same way as a denied one.

A backend only counts signed messages, and no player behind Infrarust has a chat session on the backend, so the count is 0 in most setups and no acknowledgement is sent. It matters when players who connect to the backend directly share it with players behind the proxy.

| Client | Chat and command packets | Deny, or a command run by the proxy | Modify |
|--------|--------------------------|-------------------------------------|--------|
| 1.7 to 1.18.2 | Chat Message with the text only. A command is a chat message that starts with `/` | Dropped | Sent again with the new text (`/` and the new command for a command) |
| 1.19 | Chat Message and Chat Command with a timestamp, a salt and signatures. No acknowledgements | Dropped | Sent unsigned: empty signature, preview flag off, same timestamp and salt |
| 1.19.1, 1.19.2 | The same, with the list of the last messages the client saw | Dropped. A Message Acknowledgment carrying that list is sent when it is not empty | Sent unsigned with the same list |
| 1.19.3 to 1.20.4 | Chat Message and Chat Command with a signature, a count of seen messages and a 20-bit acknowledged set | Dropped. A Message Acknowledgment with the count is sent when it is above 0 | Sent without signature, with the same count and set |
| 1.20.5 to 1.21.4 | Chat Command carries the command only. Signed Chat Command carries the signatures and the count | As above for chat and Signed Chat Command. A Chat Command is only dropped | A Chat Command with the new command. For a Signed Chat Command, a Message Acknowledgment with its count follows when it is above 0 |
| 1.21.5 and later | The same, with a checksum of the acknowledged messages | As above | As above, the checksum is kept |

The backend must accept messages without a signature. A vanilla server does when it runs in offline mode, which is how Infrarust's backends run. A Paper server enforces signed chat when `enforce-secure-profile` is `true` and it trusts a proxy as online, which is the default with Velocity forwarding: set `enforce-secure-profile=false` in `server.properties` on such a backend, or it refuses the player's chat whatever the plugins do.

Formats: [Java Edition protocol, packets](https://minecraft.wiki/w/Java_Edition_protocol/Packets) and the [1.19](https://minecraft.wiki/w/Minecraft_Wiki:Projects/wiki.vg_merge/Protocol?oldid=2772902), [1.19.2](https://minecraft.wiki/w/Minecraft_Wiki:Projects/wiki.vg_merge/Protocol?oldid=2772944), [1.19.3](https://minecraft.wiki/w/Minecraft_Wiki:Projects/wiki.vg_merge/Protocol?oldid=2773015), [1.20.5](https://minecraft.wiki/w/Minecraft_Wiki:Projects/wiki.vg_merge/Protocol?oldid=2789623) and [1.21.5](https://minecraft.wiki/w/Minecraft_Wiki:Projects/wiki.vg_merge/Protocol?oldid=2992295) revisions. Velocity's handling, for comparison: [`SessionChatHandler`](https://github.com/PaperMC/Velocity/blob/dev/3.0.0/proxy/src/main/java/com/velocitypowered/proxy/protocol/packet/chat/session/SessionChatHandler.java), [`SessionCommandHandler`](https://github.com/PaperMC/Velocity/blob/dev/3.0.0/proxy/src/main/java/com/velocitypowered/proxy/protocol/packet/chat/session/SessionCommandHandler.java) and [`ChatQueue`](https://github.com/PaperMC/Velocity/blob/dev/3.0.0/proxy/src/main/java/com/velocitypowered/proxy/protocol/packet/chat/ChatQueue.java).

### WASM chat events

Contract 0.3.0 has `chat-message` and `command-execute`, with the native fields and results. A WASM plugin needs the [`chat-intercept`](../wasm/capabilities) capability to subscribe to either. See [WASM events](../wasm/events).

## Plugin message and client events

The proxy reads plugin messages and the client's settings in `offline`, `client_only` and limbo. [Plugin messaging](./messaging) covers channels, sending, the BungeeCord channel and the client state in full.

### PluginMessageEvent

Fired for a plugin message on a channel a plugin registered with `ctx.channel_registrar()`, in the configuration phase and in play, from the client or the backend. Awaited in the player's session: the message waits for the listeners. Messages on other channels pass through untouched and fire nothing.

**Type:** Resulted

| Field | Type | Description |
|-------|------|-------------|
| `player` | `Arc<dyn Player>` | The player whose connection carries the message |
| `source` | `Endpoint` | `Client` or `Backend(ServerId)` |
| `channel` | `ChannelId` | The registered channel |
| `raw_channel` | `String` | The channel name as sent |
| `data` | `Bytes` | The payload |
| `phase` | `MessagePhase` | `Configuration` or `Play` |

**Results** (`PluginMessageResult`): `Forward` (default, `forward()`), `Handled` (`handled()`, nothing is forwarded) and `Replace(Bytes)` (`replace(data)`, forward `data` instead).

A client message on `BungeeCord`, `bungeecord:main` or a `velocity:` channel is dropped before this event, whoever registered the channel.

### PlayerClientBrandEvent

Posted when the client sends its brand (`minecraft:brand`, `MC|Brand` before 1.13) and it differs from the last one. Informational and queued. Fields: `player`, `brand`.

### PlayerSettingsChangedEvent

Posted when the client sends its settings (Client Information) and they differ from the last ones: at login and whenever the player changes an option. Informational and queued. Fields: `player`, `settings` (`ClientSettings`).

### PlayerChannelRegisterEvent

Posted when channels are registered on `minecraft:register` (`REGISTER` before 1.13). Informational and queued. Fields: `player`, `channels` (the names in the message), `direction` (`PacketDirection::Serverbound` when the client registered them, `Clientbound` when the backend did). Unregistrations post nothing.

```rust
use infrarust_api::events::client::PlayerSettingsChangedEvent;

ctx.event_bus().subscribe::<PlayerSettingsChangedEvent, _>(EventPriority::NORMAL, |event| {
    tracing::info!(
        "{} plays in {} with a view distance of {}",
        event.player.profile().username,
        event.settings.locale,
        event.settings.view_distance,
    );
});
```

WASM plugins (contract 0.3.0) receive these events, and `PlayerInfo` carries `settings` and `known-channels`. Plugin messaging for WASM needs the [`plugin-messaging`](../wasm/capabilities) capability.

## Resource pack and transfer events

The proxy reads resource pack answers and transfers in `offline`, `client_only` and limbo. See [resource packs](./api#resource-packs) and [transfers](./api#transfers) on the `Player` trait.

### PlayerResourcePackStatusEvent

Posted for every answer the client gives to a resource pack, in the configuration phase or in play, whether the proxy or a backend sent the pack. Informational and queued.

| Field | Type | Description |
|-------|------|-------------|
| `player` | `Arc<dyn Player>` | The player |
| `pack_id` | `Option<Uuid>` | The pack's UUID. From 1.20.3 it is the UUID in the client's answer. Before 1.20.3 answers carry none: it is the `ResourcePackRequest` id for a pack the proxy sent, and `None` for a backend's pack |
| `status` | `ResourcePackStatus` | What the client did with the pack |
| `origin` | `ResourcePackOrigin` | `Proxy` for a pack sent with `send_resource_pack`, `Backend` for one a server sent |

**`ResourcePackStatus`** (`#[non_exhaustive]`, `as_str()` gives the name in parentheses, `id()` the protocol value):

| Variant | Id | Final | Description |
|---------|----|-------|-------------|
| `SuccessfullyLoaded` (`successfully_loaded`) | 0 | yes | The pack is applied |
| `Declined` (`declined`) | 1 | yes | The player declined the prompt |
| `FailedDownload` (`failed_download`) | 2 | yes | The download failed |
| `Accepted` (`accepted`) | 3 | no | The player accepted; the download starts |
| `Downloaded` (`downloaded`) | 4 | no | Downloaded, not applied yet (1.20.3 and later) |
| `InvalidUrl` (`invalid_url`) | 5 | yes | The URL is not valid (1.20.3 and later) |
| `FailedReload` (`failed_reload`) | 6 | yes | The client could not reload its resources (1.20.3 and later) |
| `Discarded` (`discarded`) | 7 | yes | The pack was removed or replaced before it was applied (1.20.3 and later) |
| `Unknown(i32)` (`unknown`) | any other | no | A value this version of Infrarust does not know |

`is_final()` tells whether the client will send more answers about the pack. Answers to the proxy's packs are not forwarded to the backend; answers to a backend's packs are, unchanged.

```rust
use infrarust_api::events::resource_pack::{PlayerResourcePackStatusEvent, ResourcePackOrigin};
use infrarust_api::player::ResourcePackStatus;

ctx.event_bus().subscribe::<PlayerResourcePackStatusEvent, _>(EventPriority::NORMAL, |event| {
    if event.origin == ResourcePackOrigin::Proxy && event.status == ResourcePackStatus::Declined {
        let player = Arc::clone(&event.player);
        tokio::spawn(async move {
            player.disconnect(Component::text("This server needs its resource pack")).await;
        });
    }
});
```

### PreTransferEvent

Fired before a Transfer packet reaches the client (1.20.5 and later): when a plugin calls `Player::transfer`, and when a backend sends one, in the configuration phase or in play. Awaited: the transfer waits for the listeners.

**Type:** Resulted

| Field | Type | Description |
|-------|------|-------------|
| `player` | `Arc<dyn Player>` | The player |
| `host` | `String` | The destination host |
| `port` | `u16` | The destination port |
| `origin` | `TransferOrigin` | `Plugin` or `Backend` |

**Results** (`PreTransferResult`):

| Variant | Shortcut | Description |
|---------|----------|-------------|
| `Allowed` (default) | | Send the transfer to `host:port` |
| `Denied { reason }` | `deny(reason)` | Send nothing. `Player::transfer` returns `Err(PlayerError::Denied(reason))`; a backend's transfer is dropped |
| `Redirect { host, port }` | `redirect(host, port)` | Send the player to this destination instead |

`destination()` returns where the player will go under the current result, or `None` when it is denied.

```rust
use infrarust_api::events::transfer::{PreTransferEvent, TransferOrigin};

ctx.event_bus().subscribe::<PreTransferEvent, _>(EventPriority::NORMAL, |event| {
    if event.origin == TransferOrigin::Backend && !event.host.ends_with(".example.com") {
        event.deny(Component::text("Transfers leave the network only through the proxy"));
    }
});
```

## Packet events

### RawPacketEvent

A low-level event fired when a raw packet passes through the proxy during Play state. Use this only when higher-level events don't cover your use case.

`RawPacketEvent` does not implement `Event` or `ResultedEvent`, so `subscribe::<RawPacketEvent, _>` does not compile. The only way to receive raw packets is a packet subscription (`subscribe_packet_typed` or `subscribe_packet_async_typed`, shown below). Read and change the outcome with the event's own `result()`, `set_result()`, `drop_packet()` and `modify()` methods.

WASM plugins (contract 0.3.0) subscribe to raw packets with `event-bus.subscribe-packets` and packet filters, which needs `raw-packet`. Each matching packet waits for a guest call, so a codec filter is the better tool on the hot path; see [Raw packets](../wasm/events#raw-packets).

| Field | Type | Description |
|-------|------|-------------|
| `player_id` | `PlayerId` | The player this packet belongs to |
| `direction` | `PacketDirection` | `Serverbound` (client to server) or `Clientbound` (server to client) |
| `packet` | `RawPacket` | The raw packet data |

**Results** (`RawPacketResult`):

| Variant | Description |
|---------|-------------|
| `Pass` (default) | Forward unmodified |
| `Modify { packet }` | Replace with a different packet |
| `Drop` | Silently discard the packet |

Packet events use a different subscription API. Instead of subscribing to all packets, you register a `PacketFilter` for the specific packet ID, connection state, and direction you care about:

```rust
use infrarust_api::event::{PacketFilter, ConnectionState, PacketDirection};

let filter = PacketFilter {
    packet_id: 0x03,
    state: ConnectionState::Play,
    direction: PacketDirection::Serverbound,
};

ctx.event_bus().subscribe_packet_typed(
    filter,
    EventPriority::NORMAL,
    |event| {
        tracing::debug!("Received packet 0x03 from {:?}", event.player_id);
    },
);
```

The proxy skips event dispatch for packets with no registered listeners, so this is efficient even at high packet rates.

::: warning
Packet events run on every matching packet in the forwarding loop. Keep handlers fast to avoid adding latency.
:::

## Proxy events

These events relate to the proxy itself rather than individual players.

### ProxyPingEvent

Fired when a client pings the server list, before the proxy answers. You can modify the response to customize the MOTD, player count, version, favicon and player sample. It fires for:

- a ping to a known domain, with the response relayed from the backend, or the cached, configured or synthetic one when the backend does not answer;
- a ping to an unknown domain when [`unknown_domain_behavior`](../../configuration/global#unknown-domain-behavior) is `default_motd` (the default), with the response built from `[default_motd]`. With `drop`, the connection is closed and no event fires;
- a ping from a client older than 1.7, with `legacy: true`. Legacy pings are always answered, with the `[default_motd]` response when the ping sends no host or an unknown one.

The `ProxyPingEvent` does not implement `ResultedEvent`. Instead, mutate the `response` field directly. The event is awaited: a listener that runs past `handler_timeout` is cancelled, and the client gets the response as the listeners before it left it.

| Field | Type | Description |
|-------|------|-------------|
| `remote_addr` | `SocketAddr` | The pinging client's address. Behind a load balancer that sends the PROXY protocol, the address from the header |
| `server` | `Option<ServerId>` | The server the domain routes to. `None` for an unknown domain, or a legacy ping that sends no host |
| `virtual_host` | `Option<String>` | The domain from the handshake, lowercased, without the Forge marker or trailing dot. `None` when a legacy ping sends no host |
| `protocol_version` | `ProtocolVersion` | The client's protocol version. For a legacy ping, the pre-1.7 protocol number, or 0 for the ping formats that carry none (before 1.6) |
| `legacy` | `bool` | `true` for a ping from a client older than 1.7 |
| `response` | `PingResponse` | The response to send back (mutable) |

`ProxyPingEvent::new` builds one, for tests.

`PingResponse` fields:

| Field | Type | Description |
|-------|------|-------------|
| `description` | `Component` | The MOTD shown in the server list |
| `max_players` | `i32` | Maximum player count |
| `online_players` | `i32` | Current online player count |
| `protocol_version` | `ProtocolVersion` | The protocol version to report |
| `version_name` | `String` | Version name string (e.g. "Infrarust 2.0") |
| `favicon` | `Option<String>` | Base64-encoded 64x64 PNG, if any |
| `player_sample` | `Vec<(String, Uuid)>` | The names shown when hovering over the player count, with their UUIDs. An entry the backend sent with an id that is not a UUID reads as the nil UUID |

The proxy keeps the backend's own JSON for what no listener changed: an untouched `description` keeps its formatting and events as the backend sent them, and an untouched `player_sample` is relayed as sent. Fields the proxy does not model, such as Forge's `forgeData`, always pass through.

```rust
ctx.event_bus().subscribe::<ProxyPingEvent, _>(
    EventPriority::NORMAL,
    |event| {
        let resp = event.response_mut();
        resp.description = Component::text("My Minecraft Network").color("gold");
        resp.max_players = 500;
        resp.player_sample = vec![("Join us!".into(), uuid::Uuid::nil())];
    },
);
```

WASM plugins (contract 0.3.0) receive `proxy-ping` with the same fields and the whole response, `player_sample` included.

#### Legacy pings

A legacy ping answer only carries the MOTD, the player counts and, for clients from 1.4, the protocol number and version name. The response starts from the backend's legacy answer (relayed for a 1.6 ping with a host) or from the configuration, with the MOTD as plain text that keeps its `§` codes. A changed `description` is sent as `Component::to_legacy('§')`, an untouched one as it was. `favicon` and `player_sample` are ignored.

### ProxyInitializeEvent

Fired after the proxy finishes startup and all plugins are loaded. No fields. Use this for setup that depends on other plugins being ready.

### ProxyShutdownEvent

Fired during shutdown, once every player session has ended and before any plugin is disabled. No fields. Use this or `Plugin::on_disable` for resource cleanup. See [Proxy shutdown](#proxy-shutdown) for the full sequence.

### ConfigReloadEvent

Posted when a config provider changed the proxy's servers. Informational, delivered through the ordered queue. One event covers one batch of changes from one provider:

- the file provider: one scan of the servers directory, which runs once the directory has been quiet for 300 ms (at most 2 seconds after the first change), so files written together are one event;
- the Docker provider: one container change;
- a plugin config provider: one `send`.

It lists the servers the batch actually changed. A batch that changes nothing, such as a server file rewritten with the same content, posts no event. The servers loaded at startup post none either.

| Field | Type | Description |
|-------|------|-------------|
| `provider` | `String` | The provider type: `file`, `docker`, or `plugin:<plugin id>:<provider type>` for a plugin config provider |
| `added` | `Vec<ServerId>` | Servers that did not exist before, sorted by ID |
| `removed` | `Vec<ServerId>` | Servers that no longer exist, sorted by ID |
| `updated` | `Vec<ServerId>` | Servers whose configuration changed, sorted by ID. A file whose `id` changed counts as the old server removed and the new one added |

`is_empty()` tells whether all three lists are empty, which never happens for a posted event.

```rust
ctx.event_bus().subscribe::<ConfigReloadEvent, _>(
    EventPriority::NORMAL,
    |event| {
        for server in &event.removed {
            tracing::info!("{server} was removed by the {} provider", event.provider);
        }
    },
);
```

WASM plugins (contract 0.3.0) receive `config-reload` with `provider`, `added`, `removed` and `updated`.

### ServerStateChangeEvent

Fired when a backend server changes state (online, offline, starting, stopping, sleeping, crashed).

| Field | Type | Description |
|-------|------|-------------|
| `server` | `ServerId` | The server whose state changed |
| `old_state` | `ServerState` | Previous state |
| `new_state` | `ServerState` | New state |

`ServerState` variants: `Online`, `Offline`, `Starting`, `Stopping`, `Sleeping`, `Crashed`.

```rust
ctx.event_bus().subscribe::<ServerStateChangeEvent, _>(
    EventPriority::NORMAL,
    |event| {
        if matches!(event.new_state, ServerState::Crashed) {
            tracing::error!("Server {:?} crashed!", event.server);
        }
    },
);
```

### BackendHealthEvent

Fired when a backend address changes health state, for example when it stops accepting connections or recovers. Informational, delivered through the ordered queue.

| Field | Type | Description |
|-------|------|-------------|
| `address` | `ServerAddress` | The backend address whose health changed |
| `servers` | `Vec<ServerId>` | The servers that list this address |
| `state` | `BackendState` | The new health state |

## Ban events

The [ban service](./bans) posts these after the active ban provider changed its bans, whichever provider it is and whoever made the change: the console, the admin API, a plugin. They are queued, so listeners see them in the order the bans were issued and revoked. Plugins can listen to them but not fire them.

### BanIssuedEvent

Posted after a ban was stored, before any online player it matches is kicked.

**Type:** Informational

| Field | Type | Description |
|-------|------|-------------|
| `entry` | `BanEntry` | The stored ban, with the `id` its provider gave it |
| `source` | `BanSource` | Who issued it: `Console`, `WebApi`, `Plugin(id)`, `Player` or `System` |
| `silent` | `bool` | The issuer asked for the ban not to be announced |

### BanRevokedEvent

Posted after a ban was removed. Nothing is posted when the target was not banned.

**Type:** Informational

| Field | Type | Description |
|-------|------|-------------|
| `entry` | `BanEntry` | The ban that was removed |
| `source` | `BanSource` | Who removed it |
| `silent` | `bool` | The remover asked for this not to be announced |

```rust
ctx.event_bus().subscribe::<BanIssuedEvent, _>(EventPriority::NORMAL, |event| {
    if !event.silent {
        tracing::info!(
            "{} banned {}: {}",
            event.source,
            event.entry.target,
            event.entry.reason.as_deref().unwrap_or("no reason"),
        );
    }
});
```

WASM plugins (contract 0.3.0) receive `ban-issued` and `ban-revoked` with the entry, the source and `silent`.

## Plugin events

The plugin manager and the [service registry](./services) post these. They are queued like the other informational proxy events, and plugins can listen to them but not fire them.

### PluginEnabledEvent

Posted right after a plugin's `on_enable` succeeded, once per plugin, in enable order.

**Type:** Informational

| Field | Type | Description |
|-------|------|-------------|
| `plugin_id` | `String` | The plugin that was enabled |
| `version` | `String` | Its version from `PluginMetadata` |

The manager waits for the event to be delivered before it enables the next plugin. A plugin sees its own `PluginEnabledEvent` and those of every plugin enabled after it. The plugins enabled before it are in the [plugin registry](./api#plugincontext).

### PluginDisabledEvent

Posted after a plugin was disabled and its resources cleaned up, on shutdown or by `disable_plugin`. On shutdown the events follow `ProxyShutdownEvent`, in reverse enable order.

**Type:** Informational

| Field | Type | Description |
|-------|------|-------------|
| `plugin_id` | `String` | The plugin that was disabled |

A plugin does not receive its own `PluginDisabledEvent`: its listeners are removed before the event is posted.

### ServiceProvidedEvent

Posted when a plugin provided a service through `ctx.services().provide`.

**Type:** Informational

| Field | Type | Description |
|-------|------|-------------|
| `service` | `&'static str` | The Rust type name of the service, for logs |
| `provider` | `String` | The plugin that provides it |

`event.is::<T>()` tells whether the service is `T`:

```rust
ctx.event_bus().subscribe::<ServiceProvidedEvent, _>(EventPriority::NORMAL, |event| {
    if event.is::<dyn LoginState>() {
        tracing::info!("logins are now answered by {}", event.provider);
    }
});
```

### ServiceRemovedEvent

Posted when a service was withdrawn through its `ServiceHandle`, or because its provider was disabled. It has the same fields and `is::<T>()` check as `ServiceProvidedEvent`. When a plugin is disabled, its `ServiceRemovedEvent`s come before its `PluginDisabledEvent`.

WASM plugins (contract 0.3.0) receive `plugin-enabled` and `plugin-disabled`. The service events stay native-only.

## Custom events

Plugins can define and fire their own events. Any type that implements `Event` works: listeners subscribe to it with `subscribe` or `subscribe_async` as usual, and the firing plugin calls `fire` on its event bus.

```rust
use infrarust_api::prelude::*;

pub struct PartyInvite {
    pub from: String,
    pub to: String,
    pub accepted: Option<bool>,
}

impl Event for PartyInvite {}
```

`fire` dispatches the event inline and returns it once every listener has run, so listeners can answer by writing into it:

```rust
ctx.event_bus().subscribe::<PartyInvite, _>(EventPriority::NORMAL, |invite| {
    invite.accepted = Some(invite.to != "Mallory");
});

let bus = ctx.event_bus_handle();
let invite = bus
    .fire(PartyInvite {
        from: "Steve".into(),
        to: "Alex".into(),
        accepted: None,
    })
    .await?;
```

`fire` returns `Result<E, FireError>`. The event types defined by `infrarust_api::events` belong to the proxy: firing one of them from a plugin returns `Err(FireError::Reserved)` and no listener runs, so a plugin cannot forge a `PreLoginEvent` or a `ConfigReloadEvent`. `NamedEvent` is the one exception, since it exists for plugins to fire.

Two plugins exchange a custom event only if they use the same Rust type. Put the event types in a crate both plugins depend on.

To fire from inside a listener, keep the `Arc<dyn EventBus>` returned by `ctx.event_bus_handle()` in the listener and call `fire` on it. A listener that fires an event waits for that event's listeners, and the time counts against its own `handler_timeout`.

### NamedEvent

`NamedEvent` is a general-purpose event identified by a string name, with an opaque payload. It needs no shared Rust type, only an agreement on the name and the payload format.

| Field | Type | Description |
|-------|------|-------------|
| `name` | `String` | The message name listeners filter on |
| `source_plugin` | `String` | The ID of the plugin that fired the event |
| `content_type` | `String` | The payload format, for example `application/json` |
| `payload` | `Bytes` | The message body |
| `cancelled` | `bool` | Set by a listener through `cancel()` |
| `response` | `Option<NamedEventResponse>` | Set by a listener through `respond(content_type, payload)` |

Create one with `NamedEvent::new(name, content_type, payload)`. The proxy overwrites `source_plugin` with the ID of the firing plugin, so listeners can trust it. Every `NamedEvent` listener receives every named event, so check `name` first:

```rust
use infrarust_api::events::NamedEvent;

ctx.event_bus().subscribe::<NamedEvent, _>(EventPriority::NORMAL, |event| {
    if event.name != "economy:balance" {
        return;
    }
    event.respond("text/plain", "42");
});

let answered = ctx
    .event_bus_handle()
    .fire(NamedEvent::new("economy:balance", "text/plain", "Steve"))
    .await?;
```

WASM plugins (contract 0.3.0) fire and subscribe to `NamedEvent` with `event-bus.fire-named` and `subscribe-named`, and exchange them with native plugins. Other custom event types stay native-only. See [Named events](../wasm/events#named-events).
