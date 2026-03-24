# Acknowledgements

Infrarust V2 wouldn't exist in its current form without the open source projects listed here. Some of them I studied for weeks. Some I cloned locally and went through them at 2am trying to understand a decision design. All of them taught me something.

This is my way of saying thanks.

---

## Minecraft Ecosystem

### [Velocity](https://github.com/PaperMC/Velocity)

The single biggest influence on Infrarust's plugin architecture. Velocity's `ResultedEvent` pattern, its `EventBus` with priorities, the separation between `api/` and proxy internals, the way `KickedFromServerEvent` handles server switches. I borrowed all of that. The approach to opaque packets and multi-version support shaped how I think about a proxy's relationship with the Minecraft protocol: parse what you need, forward what you don't.

### [LimboAPI](https://github.com/Elytrium/LimboAPI)

The Elytrium team built what Velocity wouldn't: virtual worlds without a real backend. LimboAuth, LimboFilter, LimboReconnect. These plugins proved that a proxy can hold players in a fake world for authentication, queuing, or anti-bot checks. I spent a lot of time reading `LimboImpl.java` and `LimboSessionHandlerImpl.java` to understand the spawn sequence, the chunk encoding, how KeepAlive is managed.

The key lesson from LimboAPI was actually what *not* to do. Building virtual worlds as a plugin that hacks into proxy internals is fragile and breaks with updates. In Infrarust V2, the Limbo is a first-class concept built into the proxy itself.

### [PicoLimbo](https://github.com/Quozul/PicoLimbo)

A Rust limbo server that covers 1.7.2 through 1.21.11 with only 44 packets across 49+ protocol versions. That restraint, figuring out the absolute minimum a client needs to stay connected, was exactly what I needed when building my own Limbo. But the biggest help was understanding the join and registry sequence from 1.20.2 onward. The config phase introduced in 1.20.2 (KnownPacks, registry data, FinishConfig) is poorly documented and version-sensitive. PicoLimbo's implementation gave me a working reference for which packets to send, in what order, with what data, to get a client from Login to Play without a real backend. The crate structure (`minecraft_protocol`, `registries`, `registries_data`, `protocol_version`) also served as a reference point for splitting protocol concerns in Rust.

### [Pumpkin](https://github.com/Pumpkin-MC/Pumpkin)

Pumpkin showed me how to structure a Minecraft project in Rust. The crate layering (application -> domain -> foundation), the `pumpkin-protocol` organization by connection state, the `NetworkPalette::Single` chunk encoding. All of it inspired directly my protocol crate design. The `pumpkin-data` approach of generating Rust code from JSON assets at build time via `build.rs` was the answer to my registry data problem for the Limbo system.

Pumpkin also turned out to be a great testing tool. A server that starts in ~15ms and uses ~100MB of RAM is hard to beat for integration tests.

### [Valence](https://github.com/valence-rs/valence)

The `Encode`/`Decode` traits in `valence_protocol` are the best packet serialization design I found in the Rust MC space. Writing to `impl Write`, reading from `&mut &[u8]` with lifetimes for zero-copy, the `Bounded<T, MAX>` type to cap allocations on decode. I took all of those ideas. Separating `PacketEncoder`/`PacketDecoder` from the underlying `Encode`/`Decode` traits (framing vs. serialization) made the codec design much simpler for the proxy needs.

I ended up writing my own protocol crate because Valence is mono-version and parses everything, while a proxy needs opaque packets and version awareness. But the trait design came straight from here.

---

## Rust Networking & Proxy Frameworks

### [Pingora](https://github.com/cloudflare/pingora)

Cloudflare's proxy framework gave me the connection lifecycle model. The `ProxyHttp` trait has ~32 optional callback methods at different request phases (`early_request_filter` -> `upstream_peer` -> `connected_to_upstream` -> `logging`). That maps almost perfectly onto a Minecraft proxy's flow. I've adapted it into Infrarust own phase-based system: `early_filter` -> `handshake_received` -> `resolve_backend` -> `pre_connect` -> `proxy_established` -> `disconnected`.

Pingora also confirmed the workspace structure. A 20-crate layout with a facade crate, a `pingora-core` for traits and runtime, separated concerns for error handling, limits, timeouts, and pooling. If Cloudflare can run that many crates at scale, the 8-crate split is fine.

The choice of `work-stealing multi-threading` over `thread-per-core` reassured me about sticking with tokio's default runtime.
I wanted to try [monoio](https://crates.io/crates/monoio) it seemed pretty good for heavy workload, however Infrarust is much on the IO side.

### [Tower](https://github.com/tower-rs/tower)

The `Service` trait is the foundation of Infrarust's middleware pipeline. A `Layer` wrapping a `Service` into another `Service` gave composable, testable middleware for the connection handshake phase: rate limiting, ban check, IP filter, proxy protocol parsing.

Tower's `BoxFuture` pattern and `DynService` wrapper also taught me how to make async traits dyn-compatible without `async-trait` (I still suck at it but that's fine). This matters now that Rust edition 2024 supports async fn in traits natively. The split between `tower-service` (the trait) and `tower` (the implementations) is exactly what I reproduce with `infrarust-api` vs `infrarust-core`.

### [Rama](https://github.com/plabayo/rama)

Rama is the proxy framework closest to the use case. It's built on Tower's `Service`/`Layer` pattern but operates at the TCP level instead of just HTTP. Seeing the Tower model work for raw TCP connections, not just HTTP request/response, confirmed that the pipeline design (composable middleware for the handshake phase, then bifurcation into stream forwarding) was right way to go.

---

## Plugin System Inspirations

### [Bevy](https://github.com/bevyengine/bevy)

I didn't build a game engine, but Bevy's `Plugin` trait (`build(&self, app: &mut App)`) is the most ergonomic plugin registration pattern I found in Rust. One required method, sensible defaults, chainable builder. The plugin trait follows the same idea: keep it simple, let the proxy handle the complexity.

### [Envoy Proxy](https://github.com/envoyproxy/envoy)

Envoy's Proxy-Wasm implementation proved that WASM plugins work for network proxies in production. The VM-per-plugin model, the shared VM option via `vm_id`, the documented trade-offs between isolation and performance, the filter chain with priorities. This helped me design a cleaner approach to the `PluginLoader` and future WASM implementation.

---

## Rust Crates

Every one of these solved a problem I didn't need to solve anymore:

- [**tokio**](https://github.com/tokio-rs/tokio) - async runtime
- [**fastnbt**](https://github.com/owengage/fastnbt) - NBT serialization for Minecraft registry data in the Limbo system
- [**libdeflater**](https://github.com/adamkewley/libdeflater) - buffer-based compression/decompression, following Velocity's lead
- [**flate2**](https://github.com/rust-lang/flate2-rs) + [**zlib-rs**](https://github.com/trifectatechfoundation/zlib-rs) - streaming compression fallback, pure Rust
- [**papaya**](https://github.com/ibraheemdev/papaya) - concurrent HashMap for the player registry
- [**arc-swap**](https://github.com/vorner/arc-swap) - lock-free config hot-reload
- [**mimalloc**](https://github.com/purpleprotocol/mimalloc_rust) - allocator on musl (the default musl allocator is painfully slow)

---

## Documentation

- [**VitePress**](https://github.com/vuejs/vitepress) - the documentation framework I love, carried over from V1
- [**Minecraft Wiki**](https://minecraft.wiki/) - the protocol documentation that makes projects like Infrarust possible at all. Every packet format, every version difference, every edge case.
- [**Deceased Wiki.VG**](https://minecraft.wiki/w/Minecraft_Wiki:Projects/wiki.vg_merge) - The port of wiki.vg to the Minecraft Wiki, wiki.vg was the websites I used for the V1, to accomodate myself with minecraft. Thank you **Tyler Kennedy** for the awesome projet !

---

Every project here gave something away for free so that others could build with it. Infrarust is one of those others.

Thank's for everything, 
    Shadowner