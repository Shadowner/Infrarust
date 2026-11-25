---
title: Upgrading Infrarust
description: Check for a new release, install it with infrarust update, roll it back, verify a release by hand, and restart the proxy on the new version, in place on Linux without disconnecting forwarding players.
---

# Upgrading Infrarust

How you upgrade depends on where the binary came from. A binary from a GitHub release archive can replace itself with `infrarust update install`. The Docker image and binaries you built yourself are upgraded the way you installed them.

## Pick a method

| The binary comes from | Upgrade it with |
|-----------------------|-----------------|
| A release archive from GitHub | `infrarust update install`, or a new archive [downloaded by hand](#verify-a-release-by-hand) |
| The Docker image | `docker pull`, then recreate the container. See [Docker](./docker#upgrading) |
| `cargo install infrarust` | `cargo install infrarust --version <version>` |
| A build from source | A new build of the release tag |

`infrarust update check --json` tells you which case you are in: its `install_refusal` field is `null` when the binary can install releases, and otherwise says how to upgrade it.

To replace the binary by hand instead, download the archive for your platform from the [releases page](https://github.com/Shadowner/Infrarust/releases), check it as shown in [Verify a release by hand](#verify-a-release-by-hand), stop the proxy, put the new binary in place of the old one, and start the proxy again.

The examples below follow the layout of [Running with systemd](./systemd): the binary is `/opt/infrarust/infrarust`, owned by the `infrarust` user, and `/opt/infrarust` is the service's working directory.

## Check for a new release

The proxy checks for new releases on its own, once a day by default, and reports one in its log, in the console, in `/ir version` for admins, and on the admin dashboard. [Update checks](../configuration/global#update-checks) covers the settings, the channels, and what each check sends to GitHub.

To check from a shell:

```bash
sudo -u infrarust sh -c 'cd /opt/infrarust && ./infrarust -c infrarust.toml update check'
```

```
current version: 2.0.0-beta.5
update available: 2.0.0-beta.6
  security fix: no
  published:    2026-11-20T10:00:00Z
  notes:        https://github.com/Shadowner/Infrarust/releases/tag/v2.0.0-beta.6
  restart:      hot, players stay connected
install it with `infrarust update install`
```

The `restart` line says how the release takes over, see [Restart the proxy](#restart-the-proxy). The command runs as the service user from the proxy's working directory. It loads `infrarust.toml` the way the proxy does, so a relative `servers_dir` has to resolve from where it runs. Read the release notes before you install. The exit code is `0` when Infrarust is up to date, `100` when an update is available and `1` when the check failed; see [`update check`](../reference/cli#update-check) for the JSON output.

## Install the release

### From a shell

```bash
sudo -u infrarust sh -c 'cd /opt/infrarust && ./infrarust -c infrarust.toml update install'
```

The command asks `Install Infrarust 2.0.0-beta.6 over /opt/infrarust/infrarust? [y/N]`, prints each step, and ends with the path of the previous binary, kept as `infrarust.old`. Pass `-y` to skip the question, which a script or a cron job has to do. `--version <version>` installs a given release instead of the newest one, such as a pre-release on a stable channel or an older version to go back to. The release has to be among the 20 most recent ones.

Run it as the user the service runs as. The command creates `.infrarust.lock` in the binary's directory, and a lock file created by root blocks the installs the proxy runs itself from the console or the admin API, since those run as the service user.

### From the console or the admin API

The running proxy installs a release with the console command `update install [version]`, or with `POST /api/v1/proxy/update/install` on the [admin API](../plugins/builtin/admin-api#updates). Neither asks for confirmation, and neither restarts the proxy. On Linux, the console command `update apply` installs the release and then restarts the proxy in place, see [Apply an update in place](#apply-an-update-in-place).

The install then runs inside the proxy, as its user, so that user needs write access to the directory of the executable. The [systemd unit](./systemd#unit-file) gives it with `ReadWritePaths=/opt/infrarust`. A binary in a directory the service cannot write, such as `/usr/local/bin` under `ProtectSystem=strict`, is refused with `cannot write to /usr/local/bin (...)`, followed by what to do instead.

### What an install does

An install works on the file the running executable resolves to, symlinks followed, and on the directory that holds it.

1. It locks `.infrarust.lock` in the binary's directory, so only one install or rollback runs there at a time. A second one stops with `another update is running`.
2. It reads the release list and verifies the release's `manifest.json` against the release key built into the binary.
3. It downloads the archive for the binary's platform and checks its size and sha256 against the manifest.
4. It extracts the new binary next to the old one, as `.infrarust.new` (`infrarust.new.exe` on Windows). On Linux and macOS it gets mode `0755`, and when the install runs as root, the owner of the old binary.
5. It runs the new binary once to ask its version and target, which must be the ones the manifest announces.
6. It runs `check-config` with the new binary and the same options. A configuration the new version rejects stops the install with `the binary rejects the configuration <path>: <reason>`, and nothing is replaced.
7. It swaps the files. On Linux and macOS, the current binary gets a second name, `infrarust.old`, and the new one is renamed over `infrarust`, so the path always leads to a complete binary. On Windows, `infrarust.exe` is renamed to `infrarust.old.exe` and the new file to `infrarust.exe`.

If a step of the swap fails, the steps already done are undone. When the undo fails too, the error ends with `check the binary directory by hand`.

The temporary files are removed after every attempt; `.infrarust.lock` stays. Only one previous binary is kept, since each install replaces `infrarust.old`. When the running proxy installs twice before a restart, the second install keeps the version that is still running as `infrarust.old`, so a rollback returns to it.

The proxy goes on running the binary it started from. Within a minute it notices that the file on disk changed and logs a warning. From then on the console `update` command tells you to restart Infrarust, the admin API reports `binary_changed: true`, and the dashboard shows `Restart to apply`. [Restart the proxy](#restart-the-proxy) to run the new version; on Linux, a [hot restart](#restart-without-disconnecting-players) runs it without disconnecting the players on forwarding servers.

## Roll back

`update rollback` puts back the binary the last install replaced:

```bash
sudo -u infrarust sh -c 'cd /opt/infrarust && ./infrarust -c infrarust.toml update rollback'
```

It asks `Restore Infrarust 2.0.0-beta.5 over /opt/infrarust/infrarust? [y/N]`, runs `check-config` with the previous binary, and swaps the two files. The binary it replaces becomes the new `infrarust.old`, so running the command again undoes the rollback. Restart the proxy afterwards: in place when the previous version supports hot restart, by hand otherwise.

The previous binary has to accept the configuration. If you added settings that only the new version knows, remove them first, or the rollback stops with `the binary rejects the configuration`. Rollback is only available from a shell, not from the console or the admin API.

## Verify a release by hand

Every release carries `manifest.json`, its minisign signature `manifest.json.minisig`, and `SHA256SUMS`, next to the six archives. The manifest lists each archive with its size and sha256, and the signature covers the manifest. `update install` runs these checks for you; to run them on a download yourself:

```bash
TAG=v2.0.0-beta.6
BASE=https://github.com/Shadowner/Infrarust/releases/download/$TAG
curl -LO "$BASE/infrarust-linux-x86_64.tar.gz"
curl -LO "$BASE/manifest.json"
curl -LO "$BASE/manifest.json.minisig"
curl -LO "$BASE/SHA256SUMS"
```

Check the signature of the manifest with [minisign](https://jedisct1.github.io/minisign/). The release public key is in the repository, in [`crates/infrarust-core/src/update/keys/`](https://github.com/Shadowner/Infrarust/tree/main/crates/infrarust-core/src/update/keys): the key is the line after `untrusted comment:` in `release-1.pub`.

```bash
minisign -Vm manifest.json -P '<public key from release-1.pub>'
```

`minisign -Vm manifest.json -p release-1.pub` reads the key from the file instead. minisign prints `Signature and comment signature verified` and the trusted comment, `infrarust v2.0.0-beta.6`, which must name the tag you downloaded.

Check the archive against its checksum:

```bash
sha256sum -c --ignore-missing SHA256SUMS
```

`SHA256SUMS` itself is not signed. To compare the archive with the values the signature covers, take them from the manifest:

```bash
jq -r '.artifacts[] | "\(.sha256)  \(.file)"' manifest.json | sha256sum -c --ignore-missing
```

Both print `infrarust-linux-x86_64.tar.gz: OK`. The [GitHub CLI](https://cli.github.com/) checks that GitHub Actions built the archive from this repository:

```bash
gh attestation verify infrarust-linux-x86_64.tar.gz --repo Shadowner/Infrarust
```

The release workflow attests each archive and `manifest.json`.

## Binaries you built yourself

`update install` only replaces a binary that the release workflow built. Anything else, from `cargo build`, `cargo install` or an image you built from the `Dockerfile`, is refused:

```
error: this build was not made by the release pipeline; install with `cargo install infrarust --version 2.0.0-beta.6` or download the release from https://github.com/Shadowner/Infrarust/releases
```

An install swaps in the binary from a release archive. On a build you made yourself, it would replace your choice of features and target with those of the release build, so the updater leaves such a binary alone. Upgrade it the way you built it: `cargo install infrarust --version <version>`, or a new build of the release tag.

## Restart the proxy

An install or a rollback replaces the file on disk; the proxy keeps running the binary it started from until it restarts. On Linux, it can [restart in place](#restart-without-disconnecting-players) and keep the players on forwarding servers connected. A stop followed by a start ends every player's session, and players can join again as soon as the proxy is back.

`update check` and the [admin API](../plugins/builtin/admin-api#updates) give each release a `restart` value that says which of the two it needs. The console `update` command shows the same value as `hot restart`, `full restart` or `stop and start the proxy by hand`.

| `restart` | When | What to do |
|-----------|------|------------|
| `hot` | On Linux, when the release can take over the sessions of the running version | A [hot restart](#restart-without-disconnecting-players) runs it, and players on forwarding servers stay connected |
| `full` | The release cannot take over the sessions, or the proxy runs on macOS or another Unix system | Stop the proxy and start it again. Every player is disconnected |
| `manual` | On Windows | Stop the proxy and start it again yourself |

### Stop and start

With systemd:

```bash
sudo systemctl restart infrarust
journalctl -u infrarust -n 20
```

The startup line `starting infrarust v2.0.0-beta.6` confirms the version that runs. See [Running with systemd](./systemd#start-stop-and-restart) for the stop timeout.

With Docker, the upgrade is the restart: pull the new image and recreate the container.

```bash
docker compose pull
docker compose up -d
```

[Docker](./docker#upgrading) has the `docker run` version.

When you started Infrarust by hand in a terminal, type `stop` in its console and start it again. On Windows, stop the proxy and start it again yourself; `update check` reports this as `manual, stop and start the proxy by hand`.

## Restart without disconnecting players

On Linux, the proxy can restart in place. The running program replaces itself with the binary at its own path, inside the same process, and hands its connections over. The log, like this page, calls the program from before the restart the old or previous image, and the one after it the new image. The PID does not change, so systemd, Docker and game panels never see the proxy exit. Players on `passthrough`, `zero_copy` and `server_only` servers keep their connection.

The new image starts the way a fresh proxy does: it reads `infrarust.toml` and the server files again, and loads the plugins again. A hot restart is therefore also how you apply a change to `infrarust.toml` without disconnecting the players on forwarding servers. When `bind` changed, the new image listens on the new address, or keeps the old one with a warning when the new address cannot be bound.

### Start a hot restart

| From | How |
|------|-----|
| The console | `restart`. [`update apply`](#apply-an-update-in-place) installs a release first |
| A shell | `kill -USR2 <pid>`. With systemd, `sudo systemctl reload infrarust` once the unit has [`ExecReload`](./systemd#reload-without-disconnecting-players). With Docker, `docker kill --signal=USR2 infrarust` |
| The admin API | `POST /api/v1/proxy/restart`, see [Hot restart](../plugins/builtin/admin-api#hot-restart) |
| The dashboard | `Hot Restart` on the dashboard and on the Configuration page, or the `Restart to apply` badge after an install |
| A plugin | `PluginContext::proxy_restart()`, see [Hot restarts](../plugins/dev/lifecycle#hot-restarts) |

Each of these asks for the restart and returns at once. Follow the restart in the log. The old image logs `hot restart requested, checking the new binary`, then one line per step. The new image logs `taking over from the previous image`, then `handover committed, this image owns the inherited sessions`, then `inherited sessions resumed` with the number of sessions it `restored` and `dropped`, and last `infrarust is ready, accepting connections`.

One restart runs at a time. A request made while a restart is in progress, or while the proxy stops, is refused. A SIGUSR2 that arrives while the proxy is still starting is ignored, and the log says so.

### Apply an update in place

The console command `update apply` installs a release and restarts in place in one step. It takes the release the last check found, and checks first when none is known. `update apply <version>` names that same release; to install another version, use `update install <version>` and restart afterwards. A release whose `restart` is not `hot` is refused before anything is installed, with `<version> needs a full restart, nothing was installed`.

The restart still goes through the [preflight](#preflight). When the preflight refuses it, the new binary stays installed and the proxy keeps running the old one. Fix the cause, then type `restart`.

From a shell, the same takes two commands:

```bash
sudo -u infrarust sh -c 'cd /opt/infrarust && ./infrarust -c infrarust.toml update install -y'
sudo systemctl reload infrarust
```

Through the admin API, call `POST /api/v1/proxy/update/install`, wait until `GET /api/v1/proxy/update` reports `install.state` as `installed`, then call `POST /api/v1/proxy/restart`.

### What players see

- On `passthrough`, `zero_copy` and `server_only` servers, the client and the backend keep their TCP connections. Traffic stops while the sessions move to the new image, then resumes where it stopped, with no bytes lost in either direction. In our tests with a release build on a 16-core machine, the longest pause any player saw was about 70 ms with 1000 sessions and about 140 ms with 2000.
- On `offline` and `client_only` servers, the proxy holds each player's game state itself and cannot hand it over. These players are disconnected with `Proxy is restarting, please reconnect`, and can join again right away.
- A connection that is still logging in gets up to 3 seconds to finish. A login on a forwarding server that completes in time is carried over like the others. One that does not is disconnected with the same message.
- New connections, server list pings included, wait in the kernel's queue while the proxy restarts, and the new image accepts them.

A forwarding session that cannot be handed over is closed, with a warning in the log. A carried-over player whose server is no longer in the configuration keeps the connection to its backend; the new image logs a warning for it.

### Preflight

Before it changes anything, the proxy checks that the new image can take over:

1. No server with a [`local` server manager](./server-management#local-process) runs as a child process of the proxy. The new image could not manage these processes, so a local server that is starting, online or stopping blocks the restart. Stop it first with `stop-server <id>`.
2. The binary at the path the proxy started from, symlinks followed, runs and supports the handover protocol of the running version. A version without hot restart is refused.
3. That binary is built for the same target as the running one, such as `x86_64-unknown-linux-musl`.
4. Its `check-config` accepts the configuration, with the command-line options the proxy was started with.

Each run of the new binary has 10 seconds to answer. When a check fails, nothing has changed and the proxy keeps serving every player. The request was answered already, so the reason is in the log, on a `hot restart failed` warning:

```
lobby, survival run as child processes of the proxy and would be orphaned by a hot restart; stop them first
/opt/infrarust/infrarust is built for aarch64-unknown-linux-musl, but this proxy runs as x86_64-unknown-linux-musl; install the x86_64-unknown-linux-musl build
/opt/infrarust/infrarust rejects the configuration infrarust.toml: <reason>; fix it, then restart again
```

### Rollback

After the preflight, the old image stops accepting connections, waits for the logins in flight, disconnects the players it cannot hand over, stops its plugins and pauses the forwarding sessions. Then it starts the new image.

If the new image fails before it has taken the sessions over, the proxy rolls back. That covers a binary that cannot start, a configuration or command line it rejects, a listener it cannot open, and a panic during its startup. The proxy starts the previous version again in the same process, and that version resumes the sessions. Players on forwarding servers stay connected through it. The log shows `rolling the hot restart back to <version>: <reason>`.

The previous version starts from the binary the proxy was running, read through `/proc`, which works even after an install replaced the file on disk. `/proc` is mounted on any standard Linux system and in Docker containers. When that copy cannot be used, the proxy falls back to `infrarust.old` next to the binary, then to the binary at the original path, and never to the binary that just failed.

::: warning The rollback reads the same configuration
The previous version reads `infrarust.toml` again, with the same command line. A failure that does not come from the new binary stops it as well, for example a ban file that cannot be parsed. The proxy then exits and every session is lost. A new `bind` address that cannot be bound is not such a failure: both versions keep listening on the old address and log a warning.
:::

Once the new image has taken the sessions over, there is no way back. A failure after that point, such as a plugin that does not load, is logged and the new image keeps running.

### Stopping during a hot restart

A SIGTERM or SIGINT that arrives during the preflight, or while the proxy waits for the logins in flight, cancels the restart. The proxy then stops gracefully, as it would at any other time. `systemctl stop` and `docker stop` send SIGTERM.

From the moment the proxy starts disconnecting players for the restart, a stop waits for the handover: the new image starts, then stops gracefully. Sending the signal again in that phase does not make the proxy exit at once. See [Signals](../reference/cli#signals).

### Limits

- Hot restart works on Linux only. On macOS and Windows, the console `restart` command answers `Hot restart is unavailable here`, the admin API answers `503`, and the dashboard shows no hot restart action. Stop the proxy and start it again.
- Players on `offline` and `client_only` servers are disconnected, see [What players see](#what-players-see).
- A running `local` server blocks the restart, see [Preflight](#preflight).
- The new binary must support hot restart as well. Going back to a release without it takes a full restart, and `update check` reports `full` for such a release.
- The old image stops its plugins and the new one loads them again, so a plugin keeps only the state it saves. Plugin authors can read [Hot restarts](../plugins/dev/lifecycle#hot-restarts).
- A line typed in the console during the restart may be lost. Type it again once the new image logs `infrarust is ready, accepting connections`.
