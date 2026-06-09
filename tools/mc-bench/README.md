# mc-bench

End-to-end Minecraft Play-state load generator and mock backend for benchmarking
Infrarust (Layer D of the benchmark suite). It measures how long a packet takes to
traverse the proxy in intercepted mode under sustained concurrent load, and the
proxy-added latency delta versus talking to a backend directly.

The tool targets protocol versions below 1.20.2 (764) on purpose: for those versions
login transitions straight to Play with no config phase, which keeps the measured path
minimal. Protocols 764 and newer are rejected.

## How it works

The benchmark ping is an opaque Play packet (serverbound id `0x40`) carrying an 8-byte
big-endian token. The mock backend echoes it back unchanged as clientbound id `0x41`.
Both ids are high enough that Infrarust forwards them opaquely with no interception, so
the round trip measures proxy traversal rather than any special handling.

The load generator is open-loop. Each worker sends pings on a fixed wall-clock grid via
`sleep_until`, so the offered load stays constant no matter how the proxy responds. The
generator never waits for an echo before sending the next ping, which is what sidesteps
the closed-loop coordinated-omission trap. Latency is the time from a ping's actual send
to its echo, measured on one shared clock, so the reported numbers reflect proxy and
backend service time rather than the generator's own timer granularity.

## Build

```sh
cargo build -p infrarust-mc-bench --release
# binary: target/release/mc-bench
```

## Subcommand: `serve-backend`

A minimal mock backend that accepts a login and echoes benchmark pings. Per connection
it reads the Handshake and LoginStart, optionally sends `CSetCompression`, sends
`CLoginSuccess`, then in Play echoes every ping (id `0x40`, 8-byte payload) back as id
`0x41`. Other serverbound packets are ignored.

| Flag | Default | Description |
| --- | --- | --- |
| `--host <HOST>` | `127.0.0.1` | Bind address. |
| `--port <PORT>` | `25566` | Bind port. |
| `--compression <i32>` | off | If set, send `CSetCompression <threshold>` before `CLoginSuccess` and use the compressed framing afterward. |

```sh
mc-bench serve-backend --port 25566
```

## Subcommand: `load`

Open-loop Play-state load generator. Per worker it connects (TCP_NODELAY), sends
Handshake (`next_state = Login`) and LoginStart (`bench_<i>`, offline UUID), reads
clientbound frames until `CLoginSuccess` (handling `CSetCompression`), then enters Play
and sends pings at the configured rate while concurrently recording echoes.

| Flag | Default | Description |
| --- | --- | --- |
| `--host <HOST>` | `127.0.0.1` | Target host (the proxy's port, or the backend's for the baseline). |
| `--port <PORT>` | `25566` | Target port. |
| `--server-address <DOMAIN>` | `127.0.0.1` | Hostname put in the Handshake so the proxy routes correctly. Must match a `domains` entry in the proxy's server config. |
| `--protocol <i32>` | `758` (1.18.2) | Protocol to advertise. Must be below 764. |
| `--concurrency <N>` | `100` | Concurrent worker connections. |
| `--rate <hz>` | `20` | Pings per second, per connection. |
| `--duration <secs>` | `30` | Measurement window (after warmup). |
| `--warmup <secs>` | `5` | Warmup window (not recorded). |

Output: connection success/failure counts, pings sent, echoes, throughput (echoes/sec),
and mean plus p50/p90/p99/p99.9/max latency in microseconds, aggregated across all
workers into one HDR histogram.

## Self-test (no proxy)

Run the generator directly against the mock backend to confirm the tool works:

```sh
# terminal 1
cargo run -p infrarust-mc-bench --release -- serve-backend --port 25566

# terminal 2
cargo run -p infrarust-mc-bench --release -- \
    load --host 127.0.0.1 --port 25566 --concurrency 20 --rate 20 --duration 8 --warmup 2
```

All 20 connections should log in, 3200 pings should be sent and echoed, and loopback
latency should land in the low hundreds of microseconds. A run on a dev loopback (your
hardware will differ):

```
connections:   20 ok, 0 failed
pings sent:     3200
echoes:         3200
throughput:     400 echoes/sec
latency (microseconds):
  mean        127.9
  p50           120
  p90           178
  p99           375
  p99.9         673
  max           755
```

## Measuring proxy-added latency

Run the same `load` invocation against three targets and compare the percentiles. The
difference between (b) and (a) is the proxy-added latency.

### (a) Direct to the backend (baseline)

```sh
mc-bench serve-backend --port 25566
mc-bench load --host 127.0.0.1 --port 25566 --concurrency 100 --rate 20 --duration 30 --warmup 5
```

### (b) Through Infrarust in offline mode

Use the config under [`example-config/`](./example-config):

- [`example-config/infrarust.toml`](./example-config/infrarust.toml) binds the proxy to `127.0.0.1:25565`.
- [`example-config/servers/bench.toml`](./example-config/servers/bench.toml) routes domain `127.0.0.1` to backend `127.0.0.1:25566` in offline mode.

```toml
# example-config/servers/bench.toml
domains    = ["127.0.0.1"]          # must match --server-address
addresses  = ["127.0.0.1:25566"]    # the mock backend
proxy_mode = "offline"
```

```sh
# terminal 1: mock backend
mc-bench serve-backend --port 25566

# terminal 2: proxy (run from tools/mc-bench/example-config)
infrarust --config-path infrarust.toml

# terminal 3: load through the proxy (port 25565, the proxy's bind)
mc-bench load --host 127.0.0.1 --port 25565 --server-address 127.0.0.1 \
    --concurrency 100 --rate 20 --duration 30 --warmup 5
```

`--server-address 127.0.0.1` must match the `domains` entry so the proxy routes to the
mock backend.

### (c) Optional: passthrough mode

Set `proxy_mode = "passthrough"` in `bench.toml` to measure the proxy's transparent
TCP-relay path (no login interception). Re-run the same load command from (b).
