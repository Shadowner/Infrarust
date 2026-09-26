---
title: Threading Model
description: Where native plugin code runs. The proxy's tokio runtime, the Send and Sync bounds, per-handler timeouts and panic isolation, the order events arrive in, what blocking costs, and when it is safe to call back into the proxy.
outline: [2, 3]
---

# Threading Model

A native plugin runs inside the proxy process, on the proxy's tokio runtime. No thread belongs to a plugin: a listener, a command, a limbo callback or a scheduled task runs as part of the proxy task that triggered it, and that task can be a player's connection, the event queue, the console or a task the plugin scheduled. This page says which task runs your code, what the proxy guarantees about order and time, and what your code must not do on those tasks.

WASM plugins follow a different model, one call at a time per plugin; see [WASM Threading](../wasm/threading).

## The runtime

The proxy builds one multi-threaded tokio runtime at startup. Its worker threads are named `infrarust-worker`, and [`worker_threads`](../../configuration/global#worker-threads) sets how many there are (`0`, the default, lets tokio start one per CPU core).

Every piece of async plugin code is polled by that runtime: `on_enable` and `on_disable`, async listeners, command handlers, limbo handlers, providers and scheduled futures. Tokio moves tasks between workers, so a future can resume on another thread after an `.await`. Inside plugin code you are in the runtime's context: `tokio::spawn`, `tokio::time` and tokio channels work, and `block_on` panics because the thread is already driving the runtime.

Prefer the [scheduler](./api#scheduler) to `tokio::spawn` for background work. A task started through `ctx.scheduler()` belongs to your plugin, is cancelled when the plugin is disabled, and its panics are logged with your plugin id. A task you spawn yourself is none of these.

`on_enable` runs during startup, before the proxy accepts connections: plugins are enabled, the listener is bound, `ProxyInitializeEvent` fires, and only then does the accept loop start. A slow `on_enable` delays startup; it does not race with players.

## Send and Sync

Everything a plugin hands to the proxy can be called from any worker thread, and the same listener runs for many players at once: two players logging in together run your `PostLoginEvent` listener concurrently, on two threads. The API encodes this in its bounds.

| What you provide | Bound |
|------------------|-------|
| `Plugin` | `Send + Sync` |
| Sync listener (`subscribe`) | `Fn(&mut E) + Send + Sync + 'static` |
| Async listener (`subscribe_async`) | `Fn(&mut E) -> BoxFuture<'_, ()> + Send + Sync + 'static` |
| A custom event type | `Event: Send + Sync + 'static` |
| `CommandHandler`, `LimboHandler`, `PermissionProvider`, `PermissionChecker`, `BanProvider`, `CodecFilterFactory`, `TransportFilter` | `Send + Sync` |
| `CodecFilterInstance` | `Send` (one instance per connection side, never shared) |
| `Scheduler::delay` | `Box<dyn FnOnce() + Send>` |
| `Scheduler::interval`, `interval_with_delay` | `Box<dyn Fn() + Send + Sync>` |
| `Scheduler::spawn` | `BoxFuture<'static, ()>` |
| `Scheduler::delay_async` | `AsyncTask`: `Box<dyn FnOnce() -> BoxFuture<'static, ()> + Send>` |
| `Scheduler::repeat` | `RepeatingTask`: `Box<dyn Fn() -> BoxFuture<'static, ()> + Send + Sync>` |
| `Scheduler::spawn_blocking` | `Box<dyn FnOnce() + Send>` |
| A service in the [service registry](./services) | `Send + Sync + 'static` |

`BoxFuture<'a, T>` is `Pin<Box<dyn Future<Output = T> + Send + 'a>>`, so every future you return must be `Send`. The compiler then refuses a future that keeps an `Rc`, a `RefCell` borrow or a `std::sync::MutexGuard` alive across an `.await`.

State shared between listeners needs synchronisation. Atomics cover counters:

```rust
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

let online = Arc::new(AtomicUsize::new(0));

let joins = Arc::clone(&online);
ctx.event_bus()
    .subscribe::<PostLoginEvent, _>(EventPriority::NORMAL, move |_| {
        joins.fetch_add(1, Ordering::Relaxed);
    });

let leaves = Arc::clone(&online);
ctx.event_bus()
    .subscribe::<DisconnectEvent, _>(EventPriority::NORMAL, move |_| {
        leaves.fetch_sub(1, Ordering::Relaxed);
    });
```

A `std::sync::Mutex` is fine as long as the guard is dropped before the next `.await`. End the statement that uses it, as below, or use `tokio::sync::Mutex` when a lock must be held across an await:

```rust
let bans = ctx.ban_service_handle();
let greeted: Arc<Mutex<HashSet<String>>> = Arc::default();

ctx.event_bus()
    .subscribe_async::<PostLoginEvent, _>(EventPriority::NORMAL, move |event| {
        let bans = Arc::clone(&bans);
        let greeted = Arc::clone(&greeted);
        Box::pin(async move {
            // The guard is dropped at the end of this statement, before any await.
            let first = greeted
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(event.profile.username.clone());
            if !first {
                return;
            }
            let target = BanTarget::Uuid(event.profile.uuid);
            if let Ok(None) = bans.get(&target).await {
                let _ = event.player.send_message(Component::text("Welcome!"));
            }
        })
    });
```

## Isolation and timeouts

Every listener goes through the same dispatch: listeners run one after another in priority order, each sees what the previous ones changed, and each is isolated from the others.

- **Panics.** A panic while the listener builds its future or while the future is polled is caught. The proxy logs it with your plugin id (and, for an event another plugin fired, `fired_by`) and passes the event to the next listener. What the listener changed before panicking stays applied, so set the result last.
- **Timeouts.** An async listener is polled once right away. If it is not finished, it gets until `[events] handler_timeout` (10 seconds by default), counted from when it started. At that point its future is dropped, which cancels it at the `.await` it was waiting on, and the event moves on. Raw packet listeners use `packet_handler_timeout` instead.
- **Slow listeners.** A listener that takes longer than `slow_handler_threshold` (1 second by default) is logged as slow, with its plugin id.
- **Synchronous listeners cannot be cancelled.** The timeout applies to futures. A sync listener, or an async one that blocks inside a single poll, runs until it returns, and the event and the task that fired it wait for it.
- **`DisconnectEvent`** is also bounded as a whole: all of one player's `DisconnectEvent` listeners together get `[events] disconnect_deadline` (15 seconds by default), after which the rest are cancelled and the player is removed anyway.

```toml
[events]
handler_timeout = "10s"
slow_handler_threshold = "1s"
packet_handler_timeout = "10s"
disconnect_deadline = "15s"
```

Cancellation happens at an `.await`. Code after that await never runs, so do not leave shared state half-updated across one. Work that must finish, such as saving a player's data, belongs in a scheduled task, which the event timeout does not cancel.

Command handlers and limbo callbacks have no timeout. They are awaited until they return; see [player events](#player-events-run-in-the-player-s-session) for what that holds.

## Player events run in the player's session

Each connection is served by one tokio task, the player's session. The proxy fires that player's lifecycle and connection events from the session and awaits each dispatch before it continues: `ConnectionHandshakeEvent`, `PreLoginEvent`, `GameProfileRequestEvent`, `PermissionsSetupEvent`, `LoginEvent`, `PostLoginEvent`, `PlayerChooseInitialServerEvent`, `ServerPreConnectEvent`, `ServerConnectedEvent`, `ServerPostConnectEvent`, `KickedFromServerEvent`, `LimboEnterEvent`, `LimboExitEvent` and `DisconnectEvent`. `PreTransferEvent` fires in the session for a transfer the backend sends, and in the caller's task for one a plugin requests.

This gives two guarantees:

- **In order for one player.** One player's events reach your listeners one at a time, in the order they happen, and never concurrently with each other. `DisconnectEvent` comes after every other event the session fires for that player: the session dispatches it on a task of its own, bounded by `disconnect_deadline`, and waits for it before it releases the player.
- **Concurrent across players.** Different players' sessions run in parallel, so the same listener runs for several players at once. Anything a listener shares between players needs the synchronisation described above.

In `offline` and `client_only`, the session also handles the game traffic, one packet at a time. `ChatMessageEvent`, `CommandExecuteEvent`, `PluginMessageEvent` and `RawPacketEvent` are awaited between two packets, and so is a plugin command a player types: while your listener or handler runs, that player's session reads nothing more from the client or the backend. The player sees the delay.

A few player events are not fired from the session. `PlayerClientBrandEvent`, `PlayerSettingsChangedEvent`, `PlayerChannelRegisterEvent` and `PlayerResourcePackStatusEvent` go through the proxy event queue below: they keep their order among themselves, but not with the session's events, and they can arrive after the player's `DisconnectEvent`.

See [Delivery](./events#delivery) for the complete list and [Guarantees](./events#guarantees) for what holds at each step of a login.

## Proxy events go through one queue

Events that are not about one session's progress are posted to a single queue: `ServerStateChangeEvent`, `BackendHealthEvent`, `ConfigReloadEvent`, `ConnectionRejectedEvent`, `BanIssuedEvent`, `BanRevokedEvent`, `PluginEnabledEvent`, `PluginDisabledEvent`, `ServiceProvidedEvent`, `ServiceRemovedEvent`, and the client state events above.

- **The poster does not wait.** The proxy posts the event and carries on. By the time your listener runs, the state the event describes may have changed again, so read the event's fields rather than querying the current state.
- **FIFO, one at a time.** One dispatcher delivers the events in the order they were posted, and starts the next event only when every listener of the current one returned. Two health transitions or two reloads always arrive in the order they happened.
- **A slow listener delays the queue.** Each listener is still bounded by `handler_timeout`, but every queued event behind it waits. Keep queued-event listeners short, and hand longer work to the scheduler.
- **Failures do not stop the queue.** A panicking listener is isolated as usual, and the dispatcher moves on to the next event.
- **Flushed at shutdown.** After `ProxyShutdownEvent`, the proxy waits until every event posted before that point has been delivered, and only then disables plugins. Your listeners see those events before `on_disable` runs.

`ProxyInitializeEvent`, `ProxyShutdownEvent` and `ProxyPingEvent` are awaited by the code that fires them. An event your plugin fires with `EventBusExt::fire` is dispatched inline, in your task: `fire` returns once every listener has run.

## Blocking

A worker thread that blocks cannot run anything else while it blocks. Other workers can take over some of the tasks queued on it, but every blocked worker is one thread fewer for all the sessions, timers and queued events of the proxy, and with few workers a single blocking call slows the whole proxy.

Code runs directly on a worker thread in these places:

- synchronous listeners, and every async listener between two `.await`s;
- command handlers, limbo callbacks and providers, between awaits;
- the closures of `delay`, `interval` and `interval_with_delay`;
- codec filters, called for every packet of a connection.

Blocking there also defeats the time limits: tokio can only cancel a future at an `.await`, so a listener stuck in a blocking call is not cancelled at `handler_timeout`.

Blocking means synchronous file or network I/O, a blocking database driver, `std::thread::sleep`, waiting on a `std::sync` lock that another thread holds for long, or a long computation. Move it to the blocking thread pool with `ctx.scheduler().spawn_blocking`:

```rust
let scheduler = ctx.scheduler_handle();
let path = ctx.data_dir().join("last-seen.log");

ctx.event_bus()
    .subscribe::<DisconnectEvent, _>(EventPriority::NORMAL, move |event| {
        let line = format!("{}\n", event.username());
        let path = path.clone();
        scheduler.spawn_blocking(Box::new(move || {
            use std::io::Write;
            let file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path);
            if let Ok(mut file) = file {
                let _ = file.write_all(line.as_bytes());
            }
        }));
    });
```

The closure runs on tokio's blocking pool, belongs to your plugin and is logged if it panics. `cancel` stops it only before it starts: a closure that is running goes to completion. When an async listener needs the result, `tokio::task::spawn_blocking(..).await` works as well, but the proxy does not track that task for your plugin.

## Scheduled tasks

Each task scheduled through `ctx.scheduler()` is a tokio task, or a blocking-pool task for `spawn_blocking`, owned by your plugin. The proxy cancels them all when the plugin is disabled, and a plugin cannot cancel another plugin's task.

| Method | Runs | Overlap |
|--------|------|---------|
| `delay`, `delay_async` | Once, after the delay | None |
| `interval`, `interval_with_delay` | A synchronous closure at a fixed rate, inline on a worker thread | Never: the next tick waits for the closure to return; a tick missed because the runtime was busy is skipped, not replayed |
| `repeat` | Builds and awaits a new future, then waits `period` before the next run | Never: a slow run pushes the next one back. The first run starts after `initial_delay`, or after one `period` when it is `None` |
| `spawn` | A future, now | None |
| `spawn_blocking` | A closure on the blocking pool | None |

A task that panics is logged with your plugin id and ends that run only: a repeating task runs again at its next period. A period below 1 ms is raised to 1 ms. `cancel` aborts a task: an async task is dropped at its next `.await`, and a closure that already started runs to completion. See [Scheduler](./api#scheduler) for the signatures.

`repeat` is the right tool for periodic async work, such as a save every minute: because it waits for the run to finish, two saves never run at the same time, even when storage is slow. With `interval`, keep the closure short and spawn the work if it can take time.

## Calling back into the proxy

Plugin code calls proxy services all the time. Most calls are safe from anywhere; a few wait on a player's session and must not be awaited from that same session.

**Safe from any listener, command or task:**

- Player actions that queue a command for the session and return: `send_message`, `send_title`, `send_action_bar`, `clear_title`, `set_player_list_header_footer`, `show_boss_bar`, `send_resource_pack`, `remove_resource_pack`, `send_plugin_message`, `send_plugin_message_to_backend`, `store_cookie`, `send_packet`, `disconnect` and `switch_server`. Those sent while the player is still logging in wait and go out in order once the client can take them.
- Lookups in the player registry and the other services, and reading the player handle.
- Subscribing and unsubscribing, even from inside a listener. A dispatch in progress keeps the listener list it started with, so a new listener sees the next event; a removed listener is skipped even by the dispatch in progress.
- Registering or removing commands, limbo handlers, permission nodes, services and filters, at any time.

**Do not await from the same player's session:**

`Player::connect` and `Player::request_cookie` wait for the player's session to carry out the request and answer. If you await them from code the session itself is awaiting (a listener of one of that player's session events, a command that player typed, a limbo callback for that player), the session cannot handle the request until your code returns:

- a listener waits until `handler_timeout` cancels it, and the request is then carried out without anyone waiting for the answer;
- a command handler or limbo callback has no timeout, so that player's session stays stuck.

Queue the request with `switch_server` when you do not need the outcome, or await `connect` in a task of its own:

```rust
pub struct Hub {
    scheduler: Arc<dyn Scheduler>,
}

impl CommandHandler for Hub {
    fn execute<'a>(&'a self, ctx: CommandContext) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            let Some(player) = ctx.source.player().cloned() else {
                return;
            };
            let source = ctx.source.clone();
            // The command runs in the player's session: let it return, and wait for
            // the switch in a task of its own.
            self.scheduler.spawn(Box::pin(async move {
                match player.connect(ServerId::new("hub")).await {
                    Ok(result) if result.is_success() => {}
                    Ok(result) => {
                        source.send_message(Component::error(format!(
                            "Could not reach the hub: {}",
                            result.as_str()
                        )));
                    }
                    Err(e) => source.send_message(Component::error(e.to_string())),
                }
            }));
        })
    }
}
```

Awaiting these calls for another player, from a queued event or from a scheduled task is fine: the session that answers is not waiting on you.

**Firing events from a listener:** `EventBusExt::fire` dispatches inline and returns when every listener of the fired event has run, and that time counts against your own listener's `handler_timeout`. A listener that fires the event type it listens to recurses; nothing stops it but your own code.

## See also

- [Events Reference](./events): delivery, ordering guarantees and every event.
- [Plugin API](./api#scheduler): the scheduler and the other services.
- [Global Settings](../../configuration/global#plugin-event-handlers): the `[events]` keys.
- [WASM Threading](../wasm/threading): the model for sandboxed plugins.
- [Migrating from 2.0.0-beta.3](./migration): what changed in the plugin API.
