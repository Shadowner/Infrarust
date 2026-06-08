wasmtime::component::bindgen!({
    path: "../infrarust-plugin-wit/wit",
    world: "plugin",
    imports: { default: trappable },
    exports: { default: trappable },
    with: {
        "infrarust:plugin/types": crate::bindings::infrarust::plugin::types,
    },
});
