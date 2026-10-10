wit_bindgen::generate!({
    world: "plugin",
    path: "../../../../infrarust-plugin-wit/wit",
    generate_all,
});

struct Component;

fixture_common::raw_fixture!(
    Component,
    id: "cpu-spin",
    name: "CPU Spin Fixture",
    description: Some("WASM-1 cpu-spin fixture".to_string()),
    on_enable: {
        // `black_box` keeps the loop body from being optimised out; Rust preserves
        // infinite loops regardless. Only the epoch interrupt breaks out (as a trap).
        loop {
            std::hint::black_box(0u64);
        }
    }
);

export!(Component);
