---
title: Examples & Cookbook
description: Compiling WASM plugin examples for each capability, taken verbatim from the SDK test fixtures.
outline: [2, 3]
---

# Examples & Cookbook

Every example on this page is the real source of an SDK test fixture, copied without changes. The fixtures live under `crates/infrarust-loader-wasm/tests/fixtures/` and compile against the same SDK you depend on, so each one is a working reference for one capability. For a from-scratch starter project, use the template in [Getting Started](./getting-started).

:::info Fixtures, not templates
These crates are part of the loader's test suite. They are not yet published as standalone, copy-and-go templates. Read them for the patterns; build new plugins from the [Getting Started](./getting-started) layout.
:::

Each fixture imports `infrarust_plugin_sdk::prelude::*` and tags one impl block with `#[plugin(...)]`. The guest `Plugin` trait is synchronous: `on_enable(&self, ctx: &Context) -> Result<(), String>`.

## hello

What it shows: the smallest valid plugin. `on_enable` runs once at load and writes a marker into the plugin's WASI-scoped data directory.

```rust
//! WASM-1 `hello` fixture, migrated to the SDK. Writes a marker into its
//! WASI-scoped data_dir so the host test can confirm `on_enable` ran.

use infrarust_plugin_sdk::prelude::*;

#[derive(Default)]
struct Hello;

#[plugin(id = "hello", name = "Hello Fixture", description = "WASM-1 hello fixture")]
impl Plugin for Hello {
    fn on_enable(&self, _ctx: &Context) -> Result<(), String> {
        // The host preopens data_dir as the guest root; this lands in plugins/hello/.
        std::fs::write("enabled.marker", b"on_enable ran").map_err(|e| e.to_string())
    }
}
```

The path `enabled.marker` is relative to the preopened data directory, which the host mounts as the guest's root. See [Getting Started](./getting-started) for the full project layout and build steps.

## event-subscriber

What it shows: observing an event. The handler reads `PostLoginEvent` fields and never changes the outcome.

```rust
//! WASM-2 `event-subscriber` fixture, migrated to the SDK. Records the joining
//! player's username on `PostLogin` so the host test can confirm delivery.

use infrarust_plugin_sdk::prelude::*;

#[derive(Default)]
struct EventSubscriber;

#[plugin(id = "event-subscriber", name = "Event Subscriber Fixture")]
impl Plugin for EventSubscriber {
    fn on_enable(&self, ctx: &Context) -> Result<(), String> {
        ctx.on::<PostLoginEvent>(EventPriority::Normal, |event| {
            let _ = std::fs::write("post-login.marker", event.profile.username.as_bytes());
        });
        Ok(())
    }
}
```

`ctx.on::<E>(priority, closure)` registers the handler. The closure parameter type picks the event. The [Events](./events) page lists every event the SDK exposes.

## event-modifier

What it shows: changing an event outcome. The handler calls `redirect_to` on `ServerPreConnectEvent` to send every connection to a chosen backend.

```rust
//! WASM-2 `event-modifier` fixture, migrated to the SDK. Redirects every
//! connection to "backend-1" so the host test can confirm the outcome applies.

use infrarust_plugin_sdk::prelude::*;

#[derive(Default)]
struct EventModifier;

#[plugin(id = "event-modifier", name = "Event Modifier Fixture")]
impl Plugin for EventModifier {
    fn on_enable(&self, ctx: &Context) -> Result<(), String> {
        ctx.on::<ServerPreConnectEvent>(EventPriority::Normal, |event| {
            event.redirect_to("backend-1");
        });
        Ok(())
    }
}
```

`redirect_to` takes the target server id. Which events carry an outcome, and which methods set it, is in [Events](./events).

## multi-handler

What it shows: priority ordering and per-handler cancellation. Five handlers register for the same event at different priorities; one is cancelled before any event fires.

```rust
//! v0.2-B `multi-handler` fixture. Registers several `PostLogin` handlers at
//! different priorities cancels one before any event
//! fires, and appends each handler's tag to a marker file. The host test asserts
//! priority ordering, multi-handler delivery, and independent cancellation.

use std::io::Write;

use infrarust_plugin_sdk::prelude::*;

#[derive(Default)]
struct MultiHandler;

#[plugin(id = "multi-handler", name = "Multi Handler Fixture")]
impl Plugin for MultiHandler {
    fn on_enable(&self, ctx: &Context) -> Result<(), String> {
        ctx.on::<PostLoginEvent>(EventPriority::First, |_| append("A"));
        ctx.on::<PostLoginEvent>(EventPriority::Custom(32), |_| append("B"));
        ctx.on::<PostLoginEvent>(EventPriority::Normal, |_| append("C"));
        let leaked = ctx.on::<PostLoginEvent>(EventPriority::Normal, |_| append("L"));
        ctx.on::<PostLoginEvent>(EventPriority::Last, |_| append("D"));
        leaked.cancel();
        Ok(())
    }
}

fn append(tag: &str) {
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("multi.marker")
    {
        let _ = f.write_all(tag.as_bytes());
    }
}
```

`ctx.on` returns an `EventSubscription`. Calling `.cancel()` on the handle removes that one handler; the others keep running. `EventPriority::Custom(u8)` sets an arbitrary priority between the named ones. The [Events](./events) page documents the priority scale.

## command-plugin

What it shows: a registered command with a description and a tab-completer.

```rust
//! WASM-2 `command-plugin` fixture, migrated to the SDK. Registers `greet` and
//! writes its args to the data_dir on invocation so the host test can confirm
//! the command dispatch reaches the guest.

use infrarust_plugin_sdk::prelude::*;

#[derive(Default)]
struct CommandPlugin;

#[plugin(id = "command-plugin", name = "Command Plugin Fixture")]
impl Plugin for CommandPlugin {
    fn on_enable(&self, ctx: &Context) -> Result<(), String> {
        ctx.command("greet", |invocation| {
            let _ = std::fs::write("command.marker", invocation.args.join(",").as_bytes());
        })
        .description("Greets the caller")
        .completer(|partial, _cursor| {
            let p = partial.last().map(String::as_str).unwrap_or("");
            ["world", "everyone", "friend"]
                .iter()
                .filter(|c| c.starts_with(p))
                .map(|c| (*c).to_string())
                .collect()
        })
        .register();
        Ok(())
    }
}
```

`ctx.command(name, handler)` returns a builder. Chain `.description(...)` and `.completer(...)`, then call `.register()` to install it. The handler receives a `CommandInvocation` whose `args` field holds the parsed arguments. The completer gets the partial argument list plus a cursor and returns the matching suggestions. Aliases and the full builder surface are in [Commands](./commands).

## codec-modify

What it shows: the three packet verdicts in one filter. Drop a packet, mutate its payload, or inject extra packets around it.

```rust
use infrarust_plugin_sdk::prelude::*;

#[derive(Default)]
struct CodecModify;

struct OpFilter;

impl CodecFilter for OpFilter {
    fn filter(&mut self, _ctx: &CodecContext, packet: &mut Packet, out: &mut Injections) -> Verdict {
        match packet.id() {
            0x01 => Verdict::Drop,
            0x02 => {
                packet.set_data(b"MODIFIED".to_vec());
                Verdict::Pass
            }
            0x03 => {
                out.before(Packet::new(0xfe, b"before".to_vec()));
                out.after(Packet::new(0xff, b"after".to_vec()));
                Verdict::Pass
            }
            _ => Verdict::Pass,
        }
    }
}

#[plugin(id = "codec-modify", name = "Codec Modify Fixture")]
impl Plugin for CodecModify {
    fn on_enable(&self, _ctx: &Context) -> Result<(), String> {
        Ok(())
    }

    fn register_codec_filters(reg: &mut CodecRegistrar) {
        reg.add("ops", FilterPriority::Normal, |_init| Box::new(OpFilter));
    }
}
```

Codec filters are registered from the `register_codec_filters` associated function, not from `on_enable`. `reg.add(name, priority, factory)` installs a factory that builds one filter per connection. `Verdict::Drop` removes the packet, `packet.set_data(...)` rewrites its payload, and `out.before` / `out.after` queue injected packets on either side. The `codec-filter` capability is opt-in; declare it in the plugin's TOML permissions. See [Codec Filters](./codec-filters).

## codec-stateful

What it shows: per-connection filter state. Each filter instance owns a counter; the host drives the client-side and server-side instances separately to prove their state is independent.

```rust
//! Codec fixture with per-connection state: counts the packets it sees and writes
//! the running count into each payload. The host test drives the client-side and
//! server-side instances separately to prove their state is independent.

use infrarust_plugin_sdk::prelude::*;

#[derive(Default)]
struct CodecStateful;

struct Counter {
    count: u32,
}

impl CodecFilter for Counter {
    fn filter(
        &mut self,
        _ctx: &CodecContext,
        packet: &mut Packet,
        _out: &mut Injections,
    ) -> Verdict {
        self.count += 1;
        packet.set_data(self.count.to_le_bytes().to_vec());
        Verdict::Pass
    }
}

#[plugin(id = "codec-stateful", name = "Codec Stateful Fixture")]
impl Plugin for CodecStateful {
    fn on_enable(&self, _ctx: &Context) -> Result<(), String> {
        Ok(())
    }

    fn register_codec_filters(reg: &mut CodecRegistrar) {
        reg.add("counter", FilterPriority::Normal, |_init| {
            Box::new(Counter { count: 0 })
        });
    }
}
```

The factory closure builds a fresh `Counter` per connection, so `&mut self` inside `filter` mutates only that connection's state. State on a filter never leaks across connections. More on the connection model is in [Codec Filters](./codec-filters).

## limbo-handler

What it shows: holding a player in limbo, completing on a command, timing out, and completing from a scheduled closure after the dispatch returns. This fixture registers four handlers to cover each case.

The `limbo` capability is opt-in and must be listed in the plugin's TOML permissions. See [Limbo](./limbo) for the session model and outcomes.

:::details Full limbo-handler source
```rust
use std::cell::RefCell;
use std::collections::HashSet;
use std::time::Duration;

use infrarust_plugin_sdk::prelude::*;

#[derive(Default)]
struct LimboPlugin;

struct Gate {
    waiting: RefCell<HashSet<u64>>,
}

impl LimboHandler for Gate {
    fn on_player_enter(&self, session: &LimboSession) -> HandlerOutcome {
        self.waiting.borrow_mut().insert(session.player_id());
        session
            .send_message(Component::text("Type /continue to proceed"))
            .ok();
        HandlerOutcome::Hold
    }

    fn on_command(&self, session: &LimboSession, command: &str, _args: &[String]) {
        match command {
            "continue" => {
                self.waiting.borrow_mut().remove(&session.player_id());
                session.complete(HandlerOutcome::Accept);
            }
            "redirect" => {
                self.waiting.borrow_mut().remove(&session.player_id());
                session.complete(HandlerOutcome::Redirect("hub".to_string()));
            }
            _ => {
                session.send_message(Component::text("Unknown command")).ok();
            }
        }
    }

    fn on_chat(&self, session: &LimboSession, _message: &str) {
        session
            .send_message(Component::text("Please use /continue"))
            .ok();
    }

    fn on_disconnect(&self, player_id: u64) {
        self.waiting.borrow_mut().remove(&player_id);
    }
}

struct Boom;

impl LimboHandler for Boom {
    fn on_player_enter(&self, _session: &LimboSession) -> HandlerOutcome {
        panic!("boom: this handler always traps");
    }
}

struct TimedGate;

impl LimboHandler for TimedGate {
    fn on_player_enter(&self, session: &LimboSession) -> HandlerOutcome {
        session
            .send_message(Component::text("Type /continue within 5s"))
            .ok();
        HandlerOutcome::HoldWithTimeout {
            after: Duration::from_secs(5),
            on_timeout: TimeoutOutcome::Deny(Component::text("Timed out")),
        }
    }

    fn on_command(&self, session: &LimboSession, command: &str, _args: &[String]) {
        if command == "continue" {
            session.complete(HandlerOutcome::Accept);
        }
    }

    fn on_session_end(&self, _player_id: u64, _reason: SessionEndReason) {
        // Engine owns the timeout; nothing to tear down here (demo of the hook).
    }
}

struct DelayedGate;

impl LimboHandler for DelayedGate {
    fn on_player_enter(&self, session: &LimboSession) -> HandlerOutcome {
        // The handle outlives this dispatch (moved into the scheduled closure).
        let handle = session.handle();
        Context::new().delay(Duration::from_millis(50), move || {
            if !handle.cancelled() {
                handle.complete(HandlerOutcome::Accept);
            }
        });
        HandlerOutcome::Hold
    }
}

#[plugin(id = "limbo-handler", name = "Limbo Handler Fixture")]
impl Plugin for LimboPlugin {
    fn on_enable(&self, _ctx: &Context) -> Result<(), String> {
        Ok(())
    }

    fn register_limbo_handlers(reg: &mut LimboRegistrar) {
        reg.add(
            "gate",
            Gate {
                waiting: RefCell::new(HashSet::new()),
            },
        );
        reg.add("boom", Boom);
        reg.add("timed-gate", TimedGate);
        reg.add("delayed-gate", DelayedGate);
    }
}
```
:::

The four handlers map to four patterns:

| Handler | Pattern |
|---------|---------|
| `Gate` | `on_player_enter` returns `HandlerOutcome::Hold`; a later `on_command` calls `session.complete(...)` with `Accept` or `Redirect`. |
| `Boom` | The handler panics. The host fails the session closed when a limbo handler traps. |
| `TimedGate` | `on_player_enter` returns `HandlerOutcome::HoldWithTimeout`; the engine denies with `TimeoutOutcome::Deny` if no `complete` arrives in time. |
| `DelayedGate` | `session.handle()` produces a `SessionHandle` that outlives the dispatch. A scheduled closure checks `handle.cancelled()` before calling `handle.complete(...)`. |

`register_limbo_handlers` installs the handlers by name. Outcomes, the timeout model, and the session handle lifecycle are documented in [Limbo](./limbo).

## host-caller

What it shows: calling read-only host services from `on_enable`. This fixture queries the player count and a config value.

```rust
//! WASM-2 `host-caller` fixture, migrated to the SDK. Calls read-only host
//! services in `on_enable` and records the results so the host test can assert.

use infrarust_plugin_sdk::prelude::*;

#[derive(Default)]
struct HostCaller;

#[plugin(id = "host-caller", name = "Host Caller Fixture")]
impl Plugin for HostCaller {
    fn on_enable(&self, _ctx: &Context) -> Result<(), String> {
        std::fs::write("count.txt", Players.online_count().to_string().as_bytes())
            .map_err(|e| e.to_string())?;
        if let Some(greeting) = Config.get("greeting") {
            std::fs::write("greeting.txt", greeting.as_bytes()).map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}
```

`Players` and `Config` are zero-sized service handles from the prelude. `Players.online_count()` returns the current player count, and `Config.get(key)` returns an `Option<String>` for a config value. The full set of host services is in [Services](./services).

## See also

- [Getting Started](./getting-started): the from-scratch project template and build steps.
- [Events](./events): every event the SDK exposes, priorities, and outcomes.
- [Commands](./commands): the command builder, aliases, and tab-complete.
- [Codec Filters](./codec-filters): the per-connection filter model and verdicts.
- [Limbo](./limbo): session handling, outcomes, and timeouts.
- [Services](./services): read and write host services available to plugins.
