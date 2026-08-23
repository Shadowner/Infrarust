---
title: Running with systemd
description: Set up Infrarust as a systemd service with automatic restarts, journald logging, and a dedicated system user.
---

# Running with systemd

On a Linux server, systemd keeps Infrarust running across reboots and restarts it if it crashes. This page covers creating a dedicated user, writing a unit file, and reading logs with journald.

## Create a system user

Run Infrarust under its own unprivileged user rather than root:

```bash
sudo useradd --system --shell /usr/sbin/nologin --create-home --home-dir /opt/infrarust infrarust
```

Copy the binary and your configuration into that home directory:

```bash
sudo cp /usr/local/bin/infrarust /opt/infrarust/
sudo mkdir -p /opt/infrarust/servers
sudo cp infrarust.toml /opt/infrarust/
sudo cp servers/*.toml /opt/infrarust/servers/
sudo chown -R infrarust:infrarust /opt/infrarust
```

Your directory layout should look like this:

```
/opt/infrarust/
├── infrarust
├── infrarust.toml
└── servers/
    └── survival.toml
```

## Unit file

Create `/etc/systemd/system/infrarust.service`:

```ini
[Unit]
Description=Infrarust Minecraft Reverse Proxy
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=infrarust
Group=infrarust
WorkingDirectory=/opt/infrarust
ExecStart=/opt/infrarust/infrarust --config /opt/infrarust/infrarust.toml
Restart=on-failure
RestartSec=5

# Shutdown grace period: Infrarust drains active connections
# for up to 30 seconds before forcing exit
TimeoutStopSec=35

# Hardening
NoNewPrivileges=true
ProtectSystem=strict
ProtectHome=true
ReadWritePaths=/opt/infrarust

# File descriptor limit for high player counts
LimitNOFILE=65536

[Install]
WantedBy=multi-user.target
```

::: tip
`TimeoutStopSec=35` gives Infrarust time to finish its 30-second connection drain before systemd sends SIGKILL. The 5-second buffer between 30 and 35 accounts for plugin shutdown before the drain starts.
:::

Reload systemd and enable the service:

```bash
sudo systemctl daemon-reload
sudo systemctl enable infrarust
```

## Start, stop, and restart

```bash
sudo systemctl start infrarust
sudo systemctl stop infrarust
sudo systemctl restart infrarust
```

Check if the service is running:

```bash
sudo systemctl status infrarust
```

## Log level

Infrarust uses the `--log-level` flag to set the minimum log level. The default is `info`. You can override it in the unit file by editing the `ExecStart` line:

```ini
ExecStart=/opt/infrarust/infrarust --config /opt/infrarust/infrarust.toml --log-level debug
```

The `RUST_LOG` environment variable takes priority over `--log-level`. To set it in the unit file, add an `Environment` directive:

```ini
[Service]
Environment=RUST_LOG=infrarust_core=debug,infrarust=info
```

## Reading logs with journald

systemd captures all stdout and stderr output from Infrarust automatically. No log files to manage or rotate.

View recent logs:

```bash
journalctl -u infrarust -n 50
```

Follow logs in real time:

```bash
journalctl -u infrarust -f
```

Show logs since the last boot:

```bash
journalctl -u infrarust -b
```

Filter by time range:

```bash
journalctl -u infrarust --since "2025-01-15 10:00" --until "2025-01-15 12:00"
```

Show only errors and worse:

```bash
journalctl -u infrarust -p err
```

To include warnings as well, use `-p warning`.

## Binding to port 25565

Ports below 1024 require extra privileges. Since the service runs as the `infrarust` user, you have two options.

Option A: use a port above 1024. Bind Infrarust to port 25577 and redirect traffic with iptables:

```bash
sudo iptables -t nat -A PREROUTING -p tcp --dport 25565 -j REDIRECT --to-port 25577
```

Set `bind = "0.0.0.0:25577"` in your `infrarust.toml`, or pass `--bind 0.0.0.0:25577` on the `ExecStart` line to avoid editing the config file.

Option B: grant the binary the capability to bind low ports. Add the `AmbientCapabilities` directive to the unit file:

```ini{5}
[Service]
User=infrarust
Group=infrarust
ExecStart=/opt/infrarust/infrarust --config /opt/infrarust/infrarust.toml
AmbientCapabilities=CAP_NET_BIND_SERVICE
```

This lets the process bind to port 25565 without running as root.

Option C: let systemd own the socket, using socket activation. See the next section.

## Socket activation

With socket activation, systemd creates the listening socket itself and hands it to Infrarust as an inherited file descriptor. Infrarust detects it through the standard `LISTEN_FDS` protocol and accepts connections on it instead of binding its own.

This is useful in two situations:

- **Privileged ports:** systemd binds port 25565 as root before dropping to the `infrarust` user, so no `AmbientCapabilities` is needed.
- **Rootless containers:** under rootless Podman, a userspace port forwarder sits in front of the proxy and rewrites the source address of every connection, so every player appears to come from the same local IP. A socket created outside the container skips the forwarder, so connections arrive at kernel speed and keep the client source address that bans, rate limiting, and logs depend on.

Create `/etc/systemd/system/infrarust.socket`:

```ini
[Unit]
Description=Infrarust Minecraft Reverse Proxy Socket

[Socket]
ListenStream=0.0.0.0:25565
Accept=no

[Install]
WantedBy=sockets.target
```

`Accept=no` is required: Infrarust needs the listening socket itself, not one connection per instance.

Then bind the service to it in `/etc/systemd/system/infrarust.service`:

```ini{3,4}
[Unit]
Description=Infrarust Minecraft Reverse Proxy
Requires=infrarust.socket
After=infrarust.socket
```

Enable the socket:

```bash
sudo systemctl daemon-reload
sudo systemctl enable --now infrarust.socket
```

systemd now starts Infrarust on the first incoming connection. To keep the proxy running permanently instead of on demand, leave `infrarust.service` enabled as well. It still picks up the socket from systemd.

Check the logs to confirm the handover:

```bash
journalctl -u infrarust | grep "socket activation"
```

The `listener bound` line just after it reports the address the proxy is listening on.

::: warning
When a socket is passed in, the `bind` address in `infrarust.toml` and the `so_reuseport` setting are ignored. The listening address and backlog belong to the `.socket` unit. Only the first passed file descriptor is used; Infrarust listens on one socket.
:::

::: tip
The same mechanism works with any activator that speaks the systemd protocol. [`systemfd`](https://github.com/mitsuhiko/systemfd) is handy during development: `systemfd --no-pid -s tcp::25565 -- infrarust` keeps the port open across restarts.
:::

## Updating Infrarust

To deploy a new version:

```bash
sudo systemctl stop infrarust
sudo cp /path/to/new/infrarust /opt/infrarust/infrarust
sudo chown infrarust:infrarust /opt/infrarust/infrarust
sudo systemctl start infrarust
```

Check the logs after starting to confirm the new version loaded:

```bash
journalctl -u infrarust -n 10
```
