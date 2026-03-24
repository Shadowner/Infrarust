---
title: Error Codes
description: Complete reference for all Infrarust error types, what causes them, and how to fix them
outline: [2, 3]
---

# Error Codes

Infrarust uses typed errors across its crates. When something goes wrong, the error message tells you which category the problem falls into. This page lists every error variant, what triggers it, and what you can do about it.

## Core Errors

These come from `infrarust-core` and represent top-level failures during proxy operation.

### `Transport`

A transport-layer error bubbled up. See [Transport Errors](#transport-errors) for the specific variants.

### `Protocol`

A protocol-layer error bubbled up. See [Protocol Errors](#protocol-errors) for the specific variants.

### `Config`

A configuration error bubbled up. See [Configuration Errors](#configuration-errors) for the specific variants.

### `Rejected`

```
pipeline rejected: <reason>
```

A filter in the processing pipeline rejected the connection. The reason string comes from whichever filter blocked it. Check your filter configuration and plugin logs.

### `UnknownDomain`

```
no server found for domain: <domain>
```

A player connected with a hostname that doesn't match any configured server. Either the player typed the wrong address, or you need to add a `domains` entry in the server config for that hostname.

### `ConnectionClosed`

```
connection closed
```

The connection was closed normally. This is expected during player disconnects and is logged at debug level.

### `Timeout`

```
connection timeout: <details>
```

A connection or operation took too long. Check that the backend server is running and reachable from the proxy host. If the server is slow to respond, consider increasing timeout values.

### `BackendUnreachable`

```
backend unreachable: <details>
```

The proxy cannot reach the backend Minecraft server. Verify the server address in your config, check that the server is running, and confirm there are no firewall rules blocking the connection.

### `Io`

```
io error: <details>
```

A raw I/O error. Common causes include network interruptions, file permission issues, or the OS running out of file descriptors. Connection resets and broken pipes are treated as expected disconnects and logged at debug level.

### `Auth`

```
auth error: <details>
```

Authentication failed. This can mean the auth plugin rejected the player's credentials, or an upstream auth service returned an error.

### `MissingExtension`

```
missing pipeline extension: <name> — check middleware ordering
```

A filter tried to read a pipeline extension that wasn't set by an earlier filter. This usually means your filters are in the wrong order. The filter that produces the extension must run before the one that consumes it.

### `InvalidProviderId`

```
invalid provider id (expected `type@id`): <value>
```

A server provider ID doesn't follow the `type@id` format. Check your server configuration for typos in provider references. Valid examples: `docker@minecraft-lobby`, `process@survival`.

### `DockerConnection`

```
docker connection error: <details>
```

The proxy failed to communicate with the Docker daemon. Verify that Docker is running, the socket path is correct, and the proxy process has permission to access the Docker socket.

### `TelemetryInit`

```
telemetry initialization error: <details>
```

OpenTelemetry setup failed. Check your telemetry configuration (endpoint URL, authentication) and verify the collector is reachable.

### `Other`

A catch-all for errors that don't fit other categories. The message string contains the details.

## Transport Errors

These come from `infrarust-transport` and cover network-level failures.

### `Bind`

```
failed to bind to <addr>: <reason>
```

The proxy can't listen on the configured address and port. Common causes: another process is already using that port, or the address doesn't exist on this host. On Linux, binding to ports below 1024 requires root or the `CAP_NET_BIND_SERVICE` capability.

### `Accept`

```
accept error: <reason>
```

Failed to accept an incoming TCP connection. This is usually transient (the client disconnected before the accept completed). Persistent accept errors may indicate OS resource exhaustion.

### `BackendConnect`

```
failed to connect to backend <addr>: <reason>
```

A specific backend address was unreachable. The proxy will try other addresses if available.

### `AllBackendsFailed`

```
all backends failed for server <server_id>
```

Every address listed for this server failed to connect. Check that at least one backend server is running and reachable.

### `ConnectTimeout`

```
connection to <addr> timed out after <duration>
```

The backend didn't respond within the timeout window. The server may be overloaded, or a firewall may be silently dropping packets.

### `InvalidProxyProtocol`

```
invalid proxy protocol: <details>
```

The incoming connection sent a PROXY protocol header that couldn't be parsed. This happens when proxy protocol is enabled on the Infrarust listener but the upstream sender is misconfigured or not using proxy protocol.

### `ProxyProtocolDecode`

```
proxy protocol decode error: <details>
```

The PROXY protocol header was recognized but contained invalid data. Check the upstream proxy's PROXY protocol version and configuration.

### `SocketConfig`

```
socket configuration error: <details>
```

Failed to set socket options (TCP_NODELAY, buffer sizes, etc.). This typically indicates an OS-level restriction.

### `Forward`

```
forward error: <details>
```

An I/O error during packet forwarding between client and server. Usually means one side dropped the connection.

### `Splice`

```
splice error: <details>
```

The `splice(2)` syscall failed during zero-copy forwarding. Linux only. Falls back to userspace copying.

### `Shutdown`

```
shutdown
```

The transport layer received a shutdown signal. This is expected during graceful proxy shutdown.

## Protocol Errors

These come from `infrarust-protocol` and indicate problems with Minecraft protocol data.

### `Incomplete`

```
incomplete: <context>
```

Not enough bytes to decode a complete packet. This is non-fatal. The proxy waits for more data and retries. You won't normally see this in logs unless debug logging is enabled.

### `Invalid`

```
invalid: <context>
```

The packet data is corrupted or doesn't conform to the Minecraft protocol. The connection is closed. Common causes: a client sending garbage data, protocol version mismatch, or a broken mod.

### `TooLarge`

```
too large: <actual> bytes exceeds maximum of <max>
```

A packet exceeds the size limit. This is a potential attack vector (clients sending oversized packets to exhaust memory). The connection is closed immediately.

### `Io`

An I/O error occurred during protocol read/write. The original `std::io::Error` is preserved. `WouldBlock` and `UnexpectedEof` are treated as non-fatal; everything else closes the connection.

## Configuration Errors

These come from `infrarust-config` and are raised when loading or validating config files.

### `ReadFile`

```
failed to read config file <path>: <reason>
```

The config file couldn't be read from disk. Check that the file exists and the proxy process has read permission.

### `ParseToml`

```
failed to parse TOML in <path>: <reason>
```

The config file contains invalid TOML syntax. The error message from the TOML parser includes the line and column number.

### `InvalidAddress`

```
invalid server address: <value>
```

A server address in the config isn't a valid `host:port` pair. Check the `addresses` field in your server config.

### `NoDomains`

```
server '<id>' uses <mode> mode which requires at least one domain
```

Forwarding proxy modes (anything other than `Passthrough`) need at least one domain so the proxy knows which server to route to. Add a `domains` entry to the server config.

### `NoAddresses`

```
server config <id> has no addresses defined
```

A server config block has no `addresses` field. Every server needs at least one backend address to connect to.

### `DuplicateId`

```
duplicate config id: <id>
```

Two server configs have the same ID. Each server config must have a unique identifier.

### `DirectoryNotFound`

```
config directory not found: <path>
```

The configured directory for server config files doesn't exist. Create the directory or update the `config_path` in your main config.

### `Validation`

```
validation error: <details>
```

A catch-all for config validation failures that don't fit other variants.

## Server Manager Errors

These come from `infrarust-server-manager` and relate to server lifecycle operations (start, stop, wake).

### `ServerNotFound`

```
server <server_id> not found
```

The requested server ID doesn't exist in the server manager's registry. Check the server ID in your API call or configuration.

### `InvalidState`

```
server <server_id> is in state <state>, cannot <action>
```

You tried to perform an action that isn't valid for the server's current state. For example, starting a server that's already running, or stopping one that's already stopped.

### `StartTimeout`

```
server <server_id> failed to start within <duration>
```

The server didn't become ready within the expected time. Check the server's own logs for startup errors. You can increase the timeout in the server manager configuration.

### `Provider`

```
provider error for <server_id>: <message>
```

The server provider (Docker, process, Pterodactyl) returned an error. Check the provider-specific configuration and that the backing service is accessible.

### `Http`

```
HTTP request failed: <details>
```

An HTTP request to an external API (Pterodactyl, etc.) failed. Check network connectivity and API credentials.

### `Process`

```
process error: <details>
```

Failed to spawn or manage a server process. Check file permissions and that the server executable exists at the configured path.

### `ApiResponse`

```
API returned unexpected response: <details>
```

An external API returned a response the proxy didn't expect. This may indicate an API version mismatch.

### `ProcessExited`

```
server <server_id> process exited with code <code>
```

A managed server process terminated. Check the server's logs for the cause. An exit code of `None` usually means the process was killed by a signal.

### `Shutdown`

```
shutdown in progress
```

The server manager is shutting down and can't accept new operations. This is expected during graceful proxy shutdown.

## Plugin API Errors

These come from `infrarust-api` and are used by plugins to report failures.

### Player Errors

Returned by player interaction methods within filters and plugins.

`NotActive` — the player is in passthrough or zero-copy mode, so per-packet operations aren't available. Switch to a proxy mode that intercepts packets.

`Disconnected` — the player already left. No action needed.

`SendFailed` — a packet couldn't be delivered to the player. The connection may have dropped between your check and the send.

`ServerNotFound` — the target server for a switch doesn't exist in the config. Check the server ID.

`SwitchFailed` — a server transfer failed. The error message contains the specific reason.

### Service Errors

Returned when interacting with proxy services from plugin code.

`NotFound` — the requested resource doesn't exist.

`OperationFailed` — the operation couldn't be completed.

`Unavailable` — the service is temporarily down.

### Plugin Lifecycle Errors

`InitFailed` — plugin initialization failed. Check plugin configuration and dependencies.

`Custom` — a plugin-specific error. The message comes from the plugin itself.

## Plugin Loader Errors

These come from `infrarust-core`'s plugin loading system.

### `DirectoryNotAccessible`

```
plugin directory not accessible: <path>
```

The plugin directory can't be read. Check that the path exists and the proxy has read permission.

### `PluginNotFound`

```
plugin not found: <plugin_id>
```

A plugin referenced in the config wasn't found in any plugin directory.

### `InvalidFormat`

```
invalid plugin format at <path>: <reason>
```

A file in the plugin directory isn't a valid plugin. Check that you're using the correct plugin format.

### `LoadFailed`

```
failed to load plugin '<plugin_id>': <reason>
```

A plugin was found but couldn't be loaded. The reason string explains why. Common causes: missing dependencies, incompatible plugin API version.

### `UnloadFailed`

```
failed to unload plugin '<plugin_id>': <reason>
```

A plugin couldn't be cleanly unloaded. Resources may not have been fully released.

### `DuplicateId`

```
duplicate plugin id '<plugin_id>' (found in loader '<first>' and '<second>')
```

Two different plugin sources provide a plugin with the same ID. Rename one of the plugins or remove the duplicate.

## Filter Ordering Errors

### `CyclicDependency`

```
circular dependency detected involving: <filter_names>
```

Your filter `before`/`after` constraints form a cycle. For example, filter A declares `after: B` and filter B declares `after: A`. Remove or adjust the conflicting constraint.

## Codec Filter Errors

These come from `infrarust-api`'s codec filter system.

`TranslationFailed` — packet translation between protocol versions failed. This can happen with unsupported packet types during version bridging.

`MalformedPayload` — the packet payload doesn't match the expected structure for its packet ID.

`UnsupportedVersion` — the protocol version isn't supported by this codec filter.

`Internal` — an unexpected error within the codec filter itself.

## Server Switch Errors

### `DifferentNetworks`

```
servers are in different networks: '<current>' vs '<target>'
```

A player tried to switch between servers that aren't in the same network. Servers must share a `network` value in their config to allow transfers between them.

## Auth Plugin Errors

These come from the `infrarust-plugin-auth` plugin.

### Auth Errors

`Storage` — the auth storage backend returned an error. See storage errors below.

`Hashing` — password hashing failed. This usually indicates a system-level issue with the hashing library.

`Config` — the auth plugin configuration is invalid. Check the plugin config file.

`Io` — a file I/O error during auth operations.

`PasswordValidation` — a password didn't meet the validation requirements. The `reason` field explains which requirement failed.

### Auth Storage Errors

`AccountAlreadyExists` — tried to create an account with a username that's already registered.

`AccountNotFound` — tried to look up or modify an account that doesn't exist.

`Io` — file I/O error in the storage layer.

`Serialization` — failed to serialize or deserialize account data. The storage file may be corrupted.
