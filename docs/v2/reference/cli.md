---
title: CLI Reference
description: All commands, flags, and options for the Infrarust binary and bundled tools.
outline: [2, 3]
---

# CLI Reference

## infrarust

The main proxy binary. It loads a TOML configuration file, starts the proxy, and opens an interactive console.

```bash
infrarust [OPTIONS]
```

### Options

| Flag | Short | Default | Description |
|------|-------|---------|-------------|
| `--config <PATH>` | `-c` | `infrarust.toml` | Path to the proxy configuration file |
| `--bind <ADDR>` | `-b` | *(from config)* | Override the bind address (e.g. `0.0.0.0:25577`) |
| `--log-level <LEVEL>` | `-l` | `info` | Log level filter. Overridden by the `RUST_LOG` env var |
| `--version` | `-V` | | Print version |
| `--help` | `-h` | | Print help |

### Examples

Start with the default config file in the current directory:

```bash
infrarust
```

Point to a specific config file and override the bind address:

```bash
infrarust -c /etc/infrarust/proxy.toml --bind 0.0.0.0:25577
```

Enable debug logging:

```bash
infrarust --log-level debug
```

Use the `RUST_LOG` environment variable for fine-grained control (this takes priority over `--log-level`):

```bash
RUST_LOG=infrarust=trace,infrarust_core=debug infrarust
```

### Features

The binary supports an optional compile-time feature:

| Feature | Description |
|---------|-------------|
| `telemetry` | Enables OpenTelemetry tracing export. Configured in the `[telemetry]` section of the config file. |

Build with telemetry support:

```bash
cargo build --release --features telemetry
```

### Interactive Console

Once running, Infrarust drops you into an interactive console. Type `help` to see all commands. The full list is below, grouped by category.

#### Players

| Command | Aliases | Usage | Description |
|---------|---------|-------|-------------|
| `list` | `players`, `who`, `online`, `ls` | `list [server]` | List connected players |
| `find` | | `find <player>` | Find a player by name |
| `kick` | | `kick <player> [reason...]` | Kick a player |
| `kick-ip` | `kickip` | `kick-ip <ip>` | Kick all players from an IP |
| `send` | | `send <player> <server>` | Transfer a player to a server |
| `send-all` | `sendall` | `send-all <server>` | Transfer all players to a server |
| `msg` | `tell`, `whisper` | `msg <player> <message...>` | Send a message to a player |
| `broadcast` | `bc`, `say` | `broadcast <message...>` | Broadcast to all players |

#### Bans

| Command | Aliases | Usage | Description |
|---------|---------|-------|-------------|
| `ban` | | `ban <player> [duration] [reason...]` | Ban a player by username |
| `ban-ip` | `banip` | `ban-ip <ip> [duration] [reason...]` | Ban an IP address |
| `unban` | `pardon` | `unban <player>` | Unban a player |
| `unban-ip` | `unbanip`, `pardonip` | `unban-ip <ip>` | Unban an IP address |
| `banlist` | `bans` | `banlist` | List all active bans |
| `baninfo` | | `baninfo <player\|ip\|uuid>` | Show ban details |

#### Servers

| Command | Aliases | Usage | Description |
|---------|---------|-------|-------------|
| `servers` | `backends` | `servers` | List all servers |
| `server` | | `server <id>` | Show server details |
| `start` | | `start <server_id>` | Start a server |
| `stop-server` | `stopserver` | `stop-server <server_id>` | Stop a server |

#### Configuration

| Command | Aliases | Usage | Description |
|---------|---------|-------|-------------|
| `reload` | | `reload` | Reload configuration |
| `config` | | `config [key]` | Show configuration |

#### Plugins

| Command | Aliases | Usage | Description |
|---------|---------|-------|-------------|
| `plugins` | `pl` | `plugins` | List loaded plugins |
| `plugin` | | `plugin <id>` | Show plugin details |

#### System

| Command | Aliases | Usage | Description |
|---------|---------|-------|-------------|
| `help` | `?` | `help [command]` | Show help |
| `version` | `ver` | `version` | Show version info |
| `status` | `info` | `status` | Proxy overview |
| `stop` | `shutdown`, `exit`, `quit` | `stop` | Shut down the proxy |
| `clear` | `cls` | `clear` | Clear the screen |
| `gc` | | `gc` | Run garbage collection |

---

## registry-extractor

Connects to a vanilla Minecraft server and captures registry data for use in Infrarust's limbo system. Outputs both a binary file (`v<protocol>.bin`) and a debug JSON file.

```bash
registry-extractor [OPTIONS] --server <ADDRESS>
```

### Options

| Flag | Short | Default | Description |
|------|-------|---------|-------------|
| `--server <ADDRESS>` | `-s` | *(required)* | Server address to connect to |
| `--output <DIR>` | `-o` | `data/registry` | Output directory for registry files |
| `--protocol-version <INT>` | `-p` | *(auto-detect)* | Protocol version to use |
| `--username <NAME>` | `-u` | `RegExtractor` | Username for the connection |

### Examples

Extract registry data from a local server:

```bash
registry-extractor --server localhost:25565
```

Target a specific protocol version and output directory:

```bash
registry-extractor -s mc.example.com -p 767 -o ./registries
```

---

## infrarust-stress-test

Stress test tool that floods an Infrarust (or any Minecraft) server with connections. Has three subcommands for different test modes.

```bash
infrarust-stress-test <COMMAND>
```

### Subcommands

#### flood

Sends rapid Server List Ping (SLP) requests.

```bash
infrarust-stress-test flood [OPTIONS]
```

| Flag | Default | Description |
|------|---------|-------------|
| `--host <HOST>` | `127.0.0.1` | Target host |
| `--port <PORT>` | `25565` | Target port |
| `--concurrency <N>` | `200` | Number of concurrent workers |
| `--duration <SECS>` | `300` | Test duration in seconds |

#### malformed

Sends various malformed packets to test protocol handling. Distributes workers evenly across six attack types: huge hostname, bogus VarInt length, random bytes, early close, random-after-handshake, and slowloris.

```bash
infrarust-stress-test malformed [OPTIONS]
```

| Flag | Default | Description |
|------|---------|-------------|
| `--host <HOST>` | `127.0.0.1` | Target host |
| `--port <PORT>` | `25565` | Target port |
| `--concurrency <N>` | `50` | Number of concurrent workers |
| `--duration <SECS>` | `300` | Test duration in seconds |

#### mixed

Runs 70% flood workers and 30% malformed workers simultaneously.

```bash
infrarust-stress-test mixed [OPTIONS]
```

| Flag | Default | Description |
|------|---------|-------------|
| `--host <HOST>` | `127.0.0.1` | Target host |
| `--port <PORT>` | `25565` | Target port |
| `--concurrency <N>` | `200` | Number of concurrent workers |
| `--duration <SECS>` | `300` | Test duration in seconds |

### Example

Run a 60-second SLP flood with 500 concurrent connections:

```bash
infrarust-stress-test flood --host proxy.example.com --port 25577 --concurrency 500 --duration 60
```

The tool prints live stats every 5 seconds (successes, errors, connection failures, average latency) and a final summary with p99 latency and throughput.

---

## Environment Variables

| Variable | Description |
|----------|-------------|
| `RUST_LOG` | Overrides `--log-level`. Accepts [tracing directives](https://docs.rs/tracing-subscriber/latest/tracing_subscriber/filter/struct.EnvFilter.html) like `infrarust=debug,infrarust_core=trace`. |
