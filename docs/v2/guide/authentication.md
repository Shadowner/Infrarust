---
title: Authentication and Identity Forwarding
description: How proxy modes decide where Minecraft authentication happens, where encryption sits, and how a player's real identity reaches the backend.
outline: [2, 3]
---

# Authentication and Identity Forwarding

Minecraft's online mode ties a player's account to a UUID verified against Mojang's session servers. When you put a proxy in front of a backend, you have to decide who runs that verification: the client talks to the proxy, the proxy talks to the backend, and only one of them should authenticate. This page explains how Infrarust's proxy modes answer that question, where encryption is enabled, and how the verified identity is passed down to the backend so it trusts a connection it never authenticated itself.

If you only want to know which mode to pick, the [proxy modes guide](./proxy-modes) has the decision table. This page is about the authentication mechanics behind those modes.

## Where authentication happens

The `proxy_mode` on each server config (the `ProxyMode` enum, serialized snake_case) decides who authenticates. The relevant modes split along the forwarding/intercepted line described in [how it works](./how-it-works).

In a forwarding mode (`passthrough`, `zero_copy`, `server_only`), the proxy never reads the login packets. It copies raw bytes after the handshake, so it cannot speak the encryption handshake on the client's behalf. The backend has to run with `online-mode=true` and authenticate the player directly, exactly as if the proxy were a dumb TCP router. `server_only` behaves like `passthrough` at the byte level; it exists as a config signal that the backend owns authentication.

In `offline` mode, no one authenticates. The proxy parses packets (it is an intercepted mode) but skips the encryption handshake entirely and accepts whatever username the client sends. The backend runs with `online-mode=false`. UUIDs are generated from the username the same way vanilla does, with `UUID.nameUUIDFromBytes("OfflinePlayer:" + name)`, so the offline UUID for a given name is stable across restarts.

In `client_only` mode, the proxy authenticates the player against Mojang and the backend trusts the result. This is the interesting case, so it gets its own section below.

## How client_only authenticates against Mojang

When a server uses `client_only`, the intercepted handler terminates the Minecraft protocol on the proxy and runs the standard Mojang login flow before connecting to the backend. The flow lives in `MojangAuth` (`crates/infrarust-core/src/auth/mojang.rs`) and runs once per login:

1. The proxy generates a 4-byte verify token and sends an `EncryptionRequest` carrying its RSA public key (a fresh RSA-1024 key generated at startup and shared across connections via an `Arc`).
2. The client replies with an `EncryptionResponse` holding the shared secret and the verify token, both encrypted with the proxy's public key.
3. The proxy decrypts both with its private key, checks that the shared secret is 16 bytes, and confirms the returned verify token matches the one it sent.
4. It computes the Minecraft server hash (a non-standard SHA-1 over the server id, shared secret, and public key, rendered as a signed `BigInt` in hex) and calls `https://sessionserver.mojang.com/session/minecraft/hasJoined` with the username and that hash.
5. On success Mojang returns a `GameProfile` (the verified UUID, the canonical username, and the texture properties), and the proxy enables AES encryption on the client side using the shared secret.

After this, the proxy holds a verified identity that the backend never saw. The backend must run with `online-mode=false`, because the player's connection to it does not carry a Mojang handshake. That is where identity forwarding comes in: without it, the backend would have no trustworthy UUID and no skin, and any client could claim any username.

## Where encryption sits

Encryption is per-side, and the proxy mode decides which sides are encrypted.

| Mode | Client to proxy | Proxy to backend |
|------|-----------------|------------------|
| `passthrough` / `zero_copy` / `server_only` | end to end (proxy relays the encrypted bytes; the backend negotiated it) | same channel, relayed |
| `offline` | plaintext | plaintext |
| `client_only` | encrypted (proxy negotiates with the client) | plaintext |

In a forwarding mode there is one encryption handshake and the proxy is not a party to it: the client and backend negotiate directly, and the proxy copies the resulting ciphertext through without keys. In `client_only` the proxy terminates encryption with the client but speaks plaintext to the backend, which is fine on a trusted private network.

## Getting identity to the backend

Forwarding solves the problem the intercepted modes create: the backend runs `online-mode=false` and so will trust whatever the proxy tells it, which means the proxy has to tell it the real, verified identity, in a way the backend recognizes and (for the secure schemes) can verify came from the proxy. Infrarust supports the same forwarding schemes the rest of the Minecraft proxy world uses.

Forwarding is configured in the `[forwarding]` table. It can be set globally in `infrarust.toml` (the `ProxyConfig` struct) and applies to all servers by default. The `mode` field of `ForwardingConfig` is one of:

- `none` (the default): nothing is injected. Use this with forwarding proxy modes where the backend authenticates on its own.
- `bungeecord` (alias `legacy`): the classic BungeeCord scheme.
- `bungee_guard`: BungeeCord's wire format plus a shared-secret token.
- `velocity` (alias `modern`): Velocity's modern forwarding with HMAC-signed payloads.

The data being forwarded is always the same set of fields: the player's real IP, their UUID, username, profile properties (skin and cape textures with their signatures), and, where the scheme supports it, the signed chat session. What differs is how that payload is encoded and whether it is authenticated.

### bungeecord

BungeeCord forwarding appends the identity to the handshake's `server_address` field, separated by null bytes: the original address, then the real IP, then the UUID as 32 hex characters with no dashes, then a JSON array of profile properties. The backend (running with BungeeCord-style IP forwarding enabled) splits the field back apart. There is no shared secret, so any host that can reach the backend can forge this. Only use it on a network where the backend port is not reachable from untrusted clients.

### bungee_guard

BungeeGuard uses the same null-separated handshake encoding as BungeeCord but adds one more profile property named `bungeeguard-token` whose value is a shared secret. The backend's BungeeGuard plugin rejects any handshake that does not carry the matching token, which closes the "anyone can forge a handshake" hole in plain BungeeCord. The token comes from the configured secret.

### velocity

Velocity's modern forwarding does not touch the handshake. Instead, during login the backend sends a login plugin request on the `velocity:player_info` channel, and Infrarust answers it with a payload signed by HMAC-SHA256 using the shared secret. The payload carries a forwarding-format version (Infrarust supports up to `0x04` and negotiates down to what the backend asked for), the real IP, UUID, username, properties, and, for protocol 1.19 and later when the data is present, the signed chat session. Because the payload is signed, the backend can verify it actually came from the proxy and was not tampered with. This is the scheme to use with Paper/Velocity-style backends.

### The shared secret

`bungee_guard` and `velocity` both need a shared secret that the proxy and every backend agree on. Infrarust reads it from the file named by `secret_file` in the `[forwarding]` table, which defaults to `forwarding.secret`. If the file does not exist, the proxy generates a 32-character alphanumeric secret on first start, writes it with `0600` permissions on Unix, and logs where it put it. You then copy that file's contents to each backend. For a Paper server, that is `proxies.velocity.secret` in `config/paper-global.yml`.

::: warning
The forwarding secret is what stops an attacker who can reach your backend port from impersonating any player. Keep the backend unreachable from the public internet, use `bungee_guard` or `velocity` rather than plain `bungeecord`, and treat `forwarding.secret` like a password. An empty secret file is rejected at startup rather than silently accepted.
:::

## Putting it together

A typical online-mode network runs `client_only` so the proxy authenticates once, with `velocity` forwarding so each backend learns the verified identity over a signed channel:

```toml
# infrarust.toml
[forwarding]
mode = "velocity"
secret_file = "forwarding.secret"
```

```toml
# servers/lobby.toml
name = "lobby"
network = "main"
domains = ["play.mc.example.com"]
addresses = ["10.0.1.10:25565"]
proxy_mode = "client_only"
```

The lobby backend runs with `online-mode=false` and Velocity modern forwarding pointed at the same secret. The proxy verifies the player against Mojang, opens the backend connection, and answers the backend's `velocity:player_info` request with the signed profile. The player keeps their real UUID and skin, and the backend never had to talk to Mojang.

If instead you want each backend to authenticate on its own and the proxy to stay out of the way, use a forwarding proxy mode (`passthrough` or `server_only`), set `mode = "none"`, and run the backend with `online-mode=true`. No identity forwarding is needed because the backend does its own Mojang handshake.

## Next steps

- [Proxy modes](./proxy-modes) for the full forwarding versus intercepted comparison
- [How it works](./how-it-works) for where authentication sits in the connection pipeline
- [Forwarding configuration](../configuration/proxy-forwarding) for every `[forwarding]` option
- [Client-only configuration](../configuration/proxy-modes/client-only) for Mojang auth setup details
