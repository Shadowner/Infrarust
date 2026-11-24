---
title: CLI Reference
description: All commands, flags, and options for the Infrarust binary and bundled tools.
outline: [2, 3]
---

# CLI Reference

## infrarust

The main proxy binary. It loads a TOML configuration file, starts the proxy, and opens an interactive console.

```bash
infrarust [OPTIONS] [COMMAND]
```

Without a command it starts the proxy. The commands [`migrate`](#infrarust-migrate), [`check-config`](#infrarust-check-config) and [`update`](#infrarust-update) run one task and exit. The options below go before the command: `infrarust -c /etc/infrarust/proxy.toml update check`, not `infrarust update check -c ...`.

### Options

| Flag | Short | Default | Description |
|------|-------|---------|-------------|
| `--config <PATH>` | `-c` | `infrarust.toml` | Path to the proxy configuration file |
| `--bind <ADDR>` | `-b` | *(from config)* | Override the bind address (e.g. `0.0.0.0:25577`) |
| `--log-level <LEVEL>` | `-l` | `info` | Log level filter. Overridden by the `RUST_LOG` env var |
| `--plugins-dir <PATH>` | | *(from config)* | Override the plugins directory path |
| `--servers-dir <PATH>` | | *(from config)* | Override the servers directory path |
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

### Interactive console

Once running, Infrarust drops you into an interactive console. Type `help` to see all commands. The full list is below, grouped by category.

Replies that span several lines open with a `◆ Title` header and keep a `│` rail down the left edge, so they stay easy to tell apart from the log lines printed around them. Tables are plain aligned columns; on a narrow terminal the last column is cut with `…`. One-line replies start with `✓` (done), `!` (warning) or `✗` (error). A command typed with missing arguments prints its usage, and a mistyped command name suggests the closest one.

Without colors (see [Colors](#colors)) the same replies use ASCII markers: `#` for the header, `|` for the rail, and `ok:`, `warn:` or `error:` in front of one-line replies, which keeps `docker logs` output easy to grep.

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
| `op` | | `op <username>` | Grant admin to a player (effective until restart unless persisted in config) |
| `deop` | | `deop <username>` | Revoke admin from a player |
| `ops` | `oplist` | `ops` | List current admins |

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
| `config` | | `config [key]` | List the servers, or print one value of the running configuration by its dotted path, such as `keepalive.retries` (secrets redacted) |

#### Plugins

| Command | Aliases | Usage | Description |
|---------|---------|-------|-------------|
| `plugins` | `pl` | `plugins` | List loaded plugins with their state; WASM plugins also get their health and queue wait |
| `plugin` | | `plugin <id>` | Show plugin details, and for a WASM plugin its health, generation, restarts, last fault and call queue |

When at least one WASM plugin is loaded, `plugins` adds two columns. `Health` is `healthy`, `recovering`, `quarantined` or `stopped`, followed by the time until the next attempt when one is scheduled (`quarantined 12s`). `Wait p99` is the queue wait that 99 % of the plugin's calls stayed under over the last minute. Native plugins show `-` in both.

For a WASM plugin, `plugin <id>` adds these lines:

```
│ health        ✕ quarantined, next attempt in 12s
│ generation    7
│ restarts      2 of 2 in the last 5m
│ last fault    the guest trapped: panicked at src/lib.rs:12:5: boom (4s ago, generation 7)
│ queue         0 of 1024 waiting, at most 12 in the last 1m
│ queue wait    p50 21µs, p99 1.2ms, max 3.4s over 1234 calls in the last 1m
```

A quarantined or recovering plugin keeps the state `enabled`. [Fault Model](../plugins/wasm/fault-model#watching-a-plugin-s-health) explains the health values and [Threading](../plugins/wasm/threading#watching-the-queue) the queue figures.

#### System

| Command | Aliases | Usage | Description |
|---------|---------|-------|-------------|
| `help` | `?` | `help [command]` | Show help |
| `version` | `ver` | `version` | Show version info |
| `update` | | `update [install\|apply [version]]` | Check for, install or apply an update |
| `restart` | | `restart` | Restart in place, keeping the players on forwarding servers connected (Linux) |
| `status` | `info` | `status` | Proxy overview |
| `stop` | `shutdown`, `exit`, `quit` | `stop` | Shut down the proxy |
| `clear` | `cls` | `clear` | Clear the screen |
| `gc` | | `gc` | Run garbage collection |

`version` prints the version, the supported Minecraft versions and the platform. When the last update check found a newer release, it adds an `update` line, `2.0.0-beta.6 available`, followed by `security fix` for a release that fixes a security issue. It reads the result of the last check and does not contact GitHub.

`update` runs a check now, whatever `[update] check` says, and prints what it found:

```
◆ Infrarust 2.0.0-beta.5 · security update available
│ available  2.0.0-beta.6  security fix
│ published  2026-11-20T10:00:00Z
│ notes      https://github.com/Shadowner/Infrarust/releases/tag/v2.0.0-beta.6
│ restart    hot restart
│
│ type update install to install it
```

The header ends with `up to date`, `update available`, `security update available` or `check failed`. `restart` is `hot restart` when the release can take over the sessions in place, `full restart` when the proxy has to be stopped and started again, and `stop and start the proxy by hand` on Windows; see [Restart the proxy](../guide/upgrading#restart-the-proxy). A few lines appear only when they apply: `install` follows an install that is running, done or failed; `binary` says the file on disk changed since the proxy started; `error` gives the reason a check failed; and `refused` explains why this binary cannot install the release, in place of the `update install` hint.

`update install` installs the newest release on the `[update]` channel, and `update install <version>` installs that version. With no version and no release found by an earlier check, it checks first, and stops with `Infrarust <version> is up to date` when there is nothing newer or `Update check failed: <reason>` when the check fails. The install runs in the background and the command waits for it, then prints `✓ Installed 2.0.0-beta.6, restart Infrarust to run it` or `✗ Update failed: <reason>`. It does not ask for confirmation and does not restart the proxy. A refusal prints `✗ Cannot install: <reason>`, for example `an installation is already running` while another install is in progress. The console has no rollback; run [`infrarust update rollback`](#update-rollback) from a shell. The [upgrading guide](../guide/upgrading) puts these steps in order, from the check to the restart.

`update apply` installs the release the last check found and then restarts the proxy in place. With no release known, it checks first, like `update install`. `update apply <version>` must name that same release, with or without a leading `v`; any other version stops with `✗ Only the advertised update 2.0.0-beta.6 can be applied in place`, followed by the commands to install it and restart by hand. Nothing is installed when:

- the proxy has no hot restart, as on macOS and Windows: `✗ Hot restart is unavailable on this platform`;
- the release needs more than a hot restart: `✗ 2.0.0-beta.6 needs a full restart, nothing was installed`.

Otherwise the command waits for the install and prints `✓ Installed 2.0.0-beta.6, restarting in place; follow the restart in the logs`, or `✗ Update failed: <reason>`. The restart that follows goes through the same [preflight](../guide/upgrading#preflight) as `restart`, so it can still be refused, with the new binary installed. See [Apply an update in place](../guide/upgrading#apply-an-update-in-place).

`restart` asks for a [hot restart](../guide/upgrading#restart-without-disconnecting-players) and returns at once with `✓ Hot restart requested, follow it in the logs`. The restart runs its preflight afterwards, so a refusal only shows in the log, on a `hot restart failed` warning. The command answers instead:

| Reply | When |
|-------|------|
| `A restart is already in progress` | Another restart has not finished |
| `The proxy is shutting down` | The proxy is stopping |
| `Hot restart is unavailable on this platform` | The proxy has no hot restart, as on macOS and Windows. Stop the proxy and start it again |

#### Plugin commands

A line that is not one of the console commands above runs as a proxy command: a plugin command by its name, an alias, or its `<plugin_id>:<name>` form, or the built-in `ir` command. A leading `/` is ignored. The console holds every permission, and plugin replies appear as log lines under the `infrarust::console` target. When a plugin command shares a name with a console command, use the `<plugin_id>:<name>` form. `help` lists the plugin commands at the end.

### Signals

| Signal | Effect |
|--------|--------|
| `SIGTERM` | Graceful stop: the proxy stops accepting connections, ends the sessions and waits up to 30 seconds for them, then gives the plugins up to 10 seconds. It exits with `0` |
| `SIGINT` (Ctrl-C) | The same graceful stop |
| `SIGUSR2` | A [hot restart](../guide/upgrading#restart-without-disconnecting-players), on Linux. On other Unix systems the proxy logs that hot restart is unavailable and goes on running |

A second `SIGTERM` or `SIGINT` while the proxy is stopping makes it exit at once, without waiting for the sessions or the plugins, with code `143` for `SIGTERM` or `130` for `SIGINT`. A `stop` typed in the console counts as the first request.

A `SIGUSR2` that arrives while the proxy is still starting, or while a hot restart is already running, is ignored, and the log says so.

During a hot restart, a stop signal depends on how far the restart has gone:

| Phase | `SIGTERM` or `SIGINT` |
|-------|-----------------------|
| The preflight, and the wait of up to 3 seconds for the logins in flight | Cancels the restart. The proxy stops gracefully, as above |
| From the moment the proxy disconnects the players it cannot hand over, until the new image runs | Deferred. The new image starts, logs `a stop signal arrived during the restart; this image stops now`, and stops gracefully. Further signals in this phase do not force an immediate exit |

---

## infrarust migrate

Migrates V1 proxy configs (YAML) to the V2 format (TOML). Run it once when upgrading from V1.

```bash
infrarust migrate <INPUT> [OPTIONS]
```

`INPUT` is the directory containing V1 server YAML files.

### Options

| Flag | Short | Default | Description |
|------|-------|---------|-------------|
| `--output <DIR>` | `-o` | `./servers` | Directory where converted TOML files are written |
| `--config <PATH>` | | | Path to a V1 global `config.yaml` to migrate alongside server configs |

### Examples

Migrate server configs only:

```bash
infrarust migrate ./old-servers
```

Migrate server configs and the global config at the same time:

```bash
infrarust migrate ./old-servers --output ./servers --config ./old-config.yaml
```

The command prints a summary of converted files, along with any warnings or errors encountered during migration. It exits with a non-zero code if any conversion fails.

---

## infrarust check-config

Validates the proxy configuration and exits without starting the proxy.

```bash
infrarust [OPTIONS] check-config [--json]
```

| Flag | Description |
|------|-------------|
| `--json` | Print the result as one JSON line on stdout |

It reads the file given by `-c` (`infrarust.toml` by default), applies `--bind`, `--plugins-dir` and `--servers-dir`, and runs the checks the proxy runs at startup, which include that `servers_dir` exists. It does not read the server files in that directory. A missing file is an error, and the setup wizard never runs.

Without `--json`, warnings go to stderr as `warning:` lines. The result goes to stdout as `configuration is valid: <path>`, or to stderr as `error: <reason>`:

```
$ infrarust -c /opt/infrarust/infrarust.toml check-config
warning: update.interval is 30m; checking GitHub more than once an hour can hit its rate limit
configuration is valid: /opt/infrarust/infrarust.toml
```

With `--json`, the warnings go into the JSON line instead:

```json
{"ok":true,"config":"/opt/infrarust/infrarust.toml","warnings":["update.interval is 30m; checking GitHub more than once an hour can hit its rate limit"],"error":null}
```

```json
{"ok":false,"config":"infrarust.toml","warnings":[],"error":"cannot read config file: infrarust.toml: No such file or directory (os error 2)"}
```

| Exit code | Meaning |
|-----------|---------|
| `0` | The configuration is valid |
| `1` | It is invalid or cannot be read |

`infrarust update install` and `infrarust update rollback` run `check-config --json` with the binary they are about to put in place, with the same `-c`, `--bind`, `--plugins-dir` and `--servers-dir`, and stop when it reports an error. An install started from the console or the admin API passes the options the proxy was started with.

---

## infrarust update

Checks for, installs and rolls back signed releases.

```bash
infrarust [OPTIONS] update <COMMAND>
```

| Command | What it does |
|---------|--------------|
| [`check`](#update-check) | Look for a newer signed release |
| [`install`](#update-install) | Download, verify and install a release over this binary |
| [`rollback`](#update-rollback) | Put back the binary that the last install replaced |

The commands read the `[update]` section of the file given by `-c`, and they load it the way the proxy does, so relative paths such as `servers_dir = "./servers"` resolve from the directory you run them in. Run them from the proxy's working directory. They treat a missing or broken configuration differently:

| Command | No configuration file | A configuration that does not load |
|---------|-----------------------|------------------------------------|
| `check` | Uses the `[update]` defaults | Fails with the error |
| `install` | Refuses, because the new binary checks the configuration before it replaces this one | Fails with the error |
| `rollback` | Fails when the previous binary checks the configuration | Warns and goes on, since the previous binary checks the configuration before it is restored |

Warnings and errors go to stderr. `--log-level` does not apply to these commands; `RUST_LOG` does.

### update check

```bash
infrarust [OPTIONS] update check [--channel <CHANNEL>] [--json]
```

| Flag | Default | Description |
|------|---------|-------------|
| `--channel <CHANNEL>` | `[update] channel` | `auto`, `stable` or `prerelease`, for this check only. See [Channels](../configuration/global#channels) |
| `--json` | | Print the result as one JSON line on stdout |

It queries the release list even when `[update] check = false`.

```
$ infrarust update check
current version: 2.0.0-beta.5
update available: 2.0.0-beta.6
  security fix: no
  published:    2026-11-20T10:00:00Z
  notes:        https://github.com/Shadowner/Infrarust/releases/tag/v2.0.0-beta.6
  restart:      hot, players stay connected
install it with `infrarust update install`
```

When this binary cannot install the release, the last line gives the reason instead, starting with `this binary cannot install it:`. When nothing newer exists, the line after the version reads `up to date`. A failed check prints only the version on stdout, and `error: <reason>` on stderr.

The `restart` line says how the new version takes over: `hot, players stay connected` on Linux when the release can take over the sessions of the running version, `full, players are disconnected` when it cannot or on macOS, and `manual, stop and start the proxy by hand` on Windows. See [Restart the proxy](../guide/upgrading#restart-the-proxy).

With `--json`:

```json
{"current_version":"2.0.0-beta.5","last_check":"2026-11-24T10:00:00Z","last_error":null,"available":{"version":"2.0.0-beta.6","published_at":"2026-11-20T10:00:00Z","notes_url":"https://github.com/Shadowner/Infrarust/releases/tag/v2.0.0-beta.6","security":false,"restart":"hot"},"install_refusal":null}
```

| Field | Meaning |
|-------|---------|
| `current_version` | Version of the binary that ran the check |
| `last_check` | When the check ran, in RFC 3339 UTC. `null` when the configuration did not load and no check ran |
| `last_error` | Why the check failed, `null` when it worked |
| `available` | The newest release offered on the channel, `null` when there is none |
| `available.version`, `available.published_at`, `available.notes_url` | Its version, publication time and release notes page |
| `available.security` | `true` when the release fixes a security issue |
| `available.restart` | `hot`, `full` or `manual`, as on the `restart` line |
| `install_refusal` | Why this binary cannot install releases, with what to do instead. `null` when it can |

`install_refusal` includes the write test on the binary's directory, so it depends on the user who runs the command.

| Exit code | Meaning |
|-----------|---------|
| `0` | Infrarust is up to date |
| `100` | An update is available, even one this binary cannot install |
| `1` | The check failed |

A cron job or monitoring script can branch on the code:

```bash
infrarust -c /opt/infrarust/infrarust.toml update check --json > /var/tmp/infrarust-update.json
if [ $? -eq 100 ]; then
  jq -r '.available.version' /var/tmp/infrarust-update.json
fi
```

### update install

```bash
infrarust [OPTIONS] update install [--version <VERSION>] [--yes]
```

| Flag | Short | Default | Description |
|------|-------|---------|-------------|
| `--version <VERSION>` | | newest on the channel | Install this version instead. A leading `v` is accepted |
| `--yes` | `-y` | | Install without asking for confirmation |

Without `--version`, it checks the `[update]` channel first. When nothing newer exists it prints `Infrarust <version> is up to date, nothing to install` and exits with `0`. `--version` installs that release whatever the channel, older or newer than the running one, as long as it is among the 20 most recent releases and carries a signed manifest.

It refuses without downloading anything when the binary was not built by the release pipeline, when it comes from the container image, when no release archive exists for its platform, when it cannot write to the binary's directory, or when the configuration file does not exist. Each refusal says what to do instead.

It asks for confirmation before it installs, with `No` as the default:

```
Install Infrarust 2.0.0-beta.6 over /opt/infrarust/infrarust? [y/N]
```

Answering no prints `cancelled, nothing was changed`. When stdin is not a terminal, nobody can answer, so the command stops with `stdin is not a terminal, so nobody can confirm; pass --yes to go ahead`. Pass `--yes` in scripts.

Each step prints a line on stderr, and the result goes to stdout:

```
looking for the release
downloading infrarust-linux-x86_64.tar.gz (9437184 bytes) for 2.0.0-beta.6
  25%
  50%
  75%
  100%
download verified
extracting the new binary
probing the new binary
checking the configuration with the new binary
replacing the binary
installed Infrarust 2.0.0-beta.6 at /opt/infrarust/infrarust, the previous binary is kept at /opt/infrarust/infrarust.old
restart Infrarust to run it; a proxy already running from this binary notices the change and says so in its logs
```

The install does not restart a running proxy; on Linux, a [hot restart](../guide/upgrading#restart-without-disconnecting-players) runs the new binary without disconnecting the players on forwarding servers. [What an install does](../guide/upgrading#what-an-install-does) details each step.

| Exit code | Meaning |
|-----------|---------|
| `0` | Installed, or nothing to install |
| `1` | Refused, cancelled or failed |

### update rollback

```bash
infrarust [OPTIONS] update rollback [--yes]
```

| Flag | Short | Description |
|------|-------|-------------|
| `--yes` | `-y` | Roll back without asking for confirmation |

Puts back `infrarust.old`, the binary the last install replaced (`infrarust.old.exe` on Windows). The two files swap places, so the binary it replaces becomes the new `infrarust.old` and a second rollback undoes the first. Before the swap, it runs the previous binary to read its version and runs its `check-config`.

```
$ infrarust update rollback
Restore Infrarust 2.0.0-beta.5 over /opt/infrarust/infrarust? [y/N] y
restored Infrarust 2.0.0-beta.5 at /opt/infrarust/infrarust, the replaced binary is kept at /opt/infrarust/infrarust.old
restart Infrarust to run it; a proxy already running from this binary notices the change and says so in its logs
run `infrarust update rollback` again to undo
```

Without `infrarust.old` it stops with `there is no previous binary to roll back to`. The confirmation works as for `install`.

| Exit code | Meaning |
|-----------|---------|
| `0` | Rolled back |
| `1` | Refused, cancelled or failed |

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
| `--output <DIR>` | `-o` | `crates/infrarust-core/registry` | Output directory for registry files |
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

## Environment variables

| Variable | Description |
|----------|-------------|
| `RUST_LOG` | Overrides `--log-level`. Accepts [tracing directives](https://docs.rs/tracing-subscriber/latest/tracing_subscriber/filter/struct.EnvFilter.html) like `infrarust=debug,infrarust_core=trace`. |
| `INFRARUST_COLOR` | `auto` (default), `always` or `never`. Takes priority over the variables below. See [Colors](#colors). |
| `NO_COLOR` | Any non-empty value turns colors off. |
| `FORCE_COLOR`, `CLICOLOR_FORCE` | Any value other than `0` turns colors on, even without a terminal. |
| `COLORTERM` | `truecolor` or `24bit` enables 24-bit colors when colors are forced; otherwise the 256-color palette is used. |

During a [hot restart](../guide/upgrading#restart-without-disconnecting-players), the proxy passes `INFRARUST_HANDOVER_FD` and `INFRARUST_HANDOVER_ATTEMPT` to the new image. They are internal: never set them yourself.

### Colors

In `auto` mode Infrarust colors the console, the logs and the startup banner only when standard output is a terminal. Game panels such as Pterodactyl or Pelican display colors without giving the process a terminal, so set `INFRARUST_COLOR=always` there. Set `INFRARUST_COLOR=never` to keep plain ASCII output, for example when logs go to a file.

When standard output is a terminal, log lines use a short local time, and fields that do not fit on the line wrap below the message. Otherwise each event stays on a single line with a full UTC timestamp, in `key=value` form:

```
2026-09-27T14:02:11Z INFO  player joined  player=Notch server=lobby span=session>login
```
