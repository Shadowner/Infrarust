---
title: WASM Fault Model
description: What happens when a WASM plugin traps or overruns a limit, what survives the fresh instance that replaces it, and how restarts, backoff and quarantine are budgeted.
outline: [2, 3]
---

# Fault model

A fault in a WASM plugin does not disable it until the next proxy restart. The proxy throws the faulty instance away, creates a fresh one from the already compiled component, and runs the plugin's `on_enable` again. A plugin that keeps failing is quarantined for a while and then tried again.

This page covers what counts as a fault, what the proxy does about it, what the plugin keeps and loses, and how to write a plugin that recovers well. The limits that cause most faults are described in [Capabilities & Sandbox](./capabilities#the-sandbox); the settings on this page live in [`[wasm.recovery]`](../../configuration/global#recovery-after-a-fault).

## What counts as a fault

Any of these, during any call into the plugin (an event, a command, a tab completion, a scheduled task, a limbo callback, a ban or permission provider call):

- A trap: a panic in the plugin, an out-of-bounds memory access, a CPU overrun (`cpu_budget`), a memory-grow failure past `memory_limit_mb`, or the use of a dropped or invalid resource handle.
- A call still running at its [deadline](./threading#deadlines-and-host-call-timeouts): `[events] handler_timeout` minus a margin for an event or a provider call, earlier for a ban check when `[ban] check_timeout` is shorter, and `max_call_duration` after the call was queued for a command, a tab completion, a scheduled task or a limbo callback. The call is cut off, and the error gives the cause `the call ran past the event deadline` (or `ran past its deadline` for a callback), so a slow plugin can be told apart from one that crashes. One slow call therefore costs its own event and a fresh instance, not every call queued behind it.
- A call still running at `max_call_duration`, host calls included. The call is cut off.
- A panic in a host function the plugin called.

These are not faults, and the instance keeps running:

- A caller that stops waiting before the call's deadline, for example because the player left. The call goes on until it returns or reaches its deadline.
- A host call that fails or runs out of time. The plugin gets an error value and decides what to do.
- A callback that returns an error, and a call refused because the plugin's queue is full.

Codec filters are separate instances, one per connection side, and have their own rule: a trap there only disables that one instance, which lets packets through from then on. See [Codec Filters](./codec-filters).

## What happens to the call that faulted

The call that faulted gets no answer from the plugin:

| Call | Result |
|------|--------|
| Access event (`PreLoginEvent`, `LoginEvent`, `GameProfileRequestEvent`, `PermissionsSetupEvent`, `ServerPreConnectEvent`, `PlayerChooseInitialServerEvent`, `PreTransferEvent`) | The event is denied, see [A listener that does not answer](./events#a-listener-that-does-not-answer) |
| Any other event | The event keeps the result it had before this listener |
| Command | Nothing happens |
| Tab completion | No suggestions |
| Limbo `on_player_enter` | The player is denied with "Limbo handler unavailable" |
| Other limbo callbacks | Nothing happens |
| Ban check | The login is refused, a status ping is answered, see [Bans](./bans) |
| Permission snapshot | The player gets the node defaults, see [Permissions](./permissions) |

The caller gets this answer once the proxy has finished replacing the instance, so when it arrives the fresh instance has already subscribed again and the next event reaches it. Every caller gets its answer by the call's deadline at the latest: a caller whose deadline passes while the proxy is still replacing the instance gets the same answer at its deadline.

## Recovery

When a call faults, the proxy:

1. Logs one error naming the plugin, the call and the cause.
2. Discards the instance and everything it registered with the proxy: its event listeners are removed, its scheduled tasks are cancelled, and every player it still holds in limbo is released with a deny ("Limbo handler unavailable") so nobody is stuck. Calls already queued for the old instance's listeners and tasks are dropped; an access event among them is denied. For each access event the instance listened to, a guard listener at the same priority takes its place and denies that event until a fresh instance is enabled.
3. Creates a fresh instance from the compiled component, with the same plugin context, capabilities, data directory and limits.
4. Runs the guest's `on_enable` in it, with `ctx.enable_reason()` set to `EnableReason::Recovered(RecoveryInfo { attempt, cause })`. `attempt` numbers the fresh instances the proxy has started for this plugin since it was loaded, failed ones included: 1 for the first, 2 for the next, and it does not start over after a recovery that worked. It is the new instance's generation minus one. `cause` describes the fault. A trap, a cut-off or an `Err` from this `on_enable` counts as one more fault.
5. Logs one info line with the new instance's generation (the first instance is generation 1, each fresh one adds 1).

The recovery runs as part of the call that faulted. If a plugin was waiting on that call, for example the one that fired the named event whose handler trapped, an event the fresh instance's `on_enable` fires reaches that plugin without anyone waiting for it, as described in [Events a plugin causes for itself](./threading#events-a-plugin-causes-for-itself), instead of stalling until `[events] handler_timeout`. A retry after a quarantine is not part of any call.

### The cause

The error line and `RecoveryInfo.cause` carry the same text:

| Fault | Cause |
|-------|-------|
| A panic in a plugin built with the SDK | `the guest trapped: panicked at src/lib.rs:12:5: <panic message>` |
| Guest code that used up `cpu_budget` | `the call ran past cpu_budget` |
| An event or provider call cut off at its deadline | `the call ran past the event deadline` |
| A command, tab completion, scheduled task or limbo callback cut off at its deadline | `the call ran past its deadline (max_call_duration after it was queued)` |
| A call still running at `max_call_duration` | `the call ran past max_call_duration (60s)` |
| Any other trap | `the guest trapped:` and the trap, such as ``wasm trap: wasm `unreachable` instruction executed`` or `forcing trap when growing memory to 67174400 bytes` |
| A panic in a host function | `a host function panicked: <panic message>` |

A panic in a Rust guest traps with `unreachable`, which says nothing about why. The SDK installs a panic hook when `on_enable` first runs in an instance. The hook sends the panic's location and message to the proxy through the `log` interface, as an `error` line that starts with `panicked at `, just before the guest traps. The proxy keeps that line for the call it came from, logs it at `debug` only, and puts it in the cause of the fault. The line is cut at 1 KiB. A guest written without the SDK can do the same by logging such a line at `error` before it traps. A plugin built with an SDK older than this hook, or a plugin that installs its own panic hook, gets the `unreachable` cause.

Commands and limbo handlers are known to the proxy by name, so they are not registered twice. When the fresh instance registers a command or a limbo handler under a name the plugin already used, the existing registration is pointed at the fresh instance. A name that the fresh instance does not register again during its `on_enable` is removed: the command is unregistered, and the limbo handler denies players who reach it.

## What survives a recovery

| Kept | Lost |
|------|------|
| Files in the plugin's data directory | Everything in guest memory: statics, caches, counters, `RefCell` maps |
| The plugin's capabilities, configuration and limits | Event subscriptions of the old instance (`on_enable` makes new ones) |
| Commands and limbo handlers that `on_enable` registers again | Commands and limbo handlers registered later, outside `on_enable`, unless `on_enable` registers them again |
| Codec filters (separate instances; the fresh instance registers the same ids again, which the plugin still owns) | Scheduled tasks (`delay` and `interval`) |
| | Players the old instance held in limbo (released with a deny) |
| | Resource handles the guest held, such as limbo session handles |

`on_enable` runs again for every fresh instance, so it can run many times over the life of the proxy.

The order of listeners can change. The fresh instance subscribes again, and a new subscription runs after the listeners that already had the same priority. If your plugin and another one both listen to an event at the same priority, the order between them may flip after a recovery. Use distinct priorities when the order matters.

## Restart budget and quarantine

The proxy starts a fresh instance straight away, up to `max_restarts` times (5 by default) within a sliding `window` (5 minutes). A fault beyond that quarantines the plugin:

- It stays quarantined for `backoff_initial` (1 second), doubled for each quarantine in a row, up to `backoff_max` (5 minutes). The warning logged on quarantine carries the time until the next attempt (`retry_in`).
- Every call to a quarantined plugin is answered at once without running guest code, with the same results as in the table above: access events are denied, other events keep their result, commands do nothing, limbo handlers deny the player. The guard listeners keep denying the access events the plugin listened to, so a quarantined plugin that listens to `PreLoginEvent` closes the proxy to new logins until its backoff passes and a fresh instance starts. The proxy logs `access event denied` with the plugin and the event, at most five times a minute per plugin.
- When the backoff has passed, the proxy tries a fresh instance again, on its own or when the next call arrives, whichever comes first. If it works the plugin is healthy again; if it fails the plugin is quarantined for the next, longer backoff.

Restarts older than `window` stop counting. Once a fault can be restarted straight away again, the backoff starts over from `backoff_initial`. It also starts over when the fault comes after a whole `window` without any restart or retry, so with `max_restarts = 0`, where every fault quarantines the plugin, a fault after a long healthy stretch waits `backoff_initial` again rather than the doubled backoff of the previous quarantine.

```mermaid
stateDiagram-v2
    [*] --> Starting: load
    Starting --> Healthy: on_enable ok
    Starting --> Failed: fault in the first on_enable
    Healthy --> Recovering: fault
    Recovering --> Healthy: fresh instance enabled
    Recovering --> Recovering: fault in on_enable, budget left
    Recovering --> Quarantined: budget spent
    Quarantined --> Recovering: backoff passed
    Recovering --> [*]: disable or unload, recovery stopped
    Healthy --> [*]: on_disable
    Quarantined --> [*]: on_disable skipped
    Failed --> [*]
```

### A fault in the first `on_enable`

The budget only applies once the plugin has been enabled. If the first `on_enable` faults or returns an `Err`, the enable fails, the instance is discarded with everything it registered (listeners and scheduled tasks included), the proxy reports that the plugin could not be enabled, and no fresh instance is tried. An `Err` is reported as `PluginError::InitFailed` with its message. A plugin that declares it as a hard dependency is not enabled either, and gets its own error: `plugin 'x' requires 'y', which is not enabled`. The other plugins start as usual.

### A fault before the plugin is loaded

At discovery the proxy runs the guest's `metadata()` once, in a throwaway instance. A trap there, or a `metadata()` still running after the smaller of `max_call_duration` and 5 seconds, refuses that file with one error and no retry; the proxy starts with the other plugins. See [Lifecycle](./lifecycle#metadata-probe).

## Disable and unload

On `on_disable`, a plugin with a live instance runs its guest `on_disable` as usual. A quarantined plugin has no instance to run it in, so the call is skipped with a warning. In every case the proxy then removes the instance's listeners and scheduled tasks and the guard listeners of a quarantined plugin, releases the players it holds in limbo, removes the plugin's codec filters, and stops the plugin's task. Unloading the plugin does the same without calling `on_disable`.

A disable or an unload that arrives while the plugin is replacing its instance stops the recovery: the restart in progress ends (it is bounded by `max_call_duration`), no further restart is tried, and `on_disable` is skipped since there is no live instance. Either returns within one `max_call_duration` instead of waiting for every restart the budget still allows.

## Configuration

```toml
[wasm.recovery]
max_restarts = 5
window = "5m"
backoff_initial = "1s"
backoff_max = "5m"

[plugins.flaky.wasm.recovery]
max_restarts = 1
backoff_max = "30m"
```

A plugin-level table overrides only the keys it sets. See [`[wasm.recovery]`](../../reference/config-schema#wasm-recovery) for the ranges.

## Writing a plugin that recovers well

- **Persist what matters.** Write state you cannot lose to the data directory (`/` inside the sandbox) and read it back in `on_enable`. Treat guest memory as a cache.
- **Keep `on_enable` repeatable.** It must succeed when it runs a second time with files left over from the first run, and it should be quick: the caller that hit the fault waits for it.
- **Keep access handlers fast.** A handler for an access event that runs past its deadline is cut off, counted as a fault and its event denied. Do slow lookups (a web request, a database) in a scheduled task that fills a cache, and answer the event from the cache.
- **Register everything in `on_enable`.** Commands and limbo handlers registered there are rebound to the fresh instance; ones registered later are removed after a recovery.
- **Expect held players to be released.** A player your plugin held in limbo is denied when the instance goes away. They can reconnect and meet the fresh instance.
- **Watch the logs.** Repeated `wasm plugin instance failed` errors, or a `quarantined` warning, mean the plugin has a bug that recovery only hides.

## See also

- [Lifecycle](./lifecycle): the stages from discovery to disable.
- [Capabilities & Sandbox](./capabilities): the limits that cause most faults.
- [Limbo](./limbo): holds, session handles and the fail-closed rule.
- [Global configuration](../../configuration/global#wasm-plugin-sandbox): the `[wasm]` limits.
