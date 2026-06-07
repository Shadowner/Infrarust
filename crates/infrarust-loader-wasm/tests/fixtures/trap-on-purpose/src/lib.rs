//! WASM-1 `trap-on-purpose` fixture: panics in `on_enable`; the host must contain the
//! trap and quarantine the instance rather than abort the host task.

wit_bindgen::generate!({
    world: "plugin",
    path: "../../../../infrarust-plugin-wit/wit",
    generate_all,
});

struct Component;

fixture_common::raw_fixture!(
    Component,
    id: "trap-on-purpose",
    name: "Trap On Purpose Fixture",
    description: Some("WASM-1 trap fixture".to_string()),
    on_enable: {
        panic!("trap-on-purpose: intentional panic in on_enable");
    }
);

export!(Component);
