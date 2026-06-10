---
title: Building & Toolchain
description: The exact toolchain and Cargo setup to compile an Infrarust WASM plugin, plus the dual native + WASM build pattern.
outline: [2, 3]
---

# Building & Toolchain

An Infrarust WASM plugin is a Rust `cdylib` crate that depends on `infrarust-plugin-sdk` and compiles to a WebAssembly component for the `wasm32-wasip2` target. The output is a single `.wasm` file that the proxy loads at startup. This page covers the toolchain, the Cargo setup, the build command, and the dual native + WASM layout used by the built-in stats plugin.

## Prerequisites

You need a Rust toolchain with edition 2024 support and the `wasm32-wasip2` target installed:

```bash
rustup target add wasm32-wasip2
```

The proxy host and the SDK are pinned to edition 2024 and `rust-version = "1.94"`. Use a toolchain at or above that version.

## Cargo.toml

A plugin crate sets `crate-type = ["cdylib"]` and depends on `infrarust-plugin-sdk`. Pick the SDK source that matches how you consume Infrarust:

::: code-group

```toml [git]
[package]
name = "my-plugin"
version = "0.1.0"
edition = "2024"
publish = false

[lib]
crate-type = ["cdylib"]

[dependencies]
infrarust-plugin-sdk = { git = "https://github.com/Shadowner/Infrarust.git" }

[profile.release]
opt-level = "s"
lto = true
strip = true
panic = "abort"

[profile.dev]
panic = "abort"
```

```toml [path]
[package]
name = "my-plugin"
version = "0.1.0"
edition = "2024"
publish = false

[lib]
crate-type = ["cdylib"]

[dependencies]
infrarust-plugin-sdk = { path = "../Infrarust/crates/infrarust-plugin-sdk" }

[profile.release]
opt-level = "s"
lto = true
strip = true
panic = "abort"

[profile.dev]
panic = "abort"
```

```toml [crates.io]
[package]
name = "my-plugin"
version = "0.1.0"
edition = "2024"
publish = false

[lib]
crate-type = ["cdylib"]

[dependencies]
infrarust-plugin-sdk = "2.0.0-beta.1"

[profile.release]
opt-level = "s"
lto = true
strip = true
panic = "abort"

[profile.dev]
panic = "abort"
```

:::

The SDK re-exports everything a plugin needs through its prelude (the `Plugin` trait, the `#[plugin]` macro, `Context`, events, services, and the `info!`/`warn!`/`error!`/`debug!`/`trace!` log macros):

```rust
use infrarust_plugin_sdk::prelude::*;
```

## No cargo-component required

The component bindings (`wit_bindgen::generate!`) are embedded inside `infrarust-plugin-sdk`. A plain `cargo build` against the `wasm32-wasip2` target emits a WebAssembly component directly. You do not install or run `cargo-component`.

::: tip
`wasm32-wasip2` produces a component by default, unlike the older `wasm32-wasi` (preview 1) target, which produces a core module. The SDK targets preview 2 for this reason.
:::

## Build command

Build for the WASM target in release mode:

```bash
cargo build --release --target wasm32-wasip2
```

The component artifact is at `target/wasm32-wasip2/release/<crate>.wasm`, where `<crate>` is the package name with each hyphen rewritten as `_`. A package named `my-plugin` produces `target/wasm32-wasip2/release/my_plugin.wasm`.

You can avoid passing `--target` on every build by setting it as the default in `.cargo/config.toml`:

```toml
# .cargo/config.toml
[build]
target = "wasm32-wasip2"
```

With that file present, `cargo build --release` alone produces the component.

## Release profile

The recommended release profile keeps the binary small and its behavior predictable:

```toml
[profile.release]
opt-level = "s"   # optimize for size
lto = true        # link-time optimization
strip = true      # drop symbols
panic = "abort"   # no unwinding

[profile.dev]
panic = "abort"
```

::: warning
Set `panic = "abort"` in both `[profile.release]` and `[profile.dev]`. The guest is single-threaded and has no unwinding runtime; a panic in the plugin aborts the WASM instance, and the host treats that as a trap. Matching the panic strategy across profiles avoids surprising differences between debug and release builds.
:::

## Dual native + WASM build

The built-in stats plugin ships as both a native plugin (compiled into the proxy) and a WASM component, sharing one body of logic. Use this layout when you want the same plugin to run in either environment, or when the WASM API does not yet expose a capability you need natively.

The plugin is split into three crates:

```
plugins/infrarust-plugin-stats/
├── Cargo.toml          # core: shared, dependency-light logic
├── src/
├── native/
│   └── Cargo.toml      # native build (depends on infrarust-api)
└── wasm/
    └── Cargo.toml      # WASM build (cdylib + infrarust-plugin-sdk)
```

The **core** crate holds the logic with no host bindings. It depends on neither `infrarust-api` nor `infrarust-plugin-sdk`, so both builds reuse it:

```toml
# plugins/infrarust-plugin-stats/Cargo.toml
[package]
name = "infrarust-plugin-stats"
version.workspace = true
edition.workspace = true
description = "Shared logic for the Infrarust stats plugin (native + WASM double distribution)"
```

The wasm crate is the component target. It is a `cdylib`, depends on the core crate and the SDK, and declares its own `[workspace]` so it builds independently of the main workspace:

```toml
# plugins/infrarust-plugin-stats/wasm/Cargo.toml
[package]
name = "infrarust-plugin-stats-wasm"
version = "0.1.0"
edition = "2024"
publish = false

[lib]
crate-type = ["cdylib"]

[dependencies]
infrarust-plugin-stats = { path = "../" }
infrarust-plugin-sdk = { path = "../../../crates/infrarust-plugin-sdk" }

[workspace]

[profile.release]
opt-level = "s"
lto = true
strip = true
panic = "abort"

[profile.dev]
panic = "abort"
```

The wasm crate's `lib.rs` is a thin adapter: it implements the synchronous SDK `Plugin` trait, tags the impl with `#[plugin]`, and forwards to the core crate so behavior matches the native build.

```rust
use infrarust_plugin_sdk::prelude::*;
use infrarust_plugin_stats as core;

#[derive(Default)]
struct StatsPlugin;

#[plugin(id = "stats", name = "Stats Plugin")] // [!code focus]
impl Plugin for StatsPlugin {
    fn on_enable(&self, ctx: &Context) -> Result<(), String> {
        ctx.on::<PostLoginEvent>(EventPriority::Normal, |event| {
            info!("[stats] {}", core::join_log(&event.profile.username));
        });
        // ... commands and other listeners ...
        Ok(())
    }
}
```

::: info
The guest `Plugin` trait is synchronous: `on_enable(&self, ctx: &Context) -> Result<(), String>`. There is no `async`/`BoxFuture` in the WASM API. That signature belongs to the native `Plugin` trait. The guest is single-threaded with no async runtime, so keep mutable plugin state in `Cell`/`RefCell` fields. See [Getting Started](./getting-started).
:::

## How the test fixtures are built

The `infrarust-loader-wasm` crate compiles its WASM test fixtures and the stats component from a `build.rs` script (gated behind the `wasm` cargo feature). Each fixture is built with the same invocation you would run by hand, plus a `--target-dir` so the artifacts land in `OUT_DIR`:

```rust
// crates/infrarust-loader-wasm/build.rs
cmd.current_dir(cwd).args([
    "build",
    "--release",
    "--target",
    "wasm32-wasip2",
    "--target-dir",
]);
```

The fixtures live in a self-contained workspace (`crates/infrarust-loader-wasm/tests/fixtures/`) with one member crate per scenario, each a `cdylib` depending on the SDK. That workspace is a working reference for the minimal crate shape if you want to mirror the setup.

::: tip
If the `wasm32-wasip2` target is missing, the build script detects it and emits a warning instead of failing, then skips the fixture tests. Install the target with `rustup target add wasm32-wasip2` to run them.
:::

## Troubleshooting

| Symptom | Cause | Fix |
|---------|-------|-----|
| ``error: the `wasm32-wasip2` target may not be installed`` | The target is not installed for the active toolchain | `rustup target add wasm32-wasip2` |
| ``can't find crate for `std` `` | Same as above (the std-bearing target is absent) | `rustup target add wasm32-wasip2` |
| Debug and release behave differently on a panic | `panic` strategy differs between profiles | Set `panic = "abort"` in both `[profile.release]` and `[profile.dev]` |
| Binary is larger than expected | Size optimizations are off | Set `opt-level = "s"`, `lto = true`, `strip = true` in `[profile.release]` |
| Cannot find the `.wasm` artifact | Looking under the wrong name | Look in `target/wasm32-wasip2/release/`; each hyphen in the package name becomes `_` |
| `warning: dropping unsupported crate type 'dylib'` | The crate declares a non-`cdylib` type for this target | Use `crate-type = ["cdylib"]` |

## Next steps

- [Getting Started](./getting-started): write the plugin body: events, commands, and services.
- [Deploying](./deploying): install the `.wasm`, grant capabilities, and configure the plugin.
- [Examples](./examples): full plugins, including the dual native + WASM stats plugin.
- [WASM plugins overview](./): how the host loads and sandboxes components.

## See also

- [Native plugin development](../dev/getting-started): the async `Plugin` trait compiled into the proxy.
- [Configuration](../../configuration/): proxy and plugin configuration reference.
