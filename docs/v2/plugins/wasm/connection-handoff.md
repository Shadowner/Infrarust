---
title: Connection Handoff
description: Claim a raw connection by its first bytes, a handshake or a login, and let the proxy relay it to another target without the bytes ever entering the plugin.
outline: [2, 3]
---

# Connection Handoff

Some mods open their own protocol on the Minecraft port. A mod that transfers files before the game logs in, for example, connects to the server address and expects to reach its own file server, not a Minecraft login. A plugin can recognise such a connection and tell the proxy to stop treating it as Minecraft: the proxy then pipes it, byte for byte, to another target.

The plugin decides, the host relays. The plugin only sees the first bytes and the parsed handshake or Login Start; the traffic of a handed-off connection never crosses into the plugin, so a download of hundreds of megabytes costs the plugin nothing.

| Level | When | Hook | When nobody answers |
| --- | --- | --- | --- |
| Raw claim | First bytes, before the Minecraft handshake is parsed, only when they start with a prefix a plugin registered | `RawConnectionEvent` | `Pass`: the connection goes on as Minecraft |
| Handshake | Handshake parsed and routed | `ConnectionHandshakeEvent` result `Handoff` | `Allow`, as today |
| Login claim | Login Start read, before `PreLoginEvent`, the ban checks and admission | `LoginClaimEvent` | `Pass`: a normal login |
| Login plugin messages | A backend login plugin request, a client login plugin response | `LoginPluginRequestEvent`, `LoginPluginResponseEvent` | `Forward` |

The first three end with a [handoff](#the-handoff). The login plugin messages do not hand anything off: they let a plugin read, answer or rewrite the messages a modded backend exchanges with the client during the login, see [Login plugin messages](./messaging#login-plugin-messages).

## The `connection-handoff` capability

Claiming prefixes, subscribing to `RawConnectionEvent` and `LoginClaimEvent`, and answering `ConnectionHandshakeEvent` with a handoff need the opt-in `connection-handoff` capability:

```toml
[plugins.transfer-bridge]
permissions = ["connection-handoff"]
```

Without it, `ConnectionClaims::register_prefix` and the two subscriptions return a `PermissionDenied` error, and a handoff set on a `ConnectionHandshakeEvent` is ignored: the result the event had stays, and the host logs `wasm plugin set a result it is not allowed to set; ignoring it`, at most five times a minute per plugin. Compiled-in plugins hold the capability unless their config denies it. The login plugin message events need `plugin-messaging` instead.

## The handoff

A `Handoff` names where the connection goes and what the proxy writes before it starts relaying.

```rust
use infrarust_plugin_sdk::prelude::*;

let handoff = Handoff::to(HandoffTarget::server_port("modded", 30037))
    .with_prefix(b"XFER".to_vec())
    .with_reply(b"OK".to_vec())
    .without_replay()
    .with_proxy_protocol(false);
```

| Field | Default | Meaning |
| --- | --- | --- |
| `target` | | Where the connection goes, see [Targets](#targets) |
| `replay` | `true` | Send the bytes the connection already sent to the target, see [Replay](#replay) |
| `prefix` | empty | Bytes written to the target before the replay |
| `reply` | empty | Bytes written to the client before the relay starts |
| `proxy_protocol` | `None` | `Some(true)` sends a PROXY protocol v2 header to the target, `Some(false)` never does, `None` follows the target server's `send_proxy_protocol` |

`Handoff::to(target)` builds one with the defaults, `Handoff::route()` is `Handoff::to(HandoffTarget::Route)`, and `with_prefix`, `with_reply`, `without_replay`, `with_replay` and `with_proxy_protocol` change a field. The native `infrarust_api::handoff::Handoff` has the same builders and takes anything that converts into `Bytes`.

### Targets

| Target | Connects to |
| --- | --- |
| `Route` | The server the connection was routed to by its domain. The backend address is chosen by the server's load balancer exactly as for a normal connection, with the same health checks. Not available on a raw claim, which has no route yet: the connection is closed with a warning |
| `Server(id)` | Another configured server, chosen the same way. An unknown id closes the connection with a warning |
| `ServerPort { server, port }` | The host of the address the load balancer picks for `server`, on `port` instead of the Minecraft port. Meant for a file server that runs next to the Minecraft backend |
| `Address { host, port }` | An explicit address. For a WASM plugin, the host checks it against the plugin's [network allow-list](./network) when it reads the answer, see [Address targets](#address-targets) |

The connection uses the target server's `timeouts.connect`, or the proxy's `connect_timeout` for an `Address`. A target that cannot be reached closes the client connection.

### Replay

With `replay` on, the target receives `prefix` and then what the connection already sent, so the target sees the conversation from its start:

- after a raw claim, the bytes the proxy read, exactly as received (a PROXY protocol header the proxy received is already gone);
- after a handshake, the handshake as the proxy forwards it in passthrough without identity forwarding (the target server's [`domain_rewrite`](../../configuration/servers#fields-reference) applies and the Forge marker stays), then every byte the client sent after it;
- after a login claim, the same handshake, the Login Start packet as received, then every byte the client sent after it.

With `replay` off the target only gets `prefix` and then whatever the client sends next.

### The relay

The proxy writes `reply` to the client and `prefix` and the replay to the target, then relays both directions until either side closes or the proxy shuts down. On Linux it uses `splice(2)`, so the bytes stay in the kernel. A handed-off connection is not a player: there is no `PreLoginEvent`, `LoginEvent`, `PostLoginEvent` or `DisconnectEvent` for it. It keeps its slot under `max_connections` until it closes, and the transport filters see it close like any connection.

## Claiming raw connections

A raw claim catches a connection before the proxy parses it as Minecraft. A plugin registers the byte prefixes its protocol starts with, usually in `on_enable`:

```rust
ConnectionClaims::register_prefix(b"XFER")?;
```

As long as no plugin holds a prefix, the proxy reads nothing extra. Once one does, it reads the first bytes of each connection until they either start with a registered prefix or can no longer match any. A Minecraft client is told apart by its very first bytes, so it waits for nothing. A client that sent part of a prefix and stops is given 5 seconds, then handled as Minecraft.

A prefix must be 2 to 16 bytes long and must not be able to start a Minecraft handshake, a pre-1.7 ping or login, or a PROXY protocol header: the host refuses a prefix that starts with `0xFE` or `0x02`, with one byte from `0x01` to `0x7F` followed by `0x00`, with one byte from `0x80` up followed by `0x01` to `0x03` and then `0x00` (or nothing), and one that matches the start of `PROXY ` or of the PROXY v2 signature. Such a prefix answers `InvalidArgument`. A plugin holds at most 16 prefixes (`LimitExceeded` past that); registering one it holds again succeeds, and `unregister_prefix` of one it does not hold answers `NotFound`. Two plugins may register the same prefix. The host drops a plugin's prefixes when it is disabled or unloaded, and when a fault replaces its instance, before the fresh instance runs `on_enable` and registers them again.

### `RawConnectionEvent`

| Field | Type | Meaning |
| --- | --- | --- |
| `connection_id` | `u64` | The proxy's id for the connection |
| `remote_addr` | `SocketAddr` | The client, from the PROXY protocol header when there is one |
| `local_addr` | `SocketAddr` | The listener address the client reached |
| `prefix` | `Vec<u8>` | The registered prefix that matched |
| `data` | `Vec<u8>` | Every byte read so far, the prefix included |

The event goes to every listener of `RawConnectionEvent`, whichever plugin registered the prefix, so check `prefix` before you act.

| Result | Helper | Effect |
| --- | --- | --- |
| `Pass` (default) | `pass()` | Handle the connection as Minecraft; the bytes read stay in front of the stream |
| `NeedMore(n)` | `need_more(n)` | Read until `data` holds `n` bytes, then fire the event again with the longer `data`. At most 4096 bytes and 8 rounds; a client that does not send them within 5 seconds is closed |
| `Handoff(handoff)` | `handoff(h)` | Relay the connection, see [The handoff](#the-handoff). `Route` is refused here |
| `Reject` | `reject()` | Close the connection without a word |

The IP filter and the global IP bans run before the event, as for any connection. The rate limiter does not: it counts Minecraft handshakes, and a claimed connection never sends one. Keep prefixes specific to your protocol, since every connection that starts with one reaches the plugin before any rate limit.

```rust
use infrarust_plugin_sdk::prelude::*;

#[derive(Default)]
struct TransferBridge;

#[plugin(id = "transfer-bridge", name = "Transfer Bridge")]
impl Plugin for TransferBridge {
    fn on_enable(&self, ctx: &Context) -> Result<(), PluginError> {
        ConnectionClaims::register_prefix(b"XFER")?;
        ctx.on::<RawConnectionEvent>(EventPriority::NORMAL, |event| {
            if event.prefix != b"XFER" {
                return;
            }
            if event.data.len() < 8 {
                event.need_more(8);
                return;
            }
            event.handoff(Handoff::to(HandoffTarget::server_port("modded", 30037)));
        })?;
        Ok(())
    }
}
```

## Handing off at the handshake

`ConnectionHandshakeEvent` fires once the handshake is parsed and routed. `handoff(h)` relays the connection instead of going on, whatever its intent: a server list ping can be handed off too. The event keeps its other rules; see [ConnectionHandshakeEvent](../dev/events#connectionhandshakeevent).

```rust
ctx.on::<ConnectionHandshakeEvent>(EventPriority::NORMAL, |event| {
    if event.raw_host.ends_with("\0XFER\0") {
        event.handoff(Handoff::route());
    }
})?;
```

## Claiming a login

`LoginClaimEvent` fires once the Login Start packet is read, before `PreLoginEvent`, the ban checks and admission, and only when some plugin listens to it.

| Field | Type | Meaning |
| --- | --- | --- |
| `remote_addr` | `SocketAddr` | The client |
| `virtual_host` | `Option<String>` | The domain, lowercased, without the Forge marker |
| `raw_host` | `String` | The host as the client sent it |
| `port` | `u16` | The port from the handshake |
| `protocol` | `i32` | The client's protocol version |
| `server` | `Option<ServerId>` | The server the domain routes to |
| `username` | `String` | The name from the Login Start |
| `uuid` | `Option<Uuid>` | The UUID the client claimed, when its version sends one |
| `login_start` | `Vec<u8>` | The Login Start payload as sent, without its length and packet id |
| `layout_matches` | `bool` | `false` when the payload does not match the layout of the client's protocol and only a lenient decode read the name |

`pass()` keeps the normal login, `handoff(h)` relays the connection. A claimed login skips `PreLoginEvent`, the ban checks, admission and the proxy mode's login: the target gets the handshake and the Login Start as the client sent them.

Some modded clients send a Login Start that does not match their declared protocol. While someone listens to `LoginClaimEvent`, the proxy reads such a packet on every route so the plugin can claim it; a connection the plugin leaves alone is then refused on an `offline` or `client_only` route exactly as before.

## Address targets

For a WASM plugin, the host checks an `Address` target against `[plugins.<id>.wasm.network] allow` when it reads the plugin's answer. An IP address needs an IP or range rule that covers it and the port; a hostname needs a hostname rule for that name and port, and the proxy resolves the name itself when it connects. Without the `network` capability, or with an empty allow list, every address is refused. A refused address leaves the result as it was and logs the same rate-limited warning as a missing capability. `Route`, `Server` and `ServerPort` only reach configured servers and are not checked. Native plugins are not restricted.

## Security

A handoff skips authentication. The proxy does not check a claimed login against Mojang, runs no ban check and sends no forwarding data (no Velocity or BungeeCord identity): the target sees an unauthenticated client. Only hand a connection to a target that takes it over itself, such as a file server, or a backend that authenticates its own players. Never hand a login off to a backend that trusts the proxy to have authenticated it.

`reply` goes to a client nobody authenticated, and a raw claim runs before the rate limiter: keep what the plugin does on these events cheap and bounded.

## Native plugins

A native plugin with the capability reaches the same hooks through the native API:

```rust
use infrarust_api::prelude::*;

if let Some(claims) = ctx.connection_claims() {
    claims
        .register_prefix(b"XFER")
        .map_err(|error| PluginError::InitFailed(error.to_string()))?;
}
ctx.event_bus().subscribe::<RawConnectionEvent, _>(EventPriority::NORMAL, |event| {
    if event.prefix.as_ref() == b"XFER" {
        event.handoff(Handoff::to(HandoffTarget::server_port("modded", 30037)));
    }
});
```

`ctx.connection_claims()` answers `None` without `connection-handoff`. `register_prefix` and `unregister_prefix` answer a `ClaimError` (`InvalidPrefix`, `Limit`, `NotFound`), and the registrations go away when the plugin is disabled. See [Events](../dev/events#rawconnectionevent).

## See also

- [Events](./events): subscribing and the result pattern.
- [Plugin Messaging](./messaging#login-plugin-messages): the login plugin message events.
- [Network & Extra Folders](./network): the allow-list that bounds `Address` targets.
- [Capabilities](./capabilities): granting `connection-handoff`.
- [WIT API reference](./api-reference#handoffs-and-login-plugin-messages): the records behind these types.
