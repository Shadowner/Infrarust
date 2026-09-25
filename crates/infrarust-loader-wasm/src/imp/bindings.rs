wasmtime::component::bindgen!({
    path: "../infrarust-plugin-wit/wit",
    world: "plugin",
    imports: { default: async | trappable },
    exports: { default: async | trappable },
    additional_derives: [PartialEq],
});
