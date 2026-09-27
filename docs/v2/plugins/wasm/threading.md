---
title: Threading and Concurrency
description: How the host runs a WASM plugin. One actor and one call at a time, the call queue, what the serial queue costs and how to watch it, no re-entry and the call-chain guard, calls that wait on a player's own session, deadlines and host-call timeouts, why guest state needs no Send or Sync, and why WASM threads are not supported.
outline: [2, 3]
---

# Threading and Concurrency

A WASM plugin never runs on two threads at once and is never re-entered. The host drives each plugin instance from a single task and hands it one call at a time, each run to completion before the next starts. Keep plugin state in `Cell`, `RefCell` and `Rc`: you need no `Send`, no `Sync`, no locks and no atomics, and you cannot start threads of your own.

The rest of this page explains how the host gets there, what it costs, and the few places where it shows through.

## One actor per plugin

When the host loads a plugin, it starts one tokio task for it, the plugin's **actor**. The actor owns the plugin's wasmtime store: the instance, its linear memory and its WASI context. Nothing else touches that store.

Every call into the plugin is a job sent to the actor: `on_enable` and `on_disable`, each event handler, each command and tab completion, each scheduled task, each limbo callback, and each question to a [ban provider](./bans) or [permission provider](./permissions). The actor takes the jobs from its queue in the order they arrived and runs them one at a time.

```text
 player sessions   event queue   console   scheduler      many proxy tasks, any worker thread
       │ event         │ event      │ command   │ task
       ▼               ▼            ▼           ▼
 ┌──────────────────────────────────────────────────┐
 │ the plugin's queue ([wasm] queue_capacity, 1024) │
 └──────────────────────────────────────────────────┘
                          │ one job at a time, in order
                          ▼
            actor task ── owns the store ── runs the guest
```

The proxy around the plugin is multi-threaded: many players' sessions produce events at the same time, on different worker threads. They meet at the plugin's queue and reach the guest one after the other. The actor itself is an ordinary tokio task, so it may resume on a different worker thread after a pause, but the guest never sees two calls overlap.

A call that waits in a host import keeps the actor busy. `Bans::ban`, `Player::connect`, `ctx.fire_named` and the other calls that wait on the proxy suspend the guest until the answer arrives, and the plugin handles nothing else in the meantime. A slow host service therefore delays every event, command and task queued behind the call. [Deadlines](#deadlines-and-host-call-timeouts) keep that wait bounded.

Different plugins have different actors and run in parallel with each other.

### Codec filters run elsewhere

[Codec filters](./codec-filters) do not go through the actor. Each connection side gets its own codec instance, built from the same component, with its own memory, and called synchronously on that connection's packet path. Instances of different connections run in parallel on different threads, but each instance is single-threaded and shares nothing with the plugin's main instance or with other connections. A `static` or `thread_local!` in filter code is therefore per connection side.

A filter call cannot yield: it holds its worker thread until it returns, and when the proxy is lightly loaded that worker is often the one that watches every socket of the proxy, so other players' packets wait for it too. `codec_cpu_budget` (5 ms by default, counted in ticks of guest running time) bounds each call, and a filter that keeps faulting for one client address is quarantined for that address. See [What a filter costs the other connections](./codec-filters#what-a-filter-costs-the-other-connections) and [Quarantine](./codec-filters#quarantine).

## The call queue

The queue holds up to `queue_capacity` jobs, 1024 by default, set in `[wasm]` or per plugin under `[plugins.<id>.wasm]`:

```toml
[wasm]
queue_capacity = 1024

[plugins.my-plugin.wasm]
queue_capacity = 4096
```

| Situation | What happens |
|-----------|--------------|
| The queue is full | The call is refused at once; the caller does not wait. An event handler gets no answer from the plugin: an [access event](./events#a-listener-that-does-not-answer) is denied, any other event keeps the result the handlers before it left. The host logs `wasm plugin call queue is full; refusing the call` with the plugin and the operation, at most once every 5 seconds, with the number of refusals it did not log. |
| `on_enable` or `on_disable` and the queue is full | They wait for room instead of being refused |
| The job's deadline passes while it is queued | Its caller stops waiting at the deadline and gets no answer from the plugin, and the job is dropped without reaching the guest when its turn comes (logged at debug). A job nobody waits for is dropped with a warning naming the plugin and the operation, at most five a minute per plugin. |
| The caller stopped waiting while the job was queued | The job is dropped without reaching the guest (logged at debug) |
| The job was meant for an instance a recovery has since replaced | The job is dropped; an access event it carried is denied |
| The plugin is quarantined after repeated faults | The call is refused at once; see [Fault Model](./fault-model) |
| The job started | It runs until it returns or until its deadline, whichever comes first. If its caller stopped waiting for another reason in the meantime, the answer is discarded. |

The last row matters: a guest call never outlives its deadline. At the deadline the host cuts the call, discards the instance and starts a fresh one, and the cut counts as a fault with the cause `the call ran past the event deadline` ([Fault Model](./fault-model)). One slow call therefore costs its own event and one fresh instance, and the calls queued behind it run on the fresh instance within their own deadlines instead of waiting for the slow one. The other faults stop a running call too: a trap, running past `cpu_budget` or `max_call_duration`, or a host function that panics.

## What one actor costs

All the calls into a plugin share its one queue and run one after the other, whatever their kind. The time a plugin spends on a call is therefore paid by every call queued behind it: every event of every type the plugin listens to, for every player, and the plugin's commands, tab completions, tasks and limbo callbacks. Two consequences follow.

- **A plugin handles at most 1 / (time per call) calls per second.** A listener that spends 20 ms on each event caps that plugin at 50 events per second, and the proxy's other events for that plugin wait their turn. When events arrive faster, the queue grows until calls pass their deadline (an access event is then denied) or the queue is full (the call is refused).
- **One slow event type delays the others.** A slow `ServerPreConnectEvent` listener delays the same plugin's `ChatMessageEvent` listener, even though chat has nothing to do with connecting.

Measured on a 16-thread AMD machine (release build, medians of three runs; absolute values vary with the machine, the ratios do not). The `wasm_contract` bench ([Benchmarking](../../reference/benchmarking#wasm-plugins-what-the-contract-costs)) reproduces the first three rows:

| Case | Cost per call | Ceiling for one plugin |
|------|---------------|------------------------|
| Cheap event (`PlayerClientBrandEvent`, empty handler), back to back | 2.8 µs (p99 6.0 µs) | about 415,000 events/s with 16 proxy tasks feeding it |
| `ProxyPingEvent` with a full ping response, the handler reads the virtual host only | 3.8 µs (p99 5.5 µs) | about 340,000 events/s |
| `ProxyPingEvent`, the handler changes the max players | 3.1 µs (p99 5.8 µs) | |
| First cheap event after 60 ms without calls | 17 to 22 µs (p99 52 to 71 µs) | |

A `ProxyPingEvent` handler that reads the description, the favicon or the player sample pays one host call per field it reads, which copies that field into the plugin: the sample costs about 110 ns per player and the description about 400 ns per text node.

The first event after a quiet spell costs more because the actor task has to wake up and its caches are cold, which is the usual case in a calm proxy. These costs are the floor: the time your handler spends comes on top, and past a few microseconds it sets the ceiling alone.

A worked example from an endurance run: a plugin spent 20 ms of guest time in its `ServerPreConnectEvent` listener and also listened to `ChatMessageEvent`. When 300 players logged in within 2 seconds, their 300 pre-connect events made 6 seconds of work in that plugin's queue, and the chat events of the 100 players already in game waited behind them: chat p99 went from 80 ms to 3.3 s and 16 chat messages got no answer, while the proxy itself sat at 8 % CPU. A plugin whose login listener ran away until `cpu_budget` (3 s) every 9 seconds held every other event of that plugin for those 3 seconds each time.

Different plugins have different queues, so this cost stays inside the plugin that pays it. To keep a plugin responsive:

- Keep each call short. Guest time and host calls that wait on the proxy both hold the actor.
- Listen only to the events you need: each listener adds its calls to the same queue.
- Put a slow part and a latency-sensitive part in two plugins; each gets its own actor.

### Watching the queue

The host keeps, for each plugin, how many calls wait in its queue and how long calls waited before the actor took them. Recording costs about 60 ns per call (a clock read when the call is queued, and a clock read and a few atomic stores when it is taken), without locks. The figures are shown by the [admin API](../builtin/admin-api#plugins) (`runtime.queue` of `GET /api/v1/plugins/{id}`), its web dashboard (the plugin's Runtime panel), and the console commands `plugins` (the `Wait p99` column) and `plugin <id>` (the `queue` and `queue wait` lines):

| Figure | Meaning |
|--------|---------|
| depth | Calls waiting in the queue now, out of `queue_capacity` |
| peak depth | The most calls waiting at once over the last minute, counting the one the actor was taking |
| calls | Calls the actor took from the queue over the last minute |
| wait p50, p99, max | How long those calls waited between being queued and being taken, over the last minute |

"The last minute" is between 50 and 60 seconds: the host counts in slices of 10 seconds and drops the oldest slice as a new one starts. The wait includes the actor's wake-up but not the call's own run time. Calls the actor drops when it takes them (past their deadline, caller gone, meant for a replaced instance) count too, so a queue that expires calls shows long waits.

How to read them: a wait p99 that approaches the event deadline (`[events] handler_timeout` minus the margin) means events are about to be denied or dropped for want of an answer; a depth close to `queue_capacity` means calls are about to be refused. Either way the plugin spends too long per call for the rate it receives, and the cause is usually its slowest listener.

## No re-entry

The host never calls into a plugin that is already running a call. A host import is implemented by the proxy and never calls back into the guest; anything that needs the guest goes through the queue and waits its turn.

For plugin code this means: while one of your handlers runs, no other handler, command, task or callback of your plugin runs. A `RefCell` you borrow in a handler cannot be borrowed by another of your handlers at the same time, because none is running. The SDK also releases its own tables before it calls your closure, so a handler may register or unregister handlers, commands and tasks from inside itself.

### Events a plugin causes for itself

A call that waits on the proxy can cause an event the same plugin listens to. If the proxy then waited for the plugin's answer, it would wait for a job queued behind the very call that is waiting: a deadlock.

The host prevents it with a **call chain**. Each call into a plugin records the plugin in a chain that follows the work the host does for that call. When an event is about to reach a plugin that is already in the chain, the host does not wait for it:

- the event is queued for the plugin and delivered once its current call has returned;
- the proxy goes on without the plugin's answer, so a result the plugin sets in that handler is ignored, and an access event is not denied for want of that answer: a plugin that transfers a player does not block its own `PreTransferEvent`;
- the host logs `wasm plugin is still running the call that led to this event; it receives the event without the proxy waiting for it, and its answer is ignored`. The plugin's guest warnings are limited to five a minute, and the next one that gets through carries the number of warnings left out.

This covers:

- a [named event](./events#named-events) the plugin fires and listens to itself;
- an event a host call fires inline, such as the `PreTransferEvent` that `Player::transfer` fires before it moves the player;
- a loop through other WASM plugins: plugin A fires a named event, plugin B handles it and fires one that A listens to. A is in the chain, so B's event reaches A without anyone waiting for A.

The chain also counts how many times it has entered each plugin. An event queued this way starts a new call into the plugin, still in the same chain, so two plugins that answer each event from the other by firing one back would pass events between them forever, with nobody waiting and no fault to stop them. The host allows a chain to enter the same plugin 8 times. An event that would enter it a ninth time is dropped: the plugin does not receive it, and the proxy logs `wasm plugin event dropped: the call that led to it already entered this plugin 8 times; plugins may be passing events back and forth`, at most five times a minute per plugin. A round trip such as A to B and back to A enters A twice and is not affected.

Providers get the same protection. A [ban provider](./bans) that calls the ban service from inside one of its own calls gets an `Unavailable` error at once instead of waiting on itself. A [permission provider](./permissions) asked for a checker from inside its own call answers with the snapshot it already holds for that player, or the node defaults.

Events the proxy queues rather than fires inline, such as the `BanIssuedEvent` after `Bans::ban`, are delivered by the proxy's event queue on its own task. They reach your plugin after the current call, like any other event.

The chain also records the player whose session is waiting on the call, if any: see [Calls that wait on the player's own session](#calls-that-wait-on-the-player-s-own-session). An event delivered without anyone waiting for it, as above, does not carry that player.

### Calls that wait on the player's own session

`Player::connect` and `Player::request_cookie` hand the request to the player's session and wait for it to finish. A player's session is a proxy task of its own, and it waits on your plugin whenever it delivers one of that player's session events: `ChatMessageEvent`, `CommandExecuteEvent`, `PluginMessageEvent`, `RawPacketEvent`, the login and connection events, `KickedFromServerEvent`, the limbo events and `DisconnectEvent`, and every limbo callback for that player. While it waits, it cannot carry out a request.

So the host records that player in the call's chain, and a `connect` or `request_cookie` for that same player from such a call fails at once with an `Error` of kind `InvalidState`, instead of waiting until its [limit](#deadlines-and-host-call-timeouts) with the session paused. The proxy logs a warning that names the player and the call, at most once every 10 seconds per player. The same calls for another player work as usual.

Use `switch_server` in those places. It returns as soon as the session has taken the request, and the switch then runs on its own:

```rust
ctx.on::<ChatMessageEvent>(EventPriority::Normal, |event| {
    if event.message == "!hub" {
        let _ = event.player.handle().switch_server("hub");
    }
})?;
```

Commands are not in that list. A command a player types runs on that player's command queue, off the session, so a command handler can call `connect` for the player who typed it and use the outcome:

```rust
ctx.command("hub")
    .handler(|invocation| {
        let Some(player) = invocation.player() else {
            return;
        };
        match player.handle().connect("hub") {
            Ok(result) if result.is_success() => {}
            Ok(_) => {
                let _ = invocation.reply("Could not reach the hub");
            }
            Err(e) => {
                let _ = invocation.reply(format!("Could not reach the hub: {e}"));
            }
        }
    })
    .register()?;
```

The session goes on with the player's packets meanwhile, and the player's commands run one at a time, in the order they were typed. Tab completions take the same queue. See [Player commands run on a queue of their own](../dev/threading#player-commands-run-on-a-queue-of-their-own) in the native guide.

### Where the chain does not reach

The chain follows the call's own work. It does not follow work the host hands to another task, and a player's session is another task. If your plugin listens to `ServerPreConnectEvent`, `ServerConnectedEvent` or `ServerPostConnectEvent` and calls `connect`, the session fires those events while your `connect` call is still running. Their delivery waits in your queue behind it, and the session waits until each event's deadline before it goes on without your answer; `ServerPreConnectEvent` is an access event, so the connection is then refused. A plugin that listens to connection events should move players with `switch_server`.

A recovery belongs to the call that faulted: the fresh instance's `on_enable` runs in that call's chain. When plugin B waits on a call into plugin A that traps, an event A's fresh instance fires for B reaches B without anyone waiting for it, instead of waiting on B until `handler_timeout`.

## Deadlines and host-call timeouts

Each job carries a deadline, fixed when it is queued:

| Call into the plugin | Deadline |
|----------------------|----------|
| Event handler, ban provider call, permission snapshot | `[events] handler_timeout` minus a margin (9.75 s with the default 10 s); a ban check ends at `[ban] check_timeout` when that comes first |
| Command, tab completion, scheduled task, limbo callback | `max_call_duration` in `[wasm]` (60 s by default) |
| `on_enable`, `on_disable` | none; during a proxy shutdown `on_disable` is stopped after 5 seconds, see [Proxy shutdown](./lifecycle#proxy-shutdown) |

The margin is a fifth of the budget, at most 250 ms. It keeps an event's deadline before the moment the event bus gives up on the listener, so the plugin's answer, or the proxy's deny when there is none, is applied to the event. A ban check takes the deadline of the ban manager that asked, which stops waiting at `check_timeout`.

The deadline bounds four things:

- **The wait in the queue.** A job still queued when its deadline passes is dropped, as in the table above.
- **The caller's wait.** Every caller gets its answer by the deadline: the plugin's answer, or a failure if the job is still queued, still running, or waiting for a recovery.
- **Host calls.** Every host call that waits on the proxy (`Servers::start` and `stop`, every `Bans` function, `Player::connect`, `transfer`, `request_cookie`, `refresh_permissions`, `ctx.fire_named`, `Permissions::set_snapshot` and `release`) ends at the earlier of `host_call_timeout` (30 s by default) and a margin before the deadline, the same fifth capped at 250 ms. A 10 s `handler_timeout` leaves host calls 9.5 s, a 300 ms one 192 ms. `switch_server` uses its own 250 ms limit under the same rule. On expiry the host call returns an `Error` of kind `Timeout`, and your code still has time to decide, for example to deny a login when the ban check did not answer.
- **The guest call itself.** A call still running at its deadline is cut off: the instance is discarded and replaced, and the cut counts as a fault. Guest code between host calls is also bounded by `cpu_budget`, and the whole call by `max_call_duration` counted from when it starts.

`cpu_budget` (3 s by default) limits guest execution per call. The host checks it at every epoch tick (`epoch_tick`, 1 ms by default), counting from the start of the call: at each tick a running guest yields back to the tokio runtime, behind the tasks that are already waiting and after the worker has looked for network events, so a long computation never holds a worker thread, or the proxy's sockets, for more than about one tick at a time, and a call that exceeds its budget traps. A tick during which the thread was preempted by the operating system counts once. `max_call_duration` limits the whole call, host calls included, in wall-clock time. Both faults discard the instance and trigger a [recovery](./fault-model).

[Lifecycle](./lifecycle#deadlines) and [Host Services](./services#slow-services-and-deadlines) describe the deadlines from the host's and the plugin's side.

## Why guest state needs no Send or Sync

Because one actor runs one call at a time and the guest is never re-entered, only one thread ever executes a given instance's code, and never two calls of it at once. The SDK is built on that guarantee: it keeps your handlers in thread-local tables of `Rc` and `RefCell`, and none of its bounds ask for `Send` or `Sync`.

| SDK item | Bound |
|----------|-------|
| `Plugin` | `'static` |
| `ctx.on` handler | `impl FnMut(&mut E) + 'static` |
| Command handler | `impl FnMut(CommandInvocation) + 'static` |
| `ctx.delay` task | `impl FnOnce() + 'static` |
| `ctx.interval` task | `impl FnMut() + 'static` |
| `LimboHandler`, `BanProvider`, `PermissionProvider`, `CodecFilter` | no `Send` or `Sync` |

So share state with `Rc` and mutate it through `Cell` or `RefCell`:

```rust
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use infrarust_plugin_sdk::prelude::*;

#[derive(Default)]
struct Stats {
    joins: Rc<Cell<u32>>,
    last_server: Rc<RefCell<HashMap<PlayerId, ServerId>>>,
}

#[plugin(id = "stats", name = "Stats")]
impl Plugin for Stats {
    fn on_enable(&self, ctx: &Context) -> Result<(), PluginError> {
        let joins = Rc::clone(&self.joins);
        ctx.on::<PostLoginEvent>(EventPriority::Normal, move |_| {
            joins.set(joins.get() + 1);
        })?;

        let last = Rc::clone(&self.last_server);
        ctx.on::<ServerPostConnectEvent>(EventPriority::Normal, move |event| {
            last.borrow_mut()
                .insert(event.player.id, event.server.clone());
        })?;

        let joins = Rc::clone(&self.joins);
        ctx.command("joins")
            .description("Joins since the last restart")
            .handler(move |invocation| {
                let _ = invocation.reply(format!("{} joins", joins.get()));
            })
            .register()?;

        Ok(())
    }
}
```

A `Mutex` would never be contended and atomics buy nothing. Two things still apply:

- **Memory does not survive a recovery.** A fault replaces the instance, and everything in guest memory goes with it. Persist what matters to the data directory; see [What survives a recovery](./fault-model#what-survives-a-recovery).
- **Codec instances are separate.** State in a codec filter lives in that connection side's instance, not in the plugin's main instance.

Build the plugin as a normal `wasm32-wasip2` component. No thread-related target feature, shared memory or imported memory is needed.

## WASM threads are not supported

A plugin cannot run guest code on a second thread, for three independent reasons:

- **The component model does not allow it.** The canonical ABI, through which a component exchanges every value with the host, requires a plain linear memory, and the validator rejects a shared memory there. Plugins are components.
- **The host does not implement `wasi-threads`.** That proposal lets a core WebAssembly module ask the host for a new thread (`thread-spawn`), and was designed for WASI preview 1 modules (the `wasm32-wasip1-threads` target), not components. The host links WASI preview 2 and the `infrarust:plugin` interfaces only.
- **The target cannot start threads.** On `wasm32-wasip2`, wasi-libc's `pthread_create` always fails with `ENOTSUP`, so `std::thread::Builder::spawn` returns an error and `std::thread::spawn` panics, which traps the plugin.

Supporting threads would change this page, not extend it. Guest code could then run on several threads at once, so the SDK's thread-local `Rc` and `RefCell` tables would no longer be sound, handler bounds would need `Send` and `Sync`, plugin state would need locks as in native plugins, and the host would have to allow concurrent calls into one instance, which the actor exists to prevent. That would be a breaking change to the SDK.

For parallel or long work today, let the proxy do it: host calls such as the ban service or `fire_named` run on the proxy's threads while your call waits, bounded by the deadlines above. Work that cannot fit in a call's budget belongs in a [native plugin](../dev/threading).

## See also

- [Architecture](./architecture): the component model and the dispatch direction.
- [Lifecycle](./lifecycle#deadlines): deadlines as the host applies them.
- [Fault Model](./fault-model): traps, recovery, restart budget and quarantine.
- [Events](./events): subscribing, results and named events.
- [Deploying & Configuring](./deploying#sandbox-limits): the `[wasm]` limits.
- [Native Threading Model](../dev/threading): the in-process model for native plugins.
