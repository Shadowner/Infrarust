---
title: Examples & Cookbook
description: WASM plugin examples for each capability, most of them taken verbatim from the SDK test fixtures.
outline: [2, 3]
---

# Examples & Cookbook

The first examples on this page are short, self-contained plugins. The rest are the real source of SDK test fixtures, copied without changes. The fixtures live under `crates/infrarust-loader-wasm/tests/fixtures/` and compile against the same SDK you depend on, so each one is a working reference for one capability. For a from-scratch starter project, use the template in [Getting Started](./getting-started).

:::info Fixtures, not templates
The fixture crates are part of the loader's test suite. They are not published as standalone, copy-and-go templates. Read them for the patterns; build new plugins from the [Getting Started](./getting-started) layout.
:::

Each example imports `infrarust_plugin_sdk::prelude::*` and tags one impl block with `#[plugin(...)]`. The guest `Plugin` trait is synchronous: `on_enable(&self, ctx: &Context) -> Result<(), PluginError>`, and `?` works on every host call.

## hello

What it shows: the smallest valid plugin. `on_enable` runs once at load and writes a marker into the plugin's WASI-scoped data directory.

```rust
use infrarust_plugin_sdk::prelude::*;

#[derive(Default)]
struct Hello;

#[plugin(id = "hello", name = "Hello", description = "The smallest plugin")]
impl Plugin for Hello {
    fn on_enable(&self, _ctx: &Context) -> Result<(), PluginError> {
        std::fs::write("enabled.marker", "on_enable ran")?;
        Ok(())
    }
}
```

The path `enabled.marker` is relative to the preopened data directory, which the host mounts as the guest's root. See [Getting Started](./getting-started) for the full project layout and build steps.

## event-subscriber

What it shows: observing an event. The handler reads `PostLoginEvent` fields and never changes an outcome.

```rust
use infrarust_plugin_sdk::prelude::*;

#[derive(Default)]
struct EventSubscriber;

#[plugin(id = "event-subscriber", name = "Event Subscriber")]
impl Plugin for EventSubscriber {
    fn on_enable(&self, ctx: &Context) -> Result<(), PluginError> {
        ctx.on::<PostLoginEvent>(EventPriority::Normal, |event| {
            info!(
                "{} ({}) joined with protocol {}",
                event.player.username, event.player.uuid, event.protocol
            );
        })?;
        Ok(())
    }
}
```

## event-modifier

What it shows: reading and changing an event result. The first handler sends every connection to a chosen backend; the second, later one lets staff through to the server they asked for, undoing the redirect.

```rust
use infrarust_plugin_sdk::prelude::*;

#[derive(Default)]
struct EventModifier;

#[plugin(id = "event-modifier", name = "Event Modifier")]
impl Plugin for EventModifier {
    fn on_enable(&self, ctx: &Context) -> Result<(), PluginError> {
        ctx.on::<ServerPreConnectEvent>(EventPriority::Normal, |event| {
            event.redirect_to("backend-1");
        })?;
        ctx.on::<ServerPreConnectEvent>(EventPriority::Late, |event| {
            let staff = event.player.handle().has_permission("example.staff").unwrap_or(false);
            if staff && matches!(event.result(), ServerPreConnectResult::ConnectTo(_)) {
                event.allow();
            }
        })?;
        Ok(())
    }
}
```

## multi-handler

What it shows: priority ordering and per-handler cancellation. Five handlers register for the same event at different priorities; one is cancelled before any event fires.

```rust
use std::cell::RefCell;

use infrarust_plugin_sdk::prelude::*;

thread_local! {
    static ORDER: RefCell<String> = const { RefCell::new(String::new()) };
}

fn append(tag: &str) {
    ORDER.with(|order| order.borrow_mut().push_str(tag));
}

#[derive(Default)]
struct MultiHandler;

#[plugin(id = "multi-handler", name = "Multi Handler")]
impl Plugin for MultiHandler {
    fn on_enable(&self, ctx: &Context) -> Result<(), PluginError> {
        ctx.on::<PostLoginEvent>(EventPriority::First, |_| append("A"))?;
        ctx.on::<PostLoginEvent>(EventPriority::Custom(32), |_| append("B"))?;
        ctx.on::<PostLoginEvent>(EventPriority::Normal, |_| append("C"))?;
        let cancelled = ctx.on::<PostLoginEvent>(EventPriority::Normal, |_| append("L"))?;
        ctx.on::<PostLoginEvent>(EventPriority::Last, |_| append("D"))?;
        cancelled.cancel();
        Ok(())
    }
}
```

Each login appends `ABCD`: the cancelled `L` handler never runs.

## command-plugin

What it shows: registered commands with a description and a tab-completer, a command registered from inside a completer, and unregistering through the SDK. This is the `command-plugin` fixture.

```rust
use infrarust_plugin_sdk::prelude::*;

#[derive(Default)]
struct CommandPlugin;

#[plugin(id = "command-plugin", name = "Command Plugin Fixture")]
impl Plugin for CommandPlugin {
    fn on_enable(&self, ctx: &Context) -> Result<(), PluginError> {
        ctx.command("greet")
            .description("Greets the caller")
            .handler(|invocation| {
                let _ = std::fs::write("command.marker", invocation.args.join(","));
            })
            .completer(|completion| {
                let partial = completion.partial();
                ["world", "everyone", "friend"]
                    .into_iter()
                    .filter(|candidate| candidate.starts_with(partial))
                    .collect::<Vec<_>>()
            })
            .register()?;
        ctx.command("nest")
            .description("Registers `nested` from inside its own completer")
            .completer(|_| {
                let _ = Context::new()
                    .command("nested")
                    .handler(|_| {
                        let _ = std::fs::write("nested.marker", "ran");
                    })
                    .completer(|_| vec!["inner"])
                    .register();
                vec!["registered"]
            })
            .register()?;
        ctx.command("unnest")
            .description("Unregisters `nested` through the SDK")
            .handler(|_| {
                let removed = Context::new().unregister_command("nested").unwrap_or(false);
                let _ = std::fs::write("unnest.marker", removed.to_string());
            })
            .register()?;
        Ok(())
    }
}
```

When another plugin already owns `nested`, the registration from the completer is refused and the guest keeps no handler for it, so `unnest` records `false`.

## codec-modify

What it shows: the three packet verdicts in one filter. Drop a packet, mutate its payload, or inject extra packets around it. This is the `codec-modify` fixture; it needs the `codec-filter` capability.

```rust
use infrarust_plugin_sdk::prelude::*;

#[derive(Default)]
struct CodecModify;

struct OpFilter;

impl CodecFilter for OpFilter {
    fn filter(
        &mut self,
        _ctx: &CodecContext,
        packet: &mut Packet,
        out: &mut Injections,
    ) -> Verdict {
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
    fn on_enable(&self, _ctx: &Context) -> Result<(), PluginError> {
        Ok(())
    }

    fn register_codec_filters(reg: &mut CodecRegistrar) {
        reg.add("ops", FilterPriority::Normal, |_init| Box::new(OpFilter));
    }
}
```

## codec-stateful

What it shows: per-connection filter state. Each filter instance owns a counter; the host drives the client-side and server-side instances separately to prove their state is independent. This is the `codec-stateful` fixture.

```rust
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
    fn on_enable(&self, _ctx: &Context) -> Result<(), PluginError> {
        Ok(())
    }

    fn register_codec_filters(reg: &mut CodecRegistrar) {
        reg.add("counter", FilterPriority::Normal, |_init| {
            Box::new(Counter { count: 0 })
        });
    }
}
```

## limbo-handler

What it shows: holding a player in limbo, completing on a command, timing out, and completing from a scheduled closure after the dispatch returns. This is the `limbo-handler` fixture; it registers four handlers to cover each case and needs the `limbo` capability.

```rust
use std::cell::RefCell;
use std::collections::HashSet;
use std::time::Duration;

use infrarust_plugin_sdk::prelude::*;

#[derive(Default)]
struct LimboPlugin;

struct Gate {
    waiting: RefCell<HashSet<PlayerId>>,
}

impl LimboHandler for Gate {
    fn on_player_enter(&self, session: &LimboSession) -> HandlerOutcome {
        self.waiting.borrow_mut().insert(session.player_id());
        session.send_message("Type /continue to proceed").ok();
        HandlerOutcome::Hold
    }

    fn on_command(&self, session: &LimboSession, command: &str, _args: &[String]) {
        match command {
            "continue" => {
                self.waiting.borrow_mut().remove(&session.player_id());
                session.complete(HandlerOutcome::Accept).ok();
            }
            "redirect" => {
                self.waiting.borrow_mut().remove(&session.player_id());
                session
                    .complete(HandlerOutcome::Redirect("hub".into()))
                    .ok();
            }
            _ => {
                session.send_message("Unknown command").ok();
            }
        }
    }

    fn on_chat(&self, session: &LimboSession, _message: &str) {
        session.send_message("Please use /continue").ok();
    }

    fn on_disconnect(&self, player: PlayerId) {
        self.waiting.borrow_mut().remove(&player);
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
        session.send_message("Type /continue within 5s").ok();
        HandlerOutcome::HoldWithTimeout {
            after: Duration::from_secs(5),
            on_timeout: TimeoutOutcome::Deny(Component::text("Timed out")),
        }
    }

    fn on_command(&self, session: &LimboSession, command: &str, _args: &[String]) {
        if command == "continue" {
            session.complete(HandlerOutcome::Accept).ok();
        }
    }

    fn on_session_end(&self, _player: PlayerId, _reason: SessionEndReason) {}
}

struct DelayedGate;

impl LimboHandler for DelayedGate {
    fn on_player_enter(&self, session: &LimboSession) -> HandlerOutcome {
        let handle = session.handle();
        let scheduled = Context::new().delay(Duration::from_millis(50), move || {
            if !handle.cancelled() {
                handle.complete(HandlerOutcome::Accept).ok();
            }
        });
        match scheduled {
            Ok(_) => HandlerOutcome::Hold,
            Err(_) => HandlerOutcome::Accept,
        }
    }
}

#[plugin(id = "limbo-handler", name = "Limbo Handler Fixture")]
impl Plugin for LimboPlugin {
    fn on_enable(&self, _ctx: &Context) -> Result<(), PluginError> {
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

`DelayedGate` accepts the player at once when the task cannot be scheduled, so nobody is held with nothing to release them.

## host-caller

What it shows: calling read-only host services from `on_enable`. This is the `host-caller` fixture; it queries the player count and a config value.

```rust
use infrarust_plugin_sdk::prelude::*;

#[derive(Default)]
struct HostCaller;

#[plugin(id = "host-caller", name = "Host Caller Fixture")]
impl Plugin for HostCaller {
    fn on_enable(&self, _ctx: &Context) -> Result<(), PluginError> {
        std::fs::write("count.txt", Players::count().to_string())?;
        if let Ok(Some(greeting)) = Config::get("greeting") {
            std::fs::write("greeting.txt", greeting)?;
        }
        Ok(())
    }
}
```

`Players::count()` answers `0` when the plugin lacks `player-read`; `Config::get` returns an error of kind `PermissionDenied` when it lacks `config-read`, which the fixture treats like a missing value. `greeting` is a key of the mock config service the fixture runs against; on a real proxy, `Config::get` takes a dotted path of the running proxy config, such as `bind` or `keepalive.retries`. The full set of host services is in [Services](./services).

## net-probe

What it shows: outbound TCP and UDP with `std::net`, HTTP through `wasi:http`, and files in a mounted folder with `std::fs`, all limited by the `network` and `filesystem-extended` config. A short, commented plugin doing the same is on [Network & Extra Folders](./network#example). The `net-probe` fixture runs each call on command so the loader tests can check what gets through and what is refused; read `crates/infrarust-loader-wasm/tests/fixtures/net-probe/src/lib.rs` for the HTTP body loop and the error reporting.

## See also

- [Getting Started](./getting-started): the project layout and build steps.
- [Events](./events), [Commands](./commands), [Codec Filters](./codec-filters), [Limbo](./limbo), [Services](./services): the features these examples exercise.
- [Migrating to 0.3](./migration-0.3): how the 0.2.3 versions of these examples changed.
