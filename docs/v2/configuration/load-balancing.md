---
title: Load Balancing
description: Distribute players across the backend addresses of one server with weighted round-robin or least-connections, plus slow start and health-based ejection.
---

# Load balancing

When a server lists several `addresses`, Infrarust distributes new sessions across them. Every strategy is weight-aware, tracks how many players sit on each address, and pushes failing addresses out of rotation until they answer again.

This page covers balancing across the replicas of one server. If you are putting Infrarust *behind* HAProxy or a cloud load balancer, see [Running behind a load balancer](../guide/load-balancer) instead.

## Choosing a strategy

```toml
# servers/lobby.toml
domains = ["play.example.com"]
balance = "least_conn"
addresses = [
    { address = "10.0.0.1:25565", weight = 3 },
    { address = "10.0.0.2:25565", weight = 1 },
    "10.0.0.3:25565",
]
```

An address entry is either a plain `"host:port"` string, which gets weight 1, or a table with an explicit `weight`. Mixing both forms in one list is fine.

| Strategy | Behavior |
|----------|----------|
| `first_available` | Addresses tried in config order. The default, and what a single-address server does anyway. |
| `round_robin` | Smooth weighted round-robin, the same algorithm nginx uses. Spreads picks evenly rather than in bursts. |
| `least_conn` | Sends each player to the address with the fewest players per unit of weight. Recommended for multi-address servers. |

`first_available` stays the default so upgrading changes nothing until you ask for it. It ignores weights, so Infrarust warns at startup if you set weights or `slow_start` alongside it.

Weights are relative. With weights 3 and 1, the first address takes three players for every one the second takes.

## Slow start

A Minecraft server that just booted is a JVM warming up: JIT still cold, chunks not loaded, caches empty. Sending it a full share of players in the first second causes timeouts.

```toml
slow_start = "45s"
slow_start_aggression = 2.0
```

`slow_start` ramps a freshly healthy address from near zero to its nominal weight over the window. `slow_start_aggression` shapes the curve: `1.0` is linear, higher values keep the address quieter for longer before catching up. The ramp applies to `round_robin` and `least_conn`; `first_available` has no weights to modulate.

The ramp starts whenever an address becomes healthy again, which covers three cases: a backend the proxy just woke through `server_manager`, an address that recovered after being ejected, and a replica that came back on its own.

## Health and ejection

Infrarust learns which addresses work from real connection attempts. Three failures inside a 10 second window eject an address: new sessions stop going to it, though it stays at the tail of the failover list so a total outage still gets a chance rather than a kick.

Failures only count inside that window. Three failures spread across three days no longer add up to an ejection, which is the same rule nginx applies with `fail_timeout`.

An ejected address is retried after a backoff that grows with each ejection, starting at 30 seconds and capping at 5 minutes. Exactly one recovery attempt is allowed per backoff window, so a dead backend never absorbs a burst of players. When the attempt succeeds the address returns to rotation in slow start.

If every address of a server ends up ejected, Infrarust still balances across them rather than falling back to config order. A network blip that ejects everything should not turn into a stampede onto the first address.

### Active probing

The recovery attempt above normally comes from a background prober rather than from a player, so nobody pays a connect timeout for it.

```toml
# infrarust.toml
[active_health]
enabled = true            # probe ejected addresses so they can recover
kind = "tcp"              # "tcp" or "status_ping"
unhealthy_interval = "10s"
probe_healthy = false     # also probe addresses that are currently healthy
interval = "30s"          # sweep interval when probe_healthy is set
timeout = "3s"
max_concurrent = 8
```

Recovery probing is on by default and costs nothing while everything is healthy, since only ejected addresses are checked. Probing healthy addresses is opt-in: passive health already catches failures from real traffic, and a status ping reaches the server main thread on some implementations.

The `tcp` probe opens a connection and closes it, which is exactly what the proxy does when it dials a backend, so it cannot disagree with reality. The `status_ping` probe runs the full Minecraft status exchange and proves the server actually answers.

Any server can override the whole block with its own `[active_health]` table.

## Bounding failover latency

Every address the proxy tries costs up to `connect_timeout` before it moves to the next one. With four dead addresses and the default 5 second timeout, a player waits 20 seconds before landing in limbo or getting kicked.

```toml
# infrarust.toml
connect_max_attempts = 3
```

Worst-case login latency is `connect_max_attempts × connect_timeout`. Set it to `0` to try every address.

## What gets balanced

Balancing applies to the login path, to server switches, and to limbo exits. Status pings and legacy pings go through the same ordering, so a dead first address no longer makes a healthy server look offline in the server list.

Selection also counts logins that are still negotiating, not only players already attached to a backend. Without that, a burst of simultaneous logins would all read the same connection counts and pile onto one address.

## Observability

With the `telemetry` feature built in, these instruments carry an `address` attribute:

| Metric | Type | Description |
|--------|------|-------------|
| `infrarust.backend.connect.duration` | histogram | Time to open a backend connection |
| `infrarust.backend.connect.failures` | counter | Failed connection attempts |
| `infrarust.backend.health.transitions` | counter | Ejections and recoveries, tagged with the new state |
| `infrarust.backend.status.latency` | histogram | Status ping round-trip time |

Cardinality is bounded by the addresses you declare, so these are safe to keep on.

## Scope

Balancing works across the addresses of one server config. Choosing *which* server a player lands on, for example spreading players over `lobby-1` through `lobby-3`, is not built in. Plugins can do it today by redirecting on `PlayerChooseInitialServerEvent`.

Counts are per proxy instance. If several Infrarust instances front the same backends, each balances against its own view.
