---
title: Network & Extra Folders
description: Let a WASM plugin open TCP and UDP sockets, resolve names, make HTTP and HTTPS requests, and read or write host folders, only within an operator allow-list.
outline: [2, 3]
---

# Network & Extra Folders

A WASM plugin has no network and sees one folder, its data directory. Two opt-in capabilities widen that, and each one only reaches what the operator lists in config:

- `network` lets the plugin open outbound TCP connections, send UDP datagrams, resolve names and make HTTP or HTTPS requests to the destinations in `[plugins.<id>.wasm.network] allow`.
- `filesystem-extended` mounts the host folders listed in `[[plugins.<id>.wasm.mounts]]` into the guest, read-only unless a mount says otherwise.

Both fail closed. A capability without its config grants nothing, and config without its capability is ignored. Nothing new is needed in the plugin contract: the guest uses the standard WASI 0.2 imports (`wasi:sockets`, `wasi:http`, `wasi:filesystem`), so Rust's `std::net` and `std::fs` work as they do on any `wasm32-wasip2` program.

## Configuration

```toml
[plugins.libertybans]
permissions = ["network", "filesystem-extended"]

[plugins.libertybans.wasm.network]
allow = [
    "127.0.0.1:5432",      # one address, one port
    "10.0.0.0/8:3306",     # an IPv4 range
    "[::1]:*",             # an IPv6 address, any port
    "db.internal:5432",    # a hostname, resolved by the proxy
    "api.example.com:443",
    "*.example.org:443",   # any subdomain of example.org, HTTP only
    "10.1.2.3:8000-8100",  # a port range
]
dns = true
http = true

[[plugins.libertybans.wasm.mounts]]
host = "/srv/libertybans/shared"
guest = "/shared"
read_only = true
```

### `allow` rules

Each rule is `host:port`. The config is parsed when the proxy loads it, and a malformed rule stops the load with a message naming the rule and what is wrong with it, for example `invalid network rule "*:443": a bare `*` host is not allowed; write 0.0.0.0/0:* or [::/0]:* to allow every address`.

| Host form | Example | Matches |
|-----------|---------|---------|
| IPv4 address | `127.0.0.1` | That address |
| IPv6 address | `[::1]` | That address; brackets are required |
| IPv4 range | `10.0.0.0/8` | Every address in the range. Bits past the prefix must be zero |
| IPv6 range | `[fd00::/8]` | Every address in the range |
| Hostname | `db.internal` | The name itself (HTTP), and the addresses it resolves to (sockets) |
| Wildcard | `*.example.org` | Any name ending in `.example.org`, at any depth, for HTTP only. It does not match `example.org` itself, nor `evilexample.org` |

| Port form | Example | Matches |
|-----------|---------|---------|
| Number | `5432` | That port |
| Range | `8000-8100` | Both ends included |
| `*` | `*` | Every port |

A bare `*` host is refused. Allowing every destination has to be written out: `0.0.0.0/0:*` for IPv4, `[::/0]:*` for IPv6. An IPv4 address that arrives in IPv6-mapped form (`::ffff:10.1.2.3`) is matched as the IPv4 address, so `[::/0]` does not open IPv4 by the back door, and IPv4-mapped addresses cannot be written as rules.

### `dns` and `http`

| Key | Default | Effect |
|-----|---------|--------|
| `dns` | `true` when `allow` has an exact hostname rule, otherwise `false` | Lets the guest resolve names itself (`wasi:sockets/ip-name-lookup`, which is what `TcpStream::connect("db.internal:5432")` uses) |
| `http` | `true` | Lets the guest send requests through `wasi:http/outgoing-handler`. Every request is still checked against `allow` |

### `mounts`

| Key | Default | Meaning |
|-----|---------|---------|
| `host` | required | A directory on the proxy host. It is canonicalised when the plugin loads; a relative path is resolved from the proxy's working directory |
| `guest` | required | Where the plugin sees it. Must be absolute, must not be `/` (the data directory), and must not repeat or contain another mount's path |
| `read_only` | `true` | `false` lets the plugin create, change and delete files in the folder |

The guest-path rules are checked when the config loads. A host directory that does not exist fails the load of that plugin only, with an error naming the plugin and the path; the other plugins start normally.

## What the proxy enforces

### Without the capability

If `network` is missing (not granted, or taken away with `deny`), the plugin gets no socket, no name lookup and no HTTP, even when `[plugins.<id>.wasm.network]` is present. One warning at load says the table is ignored. The same goes for `mounts` without `filesystem-extended`: nothing is mounted and one warning is logged.

`network` with an empty or missing `allow` list refuses everything, and one warning at load says so.

### Sockets

Every TCP connect, UDP connect and UDP datagram is checked against the rules before the host touches the network. A destination is allowed when its address and port match an IP or range rule, or when the address is one that an allowed hostname currently resolves to and the port matches that rule.

Hostname rules are resolved by the proxy, with the system resolver, when the plugin's instance is built, and again when a connection misses, at most once per name every 30 seconds. A failed lookup keeps the previous addresses. The lookup cache belongs to the plugin and survives instance restarts.

A refused socket call fails in the guest with an access-denied error (`std::io::ErrorKind::PermissionDenied` in Rust), and the destination never sees a packet.

Listening is refused by default. A TCP bind, which every listening socket needs, is allowed only when a rule names that exact address and port, such as `0.0.0.0:25600` or `127.0.0.1:9000-9100`. Ranges and hostnames never allow a bind, so `0.0.0.0/0:*` still does not let the plugin listen. A UDP socket may bind to port `0` (an OS-chosen port), which is what sending a datagram requires; binding a fixed UDP port needs an exact rule like TCP.

### Name lookups

The guest can resolve names only when `network` is granted, `allow` is not empty and `dns` is on. A lookup does not check `allow`: the result only helps if the address it returns is allowed.

::: warning DNS can carry data out
A lookup is itself a message to the outside. A plugin can put data in the name it asks for (`secret-data.attacker.example`) and the query reaches whatever resolver the proxy host uses, even though the connection that would follow is refused. Leave `dns` off unless the plugin has to connect by name through a socket. HTTP to an allowed hostname does not need it, because the proxy resolves the request's authority itself.
:::

### HTTP and HTTPS

A request is sent only when `network` is granted, `http` is on, and its authority (the host and the explicit port, or 80 for `http` and 443 for `https`) matches a rule:

- A hostname or wildcard rule matches the authority's name. The proxy resolves the name and connects to what it returns.
- An IP or range rule matches an IP-literal authority, or, when `dns` is on, the addresses an unlisted name resolves to. Only the matching addresses are used. With `dns` off an unlisted name is refused without being resolved.

A refused request fails with the wasi-http error `HTTP-request-denied`, before any connection is made. The `Host` header always comes from the authority; the guest cannot set it.

HTTPS certificates are checked against the proxy host's trust store, the same one the proxy's own HTTPS clients use. To trust a private CA, add it to the system store, or point `SSL_CERT_FILE` (a PEM bundle) or `SSL_CERT_DIR` at your certificates when starting the proxy; when either is set, only those certificates are trusted. The connect, first-byte and between-bytes timeouts a request asks for are capped at the plugin's `host_call_timeout`.

### Denials in the log

Each refused socket call or HTTP request logs a warning with the plugin id, the kind of call (`tcp-connect`, `udp-send`, `tcp-bind`, `http`, ...), the destination and the reason (`network allow-list`, `missing capability `network``, or `http = false`). At most five such lines are written per plugin per minute; the next one carries a `suppressed` count.

### Codec filters

Codec filter instances never get network access, whatever the plugin is granted. Every socket and HTTP import traps inside a filter, as described in [Codec Filters](./codec-filters#what-a-filter-can-call).

### Extra folders

A mount is a second WASI preopen next to the data directory, which stays at `/`. The guest resolves `/shared/file.txt` to the mount and `/note.txt` or `note.txt` to the data directory.

A read-only mount grants read permission on the folder and its files, so writes, creations and deletions fail. `..` cannot leave a mount or the data directory: the host refuses a path that climbs above the preopened folder, even one that would come back inside it, and a symbolic link that points outside the folder is refused the same way.

### Restarts

A fresh instance built after a fault gets the same rules and the same mounts as the first one. The instance used to read a plugin's metadata at discovery gets neither.

## Example

A plugin that talks to a TCP service, calls a web API and reads a shared file. It uses `std::net` and `std::fs` from the standard library and the `wasip2` crate for HTTP (`wasi` 0.14 re-exports the same bindings as `wasi::http`).

```toml
[package]
name = "reporter"
version = "0.1.0"
edition = "2024"

[lib]
crate-type = ["cdylib"]

[dependencies]
infrarust-plugin-sdk = { git = "https://github.com/Shadowner/Infrarust.git" }
wasip2 = "1.0"
```

```rust
use std::io::{Read, Write};
use std::net::TcpStream;

use infrarust_plugin_sdk::prelude::*;
use wasip2::http::outgoing_handler;
use wasip2::http::types::{Fields, OutgoingRequest, Scheme};
use wasip2::io::streams::StreamError;

#[derive(Default)]
struct Reporter;

#[plugin(id = "reporter", name = "Reporter")]
impl Plugin for Reporter {
    fn on_enable(&self, ctx: &Context) -> Result<(), PluginError> {
        ctx.command("report")
            .description("Pings the stats service, fetches the MOTD and reads the shared list")
            .handler(|_| {
                match ping_stats() {
                    Ok(reply) => info!("stats replied {reply}"),
                    Err(error) => warn!("stats unreachable: {error}"),
                }
                match fetch_motd() {
                    Ok((status, body)) => info!("motd {status}: {body}"),
                    Err(error) => warn!("motd request failed: {error}"),
                }
                match std::fs::read_to_string("/shared/banlist.txt") {
                    Ok(list) => info!("{} shared entries", list.lines().count()),
                    Err(error) => warn!("shared list unreadable: {error}"),
                }
            })
            .register()?;
        Ok(())
    }
}

// A plain TCP exchange. Refused destinations fail with PermissionDenied.
fn ping_stats() -> std::io::Result<String> {
    let mut stream = TcpStream::connect("10.0.0.20:7000")?;
    stream.write_all(b"PING\n")?;
    let mut reply = String::new();
    stream.read_to_string(&mut reply)?;
    Ok(reply)
}

// An HTTPS GET through wasi:http. The proxy resolves the name, checks it
// against `allow`, and verifies the certificate.
fn fetch_motd() -> Result<(u16, String), String> {
    let request = OutgoingRequest::new(Fields::new());
    request.set_scheme(Some(&Scheme::Https)).map_err(|()| "invalid scheme")?;
    request.set_authority(Some("api.example.com")).map_err(|()| "invalid authority")?;
    request.set_path_with_query(Some("/motd")).map_err(|()| "invalid path")?;

    // A refused request comes back here as ErrorCode::HttpRequestDenied.
    let pending = outgoing_handler::handle(request, None).map_err(|code| format!("{code:?}"))?;
    pending.subscribe().block();
    let response = pending
        .get()
        .ok_or("response not ready")?
        .map_err(|()| "response already taken")?
        .map_err(|code| format!("{code:?}"))?;

    let status = response.status();
    let body = response.consume().map_err(|()| "body already taken")?;
    let stream = body.stream().map_err(|()| "body stream already taken")?;
    let mut bytes = Vec::new();
    loop {
        match stream.blocking_read(16 * 1024) {
            Ok(chunk) => bytes.extend(chunk),
            Err(StreamError::Closed) => break,
            Err(StreamError::LastOperationFailed(error)) => return Err(error.to_debug_string()),
        }
    }
    drop(stream);
    Ok((status, String::from_utf8_lossy(&bytes).into_owned()))
}
```

```toml
# infrarust.toml
[plugins.reporter]
permissions = ["network", "filesystem-extended"]

[plugins.reporter.wasm.network]
allow = ["10.0.0.20:7000", "api.example.com:443"]

[[plugins.reporter.wasm.mounts]]
host = "/srv/infrarust/shared"
guest = "/shared"
```

`dns` stays off here: the socket connects to an IP rule and the HTTP request goes to a hostname rule, which the proxy resolves. The calls block the plugin's own call queue while they wait, not the proxy; each guest call is still bounded by `max_call_duration`, and each HTTP timeout by `host_call_timeout`.

## See also

- [Capabilities & Sandbox](./capabilities): the capability model and the rest of the sandbox.
- [Deploying](./deploying): where the plugin tables live.
- [Configuration reference](../../reference/config-schema#plugins-id-wasm-network): every key with its type and default.
- [Codec Filters](./codec-filters#what-a-filter-can-call): what a filter instance can import.
