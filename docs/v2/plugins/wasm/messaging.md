---
title: Plugin Messaging
description: Register plugin channels, receive and decide on plugin messages, and send them to players, backends and servers from a WASM plugin. Includes the proxy-handled BungeeCord channel and how a plugin takes it over.
outline: [2, 3]
---

# Plugin Messaging

Minecraft clients, backend servers and proxies talk to each other on named channels through plugin message packets (Custom Payload). Client mods announce their channels, the client tells the server its brand on `minecraft:brand`, and Bukkit plugins ask the proxy for things on `BungeeCord`. A WASM plugin can register channels, hear the messages sent on them in either direction, decide what happens to each one, and send its own.

The model is the one native plugins use; [native Plugin Messaging](../dev/messaging) has the full routing order, the client state the proxy keeps and the BungeeCord subchannel reference. This page covers the WASM side: the `messaging` interface, `PluginMessageEvent` and the SDK's `Messaging` service.

## Requirements

Plugin messaging needs the opt-in `plugin-messaging` capability:

```toml
[plugins.my-plugin]
permissions = ["plugin-messaging"]
```

| Call | Without `plugin-messaging` |
|------|----------------------------|
| `Messaging::register`, `unregister`, `channels`, `send_to_player`, `send_to_backend`, `send_to_server` | `Error` of kind `PermissionDenied`, `missing capability: plugin-messaging` |
| `ctx.on::<PluginMessageEvent>(..)` | The subscription is refused with `PermissionDenied`; the handler never runs |

The client state events (`PlayerClientBrandEvent`, `PlayerSettingsChangedEvent`, `PlayerChannelRegisterEvent`) and the `client_brand`, `settings` and `known_channels` fields of `PlayerInfo` only need the baseline capabilities.

The proxy reads plugin messages only for players on `offline` and `client_only` servers. In the forwarding modes (`passthrough`, `zero_copy`, `server_only`) it copies bytes: no `PluginMessageEvent` fires, and a send to such a player returns `InvalidState`. See [Capabilities](./capabilities).

## Channel identifiers

A channel has a namespaced name since Minecraft 1.13 (`namespace:name`) and a free-form legacy name before (`BungeeCord`, `MC|Brand`). The SDK's `ChannelId` holds one or both:

```rust
let modern = ChannelId::modern("myplugin:sync");          // same name on every version
let legacy = ChannelId::legacy("MyPluginSync");           // same name on every version
let paired = ChannelId::pair("myplugin:sync", "MyPluginSync"); // legacy name below 1.13
let bungee = ChannelId::bungeecord();                    // bungeecord:main and BungeeCord
```

The SDK does not check the names; the host does on every call and returns `InvalidArgument` for a bad one. A modern name is lowercase `namespace:name` (letters, digits, `_`, `-`, `.`, and `/` in the name part), at most 128 characters. A legacy name is 1 to 20 characters without NUL. When the proxy sends a paired channel, it uses the legacy name for clients and backends below 1.13 and the modern name from 1.13.

`ChannelId` has `modern_id()`, `legacy_name()` and `matches(raw)`, which is true when `raw` is either name. Compare channels with `matches` or `modern_id()` rather than `==`: the channel in an event carries the names the proxy knows for it, which can include a legacy name another plugin registered alongside yours.

## Registering channels

A plugin only hears about channels it registered:

| Method | Returns | Description |
|--------|---------|-------------|
| `Messaging::register(&channel)` | `Result<(), Error>` | Fire `PluginMessageEvent` for this channel, under each of its names |
| `Messaging::unregister(&channel)` | `Result<bool, Error>` | Stop; `true` when this plugin had registered it |
| `Messaging::channels()` | `Result<Vec<ChannelId>, Error>` | The channels this plugin registered |

Registrations belong to the plugin. They are removed when the plugin is disabled or unloaded, and they are kept across a [recovery](./fault-model), since they live on the host; registering the same channel again in the new instance's `on_enable` changes nothing. When two plugins register the same channel, each message fires one event that both see, and the channel stays registered until both removed it.

Registering a channel also makes the proxy announce it to backends with `minecraft:register` (`REGISTER` below 1.13), next to the client's own channels, because a Bukkit plugin only sends a message on a channel the player registered. A channel registered later reaches the backends players join afterwards. Sending does not require registering the channel.

## Receiving messages

`PluginMessageEvent` fires for a plugin message on a registered channel, from the client or from the backend, in the configuration phase (1.20.2 and later) and in play. Messages on channels nobody registered are forwarded byte for byte and never reach a plugin.

| Field | Type | Description |
|-------|------|-------------|
| `player` | `PlayerRef` | The player whose connection carries the message (`id`, `uuid`, `username`) |
| `source` | `MessageEndpoint` | `Client`, or `Backend(server)` for the server that sent it |
| `channel` | `ChannelId` | The registered channel, with the names the proxy knows for it |
| `raw_channel` | `String` | The name as it was sent on the wire |
| `data` | `Vec<u8>` | The payload |
| `phase` | `MessagePhase` | `Configuration` or `Play` |

`from_client()` is true when `source` is `Client`. The event is resulted:

| `PluginMessageResult` | Shortcut | Effect |
|-----------------------|----------|--------|
| `Forward` (default) | `forward()` | Send the message on to the other side unchanged |
| `Handled` | `handled()` | The plugin consumed it: nothing is forwarded |
| `Replace(Vec<u8>)` | `replace(data)` | Forward `data` instead, on the same channel |

`result()` returns the result set so far, by this handler or an earlier one, and `set_result` sets any variant. A handler that changes nothing leaves the result as it found it.

A complete plugin that answers a client mod on its own channel and logs what backends send on it:

```rust
use infrarust_plugin_sdk::prelude::*;

const SYNC: &str = "myplugin:sync";

#[derive(Default)]
struct SyncPlugin;

#[plugin(id = "sync-plugin", name = "Sync Plugin")]
impl Plugin for SyncPlugin {
    fn on_enable(&self, ctx: &Context) -> Result<(), PluginError> {
        Messaging::register(&ChannelId::pair(SYNC, "MyPluginSync"))?;

        ctx.on::<PluginMessageEvent>(EventPriority::Normal, |event| {
            if event.channel.modern_id() != Some(SYNC) {
                return;
            }
            match &event.source {
                MessageEndpoint::Client => {
                    // Answer the client and keep its message from the backend.
                    let _ = Messaging::send_to_player(event.player.id, &event.channel, b"pong");
                    event.handled();
                }
                MessageEndpoint::Backend(server) => {
                    info!(
                        "{server} sent {} bytes for {}",
                        event.data.len(),
                        event.player.username
                    );
                }
                _ => {}
            }
        })?;

        Ok(())
    }
}
```

`MessageEndpoint` is `#[non_exhaustive]`, hence the last arm.

### Timing

The event is awaited in the player's session: the message waits for every listener, native and WASM, before the proxy forwards it, and that player's session reads no other packet in the meantime. Your handler reaches the guest through the plugin's queue like every other call, and the proxy waits for it at most `[events] handler_timeout`. A plugin that does not answer in time leaves the message with the result the earlier handlers set. Keep these handlers short, and see [Threading and Concurrency](./threading) for what else can hold the queue.

The proxy drops a client's message on `BungeeCord`, `bungeecord:main` or any `velocity:` channel before any plugin sees it, since backend plugins trust those channels to come from the proxy.

## Sending messages

| Method | Returns | Sends |
|--------|---------|-------|
| `Messaging::send_to_player(player, &channel, data)` | `Result<(), Error>` | To the player's client |
| `Messaging::send_to_backend(player, &channel, data)` | `Result<(), Error>` | To the backend the player is connected to |
| `Messaging::send_to_server(&server, &channel, data)` | `Result<u32, Error>` | To a server, through the players on it |

```rust
let channel = ChannelId::modern("myplugin:sync");

if let Some(player) = Players::by_name("Notch") {
    Messaging::send_to_player(player.id(), &channel, b"to the client")?;
    Messaging::send_to_backend(player.id(), &channel, b"to the server")?;
}

match Messaging::send_to_server(&ServerId::new("lobby"), &channel, b"refresh") {
    Ok(copies) => debug!("sent through {copies} player(s)"),
    Err(e) if e.kind == ErrorKind::Unavailable => debug!("nobody on lobby to carry it"),
    Err(e) => warn!("sync failed: {e}"),
}
```

The sends queue the message for the player's session and return at once; they do not wait for delivery, so they are not subject to `host_call_timeout`. The session writes the message with the packet of the phase the connection is in (configuration or play) and the channel's name for the player's version. Messages sent while the player is still logging in wait and go out in order. A message a plugin sends does not fire `PluginMessageEvent`. [Sending plugin messages](../dev/messaging#sending-plugin-messages) in the native guide has the exact timing, including limbo.

A plugin message needs a player connection to travel on. `send_to_server` sends one copy through one online player for each distinct backend address of the server and returns how many it sent. With no player on the server it returns `Unavailable`: the message is not kept for a later player.

| Error kind | When |
|------------|------|
| `PermissionDenied` | The plugin lacks `plugin-messaging` |
| `InvalidArgument` | The channel name is malformed, or the payload is over the limit: 1 MiB (1 048 576 bytes) to a client, 32 767 bytes to a backend or server |
| `PlayerGone` | No player with that id is online, or the player is disconnecting |
| `InvalidState` | The player is on a forwarding-mode server, or `send_to_backend` was called while the player is not connected to a backend |
| `Unavailable` | The player's command queue is full, or no player on the server could carry a `send_to_server` message |

## The BungeeCord channel

Bukkit, Spigot and Paper plugins send requests to the proxy on the `BungeeCord` channel (`bungeecord:main` since 1.13): move a player, count players, list servers, forward a message to another server. Infrarust answers them itself when two switches are on, and both are off by default:

```toml
# infrarust.toml
[plugin_messaging]
bungeecord = true
```

```toml
# servers/lobby.toml
bungeecord_channel = true
```

`bungeecord_channel` needs an `offline` or `client_only` server, since the proxy must read the backend's packets. Every request is limited to the requesting server's network, and each subchannel can be allowed or refused in `[plugin_messaging.bungeecord_permissions]`. The subchannels, their byte layouts and the defaults are listed in [native Plugin Messaging](../dev/messaging#the-bungeecord-channel).

### Taking over BungeeCord messages

A plugin that registers `ChannelId::bungeecord()` receives each BungeeCord message a backend sends, before the proxy acts on it. Its handler decides what the proxy does next:

| Result | Channel enabled for that server | Channel disabled |
|--------|---------------------------------|------------------|
| `Forward` (default) | The proxy answers the request | The message goes on to the client |
| `Handled` | The proxy does nothing: the plugin owns the request | Nothing is forwarded |
| `Replace(data)` | The proxy answers the replaced request | The replaced message goes to the client |

Clients cannot send on this channel, so these messages always come from a backend. To answer one, send the response on `ChannelId::bungeecord()` with `send_to_backend`, to the player whose connection carried the request; the proxy picks `BungeeCord` or `bungeecord:main` for the player's version.

Requests and responses are Java `DataOutput` streams: a string (`writeUTF`) is a big-endian `u16` byte count followed by the text in modified UTF-8, which is plain UTF-8 for text without NUL or characters above U+FFFF. This plugin adds a subchannel of its own and leaves the others to the proxy:

```rust
fn register_bungeecord(ctx: &Context) -> Result<(), PluginError> {
    Messaging::register(&ChannelId::bungeecord())?;

    ctx.on::<PluginMessageEvent>(EventPriority::Normal, |event| {
        if !event.channel.matches("BungeeCord") {
            return;
        }
        let Some(("MyPluginPing", _request)) = read_utf(&event.data) else {
            return;
        };
        // Ours: the proxy must neither answer nor forward it.
        event.handled();

        let mut answer = Vec::new();
        write_utf(&mut answer, "MyPluginPing");
        write_utf(&mut answer, &event.player.username);
        let _ = Messaging::send_to_backend(event.player.id, &ChannelId::bungeecord(), &answer);
    })?;
    Ok(())
}

fn read_utf(data: &[u8]) -> Option<(&str, &[u8])> {
    let (len, rest) = data.split_first_chunk::<2>()?;
    let len = usize::from(u16::from_be_bytes(*len));
    let text = rest.get(..len)?;
    Some((std::str::from_utf8(text).ok()?, &rest[len..]))
}

fn write_utf(out: &mut Vec<u8>, text: &str) {
    let len = u16::try_from(text.len()).unwrap_or(0);
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(&text.as_bytes()[..usize::from(len)]);
}
```

A request for any other subchannel keeps the default `Forward`, so the proxy answers it when the channel is enabled for that server.

## Client state

The proxy keeps what the client said about itself and replays it to every server a switch reaches. `Players::get(id)` returns a `PlayerInfo` with `client_brand`, `settings` and `known_channels` (`None` or empty until the client sent them), and three observe-only events report changes:

| Event | Fields | Fires when |
|-------|--------|------------|
| `PlayerClientBrandEvent` | `player`, `brand` | The client sends a brand that differs from the last one |
| `PlayerSettingsChangedEvent` | `player`, `settings` | The client sends settings that differ from the last ones |
| `PlayerChannelRegisterEvent` | `player`, `channels`, `direction` | The client (`PacketDirection::Serverbound`) or the backend (`Clientbound`) registers channels |

These events are queued, not awaited: they can arrive after the player left. See [Client state](../dev/messaging#client-state) for what each field holds and when the proxy replays it.

## See also

- [Plugin Messaging (native)](../dev/messaging): routing order, send timing, client state and the BungeeCord subchannels.
- [Events](./events#plugin-messages): `PluginMessageEvent` among the other events.
- [Host Services](./services#plugin-messaging): the `Messaging` service next to the others.
- [Capabilities](./capabilities): `plugin-messaging` and the other opt-ins.
- [Threading and Concurrency](./threading): how a handler reaches the guest.
- [Global Settings](../../configuration/global#plugin-messaging): `[plugin_messaging]` and the BungeeCord permissions.
