# End-to-end connection bench

Drives a real Minecraft client through a live Infrarust process into a real
Minecraft server, for every client version from 1.7.10 to the latest, in every
proxy mode and every forwarding mode.

Nothing else in the repo does this. The ~1500 unit and integration tests stop at
the middleware and bridge level; `tools/mc-bench` speaks the protocol but reuses
Infrarust's own codec and is capped below protocol 764. This bench is the only
thing that answers "does a 1.10.2 client actually get into a server through the
proxy, and does Paper accept the Velocity payload we sign?"

It is meant to be run by hand on a machine you own, not on hosted CI runners.

## Running it

```sh
./run.sh                                   # whole matrix, mock backends (~5 min)
./run.sh --skip-build                      # reuse target/release/infrarust
./run.sh --version 1.20.1 --scenario offline-velocity   # one cell, for debugging
```

Exit codes: `0` everything as expected, `1` a case failed, `2` a known finding
stopped reproducing and the baseline needs updating.

Reports land in `reports/report.md` (a version × scenario grid) and
`reports/report.json` (every observation, for diffing between runs).

## How it works

### One proxy, one domain per scenario

`proxy_mode` and `forwarding_mode` are per-server settings and routing is by
handshake domain, so the whole matrix runs against a **single** proxy process:
each scenario is one `servers/*.toml` with its own domain, and the client picks
which one it wants through `fakeHost`. No restarts, no config reloads.

`src/lab.js` renders that config into a throwaway directory on every run, so a
stale file can never make a scenario quietly test the wrong thing.

One subtlety it encodes: a per-server `forwarding_mode = "velocity"` reads its
secret from the **global** mode, and silently degrades to no forwarding when the
global mode is not velocity (`services/proxy.rs`, `config_to_core_mode`). The lab
therefore sets velocity globally and overrides per server.

### An independent client and an independent backend

Both ends are [node-minecraft-protocol][nmp], an implementation with no shared
code with Infrarust. That independence is the point: a bench built on
Infrarust's own codec would share any bug in it and report success.

All 30 versions the library supports drive a full login through to the play
state, which is what makes a single harness able to cover 1.7.10 through 26.1.

[nmp]: https://github.com/PrismarineJS/node-minecraft-protocol

### Three assertion channels

A case only passes if all three agree:

| Channel | What it proves |
|---|---|
| Client (`src/client.js`) | reached the play state, negotiated compression/encryption, was not kicked |
| Backend (`src/backend-mock.js`) | what the proxy actually sent: handshake contents, and for Velocity the signed payload |
| Proxy (`/api/v1/players`) | the proxy's own view — username, UUID, backend, protocol, and whether the session is intercepted |

The Velocity check is the strict one. The mock backend does what Paper does: it
sends a `login_plugin_request` on `velocity:player_info`, then recomputes
`HMAC-SHA256(secret, payload)` and compares it against the 32 bytes the proxy
prefixed, before decoding the payload and checking the forwarded identity
matches the player who connected. A wrong secret, a mangled field order or an
unsigned response all fail loudly.

Getting the ordering right needed care: nmp writes `success` synchronously right
after its own `login_start` handler, so hooking `beforeLogin` would leave the
proxy's response arriving after the connection had left the login state, and
would desync compression. The mock instead takes nmp's `login_start` listener,
runs the Velocity exchange, and replays the listener once the proxy has
answered — which is exactly Paper's ordering.

### Known findings

`src/expectations.js` holds deviations that are current, understood proxy
behaviour. They render as ⚠️ rather than ❌ so a run stays green, and they are
described in full in the report.

The list cannot rot into a way of hiding failures: if a known finding stops
reproducing, the run reports it as 🎉 and exits `2`, asking for the entry to be
removed.

## Scenarios

| Id | `proxy_mode` | `forwarding_mode` | Notes |
|---|---|---|---|
| `passthrough` | passthrough | none | |
| `zerocopy` | zero_copy | none | Linux only |
| `server-only` | server_only | none | byte-for-byte the same path as passthrough |
| `offline` | offline | none | |
| `offline-velocity` | offline | velocity | 1.13+; the proxy completes login itself first |
| `offline-bungeecord` | offline | bungee_cord | |
| `offline-bungeeguard` | offline | bungee_guard | |
| `client-only` | client_only | none | needs `--session-server` |
| `client-only-velocity` | client_only | velocity | needs `--session-server` |
| `passthrough-velocity-fallback` | passthrough | velocity | asserts the documented fallback to BungeeCord |
| `full-fallback` | full | none | asserts `full` still falls back to passthrough |

Velocity forwarding cannot be negotiated below protocol 393, because
`login_plugin_request` does not exist before 1.13. Those cells are skipped, not
failed.

## client_only and the session server

`client_only` verifies the player against a session server, so it needs one to
exist. `--session-server <base-url>` points both halves at the same place: the
client's join call and the proxy's `hasJoined` lookup.

The proxy side is configurable because of `[auth] session_url`, added alongside
this bench. `MojangAuth::with_session_url()` already existed and was documented
"for testing", but nothing reached it — `server.rs` always called
`MojangAuth::new()` with Mojang's URL hardcoded. The same setting is what lets
Infrarust run against authlib-injector deployments.

Without `--session-server`, those two scenarios are skipped rather than failed,
so the bench still runs with no network at all.

## Tier B: real Java servers

Tier A proves the wire protocol. It cannot prove that a real Paper server accepts
what we sign, because there the mock is the thing checking the signature.

```sh
podman compose --profile auth up -d     # drasl, only needed for client_only
./tierb.sh v1_21_4                      # starts the backends and waits for them
./run.sh --backends real --version 1.21.4 --session-server http://127.0.0.1:25585
podman compose --profile v1_21_4 down -v
```

`--backends real` starts nothing: it expects Java servers already listening on
the scenario ports. Checks that read the backend's internals are dropped rather
than failed, because that channel does not exist against a server we do not
control. What replaces them is stronger — Paper verifies the Velocity HMAC with
its own implementation and its own copy of the secret, and refuses direct
connections entirely, so a player reaching the world *is* the proof.

Some hard-won details are baked into `tierb.sh` and `docker-compose.yml`:

- There is **no `VELOCITY_*` or `SPIGOT_BUNGEECORD` environment variable** in the
  image. Both settings are staged config files, and the Velocity secret is read
  out of `src/lab.js` so it cannot drift from the proxy's.
- Paper moved its config at **1.19**: `paper.yml` before, `config/paper-global.yml`
  after. Both forms are in `fixtures/paper/`.
- Mounting the whole `config/` directory shadows the world-defaults file Paper
  generates beside it and crashes the server during world load.
- Under rootless podman the host user maps to container root while the server
  runs unprivileged, so staged config has to be writable by that user.
- Bukkit-derived servers refuse repeat connections from one IP within
  `connection-throttle` (4000 ms by default), so `--backends real` paces itself.
  Override with `--case-delay`.

## Findings so far

Running this bench found three things, all reproducible:

1. **The proxy cannot serve six Minecraft versions in any intercepted mode.**
   1.10.2, 1.11.2, 1.13.2, 1.14.4, 1.15.2 and 1.17.1 have protocol numbers
   (210, 316, 404, 498, 578, 756) that are absent from `ProtocolVersion::SUPPORTED`.
   The registry is keyed by exact protocol number, so every attempt to *send* a
   packet to such a client fails with `no packet ID for … in login/ProtocolVersion(N)`.
   All 14 failures in the full matrix are this one cause.

2. **BungeeCord/BungeeGuard forwarding does nothing in `offline` mode**, and
   against a real backend that means players cannot connect at all. Tracked as a
   known finding.

3. **`mode = "bungeecord"` and `"bungeeguard"`, as the docs spelled them, do not
   parse.** serde expects `bungee_cord` / `bungee_guard`. Fixed in the docs.

The first two are proxy behaviour and are left alone here: this bench reports,
it does not decide.
