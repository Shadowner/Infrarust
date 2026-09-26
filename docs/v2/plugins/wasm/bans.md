---
title: Bans
description: Make a WASM plugin the proxy's ban provider. Answer login checks, store bans issued from the console, the admin API and other plugins, and learn how deadlines, traps and recovery fail closed.
outline: [2, 3]
---

# Bans

A WASM plugin can own bans end to end, the way a LibertyBans-like native plugin does. Once it is the **ban provider**, the proxy asks it whether each connection may proceed, and every ban issued from the console, the admin API or another plugin is stored by it. The built-in `bans.json` list is not consulted.

To only read or issue bans through whatever provider is active, use `Bans` from [Host Services](./services#bans) with the `ban` capability instead. The provider model itself is described in [Bans](../dev/bans) in the native guide; this page covers what is specific to WASM.

## Selecting the plugin

```toml
[ban]
provider = "guard"
check_timeout = "5s"

[plugins.guard]
permissions = ["ban-provider"]
```

Until the plugin registers, logins are refused and status pings are answered, exactly as with a native provider that is missing.

## Writing a provider

Implement `BanProvider` and register it in `on_enable` with `ctx.provide_bans`:

```rust
use std::cell::RefCell;
use std::rc::Rc;

use infrarust_plugin_sdk::prelude::*;

#[derive(Clone, Default)]
struct Guard(Rc<RefCell<Vec<BanRecord>>>);

impl BanProvider for Guard {
    fn check(&self, attempt: &LoginAttempt) -> Result<Option<BanVerdict>, PluginError> {
        let bans = self.0.borrow();
        Ok(bans
            .iter()
            .filter(|ban| !ban.is_expired())
            .find(|ban| ban.target.matches(attempt))
            .map(|ban| {
                let reason = ban.reason.clone().unwrap_or_default();
                BanVerdict::new(ban.clone()).message(format!("Banned: {reason}"))
            }))
    }

    fn ban(&self, request: BanRequest, source: BanSource) -> Result<BanRecord, PluginError> {
        let mut bans = self.0.borrow_mut();
        let mut record = BanRecord::new(format!("g{}", bans.len() + 1), request.target, source);
        record.reason = request.reason;
        if let Some(duration) = request.duration {
            record = record.lasting(duration);
        }
        bans.push(record.clone());
        Ok(record)
    }

    fn unban(&self, request: UnbanRequest) -> Result<Option<BanRecord>, PluginError> {
        let mut bans = self.0.borrow_mut();
        let found = bans.iter().position(|ban| ban.target == request.target);
        Ok(found.map(|at| bans.remove(at)))
    }

    fn get(&self, target: &BanTarget) -> Result<Option<BanRecord>, PluginError> {
        Ok(self.0.borrow().iter().find(|ban| &ban.target == target).cloned())
    }

    fn list(&self, _query: &BanQuery) -> Result<BanRecordPage, PluginError> {
        Ok(BanRecordPage::new(self.0.borrow().clone(), None))
    }

    fn features(&self) -> BanFeatures {
        BanFeatures::new().ip_ranges(true)
    }
}

#[derive(Default)]
struct GuardPlugin;

#[plugin(id = "guard")]
impl Plugin for GuardPlugin {
    fn on_enable(&self, ctx: &Context) -> Result<(), PluginError> {
        ctx.provide_bans(Guard::default())?;
        Ok(())
    }
}
```

A real provider keeps its bans in its data directory (`/` inside the sandbox), since a trap discards the instance's memory.

| Registration answer | Why |
|---------------------|-----|
| `Ok(())` | The plugin is the ban provider. |
| `ErrorKind::Conflict` | `[ban] provider` names another plugin, `builtin` or `none`. |
| `ErrorKind::PermissionDenied` | The plugin lacks `ban-provider`. |

### What the proxy asks

`check` runs up to three times per connection, like for a native provider: at a status ping (`LoginStage::Status`, address only), before authentication (`PreAuth`, with the username) and after it (`PostAuth`, with the UUID and whether it was verified). `BanTarget::matches` tests a target against an attempt: exact and v4-mapped addresses, CIDR ranges, case-insensitive usernames and UUIDs. A verdict without `message` gets the proxy's default kick text built from the reason and the expiry.

`ban` receives the request and who issued it (`BanSource::Console`, `Player`, `Plugin`, `WebApi` or `System`). The proxy has already turned targets into their canonical form. It posts `BanIssuedEvent` and kicks matching online players after `ban` answers, so the provider only stores. `unban` answers the removed record, `get` and `list` read. `BanRecord` carries the typed source, and the proxy keeps it on the native `BanEntry`.

`features()` is read at registration: without `ip_ranges`, the proxy refuses range bans before they reach the plugin.

## Deadlines and failures

Every call waits at most the event budget (`[events] handler_timeout`); `check` is also bounded by `[ban] check_timeout`, whichever ends first. The proxy treats a WASM provider like a native one that failed:

| The plugin | `check` at login | `check` at a status ping | `ban`, `unban`, `get`, `list` |
|------------|------------------|--------------------------|-------------------------------|
| answers `Err(message)` | login refused | ping answered | the caller gets the error |
| traps, misses the deadline, or is quarantined | login refused | ping answered | the caller gets an `unavailable` error |

A refused login shows *Your ban status cannot be checked right now. Please try again later.* and the proxy logs the cause at `error`.

## Calling the ban service from the provider

The host never calls back into an instance that is still running. While a plugin is the ban provider, a `Bans::ban`, `unban`, `get` or `list` made from any of its own calls (a command, a scheduled task, an event handler or a provider export) would have to reach that same instance, so the host answers it at once with `ErrorKind::Unavailable`, without waiting for a deadline. A provider's own `/ban` command stores the ban itself and kicks with `Player::disconnect`; `BanIssuedEvent` only fires for bans that went through the ban service.

## Recovery

A trap does not unregister the provider. While the plugin recovers or sits in quarantine, `check` fails and logins are refused; the next call after the recovery reaches the fresh instance. The recovered instance runs `on_enable` again, and its `provide_bans` succeeds, updates the features and keeps the one registration.

## The contract

```wit
interface providers {
    register-ban-provider: func(features: ban-features) -> result<_, host-error>;
}

interface guest {
    ban-provider-check: func(attempt: login-attempt) -> result<option<ban-verdict>, string>;
    ban-provider-ban: func(request: ban-request, source: ban-source) -> result<ban-record, string>;
    ban-provider-unban: func(request: unban-request) -> result<option<ban-record>, string>;
    ban-provider-get: func(target: ban-target) -> result<option<ban-record>, string>;
    ban-provider-list: func(query: ban-query) -> result<ban-record-page, string>;
}
```

The records are defined in `ban-service`; see the [WIT API reference](./api-reference#host-services). The SDK generates these exports for every plugin and answers `err("this plugin provides no bans")` until `provide_bans` ran.

## See also

- [Bans](../dev/bans): the provider model, targets, sources and events.
- [Permissions](./permissions): the other provider a WASM plugin can be.
- [Fault Model](./fault-model): traps, recovery and quarantine.
