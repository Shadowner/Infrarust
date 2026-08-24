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

./realclient.sh --skip-build               # tier C: the actual game, anchors
```

Three tiers, in increasing order of what they prove and cost:

| Tier | Client | Backend | Covers | Entry point |
|---|---|---|---|---|
| A | node-minecraft-protocol | mock | 30 versions × 11 scenarios, ~5 min | `run.sh` |
| B | node-minecraft-protocol | real Paper | anchor versions | `run.sh --backends real` |
| C | **the real Minecraft client** | real vanilla/Paper | every release 1.7.10 → latest | `realclient.sh` |

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

## What the benches changed

The point of a bench is to change something. Two proxy defects were found by
launching the real client at all 75 releases, and both are fixed:

**Packet ids are resolved by band, not by exact protocol number.** 18 releases
could not be served in any intercepted mode because their protocol number was
absent from `ProtocolVersion::SUPPORTED`. The fix adds no versions to that
table: `register()` was already computing each mapping's version range and then
discarding it, so the registry now keeps it. A number that falls between two
named versions uses the mapping in force at that point, which also means the
next patch release Mojang ships needs no change here. An explicit upper bound on
a mapping is still honoured — which is why this beat the obvious alternative of
snapping an unknown number down to the nearest known one.

**The signed encryption response is accepted.** From 1.19 a client holding a
chat profile key answers the encryption request with a salt and a signature
instead of an RSA-encrypted verify token. The proxy decoded those bytes into the
verify-token field and then RSA-decrypted them, which fails, so every player with
a real Mojang account on 1.19, 1.19.1 and 1.19.2 was dropped mid-handshake by the
one mode whose purpose is authenticating them. The signature is now verified
against the profile key from the login start packet.

Neither defect could have been found by tiers A and B. The first needs a client
for a version node-minecraft-protocol does not implement; the second needs a
client that actually holds a chat key, which a protocol reimplementation never
sends. Both were verified the same way they were found — by re-running exactly
the releases that failed:

| | before | after |
|---|---|---|
| 1.9.1 · protocol 108 | `client_only` ❌ | ✅ |
| 1.14.4 · protocol 498 | 3 red, 329 s | 10/10, 78 s |
| 1.17.1 · protocol 756 | 3 red | 10/10 |
| 1.19 · protocol 759 | 2 red | 10/10 |
| 1.18.2, 1.19.3 (controls) | 10/10 | 10/10, unchanged |

## Tier C: the real game client

Tiers A and B both drive the login with a reimplementation of the protocol. That
is the right tool for breadth, and it is not proof: it shows that *an*
implementation gets through, not that Minecraft does. Tier C launches the actual
game — Mojang's client jar, Mojang's authlib, Mojang's netty pipeline — once per
release from 1.7.10 to the latest.

```sh
./realclient.sh --skip-build                                   # anchors, ~20 min
./realclient.sh --set all                                      # all 75 releases
./realclient.sh --versions 1.7.10 --scenario direct,passthrough  # one cell
./realclient.sh --session-server http://127.0.0.1:25585        # enables client_only
./realclient.sh --donor-mc ~/.local/share/multimc              # reuse a launcher's cache
```

Needs `Xvfb` and `unzip`. No container runtime, no root, no `/etc/hosts` entry.
Everything else is downloaded into `.mccache/` on first use: client jars,
libraries, natives, assets, server jars, and Mojang's own JREs — about 15 GB for
the full sweep. `--donor-mc` hard-links assets and libraries out of an existing
launcher instead of re-fetching them.

### It is the real launcher path

`src/real/mojang.js` resolves each version exactly as the official launcher
does: apply the library `rules`, unpack the native classifiers, materialise the
asset index by hash, and pick the JRE named in the version's own `javaVersion`
block. The argument template comes from the version JSON rather than a table
here, so a release that changes its arguments is followed rather than silently
launched wrong.

Version-specific JREs matter: `jre-legacy` (8) through 1.16.5, `java-runtime-alpha`
(16) for 1.17.x, `beta`/`gamma` (17) through 1.20.4, `delta` (21) through
1.21.11, `epsilon` (25) from 26.1. The bench uses a system JVM when the major
version matches and fetches Mojang's own otherwise.

### Auto-connect, and the one boundary that matters

The client has to join without a human clicking anything, and the flag that does
it changed:

| Releases | Flag |
|---|---|
| 1.7.10 – 1.19.4 | `--server <host> --port <port>` |
| 1.20 – latest | `--quickPlayMultiplayer <host>:<port>` |

There is no overlap: `--server` was removed in the same release that added quick
play. This is pinned to observed bytes rather than to documentation, because
getting it wrong leaves every client sitting on the main menu and the whole tier
reporting nothing:

```sh
unzip -p <client>.jar net/minecraft/client/main/Main.class | strings | grep -E '^(server|port|quickPlayMultiplayer)$'
```

### A proxy per scenario, not a domain per scenario

Tier A separates scenarios by handshake domain, which works because its client
can lie about the hostname. A real client resolves what it is told to connect
to, so tier C gives each scenario its own Infrarust process on its own port,
with a single server definition matching `domains = ["*"]`. The bench therefore
needs no DNS, no hosts file and no internet.

### The verdict

Three signals, in order of what they prove:

1. **The server logs `<player> joined the game`.** Written after the player
   entity exists, so the play state is fully established, and unchanged from
   1.7.10 to today — unlike the client's own output, whose log4j layout and
   wording move around constantly. The vanilla client never prints a join line
   in *any* version, so this has to come from the server.
2. **The server says something and the client logs it.** Once the player is in,
   the bench runs `say BENCH-<player>` on the server console and waits for that
   token to appear in the client's own output. A join only proves the login got
   through; this proves the real game decoded play-state data that travelled
   through the proxy.
3. **The proxy's `/api/v1/players`**, queried *while the player is still in the
   world*. Every proxy mode registers a session; what separates the families is
   `is_active` — `false` for passthrough/zero_copy/server_only, which cannot
   inject packets, `true` for the intercepted modes. Asking after the client is
   gone samples teardown instead, and the answer then depends on which forwarder
   was used, which is how this bench first "discovered" a difference between
   passthrough and zero_copy that does not exist.

### Controls, so the matrix can go red

| Scenario | What it proves |
|---|---|
| `direct` | the client can reach a server here at all, with no proxy involved |
| `velocity-direct-refused` | the Paper backend really does enforce Velocity forwarding |
| `offline-wrong-secret` | the same path with a deliberately wrong secret is refused |

`direct` is load-bearing. When it fails for a release, that release cannot run
the game on this machine and its proxied cells are reported ⚠️ *untrusted*
rather than ❌ — blaming the proxy for a client that never started would be a
lie. The two negative controls exist because a green Velocity row means nothing
unless an unsigned or wrongly-signed payload is observed to be rejected.

### client_only with a real client

`client_only` is the mode where the proxy runs the encryption handshake and
verifies the join against a session server. Testing it with the real game needs
the game to authenticate somewhere we control, which is
[authlib-injector](https://github.com/yushijinhun/authlib-injector) attached as
a `-javaagent` and pointed at the same drasl instance the proxy checks against.
Both halves are then genuine: real RSA, real AES, real `join`/`hasJoined`.

## Findings so far

Running this bench found three things, all reproducible:

1. **The proxy could not serve 18 Minecraft releases in any intercepted mode.**
   *Fixed — see "What the benches changed" below.* Their protocol numbers (108,
   210, 315, 316, 401, 404, 480, 485, 490, 498, 575, 578, 736, 753, 756) were
   absent from `ProtocolVersion::SUPPORTED`, and the registry was keyed by exact
   protocol number, so every attempt to *send* a packet to such a client failed.
   Tier A saw six of them; the real-client sweep over all 75 releases found the
   other nine, because it does not depend on a third-party library supporting
   the version.

2. **BungeeCord/BungeeGuard forwarding does nothing in `offline` mode**, and
   against a real backend that means players cannot connect at all. Tracked as a
   known finding.

3. **`mode = "bungeecord"` and `"bungeeguard"`, as the docs spelled them, do not
   parse.** serde expects `bungee_cord` / `bungee_guard`. Fixed in the docs.

Finding 2 is proxy behaviour and is left alone here: this bench reports, it does
not decide. Finding 1 was fixed once the sweep had measured its full extent.

Tier C then added four more, all from launching the actual game:

4. **The real client gets in, and the byte-relaying modes never break.**
   `passthrough`, `zero_copy`, `server_only` and plain `offline` carry the real
   game into the world on every release tried, from 1.7.10 up, and the player
   then receives a message the server sends afterwards. The oldest version is
   also the fastest: 1.7.10 finishes its six applicable scenarios in 20 seconds.
   `client_only` and the Velocity scenarios are the ones finding 1 locks out.

5. **Paper accepts our Velocity payload, and rejects a wrong one.** With modern
   forwarding on, Paper lets the proxied player in and refuses the same path
   when the proxy signs with a different secret — *"Unable to verify player
   details"* — and refuses a direct connection outright — *"This server requires
   you to connect with Velocity."* Those two red cells are what make the green
   ones mean something.

6. **`--server` and `--quickPlayMultiplayer` do not overlap.** `--server`/`--port`
   exists from 1.7.10 to 1.19.4 and is gone in 1.20, which is exactly where
   quick play appears. Any bench that drives the real client has to switch
   modes at that release or every client sits on the main menu.

7. **Five things this tier reported before it was right.** Recorded because a
   bench nobody can audit is worth little, and because each mistake has a shape
   worth recognising again:

   | It reported | The truth | Cause |
   |---|---|---|
   | `zero_copy` registers a session, `passthrough` does not | both do, as do all modes | polled the admin API *after* killing the client, sampling teardown |
   | 1.14 Velocity broken | works | Paper 1.14 logs the login but never the `joined the game` broadcast |
   | every negative control red | they pass | asserted the proxy's view against an `undefined` expectation |
   | 1.13 Velocity broken | the backend cannot take part | Paper keeps `velocity-support` in its config without implementing it |
   | Paper 1.14–1.19 has no forwarding | it does | read the boot log, which those builds do not write |

   None of the five was proxy behaviour. Two consequences are now baked in: the
   proxy's view is read while the player is still in the world, and whether a
   Paper build really enforces Velocity is decided by the `velocity-direct-refused`
   control rather than by anything the server says about itself.
