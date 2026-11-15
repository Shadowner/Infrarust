wasmtime::component::bindgen!({
    path: "wit",
    world: "plugin",
    imports: { default: async | trappable },
    exports: { default: async | trappable },
    additional_derives: [PartialEq],
    with: {
        "infrarust:plugin/limbo.limbo-session": crate::hosts::limbo::LimboSessionResource,
        "infrarust:plugin/limbo.limbo-session-handle": crate::hosts::limbo::LimboSessionHandleResource,
    },
});
