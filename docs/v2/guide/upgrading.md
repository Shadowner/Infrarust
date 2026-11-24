---
title: Upgrading Infrarust
description: Check for a new release, install it with infrarust update, roll it back, verify a release by hand, and restart the proxy on the new version.
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
  restart:      full, players are disconnected
install it with `infrarust update install`
```

The command runs as the service user from the proxy's working directory. It loads `infrarust.toml` the way the proxy does, so a relative `servers_dir` has to resolve from where it runs. Read the release notes before you install. The exit code is `0` when Infrarust is up to date, `100` when an update is available and `1` when the check failed; see [`update check`](../reference/cli#update-check) for the JSON output.

## Install the release

### From a shell

```bash
sudo -u infrarust sh -c 'cd /opt/infrarust && ./infrarust -c infrarust.toml update install'
```

The command asks `Install Infrarust 2.0.0-beta.6 over /opt/infrarust/infrarust? [y/N]`, prints each step, and ends with the path of the previous binary, kept as `infrarust.old`. Pass `-y` to skip the question, which a script or a cron job has to do. `--version <version>` installs a given release instead of the newest one, such as a pre-release on a stable channel or an older version to go back to. The release has to be among the 20 most recent ones.

Run it as the user the service runs as. The command creates `.infrarust.lock` in the binary's directory, and a lock file created by root blocks the installs the proxy runs itself from the console or the admin API, since those run as the service user.

### From the console or the admin API

The running proxy installs a release with the console command `update install [version]`, or with `POST /api/v1/proxy/update/install` on the [admin API](../plugins/builtin/admin-api#updates). Neither asks for confirmation, and neither restarts the proxy.

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

The proxy goes on running the binary it started from. Within a minute it notices that the file on disk changed and logs a warning. From then on the console `update` command tells you to restart Infrarust, the admin API reports `binary_changed: true`, and the dashboard shows `Restart to apply`. [Restart the proxy](#restart-the-proxy) to run the new version.

## Roll back

`update rollback` puts back the binary the last install replaced:

```bash
sudo -u infrarust sh -c 'cd /opt/infrarust && ./infrarust -c infrarust.toml update rollback'
```

It asks `Restore Infrarust 2.0.0-beta.5 over /opt/infrarust/infrarust? [y/N]`, runs `check-config` with the previous binary, and swaps the two files. The binary it replaces becomes the new `infrarust.old`, so running the command again undoes the rollback. Restart the proxy afterwards.

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

An install or a rollback replaces the file on disk; the proxy keeps running the binary it started from until you restart it. A restart ends every player's session, and players can join again as soon as the proxy is back.

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
