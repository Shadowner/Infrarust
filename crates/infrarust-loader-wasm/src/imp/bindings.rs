wasmtime::component::bindgen!({
    path: "wit",
    world: "plugin",
    imports: { default: async | trappable },
    exports: { default: async | trappable },
    additional_derives: [PartialEq],
});
