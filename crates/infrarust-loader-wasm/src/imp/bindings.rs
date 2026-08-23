//! Host bindings generated from the WIT contract by `bindgen!`.

// `path` is relative to CARGO_MANIFEST_DIR. wasmtime 45 sets async per
// import/export set; the old top-level `async: true` was removed.
//
// `trappable` makes every host import fn return `wasmtime::Result<T>`, so a host
// function can return `Err(..)` to raise a guest trap (used for `ctx == None`
// guards, poisoned locks, and resource-handle errors).
//
// The `player` resource keeps the bindgen-generated marker type; the live
// `Arc<dyn Player>` lives in the store's `ResourceTable` as a `PlayerResource`
// newtype keyed by the handle rep (see `resources.rs`)
wasmtime::component::bindgen!({
    path: "../infrarust-plugin-wit/wit",
    world: "plugin",
    imports: { default: async | trappable },
    exports: { default: async | trappable },
});
