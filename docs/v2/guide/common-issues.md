---
title: Common Issues
description: Troubleshooting guide for frequent Infrarust problems including offline servers, player kicks, proxy protocol failures, and config reload.
---

# Common Issues

## Server Shows Offline in the Server List

When a player pings a server through Infrarust and the backend is unreachable, the server appears offline in the Minecraft server list. Several things can cause this.

### The backend server is actually down

Infrarust tries to connect to every address listed in the server config. If none respond within the connect timeout (default: 5 seconds), the server is treated as offline.

Check that your backend is running and reachable from the machine running Infrarust:

```bash
# Test from the proxy host
nc -zv 192.168.1.10 25565
```

If the backend is slow to respond, increase the connect timeout in `infrarust.toml`:

```toml
connect_timeout = "10s"
```

Or override it per server in the server config file:

```toml
[timeouts]
connect = "10s"
```

### The domain doesn't match any server config

Infrarust routes ping requests by matching the hostname the client used against the `domains` field in each server config. If nothing matches, the proxy returns its default MOTD or drops the connection, depending on `unknown_domain_behavior`.

Verify the domain in your server config matches what players type in their Minecraft client:

```toml
domains = ["survival.mc.example.com"]
addresses = ["192.168.1.10:25565"]
```

Wildcard domains work too: `"*.mc.example.com"` matches any subdomain.

::: tip
The domain comparison is exact (case-insensitive). If players connect with `Survival.MC.Example.com`, the match still works. But `survival.mc.example.com` won't match a config that only lists `mc.example.com`.
:::

### Status cache is serving stale results

Infrarust caches ping responses for 5 seconds by default. If a backend just came online, players might still see the old "offline" status until the cache expires.

You can lower the TTL in `infrarust.toml`:

```toml
[status_cache]
ttl = "2s"
```

### Custom MOTD for offline state

You can configure a per-server MOTD that displays when the backend is down, instead of showing the server as unreachable:

```toml
[motd.offline]
text = "Server is under maintenance"
version_name = "Maintenance"
max_players = 0
```

## Players Kicked on Connect

Players see a disconnect message right after joining. The cause depends on what the message says.

### "Server is currently unreachable. Please try again later."

This is the default disconnect message when Infrarust cannot reach any backend address. The proxy tried every address in the server's `addresses` list and all failed.

Check that the backend is running and the address/port are correct. You can customize this message per server:

```toml
disconnect_message = "The survival server is restarting. Try again in a few minutes."
```

### "Backend refused connection"

The proxy connected to the backend, but the backend rejected the login. Common causes:

- The backend has `online-mode=true` and the proxy is using `client_only` or `offline` mode. The backend must run with `online-mode=false` when Infrarust handles authentication.
- The backend has a whitelist enabled and the player isn't on it.
- A backend plugin (like AuthMe or a firewall plugin) rejected the connection.

For `client_only` mode, set `online-mode=false` in the backend's `server.properties`:

```properties
online-mode=false
```

### "No limbo handlers configured"

The server config references limbo handlers, but none could be resolved. This happens when `limbo_handlers` lists plugin IDs that aren't loaded:

```toml
limbo_handlers = ["my-queue-plugin"]
```

Make sure the referenced plugins are installed and their IDs match exactly.

### Connection timeout with no message

If the player's client hangs and then shows "Timed out", the backend is accepting the TCP connection but not responding to the Minecraft protocol. This often means the backend port is open but the Minecraft server process is still starting up.

The default connect timeout is 5 seconds. The read/write timeouts default to 30 seconds.

### Protocol version mismatch

Infrarust supports Minecraft protocol versions from beta through current releases. If a player uses a client version that the backend doesn't support, the backend itself will kick them. Infrarust passes through whatever disconnect reason the backend sends.

## Proxy Protocol Not Working

Infrarust supports HAProxy PROXY protocol v1 and v2 for preserving the client's real IP address. There are two separate settings: one for receiving proxy protocol from an upstream proxy, and one for sending it to backends.

### Receiving proxy protocol (upstream → Infrarust)

Enable this in `infrarust.toml` when Infrarust sits behind a load balancer or another proxy that sends PROXY protocol headers:

```toml
receive_proxy_protocol = true
```

::: danger
When `receive_proxy_protocol` is enabled, Infrarust expects every incoming connection to start with a PROXY protocol header. Connections without the header will fail immediately with "data does not start with a proxy protocol signature". Do not enable this unless your upstream actually sends proxy protocol.
:::

If you see "connection closed before proxy protocol header" in the logs, the upstream is closing the connection before sending the header. Check your load balancer configuration.

### Sending proxy protocol (Infrarust → backend)

Enable this per server to forward the client's real IP to the backend:

```toml
send_proxy_protocol = true
addresses = ["192.168.1.10:25565"]
```

The backend must be configured to accept proxy protocol. For common setups:

- **BungeeCord/Velocity**: These are Minecraft proxies themselves and don't typically accept PROXY protocol on their Minecraft port. Use their own IP forwarding mechanisms instead.
- **Paper/Spigot with HAProxyDetector or similar plugin**: Install a plugin that reads PROXY protocol on the Minecraft port.
- **Vanilla behind another proxy**: Vanilla servers don't support PROXY protocol natively. You need a mod or wrapper.

Infrarust always sends **v2 (binary)** when `send_proxy_protocol` is enabled. It can receive both v1 (text) and v2 (binary).

### Mixed IPv4/IPv6

When the client connects over IPv4 but the backend listens on IPv6 (or vice versa), Infrarust maps IPv4 addresses to IPv6 (`::ffff:1.2.3.4`) in the proxy protocol header. Some backends may not handle mapped addresses correctly. If the backend logs unexpected IPs, check whether it supports IPv4-mapped IPv6 addresses.

## Config Not Reloading

Infrarust watches the server config directory for changes and applies them automatically. You don't need to restart the proxy or run a reload command.

### How hot reload works

The file provider watches the `servers_dir` directory (default: `./servers/`) for `.toml` file changes. When a file is created, modified, or deleted, Infrarust waits 200ms (debounce), then diffs the new state against the known configs. It emits incremental events: added, updated, or removed.

### Changes that hot reload covers

- Adding a new `.toml` file to `servers_dir` registers a new server.
- Editing an existing file updates that server's config (domains, addresses, proxy mode, etc.).
- Deleting a `.toml` file removes the server from routing.

### Changes that require a restart

The `infrarust.toml` global config is **not** hot-reloaded. Changes to these settings require restarting the proxy:

- `bind` (listen address)
- `receive_proxy_protocol`
- `connect_timeout`
- `worker_threads`
- `rate_limit`
- `keepalive`
- `so_reuseport`

### Config parse errors don't break existing servers

If you save a `.toml` file with a syntax error, Infrarust logs a warning and keeps the previous version of that server's config:

```
WARN skipping invalid config: ...
```

Fix the syntax error and save again. The file watcher will pick up the corrected version.

::: warning
The server config uses `deny_unknown_fields`. If you add a field name that Infrarust doesn't recognize, the entire file is rejected. Check your spelling against the [server configuration reference](../configuration/servers.md).
:::

### The servers directory doesn't exist

If `servers_dir` doesn't exist at startup, Infrarust logs a warning and starts with no servers. Creating the directory after startup won't trigger the watcher because the watch was never established. Restart the proxy after creating the directory.

### Docker provider

If you use the Docker provider instead of (or alongside) the file provider, container label changes are detected via Docker's event stream, not the filesystem watcher. See the [Docker guide](./docker.md) for details.
