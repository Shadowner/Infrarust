---
title: Telemetry
description: Configure OpenTelemetry traces and metrics export in Infrarust
---

# Telemetry

Infrarust exports traces and metrics through OpenTelemetry (OTel). Both are sent via OTLP to any compatible collector (Jaeger, Grafana Tempo, Prometheus via OTel Collector, etc.).

Telemetry is disabled by default. Add a `[telemetry]` section to your proxy config to enable it.

## Minimal setup

```toml
[telemetry]
enabled = true
```

This sends traces and metrics to `http://localhost:4317` using gRPC. If you run an OTel Collector on the same host with default settings, this is all you need.

## Full configuration

```toml
[telemetry]
enabled = true
endpoint = "http://otel-collector:4317"
protocol = "grpc"                        # "grpc" (default) or "http"

[telemetry.traces]
enabled = true
sampling_ratio = 0.1                     # sample 10% of status pings

[telemetry.metrics]
enabled = true
export_interval = "15s"

[telemetry.resource]
service_name = "infrarust"
service_version = "2.0.0-beta.2"        # defaults to Infrarust's crate version
```

## Configuration reference

### `[telemetry]`

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `enabled` | bool | `false` | Master switch. When `false`, no exporters are created. |
| `endpoint` | string | `"http://localhost:4317"` | OTLP endpoint URL. Omit the field to use the default; the config key is optional. |
| `protocol` | string | `"grpc"` | Export protocol. Accepts `"grpc"` or `"http"`. |

### `[telemetry.traces]`

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `enabled` | bool | `true` | Enable trace export. |
| `sampling_ratio` | float | `0.1` | Sampling ratio for status pings (0.0 to 1.0). Login connections are always traced at 100%. |

### `[telemetry.metrics]`

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `enabled` | bool | `true` | Enable metrics export. |
| `export_interval` | duration | `"15s"` | How often metrics are pushed to the collector. Uses `humantime` format (`"15s"`, `"1m"`, etc.). |

### `[telemetry.resource]`

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `service_name` | string | `"infrarust"` | OTel resource `service.name` attribute. |
| `service_version` | string | crate version | OTel resource `service.version` attribute. Defaults to the Infrarust binary version. |

## Protocol choice

Infrarust supports two OTLP transport protocols:

`grpc` uses Tonic (HTTP/2 + protobuf). This is the default and works with most collectors out of the box. The default port is 4317.

`http` uses HTTP/1.1 + protobuf. Use this when your collector sits behind a reverse proxy that doesn't support HTTP/2, or when gRPC is blocked by a firewall. The default port is typically 4318 on most collectors.

```toml
[telemetry]
enabled = true
endpoint = "http://otel-collector:4318"
protocol = "http"
```

## Traces

Infrarust creates two root span types:

`connection` spans cover the full lifecycle of a player login. They include these attributes:

| Attribute | Description |
|-----------|-------------|
| `client.ip` | Player's IP address |
| `server.domain` | Domain the player connected to |
| `server.id` | Matched server config ID |
| `proxy.mode` | Proxy mode (Passthrough, ClientOnly, etc.) |
| `player.username` | Player's username |
| `protocol.version` | Minecraft protocol version number |

`status.ping` spans cover server list ping requests.

### Sampling

The custom sampler (`InfrarustSampler`) applies different strategies per span type:

- Login connections (`connection`) are always sampled at 100%. You never want to miss a real player session.
- Status pings (`status.ping`) are sampled at the ratio set by `sampling_ratio`. A busy server might receive thousands of pings per minute, so the default of `0.1` (10%) keeps trace volume manageable.
- All other root spans are sampled at 100%.

The sampler is wrapped in a `ParentBased` sampler, so child spans inherit their parent's sampling decision.

Set `sampling_ratio = 1.0` to trace every status ping, or `0.0` to drop them all:

```toml
[telemetry.traces]
sampling_ratio = 0.0   # no status ping traces
```

## Metrics

All metrics use the `infrarust` meter name. They are exported at the interval set by `export_interval`.

| Metric | Type | Unit | Labels | Description |
|--------|------|------|--------|-------------|
| `infrarust.connections.total` | Counter | none | `server`, `proxy_mode` | Total connections received |
| `infrarust.connections.active` | UpDownCounter | none | none | Currently open connections |
| `infrarust.connections.rejected` | Counter | none | `reason` | Rejected connections |
| `infrarust.players.online` | UpDownCounter | none | `server` | Players currently connected |
| `infrarust.connection.duration` | Histogram | seconds | `server`, `proxy_mode` | How long each connection lasted |
| `infrarust.handshake.duration` | Histogram | seconds | none | Time spent processing the handshake |
| `infrarust.backend.connect.duration` | Histogram | seconds | `server` | Time to establish a backend connection |
| `infrarust.packets.relayed` | Counter | none | `direction` | Total packets forwarded |

## Feature gate

Telemetry is compiled behind the `telemetry` Cargo feature. If you build Infrarust without this feature, the `[telemetry]` config section is ignored and no OTel dependencies are included.

The telemetry middleware itself uses only the `tracing` crate, not `opentelemetry` directly. When no OTel subscriber is installed, spans are no-ops with roughly 2 nanoseconds of overhead per connection.

## Connecting to a collector

Any OTLP-compatible collector works. A common pattern is to run the [OpenTelemetry Collector](https://opentelemetry.io/docs/collector/) on the same host or in the same Docker network, then point Infrarust at it:

```toml
[telemetry]
enabled = true
endpoint = "http://otel-collector:4317"
```

The collector can then forward traces and metrics to whatever backend you prefer (Jaeger, Grafana Tempo, Prometheus, etc.). Infrarust sends standard OTLP; no Infrarust-specific setup is required on the collector side.
