---
title: Installing plugins
description: How to configure the plugins directory, register plugins in infrarust.toml, and grant capabilities to WASM plugins.
---

# Installing plugins

Infrarust loads plugins from the directory pointed to by `plugins_dir` (default: `./plugins`). When the binary is built with the `wasm` feature, the WASM loader scans that directory recursively for every `.wasm` file and loads each one. Built-in plugins compile into the binary and register themselves at startup, with no files to copy.

The `wasm` feature is not on by default, so a stock release build does not load `.wasm` files. Build with `cargo build --release --features wasm` to enable WASM plugin support.

## WASM plugins

A WASM plugin is a single `.wasm` file that implements the `infrarust:plugin` component model interface. To install one, copy the file into the plugins directory:

```
plugins/
  my-filter.wasm
  analytics.wasm
  .cache/          ← AOT compilation cache, managed automatically
```

The loader will find both files on the next startup. Subdirectories are also scanned, so you can organise plugins however you like. The `.cache` subdirectory is skipped during the scan.

::: tip
AOT-compiled `.cwasm` artifacts land in `plugins/.cache/`. They speed up subsequent startups by skipping recompilation. The cache is keyed on the WASM binary hash and the wasmtime version, so a wasmtime upgrade or a changed plugin automatically triggers recompilation.
:::

## Configuring a plugin in infrarust.toml

Without any configuration entry, a discovered WASM plugin runs with the [baseline capability set](#capabilities). To grant extra capabilities or to set other options, add a `[plugins.<id>]` table to `infrarust.toml`, where `<id>` matches the plugin's declared identifier:

```toml
[plugins.my-filter]
permissions = ["server-manage", "ban"]
enabled = true
```

The three fields are:

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `path` | string | none | Reserved for future use (native `.so` loader, not yet available). |
| `permissions` | string array | `[]` | Extra capabilities to grant beyond the baseline. Values are kebab-case strings. |
| `enabled` | bool | none | Parsed and stored, but not yet enforced at startup in this version. |

A `[plugins.<id>]` table is optional. You only need one if you want to grant extra capabilities.

## Capabilities

WASM plugins start with a baseline set of capabilities and can request more via `permissions`. Native (built-in) plugins are trusted and receive every capability unconditionally.

The baseline every WASM plugin receives:

- `event-bus`: subscribe to proxy events
- `player-read`: read the player registry
- `player-write`: send messages, titles, and kicks
- `command`: register in-game commands
- `scheduler`: run periodic tasks
- `config-read`: read the proxy configuration

Additional capabilities that require an explicit grant:

| Capability | Description |
|------------|-------------|
| `server-manage` | Start, stop, and query backend servers |
| `ban` | Access the ban service |
| `raw-packet` | Emit raw packets and receive `RawPacketEvent` |
| `codec-filter` | Register codec-level packet filters |
| `limbo` | Provide limbo handlers (hold players in a void world) |
| `filesystem-extended` | Access paths outside the plugin's data directory |
| `network` | Make outbound network connections |

Two more capability strings exist in the model. `transport-filter` is the one value that config parsing refuses outright: it is native-only and listing it in `permissions` puts it on the rejected list. `virtual-backend` parses fine but its proxy-side bridge is not yet implemented, so granting it does nothing today. Treat it as planned.

::: warning
Unknown capability strings and `transport-filter` are skipped rather than granted. Infrarust logs a warning to the tracing output for each one, so check the startup log if a plugin behaves like it lacks a permission you listed.
:::

## Built-in plugins

Built-in plugins are compiled into the binary and activated by Cargo feature flags. There are no files to install.

| Plugin | Feature flag | Activated by |
|--------|-------------|--------------|
| Auth | `plugin-auth` | build flag, on by default |
| Server wake | `plugin-server-wake` | build flag, on by default |
| Admin API and Web UI | always compiled | `[web]` section present in `infrarust.toml` |

The `default-plugins` feature pulls in both `plugin-auth` and `plugin-server-wake`, so a stock build has both. To build without them, disable the default features and pick only what you want:

```bash
cargo build --release --no-default-features --features "plugin-server-wake"
```

The Auth plugin's limbo handler and the Server wake plugin's hold logic are only active when those features are compiled in. Dropping `plugin-auth` from the build also removes the `/login` and `/register` commands.

## Checking loaded plugins

Any player with Admin permission can run `/ir plugins` in chat to list every plugin the proxy loaded at startup, along with its version and description. For plugin-specific commands, `/ir plugin <id>` shows what commands that plugin registered.

Both subcommands require the Admin permission level. Players without it see no response.

## Data directories

Each plugin gets a data directory at `<plugins_dir>/<plugin_id>/`. For a WASM plugin this directory is the sandbox root: it is preopened as `/`, the plugin can read and write there freely, and reaching any other path requires the `filesystem-extended` capability. Native plugins are trusted and not sandboxed, so they use the same path by convention but can touch the wider filesystem.

Built-in plugins follow the same convention, keyed on the plugin id. The auth plugin (id `auth`) stores its accounts and `config.toml` under `plugins/auth/`, and the server wake plugin (id `server_wake`) reads its `config.toml` from `plugins/server_wake/`.

## Changing plugins_dir

The default directory is `./plugins` (relative to the working directory). To use a different path, set `plugins_dir` in `infrarust.toml`:

```toml
plugins_dir = "/opt/infrarust/plugins"
```

If the directory does not exist, WASM discovery returns an empty list and startup continues without it. A directory that exists but cannot be read (for example a permission error) is a hard failure: discovery returns an error and the proxy refuses to start, so fix the permissions or remove the bad path.
