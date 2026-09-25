---
title: Plugin Messaging
description: Channel identifiers, plugin message events, sending to clients and backends, the client state the proxy keeps, and the BungeeCord channel.
outline: [2, 3]
---

# Plugin Messaging

Minecraft clients, backend servers and proxies talk to each other on named channels through plugin message packets (Custom Payload). Mods announce their channels with `minecraft:register`, the client tells the server its brand on `minecraft:brand`, and Bukkit plugins ask the proxy things on `BungeeCord`. Infrarust reads these messages in the `offline` and `client_only` modes, lets plugins watch and answer them, and keeps track of what the client said about itself so a server switch does not lose it.

In the forwarding modes (`passthrough`, `zero_copy`, `server_only`) the proxy only copies bytes: no event fires, the client state getters below return `None` or an empty list (except `virtual_host()`, known from the handshake), and every send returns `PlayerError::NotActive`.

WASM plugins (contract 0.2.3) have no plugin messaging. It comes with the next contract version; the 0.2.3 WIT does not change.

## Channel identifiers

A channel has a namespaced name since 1.13 (`namespace:name`) and a free-form legacy name before (`BungeeCord`, `MC|Brand`, at most 20 characters). `ChannelId` holds one or both:

```rust
use infrarust_api::messaging::ChannelId;

let modern = ChannelId::modern("myplugin:main")?;            // same name on every version
let legacy = ChannelId::legacy("MyPlugin")?;                  // same name on every version
let paired = ChannelId::pair("myplugin:main", "MyPlugin")?;  // legacy name below 1.13
let parsed = ChannelId::parse("myplugin:main")?;              // modern when it is namespaced

assert_eq!(paired.wire_name(ProtocolVersion::MINECRAFT_1_12), "MyPlugin");
assert_eq!(paired.wire_name(ProtocolVersion::MINECRAFT_1_13), "myplugin:main");
```

A modern name is lowercase `namespace:name` (letters, digits, `_`, `-`, `.`, and `/` in the name), at most 128 characters. `wire_name(version)` is the name sent to a client or backend of that protocol version: the legacy name below 1.13 when there is one, the modern name otherwise.

The built-in pairs complete themselves, so `ChannelId::modern("bungeecord:main")` and `ChannelId::legacy("BungeeCord")` are the same `ChannelId`:

| Constructor | Modern | Legacy (below 1.13) |
|-------------|--------|---------------------|
| `ChannelId::brand()` | `minecraft:brand` | `MC\|Brand` |
| `ChannelId::register()` | `minecraft:register` | `REGISTER` |
| `ChannelId::unregister()` | `minecraft:unregister` | `UNREGISTER` |
| `ChannelId::bungeecord()` | `bungeecord:main` | `BungeeCord` |

## Registering channels

A plugin only hears about the channels it registered. Register them in `on_enable`:

```rust
let channel = ChannelId::pair("myplugin:main", "MyPlugin")?;
ctx.channel_registrar().register(channel);
```

| Method | Returns | Description |
|--------|---------|-------------|
| `register(channel)` | `()` | Fire `PluginMessageEvent` for this channel, under each of its names |
| `unregister(&channel)` | `bool` | Stop, `true` when this plugin had registered it |
| `channels()` | `Vec<ChannelId>` | The channels this plugin registered |

Registrations belong to the plugin: they are removed when it is disabled. When two plugins register the same channel, the event fires once, for every listener, and the channel stays registered until both removed it.

The proxy tells backends about the registered channels, the way Velocity and BungeeCord do, because a Bukkit plugin only sends a plugin message on a channel the player registered: a `minecraft:register` (`REGISTER` below 1.13) message goes to the initial server once its `JoinGame` reached the client, and to every server a switch reaches, with the client's own channels. A channel registered later reaches the backends a player joins afterwards.

## PluginMessageEvent

Fired for a plugin message on a registered channel, from the client or from the backend, in the configuration phase (1.20.2+) and in play. It is awaited in the player's session: the message waits for every listener before the proxy forwards it, so keep listeners fast. Messages on channels no plugin registered are forwarded without being decoded beyond their channel name, byte for byte.

**Type:** Resulted

| Field | Type | Description |
|-------|------|-------------|
| `player` | `Arc<dyn Player>` | The player whose connection carries the message |
| `source` | `Endpoint` | `Client`, or `Backend(ServerId)` for the server that sent it |
| `channel` | `ChannelId` | The registered channel |
| `raw_channel` | `String` | The channel name as it was sent (`MyPlugin` or `myplugin:main`) |
| `data` | `Bytes` | The payload |
| `phase` | `MessagePhase` | `Configuration` or `Play` |

`player_id()` and `from_client()` are shortcuts.

**Results** (`PluginMessageResult`, `#[non_exhaustive]`, the shortcut method in parentheses):

| Variant | Description |
|---------|-------------|
| `Forward` (default, `forward()`) | Send the message on to the other side unchanged |
| `Handled` (`handled()`) | The plugin consumed it: nothing is forwarded |
| `Replace(Bytes)` (`replace(data)`) | Forward `data` instead, on the same channel |

```rust
use infrarust_api::events::messaging::PluginMessageEvent;
use infrarust_api::messaging::Endpoint;

ctx.event_bus().subscribe::<PluginMessageEvent, _>(EventPriority::NORMAL, |event| {
    if event.channel.modern_id() != Some("myplugin:main") {
        return;
    }
    if let Endpoint::Backend(server) = &event.source {
        tracing::info!("{server} says {:?}", event.data);
        event.handled();
    }
});
```

### Routing order

Every plugin message goes through the same steps, in play, in the configuration phase of the initial connection and in the configuration phase of a switch:

1. **Observers.** The proxy reads `minecraft:brand`, `minecraft:register` and `minecraft:unregister` (and their legacy names) from the client to keep the [client state](#client-state), and a `minecraft:register` from the backend for `PlayerChannelRegisterEvent`.
2. **Security.** A client message on `BungeeCord`, `bungeecord:main` or any `velocity:` channel is dropped. Backend plugins trust those channels to come from the proxy; a client must not be able to send one to its server.
3. **Plugin event.** `PluginMessageEvent`, when the channel is registered.
4. **BungeeCord.** A backend message on the BungeeCord channel is answered by the proxy when the [BungeeCord channel](#the-bungeecord-channel) is enabled for that server, and not forwarded to the client.
5. **Forward** to the other side.

Only the packet id is checked for other packets, and only the channel name is decoded for plugin messages, so the router costs nothing on the rest of the traffic.

## Sending plugin messages

```rust
let channel = ChannelId::pair("myplugin:main", "MyPlugin")?;
player.send_plugin_message(&channel, Bytes::from_static(b"to the client"))?;
player.send_plugin_message_to_backend(&channel, Bytes::from_static(b"to the server"))?;
```

The proxy picks the packet for the phase the connection is in when it sends the message: the configuration plugin message during the configuration phase, the play one otherwise, and the channel's `wire_name` for the player's version.

| | To the client | To the backend |
|---|---|---|
| Sent | In the configuration phase, until the proxy forwarded the server's Finish Configuration, and in game once `JoinGame` reached the client. Messages sent while the player logs in or between the configuration phase and `JoinGame` wait, in order | As soon as the connection to the backend is in the configuration phase or in play. During a switch, to the new backend once the switch is done |
| In limbo | Sent in play | Dropped: there is no backend |
| Largest payload | 1 MiB (`MAX_TO_CLIENT_PAYLOAD`) | 32767 bytes (`MAX_TO_BACKEND_PAYLOAD`) |
| Errors | `NotActive` (forwarding modes), `Disconnected`, `MessageTooLarge`, `SendFailed` when the player's queue is full | The same, and `NoBackend` when the player is not connected to a backend |

## Messages to a server

`ServerMessenger` sends to a backend server rather than to a player:

```rust
let messenger = ctx.server_messenger();
match messenger.send_to_server(&ServerId::new("lobby"), &channel, Bytes::from_static(b"sync")) {
    Ok(copies) => tracing::debug!("sent through {copies} player(s)"),
    Err(MessagingError::NoCarrier) => tracing::debug!("nobody on lobby to carry it"),
    Err(e) => tracing::warn!("{e}"),
}
```

A plugin message needs a player connection to travel on. The messenger sends one copy through one online player for each distinct backend address of the server (a server with two load-balanced addresses gets two copies, one per address with a player on it) and returns how many it sent. With no player on the server it returns `MessagingError::NoCarrier`: the message is not queued for a later player, unlike BungeeCord's `ServerInfo#sendData`. A payload over 32767 bytes returns `MessagingError::TooLarge`.

## Client state

The proxy keeps what the client told it about itself:

| Method | Returns | Description |
|--------|---------|-------------|
| `client_brand()` | `Option<String>` | The client's brand (`vanilla`, `fabric`, `forge`...), at most 128 characters |
| `settings()` | `Option<ClientSettings>` | The last Client Information (settings) packet |
| `known_channels()` | `Vec<String>` | Channels the client registered and did not unregister, in order, at most 1024 names of at most 128 bytes |
| `virtual_host()` | `Option<String>` | The domain the client connected to, from the handshake, in every proxy mode |
| `ping()` | `Option<Duration>` | The last keepalive round trip |

`ClientSettings` has `locale`, `view_distance`, `chat_mode` (`ChatMode::Enabled`, `CommandsOnly`, `Hidden`), `chat_colors`, `skin_parts` (`SkinParts`, with `cape()`, `jacket()`, `left_sleeve()`, `right_sleeve()`, `left_pants()`, `right_pants()`, `hat()` and `bits()`), `main_hand` (`MainHand::Left`, `Right`), `text_filtering`, `allow_listing` and `particle_status` (`ParticleStatus::All`, `Decreased`, `Minimal`). Fields a version does not send keep their default: `text_filtering` before 1.17, `allow_listing` before 1.18, `particle_status` before 1.21.2, `main_hand` before 1.9.

The ping is measured by the proxy from the keepalives the backend sends and the client answers (matched by id), and from the proxy's own keepalives in limbo. It is `None` until the first answer.

The proxy reads the brand, the settings and the channels in every phase it sees: the configuration phase of the login, play, the configuration phase of a switch, and limbo, including the configuration phase a limbo gate runs.

### Replay on a switch

A client sends its settings, its brand and its channels once, when it joins its first server. The proxy sends them again to every server a switch reaches, so the new server sees the right locale, view distance and skin layers, and the mods' channels:

- **1.20.2 and later:** in the new server's configuration phase, right after the proxy's Login Acknowledged: the brand, the Client Information, then `minecraft:register` with the client's channels and the proxy's.
- **Before 1.20.2:** right after the new server's `JoinGame`: the Client Information, the brand, then the channels.

This also covers a switch out of limbo, and the first server after a limbo gate.

### Events

These are informational and [queued](./events#delivery): the proxy posts them and does not wait. They may arrive after the player left.

| Event | Fields | Posted when |
|-------|--------|-------------|
| `PlayerClientBrandEvent` | `player`, `brand` | The client sends a brand that differs from the last one |
| `PlayerSettingsChangedEvent` | `player`, `settings` | The client sends settings that differ from the last ones |
| `PlayerChannelRegisterEvent` | `player`, `channels`, `direction` | The client (`PacketDirection::Serverbound`) or the backend (`Clientbound`) registers channels. Unregistrations post nothing |

## The BungeeCord channel

Bukkit, Spigot and Paper plugins ask the proxy for things on the `BungeeCord` channel (`bungeecord:main` since 1.13): move a player, list the servers, forward a message to another server. Infrarust answers them when both switches are on:

```toml
# infrarust.toml
[plugin_messaging]
bungeecord = true
```

```toml
# servers/lobby.toml
bungeecord_channel = true
```

`bungeecord` is off by default, and so is `bungeecord_channel` on every server. When either is off, a BungeeCord message from that server is forwarded to the client as before, and a vanilla client ignores it. Enable it only for servers whose plugins you trust: a server with the channel can move, list and message players. `bungeecord_channel` needs `offline` or `client_only`, since the proxy must read the backend's packets.

A plugin that registered `ChannelId::bungeecord()` gets the message first and can take it over with `Handled`.

### Network scoping

Every request is scoped to the requesting server's `network`, with the rule server switches use: the requesting server sees itself, the servers of its network and the servers that have no network. A server of another network, and its players, do not exist for it: they are left out of `ALL`, and a request naming them gets no response and changes nothing.

### Subchannels

Requests and responses are Java `DataOutput` streams: `UTF` is `writeUTF` (a big-endian `u16` byte count, then the text in modified UTF-8, where NUL is `C0 80` and a character above U+FFFF is a surrogate pair of 3-byte sequences), `int` a big-endian `i32`, `short` a big-endian `u16`. Every response starts with the subchannel name and goes back on the channel the server's version uses (`BungeeCord` below 1.13, `bungeecord:main` since). A request about a player or server that does not exist or is outside the network, or a subchannel that is not allowed, gets no response.

| Subchannel | Request after the name | Response after the name | Permission (default) |
|------------|------------------------|-------------------------|----------------------|
| `Connect` | `UTF` server | none, the player is switched | `connect` (on) |
| `ConnectOther` | `UTF` player, `UTF` server | none, that player is switched | `connect_other` (off) |
| `IP` | | `UTF` IP, `int` port of the player | `ip` (on) |
| `IPOther` | `UTF` player | `UTF` player, `UTF` IP, `int` port | `ip_other` (on) |
| `PlayerCount` | `UTF` server or `ALL` | `UTF` server or `ALL`, `int` count | `player_count` (on) |
| `PlayerList` | `UTF` server or `ALL` | `UTF` server or `ALL`, `UTF` names separated by `", "` | `player_list` (on) |
| `GetServers` | | `UTF` server names separated by `", "` | `get_servers` (on) |
| `GetServer` | | `UTF` the requesting server | `get_server` (on) |
| `GetPlayerServer` | `UTF` player | `UTF` player, `UTF` server | `get_player_server` (on) |
| `UUID` | | `UTF` UUID without dashes | `uuid` (on) |
| `UUIDOther` | `UTF` player | `UTF` player, `UTF` UUID without dashes | `uuid_other` (on) |
| `ServerIP` | `UTF` server | `UTF` server, `UTF` host, `short` port of its first address | `server_ip` (on) |
| `Message` | `UTF` player or `ALL`, `UTF` legacy text (`§` codes) | none, the text is sent in chat | `message` (off) |
| `MessageRaw` | `UTF` player or `ALL`, `UTF` component JSON | none | `message_raw` (off) |
| `KickPlayer` | `UTF` player, `UTF` legacy text | none, the player is disconnected | `kick_player` (off) |
| `KickPlayerRaw` | `UTF` player, `UTF` component JSON | none | `kick_player_raw` (off) |
| `Forward` | `UTF` server, `ALL` or `ONLINE`, `UTF` channel, `short` length, bytes | none, the target servers receive `UTF` channel, `short` length, bytes | `forward` (on) |
| `ForwardToPlayer` | `UTF` player, `UTF` channel, `short` length, bytes | none, the player's server receives `UTF` channel, `short` length, bytes | `forward_to_player` (on) |

Player names match without case. `Connect` and `ConnectOther` go through the [connection events](./events#server-connections) like any switch: `ServerPreConnectEvent` fires with the cause `PluginMessage` (`plugin_message`), and a listener can redirect or deny it. `Forward` to `ALL` or `ONLINE` reaches every server of the network except the requesting one, through one player per backend address as [`ServerMessenger`](#messages-to-a-server) does; a server with no player gets nothing, since nothing is queued (`ALL` and `ONLINE` are the same). `Message` and `KickPlayer` read `§` color codes.

The permissions live in `[plugin_messaging.bungeecord_permissions]`, see [the global configuration](../../configuration/global#plugin-messaging).

The layouts follow BungeeCord's [`DownstreamBridge`](https://github.com/SpigotMC/BungeeCord/blob/master/proxy/src/main/java/net/md_5/bungee/connection/DownstreamBridge.java) and Velocity's [`BungeeCordMessageResponder`](https://github.com/PaperMC/Velocity/blob/dev/3.0.0/proxy/src/main/java/com/velocitypowered/proxy/connection/backend/BungeeCordMessageResponder.java), and the [Paper plugin messaging guide](https://docs.papermc.io/paper/dev/plugin-messaging/). Where they differ:

- `ServerIP` answers a `short` port, as both proxies write it; the Paper guide reads an `int` and spells it `ServerIp`.
- `GetPlayerServer` and `PlayerCount` for an unknown player or server answer nothing, as Velocity does; BungeeCord answers an empty server name, or only the subchannel name.
- `Forward` never queues, where BungeeCord keeps a message for `ALL` or a named server until a player joins it.
