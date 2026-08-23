# {{project-name}}

An [Infrarust](https://github.com/Shadowner/Infrarust) plugin compiled to a
WebAssembly component (`wasm32-wasip2`), built on `infrarust-plugin-sdk`.

## Prerequisites

```sh
rustup target add wasm32-wasip2
```

## Build

```sh
cargo build --release
```

The component lands at:

```
target/wasm32-wasip2/release/{{crate_name}}.wasm
```

(`wasm32-wasip2` is the default target via `.cargo/config.toml`, so a plain
`cargo build` produces the component.)

## Install

Copy the `.wasm` into your Infrarust `plugins/` directory and (re)start the proxy.
Infrarust discovers any component implementing the `infrarust:plugin@0.2.3` world.

## Develop

Everything you need is in the SDK prelude:

```rust
use infrarust_plugin_sdk::prelude::*;
```

- Subscribe to events: `ctx.on::<PostLoginEvent>(EventPriority::Normal, |e| { ... })`
- Register commands: `ctx.command("name", |inv| { ... }).description("...").register()`
- Read services directly: `Players.online_count()`, `Config.get("key")`
- Schedule work: `ctx.delay(..)`, `ctx.interval(..)`
- Log to the host: `info!`, `warn!`, `error!`, `debug!`, `trace!`

The `#[plugin]` macro derives `metadata()` from your `Cargo.toml`
(`name`, `version`, `authors`, `description`), so keep those fields accurate.
{% if sdk_source == "crates-io" %}
> **Note:** `infrarust-plugin-sdk` is sourced from crates.io. If it is not yet
> published, regenerate with `--define sdk_source=git` or `--define sdk_source=path`.
{% endif %}
