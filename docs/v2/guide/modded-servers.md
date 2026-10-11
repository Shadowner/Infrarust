---
title: Modded Servers
description: What works with Forge, NeoForge and Fabric servers in each proxy mode, what the proxy does during a modded login, and the limits to plan for.
outline: [2, 3]
---

# Modded Servers

Mod loaders negotiate with the server in three places: a marker appended to the handshake host, login plugin messages exchanged before the player is logged in, and plugin messages in the configuration and play phases. How far Infrarust takes part in that negotiation depends on the proxy mode.

## At a glance

| | Forwarding modes (`passthrough`, `zero_copy`, `server_only`) | Intercepted modes (`client_only`, `offline`) |
|---|---|---|
| Handshake marker (`\0FML\0`, `\0FML2\0`, `\0FML3\0`, `\0FORGE`) | Forwarded as sent | Forwarded as sent |
| Login plugin messages | Untouched, the proxy does not parse them | Relayed between the backend and the client during the initial login |
| Configuration and play plugin messages | Untouched | Relayed; plugins can observe or rewrite registered channels |
| Large packets (registries, tags, recipes) | Untouched | Compressed on the client link with [`compression_threshold`](../configuration/global#client-compression) |
| Server switching inside a network | Not available | Available, with the limits below |

Forwarding modes never parse past the Login Start, so any loader and any version works as long as the backend accepts the client. Pick them when you don't need server switching, limbo or packet-level plugins.

## Handshake markers

Forge and NeoForge clients append a marker to the host in the handshake. Infrarust ignores the marker when it routes the connection and keeps it on the way to the backend:

- With no `domain_rewrite`, the original handshake bytes are forwarded.
- With a `domain_rewrite`, the rewritten host is followed by the client's marker, and by nothing else the client appended.
- With BungeeCord or BungeeGuard forwarding, the marker becomes the `forgeClient` and `extraData` profile properties, the convention Waterfall introduced and that Forge-side BungeeCord forwarding mods read.

See [FML markers](./routing#fml-markers) for the details.

## The modded login in intercepted modes

In `client_only`, and in `offline` with forwarding or a rewritten profile, the proxy runs its own login with the backend. Infrarust keeps the client in the login phase until that backend login has finished:

1. The proxy authenticates the client (in `client_only`) and connects to the initial backend.
2. It answers the backend's `velocity:player_info` request itself.
3. It relays every other login plugin request to the client with its message id unchanged, and relays each answer back. Requests can arrive in bursts and be answered in any order. Login cookie requests are relayed the same way.
4. Once the backend accepts the login, the proxy sends `SetCompression` and `LoginSuccess` to the client.

This covers the Forge 1.13 to 1.20.1 handshake (`fml:loginwrapper`), Fabric login queries, and any mod that negotiates during the login. Forge 1.20.2+ and NeoForge negotiate in the configuration phase, which the proxy relays as it comes.

A mod may take its time to answer a relayed request, for example to ask the player something first: the client gets up to 5 minutes before the proxy ends the login, while a silent backend gets `timeouts.read`. See [login time limits](../configuration/proxy-modes/offline#login-time-limits).

A plain `offline` login without forwarding is relayed untouched: the backend runs the login directly with the client.

### Backend requirements

- `online-mode=false` on the backend. A backend still in online mode is refused at once with a clear error instead of hanging.
- For Velocity forwarding, a forwarding mod for the backend's loader that accepts `velocity:player_info`.
- The same Minecraft version on the client, the proxy's supported range and the backend. Infrarust parses intercepted sessions, so the protocol version must be one it knows.

## Limits

### Switching servers

A login plugin message can only reach the client while it is still logging in. After the first server, the client is past that point, so on a switch the proxy answers the new backend's login plugin requests with "not understood". A backend that requires a login-phase negotiation then refuses the player, who stays on the current server or gets the fallback you configured.

- Forge 1.13 to 1.20.1 negotiates its mod list during the login, so switching between such servers does not work through any proxy without a client-side reset.
- Loaders that negotiate in the configuration phase (Forge 1.20.2+, NeoForge, Fabric on 1.20.2+) go through the configuration phase again on a switch. Whether the client accepts a second negotiation depends on the loader and the mods.
- A plugin can answer a login plugin request on the client's behalf with `LoginPluginRequestEvent` when it knows the answer, see [login plugin messages](../plugins/wasm/messaging#login-plugin-messages).

### Limbo and server wake-up before the first server

When a limbo handler or the server manager holds the player before the first backend, the proxy has to finish the client's login first. The first backend is then joined like a switch, with the same limit on login plugin messages.

### Command suggestions

The proxy adds its own commands to the command tree the backend sends. If that tree uses argument types the proxy cannot parse, which modded servers often register, the tree is forwarded unchanged. Proxy commands still run, but the client does not suggest them.

## Mods that open their own connections

Some mods open extra connections to the server address the player joined. They might share the Minecraft port with their own protocol, or send a Login Start the proxy should not run. A plugin can take those connections over and have the proxy relay them to the right backend with the [connection handoff hooks](../plugins/wasm/connection-handoff): a claim on the first bytes, on the handshake, or on the Login Start.

These connections count as new connections for [rate limiting](../configuration/global#rate-limiting). If a mod opens several at once per player, raise `max_connections` accordingly.

## Troubleshooting

| Symptom | Likely cause |
|---|---|
| Kicked during the login with "the backend is in online mode" in the proxy log | The backend has `online-mode=true` |
| A Forge client is treated as vanilla by the backend | The backend did not get the handshake marker: check `domain_rewrite` and forwarding, see [FML markers](./routing#fml-markers) |
| The session ends on a large packet | `compression_threshold` is `-1`; keep compression on |
| Switching to another modded server fails | The new backend needs a login-phase negotiation, see [Switching servers](#switching-servers) |
