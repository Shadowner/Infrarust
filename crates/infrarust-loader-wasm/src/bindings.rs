//! Host bindings generated from the WIT contract by `bindgen!`.

// `path` is relative to CARGO_MANIFEST_DIR. wasmtime 45 sets async per
// import/export set; the old top-level `async: true` was removed.
wasmtime::component::bindgen!({
    path: "../infrarust-plugin-wit/wit",
    world: "plugin",
    imports: { default: async },
    exports: { default: async },
});
